//! dm — device-mapper truth service.
//!
//! The staging layer (ko-fetch/fw-fetch in pixelrunatboot.sh) creates and
//! removes dm nodes out-of-band from TWRP: TWRP resolves logical partitions
//! once (fs_mgr_update_logical_partition) and re-resolves live at every
//! Mount() via the slotselect branch, but it never learns *who* mapped what
//! or when a node it knew vanished. Field case (gs101 raven): fw-fetch
//! mapped vendor_a while the mapper symlink was not linked yet, then
//! unmapped TWRP's own live mapping by name — by-name/vendor dangled at a
//! dead dm-0, /vendor_dlkm never mounted ("unable to update logical
//! partition"), and Unmap_Super_Devices failed the whole format on the
//! ghost name.
//!
//! This module is the single runtime source of truth for mapper state:
//! - `/tmp/fox_dm_state` ("<name> <dm-target>" per line, "# v1 epoch=..."
//!   header): volatile tmpfs snapshot, same lifetime as the dm namespace
//!   itself. Whoever needs live dm state (TWRP flows, adb debugging) reads
//!   this file instead of guessing. The shell stages keep refreshing it as
//!   a fallback (trap on EXIT); the authoritative writer is this module.
//! - `dm-watch` daemon: polls /dev/block/mapper and rewrites the snapshot
//!   on change. Snapshot-ONLY by design: it never maps or unmaps, so it
//!   can never resurrect a node Unmap_Super_Devices just destroyed at
//!   format time (that would re-break format).
//! - keep-list (`KEEP_PARTS`): LP bases first-stage/TWRP never maps itself
//!   (gs101 vendor_dlkm) whose node boot leaves behind for TWRP, so
//!   /vendor_dlkm can mount at all. Leaving a linear node is safe: same
//!   extents as LP metadata, format-time Unmap destroys it via the
//!   existing metadata entry. Mapping itself runs through the `dm-map`
//!   shell stage (Rust never forks siw directly — stage.rs is the choke
//!   point); the shell ownership guard (_siw_unmap_ours) still protects
//!   foreign nodes on the one-shot paths.
//!
//! TWRP needs no new logic: it keeps resolving nodes live at Mount() and
//! destroying by name at Unmap(); stable nodes + truth file are enough.

use crate::ko_picker::log_msg;
use crate::stage::run_stage;
use std::time::Duration;

const TAG: &str = "dm";
/// Volatile snapshot: same lifetime as the dm namespace (tmpfs, gone on
/// reboot). TWRP/adb read this instead of guessing who mapped what.
pub const STATE_PATH: &str = "/tmp/fox_dm_state";
/// The ONLY source of usable vendor nodes: TWRP-static by-name symlinks
/// may dangle at removed dm devices and are never consulted here.
pub const MAPPER_DIR: &str = "/dev/block/mapper";
/// LP bases whose node boot leaves mapped for TWRP (first-stage never maps
/// them itself; without a kept node they can never mount). Format-time
/// Unmap destroys them cleanly via the LP metadata entries. Mirrored by
/// _KEEP_MAPPED in pixelrunatboot.sh (shell fw-fetch keeps vendor_a,
/// Rust boot keeps part_touch/vendor_dlkm_a via ensure_keep_mapped).
pub const KEEP_PARTS: &[&str] = &["vendor_dlkm", "vendor"];
/// Snapshot format version (parsers must ignore unknown `#` lines).
pub const STATE_VERSION: &str = "v1";

fn info(msg: &str) {
    log_msg(TAG, "INFO", msg);
    println!("dm: INFO: {msg}");
}

fn warn(msg: &str) {
    log_msg(TAG, "WARN", msg);
    println!("dm: WARN: {msg}");
}

/// Basename of the mapper symlink target (e.g. "dm-2" for
/// vendor_dlkm_a -> /dev/block/dm-2); None when absent/unreadable.
/// Pure over the readlink result (testable); live read is [`mapper_target`].
fn target_basename(link: Option<String>) -> Option<String> {
    link.map(|t| {
        t.rsplit('/')
            .next()
            .unwrap_or_default()
            .trim()
            .to_string()
    })
    .filter(|b| !b.is_empty())
}

