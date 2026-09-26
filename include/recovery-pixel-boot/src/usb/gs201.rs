//! usb::gs201 — TEST-branch OTG implementation (R12_14.1_otg_test).
//!
//! Tensor G2 family: cheetah (Pixel 7 Pro), panther (Pixel 7), lynx
//! (Pixel 7a), felix (Pixel Fold), tangorpro (Pixel Tablet). Mirrors
//! [`super::zuma`] (the proven reference) with gs201-correct constants.
//!
//! Source analysis (branch `android-gs-pantah-6.1-android16`, HEADs
//! `ab/13436313` across pantah/lynx/felix/tangorpro/gs201_base/soc_gs):
//! - Glue: `soc_gs/drivers/usb/dwc3/dwc3-exynos-otg.c` carries the AOC
//!   gate (`host_ready && host_on`, otg.c:128) and
//!   `EXPORT_SYMBOL_GPL(dwc3_otg_host_ready)` (otg.c:537); the OTG_ID
//!   node `dwc3_exynos_otg_id` with `0`=host / `1`=device store semantics
//!   (`dwc3_exynos_host_event(dev, !id)`) lives in
//!   `soc_gs/drivers/usb/dwc3/dwc3-exynos.c:986-1035`. The AOC caller is
//!   `aoc/usb/aoc_usb_dev.c:385,395` (absent in recovery) → our kprobe
//!   shim replaces it; force-load covers vermagic drift, same as zuma.
//! - NO native role-switch path by design: the 6.1 glue has zero
//!   `usb_role_switch` refs (only the `enum usb_role`/`usb_role_string`
//!   helpers). The one role switch on this platform is registered by the
//!   TCPC (`tcpci_max77759.c:3215`) and its set callback only drives
//!   extcon/data-path + power_supply — never dwc3 host — so writing it
//!   would report a false success. Shim path only.
//! - DT (`gs201_base/dts/gs201.dtsi:722-729`): `usb@11210000`
//!   (`compatible = "samsung,exynos9-dwusb"`, `reg = <0x0 0x11210000 ...
//!   with a `dwc3` child stuck in `dr_mode = "peripheral"` (:749-757):
//!   host comes ONLY via the glue. All four device repos
//!   (pantah/lynx/felix/tangorpro) symlink `dts/gs201 -> ../../gs201/dts`
//!   and their per-board `*-usb.dtsi` overlays only retune the PHY — one
//!   OTG_ID path covers every gs201 device.
//! - TCPC `max77759tcpc@25` on `hsi2c_13` (`compatible = "max77759tcpc"`,
//!   `reg = <0x25>`; `gs201-common-typec.dtsi:12-20`); i2c driver name
//!   `max77759tcpc` (`tcpci_max77759.c:3849`); USBSW
//!   `TCPC_VENDOR_USBSW_CTRL 0x93 <- USBSW_CONNECT 0x09`
//!   (`tcpci_max77759_vendor_reg.h:94-95`); `usb` power_supply
//!   (`usb_psy_desc.name = "usb"`, `usb_psy.c:562-563`, DT
//!   `usb-psy-name = "usb"`); `CHARGER_MODE` election name
//!   (`bms/google_bms.h:692`, consumed at `tcpci_max77759.c:66,1206+`).
//! - PHY is `phy-exynos-usbdrd-super.ko` (`CONFIG_PHY_EXYNOS_USBDRD=m`,
//!   `gs201_defconfig:93`; Kbuild builds the `eusb-super` variant only
//!   under `CONFIG_PHY_EXYNOS_USBDRD_EUSB`, which gs201 does NOT set;
//!   the glue itself declares `MODULE_SOFTDEP("pre:
//!   phy-exynos-usbdrd-super")`, `dwc3-exynos.c:1523`). Stock ramdisk
//!   order in `gs201_base/vendor_ramdisk.modules.gs201`.
//!
//! Deliberately lean (R11 algorithm, like zuma): UDC detach with `"\n"`
//! (0-byte writes never reach configfs .store), ffs.ready written once,
//! explicit UDC rebind on the device switch (init triggers are
//! edge-based; a missed edge leaves the gadget dead with no retry),
//! /dev/block/otg-usb stable symlink for /usb_otg.

