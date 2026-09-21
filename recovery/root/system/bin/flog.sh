#!/sbin/sh
# flog.sh — one-shot log collector for OrangeFox test reports.
#
# Usage (adb shell or Fox terminal):
#   flog.sh
#
# Collects recovery.log, weaver.log, dmesg, logcat, props, slot info,
# reflash/install logs and backlight state into ONE archive:
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
    for _p in ro.hardware ro.product.device ro.product.model ro.boot.slot_suffix ro.build.version.incremental DOF_SCREEN_W DOF_SCREEN_H DOF_PROGRESSIVE_SCALE DOF_STATUS_H ro.recovery.keymint ro.recovery.theme sys.usb.controller sys.usb.config tw_screen_timeout_secs; do
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
