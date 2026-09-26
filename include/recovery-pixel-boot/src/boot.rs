//! boot — post-GUI early-boot setup. Port of the vendor half of runatboot.sh.
//!
//! The Rust engine does syscalls only. Every external binary runs inside a
//! `pixelrunatboot.sh` stage (siw/iw fetch, magiskboot unpack, meta fix).
//! Called as `recovery-pixel-boot boot` from an `on boot` exec (and, during
//! transition, from the runatboot.sh delegation wrapper).

use crate::config::{DeviceConfig, load_device_config_fallback};
use crate::ko_picker::{is_module_loaded, load_kernel_module, log_msg, ko_try_load, score_candidate, detect_kernel_env};
use crate::props::get_prop;
use crate::stage::{is_mounted, run_stage};
use std::path::{Path, PathBuf};

const TAG: &str = "boot";

fn info(msg: &str) {
    log_msg(TAG, "INFO", msg);
}

fn warn(msg: &str) {
    log_msg(TAG, "WARN", msg);
}

fn error(msg: &str) {
    log_msg(TAG, "ERROR", msg);
}

/// Load the device section; unknown codename -> empty (skip like the old
/// `*` branch), never fatal. Family-level fallback included: SoC-named
/// hardware ("malibu") borrows the first section of its family.
fn device_config(code: &str) -> DeviceConfig {
    match load_device_config_fallback(code) {
        Ok(c) => c,
        Err(e) => {
            warn(&format!("no config section for {code}: {e}"));
            DeviceConfig::default()
        }
    }
}

/// (suffix, unsuffix, slotnum, unslotnum). Slot suffix from the property API;
/// bootctl fallback runs as a shell stage (no forks in Rust).
pub fn detect_slots() -> (String, String, String, String) {
    let mut suffix = get_prop("ro.boot.slot_suffix");
    if suffix.is_empty() {
        match run_stage("slot-detect", &[]) {
            Ok(s) if s == "_a" || s == "_b" => {
                info(&format!("slot-detect stage: {s}"));
                suffix = s;
            }
            other => warn(&format!("slot-detect failed: {other:?}")),
        }
    }
    match suffix.as_str() {
        "_a" => ("_a".into(), "_b".into(), "0".into(), "1".into()),
        "_b" => ("_b".into(), "_a".into(), "1".into(), "0".into()),
        _ => (suffix, String::new(), String::new(), String::new()),
    }
}

/// Recursively collect files under dir whose name ends with `{module}.ko`.
fn collect_ko(dir: &Path, module: &str, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            collect_ko(&p, module, out);
        } else if let Some(n) = p.file_name().and_then(|n| n.to_str()) {
            if n.ends_with(&format!("{module}.ko")) {
                out.push(p);
            }
        }
    }
}

/// Load wanted modules from an explicit staged-file list (best score first).
/// Returns the subset of `modules` still missing afterwards.
fn load_staged(staged: &[PathBuf], modules: &[String]) -> Vec<String> {
    let env = detect_kernel_env().ok();
    let mut missing = Vec::new();
    for module in modules {
        let name = module.replace('-', "_");
        if is_module_loaded(&name) {
            continue;
        }
        let mut cands: Vec<(u32, &PathBuf)> = staged
            .iter()
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.ends_with(&format!("{module}.ko")))
                    .unwrap_or(false)
            })
            .map(|p| {
                let s = env
                    .as_ref()
                    .and_then(|e| score_candidate(p, e))
                    .map(|sc| sc.score)
                    .unwrap_or(10);
                (s, p)
            })
            .collect();
        cands.sort_by(|a, b| b.0.cmp(&a.0));
        let mut ok = is_module_loaded(&name);
        for (_, p) in &cands {
            match load_kernel_module(p) {
                Ok(()) => {
                    info(&format!("modules: {module} loaded from {}", p.display()));
                    ok = true;
                    break;
                }
                Err(e) => info(&format!("modules: {} failed: {e}", p.display())),
            }
        }
        if !ok {
            missing.push(module.to_string());
        }
    }
    missing
}

