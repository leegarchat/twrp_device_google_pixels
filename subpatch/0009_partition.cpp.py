from patch import BaseSubPatch, Colors

class SubPatch(BaseSubPatch):
    def __init__(self, manager):
        super().__init__(manager)
        self.name = "Custom partition logic (partition.cpp)"
        self.target_file = "bootable/recovery/partition.cpp" # bootable/recovery/gui/gui.cpp

        # Список изменений: [(Оригинал, Модификация), ...]
        self.CHANGES = [
            (
                # Блок 1: Оригинальный код для поиска
                r"""
			else
				LOGINFO("Unable to mount '%s'\n", Mount_Point.c_str());
			LOGINFO("Actual block device: '%s', current file system: '%s'\n", Actual_Block_Device.c_str(), Current_File_System.c_str());
			return false;
#ifdef TW_NO_EXFAT_FUSE
		}
#endif
	}

exit:
	if (Removable)
		Update_Size(Display_Error);

	if (!Symlink_Mount_Point.empty()/* && Symlink_Mount_Point != "/sdcard"*/) {
		if (!Bind_Mount(false))
			return false;
	}
	return true;
}

bool TWPartition::Bind_Mount(bool Display_Error) {
	if (TWFunc::Path_Exists(Symlink_Path)) {
		if (mount(Symlink_Path.c_str(), Symlink_Mount_Point.c_str(), "", MS_BIND, NULL) < 0) {
			return false;
		}
	}
	return true;
}
""",
                # Блок 1: Модифицированный код (результат)
                r"""
			else
				LOGINFO("Unable to mount '%s'\n", Mount_Point.c_str());
			LOGINFO("Actual block device: '%s', current file system: '%s'\n", Actual_Block_Device.c_str(), Current_File_System.c_str());
			return false;
#ifdef TW_NO_EXFAT_FUSE
		}
#endif
	}

exit:
	// f2fs checkpoint commit: if /data was just mounted and is f2fs, force-commit
	// any pending checkpoint. Without this, f2fs may be in checkpoint=disable state
	// (set by init/vold before OTA), and ALL writes during recovery are treated as
	// part of an uncommitted checkpoint transaction — they get rolled back on next
	// normal boot when vold calls cp_abortChanges or the bootloader rolls back.
	// The fix: remount with checkpoint=enable, which tells f2fs to commit the
	// checkpoint and make all subsequent writes durable.
	if (Mount_Point == "/data" && Current_File_System == "f2fs") {
		std::string cp_opts = Mount_Options;
		// Remove checkpoint_merge if present, append checkpoint=enable
		size_t pos = cp_opts.find("checkpoint_merge");
		if (pos != std::string::npos) {
			size_t end = pos + strlen("checkpoint_merge");
			if (end < cp_opts.size() && cp_opts[end] == ',') end++;
			else if (pos > 0 && cp_opts[pos-1] == ',') pos--;
			cp_opts.erase(pos, end - pos);
		}
		if (!cp_opts.empty()) cp_opts += ",";
		cp_opts += "checkpoint=enable";
		if (mount(Actual_Block_Device.c_str(), Mount_Point.c_str(), "f2fs",
				  flags | MS_REMOUNT, cp_opts.c_str()) == 0) {
			LOGINFO("f2fs checkpoint committed on /data\n");
		} else {
			LOGINFO("f2fs checkpoint commit failed (%s), writes may not persist\n", strerror(errno));
		}
	}

	if (Removable)
		Update_Size(Display_Error);

	if (!Symlink_Mount_Point.empty()/* && Symlink_Mount_Point != "/sdcard"*/) {
		if (!Bind_Mount(false))
			return false;
	}

#ifdef TW_INCLUDE_FBE
	if (Mount_Point == "/data" && Is_FBE && DataManager::GetIntValue("tw_fbe_rekey_needed") == 1) {
		LOGINFO("Reinstalling DE keys after /data remount\n");
		int retry_count = 3;
		while (!android::keystore::Decrypt_DE() && --retry_count)
			usleep(2000);
		if (retry_count > 0) {
			LOGINFO("DE keys reinstalled, CE decrypt required\n");
		} else {
			LOGINFO("Failed to reinstall DE keys after remount\n");
		}
		DataManager::SetValue("tw_fbe_rekey_needed", 0);
	}
#endif

	return true;
}

bool TWPartition::Bind_Mount(bool Display_Error) {
	if (TWFunc::Path_Exists(Symlink_Path)) {
		if (mount(Symlink_Path.c_str(), Symlink_Mount_Point.c_str(), "", MS_BIND, NULL) < 0) {
			return false;
		}
	}
	return true;
}
"""
            ),
            (
                # Блок 1: Оригинальный код для поиска
                r"""
		if (Is_Mounted()) {
			if (Mount_Point == "/data" || Mount_Point == "/sdcard" || Mount_Point == "/data/media/0") {
				LOGINFO("DEBUG: attempting again to unmount '%s'\n", Mount_Point.c_str());
				TWFunc::Exec_Cmd("umount -l " + Mount_Point);
				sleep(1);
				if (!Is_Mounted())
					return true;
			}
			if (Display_Error)
				gui_msg(Msg(msg::kError, "fail_unmount=Failed to unmount '{1}' ({2})")(Mount_Point)(strerror(errno)));
			else
				LOGINFO("Unable to unmount '%s'\n", Mount_Point.c_str());
			return false;
		} else {
			return true;
		}
	} else {
		return true;
	}
}
""",
                # Блок 1: Модифицированный код (результат)
                r"""
		umount2(Mount_Point.c_str(), flags);
		if (Is_Mounted()) {
			if (Mount_Point == "/data" || Mount_Point == "/sdcard" || Mount_Point == "/data/media/0") {
				LOGINFO("DEBUG: attempting again to unmount '%s'\n", Mount_Point.c_str());
				TWFunc::Exec_Cmd("umount -l " + Mount_Point);
				sleep(1);
				if (!Is_Mounted())
					goto unmount_done;
			}
			if (Display_Error)
				gui_msg(Msg(msg::kError, "fail_unmount=Failed to unmount '{1}' ({2})")(Mount_Point)(strerror(errno)));
			else
				LOGINFO("Unable to unmount '%s'\n", Mount_Point.c_str());
			return false;
		} else {
unmount_done:
#ifdef TW_INCLUDE_FBE
			if (Mount_Point == "/data" && Is_FBE) {
				LOGINFO("Clearing FBE key caches after /data unmount\n");
				android::keystore::Reset_FBE_Caches();
				PartitionManager.Reset_Users_Decryption_Status();
				// Do NOT reset Is_Decrypted here: dm-default-key mapper
				// (/dev/block/mapper/userdata) stays active after unmount.
				// Clearing Is_Decrypted would force Find_Actual_Block_Device()
				// to use Primary_Block_Device (sda34) which is busy.
				DataManager::SetValue("tw_fbe_rekey_needed", 1);
			}
#endif
			return true;
		}
	} else {
		return true;
	}
}
"""
            )
        ]

