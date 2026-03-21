from patch import BaseSubPatch, Colors

class SubPatch(BaseSubPatch):
    def __init__(self, manager):
        super().__init__(manager)
        self.name = "Custom reflash script (twrpRepacker.cpp)"
        self.target_file = "bootable/recovery/twrpRepacker.cpp" # bootable/recovery/gui/gui.cpp

        # Список изменений: [(Оригинал, Модификация), ...]
        self.CHANGES = [
            (
                # Блок 1: Оригинальный код для поиска
                r"""
#include <string>

#include "data.hpp"
#include "partitions.hpp"
#include "twrp-functions.hpp"
#include "twrpRepacker.hpp"
#include "twcommon.h"
#include "variables.h"
#include "gui/gui.hpp"
""",
                # Блок 1: Модифицированный код (результат)
                r"""
#include <string>
#include <cstdio>
#include <cstring>
#include <sys/wait.h>

#include "data.hpp"
#include "partitions.hpp"
#include "twrp-functions.hpp"
#include "twrpRepacker.hpp"
#include "twcommon.h"
#include "variables.h"
#include "gui/gui.hpp"
"""
            ),
            (
                # Блок 1: Оригинальный код для поиска
                r"""
bool twrpRepacker::Flash_Current_Twrp() {
#ifndef OF_RECOVERY_AB_FULL_REFLASH_RAMDISK
	// A/B with dedicated recovery partition
	std::string slot = android::base::GetProperty("ro.boot.slot_suffix", "");
	if (slot.empty())
		slot = android::base::GetProperty("ro.boot.slot", "");

	std::string dest_partition = "/recovery";
	#if defined(FOX_VENDOR_BOOT_RECOVERY) || defined(BOARD_MOVE_RECOVERY_RESOURCES_TO_VENDOR_BOOT)
		dest_partition = "/vendor_boot";
	#endif
""",
                # Блок 1: Модифицированный код (результат)
                r"""
bool twrpRepacker::Flash_Current_Twrp() {
    if (TWFunc::Path_Exists("/system/bin/reflash_twrp.sh")) {
        gui_print("- Starting custom reflash recovery script\n");

        // Run script via popen so stdout is captured and shown in the UI log
        std::string command = "/system/bin/reflash_twrp.sh 2>&1";
        FILE *fp = popen(command.c_str(), "r");
        if (!fp) {
            LOGERR("Failed to execute reflash_twrp.sh");
            return false;
        }

        char line[512];
        while (fgets(line, sizeof(line), fp)) {
            // Strip trailing newline for gui_print (it adds its own)
            size_t len = strlen(line);
            if (len > 0 && line[len - 1] == '\n') line[len - 1] = '\0';
            gui_print("%s\n", line);
        }

        int status = pclose(fp);
        if (status != 0) {
            int code = WIFEXITED(status) ? WEXITSTATUS(status) : status;
            LOGERR("Script reflash_twrp.sh failed with error code: %d", code);
            gui_print_color("error", "Script reflash_twrp.sh failed with error code: %d\n", code);
            return false;
        }
        gui_print_color("green", "- Successfully flashed recovery to both slots\n");
        return true;
    }
#ifndef OF_RECOVERY_AB_FULL_REFLASH_RAMDISK
	// A/B with dedicated recovery partition
	std::string slot = android::base::GetProperty("ro.boot.slot_suffix", "");
	if (slot.empty())
		slot = android::base::GetProperty("ro.boot.slot", "");

	std::string dest_partition = "/recovery";
	#if defined(FOX_VENDOR_BOOT_RECOVERY) || defined(BOARD_MOVE_RECOVERY_RESOURCES_TO_VENDOR_BOOT)
		dest_partition = "/vendor_boot";
	#endif
"""
            )
        ]

