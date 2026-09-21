//! otg — OTG host shim injection + VBUS auto-switch daemon.
//!
//! Port of otg_patch.sh (oneshot, service otg_enable) and otg_auto_v3.sh
//! (long-running, service otg_auto).
//!
//! Deliberate deviations from the shell versions (reviewed):
//! - otg-auto does NOT touch sys.usb.patch_dwc3 (was reset to 0 at startup).
//! - switch_to_device sets sys.usb.ffs.ready exactly once (was duplicated).

use crate::config::load_device_config;
use crate::i2c::patch_max77759_i2c_with_driver;
use crate::ko_picker::{detect_kernel_env, ko_try_load, log_msg};
use crate::props::{get_prop, set_prop};
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::Duration;

const TAG: &str = "otg";
const PROC_SHIM: &str = "/proc/otg_host_shim";
const PROC_READY: &str = "/proc/otg_host_ready";
const USB_ROLE_CLASS: &str = "/sys/class/usb_role";
/// gvotable CHARGER_MODE force values driving VBUS in host mode (6.1 trick,
/// still valid on 6.12).
const CHARGER_FORCE_VALUE: &[u8] = b"49\n";
const CHARGER_FORCE_ACTIVE: &[u8] = b"1\n";

/// Kernel 6.12+: kprobe shim is dead (dwc3_otg_host_ready gone from the
/// driver) — use the native role-switch + gvotable VBUS path instead.
/// Unknown version reads as legacy (shim path) to preserve 6.1 behavior.
fn use_native_otg() -> bool {
    match detect_kernel_env() {
        Ok(e) => (e.major, e.minor) >= (6, 12),
        Err(_) => false,
    }
}

/// First usb_role switch found (e.g. 11210000.usb-role-switch/role).
fn find_usb_role() -> Option<PathBuf> {
    std::fs::read_dir(USB_ROLE_CLASS)
        .ok()?
        .flatten()
        .map(|e| e.path().join("role"))
        .find(|p| p.exists())
}

/// First dwc3_exynos_otg_id node found under the platform bus
/// (e.g. /sys/devices/platform/11210000.usb/dwc3_exynos_otg_id).
/// None on SoCs without the exynos OTG ID register (laguna/malibu) —
/// callers must skip the write instead of failing.
fn find_otg_id() -> Option<PathBuf> {
    let root = Path::new("/sys/devices/platform");
    let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let rd = match std::fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(_) => continue,
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.file_name().map(|n| n == "dwc3_exynos_otg_id").unwrap_or(false) {
                return Some(p);
            }
            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                // One level is enough (platform/<bus>/node); still allow
                // a second level for simple_usb_bus wrappers.
                if p.parent() == Some(root) || p.parent().and_then(|q| q.parent()) == Some(root) {
                    stack.push(p);
                }
            }
        }
    }
    None
}

/// Native host path (6.12+): role switch to host + gvotable VBUS force.
/// Proven live: role flips (xHCI enumerates), CHARGER_MODE force sets
/// otg_on=1, USB mouse works. otg_id follows the role on its own; typec
/// writes fail on the TCPC side and are skipped by design.
fn native_activate() -> Result<(), String> {
    let role = find_usb_role().ok_or_else(|| "no usb_role switch found".to_string())?;
    info("switching controller role to host");
    let _ = std::fs::write(&role, b"host\n");
    sleep(Duration::from_secs(2));
    info("forcing VBUS via gvotable CHARGER_MODE");
    let _ = std::fs::write(CHARGER_VALUE, CHARGER_FORCE_VALUE);
    let _ = std::fs::write(CHARGER_ACTIVE, CHARGER_FORCE_ACTIVE);
    sleep(Duration::from_secs(2));
    let back = std::fs::read_to_string(&role).unwrap_or_default();
    if back.trim() == "host" {
        info("native host path ACTIVE (role=host, VBUS forced)");
        Ok(())
    } else {
        Err(format!("role switch did not stick: {:?}", back.trim()))
    }
}

fn info(msg: &str) {
    log_msg(TAG, "INFO", msg);
    println!("otg: INFO: {msg}");
}

fn err(msg: &str) {
    log_msg(TAG, "ERROR", msg);
    eprintln!("otg: ERROR: {msg}");
}

