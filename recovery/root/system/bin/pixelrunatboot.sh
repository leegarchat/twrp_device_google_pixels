#!/system/bin/sh
#
#   pixelrunatboot.sh — external-binary stages for recovery-pixel-boot.
#
#   The Rust engine does syscalls only (no forks). Everything that needs an
#   outside binary (resetprop, bootctl, siw/iw, unzip, mount, setenforce)
#   lives here as argv-addressed stages. Protocol: stage results go to
#   stdout, diagnostics to /tmp/recovery.log, success = exit 0.
#
#   Stages: props-apply | slot-detect | ko-fetch | fw-fetch |
#           magiskboot-unpack | meta-fix
#

LOGF="/tmp/recovery.log"
SIW="/system/bin/siw"
IW="/system/bin/iw"
SUPER="/dev/block/by-name/super"

plog() {
    echo "pixelrunatboot[$1]: $2" >> "$LOGF"
}

# Tools may arrive from the cluster without the exec bit — best effort.
chmod 755 "$SIW" "$IW" 2>/dev/null

# --- siw read <partbase> <suffix> <slotnum> > <outfile> -------------------
# Streams one LP partition to a file. Returns 0 on success.
# NOTE: every helper below MUST declare its variables `local`.
# Without it, nested calls clobber the caller's vars (sh has one global
# namespace) — this once caused _siw_stream to overwrite ko-fetch's outdir
# with the image path, staging the image into itself with rc=0.

_siw_stream() {
    local _part="$1" _sfx="$2" _slot="$3" _out="$4"
    if [ ! -x "$SIW" ]; then
        plog "siw" "binary missing: $SIW"
        return 1
    fi
    if "$SIW" read "$SUPER" -p "$_part" --suffix "$_sfx" --slot "$_slot" > "$_out" 2>>"$LOGF" \
        && [ -s "$_out" ]; then
        return 0
    fi
    plog "siw" "stream failed: ${_part} suffix=${_sfx} slot=${_slot}"
    rm -f "$_out"
    return 1
}

# --- iw extract <image> <grep-filter> <outdir> ------------------------------
# Prints staged full paths to stdout, one per line. Returns 0 if >=1 staged.
# `iw read -f <pattern>` does NOT glob (errors out), so we take the bare
# recursive listing and filter in shell. Non-files fail the -c fetch and
# are skipped. An empty result triggers the caller's map+mount fallback.
_iw_extract() {
    local _img="$1" _filt="$2" _outdir="$3"
    local _hit _base _n
    mkdir -p "$_outdir" 2>/dev/null
    "$IW" read "$_img" -f 2>>"$LOGF" | grep -F "$_filt" 2>/dev/null | grep '/' 2>/dev/null | while IFS= read -r _hit; do
        _hit=$(printf '%s' "$_hit" | tr -d '[:space:]')
        [ -n "$_hit" ] || continue
        # Depth guard (mirrors fw-fetch top-level rule): vendor images carry
        # page-size subdirs (16k-mode/) whose same-basename modules shadow
        # the flat 4K ones with incompatible symbol CRCs (goodix 16K over
        # 4K broke touch on 6.12). Take lib/modules/*.ko only.
        _rel="${_hit#/}"
        case "$_rel" in
            */*/*/*) continue ;;
        esac
        _base=$(basename "$_hit")
        if "$IW" read "$_img" -c "$_hit" > "$_outdir/$_base" 2>>"$LOGF"; then
            :
        elif "$IW" read "$_img" -c "/$_hit" > "$_outdir/$_base" 2>>"$LOGF"; then
            :
        else
            rm -f "$_outdir/$_base"
            continue
        fi
        [ -s "$_outdir/$_base" ] || { rm -f "$_outdir/$_base"; continue; }
        echo "$_outdir/$_base"
    done
    # Count staged files (subshell-safe: recount from disk).
    _n=$(find "$_outdir" -type f 2>/dev/null | wc -l)
    [ "$_n" -gt 0 ]
}

# --- siw map <partbase> <suffix-letter> <slotnum> ---------------------------
# Device-mapper mapping, Android/Recovery only (DM ioctl): creates
# /dev/block/mapper/<partbase>[_suffix]. Same selector shape as _siw_stream
# (-p base --suffix letter --slot num). Returns 0 on success.
_siw_map() {
    local _part="$1" _sfx="$2" _slot="$3"
    if [ ! -x "$SIW" ]; then
        plog "siw-map" "binary missing: $SIW"
        return 1
    fi
    if "$SIW" map "$SUPER" -p "$_part" --suffix "$_sfx" -s "$_slot" >>"$LOGF" 2>&1; then
        return 0
    fi
    plog "siw-map" "map failed: ${_part} suffix=${_sfx} slot=${_slot}"
    return 1
}

