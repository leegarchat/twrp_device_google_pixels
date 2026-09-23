#!/sbin/sh
# flog.sh — one-shot log collector for OrangeFox test reports.
#
# Usage (adb shell or Fox terminal):
#   flog.sh
#
# Collects recovery.log, weaver.log, dmesg, logcat, props, slot info,
# reflash/install logs, backlight state and a hardware inventory
# (by-name map, UDC/role, i2c/gpio, DT flash node, power, drm, mounts)
# into ONE archive:
#   /data/media/0/fox_logs/fox_logs_<timestamp>.tar.gz
# (userdata decrypted + writable). When userdata is unavailable, falls
# back to /tmp/fox_logs/fox_logs_<timestamp>.tar.gz and prints an
# `adb pull` hint. The staging dir is removed after packing, so the
# tester sends a single file.
# stdout doubles as the "what to send" checklist for the tester.

DEST_BASE_CANDS="/data/media/0/fox_logs /tmp/fox_logs"
STAMP="$(date '+%Y%m%d_%H%M%S' 2>/dev/null)"
[ -n "$STAMP" ] || STAMP="unknown_time"
ARCH="fox_logs_$STAMP.tar.gz"

BASE=""
for _cand in $DEST_BASE_CANDS; do
    if [ -d "$(dirname "$_cand")" ] && mkdir -p "$_cand" 2>/dev/null \
        && touch "$_cand/.w" 2>/dev/null; then
        rm -f "$_cand/.w"
        BASE="$_cand"
        break
    fi
done
[ -n "$BASE" ] || { echo "flog: nowhere writable, aborting"; exit 1; }

DEST="$BASE/$STAMP"
rm -rf "$DEST" 2>/dev/null
mkdir -p "$DEST" 2>/dev/null || { echo "flog: cannot create $DEST, aborting"; exit 1; }

echo "flog: collecting to $DEST"

# --- recovery core logs (full files, never excerpts) ---
for _f in /tmp/recovery.log /tmp/weaver.log /tmp/reflash_twrp.log; do
    _b=$(basename "$_f")
    if [ -f "$_f" ]; then cp -f "$_f" "$DEST/$_b" 2>/dev/null && echo "flog: + $_b" \
        || echo "flog: ! copy failed: $_f"
    else
        echo "flog: - missing: $_f"
    fi
done
for _l in /tmp/reflash_recovery/install-*.log; do
    [ -f "$_l" ] || continue
    cp -f "$_l" "$DEST/" 2>/dev/null && echo "flog: + $(basename "$_l")"
done

# --- early-boot engine logs (stub swap + rust init stages) ---
# The stub mirrors its log to /aio_stub.log (rootfs "/" is never
# over-mounted; /tmp gets a tmpfs over it and the copy is lost).
for _pair in /tmp/aio_stub.log:aio_stub_tmp.log /aio_stub.log:aio_stub.log /dev/logs/runatinit.log:aio_runatinit.log; do
    _f="${_pair%%:*}"
    _b="${_pair##*:}"
    if [ -f "$_f" ]; then cp -f "$_f" "$DEST/$_b" 2>/dev/null \
        && echo "flog: + $_b"
    else
        echo "flog: - missing: $_f"
    fi
done

# --- kernel + system logs ---
if command -v dmesg >/dev/null 2>&1 && dmesg > "$DEST/dmesg.log" 2>/dev/null \
    && [ -s "$DEST/dmesg.log" ]; then
    echo "flog: + dmesg.log"
else
    rm -f "$DEST/dmesg.log"
    echo "flog: ! dmesg failed"
fi
if command -v logcat >/dev/null 2>&1 && logcat -d -v time > "$DEST/logcat.log" 2>&1 \
    && [ -s "$DEST/logcat.log" ]; then
    echo "flog: + logcat.log"
else
    rm -f "$DEST/logcat.log"
    echo "flog: - logcat unavailable, skipped"
fi

