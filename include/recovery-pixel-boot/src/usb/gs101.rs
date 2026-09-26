//! usb::gs101 — OTG implementation (Tensor G1: oriole/raven/bluejay).
//!
//! Mirrors the zuma R11 algorithm, with gs101-correct constants taken from
//! the `android-gs-raviole-6.1-android16` kernel sources (NOT assumed from
//! zuma):
//! - 6.1 Samsung glue carries the same AOC gate (`host_ready && host_on` in
//!   `soc_gs/drivers/usb/dwc3/dwc3-exynos-otg.c:128`, `dwc3_otg_host_ready`
//!   EXPORT_SYMBOL_GPL at `:537`) → our kprobe shim replaces the AOC probe;
//!   force-load covers vermagic drift. In stock the only producer is the AOC
//!   USB driver (`aoc/usb/aoc_usb_dev.c:385` probe → `host_ready(true)`),
//!   absent in recovery, so without the shim host can never start.
//! - The glue has NO `usb_role_switch` consumer (no `role_switch` refs in
//!   `dwc3-exynos.c`; the TCPC's own role switch in `tcpci_max77759.c:2592`
//!   only drives the TCPC data path/extcon, never the controller) → 6.1
//!   shim path only, NO native branch by design. A >= 6.12 kernel on this
//!   family is unverified → fail closed, stay in device mode (adb safe).
//! - DT (`gs101/dts/gs101.dtsi:630`): `udc: usb@11110000`, reg 0x11110000,
//!   `dr_mode = "peripheral"` (host comes ONLY via the glue), no `extcon`
//!   phandle on the USB node (host_on is driven solely by the OTG_ID store).
//! - TCPC (`gs101/dts/gs101-common-typec.dtsi:103`): `max77759tcpc@25` on
//!   `hsi2c_12`, compatible `max77759tcpc` (bluejay reuses the same node via
//!   `bluejay/dts/gs101-bluejay-typec.dtsi:9`); USBSW 0x93 <- 0x09 per
//!   `tcpci_max77759_vendor_reg.h:94-95`.
//! - PHY (`gs101/gs101_defconfig:209`): CONFIG_PHY_EXYNOS_USBDRD=m → module
//!   `phy-exynos-usbdrd-super` (no EUSB variant on gs101); glue module
//!   `dwc3-exynos-usb` (`:121`); TCPC `tcpci_max77759` + `max77759_helper`
//!   (`:126`); shim `google_tcpci_shim`. Stock chain order below follows
//!   `gs101/vendor_ramdisk.modules.gs101` dep order (gvotable base first;
//!   the ramdisk file itself lists gvotable last because init uses modprobe
//!   dep resolution, but our manual insmod walk needs dep order).
//! - `usb` power_supply (VBUS sensor) is registered by `usb_psy.ko`
//!   (`usb_psy.c:562-563,873`); `CHARGER_MODE` is the gvotable election the
//!   TCPC votes on (`tcpci_max77759.c:66,1207`).
//! - Stock trees without the Samsung DLKM set stay well-behaved devices
//!   (adb safe) instead of pretending host works.

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
/// gs101 USB base is 0x11110000 (gs101.dtsi:630 `usb@11110000`), NOT zuma's
/// 11210000.
const OTG_ID: &str = "/sys/devices/platform/11110000.usb/dwc3_exynos_otg_id";
/// gvotable election the TCPC votes on (tcpci_max77759.c:66,1207).
/// Created at runtime by the gvotable/charger stack (absent = voter missing).
const CHARGER_VALUE: &str = "/sys/kernel/debug/gvotables/CHARGER_MODE/force_int_value";
const CHARGER_ACTIVE: &str = "/sys/kernel/debug/gvotables/CHARGER_MODE/force_int_active";
const UDC_FILE: &str = "/config/usb_gadget/g1/UDC";
/// Created at runtime by usb_psy.ko (usb_psy.c:562-563,873).
const VBUS_PATHS: &[&str] = &[
    "/sys/class/power_supply/usb/online",
    "/sys/class/power_supply/usb/present",
];
const OTG_USB_LINK: &str = "/dev/block/otg-usb";

