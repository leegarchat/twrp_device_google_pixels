//! Factory installer: the same 4-stage conveyor as OTA run.rs, but the
//! payload is a Google factory zip: raw images inside the nested
//! image-*.zip (DEFLATE, streamed) plus FBPK bootloader/radio table-1
//! blobs as fallback for components missing from the inner zip.
//!
//! Differences vs OTA run.rs: sizes come from zip central dirs (no
//! manifest), the super group comes from the device pixel.json
//! (ota_super), expected hashes are source-stream vs node re-read
//! (no manifest hashes exist), and bootstrap support files are read
//! via the same pread paths (FUSE-safe).

use crate::{backup, device, live, rebuild, run, weights, Out};
use crate::{ffbpk, fzip};
use payload_hs::layout;
use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

/// Expanded in one read for FBPK header sniffing (tables live here).
const HEADER_PROBE: usize = 65536;
/// Progress credit quantum (same 8 MiB pieces as the OTA path).
const PIECE: usize = 8 << 20;

#[derive(Clone)]
enum PartSrc {
    /// Nested entry: absolute outer-file offset, compressed + expanded size.
    /// `size` is the EXPANDED size (sparse containers expand to
    /// blk_sz * total_blks); `crc` is the zip CRC32 of the uncompressed
    /// (pre-sparse-decode) bytes for bitrot detection.
    Inner { off: u64, comp: u64, size: u64, stored: bool, crc: u32, sparse: bool },
    /// Pre-extracted FBPK container file + blob range inside it.
    Fbpk { file: String, off: u64, size: u64 },
}

/// Android sparse image header (system/vendor/product images ship as
/// sparse containers on some factories): magic 0xED26FF3A, then the
/// expanded size = blk_sz * total_blks. Returns None for raw images.
fn sparse_expanded(h: &[u8]) -> Option<u64> {
    if h.len() < 28 {
        return None;
    }
    let le = |o: usize| u32::from_le_bytes([h[o], h[o + 1], h[o + 2], h[o + 3]]);
    if le(0) != 0xED26FF3A {
        return None;
    }
    let blk_sz = le(12) as u64;
    let total_blks = le(16) as u64;
    let total_chunks = le(20);
    if blk_sz == 0 || blk_sz % 512 != 0 || total_blks == 0 || total_chunks == 0 {
        return None;
    }
    total_blks.checked_mul(blk_sz)
}

struct FactoryPart {
    name: String,
    weight: u8,
    size: u64,
    src: PartSrc,
}

fn fail(out: &mut Out, msg: &str, code: i32) -> i32 {
    out.ui_print(&format!("{msg}"));
    code
}

/// Open a readable stream over one factory partition's bytes.
/// Inner DEFLATE entries are inflated on the fly (bounded RAM).

