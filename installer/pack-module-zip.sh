#!/usr/bin/env bash
# Packs the UNIVERSAL installer zip: one archive for KernelSU /
# Magisk (new-style customize.sh flow), recovery (META-INF) and
# desktop (install-desktop.sh/.bat/.AppImage + tools under bin/).
#
# The ONLY hard rule for managers: no root-level install.sh — it
# flips them into legacy install.sh mode (they source it with sh
# and fail). Desktop launchers are named install-desktop.* precisely
# to stay invisible to that check. The script verifies the tripwire
# exactly the way the managers test it.
#
# Usage: ./pack-module-zip.sh [output.zip]   (an explicit path always wins)
#
# Without an argument the release name is composed from the environment:
#   {OFOX_NAME}-{OFOX_TYPE}-{OFOX_TAG}-{OFOX_FAMILY}.zip
# defaults: OFOX_NAME=OrangeFox OFOX_TYPE=R12.0 OFOX_TAG=Beta
# OFOX_FAMILY=aio. The zip is placed into the tree's builds/ directory
# (resolved by walking up from here; override with OFOX_BUILDS_DIR).
#
# OFOX_PAYLOAD=/path/to/<payload>.lz4 refreshes the bundled payload: it
# is copied next to this script and export.txt RECOVERY_IMG is rewritten
# to its basename before packing (the stale payload file is removed).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT"

OFOX_NAME="${OFOX_NAME:-OrangeFox}"
OFOX_TYPE="${OFOX_TYPE:-R12.0}"
OFOX_TAG="${OFOX_TAG:-Beta}"
OFOX_FAMILY="${OFOX_FAMILY:-aio}"

_builds_dir() {
    if [ -n "${OFOX_BUILDS_DIR:-}" ]; then printf '%s' "$OFOX_BUILDS_DIR"; return; fi
    _d="$ROOT"
    for _i in 1 2 3 4 5 6; do
        if [ -d "$_d/builds" ]; then printf '%s' "$_d/builds"; return; fi
        _d="$(dirname "$_d")"
    done
    printf '%s' "$ROOT"
}

if [ $# -ge 1 ]; then
    OUT="$1"
else
    OUT="$(_builds_dir)/${OFOX_NAME}-${OFOX_TYPE}-${OFOX_TAG}-${OFOX_FAMILY}.zip"
fi

# Optional payload refresh before the required-file checks below.
if [ -n "${OFOX_PAYLOAD:-}" ]; then
    [ -f "$OFOX_PAYLOAD" ] || { echo "OFOX_PAYLOAD not found: $OFOX_PAYLOAD" >&2; exit 1; }
    _pb="$(basename "$OFOX_PAYLOAD")"
    case "$_pb" in *.lz4) ;; *) echo "OFOX_PAYLOAD must be a .lz4 ramdisk payload" >&2; exit 1;; esac
    _old="$(grep -E '^RECOVERY_IMG=' export.txt 2>/dev/null | tail -1 | cut -d= -f2- | tr -d '\r' | sed 's/^ *//;s/ *$//')"
    cp -f "$OFOX_PAYLOAD" "$ROOT/$_pb"
    sed -i "s|^RECOVERY_IMG=.*|RECOVERY_IMG=$_pb|" export.txt
    if [ -n "$_old" ] && [ "$_old" != "$_pb" ] && [ -f "$ROOT/$_old" ]; then
        rm -f "$ROOT/$_old"
        echo "payload refreshed: $_old -> $_pb (export.txt updated)"
    else
        echo "payload refreshed: $_pb (export.txt updated)"
    fi
fi

cfg() { grep -E "^$1=" export.txt 2>/dev/null | tail -1 | cut -d= -f2- | tr -d '\r' | sed 's/^ *//;s/ *$//;s/^"\(.*\)"$/\1/;s/^'"'"'\(.*\)'"'"'$/\1/'; }
PAYLOAD="$(cfg RECOVERY_IMG OrangeFox-R12.0-test5-aio.ramdisk.lz4)"

need_file() { [ -f "$1" ] || { echo "missing required file: $1" >&2; exit 1; }; }
need_file module.prop
need_file customize.sh
need_file export.txt
need_file install-recovery.sh
need_file install-desktop.sh
need_file install-desktop.bat
need_file install-desktop.AppImage
need_file "$PAYLOAD"
need_file META-INF/com/google/android/update-binary
need_file META-INF/com/google/android/updater-script
[ -n "$(ls bin/linux/ 2>/dev/null)" ] || { echo "missing binaries: bin/linux/ is empty" >&2; exit 1; }
[ -n "$(ls bin/windows/ 2>/dev/null)" ] || { echo "missing binaries: bin/windows/ is empty" >&2; exit 1; }
[ -x bin/linux/platform-tools/fastboot ] || { echo "missing bin/linux/platform-tools/fastboot" >&2; exit 1; }
[ -f bin/windows/platform-tools/fastboot.exe ] || { echo "missing bin/windows/platform-tools/fastboot.exe" >&2; exit 1; }
command -v zip >/dev/null 2>&1 || { echo "need the 'zip' tool" >&2; exit 1; }
command -v unzip >/dev/null 2>&1 || { echo "need the 'unzip' tool" >&2; exit 1; }

# Executable bits travel inside the zip; the engine re-chmods anyway.
chmod +x install-desktop.sh install-recovery.sh META-INF/com/google/android/update-binary bin/linux/* 2>/dev/null || true
chmod 644 module.prop export.txt customize.sh 2>/dev/null || true

rm -f "$OUT"
zip -qr9 -X "$OUT" module.prop customize.sh export.txt install-recovery.sh \
    install-desktop.sh install-desktop.bat install-desktop.AppImage \
    "$PAYLOAD" bin/ META-INF/

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
echo "$NAMES" | grep -E "^(backup|dist/|target/|platform-tools)" && { echo "BAD: weight outside bin/ inside zip" >&2; exit 1; } || true
# Everything every consumer needs must be present.
for f in module.prop customize.sh export.txt install-recovery.sh \
    install-desktop.sh install-desktop.bat install-desktop.AppImage \
    "$PAYLOAD" META-INF/com/google/android/update-binary \
    META-INF/com/google/android/updater-script; do
    echo "$NAMES" | grep -qx "$f" || { echo "BAD: missing $f" >&2; exit 1; }
done
echo "$NAMES" | grep -q "^bin/linux/" || { echo "BAD: no bin/linux/" >&2; exit 1; }
echo "$NAMES" | grep -q "^bin/windows/" || { echo "BAD: no bin/windows/" >&2; exit 1; }
echo "$NAMES" | grep -q "^bin/linux/platform-tools/fastboot" || { echo "BAD: no linux platform-tools" >&2; exit 1; }
echo "$NAMES" | grep -q "^bin/windows/platform-tools/fastboot.exe" || { echo "BAD: no windows platform-tools" >&2; exit 1; }
echo "OK: universal zip verified (manager-safe, desktop-complete)"
