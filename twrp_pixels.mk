#
# Copyright (C) 2024-2026 The OrangeFox Recovery Project
#
# SPDX-License-Identifier: GPL-3.0-or-later
#

# twrp_pixels.mk — Product definition for OrangeFox Recovery on all Tensor Pixels.
# Builds a single universal (AIO) payload: stock kernel is kept, only the
# recovery ramdisk cpio is delivered and packed into the installer zip.
# Family specifics resolve at runtime (stub + recovery-pixel-boot).
#
# PRODUCT_DEVICE must match the directory name under device/google/ (pixels)
# so that the build system finds BoardConfig.mk and device.mk correctly.
# Runtime device identification is done via ro.hardware in
# recovery-pixel-boot (config in /pixelrunatboot.json).

# Inherit from those products. Most specific first.
$(call inherit-product, $(SRC_TARGET_DIR)/product/base.mk)
$(call inherit-product, $(SRC_TARGET_DIR)/product/core_64_bit_only.mk)
$(call inherit-product, $(SRC_TARGET_DIR)/product/virtual_ab_ota.mk)
$(call inherit-product, $(SRC_TARGET_DIR)/product/virtual_ab_ota/launch_with_vendor_ramdisk.mk)
$(call inherit-product, $(SRC_TARGET_DIR)/product/emulated_storage.mk)
$(call inherit-product, $(SRC_TARGET_DIR)/product/generic_ramdisk.mk)
$(call inherit-product, $(SRC_TARGET_DIR)/product/developer_gsi_keys.mk)
$(call inherit-product, $(SRC_TARGET_DIR)/product/updatable_apex.mk)

# Inherit some common TWRP
$(call inherit-product, vendor/twrp/config/common.mk)

# Inherit from pixels device tree
$(call inherit-product, device/google/pixels/device.mk)

# Product Name — "pixels" is a universal target covering all Tensor SoC Pixels.
# The recovery image auto-detects the device at runtime via ro.hardware.
PRODUCT_RELEASE_NAME := pixels
PRODUCT_DEVICE := $(PRODUCT_RELEASE_NAME)
PRODUCT_NAME := twrp_$(PRODUCT_RELEASE_NAME)
PRODUCT_BRAND := google
PRODUCT_MODEL := Pixel Series
PRODUCT_MANUFACTURER := Google
PRODUCT_GMS_CLIENTID_BASE := android-google
