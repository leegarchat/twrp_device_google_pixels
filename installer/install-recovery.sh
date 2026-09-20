#!/sbin/sh
# OrangeFox vendor_boot on-device installer (recovery / Magisk /
# KernelSU root shell, fully non-interactive, no menus).
#
# Algorithm:
#   1. snapshot vendor_boot_<slot> blocks -> /tmp/recovery_install/
#   2. backup snapshots -> /sdcard/backup_vendor_boot/ (or
#      /data/media/0/backup_vendor_boot/ when userdata is writable;
#      skipped otherwise, install proceeds)
#   3. rebuild each snapshot with the payload via
#      `install --file` (verify in, rebuild, verify out)
#   4. set the blocks writable (blockdev --setrw, best effort),
#      dd the rebuilt images back, re-read + byte-compare proof
#
# Slots come from export.txt SLOT= (both|a|b|0|1|current, default
# both); `current` resolves via ro.boot.slot_suffix. Payload and
# binary live next to this script (bin/linux/, small build first).
#
# Overridable for testing (defaults are the production paths):
#   RECOVERY_TMPDIR, RECOVERY_BYNAME, RECOVERY_SDCARDS (colon list),
#   RECOVERY_EXPORT (export.txt path).
#
# Exit codes: 0 ok, 1 usage/environment, 2 snapshot/build/flash failure.
set -u

TMPDIR="${RECOVERY_TMPDIR:-/tmp/recovery_install}"
BYNAME="${RECOVERY_BYNAME:-/dev/block/by-name}"
SDCAND="${RECOVERY_SDCARDS:-/sdcard:/data/media/0}"
BACKUP_SUB="backup_vendor_boot"

ROOT="$(cd "$(dirname "$0")" && pwd)"
EXPORT="${RECOVERY_EXPORT:-$ROOT/export.txt}"

say() { echo "$*"; }
fail() { echo "  FAIL: $1"; exit 2; }
mb() { awk -v b="$1" 'BEGIN{printf "%.1f MB", b/1048576}'; }

# --- export.txt: KEY=VALUE, # comments, CR-tolerant ---
cfg() { grep -E "^$1=" "$EXPORT" 2>/dev/null | tail -1 | cut -d= -f2- | tr -d '\r' | sed 's/^ *//;s/ *$//'; }
[ -f "$EXPORT" ] || { echo "no export.txt next to the installer ($EXPORT)"; exit 1; }

PAYLOAD_NAME="$(cfg RECOVERY_IMG)"; [ -n "$PAYLOAD_NAME" ] || PAYLOAD_NAME="OrangeFox-R12.0-test5-aio.ramdisk.lz4"
SLOT_CFG="$(cfg SLOT)"; [ -n "$SLOT_CFG" ] || SLOT_CFG="both"
PAYLOAD="$ROOT/$PAYLOAD_NAME"
[ -f "$PAYLOAD" ] || { echo "missing recovery payload: $PAYLOAD"; exit 1; }
MIN_FREE_MB="$(cfg MIN_FREE_MB)"; [ -n "$MIN_FREE_MB" ] || MIN_FREE_MB=7
case "$MIN_FREE_MB" in ''|*[!0-9]*|'0') echo "bad MIN_FREE_MB in export.txt: '$MIN_FREE_MB' (want positive MiB integer)"; exit 1;; esac
MIN_FREE_BYTES=$(( MIN_FREE_MB * 1048576 ))

# --- binary: small build first, by uname -m ---
ARCH=""
case "$(uname -m 2>/dev/null || echo unknown)" in
    x86_64|amd64) ARCH="x86_64";;
    aarch64|arm64) ARCH="arm64";;
    armv7l|armv6l|armv8l) ARCH="arm32";;
    i686|i386) ARCH="x86";;
esac
BIN=""
if [ -n "$ARCH" ]; then
    for n in "install-small-linux-$ARCH" "install-linux-$ARCH"; do
        if [ -x "$ROOT/bin/linux/$n" ]; then BIN="$ROOT/bin/linux/$n"; break; fi
    done
fi
[ -n "$BIN" ] || { echo "no installer binary for arch '$ARCH' (expected bin/linux/install[-small]-linux-$ARCH)"; exit 1; }

# --- slots ---
case "$SLOT_CFG" in
    both) SLOTS="a b";;
    a|0) SLOTS="a";;
    b|1) SLOTS="b";;
    current)
        SUF="$(getprop ro.boot.slot_suffix 2>/dev/null | tr -d '\r' | sed 's/^_//')"
        case "$SUF" in
            a) SLOTS="a";; b) SLOTS="b";;
            *) echo "cannot resolve current slot (ro.boot.slot_suffix='$SUF')"; exit 1;;
        esac;;
    *) echo "bad SLOT in export.txt: '$SLOT_CFG' (want both|a|b|0|1|current)"; exit 1;;
esac

say "OrangeFox on-device vendor_boot installer"
say "  slots: $(echo "$SLOTS" | tr ' ' '+') (export SLOT=$SLOT_CFG)"
mkdir -p "$TMPDIR" || { echo "cannot create $TMPDIR"; exit 1; }
LOG="$TMPDIR/install.log"
: > "$LOG" || { echo "cannot write $LOG"; exit 1; }
say "  log: $LOG"

