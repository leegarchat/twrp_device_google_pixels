//! usb — modular per-family USB/OTG stack.
//!
//! One submodule per SoC family with the same two-entry interface:
//! - `run_otg_patch()`: oneshot (service `otg_enable`); injects whatever
//!   the family needs for host mode and sets `sys.usb.patch_dwc3=1`, which
//!   gates the daemon. `0` + `Err` keeps the device a well-behaved gadget.
//! - `run_otg_auto()`: VBUS daemon (service `otg_auto`); never returns.
//!
//! Two host-enable mechanisms, picked per family at runtime:
//! - Exynos generations (zuma/gs101/gs201/zumapro): `otg_host_shim`
//!   (kprobe on `dwc3_otg_host_ready`, replaces the AoC probe) + OTG_ID +
//!   CHARGER_MODE VBUS force, driven by the VBUS daemon.
//! - Google-silicon generations (laguna/malibu): `aoc_vote_shim`
//!   (`USB_DR_EL` election `cast_vote("AOC",1,1)`, replaces the whole
//!   AoC/aocd stack — no vendor staging anywhere); the election
//!   self-switches, the daemon only keeps the vote + verifies + maintains
//!   `/dev/block/otg-usb`.
//!
//! Family identity: `ro.recovery.soc_family` (stamped by early-init from
//! `families/*/family.json`) wins; `ro.hardware` codename is the fallback
//! for trees where the prop is missing. `aio`/empty reads as Unknown and
//! takes the stub path: never force host on an unidentified tree.

pub mod gs101;
pub mod gs201;
pub mod gate;
pub mod laguna;
pub mod malibu;
pub mod zuma;
pub mod zumapro;

use crate::ko_picker::log_msg;
use crate::props::{get_prop, set_prop};

const TAG: &str = "otg";

/// SoC family owning the USB controller at hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Zuma,
    Gs101,
    Gs201,
    Zumapro,
    Laguna,
    Malibu,
    Unknown,
}

impl Family {
    /// Stable short name for logs and error strings.
    pub fn name(&self) -> &'static str {
        match self {
            Family::Zuma => "zuma",
            Family::Gs101 => "gs101",
            Family::Gs201 => "gs201",
            Family::Zumapro => "zumapro",
            Family::Laguna => "laguna",
            Family::Malibu => "malibu",
            Family::Unknown => "unknown",
        }
    }
}

/// Pure family resolution over (`ro.recovery.soc_family`, `ro.hardware`).
/// SoC prop wins; codename table covers prop-less trees. `aio`/empty/blank
/// is Unknown (fail closed). Pure over inputs (testable); live prop reads
/// live in [`current_family`].
pub fn detect_family(soc_family: &str, hardware: &str) -> Family {
    match soc_family.trim() {
        "gs101" => return Family::Gs101,
        "zuma" => return Family::Zuma,
        "gs201" => return Family::Gs201,
        "zumapro" => return Family::Zumapro,
        "laguna" => return Family::Laguna,
        "malibu" => return Family::Malibu,
        _ => {}
    }
    match hardware.trim() {
        "oriole" | "raven" | "bluejay" => Family::Gs101,
        "shiba" | "husky" | "akita" => Family::Zuma,
        "cheetah" | "panther" | "lynx" | "felix" | "tangorpro" => Family::Gs201,
        "comet" | "komodo" | "caiman" | "tegu" | "tokay" | "stallion" => Family::Zumapro,
        "frankel" | "blazer" | "mustang" | "rango" => Family::Laguna,
        "grizzly" | "cubs" | "kodiak" | "yogi" => Family::Malibu,
        _ => Family::Unknown,
    }
}

/// Live family identity from system properties.
pub fn current_family() -> Family {
    detect_family(
        &get_prop("ro.recovery.soc_family"),
        &get_prop("ro.hardware"),
    )
}

