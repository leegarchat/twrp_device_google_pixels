/*
 * Copyright (C) 2007 The Android Open Source Project
 *
 * Copyright (C) 2018-2026 OrangeFox Recovery Project
 * This file is part of the OrangeFox Recovery Project.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *      http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 *
 */

#ifndef _VARIABLES_HEADER_
#define _VARIABLES_HEADER_

#define FOX_RECOVERY_NAME	"OrangeFox"
#define FOX_MAIN_VERSION_STR    FOX_INTERNAL_RELEASE // defined in orangefox.mk
#define FOX_BRANCH		"12"
#define TW_VERSION_STR 		FOX_MAIN_VERSION_STR
#define BUILD_TYPE_STR 		FOX_BUILD_TYPE

// OrangeFox - Values
#define FOX_DEVICE              FOX_DEVICE_MODEL
#define OF_MAINTAINER_STR	"of_maintainer"
#define OF_FLASHLIGHT_ENABLE_STR "of_flashlight_enable"

// fordownloads values
#define OF_SCREEN_H_S              "screen_original_h"
#define OF_SCREEN_NAV_H_S          "screen_h"
#define OF_CENTER_Y_S              "center_y"

#define OF_STATUS_H_S              "status_h"
#define OF_HIDE_NOTCH_S            "allow_hide_notch"
#define OF_STATUS_INDENT_LEFT_S    "status_indent_left"
#define OF_STATUS_INDENT_RIGHT_S   "status_indent_right"

#define OF_STATUS_PLACEMENT_S      "status_info_y"
#define OF_CLOCK_POS_S             "cutout_clock"

#define OF_ALLOW_DISABLE_NAVBAR_S  "allow_disable_nav"

#define OF_SPLASH_MAX_SIZE_STR "of_splash_max_size"

#define OF_STORAGE_PATH             "/sdcard/Fox"
#define TW_STORAGE_PATH             "/data/recovery"
#define TW_SETTINGS_FILE	    ".foxs"
#define PERSIST_SETTINGS_FILE       "/persist/.foxs"

// *** OrangeFox - Variables ** //
#include <stdio.h>
#include <stdlib.h>
#include <string>
#include <unistd.h>
static const std::string Fox_Bin_Dir = "/system/bin";
static const std::string Fox_Tmp = "/tmp";

static const std::string Fox_Home =
#ifdef FOX_MISCELLANEOUS_ROOT_DIRECTORY
FOX_MISCELLANEOUS_ROOT_DIRECTORY"/Fox";
#else
OF_STORAGE_PATH;
#endif

static const std::string Fox_Settings_Path =
#ifdef FOX_SETTINGS_ROOT_DIRECTORY
FOX_SETTINGS_ROOT_DIRECTORY"/Fox";
#else
OF_STORAGE_PATH;
#endif

static const std::string Fox_ResetProp_Bin = "/system/bin/resetprop";
static const std::string FOX_THEME_PATH = Fox_Settings_Path + "/.theme";
static const std::string FOX_NAVBAR_PATH =  Fox_Settings_Path + "/.navbar";
static const std::string Fox_Home_Files = Fox_Home + "/FoxFiles";
static const std::string Fox_Logs_Dir = Fox_Home + "/logs";
static const std::string FOX_OTA_PATH =  Fox_Home + "/OTA";
static const std::string Fox_sdcard_aroma_cfg = Fox_Home + "/aromafm.cfg";
static const std::string Fox_aroma_cfg = Fox_Home_Files + "/AromaFM/AromaFM.zip.cfg";
static const std::string FFiles_dir = "/FFiles";
static const std::string Fox_tmp_dir = Fox_Tmp + "/orangefox";
static const std::string Fox_ramdisk_dir = Fox_tmp_dir + "/ramdisk"; 
static const std::string Fox_ramdisk_sbin_dir = Fox_ramdisk_dir + "/sbin"; 
static const std::string epoch_drift_file = "/persist/.fox_epoch_drift.cfg"; // to cater for any saved epoch_drifts
static const std::string Fox_OTA_info = "/orangefox.info";
static std::string Fox_Current_Device = "default";
static const std::string orangefox_cfg = "/etc/orangefox.cfg";
static const std::string Fox_Cfg = "/etc/fox.cfg";

