//! usb::zumapro — OTG implementation for Tensor G4 (R12_14.1_otg_test).
//!
//! comet (Pixel 9 Pro Fold), tegu, tokay (Pixel 9), caiman (Pixel 9 Pro),
//! komodo (Pixel 9 Pro XL). stallion has NO public source anywhere in the
//! zumapro/caimito/comet/tegu trees — its constants below are
//! carried-over-from-zuma-unverified, still fail-closed at runtime via the
//! presence gates (missing nodes = loud device-mode stay, adb safe).
//!
//! Source: android-gs-comet-6.1-android16 (newest stable 6.1-android16 for
//! the family; -beta/-dp excluded) + matching soc/gs branch of the same
//! name (same ab/13397946). Manifest default.xml lists the zumapro,
//! caimito, comet AND tegu device projects, so one branch covers the whole
//! family except stallion. No `zumapro` kernel branch exists (only per-
//! device comet/tegu/caimito branches); Pixel 9/9Pro (tokay/caiman) ARE
//! covered — caimito ships zumapro-{tokay,caiman,komodo}-*.dts.
//!
//! Hypothesis CONFIRMED — zumapro is the same Exynos DWC3 + max77759
//! pattern as zuma, down to the addresses (cited per constant):
//! - AOC gate: `host_ready && host_on` in dwc3_exynos_set_role
//!   (soc_gs/drivers/usb/dwc3/dwc3-exynos-otg.c:128); the ONLY
//!   `dwc3_otg_host_ready` refs in the glue are the definition (:515) and
//!   `EXPORT_SYMBOL_GPL(dwc3_otg_host_ready)` (:537); the AOC probe calling
//!   it ships as aoc_usb_driver.ko (vendor_ramdisk.modules.zumapro:147,
//!   absent in recovery) → kprobe shim replaces it, force-load covers
//!   vermagic drift (kprobe + procfs are ABI-stable across 6.1.x).
//! - OTG_ID node `dwc3_exynos_otg_id` (dwc3-exynos.c:1170); store at
//!   :1010-1033 runs host_event(!id), so 0 = host, 1 = device (show
//!   :1003 emits !host_on). No `usb_role_switch` usage anywhere in the
//!   glue (0 refs) → shim path on 6.1 by design; the 6.12 native branch is
//!   kept for forward-compat but is itself presence-gated (no role switch
//!   = fail closed). 6.12 zumapro glue is unverified — no 6.12 zumapro
//!   source exists yet.
//! - DT: zumapro/dts/zumapro-usb.dtsi:7 literally includes
//!   zuma/zuma-usb.dtsi (only override: phy_eusb_version) → `udc:
//!   usb@11210000`, reg 0x11210000 (zuma-usb.dtsi:184-190), dwc3 child
//!   dr_mode="peripheral" (:226: host comes ONLY via the glue),
//!   usb-psy-name="usb" (:234: VBUS sensor). Platform path 11210000.usb
//!   is VERIFIED identical to zuma, not carried over.
//! - TCPC max77759tcpc@25, compatible "max77759tcpc"
//!   (zuma-ref dts/max777x9-tcpc.dtsi:9-12); CHARGER_MODE voter is
//!   #defined in the TCPC driver itself
//!   (soc_gs/drivers/usb/typec/tcpm/google/tcpci_max77759.c:66); USBSW
//!   0x93 <- 0x09 via crate::i2c.
//! - Stock chain in zumapro/vendor_ramdisk.modules.zumapro is
//!   module-for-module identical to zuma's (sorted diff shows only
//!   non-USB deltas: gnssif, dock, dual_batt_gauge vs etm2dram, tzprot):
//!   phy-exynos-usbdrd-eusb-super (:9), dwc3-exynos-usb (:103),
//!   google_tcpci_shim (:109), usb_psy (:110), tcpci_max77759 (:111),
//!   max77759_helper (:112), google-charger (:150),
//!   max77759-charger (:155), gvotable (:176). Defconfig agrees
//!   (zumapro_defconfig: CONFIG_PHY_EXYNOS_USBDRD_EUSB=m :96,
//!   CONFIG_USB_DWC3_EXYNOS_GS=m :220, CONFIG_TYPEC_MAX77759=m :224).
//!   Board overlays are identical across comet/caimito/tegu
//!   (zuma-{comet,caimito,tegu}-usb.dtsi: same extcon + force_gen1 +
//!   super-speed cap).
//! - stock Google firmware (stable AND beta) has NONE of the modules: on
//!   such trees the preload below finds nothing and we stay a well-behaved
//!   device (adb safe) instead of pretending host works.
//!
//! Deliberately lean (R11 algorithm): UDC detach with "\n" (0-byte writes
//! never reach configfs .store), ffs.ready written once, explicit UDC
//! rebind on the device switch (init triggers are edge-based; a missed edge
//! leaves the gadget dead with no retry), /dev/block/otg-usb stable symlink
//! for /usb_otg.

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
/// VERIFIED: zuma-usb.dtsi:184 (`udc: usb@11210000`, reg 0x11210000)
/// included verbatim by zumapro-usb.dtsi:7; node registered at
/// dwc3-exynos.c:1170.
const OTG_ID: &str = "/sys/devices/platform/11210000.usb/dwc3_exynos_otg_id";
/// Created at runtime by google-charger.ko/tcpc (absent = voter missing).
/// Voter name per tcpci_max77759.c:66 (`GBMS_MODE_VOTABLE "CHARGER_MODE"`).
const CHARGER_VALUE: &str = "/sys/kernel/debug/gvotables/CHARGER_MODE/force_int_value";
const CHARGER_ACTIVE: &str = "/sys/kernel/debug/gvotables/CHARGER_MODE/force_int_active";
const UDC_FILE: &str = "/config/usb_gadget/g1/UDC";
/// Created at runtime by the charger driver (usb_psy).
/// VERIFIED: usb-psy-name = "usb" (zuma-usb.dtsi:234, shared via include).
const VBUS_PATHS: &[&str] = &[
    "/sys/class/power_supply/usb/online",
    "/sys/class/power_supply/usb/present",
];
const OTG_USB_LINK: &str = "/dev/block/otg-usb";

