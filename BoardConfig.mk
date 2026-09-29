#
# Copyright (C) 2024-2026 The OrangeFox Recovery Project
#
# SPDX-License-Identifier: GPL-3.0-or-later
#

# BoardConfig.mk — Board-level configuration for OrangeFox Recovery.
# AIO-only: one universal payload for all Tensor SoC families (Pixel 6-11,
# folds, tablet). The stock kernel is always kept; only the recovery ramdisk
# cpio is delivered. Family specifics resolve at runtime (stub + engine).
#
# Crypto: FBE with wrappedkey_v0 + metadata encryption via Trusty TEE KeyMint
# Boot: Virtual A/B with vendor_boot; recovery-in-platform payload layout

DEVICE_PATH := device/google/pixels

# Allow for building with minimal manifest
ALLOW_MISSING_DEPENDENCIES := true

# A/B (AIO-only reduced list: init_boot / vendor_kernel_boot / system_dlkm
# do not exist on Tensor G1, and the universal payload boots there too)
AB_OTA_UPDATER := true
AB_OTA_PARTITIONS += \
    boot \
    vendor_boot \
    dtbo \
    vbmeta \
    vbmeta_system \
    vbmeta_vendor \
    system \
    system_ext \
    product \
    vendor \
    vendor_dlkm \
    modem \
    abl \
    bl1 \
    bl2 \
    bl31 \
    gsa \
    gsa_bl1 \
    gcf \
    pbl \
    pvmfw \
    tzsw \
    ldfw

# Architecture
TARGET_ARCH := arm64
TARGET_ARCH_VARIANT := armv8-2a
TARGET_CPU_ABI := arm64-v8a
TARGET_CPU_ABI2 :=
TARGET_CPU_VARIANT := generic
TARGET_CPU_VARIANT_RUNTIME := cortex-a55

TARGET_2ND_ARCH := arm
TARGET_2ND_ARCH_VARIANT := armv8-2a
TARGET_2ND_CPU_ABI := armeabi-v7a
TARGET_2ND_CPU_ABI2 := armeabi
TARGET_2ND_CPU_VARIANT := generic
TARGET_2ND_CPU_VARIANT_RUNTIME := cortex-a75

TARGET_SUPPORTS_64_BIT_APPS := true
TARGET_IS_64_BIT := true

# Board
BOARD_HAS_NO_SELECT_BUTTON := true
BOARD_HAS_LARGE_FILESYSTEM := true

# Bootloader (AIO-only: single universal target)
TARGET_BOOTLOADER_BOARD_NAME := aio
TARGET_NO_BOOTLOADER := true
TARGET_USES_UEFI := true

# Build Broken
BUILD_BROKEN_DUP_RULES := true
BUILD_BROKEN_ELF_PREBUILT_PRODUCT_COPY_FILES := true
BUILD_BROKEN_MISSING_REQUIRED_MODULES := true

# Debug
TARGET_USES_LOGD := true
TWRP_INCLUDE_LOGCAT := true

# Display
TARGET_SCREEN_DENSITY := 420
TARGET_SCREEN_HEIGHT := 2400
TARGET_SCREEN_WIDTH := 1080

# Kernel
TARGET_NO_KERNEL := true
TARGET_KERNEL_ARCH := arm64
TARGET_KERNEL_HEADER_ARCH := arm64
BOARD_KERNEL_IMAGE_NAME := Image.lz4
BOARD_RAMDISK_USE_LZ4 := true
BOARD_BOOT_HEADER_VERSION := 4
BOARD_KERNEL_PAGESIZE := 2048
BOARD_KERNEL_BASE := 0x1000000
BOARD_KERNEL_OFFSET := 0x00008000
BOARD_RAMDISK_OFFSET := 0x01000000
BOARD_KERNEL_TAGS_OFFSET := 0x00000100