static int Fox_Zip_Installer_Code = 0; // 0=standard zip;1=custom ROM;2=miui ROM; 11=custom treble ROM; 22=miui treble ROM
static int Fox_IsDeactivation_Process_Called = 0; // have we called the deactivation process
static int Fox_AutoDeactivate_OnReboot = 0;   // call the deactivation process automatically on reboot (if not already called by another thread) ?
static int Fox_Force_Deactivate_Process = 0;  // for a call to Deactivate_Process()
static int Fox_Current_ROM_IsMIUI = 0; // is the currently installed ROM a MIUI ROM?

#define FOX_SURVIVAL_FOLDER		Fox_Home.c_str()
//#define FOX_UPDATE_BINARY		"META-INF/com/google/android/update-binary" // all zip installers must have this
#define FOX_MIUI_UPDATE_PATH 		"META-INF/com/miui/miui_update" 	// standard MIUI ROMs have this
#define FOX_MIUI_UPDATE_PATH_EU 	"META-INF/com/xiaomieu/xiaomieu.sh"  // Xiaomi.EU MIUI ROMs have this
#define FOX_FORCE_DEACTIVATE_PROCESS 	"fox_force_deactivate_process"
#define FOX_ZIP_INSTALLER_CODE 		"fox_zip_installer_code"
#define FOX_ZIP_INSTALLER_TREBLE 	"fox_zip_installer_treble"
#define FOX_DISABLE_OTA_AUTO_REBOOT 	"fox_disable_ota_auto_reboot_check"
#define FOX_STARTUP_SCRIPT 		"/sbin/foxstart.sh"
#define FOX_PS_BIN 			"/FFiles/ps"
#define FOX_NEW_MAGISKBOOT 		"/FFiles/magiskboot_new"
#define FOX_MAGISK_ZIP_INSTALLER 	"Magisk.zip"
#define FOX_MAGISK_UNINSTALLER 		"uninstall.zip"

#define FOX_THEME_VERSION 		"2.0"
#define FOX_PASS_IN_PERSIST 		"/persist/.fsec"
#define FOX_PRE_ROM_FLASH_SCRIPT  	"/system/bin/pre_rom_flash.sh"
#define FOX_POST_ROM_FLASH_SCRIPT 	"/system/bin/post_rom_flash.sh"
#define FOX_BEFORE_REBOOT_SCRIPT 	"/system/bin/beforereboot.sh"
#define FOX_AFTER_ROM_FLASH_SCRIPT	"/sbin/afterromflash.sh"
#define FOX_POST_DATA_FORMAT_SCRIPT 	"/sbin/postformatdata.sh"
#define TW_FORMAT_DATA_SCRIPT 		"/system/bin/formatdata.sh"

// **** //

#define FOX_SURVIVAL_FOLDER_VAR      	"fox_survival_backup_folder_path"
#define FOX_SURVIVAL_BACKUP_NAME       	"fox_survival_backup_folder_name"
#define FOX_SURVIVAL_BACKUP       	"OTA"
#define FOX_FILES_BACKUPS_FOLDER_VAR    "fox_files_backup_folder_var"
#define FOX_DISABLE_BOOT_CHK       	"fox_disable_boot_check"
#define FOX_DO_SYSTEM_ON_OTA       	"fox_include_system_survival"
#define FOX_INSTALL_PREBUILT_ZIP       	"fox_install_built_in_zip"
#define FOX_DONT_REPLACE_STOCK       	"fox_reboot_dont_disable_stock_recovery"
#define FOX_ACTUAL_BUILD_VAR            "fox_actual_build"
#define FOX_INCREMENTAL_PACKAGE         "fox_support_miui_ota"
#define FOX_ENABLE_SECURE_RO            "fox_reboot_enable_secure_ro"
#define FOX_DISABLE_SECURE_RO           "fox_reboot_disable_secure_ro"
#define FOX_ENABLE_ADB_RO             	"fox_reboot_enable_adb_ro"
#define FOX_DISABLE_ADB_RO             	"fox_reboot_disable_adb_ro"
#define FOX_ADVANCED_WARN_CHK           "fox_advanced_warning_checkbox"
#define FOX_DISABLE_MOCK_LOCATION       "fox_reboot_disable_mock_location"
#define FOX_ENABLE_MOCK_LOCATION        "fox_reboot_enable_mock_location"
#define FOX_DISABLE_SECURE_BOOT         "fox_reboot_disable_secure_boot"
#define FOX_ADVANCED_STOCK_REPLACE      "fox_reboot_advanced_stock_recovery_check"
#define FOX_SAVE_LOAD_AROMAFM           "fox_reboot_saveload_aromafm_check"
#define FOX_DISABLE_DEBUGGING           "fox_reboot_disable_debugging_check"
#define FOX_ENABLE_DEBUGGING           	"fox_reboot_forced_debugging_check"
#define FOX_DISABLE_FORCED_ENCRYPTION   "fox_reboot_forced_encryption_check"
#define FOX_DISABLE_DM_VERITY           "fox_reboot_dm_verity_check"
#define FOX_REBOOT_AFTER_RESTORE        "fox_reboot_after_restore"
#define FOX_COMPATIBILITY_DEVICE        "fox_compatibility_fox_device"
#define FOX_MAIN_SURVIVAL_TRIGGER       "fox_main_survival_trigger"
#define FOX_NO_OS_SEARCH_ENGINE         "fox_noos_engine"
#define FOX_TMP_SCRIPT_DIR       	"fox_tmp_script_directory"
#define FOX_STATUSBAR_ON_LOCK       	"fox_statusbar_on_lockpass"
#define FOX_LED_COLOR       		"fox_led_color"
#define FOX_BALANCE_CHECK       	"fox_boot_balance_check"
#define FOX_NOTIFY_AFTER_RESTORE       	"fox_inject_after_restore"
#define FOX_NOTIFY_AFTER_BACKUP       	"fox_inject_after_backup"
#define FOX_FLASHLIGHT_VAR     		"flashlight"
#define FOX_FSYNC_CHECK       		"fox_boot_fsync_check"
#define FOX_FORCE_FAST_CHARGE_CHECK     "fox_boot_fastcharge_check"
#define FOX_T2W_CHECK       		"fox_boot_t2w_check"
#define FOX_PERFORMANCE_CHECK       	"fox_boot_performance_check"
#define FOX_POWERSAVE_CHECK       	"fox_boot_powersave_check"
#define FOX_CALL_DEACTIVATION         	"fox_call_deactivation_process"
#define FOX_GOVERNOR_STABLE         	"governor_stable"