/// Stock USB chain in dep order (stock modules.dep: glue needs phy +
/// gvotable; tcpc needs shim + usb_psy + gvotable; charger needs tcpc).
/// Missing files are skipped LOUDLY (stock-Google trees don't ship them).
/// Module set verified identical to zuma in
/// zumapro/vendor_ramdisk.modules.zumapro (:9-10, :103-117, :147, :150,
/// :155-157, :176); no `phy-exynos-usbdrd-super` — that is the gs201 PHY
/// name, zumapro ships only the `eusb-super` variant like zuma (Kbuild can
/// build both, only eusb-super is in the ramdisk list).
const STOCK_USB_CHAIN: &[&str] = &[
    "gvotable",
    "usb_psy",
    "max77759_helper",
    "phy-exynos-usbdrd-eusb-super",
    "dwc3-exynos-usb",
    "google_tcpci_shim",
    "tcpci_max77759",
    "google-charger",
    "max77759-charger",
];
/// First-stage ramdisk first: on real firmware trees the whole Samsung USB
/// stack auto-loads from the vendor_kernel_boot ramdisk before we run
/// (dmesg: `init: Loading module /lib/modules/aoc_usb_driver.ko`), so the
/// chain below is normally a silent no-op safety net, not the prime mover.
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

/// True on kernels where the shim is dead by design: the 6.12 glue has no
/// `dwc3_otg_host_ready` target (verified 0 refs in the 6.12.81 .ko), so
/// host goes through the native role-switch instead. Unknown version reads
/// as legacy (shim path) to preserve 6.1 behavior.
/// NOTE (zumapro): 6.12 zumapro glue is unverified (no 6.12 source exists
/// yet); the native path below is additionally presence-gated on the role
/// switch, so it still fails closed there.
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

/// Native 6.12 host path: role-switch to host + gvotable VBUS force.
/// No shim, no OTG_ID (unverified store semantics on 6.12). Fails closed
/// with a loud reason when the role switch is absent.
fn native_activate() -> Result<(), String> {
    let role = find_usb_role().ok_or_else(|| "no usb_role switch found".to_string())?;
    info("switching controller role to host (native 6.12 path)");
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
    info("starting OTG patch routine (zumapro branch)");
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

    // 2. Host enable: native role-switch on 6.12 (no shim by design),
    // shim + host_ready on 6.1 (replaces the AOC probe, absent in recovery).
    if use_native_otg() {
        info("kernel 6.12+: native path (shim excluded by design)");
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

    // 3. TCPC data-path switch (USBSW 0x93 <- 0x09). Warning only.
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
    if use_native_otg() {
        // 6.12: the role switch owns host mode (re-assert: TCPC
        // renegotiation can park it back). OTG_ID is deliberately
        // untouched — unverified store semantics on 6.12.
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
    info("starting (zumapro branch, default HOST)");
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
