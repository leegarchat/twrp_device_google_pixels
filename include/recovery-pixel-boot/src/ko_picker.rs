//! ko_picker — kernel module selection and loading.
//!
//! Port of `_ko_try_load` from runatboot.sh / otg_patch.sh.
//! Selection matrix: uname major.minor.patch x pagesize x android generation.
//! Loading is always `finit_module(fd, "", flags=0)` — no force flags,
//! because target kernels are built with CONFIG_MODULE_FORCE_LOAD=n.
//!
//! Third (patch) kernel level: the independent touch/ prebuilt set carries
//! one directory per exact snapshot (touch/6.1/157/... vs touch/6.1/145/...).
//! An exact patch match scores a bonus but a mismatch is NOT a reject —
//! 6.1.x cross-loads in practice (field-proven .145 module on .157 kernel),
//! so wrong-patch candidates stay as scored fallbacks, nearest first.
//!
//! Presence = /proc/modules OR /sys/module (built-in drivers never appear
//! in the former). Monolithic kernels (no /proc/modules) need nothing:
//! see modules_supported(), honored by the touch chain before any load.

use std::ffi::CString;
use std::fs::OpenOptions;
use std::io::{Error, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

/// Log to both /tmp/recovery.log and /dev/kmsg (dmesg visibility).
pub fn log_msg(tag: &str, level: &str, msg: &str) {
    let line = format!("{tag}: {level}: {msg}\n");
    if let Ok(mut f) = OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/recovery.log")
    {
        let _ = f.write_all(line.as_bytes());
    }
    if let Ok(mut k) = OpenOptions::new().write(true).open("/dev/kmsg") {
        let _ = k.write_all(line.as_bytes());
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KernelEnv {
    pub major: u32,
    pub minor: u32,
    pub patch: u32, // third level: 157 in "6.1.157-..." (0 = unknown, no bonus)
    pub page_size: usize, // 4096 | 16384
    pub android_api: u32, // 13, 14, 15, ... (0 = unknown, no API bonus)
}

#[derive(Debug, PartialEq, Eq)]
pub struct ScoredCandidate<'a> {
    pub path: &'a Path,
    pub score: u32,
    pub kernel_match: bool,
    pub api_match: bool,
    pub patch_match: bool,
}

impl Ord for ScoredCandidate<'_> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.score
            .cmp(&other.score)
            .then_with(|| self.path.cmp(other.path))
    }
}

impl PartialOrd for ScoredCandidate<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

fn parse_api_token(token: &str) -> Option<u32> {
    let idx = token.find("android")?;
    let after = &token[idx + "android".len()..];
    let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse::<u32>().ok()
}

fn parse_kver_token(token: &str) -> Option<(u32, u32)> {
    let (maj_str, min_str) = token.split_once('.')?;
    // Reject tokens with trailing garbage in either half ("6x", "1ko", ...).
    if !maj_str.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let min_digits: String = min_str.bytes().take_while(|b| b.is_ascii_digit()).map(|b| b as char).collect();
    if min_digits.is_empty() || min_digits.len() != min_str.len() {
        return None;
    }
    Some((maj_str.parse::<u32>().ok()?, min_digits.parse::<u32>().ok()?))
}

/// Third kernel level from an "X.Y.Z" token ("6.1.157" with major=6,
/// minor=1 -> 157). Only full X.Y.Z tokens count: a bare "157" directory
/// is accepted separately (see below), never a bare filename token — a
/// bare number in a filename is far more likely a variant id than a patch.
fn parse_patch_token(token: &str, major: u32, minor: u32) -> Option<u32> {
    let prefix = format!("{major}.{minor}.");
    let rest = token.strip_prefix(prefix.as_str())?;
    if rest.is_empty() || !rest.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    rest.parse::<u32>().ok()
}

