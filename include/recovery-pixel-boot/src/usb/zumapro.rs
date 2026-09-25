//! usb::zumapro — STUB (Tensor G4 big: comet/komodo/caiman/tegu/tokay/stallion).
//!
//! Expected to follow the gs201/zuma Exynos DWC3 + max77759 pattern, but
//! its firmware composition is unverified. Analyze first (kernel strings,
//! DLKM inventory, DTB/DTBO nodes), then implement.
//! Until then: patch fails closed, daemon parks. Same interface as zuma.

use crate::ko_picker::log_msg;
use crate::props::set_prop;

const TAG: &str = "otg";

/// zumapro `otg-patch`: unimplemented — keep device mode (adb safe).
pub fn run_otg_patch() -> Result<(), String> {
    log_msg(TAG, "INFO", "zumapro: OTG not implemented yet, staying in device mode");
    let _ = set_prop("sys.usb.patch_dwc3", "0");
    Err("usb otg: zumapro not implemented".into())
}

/// zumapro `otg-auto`: park forever (an exiting service would respawn-loop).
pub fn run_otg_auto() -> ! {
    log_msg(TAG, "INFO", "zumapro: otg-auto parking (not implemented)");
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}
