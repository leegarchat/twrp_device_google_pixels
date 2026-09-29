#
#	This file is part of the OrangeFox Recovery Project
# 	Copyright (C) 2018-2026 The OrangeFox Recovery Project
#	
#	OrangeFox is free software: you can redistribute it and/or modify
#	it under the terms of the GNU General Public License as published by
#	the Free Software Foundation, either version 3 of the License, or
#	any later version.
#
#	OrangeFox is distributed in the hope that it will be useful,
#	but WITHOUT ANY WARRANTY; without even the implied warranty of
#	MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
#	GNU General Public License for more details.
#
# 	This software is released under GPL version 3 or any later version.
#	See <http://www.gnu.org/licenses/>.
# 	
# 	Please maintain this if you use this script or any part of it
#

LOCAL_CFLAGS += -Wno-unused-parameter -Wno-unused-function -Wno-unused-variable

# Canonical release version
FOX_INTERNAL_RELEASE := R12.0
LOCAL_CFLAGS += -DFOX_INTERNAL_RELEASE='"$(FOX_INTERNAL_RELEASE)"'

ifneq ($(FOX_MAINTAINER_PATCH_VERSION),)
    test_res := $(shell printf -- '%d' $(FOX_MAINTAINER_PATCH_VERSION) 2>/dev/null)
    ifneq ($(FOX_MAINTAINER_PATCH_VERSION),$(test_res))
        $(error Only whole numbers can be used for FOX_MAINTAINER_PATCH_VERSION)
    endif
    test_res :=
    LOCAL_CFLAGS += -DFOX_MAINTAINER_PATCH_VERSION='"$(FOX_MAINTAINER_PATCH_VERSION)"'
    LOCAL_CFLAGS += -DFOX_BUILD='"$(FOX_INTERNAL_RELEASE)""_""$(FOX_MAINTAINER_PATCH_VERSION)"'
else
    LOCAL_CFLAGS += -DFOX_BUILD='"$(FOX_INTERNAL_RELEASE)"'
endif

ifneq ($(FOX_VERSION),)
    $(error 'FOX_VERSION' is obsolete. Look at 'FOX_MAINTAINER_PATCH_VERSION' for maintainer versioning)
endif

ifeq ($(FOX_VARIANT),)
    LOCAL_CFLAGS += -DFOX_VARIANT='"default"'
else
    LOCAL_CFLAGS += -DFOX_VARIANT='"$(FOX_VARIANT)"'
endif

ifeq ($(FOX_DEVICE_MODEL),)
    DEVICE := $(subst twrp_,,$(TARGET_PRODUCT))
    LOCAL_CFLAGS += -DFOX_DEVICE_MODEL='"$(DEVICE)"'
endif

OF_CURRENT_BRANCH := $(shell git -C $(call my-dir) branch --show-current 2>/dev/null)
ifeq ($(OF_CURRENT_BRANCH),)
    LOCAL_CFLAGS += -DOF_CURRENT_BRANCH='"broken repo"'
else
    LOCAL_CFLAGS += -DOF_CURRENT_BRANCH='"$(OF_CURRENT_BRANCH)"'
endif

# include resetprop automatically
ifneq ($(TW_INCLUDE_RESETPROP),true)
   TW_INCLUDE_RESETPROP := true
endif

ifneq ($(TW_INCLUDE_LIBRESETPROP),true)
   TW_INCLUDE_LIBRESETPROP := true
endif

# turn on magiskboot automatically
OF_USE_MAGISKBOOT := 1
OF_USE_MAGISKBOOT_FOR_ALL_PATCHES := 1
LOCAL_CFLAGS += -DOF_USE_MAGISKBOOT=1
LOCAL_CFLAGS += -DOF_USE_MAGISKBOOT_FOR_ALL_PATCHES=1

ifeq ($(OF_FORCE_MAGISKBOOT_BOOT_PATCH_MIUI),1)
    LOCAL_CFLAGS += -DOF_FORCE_MAGISKBOOT_BOOT_PATCH_MIUI='"1"'
endif

# start to ease out the deprecated legacy MIUI stuff
OF_KEEP_DM_VERITY_FORCED_ENCRYPTION := 1
OF_KEEP_DM_VERITY := 1
OF_KEEP_FORCED_ENCRYPTION := 1
OF_DONT_PATCH_ENCRYPTED_DEVICE := 1

# virtual AB
ifeq ($(PRODUCT_VIRTUAL_AB_OTA),true)
    FOX_VIRTUAL_AB_DEVICE := 1
endif

ifeq ($(FOX_VIRTUAL_AB_DEVICE),1)
    LOCAL_CFLAGS += -DFOX_VIRTUAL_AB_DEVICE='"1"'
    FOX_AB_DEVICE := 1
    FOX_VANILLA_BUILD := 1
endif

# enable vbmeta patch in magiskboot 24+
ifeq ($(FOX_PATCH_VBMETA_FLAG),1)
    LOCAL_CFLAGS += -DFOX_PATCH_VBMETA_FLAG='"1"'
    $(warning Do not use "FOX_PATCH_VBMETA_FLAG" unless you are sure that it is needed!)
endif

ifeq ($(FOX_VANILLA_BUILD),1)
    LOCAL_CFLAGS += -DFOX_VANILLA_BUILD='"1"'
    OF_SKIP_ORANGEFOX_PROCESS := 1
    OF_DISABLE_MIUI_SPECIFIC_FEATURES := 1
    OF_DISABLE_OTA_MENU := 1
    OF_NO_ADDITIONAL_MIUI_PROPS_CHECK := 1
    OF_DONT_PATCH_ON_FRESH_INSTALLATION := 1
    OF_NO_MIUI_PATCH_WARNING := 1
    ifeq ($(OF_SUPPORT_ALL_BLOCK_OTA_UPDATES),1)
      $(error OF_SUPPORT_ALL_BLOCK_OTA_UPDATES is not compatible with VANILLA builds)
    endif
endif

