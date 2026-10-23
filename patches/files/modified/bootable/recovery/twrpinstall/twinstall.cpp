/*
	Copyright 2012 to 2017 bigbiff/Dees_Troy TeamWin
	This file is part of TWRP/TeamWin Recovery Project.

	Copyright (C) 2018-2025 OrangeFox Recovery Project
	This file is part of the OrangeFox Recovery Project.

	TWRP is free software: you can redistribute it and/or modify
	it under the terms of the GNU General Public License as published by
	the Free Software Foundation, either version 3 of the License, or
	(at your option) any later version.

	TWRP is distributed in the hope that it will be useful,
	but WITHOUT ANY WARRANTY; without even the implied warranty of
	MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
	GNU General Public License for more details.

	You should have received a copy of the GNU General Public License
	along with TWRP.  If not, see <http://www.gnu.org/licenses/>.
*/


#ifndef _GNU_SOURCE
#define _GNU_SOURCE
#endif

#include <ctype.h>
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <sys/mount.h>
#include <unistd.h>
#include <iostream>
#include <fstream>

#include <string.h>
#include <stdio.h>
#include <cutils/properties.h>

#include <android-base/unique_fd.h>

#include "twcommon.h"
#include "mtdutils/mounts.h"
#include "mtdutils/mtdutils.h"

#include "otautil/sysutil.h"
#include <ziparchive/zip_archive.h>
#include "twinstall/install.h"
#include "twinstall/verifier.h"
#include "variables.h"
#include "data.hpp"
#include "partitions.hpp"
#include "twrpDigestDriver.hpp"
#include "twrpDigest/twrpDigest.hpp"
#include "twrpDigest/twrpMD5.hpp"
#include "twrp-functions.hpp"
#include "orangefox.hpp"
#include "gui/gui.hpp"
#include "gui/pages.hpp"
#include "gui/blanktimer.hpp"
#include "twinstall.h"
#include "installcommand.h"
#include "../twrpRepacker.hpp"
extern "C" {
	#include "gui/gui.h"
}

#define AB_OTA "payload_properties.txt"

enum zip_type {
	UNKNOWN_ZIP_TYPE = 0,
	UPDATE_BINARY_ZIP_TYPE,
	AB_OTA_ZIP_TYPE,
	TWRP_THEME_ZIP_TYPE
};

static int Install_Theme(const char* path, ZipArchiveHandle Zip) {
#ifdef TW_OEM_BUILD // We don't do custom themes in OEM builds
	return INSTALL_CORRUPT;
#else
	std::string binary_name("ui.xml");
	ZipEntry64 binary_entry;
	if (FindEntry(Zip, binary_name, &binary_entry) != 0) {
		return INSTALL_CORRUPT;
	}
	if (!PartitionManager.Mount_Settings_Storage(true))
		return INSTALL_ERROR;
	std::string theme_path = DataManager::GetCurrentStoragePath() + "/theme";
	if (!TWFunc::Path_Exists(theme_path)) {
		if (!TWFunc::Recursive_Mkdir(theme_path)) {
			return INSTALL_ERROR;
		}
	}
	theme_path += "/ui.zip";
	if (TWFunc::copy_file(path, theme_path, 0644) != 0) {
		return INSTALL_ERROR;
	}
	LOGINFO("Installing custom theme '%s' to '%s'\n", path, theme_path.c_str());
	PageManager::RequestReload();
	return INSTALL_SUCCESS;
#endif
}