# --- [1/4] snapshot ---
say ""
say "== [1/4] snapshot blocks =="
for s in $SLOTS; do
    blk="$BYNAME/vendor_boot_$s"
    out="$TMPDIR/vendor_boot_${s}_orig.img"
    [ -e "$blk" ] || fail "block not found: $blk"
    dd if="$blk" of="$out" bs=1M 2>>"$LOG" >>"$LOG" || fail "snapshot vendor_boot_$s"
    sz="$(wc -c < "$out" 2>/dev/null | tr -d ' ')"
    [ -n "$sz" ] && [ "$sz" -gt 0 ] 2>/dev/null || fail "snapshot vendor_boot_$s is empty"
    say "  ok: vendor_boot_$s snapshot ($sz bytes)"
done

# --- [2/4] backup (fatal unless a copy fully lands) ---
# Every candidate is tried in turn: pre-existing dir, mkdir, a real
# multi-MB data-write probe, room for all snapshots, then the copy
# itself. Metadata-only ops (mkdir/touch) can pass on FUSE /sdcard
# while data writes fail with EPERM, so the probe forces bytes
# through the same path first; /data/media/0 usually works directly.
# A candidate failing at ANY stage yields to the next one. If dirs
# exist but nothing takes the set, the install dies (details in log);
# with no userdata at all (encrypted recovery) backup is skipped and
# the install proceeds. Flashing blind is never allowed.
say ""
say "== [2/4] backup =="
need_bytes=0
for s in $SLOTS; do
    sz="$(wc -c < "$TMPDIR/vendor_boot_${s}_orig.img" 2>/dev/null | tr -d ' ')"
    case "$sz" in ''|*[!0-9]*) fail "cannot size snapshot slot $s";; esac
    need_bytes=$(( need_bytes + sz ))
done
need_kb=$(( need_bytes / 1024 + 8192 ))
say "  need ~$(( need_kb / 1024 )) MB for snapshots"
BACKUP_DIR=""
COPIED=""
SEEN=0
oldifs="$IFS"; IFS=":"
for cand in $SDCAND; do
    [ -d "$cand" ] || { say "  $cand: absent, skipping"; continue; }
    mkdir -p "$cand/$BACKUP_SUB" 2>/dev/null || { say "  $cand: cannot mkdir, skipping"; continue; }
    SEEN=1
    # Data-write probe (4 MB): catches FUSE EPERM that mkdir/touch miss.
    rm -f "$cand/$BACKUP_SUB/.w" 2>/dev/null || true
    if dd if=/dev/zero of="$cand/$BACKUP_SUB/.w" bs=1M count=4 >>"$LOG" 2>&1; then
        got="$(wc -c < "$cand/$BACKUP_SUB/.w" 2>/dev/null | tr -d ' ')"
        [ "$got" = "4194304" ] || {
            say "  $cand: probe short (${got:-?} bytes) — trying next"
            rm -f "$cand/$BACKUP_SUB/.w" 2>/dev/null || true
            continue
        }
    else
        say "  $cand: data-write probe failed — trying next"
        rm -f "$cand/$BACKUP_SUB/.w" 2>/dev/null || true
        continue
    fi
    rm -f "$cand/$BACKUP_SUB/.w" 2>/dev/null || true
    free_kb="$(df -k "$cand/$BACKUP_SUB" 2>/dev/null | awk 'NR==2 {print $4}')"
    case "$free_kb" in ''|*[!0-9]*) free_kb=0;; esac
    if [ "$free_kb" -lt "$need_kb" ]; then
        say "  $cand: only $(( free_kb / 1024 )) MB free, need $(( need_kb / 1024 )) MB — trying next"
        continue
    fi
    say "  $cand: $(( free_kb / 1024 )) MB free, copying..."
    ok=1
    # NOTE: this loop runs under IFS=":" (candidate splitting), so it
    # must not rely on word-splitting $SLOTS — match a/b explicitly.
    for s in a b; do
        case " $SLOTS " in
            *" $s "*) ;;
            *) continue;;
        esac
        if cp "$TMPDIR/vendor_boot_${s}_orig.img" "$cand/$BACKUP_SUB/" >>"$LOG" 2>&1; then
            got="$(wc -c < "$cand/$BACKUP_SUB/vendor_boot_${s}_orig.img" 2>/dev/null | tr -d ' ')"
            want="$(wc -c < "$TMPDIR/vendor_boot_${s}_orig.img" 2>/dev/null | tr -d ' ')"
            [ "$got" = "$want" ] && [ -n "$got" ] || {
                say "  $cand: slot $s size mismatch after copy (want $want, got ${got:-?}) — trying next"
                ok=0; break
            }
        else
            say "  $cand: copy failed for slot $s (see $LOG) — trying next"
            ok=0; break
        fi
    done
    if [ "$ok" = 1 ]; then
        BACKUP_DIR="$cand/$BACKUP_SUB"
        COPIED=1
        break
    fi
    rm -f "$cand/$BACKUP_SUB"/vendor_boot_*_orig.img 2>/dev/null || true