ifeq ($(OF_TWRP_COMPATIBILITY_MODE),1)
    OF_DISABLE_MIUI_SPECIFIC_FEATURES := 1
endif

ifeq ($(OF_DISABLE_MIUI_SPECIFIC_FEATURES),1)
    ifeq ($(OF_SUPPORT_ALL_BLOCK_OTA_UPDATES),1)
      $(error OF_SUPPORT_ALL_BLOCK_OTA_UPDATES is not compatible with this setting)
    endif
    LOCAL_CFLAGS += -DOF_DISABLE_MIUI_SPECIFIC_FEATURES='"1"'
endif

ifeq ($(OF_DISABLE_MIUI_OTA_BY_DEFAULT),1)
    LOCAL_CFLAGS += -DOF_DISABLE_MIUI_OTA_BY_DEFAULT='"1"'
endif

ifeq ($(OF_SKIP_ORANGEFOX_PROCESS),1)
    LOCAL_CFLAGS += -DOF_SKIP_ORANGEFOX_PROCESS='"1"'
    OF_DONT_PATCH_ON_FRESH_INSTALLATION := 1
endif

ifeq ($(OF_DONT_PATCH_ON_FRESH_INSTALLATION),1)
    LOCAL_CFLAGS += -DOF_DONT_PATCH_ON_FRESH_INSTALLATION='"1"'
endif

ifneq ($(FOX_BUILD_TYPE),)
    LOCAL_CFLAGS += -DFOX_BUILD_TYPE='"$(FOX_BUILD_TYPE)"'
else
    LOCAL_CFLAGS += -DFOX_BUILD_TYPE='"Unofficial"'
endif

ifeq ($(OF_NO_MIUI_PATCH_WARNING),1)
    LOCAL_CFLAGS += -DOF_NO_MIUI_PATCH_WARNING='"1"'
endif

# stable builds - enable advanced security
ifeq ($(FOX_BUILD_TYPE),Stable)
    OF_ADVANCED_SECURITY := 1
endif

ifeq ($(AB_OTA_UPDATER),true)
    FOX_AB_DEVICE := 1
endif

ifeq ($(OF_AB_DEVICE_WITH_RECOVERY_PARTITION),1)
    FOX_AB_DEVICE := 1
    LOCAL_CFLAGS += -DOF_AB_DEVICE_WITH_RECOVERY_PARTITION='"1"'
    ifeq ($(OF_RECOVERY_AB_FULL_REFLASH_RAMDISK),1)
       LOCAL_CFLAGS += -DOF_RECOVERY_AB_FULL_REFLASH_RAMDISK
    endif
endif

ifeq ($(FOX_AB_DEVICE),1)
    LOCAL_CFLAGS += -DFOX_AB_DEVICE='"1"'
    ifneq ($(AB_OTA_UPDATER),true)
	AB_OTA_UPDATER := true
    	LOCAL_CFLAGS += -DAB_OTA_UPDATER=1
    	LOCAL_SHARED_LIBRARIES += libhardware android.hardware.boot@1.0
    	TWRP_REQUIRED_MODULES += libhardware android.hardware.boot@1.0-service android.hardware.boot@1.0-service.rc
    endif
    RECOVERY_BINARY_SOURCE_FILES += $(TARGET_OUT_EXECUTABLES)/bootctl
endif

# vendor_boot recovery
ifeq ($(FOX_VENDOR_BOOT_RECOVERY),1)
    $(warning 'FOX_VENDOR_BOOT_RECOVERY' is experimental. Exercise great caution if you use it!)
    LOCAL_CFLAGS += -DFOX_VENDOR_BOOT_RECOVERY='"1"'
    FOX_AB_DEVICE := 1
    OF_NO_SPLASH_CHANGE := 1
    FOX_VANILLA_BUILD := 1
    ifeq ($(BOARD_BOOT_HEADER_VERSION),3)
 	$(warning For a proper vendor_boot recovery build, use 'BOARD_BOOT_HEADER_VERSION := 4' and 'BOARD_INCLUDE_RECOVERY_RAMDISK_IN_VENDOR_BOOT := true')
        OF_NO_REFLASH_CURRENT_ORANGEFOX := 1
    endif
    ifeq ($(OF_RECOVERY_AB_FULL_REFLASH_RAMDISK),1)
       LOCAL_CFLAGS += -DOF_RECOVERY_AB_FULL_REFLASH_RAMDISK
       ifneq ($(OF_NO_REFLASH_CURRENT_ORANGEFOX),1)
       OF_NO_REFLASH_CURRENT_ORANGEFOX :=
       endif
    endif
endif

ifeq ($(OF_VENDOR_BOOT_RECOVERY),1)
   $(error "OF_VENDOR_BOOT_RECOVERY" is obsolete. Use "export FOX_VENDOR_BOOT_RECOVERY=1" instead)
endif
#

ifeq ($(OF_DONT_PATCH_ENCRYPTED_DEVICE),1)
    LOCAL_CFLAGS += -DOF_DONT_PATCH_ENCRYPTED_DEVICE='"1"'
endif

ifneq ($(OF_MAINTAINER),)
    LOCAL_CFLAGS += -DOF_MAINTAINER='"$(OF_MAINTAINER)"'
else
    LOCAL_CFLAGS += -DOF_MAINTAINER='"Testing build (unofficial)"'
endif

ifneq ($(OF_FLASHLIGHT_ENABLE),)
    LOCAL_CFLAGS += -DOF_FLASHLIGHT_ENABLE='"$(OF_FLASHLIGHT_ENABLE)"'
else
    LOCAL_CFLAGS += -DOF_FLASHLIGHT_ENABLE='"1"'
endif

ifneq ($(OF_SPLASH_MAX_SIZE),)
    LOCAL_CFLAGS += -DOF_SPLASH_MAX_SIZE='"$(OF_SPLASH_MAX_SIZE)"'
else
    LOCAL_CFLAGS += -DOF_SPLASH_MAX_SIZE='"4096"'
endif

