#!/bin/sh
#
#   runatboot.sh — OrangeFox Recovery early-boot script for Zuma SoC Pixels.
#
#   This file is part of the OrangeFox Recovery Project
#   Copyright (C) 2024-2026 The OrangeFox Recovery Project
#
#   OrangeFox is free software: you can redistribute it and/or modify
#   it under the terms of the GNU General Public License as published by
#   the Free Software Foundation, either version 3 of the License, or
#   any later version.
#
#   Please maintain this if you use this script or any part of it
#
# Executed after runatinit.sh (which handles device identity and props).
# Responsible for:
#   1. Device detection (ro.hardware → module list per device)
#   2. A/B slot detection and vendor_dlkm module loading (touch, haptics)
#   3. Vendor firmware copy (CS40L26 haptics) with slot fallback
#   4. Magisk binary extraction and link creation
#

slot_detect() {
    suffix=$(getprop ro.boot.slot_suffix)
    if [ -z "$suffix" ]; then
        suffix=$(bootctl get-current-slot | xargs bootctl get-suffix 2>/dev/null)
    fi
    case "$suffix" in
        _a) 
            unsuffix=_b
            slot=0
            unslot=1
            ;;
        _b) 
            unsuffix=_a 
            slot=1
            unslot=0
            ;;
    esac
}

modules_touch_install() {
    mkdir -p /dev/modules_inject/vendor_dlkm_a /dev/modules_inject/vendor_dlkm_b

    try_load_modules_from_path() {
        local path="$1"
        local loaded_any=0
        local missing_modules=""
        
        for module in $modules_touch; do
            files_finded=$(find "$path" 2>/dev/null | grep "${module}.ko$")
            if [ -z "$files_finded" ]; then
                missing_modules="$missing_modules $module"
                continue
            fi
            
            for f in $files_finded; do
                if insmod "$f" 2>>"$LOGF"; then
                    echo "I:modules: $module loaded successfully from $f" >> "$LOGF"
                    loaded_any=1
                else
                    echo "E:modules: Cannot load $module from $f" >> "$LOGF"
                    missing_modules="$missing_modules $module"
                fi
            done
        done
        
        echo "$missing_modules"
        return $loaded_any
    }

    check_modules_loaded() {
        local missing=""
        for module in $modules_touch; do
            local mod_name=$(echo "$module" | tr '-' '_')
            if ! lsmod | grep -q "$mod_name"; then
                missing="$missing $module"
            fi
        done
        if [ -n "$missing" ]; then
            echo "E:modules: Missing modules: $missing" >> "$LOGF"
            return 1
        else
            echo "I:modules: All modules loaded successfully" >> "$LOGF"
            return 0
        fi
    }

    try_slot() {
        local blk="$1"
        local mnt="$2"
        local slot_name="$3"
        local slot_num="$4"

        if [ ! -b "$blk" ]; then
            echo "W:modules: $blk not found, trying to map..." >> "$LOGF"
            if ! lptools_new --slot "$slot_num" --suffix "$slot_name" --map "vendor_dlkm$slot_name" ; then
                echo "E:modules: Failed to map $blk" >> "$LOGF"
                return 1
            fi
        fi

        if mount -r "$blk" "$mnt"; then
            echo "I:modules: Mounted $blk on $mnt" >> "$LOGF"
            missing=$(try_load_modules_from_path "$mnt")
            umount "$mnt"
            echo "I:modules: Unmounted $mnt" >> "$LOGF"
            [ -z "$missing" ] && return 0
            echo "W:modules: Missing modules after $slot_name slot attempt: $missing" >> "$LOGF"
            return 1
        else
            echo "E:modules: Cannot mount $blk" >> "$LOGF"
            return 1
        fi
    }

    echo "I:modules: Trying current slot $suffix" >> "$LOGF"
    try_slot "/dev/block/mapper/vendor_dlkm$suffix" "/dev/modules_inject/vendor_dlkm$suffix" "$suffix" "$slot"
    res=$?

    if [ $res -ne 0 ]; then
        echo "I:modules: Trying opposite slot $unsuffix" >> "$LOGF"
        try_slot "/dev/block/mapper/vendor_dlkm$unsuffix" "/dev/modules_inject/vendor_dlkm$unsuffix" "$unsuffix" "$unslot"
    fi

    if ! check_modules_loaded; then
        echo "I:modules: Trying fallback /system/modules_touch" >> "$LOGF"
        missing=$(try_load_modules_from_path "/system/modules_touch")
        if ! check_modules_loaded; then
            echo "E:modules: Final failure, modules still missing: $missing" >> "$LOGF"
            echo "I:modules: Currently loaded modules:" >> "$LOGF"
            lsmod >> "$LOGF"
        fi
    fi
}

