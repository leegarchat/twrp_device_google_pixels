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

/// Remove a logical partition from whichever metadata slot holds it.
/// Tries slot 0 then slot 1; absent everywhere = already gone (ok).
pub fn remove_meta(siw: &str, full: &str) -> Result<(), String> {
    let mut missing = 0;
    for slot in ["0", "1"] {
        match siw_cmd(siw, &["remove", SUPER_BLOCK, full, "-s", slot, "-y"]) {
            Ok(_) => return Ok(()),
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
    unreachable!();
}

/// Create a logical partition in whichever metadata slot already holds
/// its group siblings (tried in order); the group name carries the slot
/// suffix on these devices (google_dynamic_partitions_{a,b}).
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
                "slot_suffixed",
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

/// Unmount + unmap EVERYTHING stacked on super. Raw LP metadata writes
/// are refused (EPERM) while any dm mapping holds the device, so this
/// runs before any siw remove/create — not just the victim list.
/// Only names present in `live` (both slots) are unmapped; anything else
/// (userdata dm etc.) is left alone.
/// Mounts are unmounted deepest-first (nested mountpoints like
/// /vendor/firmware must go before /vendor).
pub fn unmount_unmap_all(
    siw: &str,
    live: &payload_hs::layout::LiveState,
) -> Result<Vec<String>, String> {
    use std::collections::BTreeSet;
    // pass 1: unmount, deepest first
    let mounts = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
    let mut targets: Vec<String> = mounts
        .lines()
        .filter_map(|line| {
            let mut it = line.split_whitespace();
            let src = it.next().unwrap_or("");
            let mnt = it.next().unwrap_or("");
            if (src.starts_with("/dev/block/mapper/")
                || src.starts_with("/dev/dm-"))
                && !mnt.is_empty()
            {
                Some(mnt.to_string())
            } else {
                None
            }
        })
        .collect();
    targets.sort_by_key(|m| std::cmp::Reverse(m.len()));
    targets.dedup();
    for m in &targets {
        let _ = Command::new("umount").arg(m).output();
    }
    // pass 2: unmap every known LP resident + any other stale mapper
    // node (a previous aborted run may leave mappings whose metadata
    // entry is already gone, so LiveState alone is not enough).
    let mut names: BTreeSet<String> = BTreeSet::new();
    for k in live.current.keys().chain(live.target.keys()) {
        names.insert(k.clone());
    }
    if let Ok(rd) = std::fs::read_dir("/dev/block/mapper") {
        for e in rd.flatten() {
            if let Some(n) = e.file_name().to_str() {
                if n != "by-uuid" {
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
    Ok(gone)
}