#define FOX_MIUI_ZIP_TMP                "fox_miui_zip_tmp"
#define FOX_LOADED_FINGERPRINT          "fox_loaded_signature"
#define FOX_MIN_EXPECTED_FP_SIZE 30

#define FOX_INCREMENTAL_OTA_FAIL        "fox_ota_fail"
#define FOX_RUN_SURVIVAL_BACKUP         "fox_run_survival_backup"
#define FOX_METADATA_PRE_BUILD          "fox_pre_build"
#define FOX_ENCRYPTED_DEVICE		"fox_encrypted_device"
#define FOX_MEDIA_RW			"media_rw:media_rw"
#define FOX_MEDIA_RW_DATA_FILE		"u:object_r:media_rw_data_file:s0"
//
#define TW_USE_COMPRESSION_VAR      	"tw_use_compression"
#define TW_FILENAME                 	"tw_filename"
#define TW_ZIP_INDEX                	"tw_zip_index"
#define TW_ZIP_QUEUE_COUNT       	"tw_zip_queue_count"

#define MAX_BACKUP_NAME_LEN 64
#define TW_BACKUP_TEXT              	"tw_backup_text"
#define TW_BACKUP_NAME		        "tw_backup_name"
#define TW_BACKUP_SYSTEM_VAR        	"tw_backup_system"
#define TW_BACKUP_DATA_VAR          	"tw_backup_data"
#define TW_BACKUP_BOOT_VAR          	"tw_backup_boot"
#define TW_BACKUP_RECOVERY_VAR      	"tw_backup_recovery"
#define TW_BACKUP_CACHE_VAR         	"tw_backup_cache"
#define TW_BACKUP_ANDSEC_VAR        	"tw_backup_andsec"
#define TW_BACKUP_SDEXT_VAR         	"tw_backup_sdext"
#define TW_BACKUP_AVG_IMG_RATE      	"tw_backup_avg_img_rate"
#define TW_BACKUP_AVG_FILE_RATE     	"tw_backup_avg_file_rate"
#define TW_BACKUP_AVG_FILE_COMP_RATE    "tw_backup_avg_file_comp_rate"
#define TW_BACKUP_SYSTEM_SIZE       	"tw_backup_system_size"
#define TW_BACKUP_DATA_SIZE         	"tw_backup_data_size"
#define TW_BACKUP_BOOT_SIZE         	"tw_backup_boot_size"
#define TW_BACKUP_RECOVERY_SIZE     	"tw_backup_recovery_size"
#define TW_BACKUP_CACHE_SIZE        	"tw_backup_cache_size"
#define TW_BACKUP_ANDSEC_SIZE       	"tw_backup_andsec_size"
#define TW_BACKUP_SDEXT_SIZE        	"tw_backup_sdext_size"
#define TW_STORAGE_FREE_SIZE        	"tw_storage_free_size"
#define TW_GENERATE_DIGEST_TEXT     	"tw_generate_digest_text"

