/* recovery-init-stub — static PID 1 entry point for OrangeFox recovery.
 *
 * Occupies /system/bin/init. The real AOSP init lives at
 * /system/bin/init.fox_real INSIDE /lgz_cluster.lgz. The .fox_real name
 * (not init.real) avoids collisions with Magisk/KSU chains, which use
 * init.real for the stock init backup.
 *
 * Fallback chain (first match wins, never loops, never hangs):
 *  1. unpack markers (/lgz_complite or /system/etc/lgz_complite) -> exec real;
 *  2. /system/bin/init.fox_real present (markers lost) -> exec real;
 *  3. /lgz_cluster.lgz present -> snapshot, unpack, swap, mark, exec real;
 *  4. nothing of the above -> hand off to the kernel entry /init (possibly
 *     a Magisk/KSU hook); if /init resolves back to this stub -> reboot to
 *     the bootloader.
 *
 * Plain C, libc calls only, fully static (see Android.bp): must run before
 * the cluster is unpacked, so it cannot depend on any shared library.
 * Silent by design (no logging): any bootstrap failure reboots to the
 * bootloader.
 */

#include <errno.h>
#include <fcntl.h>
#include <linux/reboot.h>
#include <string.h>
#include <sys/reboot.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

#include "snapshot.h"
#include "aioswap.h"

static const char kInitReal[] = "/system/bin/init.fox_real";
/* Unpack-completion markers (either one suffices): the tree is final,
 * skip straight to exec. Two locations for redundancy — /system may be
 * over-mounted later in the boot, the root copy always stays visible. */
static const char kMarkerRoot[] = "/lgz_complite";
static const char kMarkerEtc[] = "/system/etc/lgz_complite";
static const char kCluster[] = "/lgz_cluster.lgz";
/* Kernel entry point. May be a hook binary (Magisk/KSU) instead of the
 * usual symlink back to /system/bin/init. */
static const char kInitRoot[] = "/init";
static const char kRecovery[] = "/system/bin/recovery";
static const char kSnapshotBin[] = "/system/bin/ramdisk_snapshot";
static const char kSnapshotManifest[] = "/ramdisk_snapshot_manifest.txt";
static const char kSnapshotDir[] = "/dev/ramdisk_snapshot";
static const char kLgzBin[] = "/system/bin/lgz";

static int path_exists(const char* path) {
    return access(path, F_OK) == 0;
}

/* fork + execve + synchronous wait. Returns the child exit code, or -1. */
static int run_child(const char* prog, char* const argv[]) {
    pid_t pid = fork();
    if (pid < 0) return -1;
    if (pid == 0) {
        extern char** environ;
        execve(prog, argv, environ);
        _exit(127);
    }
    int status = 0;
    while (waitpid(pid, &status, 0) < 0) {
        if (errno != EINTR) return -1;
    }
    if (WIFEXITED(status)) return WEXITSTATUS(status);
    return -1;
}

static void reboot_bootloader(void) {
    sync();
    /* bionic reboot() takes no arg; raw reboot(2) carries "bootloader". */
    syscall(__NR_reboot, LINUX_REBOOT_MAGIC1, LINUX_REBOOT_MAGIC2,
            LINUX_REBOOT_CMD_RESTART2, "bootloader");
    for (;;) sleep(60);
}

static void exec_real(int argc, char** argv, char** envp) {
    (void)argc;
    execve(kInitReal, argv, envp);
    /* execve returns only on error (ENOENT etc.): nothing sane left to do. */
    reboot_bootloader();
}

/* Best-effort creation of one marker file. Raw syscalls only. */
static void mark_one(const char* path) {
    static const char kMark[] = "lgz_cluster unpacked\n";
    int fd = open(path, O_WRONLY | O_CREAT | O_TRUNC | O_CLOEXEC, 0644);
    if (fd < 0) return;
    {
        size_t left = sizeof(kMark) - 1;
        const char* p = kMark;
        while (left > 0) {
            ssize_t w = write(fd, p, left);
            if (w < 0) {
                if (errno == EINTR) continue;
                break;
            }
            p += w;
            left -= (size_t)w;
        }
    }
    close(fd);
}

