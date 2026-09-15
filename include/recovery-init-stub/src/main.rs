//! recovery-init-stub — static PID 1 entry point for OrangeFox recovery.
//!
//! Occupies `/system/bin/init`. The real AOSP init lives at
//! `/system/bin/init.real` INSIDE `/lgz_cluster.lgz`.
//!
//! - First invocation (`selinux_setup`, `init.real` absent): snapshot the
//!   ramdisk, unpack the cluster, verify, then exec the real init.
//! - Later invocations (`second_stage`, `init.real` present): exec through
//!   immediately (passthrough, no work).
//!
//! Deps: libc only. No threads, no signal handlers, straight-line code.
//! All logging goes to /dev/kmsg (stdout may not exist yet).

use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;

const INIT_REAL: &[u8] = b"/system/bin/init.real\0";
const CLUSTER: &[u8] = b"/lgz_cluster.lgz\0";
const RECOVERY: &[u8] = b"/system/bin/recovery\0";
const SNAPSHOT_BIN: &[u8] = b"/system/bin/ramdisk_snapshot\0";
const SNAPSHOT_MANIFEST: &[u8] = b"/ramdisk_snapshot_manifest.txt\0";
const SNAPSHOT_DIR: &[u8] = b"/dev/ramdisk_snapshot\0";
const LGZ_BIN: &[u8] = b"/system/bin/lgz\0";
const KMSG: &[u8] = b"/dev/kmsg\0";
// DEBUG post-mortem channel: raw klog partition (16MB, survives reboot,
// readable from any later booted image via dd). Best-effort only.
const KLOG: &[u8] = b"/dev/block/by-name/klog\0";
// Fallback node when ueventd has not created by-name yet (recovery
// first-stage skips DoCreateDevices, so at selinux_setup there are no
// /dev/block symlinks). DEBUG shiba-only: klog == sda2 == 8:2.
const KLOG_TMP: &[u8] = b"/dev/.klogblk\0";
// Record offset: 8MB — head of klog belongs to the bootloader's own logs
// (offset 0 was observed clobbered), 8MB reads back as zeros on shiba.
const KLOG_OFFSET: i64 = 8 * 1024 * 1024;

// Stage codes for the klog marker (last record wins — it tells us exactly
// where the previous boot died).
const STAGE_ENTER: u32 = 1;
const STAGE_PASSTHROUGH: u32 = 2;
const STAGE_SNAP_DONE: u32 = 3;
const STAGE_UNPACK_DONE: u32 = 4;
const STAGE_EXEC: u32 = 5;
const STAGE_EXEC_FAIL: u32 = 6;
const STAGE_REBOOT: u32 = 7;

// reboot(2) number on aarch64 (asm-generic unistd.h __NR_reboot). Kept as a
// const so the stub never depends on libc's SYS_* coverage.
const SYS_REBOOT_NR: libc::c_long = 142;

struct Kmsg {
    fd: libc::c_int,
}

impl Kmsg {
    fn open() -> Kmsg {
        // SAFETY: nul-terminated const path, valid flags.
        let fd = unsafe {
            libc::open(
                KMSG.as_ptr() as *const libc::c_char,
                libc::O_WRONLY | libc::O_CLOEXEC,
            )
        };
        Kmsg { fd }
    }

    fn log(&self, msg: &str) {
        if self.fd < 0 {
            return;
        }
        let full = format!("[init-stub] {msg}\n");
        let bytes = full.as_bytes();
        let mut off = 0;
        while off < bytes.len() {
            // SAFETY: pointer stays inside `bytes` for the computed length.
            let n = unsafe {
                libc::write(
                    self.fd,
                    bytes[off..].as_ptr() as *const libc::c_void,
                    (bytes.len() - off) as libc::size_t,
                )
            };
            if n <= 0 {
                break;
            }
            off += n as usize;
        }
    }
}

fn exists(path: &[u8]) -> bool {
    // SAFETY: caller passes nul-terminated const paths only.
    unsafe { libc::access(path.as_ptr() as *const libc::c_char, libc::F_OK) == 0 }
}

