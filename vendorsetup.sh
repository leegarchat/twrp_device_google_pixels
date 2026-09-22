#
#	This file is part of the OrangeFox Recovery Project
# 	Copyright (C) 2020-2026 The OrangeFox Recovery Project
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

# vendorsetup.sh — OrangeFox build variables for Pixel (Tensor G3/G4) Pixel family.
# This script is sourced by the OrangeFox build system after `lunch twrp_pixels-eng`.
# It exports all FOX_*, OF_*, TW_* environment variables that control the build.
# LGZ binaries are prebuilt (Rust, static musl) in include/ — verified below.
#
# FDEVICE must match the lunch target suffix and directory name under device/google/.
# Runtime device detection (shiba/husky/akita) is done in runatboot.sh via ro.hardware.

FDEVICE="pixels"

fox_get_target_device() {
chkdev=$(echo "$BASH_SOURCE" | grep -w $FDEVICE)
   if [ -n "$chkdev" ]; then
      FOX_BUILD_DEVICE="$FDEVICE"
   else
      chkdev=$(set | grep BASH_ARGV | grep -w $FDEVICE)
      [ -n "$chkdev" ] && FOX_BUILD_DEVICE="$FDEVICE"
   fi
}

if [ -z "$1" -a -z "$FOX_BUILD_DEVICE" ]; then
   fox_get_target_device
fi

if [ "$1" = "$FDEVICE" -o "$FOX_BUILD_DEVICE" = "$FDEVICE" ]; then

# --- Platform selection (discovered from families/ + devices/) ---
# New families appear here automatically: add families/<name>/ with
# family.conf, and optional devices/<codename>/device.conf entries.
PIXEL_TREE="$(gettop)/device/google/pixels"

# Comma list of DEVICE names whose device.conf maps to the given family.
_pixel_family_devices() {
    local fam="$1" devs="" conf
    for conf in "$PIXEL_TREE"/devices/*/device.conf; do
        [ -f "$conf" ] || continue
        local DEVICE="" FAMILY=""
        . "$conf"
        if [ "$FAMILY" = "$fam" ]; then
            devs="${devs}${devs:+,}$DEVICE"
        fi
    done
    printf '%s' "$devs"
}

if [ -n "${DEVICE_BUILD_FLAG:-}" ]; then
    echo ""
    echo "=============================================="
    echo "  DEVICE_BUILD_FLAG already set: $DEVICE_BUILD_FLAG"
    echo "  Skipping interactive menu."
    echo "=============================================="
else
echo ""
echo "=============================================="
echo "  Select target platform:"
_i=0
for _fam in $(for d in "$PIXEL_TREE"/families/*/; do basename "$d"; done | sort); do
    # families/common/ holds shared files, it is not a buildable family.
    [ "$_fam" = "common" ] && continue
    _i=$((_i + 1))
    eval "_FAM_$_i=\"$_fam\""
    echo "  $_i) $_fam ($(_pixel_family_devices "$_fam"))"
done
echo "=============================================="
printf "  Choice [1-%s] (timeout 15s, default: zuma): " "$_i"
if read -t 15 _platform_choice 2>/dev/null; then
    eval "_sel=\${_FAM_$_platform_choice:-}"
    if [ -n "$_sel" ] && [ -d "$PIXEL_TREE/families/$_sel" ]; then
        export DEVICE_BUILD_FLAG="$_sel"
    else
        export DEVICE_BUILD_FLAG="zuma"
    fi
else
    echo ""
    export DEVICE_BUILD_FLAG="zuma"
    echo "  Timeout — defaulting to zuma"
fi
fi
echo "=============================================="
echo "  Building for platform: $DEVICE_BUILD_FLAG"
echo "=============================================="

# --- Device list for this family (discovered + legacy quirks) ---
# Computed once here; reused for TARGET_DEVICE_ALT and .build_platform.conf.
# AIO covers every device in the tree (installer selects per target).
if [ "$DEVICE_BUILD_FLAG" = "aio" ]; then
    _ALL_DEVS="$(for conf in "$PIXEL_TREE"/devices/*/device.conf; do [ -f "$conf" ] || continue; _d=""; . "$conf"; printf '%s,' "$DEVICE"; done | sed 's/,$//')"
