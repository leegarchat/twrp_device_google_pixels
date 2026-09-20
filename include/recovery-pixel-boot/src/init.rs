//! init — early-init device identity. Port of runatinit.sh.
//!
//! Runs on `early-init` via exec, BEFORE USB gadget configfs and TWRP
//! data.cpp: family/device props, twrp.flags fix, LGZ zip payload restore,
//! magiskboot extraction. Called as `recovery-pixel-boot init`.
//!
//! Also the AIO second-pass family swap: the C stub swaps
//! recovery.fstab/twrp.flags/recovery.wipe pre-init, but if it missed,
//! the live files are still the AIO placeholders here — and the recovery
//! process (which parses them) has not started yet (`exec` blocks init),
//! so re-doing the swap now is safe and idempotent. (USB gadget binding
//! itself belongs to the otg-auto daemon, which runs when configfs
//! exists; see otg.rs resolve_udc.)

use crate::config::load_device_config;
use crate::props::{get_prop, set_prop};
use crate::stage::run_stage;
use std::io::Write;
use std::path::{Path, PathBuf};

const TAG: &str = "runatinit";
const DBGLOG: &str = "/dev/logs/runatinit.log";
const FLAGS_FILE: &str = "/system/etc/twrp.flags";
const ETC_DIR: &str = "/system/etc";
/// Marker in the shipped AIO live placeholders (recovery.fstab,
/// twrp.flags, recovery.wipe). A live file still carrying it means the
/// stub-time swap did not happen — redo it here.
const PLACEHOLDER_MARK: &str = "AIO LIVE placeholder";
const SWAP_NAMES: [&str; 3] = ["recovery.fstab", "twrp.flags", "recovery.wipe"];

fn dlog(log: &mut Option<std::fs::File>, msg: &str) {
    if let Some(f) = log.as_mut() {
        let _ = writeln!(f, "{msg}");
    }
}

/// Next sd letter after `last` (a->b ... y->z). None when not incrementable.
pub fn next_otg_letter(last: char) -> Option<char> {
    // Shell: tr 'a..y' 'b..z' (z maps to itself -> treated as failure).
    if ('a'..='y').contains(&last) {
        Some((last as u8 + 1) as char)
    } else {
        None
    }
}

