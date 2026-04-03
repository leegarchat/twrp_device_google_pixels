#
# Copyright (C) 2024-2026 The OrangeFox Recovery Project
#
# SPDX-License-Identifier: GPL-3.0-or-later
#

# BoardConfig.mk — Board-level configuration for OrangeFox Recovery.
# Targets four Tensor SoC families:
#   gs201   (Tensor G2): panther (Pixel 7), cheetah (Pixel 7 Pro), lynx (Pixel 7a), tangorpro (Pixel Tablet)
#   zuma    (Tensor G3): shiba (Pixel 8), husky (Pixel 8 Pro), akita (Pixel 8a)
#   zumapro (Tensor G4): tokay (Pixel 9), komodo (Pixel 9 Pro XL), caiman (Pixel 9 Pro), tegu (Pixel 9a)
#   laguna  (Tensor G5): blazer (Pixel 10 Pro), mustang (Pixel 10 Pro XL), frankel (Pixel 10)
#
# Build flag DEVICE_BUILD_FLAG selects the target family:
#   (default) → zuma (UFS 13200000, earlycon 10A00000)
#   zumapro   → zumapro (UFS 13200000, earlycon 10870000)
#   gs201     → gs201 (UFS 14700000, earlycon 10A00000)
#   laguna    → laguna (UFS 3c400000, earlycon 10870000)
#
# Crypto: FBE with wrappedkey_v0 + metadata encryption via Trusty TEE KeyMint
# Boot: Virtual A/B with vendor_boot, GKI or monolithic kernel

DEVICE_PATH := device/google/pixels

# Allow for building with minimal manifest
ALLOW_MISSING_DEPENDENCIES := true

# A/B
AB_OTA_UPDATER := true
AB_OTA_PARTITIONS += \
    boot \
    init_boot \
    vendor_boot \
    vendor_kernel_boot \
    dtbo \
    vbmeta \
    vbmeta_system \
    vbmeta_vendor \
    system \
    system_ext \
    system_dlkm \
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

# Bootloader
TARGET_BOOTLOADER_BOARD_NAME := $(DEVICE_BUILD_FLAG)
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

ifeq ($(DEVICE_BUILD_FLAG),zumapro)
VENDOR_CMDLINE := "dyndbg=\"func alloc_contig_dump_pages +p\" \
        earlycon=exynos4210,0x10870000 \
        console=ttySAC0,115200 \
        androidboot.console=ttySAC0 printk.devkmsg=on \
        cma_sysfs.experimental=Y \
        cgroup.memory=nokmem \
        rcupdate.rcu_expedited=1 \
        rcu_nocbs=all \
        rcutree.enable_rcu_lazy \
        swiotlb=noforce \
        disable_dma32=on \
        sysctl.kernel.sched_pelt_multiplier=4 \
        kasan=off \
        at24.write_timeout=100 \
        log_buf_len=1024K bootconfig"
else ifeq ($(DEVICE_BUILD_FLAG),laguna)
VENDOR_CMDLINE := "dyndbg=\"func alloc_contig_dump_pages +p\" \
        earlycon=exynos4210,0x10870000 \
        console=ttySAC0,115200 \
        androidboot.console=ttySAC0 printk.devkmsg=on \
        cma_sysfs.experimental=Y \
        cgroup.memory=nokmem \
        rcupdate.rcu_expedited=1 \
        rcu_nocbs=all \
        rcutree.enable_rcu_lazy \
        swiotlb=noforce \
        disable_dma32=on \
        sysctl.kernel.sched_pelt_multiplier=4 \
        kasan=off \
        at24.write_timeout=100 \
        fips140.load_sequential=1 \
        vh_sched.load_sequential=1 \
        init_on_alloc=0 init_on_free=1 \
        pcie_port_pm=off \
        log_buf_len=1024K bootconfig"