static int Prepare_Update_Binary(const char *path, ZipArchiveHandle Zip) {
	if (TWFunc::Block_Operations_Until_Reboot())
		return INSTALL_ERROR;

	char arches[PATH_MAX];
	property_get("ro.product.cpu.abilist", arches, "error");
	if (strcmp(arches, "error") == 0)
		property_get("ro.product.cpu.abi", arches, "error");
	vector<string> split = TWFunc::split_string(arches, ',', true);
	std::vector<string>::iterator arch;
	std::string base_name = UPDATE_BINARY_NAME;
	base_name += "-";
	ZipEntry64 binary_entry;
	std::string update_binary_string(UPDATE_BINARY_NAME);
	if (FindEntry(Zip, update_binary_string, &binary_entry) != 0) {
		for (arch = split.begin(); arch != split.end(); arch++) {
			std::string temp = base_name + *arch;
			std::string binary_name(temp.c_str());
			if (FindEntry(Zip, binary_name, &binary_entry) != 0) {
				std::string binary_name(temp.c_str());
				break;
			}
		}
	}
	LOGINFO("Extracting updater binary '%s'\n", UPDATE_BINARY_NAME);
	unlink(TMP_UPDATER_BINARY_PATH);
	android::base::unique_fd fd(
		open(TMP_UPDATER_BINARY_PATH, O_CREAT | O_WRONLY | O_TRUNC | O_CLOEXEC, 0755));
	if (fd == -1) {
		return INSTALL_ERROR;
	}
	int32_t err = ExtractEntryToFile(Zip, &binary_entry, fd);
	if (err != 0) {
		LOGERR("Could not extract '%s'\n", UPDATE_BINARY_NAME);
		return INSTALL_ERROR;
	}

	// -------------- OrangeFox: start ---------------- //
	int Fox_Ret = Fox_Prepare_Update_Binary(path, Zip);
	if (Fox_Ret != INSTALL_SUCCESS) {
	   return Fox_Ret;
	}
	// -------------- OrangeFox: end ---------------- //

	// If exists, extract file_contexts from the zip file
	std::string file_contexts("file_contexts");
	ZipEntry64 file_contexts_entry;
	if (FindEntry(Zip, file_contexts, &file_contexts_entry) != 0) {
		LOGINFO("Zip does not contain SELinux file_contexts file in its root.\n");
	} else {
		const string output_filename = "/file_contexts";
		LOGINFO("Zip contains SELinux file_contexts file in its root. Extracting to %s\n", output_filename.c_str());
		android::base::unique_fd fd(
			open(output_filename.c_str(), O_CREAT | O_WRONLY | O_TRUNC | O_CLOEXEC, 0644));
		if (fd == -1) {
			return INSTALL_ERROR;
		}
		if (ExtractEntryToFile(Zip, &file_contexts_entry, fd)) {
			LOGERR("Could not extract '%s'\n", output_filename.c_str());
			return INSTALL_ERROR;
		}
	}
	return INSTALL_SUCCESS;
}


static int Run_Update_Binary(const char *path, int* wipe_cache, zip_type ztype) {
	int ret_val, pipe_fd[2], status, zip_verify;
	int aroma_running = 0;
	char buffer[1024];
	FILE* child_data;
	pipe(pipe_fd);

	std::vector<std::string> args;
    if (ztype == UPDATE_BINARY_ZIP_TYPE) {
		ret_val = update_binary_command(path, 0, pipe_fd[1], &args);
    } else if (ztype == AB_OTA_ZIP_TYPE) {
		ret_val = abupdate_binary_command(path, 0, pipe_fd[1], &args);
	} else {
		LOGERR("Unknown zip type %i\n", ztype);
		ret_val = INSTALL_CORRUPT;
	}
    if (ret_val) {
        close(pipe_fd[0]);
        close(pipe_fd[1]);
        return ret_val;
    }

	// Convert the vector to a NULL-terminated char* array suitable for execv.
	const char* chr_args[args.size() + 1];
	chr_args[args.size()] = NULL;
	for (size_t i = 0; i < args.size(); i++)
		chr_args[i] = args[i].c_str();

	pid_t pid = fork();
	if (pid == 0) {
		close(pipe_fd[0]);
		execve(chr_args[0], const_cast<char**>(chr_args), environ);
		printf("E:Can't execute '%s': %s\n", chr_args[0], strerror(errno));
		_exit(-1);
	}
	close(pipe_fd[1]);

	*wipe_cache = 0;

	DataManager::GetValue(TW_SIGNED_ZIP_VERIFY_VAR, zip_verify);
	child_data = fdopen(pipe_fd[0], "r");
	while (fgets(buffer, sizeof(buffer), child_data) != NULL) {
		char* command = strtok(buffer, " \n");
		if (command == NULL) {
			continue;
		} else if (strcmp(command, "progress") == 0) {
			char* fraction_char = strtok(NULL, " \n");
			char* seconds_char = strtok(NULL, " \n");

			float fraction_float = strtof(fraction_char, NULL);
			int seconds_float = strtol(seconds_char, NULL, 10);

			if (zip_verify)
				DataManager::ShowProgress(fraction_float * (1 - VERIFICATION_PROGRESS_FRACTION), seconds_float);
			else
				DataManager::ShowProgress(fraction_float, seconds_float);
		} else if (strcmp(command, "set_progress") == 0) {
			char* fraction_char = strtok(NULL, " \n");
			float fraction_float = strtof(fraction_char, NULL);
			DataManager::_SetProgress(fraction_float);
		} else if (strcmp(command, "ui_print") == 0) {
			char* display_value = strtok(NULL, "\n");
	  		if (display_value) {
	      		     if (strcmp(display_value, "AROMA Filemanager Finished...") == 0 && (aroma_running == 1)) {
		  		aroma_running = 0;
		  		gui_changeOverlay("");
		  		TWFunc::copy_file(Fox_aroma_cfg, Fox_sdcard_aroma_cfg, 0644);
			     }
	      		    gui_print("%s", display_value);
	      		    if (strcmp(display_value, "(c) 2013-2015 by amarullz.com") == 0 && (aroma_running == 0)) {
		  		aroma_running = 1;
		  		gui_changeOverlay("black_out");
		  		TWFunc::copy_file(Fox_aroma_cfg, Fox_sdcard_aroma_cfg, 0644);
			     }
	    		}
	  		else {
	      			gui_print("\n");
	    		}
		} else if (strcmp(command, "wipe_cache") == 0) {
			*wipe_cache = 1;
		} else if (strcmp(command, "clear_display") == 0) {
			// Do nothing, not supported by TWRP
		} else if (strcmp(command, "log") == 0) {
			printf("%s\n", strtok(NULL, "\n"));
		} else {
			LOGERR("unknown command [%s]\n", command);
		}
	}
	fclose(child_data);

	int waitrc = TWFunc::Wait_For_Child(pid, &status, "Updater");

  	// Should never happen, but in case of crash or other unexpected condition
  	if (aroma_running == 1) {
      		gui_changeOverlay("");
    	}

  	// if updater-script doesn't find the correct device
  	if (WEXITSTATUS (status) == TW_ERROR_WRONG_DEVICE) {
       		gui_print_color("error", "\nPossible causes of this error:\n  1. Wrong device\n  2. Wrong firmware\n  3. Corrupt zip\n  4. System not mounted\n  5. Bugged updater-script.\n\nSearch online for \"error %i\". ",
       			TW_ERROR_WRONG_DEVICE);
       		gui_print_color("error", "Check \"/tmp/recovery.log\", and look above, for the specific cause of this error.\n\n");
     	}

  	if (waitrc != 0) {
      		set_miui_install_status(OTA_CORRUPT, false);
      		return INSTALL_ERROR;
    	}

	return INSTALL_SUCCESS;
}

