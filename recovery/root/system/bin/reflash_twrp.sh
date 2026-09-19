#!/sbin/sh

# reflash_twrp.sh — Reflash recovery to both vendor_boot slots from ramdisk snapshot.
# Works on all Zuma SoC Pixels (shiba/husky/akita) — same partition layout.
#
# The snapshot at /dev/ramdisk_snapshot/ preserves the exact ramdisk state
# from boot time (before LGZ decompression), so the repacked image is
# byte-identical to the original.
# stdout is captured by twrpRepacker and displayed in the recovery UI.

SNAP="/dev/ramdisk_snapshot"
FOLDER="/tmp/reflash_recovery"
LOGF="/tmp/reflash_twrp.log"

exec 2>>"$LOGF"

_log() { printf '[%s] %s\n' "$(date '+%H:%M:%S' 2>/dev/null)" "$*" >> "$LOGF"; }
_die() {
    echo "ERROR: $*"
    _log "FATAL: $*"
    exit 1
}

_log "========== reflash_twrp START =========="
_log "device=$(getprop ro.hardware 2>/dev/null)  uptime=$(awk '{print $1}' /proc/uptime 2>/dev/null)s"

echo "- Starting reflash current recovery (snapshot-based)"

[ -d "$SNAP" ] \
    || _die "Ramdisk snapshot not found at $SNAP. Was ramdisk_snapshot run at boot?"

RECOVERY_LIST="$SNAP/recovery_file_list.txt"
FSTAGE_LIST="$SNAP/first_stage_file_list.txt"
NBOOT_LZ4="$SNAP/system/bin/nboot.lz4"
[ -f "$RECOVERY_LIST" ] || _die "Missing snapshot file: $RECOVERY_LIST"
[ -f "$FSTAGE_LIST"   ] || _die "Missing snapshot file: $FSTAGE_LIST"
[ -f "$NBOOT_LZ4"     ] || _die "Missing snapshot file: $NBOOT_LZ4"
_log "snapshot files OK"

for _bin in magiskboot_29 cpio dd sha256sum; do
    command -v "$_bin" >/dev/null 2>&1 || _die "Required binary not found: $_bin"
done
_log "binaries OK"

DEV_A="/dev/block/by-name/vendor_boot_a"
DEV_B="/dev/block/by-name/vendor_boot_b"
[ -b "$DEV_A" ] || _die "Block device not found: $DEV_A"
[ -b "$DEV_B" ] || _die "Block device not found: $DEV_B"
_log "block devices OK"

rm -rf "$FOLDER"
mkdir -p "$FOLDER/vendor_ramdisk" || _die "Cannot create $FOLDER/vendor_ramdisk"

device_code=$(getprop ro.hardware)
_log "device_code=$device_code"