/// Rewrite the usb_otg line to the next sd device. Returns patched line.
pub fn patch_otg_line(line: &str, next: char) -> String {
    // Mirror of: sed "/usb_otg/s|/dev/block/sd[a-z][0-9]*|/dev/block/sd${next}1|"
    if !line.contains("usb_otg") {
        return line.to_string();
    }
    let mut out = String::with_capacity(line.len() + 4);
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if line[i..].starts_with("/dev/block/sd")
            && i + 13 < bytes.len()
            && (bytes[i + 13] as char).is_ascii_lowercase()
        {
            // consume sd[a-z][0-9]*
            let mut j = i + 14;
            while j < bytes.len() && (bytes[j] as char).is_ascii_digit() {
                j += 1;
            }
            out.push_str(&format!("/dev/block/sd{next}1"));
            i = j;
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

/// True when a live AIO file is still the unswapped placeholder.
fn needs_swap(content: &str) -> bool {
    content.contains(PLACEHOLDER_MARK)
}

/// Swap one live file from its family kit when still a placeholder.
/// Returns a short status token for the caller to log.
fn swap_one_file(etc: &Path, live: &str, family: &str) -> &'static str {
    let dst = etc.join(live);
    let cur = std::fs::read_to_string(&dst).unwrap_or_default();
    if !cur.is_empty() && !needs_swap(&cur) {
        return "already-swapped";
    }
    let src = etc.join(format!("{live}.{family}"));
    if !src.is_file() {
        return "no-kit";
    }
    match std::fs::copy(&src, &dst) {
        Ok(_) => "swapped",
        Err(_) => "copy-failed",
    }
}

/// AIO second-pass family swap (idempotent; the stub is the first pass).
/// Must run before fix_twrp_flags consumes twrp.flags and before the
/// recovery process parses recovery.fstab.
fn ensure_family_swap(log: &mut Option<std::fs::File>, family: &str) {
    if family.is_empty() {
        dlog(log, "family-swap: unknown family, skipped");
        crate::ko_picker::log_msg("boot", "WARN", "family-swap: unknown family, skipped");
        return;
    }
    let etc = Path::new(ETC_DIR);
    for name in SWAP_NAMES {
        match swap_one_file(etc, name, family) {
            "already-swapped" => dlog(log, &format!("family-swap: {name} already swapped, kept")),
            "swapped" => {
                dlog(log, &format!("family-swap: {name} <- {family} (rust second-pass)"));
                crate::ko_picker::log_msg(
                    "boot",
                    "INFO",
                    &format!("family-swap: {name} <- {family} (stub missed, rust fixed)"),
                );
            }
            "no-kit" => dlog(log, &format!("family-swap: no kit {name}.{family}, kept")),
            _ => dlog(log, &format!("family-swap: {name} copy FAILED")),
        }
    }
}

/// Device override: /system/etc/<device>.twrp.flags (placed by the builder
/// from devices/<codename>/twrp.flags) wins over the family default.
/// Must run after props reveal ro.hardware, before fix_twrp_flags.
/// Returns true when an override was applied.
pub fn swap_device_flags(etc_dir: &Path, device: &str) -> bool {
    if device.is_empty() {
        return false;
    }
    let src = etc_dir.join(format!("{device}.twrp.flags"));
    if !src.is_file() {
        return false;
    }
    if std::fs::copy(&src, etc_dir.join("twrp.flags")).is_ok() {
        crate::ko_picker::log_msg("boot", "INFO", &format!("twrp.flags: device override {device}"));
        return true;
    }
    false
}
/// Decide whether a twrp.flags line survives the by-name prune.
/// `exists` checks a partition base name against /dev/block by-name.
pub fn keep_flags_line(line: &str, exists: &dyn Fn(&str) -> bool) -> bool {
    let t = line.trim();
    if t.is_empty() || t.starts_with('#') {
        return true;
    }
    let blk = t.split_whitespace().nth(2).unwrap_or("");
    const PREFIX: &str = "/dev/block/platform/";
    if !(blk.starts_with(PREFIX) && blk.contains("/by-name/")) {
        return true;
    }
    let part = blk.rsplit('/').next().unwrap_or("");
    exists(part) || exists(&format!("{part}_a"))
}

/// Extracts the platform dir name from a `/dev/block/platform/<dir>/...`
/// flags path. Pure (testable); FS access lives in the caller.
fn flags_platform_dir(line: &str) -> Option<&str> {
    const PREFIX: &str = "/dev/block/platform/";
    let start = line.find(PREFIX)? + PREFIX.len();
    let rest = &line[start..];
    let end = rest.find('/')?;
    let dir = &rest[..end];
    if dir.is_empty() {
        return None;
    }
    Some(dir)
}

/// Rewrites stale `/dev/block/platform/<addr>.ufs/` prefixes in twrp.flags
/// lines to the live UFS platform dir. Lines whose baked dir exists are
/// never touched; only absent ones fall back to discovery. Returns the
/// number of rewritten lines. Pure over `live_dirs` (testable).
fn repoint_ufs_lines(lines: &mut [String], live_dirs: &[String]) -> usize {
    let mut fixed = 0;
    for line in lines.iter_mut() {
        let Some(dir) = flags_platform_dir(line) else {
            continue;
        };
        if !dir.contains("ufs") {
            continue;
        }
        if live_dirs.iter().any(|d| d == dir) {
            continue;
        }
        let mut sorted = live_dirs.to_vec();
        sorted.sort();
        let Some(target) = sorted.first() else {
            continue;
        };
        let old = format!("/dev/block/platform/{dir}/");
        let new = format!("/dev/block/platform/{target}/");
        *line = line.replacen(&old, &new, 1);
        fixed += 1;
    }
    fixed
}

/// Fold hinge state from input subsystem switch events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FoldState {
    /// Hinge open (or unknown): inner/main display.
    OpenInner,
    /// Hinge closed: cover/front display. Safe default (cover always exists).
    ClosedCover,
}

// Linux input bits (input-event-codes.h).
const EV_SW: u32 = 0x05;
const SW_LID: u32 = 0x00;
const SW_TABLET_MODE: u32 = 0x01;

// _IOR('E', nr, len): (2<<30) | (len<<16) | (0x45<<8) | nr, as u64.
// The libc ioctl request type differs per target (u64 on host glibc,
// c_int on bionic), so the call site casts with `as _`; the value always
// fits (top bit used is 2<<30).
const fn evioc(addr_bits_nr: u32, len: usize) -> u64 {
    (2 << 30) | ((len as u64) << 16) | (0x45 << 8) | addr_bits_nr as u64
}
const EVIOCGBIT_EV: u32 = 0x20;
const EVIOCGSW_NR: u32 = 0x1B;