fn check_modules_loaded(modules: &[String]) -> bool {
    let mut missing = Vec::new();
    for module in modules {
        let name = module.replace('-', "_");
        if !is_module_loaded(&name) {
            missing.push(module.to_string());
        }
    }
    if missing.is_empty() {
        info("modules: all modules loaded successfully");
        true
    } else {
        error(&format!("modules: missing: {}", missing.join(" ")));
        false
    }
}

/// Standard stock-module fetch+load through the ko-fetch shell stage
/// (siw|iw stream with map+mount+loop-connect fallback, both slots).
/// Shared by touch init and USB/OTG staging — one mechanism, not two:
/// call this first, mount-hunt only for non-.ko files (aocd/libs) that
/// ko-fetch cannot carry. `part` is the LP base ("vendor_dlkm"),
/// `modules` the .ko basenames (either dash spelling).
/// Fast no-op when everything is already loaded (first-stage autoload),
/// so daemons can call it every tick without re-streaming images.
pub(crate) fn fetch_stock_modules(part: &str, modules: &[String]) -> bool {
    if modules
        .iter()
        .map(|m| m.replace('-', "_"))
        .all(|n| is_module_loaded(&n))
    {
        return true;
    }
    let (suffix, unsuffix, slot, unslot) = detect_slots();
    let sfx = suffix.trim_start_matches('_');
    let usfx = unsuffix.trim_start_matches('_');
    if !sfx.is_empty() && try_slot(part, sfx, &slot, modules) {
        return true;
    }
    if !usfx.is_empty() && try_slot(part, usfx, &unslot, modules) {
        return true;
    }
    modules
        .iter()
        .map(|m| m.replace('-', "_"))
        .all(|n| is_module_loaded(&n))
}

/// Fetch .ko staging for one slot via siw|iw (shell stage), load wanted ones.
/// Returns true when nothing is missing afterwards.
fn try_slot(part: &str, sfx: &str, slotnum: &str, modules: &[String]) -> bool {
    let staged: Vec<PathBuf> = match run_stage("ko-fetch", &[part, sfx, slotnum]) {
        Ok(out) => out
            .lines()
            .filter(|l| l.ends_with(".ko"))
            .map(PathBuf::from)
            .collect(),
        Err(e) => {
            warn(&format!("modules: ko-fetch {part}_{sfx} failed: {e}"));
            return false;
        }
    };
    if staged.is_empty() {
        warn(&format!("modules: ko-fetch {part}_{sfx} staged nothing"));
        return false;
    }
    let missing = load_staged(&staged, modules);
    if missing.is_empty() {
        return true;
    }
    warn(&format!("modules: missing after {part}_{sfx}: {}", missing.join(" ")));
    false
}

fn modules_touch_install(cfg: &DeviceConfig, suffix: &str, unsuffix: &str, slot: &str, unslot: &str) {
    let sfx = suffix.trim_start_matches('_');
    let usfx = unsuffix.trim_start_matches('_');
    // Provider preload (dependency order): e.g. pwrseq-core from system_dlkm
    // before lwis on 6.12. Best-effort — a missing partition/module (6.1
    // needs nothing) only warns via try_slot.
    if !cfg.preload_modules.is_empty() && !cfg.part_sysdlkm.is_empty() {
        info(&format!("modules: preloading providers from {}", cfg.part_sysdlkm));
        if !sfx.is_empty() {
            try_slot(&cfg.part_sysdlkm, sfx, slot, &cfg.preload_modules);
        }
        if !usfx.is_empty() {
            try_slot(&cfg.part_sysdlkm, usfx, unslot, &cfg.preload_modules);
        }
    }
    info(&format!("modules: trying current slot {suffix}"));
    let mut ok = if !sfx.is_empty() {
        try_slot(&cfg.part_touch, sfx, slot, &cfg.touch_modules)
    } else {
        false
    };
    if !ok && !usfx.is_empty() {
        info(&format!("modules: trying opposite slot {unsuffix}"));
        ok = try_slot(&cfg.part_touch, usfx, unslot, &cfg.touch_modules);
    }
    if !ok {
        info("modules: trying fallback /system/modules_touch");
        let mut staged = Vec::new();
        for m in &cfg.touch_modules {
            collect_ko(Path::new("/system/modules_touch"), m, &mut staged);
        }
        let _ = load_staged(&staged, &cfg.touch_modules);
    }
    if !check_modules_loaded(&cfg.touch_modules) {
        error("modules: final failure, still missing");
    }
    // Re-resolve the thermal symlink: zone topology may have changed now
    // that vendor_dlkm drivers are loaded (early on-init scan predates them).
    // Idempotent (rm + symlink), best-effort.
    if let Err(e) = crate::temp::run_setup_temp() {
        warn(&format!("thermal refresh failed: {e}"));
    }
    let _ = ok;
}