else
_FAM_DEVS="$(_pixel_family_devices "$DEVICE_BUILD_FLAG")"
_FAM_EXTRA="$(. "$PIXEL_TREE/families/$DEVICE_BUILD_FLAG/family.conf" 2>/dev/null; printf '%s' "${ALT_EXTRA:-}")"
_ALL_DEVS="$_FAM_DEVS${_FAM_EXTRA:+,$_FAM_EXTRA}"
fi

# --- Generate .build_platform.conf for build scripts (fox_build_callback.sh etc.) ---
# Environment variables don't survive make/ninja recipe shells reliably
# (ninja passes only an allowlisted env, so LGZ_LEVEL exported by build.sh
# would never arrive) — persist everything scripts need into this file.
# Family facts (UFS/earlycon) come from families/<fam>/family.conf;
# keymint HAL type (rust|cpp|both) comes from families/<fam>/family.json
# (single source of truth, validated here).
# AIO: nothing is baked — UFS/USBCTRL stay empty (installer domain),
# KEYMINT=both (both HALs ship, wrong one exits harmlessly at runtime).
# .build_platform.conf lives next to this file; gettop is valid here
# (vendorsetup runs after envsetup sets TOP).
_conf_file="$(gettop)/device/google/pixels/.build_platform.conf"
if [ "$DEVICE_BUILD_FLAG" = "aio" ]; then
    _FAM_UFS=""
    _FAM_USBCTRL=""
    _FAM_KEYMINT="both"
else
_FAM_UFS="$(. "$PIXEL_TREE/families/$DEVICE_BUILD_FLAG/family.conf" 2>/dev/null; printf '%s' "${UFS_ADDR:-}")"
# DWC3 USB controller (11210000.dwc3 on older Tensors, a210000.dwc3 on
# malibu). Empty = pre-USBCTRL family, keep the 11210000 default in the
# recovery rc files untouched.
_FAM_USBCTRL="$(. "$PIXEL_TREE/families/$DEVICE_BUILD_FLAG/family.conf" 2>/dev/null; printf '%s' "${USBCTRL:-}")"
_FAM_KEYMINT="$(python3 -c "import json,sys; print(json.load(open('$PIXEL_TREE/families/$DEVICE_BUILD_FLAG/family.json')).get('keymint',''))" 2>/dev/null)"
fi
case "$_FAM_KEYMINT" in
    rust|cpp|both) ;;
    *) echo "  ERROR: families/$DEVICE_BUILD_FLAG/family.json needs keymint 'rust', 'cpp' or 'both'"; return 1 ;;
esac
echo "  Keymint HAL: $_FAM_KEYMINT (from family.json)"
# LGZ cluster level from build.sh (-l/--level) or environment; validated 0-3.
_LGZ_LEVEL="${LGZ_LEVEL:-0}"
case "$_LGZ_LEVEL" in
    0|1|2|3) ;;
    *) echo "  WARNING: bad LGZ_LEVEL='$_LGZ_LEVEL', defaulting to 0"; _LGZ_LEVEL=0 ;;
esac
# Recovery-in-platform test layout (var2-AIO, build.sh --platform-recovery
# refreshes this key at build time; lunch-time default is empty = split).
_RECOVERY_IN_PLATFORM="${FOX_RECOVERY_IN_PLATFORM:-}"
# Reworked (wide-variant) theme, test-gated (build.sh --new-theme
# refreshes this key at build time; lunch-time default is empty = stock).
# Exported (not just conf) so Soong inherits it via the environment
# (orangefox_defaults.go Getenv) on manual lunch+build flows too.
_REWORK_THEME="${FOX_REWORK_THEME:-}"
if [[ -n "$_REWORK_THEME" ]]; then
    export FOX_REWORK_THEME="$_REWORK_THEME"
fi
# First-stage kill-switch (build.sh -N refreshes this key at build time;
# lunch-time default is empty = first-stage included).
_NO_FIRST_STAGE="${FOX_NO_FIRST_STAGE:-}"
cat > "$_conf_file" <<_PLATFORM_EOF
# Auto-generated by vendorsetup.sh — DO NOT EDIT
# Re-generated on every lunch / vendorsetup.sh source
# Sourced by fox_build_callback.sh and other build scripts
PLATFORM=${DEVICE_BUILD_FLAG}
FAMILY=${DEVICE_BUILD_FLAG}
UFS_ADDR=${_FAM_UFS}
USBCTRL=${_FAM_USBCTRL}
KEYMINT=${_FAM_KEYMINT}
FAMILY_DEVICES=${_ALL_DEVS:-}
LGZ_LEVEL=${_LGZ_LEVEL}
RECOVERY_IN_PLATFORM=${_RECOVERY_IN_PLATFORM}
NO_FIRST_STAGE=${_NO_FIRST_STAGE}
REWORK_THEME=${_REWORK_THEME}
_PLATFORM_EOF
echo "  Wrote $_conf_file"

