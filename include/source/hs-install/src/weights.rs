//! Batch weights. Universal: derived from manifest groups + base names.
//! 1 = super-logical (manifest group members, written first, safest:
//!     the slot is inactive)
//! 2 = boot set
//! 3 = modem-adjacent firmware
//! 4 = everything else physical (bootloaders, vbmeta, firmware…)

pub const W_SUPER: u8 = 1;
pub const W_BOOT: u8 = 2;
pub const W_MODEM: u8 = 3;
pub const W_REST: u8 = 4;

const BOOT_SET: &[&str] = &[
    "boot",
    "init_boot",
    "vendor_boot",
    "vendor_kernel_boot",
    "dtbo",
];

const MODEM_SET: &[&str] = &[
    "modem",
    "pvmfw", "gcf", "ldfw",
];

/// Weight for a manifest base name. `in_group` = member of any manifest
/// dynamic group (computed by the caller from DynMeta).
pub fn weight(base: &str, in_group: bool) -> u8 {
    if in_group {
        return W_SUPER;
    }
    if BOOT_SET.contains(&base) {
        return W_BOOT;
    }
    if MODEM_SET.contains(&base) {
        return W_MODEM;
    }
    W_REST
}

/// Critical backup set: weights 2+4 (boot + bootloaders/vbmeta).
/// Super (1) and modem (3) are excluded by design: huge / reflashable.
pub fn is_critical(w: u8) -> bool {
    w == W_BOOT || w == W_REST
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn table() {
        assert_eq!(weight("system", true), W_SUPER);
        assert_eq!(weight("vendor_boot", false), W_BOOT);
        assert_eq!(weight("modem", false), W_MODEM);
        assert_eq!(weight("abl", false), W_REST);
        assert!(is_critical(W_BOOT) && is_critical(W_REST));
        assert!(!is_critical(W_SUPER) && !is_critical(W_MODEM));
    }
}