pub fn run(a: run::Args) -> i32 {
    let mut out = Out::new(a.status_fd);
    out.line("progress 0.0 600");
    let readahead_prev = std::fs::read_to_string("/sys/block/sda/queue/read_ahead_kb").ok();
    let _ = std::fs::write("/sys/block/sda/queue/read_ahead_kb", "2048\n");

    // --- 0. identity (same as OTA) ---
    let device = match device::getprop("ro.product.device") {
        Ok(d) if !d.is_empty() => d,
        _ => return fail(&mut out, "no ro.product.device prop", crate::EXIT_FAIL),
    };
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
    out.log(&format!("factory device={device} guest slot=_{target}"));

    // --- 1. probe: outer markers (central dir only, no extraction) ---
    let mut outer = match fzip::Zip::open(std::path::Path::new(&a.zip)) {
        Ok(z) => z,
        Err(e) => return fail(&mut out, &format!("zip probe: {e}"), crate::EXIT_FAIL),
    };
    let base_of = |n: &str| n.rsplit('/').next().unwrap_or(n).to_string();
    let inner_e = outer
        .entries
        .iter()
        .find(|e| {
            let b = base_of(&e.name);
            b.starts_with("image-") && b.ends_with(".zip")
        })
        .cloned();
    let bl_e = outer
        .entries
        .iter()
        .find(|e| {
            let b = base_of(&e.name);
            b.starts_with("bootloader-") && b.ends_with(".img")
        })
        .cloned();
    let rd_e = outer
        .entries
        .iter()
        .find(|e| {
            let b = base_of(&e.name);
            b.starts_with("radio-") && b.ends_with(".img")
        })
        .cloned();
    let (inner_e, bl_e, rd_e) = match (inner_e, bl_e, rd_e) {
        (Some(i), Some(b), Some(r)) => (i, b, r),
        _ => {
            return fail(
                &mut out,
                "not a factory zip (need image-*.zip + bootloader/radio)",
                crate::EXIT_FAIL,
            )
        }
    };
    let has_flash_all = outer.entries.iter().any(|e| {
        let b = base_of(&e.name);
        b == "flash-all.sh" || b == "flash-all.bat"
    });
    if !has_flash_all {
        out.log("factory: no flash-all script, continuing on markers");
    }
    if inner_e.method != fzip::METHOD_STORED {
        return fail(&mut out, "inner image zip is not STORED", crate::EXIT_FAIL);
    }
    let inner_off = match outer.data_off(&inner_e) {
        Ok(o) => o,
        Err(e) => return fail(&mut out, &format!("inner offset: {e}"), crate::EXIT_FAIL),
    };
    // Device from the inner name: image-<device>-*.zip must equal the
    // live device (hard gate: family siblings share require lines, but
    // the file set is per-device). Beta/dp Mayflower builds carry a
    // channel suffix (image-<device>_beta-*, image-<device>_dp-*):
    // strip it, codenames never contain '_' so this can't collide.
    let inner_dev = inner_e
        .name
        .rsplit('/')
        .next()
        .unwrap_or("")
        .strip_prefix("image-")
        .and_then(|s| s.split('-').next())
        .and_then(|d| d.split('_').next())
        .map(|x| x.to_string())
        .unwrap_or_default();
    if inner_dev != device {
        return fail(
            &mut out,
            &format!("factory is for '{inner_dev}' but device is '{device}': refusing"),
            crate::EXIT_DEVICE_MISMATCH,
        );
    }
    // android-info.txt (tiny).
    let mut nested = match fzip::Zip::open_nested(std::path::Path::new(&a.zip), inner_off, inner_e.size) {
        Ok(z) => z,
        Err(e) => return fail(&mut out, &format!("inner zip: {e}"), crate::EXIT_FAIL),
    };
    let ai_e = match nested.find("android-info.txt").cloned() {
        Some(e) => e,
        None => return fail(&mut out, "no android-info.txt in factory", crate::EXIT_FAIL),
    };
    let mut ai_buf = Vec::new();
    match nested.extract_to(&ai_e, &mut ai_buf) {
        Ok(c) if c == ai_e.crc => {}
        Ok(_) => return fail(&mut out, "android-info.txt CRC mismatch", crate::EXIT_FAIL),
        Err(e) => return fail(&mut out, &format!("android-info.txt: {e}"), crate::EXIT_FAIL),
    }
    let ai = String::from_utf8_lossy(&ai_buf);
    // Single-line summary only: the pipe protocol splits on newlines,
    // and every extra line would pop up on the GUI as a bare first
    // token (user sees stages + errors only; details stay in the log
    // as one record per line).
    let mut board_line = String::new();
    for line in ai.lines() {
        if let Some(v) = line.strip_prefix("require board=") {
            board_line = v.to_string();
        }
    }
    out.log(&format!("factory android-info: board=[{board_line}]"));
    // require board= must list this device.
    let mut board_ok = false;
    for line in ai.lines() {
        if let Some(v) = line.strip_prefix("require board=") {
            for cand in v.split(['|', ' ', ',']) {
                if !cand.is_empty() && cand.trim() == device {
                    board_ok = true;
                }
            }
        }
    }
    if !board_ok {
        return fail(
            &mut out,
            &format!("factory require board= does not list '{device}': refusing"),
            crate::EXIT_DEVICE_MISMATCH,
        );
    }
    for line in ai.lines() {
        if line.starts_with("require version-") {
            out.log(&format!("factory {line}"));
        }
    }

    // --- 2. stage table (json, required for factory) ---
    let stage = match run::stage_table(&device) {
        Some(t) => t,
        None => {
            return fail(
                &mut out,
                "no ota_* stage table for this device (pixel.json not merged?)",
                crate::EXIT_FAIL,
            )
        }
    };
    out.log(&format!("stage table: {} entries from pixel.json", stage.len()));

    // --- 3. helpers (same gate as OTA) ---
    let siw = format!("{}/siw", a.sysroot);
    if !std::path::Path::new(&siw).exists() {
        out.ui_print(&format!("helper missing: {siw}, stock path"));
        return crate::EXIT_NO_HELPERS;
    }

    // --- 4. partition list: inner entries classified by the table ---
    // Tuple: (weight, EXPANDED size, entry, sparse). Expanded size feeds
    // create_meta and the write loop; sparse entries decode on the fly.
    let mut wanted: HashMap<String, (u8, u64, fzip::Entry, bool)> = HashMap::new();
    let mut extras = 0usize;
    for e in nested.entries.clone() {
        if e.name.ends_with('/') {
            continue;
        }
        let base = e.name.rsplit('/').next().unwrap_or(&e.name);
        let stem = base.strip_suffix(".img").unwrap_or(base);
        if stem == "android-info.txt"
            || stem == "fastboot-info.txt"
            || !base.ends_with(".img")
            || e.name.contains('/')
        {
            extras += 1;
            continue;
        }
        if let Some(w) = stage.get(stem) {
            // Sparse sniff: some factories ship system/vendor/product as
            // Android sparse containers; raw images pass through. The
            // partition is created with the EXPANDED size and the stream
            // is decoded on the fly (see open_source).
            let (size, sparse) = match nested.read_prefix(&e, 28) {
                Ok(h) => match sparse_expanded(&h) {
                    Some(x) => (x, true),
                    None => (e.size, false),
                },
                Err(_) => (e.size, false),
            };
            if sparse {
                out.log(&format!(
                    "factory sparse: {} (packed {} -> {} out)",
                    e.name, e.size, size
                ));
            }
            wanted.insert(stem.to_string(), (*w, size, e, sparse));
        } else {
            out.log(&format!("factory extra (not in ota_*): {}", e.name));
            extras += 1;
        }
    }
    out.log(&format!("factory: {} staged partitions, {} extras skipped", wanted.len(), extras));

    // FBPK fallback for json-required parts missing from inner
    // (modem.img / bootloader .imgs on old factories).
    let mut fbpk_blobs: HashMap<String, (String, u64, u64)> = HashMap::new();
    {
        let have: std::collections::HashSet<&str> = wanted.keys().map(|s| s.as_str()).collect();
        let need_fbpk = stage.keys().any(|n| !have.contains(n.as_str()));
        if need_fbpk {
            // tmpdir must exist before the first File::create (the main
            // backup create_dir_all runs later and only covers the
            // no-fallback path).
            if let Err(e) = std::fs::create_dir_all(&a.tmpdir) {
                return fail(&mut out, &format!("mkdir {}: {e}", a.tmpdir), crate::EXIT_FAIL);
            }
            for (fw, kind) in [(&bl_e, "bootloader"), (&rd_e, "radio")] {
                let dest = format!("{}/fbpk_{kind}.img", a.tmpdir);
                {
                    let mut df = match File::create(&dest) {
                        Ok(f) => f,
                        Err(e) => return fail(&mut out, &format!("tmp fbpk: {e}"), crate::EXIT_FAIL),
                    };
                    let fe = fw.clone();
                    if let Err(e) = outer.extract_to(&fe, &mut df) {
                        return fail(&mut out, &format!("fbpk extract {kind}: {e}"), crate::EXIT_FAIL);
                    }
                }
                let mut rf = match File::open(&dest) {
                    Ok(f) => f,
                    Err(e) => return fail(&mut out, &format!("fbpk open: {e}"), crate::EXIT_FAIL),
                };
                let mut head = vec![0u8; HEADER_PROBE];
                use std::io::{Read, Seek, SeekFrom};
                let head_ok = rf.seek(SeekFrom::Start(0)).is_ok() && rf.read_exact(&mut head).is_ok();
                if !head_ok {
                    let _ = std::fs::remove_file(&dest);
                    return fail(&mut out, "fbpk header unreadable", crate::EXIT_FAIL);
                }
                let flen = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
                match ffbpk::parse(&head, flen) {
                    Ok(inv) => {
                        out.log(&format!(
                            "fbpk {kind} v{} board={} entries={}",
                            inv.version,
                            inv.board,
                            inv.table1.iter().map(|b| b.name.clone()).collect::<Vec<_>>().join(",")
                        ));
                        for b in &inv.table1 {
                            if b.size == 0 || b.off + b.size > flen {
                                continue;
                            }
                            fbpk_blobs.insert(b.name.clone(), (dest.clone(), b.off, b.size));
                        }
                    }
                    Err(e) => {
                        let _ = std::fs::remove_file(&dest);
                        return fail(&mut out, &format!("fbpk parse {kind}: {e}"), crate::EXIT_FAIL);
                    }
                }
            }
        }
    }
    let mut parts: Vec<FactoryPart> = Vec::new();
    for (stem, (w, size, e, sparse)) in &wanted {
        let off = match nested.data_off(e) {
            Ok(o) => inner_off + o,
            Err(e) => return fail(&mut out, &format!("inner offset: {e}"), crate::EXIT_FAIL),
        };
        parts.push(FactoryPart {
            name: stem.clone(),
            weight: *w,
            size: *size,
            src: PartSrc::Inner {
                off,
                comp: e.comp_size,
                size: *size,
                stored: e.method == fzip::METHOD_STORED,
                crc: e.crc,
                sparse: *sparse,
            },
        });
    }
    // json-required but absent from inner -> FBPK fallback.
    {
        let have: std::collections::HashSet<String> =
            parts.iter().map(|p| p.name.clone()).collect();
        let mut still_missing = Vec::new();
        let mut extra = Vec::new();
        for (n, w) in &stage {
            if have.contains(n) {
                continue;
            }
            if let Some((file, off, size)) = fbpk_blobs.get(n.as_str()) {
                extra.push(FactoryPart { name: n.clone(), weight: *w, size: *size, src: PartSrc::Fbpk { file: file.clone(), off: *off, size: *size } });
            } else {
                still_missing.push(n.clone());
            }
        }
        if !still_missing.is_empty() {
            return fail(
                &mut out,
                &format!("factory lacks required partitions: {} (nothing written)", still_missing.join(",")),
                crate::EXIT_FAIL,
            );
        }
        parts.extend(extra);
    }
    out.log(&format!("factory parts: {}", parts.len()));

    // --- 5. live super state + fit (same machinery as OTA) ---
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
    let super_names: Vec<String> =
        parts.iter().filter(|p| p.weight == weights::W_SUPER).map(|p| p.name.clone()).collect();
    let req: Vec<(String, u64)> = parts
        .iter()
        .filter(|p| p.weight == weights::W_SUPER)
        .map(|p| (p.name.clone(), p.size))
        .collect();
    let plans = vec![layout::plan_group(&req, &super_names, live.super_size, &live)];
    let fit = layout::strictest(&plans);
    out.log(&format!("fit {}", fit.code()));
    let kill: Vec<String> = match &fit {
        layout::FitPlan::AsIs => Vec::new(),
        layout::FitPlan::Kill(k) => k.clone(),
        layout::FitPlan::WipeAll => {
            let mut all: Vec<String> = live.current.keys().chain(live.target.keys()).cloned().collect();
            all.sort();
            all.dedup();
            all
        }
        layout::FitPlan::Impossible => {
            return fail(&mut out, "payload does not fit even a bare super: abort, nothing written", crate::EXIT_IMPOSSIBLE);
        }
    };
    if !kill.is_empty() {
        out.log(&format!("reclaim: {}", kill.join(",")));
    }

    // --- 6. scheme phase 0: every BLOCK target must exist as a block node.
    // Probe slotted node first, then the unslotted one (bootloader
    // components are unslotted on some generations).
    use std::os::unix::fs::FileTypeExt;
    let mut node_of: HashMap<String, String> = HashMap::new();
    for p in &parts {
        if super_names.iter().any(|x| x == &p.name) {
            continue;
        }
        let cand_s = format!("/dev/block/by-name/{}_{target}", p.name);
        let cand_u = format!("/dev/block/by-name/{}", p.name);
        let mut picked: Option<String> = None;
        for cand in [&cand_s, &cand_u] {
            match std::fs::metadata(cand) {
                Ok(md) if md.file_type().is_block_device() => {
                    picked = Some(cand.clone());
                    break;
                }
                _ => {}
            }
        }
        match picked {
            Some(n) => {
                node_of.insert(p.name.clone(), n);
            }
            None => {
                return fail(
                    &mut out,
                    &format!("BLOCK target missing for {} (tried {cand_s}, {cand_u}), nothing written", p.name),
                    crate::EXIT_FAIL,
                )
            }
        }
    }
    {
        let total: u64 = parts.iter().map(|p| p.size).sum();
        out.ui_print(&format!(
            "installing factory to opposite slot _{target} ({} partitions, {} MB out)",
            parts.len(),
            total / 1_000_000
        ));
    }
    let mut batches: BTreeMap<u8, Vec<&FactoryPart>> = BTreeMap::new();
    for p in &parts {
        batches.entry(p.weight).or_default().push(p);
    }
    for v in batches.values_mut() {
        v.sort_by_key(|p| std::cmp::Reverse(p.size));
    }

    let _ = std::fs::create_dir_all(&a.tmpdir);
    let mut backed: Vec<run::Backed> = Vec::new();
    for p in parts.iter().filter(|p| weights::is_critical(p.weight)) {
        let cand_s = format!("/dev/block/by-name/{}_{target}", p.name);
        let cand_u = format!("/dev/block/by-name/{}", p.name);
        let mut src_node: Option<String> = None;
        for cand in [&cand_s, &cand_u] {
            if let Ok(md) = std::fs::metadata(cand) {
                if md.file_type().is_block_device() {
                    src_node = Some(cand.clone());
                    break;
                }
            }
        }
        if let Some(node) = src_node {
            let file = format!("{}/{}.img", a.tmpdir, p.name);
            out.log(&format!("backup {}", p.name));
            // Full old node, not the payload size: restores must put
            // back the whole image when builds change sizes.
            match backup::node_size(&node) {
                Ok(sz) => {
                    if let Err(e) = backup::dump(&node, &file, sz) {
                        return fail(&mut out, &format!("backup {}: {e}", p.name), crate::EXIT_FAIL);
                    }
                    backed.push(run::Backed::Dump { node, file });
                }
                Err(e) => {
                    return fail(&mut out, &format!("backup {}: {e}", p.name), crate::EXIT_FAIL);
                }
            }
        } else {
            backed.push(run::Backed::Fresh { full: format!("{}_{target}", p.name) });
        }
    }

    // --- 7. rebuild, single-threaded (same order as OTA) ---
    out.log("rebuilding target group (single-threaded)");
    let mut to_remove: Vec<String> = kill.clone();
    for n in &super_names {
        to_remove.push(format!("{n}_{target}"));
    }
    to_remove.sort();
    to_remove.dedup();
    match rebuild::unmount_unmap_all(&siw, &live) {
        Ok(gone) => out.log(&format!("unmapped {} dm node(s)", gone.len())),
        Err(e) => return fail(&mut out, &e, crate::EXIT_FAIL),
    }
    for full in &to_remove {
        out.log(&format!("victim=[{full}]"));
        if let Err(e) = rebuild::remove_meta(&siw, full) {
            return fail(&mut out, &format!("remove {full}: {e}"), crate::EXIT_FAIL);
        }
    }
    if live.target_group.is_empty() {
        return fail(&mut out, "no live target group (cannot create super members)", crate::EXIT_FAIL);
    }
    for n in &super_names {
        let full = format!("{n}_{target}");
        let sz = parts.iter().find(|p| &p.name == n).map(|p| p.size).unwrap_or(0);
        out.log(&format!("create=[{full}] group=[{}]", live.target_group));
        if let Err(e) = rebuild::create_meta(&siw, &full, &live.target_group, sz) {
            return fail(&mut out, &format!("create {full}: {e}"), crate::EXIT_FAIL);
        }
    }
    for p in &parts {
        if super_names.iter().any(|x| x == &p.name) {
            let full = format!("{}_{target}", p.name);
            if let Err(e) = rebuild::map_writable(&siw, &full) {
                return fail(&mut out, &format!("map {full}: {e}"), crate::EXIT_FAIL);
            }
            node_of.insert(p.name.clone(), format!("/dev/block/mapper/{full}"));
        }
    }

    // --- 8. batches 1->2->3->4, barrier + per-partition verify ---
    let batch_totals: HashMap<u8, u64> =
        batches.iter().map(|(w, v)| (*w, v.iter().map(|p| p.size).sum())).collect();
    let pool = rayon::ThreadPoolBuilder::new().num_threads(a.threads).build();
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
        out.ui_print(&format!("Step {w}: installing {label}..."));
        out.log(&format!("stage {w}: {}: {}", label, names.join(", ")));
        let mut node_fds: HashMap<String, std::sync::Mutex<std::fs::File>> = HashMap::new();
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
                        // EBUSY = mounted or held open. (O_EXCL is an
                        // open flag, never an errno: do not compare
                        // against it.)
                        let msg = if e.raw_os_error() == Some(libc::EBUSY) {
                            format!("open {node}: busy (mounted or held open)")
                        } else {
                            format!("open {node}: {e}")
                        };
                        out.log(&format!("Step {w} FAILED: {msg}"));
                        if *w != 1 {
                            run::rollback(&mut out, &siw, &backed);
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
        let task_done = std::sync::atomic::AtomicU64::new(0);
        let next_pct = std::sync::atomic::AtomicU64::new(0);
        let btotal = batch_totals.get(w).copied().unwrap_or(0).max(1);
        let out_mtx = std::sync::Mutex::new(&mut out);
        let final_ms = std::sync::atomic::AtomicU64::new(0);
        let first_err = std::sync::Mutex::new(None::<String>);
        let (lo, hi) = match *w {
            1 => (0.05, 0.65),
            2 => (0.65, 0.75),
            3 => (0.75, 0.85),
            _ => (0.85, 0.95),
        };
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
        let t_dec0 = std::time::Instant::now();
        let zip_path = a.zip.clone();
        let res: Vec<Result<(), String>> = pool.install(|| {
            use rayon::prelude::*;
            batch
                .par_iter()
                .map(|j| {
                    let tag = format!("{} size={}", j.name, j.size);
                    let mu = node_fds.get(&j.name).ok_or_else(|| format!("{tag}: node gone"))?;
                    let mut f = mu.lock().map_err(|_| format!("{tag}: node lock poisoned"))?;
                    use std::io::{Seek, SeekFrom, Write};
                    f.seek(SeekFrom::Start(0)).map_err(|e| format!("{tag}: oseek: {e}"))?;
                    let (mut src, expect_crc) = open_source(&zip_path, &j.src)?;
                    use sha2::Digest;
                    let mut hasher = sha2::Sha256::new();
                    let mut left = j.size;
                    let mut chunk = vec![0u8; PIECE];
                    while left > 0 {
                        let n = std::cmp::min(left, chunk.len() as u64) as usize;
                        let mut got = 0;
                        while got < n {
                            match src.read(&mut chunk[got..n]) {
                                Ok(0) => break,
                                Ok(k) => got += k,
                                Err(e) => return Err(format!("{tag}: read: {e}")),
                            }
                        }
                        if got == 0 {
                            return Err(format!("{tag}: short source (want {n})"));
                        }
                        hasher.update(&chunk[..got]);
                        for piece in chunk[..got].chunks(1 << 20) {
                            f.write_all(piece).map_err(|e| format!("{tag}: owrite: {e}"))?;
                            credit(piece.len() as u64);
                        }
                        left -= got as u64;
                    }
                    // Zip bitrot check: CRC32 over the raw entry bytes
                    // must match the central directory (catches corrupt
                    // downloads that sha256-vs-node cannot see). Drain to
                    // raw EOF first: sparse trailers (CRC chunks) and
                    // decoder buffering sit past the expanded size.
                    if let Some(exp) = expect_crc {
                        let _ = std::io::copy(&mut src, &mut std::io::sink());
                        let got_crc = src.finish_crc();
                        if got_crc != exp {
                            return Err(format!(
                                "{tag}: zip crc mismatch (want {exp:08x}, got {got_crc:08x})"
                            ));
                        }
                    }
                    drop(f);
                    let expect: [u8; 32] = hasher.finalize().into();
                    let t_verify = std::time::Instant::now();
                    if let Ok(mut g) = out_mtx.lock() {
                        g.log(&format!("verifying {}", j.name));
                    }
                    let fail_here = |msg: String| {
                        if let Ok(mut g) = first_err.lock() {
                            if g.is_none() {
                                *g = Some(msg);
                            }
                        }
                    };
                    match payload_hs::extract::sha256_file_n(&node_of[&j.name], j.size) {
                        Ok((sum, all_zero)) => {
                            if all_zero {
                                fail_here(format!("{}: image is all zeros (corrupt write)", j.name));
                                return Ok(());
                            }
                            if sum[..] != expect[..] {
                                fail_here(format!("{}: sha256 mismatch (media or decode error)", j.name));
                                return Ok(());
                            }
                        }
                        Err(e) => {
                            fail_here(format!("hash {}: {e}", j.name));
                            return Ok(());
                        }
                    }
                    if let Err(e) = backup::fsync_node(&node_of[&j.name]) {
                        fail_here(format!("fsync {}: {e}", j.name));
                        return Ok(());
                    }
                    if a.readback {
                        match backup::readback_hash(&node_of[&j.name], j.size) {
                            Ok(sum) => {
                                if sum[..] != expect[..] {
                                    fail_here(format!("{}: readback sha256 mismatch", j.name));
                                }
                            }
                            Err(e) => fail_here(format!("{}: readback failed: {e}", j.name)),
                        }
                        if let Ok(mut g) = out_mtx.lock() {
                            g.log(&format!("readback ok: {}", j.name));
                        }
                    }
                    final_ms.fetch_add(
                        t_verify.elapsed().as_millis() as u64,
                        std::sync::atomic::Ordering::Relaxed,
                    );
                    Ok(())
                })
                .collect()
        });
        let mut failed: Option<String> = None;
        for r in res {
            if let Err(e) = r {
                if failed.is_none() {
                    failed = Some(e);
                }
            }
        }
        out.log(&format!(
            "batch {w}: stream {}ms, finalize {}ms (overlapped) | {}",
            t_dec0.elapsed().as_millis(),
            final_ms.load(std::sync::atomic::Ordering::Relaxed),
            run::mem_snapshot()
        ));
        let failed = match (failed, first_err.lock().map(|mut g| g.take())) {
            (Some(e), _) => Some(e),
            (None, Ok(Some(e))) => Some(e),
            (None, _) => None,
        };
        if let Some(e) = failed {
            out.log(&format!("Step {w} FAILED: {e}"));
            if *w != 1 {
                run::rollback(&mut out, &siw, &backed);
            }
            return fail(
                &mut out,
                "install failed; critical partitions restored, slot NOT switched",
                crate::EXIT_FAIL,
            );
        }
        out.set_progress(hi);
        out.log(&format!("stage {w} done"));
    }

    // --- done: sync, leave the slot switch to the caller ---
    if let Some(prev) = readahead_prev.as_deref() {
        let _ = std::fs::write("/sys/block/sda/queue/read_ahead_kb", prev);
    }
    out.log(&format!("done | {}", run::mem_snapshot()));
    out.log("syncing");
    unsafe { libc::sync() };
    out.set_progress(1.0);
    let _ = std::fs::remove_dir_all(&a.tmpdir);
    out.ui_print(&format!("DONE slot _{target}"));
    out.ui_print("install complete: reboot recovery to refresh slot display");
    out.line(&format!("hs_done {target}"));
    crate::EXIT_OK
}

