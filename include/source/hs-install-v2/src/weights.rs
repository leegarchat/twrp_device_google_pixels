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
    weight_with(base, in_group, None)
}

/// Weight with an explicit per-device stage table (from the device
/// pixel.json ota_* arrays: ota_super->1, ota_boot->2, ota_modem->3,
/// ota_bootloader->4). When the table names a partition it wins;
/// otherwise the built-in sets apply (defense in depth for payloads
/// newer than the tree).
pub fn weight_with(base: &str, in_group: bool, table: Option<&std::collections::HashMap<String, u8>>) -> u8 {
    if let Some(t) = table {
        if let Some(w) = t.get(base) {
            return *w;
        }
    }
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
    use std::collections::HashMap;
    #[test]
    fn table() {
        assert_eq!(weight("system", true), W_SUPER);
        assert_eq!(weight("vendor_boot", false), W_BOOT);
        assert_eq!(weight("modem", false), W_MODEM);
        assert_eq!(weight("abl", false), W_REST);
        assert!(is_critical(W_BOOT) && is_critical(W_REST));
        assert!(!is_critical(W_SUPER) && !is_critical(W_MODEM));
    }
    #[test]
    fn table_override() {
        let mut t = HashMap::new();
        // json ota_* mapping for a device without pvmfw in modem set
        t.insert("pvmfw".to_string(), W_BOOT);
        t.insert("gcf".to_string(), W_REST);
        t.insert("system".to_string(), W_SUPER);
        // table wins over builtins and groups alike
        assert_eq!(weight_with("pvmfw", false, Some(&t)), W_BOOT);
        assert_eq!(weight_with("gcf", false, Some(&t)), W_REST);
        assert_eq!(weight_with("system", true, Some(&t)), W_SUPER);
        // unnamed partitions fall back to builtins
        assert_eq!(weight_with("modem", false, Some(&t)), W_MODEM);
        assert_eq!(weight_with("abl", false, Some(&t)), W_REST);
        assert_eq!(weight_with("vendor", true, Some(&t)), W_SUPER);
    }
}
