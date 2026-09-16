#
# Copyright (C) 2024-2026 The OrangeFox Recovery Project
#
# SPDX-License-Identifier: GPL-3.0-or-later
#

# device.mk — Package list, crypto config, and build props for Tensor-based Pixels.
# Covers gs201 (Tensor G2), zuma (Tensor G3), zumapro (Tensor G4).
# Custom recovery modules are built from include/ (Rust).

LOCAL_PATH := device/google/pixels

# Enable virtual A/B OTA
$(call inherit-product, $(SRC_TARGET_DIR)/product/virtual_ab_ota/compression.mk)

# API & VNDK
PRODUCT_SHIPPING_API_LEVEL := 34
PRODUCT_TARGET_VNDK_VERSION := 34

# Dynamic Partitions
PRODUCT_USE_DYNAMIC_PARTITIONS := true

# Recovery ramdisk overlays: the common root/ FIRST, then per-device
# overlays SCOPED TO THE CURRENT FAMILY (a zuma build must not ship
# akita/tokay rc files). Per-device init stubs live in
# devices/<codename>/recovery/root/; family comes from each
# devices/<codename>/device.conf, so adding a device needs no mk edit.
# Family stubs live in families/<fam>/recovery/root/; build.sh exports
# DEVICE_BUILD_FLAG, so append exactly one family overlay.
# NOTE: keep $(LOCAL_PATH) first: build/make uses TARGET_RECOVERY_DEVICE_DIRS
# *instead of* (not in addition to) TARGET_DEVICE_DIR/recovery/root.
_pixel_dev_family = $(shell . $(LOCAL_PATH)/devices/$(1)/device.conf 2>/dev/null; printf '%s' "$$FAMILY")
TARGET_RECOVERY_DEVICE_DIRS := $(LOCAL_PATH)
ifeq ($(DEVICE_BUILD_FLAG),)
$(warning pixels: DEVICE_BUILD_FLAG empty, family scoping off - all device overlays included)
TARGET_RECOVERY_DEVICE_DIRS += $(wildcard $(LOCAL_PATH)/devices/*)
else
TARGET_RECOVERY_DEVICE_DIRS += $(foreach d,$(notdir $(wildcard $(LOCAL_PATH)/devices/*)),$(if $(filter $(DEVICE_BUILD_FLAG),$(call _pixel_dev_family,$(d))),$(LOCAL_PATH)/devices/$(d)))
TARGET_RECOVERY_DEVICE_DIRS += $(LOCAL_PATH)/families/$(DEVICE_BUILD_FLAG)
endif

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
# PRODUCT_PACKAGES += \
#     ramdisk_snapshot

# Static PID 1 stub: unpacks the LGZ cluster, then execs the real init.
# Installed as recovery_init_stub; the build callback swaps it over
# /system/bin/init (real init rides inside the cluster as init.real).
PRODUCT_PACKAGES += \
    recovery_init_stub

# FBE KDF playground: external helper for V4 synthetic-password research.
# Called by vold Decrypt with pipelines from fox_kdf.conf; static so it
# also runs standalone via adb for fast iteration without rebuilds.
PRODUCT_PACKAGES += \
    fox_fbe_kdf

# Unified Tensor daemon (Rust multicall): Trusty storage proxy (RPMB/UFS)
# + Titan Mx Weaver HAL proxy. Replaces the former C daemons
# recovery_storageproxyd and recovery_weaver.
PRODUCT_PACKAGES += \
    recovery-tensor-daemon \
    recovery-pixel-boot

# KeyMint HAL from source, per-family type from families/*/family.json
# `keymint` (rust|cpp), delivered as FOX_KEYMINT_TYPE by build.sh.
# Rust (zuma/zumapro, system/core/trusty/keymint) talks KeyMint AIDL to the
# Trusty TA; C++ (gs201/gs101, system/core/trusty/keymaster) auto-negotiates
# via GetVersion fallback for Keymaster 4.0 TAs. No prebuilt blobs.
ifeq ($(FOX_KEYMINT_TYPE),cpp)
PRODUCT_PACKAGES += android.hardware.security.keymint-service.trusty
else ifeq ($(FOX_KEYMINT_TYPE),rust)
PRODUCT_PACKAGES += android.hardware.security.keymint-service.rust.trusty
else
# FOX_KEYMINT_TYPE empty/unknown (manual lunch without build.sh): fall back
# to the family-name mapping.
ifneq (,$(filter gs201 gs101,$(DEVICE_BUILD_FLAG)))
PRODUCT_PACKAGES += android.hardware.security.keymint-service.trusty
else
PRODUCT_PACKAGES += android.hardware.security.keymint-service.rust.trusty
endif
endif


# Firstage ramdisk packages — pre-rendered plain fstabs, no BP codegen.
# Module names are historical (PRODUCT_PACKAGES unchanged); sources live in
# families/<fam>/fstab/ as static prebuilt_etc (rendered once from the old
# vendor-ref/conf-* templates with identical sed substitutions).
# families/zuma/fstab/    → fstab.zuma*                             (Tensor G3, UFS 13200000)
# families/zumapro/fstab/ → fstab.zumapro* + f2fs-flavored fstab.zuma* (Tensor G4, UFS 13200000)
# families/gs201/fstab/   → fstab.gs201*                            (Tensor G2, UFS 14700000)
# gs101                   → reuses gs201 fstab (Tensor G1, UFS 14700000 — same as gs201)
ifeq ($(DEVICE_BUILD_FLAG),zumapro)
PRODUCT_PACKAGES += fstab.zumapro.vendor_ramdisk
PRODUCT_PACKAGES += fstab.zumapro-fips.vendor_ramdisk
PRODUCT_PACKAGES += fstab.zuma.f2fs.vendor_ramdisk
PRODUCT_PACKAGES += fstab.zuma-fips.f2fs.vendor_ramdisk
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
