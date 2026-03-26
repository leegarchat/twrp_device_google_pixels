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
DBGLOG="/dev/logs/runatinit.log"

mkdir -p /dev/logs
: > "$DBGLOG"
_log() { printf '[%s] %s\n' "$(date '+%H:%M:%S' 2>/dev/null)" "$*" >> "$DBGLOG"; }

_log "========== runatinit.sh START =========="
_log "PID=$$  sh=$0"
_log "uptime=$(awk '{print $1}' /proc/uptime 2>/dev/null)s"
_log "kernel=$(uname -r 2>/dev/null)"
_log "ro.hardware=$(getprop ro.hardware 2>/dev/null)"
_log "ro.boot.slot_suffix=$(getprop ro.boot.slot_suffix 2>/dev/null)"
_log "cmdline AB slots: $(cat /proc/cmdline 2>/dev/null | tr ' ' '\n' | grep 'slot' | tr '\n' ' ')"

apply_prop_file() {
    local file="$1"
    _log "apply_prop_file: $file"
    if [ ! -f "$file" ]; then
        _log "  -> NOT FOUND, skip"
        return 1
    fi
    local _count
    _count=0
    while IFS= read -r line; do
        case "$line" in
            \#*|"") continue ;;
        esac
        key="${line%%=*}"
        value="${line#*=}"
        if [ -n "$key" ]; then
            resetprop "$key" "$value"
            _log "  SET $key=$value"
            _count=$((_count + 1))
        fi
    done < "$file"
    _log "  -> $_count props applied"
}

# Dynamically patches /system/etc/twrp.flags before TWRP reads it.
# Must run in runatinit.sh (early-init) — Process_Fstab() in twrp.cpp reads
# twrp.flags BEFORE runatboot.sh is ever called.
#
# Two operations:
#   1. USB OTG block device: next sdX letter after last internal UFS disk.
#   2. Partition prune: remove by-name entries absent on this device.
#      Logical mapper/* entries are never touched (not mapped yet).
#
# Safety: if ueventd coldboot is not done yet (by-name empty after wait),
# the prune step is skipped entirely — never prunes on an empty /dev/block.
fix_twrp_flags() {
    local flags_file="/system/etc/twrp.flags"
    _log "--- fix_twrp_flags ---"
    _log "  flags_file=$flags_file"
    if [ ! -f "$flags_file" ]; then
        _log "  NOT FOUND, skip"
        return
    fi
    local LOG="/tmp/recovery.log"
    local i last_blk last_letter next_letter tmp line blk part
    local _pruned _kept
    _pruned=0
    _kept=0

    # Wait up to 3 seconds for ueventd to create /dev/block/sd? nodes.
    # runatinit runs at early-init; ueventd coldboot is concurrent and usually
    # completes within 1-2 seconds. The loop exits immediately on first hit.
    _log "  waiting for /dev/block/sd? ..."
    i=0
    while [ "$i" -lt 3 ]; do
        ls /dev/block/sd? >/dev/null 2>&1 && break
        _log "    attempt $i: sd? not ready"
        sleep 1
        i=$((i + 1))
    done
    _log "  sd? after wait (i=$i): $(ls /dev/block/sd? 2>/dev/null | tr '\n' ' ')"

    # --- 1. USB OTG device auto-detection ---
    # UFS internal disks: sda, sdb, sdc, ... OTG gets the next letter.
    last_blk=$(ls /dev/block/sd? 2>/dev/null | sort | tail -1)
    _log "  last internal disk: ${last_blk:-<none>}"
    if [ -n "$last_blk" ]; then
        last_letter="${last_blk##*sd}"
        next_letter=$(printf '%s' "$last_letter" | tr 'abcdefghijklmnopqrstuvwxy' 'bcdefghijklmnopqrstuvwxyz')
        _log "  OTG candidate: sd${next_letter}1  (last=sd${last_letter})"
        if [ "$next_letter" != "$last_letter" ]; then
            sed -i "/usb_otg/s|/dev/block/sd[a-z][0-9]*|/dev/block/sd${next_letter}1|" "$flags_file"
            echo "I:twrp.flags: USB OTG -> /dev/block/sd${next_letter}1 (last internal: ${last_blk##*/})" >> "$LOG"
            _log "  USB OTG patched -> sd${next_letter}1"
        else
            _log "  WARNING: cannot increment '$last_letter', OTG entry not patched"
        fi
    else
        echo "W:twrp.flags: No /dev/block/sd? after wait, USB OTG entry unchanged" >> "$LOG"
        _log "  WARNING: no sd? found, OTG not patched"
    fi

    # --- 2. Prune by-name entries absent on this device ---
    # Safety guard: skip prune if by-name is not populated yet.
    local _bn_count
    _bn_count=$(ls /dev/block/platform/*/by-name/ 2>/dev/null | wc -l)
    _log "  by-name symlinks found: $_bn_count"
    if ! ls /dev/block/platform/*/by-name/ >/dev/null 2>&1; then
        echo "W:twrp.flags: by-name not ready, skipping partition prune" >> "$LOG"
        _log "  by-name not ready, prune skipped"
        return
    fi

    tmp="${flags_file}.tmp"
    : > "$tmp"
    while IFS= read -r line; do
        case "$line" in
            \#*|"") printf '%s\n' "$line" >> "$tmp"; continue ;;
        esac
        # awk: portable field extraction; busybox awk is always available in recovery
        blk=$(printf '%s\n' "$line" | awk '{print $3}')
        case "$blk" in
            /dev/block/platform/*/by-name/*)
                part="${blk##*/}"
                # A/B slotselect entries use the base name (e.g. "boot") in twrp.flags,
                # but by-name contains only suffixed symlinks (boot_a / boot_b).
                # Keep the entry if the base name OR the _a suffixed name exists.
                if ls /dev/block/platform/*/by-name/"$part" >/dev/null 2>&1 ||
                   ls /dev/block/platform/*/by-name/"${part}_a" >/dev/null 2>&1; then
                    printf '%s\n' "$line" >> "$tmp"
                    _kept=$((_kept + 1))
                else
                    echo "I:twrp.flags: Pruned absent partition: $part" >> "$LOG"
                    _log "  PRUNED: $part"
                    _pruned=$((_pruned + 1))
                fi
                ;;
            *)
                printf '%s\n' "$line" >> "$tmp"
                ;;
        esac
    done < "$flags_file"
    mv -f "$tmp" "$flags_file"
    _log "  prune done: kept=$_kept pruned=$_pruned"
    _log "fix_twrp_flags done"
}