# --- siw unmap <mapped-name> <partbase> ------------------------------------
# One-shot teardown for _siw_map mappings: `siw disconnect` only drops
# loop nodes from `connect`; dm nodes from `map` need `siw unmap`.
# Also drops the slot-less alias when it dangles (live-proven:
# /dev/block/mapper/vendor_dlkm -> vendor_dlkm_a kept tripping
# Unmap_Super_Devices into E:Unable to unmap at format).
# Best-effort: warnings only, never fails the caller.
# WARNING: unmaps by NAME — only call via _siw_unmap_ours below, which
# verifies the node still resolves to the dm target this run created.
# Unmapping a live TWRP/first-stage node (field case: fw-fetch mapped
# vendor_a while ueventd hadn't linked it yet, then removed TWRP's own
# mapping) leaves by-name/vendor dangling at a dead dm-0 and breaks
# Unmap_Super_Devices/format. Direct callers: none (kept as primitive).
_siw_unmap() {
    local _name="$1" _part="$2"
    local _a
    if [ -x "$SIW" ] && [ -n "$_name" ]; then
        "$SIW" unmap "$_name" >>"$LOGF" 2>&1 \
            || plog "siw-unmap" "unmap warning: $_name"
    fi
    if [ -n "$_part" ]; then
        _a="/dev/block/mapper/$_part"
        if [ -L "$_a" ] && [ ! -e "$_a" ]; then
            if rm -f "$_a" 2>/dev/null; then
                plog "siw-unmap" "removed dangling alias $_a"
            else
                plog "siw-unmap" "cannot remove dangling alias $_a"
            fi
        fi
    fi
}

# --- dm target of a mapper node -----------------------------------------
# Basename of the /dev/block/mapper/<name> symlink target (e.g. "dm-2");
# empty when the node is absent or dangling. Snapshot right after _siw_map
# and compare before unmapping: the name alone proves nothing (TWRP or
# first-stage may have recreated the node under the same name since).
_dm_target() {
    local _t
    _t=$(readlink "/dev/block/mapper/$1" 2>/dev/null) || { echo ""; return 1; }
    printf '%s' "${_t##*/}"
}

# --- keep-list: dm nodes left mapped for TWRP ----------------------------
# LP bases whose mapper node stays behind when WE created it. TWRP only
# resolves logical partitions (fs_mgr_update_logical_partition) and
# re-resolves live at every Mount() (Find_Actual_Block_Device slotselect
# branch) — but first-stage/TWRP never maps these itself (field cases:
# vendor_dlkm on gs101, "unable to update logical partition" at every
# boot; vendor_a destroyed again by the first-stage handoff ~5s in, so a
# fresh fw-fetch mapping is the only live one), so without a kept node
# /vendor_dlkm and /vendor can never mount. Leaving linear nodes is safe:
# same extents as LP metadata, format-time Unmap destroys them cleanly
# via the existing metadata entries. Empty = legacy one-shot
# (map+copy+unmap, no traces).
_KEEP_MAPPED="vendor_dlkm vendor"

_keep_listed() {
    case " $_KEEP_MAPPED " in
        *" $1 "*) return 0 ;;
    esac
    return 1
}

# Ensure /dev/block/mapper/<part>_<sfx> exists for TWRP (map when absent)
# and leave it mapped. No-op when already present (foreign or ours) and
# for non-listed partitions.
_keep_node_mapped() {
    local _part="$1" _sfx="$2" _slot="$3" _node
    _keep_listed "$_part" || return 0
    _node="/dev/block/mapper/${_part}_${_sfx}"
    if [ -b "$_node" ]; then
        plog "keep-mapped" "$_node already present, leaving it"
        return 0
    fi
    if _siw_map "$_part" "$_sfx" "$_slot" && [ -b "$_node" ]; then
        plog "keep-mapped" "left ${_part}_${_sfx} mapped for TWRP"
        return 0
    fi
    plog "keep-mapped" "cannot map ${_part}_${_sfx} (TWRP mount stays unavailable)"
    return 1
}