# AIO-only: no kernel image is built at all — the stock kernel is kept and
# only the recovery ramdisk cpio is delivered. The intermediate vendor_boot
# only donates its platform fragment, so a dummy cmdline is enough to
# satisfy mkbootimg.
include $(DEVICE_PATH)/families/aio/family.mk
VENDOR_CMDLINE := aio=1

BOARD_MKBOOTIMG_ARGS += --pagesize $(BOARD_KERNEL_PAGESIZE)
BOARD_MKBOOTIMG_ARGS += --header_version $(BOARD_BOOT_HEADER_VERSION)
BOARD_MKBOOTIMG_ARGS += --base $(BOARD_KERNEL_BASE)
BOARD_MKBOOTIMG_ARGS += --kernel_offset $(BOARD_KERNEL_OFFSET)
BOARD_MKBOOTIMG_ARGS += --ramdisk_offset $(BOARD_RAMDISK_OFFSET)
BOARD_MKBOOTIMG_ARGS += --tags_offset $(BOARD_KERNEL_TAGS_OFFSET)
BOARD_MKBOOTIMG_ARGS += --vendor_cmdline $(VENDOR_CMDLINE)

# Partitions - Sizes
BOARD_BOOTIMAGE_PARTITION_SIZE := 67108864
BOARD_VENDOR_BOOTIMAGE_PARTITION_SIZE := 67108864
BOARD_DTBOIMG_PARTITION_SIZE := 4194304

# Partition - Metadata
BOARD_USES_METADATA_PARTITION := true

# Partition Type
BOARD_SYSTEMIMAGE_PARTITION_TYPE := ext4
BOARD_VENDORIMAGE_FILE_SYSTEM_TYPE := ext4
BOARD_USERDATAIMAGE_FILE_SYSTEM_TYPE := f2fs

# Partitions - Super/Logical
BOARD_SUPER_PARTITION_SIZE := 8531214336
BOARD_SUPER_PARTITION_GROUPS := google_dynamic_partitions
BOARD_GOOGLE_DYNAMIC_PARTITIONS_PARTITION_LIST := system system_ext product vendor vendor_dlkm
BOARD_GOOGLE_DYNAMIC_PARTITIONS_SIZE := 8527020032

GOOGLE_BOARD_PLATFORMS += aio
TARGET_BOARD_PLATFORM := aio
PRODUCT_PLATFORM := aio
TARGET_BOARD_PLATFORM_GPU := mali-g71
BOARD_VINTF_CHECK := false

# Properties (AIO-only: neutral placeholder files; the installer swaps in
# the target family's fstab/wipe post-unpack)
TARGET_VENDOR_PROP += $(DEVICE_PATH)/families/common/vendor.prop
TARGET_RECOVERY_FSTAB := $(DEVICE_PATH)/families/aio/recovery.fstab
TARGET_RECOVERY_WIPE := $(DEVICE_PATH)/families/aio/recovery.wipe

# Recovery
TARGET_RECOVERY_PIXEL_FORMAT := ABGR_8888
TARGET_USERIMAGES_USE_EXT4 := true
TARGET_USERIMAGES_USE_F2FS := true
TARGET_USES_MKE2FS := true
RECOVERY_SDCARD_ON_DATA := true
TARGET_NO_RECOVERY := true
BOARD_RECOVERY_SNAPSHOT := false

# SPL
PLATFORM_VERSION := 99.87.36
PLATFORM_VERSION_LAST_STABLE := $(PLATFORM_VERSION)
PLATFORM_SECURITY_PATCH := 2099-12-31
BOOT_SECURITY_PATCH := $(PLATFORM_SECURITY_PATCH)
VENDOR_SECURITY_PATCH := $(PLATFORM_SECURITY_PATCH)

TW_CUSTOM_CPU_TEMP_PATH := /dev/thermal_cpu

