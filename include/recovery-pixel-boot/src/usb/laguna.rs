//! usb::laguna — OTG implementation (Tensor G5: frankel/blazer/mustang/rango).
//!
//! Election-driven host mode, zuma-style driver manipulation — NO vendor
//! staging by design (no siw/lptools/iw, no dm-mapper, no vendor mounts,
//! no aocd, no AoC sysfs polling; the old staging chain hung the daemon
//! on wedged `services`/`verify_aoc_responsive` reads and is gone).
//!
//! Mechanism (disassembly-verified on mustang 6.6.98/6.12.81
//! `google-role-sw.ko` + `aoc_usb_driver.ko`; DTBO `fragment@usb_data_role_switch`):
//! - `google-role-sw` election `USB_DR_EL`: host requires `TCPCI=host AND
//!   AOC=host` (else `NONE`); `DISABLE_USB_DATA` forces `NONE`.
//! - The AOC vote is cast by stock `aoc_usb_driver` only once the AoC
//!   `usb_control` service runs (userspace `aocd`) — which never happens
//!   in recovery (AoC firmware: `deferred probe pending` forever).
//! - Our `aoc_vote_shim` (`include/source/aoc_vote_shim/`) casts the same
//!   vote the stock probe casts for the `disable-aoc-voter` DT boolean:
//!   `get_handle("USB_DR_EL")` + `cast_vote(h, "AOC", 1, 1)`.
//!   The vote is sticky (field-proven: mustang keeps host across
//!   `stop aocd` + replug), so one cast replaces the whole AoC stack.
//! - The kernel self-switches via the election once both votes are host,
//!   so this code performs NO role/UDC writes (direct downstream writes
//!   are reverted by the election — hardware no-op per field lesson).
//! - `aoc_core`/`aoc_usb_driver` are deliberately NOT insmodded: their
//!   only job was the vote, and a loaded stock driver re-casts (0,1)
//!   while the firmware is down, fighting the shim. First-stage may have
//!   pre-loaded them — the shim's delayed re-asserts cover that race.
//!
//! Two kernel generations (6.6 stable, 6.12 beta): same election, same
//! shim source; only the prebuilt vermagic differs (major.minor is
//! enough for our own shim via the force-load fallback).
//! Anything else fails closed (device mode, adb safe).
//!
//! Success criteria (observable in recovery, never role alone):
//! downstream `role == host` AND Type-C `data_role == host` AND a source
//! power_supply `online == 1`. `patch_dwc3=1` is set only when the shim
//! is loaded (vote lands via its internal retries); otherwise `0` + `Err`.

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
/// Registered by `dwc3-google.ko`; the control file itself is
/// runtime-discovered, this is only the class dir.
const USB_ROLE_CLASS: &str = "/sys/class/usb_role";
/// TCPM core class (`data_role` is the election-visible host signal,
/// never written here).
const TYPEC_CLASS: &str = "/sys/class/typec";
/// Source-online candidates (`gcpm` first: DTBO `chg-psy-name = "gcpm"`).
const SOURCE_PSY_PATHS: &[&str] = &[
    "/sys/class/power_supply/gcpm/online",
    "/sys/class/power_supply/usb/online",
    "/sys/class/power_supply/usb/present",
];
/// UDC gadget path. Documented, never written on laguna by design.
#[allow(dead_code)]
const UDC_FILE: &str = "/config/usb_gadget/g1/UDC";
/// Fallback when `/sys/class/udc` is empty (DTB `dwc3@c400000`).
/// Reference only (test anchor; never written on laguna by design).
#[allow(dead_code)]
const UDC_FALLBACK: &str = "c400000.dwc3";
const OTG_USB_LINK: &str = "/dev/block/otg-usb";
/// Our vote shim (prebuilt `aoc_vote_shim*.ko` under
/// `/system/lib64/modules`, picked best-first for the running kernel;
/// force-load fallback covers vermagic drift — our code only).
const SHIM_MODULE: &str = "aoc_vote_shim";
const PROC_SHIM: &str = "/proc/aoc_vote_shim";
const PROC_READY: &str = "/proc/aoc_vote_ready";
/// Shim vote settle: the shim retries the election internally, one short
/// userspace settle is enough for the readback.
const SHIM_SETTLE_SECS: u64 = 2;
/// Stock USB chain in dep order (strict flags: stock modules match the
/// running kernel by construction). `aoc_core`/`aoc_usb_driver` are
/// intentionally ABSENT (see module docs); the shim owns the AOC vote.
/// Missing files are skipped LOUDLY.
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
    /// 6.6 stable (mustang cp2a/bp4a).
    V66,
    /// 6.12 beta (mustang beta4).
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

