//! init — early-init device identity. Port of runatinit.sh.
//!
//! Runs on `early-init` via exec, BEFORE USB gadget configfs and TWRP
//! data.cpp: family/device props, twrp.flags fix, LGZ zip payload restore,
//! magiskboot extraction. Called as `recovery-pixel-boot init`.

use crate::props::{get_prop, set_prop};
use crate::stage::run_stage;
use std::io::Write;
use std::path::{Path, PathBuf};

const TAG: &str = "runatinit";
const DBGLOG: &str = "/dev/logs/runatinit.log";
const PROPS_DIR: &str = "/system/etc/device_props";
const FLAGS_FILE: &str = "/system/etc/twrp.flags";

fn dlog(log: &mut Option<std::fs::File>, msg: &str) {
    if let Some(f) = log.as_mut() {
        let _ = writeln!(f, "{msg}");
    }
}

/// ro.hardware -> SoC family (1:1 with the shell case table).
pub fn family_for_device(code: &str) -> &'static str {
    match code {
        "panther" | "cheetah" | "lynx" | "gs201" => "gs201",
        "shiba" | "husky" | "akita" | "zuma" => "zuma",
        "tokay" | "komodo" | "caiman" | "tegu" | "stallion" | "zumapro" => "zumapro",
        _ => "",
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

    let device_code = get_prop("ro.hardware");
    let family = family_for_device(&device_code);
    dlog(&mut log, &format!("detected family={family}"));
    let _ = TAG;

    // Props + setenforce + gs201 flags copy (resetprop lives in the stage).
    match run_stage("props-apply", &[PROPS_DIR, family, &device_code]) {
        Ok(o) => dlog(&mut log, &format!("props-apply: {o} props")),
        Err(e) => dlog(&mut log, &format!("props-apply FAILED: {e}")),
    }

    if family == "gs201" && Path::new("/system/etc/twrp_gs201.flags").exists() {
        let _ = std::fs::copy("/system/etc/twrp_gs201.flags", "/system/etc/twrp.flags");
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
        assert_eq!(family_for_device("shiba"), "zuma");
        assert_eq!(family_for_device("husky"), "zuma");
        assert_eq!(family_for_device("akita"), "zuma");
        assert_eq!(family_for_device("zuma"), "zuma");
        assert_eq!(family_for_device("panther"), "gs201");
        assert_eq!(family_for_device("tokay"), "zumapro");
        assert_eq!(family_for_device("stallion"), "zumapro");
        assert_eq!(family_for_device("unknown"), "");
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