ifeq ($(OF_ADVANCED_SECURITY),1)
    LOCAL_CFLAGS += -DOF_ADVANCED_SECURITY='"1"'
endif

ifneq ($(FOX_CURRENT_DEV_STR),)
    LOCAL_CFLAGS += -DFOX_CURRENT_DEV_STR='"$(FOX_CURRENT_DEV_STR)"'
else
    LOCAL_CFLAGS += -DFOX_CURRENT_DEV_STR='"latest"'
endif

ifneq ($(OF_SCREEN_H),)
    LOCAL_CFLAGS += -DOF_SCREEN_H='"$(OF_SCREEN_H)"'
else
    LOCAL_CFLAGS += -DOF_SCREEN_H='"1920"'
endif

ifneq ($(OF_STATUS_H),)
    LOCAL_CFLAGS += -DOF_STATUS_H='"$(OF_STATUS_H)"'
else
    LOCAL_CFLAGS += -DOF_STATUS_H='"72"'
endif

ifneq ($(OF_HIDE_NOTCH),)
    LOCAL_CFLAGS += -DOF_HIDE_NOTCH='"$(OF_HIDE_NOTCH)"'
else
    LOCAL_CFLAGS += -DOF_HIDE_NOTCH='"0"'
endif

ifneq ($(OF_STATUS_INDENT_LEFT),)
    LOCAL_CFLAGS += -DOF_STATUS_INDENT_LEFT='"$(OF_STATUS_INDENT_LEFT)"'
else
    LOCAL_CFLAGS += -DOF_STATUS_INDENT_LEFT='"20"'
endif

ifneq ($(OF_STATUS_INDENT_RIGHT),)
    LOCAL_CFLAGS += -DOF_STATUS_INDENT_RIGHT='"$(OF_STATUS_INDENT_RIGHT)"'
else
    LOCAL_CFLAGS += -DOF_STATUS_INDENT_RIGHT='"20"'
endif

ifneq ($(OF_CLOCK_POS),)
    LOCAL_CFLAGS += -DOF_CLOCK_POS='"$(OF_CLOCK_POS)"'
else
    LOCAL_CFLAGS += -DOF_CLOCK_POS='"0"'
endif

ifneq ($(OF_ALLOW_DISABLE_NAVBAR),)
    LOCAL_CFLAGS += -DOF_ALLOW_DISABLE_NAVBAR='"$(OF_ALLOW_DISABLE_NAVBAR)"'
else
    LOCAL_CFLAGS += -DOF_ALLOW_DISABLE_NAVBAR='"1"'
endif

ifneq ($(OF_FL_PATH1),)
    LOCAL_CFLAGS += -DOF_FL_PATH1='"$(OF_FL_PATH1)"'
else
    LOCAL_CFLAGS += -DOF_FL_PATH1='""'
endif

ifneq ($(OF_FL_PATH2),)
    LOCAL_CFLAGS += -DOF_FL_PATH2='"$(OF_FL_PATH2)"'
else
    LOCAL_CFLAGS += -DOF_FL_PATH2='""'
endif

ifeq ($(OF_SKIP_FBE_DECRYPTION),1)
    LOCAL_CFLAGS += -DOF_SKIP_FBE_DECRYPTION='"1"'
endif

ifneq ($(OF_SKIP_FBE_DECRYPTION_SDKVERSION),)
    LOCAL_CFLAGS += -DOF_SKIP_FBE_DECRYPTION_SDKVERSION='"$(OF_SKIP_FBE_DECRYPTION_SDKVERSION)"'
endif

ifeq ($(OF_CLASSIC_LEDS_FUNCTION),1)
    LOCAL_CFLAGS += -DOF_CLASSIC_LEDS_FUNCTION='"1"'
endif

ifneq ($(TW_OZIP_DECRYPT_KEY),)
    OF_SUPPORT_OZIP_DECRYPTION := 1
endif

ifeq ($(OF_SUPPORT_OZIP_DECRYPTION),1)
    LOCAL_CFLAGS += -DOF_SUPPORT_OZIP_DECRYPTION='"1"'
    RECOVERY_BINARY_SOURCE_FILES += $(TARGET_RECOVERY_ROOT_OUT)/system/bin/ozip_decrypt
endif

ifneq ($(TARGET_OTA_ASSERT_DEVICE),)
ifeq ($(FOX_TARGET_DEVICES),)
    FOX_TARGET_DEVICES := $(TARGET_OTA_ASSERT_DEVICE)
endif
endif

ifneq ($(FOX_TARGET_DEVICES),)
    LOCAL_CFLAGS += -DFOX_TARGET_DEVICES='"$(FOX_TARGET_DEVICES)"'
endif

ifeq ($(OF_CHECK_OVERWRITE_ATTEMPTS),1)
    LOCAL_CFLAGS += -DOF_CHECK_OVERWRITE_ATTEMPTS='"1"'
endif

ifeq ($(OF_ENABLE_LAB),1)
    LOCAL_CFLAGS += -DOF_ENABLE_LAB='"1"'
endif

ifeq ($(FOX_USE_NANO_EDITOR), 1)
    LOCAL_CFLAGS += -DFOX_USE_NANO_EDITOR='"1"'
endif

ifeq ($(OF_NO_MIUI_OTA_VENDOR_BACKUP),1)
    LOCAL_CFLAGS += -DOF_NO_MIUI_OTA_VENDOR_BACKUP='"1"'
endif

ifeq ($(OF_REDUCE_DECRYPTION_TIMEOUT),1)
    LOCAL_CFLAGS += -DOF_REDUCE_DECRYPTION_TIMEOUT='"1"'
endif

ifeq ($(OF_DONT_KEEP_LOG_HISTORY),1)
    LOCAL_CFLAGS += -DOF_DONT_KEEP_LOG_HISTORY='"1"'
endif

ifeq ($(OF_SUPPORT_ALL_BLOCK_OTA_UPDATES),1)
    LOCAL_CFLAGS += -DOF_SUPPORT_ALL_BLOCK_OTA_UPDATES='"1"'
