//! ko_picker — kernel module selection and loading.
//!
//! Port of `_ko_try_load` from runatboot.sh / otg_patch.sh.
//! Selection matrix: uname major.minor x pagesize x android generation.
//! Loading is always `finit_module(fd, "", flags=0)` — no force flags,
//! because target kernels are built with CONFIG_MODULE_FORCE_LOAD=n.

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
    pub page_size: usize, // 4096 | 16384
    pub android_api: u32, // 13, 14, 15, ... (0 = unknown, no API bonus)
}

#[derive(Debug, PartialEq, Eq)]
pub struct ScoredCandidate<'a> {
    pub path: &'a Path,
    pub score: u32,
    pub kernel_match: bool,
    pub api_match: bool,
}

impl<'a> Ord for ScoredCandidate<'a> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.score
            .cmp(&other.score)
            .then_with(|| self.path.cmp(other.path))
    }
}

impl<'a> PartialOrd for ScoredCandidate<'a> {
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
    let mut cand_kver: Option<(u32, u32)> = None;
    for token in file_name.trim_end_matches(".ko").split(|c: char| c == '-' || c == '_') {
        if let Some(kv) = parse_kver_token(token) {
            cand_kver = Some(kv);
            break;
        }
    }
    if cand_kver.is_none() {
        for component in path.iter() {
            if let Some(comp_str) = component.to_str() {
                if let Some(kv) = parse_kver_token(comp_str) {
                    cand_kver = Some(kv);
                    break;
                }
            }
        }
    }

    let kernel_match = cand_kver == Some((env.major, env.minor));
    let api_match = env.android_api != 0 && cand_api == Some(env.android_api);

    // 4. Scoring: kernel 1000 (fallback 10), api 200, consistency 50+50.
    let mut score: u32 = 0;
    if kernel_match {
        score += 1000;
    } else {
        // Wrong branch: finit_module will almost surely return -ENOEXEC,
        // but keep as last-resort candidate like the shell brute force did.
        score += 10;
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
    // "6.1.157-android14-11" -> major=6 minor=1
    let mut parts = release.split('.');
    let major: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let minor: u32 = parts
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
    match std::fs::read_to_string("/proc/modules") {
        Ok(c) => module_present(&c, module_name),
        Err(_) => false,
    }
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
            return true;
        }
    }
    if is_module_loaded(module_name) {
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
        assert_eq!(r_exact.score, 1000 + 200 + 0 + 50);
        assert!(score_candidate(wrong_ps, &env).is_none());
        let r_old_api = score_candidate(old_api, &env).unwrap();
        assert_eq!(r_old_api.score, 1000 + 0 + 0 + 50);
        let r_old_k = score_candidate(old_k, &env).unwrap();
        assert_eq!(r_old_k.score, 10 + 200 + 0 + 50);
        assert!(r_exact > r_old_api && r_old_api > r_old_k);
    }

    #[test]
    fn file_api_wins_over_dir() {
        let env = KernelEnv {
            major: 6,
            minor: 12,
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
            page_size: 4096,
            android_api: 14,
        };
        let c = find_candidates(&t.0, "otg_host_shim", &env);
        assert_eq!(c.len(), 2);
        assert!(c[0].to_string_lossy().contains("6.1/android14/4k"));
    }
}
