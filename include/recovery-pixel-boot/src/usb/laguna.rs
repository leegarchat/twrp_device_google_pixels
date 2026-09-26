//! usb::laguna — OTG implementation (Tensor G5: frankel/blazer/mustang/rango).
//!
//! Two kernel generations, one election-driven mechanism (LOCAL-verified on
//! mustang cp2a-stable 6.6.118 + mustang beta 6.12.81; all four beta
//! boot.img kernels report the same `6.12.81-android16-6-...-4k` version):
//! - 6.6 (stable, e.g. cp2a `6.6.118-android15-8-g53e6e091166e-ab15266607-4k`,
//!   DLKM vermagic `6.6.118-...-ab15298874-4k`): `dwc3-google.ko`
//!   (`vermagic 6.6.118-...-ab15298874-4k`) carries 0 `dwc3_otg_host_ready`
//!   refs and 16 `usb_role_switch` refs — no Samsung AOC gate, host goes
//!   through the standard role-switch framework.
//! - 6.12 (beta4, `6.12.81-android16-6-g6dfbc4104d70-ab16262372-4k`, DLKM
//!   `6.12.81-...-ab16299125-4k`): same architecture, `dwc3-google.ko`
//!   0 `host_ready` / 23 `usb_role_switch`, `google-role-sw.ko` 9
//!   `usb_role_switch` (vs 6 on 6.6). Module count 217 (beta) vs 207
//!   (cp2a); USB core identical, deltas are non-USB (`aoc_ipc`,
//!   `aoss_ssr_notifier`, `max77779-usecase`, ... — local `diff` of the two
//!   `lib/modules` listings).
//! - Shim dead on BOTH: `dwc3_otg_host_ready` ×0 in both kernel Images and
//!   in both `dwc3-google.ko` (so no kprobe target — never attempt the shim).
//! - `CHARGER_MODE` ×0 in both kernel Images and ×0 in `gvotable.ko`; ×1 in
//!   `tcpci_max77759.ko` (the TCPCI `charger_mode_votable` name, not a
//!   debugfs force node) → no gvotable VBUS force path by design.
//! - TCPC is SPMI, not I2C: DTBO `max77759tcpc-spmi@4`
//!   (`compatible = max77759tcpc-spmi`, `reg = <0x4 0x0>`,
//!   `usb-psy-name = "usb"`, `chg-psy-name = "gcpm"`); driver
//!   `spmi-max77759tcpc` (`tcpci_max777x9_spmi.ko`, `alias
//!   of:N*T*Cmax77759tcpc-spmi`); no `11-0025`-style I2C client by design
//!   (crate::i2c SPMI fallback covers it).
//! - Role voter is `goog_usb_role_sw` (`compatible = google,usb-role-sw`,
//!   DTBO `fragment@usb_data_role_switch`, `status = okay`) backed by
//!   `google-role-sw.ko` (`alias of:N*T*Cgoogle,usb-role-sw`, election
//!   `USB_DR_EL`, `depends=gvotable`); eUSB2 repeater `tiusb2e11@3E`
//!   (`compatible = goog-eusb2-repeater`, `usb-role-switch` phandle).
//! - DWC3 glue is `dwc3-google.ko` (`description "Google DWC3 Glue Driver"`,
//!   `alias of:N*T*Cgoogle,dwc3-lga`, `depends google_icc,google-usb-phy`);
//!   DTB `simple_usb_bus/usb3@c450000 [google,dwc3-lga]` +
//!   `dwc3@c400000 [snps,dwc3]` (`reg 0xc400000`, `role-switch-default-mode
//!   = peripheral`, `usb-role-switch` present) → UDC fallback
//!   `c400000.dwc3` (dynamically resolved via `/sys/class/udc` first).
//! - AoC gate: `aoc_usb_driver.ko` (`usb_control` ×1 both gens,
//!   `depends aoc_core,...`) + `aoc_core.ko` (`services`, `responsive`,
//!   `services_show`); vendor.img `bin/aocd` (0755) + 6 libs (`libaoc,
//!   libbase, libevent, aoc_aconfig_flags_c_lib,
//!   libaconfig_storage_read_api_cc, libc++` — local debugfs of cp2a
//!   `vendor.img` + P11 `otg_yogi.sh:30-32`).
//!
//! Mechanism (MIRROR `azagramac/pixel10-kernel-aosp`, corroborated by the
//! local `.ko` strings above — every mirror claim has a local string/DTBO
//! fact or is marked UNVERIFIED):
//! - `google-role-sw.c:update_data_role` truth table: host requires
//!   `TCPCI=host AND AOC=host` (else `NONE`); `DISABLE_USB_DATA` forces
//!   `NONE`. Local corroboration: `google-role-sw.ko` contains the same
//!   `[votes] %s...` log, `disable-aoc-voter`, `USB_DR_EL`, and
//!   `gvotable_*` calls on both gens.
//! - AOC vote: `aoc/usb/aoc_usb_dev.c:247` casts `AOC_VOTER HOST` when
//!   `aoc_ready` (i.e. once the `usb_control` service runs — started by
//!   `aocd`). Local: `aoc_usb_driver.ko` `usb_control` ×1.
//! - TCPCI vote: `tcpci_max77759.c:1069` casts `TCPCI_VOTER` from Type-C CC
//!   (`notify_usb_data_role`). Local: kernel Images carry
//!   `preferred_role_show/store`, `data_role_show/store`,
//!   `power_role_show/store`, `port_type_show` (TCPM core, both gens).
//! - Downstream: `dwc3-google.c:424` registers `role_sw` with
//!   `allow_userspace_control=true`; `dwc3_google_setup_role_switch` +
//!   `_dwc3_google_set_role` drive host. Direct downstream writes are a
//!   hardware no-op per field lesson (election owns the role; a manual
//!   write is reverted/bypasses VBUS+eUSB enable) → this implementation
//!   NEVER writes `/sys/class/usb_role/*/role` for host enable; it stages
//!   AoC, lets the election drive host, and only VERIFIES via sysfs.
//!
//! Success criteria (observable in recovery, per prior field lesson — never
//! role alone): downstream `role == host` AND Type-C `data_role == host`
//! AND a source power_supply `online == 1`. `patch_dwc3=1` is set only when
//! all three hold; otherwise `0` + `Err` (device mode, adb safe).
//!
//! Daemon note: the kernel self-switches via the election once AoC + TCPCI
//! vote host, so `run_otg_auto` performs NO role/UDC writes by design
//! (they would be no-ops). It runs the AoC watchdog ported from P11
//! `otg_yogi.sh` (stage → insmod → `verify_aoc_responsive`/`services`
//! poll → flaky-startup `aocd` relaunch) plus the `/dev/block/otg-usb`
//! symlink maintenance, and parks (never exits) — an exiting non-oneshot
//! service would respawn-loop.

