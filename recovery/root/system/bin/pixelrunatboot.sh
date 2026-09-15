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
LPTOOLS="/system/bin/lptools_new"
SUPER="/dev/block/by-name/super"

plog() {
    echo "pixelrunatboot[$1]: $2" >> "$LOGF"
}

# Tools may arrive from the cluster without the exec bit — best effort.
chmod 755 "$SIW" "$IW" "$LPTOOLS" 2>/dev/null

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
# are skipped. An empty result triggers the caller's classic fallback.
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

# --- classic fallback: lptools map + mount + copy -------------------------
# _classic_copy <partbase> <slotsuffix(_a)> <slotnum> <mangle> <outdir> <findname>
# Copies matching files to outdir, prints staged paths. Returns 0 if >=1.
_classic_copy() {
    local _part="$1" _sfxname="$2" _slot="$3" _subdir="$4" _outdir="$5" _fname="$6"
    local _node _mnt _n _f _base
    _node="/dev/block/mapper/${_part}${_sfxname}"
    if [ ! -b "$_node" ]; then
        plog "classic" "$_node absent, mapping"
        "$LPTOOLS" --slot "$_slot" --suffix "$_sfxname" --map "${_part}${_sfxname}" >>"$LOGF" 2>&1 \
            || { plog "classic" "map failed: ${_part}${_sfxname}"; return 1; }
    fi
    _mnt="/dev/stage_mnt_$$"
    mkdir -p "$_mnt"
    if ! mount -r "$_node" "$_mnt" 2>>"$LOGF"; then
        plog "classic" "mount failed: $_node"
        rmdir "$_mnt" 2>/dev/null
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

    ko-fetch)
        # ko-fetch <partbase> <suffix-a|b> <slotnum>  (e.g. vendor_dlkm a 0)
        _part="$2"; _sfx="$3"; _slot="$4"
        _out="/dev/ko_stage/${_part}_${_sfx}"
        rm -rf "$_out"; mkdir -p "$_out"
        _img="/dev/stage_${_part}_${_sfx}.img"
        if _siw_stream "$_part" "$_sfx" "$_slot" "$_img"; then
            if _iw_extract "$_img" '.ko' "$_out"; then
                rm -f "$_img"
                exit 0
            fi
            plog "ko-fetch" "iw parse empty, classic fallback"
            rm -f "$_img"
        fi
        # Fallback: lptools map + mount + find + cp.
        if _classic_copy "$_part" "_${_sfx}" "$_slot" "" "$_out" '*.ko'; then
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
        _part="$2"; _sfx="$3"; _slot="$4"
        mkdir -p /vendor/firmware 2>/dev/null
        _n=0
        _node="/dev/block/mapper/${_part}_${_sfx}"
        if [ ! -b "$_node" ]; then
            "$LPTOOLS" --slot "$_slot" --suffix "_${_sfx}" --map "${_part}_${_sfx}" >>"$LOGF" 2>&1
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
        if [ "$_n" -gt 0 ]; then echo "$_n"; exit 0; fi
        plog "fw-fetch" "mount path empty, siw|iw stream fallback"
        _img="/dev/stage_${_part}_${_sfx}.img"
        if _siw_stream "$_part" "$_sfx" "$_slot" "$_img"; then
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
