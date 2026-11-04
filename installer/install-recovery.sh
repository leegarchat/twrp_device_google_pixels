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
#      `install --file` (verify in, rebuild, verify out) + the
#      per-device touch archive (bin/modules/<codename>.lgz next to
#      this script -> injected into the recovery payload at its root)
#   4. set the blocks writable (blockdev --setrw, best effort),
#      dd the rebuilt images back, re-read + byte-compare proof
#
# Slots come from export.txt SLOT= (both|a|b|0|1|current, default
# both); `current` resolves via ro.boot.slot_suffix. Payload and
# binary live next to this script; the tool tree (bin/) is unpacked on
# first run from the bin.lgz cluster via unpack_bin/ lean decoders.
#
# Overridable for testing (defaults are the production paths):
#   RECOVERY_TMPDIR, RECOVERY_BYNAME, RECOVERY_SDCARDS (colon list),
#   RECOVERY_EXPORT (export.txt path).
#
# Exit codes: 0 ok, 1 usage/environment, 2 snapshot/build/flash failure.
set -u

# Pure-shell script dir (no dirname dependency: bare envs may miss it).
case "$0" in
    */*) ROOT="${0%/*}";;
    *) ROOT="$PWD";;
esac
case "$ROOT" in /*) ;; *) ROOT="$PWD/$ROOT";; esac

# --- busybox bootstrap (must run before any external tool) ---
# Bare recoveries (LOS AOSP) and manager sandboxes (KernelSU) can miss
# base tools (awk, mkdir, ...). The zip ships Magisk busybox per arch in
# unpack_bin/; symlink-farm only the tools that are actually missing so
# system tools keep priority. PATH export covers child processes too.
ARCH=""
case "$(uname -m 2>/dev/null || echo unknown)" in
    x86_64|amd64) ARCH="x86_64";;
    aarch64|arm64) ARCH="arm64";;
esac
[ -n "$ARCH" ] || case "$(getprop ro.product.cpu.abi 2>/dev/null)" in
    arm64*) ARCH="arm64";; x86_64*) ARCH="x86_64";;
esac
_BB="$ROOT/unpack_bin/busybox-$ARCH"
if [ -n "$ARCH" ] && [ -f "$_BB" ]; then
    chmod +x "$_BB" 2>/dev/null || true
    if "$_BB" true 2>/dev/null; then
        _FARM="$ROOT/.bb"
        "$_BB" mkdir -p "$_FARM" 2>/dev/null || mkdir -p "$_FARM" 2>/dev/null || true
        for _t in awk mkdir wc df dd cp rm mv grep sed cut tr cat chmod uname head tail ls ln dirname; do
            if ! command -v "$_t" >/dev/null 2>&1; then
                "$_BB" ln -sf "$_BB" "$_FARM/$_t" 2>/dev/null || ln -sf "$_BB" "$_FARM/$_t" 2>/dev/null || true
            fi
        done
        PATH="$_FARM:$PATH"; export PATH
    fi
fi
unset _BB _FARM _t
# Float-capable awk if it really runs (doubles: any size, incl. >2GB
# that would overflow 32-bit shell arithmetic). Probed once.
_USE_AWK=""
if awk 'BEGIN{exit 0}' 2>/dev/null; then _USE_AWK=1; fi
echo "awk float: ${_USE_AWK:+yes}"

# --- recovery flavor + LOS enforce dance (first thing, before unpack) ---
# LOS/AOSP-based restricted recoveries (no /twres, AOSP /res/images
# layout, lineage props when present) over-restrict root: if such a
# recovery runs Enforcing, relax to 0 for the install and restore the
# original mode on error (fail()) and at normal end.
detect_recovery() {
    if [ -n "$(getprop ro.orangefox.version 2>/dev/null)" ] || command -v twrp >/dev/null 2>&1; then echo orangefox; return; fi
    if [ -n "$(getprop ro.twrp.version 2>/dev/null)" ] || [ -e /twres ]; then echo twrp; return; fi
    if [ -n "$(getprop ro.lineage.version 2>/dev/null)$(getprop ro.lineage.build.version 2>/dev/null)" ]; then echo lineage; return; fi
    if [ -d /res/images ]; then echo aosp; return; fi
    echo unknown
}
RECOVERY_FLAVOR="$(detect_recovery)"
DID_RELAX=""
# Relax only inside an actual recovery: the Magisk/KernelSU module path
# runs on booted Android (possibly LineageOS) where SELinux must not
# be touched. Distinguisher: booted Android mounts system-as-root (a
# block/dm device on /); recovery boots from ramdisk (rootfs on /)
# and mounts system at /system_root, if at all.
in_recovery() {
    _rd=""
    while read -r _d _m _o; do
        if [ "$_m" = / ]; then _rd="$_d"; break; fi
    done </proc/mounts 2>/dev/null
    case "$_rd" in
        rootfs|ramdisk|tmpfs) return 0;;
        *) return 1;;
    esac
}
if { [ "$RECOVERY_FLAVOR" = lineage ] || [ "$RECOVERY_FLAVOR" = aosp ]; } && in_recovery; then
    if [ "$(cat /sys/fs/selinux/enforce 2>/dev/null)" = 1 ]; then
        setenforce 0 2>/dev/null || /system/bin/setenforce 0 2>/dev/null || true
        [ "$(cat /sys/fs/selinux/enforce 2>/dev/null)" = 0 ] && DID_RELAX=1
    fi
fi
echo "recovery: $RECOVERY_FLAVOR, selinux: $(cat /sys/fs/selinux/enforce 2>/dev/null || echo ?)"
restore_enforce() {
    if [ -n "$DID_RELAX" ]; then
        setenforce 1 2>/dev/null || /system/bin/setenforce 1 2>/dev/null || true
        echo "selinux restored: $(cat /sys/fs/selinux/enforce 2>/dev/null || echo ?)"
    fi
}

TMPDIR_WANT="${RECOVERY_TMPDIR:-/tmp/recovery_install}"
BYNAME="${RECOVERY_BYNAME:-/dev/block/by-name}"
SDCAND="${RECOVERY_SDCARDS:-/sdcard:/data/media/0}"
BACKUP_SUB="backup_vendor_boot"

EXPORT="${RECOVERY_EXPORT:-$ROOT/export.txt}"

say() { echo "$*"; }
fail() {
    echo "  FAIL: $1"
    restore_enforce
    [ -n "$TMPDIR" ] && [ -d "$TMPDIR" ] && chmod -R a+rX "$TMPDIR" 2>/dev/null || true
    echo "selinux enforce now: $(cat /sys/fs/selinux/enforce 2>/dev/null || echo ?)"
    exit 2
}
# Human MB without relying on awk (broken/absent on bare recoveries).
# awk path (floats) when alive; shell fallback only for <2GB inputs
# (safe under 32-bit arithmetic); bigger inputs print as raw bytes
# instead of overflowing silently.
mb() {
    if [ -n "$_USE_AWK" ]; then awk -v b="$1" 'BEGIN{printf "%.1f MB", b/1048576}'; return; fi
    case "$1" in
        ''|*[!0-9]*) echo "? MB"; return;;
    esac
    if [ "${#1}" -gt 9 ]; then echo "$1 bytes"; return; fi
    echo "$(( $1 / 1048576 )).$(( ($1 % 1048576) * 10 / 1048576 )) MB"
}

# --- export.txt: KEY=VALUE, # comments, CR-tolerant ---
cfg() { grep -E "^$1=" "$EXPORT" 2>/dev/null | tail -1 | cut -d= -f2- | tr -d '\r' | sed 's/^ *//;s/ *$//'; }
[ -f "$EXPORT" ] || { echo "no export.txt next to the installer ($EXPORT)"; exit 1; }

