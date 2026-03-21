from patch import BaseSubPatch, Colors

class SubPatch(BaseSubPatch):
    def __init__(self, manager):
        super().__init__(manager)
        self.name = "Custom init modifications (init.cpp)"
        self.target_file = "system/core/init/init.cpp" # bootable/recovery/gui/gui.cpp

        # Список изменений: [(Оригинал, Модификация), ...]
        self.CHANGES = [
            (
                # Блок 1: Оригинальный код для поиска
                r"""
#include <sys/signalfd.h>
#include <sys/types.h>
#include <sys/utsname.h>
#include <unistd.h>

#define _REALLY_INCLUDE_SYS__SYSTEM_PROPERTIES_H_
""",
                # Блок 1: Модифицированный код (результат)
                r"""
#include <sys/signalfd.h>
#include <sys/types.h>
#include <sys/utsname.h>
#include <sys/wait.h>
#include <unistd.h>

#define _REALLY_INCLUDE_SYS__SYSTEM_PROPERTIES_H_
"""
            ),
            (
                # Блок 1: Оригинальный код для поиска
                r"""
    trigger_shutdown = [](const std::string& command) { shutdown_state.TriggerShutdown(command); };

    SetStdioToDevNull(argv);
    InitKernelLogging(argv);
    LOG(INFO) << "init second stage started!";

    SelinuxSetupKernelLogging();

    // Update $PATH in the case the second stage init is newer than first stage init, where it is
    // first set.
    if (setenv("PATH", _PATH_DEFPATH, 1) != 0) {
        PLOG(FATAL) << "Could not set $PATH to '" << _PATH_DEFPATH << "' in second stage";
    }
""",
                # Блок 1: Модифицированный код (результат)
                r"""
    trigger_shutdown = [](const std::string& command) { shutdown_state.TriggerShutdown(command); };

    SetStdioToDevNull(argv);
    InitKernelLogging(argv);
    LOG(INFO) << "init second stage started!";

    SelinuxSetupKernelLogging();

    // --- RAMDISK SNAPSHOT: Save ramdisk state before any modification ---
    // Must run BEFORE LGZ decompression so the snapshot preserves compressed files.
    // This allows reflash_twrp.sh to recreate the exact boot image later.
    {
        struct stat snap_st;
        if (stat("/ramdisk_snapshot_manifest.txt", &snap_st) == 0) {
            LOG(INFO) << "[SNAPSHOT] Creating ramdisk snapshot...";
            pid_t pid = fork();
            if (pid == 0) {
                execl("/system/bin/ramdisk_snapshot", "ramdisk_snapshot",
                      "/ramdisk_snapshot_manifest.txt",
                      "/dev/ramdisk_snapshot", nullptr);
                _exit(127);
            } else if (pid > 0) {
                int wstatus;
                waitpid(pid, &wstatus, 0);
                if (WIFEXITED(wstatus) && WEXITSTATUS(wstatus) == 0) {
                    LOG(INFO) << "[SNAPSHOT] Ramdisk snapshot created successfully";
                } else {
                    LOG(WARNING) << "[SNAPSHOT] Snapshot failed, status=" << wstatus
                                 << " (reflash may not work)";
                }
            } else {
                PLOG(ERROR) << "[SNAPSHOT] fork() failed";
            }
        }
    }
    // --- END RAMDISK SNAPSHOT ---

    // --- LGZ: Decompress ramdisk files at the earliest possible moment ---
    // Must run BEFORE PropertyInit, SELinux, RC parsing, or any file access.
    // Calls pre-compiled /system/bin/lgz binary instead of embedded code.
    {
        struct stat lgz_st;
        if (stat("/lgz_compressed_files.txt", &lgz_st) == 0) {
            LOG(INFO) << "[LGZ] Starting early decompression of ramdisk files...";
            pid_t pid = fork();
            if (pid == 0) {
                execl("/system/bin/lgz", "lgz", "decompress_all",
                      "/lgz_compressed_files.txt", nullptr);
                _exit(127);
            } else if (pid > 0) {
                int wstatus;
                waitpid(pid, &wstatus, 0);
                if (WIFEXITED(wstatus) && WEXITSTATUS(wstatus) == 0) {
                    LOG(INFO) << "[LGZ] Decompression completed successfully";
                } else {
                    LOG(ERROR) << "[LGZ] Decompression failed, status=" << wstatus;
                }
            } else {
                PLOG(ERROR) << "[LGZ] fork() failed";
            }
        }
    }
    // --- END LGZ ---

    // Update $PATH in the case the second stage init is newer than first stage init, where it is
    // first set.
    if (setenv("PATH", _PATH_DEFPATH, 1) != 0) {
        PLOG(FATAL) << "Could not set $PATH to '" << _PATH_DEFPATH << "' in second stage";
    }
"""
            )
        ]

