#!/sbin/sh
# otg_test.sh — OTG field test for the /usb_otg single-path + dedup work.
#
# Run ON THE DEVICE (Fox terminal: Advanced -> Terminal) WHILE the USB
# drive is attached and the PC is UNPLUGGED — one port, one role: with adb
# connected the port is in device mode and the host bus (and the drive)
# is down by design, so host-side checks are meaningless there.
#
# Usage:
#   otg_test.sh [wait_secs]     # default 30: settle window for slow sticks
#
# Polls for enumeration, runs checks T1..T8 with PASS/FAIL verdicts, saves
# an evidence bundle next to the flog archives:
#   /data/media/0/fox_logs/otg_test_<device>_<family>_<stamp>.tar.gz
# (fallback /tmp/fox_logs). Exit code = number of failed checks
# (0 = drive fully works as USB-Storage).
#
# What PASS means per check:
#   T1 DISK   kernel enumerated a removable USB disk (sdX, canonical
#             /sys device path contains "usb")
#   T2 PART   first partition node /dev/block/sdX1 exists (superfloppy
#             whole-disk sdX without partitions also passes)
#   T3 DAEMON otg-auto service is running (it owns host mode + symlink)
#   T4 LINK   /dev/block/otg-usb -> /dev/block/sdX1 (the regression test
#             for the readlink-canonicalize fix: raw sysfs links are
#             relative and never matched "usb", so the link was never made)
#   T5 UEVENT recovery.log shows the add-uevent matched (Found a match)
#   T6 VOLUMES /auto0-N subpartition appeared for the partitions
#   T7 MOUNT  /usb_otg is mounted (the standard path); /auto0-1 mounted
#             instead (or as well) = dedup did not engage -> FAIL
#   T8 RW     canary write/read/delete on the mounted /usb_otg

WAIT="${1:-30}"
case "$WAIT" in
    ''|*[!0-9]*) WAIT=30 ;;
esac

PASS=0
FAIL=0
REPORT=""

say() {
    echo "otg_test: $1"
    REPORT="$REPORT$1
"
}
chk_pass() {
    PASS=$((PASS + 1))
    say "T$1 $2 ... PASS $3"
}
chk_fail() {
    FAIL=$((FAIL + 1))
    say "T$1 $2 ... FAIL $3"
}

say "starting (settle window ${WAIT}s) — drive must be attached, PC unplugged"

# --- pre-flight: PC attached means device mode, host checks are void ---
_VBUS="$(cat /sys/class/power_supply/usb/online 2>/dev/null)"
if [ "$_VBUS" = "1" ]; then
    say "WARN: VBUS=1 (PC/charger attached) -> port is in DEVICE mode;"
    say "WARN: host-side results below are expected to fail; re-run with PC unplugged"
fi

# --- settle: wait for the partition node (slow sticks need seconds) ---
DISK=""
elapsed=0
while [ "$elapsed" -lt "$WAIT" ]; do
    for _d in /sys/block/sd?; do
        [ -d "$_d" ] || continue
        [ "$(cat "$_d/removable" 2>/dev/null)" = "1" ] || continue
        _link="$(readlink -f "$_d/device" 2>/dev/null)"
        [ -n "$_link" ] || _link="$(readlink "$_d/device" 2>/dev/null)"
        case "$_link" in
            *usb*) DISK="$(basename "$_d")" ;;
        esac
        [ -n "$DISK" ] && break
    done
    if [ -n "$DISK" ] && { [ -b "/dev/block/${DISK}1" ] || [ -b "/dev/block/$DISK" ]; }; then
        break
    fi
    DISK=""
    sleep 2
    elapsed=$((elapsed + 2))
done

# --- T1: removable USB disk on the bus ---
if [ -n "$DISK" ]; then
    _model="$(cat "/sys/block/$DISK/device/model" 2>/dev/null)"
    _size="$(cat "/sys/block/$DISK/size" 2>/dev/null)"
    chk_pass 1 "DISK" "($DISK ${_model:-?} ${_size:-?} sectors)"
else
    chk_fail 1 "DISK" "(no removable USB sd disk in /sys/block)"
fi

# --- T2: partition node probed ---
PART=""
if [ -n "$DISK" ]; then
    if [ -b "/dev/block/${DISK}1" ]; then
        PART="${DISK}1"
        chk_pass 2 "PART" "(/dev/block/$PART)"
    elif [ -b "/dev/block/$DISK" ]; then
        PART="$DISK"
        chk_pass 2 "PART" "(superfloppy /dev/block/$PART, no partitions)"
    else
        chk_fail 2 "PART" "(/dev/block/${DISK}1 missing — probe stalled?)"
    fi