# --- Version & Build ---
# Build type override: build.sh --build-type TYPE exports FOX_BUILD_TYPE
# before lunch; otherwise default to Stable. Only exact `Stable` enables
# OF_ADVANCED_SECURITY downstream (orangefox.mk) — any other value builds
# a non-Secure image (adbd up at boot, MTP autostart).
: "${FOX_BUILD_TYPE:=Stable}"
export FOX_BUILD_TYPE
export FOX_VARIANT=default
export OF_MAINTAINER=LeeGarChat
export USE_CCACHE="1"
export TARGET_ARCH="arm64"
export LC_ALL="C"

# --- Device type ---
export FOX_VIRTUAL_AB_DEVICE=1
export FOX_AB_DEVICE=1
export FOX_VENDOR_BOOT_RECOVERY=1
if [ "$DEVICE_BUILD_FLAG" = "gs201" ] || [ "$DEVICE_BUILD_FLAG" = "gs101" ]; then
    export FOX_RECOVERY_VENDOR_BOOT_PARTITION="/dev/block/platform/14700000.ufs/by-name/vendor_boot"
elif [ "$DEVICE_BUILD_FLAG" = "malibu" ]; then
    export FOX_RECOVERY_VENDOR_BOOT_PARTITION="/dev/block/platform/3c2d0000.ufs/by-name/vendor_boot"
elif [ "$DEVICE_BUILD_FLAG" = "laguna" ]; then
    export FOX_RECOVERY_VENDOR_BOOT_PARTITION="/dev/block/platform/3c400000.ufs/by-name/vendor_boot"
else
    export FOX_RECOVERY_VENDOR_BOOT_PARTITION="/dev/block/platform/13200000.ufs/by-name/vendor_boot"
fi

# --- Vanilla build (non-Xiaomi device, skip MIUI patches) ---
export FOX_VANILLA_BUILD=1
export OF_DISABLE_MIUI_SPECIFIC_FEATURES=1

# --- Multi-device support (discovered, see _ALL_DEVS above) ---
export TARGET_DEVICE_ALT="$_ALL_DEVS"
export FOX_TARGET_DEVICES="$_ALL_DEVS"
# --- OrangeFox UI ---
# OF_SCREEN_H is the compile-time DEFAULT screen height for theme scaling.
# Devices with different screen heights (e.g. husky=2244) override this at
# runtime via the DOF_SCREEN_H property set in runatboot.sh → data.cpp reads it.
export OF_SCREEN_H=2400
if [ "$DEVICE_BUILD_FLAG" = "zumapro" ]; then
    export OF_STATUS_H=150
elif [ "$DEVICE_BUILD_FLAG" = "gs201" ]; then
    export OF_STATUS_H=130
elif [ "$DEVICE_BUILD_FLAG" = "gs101" ]; then
    export OF_STATUS_H=130
else
    export OF_STATUS_H=130
fi
export OF_STATUS_INDENT_LEFT=80
export OF_STATUS_INDENT_RIGHT=80
export OF_HIDE_NOTCH=1
export OF_CLOCK_POS=1
export OF_ALLOW_DISABLE_NAVBAR=0
export OF_OPTIONS_LIST_NUM=6

# --- Compression ---
export OF_USE_LZ4_COMPRESSION=1

# --- Feature flags ---
export OF_NO_TREBLE_COMPATIBILITY_CHECK=1
export OF_ENABLE_LPTOOLS=1
export OF_USE_GREEN_LED=0
export OF_NO_SPLASH_CHANGE=1
export OF_RECOVERY_AB_FULL_REFLASH_RAMDISK=1

# --- dm-crypt teardown before /data format (metadata encryption) ---
# Without this, /dev/block/mapper/userdata stays open after unmount,
# and make_f2fs fails with "Error: In use by the system!" on the raw device.
# Uses AOSP dmctl (built from source) — preferred over prebuilt dmsetup.
export OF_USE_DMCTL=1

# --- Flashlight (Pixel 8: LM3644 torch via I2C, controlled by script) ---
export OF_FL_PATH1="cmd:/system/bin/recovery-pixel-boot torch"