else ifeq ($(DEVICE_BUILD_FLAG),gs101)
VENDOR_CMDLINE := "dyndbg=\"func alloc_contig_dump_pages +p\" \\
        earlycon=exynos4210,0x10A00000 \\
        console=ttySAC0,115200 \\
        androidboot.console=ttySAC0 \\
        printk.devkmsg=on \\
        swiotlb=noforce \\
        cma_sysfs.experimental=Y \\
        cgroup_disable=memory \\
        rcupdate.rcu_expedited=1 \\
        androidboot.usbcontroller=11110000.dwc3 \\
        rcu_nocbs=all \\
        stack_depot_disable=off \\
        page_pinner=on \\
        swiotlb=1024 \\
        disable_dma32=on \\
        at24.write_timeout=100 \\
        log_buf_len=1024K \\
        bootconfig"
VENDOR_CMDLINE := "dyndbg=\"func alloc_contig_dump_pages +p\" \
        earlycon=exynos4210,0x10A00000 \
        console=ttySAC0,115200 \
        androidboot.console=ttySAC0 \
        printk.devkmsg=on \
        swiotlb=noforce \
        cma_sysfs.experimental=Y \
        cgroup_disable=memory \
        rcupdate.rcu_expedited=1 \
        androidboot.usbcontroller=11210000.dwc3 \
        rcu_nocbs=all \
        stack_depot_disable=off \
        page_pinner=on \
        swiotlb=1024 \
        disable_dma32=on \
        at24.write_timeout=100 \
        log_buf_len=1024K \
        bootconfig"
else
VENDOR_CMDLINE := "dyndbg=\"func alloc_contig_dump_pages +p\" \
        earlycon=exynos4210,0x10A00000 \
        console=ttySAC0,115200 \
        androidboot.console=ttySAC0 \
        printk.devkmsg=on \
        swiotlb=noforce \
        cma_sysfs.experimental=Y \
        cgroup_disable=memory \
        rcupdate.rcu_expedited=1 \
        androidboot.usbcontroller=11210000.dwc3 \
        rcu_nocbs=all \
        stack_depot_disable=off \
        page_pinner=on \
        swiotlb=1024 \
        disable_dma32=on \
        at24.write_timeout=100 \
        log_buf_len=1024K \
        bootconfig"
endif
BOARD_BOOTCONFIG += androidboot.usbcontroller=11210000.dwc3
ifeq ($(DEVICE_BUILD_FLAG),gs101)
BOARD_BOOTCONFIG := androidboot.usbcontroller=11110000.dwc3
BOARD_BOOTCONFIG += androidboot.boot_devices=14700000.ufs
else ifeq ($(DEVICE_BUILD_FLAG),gs201)
BOARD_BOOTCONFIG += androidboot.boot_devices=14700000.ufs
else ifeq ($(DEVICE_BUILD_FLAG),laguna)
BOARD_BOOTCONFIG := androidboot.usbcontroller=c400000.dwc3
BOARD_BOOTCONFIG += androidboot.boot_devices=3c400000.ufs
else
BOARD_BOOTCONFIG += androidboot.boot_devices=13200000.ufs
endif
BOARD_BOOTCONFIG += androidboot.load_modules_parallel=true

BOARD_MKBOOTIMG_ARGS += --pagesize $(BOARD_KERNEL_PAGESIZE)
BOARD_MKBOOTIMG_ARGS += --header_version $(BOARD_BOOT_HEADER_VERSION)
BOARD_MKBOOTIMG_ARGS += --base $(BOARD_KERNEL_BASE)
BOARD_MKBOOTIMG_ARGS += --kernel_offset $(BOARD_KERNEL_OFFSET)
BOARD_MKBOOTIMG_ARGS += --ramdisk_offset $(BOARD_RAMDISK_OFFSET)
BOARD_MKBOOTIMG_ARGS += --tags_offset $(BOARD_KERNEL_TAGS_OFFSET)
BOARD_MKBOOTIMG_ARGS += --vendor_cmdline $(VENDOR_CMDLINE)

