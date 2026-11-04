//! LP metadata surgery via the siw binary. Single-threaded by design:
//! unmap victims -> remove (both metadata slots, mirrored) ->
//! create with manifest sizes -> map writable.
//!
//! siw CLI shapes (see super-image-worker-cli):
//!   siw remove <image> <name> -s <0|1> -y
//!   siw create <image> -n <name> --group <G> --size <bytes>
//!       --attrs slot_suffixed -s <0|1>
//!   siw unmap <name>
//!   siw map --partition <name> --force-writable <image>

use std::process::Command;

pub const SUPER_BLOCK: &str = "/dev/block/by-name/super";

fn siw_cmd(siw: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(siw)
        .args(args)
        .output()
        .map_err(|e| format!("siw {args:?}: spawn: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "siw {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

pub fn unmap(siw: &str, full: &str) {
    // Best effort: absent mappings are fine.
    let _ = siw_cmd(siw, &["unmap", full]);
}

pub fn map_writable(siw: &str, full: &str) -> Result<(), String> {
    match siw_cmd(
        siw,
        &["map", "--partition", full, "--force-writable", SUPER_BLOCK],
    ) {
        Ok(_) => Ok(()),
        Err(e) if e.contains("already mapped") => {
            // Stale node from an aborted run: drop it, map again.
            siw_cmd(siw, &["unmap", full])?;
            siw_cmd(
                siw,
                &["map", "--partition", full, "--force-writable", SUPER_BLOCK],
            )?;
            Ok(())
        }
        Err(e) => Err(e),
    }
}

/// Remove a logical partition from EVERY metadata slot holding it.
/// Tries slot 0 then slot 1; absent everywhere = already gone (ok).
/// Both copies are cleaned so mirrored tables never diverge.
pub fn remove_meta(siw: &str, full: &str) -> Result<(), String> {
    let mut missing = 0;
    for slot in ["0", "1"] {
        match siw_cmd(siw, &["remove", SUPER_BLOCK, full, "-s", slot, "-y"]) {
            Ok(_) => continue, // cleared here; keep clearing the other slot
            Err(e) => {
                if e.contains("not found") {
                    missing += 1;
                    continue;
                }
                return Err(e);
            }
        }
    }
    if missing == 2 {
        return Ok(());
    }
    Ok(())
}

/// Create a logical partition in whichever metadata slot already holds
/// its group siblings (tried in order); the group name carries the slot
/// suffix on these devices (google_dynamic_partitions_{a,b}).
/// Factory convention: FULL stored names, readonly, NO slot_suffixed
/// flag (a flag would make readers append a second suffix).
pub fn create_meta(
    siw: &str,
    full: &str,
    group: &str,
    size: u64,
) -> Result<(), String> {
    let size_s = size.to_string();
    let mut last_err = String::new();
    for slot in ["0", "1"] {
        match siw_cmd(
            siw,
            &[
                "create",
                SUPER_BLOCK,
                "-n",
                full,
                "--group",
                group,
                "--size",
                &size_s,
                "--attrs",
                "readonly",
                "-s",
                slot,
            ],
        ) {
            Ok(_) => return Ok(()),
            Err(e) => last_err = e,
        }
    }
    Err(last_err)
}

/// Unmount (deepest first) + full unmap of CURRENT + TARGET slot
/// residents and any stale mapper nodes. Both slots must go: the
/// rebuild rewrites target metadata while the old slot's dm nodes
/// would keep the table busy, and the live switch after the flash
/// destroys the old LP entries. Only super names from LiveState are
/// unmapped: userdata dm-crypt, verity nodes and friends are never
/// super residents and must never fail the install.
/// Mounts are unmounted deepest-first (nested mountpoints like
/// /vendor/firmware must go before /vendor); recovery-critical mounts
/// (/data, /sdcard, /persist, /cache, /tmp) and userdata dm are
/// excluded. A post-pass verify fails closed if any super mount
/// survives.
pub fn unmount_unmap_all(
    siw: &str,
    live: &payload_hs::layout::LiveState,
) -> Result<Vec<String>, String> {
    use std::collections::BTreeSet;
    // pass 1: unmount, deepest first.
    // Sources: /dev/block/mapper/* (symlinks), /dev/dm-* and
    // /dev/block/dm-* (direct dm nodes, e.g. dm-4 on /system_root).
    // userdata dm (userdata/userdata_exp*/zoned_device) and the
    // recovery-critical mounts (/data, /sdcard, /persist, /cache,
    // /tmp and children) are never touched.
    let mounts = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
    let mut targets: Vec<String> = mounts
        .lines()
        .filter_map(|line| {
            let mut it = line.split_whitespace();
            let src = it.next().unwrap_or("");
            let mnt = it.next().unwrap_or("");
            if mnt.is_empty() || mnt == "/" {
                return None;
            }
            if !(src.starts_with("/dev/block/mapper/")
                || src.starts_with("/dev/dm-")
                || src.starts_with("/dev/block/dm-"))
            {
                return None;
            }
            // except: userdata dm block (mirrors the mapper-name guard
            // in pass 2 and in TWRP Live_Switch_Slot).
            let base = src.rsplit('/').next().unwrap_or(src);
            if base == "userdata"
                || base == "zoned_device"
                || base.starts_with("userdata_exp")
            {
                return None;
            }
            // recovery-critical mountpoints stay up (/sdcard is /data).
            if mnt == "/tmp"
                || mnt == "/data"
                || mnt == "/sdcard"
                || mnt == "/persist"
                || mnt == "/cache"
                || mnt.starts_with("/data/")
                || mnt.starts_with("/sdcard/")
                || mnt.starts_with("/persist/")
                || mnt.starts_with("/mnt/installer")
                || mnt.starts_with("/mnt/androidwritable")
            {
                return None;
            }
            Some(mnt.to_string())
        })
        .collect();
    targets.sort_by_key(|m| std::cmp::Reverse(m.len()));
    targets.dedup();
    for m in &targets {
        let _ = Command::new("umount").arg(m).output();
    }
    // pass 2: unmap CURRENT + TARGET residents (full unmap) + stale
    // mapper nodes. The old slot must go too: a live switch after the
    // flash destroys its LP entries, and stale dm nodes keep the
    // destroy path EBUSY. Userdata and friends are never super
    // residents (LiveState holds super names only) and are skipped
    // by name below even if a stale node ever shows up.
    // (LiveState keys are already full names with slot suffix.)
    let current: BTreeSet<String> = live.current.keys().cloned().collect();
    let known: BTreeSet<String> = current
        .iter()
        .chain(live.target.keys())
        .cloned()
        .collect();
    let mut names: BTreeSet<String> = BTreeSet::new();
    for k in live.target.keys().chain(live.current.keys()) {
        if k == "userdata"
            || k == "zoned_device"
            || k.starts_with("userdata_exp")
        {
            continue;
        }
        names.insert(k.clone());
    }
    if let Ok(rd) = std::fs::read_dir("/dev/block/mapper") {
        for e in rd.flatten() {
            if let Some(n) = e.file_name().to_str() {
                if n == "by-uuid"
                    || n == "userdata"
                    || n == "zoned_device"
                    || n.starts_with("userdata_exp")
                {
                    continue;
                }
                if known.contains(n) {
                    names.insert(n.to_string());
                }
            }
        }
    }
    let mut gone = Vec::new();
    let mut busy = Vec::new();
    for n in &names {
        let out = Command::new(siw)
            .args(["unmap", n])
            .output()
            .map_err(|e| format!("siw unmap {n}: spawn: {e}"))?;
        if out.status.success() {
            gone.push(n.clone());
            continue;
        }
        // tolerate "not mapped"; fail loudly on anything else
        let err = String::from_utf8_lossy(&out.stderr);
        if err.contains("not mapped")
            || err.contains("No such")
            || !std::path::Path::new(&format!("/dev/block/mapper/{n}")).exists()
        {
            continue;
        }
        busy.push(format!("{n}: {}", err.trim()));
    }
    if !busy.is_empty() {
        return Err(format!("dm busy, refusing to proceed: {}", busy.join("; ")));
    }
    // Separate pre-write check: no super mount may survive. Re-read
    // mounts with the same filter; any leftover is a hard fail before
    // remove/create touch metadata.
    {
        let mounts = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
        let mut left: Vec<String> = Vec::new();
        for line in mounts.lines() {
            let mut it = line.split_whitespace();
            let src = it.next().unwrap_or("");
            let mnt = it.next().unwrap_or("");
            if !(src.starts_with("/dev/block/mapper/")
                || src.starts_with("/dev/dm-")
                || src.starts_with("/dev/block/dm-"))
            {
                continue;
            }
            let base = src.rsplit('/').next().unwrap_or(src);
            if base == "userdata"
                || base == "zoned_device"
                || base.starts_with("userdata_exp")
            {
                continue;
            }
            if mnt == "/tmp"
                || mnt == "/data"
                || mnt == "/sdcard"
                || mnt == "/persist"
                || mnt == "/cache"
                || mnt.starts_with("/data/")
                || mnt.starts_with("/sdcard/")
                || mnt.starts_with("/persist/")
                || mnt.starts_with("/mnt/installer")
                || mnt.starts_with("/mnt/androidwritable")
            {
                continue;
            }
            if !mnt.is_empty() {
                left.push(format!("{src} on {mnt}"));
            }
        }
        if !left.is_empty() {
            return Err(format!("super still mounted, refusing to proceed: {}", left.join("; ")));
        }
    }
    Ok(gone)
}