/// Family-dispatched `otg-patch`. Only implemented families set
/// `sys.usb.patch_dwc3=1`; stubs fail closed (device mode survives).
pub fn run_otg_patch() -> Result<(), String> {
    let fam = current_family();
    log_msg(TAG, "INFO", &format!("otg-patch for family {}", fam.name()));
    // Fool-proof gate first: a newer-than-firmware kernel panics on the
    // VBUS host switch. Blocked = no shim, no writes, patch_dwc3 stays 0
    // (the daemon below never starts).
    if gate::check(&fam) == gate::Gate::Block {
        let _ = set_prop("sys.usb.patch_dwc3", "0");
        return Err("usb otg: kernel/firmware mismatch, custom stack disabled".into());
    }
    match fam {
        Family::Zuma => zuma::run_otg_patch(),
        Family::Gs101 => gs101::run_otg_patch(),
        Family::Gs201 => gs201::run_otg_patch(),
        Family::Zumapro => zumapro::run_otg_patch(),
        Family::Laguna => laguna::run_otg_patch(),
        Family::Malibu => malibu::run_otg_patch(),
        Family::Unknown => {
            log_msg(TAG, "INFO", "unknown family: OTG stays in device mode");
            let _ = set_prop("sys.usb.patch_dwc3", "0");
            Err("usb otg: unknown family".into())
        }
    }
}

/// Family-dispatched `otg-auto`. Never returns (stubs park).
pub fn run_otg_auto() -> ! {
    let fam = current_family();
    log_msg(TAG, "INFO", &format!("otg-auto for family {}", fam.name()));
    // Belt and braces: the daemon must never drive VBUS on a gated tree
    // (e.g. patch flag set by hand). Park like the unknown family.
    if gate::check(&fam) == gate::Gate::Block {
        log_msg(TAG, "INFO", "otg-auto gated off, parking (device mode)");
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3600));
        }
    }
    match fam {
        Family::Zuma => zuma::run_otg_auto(),
        Family::Gs101 => gs101::run_otg_auto(),
        Family::Gs201 => gs201::run_otg_auto(),
        Family::Zumapro => zumapro::run_otg_auto(),
        Family::Laguna => laguna::run_otg_auto(),
        Family::Malibu => malibu::run_otg_auto(),
        Family::Unknown => {
            log_msg(TAG, "INFO", "unknown family: otg-auto parking (device mode)");
            loop {
                std::thread::sleep(std::time::Duration::from_secs(3600));
            }
        }
    }
}

/// Ensure debugfs is mounted: the gvotable CHARGER_MODE voter (VBUS
/// force) only exists under /sys/kernel/debug. Idempotent; best-effort
/// (a restricted kernel may refuse — the caller logs and moves on).
pub fn ensure_debugfs() {
    if std::path::Path::new("/sys/kernel/debug/gvotables").exists() {
        return;
    }
    // SAFETY: plain C mount(2) of a kernel pseudo-filesystem with NULL
    // data; return value is the only effect, checked by the caller.
    let rc = unsafe {
        libc::mount(
            c"debugfs".as_ptr(),
            c"/sys/kernel/debug".as_ptr(),
            c"debugfs".as_ptr(),
            0,
            std::ptr::null(),
        )
    };
    if rc == 0 {
        log_msg(TAG, "INFO", "debugfs mounted");
    }
}

/// Consecutive identical VBUS ticks needed before a role switch.
/// A dying drive glitches VBUS for a tick or two mid-transfer;
/// acting on the first edge yanked the bus (and the gadget) out
/// from under a mounted FUSE filesystem and wedged the recovery
/// UI thread in fuse_get_req.
pub const VBUS_STABLE_TICKS: u32 = 3;

/// True when USB mass storage is currently mounted (TWRP /usb_otg
/// or vold /auto0-N subpartitions). The OTG daemon must not flip
/// host->device while these are served: the switch tears down the
/// gadget and strands FUSE daemons on a dead device.
pub fn mounts_hold_usb_storage(text: &str) -> bool {
    text.lines().any(|line| {
        let mut it = line.split_whitespace();
        it.next();
        match it.next() {
            Some(mnt) => {
                mnt == "/usb_otg" || mnt == "/usb-otg" || mnt.starts_with("/auto0")
            }
            None => false,
        }
    })
}

/// Live check against /proc/mounts.
pub fn usb_storage_mounted() -> bool {
    match std::fs::read_to_string("/proc/mounts") {
        Ok(text) => mounts_hold_usb_storage(&text),
        Err(_) => false,
    }
}