/// Stock USB chain in dep order (vendor_ramdisk.modules.gs101 knowledge:
/// gvotable base first; phy before glue; shim before tcpc; tcpc before
/// charger). Missing files are skipped LOUDLY (non-Samsung trees lack them).
/// gs101 PHY is `phy-exynos-usbdrd-super` (gs101_defconfig:209, no EUSB
/// variant) — NOT zuma's `phy-exynos-usbdrd-eusb-super`.
/// xhci/AoC tail (field-proven necessity, 8.13 raven flog): host role
/// switch probes `xhci-hcd-exynos`, whose probe FAILS with -EINVAL
/// (`Offload hooks or init function is null!`, xhci-exynos.c:647-655)
/// unless `aoc_usb_driver.ko` registered its ops — its module_init calls
/// xhci_offload_helper_init() unconditionally, before aoc_driver_register
/// (aoc_usb_dev.c:421-431; hooks impl in xhci_hooks_impl_whi.c:389-401,
/// linked into aoc_usb_driver via Kbuild:13). The stock gs101 ramdisk
/// list has no aoc_usb_driver (only aoc_core/char/control), so first-stage
/// never loads it and host mode can never enumerate without this preload.
/// Order: xhci driver -> mailbox -> aoc_core -> aoc_usb_driver (hooks land
/// at insmod, long before the role switch probes xhci).
const STOCK_USB_CHAIN: &[&str] = &[
    "gvotable",
    "usb_psy",
    "max77759_helper",
    "phy-exynos-usbdrd-super",
    "dwc3-exynos-usb",
    "xhci-exynos",
    "mailbox-wc",
    "aoc_core",
    "aoc_usb_driver",
    "google_tcpci_shim",
    "tcpci_max77759",
    "google-charger",
    "max77759-charger",
];
/// First-stage ramdisk first: on real firmware trees the Samsung USB stack
/// auto-loads from the vendor_kernel_boot ramdisk before we run, so the
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

/// Position of a module in the stock chain (test-only: verifies dep order).
#[cfg(test)]
fn chain_pos(name: &str) -> Option<usize> {
    STOCK_USB_CHAIN.iter().position(|m| *m == name)
}

/// Mapper node path, mirroring `siw map` naming (`-p vendor --suffix b`
/// -> `/dev/block/mapper/vendor_b`, same helper as laguna.rs — kept local
/// until the shared usb helper lands). Pure (testable).
fn mapper_node(base: &str, slot: &str) -> String {
    format!("/dev/block/mapper/{base}_{}", slot.trim_start_matches('_'))
}

/// Create a dm-mapper node via `siw` (`siw map /dev/block/by-name/super
/// -p <part> --suffix <a|b> -s <0|1>`). Field lesson (raven test9 flog):
/// `aoc_usb_driver.ko` is in NONE of the visible roots (/lib/modules,
/// /vendor_dlkm, /vendor unmounted, /system) — the only remaining home
/// is `/vendor/lib/modules`, and nothing maps `/vendor` in recovery.
/// True when the node exists afterwards (pre-existing or just created).
fn siw_map(base: &str, slot: &str) -> bool {
    let node = mapper_node(base, slot);
    if Path::new(&node).exists() {
        return true;
    }
    let sfx = slot.trim_start_matches('_');
    let num = match slot {
        "_a" => "0",
        "_b" => "1",
        _ => {
            info(&format!("siw map: unknown slot suffix {slot:?}, trying both"));
            return siw_map(base, "_a") || siw_map(base, "_b");
        }
    };
    info(&format!("siw map: creating {node}"));
    match std::process::Command::new("/system/bin/siw")
        .args([
            "map",
            "/dev/block/by-name/super",
            "-p",
            base,
            "--suffix",
            sfx,
            "-s",
            num,
        ])
        .output()
    {
        Ok(out) if out.status.success() && Path::new(&node).exists() => {
            info(&format!("siw map: {node} created"));
            true
        }
        Ok(out) => {
            let err = String::from_utf8_lossy(&out.stderr);
            info(format!("siw map {node} FAILED: status={} {err}", out.status).trim());
            Path::new(&node).exists()
        }
        Err(e) => {
            info(&format!("siw map: cannot run /system/bin/siw: {e}"));
            false
        }
    }
}