/// Score one .ko path against the running kernel environment.
/// Returns None when the candidate is rejected by the hard pagesize filter.
pub fn score_candidate<'a>(path: &'a Path, env: &KernelEnv) -> Option<ScoredCandidate<'a>> {
    let file_name = path.file_name()?.to_str()?;
    let parent_dir = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("");

    // 1. HARD FILTER: pagesize, strictly by file suffix + parent dir name.
    let is_16k_cand = file_name.ends_with("_16k.ko") || parent_dir == "16k";
    if env.page_size == 16384 && !is_16k_cand {
        return None;
    }
    if env.page_size == 4096 && is_16k_cand {
        return None;
    }

    // 2. Android generation: file_name wins over directory
    // (tree has android16 files under android14 dirs).
    let file_api = parse_api_token(file_name);
    let dir_api = parse_api_token(parent_dir);
    let cand_api = file_api.or(dir_api);

    // 3. Kernel X.Y: file_name tokens first, then path components.
    // Patch Z: full "X.Y.Z" tokens anywhere; a bare all-digit component
    // counts only AFTER the X.Y level was seen in path order (covers the
    // touch/6.1/157/... hierarchy for .ko files without a version stamp).
    let mut cand_kver: Option<(u32, u32)> = None;
    let mut cand_patch: Option<u32> = None;
    for token in file_name.trim_end_matches(".ko").split(['-', '_']) {
        if cand_kver.is_none() {
            if let Some(kv) = parse_kver_token(token) {
                cand_kver = Some(kv);
                continue;
            }
        }
        if cand_patch.is_none() {
            if let (Some((mj, mn)), Some(p)) = (cand_kver, parse_patch_token(token, env.major, env.minor)) {
                // Only honor Z when it extends the candidate's own X.Y.
                if mj == env.major && mn == env.minor {
                    cand_patch = Some(p);
                }
            }
        }
    }
    if cand_kver.is_none() || cand_patch.is_none() {
        let mut seen_kver = cand_kver.is_some();
        for component in path.iter() {
            let Some(comp_str) = component.to_str() else {
                continue;
            };
            if cand_kver.is_none() {
                if let Some(kv) = parse_kver_token(comp_str) {
                    cand_kver = Some(kv);
                    seen_kver = true;
                    continue;
                }
            }
            if cand_patch.is_none() {
                if let Some(p) = parse_patch_token(comp_str, env.major, env.minor) {
                    if cand_kver == Some((env.major, env.minor)) {
                        cand_patch = Some(p);
                    }
                    continue;
                }
                // Bare patch level ("157" in touch/6.1/157/...): only after X.Y.
                if seen_kver
                    && !comp_str.is_empty()
                    && comp_str.bytes().all(|b| b.is_ascii_digit())
                {
                    if let Ok(p) = comp_str.parse::<u32>() {
                        cand_patch = Some(p);
                    }
                }
            }
        }
    }

    let kernel_match = cand_kver == Some((env.major, env.minor));
    let api_match = env.android_api != 0 && cand_api == Some(env.android_api);
    let patch_match = env.patch != 0 && cand_patch == Some(env.patch);
    // Proximity among same-X.Y mismatches: nearer snapshot first
    // (0..=200, exact match keeps its own 300 below and always wins).
    let patch_proximity: u32 = match (env.patch, cand_patch) {
        (0, _) | (_, None) => 0,
        (e, Some(c)) if e == c => 0, // exact: counted separately
        (e, Some(c)) => 200u32.saturating_sub(10u32.saturating_mul(e.abs_diff(c).min(20))),
    };

    // 4. Scoring: kernel 1000 (fallback 10), patch exact 300, proximity
    // 0..=200, api 200, consistency 50+50. A patch mismatch is never a
    // reject: 6.1.x cross-loads in practice, so the wrong-patch file
    // stays a fallback, nearest first.
    let mut score: u32 = 0;
    if kernel_match {
        score += 1000;
    } else {
        // Wrong branch: finit_module will almost surely return -ENOEXEC,
        // but keep as last-resort candidate like the shell brute force did.
        score += 10;
    }
    if patch_match {
        score += 300;
    } else {
        score += patch_proximity;
    }
    if api_match {
        score += 200;
    }
    if file_api.is_some() && file_api == dir_api {
        score += 50;
    }
    if (env.page_size == 16384 && parent_dir == "16k")
        || (env.page_size == 4096 && parent_dir == "4k")
    {
        score += 50;
    }

    Some(ScoredCandidate {
        path,
        score,
        kernel_match,
        api_match,
        patch_match,
    })
}