endif

ifeq ($(OF_FIX_OTA_UPDATE_MANUAL_FLASH_ERROR),1)
    LOCAL_CFLAGS += -DOF_FIX_OTA_UPDATE_MANUAL_FLASH_ERROR='"1"'
endif

ifeq ($(OF_OTA_BACKUP_STOCK_BOOT_IMAGE),1)
    LOCAL_CFLAGS += -DOF_OTA_BACKUP_STOCK_BOOT_IMAGE
endif

ifeq ($(OF_FBE_METADATA_MOUNT_IGNORE),1)
    LOCAL_CFLAGS += -DOF_FBE_METADATA_MOUNT_IGNORE='"1"'
endif

ifeq ($(OF_PATCH_AVB20),1)
    LOCAL_CFLAGS += -DOF_PATCH_AVB20='"1"'
endif

ifneq ($(OF_QUICK_BACKUP_LIST),)
    LOCAL_CFLAGS += -DOF_QUICK_BACKUP_LIST='"$(OF_QUICK_BACKUP_LIST)"'
endif

ifeq ($(OF_USE_LOCKSCREEN_BUTTON),1)
    LOCAL_CFLAGS += -DOF_USE_LOCKSCREEN_BUTTON
endif

ifeq ($(OF_USE_LZMA_COMPRESSION),1)
    ifeq ($(BOARD_RAMDISK_USE_LZMA),)
    	BOARD_RAMDISK_USE_LZMA := true
    endif
endif

ifeq ($(BOARD_RAMDISK_USE_LZ4),true)
    OF_USE_LZ4_COMPRESSION := 1
endif

ifeq ($(OF_USE_LZ4_COMPRESSION),1)
    ifneq ($(BOARD_RAMDISK_USE_LZ4),true)
    	BOARD_RAMDISK_USE_LZ4 := true
    endif
    LOCAL_CFLAGS += -DOF_USE_LZ4_COMPRESSION
endif

ifeq ($(OF_NO_TREBLE_COMPATIBILITY_CHECK),1)
    LOCAL_CFLAGS += -DOF_NO_TREBLE_COMPATIBILITY_CHECK='"1"'
endif

ifeq ($(OF_INCREMENTAL_OTA_BACKUP_SUPER),1)
    LOCAL_CFLAGS += -DOF_INCREMENTAL_OTA_BACKUP_SUPER='"1"'
endif

ifeq ($(OF_REPORT_HARMLESS_MOUNT_ISSUES),1)
    LOCAL_CFLAGS += -DOF_REPORT_HARMLESS_MOUNT_ISSUES='"1"'
endif

ifeq ($(OF_OTA_RES_CHECK_MICROSD),1)
    LOCAL_CFLAGS += -DOF_OTA_RES_CHECK_MICROSD='"1"'
endif

# samsung dynamic issues
ifeq ($(FOX_DYNAMIC_SAMSUNG_FIX),1)
    FOX_BUILD_BASH := 0
    FOX_EXCLUDE_NANO_EDITOR := 1
endif

# samsung haptics
ifeq ($(OF_USE_SAMSUNG_HAPTICS),1)
    TW_USE_SAMSUNG_HAPTICS := true
endif

# nano
ifeq ($(FOX_EXCLUDE_NANO_EDITOR),1)
    TW_EXCLUDE_NANO := true
endif

ifeq ($(FOX_USE_NANO_EDITOR),1)
    TW_EXCLUDE_NANO := true
endif

ifneq ($(TW_EXCLUDE_NANO), true)
    ifeq ($(wildcard external/nano/Android.bp),)
        $(warning Nano sources not found! You need to clone the sources.)
        $(warning Please run: "git clone --depth=1 https://github.com/LineageOS/android_external_nano -b lineage-21.0 external/nano")
        $(error Nano sources not present; exiting)
    endif
    ifeq ($(wildcard external/libncurses/Android.bp),)
        $(warning Libncurses not found! You need to clone the sources.)
        $(warning Please run: "git clone --depth=1 https://github.com/LineageOS/android_external_libncurses -b lineage-21.0 external/libncurses")
        $(error Libncurses sources not present; exiting)
    endif
endif

# bash
ifeq ($(FOX_BUILD_BASH),1)
  ifeq ($(wildcard external/bash/Android.bp),)
        $(warning Bash sources not found! You need to clone the sources.)
        $(warning Please run: "git clone --depth=1 https://github.com/LineageOS/android_external_bash -b lineage-21.0 external/bash")
        $(error Bash sources not present; exiting)
  endif
  RECOVERY_BINARY_SOURCE_FILES += $(TARGET_OUT_OPTIONAL_EXECUTABLES)/bash
  RECOVERY_LIBRARY_SOURCE_FILES += $(TARGET_OUT_SHARED_LIBRARIES)/libncurses.so
  TWRP_REQUIRED_MODULES += bash libncurses

  TWRP_REQUIRED_MODULES += \
    bash_fox
endif

# check for conflicts
ifeq ($(OF_SUPPORT_ALL_BLOCK_OTA_UPDATES),1)
   ifeq ($(OF_DISABLE_MIUI_SPECIFIC_FEATURES),1)
        $(warning You cannot use "OF_SUPPORT_ALL_BLOCK_OTA_UPDATES" with "OF_DISABLE_MIUI_SPECIFIC_FEATURES")
        $(error Fix your build vars!; exiting)
   endif
endif

# disable by default the USB storage button on the "Mount" menu
ifneq ($(OF_ENABLE_USB_STORAGE),1)
    TW_NO_USB_STORAGE := true
endif

ifeq ($(OF_DISABLE_EXTRA_ABOUT_PAGE),1)
    LOCAL_CFLAGS += -DOF_DISABLE_EXTRA_ABOUT_PAGE='"1"'
endif

ifeq ($(FOX_ENABLE_APP_MANAGER),1)
    LOCAL_CFLAGS += -DFOX_ENABLE_APP_MANAGER='"1"'