# --- guarded unmap: only what this run created ------------------------------
# _siw_unmap_ours <mapped-name> <partbase> <expected-dm-target>
# Removes the mapping only when the mapper node still resolves to the dm
# target snapshotted right after _siw_map. Anything else (node recreated
# by TWRP/first-stage under the same name, node gone) means the device is
# NOT ours — skip the unmap, drop a dangling alias at most. Best-effort,
# never fails the caller.
_siw_unmap_ours() {
    local _name="$1" _part="$2" _want="$3" _cur _a
    _cur=$(_dm_target "$_name")
    if [ -n "$_want" ] && [ "$_cur" = "$_want" ]; then
        _siw_unmap "$_name" "$_part"
        return 0
    fi
    plog "siw-unmap" "skip $_name (ours=$_want now=$_cur): foreign or gone"
    if [ -n "$_part" ]; then
        _a="/dev/block/mapper/$_part"
        if [ -L "$_a" ] && [ ! -e "$_a" ]; then
            rm -f "$_a" 2>/dev/null \
                && plog "siw-unmap" "removed dangling alias $_a" \
                || plog "siw-unmap" "cannot remove dangling alias $_a"
        fi
    fi
    return 0
}

# --- dm state snapshot (service surface) --------------------------------
# /tmp/fox_dm_state: mapper truth as owned by the staging layer
# ("<name> <dm-target>" per line, "# v1 epoch=..." header). Whoever needs
# live dm state (TWRP flows, adb debugging) reads this file instead of
# guessing who mapped what. Volatile tmpfs: same lifetime as the dm
# namespace itself (gone on reboot). Refreshed at every ko-fetch /
# fw-fetch exit via trap (this script runs one stage per process, so EXIT
# always fires exactly once per stage run).
_dm_snapshot() {
    local _t _n _s
    _s="/tmp/fox_dm_state.tmp.$$"
    {
        echo "# v1 epoch=$(date +%s 2>/dev/null || echo 0)"
        for _n in /dev/block/mapper/*; do
            [ -e "$_n" ] || [ -L "$_n" ] || continue
            _t=$(readlink "$_n" 2>/dev/null)
            printf '%s %s\n' "${_n##*/}" "${_t##*/}"
        done
    } > "$_s" 2>/dev/null
    mv -f "$_s" /tmp/fox_dm_state 2>/dev/null
}

# --- map + mount + copy fallback ------------------------------------------
# _siw_map_copy <partbase> <slotsuffix(_a)> <slotnum> <mangle> <outdir> <findname>
# Copies matching files to outdir, prints staged paths. Returns 0 if >=1.
# One-shot: a mapping created here is unmapped before return on every
# path (copy ok, copy empty, mount failed) — no traces left behind.
_siw_map_copy() {
    local _part="$1" _sfxname="$2" _slot="$3" _subdir="$4" _outdir="$5" _fname="$6"
    local _node _mnt _n _f _base _mapped _want
    _node="/dev/block/mapper/${_part}${_sfxname}"
    _mapped=0
    _want=""
    if [ ! -b "$_node" ]; then
        plog "map-copy" "$_node absent, mapping via siw"
        _siw_map "$_part" "${_sfxname#_}" "$_slot" \
            || { plog "map-copy" "map failed: ${_part}${_sfxname}"; return 1; }
        _mapped=1
        # Ownership snapshot: unmap later only if the node still
        # resolves here (see _siw_unmap_ours).
        _want=$(_dm_target "${_part}${_sfxname}")
    fi
    _mnt="/dev/stage_mnt_$$"
    mkdir -p "$_mnt"
    if ! mount -r "$_node" "$_mnt" 2>>"$LOGF"; then
        plog "map-copy" "mount failed: $_node"
        rmdir "$_mnt" 2>/dev/null
        [ "$_mapped" = 1 ] && _siw_unmap_ours "${_part}${_sfxname}" "$_part" "$_want"
        return 1
    fi
    mkdir -p "$_outdir" 2>/dev/null
    _n=0
    # maxdepth 3 = <mnt>/lib/modules/*.ko only: page-size subdirs
    # (16k-mode/) must not shadow the flat modules (see _iw_extract).
    for _f in $(find "$_mnt/$_subdir" -maxdepth 3 -type f -name "$_fname" 2>/dev/null); do
        _base=$(basename "$_f")
        if cp -f "$_f" "$_outdir/$_base" 2>>"$LOGF"; then
            echo "$_outdir/$_base"
            _n=$((_n + 1))
        fi
    done
    umount "$_mnt" 2>/dev/null
    rmdir "$_mnt" 2>/dev/null
    if [ "$_mapped" = 1 ]; then
        if _keep_listed "$_part"; then
            plog "map-copy" "leaving ${_part}${_sfxname} mapped for TWRP"
        else
            _siw_unmap_ours "${_part}${_sfxname}" "$_part" "$_want"
        fi
    fi
    [ "$_n" -gt 0 ]
}