// ---- Fox high-speed install (outer layer, AB branch of TWinstall_zip) ----
// Independent /system/bin/fox-hsinstall (Rust, static musl, no cargo).
// Single live stream: child stdout+stderr are duped into the status pipe
// (reflash_twrp.sh pattern) and relayed per line to the GUI console.
// Progress lines are absolute (peridot parity): the engine's `progress`
// is ignored, `set_progress` maps to DataManager::SetProgress (capped
// at 0.99, 1.0 only after the slot switch below).
// Exit contract:
//   0 ok (pre-ROM env + baseline run before the engine; slot switched
//     below via PartitionManager, never by the engine)
//   2 not a full OTA -> stock path
//   5 live super unusable -> stock path
//   else hard error (4 = device mismatch, 3 = no fit, 1 = failed, 6 = usage).
static constexpr const char* HS_ENGINE = "/system/bin/fox-hsinstall";
static constexpr int HS_EXIT_NOT_FULL = 2;
static constexpr int HS_EXIT_NO_HELPERS = 5;

// Stock A/B fallback: a declined/unticked HS install takes the normal
// update_engine path (same as peridot: toggle off == NotApplicable).

enum class FoxHighSpeedResult { kNotApplicable, kAborted, kSucceeded };

static std::string HsTargetSlot() {
	// Opposite of the EFFECTIVE slot. Display tracks bootctrl active
	// across live switches; ro.boot.* may be spoofed (session view) or
	// stale, so neither is the right "inactive slot" definition here.
	return PartitionManager.Get_Active_Slot_Display() == "A" ? "b" : "a";
}

