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
 * Paranoid logging by design: every branch logs to kmsg ("fox-stub: "
 * tag, lands in pstore) plus the stub log files. Bootstrap failures
 * still reboot to the bootloader, but the reason stays visible.
 */

#include <errno.h>
#include <fcntl.h>
#include <linux/reboot.h>
#include <stdlib.h>
#include <string.h>
#include <sys/reboot.h>
#include <sys/select.h>
#include <sys/stat.h>
#include <sys/sysmacros.h>
#include <sys/syscall.h>
#include <sys/time.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

#include "snapshot.h"
#include "aioswap.h"

/* Forward: paranoid kmsg sink, defined below next to fox_log. */
static void fox_klog(const char* s);

/* Stub version tag. Bump on every behavior change so pstore always tells
 * exactly which stub ran — never trust build dates or hashes from memory. */
#define FOX_STUB_VERSION "1.25"

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
/* Snapshot lives on the initramfs ROOT, not /dev: first-stage init mounts
 * a fresh tmpfs on /dev, which hides everything the stub created there —
 * that is why reflash_twrp.sh never found /dev/ramdisk_snapshot on stub
 * boots (all AIO boots). The initramfs root is never over-mounted in
 * recovery, so /ramdisk_snapshot stays visible until reboot. */
static const char kSnapshotDir[] = "/ramdisk_snapshot";
static const char kLgzBin[] = "/system/bin/lgz";

static int path_exists(const char* path) {
    return access(path, F_OK) == 0;
}

/* fork + execve + synchronous wait. Returns the child exit code, or -1.
 * The child's stdout/stderr are captured through a pipe and mirrored to
 * kmsg (capped, so a chatty child cannot flood pstore). Drained while
 * waiting so a child writing more than the pipe buffer can never block. */
#define RUN_CHILD_LOG_CAP 8192
static int run_child(const char* prog, char* const argv[]) {
    int pipefd[2];
    pid_t pid;
    size_t logged = 0;
    int status = 0;
    int child_done = 0;
    int eof = 0;
    if (pipe(pipefd) != 0) {
        /* No pipe: fall back to silent spawn, behavior unchanged. */
        pid = fork();
        if (pid < 0) {
            fox_klog("run_child: fork failed\n");
            return -1;
        }
        if (pid == 0) {
            extern char** environ;
            execve(prog, argv, environ);
            fox_klog("run_child: execve failed\n");
            _exit(127);
        }
        while (waitpid(pid, &status, 0) < 0) {
            if (errno != EINTR) {
                fox_klog("run_child: waitpid failed\n");
                return -1;
            }
        }
        if (WIFEXITED(status)) return WEXITSTATUS(status);
        return -1;
    }
    pid = fork();
    if (pid < 0) {
        fox_klog("run_child: fork failed\n");
        close(pipefd[0]);
        close(pipefd[1]);
        return -1;
    }
    if (pid == 0) {
        extern char** environ;
        int nullfd = open("/dev/null", O_RDONLY);
        if (nullfd >= 0) {
            dup2(nullfd, STDIN_FILENO);
            if (nullfd > STDERR_FILENO) close(nullfd);
        }
        dup2(pipefd[1], STDOUT_FILENO);
        dup2(pipefd[1], STDERR_FILENO);
        if (pipefd[0] > STDERR_FILENO) close(pipefd[0]);
        if (pipefd[1] > STDERR_FILENO) close(pipefd[1]);
        execve(prog, argv, environ);
        _exit(127);
    }
    close(pipefd[1]);
    fcntl(pipefd[0], F_SETFL, O_NONBLOCK);
    while (!child_done || !eof) {
        char chunk[512];
        ssize_t r = read(pipefd[0], chunk, sizeof(chunk) - 1);
        if (r > 0) {
            /* Log whole chunk as one record (may contain newlines;
             * kernel splits records itself). Cap the total volume. */
            if (logged < RUN_CHILD_LOG_CAP) {
                size_t want = (size_t)r;
                if (logged + want > RUN_CHILD_LOG_CAP)
                    want = RUN_CHILD_LOG_CAP - logged;
                chunk[want] = '\0';
                fox_klog(chunk);
                logged += want;
            }
            continue;
        }
        if (r == 0) {
            eof = 1;
        }
        if (!child_done) {
            pid_t w = waitpid(pid, &status, WNOHANG);
            if (w < 0) {
                if (errno != EINTR) {
                    fox_klog("run_child: waitpid failed\n");
                    close(pipefd[0]);
                    return -1;
                }
            } else if (w == pid) {
                child_done = 1;
            }
        }
        if (!child_done || !eof) {
            /* Brief nap: 5ms via select timeout, no timers needed. */
            struct timeval tv;
            tv.tv_sec = 0;
            tv.tv_usec = 5000;
            select(0, NULL, NULL, NULL, &tv);
        }
    }
    close(pipefd[0]);
    if (WIFEXITED(status)) {
        if (WEXITSTATUS(status) == 0) {
            fox_klog("run_child: child ok\n");
        } else {
            fox_klog("run_child: child exit != 0\n");
        }
        return WEXITSTATUS(status);
    }
    if (WIFSIGNALED(status)) {
        /* Log the killing signal number (two digits max) so pstore shows
         * SIGSEGV vs SIGABRT vs SIGILL without a debugger on device. */
        char msg[] = "run_child: child killed by signal 00\n";
        int sig = WTERMSIG(status);
        msg[34] = (char)('0' + (sig / 10) % 10);
        msg[35] = (char)('0' + sig % 10);
        fox_klog(msg);
        return -1;
    }
    fox_klog("run_child: child did not exit\n");
    return -1;
}