endif

ifeq ($(OF_NO_SPLASH_CHANGE),1)
    LOCAL_CFLAGS += -DOF_NO_SPLASH_CHANGE='"1"'
endif

ifeq ($(FOX_DELETE_MAGISK_ADDON),1)
    LOCAL_CFLAGS += -DFOX_DELETE_MAGISK_ADDON='"1"'
endif

ifeq ($(FOX_MOVE_MAGISK_INSTALLER_TO_RAMDISK),1)
    LOCAL_CFLAGS += -DFOX_MOVE_MAGISK_INSTALLER_TO_RAMDISK='"1"'
endif

ifeq ($(OF_USE_GREEN_LED),0)
    LOCAL_CFLAGS += -DOF_NO_GREEN_LED='"1"'
endif

# all partition tools - filter, and support TW_ENABLE_ALL_PARTITION_TOOLS
ifeq ($(OF_ENABLE_ALL_PARTITION_TOOLS),1)
   ifneq ($(PRODUCT_USE_DYNAMIC_PARTITIONS),true)
        $(error "OF_ENABLE_ALL_PARTITION_TOOLS" requires dynamic partitions; quitting)
   endif
   TW_ENABLE_ALL_PARTITION_TOOLS := true
endif

# lptools; disable by default; enable with OF_ENABLE_LPTOOLS=1
ifeq ($(OF_ENABLE_LPTOOLS),1)
    ifeq ($(wildcard external/lptools/Android.bp),)
        $(warning lptools sources not found! You need to run "repo sync" to clone the sources.)
        $(warning You can also run: "git clone https://github.com/phhusson/vendor_lptools external/lptools")
        $(error lptools sources not present; exiting)
    else
        TW_INCLUDE_LPTOOLS := true
    endif
endif

# adopted storage
ifeq ($(OF_SKIP_DECRYPTED_ADOPTED_STORAGE),1)
    LOCAL_CFLAGS += -DOF_SKIP_DECRYPTED_ADOPTED_STORAGE='"1"'
endif

ifneq ($(OF_DYNAMIC_FULL_SIZE),)
    LOCAL_CFLAGS += -DOF_DYNAMIC_FULL_SIZE='"$(OF_DYNAMIC_FULL_SIZE)"'
endif

# bind-unmount /sdcard before data repair/format (currently applies only to f2fs)
ifeq ($(OF_UNBIND_SDCARD_F2FS),1)
    LOCAL_CFLAGS += -DOF_UNBIND_SDCARD_F2FS='"1"'
endif

# morph the twrp flag into ours, since we'd already had the code in place
ifeq ($(TW_PREPARE_DATA_MEDIA_EARLY),true)
   OF_FIX_DECRYPTION_ON_DATA_MEDIA := 1
endif

# avoid decryption problems on some devices and ROMs
ifeq ($(OF_FIX_DECRYPTION_ON_DATA_MEDIA),1)
    LOCAL_CFLAGS += -DOF_FIX_DECRYPTION_ON_DATA_MEDIA='"1"'
endif

# deal with new error ('NO KERNEL CONFIG') when using a prebuilt kernel
ifeq ($(OF_FORCE_PREBUILT_KERNEL),1)
    TARGET_FORCE_PREBUILT_KERNEL := true
    TARGET_KERNEL_SOURCE :=
endif

# automatically deal with new error ('NO KERNEL CONFIG') when using a prebuilt kernel
# partially revert vendor_twrp commit 9d4bb8e
ifneq ($(TARGET_PREBUILT_KERNEL),)
    TARGET_KERNEL_SOURCE :=
endif

# whether to force a ramdisk checksum on reflashing OrangeFox (virtual A/B only)
ifeq ($(OF_FORCE_CHECK_RAMDISK_CHECKSUM),1)
    LOCAL_CFLAGS += -DOF_FORCE_CHECK_RAMDISK_CHECKSUM='"1"'
endif

# vAB - whether to disable the flash current OrangeFox menu
ifeq ($(OF_NO_REFLASH_CURRENT_ORANGEFOX),1)
    LOCAL_CFLAGS += -DOF_NO_REFLASH_CURRENT_ORANGEFOX='"1"'
endif

ifeq ($(TW_NO_FLASH_CURRENT_TWRP),true)
    OF_NO_REFLASH_CURRENT_ORANGEFOX := 1
endif

ifeq ($(OF_NO_ADDITIONAL_MIUI_PROPS_CHECK),1)
    LOCAL_CFLAGS += -DOF_NO_ADDITIONAL_MIUI_PROPS_CHECK
endif

# disable the MIUI OTA menu and its supports
ifeq ($(OF_DISABLE_OTA_MENU),1)
    LOCAL_CFLAGS += -DOF_DISABLE_OTA_MENU
    OF_DISABLE_MIUI_SPECIFIC_FEATURES := 1
    OF_DISABLE_MIUI_OTA_BY_DEFAULT := 1
    OF_NO_MIUI_PATCH_WARNING := 1
endif

# support custom default time zones
ifneq ($(OF_DEFAULT_TIMEZONE),)
    LOCAL_CFLAGS += -DOF_DEFAULT_TIMEZONE='"$(OF_DEFAULT_TIMEZONE)"'
else
    LOCAL_CFLAGS += -DOF_DEFAULT_TIMEZONE='"CET-1;CEST,M3.5.0,M10.5.0"'
endif

# support fs compression (requires kernel support and appropriate fstab flags)
ifeq ($(TW_ENABLE_FS_COMPRESSION),true)
  OF_ENABLE_FS_COMPRESSION := 1
endif

ifeq ($(OF_ENABLE_FS_COMPRESSION),1)
    LOCAL_CFLAGS += -DOF_ENABLE_FS_COMPRESSION
endif

# Don't spam the console with noisy loop device mount errors; just write them to the log file
ifeq ($(OF_LOOP_DEVICE_ERRORS_TO_LOG),1)
    LOCAL_CFLAGS += -DOF_LOOP_DEVICE_ERRORS_TO_LOG