use crate::i2c::patch_max77759_i2c_with_driver;
use crate::ko_picker::{
    detect_kernel_env, is_module_loaded, load_kernel_module, log_msg,
};
use crate::props::{get_prop, set_prop};
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::Duration;

const TAG: &str = "otg";
/// Registered by `dwc3-google.ko` (`dwc3_google_setup_role_switch`,
/// mirror `dwc3-google.c:424`; local `usb_role_switch` ×16/×23). The
/// control file itself is runtime-discovered; this is only the class dir.
const USB_ROLE_CLASS: &str = "/sys/class/usb_role";
/// TCPM core class (kernel Image `preferred_role_show/store`,
/// `data_role_show/store` both gens). `data_role` is the election-visible
/// host signal used for verification (never written here).
const TYPEC_CLASS: &str = "/sys/class/typec";
/// Created at runtime by `usb_psy.ko` (`usb-psy-name = "usb"`, DTBO
/// `max77759tcpc-spmi@4`) and the `gcpm` charger psy (`chg-psy-name =
/// "gcpm"`). Source-online candidates for the host check.
const SOURCE_PSY_PATHS: &[&str] = &[
    "/sys/class/power_supply/gcpm/online",
    "/sys/class/power_supply/usb/online",
    "/sys/class/power_supply/usb/present",
];
/// UDC gadget path (`/config/usb_gadget/g1/UDC`). Documented, never
/// written on laguna by design (election owns roles; see module docs).
#[allow(dead_code)]
const UDC_FILE: &str = "/config/usb_gadget/g1/UDC";
/// Fallback when `/sys/class/udc` is empty. DTB
/// `simple_usb_bus/usb3@c450000/dwc3@c400000` (`reg 0xc400000`,
/// `compatible snps,dwc3`); live UDC names are resolved dynamically first.
const UDC_FALLBACK: &str = "c400000.dwc3";
const OTG_USB_LINK: &str = "/dev/block/otg-usb";
/// AoC staging area in tmpfs (P11 `otg_yogi.sh:17` `RAM=/tmp/aoc`).
const AOC_RAM: &str = "/tmp/aoc";
const AOC_LIB: &str = "/tmp/aoc/lib";
/// Staged runtime (P11 `otg_yogi.sh:22,37-39`: `staged()` = both exist).
const AOCD_BIN: &str = "/tmp/aoc/aocd";
const AOC_KO: &str = "/tmp/aoc/aoc_usb_driver.ko";
/// The 6 userspace libs `otg_yogi.sh:30-32` copies from live vendor
/// (`libaoc libbase libevent aoc_aconfig_flags_c_lib
/// libaconfig_storage_read_api_cc libc++`); existence verified via local
/// debugfs of cp2a `vendor.img` (`bin/aocd` 0755 + matching `lib64/*.so`).
const AOC_LIBS: &[&str] = &[
    "libaoc",
    "libbase",
    "libevent",
    "aoc_aconfig_flags_c_lib",
    "libaconfig_storage_read_api_cc",
    "libc++",
];
/// Live-vendor sources tried before falling back to dm-mapper mounts:
/// already-mounted `/vendor` (recovery often has it) wins; mapper nodes
/// mirror the shell script (`/dev/block/mapper/vendor$slot`,
/// `/dev/block/mapper/vendor_dlkm$slot`, `otg_yogi.sh:20,28,36`).
const VENDOR_AOCD: &str = "/vendor/bin/aocd";
const VENDOR_LIBDIR: &str = "/vendor/lib64";
const VENDOR_DLKM_KO: &str = "/vendor_dlkm/lib/modules/aoc_usb_driver.ko";
const FIRSTSTAGE_KO: &str = "/lib/modules/aoc_usb_driver.ko";
const OTG_MNT_VENDOR: &str = "/dev/otg_mnt/vendor";
const OTG_MNT_VDLKM: &str = "/dev/otg_mnt/vdlkm";
/// Stock USB chain in dep order for manual insmod (strict flags: stock
/// modules match the running kernel by construction — local vermagic
/// `6.6.118-...-ab15298874-4k` / `6.12.81-...-ab16299125-4k`).
/// Order from local `modules.dep`: `gvotable` base first (`usb_psy` needs
/// `max77759_helper`+`gvotable`); `tcpc` needs `google_tcpci_shim`+`usb_psy`
/// +`helper`; SPMI TCPC needs the I2C TCPC; `google-role-sw` needs only
/// `gvotable` (election `USB_DR_EL`); `dwc3-google` needs
/// `google-usb-phy`+`google_icc` (local modinfo `depends`); charger stack
/// needs TCPC; `aoc_usb_driver` needs `aoc_core`+`gvotable`.
/// Exact file spellings (dashes vs underscores) match the local
/// `lib/modules` listings on both gens. Missing files are skipped LOUDLY.
const STOCK_USB_CHAIN: &[&str] = &[
    "gvotable",
    "logbuffer",
    "max77759_helper",
    "usb_psy",
    "bc_max77759",
    "google_tcpci_shim",
    "tcpci_max77759",
    "tcpci_max777x9_spmi",
    "regmap-goog-spmi",
    "max77779_pmic",
    "max77779-charger",
    "max77779-charger-spmi",
    "google-role-sw",
    "google_icc",
    "google-usb-phy",
    "google-eusb2-repeater",
    "dwc3-google",
    "google-charger",
    "aoc_core",
    "aoc_usb_driver",
];
/// First-stage ramdisk first: on real firmware the USB stack auto-loads
/// from `vendor_kernel_boot` before we run, so this chain is normally a
/// silent no-op safety net, not the prime mover.
const STOCK_ROOTS: &[&str] = &[
    "/lib/modules",
    "/vendor_dlkm/lib/modules",
    "/vendor/lib/modules",
    "/system/lib64/modules",
];

