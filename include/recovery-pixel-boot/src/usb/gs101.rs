//! usb::gs101 — STUB (Tensor G1: oriole/raven/bluejay).
//!
//! Fill when the gs101 firmware analysis lands (stock modules from the
//! live tree, dep order from its modules.dep, TCPC/charger specifics).
//! Until then: patch fails closed, daemon parks. Same interface as zuma.

use crate::ko_picker::log_msg;
use crate::props::set_prop;

const TAG: &str = "otg";

/// gs101 `otg-patch`: unimplemented — keep device mode (adb safe).
pub fn run_otg_patch() -> Result<(), String> {
    log_msg(TAG, "INFO", "gs101: OTG not implemented yet, staying in device mode");
    let _ = set_prop("sys.usb.patch_dwc3", "0");
    Err("usb otg: gs101 not implemented".into())
}

/// gs101 `otg-auto`: park forever (an exiting service would respawn-loop).
pub fn run_otg_auto() -> ! {
    log_msg(TAG, "INFO", "gs101: otg-auto parking (not implemented)");
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}