else
    chk_fail 2 "PART" "(skipped, no disk)"
fi

# --- T3: daemon owns the loop ---
_DAEMON="$(getprop init.svc.otg_auto 2>/dev/null)"
if [ "$_DAEMON" = "running" ]; then
    chk_pass 3 "DAEMON" "(otg-auto running)"
else
    chk_fail 3 "DAEMON" "(otg-auto state: ${_DAEMON:-unknown})"
fi

# --- T4: THE regression test — otg-usb symlink target ---
_LINK="$(readlink /dev/block/otg-usb 2>/dev/null)"
if [ -n "$PART" ]; then
    case "$_LINK" in
        "$PART"|"/dev/block/$PART"|"block/$PART")
            chk_pass 4 "LINK" "(otg-usb -> $_LINK)" ;;
        "")
            chk_fail 4 "LINK" "(missing — daemon never linked; dedup cannot engage)" ;;
        *)
            chk_fail 4 "LINK" "(otg-usb -> $_LINK, want $PART — stale/wrong disk?)" ;;
    esac
else
    chk_fail 4 "LINK" "(skipped, no partition)"
fi

# --- T5: kernel add-uevent reached recovery ---
if [ -z "$DISK" ]; then
    chk_fail 5 "UEVENT" "(skipped, no disk)"
elif grep -q "Found a match '$DISK'" /tmp/recovery.log 2>/dev/null; then
    chk_pass 5 "UEVENT" "('Found a match $DISK' in recovery.log)"
else
    chk_fail 5 "UEVENT" "(no match line for $DISK in recovery.log)"
fi

# --- T6: /auto0-N subpartition volumes appeared ---
_NVOL=0
for _a in /auto0-*; do
    [ -d "$_a" ] || continue
    case "$_a" in
        *-*) _NVOL=$((_NVOL + 1)) ;;
    esac
done
if [ "$_NVOL" -gt 0 ]; then
    chk_pass 6 "VOLUMES" "($_NVOL /auto0-N volume(s): $(echo /auto0-* | tr ' ' ','))"
elif grep -q "Created '/auto0-1' folder" /tmp/recovery.log 2>/dev/null; then
    chk_pass 6 "VOLUMES" "(created earlier per recovery.log, since removed)"
else
    chk_fail 6 "VOLUMES" "(no /auto0-N volumes)"
fi

# --- T7: the standard path is mounted, no duplicate mount ---
_UMOUNT="$(grep -E ' /usb_otg ' /proc/mounts 2>/dev/null | head -1)"
_AMOUNT="$(grep -E ' /auto0-[0-9]+ ' /proc/mounts 2>/dev/null | head -1)"
if [ -n "$_UMOUNT" ]; then
    chk_pass 7 "MOUNT" "(/usb_otg mounted: $(printf '%s' "$_UMOUNT" | cut -d' ' -f1-3))"
    [ -n "$_AMOUNT" ] && say "note: duplicate mount still present: $_AMOUNT"
else
    if [ -n "$_AMOUNT" ]; then
        chk_fail 7 "MOUNT" "(/usb_otg NOT mounted; old path mounted instead: $_AMOUNT)"
    else
        chk_fail 7 "MOUNT" "(neither /usb_otg nor /auto0-N mounted)"
    fi
fi

# --- T8: canary write/read/delete on the mounted standard path ---
_MP="$(printf '%s' "$_UMOUNT" | cut -d' ' -f2)"
if [ -n "$_MP" ] && [ -d "$_MP" ]; then
    _CAN="$_MP/.fox_otgtest"
    _STAMP="$(date '+%Y%m%d_%H%M%S' 2>/dev/null)"
    if printf '%s\n' "$_STAMP" > "$_CAN" 2>/dev/null \
        && [ "$(cat "$_CAN" 2>/dev/null)" = "$_STAMP" ] \
        && rm -f "$_CAN" 2>/dev/null && [ ! -e "$_CAN" ]; then
        chk_pass 8 "RW" "(canary round-trip on $_MP, cleaned up)"
    else
        rm -f "$_CAN" 2>/dev/null
        chk_fail 8 "RW" "(canary failed on $_MP — read-only mount?)"
    fi
else
    chk_fail 8 "RW" "(skipped, /usb_otg not mounted)"
fi

say "verdict: $PASS passed, $FAIL failed"