/// Kernel generation owning the laguna USB stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Gen {
    /// 6.6 stable (local cp2a `6.6.118-android15-8-...-4k`).
    V66,
    /// 6.12 beta4 (local `6.12.81-android16-6-...-4k`, all four devices).
    V612,
    Unknown,
}

fn info(msg: &str) {
    log_msg(TAG, "INFO", msg);
    println!("otg: INFO: {msg}");
}

fn read_trim(p: &str) -> String {
    std::fs::read_to_string(p).unwrap_or_default().trim().to_string()
}

/// Version gate modeled on `zuma::use_native_otg`: laguna spans exactly
/// 6.6 (stable) and 6.12 (beta4). Anything else is unverified → fail
/// closed (device mode, adb safe). Pure over (major, minor) (testable);
/// the live uname read lives in [`detect_generation`].
fn classify_generation(major: u32, minor: u32) -> Gen {
    match (major, minor) {
        (6, 6) => Gen::V66,
        (6, 12) => Gen::V612,
        _ => Gen::Unknown,
    }
}

fn detect_generation() -> Gen {
    match detect_kernel_env() {
        Ok(e) => classify_generation(e.major, e.minor),
        Err(_) => Gen::Unknown,
    }
}

/// First `role` file under the role-switch class (created dynamically by
/// `dwc3-google.ko` — no DT node exists, so runtime-discovered).
/// Pure over entry names (testable); the live readdir is [`find_usb_role`].
fn pick_role_entry(entries: &[String]) -> Option<String> {
    entries.first().cloned()
}