/// otg-patch: inject otg_host_shim, activate host_ready, set patch_dwc3, fix TCPC switch.
pub fn run_otg_patch() -> Result<(), String> {
    info("starting OTG patch routine");

    // debugfs for CHARGER_MODE gvotables (used later by otg-auto).
    // NOTE: no c".." literals — AOSP 14 Soong rustc predates their
    // stabilization; use NUL-terminated byte strings instead.
    // SAFETY: all three are valid NUL-terminated C strings ('static byte
    // arrays); mount(2) with debugfs fstype, NULL data is the standard call.
    let rc = unsafe {
        libc::mount(
            b"debugfs\0".as_ptr() as *const libc::c_char,
            b"/sys/kernel/debug\0".as_ptr() as *const libc::c_char,
            b"debugfs\0".as_ptr() as *const libc::c_char,
            0,
            std::ptr::null(),
        )
    };
    if rc == 0 {
        info("debugfs mounted");
    }

    // 6.12+ device-tree branch: shim excluded by design (its kprobe
    // target dwc3_otg_host_ready no longer exists) — native role-switch
    // + gvotable VBUS path instead. Legacy kernels keep the shim path.
    let mut native_done = false;
    if Path::new(PROC_SHIM).exists() {
        info("module already loaded (/proc/otg_host_shim exists)");
    } else if use_native_otg() {
        // 6.12+ branch: no shim by design. Mirror the otg-auto rule:
        // host force ONLY when no PC is present (VBUS=0). Unconditional
        // forcing kills adb (role flips to host with the PC attached)
        // and the daemon then flaps against a failing UDC bind.
        let vbus_file = find_vbus_path(&vbus_candidates());
        let initial = read_vbus(&vbus_file);
        info(&format!("initial VBUS: {initial}"));
        if initial == "1" {
            info("PC detected at boot, staying in DEVICE mode (no host force)");
            // Resolved without forcing: daemon still starts and manages roles.
            native_done = true;
        } else {
            info("no PC at boot, native host path");
            match native_activate() {
                Ok(()) => native_done = true,
                Err(e) => {
                    let _ = set_prop("sys.usb.patch_dwc3", "0");
                    return Err(e);
                }
            }
        }
    } else {
        info("/proc/otg_host_shim not found, injecting module");
        if !ko_try_load("otg_host_shim", None, "otg_patch") {
            // Shim path is dead on this kernel (e.g. laguna 6.6: the
            // kprobe target dwc3_otg_host_ready is absent, same as 6.12).
            // Fall back to the native role-switch + VBUS path instead of
            // giving up: without this patch_dwc3 stays 0, otg_auto never
            // starts, and both OTG-host and the device-mode UDC rebind
            // below stay dead.
            info("shim injection failed, trying native role-switch fallback");
            let vbus_file = find_vbus_path(&vbus_candidates());
            let initial = read_vbus(&vbus_file);
            info(&format!("fallback initial VBUS: {initial}"));
            if initial == "1" {
                info("PC detected, staying in DEVICE mode (no host force)");
                native_done = true;
            } else {
                match native_activate() {
                    Ok(()) => native_done = true,
                    Err(e) => {
                        let _ = set_prop("sys.usb.patch_dwc3", "0");
                        return Err(format!("shim and native host paths both failed: {e}"));
                    }
                }
            }
        }
    }

    if native_done {
        set_prop("sys.usb.patch_dwc3", "1").map_err(|e| e.to_string())?;
        info("host_ready ACTIVE via native path, sys.usb.patch_dwc3=1");
    } else if Path::new(PROC_SHIM).exists() {
        let ready = std::fs::read_to_string(PROC_READY)
            .unwrap_or_default()
            .trim()
            .to_string();
        if ready != "1" {
            info("activating host_ready via /proc/otg_host_shim");
            // Mirror `echo 1 >`: newline-terminated, like every sysfs write here.
            let _ = std::fs::write(PROC_SHIM, b"1\n");
        }
        let ready = std::fs::read_to_string(PROC_READY)
            .unwrap_or_default()
            .trim()
            .to_string();
        if ready == "1" {
            set_prop("sys.usb.patch_dwc3", "1").map_err(|e| e.to_string())?;
            info("host_ready ACTIVE, sys.usb.patch_dwc3=1");
        } else {
            let _ = set_prop("sys.usb.patch_dwc3", "0");
            return Err("host_ready NOT active after activation attempt".into());
        }
    } else {
        let _ = set_prop("sys.usb.patch_dwc3", "0");
        return Err("proc interfaces missing after injection".into());
    }

    match patch_max77759_i2c_with_driver(&tcpc_driver()) {
        Ok(()) => info("USB data path switches connected"),
        Err(e) => err(&format!("TCPC switch not configured: {e}")),
    }
    info("OTG patch routine finished");
    Ok(())
}