/// Open a readable stream over one factory partition's bytes.
/// Inner DEFLATE entries are inflated on the fly (bounded RAM);
/// FBPK blobs are plain pread ranges.
/// Entry stream with zip-CRC verification and sparse decoding.
/// The reader yields EXPANDED bytes (sparse containers decoded); the
/// CRC32 is computed over the raw post-inflate bytes and checked with
/// finish_crc() after the stream is fully consumed.
enum SrcReader {
    Plain(TeeCrc<Box<dyn Read>>),
    Sparse(SparseDecoder<TeeCrc<Box<dyn Read>>>),
}

impl Read for SrcReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        match self {
            SrcReader::Plain(t) => t.read(out),
            SrcReader::Sparse(d) => d.read(out),
        }
    }
}

impl SrcReader {
    fn finish_crc(self) -> u32 {
        match self {
            SrcReader::Plain(t) => t.crc_value(),
            SrcReader::Sparse(d) => d.into_inner().crc_value(),
        }
    }
}

/// Pass-through reader hashing everything with CRC32 (IEEE, zip order).
struct TeeCrc<R: Read> {
    inner: R,
    crc: fzip::Crc,
}

impl<R: Read> TeeCrc<R> {
    fn crc_value(&self) -> u32 {
        // Crc has no accessor; finalize on a copy (Copy-ish: re-derive
        // from the running state is impossible, so store the digest by
        // re-hashing nothing: Crc::finalize(&self.crc)).
        self.crc.finalize()
    }
}

