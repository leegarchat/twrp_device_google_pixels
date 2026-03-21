from patch import BaseSubPatch, Colors

class SubPatch(BaseSubPatch):
    def __init__(self, manager):
        super().__init__(manager)
        self.name = "Custom partition manager logic (partitionmanager.cpp)"
        self.target_file = "bootable/recovery/partitionmanager.cpp" # bootable/recovery/gui/gui.cpp

        # Список изменений: [(Оригинал, Модификация), ...]
        self.CHANGES = [
            (
                # Блок 1: Оригинальный код для поиска
                r"""
extern bool datamedia;
std::vector<users_struct> Users_List;

std::string additional_fstab = "/etc/additional.fstab";

TWPartitionManager::TWPartitionManager(void) {
	mtp_was_enabled = false;
	mtp_write_fd = -1;
	uevent_pfd.fd = -1;
	stop_backup.set_value(0);
#ifdef AB_OTA_UPDATER
	char slot_suffix[PROPERTY_VALUE_MAX];
""",
                # Блок 1: Модифицированный код (результат)
                r"""
extern bool datamedia;
std::vector<users_struct> Users_List;

std::string additional_fstab = "/etc/additional.fstab";

static bool FscryptMountMetadataEncryptedWithTimeout(
	const std::string& blk_device,
	const std::string& mount_point,
	const std::string& fs_type,
	const std::string& extra_fstab,
	int timeout_seconds) {
	pid_t pid = fork();
	if (pid < 0) {
		LOGERR("Metadata decrypt: failed to fork helper process, errno=%d\n", errno);
		return false;
	}

	if (pid == 0) {
		bool ok = android::vold::fscrypt_mount_metadata_encrypted(
			blk_device,
			mount_point,
			false,
			false,
			fs_type,
			"",
			extra_fstab);
		_exit(ok ? 0 : 1);
	}

	int status = 0;
	if (TWFunc::Wait_For_Child_Timeout(pid, &status, "MetadataDecrypt", timeout_seconds) != 0) {
		LOGERR("Metadata decrypt timed out after %d seconds\n", timeout_seconds);
		return false;
	}

	if (!WIFEXITED(status)) {
		LOGERR("Metadata decrypt: helper exited unexpectedly (status=%d)\n", status);
		return false;
	}

	if (WEXITSTATUS(status) != 0) {
		LOGINFO("Metadata decrypt helper reported failure\n");
		return false;
	}

	return true;
}

TWPartitionManager::TWPartitionManager(void) {
	mtp_was_enabled = false;
	mtp_write_fd = -1;
	uevent_pfd.fd = -1;
	stop_backup.set_value(0);
#ifdef AB_OTA_UPDATER
	char slot_suffix[PROPERTY_VALUE_MAX];
"""
            ),
            (
                # Блок 1: Оригинальный код для поиска
                r"""
static inline std::string KM_Ver_From_Manifest(std::string ver) {
	TWFunc::Get_Service_From_Manifest("/vendor", "android.hardware.keymaster", ver);
	#ifdef OF_NO_KEYMASTER_VER_4X
	LOGINFO("Keymaster_Ver::OF_NO_KEYMASTER_VER_4X is enabled; keymaster_ver will be set to '%s'\n", ver.c_str());
	#else
	if (strstr(ver.c_str(), "4")) {
		ver = "4.x";
	}
	#endif
	return ver;
}

void inline Process_Keymaster_Version(TWPartition *ven, bool Display_Error) {
	// Fetch the Keymaster Service version to be started
	std::string version;
#ifndef TW_FORCE_KEYMASTER_VER
	version = KM_Ver_From_Manifest(version);
""",
                # Блок 1: Модифицированный код (результат)
                r"""
static inline std::string KM_Ver_From_Manifest(std::string ver) {
	TWFunc::Get_Service_From_Manifest("/vendor", "android.hardware.keymaster", ver);
	#ifdef OF_NO_KEYMASTER_VER_4X
	LOGINFO("Keymaster_Ver::OF_NO_KEYMASTER_VER_4X is enabled; keymaster_ver will be set to '%s'\n", ver.c_str());
	#else
	if (strstr(ver.c_str(), "4")) {
		ver = "4.x";
	}
	#endif
	return ver;
}
void inline Process_Keymaster_Version(TWPartition *ven, bool Display_Error) {

	LOGINFO("Keymaster_Ver::KeyMint Start custom code'\n");

	std::string product_device = android::base::GetProperty("ro.product.device", "");
    if (ven) ven->UnMount(Display_Error);
    LOGINFO("Keymaster_Ver::Tensor zuma-family device detected (%s); forcing empty keymaster_ver to use AIDL KeyMint path\n", product_device.c_str());
    android::base::SetProperty(TW_KEYMASTER_VERSION_PROP, "");
    return;


	std::string keymint_version;
	TWFunc::Get_Service_From_Manifest("/vendor", "android.hardware.security.keymint", keymint_version);
	if (keymint_version.empty()) {
		TWFunc::Get_Service_From_Manifest("", "android.hardware.security.keymint", keymint_version);
	}
	LOGINFO("Keymaster_Ver::KeyMint manifest probe result='%s'\n", keymint_version.c_str());
	if (!keymint_version.empty()) {
		if (ven) ven->UnMount(Display_Error);
		LOGINFO("Keymaster_Ver::KeyMint AIDL detected in vendor manifest (version '%s'); skipping legacy keymaster_ver override\n", keymint_version.c_str());
		android::base::SetProperty(TW_KEYMASTER_VERSION_PROP, "");
		return;
	}
	LOGINFO("Keymaster_Ver::KeyMint End custom code'\n");
    

	// Fetch the Keymaster Service version to be started
	std::string version;
#ifndef TW_FORCE_KEYMASTER_VER
	version = KM_Ver_From_Manifest(version);
"""
            ),
            (
                # Блок 1: Оригинальный код для поиска
                r"""
void TWPartitionManager::Decrypt_Data() {
	#ifdef TW_INCLUDE_CRYPTO
	TWPartition* Decrypt_Data = Find_Partition_By_Path("/data");
	if (Decrypt_Data && Decrypt_Data->Is_Encrypted && !Decrypt_Data->Is_Decrypted) {
		Set_Crypto_State();
		TWPartition* Key_Directory_Partition = Find_Partition_By_Path(Decrypt_Data->Key_Directory);
		if (Key_Directory_Partition != nullptr)
			if (!Key_Directory_Partition->Is_Mounted())
				Mount_By_Path(Decrypt_Data->Key_Directory, false);
		if (!Decrypt_Data->Key_Directory.empty()) {
			Set_Crypto_Type("file");
#ifdef TW_INCLUDE_FBE_METADATA_DECRYPT
#ifdef USE_FSCRYPT
			if (android::vold::fscrypt_mount_metadata_encrypted(Decrypt_Data->Actual_Block_Device, Decrypt_Data->Mount_Point, false, false, Decrypt_Data->Current_File_System, "", TWFunc::Path_Exists(additional_fstab) ? additional_fstab : "")) {
				std::string crypto_blkdev = android::base::GetProperty("ro.crypto.fs_crypto_blkdev", "error");
				Decrypt_Data->Decrypted_Block_Device = crypto_blkdev;
				LOGINFO("Successfully decrypted metadata encrypted data partition with new block device: '%s'\n", crypto_blkdev.c_str());
#endif
				Decrypt_Data->Is_Decrypted = true; // Needed to make the mount function work correctly
				int retry_count = 10;
				while (!Decrypt_Data->Mount(false) && --retry_count)
					usleep(500);
				if (Decrypt_Data->Mount(false)) {
					if (!Decrypt_Data->Decrypt_FBE_DE()) {
						LOGERR("Unable to decrypt FBE device\n");
					}

				} else {
					LOGINFO("Failed to mount data after metadata decrypt\n");
				}
			} else {
				LOGINFO("Unable to decrypt metadata encryption\n");
			}
#else
			LOGERR("Metadata FBE decrypt support not present in this build\n");
#endif
		}
		if (Decrypt_Data->Is_FBE) {
""",
                # Блок 1: Модифицированный код (результат)
                r"""
void TWPartitionManager::Decrypt_Data() {
	#ifdef TW_INCLUDE_CRYPTO
	TWPartition* Decrypt_Data = Find_Partition_By_Path("/data");
	if (Decrypt_Data && Decrypt_Data->Is_Encrypted && !Decrypt_Data->Is_Decrypted) {
		Set_Crypto_State();
		TWPartition* Key_Directory_Partition = Find_Partition_By_Path(Decrypt_Data->Key_Directory);
		if (Key_Directory_Partition != nullptr)
			if (!Key_Directory_Partition->Is_Mounted())
				Mount_By_Path(Decrypt_Data->Key_Directory, false);
		if (!Decrypt_Data->Key_Directory.empty()) {
			Set_Crypto_Type("file");
			if (Decrypt_Data->Mount_Point == "/data" && !Decrypt_Data->Fstab_File_System.empty() && Decrypt_Data->Current_File_System != Decrypt_Data->Fstab_File_System) {
				LOGINFO("Metadata decrypt: overriding /data fs type from '%s' to fstab '%s'\n", Decrypt_Data->Current_File_System.c_str(), Decrypt_Data->Fstab_File_System.c_str());
				Decrypt_Data->Current_File_System = Decrypt_Data->Fstab_File_System;
			}
#ifdef TW_INCLUDE_FBE_METADATA_DECRYPT
#ifdef USE_FSCRYPT
			std::string keymint_state = android::base::GetProperty("init.svc.vendor.keymint.rust-trusty", "");
			if (!keymint_state.empty() && keymint_state != "running") {
				LOGINFO("Skipping metadata decrypt: keymint service state is '%s'\n", keymint_state.c_str());
			} else if (FscryptMountMetadataEncryptedWithTimeout(
				Decrypt_Data->Actual_Block_Device,
				Decrypt_Data->Mount_Point,
				Decrypt_Data->Current_File_System,
				TWFunc::Path_Exists(additional_fstab) ? additional_fstab : "",
				30)) {
				std::string crypto_blkdev = android::base::GetProperty("ro.crypto.fs_crypto_blkdev", "error");
				Decrypt_Data->Decrypted_Block_Device = crypto_blkdev;
				LOGINFO("Successfully decrypted metadata encrypted data partition with new block device: '%s'\n", crypto_blkdev.c_str());
#endif
				Decrypt_Data->Is_Decrypted = true; // Needed to make the mount function work correctly
				int retry_count = 10;
				while (!Decrypt_Data->Mount(false) && --retry_count)
					usleep(500);
				if (Decrypt_Data->Mount(false)) {
					// Metadata decrypt mounts /data via fs_mgr_do_mount() in a forked
					// child, bypassing TWPartition::Mount() and Check_FS_Type().
					// Current_File_System is still "ext4" (encrypted block unprobed).
					// Re-probe the now-decrypted block device to detect f2fs.
					Decrypt_Data->Check_FS_Type();
					// f2fs checkpoint commit: without explicit commit, f2fs may still
					// have checkpoint disabled (from a prior OTA), causing all recovery
					// writes to be rolled back.
					if (Decrypt_Data->Current_File_System == "f2fs") {
						string cp_opts = Decrypt_Data->Mount_Options;
						size_t cpos = cp_opts.find("checkpoint_merge");
						if (cpos != string::npos) {
							size_t cend = cpos + strlen("checkpoint_merge");
							if (cend < cp_opts.size() && cp_opts[cend] == ',') cend++;
							else if (cpos > 0 && cp_opts[cpos-1] == ',') cpos--;
							cp_opts.erase(cpos, cend - cpos);
						}
						if (!cp_opts.empty()) cp_opts += ",";
						cp_opts += "checkpoint=enable";
						if (mount(Decrypt_Data->Decrypted_Block_Device.c_str(),
								  Decrypt_Data->Mount_Point.c_str(), "f2fs",
								  MS_REMOUNT | Decrypt_Data->Mount_Flags,
								  cp_opts.c_str()) == 0) {
							LOGINFO("f2fs checkpoint committed on /data (post metadata decrypt)\n");
						} else {
							LOGINFO("f2fs checkpoint commit failed on /data: %s\n", strerror(errno));
						}
					}
					if (!Decrypt_Data->Decrypt_FBE_DE()) {
						LOGERR("Unable to decrypt FBE device\n");
					}
					if (DataManager::GetIntValue("tw_mtp_enabled") != 0) {
						LOGINFO("Restarting MTP after metadata decrypt\n");
						Disable_MTP();
						usleep(500000);
						if (!Enable_MTP())
							Disable_MTP();
					}

				} else {
					LOGINFO("Failed to mount data after metadata decrypt\n");
				}
			} else {
				LOGINFO("Unable to decrypt metadata encryption\n");
			}
#else
			LOGERR("Metadata FBE decrypt support not present in this build\n");
#endif
		}
		if (Decrypt_Data->Is_FBE) {
"""
            ),
            (
                # Блок 1: Оригинальный код для поиска
                r"""
		DataManager::SetValue(TW_IS_DECRYPTED, 1);
		dat->Is_Decrypted = true;
		if (!Block_Device.empty()) {
			dat->Decrypted_Block_Device = Block_Device;
			gui_msg(Msg("decrypt_success_dev=Data successfully decrypted, new block device: '{1}'")(Block_Device));
		} else {
			gui_msg("decrypt_success_nodev=Data successfully decrypted");
		}
		property_set("twrp.decrypt.done", "true");
		dat->Setup_File_System(false);
		dat->Current_File_System = dat->Fstab_File_System;  // Needed if we're ignoring blkid because encrypted devices start out as emmc

		sleep(1); // Sleep for a bit so that the device will be ready

		// Mount only /data
		dat->Symlink_Path = ""; // Not to let it to bind mount /data/media again
		if (!dat->Mount(false)) {
			LOGERR("Unable to mount /data after decryption");
		}
""",
                # Блок 1: Модифицированный код (результат)
                r"""
		DataManager::SetValue(TW_IS_DECRYPTED, 1);
		dat->Is_Decrypted = true;
		if (!Block_Device.empty()) {
			dat->Decrypted_Block_Device = Block_Device;
			gui_msg(Msg("decrypt_success_dev=Data successfully decrypted, new block device: '{1}'")(Block_Device));
		} else {
			gui_msg("decrypt_success_nodev=Data successfully decrypted");
		}
		property_set("twrp.decrypt.done", "true");
		dat->Setup_File_System(false);
		if (dat->Ignore_Blkid)
			dat->Current_File_System = dat->Fstab_File_System; // Needed on devices where blkid probing is intentionally disabled
		else
			dat->Check_FS_Type();

		sleep(1); // Sleep for a bit so that the device will be ready

		// Mount only /data
		dat->Symlink_Path = ""; // Not to let it to bind mount /data/media again
		if (!dat->Mount(false)) {
			LOGERR("Unable to mount /data after decryption");
		}
"""
            ),
            (
                # Блок 1: Оригинальный код для поиска
                r"""
	if (all_is_decrypted == 1) {
		LOGINFO("All found users are decrypted.\n");
		DataManager::SetValue("tw_all_users_decrypted", "1");
		property_set("twrp.all.users.decrypted", "true");
	} else
		DataManager::SetValue("tw_all_users_decrypted", "0");
#endif
}

int TWPartitionManager::Decrypt_Device(string Password, int user_id) {
#ifdef TW_INCLUDE_CRYPTO
  char crypto_blkdev[PROPERTY_VALUE_MAX];
  std::vector < TWPartition * >::iterator iter;

""",
                # Блок 1: Модифицированный код (результат)
                r"""
	if (all_is_decrypted == 1) {
		LOGINFO("All found users are decrypted.\n");
		DataManager::SetValue("tw_all_users_decrypted", "1");
		property_set("twrp.all.users.decrypted", "true");
	} else
		DataManager::SetValue("tw_all_users_decrypted", "0");
#endif
}

void TWPartitionManager::Reset_Users_Decryption_Status() {
#ifdef TW_INCLUDE_FBE
	std::vector<users_struct>::iterator iter;
	for (iter = Users_List.begin(); iter != Users_List.end(); iter++) {
		(*iter).isDecrypted = false;
		string user_prop_decrypted = "twrp.user." + (*iter).userId + ".decrypt";
		property_set(user_prop_decrypted.c_str(), "0");
	}
	DataManager::SetValue("tw_all_users_decrypted", "0");
	property_set("twrp.all.users.decrypted", "false");
	LOGINFO("Reset all users decryption status\n");
#endif
}

int TWPartitionManager::Decrypt_Device(string Password, int user_id) {
#ifdef TW_INCLUDE_CRYPTO
  char crypto_blkdev[PROPERTY_VALUE_MAX];
  std::vector < TWPartition * >::iterator iter;
"""
            ),
            (
                # Блок 1: Оригинальный код для поиска
                r"""
					int tmp_user_id = atoi((*iter).userId.c_str());
					gui_msg(Msg("decrypting_user_fbe=Attempting to decrypt FBE for user {1}...")(tmp_user_id));
					if (android::keystore::Decrypt_User(tmp_user_id, Password) ||
					(Password != "!" && android::keystore::Decrypt_User(tmp_user_id, "!"))) { // "!" means default password
						gui_msg(Msg("decrypt_user_success_fbe=User {1} Decrypted Successfully")(tmp_user_id));
						Mark_User_Decrypted(tmp_user_id);
					} else {
						gui_msg(Msg("decrypt_user_fail_fbe=Failed to decrypt user {1}")(tmp_user_id));
					}
				}
				Post_Decrypt("");
			}

			return 0;
		} else {
			gui_msg(Msg(msg::kError, "decrypt_user_fail_fbe=Failed to decrypt user {1}")(user_id));
		}
#else
      LOGERR("FBE support is not present\n");
#endif
		return -1;
	}
""",
                # Блок 1: Модифицированный код (результат)
                r"""
					int tmp_user_id = atoi((*iter).userId.c_str());
					gui_msg(Msg("decrypting_user_fbe=Attempting to decrypt FBE for user {1}...")(tmp_user_id));
					if (android::keystore::Decrypt_User(tmp_user_id, Password) ||
					(Password != "!" && android::keystore::Decrypt_User(tmp_user_id, "!"))) { // "!" means default password
						gui_msg(Msg("decrypt_user_success_fbe=User {1} Decrypted Successfully")(tmp_user_id));
						Mark_User_Decrypted(tmp_user_id);
					} else {
						gui_msg(Msg("decrypt_user_fail_fbe=Failed to decrypt user {1}")(tmp_user_id));
					}
				}
				Post_Decrypt("");
			}
			if (DataManager::GetIntValue("tw_mtp_enabled") != 0) {
				LOGINFO("Restarting MTP after user decrypt (user %d)\n", user_id);
				Disable_MTP();
				usleep(500000);
				if (!Enable_MTP())
					Disable_MTP();
			}

			return 0;
		} else {
			gui_msg(Msg(msg::kError, "decrypt_user_fail_fbe=Failed to decrypt user {1}")(user_id));
		}
#else
      LOGERR("FBE support is not present\n");
#endif
		return -1;
	}
"""
            )
        ]