# --- siw connect (loop device) + mount + copy fallback ----------------------
# _siw_connect_copy <partbase> <suffix-a|b> <slotnum> <subdir> <outdir> <findname>
# Loop-device mapping for when device-mapper mapping yields no usable node
# (seen live: `siw map` exits 0 but creates nothing for vendor; loop
# devices bypass device-mapper entirely). Copies matching files to outdir,
# prints staged paths. Returns 0 if >=1. Always disconnects the loop node.
_siw_connect_copy() {
    local _part="$1" _sfx="$2" _slot="$3" _subdir="$4" _outdir="$5" _fname="$6"
    local _loop _mnt _n _f _base
    if [ ! -x "$SIW" ]; then
        plog "siw-connect" "binary missing: $SIW"
        return 1
    fi
    _loop=$("$SIW" connect "$SUPER" -p "$_part" --suffix "$_sfx" -s "$_slot" 2>>"$LOGF" | grep -o '/dev/loop[0-9][0-9]*' | head -1)
    if [ -z "$_loop" ] || [ ! -b "$_loop" ]; then
        plog "siw-connect" "no loop node for ${_part} suffix=${_sfx} slot=${_slot}"
        return 1
    fi
    plog "siw-connect" "loop node $_loop for ${_part}_${_sfx}"
    _mnt="/dev/stage_mnt_$$"
    mkdir -p "$_mnt"
    _n=0
    if mount -r "$_loop" "$_mnt" 2>>"$LOGF"; then
        mkdir -p "$_outdir" 2>/dev/null
        for _f in $(find "$_mnt/$_subdir" -maxdepth 3 -type f -name "$_fname" 2>/dev/null); do
            _base=$(basename "$_f")
            if cp -f "$_f" "$_outdir/$_base" 2>>"$LOGF"; then
                echo "$_outdir/$_base"
                _n=$((_n + 1))
            fi
        done
        umount "$_mnt" 2>/dev/null
    else
        plog "siw-connect" "mount failed: $_loop"
    fi
    rmdir "$_mnt" 2>/dev/null
    "$SIW" disconnect "$SUPER" -p "$_part" --suffix "$_sfx" -s "$_slot" >>"$LOGF" 2>&1 \
        || plog "siw-connect" "disconnect warning for ${_part}_${_sfx}"
    [ "$_n" -gt 0 ]
}

