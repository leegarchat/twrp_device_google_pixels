/* recovery-init-stub — static PID 1 entry point for OrangeFox recovery.
 *
 * Occupies /system/bin/init. The real AOSP init lives at
 * /system/bin/fox.init INSIDE /lgz_cluster.lgz. The fox.init name (not
 * init.real) avoids collisions with Magisk/KSU chains, which use
 * init.real for the stock init backup.
 *
 * Magisk hexpatch canary (gs101 normal boot): magiskinit's
 * hexpatch_init_for_second_stage() rewrites EVERY 16-byte occurrence of
 * "/system/bin/init" in the /init file (which resolves to this stub) to
 * "/data/magiskinit", so stock first-stage init chain-loads magiskinit
 * as second stage. kCanaryInit below is the INTENTIONAL single such
 * site in this binary (see build-time assert in fox_build_callback.sh):
 * at runtime the stub compares it against a piecewise-built reference
 * (short literals only — the 16B pattern can never occur in the check
 * itself, and -Os const-folding cannot materialize it); a mismatch means
 * magisk hexpatched us, and the stub applies the same 16->16 patch to
 * the fox.init FILE (stock init needs it for the identical reason)
 * using the detected value, then execs it. Pristine canary (no magisk,
 * KSU, etc.) = zero behavior change. See fox_maybe_repatch().
 *
 * Fallback chain (first match wins, never loops, never hangs):
 *  1. unpack markers (/lgz_complite or /system/etc/lgz_complite) -> exec real;
 *  2. /system/bin/fox.init present (markers lost) -> exec real;
 *  3. /lgz_cluster.lgz present -> snapshot, unpack, swap, mark, exec real;
 *  4. nothing of the above -> hand off to the kernel entry /init (possibly
 *     a Magisk/KSU hook); if /init resolves back to this stub -> reboot to
 *     the bootloader.
 *
 * Plain C, libc calls only, fully static (see Android.bp): must run before
 * the cluster is unpacked, so it cannot depend on any shared library.
 * Near-silent by design: only magisk-detection lines go to the stub log,
 * any bootstrap failure reboots to the bootloader.
 */

#include <errno.h>
#include <fcntl.h>
#include <linux/reboot.h>
#include <stdlib.h>
#include <string.h>
#include <sys/reboot.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

#include "snapshot.h"
#include "aioswap.h"

static const char kFoxInit[] = "/system/bin/fox.init";
/* Magisk hexpatch canary: the INTENTIONAL single 16-byte "/system/bin/init"
 * site in this binary (build assert enforces the count). magiskinit's
 * hexpatch rewrites it to the second-stage path ("/data/magiskinit");
 * detection below reads it back. Do NOT "clean up" this literal and do
 * not let the full 16B pattern appear anywhere else in this TU. */
static const char kCanaryInit[] = "/system/bin/init";
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

/* Best-effort append of one NUL-terminated line to the stub logs
 * (same files aioswap uses; flog.sh collects them). */
static void fox_log(const char* s) {
    static const char* const paths[] = { "/tmp/aio_stub.log", "/aio_stub.log", NULL };
    int i;
    size_t n = strlen(s);
    for (i = 0; paths[i] != NULL; i++) {
        int fd = open(paths[i], O_WRONLY | O_CREAT | O_APPEND | O_CLOEXEC, 0644);
        size_t left;
        const char* p;
        if (fd < 0) continue;
        left = n;
        p = s;
        while (left > 0) {
            ssize_t w = write(fd, p, left);
            if (w < 0) {
                if (errno == EINTR) continue;
                break;
            }
            p += w;
            left -= (size_t)w;
        }
        close(fd);
    }
}

/* Copy the live canary bytes (17 incl NUL) to out. Volatile reads: the
 * compiler must emit real loads and cannot fold this against the
 * initializer (magisk hexpatch rewrites the file image at flash time,
 * invisible to the build). */
static void canary_value(char out[17]) {
    const volatile char* c = kCanaryInit;
    unsigned i;
    for (i = 0; i < 17; i++) out[i] = c[i];
}

/* 1 iff v differs from the stock second-stage path. Fold-proof by
 * construction: only <16B literals are used, so the 16B magisk pattern
 * can never occur in this function. v must hold 17 bytes. Pure and
 * host-testable. */
static int fox_canary_changed(const char* v) {
    if (!v) return 0;
    if (memcmp(v, "/system", 7) != 0) return 1;
    if (memcmp(v + 7, "/bin", 4) != 0) return 1;
    if (memcmp(v + 11, "/init", 6) != 0) return 1;
    return 0;
}

/* Replace ALL 16B occurrences of the stock second-stage path with newpath
 * inside the file at path (in place, same length — mirrors magisk's
 * hexpatch_init_for_second_stage). newpath must hold 16 chars.
 * Returns patched count (0 = stock pattern absent: idempotent no-op for
 * re-entry and already-patched files), -1 on IO error. The 16B search
 * pattern is assembled at runtime from short parts, so it can never
 * occur in this binary. Pure file logic, host-testable. */
