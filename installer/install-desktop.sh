#!/usr/bin/env bash
# Thin launcher for `bootsmasher install` (the whole installer lives in
# the binary: menus, fetch/rebuild/report/flash, export.txt paths).
# Binaries live in bin/<os>/ as install[-small]-<os>-<arch>; the full
# build wins, small is the fallback. Then legacy spots (next to this
# script, cargo target dir, PATH). Every argument is forwarded.
# On interactive terminal runs (no --force, not --help) the script
# pauses for Enter at the end, so a window opened by double-clicking
# (or install.AppImage) stays readable until RESULT is confirmed.
# After a successful flash it also offers to reboot the device to
# recovery via fastboot (paths and serial taken from export.txt and
# the run's install.log; default answer is No).
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
# next to this script). Needed below for the backup dir, fastboot
# paths and the reboot offer; the binary gets the same file pinned.
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
EBASE="$(dirname "$EXPORT_FILE")"
# One KEY=VALUE out of export.txt (# comments, CR and quotes tolerated).
cfg_val() {
    local v=""
    [ -f "$EXPORT_FILE" ] && v="$(grep -E "^$1=" "$EXPORT_FILE" 2>/dev/null | tail -1 | cut -d= -f2- | tr -d '\r' | sed 's/^ *//;s/ *$//;s/^"\(.*\)"$/\1/;s/^'"'"'\(.*\)'"'"'$/\1/')"
    [ -n "$v" ] || v="$2"
    printf '%s' "$v"
}
BACKUP_DIR="$(cfg_val BACKUP_DIR backup)"
case "$BACKUP_DIR" in /*) ;; *) BACKUP_DIR="$EBASE/$BACKUP_DIR";; esac
BEFORE="$(ls -t "$BACKUP_DIR" 2>/dev/null | head -1)"

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

# After a successful flash (fresh backup stamp whose install.log says
# RESULT: OK), offer to reboot the device to recovery. Aborts and
# failures never reach the question. Default answer is No.
if [ -t 0 ] && [ -t 1 ] && [ "$RC" = 0 ]; then
    ASK=1
    for a in "$@"; do
        case "$a" in
            --force|--file) ASK=0; break;;
        esac
    done
    if [ "$ASK" = 1 ]; then
        AFTER="$(ls -t "$BACKUP_DIR" 2>/dev/null | head -1)"
        RUNLOG="$BACKUP_DIR/$AFTER/install.log"
        if [ -n "$AFTER" ] && [ "$AFTER" != "$BEFORE" ] && [ -f "$RUNLOG" ] \
            && grep -q "RESULT: OK" "$RUNLOG" 2>/dev/null; then
            SERIAL="$(grep -E '^device=' "$RUNLOG" 2>/dev/null | tail -1 | cut -d= -f2- | tr -d '\r ')"
            if [ -n "$SERIAL" ]; then
                ans=""
                printf '%s' "Reboot the device to recovery now? [y/N] "
                read -r ans < /dev/tty 2>/dev/null || read -r ans || true
                case "$ans" in
                    [yY]*)
                        PT="$(cfg_val PLATFORM_TOOLS_LINUX platform-tools-linux)"
                        case "$PT" in /*) ;; *) PT="$EBASE/$PT";; esac
                        FB="$PT/$(cfg_val FASTBOOT_BIN fastboot)"
                        if [ ! -x "$FB" ]; then
                            echo "  fastboot not found ($FB): reboot manually (bootloader menu -> Recovery mode)"
                        else
                            echo "  rebooting $SERIAL to recovery..."
                            if command -v timeout >/dev/null 2>&1; then
                                timeout 60 "$FB" -s "$SERIAL" reboot recovery
                                FRC=$?
                            else
                                "$FB" -s "$SERIAL" reboot recovery
                                FRC=$?
                            fi
                            if [ "$FRC" = 0 ]; then
                                echo "  reboot command sent"
                            else
                                echo "  reboot failed: select Recovery mode in the bootloader menu manually"
                            fi
                        fi
                        ;;
                    *) echo "  (leaving the device in the bootloader)";;
                esac
            fi
        fi
    fi
fi

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