#define TW_RESTORE_TEXT             "tw_restore_text"
#define TW_RESTORE_SYSTEM_VAR       "tw_restore_system"
#define TW_RESTORE_DATA_VAR         "tw_restore_data"
#define TW_RESTORE_BOOT_VAR         "tw_restore_boot"
#define TW_RESTORE_RECOVERY_VAR     "tw_restore_recovery"
#define TW_RESTORE_CACHE_VAR        "tw_restore_cache"
#define TW_RESTORE_ANDSEC_VAR       "tw_restore_andsec"
#define TW_RESTORE_SDEXT_VAR        "tw_restore_sdext"
#define TW_RESTORE_AVG_IMG_RATE     "tw_restore_avg_img_rate"
#define TW_RESTORE_AVG_FILE_RATE    "tw_restore_avg_file_rate"
#define TW_RESTORE_AVG_FILE_COMP_RATE    "tw_restore_avg_file_comp_rate"
#define TW_RESTORE_FILE_DATE        "tw_restore_file_date"
#define TW_VERIFY_DIGEST_TEXT       "tw_verify_digest_text"
#define TW_UPDATE_SYSTEM_DETAILS_TEXT "tw_update_system_details_text"

#define TW_VERSION_VAR              "tw_version"
#define TW_GUI_SORT_ORDER           "tw_gui_sort_order"
#define TW_ZIP_LOCATION_VAR         "tw_zip_location"
#define TW_ZIP_INTERNAL_VAR         "tw_zip_internal"
#define TW_ZIP_EXTERNAL_VAR         "tw_zip_external"
#define TW_DISABLE_FREE_SPACE_VAR   "tw_disable_free_space"
#define TW_FORCE_DIGEST_CHECK_VAR   "tw_force_digest_check"
#define TW_SKIP_DIGEST_CHECK_VAR    "tw_skip_digest_check"
#define TW_SKIP_DIGEST_GENERATE_VAR "tw_skip_digest_generate"
#define TW_SIGNED_ZIP_VERIFY_VAR    "tw_signed_zip_verify"
#define TW_INSTALL_REBOOT_VAR       "tw_install_reboot"
#define TW_TIME_ZONE_VAR            "tw_time_zone"
#define TW_RM_RF_VAR                "tw_rm_rf"

#define TW_BACKUPS_FOLDER_VAR       "tw_backups_folder"
#define TW_AUTO_DISABLE_AVB2_VAR    "tw_auto_disable_avb2"
#define OF_AUTO_DISABLE_VBMETA_AVB2_VAR  "fox_auto_disable_vbmeta_avb2"
#define OF_PATCH_AVB20_VAR	    "fox_patch_avb_20"

#define TW_SDEXT_SIZE               "tw_sdext_size"
#define TW_SWAP_SIZE                "tw_swap_size"
#define TW_SDPART_FILE_SYSTEM       "tw_sdpart_file_system"
#define TW_TIME_ZONE_GUISEL         "tw_time_zone_guisel"
#define TW_TIME_ZONE_GUIOFFSET      "tw_time_zone_guioffset"
#define TW_TIME_ZONE_GUIDST         "tw_time_zone_guidst"

#define TW_ACTION_BUSY              "tw_busy"

#define TW_ALLOW_PARTITION_SDCARD   "tw_allow_partition_sdcard"

#define TW_SCREEN_OFF               "tw_screen_off"

#define TW_REBOOT_SYSTEM            "tw_reboot_system"
#define TW_REBOOT_RECOVERY          "tw_reboot_recovery"
#define TW_REBOOT_POWEROFF          "tw_reboot_poweroff"
#define TW_REBOOT_BOOTLOADER        "tw_reboot_bootloader"

#define TW_USE_EXTERNAL_STORAGE     "tw_use_external_storage"
#define TW_HAS_INTERNAL             "tw_has_internal"
#define TW_INTERNAL_PATH            "tw_internal_path"         // /data/media or /internal
#define TW_INTERNAL_MOUNT           "tw_internal_mount"        // /data or /internal
#define TW_INTERNAL_LABEL           "tw_internal_label"        // data or internal
#define TW_HAS_EXTERNAL             "tw_has_external"
#define TW_EXTERNAL_PATH            "tw_external_path"         // /sdcard or /external/sdcard2
#define TW_EXTERNAL_MOUNT           "tw_external_mount"        // /sdcard or /external
#define TW_EXTERNAL_LABEL           "tw_external_label"        // sdcard or external