impl<R: Read> Read for TeeCrc<R> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(out)?;
        self.crc.update(&out[..n]);
        Ok(n)
    }
}

/// Streaming Android sparse decoder (magic 0xED26FF3A). Chunk types:
/// RAW 0xCAC1 (data follows), FILL 0xCAC2 (4-byte pattern), DONTCARE
/// 0xCAC3 (zeros), CRC 0xCAC4 (skipped). Bounded RAM: RAW data streams
/// straight into the caller's buffer, fills repeat a 4-byte pattern.
struct SparseDecoder<R: Read> {
    inner: R,
    blk_sz: u64,
    chunk_hdr_sz: u64,
    chunks_left: u32,
    out_left: u64,
    fill: [u8; 4],
    fill_pos: usize,
    raw_chunk: bool,
    header_done: bool,
}

impl<R: Read> SparseDecoder<R> {
    fn into_inner(self) -> R {
        self.inner
    }

    fn read_u16(&mut self, b: &mut [u8; 2]) -> std::io::Result<()> {
        self.inner.read_exact(&mut b[..])
    }

    fn read_u32(&mut self) -> std::io::Result<u32> {
        let mut b = [0u8; 4];
        self.inner.read_exact(&mut b)?;
        Ok(u32::from_le_bytes(b))
    }