PAYLOAD_NAME="$(cfg RECOVERY_IMG)"; [ -n "$PAYLOAD_NAME" ] || PAYLOAD_NAME="OrangeFox-R12.0-test5-aio.ramdisk.lz4"
SLOT_CFG="$(cfg SLOT)"; [ -n "$SLOT_CFG" ] || SLOT_CFG="both"
PAYLOAD="$ROOT/$PAYLOAD_NAME"
[ -f "$PAYLOAD" ] || { echo "missing recovery payload: $PAYLOAD"; exit 1; }
MIN_FREE_MB="$(cfg MIN_FREE_MB)"; [ -n "$MIN_FREE_MB" ] || MIN_FREE_MB=10
case "$MIN_FREE_MB" in ''|*[!0-9]*|'0') echo "bad MIN_FREE_MB in export.txt: '$MIN_FREE_MB' (want positive MiB integer)"; exit 1;; esac
MIN_FREE_BYTES=$(( MIN_FREE_MB * 1048576 ))

# --- device: codename for pool validation + touch-section selection ---
# getprop works in booted Android and in recovery. Empty = proceed legacy
# (no --device, no touch section); a codename outside the Pixel pool fails
# inside the binary with exit 1. Remembered: codename -> $TMPDIR/device.txt.
DEVICE="$(getprop ro.product.device 2>/dev/null | tr -d '\r' | sed 's/^ *//;s/ *$//')"
[ -n "$DEVICE" ] || DEVICE="$(getprop ro.build.product 2>/dev/null | tr -d '\r' | sed 's/^ *//;s/ *$//')"
DEVICE_ARGS=""
INJECT_ARGS=""
if [ -n "$DEVICE" ]; then
    DEVICE_ARGS="--device $DEVICE"
    # Touch archive resolution lives AFTER the bin.lgz unpack block
    # below: in the cluster layout bin/ only exists after unpack, so an
    # early check would always miss (recovery path). Raw layouts (bin/
    # shipped in the zip) resolve on that same late pass.
