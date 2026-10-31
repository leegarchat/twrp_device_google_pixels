#!/usr/bin/env bash
# Packs the UNIVERSAL installer zip: one archive for KernelSU /
# Magisk (new-style customize.sh flow), recovery (META-INF) and
# desktop (install-desktop.sh/.bat/.exe/.AppImage + install-macos.command).
#
# Tool layout per variant (OFOX_VARIANT):
#   full_compressed  bin.lgz (every tool as ONE lgz cluster, bin/ packed
#                    with --preserve-perms) + unpack_bin/ per-OS lean
#                    decoders that unpack bin.lgz on first run (launchers
#                    do it up front; install-recovery.sh covers recovery,
#                    customize.sh the Magisk branch, .bat/.exe Windows).
#                    No raw bin/ in the zip. Smallest download.
#   full             classic raw layout, single zip pass, no lgz at all:
#                    raw bin/ tree (platform-tools included), no bin.lgz,
#                    no unpack_bin/. Launchers skip the unpack block
#                    (no-op without bin.lgz) and use bin/ directly.
#                    For Windows hosts where the lean lgz decoder fails.
#   minimal          raw layout like full, but WITHOUT any platform-tools
#                    (no bin/*/platform-tools) and no lgz bits. For
#                    recovery/Magisk installs or hosts with platform-tools
#                    on PATH (FALLBACK_PATH=1 in export.txt already allows
#                    it) or a custom path in export.txt. Smallest raw zip.
#   (touch archives bin/modules/*.lgz are NEVER unpacked by the
#   installer in any variant: they ride inside bin.lgz (compressed) or
#   as raw files (full/minimal) and are fed into the payload cpio as-is
#   via --inject-lgz.)
#
# Output names (no explicit path given):
#   {OFOX_NAME}-{OFOX_TYPE}-{OFOX_TAG}-{OFOX_FAMILY}-{OFOX_VARIANT}.zip
# e.g. OrangeFox-R12.0-Beta-aio-full_compressed.zip
#
# The ONLY hard rule for managers: no root-level install.sh — it
# flips them into legacy install.sh mode (they source it with sh
# and fail). Desktop launchers are named install-desktop.* precisely
# to stay invisible to that check. The script verifies the tripwire
# exactly the way the managers test it.
#
# Usage: ./pack-module-zip.sh [output.zip] [--variant V]   (an explicit path always wins)
#
# Variant: OFOX_VARIANT=full_compressed|full|minimal (default
# full_compressed), or --variant V anywhere in the args. build.sh packs
# all three in a loop.
#
# Without an argument the release name is composed from the environment:
#   {OFOX_NAME}-{OFOX_TYPE}-{OFOX_TAG}-{OFOX_FAMILY}.zip
# defaults: OFOX_NAME=OrangeFox OFOX_TYPE=R12.0 OFOX_TAG=Beta
# OFOX_FAMILY=aio. The zip is placed into the tree's builds/ directory
# (resolved by walking up from here; override with OFOX_BUILDS_DIR).
#
# LGZ_LEVEL=0..3 (default 0) selects the bin.lgz cluster compression
# level — same flag as the top-level build.sh -l/--level. The full lgz
# binary is taken from ../include/prebuilt/lgz_compress_full_x64
# (LGZ_BIN overrides with an explicit path).
#
# OFOX_PAYLOAD=/path/to/<payload>.lz4 refreshes the bundled payload for
# this zip only: it is copied next to this script and export.txt
# RECOVERY_IMG is rewritten to its basename before packing. The tree is
# left untouched afterwards (export.txt restored from backup, our payload
# copy AND the staged bin.lgz removed, stale payloads never deleted) so
# routine builds don't dirty git status; fully ignoring export.txt is
# not an option (the installer reads it at runtime).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT"

# Restores the in-tree files touched for the payload refresh (backup copy
# of export.txt + our payload copy + the staged bin.lgz cluster).
# Runs on every exit via trap; a plain repack without OFOX_PAYLOAD is
# a no-op (no backup was taken), bin.lgz is always rebuilt-or-removed.
_ofx_restore() {
    if [ -n "${_export_bak:-}" ] && [ -f "${_export_bak:-}" ]; then
        cp -f "$_export_bak" export.txt
        rm -f "$_export_bak"
    fi
    if [ -z "${_pb_existed:-}" ] && [ -n "${_pb:-}" ]; then
        rm -f "$ROOT/$_pb"
    fi
    if [ -n "${_bin_lgz_built:-}" ]; then
        rm -f "$ROOT/bin.lgz"
    fi
}
trap _ofx_restore EXIT

