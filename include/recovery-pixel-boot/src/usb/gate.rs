//! usb::gate — fool-proof gate for the custom OTG/host stack.
//!
//! Field lesson: a kernel from a NEWER snapshot than the firmware modules
//! (the "latest kernel" crowd: Wild 6.1.177 on a tree provisioned for
//! 6.1.145) panics when our stack force-switches VBUS power to host.
//! Touch modules cross-load fine under the same drift, but the USB power
//! switch does not forgive. So before any shim/OTG_ID/CHARGER write, we
//! compare the RUNNING kernel's patch level against the FIRMWARE modules'
//! patch level, read from .ko FILES (the kernel never reports what a
//! loaded module was built for — there is no vermagic query API).
//!
//! Rules:
//! - Empty lsmod (no loaded modules): nothing version-sensitive is in
//!   play — first-stage loads either stuck (stock GKI proves the tree
//!   modular) or never happened (monolithic/custom like Sultan, where
//!   everything needed is built in). Gate is vacuous — proceed. This is
//!   the primary discriminator: it reads the live state, not config
//!   files or uname strings.
//! - Monolithic kernel (module syscalls missing): nothing to inject,
//!   gate is vacuous — proceed.
//! - No readable stock .ko anywhere: cannot prove foolishness — proceed
//!   with a loud note (fail-open, today's behavior).
//! - Majority firmware patch == running patch: proceed.
//! - Otherwise: BLOCK. No services, no VBUS writes — the tree stays a
//!   well-behaved device (adb safe) instead of panicking.
//!
//! Read priority (first readable wins the scan order; verdict is by
//! MAJORITY over up to SCAN_CAP modules — stock trees carry stray
//! snapshots, e.g. one 6.1.75 .ko among 6.1.157, which must not trip
//! the gate):
//! - gs101: `/lib/modules` first (first-stage unpacked the vendor_boot
//!   dlkm fragment into the live root), then vendor_dlkm, vendor, ko_stage.
//! - others: vendor_dlkm first (gs201+ stage modules from there), then
//!   root, vendor, ko_stage (vendor_kernel_boot ramdisk content arrives
//!   via ko_stage streams, never as a live dir).
//!
//! `/system/**` is NEVER scanned: our own shims are cross-version by
//! design and would always mismatch.

use super::{Family, TAG};
use crate::ko_picker::{detect_kernel_env, is_monolithic, log_msg, modules_supported, read_ko_vermagic};
use std::path::{Path, PathBuf};

/// How many .ko files the scan reads before voting (cheap + decisive).
const SCAN_CAP: usize = 32;

/// substrings that promote a module to the front of the scan (the USB
/// stack's own neighborhood — most representative of firmware age).
const USB_HINTS: &[&str] = &["dwc3", "tcpci", "tcpm", "xhci", "usb", "typec", "max77759", "google-charger"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    /// Custom OTG stack may run.
    Proceed,
    /// Custom OTG stack must stay off (device mode only).
    Block,
}

/// Pure verdict over a kernel patch and sampled firmware patches.
/// Empty sample = unprovable = proceed (fail-open, loud at call site).
pub fn decide(kernel_patch: u32, firmware_patches: &[u32]) -> Gate {
    if firmware_patches.is_empty() {
        return Gate::Proceed;
    }
    let mut counts: Vec<(u32, usize)> = Vec::new();
    for p in firmware_patches {
        match counts.iter_mut().find(|(v, _)| v == p) {
            Some(e) => e.1 += 1,
            None => counts.push((*p, 1)),
        }
    }
    let (top, _) = counts.into_iter().max_by_key(|(_, n)| *n).unwrap();
    if top == kernel_patch {
        Gate::Proceed
    } else {
        Gate::Block
    }
}

/// Stock module search roots for a family, in read priority (see above).
fn stock_roots(fam: &Family) -> Vec<PathBuf> {
    let roots: &[&str] = match fam {
        // gs101: first-stage dlkm content lives in the live root.
        Family::Gs101 => &[
            "/lib/modules",
            "/vendor_dlkm/lib/modules",
            "/vendor/lib/modules",
            "/dev/ko_stage",
        ],
        _ => &[
            "/vendor_dlkm/lib/modules",
            "/lib/modules",
            "/vendor/lib/modules",
            "/dev/ko_stage",
        ],
    };
    roots.iter().map(PathBuf::from).collect()
}

/// Collect up to `cap` .ko paths under `roots` (recursive), USB-hinted
/// names first. Pure over enumerated file lists (testable); the live
/// walk is [`collect_stock_kos`].
fn order_candidates(mut files: Vec<PathBuf>, cap: usize) -> Vec<PathBuf> {
    files.sort_by_key(|p| {
        let n = p.file_name().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
        let hinted = USB_HINTS.iter().any(|h| n.contains(h));
        (!hinted, p.clone())
    });
    files.truncate(cap);
    files
}

fn visit_dir(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            visit_dir(&p, out);
        } else if p.extension().and_then(|e| e.to_str()) == Some("ko") {
            out.push(p);
        }
    }
}

