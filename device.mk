#
# Copyright (C) 2024-2026 The OrangeFox Recovery Project
#
# SPDX-License-Identifier: GPL-3.0-or-later
#

# device.mk — Package list, crypto config, and build props (AIO-only).
# One universal payload for all Tensor Pixels; family specifics resolve at
# runtime. Custom recovery modules are built from include/ (Rust).

LOCAL_PATH := device/google/pixels

# Enable virtual A/B OTA
$(call inherit-product, $(SRC_TARGET_DIR)/product/virtual_ab_ota/compression.mk)

# API & VNDK
PRODUCT_SHIPPING_API_LEVEL := 34
PRODUCT_TARGET_VNDK_VERSION := 34

# Dynamic Partitions
PRODUCT_USE_DYNAMIC_PARTITIONS := true

# Recovery ramdisk overlays: the common root/ FIRST, then every device +
# every family overlay in one universal cpio. The installer selects family
# files post-unpack (include/aio/aio_swap.sh); the rest resolves at runtime
# via ro.hardware (recovery-pixel-boot).
# NOTE: keep $(LOCAL_PATH) first: build/make uses TARGET_RECOVERY_DEVICE_DIRS
# *instead of* (not in addition to) TARGET_DEVICE_DIR/recovery/root.
TARGET_RECOVERY_DEVICE_DIRS := $(LOCAL_PATH)
TARGET_RECOVERY_DEVICE_DIRS += $(wildcard $(LOCAL_PATH)/devices/*)
TARGET_RECOVERY_DEVICE_DIRS += $(wildcard $(LOCAL_PATH)/families/*)

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
# /system/bin/init (real init rides inside the cluster as fox.init).
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

# KeyMint HALs from source (AIO-only: both always ship in the universal
# payload). Rust (system/core/trusty/keymint) talks KeyMint AIDL to the
# Trusty TA; C++ (system/core/trusty/keymaster) auto-negotiates via
# GetVersion fallback for Keymaster 4.0 TAs. No prebuilt blobs.
# Both services start at boot; the wrong one exits harmlessly (or the
# installer pre-selects via ro.recovery.keymint).
PRODUCT_PACKAGES += android.hardware.security.keymint-service.trusty
PRODUCT_PACKAGES += android.hardware.security.keymint-service.rust.trusty


# AIO-only: first-stage (vendor_ramdisk) components are always skipped
# (build.sh exports FOX_NO_FIRST_STAGE=1; stock first_stage is preserved by
# the installer). No fstab.*.vendor_ramdisk, no linker/e2fs vendor_ramdisk
# tools. The recovery ramdisk is unaffected.
