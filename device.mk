#
# Copyright (C) 2024-2026 The OrangeFox Recovery Project
#
# SPDX-License-Identifier: GPL-3.0-or-later
#

# device.mk — Package list, crypto config, and build props for Tensor-based Pixels.
# Covers gs201 (Tensor G2), zuma (Tensor G3), zumapro (Tensor G4), laguna (Tensor G5).
# Custom recovery modules (weaver, storageproxyd, etc.) are built from selfcode/.

LOCAL_PATH := device/google/pixels

# Enable virtual A/B OTA
$(call inherit-product, $(SRC_TARGET_DIR)/product/virtual_ab_ota/compression.mk)

# API & VNDK
PRODUCT_SHIPPING_API_LEVEL := 34
PRODUCT_TARGET_VNDK_VERSION := 34

# Dynamic Partitions
PRODUCT_USE_DYNAMIC_PARTITIONS := true

# Boot control HAL (Pixel-specific implementation)
PRODUCT_PACKAGES += \
    android.hardware.boot@1.2-service-pixel \
    android.hardware.boot@1.2-impl-pixel

# Core packages
PRODUCT_PACKAGES += \
    fastbootd \
    update_engine \
    update_engine_sideload \
    update_verifier

# Vendor services
PRODUCT_PACKAGES += \
    vndservicemanager \
    vndservice \
    bootctl

# Libraries
PRODUCT_PACKAGES += \
    libtrusty \
    libsysutils \
    libhidltransport.vendor

RECOVERY_LIBRARY_SOURCE_FILES += \
    $(TARGET_OUT_SHARED_LIBRARIES)/libsysutils.so

TARGET_RECOVERY_DEVICE_MODULES += libion
RECOVERY_LIBRARY_SOURCE_FILES += \
    $(TARGET_OUT_SHARED_LIBRARIES)/libion.so

# Crypto: FBE metadata decryption via Trusty TEE KeyMint
PRODUCT_PROPERTY_OVERRIDES += \
    ro.hardware.keystore=trusty \
    ro.hardware.gatekeeper=trusty

# Metadata
BOARD_USES_METADATA_PARTITION := true

# Virtual A/B
ENABLE_VIRTUAL_AB := true

# Build properties — defaults to shiba fingerprint, overridden per-device at runtime by runatboot.sh
PRODUCT_BUILD_PROP_OVERRIDES += \
    BuildDesc="shiba-user 15 AP3A.241005.015 12366759 release-keys" \
    BuildFingerprint=google/shiba/shiba:15/AP3A.241005.015/12366759:user/release-keys \
    DeviceProduct=shiba

PRODUCT_SOONG_NAMESPACES += $(LOCAL_PATH)

# Ramdisk snapshot tool (copies ramdisk state before LGZ decompression)
PRODUCT_PACKAGES += \
    ramdisk_snapshot

# Persistent storage proxy for Trusty TEE RPMB (needed before keymint)
PRODUCT_PACKAGES += \
    recovery_storageproxyd

# A14-native Weaver HAL proxy (talks to Titan M2 via /dev/gsc0 directly)
PRODUCT_PACKAGES += \
    recovery_weaver

# gs201/gs101 Trusty TA speaks Keymaster 4.0 (not KeyMint AIDL) — the AOSP C++ binary
# auto-negotiates via GetVersion fallback. Build it so callback can swap it in.
ifneq (,$(filter gs201 gs101,$(DEVICE_BUILD_FLAG)))
PRODUCT_PACKAGES += android.hardware.security.keymint-service.trusty
endif


# Firstage ramdisk packages
# conf-zuma/Android.bp    → fstab.zuma* from fstab.zuma.in        (Tensor G3, UFS 13200000)
# conf-zumapro/f2fs/      → fstab.zumapro* from modular sources   (Tensor G4, UFS 13200000)
# conf-gs201/Android.bp   → fstab.gs201* from fstab.gs201.in      (Tensor G2, UFS 14700000)
# conf-laguna/f2fs/       → fstab.laguna* from modular sources    (Tensor G5, UFS 3c400000)
# conf-gs101/             → future: fstab.gs101* (Tensor G1, UFS 14700000 — same as gs201)
ifeq ($(DEVICE_BUILD_FLAG),zumapro)
PRODUCT_PACKAGES += fstab.zumapro.vendor_ramdisk
PRODUCT_PACKAGES += fstab.zumapro-fips.vendor_ramdisk
PRODUCT_PACKAGES += fstab.zuma.f2fs.vendor_ramdisk
PRODUCT_PACKAGES += fstab.zuma-fips.f2fs.vendor_ramdisk
else ifeq ($(DEVICE_BUILD_FLAG),laguna)
PRODUCT_PACKAGES += fstab.laguna.vendor_ramdisk
PRODUCT_PACKAGES += fstab.laguna-fips.vendor_ramdisk
else ifeq ($(DEVICE_BUILD_FLAG),gs201)
PRODUCT_PACKAGES += fstab.gs201.vendor_ramdisk
PRODUCT_PACKAGES += fstab.gs201-fips.vendor_ramdisk
else ifeq ($(DEVICE_BUILD_FLAG),gs101)
# gs101 uses same UFS address (14700000) as gs201 — reuse gs201 fstab for now.
PRODUCT_PACKAGES += fstab.gs201.vendor_ramdisk
PRODUCT_PACKAGES += fstab.gs201-fips.vendor_ramdisk
else
PRODUCT_PACKAGES += fstab.zuma.vendor_ramdisk
PRODUCT_PACKAGES += fstab.zuma-fips.vendor_ramdisk
endif

# service \
# 	strace \

PRODUCT_PACKAGES += \
    linker.vendor_ramdisk \
    resize2fs.vendor_ramdisk \
    resize.f2fs.vendor_ramdisk \
    dump.f2fs.vendor_ramdisk \
    fsck.vendor_ramdisk \
    tune2fs.vendor_ramdisk \
    e2fsck.vendor_ramdisk