#define TW_HAS_DATA_MEDIA           "tw_has_data_media"

#define TW_HAS_BOOT_PARTITION       "tw_has_boot_partition"
#define TW_HAS_RECOVERY_PARTITION   "tw_has_recovery_partition"
#define TW_HAS_ANDROID_SECURE       "tw_has_android_secure"
#define TW_HAS_SDEXT_PARTITION      "tw_has_sdext_partition"
#define TW_HAS_USB_STORAGE          "tw_has_usb_storage"
#define TW_NO_BATTERY_PERCENT       "tw_no_battery_percent"
#define TW_POWER_BUTTON             "tw_power_button"
#define TW_SIMULATE_ACTIONS         "tw_simulate_actions"
#define TW_SIMULATE_FAIL            "tw_simulate_fail"
#define TW_DONT_UNMOUNT_SYSTEM      "tw_dont_unmount_system"
// #define TW_ALWAYS_RMRF              "tw_always_rmrf"

#define TW_HAS_INJECTTWRP           "tw_has_injecttwrp"
#define TW_INJECT_AFTER_ZIP         "tw_inject_after_zip"
#define TW_HAS_DATADATA             "tw_has_datadata"
#define TW_FLASH_ZIP_IN_PLACE       "tw_flash_zip_in_place"
#define TW_MIN_SYSTEM_SIZE          "50" // minimum system size to allow a reboot
#define TW_MIN_SYSTEM_VAR           "tw_min_system"
#define TW_DOWNLOAD_MODE            "tw_download_mode"
#define TW_EDL_MODE                 "tw_edl_mode"
#define TW_FASTBOOT_MODE            "tw_fastboot_mode"
#define TW_IS_ENCRYPTED             "tw_is_encrypted"
#define TW_IS_DECRYPTED             "tw_is_decrypted"
#define TW_CRYPTO_PWTYPE            "tw_crypto_pwtype"
#define TW_HAS_CRYPTO               "tw_has_crypto"
#define TW_IS_FBE                   "tw_is_fbe"
#define TW_CRYPTO_PASSWORD          "tw_crypto_password"
#define TW_SDEXT_DISABLE_EXT4       "tw_sdext_disable_ext4"
#define TW_MILITARY_TIME            "tw_military_time"
#define TW_USE_SHA2                 "tw_use_sha2"
#define TW_NO_SHA2                  "tw_no_sha2"
#define TW_UNMOUNT_SYSTEM           "tw_unmount_system"
#define TW_UNMOUNT_VENDOR           "tw_unmount_vendor"

// Also used:
//   tw_boot_is_mountable
//   tw_system_is_mountable
//   tw_data_is_mountable
//   tw_cache_is_mountable
//   tw_sdcext_is_mountable
//   tw_sdcint_is_mountable
//   tw_sd-ext_is_mountable
//   tw_sp1_is_mountable
//   tw_sp2_is_mountable
//   tw_sp3_is_mountable

// Error codes
// error code 7: device mismatch
#define TW_ERROR_WRONG_DEVICE 7

// Max archive size for tar backups before we split (1.5GB)
#define MAX_ARCHIVE_SIZE 1610612736LLU
//#define MAX_ARCHIVE_SIZE 52428800LLU // 50MB split for testing

// define maximum number of archives
#define MAX_ARCHIVE_COUNT 247

#ifndef CUSTOM_LUN_FILE
#define CUSTOM_LUN_FILE 	"/config/usb_gadget/g1/functions/mass_storage.0/lun.%d/file"
#endif

#define SCRIPT_FILE_TMP 	"/tmp/openrecoveryscript"
#define TMP_LOG_FILE 		"/tmp/recovery.log"
#define UPDATER_SCRIPT  	"META-INF/com/google/android/updater-script"
#define AROMA_CONFIG 		"META-INF/com/google/android/aroma-config"
#define TW_IS_SUPER             "tw_is_super"
#define TW_VIRTUAL_AB_ENABLED   "tw_virtual_ab.enabled"
#define TW_AUTO_REFLASHTWRP_VAR "tw_auto_reflashtwrp"

// BUILD PROPS
#define TW_FASTBOOT_MODE_PROP         "ro.twrp.fastbootd"
#define TW_MODULES_MOUNTED_PROP       "twrp.modules.loaded"     // property for successfully mounted modules
#define TW_KEYMASTER_VERSION_PROP     "keymaster_ver"

#endif  // _VARIABLES_HEADER_
