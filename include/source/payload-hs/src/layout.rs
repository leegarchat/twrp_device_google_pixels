//! Phase-2 planner: does the payload fit the opposite slot, and what
//! must die to make room? Pure function over numbers — no I/O, fully
//! unit-testable. Live numbers come from `siw info` on device; the
//! manifest numbers come from [`crate::manifest`].
//!
//! Slot model: `current` = booted slot (untouchable except the kill
//! list we compute), `target` = opposite slot (fully rebuilt anyway,
//! so its space is always reclaimed). Super space is shared.
//!
//! Decision ladder:
//! 1. FITS_AS_IS — target rebuild alone frees enough.
//! 2. KILL_ONE([p]) — deleting one current-slot partition covers it.
//! 3. KILL_TWO([p1, p2]) — two current-slot partitions cover it.
//! 4. WIPE_ALL — nothing but a bare super fits.
//! 5. IMPOSSIBLE — not even a bare super fits. ERROR, nothing touched.

use std::collections::BTreeMap;

/// Live LP state, slot suffixes included (`system_a`, `vendor_b`, ...).
/// JSON form: {"super_size":N,"current":{"system_a":N,...},"target":{...}}
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct LiveState {
    pub super_size: u64,
    /// partition full name -> bytes currently allocated
    pub current: BTreeMap<String, u64>,
    pub target: BTreeMap<String, u64>,
    /// live group name holding the target slot (slot-suffixed on-device,
    /// e.g. google_dynamic_partitions_a; empty if unknown)
    pub target_group: String,
}

/// Strip a trailing _a/_b slot suffix.
pub fn base_name(full: &str) -> &str {
    if full.len() > 2 {
        let (b, s) = full.split_at(full.len() - 2);
        if s == "_a" || s == "_b" {
            return b;
        }
    }
    full
}

/// Bytes the LP metadata itself eats; conservative constant.
pub const LP_METADATA_RESERVE: u64 = 1 << 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FitPlan {
    /// Rebuild the target group, kill nothing.
    AsIs,
    /// Rebuild target + delete these current-slot partitions (full names).
    /// len 1 = KILL_ONE, len 2 = KILL_TWO, longer = staged wipe list.
    Kill(Vec<String>),
    /// Delete every logical partition, then rebuild target.
    WipeAll,
    /// Even a bare super is too small. Abort before any write.
    Impossible,
}

impl FitPlan {
    pub fn code(&self) -> &'static str {
        match self {
            FitPlan::AsIs => "FITS_AS_IS",
            FitPlan::Kill(k) if k.len() == 1 => "KILL_ONE",
            FitPlan::Kill(k) if k.len() == 2 => "KILL_TWO",
            FitPlan::Kill(_) => "KILL_MANY",
            FitPlan::WipeAll => "WIPE_ALL",
            FitPlan::Impossible => "IMPOSSIBLE",
        }
    }
}

/// `required`: (partition base name, expanded bytes) for ONE group.
/// `members`: base names belonging to this manifest group — only these
/// target-slot residents are reclaimed by the rebuild; anything else
/// (other groups) stays. `budget`: group budget from the manifest.
/// Returns the plan for that group. Callers run it per group and take
/// the strictest result.
pub fn plan_group(
    required: &[(String, u64)],
    members: &[String],
    budget: u64,
    live: &LiveState,
) -> FitPlan {
    let need: u64 = required.iter().map(|(_, s)| s).sum();

    // Bare-super gate first: no plan can beat physics.
    if need + LP_METADATA_RESERVE > live.super_size {
        return FitPlan::Impossible;
    }
    if need > budget {
        // Manifest asks for more than its own group allows — same as
        // impossible: no kill list fixes an over-budget group.
        return FitPlan::Impossible;
    }

    let used_all: u64 = live.current.values().sum::<u64>()
        + live.target.values().sum::<u64>();
    let free = live.super_size.saturating_sub(used_all);

    // Rebuilding the target group reclaims only its own residents.
    let reclaim_tgt: u64 = live
        .target
        .iter()
        .filter(|(n, _)| members.iter().any(|m| m == base_name(n)))
        .map(|(_, s)| s)
        .sum();
    let avail = free + reclaim_tgt;
    if need <= avail {
        return FitPlan::AsIs;
    }

    // Analysis only: greedily sacrifice current-slot partitions,
    // largest first (product usually covers it alone).
    let mut cur: Vec<(String, u64)> = live
        .current
        .iter()
        .map(|(k, v)| (k.clone(), *v))
        .collect();
    cur.sort_by_key(|(_, s)| std::cmp::Reverse(*s));

    let mut deficit = need - avail;
    let mut kill: Vec<String> = Vec::new();
    for (name, size) in &cur {
        if deficit == 0 {
            break;
        }
        kill.push(name.clone());
        deficit = deficit.saturating_sub(*size);
    }
    if deficit > 0 {
        // Even eating the whole current slot is not enough: the target
        // slot's non-group residents (other groups) block us -> wipe all.
        // Bare-super gate already passed, so WipeAll always fits.
        return FitPlan::WipeAll;
    }
    FitPlan::Kill(kill)
}

