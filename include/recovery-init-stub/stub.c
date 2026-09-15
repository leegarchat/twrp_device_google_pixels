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

static const char kInitReal[] = "/system/bin/init.real";
static const char kCluster[] = "/lgz_cluster.lgz";
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

int main(int argc, char** argv, char** envp) {
    /* Cluster already unpacked (second and later invocations): passthrough. */
    if (path_exists(kInitReal)) exec_real(argc, argv, envp);

    /* First invocation: unpack path. Mirrors the IsRecoveryMode gate
     * (system/core/init/util.cpp): recovery binary OR cluster present. */
    int recovery_mode = path_exists(kRecovery) || path_exists(kCluster);
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
        if (run_child(kLgzBin, lgz_argv) != 0 && recovery_mode) reboot_bootloader();
    }

    exec_real(argc, argv, envp);
    return 127;
}