OFOX_NAME="${OFOX_NAME:-OrangeFox}"
OFOX_TYPE="${OFOX_TYPE:-R12.0}"
OFOX_TAG="${OFOX_TAG:-Beta}"
OFOX_FAMILY="${OFOX_FAMILY:-aio}"

# Variant: explicit --variant V arg wins, else the environment.
_V_OUT=""
for _va in "$@"; do
    case "$_va" in
        --variant=*) OFOX_VARIANT="${_va#--variant=}" ;;
        *) if [ -z "$_V_OUT" ]; then _V_OUT="$_va"; else echo "unexpected argument: $_va" >&2; exit 1; fi ;;
    esac
done
OFOX_VARIANT="${OFOX_VARIANT:-full_compressed}"
case "$OFOX_VARIANT" in
    full_compressed|full|minimal) ;;
    *) echo "bad OFOX_VARIANT='$OFOX_VARIANT' (want full_compressed|full|minimal)" >&2; exit 1;;
esac

_builds_dir() {
    if [ -n "${OFOX_BUILDS_DIR:-}" ]; then printf '%s' "$OFOX_BUILDS_DIR"; return; fi
    _d="$ROOT"
    for _i in 1 2 3 4 5 6; do
        if [ -d "$_d/builds" ]; then printf '%s' "$_d/builds"; return; fi
        _d="$(dirname "$_d")"
    done
    printf '%s' "$ROOT"
}

if [ -n "$_V_OUT" ]; then
    OUT="$_V_OUT"
else
    OUT="$(_builds_dir)/${OFOX_NAME}-${OFOX_TYPE}-${OFOX_TAG}-${OFOX_FAMILY}-${OFOX_VARIANT}.zip"
fi
unset _V_OUT

# Optional payload refresh before the required-file checks below.
# Tree-neutral: export.txt is backed up (trap restores it on exit) and
# only our own payload copy is removed afterwards; anything the user had
# (including a stale payload or export.txt edits) is left exactly as is.
if [ -n "${OFOX_PAYLOAD:-}" ]; then
    [ -f "$OFOX_PAYLOAD" ] || { echo "OFOX_PAYLOAD not found: $OFOX_PAYLOAD" >&2; exit 1; }
    _pb="$(basename "$OFOX_PAYLOAD")"
    case "$_pb" in *.lz4) ;; *) echo "OFOX_PAYLOAD must be a .lz4 ramdisk payload" >&2; exit 1;; esac
    _old="$(grep -E '^RECOVERY_IMG=' export.txt 2>/dev/null | tail -1 | cut -d= -f2- | tr -d '\r' | sed 's/^ *//;s/ *$//')"
    _export_bak="$(mktemp)"
    cp -f export.txt "$_export_bak"
    _pb_existed=""
    [ -f "$ROOT/$_pb" ] && _pb_existed=1
    cp -f "$OFOX_PAYLOAD" "$ROOT/$_pb"
    sed -i "s|^RECOVERY_IMG=.*|RECOVERY_IMG=$_pb|" export.txt
    if [ -n "$_old" ] && [ "$_old" != "$_pb" ]; then
        echo "payload refreshed for this zip: $_old -> $_pb (tree files restored afterwards)"
    else
        echo "payload refreshed for this zip: $_pb (tree files restored afterwards)"
    fi
fi

cfg() { grep -E "^$1=" export.txt 2>/dev/null | tail -1 | cut -d= -f2- | tr -d '\r' | sed 's/^ *//;s/ *$//;s/^"\(.*\)"$/\1/;s/^'"'"'\(.*\)'"'"'$/\1/'; }
PAYLOAD="$(cfg RECOVERY_IMG OrangeFox-R12.0-test5-aio.ramdisk.lz4)"

# Cluster level for bin.lgz (full_compressed only): same flag as
# build.sh -l/--level. Raw variants (full/minimal) ignore it.
LGZ_LEVEL="${LGZ_LEVEL:-0}"
case "$LGZ_LEVEL" in 0|1|2|3) ;; *) echo "bad LGZ_LEVEL='$LGZ_LEVEL' (want 0-3)" >&2; exit 1;; esac