use crate::i2c::patch_max77759_i2c_with_driver;
use crate::ko_picker::{
    detect_kernel_env, find_candidates, is_module_loaded, ko_try_load,
    load_kernel_module, load_kernel_module_force, log_msg,
};
use crate::props::{get_prop, set_prop};
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::Duration;

const TAG: &str = "otg";
const USB_ROLE_CLASS: &str = "/sys/class/usb_role";
const PROC_SHIM: &str = "/proc/otg_host_shim";
const PROC_READY: &str = "/proc/otg_host_ready";
/// Created at runtime by dwc3-exynos-usb.ko (absent = glue not loaded).
/// SoC base is `usb@11210000` (gs201.dtsi:722-729, shared by all gs201
/// boards via the dts/gs201 symlink); node name + 0=host/1=device store
/// semantics from dwc3-exynos.c:986-1035.
const OTG_ID: &str = "/sys/devices/platform/11210000.usb/dwc3_exynos_otg_id";
/// gvotable debugfs election `CHARGER_MODE` (name in bms/google_bms.h:692;
/// absent = charger stack missing).
const CHARGER_VALUE: &str = "/sys/kernel/debug/gvotables/CHARGER_MODE/force_int_value";
const CHARGER_ACTIVE: &str = "/sys/kernel/debug/gvotables/CHARGER_MODE/force_int_active";
const UDC_FILE: &str = "/config/usb_gadget/g1/UDC";
/// Created at runtime by usb_psy.ko (`usb_psy_desc.name = "usb"`,
/// usb_psy.c:562-563; DT `usb-psy-name = "usb"`).
const VBUS_PATHS: &[&str] = &[
    "/sys/class/power_supply/usb/online",
    "/sys/class/power_supply/usb/present",
];
const OTG_USB_LINK: &str = "/dev/block/otg-usb";

/// Stock USB chain in dep order (mirrors the zuma chain; the single
/// family delta is the PHY: gs201 builds `phy-exynos-usbdrd-super`, not
/// the `eusb-super` variant — see gs201_defconfig:93, phy Kbuild:8, the
/// glue's own `MODULE_SOFTDEP("pre: phy-exynos-usbdrd-super")`, and the
/// `vendor_ramdisk.modules.gs201` load order).
/// glue needs phy + gvotable (gvotable_* calls in dwc3-exynos-otg.c);
/// tcpc needs shim + usb_psy (`google_shim_tcpci_*` / `usb_psy_*` calls
/// in tcpci_max77759.c) + helper (max77759_* regmap helpers from
/// max77759_helper.c); charger stack needs tcpc.
/// Missing files are skipped LOUDLY (non-stock trees don't ship them).
const STOCK_USB_CHAIN: &[&str] = &[
    "gvotable",
    "usb_psy",
    "max77759_helper",
    "phy-exynos-usbdrd-super",
    "dwc3-exynos-usb",
    "google_tcpci_shim",
    "tcpci_max77759",
    "google-charger",
    "max77759-charger",
];
/// First-stage ramdisk first: on real firmware trees the whole Samsung USB
/// stack auto-loads from the vendor_kernel_boot ramdisk before we run, so
/// the chain below is normally a silent no-op safety net, not the prime
/// mover.
const STOCK_ROOTS: &[&str] = &[
    "/lib/modules",
    "/vendor_dlkm/lib/modules",
    "/vendor/lib/modules",
    "/system/lib64/modules",
];

fn info(msg: &str) {
    log_msg(TAG, "INFO", msg);
    println!("otg: INFO: {msg}");
}

