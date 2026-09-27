#!/usr/bin/env bash
# Thin launcher for `bootsmasher install` (the whole installer lives in
# the binary: menus, fetch/rebuild/report/flash, export.txt paths).
# Binaries live in bin/<os>/ as install[-small]-<os>-<arch>; the full
# build wins, small is the fallback. Then legacy spots (next to this
# script, cargo target dir, PATH). Every argument is forwarded.
# Drag-and-drop: when the FIRST argument is an existing file (not a
# flag), it is consumed as the recovery cpio payload and forwarded as
# --recovery-img (overrides export.txt RECOVERY_IMG for this run, no
# editing needed). This covers drops onto this script and onto
# install-desktop.AppImage (its AppRun forwards args here unchanged).
# File managers without drop-onto-executable (KDE Dolphin): just
# double-click — when the export.txt payload is missing the system
# file picker (kdialog/zenity) asks for it instead of failing.
# On interactive terminal runs (no --force, not --help) the script
# pauses for Enter at the end, so a window opened by double-clicking
# (or install.AppImage) stays readable until RESULT is confirmed.
# (The reboot-to-recovery question lives inside the binary now, so
# the launcher never asks twice.)
set -u

# Drag-and-drop payload: a dropped file becomes --recovery-img.
# Only $1 qualifies (flags and the rest pass through untouched).
DROP=""
if [ $# -ge 1 ] && [ -n "${1:-}" ] && [ "${1#-}" = "$1" ] && [ -f "$1" ]; then
    DROP="$1"
    shift
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
case "$(uname -m 2>/dev/null || echo unknown)" in
    x86_64|amd64)   ARCH="x86_64";;
    aarch64|arm64)  ARCH="arm64";;
    armv7l|armv6l)  ARCH="arm32";;
    i686|i386)      ARCH="x86";;
    *)              ARCH="";;
esac

BIN=""
if [ -n "$ARCH" ]; then
    for n in "install-linux-$ARCH" "install-small-linux-$ARCH"; do
        if [ -x "$ROOT/bin/linux/$n" ]; then BIN="$ROOT/bin/linux/$n"; break; fi
    done
fi
if [ -z "$BIN" ]; then
    for n in bootsmasher bootsmasher-x86_64 bootsmasher-aarch64; do
        if [ -x "$ROOT/$n" ]; then BIN="$ROOT/$n"; break; fi
    done
fi
if [ -z "$BIN" ]; then
    for n in bootsmasher bootsmasher-x86_64 bootsmasher-aarch64; do
        if [ -x "$ROOT/../target/release/$n" ]; then BIN="$ROOT/../target/release/$n"; break; fi
    done
fi
if [ -z "$BIN" ]; then
    BIN="$(command -v bootsmasher 2>/dev/null || true)"
fi
if [ -z "$BIN" ]; then
    echo "no installer binary (expected bin/linux/install[-small]-linux-<arch> next to install.sh)" >&2
    exit 1
fi

# Which export.txt this run uses (caller's --export wins, else the one
# next to this script). The binary gets the same file pinned.
EXPORT_FILE="$ROOT/export.txt"
PREV=""
for a in "$@"; do
    if [ "$PREV" = "--export" ]; then EXPORT_FILE="$a"; PREV=""; continue; fi
    case "$a" in
        --export) PREV="--export";;
        --export=*) EXPORT_FILE="${a#--export=}";;
    esac