/// Live downstream role-switch control file, if the kernel registered one.
fn find_usb_role() -> Option<PathBuf> {
    let names: Vec<String> = std::fs::read_dir(USB_ROLE_CLASS)
        .ok()?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    pick_role_entry(&names).map(|n| Path::new(USB_ROLE_CLASS).join(n).join("role"))
}

/// First `data_role` file under the Type-C class (TCPM core; kernel Image
/// `data_role_show/store` both gens). Pure over entry names (testable).
fn pick_typec_entry(entries: &[String]) -> Option<String> {
    entries
        .iter()
        .find(|n| !n.is_empty())
        .cloned()
}

/// Live Type-C `data_role` file, if TCPM registered a port.
fn find_typec_data_role() -> Option<PathBuf> {
    let names: Vec<String> = std::fs::read_dir(TYPEC_CLASS)
        .ok()?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    // Prefer a port entry; fall back to the first entry that owns a
    // `data_role` file (partners/cables also live here).
    let mut ordered: Vec<String> = Vec::new();
    if let Some(p) = pick_typec_entry(
        &names.iter().filter(|n| n.starts_with("port")).cloned().collect::<Vec<_>>(),
    ) {
        ordered.push(p);
    }
    for n in &names {
        if !ordered.contains(n) {
            ordered.push(n.clone());
        }
    }
    for n in ordered {
        let cand = Path::new(TYPEC_CLASS).join(&n).join("data_role");
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

/// Pure host check over already-read sysfs values (testable).
/// All three must read host/online: role alone is never enough (prior
/// field lesson — a stuck role switch reports false success).
fn is_host_verified(role: &str, data_role: &str, psy_online: &str) -> bool {
    role.trim() == "host" && data_role.trim() == "host" && psy_online.trim() == "1"
}

/// Best source-online reading among the candidate PSYs (`gcpm` first:
/// DTBO `chg-psy-name = "gcpm"`; then the `usb` psy). Returns the raw
/// trimmed string (`"1"` = sourcing VBUS) plus the path that produced it.
fn read_source_psy() -> (String, String) {
    for p in SOURCE_PSY_PATHS {
        if Path::new(p).exists() {
            let v = read_trim(p);
            if !v.is_empty() {
                return (v, p.to_string());
            }
        }
    }
    (String::new(), String::new())
}

/// Verified-host gate: downstream `role == host` AND Type-C
/// `data_role == host` AND source PSY `online == 1`. Loud about each leg
/// so a partial state never silently passes.
fn verify_host() -> bool {
    let role_path = find_usb_role();
    let role = role_path
        .as_ref()
        .map(|p| read_trim(&p.to_string_lossy()))
        .unwrap_or_default();
    let data_path = find_typec_data_role();
    let data_role = data_path
        .as_ref()
        .map(|p| read_trim(&p.to_string_lossy()))
        .unwrap_or_default();
    let (psy, psy_path) = read_source_psy();
    info(&format!(
        "verify: role={role:?} data_role={data_role:?} psy={psy:?} ({psy_path})"
    ));
    if role.is_empty() || data_role.is_empty() || psy.is_empty() {
        info("verify: missing leg (role/data_role/psy absent) → NOT host");
        return false;
    }
    let ok = is_host_verified(&role, &data_role, &psy);
    info(if ok {
        "verify: HOST confirmed (role+data_role+psy)"
    } else {
        "verify: NOT host (need role=host AND data_role=host AND psy=1)"
    });
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

/// Slot suffix for dm-mapper nodes: `ro.boot.slot_suffix` wins;
/// `/proc/bootconfig` (`androidboot.slot_suffix = "_a"`) is the fallback
/// (same two sources as P11 `otg_yogi.sh:19`).
fn slot_suffix() -> String {
    let prop = get_prop("ro.boot.slot_suffix").trim().to_string();
    if prop == "_a" || prop == "_b" {
        return prop;
    }
    if let Ok(cfg) = std::fs::read_to_string("/proc/bootconfig") {
        for token in cfg.split(['"', ' ', '\n', '\t', '=', ',']) {
            let t = token.trim();
            if t == "_a" || t == "_b" {
                return t.to_string();
            }
        }
        for line in cfg.lines() {
            if line.contains("androidboot.slot_suffix") {
                if line.contains("_b") {
                    return "_b".to_string();
                }
                if line.contains("_a") {
                    return "_a".to_string();
                }
            }
        }
    }
    prop
}

/// Mount helper for the dm-mapper vendor mounts (read-only, like the shell
/// `mount -o ro ...` in `otg_yogi.sh:28,36`). Best-effort: mapper nodes
/// are not up when the service first fires (script retries from its loop).
fn mount_ro(source: &str, target: &str) -> bool {
    let _ = std::fs::create_dir_all(target);
    // SAFETY: source/target are valid NUL-terminated C strings built from
    // live Strings; mount(2) with fstype "ext4"+NULL data would be wrong
    // for dm-verity, so fstype is NULL (kernel auto-probe) with MS_RDONLY.
    let src = match std::ffi::CString::new(source) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let tgt = match std::ffi::CString::new(target) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let rc = unsafe {
        libc::mount(
            src.as_ptr(),
            tgt.as_ptr(),
            std::ptr::null(),
            libc::MS_RDONLY,
            std::ptr::null(),
        )
    };
    rc == 0
}

fn umount(target: &str) {
    // SAFETY: target is a valid NUL-terminated C string; umount(2) has no
    // thread-safety concerns and failure is intentionally ignored.
    if let Ok(tgt) = std::ffi::CString::new(target) {
        unsafe {
            libc::umount(tgt.as_ptr());
        }
    }
}

fn copy_if_src(dst: &str, src: &str) -> bool {
    if Path::new(dst).is_file() {
        return true;
    }
    if !Path::new(src).is_file() {
        return false;
    }
    if let Some(parent) = Path::new(dst).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::copy(src, dst) {
        Ok(_) => true,
        Err(e) => {
            info(&format!("copy {src} -> {dst} FAILED: {e}"));
            false
        }
    }
}

/// Stage the AoC runtime into tmpfs so a later vendor unmount cannot pull
/// it out from under the running daemon (P11 `otg_yogi.sh:10-13`).
/// Tries already-mounted `/vendor` (+ `/vendor_dlkm`, `/lib/modules`)
/// first, then dm-mapper mounts with the live slot suffix. Returns true
/// when both `aocd` and the KO are staged.
fn stage_aoc() -> bool {
    let _ = std::fs::create_dir_all(AOC_RAM);
    let _ = std::fs::create_dir_all(AOC_LIB);
    // Fast path: vendor already mounted (common in recovery).
    copy_if_src(AOCD_BIN, VENDOR_AOCD);
    for lib in AOC_LIBS {
        let dst = format!("{AOC_LIB}/{lib}.so");
        let src = format!("{VENDOR_LIBDIR}/{lib}.so");
        copy_if_src(&dst, &src);
    }
    copy_if_src(AOC_KO, VENDOR_DLKM_KO);
    copy_if_src(AOC_KO, FIRSTSTAGE_KO);
    if Path::new(AOCD_BIN).is_file() && Path::new(AOC_KO).is_file() {
        return true;
    }
    // Slow path: dm-mapper mounts (mapper nodes appear late — the daemon
    // retries this from its loop, like the shell script).
    let slot = slot_suffix();
    let mapped_vendor = format!("/dev/block/mapper/vendor{slot}");
    let mapped_dlkm = format!("/dev/block/mapper/vendor_dlkm{slot}");
    if !Path::new(AOCD_BIN).is_file()
        && Path::new(&mapped_vendor).exists()
        && mount_ro(&mapped_vendor, OTG_MNT_VENDOR)
    {
        copy_if_src(AOCD_BIN, &format!("{OTG_MNT_VENDOR}/bin/aocd"));
        for lib in AOC_LIBS {
            let dst = format!("{AOC_LIB}/{lib}.so");
            let src = format!("{OTG_MNT_VENDOR}/lib64/{lib}.so");
            copy_if_src(&dst, &src);
        }
        umount(OTG_MNT_VENDOR);
        use std::os::unix::fs::PermissionsExt;
        if let Ok(md) = std::fs::metadata(AOCD_BIN) {
            let mut perm = md.permissions();
            perm.set_mode(0o755);
            let _ = std::fs::set_permissions(AOCD_BIN, perm);
        }
    }
    if !Path::new(AOC_KO).is_file()
        && Path::new(&mapped_dlkm).exists()
        && mount_ro(&mapped_dlkm, OTG_MNT_VDLKM)
    {
        // `find $MD -name aoc_usb_driver.ko | head -1` in shell.
        let mut found: Option<PathBuf> = None;
        if let Ok(rd) = std::fs::read_dir(OTG_MNT_VDLKM) {
            let mut stack: Vec<PathBuf> =
                rd.flatten().map(|e| e.path()).collect();
            while let Some(p) = stack.pop() {
                if p.is_dir() {
                    if let Ok(inner) = std::fs::read_dir(&p) {
                        for e in inner.flatten() {
                            stack.push(e.path());
                        }
                    }
                } else if p
                    .file_name()
                    .map(|n| n == "aoc_usb_driver.ko")
                    .unwrap_or(false)
                {
                    found = Some(p);
                    break;
                }
            }
        }
        if let Some(ko) = found {
            copy_if_src(AOC_KO, &ko.to_string_lossy());
        }
        umount(OTG_MNT_VDLKM);
    }
    Path::new(AOCD_BIN).is_file() && Path::new(AOC_KO).is_file()
}

/// `verify_aoc_responsive` gate (P11 `otg_yogi.sh:44`): the AoC core
/// reports `responsive` once its firmware is up (local `aoc_core.ko`
/// `responsive`/`Verifying if AOC is responsive...` strings).
fn aoc_responsive() -> bool {
    let mut hits: Vec<PathBuf> = Vec::new();
    if let Ok(rd) = std::fs::read_dir("/sys/devices/platform") {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.ends_with(".aoc") {
                let cand = e.path().join("verify_aoc_responsive");
                if cand.is_file() {
                    hits.push(cand);
                }
            }
        }
    }
    for h in hits {
        if read_trim(&h.to_string_lossy()) == "responsive" {
            return true;
        }
    }
    false
}

/// `usb_control` service gate (P11 `otg_yogi.sh:45`): `aocd` has started
/// the AoC `usb_control` service (local `aoc_usb_driver.ko` `usb_control`
/// ×1). This is the AOC vote becoming HOST in the `google-role-sw`
/// election (mirror `aoc_usb_dev.c:247`).
fn aoc_has_usb_control() -> bool {
    if let Ok(rd) = std::fs::read_dir("/sys/devices/platform") {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if !name.ends_with(".aoc") {
                continue;
            }
            let svc = e.path().join("services");
            // `services` may be a file or a dir per instance; handle both.
            if svc.is_file() {
                if read_trim(&svc.to_string_lossy()).contains("usb_control") {
                    return true;
                }
            } else if svc.is_dir() {
                if let Ok(inner) = std::fs::read_dir(&svc) {
                    for f in inner.flatten() {
                        let content = read_trim(&f.path().to_string_lossy());
                        if content.contains("usb_control") {
                            return true;
                        }
                    }
                }
            }
            // Fallback: recursive find for a `services` file below the
            // platform device (mirrors shell `find ... -name services`).
            let mut stack = vec![e.path()];
            while let Some(p) = stack.pop() {
                if p.is_dir() {
                    if let Ok(inner) = std::fs::read_dir(&p) {
                        for ent in inner.flatten() {
                            let ep = ent.path();
                            if ep
                                .file_name()
                                .map(|n| n == "services")
                                .unwrap_or(false)
                            {
                                if read_trim(&ep.to_string_lossy())
                                    .contains("usb_control")
                                {
                                    return true;
                                }
                            } else if ep.is_dir() {
                                stack.push(ep);
                            }
                        }
                    }
                }
            }
        }
    }
    false
}

fn aocd_running() -> bool {
    if let Ok(out) = std::process::Command::new("pidof").arg("aocd").output() {
        if out.status.success() && !out.stdout.is_empty() {
            return true;
        }
    }
    // Fallback: scan /proc for an `aocd` comm (no `pidof` in minimal envs).
    if let Ok(rd) = std::fs::read_dir("/proc") {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if !name.bytes().all(|b| b.is_ascii_digit()) {
                continue;
            }
            let comm = format!("/proc/{name}/comm");
            if read_trim(&comm) == "aocd" {
                return true;
            }
        }
    }
    false
}

