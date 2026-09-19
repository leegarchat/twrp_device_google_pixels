#!/sbin/sh

# reflash_twrp.sh — Reflash recovery to both vendor_boot slots (AIO build).
# Works on all Tensor Pixels — same installer flow for every family.
#
# Unlike the legacy script (magiskboot full-image repack with first_stage
# rebuild + kernel cmdline stamping), this follows the on-device installer
# structure (bootsmasher recovery_install_components/install-recovery.sh):
#
#   1. snapshot vendor_boot_a/b blocks -> work dir
#   2. backup snapshots -> /sdcard/backup_vendor_boot/ (best effort;
#      install proceeds without backup when userdata is unwritable)
#   3. build a recovery-ONLY cpio payload from /dev/ramdisk_snapshot
#      (no first_stage paths, no nboot, no first_stage lists) and rebuild
#      each slot image with it via `bootsmasher-install install --file` —
#      first_stage, header, cmdline and dtb come from each slot's own
#      stock image ("smart replace", same binary as the installer uses)
#   4. enforce MIN_FREE_MB policy, flash both slots, fetch-back verify
#
# stdout is captured by twrpRepacker and displayed in the recovery UI.

SNAP="/dev/ramdisk_snapshot"
FOLDER="/tmp/reflash_recovery"
LOGF="/tmp/reflash_twrp.log"
INST_BIN="/system/bin/bootsmasher-install"
# Same default as the installer (export.txt MIN_FREE_MB); strict, no bypass.
MIN_FREE_MB=7
BACKUP_SUB="backup_vendor_boot"

exec 2>>"$LOGF"

_log() { printf '[%s] %s\n' "$(date '+%H:%M:%S' 2>/dev/null)" "$*" >> "$LOGF"; }
_die() {
    echo "ERROR: $*"
    _log "FATAL: $*"
    exit 1
}

_log "========== reflash_twrp START (AIO) =========="
_log "device=$(getprop ro.hardware 2>/dev/null) uptime=$(awk '{print $1}' /proc/uptime 2>/dev/null)s"

echo "- Starting reflash current recovery (AIO snapshot-based)"

[ -d "$SNAP" ] \
    || _die "Ramdisk snapshot not found at $SNAP. Was ramdisk_snapshot run at boot?"

RECOVERY_LIST="$SNAP/recovery_file_list.txt"
[ -f "$RECOVERY_LIST" ] || _die "Missing snapshot file: $RECOVERY_LIST"
_log "snapshot list OK"

for _bin in "$INST_BIN" cpio dd cmp; do
    command -v "$_bin" >/dev/null 2>&1 || _die "Required binary not found: $_bin"
done
_log "binaries OK"

DEV_A="/dev/block/by-name/vendor_boot_a"
DEV_B="/dev/block/by-name/vendor_boot_b"
[ -b "$DEV_A" ] || _die "Block device not found: $DEV_A"
[ -b "$DEV_B" ] || _die "Block device not found: $DEV_B"
_log "block devices OK"

rm -rf "$FOLDER"
mkdir -p "$FOLDER" || _die "Cannot create $FOLDER"

# --- [1/4] snapshot vendor_boot blocks ---
echo "- Snapshotting vendor_boot blocks..."
for s in a b; do
    if [ "$s" = "a" ]; then _dev="$DEV_A"; else _dev="$DEV_B"; fi
    dd if="$_dev" of="$FOLDER/vendor_boot_${s}_orig.img" bs=1M 2>>"$LOGF" \
        || _die "snapshot vendor_boot_$s failed"
    _sz="$(wc -c < "$FOLDER/vendor_boot_${s}_orig.img" 2>/dev/null | tr -d ' ')"
    [ -n "$_sz" ] && [ "$_sz" -gt 0 ] 2>/dev/null \
        || _die "snapshot vendor_boot_$s is empty"
    _log "snapshot vendor_boot_$s: $_sz bytes"
done
echo "- Snapshots OK"

# --- [2/4] backup (best effort; install proceeds without it) ---
BACKUP_DIR=""
for cand in /sdcard /data/media/0; do
    if [ -d "$cand" ] && mkdir -p "$cand/$BACKUP_SUB" 2>/dev/null \
        && touch "$cand/$BACKUP_SUB/.w" 2>/dev/null; then
        rm -f "$cand/$BACKUP_SUB/.w"
        BACKUP_DIR="$cand/$BACKUP_SUB"
        break
    fi
done
if [ -z "$BACKUP_DIR" ]; then
    echo "- WARNING: no writable userdata, backup skipped"
    _log "WARNING: backup skipped (no writable userdata)"
else
    cp "$FOLDER"/vendor_boot_*_orig.img "$BACKUP_DIR/" 2>>"$LOGF" \
        || _die "backup copy failed"
    echo "- Backup -> $BACKUP_DIR/"
    _log "backup -> $BACKUP_DIR/"
fi

