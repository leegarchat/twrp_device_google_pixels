//! usb::gs201 — STUB (Tensor G2: cheetah/panther/lynx/felix/tangorpro).
//!
//! Firmware side is analyzed (dwc3-exynos-usb.ko + tcpci_max77759.ko +
//! google-charger.ko in vendor_dlkm, order in modules.dep); all that is
//! left is preloading that chain before the zuma-style shim path.
//! Until then: patch fails closed, daemon parks. Same interface as zuma.

use crate::ko_picker::log_msg;
use crate::props::set_prop;

const TAG: &str = "otg";

/// gs201 `otg-patch`: unimplemented — keep device mode (adb safe).
pub fn run_otg_patch() -> Result<(), String> {
    log_msg(TAG, "INFO", "gs201: OTG not implemented yet, staying in device mode");
    let _ = set_prop("sys.usb.patch_dwc3", "0");
    Err("usb otg: gs201 not implemented".into())
}

/// gs201 `otg-auto`: park forever (an exiting service would respawn-loop).
pub fn run_otg_auto() -> ! {
    log_msg(TAG, "INFO", "gs201: otg-auto parking (not implemented)");
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}