static void reboot_bootloader(void) {
    sync();
    /* bionic reboot() takes no arg; raw reboot(2) carries "bootloader". */
    syscall(__NR_reboot, LINUX_REBOOT_MAGIC1, LINUX_REBOOT_MAGIC2,
            LINUX_REBOOT_CMD_RESTART2, "bootloader");
    for (;;) sleep(60);
}

static void fox_init_dev_and_stdio(void) {
    mkdir("/dev", 0755);
    mknod("/dev/kmsg", S_IFCHR | 0600, makedev(1, 11));
    mknod("/dev/null", S_IFCHR | 0666, makedev(1, 3));
    mknod("/dev/console", S_IFCHR | 0600, makedev(5, 1));
    {
        int fd = open("/dev/null", O_RDWR);
        if (fd >= 0) {
            if (fd != STDIN_FILENO) dup2(fd, STDIN_FILENO);
            if (fd != STDOUT_FILENO) dup2(fd, STDOUT_FILENO);
            if (fd != STDERR_FILENO) dup2(fd, STDERR_FILENO);
            if (fd > STDERR_FILENO) close(fd);
        }
    }
}

/* Paranoid kmsg sink: every stub decision lands in pstore via
 * /dev/kmsg, which survives reboot into console-ramoops. Best-effort:
 * if kmsg is unavailable the boot proceeds exactly as before.
 * Lines carry the "fox-stub: " tag for easy grepping. */