# Full lgz binary: needed only for the full_compressed cluster pack
# (in-tree prebuilt wins, PATH fallback next, explicit LGZ_BIN first).
if [ "$OFOX_VARIANT" = full_compressed ]; then
    if [ -n "${LGZ_BIN:-}" ]; then
        : "${LGZ_BIN:?}"
    elif [ -x "$ROOT/../include/prebuilt/lgz_compress_full_x64" ]; then
        LGZ_BIN="$ROOT/../include/prebuilt/lgz_compress_full_x64"
    elif command -v lgz_compress >/dev/null 2>&1; then
        LGZ_BIN="lgz_compress"
    else
        echo "need the full lgz binary (../include/prebuilt/lgz_compress_full_x64 or lgz_compress on PATH)" >&2
        exit 1
    fi
    [ -x "$LGZ_BIN" ] || { echo "lgz binary not executable: $LGZ_BIN" >&2; exit 1; }
fi

need_file() { [ -f "$1" ] || { echo "missing required file: $1" >&2; exit 1; }; }
need_file module.prop
need_file customize.sh
need_file export.txt
need_file install-recovery.sh
need_file install-desktop.sh
need_file install-macos.command
need_file install-desktop.bat
need_file install-desktop.AppImage
need_file install-desktop.exe
need_file "$PAYLOAD"
need_file META-INF/com/google/android/update-binary
need_file META-INF/com/google/android/updater-script
# Installer binaries: every OS must be staged (they ride bin.lgz in
# full_compressed, raw bin/ in full/minimal).
[ -n "$(ls bin/linux/ 2>/dev/null)" ] || { echo "missing binaries: bin/linux/ is empty" >&2; exit 1; }
[ -n "$(ls bin/windows/ 2>/dev/null)" ] || { echo "missing binaries: bin/windows/ is empty" >&2; exit 1; }
[ -n "$(ls bin/macos/ 2>/dev/null)" ] || { echo "missing binaries: bin/macos/ is empty" >&2; exit 1; }
for _ib in bin/linux/install-linux-x86_64 bin/linux/install-linux-arm64 \
    bin/windows/install-windows-x86_64.exe bin/windows/install-windows-arm64.exe \
    bin/macos/install-macos-universal; do
    [ -f "$_ib" ] || { echo "missing installer binary: $_ib" >&2; exit 1; }
done
# Platform-tools: bundled in full_compressed and full, absent in minimal
# (PATH fallback via FALLBACK_PATH=1 in export.txt covers that layout).
if [ "$OFOX_VARIANT" != minimal ]; then
    [ -x bin/linux/platform-tools/fastboot ] || { echo "missing bin/linux/platform-tools/fastboot" >&2; exit 1; }
    [ -f bin/windows/platform-tools/fastboot.exe ] || { echo "missing bin/windows/platform-tools/fastboot.exe" >&2; exit 1; }
    [ -x bin/macos/platform-tools/fastboot ] || { echo "missing bin/macos/platform-tools/fastboot" >&2; exit 1; }
fi
# Cluster decoders: full_compressed only (lean, unpack-only).
if [ "$OFOX_VARIANT" = full_compressed ]; then
    for _dec in unpack_bin/lgz-lean-linux-x86_64 unpack_bin/lgz-lean-linux-arm64 \
        unpack_bin/lgz-lean-windows-x86_64.exe unpack_bin/lgz-lean-windows-arm64.exe \
        unpack_bin/lgz-lean-macos-universal; do
        need_file "$_dec"
    done
fi
command -v zip >/dev/null 2>&1 || { echo "need the 'zip' tool" >&2; exit 1; }
command -v unzip >/dev/null 2>&1 || { echo "need the 'unzip' tool" >&2; exit 1; }