endif

# renamed build vars - throw up errors:
ifeq ($(OF_VIRTUAL_AB_DEVICE),1)
   $(error "OF_VIRTUAL_AB_DEVICE" is obsolete. Use "export FOX_VIRTUAL_AB_DEVICE=1" instead)
endif

ifeq ($(OF_AB_DEVICE),1)
   $(error "OF_AB_DEVICE" is obsolete. Use "export FOX_AB_DEVICE=1" instead)
endif

ifeq ($(OF_PATCH_VBMETA_FLAG),1)
   $(error "OF_PATCH_VBMETA_FLAG" is obsolete. Use "export FOX_PATCH_VBMETA_FLAG=1" instead)
endif

ifeq ($(OF_VANILLA_BUILD),1)
   $(error "OF_VANILLA_BUILD" is obsolete. Use "export FOX_VANILLA_BUILD=1" instead)
endif

ifneq ($(OF_TARGET_DEVICES),)
   $(error "OF_TARGET_DEVICES" is obsolete. Use "FOX_TARGET_DEVICES" instead)
endif

ifeq ($(FOX_USE_LZMA_COMPRESSION),1)
   $(error "FOX_USE_LZMA_COMPRESSION" is obsolete. Use "export OF_USE_LZMA_COMPRESSION=1" instead)
endif

ifeq ($(FOX_ADVANCED_SECURITY),1)
   $(error "FOX_ADVANCED_SECURITY" is obsolete. Use "export OF_ADVANCED_SECURITY=1" instead)
endif

ifeq ($(FOX_USE_LZ4_COMPRESSION),1)
   $(error "FOX_USE_LZ4_COMPRESSION" is obsolete. Use "export OF_USE_LZ4_COMPRESSION=1" instead)
endif

# whether to display debug information about the target partition when formatting data
ifeq ($(OF_DISPLAY_FORMAT_FILESYSTEMS_DEBUG_INFO),1)
    LOCAL_CFLAGS += -DOF_DISPLAY_FORMAT_FILESYSTEMS_DEBUG_INFO
endif

ifneq ($(FOX_BUGGED_AOSP_ARB_WORKAROUND),)
    LOCAL_CFLAGS += -DFOX_BUGGED_AOSP_ARB_WORKAROUND='"$(FOX_BUGGED_AOSP_ARB_WORKAROUND)"'
endif

# some mtk devices will need this, consequent upon recent build system commits
ifeq ($(OF_FORCE_USE_RECOVERY_FSTAB),1)
   $(warning "OF_FORCE_USE_RECOVERY_FSTAB" is deprecated. Use "TW_SKIP_ADDITIONAL_FSTAB := true" instead)
   TW_SKIP_ADDITIONAL_FSTAB := true
endif

# default keymaster version
ifneq ($(OF_DEFAULT_KEYMASTER_VERSION),)
    LOCAL_CFLAGS += -DOF_DEFAULT_KEYMASTER_VERSION='"$(OF_DEFAULT_KEYMASTER_VERSION)"'
endif

# enforce the keymaster version from the device tree
ifeq ($(TW_FORCE_KEYMASTER_VER),true)
    ifeq ($(OF_DEFAULT_KEYMASTER_VERSION),)
      $(error Using "TW_FORCE_KEYMASTER_VER" also requires "OF_DEFAULT_KEYMASTER_VERSION")
    endif
endif

# allow disabling support for 'keymaster_ver=4.x'
ifeq ($(OF_NO_KEYMASTER_VER_4X),1)
    ifeq ($(OF_DEFAULT_KEYMASTER_VERSION),)
      $(error Using "OF_NO_KEYMASTER_VER_4X" also requires "OF_DEFAULT_KEYMASTER_VERSION")
    endif
    LOCAL_CFLAGS += -DOF_NO_KEYMASTER_VER_4X
endif

# support disabling avb2.0 by patching vbmeta/vbmeta_system
ifeq ($(OF_SUPPORT_VBMETA_AVB2_PATCHING),1)
    LOCAL_CFLAGS += -DOF_SUPPORT_VBMETA_AVB2_PATCHING='"1"'
endif

# custom settings directory
ifeq ($(FOX_USE_DATA_RECOVERY_FOR_SETTINGS),1)
    ifneq ($(FOX_SETTINGS_ROOT_DIRECTORY),)
       $(error You cannot use "FOX_SETTINGS_ROOT_DIRECTORY" with "FOX_USE_DATA_RECOVERY_FOR_SETTINGS")
    endif
    ifneq ($(FOX_MISCELLANEOUS_ROOT_DIRECTORY),)
       $(error You cannot use "FOX_MISCELLANEOUS_ROOT_DIRECTORY" with "FOX_USE_DATA_RECOVERY_FOR_SETTINGS")
    endif
    LOCAL_CFLAGS += -DFOX_SETTINGS_ROOT_DIRECTORY='"/data/recovery"'
    LOCAL_CFLAGS += -DFOX_MISCELLANEOUS_ROOT_DIRECTORY='"/data/recovery"'
    LOCAL_CFLAGS += -DFOX_USE_DATA_RECOVERY_FOR_SETTINGS
endif

ifneq ($(FOX_SETTINGS_ROOT_DIRECTORY),)
    ifeq ($(FOX_MISCELLANEOUS_ROOT_DIRECTORY),)
       LOCAL_CFLAGS += -DFOX_MISCELLANEOUS_ROOT_DIRECTORY='"$(FOX_SETTINGS_ROOT_DIRECTORY)"'
    endif
    LOCAL_CFLAGS += -DFOX_SETTINGS_ROOT_DIRECTORY='"$(FOX_SETTINGS_ROOT_DIRECTORY)"'
endif

