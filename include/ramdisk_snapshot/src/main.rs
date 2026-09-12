//! ramdisk_snapshot — copy ramdisk state (pre-LGZ-unpack) to a snapshot dir.
//!
//! Rust port of ramdisk_snapshot.c. Reads a manifest file
//! (`<type> <octal_perms> <uid> <gid> <path>`, symlinks as
//! `l <perms> <uid> <gid> <path> -> <target>`) and copies every entry from
//! the root filesystem to a snapshot directory (default
//! `/dev/ramdisk_snapshot`). Preserves the exact ramdisk state (packed
//! cluster included) so reflash_twrp.sh can rebuild the vendor_boot cpio.
//! Also snapshots the current vendor_boot block device.
//!
//! Exit code: 0 ok, 1 if any entry failed (init treats this as a warning).

use std::ffi::CString;
#[cfg(target_os = "android")]
use std::ffi::CStr;
use std::fs;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

const DEFAULT_MANIFEST: &str = "/ramdisk_snapshot_manifest.txt";
const DEFAULT_SNAP_DIR: &str = "/dev/ramdisk_snapshot";
const VENDOR_BOOT_SNAP: &str = "/dev/vendor_boot_snapshot";
const LOG_FILE: &str = "/dev/vendor_boot_snapshot/logs";

// ---------------------------------------------------------------------------
// logging (stdout/stderr + optional log file, like the C version)
// ---------------------------------------------------------------------------

struct Logger {
    file: Option<fs::File>,
}

impl Logger {
    fn new() -> Logger {
        let _ = fs::create_dir_all(VENDOR_BOOT_SNAP);
        let file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(LOG_FILE)
            .ok();
        let mut log = Logger { file };
        if log.file.is_some() {
            log.out(false, "\n=========================================\n--- Starting ramdisk_snapshot ---\n=========================================\n");
        }
        log
    }

    fn out(&mut self, is_error: bool, msg: &str) {
        if is_error {
            eprint!("{msg}");
        } else {
            print!("{msg}");
        }
        if let Some(f) = self.file.as_mut() {
            let _ = f.write_all(msg.as_bytes());
            let _ = f.flush();
        }
    }

    fn info(&mut self, args: std::fmt::Arguments<'_>) {
        self.out(false, &format!("{args}"));
    }

    fn err(&mut self, args: std::fmt::Arguments<'_>) {
        self.out(true, &format!("{args}"));
    }
}

// ---------------------------------------------------------------------------
// fs helpers
// ---------------------------------------------------------------------------

/// Recursively create directories along a path (mkdir -p),
/// applying the mode to each created component like the C version.
fn mkdirs(path: &Path, mode: u32) {
    let mut current = PathBuf::new();
    for comp in path.components() {
        current.push(comp);
        if current.as_os_str().is_empty() {
            continue;
        }
        let _ = fs::create_dir(&current);
        let _ = set_mode(&current, mode);
    }
}

fn c_path(path: &Path) -> Option<CString> {
    CString::new(path.as_os_str().as_bytes()).ok()
}

fn set_mode(path: &Path, mode: u32) -> bool {
    match c_path(path) {
        // Safety: `c` is a valid NUL-terminated path; chmod has no
        // additional preconditions and its error is returned as bool.
        Some(c) => unsafe { libc::chmod(c.as_ptr(), mode) == 0 },
        None => false,
    }
}

fn ensure_parent(path: &Path) {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            mkdirs(parent, 0o755);
        }
    }
}

