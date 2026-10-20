//! Live super state via `siw info -f json`.
//! Split into current/target by the booted slot suffix.

use payload_hs::layout::LiveState;
use std::collections::BTreeMap;
use std::process::Command;

pub const SUPER_BLOCK: &str = "/dev/block/by-name/super";

/// siw 0.2.x JSON is FLAT per invocation: `siw info -f json -s N`
/// returns `{image_size, partitions: [{name, size, group}], ...}` for
/// that slot only (there is no nested `slots` array; an earlier schema
/// had one, and the old nested parser silently yielded empty tables).
#[derive(serde::Deserialize)]
struct SiwRoot {
    image_size: u64,
    #[serde(default)]
    partitions: Vec<SiwPart>,
}

#[derive(serde::Deserialize)]
struct SiwPart {
    name: String,
    size: u64,
    #[serde(default)]
    group: String,
}

/// Query the live super table. `booted` = 'a' | 'b'.
/// Reads both metadata slots explicitly (-s 0/1) and merges: names carry
/// their own suffixes, first occurrence wins.
pub fn query(siw: &str, booted: char) -> Result<LiveState, String> {
    let target = if booted == 'a' { 'b' } else { 'a' };
    let mut st = LiveState {
        super_size: 0,
        current: BTreeMap::new(),
        target: BTreeMap::new(),
        target_group: String::new(),
    };
    // Metadata slots mirror each other: merge, first occurrence wins
    // (names carry their own _a/_b suffixes).
    let mut seen = std::collections::BTreeSet::new();
    // Live group name holding the TARGET slot's residents (slot-suffixed
    // on these devices, e.g. google_dynamic_partitions_a).
    let mut target_group = String::new();
    for mslot in ["0", "1"] {
        let out = Command::new(siw)
            .args(["info", "-f", "json", "-s", mslot, SUPER_BLOCK])
            .output()
            .map_err(|e| format!("siw info: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "siw info -s {mslot} failed: {}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        let info: SiwRoot = serde_json::from_slice(&out.stdout)
            .map_err(|e| format!("siw json: {e}"))?;
        if info.image_size == 0 {
            return Err("siw info: zero super size".to_string());
        }
        if st.super_size == 0 {
            st.super_size = info.image_size;
        }
        for p in &info.partitions {
            if !seen.insert(p.name.clone()) {
                continue;
            }
            if p.name.ends_with(&format!("_{booted}")) {
                st.current.insert(p.name.clone(), p.size);
            } else if p.name.ends_with("_a") || p.name.ends_with("_b") {
                st.target.insert(p.name.clone(), p.size);
                if target_group.is_empty() && !p.group.is_empty() {
                    // group of a target-slot resident (take the first)
                    let suffix = &p.name[p.name.len() - 2..];
                    if suffix == &format!("_{target}") {
                        target_group = p.group.clone();
                    }
                }
            }
            // slotless residents: ignored (not our slots to rebuild)
        }
    }
    st.target_group = target_group;
    Ok(st)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real siw 0.2.4 shape: flat per-slot document, NO nested "slots".
    // (The pre-0.2.4 nested schema silently parsed to empty tables here
    // and blinded live state: current=0 target=0, no fit, no backups.)
    const FLAT_SAMPLE: &str = r#"{
        "image_size": 8531214336,
        "metadata_slot": 1,
        "partitions": [
            {"name": "system_a", "size": 145989632, "group": "google_dynamic_partitions_a"},
            {"name": "product_b", "size": 4079484928, "group": "google_dynamic_partitions_b"}
        ]
    }"#;

    #[test]
    fn flat_siw_json_parses() {
        let info: SiwRoot = serde_json::from_str(FLAT_SAMPLE).unwrap();
        assert_eq!(info.image_size, 8531214336);
        assert_eq!(info.partitions.len(), 2);
        assert_eq!(info.partitions[0].name, "system_a");
        assert_eq!(info.partitions[1].group, "google_dynamic_partitions_b");
    }
}