/// Late backlight nudge for late-probing panels (malibu DPU et al).
///
/// The panel backlight node appears only after display DLKM probes,
/// which can lose the race with TWRP's one-shot brightness discovery
/// at startup: the slider then stays hidden and the screen sits at the
/// driver default (dark, reported as "brightness 0"). If every live
/// backlight node reads below 10% of its max, raise it to 50% — a
/// no-op when TWRP already set a sane value, and TWRP's own writes
/// still win afterwards. Pure over `dir` (testable).
fn nudge_backlight_dir(dir: &Path) -> bool {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return false,
    };
    let mut nudged = false;
    for e in entries.flatten() {
        let node = e.path().join("brightness");
        let max_node = e.path().join("max_brightness");
        let cur: u32 = std::fs::read_to_string(&node)
            .unwrap_or_default()
            .trim()
            .parse()
            .unwrap_or(0);
        let max: u32 = std::fs::read_to_string(&max_node)
            .unwrap_or_default()
            .trim()
            .parse()
            .unwrap_or(0);
        if max == 0 || cur * 10 >= max {
            continue;
        }
        let target = max / 2;
        if std::fs::write(&node, format!("{target}\n")).is_ok() {
            info(&format!(
                "brightness: nudged {} -> {target} (max {max})",
                node.display()
            ));
            nudged = true;
        }
    }
    nudged
}

fn nudge_brightness() {
    nudge_backlight_dir(Path::new("/sys/class/backlight"));
}

fn find_magisk_zip() -> Option<PathBuf> {
    let entries = std::fs::read_dir("/system/bin").ok()?;
    for e in entries.flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        if n.starts_with("Magisk-") && n.ends_with(".zip") && e.path().is_file() {
            return Some(e.path());
        }
    }
    None
}