# Partitions - Blocks
ifeq ($(DEVICE_BUILD_FLAG),zumapro)
BOARD_FLASH_BLOCK_SIZE := 4096
else
BOARD_FLASH_BLOCK_SIZE := 131072
endif

# gs101: vendor_boot contains DLKM+DTB — must patch stock, not overwrite
ifeq ($(DEVICE_BUILD_FLAG),gs101)
VENDOR_BOOT_PATCH_STOCK := true
-include $(DEVICE_PATH)/custom_bootimg.mk
endif

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

GOOGLE_BOARD_PLATFORMS += $(DEVICE_BUILD_FLAG)
TARGET_BOARD_PLATFORM := $(DEVICE_BUILD_FLAG)
PRODUCT_PLATFORM := $(DEVICE_BUILD_FLAG)
TARGET_BOARD_PLATFORM_GPU := mali-g71
BOARD_VINTF_CHECK := false

# Properties
TARGET_VENDOR_PROP += $(DEVICE_PATH)/prebuilt/vendor.prop
TARGET_RECOVERY_FSTAB := $(DEVICE_PATH)/prebuilt/$(DEVICE_BUILD_FLAG)/recovery.fstab

# Recovery
TARGET_RECOVERY_PIXEL_FORMAT := ABGR_8888
TARGET_USERIMAGES_USE_EXT4 := true
TARGET_USERIMAGES_USE_F2FS := true
TARGET_USES_MKE2FS := true
RECOVERY_SDCARD_ON_DATA := true
TARGET_NO_RECOVERY := true
TARGET_RECOVERY_WIPE := $(DEVICE_PATH)/prebuilt/recovery.wipe
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
TW_MAX_BRIGHTNESS := 3827
TW_DEFAULT_BRIGHTNESS := 219
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

# TWRP Configuration - Vendor Modules (GKI only — monolithic kernels have all drivers built-in)
# ifneq ($(FOX_KERNEL_TYPE),non-gki)
# TW_LOAD_VENDOR_BOOT_MODULES := true
# TW_LOAD_VENDOR_MODULES_EXCLUDE_GKI := true
# endif



# Vendor Boot
BOARD_MOVE_RECOVERY_RESOURCES_TO_VENDOR_BOOT := true
BOARD_INCLUDE_RECOVERY_RAMDISK_IN_VENDOR_BOOT := true

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

BOARD_RECOVERY_IMAGE_PREPARE = bash $(DEVICE_PATH)/fox_build_callback.sh $(TARGET_RECOVERY_ROOT_OUT) --second-call

# Workaround
TARGET_COPY_OUT_VENDOR := vendor

earlycon=exynos4210,0x10870000 console=ttySAC0,115200 androidboot.console=ttySAC0 printk.devkmsg=on cma_sysfs.experimental=Y rcupdate.rcu_expedited=1 rcu_nocbs=all rcutree.enable_rcu_lazy swiotlb=noforce cgroup.memory=nokmem disable_dma32=on sysctl.kernel.sched_pelt_multiplier=4 kasan=off at24.write_timeout=100 log_buf_len=1024K fips140.load_sequential=1 exynos_drm.load_sequential=1 g2d.load_sequential=1 samsung_iommu_v9.load_sequential=1 bootconfig]
dyndbg="func alloc_contig_dump_pages +p" earlycon=exynos4210,0x10A00000 console=ttySAC0,115200 androidboot.console=ttySAC0 printk.devkmsg=on swiotlb=noforce cma_sysfs.experimental=Y cgroup_disable=memory rcupdate.rcu_expedited=1 androidboot.usbcontroller=11210000.dwc3 rcu_nocbs=all stack_depot_disable=off page_pinner=on swiotlb=1024 disable_dma32=on at24.write_timeout=100 log_buf_len=1024K bootconfig