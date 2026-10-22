//! install-rom orchestration: checks -> backup -> rebuild -> batches.
//! Batch order: super group (weight 1) -> boot (2) -> modem (3) -> rest (4).
//! Barrier per batch: all threads must validate, else finish the batch
//! and roll back. Slot is NEVER switched here (caller does bootctl).

use crate::{backup, device, live, ota, rebuild, weights, Out};
use payload_hs::{layout, manifest};
use std::collections::{BTreeMap, HashMap};

pub struct Args {
    pub zip: String,
    pub status_fd: Option<i32>,
    pub slot: Option<char>,
    pub booted_slot: Option<char>,
    pub sysroot: String,
    pub tmpdir: String,
    pub threads: usize,
    pub readback: bool,
}

pub fn flag(args: &[String], name: &str) -> Option<String> {
    // Both "--flag value" and "--flag=value" forms (the installer execs
    // the glued form; humans type the split one).
    let glued = format!("{name}=");
    let mut it = args.iter().peekable();
    while let Some(a) = it.next() {
        if a == name {
            return it.peek().map(|s| s.to_string());
        }
        if let Some(v) = a.strip_prefix(&glued) {
            return Some(v.to_string());
        }
    }
    None
}

fn fail(out: &mut Out, msg: &str, code: i32) -> i32 {
    out.ui_print(&format!("{msg}"));
    code
}

