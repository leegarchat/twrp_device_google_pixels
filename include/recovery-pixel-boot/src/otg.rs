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
            let _ = set_prop("sys.usb.patch_dwc3", "0");
            return Err("module injection failed".into());
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

const OTG_ID: &str = "/sys/devices/platform/11210000.usb/dwc3_exynos_otg_id";
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
    let _ = std::fs::write(OTG_ID, b"0\n");
    *current = "host".into();
}

fn switch_to_device(current: &mut String) {
    if current.as_str() == "device" {
        return;
    }
    info(">>> SWITCHING TO DEVICE MODE <<<");
    let _ = std::fs::write(CHARGER_ACTIVE, b"0\n");
    let _ = std::fs::write(OTG_ID, b"1\n");
    // Single write (shell version wrote it twice).
    let _ = set_prop("sys.usb.ffs.ready", "1");
    *current = "device".into();
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

    let initial = read_vbus(&vbus_file);
    info(&format!("initial VBUS: {initial}"));
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
    // Debounce + settle: VBUS sensing bounces on plug/unplug (contact
    // bounce, TCPC renegotiation) and our own host-side boost can reflect
    // back into the sensor. Without this the daemon flaps host<->device
    // every second, tearing down enumeration mid-transfer. A switch needs
    // DEBOUNCE_READS identical polls and a quiet window after the previous
    // switch.
    const DEBOUNCE_READS: u32 = 3;
    const SETTLE_SECS: u64 = 5;
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