    fn next_chunk(&mut self) -> std::io::Result<bool> {
        if self.chunks_left == 0 {
            return Ok(false);
        }
        let mut t = [0u8; 2];
        self.read_u16(&mut t)?;
        let ctype = u16::from_le_bytes(t);
        let mut r = [0u8; 2];
        self.read_u16(&mut r)?;
        let chunk_sz = self.read_u32()? as u64;
        let total_sz = self.read_u32()? as u64;
        // Honor extended chunk headers (spec: chunk_hdr_sz >= 12).
        let extra = self.chunk_hdr_sz.saturating_sub(12);
        if extra > 0 {
            let mut left = extra;
            let mut tmp = [0u8; 32];
            while left > 0 {
                let n = std::cmp::min(left, tmp.len() as u64) as usize;
                self.inner.read_exact(&mut tmp[..n])?;
                left -= n as u64;
            }
        }
        self.chunks_left -= 1;
        let chdr = self.chunk_hdr_sz;
        match ctype {
            0xCAC1 => {
                let data = total_sz.checked_sub(chdr).ok_or_else(|| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, "sparse raw total_sz < hdr")
                })?;
                if data != chunk_sz * self.blk_sz {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "sparse raw size mismatch",
                    ));
                }
                self.out_left = data;
                self.fill_pos = 0;
                self.raw_chunk = true;
                Ok(true)
            }
            0xCAC2 => {
                if total_sz != chdr + 4 {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "sparse fill bad total_sz",
                    ));
                }
                self.inner.read_exact(&mut self.fill)?;
                self.out_left = chunk_sz * self.blk_sz;
                self.fill_pos = 0;
                self.raw_chunk = false;
                Ok(true)
            }
            0xCAC3 => {
                if total_sz != chdr {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "sparse dontcare bad total_sz",
                    ));
                }
                self.out_left = chunk_sz * self.blk_sz;
                self.fill = [0, 0, 0, 0];
                self.fill_pos = 0;
                self.raw_chunk = false;
                Ok(true)
            }
            0xCAC4 => {
                // CRC32 chunk: header + 4 digest bytes, no output.
                let skip = total_sz.checked_sub(chdr).ok_or_else(|| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, "sparse crc total_sz < hdr")
                })?;
                let mut left = skip;
                let mut tmp = [0u8; 64];
                while left > 0 {
                    let n = std::cmp::min(left, tmp.len() as u64) as usize;
                    self.inner.read_exact(&mut tmp[..n])?;
                    left -= n as u64;
                }
                self.next_chunk()
            }
            _ => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("sparse unknown chunk {ctype:#X}"),
            )),
        }
    }
}