/// Best-effort post-mortem marker: overwrite offset 0 of the klog partition
/// with [magic u32][stage u32][status i32][errno i32]. Survives reboot into
/// any image; read back with `dd if=/dev/block/by-name/klog bs=16 count=1`.
/// Silently skipped if the node is not up yet (ueventd ordering).
fn klog_mark(log: &Kmsg, stage: u32, status: i32) {
    // SAFETY: nul-terminated const paths; fixed 16-byte stack buffer, fully
    // initialized before the write; fd success checked, lseek + fsync
    // verified, temp node unlinked after close. mknod numbers are
    // shiba-debug-only (klog == sda2 == major 8 minor 2).
    unsafe {
        let mut fd = libc::open(
            KLOG.as_ptr() as *const libc::c_char,
            libc::O_WRONLY | libc::O_CLOEXEC,
        );
        if fd < 0 {
            log.log("klog by-name absent, mknod sda2 fallback");
            // SAFETY: fixed tmp path; S_IFBLK|0600; shiba-debug-only numbers.
            let _ = libc::mknod(
                KLOG_TMP.as_ptr() as *const libc::c_char,
                libc::S_IFBLK | 0o600,
                libc::makedev(8, 2),
            );
            fd = libc::open(
                KLOG_TMP.as_ptr() as *const libc::c_char,
                libc::O_WRONLY | libc::O_CLOEXEC,
            );
        }
        if fd < 0 {
            log.log("klog marker skipped: no block node available");
            return;
        }
        // From here on errno is meaningless (all calls succeeded so far);
        // the record carries zeros in the reserved field by construction.
        let mut rec = [0u8; 16];
        rec[0..4].copy_from_slice(&0x54535542u32.to_le_bytes()); // "BUST" LE
        rec[4..8].copy_from_slice(&stage.to_le_bytes());
        rec[8..12].copy_from_slice(&status.to_le_bytes());
        // bytes 12..16 reserved (zero): errno at this point is always stale.
        if libc::lseek(fd, KLOG_OFFSET as libc::off_t, libc::SEEK_SET) < 0 {
            log.log("klog marker skipped: lseek failed");
            libc::close(fd);
            return;
        }
        let mut off = 0;
        while off < rec.len() {
            let n = libc::write(
                fd,
                rec[off..].as_ptr() as *const libc::c_void,
                (rec.len() - off) as libc::size_t,
            );
            if n <= 0 {
                break;
            }
            off += n as usize;
        }
        libc::fsync(fd);
        libc::close(fd);
        libc::unlink(KLOG_TMP.as_ptr() as *const libc::c_char);
    }
}

/// fork + execv + synchronous waitpid. Returns the child exit code, or -1 on
/// fork/wait failure or signal death. argv entries must be nul-terminated.
fn run(prog: &[u8], argv: &[&[u8]], log: &Kmsg) -> i32 {
    // SAFETY: fork/exec/waitpid used in the standard synchronous pattern;
    // the stub is single-threaded, so fork is async-signal-safe here.
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        log.log("fork failed");
        return -1;
    }
    if pid == 0 {
        let argv_ptrs: Vec<*const libc::c_char> = argv
            .iter()
            .map(|a| a.as_ptr() as *const libc::c_char)
            .chain(std::iter::once(std::ptr::null()))
            .collect();
        // SAFETY: single-threaded fork child; argv built from nul-terminated
        // consts; _exit(127) on execv failure runs no destructors.
        unsafe {
            libc::execv(
                prog.as_ptr() as *const libc::c_char,
                argv_ptrs.as_ptr(),
            );
            libc::_exit(127);
        }
    }
    let mut status = 0;
    loop {
        // SAFETY: status is a valid out-param; EINTR retries are explicit.
        let r = unsafe { libc::waitpid(pid, &mut status, 0) };
        if r == pid {
            break;
        }
        if r < 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
            log.log("waitpid failed");
            return -1;
        }
    }
    if libc::WIFEXITED(status) {
        libc::WEXITSTATUS(status)
    } else {
        -1
    }
}