// --- otg-auto daemon ---

const CHARGER_VALUE: &str = "/sys/kernel/debug/gvotables/CHARGER_MODE/force_int_value";
const CHARGER_ACTIVE: &str = "/sys/kernel/debug/gvotables/CHARGER_MODE/force_int_active";
const UDC_FILE: &str = "/config/usb_gadget/g1/UDC";
const UDC_NAME: &str = "11210000.dwc3";

fn device_cfg() -> crate::config::DeviceConfig {
    load_device_config(&get_prop("ro.hardware")).unwrap_or_default()
}

fn tcpc_driver() -> String {
    let d = device_cfg().tcpc_driver;
    if d.is_empty() {
        "max77759tcpc".to_string()
    } else {
        d
    }
}

fn vbus_candidates() -> Vec<String> {
    let v = device_cfg().vbus_paths;
    if v.is_empty() {
        crate::config::default_vbus_paths()
    } else {
        v
    }
}

fn find_vbus_path(cands: &[String]) -> Option<String> {
    cands
        .iter()
        .find(|p| Path::new(p).exists())
        .map(|s| s.to_string())
}

fn read_vbus(path: &Option<String>) -> String {
    match path {
        Some(p) => std::fs::read_to_string(p)
            .unwrap_or_default()
            .trim()
            .to_string(),
        None => "0".into(),
    }
}

fn switch_to_host(current: &mut String) {
    if current.as_str() == "host" {
        return;
    }
    info(">>> SWITCHING TO HOST MODE <<<");
    // All writes mirror `echo ... >` (newline-terminated). The "\n" on the
    // UDC detach is load-bearing: a 0-byte write never reaches the configfs
    // .store callback, leaving the controller stuck in peripheral mode.
    let _ = std::fs::write(UDC_FILE, b"\n");
    let _ = std::fs::write(CHARGER_VALUE, b"49\n");
    let _ = std::fs::write(CHARGER_ACTIVE, b"1\n");
    // Exynos OTG-ID register; absent on laguna/malibu (skipped, the role
    // switch above already steered the controller).
    if let Some(otg_id) = find_otg_id() {
        let _ = std::fs::write(otg_id, b"0\n");
    }
    *current = "host".into();
}

fn switch_to_device(current: &mut String) {
    if current.as_str() == "device" {
        return;
    }
    info(">>> SWITCHING TO DEVICE MODE <<<");
    let _ = std::fs::write(CHARGER_ACTIVE, b"0\n");
    // Exynos OTG-ID register; absent on laguna/malibu (skipped).
    if let Some(otg_id) = find_otg_id() {
        let _ = std::fs::write(otg_id, b"1\n");
    }
    // Single write (shell version wrote it twice).
    let _ = set_prop("sys.usb.ffs.ready", "1");
    // Self-healing rebind: init triggers are edge-based, so a missed edge
    // (spurious host switch at boot, ffs.ready already 1) leaves UDC
    // unbound forever with no further property change to re-fire the
    // usb.rc bind trigger. Writing the controller explicitly re-binds the
    // gadget regardless of edge state; starting adbd is a no-op when it
    // already runs and covers the never-started race.
    let controller = resolve_udc();
    info(&format!("device mode: binding UDC {controller}"));
    match std::fs::write(UDC_FILE, format!("{controller}\n").as_bytes()) {
        Ok(_) => {
            // Cosmetic consistency: init's rc setprop used the blanked
            // literal on unswapped trees. Nothing re-sets it afterwards
            // (the per-second trigger only rewrites UDC + state), so a
            // value set here sticks for getprop readers.
            let _ = set_prop("sys.usb.controller", &controller);
        }
        Err(e) => info(&format!("device mode: UDC bind FAILED: {e}")),
    }
    let _ = set_prop("ctl.start", "adbd");
    *current = "device".into();
}