/// Strictest of per-group plans: Impossible > WipeAll > Kill(longer) >
/// Kill(shorter) > AsIs.
pub fn strictest(plans: &[FitPlan]) -> FitPlan {
    let rank = |p: &FitPlan| -> (u8, usize) {
        match p {
            FitPlan::Impossible => (4, 0),
            FitPlan::WipeAll => (3, 0),
            FitPlan::Kill(k) => (2, k.len()),
            FitPlan::AsIs => (0, 0),
        }
    };
    plans
        .iter()
        .max_by_key(|p| rank(p))
        .cloned()
        .unwrap_or(FitPlan::AsIs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn live(super_size: u64, cur: &[(&str, u64)], tgt: &[(&str, u64)]) -> LiveState {
        LiveState {
            super_size,
            current: cur.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
            target: tgt.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
            target_group: String::new(),
        }
    }

    fn req(parts: &[(&str, u64)]) -> Vec<(String, u64)> {
        parts.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    fn members(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn as_is_when_target_reclaim_covers() {
        // super 1000, both slots hold 400/300, need 500: free 300 +
        // reclaim 300 = 600 >= 500.
        let l = live(8_000_000, &[("p_b", 3_000_000)], &[("p_a", 2_500_000)]);
        assert_eq!(
            plan_group(&req(&[("p", 4_000_000)]), &members(&["p"]), 5_000_000, &l),
            FitPlan::AsIs
        );
    }

    #[test]
    fn kill_one_covers_deficit() {
        // free 2M + reclaim 2.5M = 4.5M, need 6.8M -> deficit 2.3M,
        // largest current (p_b:3M) covers it alone.
        let l = live(8_000_000, &[("p_b", 3_000_000), ("q_b", 500_000)], &[("p_a", 2_500_000)]);
        assert_eq!(
            plan_group(&req(&[("p", 6_800_000)]), &members(&["p"]), 8_000_000, &l),
            FitPlan::Kill(vec!["p_b".to_string()])
        );
    }

    #[test]
    fn kill_two_when_one_is_not_enough() {
        // avail 4M, need 6M -> deficit 2M; biggest single is 1.5M.
        let l = live(
            8_000_000,
            &[("p_b", 1_500_000), ("q_b", 1_500_000), ("r_b", 1_000_000)],
            &[("p_a", 2_500_000)],
        );
        assert_eq!(
            plan_group(&req(&[("p", 6_000_000)]), &members(&["p"]), 7_000_000, &l),
            FitPlan::Kill(vec!["p_b".to_string(), "q_b".to_string()])
        );
    }

    #[test]
    fn wipe_all_when_current_slot_insufficient() {
        // deficit 500, whole current slot is 200 -> must wipe all.
        let l = live(
            8_000_000,
            &[("p_b", 1_000_000)],
            &[("p_a", 2_500_000), ("j_a", 2_000_000)],
        );
        assert_eq!(
            plan_group(&req(&[("p", 6_500_000)]), &members(&["p"]), 7_000_000, &l),
            FitPlan::WipeAll
        );
    }

    #[test]
    fn impossible_when_bare_super_too_small() {
        let l = live(8_000_000, &[("p_b", 1_000_000)], &[("p_a", 2_500_000)]);
        assert_eq!(
            plan_group(&req(&[("p", 9_000_000)]), &members(&["p"]), 12_000_000, &l),
            FitPlan::Impossible
        );
    }

    #[test]
    fn impossible_when_over_group_budget() {
        let l = live(80_000_000, &[("p_b", 2_000_000)], &[("p_a", 3_000_000)]);
        assert_eq!(
            plan_group(&req(&[("p", 7_000_000)]), &members(&["p"]), 6_000_000, &l),
            FitPlan::Impossible
        );
    }

    #[test]
    fn strictest_ordering() {
        assert_eq!(
            strictest(&[FitPlan::AsIs, FitPlan::Kill(vec!["a".into()])]),
            FitPlan::Kill(vec!["a".into()])
        );
        assert_eq!(
            strictest(&[FitPlan::WipeAll, FitPlan::Kill(vec!["a".into(), "b".into()])]),
            FitPlan::WipeAll
        );
        assert_eq!(
            strictest(&[FitPlan::Impossible, FitPlan::WipeAll]),
            FitPlan::Impossible
        );
    }

    #[test]
    fn base_name_strips_slot() {
        assert_eq!(base_name("system_a"), "system");
        assert_eq!(base_name("vendor_dlkm_b"), "vendor_dlkm");
        assert_eq!(base_name("userdata"), "userdata");
    }

    #[test]
    fn only_group_members_reclaimed() {
        // Target holds group member p_a (300) + foreign junk j_a (400).
        // Rebuild reclaims only p_a: free 100 + 300 = 400 < 500.
        let l = live(
            8_000_000,
            &[("q_b", 2_000_000)],
            &[("p_a", 3_000_000), ("j_a", 3_000_000)],
        );
        let r = plan_group(&req(&[("p", 4_000_000)]), &members(&["p"]), 5_000_000, &l);
        // deficit 100, kill q_b (200) -> KILL_ONE, junk untouched.
        assert_eq!(r, FitPlan::Kill(vec!["q_b".to_string()]));
    }
}
