#!/bin/bash
# aio_swap.sh — reference post-unpack family swap for the all-in-one cpio.
#
# NOTE: on-device boots do NOT need this script — recovery-init-stub
# (aioswap.c) performs the same swap at boot time, post LGZ unpack and
# pre init, driven by androidboot.hardware (plus the ofx_swap=<fam>
# cmdline test hook). This script mirrors that logic for EXTERNAL use
# (installer tooling operating on an unpacked cpio on host/PC).
#
# The AIO ramdisk ships family-neutral defaults (zuma placeholders) plus a
# swap kit: /etc/recovery.fstab.<fam>, /system/etc/twrp.flags.<fam>,
# /vendor/etc/vintf/manifest/keymint.<fam>.xml, both keymint HAL binaries,
# and /system/etc/aio/families.txt (<fam>:keymint=:usbctrl=).
# The installer runs this (or replicates it) on the unpacked cpio root
# right after lgz-unpack, before repacking/flashing for the target family.
#
# Usage:
#   aio_swap.sh <unpacked_root> <family>
#
# Effects for <family> (example: gs101):
#   etc/recovery.fstab                  <- etc/recovery.fstab.gs101
#   system/etc/twrp.flags               <- system/etc/twrp.flags.gs101
#   init.recovery.{pixel_common,usb}.rc  : 11210000.usb/.dwc3 -> family USBCTRL
#   vendor/etc/vintf/manifest/           : keep keymint.gs101.xml, drop siblings
#   vendor/bin/hw/                       : keep the matching keymint HAL only
#   prop.default (or default.prop)       : enforce ro.recovery.keymint=<rust|cpp>
# Without arguments (or with --list) prints families.txt and exits 0.
set -euo pipefail

ROOT="${1:-}"
FAM="${2:-}"
MAN="system/etc/aio/families.txt"

usage() {
    echo "Usage: $0 <unpacked_root> <family>"
    echo "Swap-kit families are listed in <root>/$MAN"
}

if [[ "$ROOT" == "--list" || -z "$ROOT" || -z "$FAM" ]]; then
    if [[ -n "$ROOT" && -f "$ROOT/$MAN" ]]; then
        cat "$ROOT/$MAN"
    else
        usage
    fi
    exit 0
fi

if [[ ! -f "$ROOT/$MAN" ]]; then
    echo "ERROR: no swap-kit manifest: $ROOT/$MAN (not an AIO cpio?)" >&2
    exit 1
fi
rec="$(grep -E "^${FAM}:" "$ROOT/$MAN" || true)"
if [[ -z "$rec" ]]; then
    echo "ERROR: unknown family '$FAM'. Available:" >&2
    cat "$ROOT/$MAN" >&2
    exit 1
fi
KM="$(echo "$rec" | sed -n 's/.*:keymint=\([^:]*\).*/\1/p')"
USBCTRL="$(echo "$rec" | sed -n 's/.*:usbctrl=\([^:]*\).*/\1/p')"
USBPATH="$(echo "$rec" | sed -n 's/.*:usbpath=\([^:]*\).*/\1/p')"
echo "[aio-swap] family=$FAM keymint=$KM usbctrl=${USBCTRL:-<default 11210000.dwc3>} usbpath=${USBPATH:-<default <base>.usb>}"

# --- recovery.fstab (live file TWRP parses at boot) ---
# Canonical location is system/etc (root /etc is a symlink to it).
if [[ -f "$ROOT/system/etc/recovery.fstab.$FAM" ]]; then
    cp -f "$ROOT/system/etc/recovery.fstab.$FAM" "$ROOT/system/etc/recovery.fstab"
    echo "[aio-swap]   recovery.fstab <- $FAM"
else
    echo "[aio-swap]   WARNING: no system/etc/recovery.fstab.$FAM, keeping placeholder" >&2
fi

# --- twrp.flags (live file) ---
if [[ -f "$ROOT/system/etc/twrp.flags.$FAM" ]]; then
    cp -f "$ROOT/system/etc/twrp.flags.$FAM" "$ROOT/system/etc/twrp.flags"
    echo "[aio-swap]   twrp.flags <- $FAM"