fn collect_stock_kos(roots: &[PathBuf], cap: usize) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for r in roots {
        visit_dir(r, &mut files);
        if files.len() >= cap * 2 {
            break;
        }
    }
    order_candidates(files, cap)
}

fn info(msg: &str) {
    log_msg(TAG, "INFO", msg);
    println!("otg: INFO: {msg}");
}

/// The gate. Logs the verdict loudly either way; returns it.
pub fn check(fam: &Family) -> Gate {
    if !modules_supported() {
        info("gate: monolithic kernel, no module check (vendor stack owns USB)");
        return Gate::Proceed;
    }
    // Same live-state predicate as the touch branch
    // (`ko_picker::is_monolithic`): empty lsmod, nothing to check.
    if is_monolithic() {
        info("gate: lsmod empty, no loaded modules — nothing version-sensitive in play — proceeding");
        return Gate::Proceed;
    }
    let kernel_patch = match detect_kernel_env() {
        Ok(e) => {
            info(&format!("gate: running kernel {}.{}.{}", e.major, e.minor, e.patch));
            e.patch
        }
        Err(e) => {
            info(&format!("gate: uname failed ({e}), cannot prove mismatch — proceeding"));
            return Gate::Proceed;
        }
    };
    let roots = stock_roots(fam);
    let files = collect_stock_kos(&roots, SCAN_CAP);
    if files.is_empty() {
        info("gate: no readable stock .ko, cannot prove mismatch — proceeding");
        return Gate::Proceed;
    }
    let mut patches = Vec::new();
    for f in &files {
        if let Some((_, _, p)) = read_ko_vermagic(f) {
            patches.push(p);
        }
    }
    if patches.is_empty() {
        info("gate: stock .ko present but vermagic unreadable — proceeding");
        return Gate::Proceed;
    }
    match decide(kernel_patch, &patches) {
        Gate::Proceed => {
            info(&format!(
                "gate: firmware modules match kernel patch ({kernel_patch}) — custom OTG allowed"
            ));
            Gate::Proceed
        }
        Gate::Block => {
            let mut dist: Vec<(u32, usize)> = Vec::new();
            for p in &patches {
                match dist.iter_mut().find(|(v, _)| v == p) {
                    Some(e) => e.1 += 1,
                    None => dist.push((*p, 1)),
                }
            }
            info(&format!(
                "gate: BLOCKED — kernel patch {kernel_patch} vs firmware {dist:?} \
                 (custom/newer kernel on older firmware?). Custom OTG stack stays OFF; \
                 device mode only, no VBUS switch, no panic"
            ));
            Gate::Block
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ko_picker::parse_loaded_modules;

    #[test]
    fn loaded_modules_parsing() {
        // Sultan (custom): empty file, header-only lsmod, blanks.
        assert!(parse_loaded_modules("").is_empty());
        assert!(parse_loaded_modules("Module                  Size  Used by\n").is_empty());
        assert!(parse_loaded_modules("\n  \n").is_empty());
        // Stock GKI: names are the first column.
        let t = "touch_offload 28672 0\nsec_touch 667648 2 sec_touch,goog_touch_interface\n";
        assert_eq!(parse_loaded_modules(t), vec!["touch_offload", "sec_touch"]);
    }

    #[test]
    fn verdict_matrix() {
        // Match → proceed.
        assert_eq!(decide(157, &[157, 157, 145]), Gate::Proceed);
        // Majority mismatch → block.
        assert_eq!(decide(177, &[145, 145, 145]), Gate::Block);
        // One stray snapshot must not trip the gate (oriole dlkm case).
        assert_eq!(decide(157, &[157, 157, 75]), Gate::Proceed);
        // Tie → max_by_key picks deterministically; still a single sample
        // mismatch blocks (strict by design).
        assert_eq!(decide(157, &[145]), Gate::Block);
        // Empty = unprovable = proceed.
        assert_eq!(decide(157, &[]), Gate::Proceed);
    }

    #[test]
    fn roots_prioritize_gs101_root() {
        let g = stock_roots(&Family::Gs101);
        assert_eq!(g[0], PathBuf::from("/lib/modules"));
        let z = stock_roots(&Family::Zuma);
        assert_eq!(z[0], PathBuf::from("/vendor_dlkm/lib/modules"));
        // Our own shims are never scanned.
        for r in g.iter().chain(z.iter()) {
            assert!(!r.starts_with("/system"));
        }
    }

    #[test]
    fn usb_hints_sort_first() {
        let files = vec![
            PathBuf::from("/x/zzz_touch.ko"),
            PathBuf::from("/x/dwc3-exynos-usb.ko"),
            PathBuf::from("/x/aaa.ko"),
        ];
        let out = order_candidates(files, 32);
        assert_eq!(
            out[0].file_name().unwrap().to_str().unwrap(),
            "dwc3-exynos-usb.ko"
        );
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn cap_bounds_scan() {
        let files: Vec<PathBuf> =
            (0..100).map(|i| PathBuf::from(format!("/x/m{i}.ko"))).collect();
        assert_eq!(order_candidates(files, 32).len(), 32);
    }
}