fn reboot_bootloader(log: &Kmsg) -> ! {
    log.log("bootstrap failed, rebooting to bootloader");
    klog_mark(log, STAGE_REBOOT, 0);
    // SAFETY: raw reboot(2) with UAPI magic constants; property service is
    // not up yet, so this is the only reboot path available. sync() flushes
    // kmsg first; if the syscall ever returns, PID 1 parks in sleep.
    unsafe {
        libc::sync();
        // Soong toolchain predates c"" literals: keep the byte-string form.
        #[allow(clippy::manual_c_str_literals)]
        let arg = b"bootloader\0".as_ptr() as usize;
        libc::syscall(
            SYS_REBOOT_NR,
            libc::LINUX_REBOOT_MAGIC1 as libc::c_long,
            libc::LINUX_REBOOT_MAGIC2 as libc::c_long,
            libc::LINUX_REBOOT_CMD_RESTART2 as libc::c_long,
            arg,
        );
        // If the syscall ever returns, park PID 1 instead of falling off.
        loop {
            libc::sleep(60);
        }
    }
}

fn exec_real(args: &[CString], log: &Kmsg) -> ! {
    let argv: Vec<*const libc::c_char> = args
        .iter()
        .map(|a| a.as_ptr())
        .chain(std::iter::once(std::ptr::null()))
        .collect();
    klog_mark(log, STAGE_EXEC, 0);
    // SAFETY: INIT_REAL is nul-terminated; argv mirrors our own argv
    // (argv[0] preserved, so AOSP init sees its usual context).
    unsafe {
        libc::execv(
            INIT_REAL.as_ptr() as *const libc::c_char,
            argv.as_ptr(),
        );
    }
    // execv returns only on error (ENOENT etc.) — nothing sane left to do.
    // Capture errno before any other libc call (log.log would clobber it).
    let exec_errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(-1);
    log.log("FATAL: exec init.real failed");
    klog_mark(log, STAGE_EXEC_FAIL, exec_errno);
    reboot_bootloader(log);
}

fn main() {
    let log = Kmsg::open();
    klog_mark(&log, STAGE_ENTER, 0);

    // Preserve our argv verbatim for the real init (argv[0] included).
    let mut args: Vec<CString> = std::env::args_os()
        .map(|a| CString::new(a.as_bytes()).unwrap_or_else(|_| CString::new("init").unwrap()))
        .collect();
    if args.is_empty() {
        args.push(CString::new("init").unwrap());
    }

    // Cluster already unpacked (second and later invocations): passthrough.
    if exists(INIT_REAL) {
        klog_mark(&log, STAGE_PASSTHROUGH, 0);
        exec_real(&args, &log);
    }

    // First invocation: unpack path. Mirrors the IsRecoveryMode gate
    // (system/core/init/util.cpp): recovery binary OR cluster present.
    let recovery_mode = exists(RECOVERY) || exists(CLUSTER);
    if exists(CLUSTER) {
        if exists(SNAPSHOT_MANIFEST) {
            log.log("snapshot: running ramdisk_snapshot (pre-unpack state)");
            // Mirrors init.cpp: snapshot failure is a warning, never fatal.
            let snap_st = run(
                SNAPSHOT_BIN,
                &[b"ramdisk_snapshot\0", SNAPSHOT_MANIFEST, SNAPSHOT_DIR],
                &log,
            );
            klog_mark(&log, STAGE_SNAP_DONE, snap_st);
            if snap_st != 0 {
                log.log("snapshot failed, continuing anyway");
            }
        }
        log.log("unpack: lgz decompress /lgz_cluster.lgz /");
        let st = run(
            LGZ_BIN,
            &[b"lgz\0", b"decompress\0", CLUSTER, b"/\0"],
            &log,
        );
        klog_mark(&log, STAGE_UNPACK_DONE, st);
        if st != 0 {
            log.log("unpack failed");
            if recovery_mode {
                reboot_bootloader(&log);
            }
            log.log("unpack failed outside recovery mode, continuing anyway");
        } else {
            log.log("unpack OK");
        }
    }

    exec_real(&args, &log);
}