fix_kerror7() {
    if ! mountpoint -q /metadata ; then
        mount /metadata
    fi
    if [ -d /metadata/ota ]; then
        rm -rf /metadata/ota
    fi
    umount /metadata
}

magisk_link_to_OF_FILES() {
    Magisk_zip="$1"
    mkdir -p /FFiles/OF_Magisk/ /sdcard/Fox/FoxFiles
    cp -f "$Magisk_zip" /FFiles/OF_Magisk/Magisk.zip
    cp -f "$Magisk_zip" /FFiles/OF_Magisk/uninstall.zip
    magisk_on_data_media "$Magisk_zip" &
}

_bb_sleep() {
    if [ -x "$_BB" ]; then "$_BB" sleep "$@"; else sleep "$@"; fi
}

_bb_mountpoint() {
    if [ -x "$_BB" ]; then "$_BB" mountpoint "$@"; else mountpoint "$@"; fi
}

magisk_on_data_media(){
    local Magisk_zip="$1"
    while true; do
        
        if [ -d /data/media/0 ] && _bb_mountpoint -q /data; then
            if [ ! -f /data/media/0/Fox/FoxFiles/Magisk.zip ] || [ ! -f /sdcard/Fox/FoxFiles/uninstall.zip ]; then
                echo "I:magisk: Copying Magisk zip to /data/media/0 for sideload/install from stock recovery" >> "$LOGF"
                mkdir -pv /data/media/0/Fox/FoxFiles
                cp -f "$Magisk_zip" /data/media/0/Fox/FoxFiles/uninstall.zip
            fi
            if [ ! -f /data/media/0/Fox/FoxFiles/Magisk.zip ] || [ ! -f /sdcard/Fox/FoxFiles/Magisk.zip ]; then
                echo "I:magisk: Copying Magisk zip to /data/media/0 for sideload/install from stock recovery" >> "$LOGF"
                mkdir -pv /data/media/0/Fox/FoxFiles
                cp -f "$Magisk_zip" /data/media/0/Fox/FoxFiles/Magisk.zip
            fi
            
        fi
        _bb_sleep 2
    done
}

find_magisk_zip() {
    local dir="$1"
    local file
    for file in "${dir}"/Magisk-*.zip; do
        if [ -f "$file" ]; then
            echo "$file"
            return 0
        fi
    done
    
}

#
# load_susfs_rename_fix — insmod the Baseband Guard fast-symlink panic fix.
#
# Baseband Guard's bb_inode_rename LSM hook calls page_get_link() to resolve
# symlink targets. On rootfs/tmpfs "fast symlinks" (target stored inline in
# inode->i_link) no page mapping exists, so page_get_link() dereferences a
# NULL inode->i_mapping->a_ops and panics. This module intercepts the hook
# via kprobe and skips it for fast symlinks only.
#
# Guards:
#   - /proc/modules absent  → monolithic kernel (CONFIG_MODULES=n): skip.
#   - ko file absent        → module not installed in this build: skip.
#   - register_kprobe fails → BBG absent or upstream-fixed kernel: no-op.
#
load_susfs_rename_fix() {
    local ko="/system/lib64/modules/susfs_rename_fix.ko"

    # Monolithic kernels (CONFIG_MODULES=n) have no /proc/modules.
    # insmod would return -ENOSYS (harmless), but skip explicitly for clarity.
    if [ ! -e /proc/modules ]; then
        echo "I:susfs_fix: /proc/modules absent \u2014 monolithic kernel, skipping" >> "$LOGF"
        return 0
    fi

    if [ ! -f "$ko" ]; then
        echo "W:susfs_fix: $ko not found, skipping BBG fast-symlink fix" >> "$LOGF"
        return 0
    fi

    if insmod "$ko" 2>>"$LOGF"; then
        echo "I:susfs_fix: $ko loaded \u2014 fast-symlink rename panic fix active" >> "$LOGF"
    else
        echo "W:susfs_fix: insmod $ko failed \u2014 unsupported kernel or already loaded" >> "$LOGF"
    fi
}

TARGET_MAGISK_ZIP=$(find_magisk_zip /system/bin)