static FoxHighSpeedResult FoxTryHighSpeedInstall(const std::string& package) {
	if (DataManager::GetIntValue("tw_high_speed_flash") == 0)
		return FoxHighSpeedResult::kNotApplicable;

	// ADB sideload serves /sideload/package.zip as an on-demand FUSE
	// stream (single-block cache, every miss is a host round-trip).
	// The engine needs thousands of random preads over gigabytes,
	// which either stalls for hours or ends in EIO -> hard abort.
	// The stock update_engine_sideload path is designed for exactly
	// this stream (single seek + sequential read), so decline before
	// fork() and let it through. Same prefix style as the digest
	// skip in TWinstall_zip below.
	if (package.compare(0, 9, "/sideload") == 0) {
		LOGINFO("hs: sideload package, stock update_engine path\n");
		return FoxHighSpeedResult::kNotApplicable;
	}

	// Live-slot guard turned normalizer: never flash the physically booted
	// slot. Booted is the /tmp snapshot truth (never ro.boot.* / bootctl,
	// which show the spoofed slot after a live switch). On a trip, switch
	// the session back to the booted slot and continue — the ROM always
	// lands on the real opposite slot, no reboot required. Only an
	// unreadable booted slot or a failed switch aborts (nothing flashed
	// yet, so a plain return is safe).
	// External runtime switchers (DFE NEO-style zips, manual bootctl)
	// move bootctrl active behind TWRP's back. Reconcile first so the
	// target math below runs on the truth; no-op when already in sync.
	{
		std::string boot_active = PartitionManager.Get_Bootctrl_Active_Slot();
		if (!boot_active.empty() && boot_active != PartitionManager.Get_Active_Slot_Display()) {
			LOGINFO("hs: bootctrl says %s, session shows %s; reconciling\n",
				boot_active.c_str(), PartitionManager.Get_Active_Slot_Display().c_str());
			if (!PartitionManager.Live_Switch_Slot(boot_active)) {
				gui_err("switch_slot_fail=Slot switch failed. See the log for details.");
				return FoxHighSpeedResult::kAborted;
			}
		}
	}
	std::string hs_target = HsTargetSlot();
	std::string hs_booted = PartitionManager.Get_Booted_Slot_Display();
	if (hs_booted.empty()) {
		gui_err("hs_no_booted_slot=Cannot read the booted slot. Refusing to flash.");
		return FoxHighSpeedResult::kAborted;
	}
	if ((hs_target == "b") == (hs_booted == "B")) {
		gui_warn("slot_live_reswitch=Target is the live booted slot; switching back to it and continuing.");
		std::string hs_back(1, hs_booted[0]);
		if (!PartitionManager.Live_Switch_Slot(hs_back)) {
			gui_err("switch_slot_fail=Slot switch failed. See the log for details.");
			return FoxHighSpeedResult::kAborted;
		}
		hs_target = HsTargetSlot();
	}
	std::string hs_booted_arg = std::string("--booted-slot=") + (char)tolower(hs_booted[0]);

	// Stock AB pre-ROM environment (mounts, bind sh, pre script,
	// recovery-overwrite baseline) BEFORE any destructive write —
	// same order and same calls as the stock branch below. The engine
	// replaces Run_Update_Binary; everything else is identical.
	bool hs_system_mounted = PartitionManager.Is_Mounted_By_Path(PartitionManager.Get_Android_Root_Path());
	bool hs_vendor_mounted = PartitionManager.Is_Mounted_By_Path("/vendor");
	PartitionManager.Mount_By_Path(PartitionManager.Get_Android_Root_Path(), false);
	PartitionManager.Mount_By_Path("/vendor", false);
	TWFunc::copy_file("/system/bin/sh", "/tmp/sh", 0755);
	mount("/tmp/sh", "/system/bin/sh", "auto", MS_BIND, NULL);
	usleep(32);
	if (TWFunc::Path_Exists(FOX_PRE_ROM_FLASH_SCRIPT))
		TWFunc::RunFoxScript(FOX_PRE_ROM_FLASH_SCRIPT, package);
	TWFunc::IsRecoveryOverwritten(true);
	auto hs_cleanup_mounts = [&]() {
		umount("/system/bin/sh");
		unlink("/tmp/sh");
		if (!hs_vendor_mounted)
			PartitionManager.UnMount_By_Path("/vendor", false);
		if (!hs_system_mounted)
			PartitionManager.UnMount_By_Path(PartitionManager.Get_Android_Root_Path(), false);
	};

	int pipe_fd[2];
	if (pipe(pipe_fd) != 0) {
		LOGERR("hs: pipe failed: %s\n", strerror(errno));
		hs_cleanup_mounts();
		return FoxHighSpeedResult::kAborted;
	}

	pid_t pid = fork();
	if (pid == -1) {
		LOGERR("hs: fork failed: %s\n", strerror(errno));
		close(pipe_fd[0]);
		close(pipe_fd[1]);
		hs_cleanup_mounts();
		return FoxHighSpeedResult::kAborted;
	}
	if (pid == 0) {
		// Merge stdout+stderr into the status pipe: one live stream.
		dup2(pipe_fd[1], STDOUT_FILENO);
		dup2(pipe_fd[1], STDERR_FILENO);
		close(pipe_fd[0]);
		execl(HS_ENGINE, "fox-hsinstall",
			"--zip", package.c_str(), hs_booted_arg.c_str(), (char*)nullptr);
		printf("E:hs: can't execute '%s': %s\n", HS_ENGINE, strerror(errno));
		_exit(-1);
	}
	close(pipe_fd[1]);

	// Reset the bar: engine fractions are absolute (0..1 of the whole
	// install, peridot-style). The engine's own `progress` line is
	// ignored below — a stale ShowProgress portion would pin every
	// later _SetProgress to zero.
	DataManager::SetProgress(0);
	char buffer[1024];
	FILE* child_data = fdopen(pipe_fd[0], "r");
	while (fgets(buffer, sizeof(buffer), child_data) != NULL) {
		char* command = strtok(buffer, " \n");
		if (command == NULL)
			continue;
		if (strcmp(command, "progress") == 0) {
			// Ignored on purpose (see above); keep a trace in the log.
			LOGINFO("hs: engine progress line ignored\n");
		} else if (strcmp(command, "set_progress") == 0) {
			char* fraction_char = strtok(NULL, " \n");
			if (fraction_char) {
				float frac = strtof(fraction_char, NULL);
				// Withhold 100% until the slot switch below succeeds.
				if (frac > 0.99f)
					frac = 0.99f;
				DataManager::SetProgress(frac);
			}
		} else if (strcmp(command, "ui_print") == 0) {
			char* display_value = strtok(NULL, "\n");
			if (display_value) {
				gui_print("hs: %s\n", display_value);
				LOGINFO("hs: %s\n", display_value);
			}
		} else if (strcmp(command, "log") == 0) {
			char* display_value = strtok(NULL, "\n");
			if (display_value)
				LOGINFO("hs-engine: %s\n", display_value);
		} else {
			// Human-readable engine output: relay live to the GUI
			// console (reflash_twrp.sh pattern) + log it.
			size_t len = strlen(buffer);
			while (len > 0 &&
			       (buffer[len - 1] == '\n' || buffer[len - 1] == '\r'))
				buffer[--len] = '\0';
			if (len > 0)
				gui_print("%s\n", buffer);
			LOGINFO("hs: %s\n", buffer);
		}
	}
	fclose(child_data);
	int status;
	waitpid(pid, &status, 0);
	int code = WIFEXITED(status) ? WEXITSTATUS(status) : -1;
	LOGINFO("hs: engine exit %d\n", code);

	if (code == 0) {
		// DFE NEO compat: it greps recovery.log for the literal AOSP-style
		// pair "source_slot: X" / "target_slot: Y" to resolve the
		// both-slots-good ambiguity. Emitted on engine success (the flash
		// really went booted->target), independent of the live switch
		// below — so even a failed switch leaves DFE pointing at the
		// freshly flashed slot, which then self-heals the session.
		// Single pair per boot by construction (a second flash in the
		// same session re-normalizes to the same target first).
		LOGINFO("hs: source_slot: %s\n", hs_booted.c_str());
		LOGINFO("hs: target_slot: %s\n", hs_target == "b" ? "B" : "A");
		// Engine wrote the target slot; restore the pre-ROM mount
		// state first (stock order). The session switch below runs
		// only with the live-switch toggle on; with it off the
		// session stays and the classic "reboot to switch" applies.
		// Reconcile + guard normalization above always run.
		hs_cleanup_mounts();
		std::string target = HsTargetSlot();
		std::string slot_up = (target == "b") ? "B" : "A";
		if (DataManager::GetIntValue("tw_live_switch") != 0) {
			if (!PartitionManager.Live_Switch_Slot(slot_up)) {
				gui_err("hs_slot_fail=High-speed install wrote the update but the slot switch failed.");
				return FoxHighSpeedResult::kAborted;
			}
			gui_msg("flash_ab_live_done=Slot switched live; you can flash additional zips without reboot.");
		} else {
			// Classic post: no live session switch, but the bootloader
			// MUST point at the freshly flashed slot (stock update_engine
			// does this internally; the hs engine never touches bootctrl
			// by design). Without this the reboot stays on the old slot.
			PartitionManager.Set_Active_Slot(slot_up);
			if (PartitionManager.Get_Active_Slot_Display() != slot_up) {
				gui_err("hs_slot_fail=High-speed install wrote the update but the slot switch failed.");
				return FoxHighSpeedResult::kAborted;
			}
			gui_warn("flash_ab_reboot=To flash additional zips, please reboot recovery to switch to the updated slot.");
		}
		if (android::base::GetBoolProperty("ro.virtual_ab.enabled", false)) {
			gui_warn("mount_vab_partitions=Devices on super may not mount until after rebooting recovery.");
		}		DataManager::SetProgress(1.0f);
		LOGINFO("hs: wrote slot %s (live switch %s)\n", target.c_str(),
			DataManager::GetIntValue("tw_live_switch") != 0 ? "on" : "off");
		return FoxHighSpeedResult::kSucceeded;
	}
	if (code == HS_EXIT_NOT_FULL || code == HS_EXIT_NO_HELPERS) {
		LOGINFO("hs: declined (%d), stock update_engine path\n", code);
		hs_cleanup_mounts();
		return FoxHighSpeedResult::kNotApplicable;
	}
	hs_cleanup_mounts();
	gui_err("hs_failed=High-speed install failed. See the log for details.");
	return FoxHighSpeedResult::kAborted;
}

