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
			LOGINFO("DE keys reinstalled\n");
			// Auto-decrypt CE using cached password from previous decrypt session
			string cached_pw;
			DataManager::GetValue("tw_fbe_cached_password", cached_pw);
			if (!cached_pw.empty()) {
				LOGINFO("Auto-reinstalling CE keys with cached credentials\n");
				if (PartitionManager.Decrypt_Device(cached_pw, 0) == 0) {
					LOGINFO("FBE auto-decrypt CE succeeded after remount\n");
				} else {
					LOGINFO("FBE auto-decrypt CE failed, clearing cached password\n");
					DataManager::SetValue("tw_fbe_cached_password", "");
				}
			} else {
				LOGINFO("No cached password, CE decrypt required\n");
			}
		} else {
			LOGINFO("Failed to reinstall DE keys after remount\n");
		}
		DataManager::SetValue("tw_fbe_rekey_needed", 0);
	}
#endif

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
            ),
            (
                # Исправление размонтирования /sdcard: unmount всех слоёв bind mount
                # /sdcard монтируется дважды (Mount→Bind_Mount и Post_Decrypt→Bind_Mount),
                # а также убираем sub-mount'ы под /data, мешающие чистому unmount
                r"""		if (!Symlink_Mount_Point.empty())
			umount2(Symlink_Mount_Point.c_str(), flags);""",
                r"""		if (!Symlink_Mount_Point.empty()) {
			// Unmount all stacked bind mount layers (/sdcard may be bind-mounted
			// multiple times: from Mount and from Post_Decrypt)
			while (umount2(Symlink_Mount_Point.c_str(), flags) == 0);
		}

		// Unmount sub-mounts under /data that block clean unmount
		// (e.g. /data/user/0 bind mount from fscrypt_init_user0)
		if (Mount_Point == "/data") {
			umount2("/data/user/0", flags);
			umount2("/data/media/0", flags);
		}"""
            ),
            (
                # Блок 3: FBE decrypt fix — default cipher names for Android 14+ vendor fstabs
                # Vendor fstab.gs201 uses shorthand "fileencryption=::inlinecrypt_optimized+wrappedkey_v0"
                # (cipher names omitted). AOSP fs_mgr fills defaults aes-256-xts / aes-256-cts, but
                # OrangeFox parser doesn't — additional.fstab overwrites fbe.contents with empty string,
                # causing fscrypt_init_user0 to fail (can't determine cipher).
                r"""
		case TWFLAG_FILEENCRYPTION:
			// This flag isn't used by TWRP but is needed in 9.0 FBE decrypt
			// fileencryption=ice:aes-256-heh
			{
				std::string FBE = str;
				size_t colon_loc = FBE.find(":");
				if (colon_loc == std::string::npos) {
					property_set("fbe.contents", FBE.c_str());
					property_set("fbe.filenames", "");
					LOGINFO("FBE contents '%s', filenames ''\n", FBE.c_str());
					break;
				}
				std::string FBE_contents, FBE_filenames;
				FBE_contents = FBE.substr(0, colon_loc);
				FBE_filenames = FBE.substr(colon_loc + 1);
				property_set("fbe.contents", FBE_contents.c_str());
				property_set("fbe.filenames", FBE_filenames.c_str());
				LOGINFO("FBE contents '%s', filenames '%s'\n", FBE_contents.c_str(), FBE_filenames.c_str());
			}
			break;
		case TWFLAG_METADATA_ENCRYPTION:
			// This flag isn't used by TWRP but is needed for FBEv2 metadata decryption
			// metadata_encryption=aes-256-xts:wrappedkey_v0
			{
				std::string META = str;
				size_t colon_loc = META.find(":");
				if (colon_loc == std::string::npos) {
					property_set("metadata.contents", META.c_str());
					property_set("metadata.filenames", "");
					LOGINFO("Metadata contents '%s', filenames ''\n", META.c_str());
					break;
				}
				std::string META_contents, META_filenames;
				META_contents = META.substr(0, colon_loc);
				META_filenames = META.substr(colon_loc + 1);
				property_set("metadata.contents", META_contents.c_str());
				property_set("metadata.filenames", META_filenames.c_str());
				LOGINFO("Metadata contents '%s', filenames '%s'\n", META_contents.c_str(), META_filenames.c_str());
			}
			break;
""",
                r"""
		case TWFLAG_FILEENCRYPTION:
			// fileencryption=contents:filenames:flags
			// Android 14+ vendor fstabs may omit cipher names (e.g. "::inlinecrypt_optimized+wrappedkey_v0").
			// AOSP fs_mgr defaults: contents = aes-256-xts, filenames = aes-256-cts.
			{
				std::string FBE = str;
				size_t colon_loc = FBE.find(":");
				if (colon_loc == std::string::npos) {
					if (FBE.empty()) FBE = "aes-256-xts";
					property_set("fbe.contents", FBE.c_str());
					property_set("fbe.filenames", "");
					LOGINFO("FBE contents '%s', filenames ''\n", FBE.c_str());
					break;
				}
				std::string FBE_contents, FBE_filenames;
				FBE_contents = FBE.substr(0, colon_loc);
				FBE_filenames = FBE.substr(colon_loc + 1);
				if (FBE_contents.empty()) FBE_contents = "aes-256-xts";
				// filenames field may itself contain ":flags" — default cipher when empty
				size_t fn_colon = FBE_filenames.find(":");
				if (fn_colon != std::string::npos) {
					std::string fn_cipher = FBE_filenames.substr(0, fn_colon);
					if (fn_cipher.empty()) fn_cipher = "aes-256-cts";
					FBE_filenames = fn_cipher + FBE_filenames.substr(fn_colon);
				} else if (FBE_filenames.empty()) {
					FBE_filenames = "aes-256-cts";
				}
				property_set("fbe.contents", FBE_contents.c_str());
				property_set("fbe.filenames", FBE_filenames.c_str());
				LOGINFO("FBE contents '%s', filenames '%s'\n", FBE_contents.c_str(), FBE_filenames.c_str());
			}
			break;
		case TWFLAG_METADATA_ENCRYPTION:
			// metadata_encryption=contents:flags
			// Vendor fstabs may omit cipher name (e.g. ":wrappedkey_v0").
			// AOSP default: aes-256-xts.
			{
				std::string META = str;
				size_t colon_loc = META.find(":");
				if (colon_loc == std::string::npos) {
					if (META.empty()) META = "aes-256-xts";
					property_set("metadata.contents", META.c_str());
					property_set("metadata.filenames", "");
					LOGINFO("Metadata contents '%s', filenames ''\n", META.c_str());
					break;
				}
				std::string META_contents, META_filenames;
				META_contents = META.substr(0, colon_loc);
				META_filenames = META.substr(colon_loc + 1);
				if (META_contents.empty()) META_contents = "aes-256-xts";
				property_set("metadata.contents", META_contents.c_str());
				property_set("metadata.filenames", META_filenames.c_str());
				LOGINFO("Metadata contents '%s', filenames '%s'\n", META_contents.c_str(), META_filenames.c_str());
			}
			break;
"""
            ),
            (
                # Исправление дублирования bind mount /sdcard при remount
                # /sdcard монтируется дважды: Mount→Bind_Mount и Post_Decrypt→Bind_Mount
                r"""bool TWPartition::Bind_Mount(bool Display_Error) {
	if (TWFunc::Path_Exists(Symlink_Path)) {
		if (mount(Symlink_Path.c_str(), Symlink_Mount_Point.c_str(), "", MS_BIND, NULL) < 0) {
			return false;
		}
	}
	return true;
}""",
                r"""bool TWPartition::Bind_Mount(bool Display_Error) {
	if (TWFunc::Path_Exists(Symlink_Path)) {
		// Skip if already bind-mounted (prevents duplicate layers after
		// remount+auto-decrypt: Mount→Bind_Mount, then Post_Decrypt→Bind_Mount)
		if (TWFunc::Path_Exists(Symlink_Mount_Point)) {
			struct stat src_st, dst_st;
			if (stat(Symlink_Path.c_str(), &src_st) == 0 &&
				stat(Symlink_Mount_Point.c_str(), &dst_st) == 0 &&
				src_st.st_dev == dst_st.st_dev && src_st.st_ino == dst_st.st_ino) {
				return true; // already bind-mounted
			}
		}
		if (mount(Symlink_Path.c_str(), Symlink_Mount_Point.c_str(), "", MS_BIND, NULL) < 0) {
			return false;
		}
	}
	return true;
}"""
            )
        ]