/// Resolve the UDC name to bind. sysfs is hardware ground truth: the
/// sys.usb.controller prop wins only when it names a real sysfs UDC
/// (properly swapped tree). On an unswapped tree the prop holds the
/// blanked "UNKNOWN.dwc3" literal (or garbage) and must NOT be written
/// to g1/UDC — the bind would fail and the gadget stays dead. Pure over
/// inputs (testable); sysfs/prop reads live in the caller.
fn pick_controller(prop: &str, sysfs: &[String]) -> String {
    let p = prop.trim();
    if !p.is_empty() && !p.contains("UNKNOWN") && sysfs.iter().any(|e| e == p) {
        return p.to_string();
    }
    if let Some(first) = sysfs.first() {
        return first.clone();
    }
    if !p.is_empty() && !p.contains("UNKNOWN") {
        return p.to_string();
    }
    UDC_NAME.to_string()
}

/// Live UDC resolution: sorted sysfs entries + current prop value.
fn resolve_udc() -> String {
    let mut sysfs: Vec<String> = std::fs::read_dir("/sys/class/udc")
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| !n.is_empty())
                .collect()
        })
        .unwrap_or_default();
    sysfs.sort();
    pick_controller(&get_prop("sys.usb.controller"), &sysfs)
}

/// usb-rebind (oneshot, service usb_rebind): role-aware UDC bind retry.
///
/// The init `write g1/UDC` triggers fire on property edges
/// (sys.usb.config/configfs/ffs.ready) with no notion of controller role.
/// On google-usb-role-sw SoCs (laguna/malibu) the DWC3 refuses gadget start
/// with -ENODEV (-19) while the role voter still sits at `none`, and the
/// failed bind is never retried — adbd then loops on FUNCTIONFS_BIND while
/// MTP can't open its bulk endpoint. This service polls the role switch
/// (when present) and re-attempts the bind for ~30s; the long-running
/// otg-auto daemon owns later plug transitions via its VBUS poll.
pub fn run_usb_rebind() -> Result<(), String> {
    info("usb-rebind: waiting for device role, then binding UDC");
    for i in 0..30 {
        let role = find_usb_role()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        if role == "host" {
            info("usb-rebind: role=host, leaving controller to host mode");
            sleep(Duration::from_secs(2));
            continue;
        }
        let controller = resolve_udc();
        match std::fs::write(UDC_FILE, format!("{controller}\n").as_bytes()) {
            Ok(_) => {
                info(&format!(
                    "usb-rebind: UDC bound to {controller} (attempt {i}, role={})",
                    if role.is_empty() { "unknown" } else { &role }
                ));
                let _ = set_prop("sys.usb.controller", &controller);
                let _ = set_prop("ctl.start", "adbd");
                return Ok(());
            }
            Err(e) => {
                if i % 5 == 0 {
                    info(&format!("usb-rebind: bind {controller} failed (attempt {i}): {e}"));
                }
            }
        }
        sleep(Duration::from_secs(1));
    }
    Err("usb-rebind: UDC still unbound after 30s".into())
}