# --- Kernel boot strings from /pixelrunatboot.json (kernel_bootcfg) ---
# SINGLE record of the kernel this image was built with (no uname probing:
# uname shows only the current slot and can carry arbitrary maintainer
# strings). Stamp it verbatim; when the record is absent fall back to the
# nboot.lz4 base header untouched.
KB_JSON="/pixelrunatboot.json"
STAMP_BOOTCFG=0
K_CMDLINE=""
K_BOOTCONFIG_RAW=""
K_VER=""
if [ -f "$KB_JSON" ]; then
    KB_PAIR=$(awk -v d="$device_code" '
      /"kernel_bootcfg"/ {t=1; next}
      t==1 && index($0, "\"" d "\"") {t=2; next}
      t==2 && /"ver"/ {v=$0; t=3; next}
      t==3 && /"cmdline"/ {c=$0; t=4; next}
      t==4 && /"bootconfig"/ {print v; print c; print $0; exit}
    ' "$KB_JSON")
    K_VER=$(printf '%s' "$KB_PAIR" | sed -n '1p' | sed 's/.*"ver": "\(.*\)".*/\1/')
    K_CMDLINE_RAW=$(printf '%s' "$KB_PAIR" | sed -n '2p' | sed 's/.*"cmdline": "\(.*\)".*/\1/')
    K_BOOTCONFIG_RAW=$(printf '%s' "$KB_PAIR" | sed -n '3p' | sed 's/.*"bootconfig": "\(.*\)".*/\1/')
    if [ -n "$K_CMDLINE_RAW" ]; then
        # JSON unescape: \" -> " (cmdline dyndbg flag), \n stays escaped for bootconfig.
        K_CMDLINE=$(printf '%s' "$K_CMDLINE_RAW" | sed 's/\\"/"/g')
        STAMP_BOOTCFG=1
    fi
fi
if [ "$STAMP_BOOTCFG" = "1" ]; then
    echo "- Kernel profile (build): $device_code / $K_VER"
    _log "cmdline=[$K_CMDLINE]"
    _log "bootconfig_raw=[$K_BOOTCONFIG_RAW]"
else
    echo "- WARNING: no kernel_bootcfg for '$device_code', keeping base (nboot) header"
    _log "WARNING: kernel_bootcfg fallback to nboot default"
fi

# DFE is gone (decrypt works natively): the hw-init fstab lines are always
# stripped for the reflashed image, no marker files involved.
for f in "$SNAP/first_stage_ramdisk/system/etc"/fstab*; do
    [ -f "$f" ] || continue
    if grep -q "/vendor/etc/init/hw" "$f"; then
        echo "- Patching fstab: $(basename "$f")"
        sed -i '/\/vendor\/etc\/init\/hw/d' "$f" || _die "fstab patch failed: $f"
    fi
done

umount -fl /vendor 2>/dev/null
umount -fl /system_root 2>/dev/null

grep -q "first_stage_file_list.txt"       "$RECOVERY_LIST" || echo "first_stage_file_list.txt"       >> "$RECOVERY_LIST"
grep -q "ramdisk_snapshot_manifest.txt"   "$RECOVERY_LIST" || echo "ramdisk_snapshot_manifest.txt"   >> "$RECOVERY_LIST"

echo "- Decompressing base vendor_boot image..."
cd "$FOLDER" || _die "Cannot cd to $FOLDER"

magiskboot_29 decompress "$NBOOT_LZ4" ./empty.img \
    || _die "magiskboot_29 decompress failed"
[ -s "$FOLDER/empty.img" ] || _die "empty.img is empty/missing after decompress"
_log "empty.img size=$(stat -c %s "$FOLDER/empty.img" 2>/dev/null) bytes"

# Unpack the header (-h): exposes `header` (name=/cmdline=) and `bootconfig`
# for substitution. NOTE: magiskboot exits nonzero on padded/stock images
# even when extraction succeeds, so success = files exist, not rc.
# NOTE 2: this also extracts the stub vendor_ramdisk fragments, so our
# fresh cpios MUST be (re)created AFTER this step.
# NOTE 3: a vendor_boot must never carry a dtb (stale OFox builder anomaly) —
# strip it so repack produces DTB_SZ 0 like stock (malibu stock has none).
echo "- Unpacking base header for cmdline/bootconfig substitution..."
magiskboot_29 unpack -h empty.img >>"$LOGF" 2>&1 || true
[ -f "$FOLDER/header" ] || _die "header missing after unpack -h (see $LOGF)"
[ -f "$FOLDER/bootconfig" ] || _die "bootconfig missing after unpack -h (see $LOGF)"
rm -f "$FOLDER/dtb"
_log "dtb stripped (vendor_boot ships dtb-free)"

echo "- Stamping build kernel cmdline ($K_VER) into header..."
if [ "$STAMP_BOOTCFG" = "1" ]; then
    printf 'name=\ncmdline=%s\n' "$K_CMDLINE" > "$FOLDER/header" \
        || _die "Cannot write header"
    printf '%b\n' "$K_BOOTCONFIG_RAW" > "$FOLDER/bootconfig" \
        || _die "Cannot write bootconfig"
    _log "header+bootconfig stamped ($K_VER)"
else
    _log "header+bootconfig left as unpacked (nboot default)"
fi

echo "- Creating recovery ramdisk cpio from snapshot..."
cd "$SNAP" || _die "Cannot cd to $SNAP"

cpio -H newc -o < "$RECOVERY_LIST" > "$FOLDER/vendor_ramdisk/recovery.cpio" 2>/dev/null \
    || _die "cpio failed building recovery ramdisk"
[ -s "$FOLDER/vendor_ramdisk/recovery.cpio" ] \
    || _die "recovery.cpio is empty after cpio"
_log "recovery.cpio size=$(stat -c %s "$FOLDER/vendor_ramdisk/recovery.cpio" 2>/dev/null) bytes"

echo "- Creating first_stage ramdisk cpio from snapshot..."
cpio -H newc -o < "$FSTAGE_LIST" > "$FOLDER/vendor_ramdisk/ramdisk.cpio" 2>/dev/null \
    || _die "cpio failed building first_stage ramdisk"
[ -s "$FOLDER/vendor_ramdisk/ramdisk.cpio" ] \
    || _die "ramdisk.cpio is empty after cpio"
_log "ramdisk.cpio size=$(stat -c %s "$FOLDER/vendor_ramdisk/ramdisk.cpio" 2>/dev/null) bytes"

echo "- Repacking vendor_boot image (cmdline=${K_VER:-nboot default})..."
cd "$FOLDER" || _die "Cannot cd to $FOLDER"
magiskboot_29 repack "$FOLDER/empty.img" \
    || _die "magiskboot_29 repack failed"
[ -s "$FOLDER/new-boot.img" ] || _die "new-boot.img is empty/missing after repack"

IMG_SIZE=$(stat -c %s "$FOLDER/new-boot.img")
[ "$IMG_SIZE" -gt 1048576 ] \
    || _die "new-boot.img suspiciously small: $IMG_SIZE bytes (< 1 MB)"
_log "new-boot.img size=$IMG_SIZE bytes"

IMG_HASH=$(sha256sum "$FOLDER/new-boot.img" | awk '{print $1}')
[ -n "$IMG_HASH" ] || _die "sha256sum failed for new-boot.img"
_log "new-boot.img sha256=$IMG_HASH"

echo "- Flashing to vendor_boot_a..."
dd if="$FOLDER/new-boot.img" of="$DEV_A" bs=4M conv=fsync 2>>"$LOGF" \
    || _die "dd flash to vendor_boot_a failed"
_log "flash vendor_boot_a done"

echo "- Flashing to vendor_boot_b..."
dd if="$FOLDER/new-boot.img" of="$DEV_B" bs=4M conv=fsync 2>>"$LOGF" \
    || _die "dd flash to vendor_boot_b failed"
_log "flash vendor_boot_b done"

sync

echo "- Verifying written images (parallel)..."

BLKS=$(( (IMG_SIZE + 1048575) / 1048576 ))

_verify_slot() {
    local dev="$1" result_file="$2"
    local hash
    hash=$(dd if="$dev" bs=1M count="$BLKS" 2>/dev/null \
            | head -c "$IMG_SIZE" \
            | sha256sum | awk '{print $1}')
    if [ "$hash" = "$IMG_HASH" ]; then
        echo "OK" > "$result_file"
    else
        printf 'FAIL:expected=%s got=%s\n' "$IMG_HASH" "$hash" > "$result_file"
    fi
}

RES_A="/tmp/rfv_a.result"
RES_B="/tmp/rfv_b.result"

_verify_slot "$DEV_A" "$RES_A" &
PID_A=$!
_verify_slot "$DEV_B" "$RES_B" &
PID_B=$!

wait $PID_A
wait $PID_B

RESULT_A=$(cat "$RES_A" 2>/dev/null)
RESULT_B=$(cat "$RES_B" 2>/dev/null)
_log "verify vendor_boot_a: $RESULT_A"
_log "verify vendor_boot_b: $RESULT_B"

VERIFY_OK=1
[ "$RESULT_A" = "OK" ] || { echo "ERROR: vendor_boot_a verify FAILED: $RESULT_A"; VERIFY_OK=0; }
[ "$RESULT_B" = "OK" ] || { echo "ERROR: vendor_boot_b verify FAILED: $RESULT_B"; VERIFY_OK=0; }
[ "$VERIFY_OK" = "1" ] || _die "Post-flash verification failed — partition data does not match image!"

echo "- Both slots verified OK (sha256 match)"
echo "- Recovery reflashed to both slots successfully"
_log "========== reflash_twrp END OK ==========="

exit 0