/* Drop both unpack-completion markers. Best-effort, never fatal:
 * a missing marker just means the next invocation retries the unpack. */
static void mark_unpacked(void) {
    /* /system/etc may not exist in minimal ramdisks; ensure it, ignore errors. */
    mkdir("/system/etc", 0755);
    mark_one(kMarkerRoot);
    mark_one(kMarkerEtc);
}

/* True when the kernel entry /init resolves back to this stub, i.e. handing
 * off to it would re-enter this same binary (infinite loop). A regular file
 * or a foreign symlink (Magisk/KSU hook) returns false: handing off is safe.
 * readlink needs <unistd.h>; strcmp needs <string.h> (both included). */
static int root_init_is_self(void) {
    char buf[256];
    ssize_t n = readlink(kInitRoot, buf, sizeof(buf) - 1);
    if (n < 0) return 0;
    buf[n] = '\0';
    return strcmp(buf, "system/bin/init") == 0 ||
           strcmp(buf, "/system/bin/init") == 0;
}

int main(int argc, char** argv, char** envp) {
    /* 1. Fast path: unpacked tree. Either marker suffices. */
    if (path_exists(kMarkerRoot) || path_exists(kMarkerEtc))
        exec_real(argc, argv, envp);
    /* 2. Real binary present without markers (markers lost, unpack done). */
    if (path_exists(kInitReal))
        exec_real(argc, argv, envp);

    /* 3. Slow path: unpack the cluster now. recovery_mode gates the
     * bootloader reboot on unpack failure (a missing cluster means a plain
     * system boot image, which must never be failed by us). */
    int recovery_mode = path_exists(kRecovery) || path_exists(kCluster);
    int unpacked = 0;
    if (path_exists(kCluster)) {
        if (path_exists(kSnapshotManifest)) {
            /* Built-in snapshot (snapshot.c). The Rust ramdisk_snapshot
             * binary stays on board as fallback for the legacy init.cpp
             * path and emergencies — unused on the normal stub branch. */
            int snap_st = snapshot_run(kSnapshotManifest, kSnapshotDir);
            if (snap_st != 0) {
                char* const snap_argv[] = {
                    (char*)"ramdisk_snapshot",
                    (char*)kSnapshotManifest,
                    (char*)kSnapshotDir,
                    NULL,
                };
                snap_st = run_child(kSnapshotBin, snap_argv);
            }
            (void)snap_st; /* snapshot is best-effort, never fatal */
        }
        char* const lgz_argv[] = {
            (char*)"lgz", (char*)"decompress", (char*)kCluster, (char*)"/", NULL,
        };
        if (run_child(kLgzBin, lgz_argv) != 0) {
            if (recovery_mode) reboot_bootloader();
        } else {
            unpacked = 1;
        }
    }

    /* AIO family swap (aioswap.c): the tree is fully unpacked here and the
     * real init has not parsed anything yet, so family files (fstab,
     * flags, wipe, USB controller in rc, KeyMint set, selector prop) can
     * still be swapped. Best-effort, never fatal. */
    aioswap_run();

    /* Markers stay truthful: only when the cluster was actually unpacked.
     * Without an unpack there is no final tree to fast-path to. */
    if (unpacked)
        mark_unpacked();

    /* 4. The unpack may have produced the real init: re-check before
     * falling through (covers markers-lost and just-unpacked alike). */
    if (path_exists(kInitReal))
        exec_real(argc, argv, envp);

    /* 5. Last resort: no markers, no init.fox_real, no (usable) cluster.
     * Hand off to the kernel entry /init (possibly a Magisk/KSU hook);
     * if it resolves back to us there is nothing left to try. */
    if (!root_init_is_self()) {
        execve(kInitRoot, argv, envp);
    }
    reboot_bootloader();
    return 127;
}
