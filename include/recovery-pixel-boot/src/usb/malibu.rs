//! usb::malibu — OTG implementation (Tensor G6: grizzly/cubs/kodiak/yogi).
//!
//! Election-driven host mode, zuma-style driver manipulation — NO vendor
//! staging by design (no siw/loop/dm-mapper, no vendor mounts, no aocd,
//! no AoC sysfs polling).
//!
//! Mechanism (disassembly-verified on grizzly/yogi 6.12.69
//! `google-role-sw.ko` + `aoc_usb_driver.ko`; DTBO
//! `fragment@usb_data_role_switch`): identical to laguna — `google-role-sw`
//! election `USB_DR_EL` needs `TCPCI=host AND AOC=host`, and our
//! `aoc_vote_shim` (`include/source/aoc_vote_shim/`, shared with laguna:
//! same election name, same voter, same constants) casts the AOC vote
//! directly: `get_handle("USB_DR_EL")` + `cast_vote(h, "AOC", 1, 1)`.
//!
//! Malibu bonus, verified in stock DTBO (grizzly 46 overlays, yogi 9):
//! `host-mode-aoc-optional` is PRESENT on `goog_usb_role_sw`, which the
//! stock `aoc_usb_probe` reads via
//! `of_find_compatible_node("google,usb-role-sw")` and treats as
//! "AoC not required". So on stock malibu the gate is open from boot
//! and the shim is belt-and-suspenders (best-effort, never a failure
//! gate): even a missing shim prebuilt keeps patch green.
//!
//! The kernel self-switches via the election, so this code performs NO
//! role/UDC writes and NO i2c TCPC patch (SPMI bus: `max77759tcpc-spmi@4`,
//! self-managed by the stock driver). `aoc_core`/`aoc_usb_driver` are
//! deliberately NOT insmodded (their only job was the vote).
//! 6.12-only family; anything else fails closed (device mode, adb safe).
//!
//! Success predicate (field-proven, never role alone): controller
//! `role == host` AND TCPC source psy `online == 1`.

use crate::ko_picker::{
    detect_kernel_env, find_candidates, is_module_loaded, ko_try_load,
    load_kernel_module, load_kernel_module_force, log_msg,
};
use crate::props::set_prop;
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::Duration;

const TAG: &str = "otg";
/// Role-switch class (live entries `a200000.usb3-role-switch`,
/// `i2c-8-003e-eusb2-repeater-role-switch`).
const USB_ROLE_CLASS: &str = "/sys/class/usb_role";
/// Preferred role-switch entry: the DWC3 controller switch.
const ROLE_PREFER: &str = "usb3";
/// Source psy class + exact psy name (live chg log:
/// `chg_psy_changed name=tcpm-source-psy-spmi-max77759tcpc`).
const PSY_CLASS: &str = "/sys/class/power_supply";
const SOURCE_PSY: &str = "tcpm-source-psy-spmi-max77759tcpc";
/// UDC fallback: base DTB `dwc3@a210000` under `usb3@a200000`.
/// Reference only (never written on malibu by design).
#[allow(dead_code)]
const UDC_FALLBACK: &str = "a210000.dwc3";
/// Stable symlink for `/usb_otg`.
const OTG_USB_LINK: &str = "/dev/block/otg-usb";
/// Our vote shim (shared prebuilt with laguna; best-effort here — the
/// stock DT already opens the gate, see module docs).
const SHIM_MODULE: &str = "aoc_vote_shim";
const PROC_SHIM: &str = "/proc/aoc_vote_shim";
const PROC_READY: &str = "/proc/aoc_vote_ready";
/// Shim vote settle: the shim retries the election internally.
const SHIM_SETTLE_SECS: u64 = 2;
/// Supervisor tick.
const TICK_SECS: u64 = 3;
/// Stock USB chain in dep order (strict flags: stock modules match the
/// running kernel by construction). First-stage (`vendor_kernel_boot`
/// `modules.load`) normally autoloads all of these before we run, so
/// this is a safety net. `aoc_core`/`aoc_usb_driver` intentionally
/// ABSENT (shim owns the vote). Missing files are skipped LOUDLY.
const STOCK_USB_CHAIN: &[&str] = &[
    "gvotable",
    "google_icc",
    "google-usb-phy",
    "google-role-sw",
    "dwc3-google",
    "dwc3-google-aux",
    "usb_psy",
    "tcpci_max77759",
    "tcpci_max777x9_spmi",
    "max77779-charger",
    "max77779-charger-spmi",
    "google-charger",
];
/// First-stage ramdisk first (see above).
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

