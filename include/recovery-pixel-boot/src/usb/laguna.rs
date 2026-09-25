//! usb::laguna — STUB (Tensor G5: frankel/blazer/mustang/rango).
//!
//! Known different: Type-C policy via TCPM (`preferred_role`/`port_type`),
//! SPMI (not i2c) TCPC, no gvotable CHARGER_MODE force; direct role-switch
//! writes are a hardware no-op. AoC staging a la malibu is the candidate
//! path — verify against the laguna DTBO/firmware before implementing.
//! Until then: patch fails closed, daemon parks. Same interface as zuma.

use crate::ko_picker::log_msg;
use crate::props::set_prop;

const TAG: &str = "otg";

/// laguna `otg-patch`: unimplemented — keep device mode (adb safe).
pub fn run_otg_patch() -> Result<(), String> {
    log_msg(TAG, "INFO", "laguna: OTG not implemented yet, staying in device mode");
    let _ = set_prop("sys.usb.patch_dwc3", "0");
    Err("usb otg: laguna not implemented".into())
}

/// laguna `otg-auto`: park forever (an exiting service would respawn-loop).
pub fn run_otg_auto() -> ! {
    log_msg(TAG, "INFO", "laguna: otg-auto parking (not implemented)");
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}