fn read_trim(p: &str) -> String {
    std::fs::read_to_string(p).unwrap_or_default().trim().to_string()
}

/// Position of a module in the stock chain (test-only: verifies dep order).
#[cfg(test)]
fn chain_pos(name: &str) -> Option<usize> {
    STOCK_USB_CHAIN.iter().position(|m| *m == name)
}

/// Best-effort insmod of one STOCK module (strict flags: stock modules
/// always match the running kernel by construction). Returns true when
/// loaded or already loaded.
fn insmod_stock(name: &str) -> bool {
    if is_module_loaded(name) {
        return true;
    }
    for root in STOCK_ROOTS {
        let p = Path::new(root).join(format!("{name}.ko"));
        if !p.is_file() {
            continue;
        }
        match load_kernel_module(&p) {
            Ok(()) => {
                info(&format!("stock module loaded: {name}"));
                return true;
            }
            Err(e) => {
                info(&format!("stock module {name} FAILED: {e}"));
                return false;
            }
        }
    }
    info(&format!("stock module {name} not found (tree lacks it?)"));
    false
}

/// Load our shim: strict first (works when vermagic matches), then forced
/// (IGNORE_VERMAGIC/MODVERSIONS — our code only, see module docs).
fn load_shim() -> bool {
    if Path::new(PROC_SHIM).exists() {
        info("shim already loaded (/proc/otg_host_shim exists)");
        return true;
    }
    if ko_try_load("otg_host_shim", Some(PROC_SHIM), TAG) {
        return true;
    }
    info("strict shim load failed (vermagic?), trying forced load");
    let env = match detect_kernel_env() {
        Ok(e) => e,
        Err(e) => {
            info(&format!("detect_kernel_env failed: {e}"));
            return false;
        }
    };
    for cand in find_candidates(Path::new("/system/lib64/modules"), "otg_host_shim", &env) {
        info(&format!("force-trying {}", cand.display()));
        match load_kernel_module_force(&cand) {
            Ok(()) => {
                info(&format!("shim force-loaded {}", cand.display()));
                return true;
            }
            Err(e) => info(&format!("force load failed: {e}")),
        }
    }
    false
}

/// gs201 (Tensor G2) is a 6.1-only family: the 6.1 glue owns host mode via
/// the `host_ready && host_on` gate and has no `usb_role_switch` consumer
/// (the TCPC's own role switch in `tcpci_max77759.c:3215` only drives the
/// TCPC data path/extcon, never the controller), so host goes through the
/// shim path. A >= 6.12 kernel here is unverified (role-switch store
/// semantics unknown on this SoC) → fail closed.
/// True on kernels where the shim path is dead: a 6.12 glue (as seen on the
/// zuma beta .003 6.12.81 .ko) carries no `dwc3_otg_host_ready` target, so
/// host goes through the native role-switch instead. Unverified on gs201
/// silicon, but presence-gated + readback-verified, so a missing switch
/// fails closed instead of faking host. Unknown version reads as legacy
/// (shim path) to preserve 6.1 behavior.
fn use_native_otg() -> bool {
    match detect_kernel_env() {
        Ok(e) => (e.major, e.minor) >= (6, 12),
        Err(_) => false,
    }
}

/// First `role` file under the role-switch class (created dynamically by
/// the glue driver — no DT node exists, so this is runtime-discovered).
/// Pure over entry names (testable); the live readdir is [`find_usb_role`].
fn pick_role_entry(entries: &[String]) -> Option<String> {
    entries.first().cloned()
}

/// Live role-switch control file, if the kernel registered one.
fn find_usb_role() -> Option<PathBuf> {
    let names: Vec<String> = std::fs::read_dir(USB_ROLE_CLASS)
        .ok()?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    pick_role_entry(&names).map(|n| Path::new(USB_ROLE_CLASS).join(n).join("role"))
}