/// Apply ownership/mtimes from the LIVE source, falling back to manifest
/// values when the source cannot be lstatted (same policy as the C version).
fn apply_metadata(
    log: &mut Logger,
    src: &Path,
    dst: &Path,
    m_mode: u32,
    m_uid: u32,
    m_gid: u32,
    is_link: bool,
) {
    // Safety: all pointers passed below come from `CString`s that outlive
    // the calls; `st` is a valid zeroed `libc::stat` for lstat output.
    // chown/chmod/lchown/utimensat failures are intentionally ignored
    // (best-effort metadata, same as the C version).
    unsafe {
        let mut st: libc::stat = std::mem::zeroed();
        let use_live = c_path(src)
            .map(|c| libc::lstat(c.as_ptr(), &mut st) == 0)
            .unwrap_or(false);
        let (uid, gid) = if use_live {
            (st.st_uid, st.st_gid)
        } else {
            (m_uid, m_gid)
        };
        if let Some(c) = c_path(dst) {
            if is_link || (use_live && (st.st_mode & libc::S_IFMT) == libc::S_IFLNK) {
                libc::lchown(c.as_ptr(), uid, gid);
            } else {
                libc::chown(c.as_ptr(), uid, gid);
                let mode = if use_live { st.st_mode & 0o7777 } else { m_mode & 0o7777 };
                libc::chmod(c.as_ptr(), mode);
            }
            if use_live {
                let times = [
                    libc::timespec { tv_sec: st.st_atime, tv_nsec: st.st_atime_nsec },
                    libc::timespec { tv_sec: st.st_mtime, tv_nsec: st.st_mtime_nsec },
                ];
                libc::utimensat(libc::AT_FDCWD, c.as_ptr(), times.as_ptr(), libc::AT_SYMLINK_NOFOLLOW);
            }
        }
        let _ = log;
    }
}

fn copy_file(log: &mut Logger, src: &Path, dst: &Path) -> bool {
    match fs::copy(src, dst) {
        Ok(_) => true,
        Err(e) => {
            log.err(format_args!("[SNAPSHOT] copy({}) -> {}: {e}\n", src.display(), dst.display()));
            false
        }
    }
}

// ---------------------------------------------------------------------------
// debug dumps (cmdline / bootconfig / props / UFS dir)
// ---------------------------------------------------------------------------

fn dump_file(log: &mut Logger, title: &str, path: &str) {
    log.info(format_args!("\n--- DUMPING {title} ---\n"));
    match fs::read_to_string(path) {
        Ok(text) => {
            log.info(format_args!("{text}"));
            if !text.ends_with('\n') {
                log.info(format_args!("\n"));
            }
        }
        Err(_) => log.err(format_args!("[SNAPSHOT] cannot open {path}\n")),
    }
}

// Bionic property iteration (linked from libc at Soong build time;
// declared manually so plain cargo check passes without linking).
// Opaque handle type: only ever passed behind a pointer.
#[allow(dead_code)]
#[cfg(target_os = "android")]
enum PropInfo {}

#[cfg(target_os = "android")]
type PropReadCb = extern "C" fn(*mut libc::c_void, *const libc::c_char, *const libc::c_char, u32);

#[cfg(target_os = "android")]
extern "C" {
    fn __system_property_foreach(
        func: extern "C" fn(*const PropInfo, *mut libc::c_void),
        cookie: *mut libc::c_void,
    ) -> i32;
    fn __system_property_read_callback(
        pi: *const PropInfo,
        func: PropReadCb,
        cookie: *mut libc::c_void,
    );
}

#[cfg(target_os = "android")]
extern "C" fn prop_read_cb(
    _cookie: *mut libc::c_void,
    name: *const libc::c_char,
    value: *const libc::c_char,
    _serial: u32,
) {
    // Safety: bionic guarantees non-null NUL-terminated name/value here.
    let name = unsafe { CStr::from_ptr(name).to_string_lossy() };
    // Safety: same guarantee for the value pointer.
    let value = unsafe { CStr::from_ptr(value).to_string_lossy() };
    eprintln!("[{name}]: [{value}]");
}

#[cfg(target_os = "android")]
extern "C" fn prop_foreach_cb(pi: *const PropInfo, cookie: *mut libc::c_void) {
    // Safety: `pi` comes from the foreach iterator, `cookie` is unused (null).
    unsafe { __system_property_read_callback(pi, prop_read_cb, cookie) };
}

#[cfg(target_os = "android")]
fn dump_getprop(log: &mut Logger) {
    log.info(format_args!("\n--- DUMPING PROPERTIES (Native Bionic) ---\n"));
    // Safety: stateless bionic iterator with a valid callback and null cookie.
    let rc = unsafe { __system_property_foreach(prop_foreach_cb, std::ptr::null_mut()) };
    if rc != 0 {
        log.err(format_args!("[SNAPSHOT] cannot read properties (service not up yet?)\n"));
    }
    log.info(format_args!("--------------------------\n"));
}