done
case "$EXPORT_FILE" in /*) ;; *) EXPORT_FILE="$PWD/$EXPORT_FILE";; esac

# System file picker: when the run has no payload — nothing dropped,
# no explicit --recovery-img — and the export.txt payload the binary
# would pin does not exist, ask the desktop for the file instead of
# failing (covers stale export.txt after unpacking a new zip next to
# old configs, and file managers without drop-onto-executable like
# KDE Dolphin, where a bare double-click lands here too via
# install-desktop.AppImage).
# Skipped for scripted/non-payload flows (--force/--file/help and an
# explicit --recovery-img: the caller knows what it is doing), when
# the export payload is healthy, and when no desktop session/dialog
# is available (headless runs keep the binary's clear error).
_NEED_PICK=1
if [ -n "$DROP" ]; then
    _NEED_PICK=0
else
    for a in "$@"; do
        case "$a" in
            --recovery-img|--recovery-img=*|--force|-h|--help|--file) _NEED_PICK=0; break;;
        esac
    done
fi
if [ "$_NEED_PICK" = 1 ] && { [ -n "${DISPLAY:-}" ] || [ -n "${WAYLAND_DISPLAY:-}" ]; }; then
    _PAYLOAD=""
    if [ -f "$EXPORT_FILE" ]; then
        _LINE="$(grep -E '^[[:space:]]*RECOVERY_IMG[[:space:]]*=' "$EXPORT_FILE" 2>/dev/null | tail -1)"
        _VAL="${_LINE#*=}"
        _VAL="$(printf '%s' "$_VAL" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//')"
        case "$_VAL" in
            \"*\") _VAL="${_VAL#\"}"; _VAL="${_VAL%\"}";;
            \'*\') _VAL="${_VAL#\'}"; _VAL="${_VAL%\'}";;
        esac
        case "$_VAL" in
            /*) _PAYLOAD="$_VAL";;
            *) [ -n "$_VAL" ] && _PAYLOAD="$(dirname "$EXPORT_FILE")/$_VAL";;
        esac
    fi
    if [ ! -f "$_PAYLOAD" ]; then
        _START="$(dirname "$EXPORT_FILE")"
        _PICK=""
        if command -v kdialog >/dev/null 2>&1; then
            _PICK="$(kdialog --title "OrangeFox recovery payload" \
                --getopenfilename "$_START" '*.lz4 *.cpio *.img | Recovery payload' \
                2>/dev/null || true)"
        elif command -v zenity >/dev/null 2>&1; then
            _PICK="$(zenity --file-selection --title="OrangeFox recovery payload" \
                --filename="$_START/" --file-filter='Payload | *.lz4 *.cpio *.img' \
                --file-filter='All | *' 2>/dev/null || true)"
        fi
        if [ -n "$_PICK" ]; then
            echo "Payload (file picker): $_PICK"
            DROP="$_PICK"
        fi
    fi
    unset _PAYLOAD _LINE _VAL _START _PICK
fi
unset _NEED_PICK

# Menu runs are interactive; usage/help/--force/--file are not.
INTERACTIVE=1
for a in "$@"; do
    case "$a" in
        --force|-h|--help|--file) INTERACTIVE=0; break;;
    esac
done
if [ "$INTERACTIVE" = 1 ]; then
    echo "Arrow-key menus: Up/Down to move, Enter to choose, q/Esc to exit."
    echo ""
fi

# Pin export.txt to the resolved file, so the launcher works from any
# working directory (incl. AppImage).
HAS_EXPORT=0
for a in "$@"; do
    if [ "$a" = "--export" ] || [[ "$a" == --export=* ]]; then HAS_EXPORT=1; break; fi
done
if [ "$HAS_EXPORT" = 0 ]; then
    set -- --export "$EXPORT_FILE" "$@"
fi
# Dropped payload (see top): forwarded as --recovery-img, which the
# binary applies over export.txt RECOVERY_IMG for this run only.
if [ -n "$DROP" ]; then
    echo "Payload (dropped file): $DROP"
    set -- --recovery-img "$DROP" "$@"
fi
"$BIN" install "$@"
RC=$?

# Keep a double-click terminal window readable: pause on interactive
# runs (tty on stdin+stdout, no --force, not --help) so the RESULT
# stays visible until confirmed. Scripted/piped runs never pause.
PAUSE=0
if [ -t 0 ] && [ -t 1 ]; then
    PAUSE=1
    for a in "$@"; do
        case "$a" in
            --force|-h|--help) PAUSE=0; break;;
        esac
    done
fi
if [ "$PAUSE" = 1 ]; then
    printf '%s' "Press Enter to exit... "
    read -r _ < /dev/tty 2>/dev/null || read -r _ || true
fi
exit $RC