fn kill_aocd() {
    if let Ok(out) = std::process::Command::new("pidof").arg("aocd").output() {
        let pids = String::from_utf8_lossy(&out.stdout).into_owned();
        for tok in pids.split_whitespace() {
            if let Ok(pid) = tok.parse::<i32>() {
                // SAFETY: kill(2) with SIGKILL on a live pidof result; ESRCH
                // and permission errors are ignored (best-effort relaunch).
                unsafe {
                    libc::kill(pid, libc::SIGKILL);
                }
            }
        }
    }
}

/// Launch the staged `aocd` with the staged libs first in the loader path
/// (P11 `otg_yogi.sh:74`: `LD_LIBRARY_PATH="$RAM/lib":/system/lib64`).
fn launch_aocd() {
    if !Path::new(AOCD_BIN).is_file() {
        return;
    }
    use std::os::unix::fs::PermissionsExt;
    if let Ok(md) = std::fs::metadata(AOCD_BIN) {
        let mut perm = md.permissions();
        perm.set_mode(0o755);
        let _ = std::fs::set_permissions(AOCD_BIN, perm);
    }
    let libs = format!("{AOC_LIB}:/system/lib64");
    match std::process::Command::new(AOCD_BIN)
        .env("LD_LIBRARY_PATH", libs)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(_) => info("aocd launched from staged RAM copy"),
        Err(e) => info(&format!("aocd launch FAILED: {e}")),
    }
}