/// Host builds (cargo test) have no bionic: skip the property dump.
#[cfg(not(target_os = "android"))]
fn dump_getprop(log: &mut Logger) {
    log.info(format_args!("\n--- DUMPING PROPERTIES (skipped: host build) ---\n"));
}

fn find_ufs_platform_path() -> (PathBuf, bool) {
    if let Ok(entries) = fs::read_dir("/dev/block/platform") {
        for ent in entries.flatten() {
            let name = ent.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            if name.contains("ufs") {
                return (PathBuf::from(format!("/dev/block/platform/{name}")), true);
            }
        }
    }
    // Fallback: GS201 default (matches the C version).
    (PathBuf::from("/dev/block/platform/13200000.ufs"), false)
}

fn dump_dir_contents(log: &mut Logger, dir_path: &Path) {
    log.info(format_args!("[SNAPSHOT] Contents of {}:\n", dir_path.display()));
    match fs::read_dir(dir_path) {
        Ok(entries) => {
            for ent in entries.flatten() {
                let name = ent.file_name().to_string_lossy().into_owned();
                let full = dir_path.join(&name);
                match fs::read_link(&full) {
                    Ok(target) => log.info(format_args!("  - {name} -> {}\n", target.display())),
                    Err(_) => log.info(format_args!("  - {name}\n")),
                }
            }
        }
        Err(e) => log.info(format_args!("  (cannot open directory: {e})\n")),
    }
}

fn dump_debug_info(log: &mut Logger) {
    dump_file(log, "/proc/cmdline", "/proc/cmdline");
    dump_file(log, "/proc/bootconfig", "/proc/bootconfig");
    dump_getprop(log);
    log.info(format_args!("\n--- DUMPING TARGET DIRECTORIES ---\n"));
    let (ufs_path, found) = find_ufs_platform_path();
    if !found {
        log.info(format_args!(
            "[SNAPSHOT] Warning: UFS not found, using fallback {}\n",
            ufs_path.display()
        ));
    }
    dump_dir_contents(log, &ufs_path);
    log.info(format_args!("----------------------------------\n\n"));
}

// ---------------------------------------------------------------------------
// vendor_boot snapshot (sysfs PARTNAME scan + boot slot detection)
// ---------------------------------------------------------------------------

fn get_or_create_block_device(target_partname: &str) -> Option<PathBuf> {
    let entries = fs::read_dir("/sys/class/block").ok()?;
    for ent in entries.flatten() {
        let name = ent.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let uevent = fs::read_to_string(format!("/sys/class/block/{name}/uevent")).unwrap_or_default();
        let matched = uevent.lines().any(|line| {
            line.strip_prefix("PARTNAME=")
                .map(|pname| pname == target_partname)
                .unwrap_or(false)
        });
        if !matched {
            continue;
        }
        let dev_path = PathBuf::from(format!("/dev/block/{name}"));
        if dev_path.exists() {
            return Some(dev_path);
        }
        // Node missing: recreate via mknod from sysfs dev number.
        if let Ok(dev) = fs::read_to_string(format!("/sys/class/block/{name}/dev")) {
            let mut it = dev.trim().split(':');
            if let (Some(maj), Some(min)) =
                (it.next().and_then(|s| s.parse::<u32>().ok()), it.next().and_then(|s| s.parse::<u32>().ok()))
            {
                let _ = fs::create_dir_all("/dev/block");
                // Safety: `c` is a valid path CString; maj/min come from
                // sysfs dev number parsing above; mknod failure is handled.
                unsafe {
                    if let Some(c) = c_path(&dev_path) {
                        // mode must carry the file type for mknod.
                        if libc::mknod(c.as_ptr(), libc::S_IFBLK | 0o600, libc::makedev(maj, min)) == 0 {
                            eprintln!("[SNAPSHOT] node {} was missing, recreated via mknod {maj}:{min}", dev_path.display());
                            return Some(dev_path);
                        }
                    }
                }
            }
        }
        return None;
    }
    None
}

