# Copyright (C) 2025-2026 The OrangeFox Recovery Project
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Custom vendor_boot build rules for Pixel devices.
#
# gs201/zuma/zumapro: Standard AOSP vendor_boot build (recovery ramdisk in vendor_boot,
#   DLKM/DTB in separate vendor_kernel_boot partition). No custom rules needed.
#
# gs101 (Pixel 6 series): SUPERSEDED. The stock-patching mode below is kept
# for reference only — nothing enables VENDOR_BOOT_PATCH_STOCK anymore.
# Current gs101 flow (see families/gs101/family.mk): single platform fragment
# (first-stage + recovery merged, stock .ko bundled inside), no dtb/dlkm
# fragments; build.sh extracts the platform ramdisk as lz4_legacy for
# `fastboot flash vendor_boot:default`. The device keeps its own dtb/dlkm.

VENDOR_BOOT_PATCH_DIR := $(PRODUCT_OUT)/vendor_boot_patch
FOX_MAGISKBOOT ?= $(PWD)/vendor/recovery/tools/magiskboot

ifeq ($(VENDOR_BOOT_PATCH_STOCK),true)
# --- gs101 mode: patch stock vendor_boot, replace only recovery ramdisk fragment ---

VENDOR_BOOT_STOCK ?= $(PWD)/$(DEVICE_PATH)/families/gs101/vendor_boot_stock.img

ifdef BUILDING_VENDOR_BOOT_IMAGE
$(INSTALLED_VENDOR_BOOTIMAGE_TARGET): $(recovery_uncompressed_ramdisk) $(FOX_MAGISKBOOT)
	$(call pretty,"Target vendor_boot image: $@ (stock-patched, gs101 mode)")
	@if [ ! -f "$(VENDOR_BOOT_STOCK)" ]; then \
		echo "ERROR: gs101 mode requires stock vendor_boot at $(VENDOR_BOOT_STOCK)"; \
		echo "Place the stock vendor_boot.img for the target device there and retry."; \
		exit 1; \
	fi
	@rm -rf "$(VENDOR_BOOT_PATCH_DIR)"
	@mkdir -p "$(VENDOR_BOOT_PATCH_DIR)"
	@cp -f "$(VENDOR_BOOT_STOCK)" "$(VENDOR_BOOT_PATCH_DIR)/stock.img"
	@cd "$(VENDOR_BOOT_PATCH_DIR)" && "$(FOX_MAGISKBOOT)" unpack -n stock.img
	@if [ -f "$(VENDOR_BOOT_PATCH_DIR)/vendor_ramdisk/recovery.cpio" ]; then \
		cp -f "$(recovery_uncompressed_ramdisk)" \
			"$(VENDOR_BOOT_PATCH_DIR)/vendor_ramdisk/recovery.cpio"; \
	else \
		cp -f "$(recovery_uncompressed_ramdisk)" \
			"$(VENDOR_BOOT_PATCH_DIR)/vendor_ramdisk_recovery.cpio"; \
	fi
	@cd "$(VENDOR_BOOT_PATCH_DIR)" && "$(FOX_MAGISKBOOT)" repack stock.img new_vendor_boot.img
	@cp -f "$(VENDOR_BOOT_PATCH_DIR)/new_vendor_boot.img" "$@"
	$(call assert-max-image-size,$@,$(BOARD_VENDOR_BOOTIMAGE_PARTITION_SIZE))

# OrangeFox post-processing (zip creation, etc.)
ifneq ($(NOT_ORANGEFOX),1)
	$(BASH) $(FOX_VENDOR) FOX_VENDOR_CMD="Fox_After_Recovery_Image" \
	FOX_MANIFEST_VER="14.1" \
	BOARD_BOOT_HEADER_VERSION="$(BOARD_BOOT_HEADER_VERSION)" \
	TARGET_ARCH="$(TARGET_ARCH)" \
	TARGET_RECOVERY_ROOT_OUT="$(TARGET_RECOVERY_ROOT_OUT)" \
	TARGET_VENDOR_RAMDISK_OUT="$(TARGET_VENDOR_RAMDISK_OUT)" \
	MKBOOTIMG="$(MKBOOTIMG)" \
	MKBOOTFS="$(MKBOOTFS)" \
	INTERNAL_RECOVERYIMAGE_ARGS='"$(INTERNAL_RECOVERYIMAGE_ARGS)"' \
	INTERNAL_MKBOOTIMG_VERSION_ARGS="$(INTERNAL_MKBOOTIMG_VERSION_ARGS)" \
	BOARD_MKBOOTIMG_ARGS='"$(BOARD_MKBOOTIMG_ARGS)"' \
	TARGET_OUT="$(TARGET_OUT)" \
	RECOVERY_RAMDISK_COMPRESSOR="$(RECOVERY_RAMDISK_COMPRESSOR)" \
	INSTALLED_RECOVERYIMAGE_TARGET="$(INSTALLED_RECOVERYIMAGE_TARGET)" \
	INSTALLED_BOOTIMAGE_TARGET="$(INSTALLED_BOOTIMAGE_TARGET)" \
	BOARD_BOOTIMAGE_PARTITION_SIZE=$(BOARD_BOOTIMAGE_PARTITION_SIZE) \
	BOARD_RECOVERYIMAGE_PARTITION_SIZE=$(BOARD_RECOVERYIMAGE_PARTITION_SIZE) \
	BOARD_USES_RECOVERY_AS_BOOT=$(BOARD_USES_RECOVERY_AS_BOOT) \
	INTERNAL_KERNEL_CMDLINE="$(INTERNAL_KERNEL_CMDLINE)" \
	vendor_ramdisk="$(INTERNAL_VENDOR_RAMDISK_TARGET)" \
	INSTALLED_VENDOR_BOOTIMAGE_TARGET="$(INSTALLED_VENDOR_BOOTIMAGE_TARGET)" \
	BOARD_VENDOR_BOOTIMAGE_PARTITION_SIZE=$(BOARD_VENDOR_BOOTIMAGE_PARTITION_SIZE) \
	INTERNAL_VENDOR_BOOTIMAGE_ARGS='"$(INTERNAL_VENDOR_BOOTIMAGE_ARGS)"' \
	BOARD_INCLUDE_RECOVERY_RAMDISK_IN_VENDOR_BOOT=$(BOARD_INCLUDE_RECOVERY_RAMDISK_IN_VENDOR_BOOT) \
	BOARD_MOVE_RECOVERY_RESOURCES_TO_VENDOR_BOOT=$(BOARD_MOVE_RECOVERY_RESOURCES_TO_VENDOR_BOOT) \
	recovery_ramdisk="$(recovery_ramdisk)"
endif
endif

endif
# --- end gs101 stock-patching mode ---
# For gs201/zuma/zumapro: no custom rules — AOSP default vendor_boot build is used.