/// Native 6.12 fallback host path: role-switch to host + gvotable VBUS
/// force. No shim, no OTG_ID (store semantics unverified on 6.12 gs201).
/// Fails closed with a loud reason when the role switch is absent.
fn native_activate() -> Result<(), String> {
    let role = find_usb_role().ok_or_else(|| "no usb_role switch found".to_string())?;
    info("switching controller role to host (native 6.12 fallback)");
    let _ = std::fs::write(role.as_path(), b"host\n");
    sleep(Duration::from_secs(2));
    if Path::new(CHARGER_VALUE).exists() {
        info("forcing VBUS via gvotable CHARGER_MODE");
        let _ = std::fs::write(CHARGER_VALUE, b"49\n");
        let _ = std::fs::write(CHARGER_ACTIVE, b"1\n");
    } else {
        info("native: no CHARGER_MODE voter (no VBUS force?)");
    }
    sleep(Duration::from_secs(2));
    let back = read_trim(&role.to_string_lossy());
    if back == "host" {
        info("native host path ACTIVE (role=host)");
        Ok(())
    } else {
        Err(format!("role switch did not stick: {back:?}"))
    }
}

/// otg-patch (oneshot, service otg_enable): stock chain -> shim ->
/// host_ready -> TCPC switch. Sets sys.usb.patch_dwc3=1 on success (gates
/// otg_auto); 0 + Err otherwise.
pub fn run_otg_patch() -> Result<(), String> {
    info("starting OTG patch routine (gs201 test branch)");
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

    // 1. Stock USB chain from the LIVE firmware (dep order). Each step is
    // best-effort but loud: the summary below tells exactly which half of
    // host mode is missing on this tree.
    let mut chain_ok = true;
    for m in STOCK_USB_CHAIN {
        chain_ok &= insmod_stock(m);
    }
    let glue = Path::new(OTG_ID).exists();
    info(&format!(
        "chain done (all_found={chain_ok}): OTG_ID node {}, CHARGER_MODE voter {}",
        if glue { "present" } else { "MISSING" },
        if Path::new(CHARGER_VALUE).exists() {
            "present"
        } else {
            "MISSING"
        }
    ));

    // 2. Host enable: native role-switch on 6.12 (shim target gone, same as
    // the zuma 6.12 glue), shim + host_ready on 6.1 (it kprobes
    // `dwc3_otg_host_ready`, replacing the AOC probe that never runs in
    // recovery).
    if use_native_otg() {
        info("kernel 6.12+: native fallback path (shim excluded by design)");
        if let Err(e) = native_activate() {
            let _ = set_prop("sys.usb.patch_dwc3", "0");
            return Err(e);
        }
        set_prop("sys.usb.patch_dwc3", "1").map_err(|e| e.to_string())?;
        info("host path ACTIVE via native role-switch, sys.usb.patch_dwc3=1");
    } else {
        if !load_shim() {
            let _ = set_prop("sys.usb.patch_dwc3", "0");
            return Err("shim unavailable".into());
        }
        if read_trim(PROC_READY) != "1" {
            info("activating host_ready via /proc/otg_host_shim");
            let _ = std::fs::write(PROC_SHIM, b"1\n");
        }
        if read_trim(PROC_READY) == "1" {
            set_prop("sys.usb.patch_dwc3", "1").map_err(|e| e.to_string())?;
            info("host_ready ACTIVE, sys.usb.patch_dwc3=1");
        } else {
            let _ = set_prop("sys.usb.patch_dwc3", "0");
            return Err("host_ready NOT active after activation attempt".into());
        }
    }

    // 3. TCPC data-path switch (USBSW 0x93 <- 0x09,
    // tcpci_max77759_vendor_reg.h:94-95). Warning only.
    match patch_max77759_i2c_with_driver("max77759tcpc") {
        Ok(()) => info("USB data path switches connected"),
        Err(e) => info(&format!("TCPC switch not configured: {e}")),
    }
    info("OTG patch routine finished");
    Ok(())
}

// --- otg-auto daemon (R11 algorithm, lean) ---

