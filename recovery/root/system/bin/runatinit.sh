#!/bin/sh
#
#   runatinit.sh — OrangeFox Recovery early-init device identity script.
#
#   This file is part of the OrangeFox Recovery Project
#   Copyright (C) 2024-2026 The OrangeFox Recovery Project
#
#   OrangeFox is free software: you can redistribute it and/or modify
#   it under the terms of the GNU General Public License as published by
#   the Free Software Foundation, either version 3 of the License, or
#   any later version.
#
# Runs in on early-init phase via exec, BEFORE:
#   - on init (USB controller, Trusty TEE)
#   - on early-boot (USB gadget strings use ${ro.product.model})
#   - TWRP data.cpp (DataManager reads ro.product props)
#
# Detects device codename from ro.hardware and applies:
#   1. Family-common properties (zuma_common.prop)
#   2. Device-specific properties (shiba.prop, husky.prop, etc.)
#   3. LGZ decompression of build-time compressed zip payloads
#   4. Magisk binary extraction and link creation
#
# This ensures correct device identity for MTP/USB enumeration and UI
# BEFORE the USB gadget writes ${ro.product.model} to configfs.
#

PROPS_DIR="/system/etc/device_props"

apply_prop_file() {
    local file="$1"
    [ -f "$file" ] || return 1
    while IFS= read -r line; do
        case "$line" in
            \#*|"") continue ;;
        esac
        key="${line%%=*}"
        value="${line#*=}"
        [ -n "$key" ] && resetprop "$key" "$value"
    done < "$file"
}

setenforce 0

device_code=$(getprop ro.hardware)
case "$device_code" in
    panther|cheetah|lynx|gs201)      family="gs201" ;;
    shiba|husky|akita|zuma)          family="zuma" ;;
    tokay|komodo|caiman|tegu|zumapro) family="zumapro" ;;
    *)                                family="" ;;
esac

if [ -n "$family" ]; then
    apply_prop_file "${PROPS_DIR}/${family}_common.prop"
fi
apply_prop_file "${PROPS_DIR}/${device_code}.prop"
if [ "$family" = "gs201" ] && [ -f /system/etc/twrp_gs201.flags ]; then
    cp -f /system/etc/twrp_gs201.flags /system/etc/twrp.flags
fi
lgz_decompress_zips() {
    local manifest="/lgz_zip_manifest.txt"
    [ -f "$manifest" ] || return 0

    local lgz="/system/bin/lgz"
    [ -x "$lgz" ] || return 0

    echo "I:lgz-zip: Decompressing zip contents..." >> /tmp/recovery.log

    while IFS= read -r zippath; do
        case "$zippath" in \#*|"") continue ;; esac
        [ -f "$zippath" ] || continue

        tmpdir=$(mktemp -d /tmp/lgz_zip.XXXXXX)

        if unzip -q -o "$zippath" -d "$tmpdir" 2>/dev/null; then
            find "$tmpdir" -type f | while IFS= read -r f; do
                if "$lgz" decompress "$f" "${f}.dec" 2>/dev/null; then
                    mv -f "${f}.dec" "$f"
                fi
            done
            rm -f "${zippath}.tmp"
            (cd "$tmpdir" && zip -0 -r -q "${zippath}.tmp" .) 2>/dev/null

            if [ -f "${zippath}.tmp" ]; then
                mv -f "${zippath}.tmp" "$zippath"
                echo "I:lgz-zip: Restored: $zippath" >> /tmp/recovery.log
            fi
        fi

        rm -rf "$tmpdir"
    done < "$manifest"
}

unzip_magiskboot_binary() {
    mkdir -p /tmp/magisk_unzip
    cd /tmp/magisk_unzip || return
    unzip -q "$TARGET_MAGISK_ZIP"
    cp lib/arm64-v8a/libmagiskboot.so /system/bin/magiskboot_29
    cp lib/arm64-v8a/libmagiskboot.so /system/bin/magiskboot
    cp lib/arm64-v8a/libbusybox.so /system/bin/busybox
    chmod 777 /system/bin/magiskboot_29
    chmod 777 /system/bin/magiskboot
    chmod 777 /system/bin/busybox
    rm -f /system/bin/ln
    /system/bin/busybox ln -s /system/bin/busybox /system/bin/ln
    cd /tmp || return
    rm -rf /tmp/magisk_unzip
}

lgz_decompress_zips
unzip_magiskboot_binary

setprop servicemanager.ready true
resetprop servicemanager.ready true

exit 0
