//! install-rom orchestration: checks -> backup -> rebuild -> batches.
//! Batch order: super group (weight 1) -> boot (2) -> modem (3) -> rest (4).
//! Barrier per batch: all threads must validate, else finish the batch
//! and roll back. Slot is NEVER switched here (caller does bootctl).

use crate::{backup, device, live, ota, rebuild, weights, Out};
use payload_hs::{extract, layout, manifest};
use std::collections::{BTreeMap, HashMap};

pub struct Args {
    pub zip: String,
    pub status_fd: Option<i32>,
    pub slot: Option<char>,
    pub sysroot: String,
    pub tmpdir: String,
    pub threads: usize,
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
    out.ui_print(&format!("hs-install: {msg}"));
    code
}

pub fn run(a: Args) -> i32 {
    let mut out = Out::new(a.status_fd);
    out.line("progress 1.0 600");

    // --- 0. identity: device + slots (universal, no hardcodes) ---
    let device = match device::getprop("ro.product.device") {
        Ok(d) if !d.is_empty() => d,
        _ => return fail(&mut out, "no ro.product.device prop", crate::EXIT_FAIL),
    };
    let booted = match a.slot {
        Some(s) => {
            // --slot forces the TARGET (host testing); booted = the other.
            if s == 'a' {
                'b'
            } else {
                'a'
            }
        }
        None => match device::booted_slot() {
            Ok(b) => b,
            Err(e) => return fail(&mut out, &e, crate::EXIT_FAIL),
        },
    };
    let target = match a.slot {
        Some(s) => s,
        None => device::target_slot(booted),
    };
    out.ui_print(&format!("hs-install: device={device} guest slot=_{target}"));

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
        out.ui_print(&format!("hs-install: not full OTA ({r}), stock path"));
        return crate::EXIT_NOT_FULL;
    }

    // --- 3. helpers: siw + lptools_new must exist, else stock path ---
    let siw = format!("{}/siw", a.sysroot);
    let lptools = format!("{}/lptools_new", a.sysroot);
    for h in [&siw, &lptools] {
        if !std::path::Path::new(h).exists() {
            out.ui_print(&format!("hs-install: helper missing: {h}, stock path"));
            return crate::EXIT_NO_HELPERS;
        }
    }

    // --- 4. live super state + fit (phase 2) ---
    let live = match live::query(&siw, booted) {
        Ok(l) => l,
        Err(e) => return fail(&mut out, &e, crate::EXIT_FAIL),
    };
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
    out.ui_print(&format!("hs-install: fit {}", fit.code()));
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
        out.ui_print(&format!("hs-install: reclaim: {}", kill.join(",")));
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
            out.ui_print(&format!("hs-install: backup {}", p.name));
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
    out.ui_print("hs-install: rebuilding target group (single-threaded)");
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
        Ok(gone) => out.ui_print(&format!("hs-install: unmapped {} dm node(s)", gone.len())),
        Err(e) => return fail(&mut out, &e, crate::EXIT_FAIL),
    }
    for full in &to_remove {
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
    let total_out: u64 = parts.iter().map(|p| p.out_size(bs)).sum();
    let mut done_out = 0u64;
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(a.threads)
        .build();
    let pool = match pool {
        Ok(p) => p,
        Err(e) => return fail(&mut out, &format!("threads: {e}"), crate::EXIT_FAIL),
    };
    for (w, batch) in &batches {
        out.ui_print(&format!("hs-install: batch weight {w} ({} parts)", batch.len()));
        let res: Vec<Result<(String, u64, [u8; 32]), String>> = pool.install(|| {
            use rayon::prelude::*;
            batch
                .par_iter()
                .map(|p| write_one(&a.zip, layout.data_start, bs, p, &node_of[&p.name]))
                .collect()
        });
        // barrier: every thread must validate, else finish batch and roll back
        let mut failed: Option<String> = None;
        for (p, r) in batch.iter().zip(res) {
            match r {
                Ok((_, n, sum)) => {
                    if p.new_hash.len() == 32 && sum[..] != p.new_hash[..] {
                        failed = Some(format!(
                            "{}: sha256 mismatch (media or decode error)",
                            p.name
                        ));
                    } else {
                        done_out += n;
                        out.set_progress(done_out as f64 / total_out as f64 * 0.9 + 0.05);
                    }
                }
                Err(e) => failed = Some(e),
            }
            if failed.is_some() {
                break;
            }
        }
        if let Some(e) = failed {
            out.ui_print(&format!("hs-install: batch {w} FAILED: {e}"));
            rollback(&mut out, &siw, &backed);
            return fail(
                &mut out,
                "install failed; critical partitions restored, slot NOT switched",
                crate::EXIT_FAIL,
            );
        }
        // fsync every node of the finished batch before the next one starts
        for p in batch.iter() {
            if let Err(e) = backup::fsync_node(&node_of[&p.name]) {
                rollback(&mut out, &siw, &backed);
                return fail(&mut out, &format!("fsync {}: {e}", p.name), crate::EXIT_FAIL);
            }
        }
        out.ui_print(&format!("hs-install: batch {w} ok"));
    }

    // --- 8. done: sync, leave the slot switch to the caller ---
    out.ui_print("hs-install: syncing");
    let _ = std::process::Command::new("sync").output();
    out.set_progress(1.0);
    out.ui_print(&format!("hs-install: DONE slot _{target}"));
    out.line(&format!("hs_done {target}"));
    0
}

/// Write one partition to its node + hash-as-written. Runs on a worker.
fn write_one(
    zip: &str,
    data_start: u64,
    bs: u64,
    p: &manifest::Part,
    node: &str,
) -> Result<(String, u64, [u8; 32]), String> {
    use sha2::Digest as _;
    let mut h = sha2::Sha256::new();
    let st = extract::extract_partition_into(zip, data_start, bs, p, node, false, Some(&mut h))?;
    Ok((p.name.clone(), st.output_bytes, h.finalize().into()))
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
    out.ui_print("hs-install: rollback: restoring critical partitions");
    for b in backed {
        match b {
            Backed::Dump { node, file } => match backup::restore(file, node) {
                Ok(()) => out.ui_print(&format!("hs-install: rollback ok: {node}")),
                Err(e) => out.ui_print(&format!("hs-install: rollback FAILED {node}: {e}")),
            },
            Backed::Fresh { full } => match rebuild::remove_meta(siw, full) {
                Ok(()) => out.ui_print(&format!("hs-install: rollback removed fresh {full}")),
                Err(e) => {
                    out.ui_print(&format!("hs-install: rollback FAILED {full}: {e}"))
                }
            },
        }
    }
}