# --- device identity + runtime state ---
{
    echo "## props"
    getprop 2>/dev/null
    echo ""
    echo "## uname"
    uname -a 2>/dev/null
    echo ""
    echo "## key facts"
    for _p in ro.hardware ro.product.device ro.product.model ro.boot.slot_suffix ro.build.version.incremental DOF_SCREEN_W DOF_SCREEN_H DOF_PROGRESSIVE_SCALE DOF_STATUS_H DOF_THEME ro.recovery.keymint sys.usb.controller sys.usb.config tw_screen_timeout_secs; do
        printf '%s=%s\n' "$_p" "$(getprop "$_p" 2>/dev/null)"
    done
} > "$DEST/props.txt" 2>/dev/null
echo "flog: + props.txt"

# --- backlight state (screen issues: values while broken are evidence) ---
{
    for _bl in /sys/class/backlight/*; do
        [ -d "$_bl" ] || continue
        echo "== $_bl"
        for _n in brightness actual_brightness max_brightness; do
            [ -f "$_bl/$_n" ] && printf '  %s=%s\n' "$_n" "$(cat "$_bl/$_n" 2>/dev/null)"
        done
    done
} > "$DEST/backlight.txt" 2>/dev/null
echo "flog: + backlight.txt"

# --- hardware inventory (which silicon/partition layout this unit has) ---
# by-name: GPT partition -> sdX mapping (UFS LUN order differs per unit:
# kodiak userdata=sda42, grizzly=sdb42). Missing userdata_exp.* here =
# never-expanded userdata (decrypt/mount suspect #1).
# sys dumps: UDC (gadget bind target), usb_role (role voter), i2c drivers
# (TCPC switch), gpio chips (torch HWEN), power_supply (VBUS/charger),
# devicetree flash@ (torch LWIS), drm (panel), block queue (UFS).
{
    echo "## /dev/block/by-name"
    ls -la /dev/block/by-name/ 2>/dev/null
    echo ""
    echo "## sdX partition tables"
    for _d in /sys/block/sd*/; do
        _b=$(basename "$_d")
        printf '%s: %s\n' "$_b" "$(cat "$_d/device/model" 2>/dev/null) $(cat "$_d/size" 2>/dev/null) sectors"
    done
    echo ""
    echo "## /sys/class/udc"
    ls /sys/class/udc/ 2>/dev/null
    echo ""
    echo "## /sys/class/usb_role (role voter)"
    for _r in /sys/class/usb_role/*/role; do
        [ -f "$_r" ] && printf '%s=%s\n' "$_r" "$(cat "$_r" 2>/dev/null)"
    done
    echo ""
    echo "## /sys/bus/i2c/drivers (TCPC candidates)"
    ls /sys/bus/i2c/drivers/ 2>/dev/null
    echo ""
    echo "## /sys/bus/i2c/devices (clients: *-0063 = LM3644, i2c-N of_node)"
    ls /sys/bus/i2c/devices/ 2>/dev/null
    echo ""
    echo "## /sys/bus/spmi (laguna TCPC lives here, not on i2c)"
    echo "-- drivers:"
    ls /sys/bus/spmi/drivers/ 2>/dev/null
    echo "-- devices:"
    ls /sys/bus/spmi/devices/ 2>/dev/null
    echo "-- tcpc driver bindings (bound client = 0-XX symlink, its absence = unbound):"
    for _d in /sys/bus/spmi/drivers/*tcpc* /sys/bus/spmi/drivers/*typec* /sys/bus/spmi/drivers/*tcpci* /sys/bus/spmi/drivers/*77759*; do
        [ -d "$_d" ] || continue
        echo "== $_d"
        ls -la "$_d" 2>/dev/null | grep -E "^l" || echo "  (no symlinks)"
    done
    echo ""
    echo "## /sys/bus/gpio/devices"
    ls /sys/bus/gpio/devices/ 2>/dev/null
    echo ""
    echo "## devicetree flash@ (torch LWIS node)"
    for _f in $(find /sys/firmware/devicetree/base -name "flash@*" -type d 2>/dev/null); do
        echo "== $_f"
        printf '  compatible=%s\n' "$(tr '\0' ' ' < "$_f/compatible" 2>/dev/null)"
        for _p in i2c-addr i2c-bus enable-gpios; do
            [ -f "$_f/$_p" ] && printf '  %s=%s\n' "$_p" "$(od -An -tx1 "$_f/$_p" 2>/dev/null | tr -d ' \n')"
        done
    done
    echo ""
    echo "## power_supply (VBUS/charger/otg)"
    for _p in /sys/class/power_supply/*/; do
        _n=$(basename "$_p")
        printf '%s: online=%s present=%s type=%s\n' "$_n" \
            "$(cat "$_p/online" 2>/dev/null)" "$(cat "$_p/present" 2>/dev/null)" "$(cat "$_p/type" 2>/dev/null)"
    done
    echo ""
    echo "## /sys/class/leds (vibrator/haptics)"
    ls /sys/class/leds/ 2>/dev/null
    echo ""
    echo "## drm cards"
    ls /sys/class/drm/ 2>/dev/null | head -20
    echo ""
    echo "## mounts"
    mount 2>/dev/null | grep -E "^/dev" || cat /proc/mounts 2>/dev/null | grep -E "^/dev"
} > "$DEST/hardware.txt" 2>/dev/null
echo "flog: + hardware.txt"