else
    say "  WARNING: cannot detect device codename (getprop empty), installing without --device/touch archive"
fi

# --- binary: small build first, by uname -m ---
ARCH=""
case "$(uname -m 2>/dev/null || echo unknown)" in
    x86_64|amd64) ARCH="x86_64";;
    aarch64|arm64) ARCH="arm64";;
    armv7l|armv6l|armv8l) ARCH="arm32";;
    i686|i386) ARCH="x86";;
esac

# ---- bin.lgz first-run unpack ----
# The zip ships every tool as one lgz cluster (bin.lgz) plus per-OS lean
# decoders in unpack_bin/. Unpack once into $ROOT/bin; the marker pins
# the cluster byte size so re-runs skip and updates re-unpack.
# RECOVERY_UNPACK_FORCE=1 (or FOX_UNPACK_FORCE=1) forces a refresh.
# Without bin.lgz (classic layout) this block is a no-op.
_UP_LGZ="$ROOT/bin.lgz"
_UP_DST="$ROOT/bin"
_UP_DEC="$ROOT/unpack_bin/lgz-lean-linux-$ARCH"
if [ -f "$_UP_LGZ" ]; then
    _UP_FORCE="${RECOVERY_UNPACK_FORCE:-${FOX_UNPACK_FORCE:-0}}"
    _UP_NEED=1
    if [ "$_UP_FORCE" != 1 ] && [ -f "$_UP_DST/.lgz-cluster-size" ]; then
        _UP_WANT="$(wc -c < "$_UP_LGZ" | tr -d ' ')"
        _UP_HAVE="$(cat "$_UP_DST/.lgz-cluster-size" 2>/dev/null | tr -d ' \r\n')"
        if [ -n "$_UP_WANT" ] && [ "$_UP_WANT" = "$_UP_HAVE" ]; then
            _UP_NEED=0
        fi
    fi
    if [ "$_UP_NEED" = 1 ]; then
        if [ -z "$ARCH" ] || [ ! -f "$_UP_DEC" ]; then
            echo "no lgz decoder for linux/$ARCH (want unpack_bin/lgz-lean-linux-$ARCH next to install-recovery.sh)" >&2
            exit 1
        fi
        chmod +x "$_UP_DEC" 2>/dev/null || true
        [ -x "$_UP_DEC" ] || { echo "lgz decoder not executable: $_UP_DEC" >&2; exit 1; }
        echo "Unpacking tools (bin.lgz)..."
        "$_UP_DEC" decompress "$_UP_LGZ" "$_UP_DST" || { echo "bin.lgz unpack failed" >&2; exit 1; }
        wc -c < "$_UP_LGZ" | tr -d ' ' > "$_UP_DST/.lgz-cluster-size"
    fi
fi
unset _UP_LGZ _UP_DST _UP_DEC _UP_FORCE _UP_NEED _UP_WANT _UP_HAVE
# Touch archive, resolved AFTER unpack: bin/modules/<device>.lgz exists
# here in every layout (unpacked from bin.lgz, or shipped raw).
if [ -n "$DEVICE" ]; then
    MODULES_DIR="$(cfg MODULES_DIR)"; [ -n "$MODULES_DIR" ] || MODULES_DIR="modules"
    _lgz="$ROOT/$MODULES_DIR/$DEVICE.lgz"
    if [ -f "$_lgz" ]; then
        INJECT_ARGS="--inject-lgz $_lgz"
        say "  touch archive: $MODULES_DIR/$DEVICE.lgz -> payload root"
    else
        say "  WARNING: no $MODULES_DIR/$DEVICE.lgz next to installer, installing without touch archive"
    fi