# --- evidence bundle (same home as flog archives) ---
_FOX_DEV="$(getprop ro.product.device 2>/dev/null)"
_FOX_FAM="$(getprop ro.recovery.soc_family 2>/dev/null)"
[ -n "$_FOX_DEV" ] || _FOX_DEV="unknown"
[ -n "$_FOX_FAM" ] || _FOX_FAM="unknown"
_FOX_DEV="$(printf '%s' "$_FOX_DEV" | tr -c '[:alnum:]_' '_' | tr '[:upper:]' '[:lower:]')"
_FOX_FAM="$(printf '%s' "$_FOX_FAM" | tr -c '[:alnum:]_' '_' | tr '[:upper:]' '[:lower:]')"
_STAMP="$(date '+%Y%m%d_%H%M%S' 2>/dev/null)"
[ -n "$_STAMP" ] || _STAMP="unknown_time"
ARCH="otg_test_${_FOX_DEV}_${_FOX_FAM}_$_STAMP.tar.gz"

BASE=""
for _cand in /data/media/0/fox_logs /tmp/fox_logs; do
    if [ -d "$(dirname "$_cand")" ] && mkdir -p "$_cand" 2>/dev/null \
        && touch "$_cand/.w" 2>/dev/null; then
        rm -f "$_cand/.w"
        BASE="$_cand"
        break
    fi
done
[ -n "$BASE" ] || { say "nowhere writable, report only"; echo "$REPORT"; exit "$FAIL"; }

DEST="$BASE/$_STAMP"
rm -rf "$DEST" 2>/dev/null
mkdir -p "$DEST" 2>/dev/null || { say "cannot create $DEST"; echo "$REPORT"; exit "$FAIL"; }

printf '%s\n' "$REPORT" > "$DEST/report.txt" 2>/dev/null
{
    echo "## verdict: $PASS passed, $FAIL failed"
    echo "## disk=$DISK part=$PART link=$_LINK"
    echo ""
    echo "## /dev/block sd + otg-usb"
    ls -la /dev/block/sd* /dev/block/otg-usb 2>/dev/null
    echo ""
    echo "## sysfs disks (model + sectors + removable + canonical device link)"
    for _d in /sys/block/sd?; do
        [ -d "$_d" ] || continue
        _b=$(basename "$_d")
        _cl="$(readlink -f "$_d/device" 2>/dev/null)"
        printf '%s: model=%s size=%s removable=%s devlink=%s\n' "$_b" \
            "$(cat "$_d/device/model" 2>/dev/null)" "$(cat "$_d/size" 2>/dev/null)" \
            "$(cat "$_d/removable" 2>/dev/null)" "${_cl:-?}"
    done
    echo ""
    echo "## /sys/bus/usb/devices"
    ls /sys/bus/usb/devices/ 2>/dev/null | tr '\n' ' '
    echo ""
    echo ""
    echo "## mounts (usb_otg/auto)"
    grep -E "usb_otg|/auto0" /proc/mounts 2>/dev/null || echo "(none)"
    echo ""
    echo "## daemon + VBUS + roles"
    printf 'init.svc.otg_auto=%s\n' "$(getprop init.svc.otg_auto 2>/dev/null)"
    printf 'usb/online=%s\n' "$(cat /sys/class/power_supply/usb/online 2>/dev/null)"
    for _r in /sys/class/usb_role/*/role; do
        [ -f "$_r" ] && printf '%s=%s\n' "$_r" "$(cat "$_r" 2>/dev/null)"
    done
    for _t in /sys/class/typec/port0/data_role /sys/class/typec/port0/power_role; do
        [ -f "$_t" ] && printf '%s=%s\n' "$_t" "$(cat "$_t" 2>/dev/null)"
    done
} > "$DEST/state.txt" 2>/dev/null
dmesg 2>/dev/null | grep -iE "usb|scsi|sd[a-z]|xhci|dwc3|otg|mass|storage|disconnect|reset|New USB|Attached" \
    > "$DEST/dmesg_usb.log" 2>/dev/null
grep -E "otg|OTG|usb_otg|auto0|sde|sd[a-z]|Kingston|Mass Storage|Found a match|exfat|fuse|Is_Mounted|was removed" \
    /tmp/recovery.log 2>/dev/null > "$DEST/recovery_otg.log" 2>/dev/null
chmod 755 "$DEST" 2>/dev/null
chmod 644 "$DEST"/* 2>/dev/null

if command -v tar >/dev/null 2>&1 \
    && tar -czf "$BASE/$ARCH" -C "$BASE" "$_STAMP" 2>/dev/null \
    && [ -f "$BASE/$ARCH" ]; then
    chmod 644 "$BASE/$ARCH" 2>/dev/null
    rm -rf "$DEST" 2>/dev/null
    say "DONE -> $BASE/$ARCH"
else
    say "tar failed, loose files in $DEST"
    say "DONE -> $DEST"
fi
case "$BASE" in
    /tmp/*) say "pull via: adb pull $BASE/$ARCH" ;;
    *) say "send this single file with the bug report" ;;
esac
exit "$FAIL"