static void fox_klog(const char* s) {
    /* Single write: tag + message in one record. Two separate writes
     * make the kernel split them into two pstore lines ("fox-stub: "
     * alone, message on the next line). */
    static const char tag[] = "<6>fox-stub: ";
    char buf[256];
    size_t tlen = sizeof(tag) - 1;
    size_t slen = strlen(s);
    size_t total = tlen + slen;
    size_t left;
    const char* p;
    int fd;
    if (total >= sizeof(buf)) {
        slen = sizeof(buf) - 1 - tlen;
        total = sizeof(buf) - 1;
    }
    memcpy(buf, tag, tlen);
    memcpy(buf + tlen, s, slen);
    buf[total] = '\0';
    fd = open("/dev/kmsg", O_WRONLY | O_CLOEXEC);
    if (fd < 0) {
        /* The stub runs as PID 1 before /dev exists: create the kmsg
         * node ourselves on the writable initramfs root so the trail
         * still reaches pstore. Major 1, minor 11. */
        mkdir("/dev", 0755);
        mknod("/dev/kmsg", S_IFCHR | 0600, makedev(1, 11));
        fd = open("/dev/kmsg", O_WRONLY | O_CLOEXEC);
        if (fd < 0) return;
    }
    p = buf;
    left = total;
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

/* Best-effort append of one NUL-terminated line to the stub logs
 * (same files aioswap uses; flog.sh collects them). Mirrored to kmsg
 * so the stub trail is also visible in pstore. */
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
    fox_klog(s);
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
    fox_klog("exec_real: handing off to fox.init\n");
    fox_maybe_repatch();
    execve(kFoxInit, argv, envp);
    /* execve returns only on error (ENOENT etc.): nothing sane left to do. */
    fox_klog("exec_real: execve fox.init failed, rebooting\n");
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

/* file-like identity check of the lgz decoder before exec: size,
 * ELF magic, e_type/e_machine and a fast FNV-1a over the bytes.
 * Lets pstore prove WHICH binary ran instead of guessing versions. */
static void fox_check_lgz(void) {
    static const char hd[] = "0123456789abcdef";
    unsigned char eh[64];
    unsigned long sz = 0, h = 2166136261UL;
    struct stat st;
    int fd, i;
    char line[192], *pp = line;
    const char *pfx = "main: lgz file: ";
    const char *tp;
    if (stat(kLgzBin, &st) != 0) {
        fox_klog("main: lgz file: missing\n");
        return;
    }
    sz = (unsigned long)st.st_size;
    fd = open(kLgzBin, O_RDONLY | O_CLOEXEC);
    if (fd < 0) {
        fox_klog("main: lgz file: unopenable\n");
        return;
    }
    {
        ssize_t r = 0, got = 0;
        while (got < (ssize_t)sizeof(eh)) {
            r = read(fd, eh + got, sizeof(eh) - (size_t)got);
            if (r <= 0) break;
            got += r;
        }
        if (got != (ssize_t)sizeof(eh)) {
            fox_klog("main: lgz file: short header\n");
            close(fd);
            return;
        }
        for (i = 0; i < (int)sizeof(eh); i++) {
            h ^= eh[i];
            h *= 16777619UL;
        }
        for (;;) {
            unsigned char buf[65536];
            ssize_t n = read(fd, buf, sizeof(buf));
            int k;
            if (n <= 0) break;
            for (k = 0; k < n; k++) {
                h ^= buf[k];
                h *= 16777619UL;
            }
        }
    }
    close(fd);
    while (*pfx) *pp++ = *pfx++;
    if (sz == 0) {
        *pp++ = '0';
    } else {
        char rbuf[24];
        int rn = 0;
        unsigned long t = sz;
        while (t > 0 && rn < (int)sizeof(rbuf)) {
            rbuf[rn++] = (char)('0' + (t % 10));
            t /= 10;
        }
        while (rn > 0) *pp++ = rbuf[--rn];
    }
    tp = " magic=";
    while (*tp) *pp++ = *tp++;
    for (i = 0; i < 4; i++) {
        *pp++ = hd[(eh[i] >> 4) & 0xF];
        *pp++ = hd[eh[i] & 0xF];
    }
    {
        unsigned etype = (unsigned)eh[16] | ((unsigned)eh[17] << 8);
        unsigned emach = (unsigned)eh[18] | ((unsigned)eh[19] << 8);
        int sh;
        tp = " etype=";
        while (*tp) *pp++ = *tp++;
        *pp++ = hd[(etype >> 12) & 0xF];
        *pp++ = hd[(etype >> 8) & 0xF];
        *pp++ = hd[(etype >> 4) & 0xF];
        *pp++ = hd[etype & 0xF];
        tp = " emachine=";
        while (*tp) *pp++ = *tp++;
        *pp++ = hd[(emach >> 12) & 0xF];
        *pp++ = hd[(emach >> 8) & 0xF];
        *pp++ = hd[(emach >> 4) & 0xF];
        *pp++ = hd[emach & 0xF];
        tp = " fnv=";
        while (*tp) *pp++ = *tp++;
        for (sh = 28; sh >= 0; sh -= 4) *pp++ = hd[(h >> sh) & 0xF];
    }
    *pp++ = '\n';
    *pp = '\0';
    fox_klog(line);
}

int main(int argc, char** argv, char** envp) {
    fox_init_dev_and_stdio();
    fox_klog("main: stub " FOX_STUB_VERSION " start\n");
    /* Environment dump: argv, uid/gid, cwd and the environment block.
     * Everything the child will inherit, visible in pstore. Bounded:
     * at most 24 vars, values truncated, so a huge env cannot flood. */
    {
        char line[192], *pp;
        const char *pfx = "main: env argc=";
        int i, n;
        pp = line;
        while (*pfx) *pp++ = *pfx++;
        if (argc < 0) argc = 0;
        if (argc > 99) argc = 99;
        *pp++ = (char)('0' + (argc / 10));
        *pp++ = (char)('0' + (argc % 10));
        *pp++ = ' ';
        *pp++ = 'u';
        *pp++ = 'i';
        *pp++ = 'd';
        *pp++ = '=';
        {
            unsigned u = (unsigned)getuid();
            char rb[16];
            int rn = 0;
            if (u == 0) {
                *pp++ = '0';
            } else {
                while (u > 0 && rn < (int)sizeof(rb)) {
                    rb[rn++] = (char)('0' + (u % 10));
                    u /= 10;
                }
                while (rn > 0) *pp++ = rb[--rn];
            }
        }
        *pp++ = '\n';
        *pp = '\0';
        fox_klog(line);
        for (i = 0; i < argc && i < 8; i++) {
            const char *a = argv[i] ? argv[i] : "(null)";
            char al[160];
            char *q = al;
            const char *ap = "main: argv: ";
            int k = 0;
            while (*ap) *q++ = *ap++;
            while (a[k] != '\0' && k < 120) *q++ = a[k++];
            *q++ = '\n';
            *q = '\0';
            fox_klog(al);
        }
        {
            char cwd[160], *q;
            const char *cp = "main: cwd: ";
            q = cwd;
            while (*cp) *q++ = *cp++;
            if (getcwd(q, (size_t)(cwd + sizeof(cwd) - 2 - q)) == NULL) {
                const char *nc = "(unknown)";
                while (*nc) *q++ = *nc++;
            }
            *q++ = '\n';
            *q = '\0';
            fox_klog(cwd);
        }
        for (n = 0; envp && envp[n] && n < 24; n++) {
            const char *e = envp[n];
            char el[192], *q = el;
            const char *ep = "main: env: ";
            int k = 0;
            while (*ep) *q++ = *ep++;
            while (e[k] != '\0' && k < 150) *q++ = e[k++];
            *q++ = '\n';
            *q = '\0';
            fox_klog(el);
        }
        if (envp) {
            int total = 0;
            while (envp[total]) total++;
            if (total > n) fox_klog("main: env: (truncated)\n");
        } else {
            fox_klog("main: env: (none)\n");
        }
    }
    if (path_exists(kMarkerRoot) || path_exists(kMarkerEtc)) {
        fox_klog("main: unpack marker present, fast path\n");
        exec_real(argc, argv, envp);
    }
    fox_klog("main: no markers\n");
    /* 2. Real binary present without markers (markers lost, unpack done). */
    if (path_exists(kFoxInit)) {
        fox_klog("main: fox.init present without markers\n");
        exec_real(argc, argv, envp);
    }
    fox_klog("main: fox.init missing\n");

    /* 3. Slow path: unpack the cluster now. recovery_mode gates the
     * bootloader reboot on unpack failure (a missing cluster means a plain
     * system boot image, which must never be failed by us). */
    int recovery_mode = path_exists(kRecovery) || path_exists(kCluster);
    if (recovery_mode) {
        fox_klog("main: recovery_mode=1\n");
    } else {
        fox_klog("main: recovery_mode=0\n");
    }
    /* Pre-flight the cluster before handing it to lgz: size + magic.
     * A truncated/corrupt payload is the prime suspect when the
     * decompressor dies instantly, and this check needs no child. */
    {
        struct stat st;
        if (stat(kCluster, &st) == 0) {
            char line[96];
            int fd = open(kCluster, O_RDONLY | O_CLOEXEC);
            /* "cluster bytes=N magic=........" (8 raw bytes as-is). */
            unsigned long sz = (unsigned long)st.st_size;
            char *pp = line;
            const char *pfx = "main: cluster bytes=";
            while (*pfx) *pp++ = *pfx++;
            if (sz == 0) {
                *pp++ = '0';
            } else {
                char rbuf[24];
                int rn = 0;
                while (sz > 0 && rn < (int)sizeof(rbuf)) {
                    rbuf[rn++] = (char)('0' + (sz % 10));
                    sz /= 10;
                }
                while (rn > 0) *pp++ = rbuf[--rn];
            }
            {
                const char *mp = " magic=";
                while (*mp) *pp++ = *mp++;
            }
            if (fd >= 0) {
                unsigned char mg[8];
                ssize_t rr = 0, got = 0;
                while (got < 8) {
                    rr = read(fd, mg + got, 8 - (size_t)got);
                    if (rr <= 0) break;
                    got += rr;
                }
                if (got == 8) {
                    static const char hd[] = "0123456789abcdef";
                    int k;
                    for (k = 0; k < 8; k++) {
                        *pp++ = hd[(mg[k] >> 4) & 0xF];
                        *pp++ = hd[mg[k] & 0xF];
                    }
                } else {
                    const char *me = "<short>";
                    while (*me) *pp++ = *me++;
                }
                close(fd);
            } else {
                const char *me = "<unopenable>";
                while (*me) *pp++ = *me++;
            }
            *pp++ = '\n';
            *pp = '\0';
            fox_klog(line);
        } else {
            fox_klog("main: cluster stat failed\n");
        }
    }
    int unpacked = 0;
    if (path_exists(kCluster)) {
        fox_klog("main: cluster present\n");
        if (path_exists(kSnapshotManifest)) {
            fox_klog("main: snapshot manifest present\n");
            /* Built-in snapshot (snapshot.c). The Rust ramdisk_snapshot
             * binary stays on board as fallback for the legacy init.cpp
             * path and emergencies — unused on the normal stub branch. */
            int snap_st = snapshot_run(kSnapshotManifest, kSnapshotDir);
            if (snap_st == 0) {
                fox_klog("main: builtin snapshot ok\n");
            } else {
                fox_klog("main: builtin snapshot failed, trying fallback\n");
                char* const snap_argv[] = {
                    (char*)"ramdisk_snapshot",
                    (char*)kSnapshotManifest,
                    (char*)kSnapshotDir,
                    NULL,
                };
                snap_st = run_child(kSnapshotBin, snap_argv);
                if (snap_st == 0) {
                    fox_klog("main: fallback snapshot ok\n");
                } else {
                    fox_klog("main: fallback snapshot failed\n");
                }
            }
            (void)snap_st; /* snapshot is best-effort, never fatal */
        } else {
            fox_klog("main: no snapshot manifest\n");
        }
        char* const lgz_argv[] = {
            (char*)"lgz", (char*)"decompress", (char*)kCluster, (char*)"/", NULL,
        };
        fox_check_lgz();
        fox_klog("main: running lgz decompress\n");
        if (run_child(kLgzBin, lgz_argv) != 0) {
            fox_klog("main: lgz decompress failed\n");
            if (recovery_mode) reboot_bootloader();
            fox_klog("main: lgz failed, non-recovery, continuing\n");
        } else {
            fox_klog("main: lgz decompress ok\n");
            unpacked = 1;
        }
    } else {
        fox_klog("main: no cluster\n");
    }

    /* AIO family swap (aioswap.c): the tree is fully unpacked here and the
     * real init has not parsed anything yet, so family files (fstab,
     * flags, wipe, USB controller in rc, KeyMint set, selector prop) can
     * still be swapped. Best-effort, never fatal. */
    aioswap_run();

    /* Markers stay truthful: only when the cluster was actually unpacked.
     * Without an unpack there is no final tree to fast-path to. */
    if (unpacked) {
        fox_klog("main: marking unpacked\n");
        mark_unpacked();
    } else {
        fox_klog("main: nothing unpacked, no markers\n");
    }

    /* 4. The unpack may have produced the real init: re-check before
     * falling through (covers markers-lost and just-unpacked alike). */
    if (path_exists(kFoxInit)) {
        fox_klog("main: fox.init present after unpack\n");
        exec_real(argc, argv, envp);
    }
    fox_klog("main: fox.init still missing\n");

    /* 5. Last resort: no markers, no fox.init, no (usable) cluster.
     * Hand off to the kernel entry /init (possibly a Magisk/KSU hook);
     * if it resolves back to us there is nothing left to try. */
    if (!root_init_is_self()) {
        fox_klog("main: handing off to /init\n");
        execve(kInitRoot, argv, envp);
        fox_klog("main: /init execve failed\n");
    } else {
        fox_klog("main: /init resolves to self, no handoff\n");
    }
    fox_klog("main: rebooting to bootloader\n");
    reboot_bootloader();
    return 127;
}
