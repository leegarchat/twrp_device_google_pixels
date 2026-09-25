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
const TYPEC_CLASS: &str = "/sys/class/typec";
const PSY_CLASS: &str = "/sys/class/power_supply";
const LAGUNA_HOST_ROLE_REQUESTS: &[(&str, &str)] = &[
    ("preferred_role", "source"),
    ("port_type", "source"),
];
const LAGUNA_DEVICE_ROLE_REQUESTS: &[(&str, &str)] = &[
    ("port_type", "dual"),
    ("preferred_role", "sink"),
];
/// gvotable CHARGER_MODE force values driving VBUS in host mode (6.1 trick).
/// ABSENT on laguna 6.6 stock kernels (verified: zero gvotables/CHARGER_MODE
/// strings in mustang bp4a 6.6.98 + cp2a 6.6.118 images) — laguna must use
/// the TCPM typec path below instead. Still valid where the framework
/// exists; writes are now result-checked and logged.
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

/// gs101 fallback detector (pure inputs, testable).
///
/// Post-8.0 USB churn (unconditional OTG_ID register writes, usb-rebind
/// service) regressed raven/oriole/bluejay: stuck vibration + frozen
/// touch on decrypt screens, dead adb — while shiba/laguna/malibu need
/// the new logic. The fallback keeps: no usb-rebind service on gs101,
/// and OTG_ID writes only as settled, conditional transitions (host
/// assert on VBUS=0 entry, device restore on return — never blindly at
/// boot, so a live ADB session cannot be killed). Everything else
/// (shim, otg-auto, TCPC) is untouched on all families.
fn is_gs101_family(soc_family: &str, hardware: &str) -> bool {
    soc_family.trim() == "gs101"
        || matches!(hardware.trim(), "oriole" | "raven" | "bluejay")
}

fn is_gs101() -> bool {
    is_gs101_family(&get_prop("ro.recovery.soc_family"), &get_prop("ro.hardware"))
}

/// laguna family detector (pure inputs, testable).
///
/// OTG on laguna (Tensor G5: frankel/blazer/mustang/rango) is owned
/// end-to-end by TCPM: TCPCI vote -> goog_usb_role_sw glue -> downstream
/// role-sw-dev + _dwc3_google_set_role + eusb2 repeater regulators + VBUS
/// via the max77779 charger OTG usecase. Writing the downstream
/// c450000.usb3-role-switch directly is a hardware no-op (field-proven:
/// sysfs reads back "host" while _dwc3_google_set_role host never fires),
/// and the gvotable CHARGER_MODE VBUS force does not exist on laguna 6.6
/// kernels — so laguna drives host mode through TCPM's preferred_role/
/// port_type controls instead of power_role. Family-level: all four devices share
/// the max77759 SPMI TCPC + glue topology (stock DT).
fn is_laguna_family(soc_family: &str, hardware: &str) -> bool {
    soc_family.trim() == "laguna"
        || matches!(
            hardware.trim(),
            "frankel" | "blazer" | "mustang" | "rango"
        )
}

fn is_laguna() -> bool {
    is_laguna_family(&get_prop("ro.recovery.soc_family"), &get_prop("ro.hardware"))
}

/// Prefer port0, else the first port*. Pure over inputs (testable); the
/// live readdir wrapper is find_typec_port().
fn pick_typec_port(entries: &[String]) -> Option<String> {
    if entries.iter().any(|e| e == "port0") {
        return Some("port0".to_string());
    }
    entries.iter().find(|e| e.starts_with("port")).cloned()
}

/// Live Type-C port dir (e.g. /sys/class/typec/port0).
fn find_typec_port() -> Option<PathBuf> {
    let names: Vec<String> = std::fs::read_dir(TYPEC_CLASS)
        .ok()?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    pick_typec_port(&names).map(|n| Path::new(TYPEC_CLASS).join(n))
}

/// First tcpm-source-psy-*/online node. Pure over inputs (testable);
/// live wrapper is find_source_psy_online().
fn pick_source_psy(entries: &[String]) -> Option<String> {
    entries
        .iter()
        .find(|e| e.starts_with("tcpm-source-psy"))
        .cloned()
}

fn find_source_psy_online() -> Option<PathBuf> {
    let names: Vec<String> = std::fs::read_dir(PSY_CLASS)
        .ok()?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    pick_source_psy(&names).map(|n| Path::new(PSY_CLASS).join(n).join("online"))
}

fn read_trim(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_default().trim().to_string()
}

/// The stock Laguna kernel rejects `power_role` writes when the Type-C
/// port has no `pr_set` callback. Request the unattached DRP preference
/// (`preferred_role`/`try_role`) and set `port_type`; both are standard TCPM
/// controls with independent kernel callbacks.
fn laguna_role_requests(host: bool) -> &'static [(&'static str, &'static str)] {
    if host {
        LAGUNA_HOST_ROLE_REQUESTS
    } else {
        LAGUNA_DEVICE_ROLE_REQUESTS
    }
}