/// One AoC bring-up attempt: insmod the staged KO (strict: matches the
/// running kernel by construction) then, if `usb_control` is missing but
/// the AoC is responsive, relaunch `aocd` (flaky startup — a fresh
/// instance is what eventually gets the service up, P11 `otg_yogi.sh:60`).
fn aoc_bringup_once() -> bool {
    if !is_module_loaded("aoc_usb_driver") && Path::new(AOC_KO).is_file() {
        match load_kernel_module(Path::new(AOC_KO)) {
            Ok(()) => info("aoc_usb_driver loaded from staged RAM copy"),
            Err(e) => info(&format!("staged aoc_usb_driver FAILED: {e}")),
        }
    }
    if aoc_has_usb_control() {
        info("aocd up; AOC vote live (usb_control present)");
        return true;
    }
    if aoc_responsive() {
        info("AoC responsive but no usb_control; relaunching aocd (flaky startup)");
        kill_aocd();
        launch_aocd();
        sleep(Duration::from_secs(4));
        if aoc_has_usb_control() {
            info("aocd up; AOC vote live (usb_control present)");
            return true;
        }
        info("usb_control still missing after relaunch (will retry from daemon)");
    } else {
        info("AoC not responsive yet (firmware still coming up?)");
    }
    false
}

/// First removable USB disk (sdX1 preferred, else whole disk). Shared with
/// the zuma/gs R11 algorithm; uses `as_bytes().get()` per clippy.
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