setenforce 0
LOGF="/tmp/recovery.log"

# Dump busybox to /dev tmpfs so critical applets (sleep, mountpoint) survive
# package manager operations (e.g. NikGapps) that may replace /system/bin.
_BB_DIR="/dev/.fox_bb"
_BB="$_BB_DIR/busybox"
if [ -f /system/bin/busybox ]; then
    mkdir -p "$_BB_DIR"
    if cp -f /system/bin/busybox "$_BB" 2>/dev/null && chmod 755 "$_BB"; then
        echo "I:busybox: Dumped to $_BB" >> "$LOGF"
    else
        _BB=""
        echo "W:busybox: Failed to dump, will use PATH" >> "$LOGF"
    fi
else
    _BB=""
    echo "W:busybox: Not found in /system/bin, will use PATH" >> "$LOGF"
fi

chmod 777 /system/bin/*
device_code=$(getprop ro.hardware)
slot_detect
load_susfs_rename_fix

case "$device_code" in
    panther)
        # Pixel 7 — Focaltech touch
        modules_touch="stmvl53l1 lwis cl_dsp-core cs40l26-core cs40l26-i2c goodixfp heatmap goog_touch_interface focal_touch fps_touch_handler"
        ;;
    cheetah)
        # Pixel 7 Pro — Synaptics touch
        modules_touch="stmvl53l1 lwis cl_dsp-core cs40l26-core cs40l26-i2c goodixfp heatmap goog_touch_interface syna_touch fps_touch_handler"
        ;;
    lynx)
        # Pixel 7a — Goodix + Focaltech touch (dual-source)
        modules_touch="stmvl53l1 lwis cl_dsp-core cs40l26-core cs40l26-i2c goodixfp heatmap goog_touch_interface goodix_brl_touch focal_touch fps_touch_handler"
        ;;
    tangorpro)
        # Pixel Tablet — Novatek NVT SPI touch (10.95" LCD, no camera ToF, no under-display FP)
        modules_touch="heatmap goog_touch_interface touch_bus_negotiator touch_offload goog_usi_stylus nvt_touch fps_touch_handler"
        ;;
    # === zuma — Tensor G3 (Pixel 8 family) ===
    shiba)
        # Pixel 8
        modules_touch="stmvl53l1 lwis cl_dsp-core cs40l26-core cs40l26-i2c goodixfp heatmap goog_touch_interface sec_touch ftm5 goodix_brl_touch fps_touch_handler"
        ;;
    husky)
        # Pixel 8 Pro — same module set as shiba
        modules_touch="stmvl53l1 lwis cl_dsp-core cs40l26-core cs40l26-i2c goodixfp heatmap goog_touch_interface sec_touch ftm5 goodix_brl_touch fps_touch_handler"
        ;;
    akita)
        # Pixel 8a — no sec_touch/ftm5 (Goodix only)
        modules_touch="stmvl53l1 lwis cl_dsp-core cs40l26-core cs40l26-i2c goodixfp heatmap goog_touch_interface goodix_brl_touch fps_touch_handler"
        ;;
    # === zumapro — Tensor G4 (Pixel 9 family) ===
    tokay)
        # Pixel 9 — Synaptics + Samsung touch, QBT ultrasonic fingerprint
        modules_touch="stmvl53l1 lwis cl_dsp-core cs40l26-core cs40l26-i2c goodixfp qbt_handler heatmap goog_touch_interface sec_touch syna_touch fps_touch_handler"
        ;;
    komodo)
        # Pixel 9 Pro XL — Synaptics + Samsung touch, QBT ultrasonic fingerprint
        modules_touch="stmvl53l1 lwis cl_dsp-core cs40l26-core cs40l26-i2c goodixfp qbt_handler heatmap goog_touch_interface sec_touch syna_touch fps_touch_handler"
        ;;
    caiman)
        # Pixel 9 Pro — Synaptics + Samsung touch, QBT ultrasonic fingerprint
        modules_touch="stmvl53l1 lwis cl_dsp-core cs40l26-core cs40l26-i2c goodixfp qbt_handler heatmap goog_touch_interface sec_touch syna_touch fps_touch_handler"
        ;;
    tegu)
        # Pixel 9a — Synaptics touch (no sec_touch, no QBT)
        modules_touch="stmvl53l1 lwis cl_dsp-core cs40l26-core cs40l26-i2c goodixfp heatmap goog_touch_interface syna_touch fps_touch_handler"
        ;;
    # === laguna — Tensor G5 (Pixel 10 family) ===
    blazer)
        # Pixel 10 Pro — Focaltech + Synaptics touch (dual-source)
        modules_touch="lwis cl_dsp-core cs40l26-core cs40l26-i2c focal_touch syna_touch"
        ;;
    mustang)
        # Pixel 10 Pro XL — Focaltech + Synaptics touch (dual-source)
        modules_touch="lwis cl_dsp-core cs40l26-core cs40l26-i2c focal_touch syna_touch"
        ;;
    frankel)
        # Pixel 10 — Focaltech + Synaptics touch (dual-source)
        modules_touch="lwis cl_dsp-core cs40l26-core cs40l26-i2c focal_touch syna_touch"
        ;;
    *)
        modules_touch=""
        ;;
esac

if [ -n "$modules_touch" ]; then
    echo "I:vendor_fw: Copying haptics firmware from vendor partition..." >> "$LOGF"
    tmp_mnt="/tmp/vendor_fw_mnt"
    mkdir -p "$tmp_mnt" /vendor/firmware
    try_mount_vendor_fw() {
        local blk="$1"
        local slot_name="$2"
        local slot_num="$3"

        if [ ! -b "$blk" ]; then
            echo "W:vendor_fw: $blk not found, trying to map..." >> "$LOGF"
            if ! lptools_new --slot "$slot_num" --suffix "$slot_name" --map "vendor$slot_name" ; then
                echo "E:vendor_fw: Failed to map vendor$slot_name" >> "$LOGF"
                return 1
            fi
        fi

        if mount -r "$blk" "$tmp_mnt" 2>>"$LOGF"; then
            local _copied=0
            cp "$tmp_mnt"/firmware/* /vendor/firmware/ 2>>"$LOGF"
            umount "$tmp_mnt" 2>/dev/null
        else
            echo "W:vendor_fw: Cannot mount $blk" >> "$LOGF"
        fi
        return 1
    }

    if try_mount_vendor_fw "/dev/block/mapper/vendor${suffix}" "$suffix" "$slot"; then
        : # success
    elif try_mount_vendor_fw "/dev/block/mapper/vendor${unsuffix}" "$unsuffix" "$unslot"; then
        : # success from opposite slot
    elif [ -b /dev/block/by-name/vendor ] && mount -r /dev/block/by-name/vendor "$tmp_mnt" 2>>"$LOGF"; then
        cp "$tmp_mnt"/firmware/* /vendor/firmware/ 2>>"$LOGF"
        umount "$tmp_mnt" 2>/dev/null
        echo "I:vendor_fw: Firmware copied from /dev/block/by-name/vendor" >> "$LOGF"
    else
        echo "E:vendor_fw: Failed to copy firmware from any vendor slot" >> "$LOGF"
    fi

    rmdir "$tmp_mnt" 2>/dev/null
    ls /vendor/firmware/ >> "$LOGF" 2>&1

    modules_touch_install

    soc_family=$(getprop ro.recovery.soc_family)
    case "$soc_family" in
        gs201)
            cs40l26_pm="/sys/devices/platform/10d50000.hsi2c/i2c-0/0-0043/power/control"
            ;;
        zumapro)
            cs40l26_pm="/sys/devices/platform/10c80000.hsi2c/i2c-0/0-0043/power/control"
            ;;
        laguna)
            cs40l26_pm="/sys/devices/platform/10c80000.hsi2c/i2c-0/0-0043/power/control"
            ;;
        *)
            cs40l26_pm="/sys/devices/platform/10c80000.hsi2c/i2c-0/0-0043/power/control"
            ;;
    esac
    if [ -f "$cs40l26_pm" ]; then
        echo on > "$cs40l26_pm"
        echo "I:haptics: CS40L26 runtime PM set to 'on'" >> "$LOGF"
    fi
fi

if [ -c "/dev/lwis-flash-lm3644" ]; then
    echo "I:torch: /dev/lwis-flash-lm3644 available, LM3644 I2C torch ready" >> "$LOGF"
else
    echo "W:torch: /dev/lwis-flash-lm3644 not found" >> "$LOGF"
fi

fix_kerror7
if [ -n "$TARGET_MAGISK_ZIP" ]; then
    magisk_link_to_OF_FILES "$TARGET_MAGISK_ZIP"
else
    echo "W:magisk: No Magisk zip found in /system/bin, skipping copy and link creation" >> "$LOGF"
fi

exit 0