impl<R: Read> Read for SparseDecoder<R> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if !self.header_done {
            let mut h = [0u8; 28];
            self.inner.read_exact(&mut h)?;
            if u32::from_le_bytes([h[0], h[1], h[2], h[3]]) != 0xED26FF3A {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "sparse magic missing in stream",
                ));
            }
            self.blk_sz = u32::from_le_bytes([h[12], h[13], h[14], h[15]]) as u64;
            let total_chunks =
                u32::from_le_bytes([h[20], h[21], h[22], h[23]]);
            let file_hdr_sz = u16::from_le_bytes([h[8], h[9]]) as u64;
            let chunk_hdr_sz = u16::from_le_bytes([h[10], h[11]]) as u64;
            if self.blk_sz == 0 || total_chunks == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "sparse bad header",
                ));
            }
            if file_hdr_sz < 28 || file_hdr_sz > 256 || chunk_hdr_sz < 12 || chunk_hdr_sz > 64 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "sparse bad header sizes",
                ));
            }
            // Skip extended file-header bytes, if any.
            let mut left = file_hdr_sz - 28;
            let mut tmp = [0u8; 64];
            while left > 0 {
                let n = std::cmp::min(left, tmp.len() as u64) as usize;
                self.inner.read_exact(&mut tmp[..n])?;
                left -= n as u64;
            }
            self.chunk_hdr_sz = chunk_hdr_sz;
            self.chunks_left = total_chunks;
            self.header_done = true;
        }
        if out.is_empty() {
            return Ok(0);
        }
        while self.out_left == 0 {
            if !self.next_chunk()? {
                return Ok(0);
            }
        }
        // RAW chunks stream straight through; FILL/DONTCARE repeat fill[].
        if self.raw_chunk {
            let n = std::cmp::min(self.out_left, out.len() as u64) as usize;
            let k = self.inner.read(&mut out[..n])?;
            self.out_left -= k as u64;
            return Ok(k);
        }
        let mut n = 0;
        while n < out.len() && self.out_left > 0 {
            out[n] = self.fill[self.fill_pos];
            self.fill_pos = (self.fill_pos + 1) % 4;
            n += 1;
            self.out_left -= 1;
        }
        Ok(n)
    }
}