ifneq ($(FOX_MISCELLANEOUS_ROOT_DIRECTORY),)
    ifneq ($(FOX_USE_DATA_RECOVERY_FOR_SETTINGS),1)
        $(warning "FOX_MISCELLANEOUS_ROOT_DIRECTORY" is used. This is EXPERIMENTAL. Ensure that "$(FOX_MISCELLANEOUS_ROOT_DIRECTORY)" will ALWAYS be accessible on the device)
    endif
    LOCAL_CFLAGS += -DFOX_MISCELLANEOUS_ROOT_DIRECTORY='"$(FOX_MISCELLANEOUS_ROOT_DIRECTORY)"'
endif

ifeq ($(FOX_ALLOW_EARLY_SETTINGS_LOAD),1)
    #ifeq ($(FOX_SETTINGS_ROOT_DIRECTORY),)
    #   $(error You cannot use "FOX_ALLOW_EARLY_SETTINGS_LOAD" without "FOX_SETTINGS_ROOT_DIRECTORY")
    #endif
    LOCAL_CFLAGS += -DFOX_ALLOW_EARLY_SETTINGS_LOAD='"1"'
endif

# whether to wipe /metadata after formatting data
ifeq ($(OF_WIPE_METADATA_AFTER_DATAFORMAT),1)
   ifeq ($(TW_INCLUDE_FBE_METADATA_DECRYPT),true)
	ifeq ($(BOARD_USES_METADATA_PARTITION),true)
		LOCAL_CFLAGS += -DOF_WIPE_METADATA_AFTER_DATAFORMAT
	endif
   endif
endif

# bale out now if TW_MAX_BRIGHTNESS is not set
ifeq ($(TW_MAX_BRIGHTNESS),)
  $(warning 'TW_MAX_BRIGHTNESS' is not set! You must provide a value for 'TW_MAX_BRIGHTNESS' in your device tree)
endif

# whether to use legacy services for battery, or health services (default - but broken on Mtk)
ifeq ($(OF_USE_LEGACY_BATTERY_SERVICES),1)
 TW_USE_LEGACY_BATTERY_SERVICES := true
endif

# refuse obsolete code
ifeq ($(TW_INCLUDE_INJECTTWRP),true)
    $(error 'TW_INCLUDE_INJECTTWRP' is obsolete. Remove it from your device tree)
endif

# support setting the number of items on the 'options' listmenu before creating scrollbar
ifneq ($(OF_OPTIONS_LIST_NUM),)
    LOCAL_CFLAGS += -DOF_OPTIONS_LIST_NUM='"$(OF_OPTIONS_LIST_NUM)"'
endif

# whether to bind-mount /sdcard after formatting data to deal with MTP issues: can be problematic for encryption
ifeq ($(OF_BIND_MOUNT_SDCARD_ON_FORMAT),1)
    LOCAL_CFLAGS += -DOF_BIND_MOUNT_SDCARD_ON_FORMAT
endif

# whether to refresh the encryption props before formatting /data
ifeq ($(OF_REFRESH_ENCRYPTION_PROPS_BEFORE_FORMAT),1)
    LOCAL_CFLAGS += -DOF_REFRESH_ENCRYPTION_PROPS_BEFORE_FORMAT
endif

ifeq ($(OF_UNMOUNT_SDCARDS_BEFORE_REBOOT),1)
    LOCAL_CFLAGS += -DOF_UNMOUNT_SDCARDS_BEFORE_REBOOT
endif

# whether to use the updated magiskboot
ifeq ($(FOX_USE_UPDATED_MAGISKBOOT),1)
    LOCAL_CFLAGS += -DFOX_USE_UPDATED_MAGISKBOOT
endif

# whether to skip building Infozip zip from source
ifeq ($(FOX_EXCLUDE_ZIP),1)
    LOCAL_CFLAGS += -DFOX_EXCLUDE_ZIP
    TW_EXCLUDE_ZIP := true
endif

# if using the prebuilt LZ4 binary, ensure that liblz4.so is included
ifeq ($(FOX_USE_LZ4_BINARY),1)
    RECOVERY_LIBRARY_SOURCE_FILES += $(TARGET_OUT_SHARED_LIBRARIES)/liblz4.so
endif

# whether to force f2fs when formatting data
ifeq ($(OF_FORCE_DATA_FORMAT_F2FS),1)
    ifeq ($(OF_FORCE_DATA_FORMAT_EXT4),1)
        $(error You cannot use both 'OF_FORCE_DATA_FORMAT_F2FS' and 'OF_FORCE_DATA_FORMAT_EXT4' at the same time!)
    endif
    LOCAL_CFLAGS += -DOF_FORCE_DATA_FORMAT_F2FS
endif

# whether to force ext4 when formatting data
ifeq ($(OF_FORCE_DATA_FORMAT_EXT4),1)
    ifeq ($(OF_FORCE_DATA_FORMAT_F2FS),1)
        $(error You cannot use both 'OF_FORCE_DATA_FORMAT_F2FS' and 'OF_FORCE_DATA_FORMAT_EXT4' at the same time!)
    endif
    LOCAL_CFLAGS += -DOF_FORCE_DATA_FORMAT_EXT4
    OF_WIPE_METADATA_AFTER_DATAFORMAT := 1
endif

# whether to use legacy time_fixup code (if the clock is persistently wrong)
ifeq ($(OF_USE_LEGACY_TIME_FIXUP),1)
    LOCAL_CFLAGS += -DOF_USE_LEGACY_TIME_FIXUP
endif

# boot header version (default to v1 if not set)
ifeq ($(BOARD_BOOT_HEADER_VERSION),)
    $(warning 'BOARD_BOOT_HEADER_VERSION' is not set. Defaulting to v1)
    LOCAL_CFLAGS += -DBOARD_BOOT_HEADER_VERSION=1
endif

ifneq ($(PRODUCT_PLATFORM),)
     LOCAL_CFLAGS += -DPRODUCT_PLATFORM=$(PRODUCT_PLATFORM)
endif

ifeq ($(OF_FORCE_CASEFOLDING),1)
     LOCAL_CFLAGS += -DOF_FORCE_CASEFOLDING