case "$1" in
    props-apply)
        # props-apply <family> <key=value>...
        # Pairs come from /pixelrunatboot.json via the Rust engine (already
        # family-merged at build time); argv carries values with spaces/= intact.
        _fam="$2"; shift 2
        command -v resetprop >/dev/null 2>&1 \
            || { plog "props-apply" "resetprop missing"; exit 1; }
        setenforce 0 2>>"$LOGF"
        _n=0
        for _pair in "$@"; do
            _k="${_pair%%=*}"; _v="${_pair#*=}"
            [ -n "$_k" ] || continue
            if resetprop "$_k" "$_v" 2>>"$LOGF"; then
                _n=$((_n + 1))
            fi
        done
        echo "$_n"
        ;;

    slot-detect)
        # bootctl fallback; prints _a / _b to stdout.
        _sfx=$(bootctl get-current-slot 2>/dev/null | xargs bootctl get-suffix 2>/dev/null)
        _sfx=$(printf '%s' "$_sfx" | tr -d '[:space:]')
        case "$_sfx" in
            _a|_b) echo "$_sfx"; exit 0 ;;
        esac
        _cur=$(bootctl get-current-slot 2>/dev/null | tr -d '[:space:]')
        case "$_cur" in
            0) echo "_a" ;;
            1) echo "_b" ;;
            *) plog "slot-detect" "unknown slot: $_cur"; exit 1 ;;
        esac
        ;;

    dm-map)
        # dm-map <partbase> <suffix-a|b> <slotnum>  (e.g. vendor_dlkm a 0)
        # Map-and-LEAVE for the rust dm keep-list (dm.rs ensure_keep_mapped):
        # ensures /dev/block/mapper/<part>_<sfx> exists and prints it.
        # Never unmaps (the dm-watch daemon is snapshot-only, so a
        # format-time destroy stays destroyed). No-op when already present.
        trap '_dm_snapshot' EXIT
        _part="$2"; _sfx="$3"; _slot="$4"
        _node="/dev/block/mapper/${_part}_${_sfx}"
        if [ -b "$_node" ]; then
            echo "$_node"
            exit 0
        fi
        if _siw_map "$_part" "$_sfx" "$_slot" && [ -b "$_node" ]; then
            plog "dm-map" "mapped $_node for TWRP"
            echo "$_node"
            exit 0
        fi
        plog "dm-map" "map failed: ${_part}_${_sfx}"
        exit 1
        ;;

    ko-fetch)
        # ko-fetch <partbase> <suffix-a|b> <slotnum>  (e.g. vendor_dlkm a 0)
        trap '_dm_snapshot' EXIT
        _part="$2"; _sfx="$3"; _slot="$4"        _out="/dev/ko_stage/${_part}_${_sfx}"
        rm -rf "$_out"; mkdir -p "$_out"
        _img="/dev/stage_${_part}_${_sfx}.img"
        # Skip the stream when iw is missing/unusable: its parse step
        # cannot succeed then — straight to map+mount.
        if [ -x "$IW" ] && _siw_stream "$_part" "$_sfx" "$_slot" "$_img"; then
            if _iw_extract "$_img" '.ko' "$_out"; then
                rm -f "$_img"
                # Stream path creates no dm node; keep-listed partitions
                # still need one for TWRP (see _KEEP_MAPPED).
                _keep_node_mapped "$_part" "$_sfx" "$_slot"
                exit 0
            fi
            plog "ko-fetch" "iw parse empty, map+mount fallback"
            rm -f "$_img"
        fi
        # Fallback: siw map + mount + find + cp.
        if _siw_map_copy "$_part" "_${_sfx}" "$_slot" "" "$_out" '*.ko'; then
            exit 0
        fi
        # Fallback 2: siw connect (loop device) + mount + find + cp.
        # Covers dm-mapping yielding no usable node.
        if _siw_connect_copy "$_part" "$_sfx" "$_slot" "" "$_out" '*.ko'; then
            exit 0
        fi
        plog "ko-fetch" "all methods failed: ${_part}_${_sfx}"
        exit 1
        ;;

    fw-fetch)
        # fw-fetch <partbase> <suffix-a|b> <slotnum>: firmware/* -> /vendor/firmware/.
        # Prints copied count. rc=0 if anything was copied.
        # Order is deliberate: read-only mount first (instant, no bulk copy;
        # vendor is ~1GB — streaming it to tmpfs cost 86s once), siw|iw
        # stream second, by-name mount last.
        # (Plain assignments: case branches run at top level where `local`
        # is not portable; helpers localize their own vars, so no clobber.)
        trap '_dm_snapshot' EXIT
        _part="$2"; _sfx="$3"; _slot="$4"
        mkdir -p /vendor/firmware 2>/dev/null
        _n=0
        _node="/dev/block/mapper/${_part}_${_sfx}"
        _fw_mapped=0
        _fw_want=""
        if [ ! -b "$_node" ]; then
            if _siw_map "$_part" "$_sfx" "$_slot"; then
                # map may exit 0 without creating the node (seen live
                # on vendor); only teardown what actually appeared, and
                # snapshot the dm target for the ownership check.
                if [ -b "$_node" ]; then
                    _fw_mapped=1
                    _fw_want=$(_dm_target "${_part}_${_sfx}")
                fi
            else
                plog "fw-fetch" "siw map failed: ${_part}_${_sfx}"
            fi
        fi
        _mnt="/dev/stage_mnt_$$"
        mkdir -p "$_mnt"
        if mount -r "$_node" "$_mnt" 2>>"$LOGF"; then
            for _f in "$_mnt"/firmware/*; do
                [ -f "$_f" ] || continue
                cp -f "$_f" /vendor/firmware/ 2>>"$LOGF" && _n=$((_n + 1))
            done
            umount "$_mnt" 2>/dev/null
        fi
        rmdir "$_mnt" 2>/dev/null
        # One-shot: unmap what this run mapped (ownership-checked),
        # drop a dangling alias — unless keep-listed for TWRP.
        if [ "$_fw_mapped" = 1 ]; then
            if _keep_listed "$_part"; then
                plog "fw-fetch" "leaving ${_part}_${_sfx} mapped for TWRP"
            else
                _siw_unmap_ours "${_part}_${_sfx}" "$_part" "$_fw_want"
            fi
        fi
        if [ "$_n" -gt 0 ]; then echo "$_n"; exit 0; fi
        plog "fw-fetch" "mount path empty, siw|iw stream fallback"
        _img="/dev/stage_${_part}_${_sfx}.img"
        # Skip the stream when iw is missing/unusable: its parse step
        # cannot succeed then — straight to by-name fallback.
        if [ -x "$IW" ] && _siw_stream "$_part" "$_sfx" "$_slot" "$_img"; then
            for _staged in $(_iw_extract "$_img" '/firmware/' /dev/fw_stage 2>/dev/null); do
                # Top-level firmware/* only (mirror the mount+glob semantics):
                # deeper hits are payload stores from elsewhere in the image.
                _rel="${_staged#/dev/fw_stage/}"
                case "$_rel" in
                    */*) continue ;;
                esac
                if cp -f "$_staged" /vendor/firmware/ 2>>"$LOGF"; then
                    _n=$((_n + 1))
                fi
            done
            rm -rf /dev/fw_stage "$_img"
            if [ "$_n" -gt 0 ]; then echo "$_n"; exit 0; fi
            plog "fw-fetch" "iw parse empty, by-name fallback"
        fi
        _n=0
        # Last resort: read the by-name node directly (no LP involved).
        if [ "$_n" -eq 0 ] && [ -b "/dev/block/by-name/${_part}" ]; then
            mkdir -p "$_mnt" 2>/dev/null
            if mount -r "/dev/block/by-name/${_part}" "$_mnt" 2>>"$LOGF"; then
                for _f in "$_mnt"/firmware/*; do
                    [ -f "$_f" ] || continue
                    cp -f "$_f" /vendor/firmware/ 2>>"$LOGF" && _n=$((_n + 1))
                done
                umount "$_mnt" 2>/dev/null
            fi
            rmdir "$_mnt" 2>/dev/null
        fi
        echo "$_n"
        [ "$_n" -gt 0 ]
        ;;

    magiskboot-unpack)
        # magiskboot-unpack <Magisk.zip>: boot/busybox binaries to /system/bin.
        _zip="$2"
        [ -f "$_zip" ] || { plog "magiskboot-unpack" "zip missing"; exit 1; }
        _work=/tmp/magisk_unzip
        rm -rf "$_work"; mkdir -p "$_work"
        unzip -q "$_zip" -d "$_work" 2>>"$LOGF" \
            || { plog "magiskboot-unpack" "unzip failed"; rm -rf "$_work"; exit 1; }
        cp -f "$_work/lib/arm64-v8a/libmagiskboot.so" /system/bin/magiskboot_29 2>>"$LOGF"
        cp -f "$_work/lib/arm64-v8a/libmagiskboot.so" /system/bin/magiskboot 2>>"$LOGF"
        cp -f "$_work/lib/arm64-v8a/libbusybox.so" /system/bin/busybox 2>>"$LOGF"
        chmod 777 /system/bin/magiskboot_29 /system/bin/magiskboot /system/bin/busybox 2>>"$LOGF"
        rm -f /system/bin/ln
        /system/bin/busybox ln -s /system/bin/busybox /system/bin/ln 2>>"$LOGF"
        rm -rf "$_work"
        plog "magiskboot-unpack" "done: $_zip"
        ;;

    meta-fix)
        # kerror7: drop stale OTA state from /metadata.
        # The block node may appear late (ueventd coldboot vs on-boot exec),
        # so wait + retry instead of failing once.
        _tries=0
        while [ "$_tries" -lt 5 ] && [ ! -e /dev/block/by-name/metadata ]; do
            sleep 1
            _tries=$((_tries + 1))
        done
        if ! mountpoint -q /metadata 2>/dev/null; then
            # Explicit device+fstype: recovery has no /etc/fstab, so a bare
            # `mount /metadata` always fails with "bad /etc/fstab".
            # /metadata is f2fs on all families (see families/*/recovery.fstab).
            mkdir -p /metadata 2>/dev/null
            if ! mount -t f2fs /dev/block/by-name/metadata /metadata 2>>"$LOGF" \
                && ! mount -t ext4 /dev/block/by-name/metadata /metadata 2>>"$LOGF"; then
                plog "meta-fix" "mount failed after wait"
                exit 1
            fi
        fi
        [ -d /metadata/ota ] && rm -rf /metadata/ota
        umount /metadata 2>/dev/null
        plog "meta-fix" "done"
        ;;

    *)
        echo "usage: $0 {props-apply|slot-detect|ko-fetch|fw-fetch|magiskboot-unpack|meta-fix}" >&2
        exit 2
        ;;
esac