/// Current slot suffix (`ro.boot.slot_suffix`, bootconfig fallback).
fn slot_suffix() -> String {
    let prop = get_prop("ro.boot.slot_suffix").trim().to_string();
    if prop == "_a" || prop == "_b" {
        return prop;
    }
    std::fs::read_to_string("/proc/bootconfig")
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let (k, v) = l.split_once('=')?;
            (k.trim() == "androidboot.slot_suffix").then(|| v.trim().trim_matches('"').to_string())
        })
        .find(|s| s == "_a" || s == "_b")
        .unwrap_or_default()
}

/// Read-only mount helper (NULL fstype = kernel auto-probe, like the
/// shell `mount -r`). Returns true on success.
fn mount_ro(src: &str, dst: &str) -> bool {
    let (Some(s), Some(d)) = (
        std::ffi::CString::new(src).ok(),
        std::ffi::CString::new(dst).ok(),
    ) else {
        return false;
    };
    // SAFETY: s/d are live NUL-terminated CStrings; mount(2) ro with NULL
    // fstype/data is the standard probe call.
    let rc = unsafe {
        libc::mount(
            s.as_ptr(),
            d.as_ptr(),
            std::ptr::null(),
            libc::MS_RDONLY,
            std::ptr::null(),
        )
    };
    if rc != 0 {
        info(&format!(
            "mount {src} on {dst} FAILED: {}",
            std::io::Error::last_os_error()
        ));
    }
    rc == 0
}

/// argv builder for `siw connect` (pure, testable): loop-device mapping
/// that bypasses device-mapper entirely. Unknown slot reads as None
/// (no unbounded recursion across daemon ticks).
fn siw_connect_args(base: &str, slot: &str) -> Option<Vec<String>> {
    let num = match slot {
        "_a" => "0",
        "_b" => "1",
        _ => return None,
    };
    Some(vec![
        "connect".to_string(),
        "/dev/block/by-name/super".to_string(),
        "-p".to_string(),
        base.to_string(),
        "--suffix".to_string(),
        slot.trim_start_matches('_').to_string(),
        "-s".to_string(),
        num.to_string(),
    ])
}

/// Loop node (`/dev/loopN`) from `siw connect` output (pure, testable).
fn parse_loop_node(output: &str) -> Option<String> {
    output.split_whitespace().find_map(|tok| {
        let t = tok.trim_start_matches(|c: char| !c.is_ascii_alphanumeric() && c != '/');
        let t = t.trim_end_matches(|c: char| !c.is_ascii_alphanumeric());
        let rest = t.strip_prefix("/dev/loop")?;
        (!rest.is_empty() && rest.bytes().all(|c| c.is_ascii_digit())).then(|| t.to_string())
    })
}

/// Loop-device mapping fallback (`siw connect`) for when dm mapping yields
/// no usable node (seen live: `siw map` exits 0 but creates nothing for
/// vendor). Returns the /dev/loopN node — caller mounts ro, copies,
/// umounts, then MUST call [`siw_disconnect`]. Best-effort + loud.
fn siw_connect_node(base: &str, slot: &str) -> Option<String> {
    let args = match siw_connect_args(base, slot) {
        Some(a) => a,
        None => {
            info(&format!("siw connect: unknown slot suffix {slot:?}, skipped"));
            return None;
        }
    };
    info(&format!("siw connect: mapping {base} (slot {slot}) via loop device"));
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    match std::process::Command::new("/system/bin/siw")
        .args(&arg_refs)
        .output()
    {
        Ok(out) => {
            let combined = format!(
                "{}\n{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            match parse_loop_node(&combined) {
                Some(node) if Path::new(&node).exists() => {
                    info(&format!("siw connect: {node} for {base}"));
                    Some(node)
                }
                Some(node) => {
                    info(&format!("siw connect: parsed {node} but node absent"));
                    None
                }
                None => {
                    info(&format!("siw connect {base} FAILED: status={}", out.status));
                    None
                }
            }
        }
        Err(e) => {
            info(&format!("siw connect: cannot run /system/bin/siw: {e}"));
            None
        }
    }
}

