//! usb — modular per-family USB/OTG stack.
//!
//! One submodule per SoC family with the same two-entry interface:
//! - `run_otg_patch()`: oneshot (service `otg_enable`); injects whatever
//!   the family needs for host mode and sets `sys.usb.patch_dwc3=1`, which
//!   gates the daemon. `0` + `Err` keeps the device a well-behaved gadget.
//! - `run_otg_auto()`: VBUS daemon (service `otg_auto`); never returns.
//!
//! Only `zuma` is implemented (field-proven on shiba/EVOX). `gs101`,
//! `gs201`, `zumapro`, `laguna` and `malibu` are stubs with the identical
//! interface: patch fails closed (device mode, adb safe), the daemon parks
//! instead of exiting (an exiting non-oneshot service would respawn-loop).
//! Fill a stub file when that family's firmware analysis lands — the
//! dispatcher below needs no changes.
//!
//! Family identity: `ro.recovery.soc_family` (stamped by early-init from
//! `families/*/family.json`) wins; `ro.hardware` codename is the fallback
//! for trees where the prop is missing. `aio`/empty reads as Unknown and
//! takes the stub path: never force host on an unidentified tree.

pub mod gs101;
pub mod gs201;
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
    match current_family() {
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
    match current_family() {
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

#[cfg(test)]
mod tests {
    use super::*;

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
