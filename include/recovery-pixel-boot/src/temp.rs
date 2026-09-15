//! temp — thermal zone selection (port of setup_cpu_temp.sh).
//!
//! The CPU BIG zone is a hotspot (reads ~90C on a cold device), so the TWRP
//! widget must NOT point at it. Like the Android-side sensor selection, we
//! use the HAL-style representative sensor (`soc_therm`) instead of any
//! average: zone numbering shifts across Tensor generations and driver load
//! order, but the named sensor is stable. Called as
//! `recovery-pixel-boot setup-temp` from an `on init` exec and re-run from
//! the boot stage after vendor_dlkm drivers land.

use crate::config::load_device_config;
use crate::ko_picker::log_msg;
use crate::props::get_prop;
use std::path::Path;

const TAG: &str = "temp";
const TARGET: &str = "/dev/thermal_cpu";
const ZONE_GLOB_DIR: &str = "/sys/class/thermal";

/// Pick the winning zone-temp path from (name, type) pairs.
/// Exact path mode is handled by the caller; this is the auto matcher.
pub fn pick_zone<'a>(zones: &[(&'a str, &str)], types: &[String]) -> Option<&'a str> {
    for (name, typ) in zones {
        if types.iter().any(|t| t == typ) {
            return Some(name);
        }
    }
    None
}

pub fn run_setup_temp() -> Result<(), String> {
    let code = get_prop("ro.hardware");
    let cfg = load_device_config(&code).unwrap_or_default();
    let _ = std::fs::remove_file(TARGET);

    // Exact-path mode: one configured temp node, no scanning.
    if !cfg.thermal_temp_path.is_empty() {
        if Path::new(&cfg.thermal_temp_path).exists() {
            std::os::unix::fs::symlink(&cfg.thermal_temp_path, TARGET)
                .map_err(|e| format!("symlink failed: {e}"))?;
            log_msg(TAG, "INFO", &format!("thermal_cpu -> {}", cfg.thermal_temp_path));
            return Ok(());
        }
        log_msg(
            TAG,
            "WARN",
            &format!("exact {} missing, falling back to auto", cfg.thermal_temp_path),
        );
    }

    // Auto mode: enumerate zones, match type names.
    let mut zones: Vec<(String, String)> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(ZONE_GLOB_DIR) {
        let mut names: Vec<String> = rd
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("thermal_zone"))
            .collect();
        names.sort();
        for n in &names {
            let t = std::fs::read_to_string(format!("{ZONE_GLOB_DIR}/{n}/type"))
                .unwrap_or_default()
                .trim()
                .to_string();
            zones.push((format!("{ZONE_GLOB_DIR}/{n}/temp"), t));
        }
    }
    // Priority: exact path -> representative soc sensor (never the CPU
    // hotspot) -> BIG-cluster auto list -> zone0 fallback.
    let soc_type = if cfg.thermal_soc_type.is_empty() {
        "soc_therm".to_string()
    } else {
        cfg.thermal_soc_type.clone()
    };
    let refs: Vec<(&str, &str)> = zones
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let winner = pick_zone(&refs, &[soc_type])
        .or_else(|| pick_zone(&refs, &cfg.thermal_zone_types))
        .map(|s| s.to_string());

    // Fallback: zone0 (TWRP default) when nothing matched.
    let temp = winner.unwrap_or_else(|| format!("{ZONE_GLOB_DIR}/thermal_zone0/temp"));
    std::os::unix::fs::symlink(&temp, TARGET)
        .map_err(|e| format!("symlink failed: {e}"))?;
    log_msg(TAG, "INFO", &format!("thermal_cpu -> {temp}"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn types() -> Vec<String> {
        ["BIG", "CLUSTER2"].iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn picks_big_over_zone0() {
        let zones = [
            ("/sys/class/thermal/thermal_zone0/temp", "soc"),
            ("/sys/class/thermal/thermal_zone1/temp", "BIG"),
        ];
        assert_eq!(
            pick_zone(&zones, &types()),
            Some("/sys/class/thermal/thermal_zone1/temp")
        );
    }

    #[test]
    fn no_match_is_none() {
        let zones = [("/sys/class/thermal/thermal_zone0/temp", "soc")];
        assert_eq!(pick_zone(&zones, &types()), None);
    }

    #[test]
    fn soc_therm_wins_over_hotspot() {
        // Representative sensor first: BIG may read 90C on a cold device.
        let zones = [
            ("/sys/class/thermal/thermal_zone0/temp", "BIG"),
            ("/sys/class/thermal/thermal_zone17/temp", "soc_therm"),
        ];
        assert_eq!(
            pick_zone(&zones, &["soc_therm".to_string()]),
            Some("/sys/class/thermal/thermal_zone17/temp")
        );
    }
}
