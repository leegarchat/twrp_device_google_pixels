/* recovery-init-stub — static PID 1 entry point for OrangeFox recovery.
 *
 * Occupies /system/bin/init. The real AOSP init lives at
 * /system/bin/init.real INSIDE /lgz_cluster.lgz.
 *
 * - First invocation (selinux_setup, init.real absent): snapshot the ramdisk,
 *   unpack the cluster, verify, then exec the real init.
 * - Later invocations (second_stage, init.real present): exec through
 *   immediately (passthrough, no work).
 *
 * Plain C, libc calls only (no threads, no stdio buffering: direct write(2)
 * to /dev/kmsg). Built static (see Android.bp): must run before the cluster
 * is unpacked, so it cannot depend on any shared library.
 */

#include <errno.h>
#include <fcntl.h>
#include <linux/reboot.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/reboot.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/sysmacros.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

static const char kInitReal[] = "/system/bin/init.real";
static const char kCluster[] = "/lgz_cluster.lgz";
static const char kRecovery[] = "/system/bin/recovery";
static const char kSnapshotBin[] = "/system/bin/ramdisk_snapshot";
static const char kSnapshotManifest[] = "/ramdisk_snapshot_manifest.txt";
static const char kSnapshotDir[] = "/dev/ramdisk_snapshot";
static const char kLgzBin[] = "/system/bin/lgz";
static const char kKmsg[] = "/dev/kmsg";
static const char kKlog[] = "/dev/block/by-name/klog";
static const char kKlogTmp[] = "/dev/.klogblk";
/* klog record offset: 8MB — head of klog belongs to the bootloader's logs. */
static const off_t kKlogOffset = 8 * 1024 * 1024;

/* DEBUG post-mortem stages (klog record: [BUST LE][stage][status][0]). */
enum {
    STAGE_ENTER = 1,
    STAGE_PASSTHROUGH = 2,
    STAGE_SNAP_DONE = 3,
    STAGE_UNPACK_DONE = 4,
    STAGE_EXEC = 5,
    STAGE_EXEC_FAIL = 6,
    STAGE_REBOOT = 7,
};

static int g_kmsg_fd = -1;

static void kmsg_write(const char* msg, size_t len) {
    size_t off = 0;
    while (off < len) {
        ssize_t n = write(g_kmsg_fd, msg + off, len - off);
        if (n <= 0) break;
        off += (size_t)n;
    }
}

static void kmsg_log(const char* msg) {
    if (g_kmsg_fd < 0) return;
    kmsg_write("[init-stub] ", 12);
    kmsg_write(msg, strlen(msg));
    kmsg_write("\n", 1);
}

static int path_exists(const char* path) {
    return access(path, F_OK) == 0;
}

/* Best-effort post-mortem marker on the raw klog partition (survives reboot).
 * by-name may be absent (recovery first-stage skips ueventd) — then mknod the
 * shiba-debug-only node (klog == sda2 == 8:2). */
static void klog_mark(unsigned stage, int status) {
    int fd = open(kKlog, O_WRONLY | O_CLOEXEC);
    if (fd < 0) {
        kmsg_log("klog by-name absent, mknod sda2 fallback");
        mknod(kKlogTmp, S_IFBLK | 0600, makedev(8, 2));
        fd = open(kKlogTmp, O_WRONLY | O_CLOEXEC);
    }
    if (fd < 0) {
        kmsg_log("klog marker skipped: no block node available");
        return;
    }
    unsigned char rec[16] = {0};
    unsigned magic = 0x54535542;
    memcpy(rec, &magic, 4);
    memcpy(rec + 4, &stage, 4);
    memcpy(rec + 8, &status, 4);
    if (lseek(fd, kKlogOffset, SEEK_SET) < 0) {
        kmsg_log("klog marker skipped: lseek failed");
        close(fd);
        return;
    }
    size_t off = 0;
    while (off < sizeof(rec)) {
        ssize_t n = write(fd, rec + off, sizeof(rec) - off);
        if (n <= 0) break;
        off += (size_t)n;
    }
    fsync(fd);
    close(fd);
    unlink(kKlogTmp);
}

/* fork + execve + synchronous wait. Returns the child exit code, or -1. */
static int run_child(const char* prog, char* const argv[]) {
    pid_t pid = fork();
    if (pid < 0) {
        kmsg_log("fork failed");
        return -1;
    }
    if (pid == 0) {
        extern char** environ;
        execve(prog, argv, environ);
        _exit(127);
    }
    int status = 0;
    while (waitpid(pid, &status, 0) < 0) {
        if (errno != EINTR) {
            kmsg_log("waitpid failed");
            return -1;
        }
    }
    if (WIFEXITED(status)) return WEXITSTATUS(status);
    return -1;
}

static void reboot_bootloader(void) {
    kmsg_log("bootstrap failed, rebooting to bootloader");
    klog_mark(STAGE_REBOOT, 0);
    sync();
    /* bionic reboot() takes no arg; raw reboot(2) carries "bootloader". */
    syscall(__NR_reboot, LINUX_REBOOT_MAGIC1, LINUX_REBOOT_MAGIC2,
            LINUX_REBOOT_CMD_RESTART2, "bootloader");
    for (;;) sleep(60);
}

static void exec_real(int argc, char** argv, char** envp) {
    (void)argc;
    klog_mark(STAGE_EXEC, 0);
    execve(kInitReal, argv, envp);
    /* execve returns only on error (ENOENT etc.). */
    int e = errno;
    kmsg_log("FATAL: exec init.real failed");
    klog_mark(STAGE_EXEC_FAIL, e);
    reboot_bootloader();
}

int main(int argc, char** argv, char** envp) {
    g_kmsg_fd = open(kKmsg, O_WRONLY | O_CLOEXEC);
    klog_mark(STAGE_ENTER, 0);

    /* Cluster already unpacked (second and later invocations): passthrough. */
    if (path_exists(kInitReal)) {
        klog_mark(STAGE_PASSTHROUGH, 0);
        exec_real(argc, argv, envp);
    }

    /* First invocation: unpack path. Mirrors the IsRecoveryMode gate
     * (system/core/init/util.cpp): recovery binary OR cluster present. */
    int recovery_mode = path_exists(kRecovery) || path_exists(kCluster);
    if (path_exists(kCluster)) {
        if (path_exists(kSnapshotManifest)) {
            kmsg_log("snapshot: running ramdisk_snapshot (pre-unpack state)");
            /* Mirrors init.cpp: snapshot failure is a warning, never fatal. */
            char* const snap_argv[] = {
                (char*)"ramdisk_snapshot",
                (char*)kSnapshotManifest,
                (char*)kSnapshotDir,
                NULL,
            };
            int snap_st = run_child(kSnapshotBin, snap_argv);
            klog_mark(STAGE_SNAP_DONE, snap_st);
            if (snap_st != 0) kmsg_log("snapshot failed, continuing anyway");
        }
        kmsg_log("unpack: lgz decompress /lgz_cluster.lgz /");
        char* const lgz_argv[] = {
            (char*)"lgz", (char*)"decompress", (char*)kCluster, (char*)"/", NULL,
        };
        int st = run_child(kLgzBin, lgz_argv);
        klog_mark(STAGE_UNPACK_DONE, st);
        if (st != 0) {
            kmsg_log("unpack failed");
            if (recovery_mode) reboot_bootloader();
            kmsg_log("unpack failed outside recovery mode, continuing anyway");
        } else {
            kmsg_log("unpack OK");
        }
    }

    exec_real(argc, argv, envp);
    return 127;
}