pub fn run(a: Args) -> i32 {
    let mut out = Out::new(a.status_fd);
    // Progress bar starts at zero: an initial 1.0 would pin the GUI bar
    // at 100% and every later set_progress would jump backwards.
    out.line("progress 0.0 600");

    // --- 0. identity: device + slots (universal, no hardcodes) ---
    let device = match device::getprop("ro.product.device") {
        Ok(d) if !d.is_empty() => d,
        _ => return fail(&mut out, "no ro.product.device prop", crate::EXIT_FAIL),
    };
    // Booted slot is always real (ro.boot.slot_suffix). --slot only
    // forces the TARGET (host testing); it must never rewrite history
    // into live::query, or backup/fit/kill split current/target wrong
    // and a live slot gets overwritten.
    let booted = match device::booted_slot_resolved(a.booted_slot) {
        Ok(b) => b,
        Err(e) => return fail(&mut out, &e, crate::EXIT_FAIL),
    };
    let target = match a.slot {
        Some(s) => s,
        None => device::target_slot(booted),
    };
    if target == booted {
        return fail(
            &mut out,
            &format!("target slot _{target} == booted slot: refusing"),
            crate::EXIT_FAIL,
        );
    }
    out.ui_print(&format!("device={device} guest slot=_{target}"));

    // --- 1. device gate: pre-device must match (hard error, no fallback) ---
    if let Err(e) = ota::check_device(&a.zip, &device) {
        return fail(&mut out, &e, crate::EXIT_DEVICE_MISMATCH);
    }

    // --- 2. manifest: full OTA only, else stock path ---
    let layout = match payload_hs::zipx::probe(&a.zip) {
        Ok(l) => l,
        Err(e) => return fail(&mut out, &format!("zip probe: {e}"), crate::EXIT_FAIL),
    };
    let parts = match manifest::parse(&layout.manifest) {
        Ok(p) => p,
        Err(e) => return fail(&mut out, &format!("manifest: {e}"), crate::EXIT_FAIL),
    };
    if let Err(r) = manifest::verdict_full(&parts) {
        out.ui_print(&format!("not full OTA ({r}), stock path"));
        return crate::EXIT_NOT_FULL;
    }
    // Delta signal #2 (peridot parity): minor_version != 0 means a delta
    // payload even if every op looks plain -> stock path, never an error.
    match manifest::minor_version(&layout.manifest) {
        Ok(v) if v != 0 => {
            out.ui_print(&format!(
                "not full OTA (manifest minor_version={v}), stock path"
            ));
            return crate::EXIT_NOT_FULL;
        }
        Err(e) => return fail(&mut out, &format!("manifest: {e}"), crate::EXIT_FAIL),
        _ => {}
    }

    // Exact-tiling check (peridot validate() parity): a stock
    // update_engine would refuse these too -> hard error, never stock.
    // Runs on raw manifest sizes (block size known from probe already).
    if let Err(e) = manifest::validate_tiling(&parts, layout.block_size) {
        return fail(&mut out, &e, crate::EXIT_FAIL);
    }

    // --- 3. helpers: siw must exist, else stock path. (lptools_new was
    // historically checked here too, but no code path invokes it: all LP
    // surgery goes through siw. Requiring it would only force spurious
    // stock fallbacks.)
    let siw = format!("{}/siw", a.sysroot);
    if !std::path::Path::new(&siw).exists() {
        out.ui_print(&format!("helper missing: {siw}, stock path"));
        return crate::EXIT_NO_HELPERS;
    }

    // --- 4. live super state + fit (phase 2) ---
    let live = match live::query(&siw, booted) {
        Ok(l) => l,
        Err(e) => return fail(&mut out, &e, crate::EXIT_FAIL),
    };
    out.log(&format!(
        "threads={} super={} current={} target={} group={}",
        a.threads,
        live.super_size,
        live.current.len(),
        live.target.len(),
        live.target_group
    ));
    let dm = match manifest::parse_dynmeta(&layout.manifest) {
        Ok(d) => d,
        Err(e) => return fail(&mut out, &format!("dynmeta: {e}"), crate::EXIT_FAIL),
    };
    let bs = layout.block_size;
    let out_of = |name: &str| -> u64 {
        parts
            .iter()
            .find(|p| p.name == name)
            .map(|p| p.out_size(bs))
            .unwrap_or(0)
    };
    let mut plans = Vec::new();
    let mut group_of: HashMap<String, String> = HashMap::new();
    if let Some(m) = &dm {
        for g in &m.groups {
            let req: Vec<(String, u64)> = g
                .parts
                .iter()
                .map(|n| (n.clone(), out_of(n)))
                .collect();
            for n in &g.parts {
                group_of.insert(n.clone(), g.name.clone());
            }
            plans.push(layout::plan_group(&req, &g.parts, g.size, &live));
        }
    }
    let fit = layout::strictest(&plans);
    out.ui_print(&format!("fit {}", fit.code()));
    let kill: Vec<String> = match &fit {
        layout::FitPlan::AsIs => Vec::new(),
        layout::FitPlan::Kill(k) => k.clone(),
        layout::FitPlan::WipeAll => {
            // wipe = every logical resident of both slots
            let mut all: Vec<String> = live
                .current
                .keys()
                .chain(live.target.keys())
                .cloned()
                .collect();
            all.sort();
            all.dedup();
            all
        }
        layout::FitPlan::Impossible => {
            return fail(
                &mut out,
                "payload does not fit even a bare super: abort, nothing written",
                crate::EXIT_IMPOSSIBLE,
            )
        }
    };
    if !kill.is_empty() {
        out.ui_print(&format!("reclaim: {}", kill.join(",")));
    }

    // --- 5. order: weights; backup critical (2+4) of the TARGET slot ---
    let in_group = |n: &str| group_of.contains_key(n);
    let w_of = |n: &str| weights::weight(n, in_group(n));
    let present_target = |base: &str| -> Option<(String, u64)> {
        let full = format!("{base}_{target}");
        // Physical node backing a slot-suffixed partition.
        let node = format!("/dev/block/by-name/{full}");
        live.target.get(&full).map(|s| (node, *s))
    };
    // Scheme phase 0: every BLOCK (non-group) target must already exist
    // as a block node. metadata() follows the by-name symlink (unlike
    // symlink_metadata, which describes the link itself and is never a
    // block device). Nothing has been written yet; fail here, not
    // mid-flash. Logical members are created from the group below.
    {
        use std::os::unix::fs::FileTypeExt;
        for p in &parts {
            if group_of.contains_key(&p.name) {
                continue;
            }
            let node = device::by_name(&p.name, target);
            match std::fs::metadata(&node) {
                Ok(md) if md.file_type().is_block_device() => {}
                Ok(_) => {
                    return fail(
                        &mut out,
                        &format!(
                            "BLOCK target not a block device: {node}, nothing written"
                        ),
                        crate::EXIT_FAIL,
                    )
                }
                Err(e) => {
                    return fail(
                        &mut out,
                        &format!("BLOCK target missing: {node} ({e}), nothing written"),
                        crate::EXIT_FAIL,
                    )
                }
            }
        }
    }
    // Commit point: every phase-0 check passed (probe, verdict, minor,
    // tiling, helpers, live, fit, by-name). Announce like update_engine.
    {
        let total: u64 = parts.iter().map(|p| p.out_size(bs)).sum();
        out.ui_print(&format!(
            "installing to opposite slot _{target} (full OTA, {} partitions, {} MB out)",
            parts.len(),
            total / 1_000_000
        ));
    }
    let mut batches: BTreeMap<u8, Vec<&manifest::Part>> = BTreeMap::new();
    for p in &parts {
        batches.entry(w_of(&p.name)).or_default().push(p);
    }
    // LPT inside every batch: biggest expanded first.
    for v in batches.values_mut() {
        v.sort_by_key(|p| std::cmp::Reverse(p.out_size(bs)));
    }

    let _ = std::fs::create_dir_all(&a.tmpdir);
    let mut backed: Vec<Backed> = Vec::new(); // rollback ledger
    for p in parts.iter().filter(|p| weights::is_critical(w_of(&p.name))) {
        if let Some((node, size)) = present_target(&p.name) {
            let file = format!("{}/{}.img", a.tmpdir, p.name);
            out.ui_print(&format!("backup {}", p.name));
            if let Err(e) = backup::dump(&node, &file, size) {
                return fail(&mut out, &format!("backup {}: {e}", p.name), crate::EXIT_FAIL);
            }
            backed.push(Backed::Dump { node, file });
        } else {
            // Not a live resident yet: created fresh by the rebuild.
            // Rollback removes it to restore the prior state.
            backed.push(Backed::Fresh {
                full: format!("{}_{target}", p.name),
            });
        }
    }

    // --- 6. rebuild, single thread: unmount+unmap victims, remove, create, map ---
    out.ui_print("rebuilding target group (single-threaded)");
    let mut to_remove: Vec<String> = kill.clone();
    if let Some(m) = &dm {
        for g in &m.groups {
            for n in &g.parts {
                to_remove.push(format!("{n}_{target}"));
            }
        }
    }
    to_remove.sort();
    to_remove.dedup();
    // Unmount + unmap EVERYTHING on super first: raw LP writes are
    // refused while any dm mapping holds the device. Removals happen
    // only after the table is fully unmapped.
    match rebuild::unmount_unmap_all(&siw, &live) {
        Ok(gone) => out.ui_print(&format!("unmapped {} dm node(s)", gone.len())),
        Err(e) => return fail(&mut out, &e, crate::EXIT_FAIL),
    }
    for full in &to_remove {
        out.log(&format!("victim=[{full}]"));
        if let Err(e) = rebuild::remove_meta(&siw, full) {
            return fail(&mut out, &format!("remove {full}: {e}"), crate::EXIT_FAIL);
        }
    }
    // create target members with manifest sizes (group budget already
    // fits). Group name = the live slot-suffixed one when known, else the
    // manifest name, else manifest name + slot suffix.
    let live_group = if !live.target_group.is_empty() {
        live.target_group.clone()
    } else {
        String::new()
    };
    if let Some(m) = &dm {
        for g in &m.groups {
            let grp = if !live_group.is_empty() {
                live_group.clone()
            } else {
                g.name.clone()
            };
            let grp2 = format!("{}_{target}", g.name);
            for n in &g.parts {
                let full = format!("{n}_{target}");
                out.log(&format!(
                    "create=[{full}] group=[{grp}] part=[{n}]"
                ));
                if let Err(e) = rebuild::create_meta(&siw, &full, &grp, out_of(n))
                    .or_else(|_| rebuild::create_meta(&siw, &full, &grp2, out_of(n)))
                {
                    return fail(&mut out, &format!("create {full}: {e}"), crate::EXIT_FAIL);
                }
            }
        }
    }
    // map every payload partition of the target slot for writing
    let mut node_of: HashMap<String, String> = HashMap::new();
    for p in &parts {
        if group_of.contains_key(&p.name) {
            let full = format!("{}_{target}", p.name);
            if let Err(e) = rebuild::map_writable(&siw, &full) {
                return fail(&mut out, &format!("map {full}: {e}"), crate::EXIT_FAIL);
            }
            node_of.insert(p.name.clone(), format!("/dev/block/mapper/{full}"));
        } else {
            node_of.insert(p.name.clone(), device::by_name(&p.name, target));
        }
    }

    // --- 7. batches 1->2->3->4, barrier + per-thread validation ---
    // Per-stage progress bands: stage 1 fills the bar to 65%, stage 2
    // to 75%, stage 3 to 85%, stage 4 to 95%; the post-processing
    // tail (sync, slot switch) takes it to 100%. Bands keep small
    // stages visible where a flat byte-proportional 5..95% buried them.
    let batch_totals: HashMap<u8, u64> = batches
        .iter()
        .map(|(w, v)| (*w, v.iter().map(|p| p.out_size(bs)).sum()))
        .collect();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(a.threads)
        .build();
    let pool = match pool {
        Ok(p) => p,
        Err(e) => return fail(&mut out, &format!("threads: {e}"), crate::EXIT_FAIL),
    };
    for (w, batch) in &batches {
        let label = match *w {
            1 => "system partitions",
            2 => "boot partitions",
            3 => "modem partitions",
            _ => "remaining partitions",
        };
        let names: Vec<&str> = batch.iter().map(|p| p.name.as_str()).collect();
        out.ui_print(&format!(
            "stage {w}/4: {}: {}",
            label,
            names.join(", ")
        ));
        // Open one fd per node (O_EXCL claim: fail fast if a target is
        // busy instead of writing under a live user). Shared by the op
        // workers of this batch, one mutex each; ops on different nodes
        // never contend, ops on one node touch disjoint extents.
        let mut node_fds: HashMap<String, std::sync::Mutex<std::fs::File>> =
            HashMap::new();
        {
            use std::os::unix::fs::OpenOptionsExt;
            for j in batch.iter() {
                let node = &node_of[&j.name];
                match std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .custom_flags(libc::O_EXCL)
                    .open(node)
                {
                    Ok(f) => {
                        node_fds.insert(j.name.clone(), std::sync::Mutex::new(f));
                    }
                    Err(e) => {
                        let msg = if e.raw_os_error() == Some(libc::EBUSY) {
                            format!("open {node}: busy (mounted or held open)")
                        } else {
                            format!("open {node}: {e}")
                        };
                        out.ui_print(&format!("stage {w}/4 FAILED: {msg}"));
                        // Rollback restores critical (2+4) nodes; weight 1
                        // (super group) has no restorable state yet, and its
                        // Fresh ledger would delete just-rebuilt entries.
                        if *w != 1 {
                            rollback(&mut out, &siw, &backed);
                        }
                        return fail(
                            &mut out,
                            "install failed; critical partitions restored, slot NOT switched",
                            crate::EXIT_FAIL,
                        );
                    }
                }
            }
        }
        // Flatten to op tasks: every op decodes independently (CPU-bound,
        // the actual parallelism), scatter-writes go under the per-node
        // lock. ZERO fills inline (chunked); DISCARD needs no writes.
        struct OpTask<'x> {
            part: &'x str,
            op: &'x manifest::Op,
            op_i: usize,
            expanded: u64,
        }
        let mut tasks: Vec<OpTask> = Vec::new();
        for j in batch.iter() {
            for (i, o) in j.ops.iter().enumerate() {
                if o.typ == manifest::OP_DISCARD {
                    continue;
                }
                let mut expanded = 0u64;
                for e in &o.dst {
                    expanded += e.1 * bs;
                }
                tasks.push(OpTask {
                    part: &j.name,
                    op: o,
                    op_i: i,
                    expanded,
                });
            }
        }
        // Live progress: atomic byte counter, GUI update on 0.5% steps
        let task_done = std::sync::atomic::AtomicU64::new(0);
        let next_pct = std::sync::atomic::AtomicU64::new(0);
        // Op granularity forecast for the log: a huge max op here
        // predicts a quiet bar tail (decode of one op is atomic).
        {
            let max_op = tasks.iter().map(|t| t.expanded).max().unwrap_or(0);
            out.log(&format!(
                "batch {w}: {} ops, max op {} MB",
                tasks.len(),
                max_op / 1_000_000
            ));
        }
        let (lo, hi) = match *w {
            1 => (0.05, 0.65),
            2 => (0.65, 0.75),
            3 => (0.75, 0.85),
            _ => (0.85, 0.95),
        };
        let btotal = batch_totals.get(w).copied().unwrap_or(0).max(1);
        let out_mtx = std::sync::Mutex::new(&mut out);
        // Piecewise credit shared by every write site below (ZERO chunks,
        // REPLACE pieces, end-of-task remainder): bytes are credited as
        // actually written, so even a lone thread chewing one giant op
        // ticks the bar instead of freezing it. Emission only on
        // high-water-mark cross (0.5% bar steps), same gate as before.
        let span = hi - lo;
        let credit = |bytes: u64| {
            if bytes == 0 {
                return;
            }
            let dd = task_done
                .fetch_add(bytes, std::sync::atomic::Ordering::Relaxed)
                + bytes;
            let fr = lo + dd as f64 / btotal as f64 * span;
            let pc = (fr * 200.0) as u64;
            if pc > next_pct.fetch_max(pc, std::sync::atomic::Ordering::Relaxed) {
                if let Ok(mut g) = out_mtx.lock() {
                    g.set_progress(fr.min(1.0));
                }
            }
        };
        let res: Vec<Result<(), String>> = pool.install(|| {
            use rayon::prelude::*;
            tasks
                .par_iter()
                .map(|t| {
                    let tag = format!(
                        "{} op {} type={} off={} len={}",
                        t.part, t.op_i, t.op.typ, t.op.off, t.op.len
                    );
                    // Bytes credited as actually written (see `credit`
                    // above); the bulk remainder at task end only covers
                    // what piecewise sites have not.
                    let mut credited = 0u64;
                    if t.op.typ == manifest::OP_ZERO {
                        let mu = node_fds.get(t.part).ok_or_else(|| {
                            format!("{tag}: node gone (internal error)")
                        })?;
                        let mut f = mu.lock().map_err(|_| {
                            format!("{tag}: node lock poisoned")
                        })?;
                        use std::io::{Seek, SeekFrom, Write};
                        let zeros = vec![0u8; 1 << 20];
                        for (sb, nb) in &t.op.dst {
                            let mut at = sb * bs;
                            let mut n = nb * bs;
                            f.seek(SeekFrom::Start(at))
                                .map_err(|e| format!("{tag}: oseek: {e}"))?;
                            while n > 0 {
                                let k = (n as usize).min(zeros.len());
                                f.write_all(&zeros[..k])
                                    .map_err(|e| format!("{tag}: zero fill: {e}"))?;
                                at += k as u64;
                                n -= k as u64;
                                credited += k as u64;
                                credit(k as u64);
                            }
                        }
                        drop(f);
                    } else {
                        let plain =
                            payload_hs::extract::fetch_decode(&a.zip, layout.data_start, t.op)
                                .map_err(|e| format!("{tag}: {e}"))?;
                        if plain.len() as u64 != t.expanded {
                            return Err(format!(
                                "{tag}: decode size {} != dst {} (manifest/extents mismatch?)",
                                plain.len(),
                                t.expanded
                            ));
                        }
                        let mu = node_fds.get(t.part).ok_or_else(|| {
                            format!("{tag}: node gone (internal error)")
                        })?;
                        let mut f = mu.lock().map_err(|_| {
                            format!("{tag}: node lock poisoned")
                        })?;
                        use std::io::{Seek, SeekFrom, Write};
                        let mut pos = 0usize;
                        for (sb, nb) in &t.op.dst {
                            let n = (*nb * bs) as usize;
                            let chunk = plain.get(pos..pos + n).ok_or_else(|| {
                                format!("{tag}: short decode (want {n})")
                            })?;
                            f.seek(SeekFrom::Start(sb * bs))
                                .map_err(|e| format!("{tag}: oseek: {e}"))?;
                            // Piecewise credit (8 MiB pieces): a single giant
                            // extent would otherwise hold its thread
                            // silent for tens of seconds — the frozen tail
                            // of the progress bar.
                            for piece in chunk.chunks(8 << 20) {
                                f.write_all(piece)
                                    .map_err(|e| format!("{tag}: owrite: {e}"))?;
                                let k = piece.len() as u64;
                                credited += k;
                                credit(k);
                            }
                            pos += n;
                        }
                        drop(f);
                    }
                    // Remainder of the bulk credit (zero when piecewise
                    // sites above already accounted every byte).
                    credit(t.expanded.saturating_sub(credited));
                    Ok(())
                })
                .collect()
        });
        drop(out_mtx);
        // barrier: every op must validate, else finish batch and roll back.
        // collect() waits for all workers: the batch always drains fully.
        let mut failed: Option<String> = None;
        for r in res {
            if let Err(e) = r {
                if failed.is_none() {
                    failed = Some(e);
                }
            }
        }
        if let Some(e) = failed {
            out.ui_print(&format!("stage {w}/4 FAILED: {e}"));
            if *w != 1 {
                rollback(&mut out, &siw, &backed);
            }
            return fail(
                &mut out,
                "install failed; critical partitions restored, slot NOT switched",
                crate::EXIT_FAIL,
            );
        }
        out.set_progress(hi);
        // fsync every node of the finished batch before validation: the
        // hash below must cover durable bytes, not page cache.
        for p in batch.iter() {
            if let Err(e) = backup::fsync_node(&node_of[&p.name]) {
                if *w != 1 {
                    rollback(&mut out, &siw, &backed);
                }
                return fail(&mut out, &format!("fsync {}: {e}", p.name), crate::EXIT_FAIL);
            }
        }
        // Re-read + hash from the node (post-fsync) and compare against
        // new_partition_info.hash. Fresh page cache makes this cheap;
        // torn writes surface here instead of booting into them.
        for p in batch.iter() {
            let sum = match payload_hs::extract::sha256_file_n(
                &node_of[&p.name],
                p.out_size(bs),
            ) {
                Ok(s) => s,
                Err(e) => {
                    if *w != 1 {
                        rollback(&mut out, &siw, &backed);
                    }
                    return fail(&mut out, &format!("hash {}: {e}", p.name), crate::EXIT_FAIL);
                }
            };
            if p.new_hash.len() == 32 && sum[..] != p.new_hash[..] {
                if *w != 1 {
                    rollback(&mut out, &siw, &backed);
                }
                return fail(
                    &mut out,
                    &format!("{}: sha256 mismatch (media or decode error)", p.name),
                    crate::EXIT_FAIL,
                );
            }
        }
        // --readback: BLKFLSBUF + re-read hash from storage (not page
        // cache). Only when asked: it doubles batch I/O.
        if a.readback {
            for p in batch.iter() {
                let node = &node_of[&p.name];
                match backup::readback_hash(node, p.out_size(bs)) {
                    Ok(sum) => {
                        if p.new_hash.len() == 32 && sum[..] != p.new_hash[..] {
                            if *w != 1 {
                                rollback(&mut out, &siw, &backed);
                            }
                            return fail(
                                &mut out,
                                &format!("{}: readback sha256 mismatch", p.name),
                                crate::EXIT_FAIL,
                            );
                        }
                    }
                    Err(e) => {
                        if *w != 1 {
                            rollback(&mut out, &siw, &backed);
                        }
                        return fail(
                            &mut out,
                            &format!("{}: readback failed: {e}", p.name),
                            crate::EXIT_FAIL,
                        );
                    }
                }
            }
            out.log(&format!("batch {w}: readback ok"));
        }
        out.ui_print(&format!("stage {w}/4 done"));
    }

    // --- 8. done: sync, leave the slot switch to the caller ---
    out.ui_print("syncing");
    unsafe { libc::sync() };
    out.set_progress(1.0);
    // Scheme phase 6: drop the /tmp backup on success (kept on failure
    // for forensics).
    let _ = std::fs::remove_dir_all(&a.tmpdir);
    out.ui_print(&format!("DONE slot _{target}"));
    out.ui_print(&format!(
        "install complete: reboot recovery to refresh slot display (current _{booted}, next _{target})"
    ));
    out.line(&format!("hs_done {target}"));
    crate::EXIT_OK
}

/// Rollback ledger entry.
#[derive(Debug, Clone)]
pub enum Backed {
    /// Live resident dumped to a file: write it back.
    Dump { node: String, file: String },
    /// Created fresh by the rebuild: remove the LP entry.
    Fresh { full: String },
}

fn rollback(out: &mut Out, siw: &str, backed: &[Backed]) {
    out.ui_print("rollback: restoring critical partitions");
    for b in backed {
        match b {
            Backed::Dump { node, file } => match backup::restore(file, node) {
                Ok(()) => out.ui_print(&format!("rollback ok: {node}")),
                Err(e) => out.ui_print(&format!("rollback FAILED {node}: {e}")),
            },
            Backed::Fresh { full } => match rebuild::remove_meta(siw, full) {
                Ok(()) => out.ui_print(&format!("rollback removed fresh {full}")),
                Err(e) => {
                    out.ui_print(&format!("rollback FAILED {full}: {e}"))
                }
            },
        }
    }
}