TW_THEME := portrait_hdpi
TW_DEFAULT_LANGUAGE := en
TW_EXTRA_LANGUAGES := true
TW_INPUT_BLACKLIST := "hbtp_vm"
TW_USE_TOOLBOX := true
TW_NO_SCREEN_BLANK := true
TW_NO_LEGACY_PROPS := true
TW_DEFAULT_BRIGHTNESS := 1900
TW_BRIGHTNESS_PATH := "/sys/class/backlight/panel/brightness"
TW_FRAMERATE := 120

# TWRP Configuration - Excludes
TW_EXCLUDE_APEX := true
TW_EXCLUDE_DEFAULT_USB_INIT := true
TW_EXCLUDE_TWRPAPP := true

# TWRP Configuration - Crypto (FBE metadata decryption via Trusty TEE KeyMint)
TW_INCLUDE_CRYPTO := true
TW_INCLUDE_CRYPTO_FBE := true
TW_INCLUDE_FBE_METADATA_DECRYPT := true
TW_USE_FSCRYPT_POLICY := 2
# OF_SKIP_FBE_DECRYPTION := 1
OF_FORCE_DATA_FORMAT_F2FS := 1

# TWRP Configuration - Includes
TW_INCLUDE_FASTBOOTD := true
TW_INCLUDE_RESETPROP := true
TW_INCLUDE_LIBRESETPROP := true
TW_INCLUDE_REPACKTOOLS := true
TW_INCLUDE_NTFS_3G := true
TW_INCLUDE_FUSE_EXFAT := true
TW_INCLUDE_FUSE_NTFS := true
TW_INCLUDE_LPTOOLS := true

# Vendor Boot (AIO-only: recovery-in-platform payload layout — first-stage
# + recovery merged into one ramdisk, stock kernel kept)
BOARD_MOVE_RECOVERY_RESOURCES_TO_VENDOR_BOOT := true
# Reworked (wide-variant) theme, test-gated: build.sh --new-theme exports
# FOX_REWORK_THEME=1, which Soong inherits via the process environment
# (orangefox_defaults.go reads it with Getenv and emits
# -DFOX_REWORK_THEME for libguitwrp). NOTE: do NOT `export` it here —
# Kati forbids the export keyword in BoardConfig files (global setting);
# the shell export from build.sh/vendorsetup.sh is the channel.
# Empty (default) = stock base theme only.
BOARD_INCLUDE_RECOVERY_RAMDISK_IN_VENDOR_BOOT := false

# AVB
BOARD_AVB_ENABLE := true
BOARD_AVB_ROLLBACK_INDEX := 0
BOARD_AVB_MAKE_VBMETA_IMAGE_ARGS += --flags 3
BOARD_AVB_VENDOR_BOOT_KEY_PATH := external/avb/test/data/testkey_rsa4096.pem
BOARD_AVB_VENDOR_BOOT_ALGORITHM := SHA256_RSA4096
BOARD_AVB_VENDOR_BOOT_ROLLBACK_INDEX := 0
BOARD_AVB_VENDOR_BOOT_ROLLBACK_INDEX_LOCATION := 2

# Board Info
TARGET_BOARD_INFO_FILE := $(DEVICE_PATH)/board-info.txt

# Additional flags
SELINUX_IGNORE_NEVERALLOWS := true
BOARD_ROOT_EXTRA_FOLDERS := bluetooth dsp firmware persist
BOARD_SUPPRESS_SECURE_ERASE := true
BOARD_MOVE_GSI_AVB_KEYS_TO_VENDOR_BOOT := true
ENABLE_SCHEDBOOST := true
TW_BATTERY_SYSFS_WAIT_SECONDS := 6
TW_VERSION := LeeGarChat
LC_ALL := C
TARGET_USE_CUSTOM_LUN_FILE_PATH := /config/usb_gadget/g1/functions/mass_storage.0/lun.%d/file

BOARD_RECOVERY_IMAGE_PREPARE = bash $(DEVICE_PATH)/include/prebuilt/fox_build_callback.sh $(TARGET_RECOVERY_ROOT_OUT) --second-call

# Workaround
TARGET_COPY_OUT_VENDOR := vendor