fn magisk_link(zip: &Path) {
    let _ = std::fs::create_dir_all("/FFiles/OF_Magisk");
    let _ = std::fs::create_dir_all("/sdcard/Fox/FoxFiles");
    let _ = std::fs::copy(zip, "/FFiles/OF_Magisk/Magisk.zip");
    let _ = std::fs::copy(zip, "/FFiles/OF_Magisk/uninstall.zip");
    // Background daemon (forked process: survives boot's exit).
    // Pure filesystem + fork: no external binaries involved.
    let owned = zip.to_path_buf();
    // SAFETY: only the main thread exists at this point (boot spawns no
    // threads), no locks are held; the child enters an infinite daemon loop
    // and never touches shared state, the parent just continues.
    let pid = unsafe { libc::fork() };
    if pid == 0 {
        // SAFETY: setsid takes no pointers; detaches the daemon from init's
        // session/control terminal.
        unsafe {
            libc::setsid();
        }
        loop {
            if Path::new("/data/media/0").is_dir() && is_mounted("/data") {
                let _ = std::fs::create_dir_all("/data/media/0/Fox/FoxFiles");
                let _ = std::fs::copy(&owned, "/data/media/0/Fox/FoxFiles/uninstall.zip");
                let _ = std::fs::copy(&owned, "/data/media/0/Fox/FoxFiles/Magisk.zip");
            }
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
    } else if pid == -1 {
        // fork failed: best-effort thread fallback (dies with us).
        warn("magisk: fork failed, using thread fallback");
        std::thread::spawn(move || loop {
            if Path::new("/data/media/0").is_dir() {
                let _ = std::fs::create_dir_all("/data/media/0/Fox/FoxFiles");
                let _ = std::fs::copy(&owned, "/data/media/0/Fox/FoxFiles/uninstall.zip");
                let _ = std::fs::copy(&owned, "/data/media/0/Fox/FoxFiles/Magisk.zip");
            }
            std::thread::sleep(std::time::Duration::from_secs(2));
        });
    }
}

/// Entry point for the `boot` subcommand.
pub fn run_boot() -> Result<(), String> {
    // Dump busybox to /dev tmpfs so applets survive package-manager replaces.
    if Path::new("/system/bin/busybox").exists() {
        let _ = std::fs::create_dir_all("/dev/.fox_bb");
        if std::fs::copy("/system/bin/busybox", "/dev/.fox_bb/busybox").is_ok() {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(
                "/dev/.fox_bb/busybox",
                std::fs::Permissions::from_mode(0o755),
            );
            info("busybox: dumped to /dev/.fox_bb/busybox");
        } else {
            warn("busybox: dump failed, will use PATH");
        }
    }

    if let Ok(entries) = std::fs::read_dir("/system/bin") {
        use std::os::unix::fs::PermissionsExt;
        for e in entries.flatten() {
            let _ = std::fs::set_permissions(e.path(), std::fs::Permissions::from_mode(0o777));
        }
    }

    let device_code = crate::config::resolve_device_code();
    let (suffix, unsuffix, slot, unslot) = detect_slots();

    if ko_try_load("susfs_rename_fix", Some("/proc/susfs_rename_fix"), "susfs_fix") {
        info("susfs_fix: loaded — fast-symlink rename panic fix active");
    } else {
        warn("susfs_fix: no usable susfs_rename_fix.ko for this kernel");
    }

    let cfg = device_config(&device_code);
    info(&format!(
        "device {device_code} family={} soc={}",
        cfg.family, cfg.soc_family
    ));
    if !cfg.touch_modules.is_empty() {
        // Haptics firmware via siw|iw stages (no mounts in Rust).
        let sfx = suffix.trim_start_matches('_');
        let usfx = unsuffix.trim_start_matches('_');
        let mut fw_ok = false;
        if !sfx.is_empty() {
            fw_ok = run_stage("fw-fetch", &[&cfg.part_vendor, sfx, &slot])
                .map(|o| {
                    info(&format!("vendor_fw: staged files: {o}"));
                    true
                })
                .unwrap_or(false);
        }
        if !fw_ok && !usfx.is_empty() {
            fw_ok = run_stage("fw-fetch", &[&cfg.part_vendor, usfx, &unslot]).is_ok();
        }
        if !fw_ok {
            warn("vendor_fw: firmware fetch failed on all slots");
        }

        modules_touch_install(&cfg, &suffix, &unsuffix, &slot, &unslot);

        let pm = cfg.cs40l26_pm.as_str();
        if !pm.is_empty() && Path::new(pm).exists() {
            let _ = std::fs::write(pm, b"on\n");
            info("haptics: CS40L26 runtime PM set to 'on'");
        }
    }

    // USB/OTG stock modules, right after touch, same process, strictly
    // sequential: concurrent ko-fetch streams to shared /dev/ko_stage +
    // /dev/stage_*.img paths would clobber each other. The OTG daemon
    // never streams images itself — it only picks up staged files and
    // read-only mounts from here on.
    if !cfg.usb_modules.is_empty() {
        let part = if cfg.part_usb.is_empty() {
            "vendor_dlkm"
        } else {
            cfg.part_usb.as_str()
        };
        info(&format!(
            "usb modules: loading {} from {part}",
            cfg.usb_modules.join(" ")
        ));
        if fetch_stock_modules(part, &cfg.usb_modules) {
            info("usb modules: all loaded successfully");
        } else {
            warn(&format!(
                "usb modules: missing after fetch: {} (OTG degrades to device mode)",
                cfg.usb_modules.join(" ")
            ));
        }
    }

    // Backlight may still read dark here (late panel probe vs TWRP's
    // one-shot discovery); nudge before the GUI settles. No-op when
    // TWRP already set a sane value.
    nudge_brightness();

    if Path::new("/dev/lwis-flash-lm3644").exists() {
        info("torch: /dev/lwis-flash-lm3644 available");
    } else {
        // Not fatal on LWIS families (malibu/laguna): torch drives the
        // LM3644 straight from the flash@ device-tree node (see torch.rs
        // LWIS fallback) and never needs the camera-stack /dev node.
        info("torch: no /dev/lwis-flash-lm3644 (LWIS-DT direct drive applies on malibu/laguna)");
    }

    match run_stage("meta-fix", &[]) {
        Ok(_) => info("meta-fix: done"),
        Err(e) => warn(&format!("meta-fix failed: {e}")),
    }
    match find_magisk_zip() {
        Some(z) => magisk_link(&z),
        None => warn("magisk: no Magisk zip in /system/bin, skipping"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touch_matrix_spot_check() {
        // Module lists now come from /pixelrunatboot.json, not code.
        // Contract check: a shiba-shaped section loads with the right set.
        let d = std::env::temp_dir().join(format!("fox_test_matrix_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("c.json");
        std::fs::write(
            &f,
            r#"{"shiba": {"touch_modules": ["sec_touch", "ftm5", "fps_touch_handler"]}}"#,
        )
        .unwrap();
        let c = crate::config::load_device_config_from(&f, "shiba").unwrap();
        assert!(c.touch_modules.contains(&"sec_touch".to_string()));
        assert!(!c.touch_modules.contains(&"syna_touch".to_string()));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn backlight_nudge_raises_only_dark_nodes() {
        let d = std::env::temp_dir().join(format!("fox_test_bl_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let dark = d.join("panel0");
        let bright = d.join("panel1");
        let broken = d.join("panel2");
        for p in [&dark, &bright, &broken] {
            std::fs::create_dir_all(p).unwrap();
        }
        std::fs::write(dark.join("brightness"), b"0\n").unwrap();
        std::fs::write(dark.join("max_brightness"), b"3827\n").unwrap();
        std::fs::write(bright.join("brightness"), b"3827\n").unwrap();
        std::fs::write(bright.join("max_brightness"), b"3827\n").unwrap();
        // No max file: untouched.
        std::fs::write(broken.join("brightness"), b"0\n").unwrap();
        assert!(nudge_backlight_dir(&d));
        let v: u32 = std::fs::read_to_string(dark.join("brightness")).unwrap().trim().parse().unwrap();
        assert_eq!(v, 3827 / 2);
        let v: u32 = std::fs::read_to_string(bright.join("brightness")).unwrap().trim().parse().unwrap();
        assert_eq!(v, 3827);
        let v: u32 = std::fs::read_to_string(broken.join("brightness")).unwrap().trim().parse().unwrap();
        assert_eq!(v, 0);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn ko_collect_matches_suffix() {
        let d = std::env::temp_dir().join(format!("fox_test_boot_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("sub")).unwrap();
        std::fs::write(d.join("sub").join("sec_touch.ko"), b"x").unwrap();
        std::fs::write(d.join("sub").join("sec_touch.ko.bak"), b"x").unwrap();
        std::fs::write(d.join("other.ko"), b"x").unwrap();
        let mut out = Vec::new();
        collect_ko(&d, "sec_touch", &mut out);
        assert_eq!(out.len(), 1);
        let _ = std::fs::remove_dir_all(&d);
    }
}
