//! usb::malibu — STUB (Tensor G6: grizzly/cubs/kodiak/yogi).
//!
//! Verified path (6.12 kernel: no `dwc3_otg_host_ready`, role-switch +
//! typec native): stage the stock AoC runtime (`aocd` + libs +
//! `aoc_usb_driver.ko`) from the live vendor, like the reference
//! `otg_yogi.sh` does — full AoC firmware is already up from boot.
//! Until then: patch fails closed, daemon parks. Same interface as zuma.

use crate::ko_picker::log_msg;
use crate::props::set_prop;

const TAG: &str = "otg";

/// malibu `otg-patch`: unimplemented — keep device mode (adb safe).
pub fn run_otg_patch() -> Result<(), String> {
    log_msg(TAG, "INFO", "malibu: OTG not implemented yet, staying in device mode");
    let _ = set_prop("sys.usb.patch_dwc3", "0");
    Err("usb otg: malibu not implemented".into())
}

/// malibu `otg-auto`: park forever (an exiting service would respawn-loop).
pub fn run_otg_auto() -> ! {
    log_msg(TAG, "INFO", "malibu: otg-auto parking (not implemented)");
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}