else
    echo "[aio-swap]   WARNING: no system/etc/twrp.flags.$FAM, keeping placeholder" >&2
fi

# --- recovery.wipe (factory-reset target list, same layout as fstab) ---
if [[ -f "$ROOT/system/etc/recovery.wipe.$FAM" ]]; then
    cp -f "$ROOT/system/etc/recovery.wipe.$FAM" "$ROOT/system/etc/recovery.wipe"
    echo "[aio-swap]   recovery.wipe <- $FAM"
else
    echo "[aio-swap]   WARNING: no system/etc/recovery.wipe.$FAM, keeping placeholder" >&2
fi
# --- USB controller (rc files stay open in the cpio, sed is safe) ---
# New images carry UNKNOWN markers (no zuma defaults); older ones still
# have the baked 11210000 default — substitute both spellings. Empty
# manifest usbctrl means the 11210000 default, always written. The bus
# parent dir comes from the manifest usbpath when present (laguna/malibu
# live under simple_usb_bus); otherwise "<base>.usb" directly under
# /sys/devices/platform (gs101/gs201/zuma/zumapro layout).
if [[ -z "$USBCTRL" ]]; then USBCTRL="11210000.dwc3"; fi
usb_base="${USBCTRL%.dwc3}"
usb_bus="${USBPATH:-${usb_base}.usb}"
for rc in "$ROOT/init.recovery.pixel_common.rc" "$ROOT/init.recovery.usb.rc"; do
    [[ -f "$rc" ]] || continue
    sed -i "s|00000000\.usb|${usb_bus}|g; s|UNKNOWN\.dwc3|${USBCTRL}|g; s|11210000\.usb|${usb_bus}|g; s|11210000\.dwc3|${USBCTRL}|g" "$rc"
    echo "[aio-swap]   USB controller -> $USBCTRL (bus $usb_bus, $(basename "$rc"))"
done

# --- VINTF keymint fragments: keep the target one, drop siblings ---
shopt -s nullglob
for frag in "$ROOT"/vendor/etc/vintf/manifest/keymint.*.xml; do
    if [[ "$(basename "$frag")" == "keymint.$FAM.xml" ]]; then
        echo "[aio-swap]   VINTF fragment kept: $(basename "$frag")"
    else
        rm -f "$frag"
        echo "[aio-swap]   VINTF fragment dropped: $(basename "$frag")"
    fi
done

# --- Keymint binaries: keep the matching HAL, drop the other (tmpfs diet) ---
RUST_BIN="android.hardware.security.keymint-service.rust.trusty"
CPP_BIN="android.hardware.security.keymint-service.trusty"
if [[ "$KM" == "cpp" ]]; then
    rm -f "$ROOT/vendor/bin/hw/$RUST_BIN" && echo "[aio-swap]   keymint binary kept: cpp"
elif [[ "$KM" == "rust" ]]; then
    rm -f "$ROOT/vendor/bin/hw/$CPP_BIN" && echo "[aio-swap]   keymint binary kept: rust"
else
    echo "ERROR: bad keymint '$KM' for $FAM" >&2
    exit 1
fi

# --- ro.recovery.keymint prop (gated triggers in pixel_common.rc) ---
prop_file=""
for cand in "$ROOT/prop.default" "$ROOT/default.prop"; do
    if [[ -f "$cand" ]]; then prop_file="$cand"; break; fi
done
if [[ -n "$prop_file" ]]; then
    grep -v '^ro\.recovery\.keymint=' "$prop_file" > "$prop_file.tmp" || true
    mv -f "$prop_file.tmp" "$prop_file"
    echo "ro.recovery.keymint=$KM" >> "$prop_file"
    echo "[aio-swap]   prop set in $(basename "$prop_file"): ro.recovery.keymint=$KM"
else
    echo "[aio-swap]   WARNING: no prop.default/default.prop found" >&2
fi

echo "[aio-swap] done: $ROOT is now a $FAM image"