int TWinstall_zip(const char *path, int *wipe_cache, bool check_for_digest)
{
  int ret_val, zip_verify = 1, unmount_system = 1, reflashtwrp = 0, unmount_vendor = 1;
  bool run_rom_scripts = false;

  if (TWFunc::Block_Operations_Until_Reboot())
	return INSTALL_CORRUPT;

  if (strcmp(path, "error") == 0)
    {
      LOGERR("Failed to get adb sideload file: '%s'\n", path);
      return INSTALL_CORRUPT;
    }

  if (DataManager::GetIntValue(FOX_INSTALL_PREBUILT_ZIP) == 1)
     {
         DataManager::SetValue(FOX_ZIP_INSTALLER_CODE, 0); // internal zip = standard zip installer
         DataManager::SetValue(FOX_ZIP_INSTALLER_TREBLE, 0);
     }    
  else   
    {
	gui_msg(Msg("installing_zip=Installing zip file '{1}'")(path));
	if (strlen(path) < 9 || strncmp(path, "/sideload", 9) != 0) {
		string digest_str;
		string Full_Filename = path;

		if (check_for_digest) {
			gui_msg("check_for_digest=Checking for Digest file...");
			if (*path != '@' && !twrpDigestDriver::Check_File_Digest(Full_Filename)) {
				LOGERR("Aborting zip install: Digest verification failed\n");
				return INSTALL_CORRUPT;
			}
		}
	}
    }

  DataManager::GetValue(TW_UNMOUNT_SYSTEM, unmount_system);
  DataManager::GetValue(TW_UNMOUNT_VENDOR, unmount_vendor);

#ifndef TW_OEM_BUILD
  DataManager::GetValue(TW_SIGNED_ZIP_VERIFY_VAR, zip_verify);
#endif

  DataManager::SetProgress(0);

	auto package = Package::CreateMemoryPackage(path);
	if (!package) {
		return INSTALL_CORRUPT;
	}

	if (zip_verify) {
		gui_msg("verify_zip_sig=Verifying zip signature...");
		static constexpr const char* CERTIFICATE_ZIP_FILE = "/system/etc/security/otacerts.zip";
		std::vector<Certificate> loaded_keys = LoadKeysFromZipfile(CERTIFICATE_ZIP_FILE);
		if (loaded_keys.empty()) {
			LOGERR("Failed to load keys\n");
			return -1;
		}
		LOGINFO("%zu key(s) loaded from %s\n", loaded_keys.size(), CERTIFICATE_ZIP_FILE);

		ret_val = verify_file(package.get(), loaded_keys, std::bind(&DataManager::SetProgress, std::placeholders::_1));
		if (ret_val != VERIFY_SUCCESS) {
			LOGINFO("Zip signature verification failed: %i\n", ret_val);
			gui_err("verify_zip_fail=Zip signature verification failed!");
#ifdef USE_MINZIP
			sysReleaseMap(&map);
#endif
			return -1;
		} else {
			gui_msg("verify_zip_done=Zip signature verified successfully.");
		}
    }
    
    ZipArchiveHandle Zip = package->GetZipArchiveHandle();
    if (!Zip) {
      set_miui_install_status(OTA_CORRUPT, true);
      gui_err("zip_corrupt=Zip file is corrupt!");
      return INSTALL_CORRUPT;
    }

    if (unmount_system) {
	if (PartitionManager.Is_Mounted_By_Path(PartitionManager.Get_Android_Root_Path())) {
		gui_msg("unmount_system=Unmounting System...");
		if (PartitionManager.UnMount_By_Path(PartitionManager.Get_Android_Root_Path(), false)) {
			//unlink(PartitionManager.Get_Android_Root_Path().c_str());
			//mkdir(PartitionManager.Get_Android_Root_Path().c_str(), 0755);
		}
		else {
			gui_msg("unmount_system_err=Failed to unmount System");
		        return -1;
		}
	}
   }

   if (unmount_vendor) {
	if (PartitionManager.Is_Mounted_By_Path("/vendor")) {
		gui_msg("unmount_vendor=Unmounting Vendor...");
		if (PartitionManager.UnMount_By_Path("/vendor", false)) {
		   	//unlink("/vendor");
		   	//mkdir("/vendor", 0755);
		} else {
			gui_msg("unmount_vendor_err=Failed to unmount Vendor");
			return -1;
		}
	}
   }

   // DJ9, 20200622: try to avoid a situation where blockimg will bomb out when trying to create a stash
   if (TWFunc::Path_Exists("/cache/.") && !TWFunc::Path_Exists("/cache/recovery/.") && !TWFunc::Path_Exists("/data/cache/.")) {
	LOGINFO("Recreating the /cache/recovery/ folder ...\n");
	if (!TWFunc::Recursive_Mkdir("/cache/recovery", false))
	   LOGERR("Could not create /cache/recovery - blockimg may have problems with creating stashes\n");
   }
   // DJ9

  time_t start, stop;
  time(&start);

  std::string update_binary_name(UPDATE_BINARY_NAME);
  ZipEntry64 update_binary_entry;
  if (FindEntry(Zip, update_binary_name, &update_binary_entry) == 0) {
		LOGINFO("Update binary zip\n");
		// Additionally verify the compatibility of the package.
		if (!Fox_Skip_Treble_Compatibility_Check() && !verify_package_compatibility(Zip)) {
			gui_err("zip_compatible_err=Zip Treble compatibility error!");
			ret_val = INSTALL_CORRUPT;
		} else {
			ret_val = Prepare_Update_Binary(path, Zip);
			if (ret_val == INSTALL_SUCCESS) {
				usleep(32);
				run_rom_scripts = ((DataManager::GetIntValue(FOX_ZIP_INSTALLER_CODE) != 0) // only run after flashing a ROM
	  			&& (DataManager::GetIntValue(FOX_INSTALL_PREBUILT_ZIP) != 1)); // don't run for built-in zips

				if (run_rom_scripts && TWFunc::Path_Exists(FOX_PRE_ROM_FLASH_SCRIPT)) {
					TWFunc::RunFoxScript(FOX_PRE_ROM_FLASH_SCRIPT, path);
	  			}

				ret_val = Run_Update_Binary(path, wipe_cache, UPDATE_BINARY_ZIP_TYPE);

				if (DataManager::GetIntValue("fox_processing_asserts") != 0) {
					TWFunc::Fox_Property_Set("ro.product.device", DataManager::GetStrValue("fox_product_device"));
					DataManager::SetValue("fox_processing_asserts", "0");
					LOGINFO("\nDevice code name restored\n");
				}
			}
		}
	} else {
		std::string ab_binary_name(AB_OTA);
		ZipEntry64 ab_binary_entry;
		if (FindEntry(Zip, ab_binary_name, &ab_binary_entry) == 0) {
			LOGINFO("AB zip\n");
			// Fox high-speed branch (outer layer): the toggle + payload
			// probe run inside; kNotApplicable = stock path below, untouched.
			FoxHighSpeedResult hs_res = FoxTryHighSpeedInstall(path);
			if (hs_res == FoxHighSpeedResult::kSucceeded) {
				DataManager::SetValue(FOX_ZIP_INSTALLER_CODE, 1); // mark as custom ROM install
				run_rom_scripts = true;
				ret_val = INSTALL_SUCCESS;
			} else if (hs_res == FoxHighSpeedResult::kAborted) {
				ret_val = INSTALL_ERROR;
		} else {
			// 0. reconcile with bootctrl: an external runtime switcher
			// (DFE NEO-style zip, manual bootctl) may have moved the
			// active slot behind TWRP's back.
			{
				std::string boot_active = PartitionManager.Get_Bootctrl_Active_Slot();
				if (!boot_active.empty() && boot_active != PartitionManager.Get_Active_Slot_Display()) {
					LOGINFO("stock: bootctrl says %s, session shows %s; reconciling\n",
						boot_active.c_str(), PartitionManager.Get_Active_Slot_Display().c_str());
					if (!PartitionManager.Live_Switch_Slot(boot_active)) {
						gui_err("switch_slot_fail=Slot switch failed. See the log for details.");
						return INSTALL_ERROR;
					}
				}
			}
			// 1. live-slot guard turned normalizer: never flash the physically
			// booted slot. Booted is the snapshot truth (/tmp/fox_booted_slot),
			// so a stale or spoofed session cannot trick this branch. On trip,
			// switch back to the booted slot and continue — the ROM always
			// lands on the real opposite slot, no reboot required. Only an
			// unreadable booted slot or a failed switch aborts (nothing
			// flashed yet, so an early return is safe).
			std::string stock_booted = PartitionManager.Get_Booted_Slot_Display();
			if (stock_booted.empty()) {
				gui_err("no_booted_slot=Cannot read the booted slot. Refusing to flash.");
				return INSTALL_ERROR;
			}
			std::string stock_target = (PartitionManager.Get_Active_Slot_Display()=="A")?"b":"a";
			if ((stock_target == "b") == (stock_booted == "B")) {
				gui_warn("slot_live_reswitch=Target is the live booted slot; switching back to it and continuing.");
				std::string stock_back(1, stock_booted[0]);
				if (!PartitionManager.Live_Switch_Slot(stock_back)) {
					gui_err("switch_slot_fail=Slot switch failed. See the log for details.");
					return INSTALL_ERROR;
				}
				stock_target = (PartitionManager.Get_Active_Slot_Display()=="A")?"b":"a";
			}
			gui_msg(Msg(msg::kHighlight, "flash_ab_inactive=Flashing A/B zip to inactive slot: {1}")(stock_target=="a"?"A":"B"));
			// We need this so backuptool can do its magic
			bool system_mount_state = PartitionManager.Is_Mounted_By_Path(PartitionManager.Get_Android_Root_Path());
			bool vendor_mount_state = PartitionManager.Is_Mounted_By_Path("/vendor");
			PartitionManager.Mount_By_Path(PartitionManager.Get_Android_Root_Path(), false);
			PartitionManager.Mount_By_Path("/vendor", false);
			TWFunc::copy_file("/system/bin/sh", "/tmp/sh", 0755);
			mount("/tmp/sh", "/system/bin/sh", "auto", MS_BIND, NULL);

			run_rom_scripts = true;
			usleep(32);

			if (run_rom_scripts && TWFunc::Path_Exists(FOX_PRE_ROM_FLASH_SCRIPT)) {
				TWFunc::RunFoxScript(FOX_PRE_ROM_FLASH_SCRIPT, path);
			}

			TWFunc::IsRecoveryOverwritten(true);

			ret_val = Run_Update_Binary(path, wipe_cache, AB_OTA_ZIP_TYPE);

			DataManager::SetValue(FOX_ZIP_INSTALLER_CODE, 1); // mark as custom ROM install

			umount("/system/bin/sh");
			unlink("/tmp/sh");
			if (!vendor_mount_state)
				PartitionManager.UnMount_By_Path("/vendor", false);
			if (!system_mount_state)
				PartitionManager.UnMount_By_Path(PartitionManager.Get_Android_Root_Path(), false);
			std::string stock_up = (stock_target == "b") ? "B" : "A";
			if (DataManager::GetIntValue("tw_live_switch") != 0) {
				if (!PartitionManager.Live_Switch_Slot(stock_up)) {
					gui_err("switch_slot_fail=Slot switch failed. See the log for details.");
					ret_val = INSTALL_ERROR;
				} else {
					gui_msg("flash_ab_live_done=Slot switched live; you can flash additional zips without reboot.");
					if (android::base::GetBoolProperty("ro.virtual_ab.enabled", false)) {
						gui_warn("mount_vab_partitions=Devices on super may not mount until after rebooting recovery.");
					}
				}
			} else {
				gui_warn("flash_ab_reboot=To flash additional zips, please reboot recovery to switch to the updated slot.");
			}
		}
		} else {
			std::string binary_name("ui.xml");
			ZipEntry64 binary_entry;
			if (FindEntry(Zip, binary_name, &binary_entry) == 0) {
				LOGINFO("OrangeFox theme zip\n");
				ret_val = Install_Theme(path, Zip);
			} else {
				ret_val = INSTALL_CORRUPT;
			}
		}
   }

  time(&stop);
  int total_time = (int) difftime(stop, start);

  if (ret_val == INSTALL_CORRUPT)
    {
        set_miui_install_status(OTA_CORRUPT, true);
        gui_err("invalid_zip_format=Invalid zip file format!");
    }
  else
  if (ret_val == INSTALL_ERROR)
     {
	set_miui_install_status(OTA_ERROR, false);
     }
  else // success - so let us see whether we need to run OTA_BAK
  {
     // if MIUI-specific features have been disabled
     if (Fox_Skip_OTA()) // yes
     {
         //LOGINFO("OrangeFox: not running the incremental OTA backup (OTA_BAK).\n");
     }  
     else // else let us proceed with the OTA stuff
     if (DataManager::GetIntValue(FOX_INCREMENTAL_OTA_FAIL) != 1)
     {
      	if (DataManager::GetIntValue(FOX_INCREMENTAL_PACKAGE) == 1 && DataManager::GetIntValue(FOX_ZIP_INSTALLER_CODE) != 0)
      	  {
      	    if (TWinstall_Run_OTA_BAK (true)) // true, because the value of Fox_Zip_Installer_Code to be set
      	      {
	        if (Fox_OTA_Backup_Stock_Boot_Image()) // whether to create an additional backup of the stock boot image
	           {
	      		usleep(2048);
	      		string ota_folder = DataManager::GetStrValue("ota_bak_folder");
	      		usleep(2048);
	      		if (ota_folder.empty())
	      		   ota_folder = FOX_OTA_PATH;
			string ota_bootimg = ota_folder + "/boot.img";
			if (TWFunc::Path_Exists(boot_bak_img)) {
			   if (TWFunc::copy_file(boot_bak_img, ota_bootimg, 0644) == 0) {
			   	LOGINFO("OrangeFox: stock boot image extracted into the OTA directory.\n");
			     }
			   unlink(boot_bak_img.c_str());
		 	}
	           }
      	      }
      	  }

      	DataManager::SetValue(FOX_METADATA_PRE_BUILD, 0);
      	DataManager::SetValue(FOX_MIUI_ZIP_TMP, 0);
      	DataManager::SetValue(FOX_INCREMENTAL_OTA_FAIL, 0);
      	DataManager::SetValue(FOX_LOADED_FINGERPRINT, 0);
      	DataManager::SetValue(FOX_RUN_SURVIVAL_BACKUP, 0);
      
     } // end of OTA stuff
    LOGINFO("Install took %i second(s).\n", total_time);
   }

   if (ret_val == INSTALL_SUCCESS)
      set_miui_install_status(OTA_SUCCESS, false);

   usleep(32);
   if (DataManager::GetIntValue(FOX_ZIP_INSTALLER_CODE) != 0) // just flashed a ROM
   {
      usleep(16);
      TWFunc::Check_OrangeFox_Overwrite_FromROM(false, path);
   }

   if (run_rom_scripts && TWFunc::Path_Exists(FOX_POST_ROM_FLASH_SCRIPT)) {
   	usleep(2048);
   	TWFunc::RunFoxScript(FOX_POST_ROM_FLASH_SCRIPT, path);
   	sleep(1);
   	DataManager::SetValue("found_fox_overwriting_rom", "0");
   	TWFunc::Fox_Property_Set("found_fox_overwriting_rom", "");
   }

  return ret_val;
}