/// Detect the running kernel environment.
/// major.minor + android generation come from `uname -r` ("6.1.157-android14-11");
/// pagesize from sysconf (auxv fallback not needed — libc covers it).
pub fn detect_kernel_env() -> Result<KernelEnv, Error> {
    // SAFETY: zeroed() then fully overwritten by uname(2) on success; the
    // buffer is never read if uname fails (early return below).
    let mut uts: libc::utsname = unsafe { std::mem::zeroed() };
    // SAFETY: uts is a valid mutable utsname; return value checked.
    if unsafe { libc::uname(&mut uts) } != 0 {
        return Err(Error::last_os_error());
    }
    // NUL-terminated by the kernel (truncated with NUL on overflow); CStr
    // avoids any c_char signedness casts across aarch64/x86_64.
    // SAFETY: release is a valid NUL-terminated C string per uname(2).
    let release = unsafe { std::ffi::CStr::from_ptr(uts.release.as_ptr()) }
        .to_string_lossy()
        .into_owned();
    // "6.1.157-android14-11" -> major=6 minor=1 patch=157
    let mut parts = release.split('.');
    let major: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let minor: u32 = parts
        .next()
        .map(|s| s.bytes().take_while(|b| b.is_ascii_digit()).map(|b| b as char).collect::<String>())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let patch: u32 = parts
        .next()
        .map(|s| s.bytes().take_while(|b| b.is_ascii_digit()).map(|b| b as char).collect::<String>())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    // android generation from uname first ("android14"), else SDK property map.
    let mut android_api = parse_api_token(&release).unwrap_or(0);
    if android_api == 0 {
        android_api = match crate::props::get_prop("ro.build.version.sdk").as_str() {
            "33" => 13,
            "34" => 14,
            "35" => 15,
            "36" => 16,
            _ => 0,
        };
    }
    let ps = {
        // SAFETY: sysconf with a valid _SC_* constant: no pointers, no state.
        unsafe { libc::sysconf(libc::_SC_PAGESIZE) }
    };
    let page_size = if ps > 0 { ps as usize } else { 4096 };
    Ok(KernelEnv {
        major,
        minor,
        patch,
        page_size,
        android_api,
    })
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
        } else if p.is_file() {
            out.push(p);
        }
    }
}

/// Recursively collect `<module_name>*.ko` files under search_root,
/// ordered best-first. Matches both flat and hierarchical layouts.
pub fn find_candidates(search_root: &Path, module_name: &str, env: &KernelEnv) -> Vec<PathBuf> {
    let mut files = Vec::new();
    visit_dir(search_root, &mut files);
    let mut scored: Vec<(u32, PathBuf)> = Vec::new();
    for p in files {
        let name = match p.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => continue,
        };
        if !name.starts_with(module_name) || !name.ends_with(".ko") {
            continue;
        }
        if let Some(sc) = score_candidate(&p, env) {
            scored.push((sc.score, p));
        }
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, p)| p).collect()
}

/// Load one .ko via finit_module(fd, "", flags=0).
pub fn load_kernel_module(path: &Path) -> Result<(), Error> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC)
        .open(path)?;
    let empty_params = CString::new("").unwrap();
    // Raw syscall: bionic only exports the finit_module wrapper since API 35,
    // while recovery targets API 34. libc::SYS_finit_module is arch-correct.
    // SAFETY: fd is our open .ko, params points to a live empty CString,
    // flags is strictly 0 (no IGNORE_VERMAGIC/MODVERSIONS).
    let ret = unsafe {
        libc::syscall(
            libc::SYS_finit_module,
            file.as_raw_fd(),
            empty_params.as_ptr(),
            0 as libc::c_int, // flags STRICTLY 0 (no IGNORE_VERMAGIC/MODVERSIONS)
        )
    };
    if ret != 0 {
        let err = Error::last_os_error();
        // Already loaded (e.g. as another module's dependency): not a failure.
        if err.raw_os_error() == Some(libc::EEXIST) {
            return Ok(());
        }
        return Err(err);
    }
    Ok(())
}