# --- Encryption (FBE metadata decryption via Trusty TEE KeyMint) ---
# Do not force legacy Keymaster on Tensor 3. Recovery must use AIDL KeyMint.
unset OF_DEFAULT_KEYMASTER_VERSION
# export OF_DEFAULT_KEYMASTER_VERSION="4.1"
# OF_SKIP_FBE_DECRYPTION is controlled in BoardConfig during staged bring-up.


# --- Binaries & Tools ---
export FOX_REPLACE_TOOLBOX_GETPROP=1
export FOX_USE_BASH_SHELL=1
export FOX_BASH_TO_SYSTEM_BIN=1
# export FOX_ASH_IS_BASH=1
# export FOX_USE_TAR_BINARY=1
# export FOX_USE_SED_BINARY=1
# export FOX_USE_XZ_UTILS=1
# export FOX_USE_LZ4_BINARY=1
# export FOX_USE_ZSTD_BINARY=1
# export FOX_USE_UPDATED_MAGISKBOOT=1
# export OF_USE_MAGISKBOOT_FOR_ALL_PATCHES=1

# --- App & Features ---
export FOX_ENABLE_APP_MANAGER=1
export FOX_DELETE_AROMAFM=1
# Magisk root zip is out of the build (decrypt is native, size matters):
# vendor skips Magisk.zip/uninstall.zip copy and neuters the addon UI.
# magiskboot TOOL support (TW_INCLUDE_REPACKTOOLS) is untouched — 3rd-party
# zips still flash fine.
export FOX_DELETE_MAGISK_ADDON=1

# --- Storage ---
export OF_QUICK_BACKUP_LIST="/boot;/vendor_boot;/data;"
export OF_UNBIND_SDCARD_F2FS=1
export OF_BIND_MOUNT_SDCARD_ON_FORMAT=1
export OF_DYNAMIC_FULL_SIZE=8531214336

# --- Battery ---
export OF_USE_LEGACY_BATTERY_SERVICES=1

# --- Logging ---
export OF_DONT_KEEP_LOG_HISTORY=0
export FOX_INSTALLER_DISABLE_AUTOREBOOT=0
export FOX_USE_DATA_RECOVERY_FOR_SETTINGS=0


# --- KernelSU ---
export FOX_ENABLE_KERNELSU_SUPPORT=1
export FOX_ENABLE_KERNELSU_NEXT_SUPPORT=1

# --- Size reduction: theme/font cleanup handled in fox_build_callback.sh ---
export FOX_DELETE_INITD_ADDON=1

# --- Custom build callback: LGZ cluster pack, platform injection (fox_build_callback.sh) ---
export FOX_LOCAL_CALLBACK_SCRIPT="$(gettop)/device/google/pixels/include/prebuilt/fox_build_callback.sh"

# --- LGZ (Rust): prebuilt static binaries, no compilation ---
# Host compressor (x86_64, full flavor) and device decompressor (arm64,
# lean flavor) live in include/prebuilt/. The callback installs the latter as
# /system/bin/lgz and packs the ramdisk with the former.
LGZ_HOST_BIN="$(gettop)/device/google/pixels/include/prebuilt/lgz_compress_full_x64"
LGZ_DEVICE_BIN="$(gettop)/device/google/pixels/include/prebuilt/lgz_compress_lean_arm64"
if [ -x "$LGZ_HOST_BIN" ]; then
    echo "[LGZ]   Host compressor OK: $LGZ_HOST_BIN"
else
    echo "[LGZ]   ERROR: host compressor missing/not executable: $LGZ_HOST_BIN"
fi
if [ -f "$LGZ_DEVICE_BIN" ]; then
    echo "[LGZ]   Device decompressor OK: $LGZ_DEVICE_BIN"
else
    echo "[LGZ]   ERROR: device decompressor missing: $LGZ_DEVICE_BIN"
fi

    export | grep "FOX"
    export | grep "OF_"
    export | grep "TARGET_"
    export | grep "TW_"
	# let's see what are our build VARs
	if [ -n "$FOX_BUILD_LOG_FILE" -a -f "$FOX_BUILD_LOG_FILE" ]; then
  	   export | grep "FOX" >> $FOX_BUILD_LOG_FILE
  	   export | grep "OF_" >> $FOX_BUILD_LOG_FILE
   	   export | grep "TARGET_" >> $FOX_BUILD_LOG_FILE
  	   export | grep "TW_" >> $FOX_BUILD_LOG_FILE
 	fi
fi