/// sys.usb.controller wins only when it names a real /sys/class/udc entry
/// (else the blanked UNKNOWN literal would kill a bind). Laguna fallback
/// is the DTB `dwc3@c400000` child (`UDC_FALLBACK`), not zuma's 11210000.
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
    sysfs.into_iter().next().unwrap_or_else(|| UDC_FALLBACK.to_string())
}

/// laguna `otg-patch` (oneshot): stock chain → TCPC note → AoC staging →
/// verified host. Sets `sys.usb.patch_dwc3=1` ONLY on verified host
/// (role+data_role+psy); `0` + `Err` otherwise (device mode, adb safe).
pub fn run_otg_patch() -> Result<(), String> {
    let gen = detect_generation();
    info(&format!("starting OTG patch routine (laguna, gen={gen:?})"));
    match gen {
        Gen::Unknown => {
            info("unsupported kernel generation (need 6.6 stable or 6.12 beta4), staying in device mode");
            let _ = set_prop("sys.usb.patch_dwc3", "0");
            return Err("laguna otg: unsupported kernel generation".into());
        }
        Gen::V66 => info("kernel 6.6 stable branch (cp2a/bp4a firmware composition)"),
        Gen::V612 => info("kernel 6.12 beta4 branch (same election mechanism, 6.12 module set)"),
    }

    // 1. Stock USB chain from the LIVE firmware (dep order). Best-effort
    // but loud: the summary tells exactly which half is missing.
    let mut chain_ok = true;
    for m in STOCK_USB_CHAIN {
        chain_ok &= insmod_stock(m);
    }
    info(&format!("chain done (all_found={chain_ok})"));

    // 2. TCPC data-path: SPMI-owned by design (DTBO `max77759tcpc-spmi@4`,
    // driver `spmi-max77759tcpc`). The i2c helper's SPMI fallback
    // recognizes the probed SPMI client as success ("driver-managed phy
    // mux") — warning only either way, never a host gate.
    match patch_max77759_i2c_with_driver("max77759tcpc-spmi") {
        Ok(()) => info("TCPC path OK (SPMI driver-managed or I2C patched)"),
        Err(e) => info(&format!("TCPC switch not configured: {e}")),
    }

    // 3. AoC staging (bounded in the oneshot; the daemon retries forever).
    // Handles both already-mounted /vendor and dm-mapper mounts.
    let mut staged = Path::new(AOCD_BIN).is_file() && Path::new(AOC_KO).is_file();
    for attempt in 1..=3 {
        if staged {
            break;
        }
        info(&format!("AoC staging attempt {attempt}/3"));
        staged = stage_aoc();
        if !staged {
            sleep(Duration::from_secs(2));
        }
    }
    if staged {
        info("staged aocd + libs + module into RAM (/tmp/aoc)");
    } else {
        info("AoC staging incomplete (mapper nodes not up yet?)");
    }

    // 4. AoC bring-up (bounded): insmod + flaky-startup relaunch.
    let mut aoc_ok = aoc_has_usb_control();
    for attempt in 1..=3 {
        if aoc_ok {
            break;
        }
        info(&format!("AoC bring-up attempt {attempt}/3"));
        aoc_ok = aoc_bringup_once();
        if !aoc_ok {
            sleep(Duration::from_secs(2));
        }
    }

    // 5. Verified-host gate (never role alone).
    if aoc_ok && verify_host() {
        let udc = resolve_udc();
        info(&format!("laguna host path ACTIVE (UDC {udc}), sys.usb.patch_dwc3=1"));
        set_prop("sys.usb.patch_dwc3", "1").map_err(|e| e.to_string())?;
        info("OTG patch routine finished");
        Ok(())
    } else {
        info("host NOT verified (need usb_control + role=host + data_role=host + psy=1)");
        let _ = set_prop("sys.usb.patch_dwc3", "0");
        Err("laguna otg: host not verified (staying in device mode)".into())
    }
}