done
IFS="$oldifs"
if [ -n "$COPIED" ]; then
    say "  ok: snapshots -> $BACKUP_DIR/"
elif [ "$SEEN" = 1 ]; then
    {
        say "  copies failed on every candidate (tried: $SDCAND)"
        echo "--- backup diagnostics ---"
        echo "need_kb=$need_kb slots='$SLOTS'"
        df -h 2>&1 || true
        echo "---"
        df -k 2>&1 || true
        echo "---"
        ls -ld $SDCAND 2>&1 || true
        echo "--- mounts ---"
        mount 2>/dev/null | grep -iE "sdcard|emulated|/data|media" || true
        echo "--- end diagnostics ---"
    } >>"$LOG" 2>&1
    say "  copies failed on every candidate (tried: $SDCAND, details in $LOG)"
    fail "backup failed on all locations, nothing flashed"
else
    say "  no writable userdata ($SDCAND): backup skipped"
fi

# --- [3/4] rebuild ---
say ""
say "== [3/4] rebuild =="
for s in $SLOTS; do
    "$BIN" install --file -i "$TMPDIR/vendor_boot_${s}_orig.img" -c "$PAYLOAD" \
        -o "$TMPDIR/recovery_${s}.img" --log "$TMPDIR/install-${s}.log" >>"$LOG" 2>&1 \
        || fail "rebuild slot $s (see $TMPDIR/install-${s}.log)"
    say "  ok: slot $s rebuilt ($(wc -c < "$TMPDIR/recovery_${s}.img" | tr -d ' ') bytes)"
done

# --- free-space policy (export MIN_FREE_MB, strict: no bypass) ---
say ""
say "== policy: keep ${MIN_FREE_MB} MB free =="
for s in $SLOTS; do
    img="$TMPDIR/recovery_${s}.img"
    blk="$BYNAME/vendor_boot_$s"
    isz="$(wc -c < "$img" | tr -d ' ')"
    psz="$(blockdev --getsize64 "$blk" 2>/dev/null | tr -d ' \r\n')"
    case "$psz" in ''|*[!0-9]*) psz="$(wc -c < "$TMPDIR/vendor_boot_${s}_orig.img" | tr -d ' ')";; esac
    free=$(( psz - isz ))
    [ "$free" -lt 0 ] && free_show=0 || free_show="$free"
    if [ "$free" -lt 0 ]; then
        say "  slot $s: $(mb "$isz") / $(mb "$psz"), free $(mb "$free_show") — DOES NOT FIT"
        fail "slot $s: image does not fit partition, nothing flashed"
    elif [ "$free" -lt "$MIN_FREE_BYTES" ]; then
        say "  slot $s: $(mb "$isz") / $(mb "$psz"), free $(mb "$free_show") — BELOW $MIN_FREE_MB MB POLICY"
        fail "slot $s: less than $MIN_FREE_MB MB free left (export.txt MIN_FREE_MB), nothing flashed"
    else
        say "  slot $s: $(mb "$isz") / $(mb "$psz"), free $(mb "$free_show") — OK"
    fi
done

# --- [4/4] flash + fetch-back proof ---
say ""
say "== [4/4] flash =="
for s in $SLOTS; do
    blk="$BYNAME/vendor_boot_$s"
    img="$TMPDIR/recovery_${s}.img"
    if command -v blockdev >/dev/null 2>&1; then
        blockdev --setrw "$blk" >>"$LOG" 2>&1 || true
    elif command -v setrw >/dev/null 2>&1; then
        setrw "$blk" >>"$LOG" 2>&1 || true
    fi
    dd if="$img" of="$blk" bs=1M 2>>"$LOG" >>"$LOG" || fail "flash vendor_boot_$s"
    sync 2>/dev/null || true
    say "  ok: vendor_boot_$s flashed"
done
for s in $SLOTS; do
    img="$TMPDIR/recovery_${s}.img"
    chk="$TMPDIR/check_${s}.img"
    sz="$(wc -c < "$img" | tr -d ' ')"
    cnt=$(( (sz + 1048575) / 1048576 ))
    if dd if="$BYNAME/vendor_boot_$s" of="$chk" bs=1M count="$cnt" 2>>"$LOG" >>"$LOG" \
        && cmp -n "$sz" "$img" "$chk" >>"$LOG" 2>&1; then
        say "  ok: slot $s on device matches"
    elif command -v cmp >/dev/null 2>&1; then
        fail "slot $s fetch-back mismatch (see log)"
    else
        say "  warn: no cmp tool, proof skipped for slot $s"
    fi
done

if [ -n "$BACKUP_DIR" ]; then
    cp "$LOG" "$TMPDIR"/install-*.log "$BACKUP_DIR/" 2>/dev/null || true
    say "  logs -> $BACKUP_DIR/"
fi
say ""
say "RESULT: OK (snapshots: $TMPDIR/; backup: ${BACKUP_DIR:-skipped})"
