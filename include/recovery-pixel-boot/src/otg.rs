//! otg — OTG host shim injection + VBUS auto-switch daemon.
//!
//! Port of otg_patch.sh (oneshot, service otg_enable) and otg_auto_v3.sh
//! (long-running, service otg_auto).
//!
//! Deliberate deviations from the shell versions (reviewed):
//! - otg-auto does NOT touch sys.usb.patch_dwc3 (was reset to 0 at startup).
//! - switch_to_device sets sys.usb.ffs.ready exactly once (was duplicated).

use crate::i2c::patch_max77759_i2c;
use crate::ko_picker::{ko_try_load, log_msg};
use crate::props::{get_prop, set_prop};
use std::path::Path;
use std::thread::sleep;
use std::time::Duration;

const TAG: &str = "otg";
const PROC_SHIM: &str = "/proc/otg_host_shim";
const PROC_READY: &str = "/proc/otg_host_ready";

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

    if Path::new(PROC_SHIM).exists() {
        info("module already loaded (/proc/otg_host_shim exists)");
    } else {
        info("/proc/otg_host_shim not found, injecting module");
        if !ko_try_load("otg_host_shim", None, "otg_patch") {
            let _ = set_prop("sys.usb.patch_dwc3", "0");
            return Err("module injection failed".into());
        }
    }

    if Path::new(PROC_SHIM).exists() {
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

    match patch_max77759_i2c() {
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
const VBUS_PATHS: &[&str] = &[
    "/sys/class/power_supply/usb/online",
    "/sys/class/power_supply/usb/present",
    "/sys/class/power_supply/usb-charger/online",
];

fn find_vbus_path() -> Option<String> {
    VBUS_PATHS
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
    let vbus_file = find_vbus_path();
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
    let mut prev = initial;
    loop {
        let vbus = read_vbus(&vbus_file);
        if vbus != prev {
            info(&format!("VBUS change: {prev} -> {vbus}"));
            if vbus == "1" {
                switch_to_device(&mut current);
            } else {
                switch_to_host(&mut current);
            }
            prev = vbus;
        }
        // ffs.ready is polled for parity with shell dump_state logging.
        let _ = get_prop("sys.usb.ffs.ready");
        sleep(Duration::from_secs(1));
    }
}