/// otg-auto: default HOST, PC detected by external VBUS -> DEVICE. Never exits.
pub fn run_otg_auto() -> ! {
    info("starting (VBUS detection, default HOST)");
    // NOTE: intentionally no `set_prop(patch_dwc3, 0)` here — the flag belongs
    // to otg-patch and gates our own `on property` trigger.
    let vbus_file = find_vbus_path(&vbus_candidates());
    match &vbus_file {
        Some(p) => info(&format!("using VBUS path: {p}")),
        None => err("no VBUS sysfs path found, assuming 0"),
    }
    info("waiting 3s for boot to settle");
    sleep(Duration::from_secs(3));

    // Debounce + settle: VBUS sensing bounces on plug/unplug (contact
    // bounce, TCPC renegotiation) and our own host-side boost can reflect
    // back into the sensor. Without this the daemon flaps host<->device
    // every second, tearing down enumeration mid-transfer. A switch needs
    // DEBOUNCE_READS identical polls and a quiet window after the previous
    // switch.
    const DEBOUNCE_READS: u32 = 3;
    const SETTLE_SECS: u64 = 5;
    // The initial read is debounced too: a single poll can catch a
    // PD-negotiation dip with the cable plugged in, causing a spurious
    // host switch (UDC unbind) at boot with adb dead until replug.
    // Poll up to ~8s for a stable value; fall back to the last read.
    let mut initial = read_vbus(&vbus_file);
    let mut initial_stable: u32 = 0;
    for _ in 0..8 {
        sleep(Duration::from_secs(1));
        let vbus = read_vbus(&vbus_file);
        if vbus == initial {
            initial_stable += 1;
            if initial_stable >= DEBOUNCE_READS {
                break;
            }
        } else {
            initial = vbus;
            initial_stable = 0;
        }
    }
    info(&format!("initial VBUS (stable): {initial}"));
    let mut current = String::new();
    if initial == "1" {
        info("PC detected at boot (VBUS=1), starting in DEVICE mode");
        switch_to_device(&mut current);
    } else {
        info("no PC at boot (VBUS=0), starting in HOST mode");
        switch_to_host(&mut current);
    }
    // Keep UDC name available for debugging parity with the shell version.
    let _ = UDC_NAME;
    let mut prev = initial.clone();
    let mut last_raw = initial;
    let mut stable: u32 = 0;
    let mut cooldown: u64;
    // Monotonic-ish clock via /proc/uptime (no Instant persistence issues
    // across the infinite loop, std only).
    let uptime_secs = || {
        std::fs::read_to_string("/proc/uptime")
            .ok()
            .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok())
            .unwrap_or(0.0) as u64
    };
    // The boot switch above just ran: let hardware settle before listening.
    cooldown = uptime_secs() + SETTLE_SECS;    loop {
        let vbus = read_vbus(&vbus_file);
        if vbus != last_raw {
            last_raw = vbus.clone();
            stable = 0;
        } else if stable < DEBOUNCE_READS {
            stable += 1;
        }
        if stable >= DEBOUNCE_READS && vbus != prev {
            if uptime_secs() < cooldown {
                info(&format!("VBUS change {prev} -> {vbus} ignored (settle window)"));
            } else {
                info(&format!("VBUS change (stable): {prev} -> {vbus}"));
                if vbus == "1" {
                    switch_to_device(&mut current);
                } else {
                    switch_to_host(&mut current);
                }
                prev = vbus.clone();
                cooldown = uptime_secs() + SETTLE_SECS;
            }
        }
        // ffs.ready is polled for parity with shell dump_state logging.
        let _ = get_prop("sys.usb.ffs.ready");
        sleep(Duration::from_secs(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn udc_prefers_valid_prop() {
        let sys = vec!["11210000.dwc3".to_string()];
        // Swapped tree: prop names the real UDC -> prop wins.
        assert_eq!(pick_controller("11210000.dwc3", &sys), "11210000.dwc3");
        assert_eq!(pick_controller("11210000.dwc3\n", &sys), "11210000.dwc3");
    }

    #[test]
    fn udc_rejects_blanked_literal() {
        let sys = vec!["11210000.dwc3".to_string()];
        // Unswapped tree: blanked literal must never reach g1/UDC.
        assert_eq!(pick_controller("UNKNOWN.dwc3", &sys), "11210000.dwc3");
        assert_eq!(pick_controller("", &sys), "11210000.dwc3");
        assert_eq!(pick_controller("garbage.dwc3", &sys), "11210000.dwc3");
    }

    #[test]
    fn udc_no_sysfs_falls_back() {
        let empty: Vec<String> = Vec::new();
        // No sysfs: valid prop still usable, blanked -> compiled default.
        assert_eq!(pick_controller("c400000.dwc3", &empty), "c400000.dwc3");
        assert_eq!(pick_controller("UNKNOWN.dwc3", &empty), UDC_NAME);
        assert_eq!(pick_controller("", &empty), UDC_NAME);
    }
}