fn write_typec_role_attr(port: &Path, attribute: &str, value: &str) -> bool {
    let path = port.join(attribute);
    if !path.exists() {
        info(&format!("typec {attribute} unavailable"));
        return false;
    }
    match std::fs::write(&path, format!("{value}\n").as_bytes()) {
        Ok(()) => {
            info(&format!("typec {attribute} <- {value}: {}", read_trim(&path)));
            true
        }
        Err(e) => {
            info(&format!("typec {attribute}={value} write FAILED: {e}"));
            false
        }
    }
}

fn drive_laguna_role(port: &Path, host: bool) -> bool {
    let mut accepted = false;
    for &(attribute, value) in laguna_role_requests(host) {
        accepted |= write_typec_role_attr(port, attribute, value);
    }
    accepted
}

/// Pure gs101 OTG_ID transition decision (testable).
/// Entering host (settled VBUS=0) always asserts 0; leaving to device
/// restores 1 ONLY when coming from a host session we asserted ourselves
/// (prev == "host"). The initial boot-device call (prev empty/device)
/// never writes, so the proven device/ADB bring-up is untouchable.
fn gs101_otg_id_value(host: bool, prev_mode: &str) -> Option<&'static str> {
    if host {
        Some("0")
    } else if prev_mode == "host" {
        Some("1")
    } else {
        None
    }
}
/// (e.g. /sys/devices/platform/11210000.usb/dwc3_exynos_otg_id).
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

/// Native host path dispatcher: laguna goes through TCPM Type-C role controls,
/// everything else keeps the role-switch + gvotable path.
fn native_activate() -> Result<(), String> {
    if is_laguna() {
        return native_activate_laguna();
    }
    native_activate_legacy()
}

/// Laguna host path via TCPM's preferred-role/port-type API.
///
/// preferred_role=source / port_type=source lets TCPM source VBUS
/// (max77779 charger OTG usecase), vote host into goog_usb_role_sw (downstream role-sw-dev,
/// _dwc3_google_set_role, eusb2 repeater regulators + mux). The 6.6
/// power_role attribute is read-only when the port has no pr_set callback;
/// preferred_role uses try_role and port_type uses the TCPM port_type_set
/// callback instead. Success is
/// verified on hardware state — data_role reads host AND the TCPM source
/// psy reports online — never on the request alone. No Type-C port or
/// writable request fails loudly rather than reporting a false host state.
fn native_activate_laguna() -> Result<(), String> {
    let port = match find_typec_port() {
        Some(p) => p,
        None => {
            return Err("laguna: no /sys/class/typec port".into());
        }
    };
    info(&format!("laguna: typec port {}", port.display()));
    if !drive_laguna_role(&port, true) {
        return Err("laguna: kernel rejected preferred_role and port_type host requests".into());
    }
    for i in 0..10 {
        let data = read_trim(&port.join("data_role"));
        let opmode = read_trim(&port.join("power_operation_mode"));
        let src = find_source_psy_online()
            .map(|p| read_trim(&p))
            .unwrap_or_default();
        info(&format!(
            "laguna: poll {i} data_role={data} opmode={opmode} source_psy={src}"
        ));
        if data == "host" && src == "1" {
            info("laguna host path ACTIVE (data_role=host, VBUS sourced)");
            return Ok(());
        }
        sleep(Duration::from_secs(1));
    }
    Err("laguna: host not confirmed (data_role/source psy)".into())
}