# Executable bits travel inside the zip; the engine re-chmods anyway.
# (bin/ modes matter too: bin.lgz is packed with --preserve-perms and
# the launchers restore them on unpack; raw variants keep zip modes.)
if [ "$OFOX_VARIANT" = full_compressed ]; then
    chmod +x install-desktop.sh install-macos.command install-recovery.sh META-INF/com/google/android/update-binary unpack_bin/* bin/linux/* bin/windows/* bin/macos/* 2>/dev/null || true
else
    chmod +x install-desktop.sh install-macos.command install-recovery.sh META-INF/com/google/android/update-binary bin/linux/* bin/windows/* bin/macos/* 2>/dev/null || true
fi
chmod 644 module.prop export.txt customize.sh 2>/dev/null || true

rm -f "$OUT"
# Per-device touch archives (bin/modules/<device>.lgz, built by
# fox_build_callback.sh): optional — devices without a local harvest
# simply ship no archive (installer warns, stock touch path covers it).
# full_compressed: ride inside bin.lgz (packed below), counted for the
# cluster-content check. full/minimal: ride as raw files under bin/.
MODULES_LIST=""
if [ -d bin/modules ]; then
    MODULES_LIST="$(cd bin/modules && ls *.lgz 2>/dev/null | sed 's|^|bin/modules/|' || true)"
fi
if [ "$OFOX_VARIANT" = full_compressed ]; then
    # Pack the whole tool tree into one lgz cluster at the zip root
    # (--preserve-perms so +x bits on tools/launchers survive the
    # unpack; level follows build.sh -l/--level via LGZ_LEVEL).
    echo "[$OFOX_VARIANT] packing bin/ -> bin.lgz (level $LGZ_LEVEL, --preserve-perms) ..."
    "$LGZ_BIN" pack bin bin.lgz -l "$LGZ_LEVEL" --preserve-perms
    _bin_lgz_built=1
    zip -qr9 -X "$OUT" module.prop customize.sh export.txt install-recovery.sh \
        install-desktop.sh install-macos.command install-desktop.bat install-desktop.exe install-desktop.AppImage \
        "$PAYLOAD" bin.lgz unpack_bin/ META-INF/
else
    # Raw layout: single zip pass, no lgz. Drop stale unpack markers so
    # the launchers never mistake this tree for an unpacked cluster.
    rm -f bin/.lgz-cluster-size bin/*/.lgz-cluster-size 2>/dev/null || true
    if [ "$OFOX_VARIANT" = minimal ]; then
        echo "[$OFOX_VARIANT] packing raw bin/ without platform-tools (single zip pass) ..."
        zip -qr9 -X "$OUT" module.prop customize.sh export.txt install-recovery.sh \
            install-desktop.sh install-macos.command install-desktop.bat install-desktop.exe install-desktop.AppImage \
            "$PAYLOAD" bin/ META-INF/ \
            -x 'bin/linux/platform-tools/*' 'bin/windows/platform-tools/*' 'bin/macos/platform-tools/*'
    else
        echo "[$OFOX_VARIANT] packing raw bin/ with platform-tools (single zip pass) ..."
        zip -qr9 -X "$OUT" module.prop customize.sh export.txt install-recovery.sh \
            install-desktop.sh install-macos.command install-desktop.bat install-desktop.exe install-desktop.AppImage \
            "$PAYLOAD" bin/ META-INF/ \
            -x 'bin/.lgz-cluster-size' 'bin/*/.lgz-cluster-size'
    fi
fi

echo "--- $OUT ($(du -h "$OUT" | cut -f1)) ---"
unzip -l "$OUT" | head -12

NAMES="$(unzip -l "$OUT" | awk 'NR>3 && $4 != "" {print $4}')"
# The manager legacy tripwire, tested exactly like installers do:
# `unzip -l zip install.sh | grep -q install.sh` must find nothing.
if unzip -l "$OUT" install.sh | grep -q install.sh; then
    echo "BAD: legacy install.sh tripwire fires (managers would take the legacy branch)" >&2
    exit 1