fi
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
# Booted Android has no /tmp (root / is read-only), while recovery does.
# Try the wanted dir first, then writable fallbacks. A stale file at the
# path (not a dir) is removed before retrying.
TMPDIR=""
for _cand in "$TMPDIR_WANT" "/data/local/tmp/recovery_install" "/data/adb/recovery_install.tmp"; do
    [ -n "$_cand" ] || continue
    [ -e "$_cand" ] && [ ! -d "$_cand" ] && rm -f "$_cand" 2>/dev/null || true
    if mkdir -p "$_cand" 2>/dev/null; then TMPDIR="$_cand"; break; fi
done
[ -n "$TMPDIR" ] || { echo "cannot create tmpdir (tried $TMPDIR_WANT /data/local/tmp/recovery_install /data/adb/recovery_install.tmp)"; exit 1; }
[ "$TMPDIR" != "$TMPDIR_WANT" ] && say "  tmp: $TMPDIR_WANT unavailable, using $TMPDIR"
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
# True if $1 sits on a mounted fs other than rootfs: filters out fake
# empty dirs (unmounted /sdcard on encrypted userdata) so we never
# probe/copy into rootfs RAM and df always has a row to parse.
is_mounted() {
    _mp="$1"
    if [ -L "$_mp" ]; then
        _rl="$(ls -ld "$_mp" 2>/dev/null | sed -n 's/.*-> //p')"
        [ -n "$_rl" ] && _mp="$_rl"
    fi
    while [ -n "$_mp" ] && [ "$_mp" != "/" ] && [ "$_mp" != "." ]; do
        if grep -qE " $_mp " /proc/mounts 2>/dev/null; then return 0; fi
        _mp="${_mp%/*}"
        [ -z "$_mp" ] && _mp="/"
    done
    return 1
}
# Available KB for the fs holding $1. Resolves the owning mount via
# /proc/mounts (walk up), then takes the device row from a FULL df:
# df hides duplicate-device mounts (sda34 on both /data and /sdcard),
# so a direct `df -k path` may print no row at all. All-builtin.
# Called from the candidate loop which runs under IFS=":" - reset
# to whitespace for read-splitting, restore before return.
fs_avail_kb() {
    _fifs="$IFS"; IFS=" 	"
    _fp="$1"; _fdev=""; _fav=""
    while [ -n "$_fp" ] && [ "$_fp" != / ]; do
        while read -r _d _m _o; do
            if [ "$_m" = "$_fp" ]; then _fdev="$_d"; break; fi
        done </proc/mounts
        [ -n "$_fdev" ] && break
        _fp="${_fp%/*}"; [ -z "$_fp" ] && _fp=/
        [ "$_fp" = / ] && break
    done
    if [ -n "$_fdev" ]; then
        _fav="$(df -k 2>/dev/null | {
            read -r _h
            while read -r _f _t _u _a _r; do
                if [ "$_f" = "$_fdev" ]; then echo "$_a"; break; fi
            done
        })"
    fi
    IFS="$_fifs"
    echo "$_fav"
}
BACKUP_DIR=""
COPIED=""
SEEN=0
oldifs="$IFS"; IFS=":"
for cand in $SDCAND; do
    [ -d "$cand" ] || { say "  $cand: absent, skipping"; continue; }
    is_mounted "$cand" || { say "  $cand: not mounted, skipping"; continue; }
    mkdir -p "$cand/$BACKUP_SUB" 2>/dev/null || { say "  $cand: cannot mkdir, skipping"; continue; }
    SEEN=1
    # Data-write probe (4 MB): catches FUSE EPERM that mkdir/touch miss.
    rm -f "$cand/$BACKUP_SUB/.w" 2>/dev/null || true
    if dd if=/dev/zero of="$cand/$BACKUP_SUB/.w" bs=1M count=4 >>"$LOG" 2>&1; then
        got="$(wc -c < "$cand/$BACKUP_SUB/.w" 2>/dev/null | tr -d ' ')"
        [ "$got" = "4194304" ] || {
            say "  $cand: probe short (${got:-?} bytes) - trying next"
            rm -f "$cand/$BACKUP_SUB/.w" 2>/dev/null || true
            continue
        }
    else
        say "  $cand: data-write probe failed - trying next"
        rm -f "$cand/$BACKUP_SUB/.w" 2>/dev/null || true
        continue
    fi
    rm -f "$cand/$BACKUP_SUB/.w" 2>/dev/null || true
    free_kb="$(fs_avail_kb "$cand/$BACKUP_SUB" 2>/dev/null || true)"
    case "$free_kb" in ''|*[!0-9]*) free_kb=0;; esac
    if [ "$free_kb" -lt "$need_kb" ]; then
        say "  $cand: only $(( free_kb / 1024 )) MB free, need $(( need_kb / 1024 )) MB - trying next"
        continue
    fi
    say "  $cand: $(( free_kb / 1024 )) MB free, copying..."
    ok=1
    # NOTE: this loop runs under IFS=":" (candidate splitting), so it
    # must not rely on word-splitting $SLOTS - match a/b explicitly.
    for s in a b; do
        case " $SLOTS " in
            *" $s "*) ;;
            *) continue;;
        esac
        if cp "$TMPDIR/vendor_boot_${s}_orig.img" "$cand/$BACKUP_SUB/" >>"$LOG" 2>&1; then
            got="$(wc -c < "$cand/$BACKUP_SUB/vendor_boot_${s}_orig.img" 2>/dev/null | tr -d ' ')"
            want="$(wc -c < "$TMPDIR/vendor_boot_${s}_orig.img" 2>/dev/null | tr -d ' ')"
            [ "$got" = "$want" ] && [ -n "$got" ] || {
                say "  $cand: slot $s size mismatch after copy (want $want, got ${got:-?}) - trying next"
                ok=0; break
            }
        else
            say "  $cand: copy failed for slot $s (see $LOG) - trying next"
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
        _oifs2="$IFS"; IFS=":"
        for _c in $SDCAND; do ls -ld "$_c" 2>&1 || true; done
        IFS="$_oifs2"; unset _oifs2 _c
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
# Install layout type from export.txt RECOVERY_IS_PLATFORM (empty = Type A
# classic; var1 = Type B all-in-platform; var2 = Type C payload-only
# platform). Passed only when set: old binaries reject unknown flags.
RIP_ARGS=""
_RIP_VAL="$(cfg RECOVERY_IS_PLATFORM)"
case "$_RIP_VAL" in
    "") ;;
    var1|var2) RIP_ARGS="--recovery-is-platform $_RIP_VAL"
        say "  layout: Type $([ "$_RIP_VAL" = var1 ] && echo B || echo C) ($_RIP_VAL)" ;;
    *) fail "bad RECOVERY_IS_PLATFORM in export.txt: '$_RIP_VAL' (want empty|var1|var2)" ;;