# --- [3/4] recovery-only payload + per-slot rebuild ---
# Payload = snapshot files minus everything that is not recovery content:
# first_stage paths (each slot's own stock image provides first_stage),
# the nboot base header and first_stage lists (magiskboot flow is gone).
# KEPT ON PURPOSE: lgz_cluster.lgz (packed files — binaries/libs/toybox —
# without it the reflashed image would miss every LGZ-packed file at the
# next boot), recovery_file_list.txt + ramdisk_snapshot_manifest.txt
# (the snapshot/reflash mechanism needs them in the running ramdisk).
# Entries absent from the snapshot (LGZ-packed, live only after unpack)
# are skipped here — the cluster inside the payload restores them.
echo "- Building recovery-only payload from snapshot..."
grep -v -e "^first_stage_ramdisk" -e "first_stage_ramdisk/" \
        -e "^nboot" -e "bin/nboot.lz4$" \
        -e "^ramdisk-files.txt$" \
        -e "^first_stage-ramdisk-files.txt$" -e "^first_stage_file_list.txt$" \
    "$RECOVERY_LIST" > "$FOLDER/payload_list_full.txt" \
    || _die "Cannot write payload list"
if grep -q "first_stage_ramdisk" "$FOLDER/payload_list_full.txt"; then
    _die "Payload list still contains first_stage paths (refusing)"
fi
: > "$FOLDER/payload_list.txt" || _die "Cannot write payload list"
while IFS= read -r _f; do
    [ -n "$_f" ] || continue
    if [ -e "$SNAP/$_f" ] || [ -L "$SNAP/$_f" ]; then
        printf '%s\n' "$_f" >> "$FOLDER/payload_list.txt"
    else
        _log "payload: skip (LGZ-packed, restored from cluster): $_f"
    fi
done < "$FOLDER/payload_list_full.txt"

cd "$SNAP" || _die "Cannot cd to $SNAP"
cpio -H newc -o < "$FOLDER/payload_list.txt" > "$FOLDER/payload.cpio" 2>>"$LOGF" \
    || _die "cpio failed building recovery payload"
[ -s "$FOLDER/payload.cpio" ] || _die "payload.cpio is empty after cpio"
_log "payload.cpio size=$(stat -c %s "$FOLDER/payload.cpio" 2>/dev/null) bytes"

echo "- Rebuilding slot images (smart replace, stock first_stage kept)..."
for s in a b; do
    "$INST_BIN" install --file -i "$FOLDER/vendor_boot_${s}_orig.img" \
        -c "$FOLDER/payload.cpio" -o "$FOLDER/recovery_${s}.img" \
        --log "$FOLDER/install-${s}.log" >>"$LOGF" 2>&1 \
        || _die "rebuild slot $s failed (see $FOLDER/install-${s}.log)"
    _log "rebuild slot $s OK"
done
echo "- Rebuild OK"

# --- policy: keep MIN_FREE_MB free (strict, like the installer) ---
echo "- Checking free-space policy (>= $MIN_FREE_MB MB per slot)..."
MIN_FREE_BYTES=$(( MIN_FREE_MB * 1048576 ))
for s in a b; do
    if [ "$s" = "a" ]; then _dev="$DEV_A"; else _dev="$DEV_B"; fi
    _isz="$(wc -c < "$FOLDER/recovery_${s}.img" | tr -d ' ')"
    _psz="$(blockdev --getsize64 "$_dev" 2>/dev/null | tr -d ' \r\n')"
    case "$_psz" in ''|*[!0-9]*) _psz="$(wc -c < "$FOLDER/vendor_boot_${s}_orig.img" | tr -d ' ')";; esac
    _free=$(( _psz - _isz ))
    if [ "$_free" -lt "$MIN_FREE_BYTES" ]; then
        _die "slot $s: only $(( _free / 1048576 )) MB free (need $MIN_FREE_MB), nothing flashed"
    fi
    _log "slot $s: image $_isz / partition $_psz, free $_free — OK"
done

# --- [4/4] flash + fetch-back proof ---
echo "- Flashing to vendor_boot_a..."
if command -v blockdev >/dev/null 2>&1; then
    blockdev --setrw "$DEV_A" >>"$LOGF" 2>&1 || true
fi
dd if="$FOLDER/recovery_a.img" of="$DEV_A" bs=1M conv=fsync 2>>"$LOGF" \
    || _die "dd flash to vendor_boot_a failed"
_log "flash vendor_boot_a done"

echo "- Flashing to vendor_boot_b..."
if command -v blockdev >/dev/null 2>&1; then
    blockdev --setrw "$DEV_B" >>"$LOGF" 2>&1 || true
fi
dd if="$FOLDER/recovery_b.img" of="$DEV_B" bs=1M conv=fsync 2>>"$LOGF" \
    || _die "dd flash to vendor_boot_b failed"
_log "flash vendor_boot_b done"

sync

echo "- Verifying written images..."
for s in a b; do
    if [ "$s" = "a" ]; then _dev="$DEV_A"; else _dev="$DEV_B"; fi
    _sz="$(wc -c < "$FOLDER/recovery_${s}.img" | tr -d ' ')"
    _cnt=$(( (_sz + 1048575) / 1048576 ))
    if dd if="$_dev" of="$FOLDER/check_${s}.img" bs=1M count="$_cnt" 2>>"$LOGF" \
        && cmp -n "$_sz" "$FOLDER/recovery_${s}.img" "$FOLDER/check_${s}.img" >>"$LOGF" 2>&1; then
        _log "verify vendor_boot_$s: OK"
    else
        _die "vendor_boot_$s fetch-back mismatch — partition data does not match image!"
    fi
done

echo "- Both slots verified OK (byte match)"
echo "- Recovery reflashed to both slots successfully"
_log "========== reflash_twrp END OK =========="

exit 0