/// Release a loop device created by [`siw_connect_node`] (best-effort).
fn siw_disconnect(base: &str, slot: &str) {
    let (num, sfx) = match slot {
        "_a" => ("0", "a"),
        "_b" => ("1", "b"),
        _ => return,
    };
    match std::process::Command::new("/system/bin/siw")
        .args([
            "disconnect",
            "/dev/block/by-name/super",
            "-p",
            base,
            "--suffix",
            sfx,
            "-s",
            num,
        ])
        .output()
    {
        Ok(out) if out.status.success() => info(&format!("siw disconnect: {base} released")),
        Ok(out) => info(&format!("siw disconnect {base} warning: status={}", out.status)),
        Err(e) => info(&format!("siw disconnect: cannot run siw: {e}")),
    }
}

/// Last-resort hunt for `aoc_usb_driver.ko`: ko_stage first, then the
/// standard ko-fetch mechanism (siw|iw stream, map+mount+loop fallback),
/// then the live `/vendor` image — `/dev/block/by-name/vendor` first
/// (unsuffixed = current slot, first-stage pre-mapped; siw map on vendor
/// is a dead end, field-proven on laguna), siw mapper and siw loop mounts
/// as fallbacks.
/// Mounted ro under a private dir, unmounted after; insmod straight from
/// the mount (a loaded module never needs the file again). Returns true
/// when the hooks provider is loaded afterwards.
fn load_aoc_from_vendor() -> bool {
    if is_module_loaded("aoc_usb_driver") {
        return true;
    }
    for sfx in ["_a", "_b"] {
        let cand = format!("/dev/ko_stage/vendor_dlkm{sfx}/aoc_usb_driver.ko");
        if Path::new(&cand).is_file() {
            match load_kernel_module(Path::new(&cand)) {
                Ok(()) => {
                    info(&format!("aoc_usb_driver loaded from {cand} (xhci hooks live)"));
                    return true;
                }
                Err(e) => info(&format!("aoc_usb_driver insmod from {cand} FAILED: {e}")),
            }
        }
    }
    // NOTE: no image streaming here — `usb_modules` from pixel.json are
    // loaded by the boot process (sequential, race-free) before any daemon
    // starts; concurrent ko-fetch streams would clobber the shared
    // /dev/ko_stage + /dev/stage_*.img paths. Vendor mounts below are
    // read-only with private dirs — race-free by construction.
    const MNT: &str = "/dev/otg_mnt_vendor";
    let slot = slot_suffix();
    let _ = std::fs::create_dir_all(MNT);
    // True when MNT currently holds a loop-device mount (needs
    // siw_disconnect after umount); by-name/dm mounts need no release.
    let mut from_loop = false;
    if mount_ro("/dev/block/by-name/vendor", MNT) {
        info("aoc hunt: vendor via by-name (first-stage mapped)");
    } else {
        if slot.is_empty() {
            info("aoc hunt: unknown slot, cannot map vendor");
            return false;
        }
        // Loop device before dm-mapper: fully independent of dm state
        // (live-proven on shiba: loop0 + dm-0 side by side), while
        // `siw map` on vendor is known-flaky (exit 0, no node).
        let mut siw_mounted = false;
        if let Some(loopnode) = siw_connect_node("vendor", &slot) {
            if mount_ro(&loopnode, MNT) {
                info("aoc hunt: vendor via siw loop device");
                siw_mounted = true;
                from_loop = true;
            } else {
                siw_disconnect("vendor", &slot);
            }
        }
        if !siw_mounted && siw_map("vendor", &slot) {
            let node = mapper_node("vendor", &slot);
            siw_mounted = mount_ro(&node, MNT);
        }
        if !siw_mounted {
            info("aoc hunt: vendor mount FAILED (by-name, loop and siw map)");
            return false;
        }
    }
    let src_dir = MNT;
    let mut found: Option<std::path::PathBuf> = None;
    let mut stack = vec![std::path::PathBuf::from(format!("{src_dir}/lib/modules"))];
    while let Some(p) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&p) else {
            continue;
        };
        for e in rd.flatten() {
            let q = e.path();
            if q.is_dir() {
                stack.push(q);
            } else if q.file_name().map(|n| n == "aoc_usb_driver.ko").unwrap_or(false) {
                found = Some(q);
                break;
            }
        }
        if found.is_some() {
            break;
        }
    }
    let mut ok = false;
    if let Some(ko) = found {
        info(&format!("aoc hunt: found {}", ko.display()));
        match load_kernel_module(&ko) {
            Ok(()) => {
                info("aoc_usb_driver loaded from vendor (xhci hooks live)");
                ok = true;
            }
            Err(e) => info(&format!("aoc_usb_driver insmod from vendor FAILED: {e}")),
        }
    } else {
        info("aoc hunt: aoc_usb_driver.ko NOT on vendor (not shipped for gs101?)");
    }
    // SAFETY: MNT is a valid NUL-terminated C string; umount(2) failure ignored.
    unsafe {
        libc::umount(b"/dev/otg_mnt_vendor\0".as_ptr() as *const libc::c_char);
    }
    if from_loop {
        siw_disconnect("vendor", &slot);
    }
    ok
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