/// Malibu exists only on 6.12. Pure over (major, minor) (testable).
fn kernel_supported(major: u32, minor: u32) -> bool {
    (major, minor) == (6, 12)
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
fn vote_asserted() -> bool {
    read_trim(PROC_READY) == "1"
}

/// Load our vote shim (best-effort on malibu: stock DT already opens the
/// gate). Strict first, then forced (our code only). Pokes the vote and
/// settles briefly; the shim retries the election internally.
fn load_vote_shim() {
    if vote_asserted() {
        info("vote shim already active (/proc/aoc_vote_ready=1)");
        return;
    }
    if Path::new(PROC_SHIM).exists() {
        info("vote shim loaded, vote pending — poking");
    } else if ko_try_load(SHIM_MODULE, Some(PROC_SHIM), TAG) {
        info("vote shim loaded (strict)");
    } else {
        info("strict shim load failed (vermagic?), trying forced load");
        match detect_kernel_env() {
            Ok(env) => {
                for cand in find_candidates(Path::new("/system/lib64/modules"), SHIM_MODULE, &env)
                {
                    info(&format!("force-trying {}", cand.display()));
                    match load_kernel_module_force(&cand) {
                        Ok(()) => {
                            info(&format!("vote shim force-loaded {}", cand.display()));
                            break;
                        }
                        Err(e) => info(&format!("force load failed: {e}")),
                    }
                }
            }
            Err(e) => info(&format!("detect_kernel_env failed: {e}")),
        }
    }
    if Path::new(PROC_SHIM).exists() {
        let _ = std::fs::write(PROC_SHIM, b"1\n");
        sleep(Duration::from_secs(SHIM_SETTLE_SECS));
    }
    if vote_asserted() {
        info("AOC vote live (shim asserted)");
    } else {
        info("shim vote not asserted (stock DT gate still carries host)");
    }
}

/// Controller role-switch `role` file: prefer the DWC3 controller entry
/// (`a200000.usb3-role-switch`), else first sorted entry. Pure over names
/// (testable); live readdir in [`find_data_role`].
fn pick_role_entry(entries: &[String]) -> Option<String> {
    if entries.is_empty() {
        return None;
    }
    entries
        .iter()
        .find(|n| n.contains(ROLE_PREFER))
        .or_else(|| entries.first())
        .cloned()
}

/// Live data-role control file.
fn find_data_role() -> Option<PathBuf> {
    let mut names: Vec<String> = std::fs::read_dir(USB_ROLE_CLASS)
        .ok()?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    pick_role_entry(&names).map(|n| Path::new(USB_ROLE_CLASS).join(n).join("role"))
}

/// Current controller data role (`host`/`device`/…).
fn read_data_role() -> String {
    find_data_role()
        .map(|p| read_trim(&p.to_string_lossy()))
        .unwrap_or_default()
}

/// Source psy `online` file: exact live name wins, then any
/// `*tcpm-source*`, then any `*source*` (name drifts across spins).
/// Pure over names (testable).
fn pick_source_psy(entries: &[String]) -> Option<String> {
    if entries.is_empty() {
        return None;
    }
    entries
        .iter()
        .find(|n| *n == SOURCE_PSY)
        .or_else(|| entries.iter().find(|n| n.contains("tcpm-source")))
        .or_else(|| entries.iter().find(|n| n.contains("source")))
        .cloned()
}

/// Live source psy `online` file.
fn find_source_psy() -> Option<PathBuf> {
    let names: Vec<String> = std::fs::read_dir(PSY_CLASS)
        .ok()?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    pick_source_psy(&names).map(|n| Path::new(PSY_CLASS).join(n).join("online"))
}

/// TCPC-sourced VBUS state (`1` = sourcing).
fn read_source_online() -> String {
    find_source_psy()
        .map(|p| read_trim(&p.to_string_lossy()))
        .unwrap_or_default()
}

/// Host path ACTIVE only on BOTH signals — never the role alone: a bare
/// `role=host` readback with no sourced VBUS is not host mode.
fn host_active() -> bool {
    read_data_role() == "host" && read_source_online() == "1"
}

/// First removable USB disk (`sdX1` preferred, else whole disk).
/// Pure core over candidates (testable); live sysfs walk in [`pick_otg_disk`].
fn choose_otg_disk(cands: &[(&str, bool, bool, bool)]) -> Option<String> {
    let mut names: Vec<&str> = cands
        .iter()
        .filter(|(_, removable, has_usb, _)| *removable && *has_usb)
        .map(|(n, _, _, _)| *n)
        .collect();
    names.sort();
    names.first().map(|n| {
        let has_part = cands
            .iter()
            .find(|(m, _, _, _)| m == n)
            .map(|(_, _, _, p)| *p)
            .unwrap_or(false);
        if has_part {
            format!("/dev/block/{n}1")
        } else {
            format!("/dev/block/{n}")
        }
    })
}

/// Live removable-USB-disk pick.
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
    let mut cands: Vec<(String, bool, bool, bool)> = Vec::new();
    for n in &names {
        let dir = Path::new("/sys/block").join(n);
        let removable = read_trim(&dir.join("removable").to_string_lossy()) == "1";
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
        let has_usb = dev.contains("usb");
        let has_part = Path::new(&format!("/dev/block/{n}1")).exists();
        cands.push((n.clone(), removable, has_usb, has_part));
    }
    let refs: Vec<(&str, bool, bool, bool)> = cands
        .iter()
        .map(|(n, r, u, p)| (n.as_str(), *r, *u, *p))
        .collect();
    choose_otg_disk(&refs)
}