/// Force-load a .ko, ignoring vermagic/modversion mismatches.
///
/// finit_module(2) flags: MODULE_INIT_IGNORE_MODVERSIONS (0x1) +
/// MODULE_INIT_IGNORE_VERMAGIC (0x2). Recovery-only escape hatch for OUR
/// OWN shim (kprobe + procfs: ABI-stable across 6.1.x) when the prebuilt
/// was stamped against a different 6.1.x snapshot than the running stock
/// kernel (e.g. 6.1.176 prebuilt on a 6.1.157 device kernel). NEVER use
/// for stock vendor modules (those always match by construction).
/// EEXIST (already loaded) still counts as success.
pub fn load_kernel_module_force(path: &Path) -> Result<(), Error> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC)
        .open(path)?;
    let empty_params = CString::new("").unwrap();
    // SAFETY: same contract as load_kernel_module; flags 0x3 is the
    // documented finit_module ignore mask (uapi linux/module.h).
    let ret = unsafe {
        libc::syscall(
            libc::SYS_finit_module,
            file.as_raw_fd(),
            empty_params.as_ptr(),
            0x1 | 0x2,
        )
    };
    if ret != 0 {
        let err = Error::last_os_error();
        if err.raw_os_error() == Some(libc::EEXIST) {
            return Ok(());
        }
        return Err(err);
    }
    Ok(())
}

/// Already loaded? (mirrors `grep -q "^<mod> " /proc/modules`).
///
/// The kernel reports module names with underscores (`dwc3_exynos_usb`)
/// while callers use the file spelling with dashes (`dwc3-exynos-usb`),
/// so both variants are matched (field-proven on zuma: the whole Samsung
/// USB stack pre-loads from the vendor_kernel_boot ramdisk and was
/// misreported as missing).
/// Pure core of [`is_module_loaded`] over a /proc/modules dump (testable).
fn module_present(content: &str, module_name: &str) -> bool {
    let dashed = format!("{module_name} ");
    let underscored = format!("{} ", module_name.replace('-', "_"));
    content
        .lines()
        .any(|l| l.starts_with(&dashed) || l.starts_with(&underscored))
}

pub fn is_module_loaded(module_name: &str) -> bool {
    if let Ok(c) = std::fs::read_to_string("/proc/modules") {
        if module_present(&c, module_name) {
            return true;
        }
    }
    // Built-in drivers never appear in /proc/modules; their /sys/module
    // entry is the presence proof (no insmod needed, and attempting one
    // only errors out).
    sysfs_module_present(Path::new("/sys/module"), module_name)
}

/// `/sys/module/<name>` presence over an explicit root (testable).
fn sysfs_module_present(sys_root: &Path, module_name: &str) -> bool {
    sys_root.join(module_name).is_dir()
        || sys_root.join(module_name.replace('-', "_")).is_dir()
}

/// Module loading is meaningful only when the kernel supports modules.
/// Monolithic builds (CONFIG_MODULES=n, no /proc/modules) need nothing:
/// every driver is built in. Pure over the proc root (testable).
fn modules_supported_at(proc_root: &Path) -> bool {
    proc_root.join("modules").exists()
}

/// This boot's answer (live /proc).
pub fn modules_supported() -> bool {
    modules_supported_at(Path::new("/proc"))
}

/// Live loaded-module set: first column of /proc/modules (the same
/// source `lsmod` prints). Pure over file text (testable).
pub fn parse_loaded_modules(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| {
            let name = l.split_whitespace().next()?;
            // lsmod pretty-print header, not a module (/proc/modules itself
            // has no header, but be tolerant of either input).
            if name.is_empty() || name == "Module" {
                return None;
            }
            Some(name.to_string())
        })
        .collect()
}