/// VBUS edge debouncer over the 1s poll ticks. Feed every raw
/// reading; yields the new value exactly once, after
/// VBUS_STABLE_TICKS consecutive identical ticks.
pub struct VbusDebounce {
    pending: Option<String>,
    stable: u32,
}

impl VbusDebounce {
    pub fn new() -> Self {
        VbusDebounce {
            pending: None,
            stable: 0,
        }
    }

    pub fn feed(&mut self, current: &str) -> Option<String> {
        match &self.pending {
            Some(p) if p == current => {
                self.stable += 1;
                if self.stable >= VBUS_STABLE_TICKS {
                    self.pending = None;
                    self.stable = 0;
                    Some(current.to_string())
                } else {
                    None
                }
            }
            _ => {
                self.pending = Some(current.to_string());
                self.stable = 1;
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debounce_needs_three_stable_ticks() {
        let mut d = VbusDebounce::new();
        assert_eq!(d.feed("0"), None);
        assert_eq!(d.feed("1"), None);
        assert_eq!(d.feed("0"), None);
        assert_eq!(d.feed("0"), None);
        assert_eq!(d.feed("0"), Some("0".to_string()));
        // consumed: next identical tick starts a new round
        assert_eq!(d.feed("0"), None);
    }

    #[test]
    fn debounce_glitch_resets() {
        let mut d = VbusDebounce::new();
        assert_eq!(d.feed("1"), None);
        assert_eq!(d.feed("1"), None);
        assert_eq!(d.feed("0"), None);
        assert_eq!(d.feed("1"), None);
        assert_eq!(d.feed("1"), None);
        assert_eq!(d.feed("1"), Some("1".to_string()));
    }

    #[test]
    fn storage_mount_detection() {
        assert!(mounts_hold_usb_storage(
            "adb /dev/usb-ffs/adb functionfs rw 0 0\n/dev/block/sde1 /usb_otg fuseblk rw 0 0\n"
        ));
        assert!(mounts_hold_usb_storage(
            "/dev/block/sde2 /auto0-1 vfat rw 0 0\n"
        ));
        assert!(!mounts_hold_usb_storage(
            "adb /dev/usb-ffs/adb functionfs rw 0 0\n/dev/block/sda34 /data f2fs rw 0 0\n"
        ));
        assert!(!mounts_hold_usb_storage(""));
    }

    #[test]
    fn soc_prop_wins_over_codename() {
        assert_eq!(detect_family("zuma", ""), Family::Zuma);
        assert_eq!(detect_family("gs101\n", "shiba"), Family::Gs101);
        assert_eq!(detect_family("laguna", "shiba"), Family::Laguna);
        assert_eq!(detect_family("malibu", "raven"), Family::Malibu);
    }

    #[test]
    fn codename_fallback_covers_all_devices() {
        for c in ["oriole", "raven", "bluejay"] {
            assert_eq!(detect_family("", c), Family::Gs101, "{c}");
        }
        for c in ["shiba", "husky", "akita"] {
            assert_eq!(detect_family("", c), Family::Zuma, "{c}");
        }
        for c in ["cheetah", "panther", "lynx", "felix", "tangorpro"] {
            assert_eq!(detect_family("", c), Family::Gs201, "{c}");
        }
        for c in ["comet", "komodo", "caiman", "tegu", "tokay", "stallion"] {
            assert_eq!(detect_family("", c), Family::Zumapro, "{c}");
        }
        for c in ["frankel", "blazer", "mustang", "rango"] {
            assert_eq!(detect_family("", c), Family::Laguna, "{c}");
        }
        for c in ["grizzly", "cubs", "kodiak", "yogi"] {
            assert_eq!(detect_family("", c), Family::Malibu, "{c}");
        }
    }

    #[test]
    fn unknown_fails_closed() {
        assert_eq!(detect_family("", ""), Family::Unknown);
        assert_eq!(detect_family("aio", "shiba"), Family::Zuma);
        assert_eq!(detect_family("aio", ""), Family::Unknown);
        assert_eq!(detect_family("UNKNOWN", "UNKNOWN.dwc3"), Family::Unknown);
        assert_eq!(Family::Unknown.name(), "unknown");
        assert_eq!(Family::Zuma.name(), "zuma");
    }
}
