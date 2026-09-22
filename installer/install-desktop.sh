#!/usr/bin/env bash
# Thin launcher for `bootsmasher install` (the whole installer lives in
# the binary: menus, fetch/rebuild/report/flash, export.txt paths).
# Binaries live in bin/<os>/ as install[-small]-<os>-<arch>; the full
# build wins, small is the fallback. Then legacy spots (next to this
# script, cargo target dir, PATH). Every argument is forwarded.
# On interactive terminal runs (no --force, not --help) the script
# pauses for Enter at the end, so a window opened by double-clicking
# (or install.AppImage) stays readable until RESULT is confirmed.
# (The reboot-to-recovery question lives inside the binary now, so
# the launcher never asks twice.)
set -u

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