/// Live dm target of a mapper node, if the symlink resolves to a name.
/// Single-node lookup for future query use; the bulk path goes through
/// [`list_mapper`]. Kept (not removed) as service surface.
#[allow(dead_code)]
pub fn mapper_target(name: &str) -> Option<String> {
    let link = std::fs::read_link(format!("{MAPPER_DIR}/{name}"))
        .map(|p| p.to_string_lossy().into_owned())
        .ok();
    target_basename(link)
}

/// True when `path` is a live block device (metadata says block).
/// Dangling symlinks and regular files are not live.
pub fn is_live_block(path: &str) -> bool {
    std::fs::metadata(path)
        .map(|m| std::os::unix::fs::FileTypeExt::is_block_device(&m.file_type()))
        .unwrap_or(false)
}

/// Live mapper truth: (name, dm-target) for every entry that resolves to
/// a target name. The `by-uuid` subdir and unreadable entries are skipped.
/// Liveness (block device) is NOT required here: a dangling alias is still
/// truth worth reporting (it explains Unmap failures); consumers decide.
pub fn list_mapper() -> Vec<(String, String)> {
    list_mapper_under(std::path::Path::new(MAPPER_DIR))
}

/// Testable core of [`list_mapper`] over an arbitrary dir.
fn list_mapper_under(dir: &std::path::Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.is_empty() || name == "by-uuid" {
            continue;
        }
        // Symlink target basename; fall back to canonicalized basename so
        // absolute symlinks (dm-0) and relative ones (dm-2) both work.
        let target = std::fs::read_link(e.path())
            .map(|p| p.to_string_lossy().into_owned())
            .ok()
            .and_then(|t| target_basename(Some(t)))
            .or_else(|| {
                e.path()
                    .canonicalize()
                    .ok()
                    .and_then(|p| {
                        p.file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                    })
                    .and_then(|t| target_basename(Some(t)))
            });
        if let Some(t) = target {
            out.push((name, t));
        }
    }
    out.sort();
    out
}

/// Render the snapshot body for a mapper listing (header + lines).
/// Pure (testable); the live readdir is [`list_mapper`], the clock read is
/// in [`write_snapshot`].
fn render_snapshot(entries: &[(String, String)], epoch: u64) -> String {
    let mut s = format!("# {STATE_VERSION} epoch={epoch}\n");
    for (name, target) in entries {
        s.push_str(&format!("{name} {target}\n"));
    }
    s
}

/// Atomically rewrite the state file (tmp + rename, so readers never see a
/// torn snapshot). Best-effort: logs and returns the io error.
pub fn write_snapshot() -> Result<(), String> {
    let entries = list_mapper();
    let epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let body = render_snapshot(&entries, epoch);
    let tmp = format!("{STATE_PATH}.tmp.{}", std::process::id());
    std::fs::write(&tmp, body.as_bytes())
        .map_err(|e| format!("dm snapshot write failed: {e}"))?;
    std::fs::rename(&tmp, STATE_PATH)
        .map_err(|e| format!("dm snapshot rename failed: {e}"))?;
    Ok(())
}

/// Oneshot: refresh the snapshot now (idempotent, never fails boot).
/// Called at the end of the boot stage and by the `dm-state` subcommand.
pub fn refresh(reason: &str) -> Result<(), String> {
    match write_snapshot() {
        Ok(()) => {
            info(&format!("state refreshed ({reason})"));
            Ok(())
        }
        Err(e) => {
            warn(&e);
            Err(e)
        }
    }
}