/// Legacy native host path (role switch to host + gvotable VBUS force), used
/// for non-Laguna families on kernels where the shim is unavailable.
fn native_activate_legacy() -> Result<(), String> {
    let role = find_usb_role().ok_or_else(|| "no usb_role switch found".to_string())?;
    info("switching controller role to host");
    let _ = std::fs::write(&role, b"host\n");
    sleep(Duration::from_secs(2));
    info("forcing VBUS via gvotable CHARGER_MODE");
    if let Err(e) = std::fs::write(CHARGER_VALUE, CHARGER_FORCE_VALUE) {
        info(&format!("gvotable value write FAILED (framework absent?): {e}"));
    }
    if let Err(e) = std::fs::write(CHARGER_ACTIVE, CHARGER_FORCE_ACTIVE) {
        info(&format!("gvotable active write FAILED (framework absent?): {e}"));
    }
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
    // VBUS boost vote (max77759 OTG usecase via CHARGER_MODE). Checked here
    // (unlike the old fire-and-forget): without sourced VBUS the accessory
    // stays dark even with the controller in host mode. Absence is LOUD —
    // it means this kernel needs a different boost path.
    let mut vbus_ok = true;
    if let Err(e) = std::fs::write(CHARGER_VALUE, b"49\n") {
        info(&format!("host: gvotable value write FAILED: {e} (no VBUS force?)"));
        vbus_ok = false;
    }
    if let Err(e) = std::fs::write(CHARGER_ACTIVE, b"1\n") {
        info(&format!("host: gvotable active write FAILED: {e} (no VBUS force?)"));
        vbus_ok = false;
    }
    if vbus_ok {
        info("host: VBUS force votes written");
    }
    // Laguna: request host through Type-C policy instead of the downstream
    // role switch/gvotable path, which does not source VBUS on 6.6.
    if is_laguna() {
        if let Some(port) = find_typec_port() {
            if !drive_laguna_role(&port, true) {
                info("laguna: no writable Type-C host control");
            }
        } else {
            info("laguna: no typec port for host drive");
        }
    }
    // Exynos OTG-ID register (host_on); absent on laguna/malibu (skipped,
    // the role switch above already steered the controller).
    // gs101: write host (0) ONLY on the settled VBUS=0 transition into host
    // mode — never blindly. The shim sets host_ready, but without host_on
    // the DWC3 never leaves peripheral mode (field: host switch with no
    // enumeration, otg_on=0 forever). Writing with a PC attached would kill
    // adb, which cannot happen here: this branch runs only at VBUS=0.
    if is_gs101() {
        // gs101_otg_id_value(true, _) is always Some("0"); the option
        // keeps the node-missing branch explicit and logged.
        if let Some(v) = gs101_otg_id_value(true, current.as_str()) {
            match find_otg_id() {
                Some(otg_id) => match std::fs::write(&otg_id, format!("{v}\n").as_bytes()) {
                    Ok(()) => info(&format!("gs101: host_on asserted via {}", otg_id.display())),
                    Err(e) => info(&format!("gs101: OTG_ID host write FAILED: {e}")),
                },
                None => info("gs101: no dwc3_exynos_otg_id node, host_on unavailable"),
            }
        }
    } else if let Some(otg_id) = find_otg_id() {
        let _ = std::fs::write(otg_id, b"0\n");
    }
    *current = "host".into();
    if is_gs101() {
        confirm_gs101_host();
    }
}

/// gs101 host confirmation: the TCPM source psy reports online=1 only
/// while WE source VBUS. Poll 10s; a negative verdict names the missing
/// half (controller role vs VBUS power) instead of claiming host.
fn confirm_gs101_host() {
    for i in 0..10 {
        let src = find_source_psy_online()
            .map(|p| read_trim(&p))
            .unwrap_or_default();
        if src == "1" {
            info("gs101 HOST ACTIVE (TCPM source psy online, VBUS sourced)");
            return;
        }
        if i % 3 == 0 {
            info(&format!("gs101 host poll {i}: source_psy={src} (want 1)"));
        }
        sleep(Duration::from_secs(1));
    }
    info("gs101 host NOT confirmed (source_psy never online: no VBUS and/or no role switch)");
}