fn bit_is_set(bits: &[u64], bit: u32) -> bool {
    let i = (bit / 64) as usize;
    i < bits.len() && (bits[i] >> (bit % 64)) & 1 == 1
}

/// Reads one ioctl bitmask from an open input fd; None on failure.
fn ioctl_bits(fd: i32, req: u64, words: usize) -> Option<Vec<u64>> {
    let mut buf = vec![0u64; words];
    // Safety: fd is open, buffer is owned and sized.
    let rc = unsafe { libc::ioctl(fd, req as _, buf.as_mut_ptr()) };
    if rc < 0 {
        return None;
    }
    Some(buf)
}

/// Hinge probe of a single /dev/input/eventN node.
/// Returns Some(state) when the node exposes a lid/tablet-mode switch.
fn probe_hinge_fd(fd: i32) -> Option<FoldState> {
    let ev = ioctl_bits(fd, evioc(EVIOCGBIT_EV, 4), 1)?;
    if !bit_is_set(&ev, EV_SW) {
        return None;
    }
    let sw = ioctl_bits(fd, evioc(EVIOCGBIT_EV + EV_SW, 8), 1)?;
    if !bit_is_set(&sw, SW_LID) && !bit_is_set(&sw, SW_TABLET_MODE) {
        return None;
    }
    let state = ioctl_bits(fd, evioc(EVIOCGSW_NR, 8), 1)?;
    // Lid switch set = folded shut = cover; tablet-mode set = flat = inner.
    if bit_is_set(&sw, SW_LID) && bit_is_set(&state, SW_LID) {
        return Some(FoldState::ClosedCover);
    }
    if bit_is_set(&sw, SW_LID) {
        return Some(FoldState::OpenInner);
    }
    if bit_is_set(&state, SW_TABLET_MODE) {
        return Some(FoldState::OpenInner);
    }
    Some(FoldState::ClosedCover)
}