# --- pstore (survives reboot; empty dir listing is also an answer) ---
if [ -d /sys/fs/pstore ] && ls -la /sys/fs/pstore/ > "$DEST/pstore_ls.txt" 2>/dev/null; then
    for _p in /sys/fs/pstore/console-ramoops-0 /sys/fs/pstore/dmesg-ramoops-0; do
        [ -f "$_p" ] && cp -f "$_p" "$DEST/$(basename "$_p").txt" 2>/dev/null
    done
    echo "flog: + pstore snapshot"
else
    rm -f "$DEST/pstore_ls.txt"
    echo "flog: - no pstore, skipped"
fi

# --- installer config (which payload/slot policy produced this boot) ---
for _f in /system/etc/aio/families.txt /pixelrunatboot.json; do
    [ -f "$_f" ] && cp -f "$_f" "$DEST/$(basename "$_f")" 2>/dev/null
done

# --- permissions: cp inherits the source mode (recovery.log is 0600),
# which MTP/PC readers cannot open. Force world-readable copies. ---
chmod 755 "$DEST" 2>/dev/null
chmod 644 "$DEST"/* 2>/dev/null
echo "flog: perms forced to 644"

# --- pack everything into ONE timestamped archive, drop the staging dir ---
if command -v tar >/dev/null 2>&1 \
    && tar -czf "$BASE/$ARCH" -C "$BASE" "$STAMP" 2>/dev/null \
    && [ -f "$BASE/$ARCH" ]; then
    chmod 644 "$BASE/$ARCH" 2>/dev/null
    rm -rf "$DEST" 2>/dev/null
    echo ""
    echo "flog: DONE -> $BASE/$ARCH ($(du -h "$BASE/$ARCH" 2>/dev/null | cut -f1))"
else
    echo "flog: ! tar failed, leaving loose files in $DEST"
    echo ""
    echo "flog: DONE -> $DEST"
    ls -la "$DEST" | awk '$9 != "." && $9 != ".." {print "  "$9" "$5}'
    case "$BASE" in
        /tmp/*)
            echo "flog: userdata not writable — pull via: adb pull $DEST"
            ;;
        *)
            echo "flog: send every file above with the bug report"
            ;;
    esac
    exit 0
fi
case "$BASE" in
    /tmp/*)
        echo "flog: userdata not writable — pull via: adb pull $BASE/$ARCH"
        ;;
    *)
        echo "flog: send this single file with the bug report"
        ;;
esac
exit 0