fn switch_to_device(current: &mut String) {
    if current.as_str() == "device" {
        return;
    }
    info(">>> SWITCHING TO DEVICE MODE <<<");
    let _ = std::fs::write(CHARGER_ACTIVE, b"0\n");
    // Laguna: restore DRP mode and prefer sink so TCPM drops source/VBUS and
    // votes device into the USB role-switch glue.
    if is_laguna() {
        if let Some(port) = find_typec_port() {
            if !drive_laguna_role(&port, false) {
                info("laguna: no writable Type-C device control");
            }
        } else {
            info("laguna: no typec port for device drive");
        }
    }
    // Exynos OTG-ID register; absent on laguna/malibu (skipped).
    // gs101: restore device (1) ONLY when returning from a host session we
    // asserted ourselves (previous state == host). The initial boot-device
    // call (previous state empty) keeps the proven no-write path, so a
    // register write can never break the working device/ADB bring-up; it
    // only ever undoes our own host assertion above.
    if is_gs101() {
        match gs101_otg_id_value(false, current.as_str()) {
            Some(v) => match find_otg_id() {
                Some(otg_id) => match std::fs::write(&otg_id, format!("{v}\n").as_bytes()) {
                    Ok(()) => info(&format!("gs101: device restored via {}", otg_id.display())),
                    Err(e) => info(&format!("gs101: OTG_ID device write FAILED: {e}")),
                },
                None => info("gs101: no dwc3_exynos_otg_id node for device restore"),
            },
            None => info("gs101 fallback: OTG_ID write skipped (initial device path)"),
        }
    } else if let Some(otg_id) = find_otg_id() {
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
    // gs101 fallback: no rebind service ran in 8.0 and init-trigger binds
    // succeed on exynos (field-proven: UDC ends bound); the 30s write loop
    // only churns UDC/adbd on Pixel 6. Other families unaffected.
    if is_gs101() {
        info("gs101 fallback: usb-rebind skipped (init triggers own the bind)");
        return Ok(());
    }
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
        // Already bound (by an init trigger or otg-auto's rebind)? Then
        // there is nothing to do — writing the same UDC again just fails
        // EBUSY and spams the log for the rest of the 30 attempts.
        let current = std::fs::read_to_string(UDC_FILE).unwrap_or_default();
        if current.trim() == controller {
            info(&format!("usb-rebind: UDC already bound to {controller}, done"));
            return Ok(());
        }
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

    #[test]
    fn gs101_detected_by_family_or_codename() {        // soc_family prop is primary (stamped by our early-init).
        assert!(is_gs101_family("gs101", ""));
        assert!(is_gs101_family("gs101\n", "raven"));
        // ro.hardware fallback covers trees where the prop is missing.
        assert!(is_gs101_family("", "oriole"));
        assert!(is_gs101_family("", "raven"));
        assert!(is_gs101_family("", "bluejay"));
        // Other families stay on the new logic.
        assert!(!is_gs101_family("zuma", "shiba"));
        assert!(!is_gs101_family("zumapro", "komodo"));
        assert!(!is_gs101_family("laguna", "mustang"));
        assert!(!is_gs101_family("malibu", "grizzly"));
        assert!(!is_gs101_family("gs201", "cheetah"));
        assert!(!is_gs101_family("", ""));
        assert!(!is_gs101_family("UNKNOWN", "UNKNOWN.dwc3"));
    }

    #[test]
    fn gs101_otg_id_transition_matrix() {
        // Host entry always asserts 0 (settled VBUS=0 only — no live ADB to kill).
        assert_eq!(gs101_otg_id_value(true, ""), Some("0"));
        assert_eq!(gs101_otg_id_value(true, "device"), Some("0"));
        assert_eq!(gs101_otg_id_value(true, "host"), Some("0"));
        // Device restore only undoes our own host assertion.
        assert_eq!(gs101_otg_id_value(false, "host"), Some("1"));
        // Initial boot-device path never writes (proven device/ADB bring-up).
        assert_eq!(gs101_otg_id_value(false, ""), None);
        assert_eq!(gs101_otg_id_value(false, "device"), None);
    }

    #[test]
    fn laguna_detected_by_family_or_codename() {
        // All four Tensor G5 devices share the TCPC+glue topology.
        assert!(is_laguna_family("laguna", ""));
        assert!(is_laguna_family("laguna\n", "mustang"));
        assert!(is_laguna_family("", "frankel"));
        assert!(is_laguna_family("", "blazer"));
        assert!(is_laguna_family("", "mustang"));
        assert!(is_laguna_family("", "rango"));
        // Other families keep their own paths (incl. malibu 6.12 native).
        assert!(!is_laguna_family("zuma", "shiba"));
        assert!(!is_laguna_family("zumapro", "komodo"));
        assert!(!is_laguna_family("malibu", "grizzly"));
        assert!(!is_laguna_family("gs101", "raven"));
        assert!(!is_laguna_family("gs201", "cheetah"));
        assert!(!is_laguna_family("", ""));
    }

    #[test]
    fn typec_port_prefers_port0() {
        let two = vec!["port1".to_string(), "port0".to_string()];
        assert_eq!(pick_typec_port(&two), Some("port0".to_string()));
        let one = vec!["port1".to_string()];
        assert_eq!(pick_typec_port(&one), Some("port1".to_string()));
        let empty: Vec<String> = Vec::new();
        assert_eq!(pick_typec_port(&empty), None);
        // Non-port entries never match.
        let junk = vec!["power".to_string(), "portX".to_string()];
        assert_eq!(pick_typec_port(&junk), Some("portX".to_string()));
    }

    #[test]
    fn source_psy_picks_tcpm_node() {
        let entries = vec![
            "battery".to_string(),
            "usb".to_string(),
            "tcpm-source-psy-spmi-max77759tcpc".to_string(),
            "main-charger".to_string(),
        ];
        assert_eq!(
            pick_source_psy(&entries),
            Some("tcpm-source-psy-spmi-max77759tcpc".to_string())
        );
        let empty: Vec<String> = Vec::new();
        assert_eq!(pick_source_psy(&empty), None);
    }

    #[test]
    fn laguna_host_uses_kernel_typec_requests() {
        assert_eq!(
            laguna_role_requests(true),
            &[("preferred_role", "source"), ("port_type", "source")]
        );
    }

    #[test]
    fn laguna_device_restores_drp_and_prefers_sink() {
        assert_eq!(
            laguna_role_requests(false),
            &[("port_type", "dual"), ("preferred_role", "sink")]
        );
    }
}