/// Scans an input dir for hinge switches. Split out for testability.
fn detect_fold_state_in(input_dir: &std::path::Path) -> FoldState {
    let rd = match std::fs::read_dir(input_dir) {
        Ok(rd) => rd,
        Err(_) => return FoldState::ClosedCover,
    };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !name.starts_with("event") {
            continue;
        }
        // Safety: path from read_dir; O_RDONLY|O_CLOEXEC; fd closed below.
        let cpath = match std::ffi::CString::new(e.path().to_string_lossy().into_owned()) {
            Ok(c) => c,
            Err(_) => continue,
        };
        // Safety: path is a valid CString from read_dir; flags are valid; fd checked below.
        let fd = unsafe { libc::open(cpath.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
        if fd < 0 {
            continue;
        }
        let found = probe_hinge_fd(fd);
        // Safety: fd came from a successful open just above and is closed exactly once.
        unsafe { libc::close(fd) };
        if let Some(state) = found {
            return state;
        }
    }
    FoldState::ClosedCover
}

/// Live hinge state; unknown/absent sensor = ClosedCover (safe default).
fn detect_fold_state() -> FoldState {
    detect_fold_state_in(std::path::Path::new("/dev/input"))
}

/// Picks the active virtual canvas: folds use hinge state (inner when open
/// and inner geometry exists, else cover); slabs always use front.
fn pick_display_geom(
    cfg: &crate::config::DeviceConfig,
    hinge: FoldState,
) -> (crate::config::DisplayGeom, &'static str) {
    if cfg.is_fold {
        if hinge == FoldState::OpenInner {
            if let Some(g) = cfg.inner_display {
                return (g, "inner");
            }
        }
        return (cfg.front_display, "front");
    }
    (cfg.front_display, "front")
}

/// Applies display geometry before recovery UI starts (early-init stage):
/// DOF_SCREEN_W/H virtual canvas + progressive letterbox mode. Runs before
/// twrp.cpp SetDefaultValues/latches geometry, so first frame is correct.
fn apply_display_geometry(
    log: &mut Option<std::fs::File>,
    cfg: &crate::config::DeviceConfig,
    hinge: FoldState,
) {
    let (geom, which) = pick_display_geom(cfg, hinge);
    let _ = set_prop("DOF_SCREEN_W", &geom.w.to_string());
    let _ = set_prop("DOF_SCREEN_H", &geom.h.to_string());
    let _ = set_prop("DOF_PROGRESSIVE_SCALE", "1");
    // Per-device status-bar height (pixel.json status_h): lets data.cpp drop
    // the compile-time OF_STATUS_H default, same pattern as DOF_SCREEN_H.
    let _ = set_prop("DOF_STATUS_H", &cfg.status_h.to_string());
    crate::ko_picker::log_msg(
        "boot",
        "INFO",
        &format!("display: {which} canvas {}x{} status_h={} (fold={}, hinge={hinge:?})", geom.w, geom.h, cfg.status_h, cfg.is_fold),
    );
    dlog(log, &format!("display: {which} canvas {}x{} status_h={}", geom.w, geom.h, cfg.status_h));
}

fn by_name_exists(part: &str) -> bool {
    // ls /dev/block/platform/*/by-name/<part>
    if let Ok(platform) = std::fs::read_dir("/dev/block/platform") {
        for plat in platform.flatten() {
            let cand = plat.path().join("by-name").join(part);
            // symlink_metadata: by-name entries are symlinks that may dangle
            // while ueventd coldboot is still running; exists() follows the
            // link and would wrongly report a live partition as absent.
            if cand.symlink_metadata().is_ok() {
                return true;
            }
        }
    }
    false
}

/// fix_twrp_flags: OTG sd-letter bump + by-name prune. Mirrors the shell logic.
fn fix_twrp_flags(log: &mut Option<std::fs::File>) {
    if !Path::new(FLAGS_FILE).exists() {
        dlog(log, "fix_twrp_flags: NOT FOUND, skip");
        return;
    }
    // Wait up to 3s for ueventd coldboot sd? nodes.
    for i in 0..3 {
        let hit = std::fs::read_dir("/dev/block")
            .map(|rd| {
                rd.flatten().any(|e| {
                    let n = e.file_name().to_string_lossy().into_owned();
                    n.len() == 3 && n.starts_with("sd")
                })
            })
            .unwrap_or(false);
        if hit {
            break;
        }
        dlog(log, &format!("waiting for /dev/block/sd? ... attempt {i}"));
        std::thread::sleep(std::time::Duration::from_secs(1));
    }

    let content = std::fs::read_to_string(FLAGS_FILE).unwrap_or_default();
    let mut lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();

    // 1. USB OTG device: next letter after last internal UFS disk.
    let mut last: Option<char> = None;
    if let Ok(rd) = std::fs::read_dir("/dev/block") {
        let mut disks: Vec<String> = rd
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.len() == 3 && n.starts_with("sd"))
            .collect();
        disks.sort();
        if let Some(l) = disks.last() {
            last = l.chars().nth(2);
        }
    }
    if let Some(l) = last {
        if let Some(next) = next_otg_letter(l) {
            for line in lines.iter_mut() {
                if line.contains("usb_otg") {
                    *line = patch_otg_line(line, next);
                }
            }
            crate::ko_picker::log_msg(
                "boot",
                "INFO",
                &format!("twrp.flags: USB OTG -> /dev/block/sd{next}1"),
            );
            dlog(log, &format!("OTG patched -> sd{next}1"));
        } else {
            dlog(log, &format!("WARNING: cannot increment '{l}'"));
        }
    } else {
        crate::ko_picker::log_msg("boot", "WARN", "twrp.flags: no sd? found, OTG unchanged");
    }

    // 1b. UFS platform fallback: repoint baked family paths that are absent
    // on this device to the live *ufs* dir (new SoC / stale family file).
    // Static-first: existing dirs are never rewritten. Runs before the
    // prune so repointed lines resolve instead of being dropped.
    let live_ufs: Vec<String> = std::fs::read_dir("/dev/block/platform")
        .map(|rd| {
            let mut v: Vec<String> = rd
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.contains("ufs"))
                .collect();
            v.sort();
            v
        })
        .unwrap_or_default();
    if !live_ufs.is_empty() {
        let fixed = repoint_ufs_lines(&mut lines, &live_ufs);
        if fixed > 0 {
            crate::ko_picker::log_msg(
                "boot",
                "INFO",
                &format!("twrp.flags: UFS fallback repointed {fixed} line(s) -> {}", live_ufs[0]),
            );
            dlog(log, &format!("UFS fallback: repointed {fixed} line(s) -> {}", live_ufs[0]));
        }
    }

    // 2. Prune by-name entries absent on this device (guard: skip when empty).
    let populated = std::fs::read_dir("/dev/block/platform")
        .map(|rd| {
            rd.flatten().any(|plat| {
                plat.path()
                    .join("by-name")
                    .read_dir()
                    .map(|mut r| r.next().is_some())
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false);
    if !populated {
        crate::ko_picker::log_msg("boot", "WARN", "twrp.flags: by-name not ready, prune skipped");
        dlog(log, "by-name not ready, prune skipped");
    } else {
        let mut kept = 0;
        let mut pruned = 0;
        let mut out = Vec::new();
        for line in &lines {
            if keep_flags_line(line, &by_name_exists) {
                kept += 1;
                out.push(line.clone());
            } else {
                pruned += 1;
                dlog(log, &format!("PRUNED: {line}"));
            }
        }
        lines = out;
        dlog(log, &format!("prune done: kept={kept} pruned={pruned}"));
    }
    let _ = std::fs::write(FLAGS_FILE, lines.join("\n") + "\n");
}

/// Entry point for the `init` subcommand.
///
/// Property application, setenforce and magiskboot extraction run as
/// pixelrunatboot.sh stages (external binaries); file fixups and the
/// servicemanager flag stay in Rust (syscalls only).
pub fn run_init() -> Result<(), String> {
    let _ = std::fs::create_dir_all("/dev/logs");
    let mut log = std::fs::File::create(DBGLOG).ok();
    dlog(&mut log, "========== recovery-pixel-boot init START ==========");
    dlog(
        &mut log,
        &format!(
            "kernel={} ro.hardware={} slot={}",
            std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default().trim(),
            get_prop("ro.hardware"),
            get_prop("ro.boot.slot_suffix"),
        ),
    );

    let device_code = crate::config::resolve_device_code();
    let cfg = load_device_config(&device_code).unwrap_or_default();
    dlog(&mut log, &format!("detected family={}", cfg.family));
    let _ = TAG;

    // Props + setenforce + gs201 flags copy (resetprop lives in the stage).
    // Pairs come from /pixelrunatboot.json (family-merged at build time).
    let mut args: Vec<String> = vec![cfg.family.clone()];
    args.extend(cfg.props.iter().map(|(k, v)| format!("{k}={v}")));
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    match run_stage("props-apply", &arg_refs) {
        Ok(o) => dlog(&mut log, &format!("props-apply: {o} props")),
        Err(e) => dlog(&mut log, &format!("props-apply FAILED: {e}")),
    }

    // Re-resolve: applied props may have corrected ro.product.device
    // (ro.hardware from the bootloader can be family-level). Only the
    // flags swap below needs the corrected code; props/family already ran.
    let mut final_code = device_code.clone();
    {
        let c2 = crate::config::resolve_device_code();
        if c2 != device_code && crate::config::device_section_exists(&c2) {
            dlog(&mut log, &format!("device corrected: {device_code} -> {c2}"));
            final_code = c2;
        }
    }

    // AIO second-pass family swap (stub is the first pass): placeholders
    // still live here mean the stub missed. Idempotent — a swapped tree
    // is detected per file and left alone. Must precede everything that
    // consumes twrp.flags / recovery.fstab below.
    ensure_family_swap(&mut log, &cfg.family);

    // Display geometry: earliest race-free point (early-init exec, before
    // twrp.cpp SetDefaultValues reads DOF_* and before minui opens DRM).
    // Re-load config under the corrected code so folds resolve geometry.
    {
        let cfg_final = load_device_config(&final_code).unwrap_or_default();
        let hinge = if cfg_final.is_fold {
            detect_fold_state()
        } else {
            FoldState::ClosedCover
        };
        apply_display_geometry(&mut log, &cfg_final, hinge);
    }

    // Device override (<device>.twrp.flags from devices/<codename>/) wins
    // over the family default; runs after props reveal ro.hardware and
    // before fix_twrp_flags patches/prunes the file.
    if swap_device_flags(Path::new("/system/etc"), &final_code) {
        dlog(&mut log, &format!("device flags override: {final_code}"));
    }

    fix_twrp_flags(&mut log);
    // NOTE: no lgz_decompress_zips step. The solid UCOMP02 cluster ingests
    // *.zip transparently at build time and `lgz decompress` (init.cpp)
    // restores them; the old /lgz_zip_manifest.txt flow is dead (the build
    // no longer generates the manifest).

    let mut magisk: Option<PathBuf> = None;
    if let Ok(rd) = std::fs::read_dir("/system/bin") {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if n.starts_with("Magisk-") && n.ends_with(".zip") {
                magisk = Some(e.path());
                break;
            }
        }
    }
    match magisk {
        Some(z) => match run_stage("magiskboot-unpack", &[&z.to_string_lossy()]) {
            Ok(_) => dlog(&mut log, "magiskboot-unpack: done"),
            Err(e) => dlog(&mut log, &format!("magiskboot-unpack FAILED: {e}")),
        },
        None => {
            dlog(&mut log, "no Magisk zip found");
            crate::ko_picker::log_msg("boot", "WARN", "magisk: no zip, skipping extraction");
        }
    }

    let _ = set_prop("servicemanager.ready", "true");
    dlog(&mut log, "========== recovery-pixel-boot init END ==========");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn family_mapping() {
        // Family now comes from /pixelrunatboot.json, not code.
        // Spot-check the contract via the config loader fixture shape.
        let d = std::env::temp_dir().join(format!("fox_test_fam_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("c.json");
        std::fs::write(&f, r#"{"shiba": {"family": "zuma"}}"#).unwrap();
        let c = crate::config::load_device_config_from(&f, "shiba").unwrap();
        assert_eq!(c.family, "zuma");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn family_swap_marker() {
        assert!(needs_swap("# recovery.fstab — AIO LIVE placeholder (intentionally empty).\n"));
        assert!(!needs_swap("# recovery.fstab — Partition layout for Zuma SoC\n"));
        // Empty/missing live file has no marker, but swap_one_file still
        // attempts the kit copy (the `is_empty` branch) — covered below.
        assert!(!needs_swap(""));
        assert!(!needs_swap(&std::fs::read_to_string("/nonexistent-fox").unwrap_or_default()));
    }

    #[test]
    fn family_swap_roundtrip() {
        let d = std::env::temp_dir().join(format!("fox_test_famswap_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("recovery.fstab"), b"# AIO LIVE placeholder\n").unwrap();
        std::fs::write(d.join("recovery.fstab.zuma"), b"REAL-KIT").unwrap();
        assert_eq!(swap_one_file(&d, "recovery.fstab", "zuma"), "swapped");
        assert_eq!(std::fs::read(&d.join("recovery.fstab")).unwrap(), b"REAL-KIT");
        // Second pass is a no-op.
        assert_eq!(swap_one_file(&d, "recovery.fstab", "zuma"), "already-swapped");
        // Missing kit keeps the placeholder.
        std::fs::write(d.join("twrp.flags"), b"# AIO LIVE placeholder\n").unwrap();
        assert_eq!(swap_one_file(&d, "twrp.flags", "zuma"), "no-kit");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn otg_letter_steps() {
        assert_eq!(next_otg_letter('a'), Some('b'));
        assert_eq!(next_otg_letter('y'), Some('z'));
        assert_eq!(next_otg_letter('z'), None);
    }

    #[test]
    fn otg_line_rewrite() {
        let line = "/usb_otg   vfat   /dev/block/sdc1   flags";
        assert_eq!(
            patch_otg_line(line, 'd'),
            "/usb_otg   vfat   /dev/block/sdd1   flags"
        );
        let plain = "# comment";
        assert_eq!(patch_otg_line(plain, 'd'), plain);
        // Trailing "/dev/block/sd" with no letter: must not panic, left as-is.
        let edge = "/usb_otg vfat /dev/block/sd";
        assert_eq!(patch_otg_line(edge, 'd'), edge);
        // Letter with no partition number still rewrites.
        let no_num = "/usb_otg vfat /dev/block/sdc flags";
        assert_eq!(patch_otg_line(no_num, 'd'), "/usb_otg vfat /dev/block/sdd1 flags");
    }

    #[test]
    fn device_override_swap() {
        let d = std::env::temp_dir().join(format!("fox_test_swap_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("twrp.flags"), b"FAMILY").unwrap();
        std::fs::write(d.join("shiba.twrp.flags"), b"DEVICE").unwrap();
        assert!(swap_device_flags(&d, "shiba"));
        assert_eq!(std::fs::read(&d.join("twrp.flags")).unwrap(), b"DEVICE");
        assert!(!swap_device_flags(&d, "husky"));
        assert!(!swap_device_flags(&d, ""));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn ufs_platform_dir_parse() {
        assert_eq!(
            flags_platform_dir("/m f2fs /dev/block/platform/13200000.ufs/by-name/metadata flags"),
            Some("13200000.ufs")
        );
        assert_eq!(flags_platform_dir("/a ext4 /dev/block/sda1"), None);
        assert_eq!(flags_platform_dir("# comment"), None);
        assert_eq!(flags_platform_dir("/x emmc /dev/block/platform/"), None);
    }

    #[test]
    fn ufs_fallback_repoints_only_absent_dirs() {
        let live = vec!["3c2d0000.ufs".to_string()];
        // Absent family dir -> repointed to the live one.
        let mut lines = vec![
            "/m f2fs /dev/block/platform/13200000.ufs/by-name/metadata flags".to_string(),
            "/s emmc /dev/block/platform/13200000.ufs/by-name/super flags".to_string(),
        ];
        assert_eq!(repoint_ufs_lines(&mut lines, &live), 2);
        assert!(lines[0].contains("/dev/block/platform/3c2d0000.ufs/by-name/metadata"));
        // Present dir -> untouched.
        let mut ok = vec!["/m f2fs /dev/block/platform/3c2d0000.ufs/by-name/metadata flags".to_string()];
        assert_eq!(repoint_ufs_lines(&mut ok, &live), 0);
        // Non-ufs platform dir -> untouched.
        let mut other = vec!["/u auto /dev/block/platform/soc/by-name/x flags".to_string()];
        assert_eq!(repoint_ufs_lines(&mut other, &live), 0);
        // No live dirs -> nothing rewritten.
        let mut lines2 = vec!["/m f2fs /dev/block/platform/13200000.ufs/by-name/metadata flags".to_string()];
        let empty: Vec<String> = Vec::new();
        assert_eq!(repoint_ufs_lines(&mut lines2, &empty), 0);
        assert!(lines2[0].contains("13200000.ufs"));
    }

    #[test]
    fn fold_display_pick() {
        use crate::config::{DeviceConfig, DisplayGeom};
        let mut cfg = DeviceConfig::default();
        cfg.front_display = DisplayGeom { w: 1080, h: 2424 };
        // Slab ignores hinge.
        assert_eq!(pick_display_geom(&cfg, FoldState::OpenInner).1, "front");
        // Fold without inner geometry falls back to cover.
        cfg.is_fold = true;
        assert_eq!(pick_display_geom(&cfg, FoldState::OpenInner).1, "front");
        // Fold open with inner geometry.
        cfg.inner_display = Some(DisplayGeom { w: 2076, h: 2152 });
        let (g, which) = pick_display_geom(&cfg, FoldState::OpenInner);
        assert_eq!(which, "inner");
        assert_eq!((g.w, g.h), (2076, 2152));
        // Fold closed -> cover.
        let (g, which) = pick_display_geom(&cfg, FoldState::ClosedCover);
        assert_eq!(which, "front");
        assert_eq!((g.w, g.h), (1080, 2424));
    }

    #[test]
    fn hinge_scan_empty_dir_defaults_to_cover() {
        let d = std::env::temp_dir().join(format!("fox_test_hinge_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        assert_eq!(detect_fold_state_in(&d), FoldState::ClosedCover);
        assert_eq!(detect_fold_state_in(std::path::Path::new("/nonexistent-fox")), FoldState::ClosedCover);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn bit_test_helper() {
        assert!(bit_is_set(&[0b101], 0));
        assert!(!bit_is_set(&[0b101], 1));
        assert!(bit_is_set(&[0b101], 2));
        assert!(!bit_is_set(&[0b101], 9));
        assert!(!bit_is_set(&[], 0));
    }

    #[test]
    fn prune_keeps_non_byname_and_present() {
        let exists = |p: &str| p == "vendor" || p == "vendor_a";
        // non-by-name line always kept
        assert!(keep_flags_line("/a ext4 /dev/block/sda1", &exists));
        // present partition kept (base or _a)
        assert!(keep_flags_line("/v ext4 /dev/block/platform/1/by-name/vendor", &exists));
        assert!(keep_flags_line("/b ext4 /dev/block/platform/1/by-name/boot", &|p: &str| p
            == "boot_a"));
        // absent pruned
        assert!(!keep_flags_line("/m ext4 /dev/block/platform/1/by-name/modem", &exists));
        // comments kept
        assert!(keep_flags_line("# c", &exists));
    }
}