setenforce 0
_log "setenforce 0 done"

_log "--- device detection ---"
device_code=$(getprop ro.hardware)
_log "ro.hardware=$device_code"
_log "ro.boot.mode=$(getprop ro.boot.mode 2>/dev/null)"
_log "/dev/block contents: $(ls /dev/block/ 2>/dev/null | tr '\n' ' ')"

case "$device_code" in
    panther|cheetah|lynx|gs201)      family="gs201" ;;
    shiba|husky|akita|zuma)          family="zuma" ;;
    tokay|komodo|caiman|tegu|zumapro) family="zumapro" ;;
    *)                                family="" ;;
esac
_log "detected family=$family"

_log "--- applying props ---"
if [ -n "$family" ]; then
    apply_prop_file "${PROPS_DIR}/${family}_common.prop"
fi
apply_prop_file "${PROPS_DIR}/${device_code}.prop"

if [ "$family" = "gs201" ] && [ -f /system/etc/twrp_gs201.flags ]; then
    _log "gs201: cp twrp_gs201.flags -> twrp.flags"
    cp -f /system/etc/twrp_gs201.flags /system/etc/twrp.flags
fi

_log "--- calling fix_twrp_flags ---"
fix_twrp_flags
_log "--- defining lgz/magisk functions ---"
lgz_decompress_zips() {
    local manifest="/lgz_zip_manifest.txt"
    _log "--- lgz_decompress_zips ---"
    _log "  manifest=$manifest"
    if [ ! -f "$manifest" ]; then
        _log "  manifest not found, skip"
        return 0
    fi

    local lgz="/system/bin/lgz"
    chmod 777 "$lgz" 2>/dev/null
    if [ ! -x "$lgz" ]; then
        _log "  lgz not executable: $lgz"
        return 0
    fi
    _log "  lgz=$lgz"

    # ── Probe 'zip' binary ────────────────────────────────────────────────
    # Log the full picture of every zip-related binary available right now.
    _log "  --- zip binary probe ---"
    _log "    ls -la /system/bin/zip:     $(ls -la /system/bin/zip 2>&1)"
    _log "    readlink /system/bin/zip:   $(readlink /system/bin/zip 2>&1) (rc=$?)"
    _log "    ls -la /system/bin/unzip:   $(ls -la /system/bin/unzip 2>&1)"
    _log "    readlink /system/bin/unzip: $(readlink /system/bin/unzip 2>&1)"
    _log "    ls -la /system/bin/ziptool: $(ls -la /system/bin/ziptool 2>&1)"
    _log "    ziptool symlinks:           $(ls -la /system/bin/ 2>/dev/null | grep ziptool)"
    _log "    ls -la /system/bin/busybox: $(ls -la /system/bin/busybox 2>&1)"
    _log "    ls -la /system/bin/toybox:  $(ls -la /system/bin/toybox 2>&1)"
    # Check if busybox has the zip applet compiled in
    if [ -x /system/bin/busybox ]; then
        local _bb_has_zip
        _bb_has_zip=$(/system/bin/busybox --list 2>/dev/null | grep '^zip$' || true)
        _log "    busybox --list | grep zip: '${_bb_has_zip}'"
    fi
    # Check if toybox has zip
    if [ -x /system/bin/toybox ]; then
        local _tb_has_zip
        _tb_has_zip=$(/system/bin/toybox --help 2>&1 | grep '\bzip\b' | head -1 || true)
        _log "    toybox --help grep zip: '${_tb_has_zip}'"
    fi

    # Determine which zip binary is actually usable right now.
    # /sbin/zip is a static ELF available from early-init but /sbin is not in PATH.
    # Priority: /sbin/zip → /system/bin/zip → busybox zip applet
    local _zip_bin
    _zip_bin=""
    printf 'x' > /tmp/_ziptest_in.txt
    for _zcandidate in /sbin/zip /system/bin/zip; do
        [ -x "$_zcandidate" ] || continue
        if (cd /tmp && "$_zcandidate" -0 /tmp/_ziptest_out.zip _ziptest_in.txt) >/dev/null 2>&1 \
                && [ -s /tmp/_ziptest_out.zip ]; then
            _zip_bin="$_zcandidate"
            _log "    -> $_zcandidate functional: YES"
            break
        else
            _log "    -> $_zcandidate exists but NOT functional (symlink to ziptool?)"
        fi
        rm -f /tmp/_ziptest_out.zip
    done
    if [ -z "$_zip_bin" ] && [ -x /system/bin/busybox ]; then
        if (cd /tmp && /system/bin/busybox zip -0 /tmp/_ziptest_out.zip _ziptest_in.txt) >/dev/null 2>&1 \
                && [ -s /tmp/_ziptest_out.zip ]; then
            _zip_bin="/system/bin/busybox zip"
            _log "    -> busybox zip applet functional: YES"
        else
            local _bb_zip_err
            _bb_zip_err=$(/system/bin/busybox zip 2>&1 | head -1 || true)
            _log "    -> busybox zip NOT functional: ${_bb_zip_err}"
        fi
    fi
    rm -f /tmp/_ziptest_in.txt /tmp/_ziptest_out.zip
    if [ -z "$_zip_bin" ]; then
        _log "    -> NO usable zip binary found! repack will fail"
    else
        _log "    -> using: $_zip_bin"
    fi
    _log "  --- end zip probe ---"

    echo "I:lgz-zip: Decompressing zip contents..." >> /tmp/recovery.log

    local _lcount _zip_rc _lgz_ok _lgz_total
    _lcount=0
    while IFS= read -r zippath; do
        case "$zippath" in \#*|"") continue ;; esac
        if [ ! -f "$zippath" ]; then
            _log "  zip not found: $zippath"
            continue
        fi
        _log "  processing: $zippath"
        local tmpdir
        tmpdir=$(mktemp -d /tmp/lgz_zip.XXXXXX)

        if unzip -q -o "$zippath" -d "$tmpdir" 2>>"$DBGLOG"; then
            _lgz_total=$(find "$tmpdir" -type f | wc -l)
            _lgz_ok=0
            _log "    unzip ok -> $tmpdir  (files: $_lgz_total)"
            find "$tmpdir" -type f | while IFS= read -r f; do
                if "$lgz" decompress "$f" "${f}.dec" 2>/dev/null; then
                    mv -f "${f}.dec" "$f"
                    _lgz_ok=$((_lgz_ok + 1))
                fi
            done
            _log "    lgz decompress: $_lgz_ok/$_lgz_total files modified"
            rm -f "${zippath}.tmp"
            _zip_rc=127
            if [ -n "$_zip_bin" ]; then
                case "$_zip_bin" in
                    */busybox\ zip|*/busybox\ *)
                        # two-word busybox applet call
                        local _bb _app
                        _bb="${_zip_bin%% *}"
                        _app="${_zip_bin#* }"
                        (cd "$tmpdir" && "$_bb" "$_app" -0 -r "${zippath}.tmp" . 2>>"$DBGLOG")
                        _zip_rc=$?
                        ;;
                    *)
                        (cd "$tmpdir" && "$_zip_bin" -0 -r "${zippath}.tmp" . 2>>"$DBGLOG")
                        _zip_rc=$?
                        ;;
                esac
            fi
            _log "    zip repack rc=$_zip_rc  output_exists=$([ -f "${zippath}.tmp" ] && echo yes || echo no)"

            if [ -f "${zippath}.tmp" ]; then
                mv -f "${zippath}.tmp" "$zippath"
                echo "I:lgz-zip: Restored: $zippath" >> /tmp/recovery.log
                _log "    repack ok: $zippath"
                _lcount=$((_lcount + 1))
            else
                _log "    repack FAILED: $zippath  (zip_rc=$_zip_rc  zip_bin=${_zip_bin:-<none>})"
            fi
        else
            _log "    unzip FAILED (rc=$?): $zippath"
        fi

        rm -rf "$tmpdir"
    done < "$manifest"
    _log "lgz_decompress_zips done (processed: $_lcount)"
}