endif

ifneq ($(TW_LOAD_VENDOR_MODULES),)
    ifeq ($(OF_SKIP_PREBUILT_MODULES),1)
        LOCAL_CFLAGS += -DOF_SKIP_PREBUILT_MODULES
    endif
endif

ifeq ($(FOX_USE_DMSETUP),1)
  ifeq ($(OF_USE_DMCTL),1)
    $(error You cannot use both 'FOX_USE_DMSETUP' and 'OF_USE_DMCTL' at the same time)
  else
    LOCAL_CFLAGS += -DFOX_USE_DMSETUP='"1"'
  endif
endif

ifeq ($(FOX_USE_DMCTL),1)
  $(error 'FOX_USE_DMCTL' is obsolete. Use 'OF_USE_DMCTL' instead)
endif

ifeq ($(OF_USE_DMCTL),1)
  ifeq ($(FOX_USE_DMSETUP),1)
    $(error You cannot use both 'FOX_USE_DMSETUP' and 'OF_USE_DMCTL' at the same time)
  else
    LOCAL_CFLAGS += -DOF_USE_DMCTL='"1"'
    TWRP_REQUIRED_MODULES += dmctl
    RECOVERY_BINARY_SOURCE_FILES += $(TARGET_OUT_EXECUTABLES)/dmctl
    TWRP_REQUIRED_MODULES += dmuserd
    RECOVERY_BINARY_SOURCE_FILES += $(TARGET_OUT_EXECUTABLES)/dmuserd
  endif
endif

# backup/restore bug - temporary workaround
# activate the workaround by default
OF_WORKAROUND_BACKUP_BUG := 1
ifeq ($(OF_WORKAROUND_BACKUP_BUG),1)
     LOCAL_CFLAGS += -DOF_WORKAROUND_BACKUP_BUG
endif

# ksu and variants
ifeq ($(FOX_ENABLE_KERNELSU_SUPPORT),1)
    ifneq ($(FOX_VIRTUAL_AB_DEVICE),1)
     $(error FOX_ENABLE_KERNELSU_SUPPORT is only valid for Virtual_AB devices with a GKI 5.x or 6.x kernel)
    endif
    LOCAL_CFLAGS += -DFOX_ENABLE_KERNELSU_SUPPORT
endif
ifeq ($(FOX_ENABLE_KERNELSU_NEXT_SUPPORT),1)
    ifneq ($(FOX_VIRTUAL_AB_DEVICE),1)
     $(error FOX_ENABLE_KERNELSU_NEXT_SUPPORT is only valid for Virtual_AB devices with a GKI 5.x or 6.x kernel)
    endif
    LOCAL_CFLAGS += -DFOX_ENABLE_KERNELSU_NEXT_SUPPORT
endif
ifeq ($(FOX_ENABLE_SUKISU_SUPPORT),1)
    ifneq ($(FOX_VIRTUAL_AB_DEVICE),1)
     $(error FOX_ENABLE_SUKISU_SUPPORT is only valid for Virtual_AB devices with a GKI 5.x or 6.x kernel)
    endif
    LOCAL_CFLAGS += -DFOX_ENABLE_SUKISU_SUPPORT
endif

# mask read errors on Get_Folder_Size?
ifeq ($(OF_MASK_GET_FOLDER_SIZE_READ_ERRORS),1)
    LOCAL_CFLAGS += -DOF_MASK_GET_FOLDER_SIZE_READ_ERRORS=1
endif

# disable automatic rebooting after openrecoveryscript finishes
ifeq ($(OF_DISABLE_ORS_AUTO_REBOOT),1)
    LOCAL_CFLAGS += -DOF_DISABLE_ORS_AUTO_REBOOT
endif

# whether to use aidl instead of hidl for bootcontrol
ifeq ($(OF_USE_AIDL_BOOT_CONTROL),1)
    LOCAL_CFLAGS += -DOF_USE_AIDL_BOOT_CONTROL
endif

# enable the FRP deletion addon?
ifeq ($(OF_ENABLE_FRP_ADDON),1)
    LOCAL_CFLAGS += -DOF_ENABLE_FRP_ADDON
endif

ifeq ($(TW_INCLUDE_OMAPI),true)
    ifeq ($(wildcard external/se_omapi/Android.bp),)
        $(warning se_omapi sources not found! You need to clone the sources.)
        $(warning Please run: "https://gitlab.com/OrangeFox/external/se_omapi.git -b fox_14.1 external/se_omapi")
        $(error se_omapi sources not present; exiting.)
    endif
endif

# fastboot reboot
ifeq ($(OF_NO_REBOOT_FASTBOOT),1)
    LOCAL_CFLAGS += -DOF_NO_REBOOT_FASTBOOT
endif

# initd
ifneq ($(FOX_DELETE_INITD_ADDON),0)
    LOCAL_CFLAGS += -DFOX_DELETE_INITD_ADDON
endif

# whether to skip substituting some permissions
ifeq ($(OF_DONT_SUBSTITUTE_PERMISSIONS),1)
    LOCAL_CFLAGS += -DOF_DONT_SUBSTITUTE_PERMISSIONS
endif

# whether to format (instead of just wiping) data in response to OpenRecovery "--wipe-data" instructions (virtual A/B only)
ifeq ($(OF_VAB_ORS_WIPE_DATA_IS_FORMAT),1)
    ifneq ($(FOX_VIRTUAL_AB_DEVICE),1)
     $(error Enable 'FOX_VIRTUAL_AB_DEVICE' before you can use 'OF_VAB_ORS_WIPE_DATA_IS_FORMAT')
    endif
    LOCAL_CFLAGS += -DOF_VAB_ORS_WIPE_DATA_IS_FORMAT
endif


# blocking
ifeq ($(OF_BLOCK_OPERATIONS_AFTER_ROM_FLASH),1)
    LOCAL_CFLAGS += -DOF_BLOCK_OPERATIONS_AFTER_ROM_FLASH='"1"'
endif
#