fn find_vbus() -> Option<String> {
    VBUS_PATHS
        .iter()
        .find(|p| Path::new(p).exists())
        .map(|s| s.to_string())
}

fn read_vbus(path: &Option<String>) -> String {
    match path {
        Some(p) => read_trim(p),
        None => "0".into(),
    }
}

/// sys.usb.controller wins only when it names a real /sys/class/udc entry
/// (else the blanked UNKNOWN literal would kill the bind).
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
    let prop = get_prop("sys.usb.controller").trim().to_string();
    if !prop.is_empty() && !prop.contains("UNKNOWN") && sysfs.iter().any(|e| e == &prop) {
        return prop;
    }
    // gs201 DWC3 child of usb@11210000 (gs201.dtsi:722-749).
    sysfs.into_iter().next().unwrap_or_else(|| "11210000.dwc3".to_string())
}

/// First removable USB disk (sdX1 preferred, else whole disk).
fn pick_otg_disk() -> Option<String> {
    let mut names: Vec<String> = std::fs::read_dir("/sys/block")
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| {
                    n.len() == 3
                        && n.starts_with("sd")
                        && n.as_bytes().get(2).copied().map(|c| c.is_ascii_lowercase()).unwrap_or(false)
                })
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    for n in &names {
        let dir = Path::new("/sys/block").join(n);
        if read_trim(&dir.join("removable").to_string_lossy()) != "1" {
            continue;
        }
        // read_link returns the raw (usually relative:
        // ../../../1-1:1.0/...) target, which never names the USB ancestors
        // — canonicalize first so the "usb" check sees the real
        // /sys/devices/... path (field-proven: the raw link matched
        // nothing, otg-usb was never created, OTG dedup never engaged).
        let dev = dir
            .join("device")
            .canonicalize()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| {
                std::fs::read_link(dir.join("device"))
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default()
            });
        if !dev.contains("usb") {
            continue;
        }
        let p1 = format!("/dev/block/{n}1");
        return Some(if Path::new(&p1).exists() {
            p1
        } else {
            format!("/dev/block/{n}")
        });
    }
    None
}

/// Keep /dev/block/otg-usb (/usb_otg) on the current OTG disk; relink
/// only on change. Best-effort: no disk attached = nothing to do.
fn ensure_otg_symlink() {
    let target = match pick_otg_disk() {
        Some(t) => t,
        None => return,
    };
    let cur = std::fs::read_link(OTG_USB_LINK)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    if cur == target {
        return;
    }
    let _ = std::fs::remove_file(OTG_USB_LINK);
    match std::os::unix::fs::symlink(&target, OTG_USB_LINK) {
        Ok(()) => info(&format!("otg-usb -> {target}")),
        Err(e) => info(&format!("otg-usb symlink FAILED: {e}")),
    }
}

fn switch_to_host(current: &mut String) {
    if current.as_str() == "host" {
        return;
    }
    info(">>> SWITCHING TO HOST MODE <<<");
    let _ = std::fs::write(UDC_FILE, b"\n");
    if Path::new(CHARGER_VALUE).exists() {
        let _ = std::fs::write(CHARGER_VALUE, b"49\n");
        let _ = std::fs::write(CHARGER_ACTIVE, b"1\n");
    } else {
        info("host: no CHARGER_MODE voter (no VBUS force?)");
    }
    // OTG_ID 0 asserts host_on through the glue (`dwc3_exynos_host_event`
    // via dwc3_exynos_otg_id_store, dwc3-exynos.c:1010-1033); with the
    // shim's host_ready the `host_ready && host_on` gate lets host start.
    // On native 6.12 the role switch owns host mode instead (re-assert:
    // TCPC renegotiation can park it back); OTG_ID stays untouched there
    // (unverified store semantics on 6.12).
    if use_native_otg() {
        match find_usb_role() {
            Some(role) => {
                let _ = std::fs::write(role.as_path(), b"host\n");
            }
            None => info("host: no usb_role switch for native re-assert"),
        }
    } else if Path::new(OTG_ID).exists() {
        let _ = std::fs::write(OTG_ID, b"0\n");
    } else {
        info("host: no OTG_ID node (glue missing?)");
    }
    *current = "host".into();
}