/// Whatever the kernel has live right now. Empty on a fresh custom
/// build (Sultan: support compiled in, nothing staged loads) and on a
/// true monolithic kernel alike.
pub fn loaded_module_names() -> Vec<String> {
    match std::fs::read_to_string("/proc/modules") {
        Ok(t) => parse_loaded_modules(&t),
        Err(_) => Vec::new(),
    }
}

/// Monolithic-for-our-purposes: lsmod is empty, so there is nothing
/// version-sensitive in play — every driver we could inject is either
/// built in or was never staged. This is the same live-state predicate
/// the USB gate uses, and the touch branch must agree with it: on
/// monolith the whole module-injection branch is skipped, touch is
/// interpreted as working, and the HW GUI fallback never fires.
pub fn is_monolithic() -> bool {
    loaded_module_names().is_empty()
}

/// `vermagic=` triplet out of raw .ko bytes ("6.1.157-android14-..." ->
/// (6, 1, 157)). Pure (testable); backing for the OTG fool-proof gate,
/// which cannot ask the kernel what a loaded module was built for.
pub fn ko_vermagic(data: &[u8]) -> Option<(u32, u32, u32)> {
    const NEEDLE: &[u8] = b"vermagic=";
    let pos = data.windows(NEEDLE.len()).position(|w| w == NEEDLE)?;
    let rest = &data[pos + NEEDLE.len()..];
    let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
    let s = std::str::from_utf8(&rest[..end]).ok()?;
    let ver = s.split('-').next()?;
    let mut it = ver.split('.');
    let maj: u32 = it.next()?.parse().ok()?;
    let min: u32 = it.next()?.parse().ok()?;
    let pat: u32 = it.next()?.parse().ok()?;
    Some((maj, min, pat))
}

/// vermagic triplet of a .ko FILE (None when unreadable).
pub fn read_ko_vermagic(path: &Path) -> Option<(u32, u32, u32)> {
    let data = std::fs::read(path).ok()?;
    // .modinfo lives early; 1 MiB cap keeps the gate cheap on huge files.
    ko_vermagic(&data[..data.len().min(1 << 20)])
}

/// Persistent per-boot record of successfully injected modules.
///
/// `/proc/modules` answers "loaded RIGHT NOW". Dependent extras (vibro
/// firmware, panel helpers, ...) gate on "touch was already injected this
/// boot, proceed immediately" — without re-running the whole fetch/score
/// chain. Every successful injection path records here (strict, force,
/// EEXIST, and the already-loaded fast paths), so the record is complete
/// regardless of which loader won. Volatile by design: /tmp dies on reboot.
const LOADED_RECORD_PATH: &str = "/tmp/fox_ko_loaded";

fn normalize_module_name(name: &str) -> String {
    name.replace('-', "_")
}

/// Append `module_name` to the record (deduplicated). Best-effort: a
/// missing /tmp never fails the boot.
pub fn record_loaded_module(module_name: &str) {
    let norm = normalize_module_name(module_name);
    if let Ok(content) = std::fs::read_to_string(LOADED_RECORD_PATH) {
        if content.lines().any(|l| l.trim() == norm) {
            return;
        }
    }
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(LOADED_RECORD_PATH) {
        let _ = writeln!(f, "{norm}");
    }
}

/// True when the module was recorded this boot OR is live right now.
/// Extras use this as their "touch already injected, go" gate.
pub fn is_module_recorded(module_name: &str) -> bool {
    let norm = normalize_module_name(module_name);
    if let Ok(content) = std::fs::read_to_string(LOADED_RECORD_PATH) {
        if content.lines().any(|l| l.trim() == norm) {
            return true;
        }
    }
    is_module_loaded(&norm)
}