fn open_source(zip_path: &str, src: &PartSrc) -> Result<(SrcReader, Option<u32>), String> {
    match src {
        PartSrc::Inner { off, comp, size, stored, crc, sparse } => {
            let mut f = File::open(zip_path).map_err(|e| format!("open zip: {e}"))?;
            // Single seek to the entry data, then a bounded Take: no
            // lseek per chunk. Raw range is packed bytes (comp for
            // DEFLATE; for STORED packed == file size, except sparse
            // where size holds the EXPANDED size).
            f.seek(SeekFrom::Start(*off))
                .map_err(|e| format!("seek zip: {e}"))?;
            let raw_left = if *stored && !*sparse { *size } else { *comp };
            let raw: Box<dyn Read> = if *stored {
                Box::new(f.take(raw_left))
            } else {
                Box::new(flate2::bufread::DeflateDecoder::new(
                    std::io::BufReader::with_capacity(1 << 16, f.take(raw_left)),
                ))
            };
            let tee = TeeCrc { inner: raw, crc: fzip::Crc::new() };
            if *sparse {
                Ok((SrcReader::Sparse(SparseDecoder {
                    inner: tee,
                    blk_sz: 0,
                    chunk_hdr_sz: 12,
                    chunks_left: 0,
                    out_left: 0,
                    fill: [0, 0, 0, 0],
                    fill_pos: 0,
                    raw_chunk: false,
                    header_done: false,
                }), Some(*crc)))
            } else {
                Ok((SrcReader::Plain(tee), Some(*crc)))
            }
        }
        PartSrc::Fbpk { file, off, size } => {
            let mut f = File::open(file).map_err(|e| format!("open fbpk: {e}"))?;
            f.seek(SeekFrom::Start(*off))
                .map_err(|e| format!("seek fbpk: {e}"))?;
            let raw: Box<dyn Read> = Box::new(f.take(*size));
            let tee = TeeCrc { inner: raw, crc: fzip::Crc::new() };
            // FBPK blobs have no zip CRC: no expected value.
            Ok((SrcReader::Plain(tee), None))
        }
    }
}