esac
for s in $SLOTS; do
    _blk="$BYNAME/vendor_boot_$s"
    _psz="$(blockdev --getsize64 "$_blk" 2>/dev/null | tr -d ' \r\n')"
    case "$_psz" in ''|*[!0-9]*) _psz="$(wc -c < "$TMPDIR/vendor_boot_${s}_orig.img" | tr -d ' ')";; esac
    # shellcheck disable=SC2086 (flag args are pre-split intentionally)
    "$BIN" install --file -i "$TMPDIR/vendor_boot_${s}_orig.img" -c "$PAYLOAD" \
        -o "$TMPDIR/recovery_${s}.img" --log "$TMPDIR/install-${s}.log" $RIP_ARGS \
        $DEVICE_ARGS $INJECT_ARGS --partition-size "$_psz" --min-free-mb "$MIN_FREE_MB" >>"$LOG" 2>&1 \
        || fail "rebuild slot $s (see $TMPDIR/install-${s}.log)"
    say "  ok: slot $s rebuilt ($(wc -c < "$TMPDIR/recovery_${s}.img" | tr -d ' ') bytes)"
done
[ -n "$DEVICE" ] && echo "$DEVICE" > "$TMPDIR/device.txt" 2>/dev/null || true

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
        say "  slot $s: $(mb "$isz") / $(mb "$psz"), free $(mb "$free_show") - DOES NOT FIT"
        fail "slot $s: image does not fit partition, nothing flashed"
    elif [ "$free" -lt "$MIN_FREE_BYTES" ]; then
        say "  slot $s: $(mb "$isz") / $(mb "$psz"), free $(mb "$free_show") - BELOW $MIN_FREE_MB MB POLICY"
        fail "slot $s: less than $MIN_FREE_MB MB free left (export.txt MIN_FREE_MB), nothing flashed"
    else
        say "  slot $s: $(mb "$isz") / $(mb "$psz"), free $(mb "$free_show") - OK"
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
# World-readable logs for post-run inspection; original SELinux mode
# restored (was relaxed at start only on LOS/AOSP restricted recoveries).
chmod -R a+rX "$TMPDIR" 2>/dev/null || true
restore_enforce