static int fox_patch_file(const char* path, const char newpath[16]) {
    char from[16];
    struct stat st;
    unsigned char* buf;
    int fd;
    size_t i, count = 0, got = 0;
    memcpy(from, "/system", 7);
    memcpy(from + 7, "/bin", 4);
    memcpy(from + 11, "/init", 5);
    if (stat(path, &st) != 0) return -1;
    if (st.st_size < 16 || st.st_size > (off_t)(32 * 1024 * 1024)) return -1;
    buf = (unsigned char*)malloc((size_t)st.st_size);
    if (!buf) return -1;
    fd = open(path, O_RDONLY | O_CLOEXEC);
    if (fd < 0) { free(buf); return -1; }
    while (got < (size_t)st.st_size) {
        ssize_t r = read(fd, buf + got, (size_t)st.st_size - got);
        if (r < 0) {
            if (errno == EINTR) continue;
            break;
        }
        if (r == 0) break;
        got += (size_t)r;
    }
    close(fd);
    if (got != (size_t)st.st_size) { free(buf); return -1; }
    for (i = 0; i + 16 <= (size_t)st.st_size; i++) {
        if (memcmp(buf + i, from, 16) == 0) {
            memcpy(buf + i, newpath, 16);
            count++;
        }
    }
    if (count == 0) { free(buf); return 0; }
    fd = open(path, O_WRONLY | O_TRUNC | O_CLOEXEC);
    if (fd < 0) { free(buf); return -1; }
    {
        size_t left = (size_t)st.st_size;
        unsigned char* p = buf;
        while (left > 0) {
            ssize_t w = write(fd, p, left);
            if (w < 0) {
                if (errno == EINTR) continue;
                break;
            }
            p += (size_t)w;
            left -= (size_t)w;
        }
        if (left != 0) { close(fd); free(buf); return -1; }
    }
    close(fd);
    free(buf);
    return (int)count;
}

/* Magisk-compat re-patch, called from exec_real() on every branch: if the
 * canary was rewritten (magisk hexpatched this stub), apply the same
 * 16->16 substitution to the fox.init FILE so stock first-stage init
 * chain-loads magiskinit as second stage exactly like on stock+magisk.
 * Idempotent (second run finds no stock pattern). Fail-safe: any doubt
 * (non-path value, missing file) leaves fox.init pristine -> plain
 * unrooted boot instead of a brick. Best-effort, never fatal. */
static void fox_maybe_repatch(void) {
    char cur[17];
    char newpath[16];
    char line[48];
    int n;
    canary_value(cur);
    if (!fox_canary_changed(cur)) return;
    memcpy(line, "fox: canary=", 13);
    memcpy(line + 13, cur, 16);
    line[29] = '\n';
    line[30] = '\0';
    fox_log(line);
    if (cur[0] != '/') {
        fox_log("fox: canary not a path, skipping re-patch\n");
        return;
    }
    memcpy(newpath, cur, 16);
    n = fox_patch_file(kFoxInit, newpath);
    if (n > 0) {
        fox_log("fox: fox.init re-patched for magisk compat\n");
    } else if (n == 0) {
        fox_log("fox: fox.init already patched, skipping\n");
    } else {
        fox_log("fox: fox.init re-patch IO failed, pristine boot\n");
    }
}

static void exec_real(int argc, char** argv, char** envp) {
    (void)argc;
    fox_maybe_repatch();
    execve(kFoxInit, argv, envp);
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
 * The "/system/bin/init" comparison is piecewise (short literals only): a
 * whole 16B literal here would be rewritten by the magisk hexpatch, silently
 * inverting this guard exactly when it matters. readlink needs
 * <unistd.h>; memcmp needs <string.h> (both included). */
static int path_is_stock_init(const char* p) {
    if (!p) return 0;
    if (memcmp(p, "/system", 7) != 0) return 0;
    if (memcmp(p + 7, "/bin", 4) != 0) return 0;
    if (memcmp(p + 11, "/init", 5) != 0) return 0;
    return p[16] == '\0';
}

static int root_init_is_self(void) {
    char buf[256];
    ssize_t n = readlink(kInitRoot, buf, sizeof(buf) - 1);
    if (n < 0) return 0;
    buf[n] = '\0';
    return strcmp(buf, "system/bin/init") == 0 || path_is_stock_init(buf);
}

int main(int argc, char** argv, char** envp) {
    /* 1. Fast path: unpacked tree. Either marker suffices. */
    if (path_exists(kMarkerRoot) || path_exists(kMarkerEtc))
        exec_real(argc, argv, envp);
    /* 2. Real binary present without markers (markers lost, unpack done). */
    if (path_exists(kFoxInit))
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
    if (path_exists(kFoxInit))
        exec_real(argc, argv, envp);

    /* 5. Last resort: no markers, no fox.init, no (usable) cluster.
     * Hand off to the kernel entry /init (possibly a Magisk/KSU hook);
     * if it resolves back to us there is nothing left to try. */
    if (!root_init_is_self()) {
        execve(kInitRoot, argv, envp);
    }
    reboot_bootloader();
    return 127;
}