fn boot_slot_from_text(text: &str, key: &str) -> Option<String> {
    // Handles both `androidboot.slot_suffix="_a"` (bootconfig, spaces and
    // quotes) and `androidboot.slot_suffix=_a` (cmdline) spellings.
    let pos = text.find(key)?;
    let val = text[pos + key.len()..].trim_start_matches(['=', ' ', '"']);
    let end = val.find(['"', '\n', ' ']).unwrap_or(val.len());
    let slot = &val[..end];
    if slot.is_empty() {
        None
    } else {
        Some(slot.to_string())
    }
}

fn get_boot_slot() -> String {
    if let Ok(text) = fs::read_to_string("/proc/bootconfig") {
        if let Some(slot) = boot_slot_from_text(&text, "androidboot.slot_suffix") {
            return slot;
        }
    }
    if let Ok(text) = fs::read_to_string("/proc/cmdline") {
        if let Some(slot) = boot_slot_from_text(&text, "androidboot.slot_suffix") {
            return slot;
        }
    }
    String::new()
}

fn snapshot_vendor_boot(log: &mut Logger) {
    log.info(format_args!("[SNAPSHOT] Backing up vendor_boot...\n"));
    mkdirs(Path::new(VENDOR_BOOT_SNAP), 0o755);

    let mut boot_slot = get_boot_slot();
    if boot_slot.is_empty() {
        log.info(format_args!("[SNAPSHOT] No slot suffix found; guessing from /dev/block/by-name/boot_*\n"));
        if Path::new("/dev/block/by-name/boot_b").exists() {
            boot_slot = "_b".to_string();
        } else if Path::new("/dev/block/by-name/boot_a").exists() {
            boot_slot = "_a".to_string();
        }
    }

    let slots: Vec<String> = if boot_slot.is_empty() {
        log.info(format_args!("[SNAPSHOT] Slot still unknown; dumping both _a and _b\n"));
        vec!["_a".to_string(), "_b".to_string()]
    } else {
        vec![boot_slot]
    };

    for slot in slots.iter() {
        let partname = format!("vendor_boot{slot}");
        log.info(format_args!("[SNAPSHOT] Looking for {partname} via sysfs...\n"));
        match get_or_create_block_device(&partname) {
            Some(dev_path) => {
                let dst = if slots.len() == 1 {
                    PathBuf::from(format!("{VENDOR_BOOT_SNAP}/vendor_boot.img"))
                } else {
                    PathBuf::from(format!("{VENDOR_BOOT_SNAP}/vendor_boot{slot}.img"))
                };
                log.info(format_args!("[SNAPSHOT] Copying {} -> {}\n", dev_path.display(), dst.display()));
                if copy_file(log, &dev_path, &dst) {
                    log.info(format_args!("[SNAPSHOT] OK: {partname} snapshotted\n"));
                } else {
                    log.err(format_args!("[SNAPSHOT] FAILED to copy {}\n", dev_path.display()));
                }
            }
            None => log.err(format_args!("[SNAPSHOT] partition {partname} not found in sysfs\n")),
        }
    }
}

// ---------------------------------------------------------------------------
// manifest pass
// ---------------------------------------------------------------------------

struct ManifestEntry {
    is_link: bool,
    is_dir: bool,
    mode: u32,
    uid: u32,
    gid: u32,
    path: PathBuf,
    link_target: Option<PathBuf>,
}