/// Ensure /dev/block/mapper/<part>_<sfx> exists, mapping it via the
/// `dm-map` shell stage when absent, and leave it mapped (keep-list only).
/// No-op when already present (foreign or ours) or when `part` is not
/// keep-listed. Returns true when a usable node exists afterwards.
pub fn ensure_keep_mapped(part: &str, sfx_letter: &str, slotnum: &str) -> bool {
    if !KEEP_PARTS.contains(&part) {
        return false;
    }
    let node = format!("{MAPPER_DIR}/{part}_{sfx_letter}");
    if is_live_block(&node) {
        info(&format!("keep: {node} already present, leaving it"));
        let _ = refresh("keep-present");
        return true;
    }
    info(&format!("keep: {node} absent, mapping for TWRP"));
    match run_stage("dm-map", &[part, sfx_letter, slotnum]) {
        Ok(out) => {
            let ok = is_live_block(&node)
                || out
                    .lines()
                    .any(|l| l.trim() == node && is_live_block(l.trim()));
            if ok {
                info(&format!("keep: left {part}_{sfx_letter} mapped for TWRP"));
            } else {
                warn(&format!("keep: dm-map ok but {node} unusable"));
            }
            let _ = refresh("keep-mapped");
            ok
        }
        Err(e) => {
            warn(&format!("keep: dm-map {part}_{sfx_letter} failed: {e}"));
            let _ = refresh("keep-failed");
            false
        }
    }
}

/// `dm-state` subcommand: refresh + print the snapshot path.
pub fn run_dm_state() -> Result<(), String> {
    refresh("dm-state")?;
    let body = std::fs::read_to_string(STATE_PATH).unwrap_or_default();
    print!("{body}");
    Ok(())
}

/// `dm-watch` daemon: poll the mapper dir, rewrite the snapshot on change.
/// Snapshot-ONLY: never maps, never unmaps (a format-time destroy must
/// stay destroyed). Never exits; an exiting non-oneshot service would
/// respawn-loop.
pub fn run_dm_watch() -> ! {
    info("starting (dm truth watch, snapshot-only, no remap)");
    let mut last: Vec<(String, String)> = Vec::new();
    loop {
        let cur = list_mapper();
        if cur != last {
            match write_snapshot() {
                Ok(()) => info(&format!(
                    "state: {} node(s): {}",
                    cur.len(),
                    cur.iter()
                        .map(|(n, t)| format!("{n}->{t}"))
                        .collect::<Vec<_>>()
                        .join(" ")
                )),
                Err(e) => warn(&e),
            }
            last = cur;
        }
        std::thread::sleep(Duration::from_secs(3));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_basename_shapes() {
        assert_eq!(
            target_basename(Some("/dev/block/dm-2".into())),
            Some("dm-2".into())
        );
        assert_eq!(target_basename(Some("dm-0".into())), Some("dm-0".into()));
        assert_eq!(target_basename(Some("".into())), None);
        assert_eq!(target_basename(None), None);
    }

    #[test]
    fn render_snapshot_format() {
        let entries = vec![
            ("system_a".to_string(), "dm-1".to_string()),
            ("vendor_dlkm_a".to_string(), "dm-0".to_string()),
        ];
        let body = render_snapshot(&entries, 123);
        assert!(body.starts_with("# v1 epoch=123\n"));
        assert!(body.contains("system_a dm-1\n"));
        assert!(body.contains("vendor_dlkm_a dm-0\n"));
        assert_eq!(body.lines().count(), 3);
    }

    #[test]
    fn list_mapper_under_skips_by_uuid_and_sorts() {
        let d = std::env::temp_dir().join(format!("fox_test_dm_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("by-uuid")).unwrap();
        std::os::unix::fs::symlink("dm-2", d.join("vendor_dlkm_a")).unwrap();
        std::os::unix::fs::symlink("/dev/block/dm-1", d.join("system_a")).unwrap();
        std::fs::write(d.join("by-uuid").join("x"), b"y").unwrap();
        let listed = list_mapper_under(&d);
        assert_eq!(
            listed,
            vec![
                ("system_a".to_string(), "dm-1".to_string()),
                ("vendor_dlkm_a".to_string(), "dm-2".to_string()),
            ]
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn keep_list_covers_vendor_dlkm_only() {
        assert!(KEEP_PARTS.contains(&"vendor_dlkm"));
        assert!(KEEP_PARTS.contains(&"vendor"));
        assert!(!KEEP_PARTS.contains(&"system"));
    }
}