/// Version gate: laguna spans exactly 6.6 (stable) and 6.12 (beta).
/// Anything else is unverified → fail closed. Pure over (major, minor).
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
/// `dwc3-google.ko`). Pure over entry names (testable).
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

/// Live Type-C `data_role` file, if TCPM registered a port.
fn find_typec_data_role() -> Option<PathBuf> {
    let names: Vec<String> = std::fs::read_dir(TYPEC_CLASS)
        .ok()?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    let mut ordered: Vec<String> = Vec::new();
    if let Some(p) = names.iter().find(|n| n.starts_with("port")).cloned() {
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
/// All three must read host/online: role alone is never enough.
fn is_host_verified(role: &str, data_role: &str, psy_online: &str) -> bool {
    role.trim() == "host" && data_role.trim() == "host" && psy_online.trim() == "1"
}

/// Best source-online reading among the candidate PSYs. Returns the raw
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
/// `data_role == host` AND source PSY `online == 1`. Loud about each leg.
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

/// Best-effort insmod of one STOCK module (strict flags). Returns true
/// when loaded or already loaded.
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

/// The shim vote is asserted once `/proc/aoc_vote_ready` reads `1`.
/// Pure readback of the shim's own state (testable shape, live path).
fn vote_asserted() -> bool {
    read_trim(PROC_READY) == "1"
}

/// Load our vote shim: strict first (works when vermagic matches), then
/// forced (IGNORE_VERMAGIC/MODVERSIONS — our code only, same contract as
/// the zuma `otg_host_shim`). Pokes the vote and settles briefly; the
/// shim retries the election internally, so no polling here.
fn load_vote_shim() -> bool {
    if vote_asserted() {
        info("vote shim already active (/proc/aoc_vote_ready=1)");
        return true;
    }
    if Path::new(PROC_SHIM).exists() {
        info("vote shim loaded, vote pending — poking");
    } else if ko_try_load(SHIM_MODULE, Some(PROC_SHIM), TAG) {
        info("vote shim loaded (strict)");
    } else {
        info("strict shim load failed (vermagic?), trying forced load");
        let env = match detect_kernel_env() {
            Ok(e) => e,
            Err(e) => {
                info(&format!("detect_kernel_env failed: {e}"));
                return false;
            }
        };
        let mut ok = false;
        for cand in find_candidates(Path::new("/system/lib64/modules"), SHIM_MODULE, &env) {
            info(&format!("force-trying {}", cand.display()));
            match load_kernel_module_force(&cand) {
                Ok(()) => {
                    info(&format!("vote shim force-loaded {}", cand.display()));
                    ok = true;
                    break;
                }
                Err(e) => info(&format!("force load failed: {e}")),
            }
        }
        if !ok {
            return false;
        }
    }
    if Path::new(PROC_SHIM).exists() {
        let _ = std::fs::write(PROC_SHIM, b"1\n");
        sleep(Duration::from_secs(SHIM_SETTLE_SECS));
    }
    if vote_asserted() {
        info("AOC vote live (shim asserted)");
        true
    } else {
        info("vote NOT asserted yet (shim still acquiring election? will retry from daemon)");
        Path::new(PROC_SHIM).exists()
    }
}

/// First removable USB disk (sdX1 preferred, else whole disk). Shared
/// zuma/gs algorithm; uses `as_bytes().get()` per clippy.
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
        // Canonicalize first so the "usb" check sees the real
        // /sys/devices/... path (field-proven: the raw link matched
        // nothing, otg-usb was never created).
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

/// laguna `otg-patch` (oneshot): stock chain preload, TCPC note, vote
/// shim — then releases the daemon. No staging, no sleeps-in-bringup
/// (the shim retries the election internally). Sets
/// `sys.usb.patch_dwc3=1` to start `otg_auto`; device mode (adb) always
/// survives regardless.
pub fn run_otg_patch() -> Result<(), String> {
    let gen = detect_generation();
    info(&format!("starting OTG patch routine (laguna, gen={gen:?})"));
    match gen {
        Gen::Unknown => {
            info("unsupported kernel generation (need 6.6 stable or 6.12 beta), staying in device mode");
            let _ = set_prop("sys.usb.patch_dwc3", "0");
            return Err("laguna otg: unsupported kernel generation".into());
        }
        Gen::V66 => info("kernel 6.6 stable branch"),
        Gen::V612 => info("kernel 6.12 beta branch (same election, 6.12 shim prebuilt)"),
    }

    // 1. Stock USB chain from the LIVE firmware (dep order). Best-effort
    // but loud: the summary tells exactly which half is missing.
    let mut chain_ok = true;
    for m in STOCK_USB_CHAIN {
        chain_ok &= insmod_stock(m);
    }
    info(&format!("chain done (all_found={chain_ok})"));

    // 2. TCPC data-path: SPMI-owned by design (DTBO `max77759tcpc-spmi@4`).
    // Warning only either way, never a host gate.
    match patch_max77759_i2c_with_driver("max77759tcpc-spmi") {
        Ok(()) => info("TCPC path OK (SPMI driver-managed or I2C patched)"),
        Err(e) => info(&format!("TCPC switch not configured: {e}")),
    }

    // 3. The AOC vote (replaces the entire AoC/aocd stack).
    if !load_vote_shim() {
        let _ = set_prop("sys.usb.patch_dwc3", "0");
        return Err("laguna otg: vote shim unavailable".into());
    }
    set_prop("sys.usb.patch_dwc3", "1").map_err(|e| e.to_string())?;
    info("OTG patch routine finished, daemon released (sys.usb.patch_dwc3=1)");
    Ok(())
}

/// laguna `otg-auto`: vote keeper + host verify + `/dev/block/otg-usb`
/// maintenance. Never exits. Performs NO role/UDC writes by design: the
/// `google-role-sw` election owns the data role; VBUS/typec state is
/// logged for observability.
pub fn run_otg_auto() -> ! {
    info("starting (laguna vote keeper + otg-usb maintenance, no role writes)");
    info(&format!(
        "patch gate sys.usb.patch_dwc3={}",
        get_prop("sys.usb.patch_dwc3")
    ));
    info("waiting 3s for boot to settle");
    sleep(Duration::from_secs(3));
    // Edge-triggered host verification (loud legs via verify_host, no
    // per-tick spam).
    let mut last_state = (String::new(), String::new(), String::new());
    let mut was_verified = false;
    loop {
        // Keeper: the vote is sticky, but re-poke if the shim reports
        // it dropped (e.g. election re-created late).
        if !vote_asserted() {
            if Path::new(PROC_SHIM).exists() {
                info("daemon: vote not asserted, re-poking shim");
                let _ = std::fs::write(PROC_SHIM, b"1\n");
            } else {
                info("daemon: vote shim missing, reloading");
                load_vote_shim();
            }
        }
        // Verify on state change only; verify_host() logs every leg.
        let role = find_usb_role()
            .map(|p| read_trim(&p.to_string_lossy()))
            .unwrap_or_default();
        let data_role = find_typec_data_role()
            .map(|p| read_trim(&p.to_string_lossy()))
            .unwrap_or_default();
        let (psy, _) = read_source_psy();
        let triple = (role, data_role, psy);
        if triple != last_state
            && (!triple.0.is_empty() || !triple.1.is_empty() || !triple.2.is_empty())
        {
            last_state = triple.clone();
            verify_host();
        }
        let now_verified = is_host_verified(&triple.0, &triple.1, &triple.2);
        if now_verified && !was_verified {
            info("daemon: HOST verified (role+data_role+psy) — serving attach");
        }
        was_verified = now_verified;
        ensure_otg_symlink();
        sleep(Duration::from_secs(3));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_gate_covers_both_laguna_kernels() {
        // mustang cp2a/bp4a stable 6.6.x, beta 6.12.81.
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

    #[test]
    fn stock_chain_has_no_aoc_modules() {
        // The shim owns the AOC vote; insmodding the stock AoC drivers
        // would re-cast (0,1) against us while the firmware is down.
        for m in STOCK_USB_CHAIN {
            assert!(!m.contains("aoc"), "{m} must not be in the chain");
        }
        // The election driver itself must be present.
        assert!(STOCK_USB_CHAIN.contains(&"google-role-sw"));
        assert!(STOCK_USB_CHAIN.contains(&"dwc3-google"));
    }
}