fn parse_manifest_line(line: &str) -> Option<ManifestEntry> {
    // Symlink form: `l <perms> <uid> <gid> <path> -> <target>`
    if let Some(arrow) = line.find(" -> ") {
        let (head, target) = (&line[..arrow], &line[arrow + 4..]);
        let mut parts = head.split_whitespace();
        let (typ, perms, uid, gid, path) = (
            parts.next()?,
            parts.next()?,
            parts.next()?,
            parts.next()?,
            parts.next()?,
        );
        if typ != "l" || parts.next().is_some() {
            return None;
        }
        return Some(ManifestEntry {
            is_link: true,
            is_dir: false,
            mode: u32::from_str_radix(perms, 8).ok()?,
            uid: uid.parse().ok()?,
            gid: gid.parse().ok()?,
            path: PathBuf::from(path),
            link_target: Some(PathBuf::from(target)),
        });
    }
    let mut parts = line.split_whitespace();
    let (typ, perms, uid, gid, path) = (
        parts.next()?,
        parts.next()?,
        parts.next()?,
        parts.next()?,
        parts.next()?,
    );
    if parts.next().is_some() || (typ != "d" && typ != "f") {
        return None;
    }
    Some(ManifestEntry {
        is_link: false,
        is_dir: typ == "d",
        mode: u32::from_str_radix(perms, 8).ok()?,
        uid: uid.parse().ok()?,
        gid: gid.parse().ok()?,
        path: PathBuf::from(path),
        link_target: None,
    })
}

fn snapshot_manifest(log: &mut Logger, manifest_path: &str, snap_dir: &Path) -> (u32, u32) {
    let text = match fs::read_to_string(manifest_path) {
        Ok(t) => t,
        Err(e) => {
            log.err(format_args!("[SNAPSHOT] Cannot open manifest {manifest_path}: {e}\n"));
            return (0, 1);
        }
    };

    log.info(format_args!("[SNAPSHOT] Creating ramdisk snapshot in {}\n", snap_dir.display()));
    mkdirs(snap_dir, 0o755);

    let mut ok_count = 0u32;
    let mut fail_count = 0u32;

    for raw in text.lines() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let entry = match parse_manifest_line(line) {
            Some(e) => e,
            None => {
                fail_count += 1;
                continue;
            }
        };
        // Manifest paths are absolute in the live root ("/system/bin/...").
        let src = PathBuf::from(format!("/{}", entry.path.to_string_lossy().trim_start_matches('/')));
        let rel = entry.path.to_string_lossy().trim_start_matches('/').to_string();
        let dst = snap_dir.join(&rel);

        if entry.is_link {
            let target = entry.link_target.unwrap_or_default();
            ensure_parent(&dst);
            let _ = fs::remove_file(&dst);
            match symlink(&target, &dst) {
                Ok(()) => {
                    apply_metadata(log, &src, &dst, entry.mode, entry.uid, entry.gid, true);
                    ok_count += 1;
                }
                Err(e) => {
                    log.err(format_args!(
                        "[SNAPSHOT] symlink {} -> {}: {e}\n",
                        dst.display(),
                        target.display()
                    ));
                    fail_count += 1;
                }
            }
            continue;
        }

        if rel.is_empty() || rel == "/" {
            // The manifest's own root entry (listed as "/"): ensure dir.
            mkdirs(&dst, entry.mode);
            ok_count += 1;
            continue;
        }

        if entry.is_dir {
            mkdirs(&dst, entry.mode);
            apply_metadata(log, &src, &dst, entry.mode, entry.uid, entry.gid, false);
            ok_count += 1;
        } else {
            ensure_parent(&dst);
            if copy_file(log, &src, &dst) {
                apply_metadata(log, &src, &dst, entry.mode, entry.uid, entry.gid, false);
                ok_count += 1;
            } else {
                fail_count += 1;
            }
        }
    }

    log.info(format_args!("[SNAPSHOT] Ramdisk Complete: {ok_count} entries copied, {fail_count} errors\n"));
    (ok_count, fail_count)
}

// ---------------------------------------------------------------------------

fn main() {
    let mut log = Logger::new();
    dump_debug_info(&mut log);

    let args: Vec<String> = std::env::args().collect();
    let manifest_path = args.get(1).map(String::as_str).unwrap_or(DEFAULT_MANIFEST);
    let snap_dir = args.get(2).map(String::as_str).unwrap_or(DEFAULT_SNAP_DIR);

    let (_ok, fail) = snapshot_manifest(&mut log, manifest_path, Path::new(snap_dir));
    snapshot_vendor_boot(&mut log);

    std::process::exit(if fail > 0 { 1 } else { 0 });
}