fn switch_to_device(current: &mut String) {
    if current.as_str() == "device" {
        return;
    }
    info(">>> SWITCHING TO DEVICE MODE <<<");
    if Path::new(CHARGER_ACTIVE).exists() {
        let _ = std::fs::write(CHARGER_ACTIVE, b"0\n");
    }
    // OTG_ID restore is a 6.1-only step (we asserted it there); on native
    // 6.12 the role switch owns the mode and OTG_ID stays untouched.
    if !use_native_otg() && Path::new(OTG_ID).exists() {
        let _ = std::fs::write(OTG_ID, b"1\n");
    }
    let _ = set_prop("sys.usb.ffs.ready", "1");
    // Self-healing bind: init triggers are edge-based; a missed edge leaves
    // UDC unbound forever. Explicit bind + adbd start covers it.
    let controller = resolve_udc();
    match std::fs::write(UDC_FILE, format!("{controller}\n").as_bytes()) {
        Ok(_) => {
            let _ = set_prop("sys.usb.controller", &controller);
        }
        Err(e) => info(&format!("device mode: UDC bind FAILED: {e}")),
    }
    let _ = set_prop("ctl.start", "adbd");
    *current = "device".into();
}

/// otg-auto: default HOST, external VBUS (PC) -> DEVICE. Never exits.
pub fn run_otg_auto() -> ! {
    info("starting (gs201 test branch, default HOST)");
    let vbus_file = find_vbus();
    match &vbus_file {
        Some(p) => info(&format!("using VBUS path: {p}")),
        None => info("no VBUS sysfs path (charger driver not loaded?), assuming 0"),
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
        if current == "host" {
            ensure_otg_symlink();
        }
        sleep(Duration::from_secs(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chain_dep_order_holds() {
        // gvotable base first; phy before glue; shim before tcpc;
        // tcpc before charger; glue before tcpc.
        let pos = |m: &str| chain_pos(m).expect(m);
        assert!(pos("gvotable") < pos("phy-exynos-usbdrd-super"));
        assert!(pos("phy-exynos-usbdrd-super") < pos("dwc3-exynos-usb"));
        assert!(pos("dwc3-exynos-usb") < pos("tcpci_max77759"));
        assert!(pos("google_tcpci_shim") < pos("tcpci_max77759"));
        assert!(pos("tcpci_max77759") < pos("google-charger"));
        assert!(pos("max77759_helper") < pos("tcpci_max77759"));
    }

    #[test]
    fn gs201_uses_non_eusb_phy() {
        // gs201_defconfig:93 has CONFIG_PHY_EXYNOS_USBDRD=m and no EUSB
        // variant — the zuma `eusb-super` entry must NOT be in our chain.
        assert!(chain_pos("phy-exynos-usbdrd-super").is_some());
        assert!(chain_pos("phy-exynos-usbdrd-eusb-super").is_none());
    }

    #[test]
    fn otg_id_matches_gs201_usb_base() {
        // gs201.dtsi:722 usb@11210000, shared by all gs201 boards via the
        // dts/gs201 symlink — same address as zuma, unlike gs101's 11110000.
        assert!(OTG_ID.contains("11210000.usb"));
        assert!(!OTG_ID.contains("11110000"));
    }

    #[test]
    fn role_entry_picks_first() {
        let two = vec!["11210000.usb-role-switch".to_string(), "other".to_string()];
        assert_eq!(
            pick_role_entry(&two),
            Some("11210000.usb-role-switch".to_string())
        );
        let empty: Vec<String> = Vec::new();
        assert_eq!(pick_role_entry(&empty), None);
    }
}