fi
# No desktop weight outside its own names, no build leftovers.
echo "$NAMES" | grep -x -e install.sh -e install.bat -e install.AppImage && { echo "BAD: bare desktop launcher at zip root" >&2; exit 1; } || true
if [ "$OFOX_VARIANT" = full_compressed ]; then
    echo "$NAMES" | grep -E "^(backup|dist/|target/|platform-tools)" && { echo "BAD: weight outside bin.lgz inside zip" >&2; exit 1; } || true
    # Raw tool tree must not leak: tools ship only as the bin.lgz cluster.
    echo "$NAMES" | grep -q "^bin/" && { echo "BAD: raw bin/ leaked into the universal zip (want only bin.lgz)" >&2; exit 1; } || true
    # Everything every consumer needs must be present.
    for f in module.prop customize.sh export.txt install-recovery.sh \
        install-desktop.sh install-macos.command install-desktop.bat install-desktop.exe install-desktop.AppImage \
        "$PAYLOAD" bin.lgz META-INF/com/google/android/update-binary \
        META-INF/com/google/android/updater-script \
        unpack_bin/lgz-lean-linux-x86_64 unpack_bin/lgz-lean-linux-arm64 \
        unpack_bin/lgz-lean-windows-x86_64.exe unpack_bin/lgz-lean-windows-arm64.exe \
        unpack_bin/lgz-lean-macos-universal; do
        echo "$NAMES" | grep -qx "$f" || { echo "BAD: missing $f" >&2; exit 1; }
    done
    # Cluster contents: every OS toolset, installer binaries and staged
    # touch archives must be inside bin.lgz (staged but missing = pack bug,
    # fail loudly). `lgz list` reads the container table without unpacking.
    _CLUSTER_LIST="$("$LGZ_BIN" list bin.lgz)"
    for f in linux/platform-tools/fastboot \
        windows/platform-tools/fastboot.exe \
        macos/platform-tools/fastboot \
        linux/install-linux-x86_64 linux/install-linux-arm64 \
        windows/install-windows-x86_64.exe windows/install-windows-arm64.exe \
        macos/install-macos-universal; do
        echo "$_CLUSTER_LIST" | grep -q "$f" || { echo "BAD: $f missing from bin.lgz" >&2; exit 1; }
    done
    if [ -n "$MODULES_LIST" ]; then
        # shellcheck disable=SC2086
        for f in $MODULES_LIST; do
            # Cluster paths are relative to bin/: strip the prefix.
            echo "$_CLUSTER_LIST" | grep -q "${f#bin/}" || { echo "BAD: $f missing from bin.lgz" >&2; exit 1; }
        done
        echo "OK: $(echo "$MODULES_LIST" | wc -w) device touch archives inside bin.lgz"
    fi
    unset _CLUSTER_LIST
else
    # Raw layouts: no lgz bits allowed, raw bin/ required instead.
    echo "$NAMES" | grep -q "^bin\.lgz" && { echo "BAD: bin.lgz leaked into raw variant $OFOX_VARIANT" >&2; exit 1; } || true
    echo "$NAMES" | grep -q "^unpack_bin/" && { echo "BAD: unpack_bin/ leaked into raw variant $OFOX_VARIANT" >&2; exit 1; } || true
    echo "$NAMES" | grep -q "^bin/" || { echo "BAD: raw bin/ missing from variant $OFOX_VARIANT" >&2; exit 1; }
    for f in module.prop customize.sh export.txt install-recovery.sh \
        install-desktop.sh install-macos.command install-desktop.bat install-desktop.exe install-desktop.AppImage \
        "$PAYLOAD" META-INF/com/google/android/update-binary \
        META-INF/com/google/android/updater-script \
        bin/linux/install-linux-x86_64 bin/linux/install-linux-arm64 \
        bin/windows/install-windows-x86_64.exe bin/windows/install-windows-arm64.exe \
        bin/macos/install-macos-universal; do
        echo "$NAMES" | grep -qx "$f" || { echo "BAD: missing $f" >&2; exit 1; }
    done
    if [ "$OFOX_VARIANT" = minimal ]; then
        # Minimal ships no platform-tools at all (PATH fallback covers it).
        echo "$NAMES" | grep -q "^bin/.*/platform-tools/" && { echo "BAD: platform-tools leaked into minimal zip" >&2; exit 1; } || true
    else
        for f in bin/linux/platform-tools/fastboot \
            bin/windows/platform-tools/fastboot.exe \
            bin/macos/platform-tools/fastboot; do
            echo "$NAMES" | grep -qx "$f" || { echo "BAD: missing $f" >&2; exit 1; }
        done
    fi
    if [ -n "$MODULES_LIST" ]; then
        # shellcheck disable=SC2086
        for f in $MODULES_LIST; do
            echo "$NAMES" | grep -qx "$f" || { echo "BAD: $f missing from raw variant $OFOX_VARIANT" >&2; exit 1; }
        done
        echo "OK: $(echo "$MODULES_LIST" | wc -w) device touch archives inside bin/"
    else
        echo "WARNING: no device touch archives staged (bin/modules/ empty or missing)"
    fi
fi
echo "OK: $OFOX_VARIANT zip verified (manager-safe, desktop-complete)"