unzip_magiskboot_binary() {
    local zip="$1"
    _log "--- unzip_magiskboot_binary ---"
    _log "  zip=$zip"
    if [ ! -f "$zip" ]; then
        _log "  zip not found, skip"
        return
    fi
    mkdir -p /tmp/magisk_unzip
    cd /tmp/magisk_unzip || { _log "  cd /tmp/magisk_unzip FAILED"; return; }
    _log "  unzipping..."
    if ! unzip -q "$zip"; then
        _log "  unzip FAILED (rc=$?)"
        cd /tmp || true; rm -rf /tmp/magisk_unzip; return
    fi
    _log "  unzip ok, copying binaries..."
    cp lib/arm64-v8a/libmagiskboot.so /system/bin/magiskboot_29
    _log "  cp magiskboot_29 rc=$?"
    cp lib/arm64-v8a/libmagiskboot.so /system/bin/magiskboot
    _log "  cp magiskboot rc=$?"
    cp lib/arm64-v8a/libbusybox.so /system/bin/busybox
    _log "  cp busybox rc=$?"
    chmod 777 /system/bin/magiskboot_29 /system/bin/magiskboot /system/bin/busybox
    rm -f /system/bin/ln
    /system/bin/busybox ln -s /system/bin/busybox /system/bin/ln
    _log "  busybox ln -> /system/bin/ln"
    _log "  busybox: $(/system/bin/busybox 2>&1 | head -1)"
    cd /tmp || true
    rm -rf /tmp/magisk_unzip
    _log "unzip_magiskboot_binary done"
}

_log "--- calling lgz_decompress_zips ---"
lgz_decompress_zips
_log "lgz_decompress_zips returned"

_log "--- searching Magisk zip in /system/bin/ ---"
_log "  candidates: $(ls /system/bin/Magisk-*.zip 2>/dev/null | tr '\n' ' ')"
TARGET_MAGISK_ZIP=""
for _f in /system/bin/Magisk-*.zip; do
    [ -f "$_f" ] && TARGET_MAGISK_ZIP="$_f" && break
done
if [ -n "$TARGET_MAGISK_ZIP" ]; then
    _log "  found: $TARGET_MAGISK_ZIP"
    unzip_magiskboot_binary "$TARGET_MAGISK_ZIP"
else
    _log "  no Magisk zip found"
    echo "W:magisk: No Magisk zip found in /system/bin, skipping binary extraction" >> /tmp/recovery.log
fi

_log "--- setprop servicemanager.ready ---"
setprop servicemanager.ready true
resetprop servicemanager.ready true

_log "uptime=$(awk '{print $1}' /proc/uptime 2>/dev/null)s"
_log "========== runatinit.sh END =========="

exit 0