#[cfg(test)]
mod sparse_tests {
    use super::*;
    use std::io::Read;

    fn hdr(blk_sz: u32, blks: u32, chunks: u32) -> Vec<u8> {
        let mut h = vec![0u8; 28];
        h[0..4].copy_from_slice(&0xED26FF3Au32.to_le_bytes());
        h[4..6].copy_from_slice(&1u16.to_le_bytes());
        h[8..10].copy_from_slice(&28u16.to_le_bytes());
        h[10..12].copy_from_slice(&12u16.to_le_bytes());
        h[12..16].copy_from_slice(&blk_sz.to_le_bytes());
        h[16..20].copy_from_slice(&blks.to_le_bytes());
        h[20..24].copy_from_slice(&chunks.to_le_bytes());
        h
    }

    fn chunk(ctype: u16, blk: u32, extra: &[u8]) -> Vec<u8> {
        let mut c = vec![0u8; 12];
        c[0..2].copy_from_slice(&ctype.to_le_bytes());
        c[4..8].copy_from_slice(&blk.to_le_bytes());
        c[8..12].copy_from_slice(&(12 + extra.len() as u32).to_le_bytes());
        c.extend_from_slice(extra);
        c
    }

    #[test]
    fn decode_raw_fill_dontcare() {
        // 4 blocks x 8 bytes: raw(0xAB), fill(01020304), dontcare, raw(0xCD).
        let mut img = hdr(8, 4, 4);
        img.extend(chunk(0xCAC1, 1, &[0xAB; 8]));
        img.extend(chunk(0xCAC2, 1, &[0x01, 0x02, 0x03, 0x04]));
        img.extend(chunk(0xCAC3, 1, &[]));
        img.extend(chunk(0xCAC1, 1, &[0xCD; 8]));
        let mut dec = SparseDecoder {
            inner: &img[..],
            blk_sz: 0,
            chunk_hdr_sz: 12,
            chunks_left: 0,
            out_left: 0,
            fill: [0; 4],
            fill_pos: 0,
            raw_chunk: false,
            header_done: false,
        };
        let mut out = Vec::new();
        dec.read_to_end(&mut out).unwrap();
        assert_eq!(out.len(), 32);
        assert!(out[0..8].iter().all(|&b| b == 0xAB));
        assert_eq!(&out[8..16], &[0x01, 0x02, 0x03, 0x04, 0x01, 0x02, 0x03, 0x04]);
        assert!(out[16..24].iter().all(|&b| b == 0));
        assert!(out[24..32].iter().all(|&b| b == 0xCD));
    }

    #[test]
    fn sniff_rejects_raw() {
        assert!(sparse_expanded(&[0u8; 28]).is_none());
        let mut not_sparse = vec![0u8; 28];
        not_sparse[0] = 0x58;
        assert!(sparse_expanded(&not_sparse).is_none());
    }
}