/// True on kernels where the shim path is dead: a 6.12 glue (as seen on the
/// zuma beta .003 6.12.81 .ko) carries no `dwc3_otg_host_ready` target, so
/// host goes through the native role-switch instead. Unverified on gs101
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
/// force. No shim, no OTG_ID (store semantics unverified on 6.12 gs101).
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
    info("starting OTG patch routine (gs101)");
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
    // The hooks provider lives outside every visible root (raven test9
    // flog: not in /lib/modules, /vendor_dlkm or /system) — hunt it on
    // the live vendor image before declaring the chain state.
    if !is_module_loaded("aoc_usb_driver") {
        load_aoc_from_vendor();
    }
    info(&format!(
        "chain done (all_found={chain_ok}): OTG_ID node {}, CHARGER_MODE voter {}, xhci hooks {}",
        if glue { "present" } else { "MISSING" },
        if Path::new(CHARGER_VALUE).exists() {
            "present"
        } else {
            "MISSING"
        },
        if Path::new("/sys/module/aoc_usb_driver").exists() {
            "registered (aoc_usb_driver loaded)"
        } else {
            "MISSING (xhci host probe will fail -22)"
        }
    ));

    // 2. Host enable: native role-switch on 6.12 (shim target gone, same as
    // the zuma 6.12 glue), shim + host_ready on 6.1 (replaces the AOC
    // probe, absent in recovery — aoc_usb_dev.c:385 is the only stock
    // producer).
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
    // gs101 DWC3 child of usb@11110000 (gs101.dtsi:630,655-657).
    sysfs.into_iter().next().unwrap_or_else(|| "11110000.dwc3".to_string())
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
        let dev = std::fs::read_link(dir.join("device"))
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
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
    // Lazy hooks hunt: the patch-time hunt runs before ko_stage exists
    // and before dm-0 may be mapped, so retry here — by now the tree is
    // settled. No-op when hooks are already live. Without them the xhci
    // probe below fails -22 and enumeration is impossible.
    if !use_native_otg() && !is_module_loaded("aoc_usb_driver") {
        info("host: hooks missing, late aoc hunt");
        load_aoc_from_vendor();
    }
    let _ = std::fs::write(UDC_FILE, b"\n");
    if Path::new(CHARGER_VALUE).exists() {
        let _ = std::fs::write(CHARGER_VALUE, b"49\n");
        let _ = std::fs::write(CHARGER_ACTIVE, b"1\n");
    } else {
        info("host: no CHARGER_MODE voter (no VBUS force?)");
    }
    // OTG_ID store 0 -> host_event(!0=1) -> host_on=1 (dwc3-exynos.c:1028);
    // host starts only with host_ready (shim) && host_on (this write).
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
    // OTG_ID restore 1 -> host_on=0 (we asserted it on the host switch).
    // Native 6.12 never asserted it, so the restore is 6.1-only.
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
    info("starting (gs101, default HOST)");
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
    fn xhci_hooks_chain_order_holds() {
        // xhci probe at role-switch time needs aoc_usb_driver's ops
        // (xhci-exynos.c:647-655); hooks register at insmod
        // (aoc_usb_dev.c:421-431), so the AoC tail must come after the
        // glue and before any host switch: mailbox -> core -> usb driver.
        let pos = |m: &str| chain_pos(m).expect(m);
        assert!(pos("dwc3-exynos-usb") < pos("xhci-exynos"));
        assert!(pos("xhci-exynos") < pos("mailbox-wc"));
        assert!(pos("mailbox-wc") < pos("aoc_core"));
        assert!(pos("aoc_core") < pos("aoc_usb_driver"));
        assert!(pos("aoc_usb_driver") < pos("tcpci_max77759"));
    }

    #[test]
    fn gs101_uses_non_eusb_phy() {
        // gs101_defconfig:209 has CONFIG_PHY_EXYNOS_USBDRD=m and no EUSB
        // variant — the zuma `eusb-super` entry must NOT be in our chain.
        assert!(chain_pos("phy-exynos-usbdrd-super").is_some());
        assert!(chain_pos("phy-exynos-usbdrd-eusb-super").is_none());
    }

    #[test]
    fn otg_id_matches_gs101_usb_base() {
        // gs101.dtsi:630 usb@11110000 — never zuma's 11210000.
        assert!(OTG_ID.contains("11110000.usb"));
        assert!(!OTG_ID.contains("11210000"));
    }

    #[test]
    fn role_entry_picks_first() {
        let two = vec!["11110000.usb-role-switch".to_string(), "other".to_string()];
        assert_eq!(
            pick_role_entry(&two),
            Some("11110000.usb-role-switch".to_string())
        );
        let empty: Vec<String> = Vec::new();
        assert_eq!(pick_role_entry(&empty), None);
    }

    #[test]
    fn siw_connect_args_cover_both_slots() {
        let a = siw_connect_args("vendor", "_a").unwrap();
        assert_eq!(a[0], "connect");
        assert!(a.contains(&"vendor".to_string()));
        assert!(a.contains(&"a".to_string()));
        assert!(a.contains(&"0".to_string()));
        let b = siw_connect_args("vendor_dlkm", "_b").unwrap();
        assert!(b.contains(&"1".to_string()));
        assert!(siw_connect_args("vendor", "").is_none());
    }

    #[test]
    fn loop_node_parsed_from_chatter() {
        assert_eq!(
            parse_loop_node("Created /dev/loop0 for vendor_b"),
            Some("/dev/loop0".to_string())
        );
        assert_eq!(parse_loop_node("/dev/loop3"), Some("/dev/loop3".to_string()));
        assert_eq!(parse_loop_node("error: no such partition"), None);
        assert_eq!(parse_loop_node(""), None);
    }

    #[test]
    fn mapper_node_naming_matches_siw() {
        assert_eq!(mapper_node("vendor", "_b"), "/dev/block/mapper/vendor_b");
        assert_eq!(mapper_node("vendor_dlkm", "_a"), "/dev/block/mapper/vendor_dlkm_a");
    }
}