/// Keep `/dev/block/otg-usb` (`/usb_otg`) on the current OTG disk; relink
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

/// otg-patch (oneshot, service `otg_enable`): stock chain preload, vote
/// shim (best-effort — stock DT carries host), then releases the
/// supervisor. No staging, no sleeps-in-bringup. Sets
/// `sys.usb.patch_dwc3=1` to start `otg_auto`; device mode (adb) always
/// survives regardless.
pub fn run_otg_patch() -> Result<(), String> {
    info("starting OTG patch routine (malibu: chain + vote shim)");
    match crate::ko_picker::detect_kernel_env() {
        Ok(e) if kernel_supported(e.major, e.minor) => {
            info(&format!("kernel {}.{} (malibu 6.12-only path)", e.major, e.minor));
        }
        Ok(e) => {
            info(&format!(
                "kernel {}.{}: malibu host path UNVERIFIED here, staying in device mode",
                e.major, e.minor
            ));
            let _ = set_prop("sys.usb.patch_dwc3", "0");
            return Err("malibu otg: unsupported kernel (needs 6.12 role-switch)".into());
        }
        Err(e) => {
            info(&format!("detect_kernel_env failed ({e}), staying in device mode"));
            let _ = set_prop("sys.usb.patch_dwc3", "0");
            return Err("malibu otg: kernel env unknown".into());
        }
    }

    // Stock USB chain from the LIVE firmware (dep order; first-stage
    // normally pre-loads all of it — this is a safety net).
    let mut chain_ok = true;
    for m in STOCK_USB_CHAIN {
        chain_ok &= insmod_stock(m);
    }
    info(&format!("chain done (all_found={chain_ok})"));

    // No i2c TCPC patch by design (SPMI bus, stock driver self-manages).
    // The AOC vote: best-effort shim on top of the stock-DT gate.
    load_vote_shim();
    set_prop("sys.usb.patch_dwc3", "1").map_err(|e| e.to_string())?;
    info("OTG patch routine finished, supervisor released (sys.usb.patch_dwc3=1)");
    Ok(())
}

