//! OTA package identity: pre-device vs the live device prop.
//! Mirrors stock CheckPackageMetadata (pre-device == ro.product.device),
//! but as a hard error — flashing another device's payload is a brick.

use payload_hs::zipx;

/// Returns the package's pre-device value.
pub fn pre_device(zip: &str) -> Result<String, String> {
    let md = zipx::read_entry(zip, "META-INF/com/android/metadata")?;
    let s = String::from_utf8_lossy(&md);
    for line in s.lines() {
        if let Some(v) = line.strip_prefix("pre-device=") {
            return Ok(v.trim().to_string());
        }
    }
    Err("OTA metadata has no pre-device".to_string())
}

/// Hard gate: package must be built for THIS device. No fallback —
/// a wrong-device payload must never reach the writer.
pub fn check_device(zip: &str, device: &str) -> Result<(), String> {
    let pre = pre_device(zip)?;
    // Stock compares a single value; tolerate '|'/' '-separated lists
    // some ports ship, but the live device must be a member.
    let ok = pre
        .split(['|', ' ', ','])
        .any(|d| !d.is_empty() && d.trim() == device);
    if pre.is_empty() {
        return Err("pre-device empty: refusing".to_string());
    }
    if !ok {
        return Err(format!(
            "package is for '{pre}' but device is '{device}': refusing"
        ));
    }
    Ok(())
}