/// laguna `otg-auto`: AoC watchdog + `/dev/block/otg-usb` maintenance.
/// Never exits. Performs NO role/UDC writes by design: the
/// `google-role-sw` election owns the data role (mirror truth table:
/// host needs TCPCI+AOC votes); direct downstream writes are a hardware
/// no-op per field lesson, so forcing them would only fake success.
/// VBUS/typec state is logged for observability; the symlink keeps
/// `/usb_otg` on the current removable disk.
pub fn run_otg_auto() -> ! {
    info("starting (laguna AoC watchdog + otg-usb maintenance, no role writes)");
    info(&format!(
        "patch gate sys.usb.patch_dwc3={}",
        get_prop("sys.usb.patch_dwc3")
    ));
    info("waiting 3s for boot to settle");
    sleep(Duration::from_secs(3));
    loop {
        if !(Path::new(AOCD_BIN).is_file() && Path::new(AOC_KO).is_file())
            && stage_aoc()
        {
            info("staged aocd + libs + module into RAM");
        }
        if Path::new(AOCD_BIN).is_file() && Path::new(AOC_KO).is_file() {
            if !is_module_loaded("aoc_usb_driver") && Path::new(AOC_KO).is_file() {
                match load_kernel_module(Path::new(AOC_KO)) {
                    Ok(()) => info("aoc_usb_driver loaded from staged RAM copy"),
                    Err(e) => info(&format!("staged aoc_usb_driver FAILED: {e}")),
                }
            }
            if aoc_has_usb_control() {
                info("aocd up; AOC vote live");
            } else if aoc_responsive() {
                info("AoC responsive without usb_control; relaunching aocd");
                kill_aocd();
                launch_aocd();
                sleep(Duration::from_secs(4));
                if aoc_has_usb_control() {
                    info("aocd up; AOC vote live");
                }
            } else if !aocd_running() {
                info("aocd not running; launching staged copy");
                launch_aocd();
                sleep(Duration::from_secs(4));
            }
            // Observability only: log the election-visible state, never
            // force it (direct writes are a no-op on this SoC).
            let role = find_usb_role()
                .map(|p| read_trim(&p.to_string_lossy()))
                .unwrap_or_default();
            let data_role = find_typec_data_role()
                .map(|p| read_trim(&p.to_string_lossy()))
                .unwrap_or_default();
            let (psy, _) = read_source_psy();
            if !role.is_empty() || !data_role.is_empty() || !psy.is_empty() {
                info(&format!("state: role={role:?} data_role={data_role:?} psy={psy:?}"));
            }
            ensure_otg_symlink();
        }
        sleep(Duration::from_secs(3));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_gate_covers_both_laguna_kernels() {
        // Local boot.img versions: cp2a/bp4a stable 6.6.x, beta4 6.12.81.
        assert_eq!(classify_generation(6, 6), Gen::V66);
        assert_eq!(classify_generation(6, 12), Gen::V612);
        // Anything else (zuma 6.1, future 6.14, unknown 0.0) fails closed.
        assert_eq!(classify_generation(6, 1), Gen::Unknown);
        assert_eq!(classify_generation(6, 14), Gen::Unknown);
        assert_eq!(classify_generation(0, 0), Gen::Unknown);
    }

    #[test]
    fn role_entry_picks_first() {
        let two = vec!["c400000.usb-role-switch".to_string(), "other".to_string()];
        assert_eq!(
            pick_role_entry(&two),
            Some("c400000.usb-role-switch".to_string())
        );
        let empty: Vec<String> = Vec::new();
        assert_eq!(pick_role_entry(&empty), None);
    }

    #[test]
    fn host_needs_all_three_legs() {
        // Prior field lesson: never role alone.
        assert!(is_host_verified("host", "host", "1"));
        assert!(!is_host_verified("host", "host", "0"));
        assert!(!is_host_verified("host", "device", "1"));
        assert!(!is_host_verified("device", "host", "1"));
        assert!(!is_host_verified("", "host", "1"));
        // Sysfs reads carry trailing newlines — trim covers them.
        assert!(is_host_verified("host\n", "host\n", "1\n"));
    }

    #[test]
    fn udc_fallback_matches_laguna_dwc3_base() {
        // DTB `dwc3@c400000` (reg 0xc400000) — never zuma's 11210000.
        assert!(UDC_FALLBACK.contains("c400000"));
        assert!(!UDC_FALLBACK.contains("11210000"));
    }
}