/// Pick the best .ko for this kernel and load it.
/// Returns true when loaded or already loaded.
pub fn ko_try_load(module_name: &str, proc_entry: Option<&str>, tag: &str) -> bool {
    // Monolithic kernel (CONFIG_MODULES=n): nothing to do.
    if !Path::new("/proc/modules").exists() {
        log_msg(tag, "INFO", "monolithic kernel (/proc/modules absent), skipping");
        return true;
    }
    if let Some(pe) = proc_entry {
        if Path::new(pe).exists() {
            record_loaded_module(module_name);
            return true;
        }
    }
    if is_module_loaded(module_name) {
        record_loaded_module(module_name);
        return true;
    }
    let env = match detect_kernel_env() {
        Ok(e) => e,
        Err(e) => {
            log_msg(tag, "ERROR", &format!("detect_kernel_env failed: {e}"));
            return false;
        }
    };
    log_msg(
        tag,
        "INFO",
        &format!(
            "kernel {}.{} pagesize={} api={}",
            env.major, env.minor, env.page_size, env.android_api
        ),
    );
    let cands = find_candidates(Path::new("/system/lib64/modules"), module_name, &env);
    if cands.is_empty() {
        log_msg(tag, "ERROR", &format!("no candidate files for {module_name}"));
        return false;
    }
    for cand in &cands {
        log_msg(tag, "INFO", &format!("trying {}", cand.display()));
        match load_kernel_module(cand) {
            Ok(()) => {
                log_msg(tag, "INFO", &format!("loaded {}", cand.display()));
                record_loaded_module(module_name);
                return true;
            }
            Err(e) => {
                log_msg(tag, "INFO", &format!("failed {}: {e}", cand.display()));
            }
        }
    }
    log_msg(tag, "ERROR", &format!("all candidates failed for {module_name}"));
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    pub struct TempDir(pub PathBuf);
    impl TempDir {
        pub fn new(name: &str) -> Self {
            let p = std::env::temp_dir().join(format!("fox_test_{name}_{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn module_present_tolerates_dash_underscore() {
        // Real zuma /proc/modules fragment: kernel spells underscores.
        let dump = "dwc3_exynos_usb 49152 4 aoc_usb_driver,exynos_drm,xhci_exynos,exynos_pd, Live 0x0000000000000000 (O)\n\
                    tcpci_max77759 73728 11 google_cpm,google_charger,pca9468,ln8411, Live 0x0000000000000000 (O)\n\
                    gvotable 49152 13 google_bcl, Live 0x0000000000000000 (O)\n";
        // Dashed file spelling matches the underscored live entry.
        assert!(module_present(dump, "dwc3-exynos-usb"));
        assert!(module_present(dump, "dwc3_exynos_usb"));
        assert!(module_present(dump, "tcpci_max77759"));
        assert!(module_present(dump, "gvotable"));
        // Prefix-only hits must not match (google_charger is a *dependency*,
        // not a loaded module line here).
        assert!(!module_present(dump, "google-charger"));
        assert!(!module_present(dump, "dwc3"));
        assert!(!module_present("", "gvotable"));
    }

    #[test]
    fn exact_match_beats_fallbacks() {
        let env = KernelEnv {
            major: 6,
            minor: 1,
            patch: 0,
            page_size: 4096,
            android_api: 14,
        };
        let exact = Path::new("/system/lib64/modules/otg/6.1/android14/4k/otg_host_shim_android14-6.1_4k.ko");
        let old_api = Path::new("/system/lib64/modules/otg/6.1/android13/4k/otg_host_shim_android13-6.1_4k.ko");
        let old_k = Path::new("/system/lib64/modules/otg/5.15/android14/4k/otg_host_shim_android14-5.15_4k.ko");
        let wrong_ps = Path::new("/system/lib64/modules/otg/6.1/android14/16k/otg_host_shim_android14-6.1_16k.ko");

        let r_exact = score_candidate(exact, &env).unwrap();
        // 1000 (kver) + 200 (api) + 0 (parent "4k" carries no API token,
        // no file/dir consistency bonus) + 50 (4k dir).
        assert_eq!(r_exact.score, 1250); // 1000 kver + 200 api + 50 4k dir
        assert!(score_candidate(wrong_ps, &env).is_none());
        let r_old_api = score_candidate(old_api, &env).unwrap();
        assert_eq!(r_old_api.score, 1050); // 1000 kver + 50 4k dir
        let r_old_k = score_candidate(old_k, &env).unwrap();
        assert_eq!(r_old_k.score, 260); // 10 fallback + 200 api + 50 4k dir
        assert!(r_exact > r_old_api && r_old_api > r_old_k);
    }

    #[test]
    fn file_api_wins_over_dir() {
        let env = KernelEnv {
            major: 6,
            minor: 12,
            patch: 0,
            page_size: 4096,
            android_api: 14,
        };
        // android16 file inside an android14 dir (real tree case).
        let desync = Path::new("system/lib64/modules/otg/6.12/android14/4k/otg_host_shim_android16-6.12_4k.ko");
        let r = score_candidate(desync, &env).unwrap();
        assert!(r.kernel_match);
        assert!(!r.api_match);
        // 1000 (kver) + 0 (api) + 0 (file!=dir, no consistency bonus) + 50 (4k dir)
        assert_eq!(r.score, 1050);
    }

    #[test]
    fn unknown_api_gets_no_bonus_but_still_loads() {
        let env = KernelEnv {
            major: 6,
            minor: 1,
            patch: 0,
            page_size: 4096,
            android_api: 0,
        };
        let p = Path::new("/m/otg_host_shim_android14-6.1_4k.ko");
        let r = score_candidate(p, &env).unwrap();
        assert!(r.kernel_match && !r.api_match);
    }

    #[test]
    fn find_candidates_orders_best_first() {
        let t = TempDir::new("ko_find");
        let mk = |rel: &str| {
            let p = t.0.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, b"fake-ko").unwrap();
        };
        mk("otg/5.15/android14/4k/otg_host_shim_android14-5.15_4k.ko");
        mk("otg/6.1/android14/4k/otg_host_shim_android14-6.1_4k.ko");
        mk("otg/6.1/android14/16k/otg_host_shim_android14-6.1_16k.ko");
        mk("otg/6.1/android14/4k/unrelated.ko");
        let env = KernelEnv {
            major: 6,
            minor: 1,
            patch: 0,
            page_size: 4096,
            android_api: 14,
        };
        let c = find_candidates(&t.0, "otg_host_shim", &env);
        assert_eq!(c.len(), 2);
        assert!(c[0].to_string_lossy().contains("6.1/android14/4k"));
    }

    #[test]
    fn patch_exact_beats_patch_mismatch() {
        // touch/ layout: one dir per snapshot, version stamped in filename.
        let env = KernelEnv {
            major: 6,
            minor: 1,
            patch: 157,
            page_size: 4096,
            android_api: 14,
        };
        let exact = Path::new("/system/lib64/modules/touch/6.1/157/android14/4k/goodix_brl_touch_android14-6.1.157_4k.ko");
        let stale = Path::new("/system/lib64/modules/touch/6.1/145/android14/4k/goodix_brl_touch_android14-6.1.145_4k.ko");
        let r_exact = score_candidate(exact, &env).unwrap();
        let r_stale = score_candidate(stale, &env).unwrap();
        assert!(r_exact.patch_match);
        assert!(!r_stale.patch_match);
        // 1000 (kver) + 300 (patch) + 200 (api) + 0 + 50 (4k dir).
        assert_eq!(r_exact.score, 1550); // 1000 kver + 300 patch + 200 api + 50 4k dir
        // Mismatch is NOT a reject: still a scored fallback (cross-loads).
        // Proximity: |157-145|=12 -> 200-120=80.
        assert_eq!(r_stale.score, 1330); // 1000 kver + 80 proximity + 200 api + 50 4k dir
        assert!(r_exact > r_stale);
    }

    #[test]
    fn patch_proximity_prefers_nearest() {
        // Same branch, two stale snapshots: the nearer wins, exact wins all.
        let env = KernelEnv {
            major: 6,
            minor: 1,
            patch: 157,
            page_size: 4096,
            android_api: 14,
        };
        let near = Path::new("/m/touch/6.1/162/android14/4k/m.ko");
        let far = Path::new("/m/touch/6.1/145/android14/4k/m.ko");
        let other_branch = Path::new("/m/touch/6.1/162/android14/4k/m.ko");
        let rn = score_candidate(near, &env).unwrap();
        let rf = score_candidate(far, &env).unwrap();
        assert!(!rn.patch_match && !rf.patch_match);
        assert!(rn.score > rf.score, "162 must beat 145 on a 157 kernel");
        // Exact still unbeatable: 300 > any proximity (<=200).
        let exact = Path::new("/m/touch/6.1/157/android14/4k/m.ko");
        let re = score_candidate(exact, &env).unwrap();
        assert!(re.score > rn.score);
        // Cross-branch stays last resort regardless of patch number.
        let env5 = KernelEnv { major: 5, minor: 15, patch: 110, page_size: 4096, android_api: 14 };
        let r5 = score_candidate(other_branch, &env5).unwrap();
        assert!(!r5.kernel_match);
        assert!(r5.score < rf.score);
    }

    #[test]
    fn bare_patch_dir_counts_after_kver() {
        // Unstamped .ko under the patch-level dir still matches by hierarchy.
        let env = KernelEnv {
            major: 6,
            minor: 1,
            patch: 157,
            page_size: 4096,
            android_api: 14,
        };
        let p = Path::new("/system/lib64/modules/touch/6.1/157/android14/4k/goog_touch_interface.ko");
        let r = score_candidate(p, &env).unwrap();
        assert!(r.kernel_match);
        assert!(r.patch_match);
    }

    #[test]
    fn bare_number_before_kver_is_not_a_patch() {
        // A bare number with no X.Y context must not invent a patch match.
        let env = KernelEnv {
            major: 6,
            minor: 1,
            patch: 157,
            page_size: 4096,
            android_api: 14,
        };
        let p = Path::new("/m/157/goodix_brl_touch.ko");
        let r = score_candidate(p, &env).unwrap();
        assert!(!r.kernel_match);
        assert!(!r.patch_match);
    }

    #[test]
    fn sysfs_presence_covers_builtin_spelling() {
        let t = TempDir::new("ko_sysfs");
        std::fs::create_dir_all(t.0.join("sys").join("goodix_brl_touch")).unwrap();
        let sys = t.0.join("sys");
        assert!(sysfs_module_present(&sys, "goodix_brl_touch"));
        assert!(sysfs_module_present(&sys, "goodix-brl-touch"));
        assert!(!sysfs_module_present(&sys, "nope"));
    }

    #[test]
    fn monolithic_gate_reads_proc_root() {
        let t = TempDir::new("ko_mono");
        let proc = t.0.join("proc");
        std::fs::create_dir_all(&proc).unwrap();
        assert!(!modules_supported_at(&proc));
        std::fs::write(proc.join("modules"), b"").unwrap();
        assert!(modules_supported_at(&proc));
    }

    #[test]
    fn vermagic_triplet_parsing() {
        let mk = |v: &[u8]| {
            let mut d = b"\x7fELF....vermagic=".to_vec();
            d.extend_from_slice(v);
            d.push(0);
            d.extend_from_slice(b"more");
            d
        };
        assert_eq!(ko_vermagic(&mk(b"6.1.157-android14-11-gabc")), Some((6, 1, 157)));
        assert_eq!(ko_vermagic(&mk(b"5.15.110-android14-xyz")), Some((5, 15, 110)));
        assert_eq!(ko_vermagic(&mk(b"6.6.118")), Some((6, 6, 118)));
        assert_eq!(ko_vermagic(b"no needle here"), None);
        assert_eq!(ko_vermagic(&mk(b"6.1")), None);
        assert_eq!(ko_vermagic(&mk(b"6.x.1-foo")), None);
    }

    #[test]
    fn loaded_record_roundtrip() {
        // Record file: dash/underscore spellings unify, unknown names miss.
        let name = format!("fox_test_mod_{}", std::process::id());
        assert!(!is_module_recorded(&name));
        record_loaded_module(&name.replace('_', "-"));
        assert!(is_module_recorded(&name));
        assert!(is_module_recorded(&name.replace('_', "-")));
        assert!(!is_module_recorded(&format!("{name}_nope")));
    }
}