/// otg-auto (service `otg_auto`): vote keeper + host observe +
/// `otg-usb` symlink. Never exits. Never touches UDC/gadget/role/voter
/// state — TCPC + role-sw negotiate modes on their own, so device mode
/// (adb) always survives.
pub fn run_otg_auto() -> ! {
    info("starting (malibu supervisor: vote keeper + otg-usb symlink)");
    loop {
        // Keeper: re-poke if the shim reports the vote dropped.
        if !vote_asserted() && Path::new(PROC_SHIM).exists() {
            info("supervisor: vote not asserted, re-poking shim");
            let _ = std::fs::write(PROC_SHIM, b"1\n");
        }
        if pick_otg_disk().is_some() {
            info(&format!(
                "disk present: data_role={:?} source_online={:?} host_active={}",
                read_data_role(),
                read_source_online(),
                host_active()
            ));
        }
        ensure_otg_symlink();
        sleep(Duration::from_secs(TICK_SECS));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_gate_is_612_only() {
        assert!(kernel_supported(6, 12));
        assert!(!kernel_supported(6, 6));
        assert!(!kernel_supported(6, 1));
        assert!(!kernel_supported(0, 0));
    }

    #[test]
    fn role_entry_prefers_controller() {
        // Live kodiak entries: controller switch wins over the repeater one.
        let two = vec![
            "i2c-8-003e-eusb2-repeater-role-switch".to_string(),
            "a200000.usb3-role-switch".to_string(),
        ];
        assert_eq!(
            pick_role_entry(&two),
            Some("a200000.usb3-role-switch".to_string())
        );
        // No controller entry: first entry (zuma-compatible fallback).
        let one = vec!["other-switch".to_string()];
        assert_eq!(pick_role_entry(&one), Some("other-switch".to_string()));
        let empty: Vec<String> = Vec::new();
        assert_eq!(pick_role_entry(&empty), None);
    }

    #[test]
    fn source_psy_prefers_live_name() {
        // Live kodiak psy set: exact TCPC source name wins.
        let names = vec![
            "battery".to_string(),
            "usb".to_string(),
            SOURCE_PSY.to_string(),
        ];
        assert_eq!(pick_source_psy(&names), Some(SOURCE_PSY.to_string()));
        // Renamed spin: any tcpm-source fallback still finds it.
        let renamed = vec![
            "usb".to_string(),
            "tcpm-source-psy-spmi-foo".to_string(),
        ];
        assert_eq!(
            pick_source_psy(&renamed),
            Some("tcpm-source-psy-spmi-foo".to_string())
        );
        let none = vec!["battery".to_string(), "usb".to_string()];
        assert_eq!(pick_source_psy(&none), None);
    }

    #[test]
    fn disk_pick_prefers_first_partition() {
        // (name, removable, device-link-via-usb, has-partition).
        let cands = [
            ("sdb", true, true, false),
            ("sda", true, true, true),
            ("sdc", false, true, true),
            ("sdd", true, false, true),
        ];
        // sda wins (removable + usb); partition preferred.
        assert_eq!(
            choose_otg_disk(&cands),
            Some("/dev/block/sda1".to_string())
        );
        // Whole disk when no partition node exists yet.
        assert_eq!(
            choose_otg_disk(&[("sdb", true, true, false)]),
            Some("/dev/block/sdb".to_string())
        );
        // Non-removable / non-USB devices never match.
        assert_eq!(choose_otg_disk(&[("sdc", false, true, true)]), None);
        assert_eq!(choose_otg_disk(&[("sdd", true, false, true)]), None);
        let empty: [(&str, bool, bool, bool); 0] = [];
        assert_eq!(choose_otg_disk(&empty), None);
    }

    #[test]
    fn stock_chain_has_no_aoc_modules() {
        // The shim owns the AOC vote; insmodding the stock AoC drivers
        // would re-cast (0,1) against us while the firmware is down.
        // (On stock malibu DT the gate is open anyway — this only
        // avoids fighting ourselves.)
        for m in STOCK_USB_CHAIN {
            assert!(!m.contains("aoc"), "{m} must not be in the chain");
        }
        assert!(STOCK_USB_CHAIN.contains(&"google-role-sw"));
        assert!(STOCK_USB_CHAIN.contains(&"dwc3-google"));
    }
}
