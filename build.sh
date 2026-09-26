#!/bin/bash
#
# build.sh — OrangeFox Recovery build script for all Tensor Pixel devices.
#
# Usage:
#   ./build.sh [--family DEV|FAMILY] [--notrm] [-j N] [--name TAG] [--patch N] [--level 0-3]
#              [-k|--kernel VER] [--force] [--list] [--build-type TYPE]
#              [-N|--no-first-stage] [-c|--cpio-only] [--platform-recovery] [--new-theme]
#   source ./build.sh [...]   # same, but runs in the current shell (env kept)
#
# Options:
#   -f, --family TARGET Device codename (husky, shiba, ...) or SoC family
#                     (gs201/zuma/zumapro/gs101). Families and devices are
#                     discovered from families/ and devices/ — no hardcoded list.
#                     If omitted, vendorsetup.sh interactive menu is used.
#   -k, --kernel VER  Kernel profile version from families/*/family.json
#                     `kernels` (e.g., 6.1, 6.12). Device pixel.json may
#                     override the family profile wholesale. Devices sharing
#                     the effective profile build ONE image (zuma.img);
#                     diverged devices build their own (zuma_husky.img).
#                     Without -k, an interactive version picker is shown.
#   --force           Non-interactive mode: with -k builds silently;
#                     WITHOUT -k aborts with the available version list
#                     (no silent default in scripts/CI).
#   --list            Print the family/device/kernel tree (families with
#                     keymint, default + available kernels; devices with
#                     effective kernels, "[override]" marks a device-level
#                     `kernels` entry) and exit. Read-only, builds nothing.
#   --build-type TYPE Build type string, default Stable. Only exact `Stable`
#                     enables OF_ADVANCED_SECURITY downstream (adbd stopped
#                     at boot, MTP autostart off); any other value builds a
#                     non-Secure image. Exported for vendorsetup.sh/lunch.
#   --notrm           Don't clean out/target/product/pixels before build.
#                     (Between kernel-profile groups a clean is mandatory
#                     and always performed, with a notice.)
#   -n, --name TAG    Name tag for output files. Copies final .img/.zip to
#                     builds/OrangeFox-<VERSION>-{TAG}-{family}.img/zip
#   -p, --patch N     Set FOX_MAINTAINER_PATCH_VERSION (numbers only).
#                     E.g., "--patch 5" will result in version R11.3_5.
#   -l, --level N     LGZ cluster compression level 0-3 (default 0=fast).
#                     Exported as LGZ_LEVEL for fox_build_callback.sh.
#   -j N              Parallel build jobs (also as -jN). Default: nproc.
#                     Exported as JOBS for the Soong/make invocation.
#   --platform-recovery
#                     Recovery-in-platform test layout (var2-AIO): merge
#                     first-stage + recovery into one ramdisk
#                     (BOARD_INCLUDE_RECOVERY_RAMDISK_IN_VENDOR_BOOT=false,
#                     like gs101) and pack first_stage files into the payload
#                     cpio. Exported as FOX_RECOVERY_IN_PLATFORM=1 for
#                     BoardConfig.mk + .build_platform.conf (callback merge).
#   -N, --no-first-stage
#                     Skip first-stage (vendor_ramdisk) components: no
#                     fstab.*.vendor_ramdisk, no linker/e2fs vendor_ramdisk
#                     tools (see device.mk). Recovery ramdisk is unaffected.
#                     Exported as FOX_NO_FIRST_STAGE=1 for device.mk.
#   -c, --cpio-only   Deliver only the ramdisk cpio.lz4 (lz4_legacy):
#                     gs101 → platform fragment (`fastboot flash vendor_boot:`),
#                     other families → recovery fragment
#                     (`fastboot flash vendor_boot:recovery`). The .img/.zip
#                     are NOT copied to builds/ in this mode.
#   --new-theme       Build the reworked wide-variant theme (twres_1280/
#                     1344/1440/1600/1840/2076 XML overlays via
#                     tools/theme_wide.py). Test-gated: default builds ship
#                     the stock base theme only. Exported as
#                     FOX_REWORK_THEME=1 (Soong Getenv + vendorsetup.sh).
#   --push GROUP      Push the finished AIO installer zip to Telegram chat(s)
#                     via tools/tg_push.py (bot token + chat ids live in the
#                     gitignored .tg_push.json: {"token": "...",
#                     "group": {"name": id}}). Several groups allowed, comma
#                     separated: --push testers,g6. Non-fatal: a push failure
#                     only warns, the build itself is already delivered.
#                     Reserved name "admin" sends into admin DMs instead of
#                     groups (admin user ids live in the "admin" map of
#                     .tg_push.json; the user must have /start'ed the bot).
#   -g, --git-tag     Tag this build in git: -n/--name value + datetime
#                     (e.g. -n test8.7 -> test8.7-20260923-0130). Refuses to
#                     build on a dirty tree (uncommitted changes = error
#                     before anything runs). The tag is deleted automatically
#                     if the build fails; pushing is manual (VS Code).
#   -D, --diff-tag    With --push: collect `git log` between the previous
#                     reachable tag and the fresh -g tag (or HEAD) and send
#                     it after the zip — as a text message, or as a
#                     changes_<tag>.txt file ("changes" caption) when longer
#                     than 2400 chars. Warns and sends zip-only without history.
#   --diff-from TAG   With --push: force the change list as `git log`
#                     TAG..HEAD (fresh -g tag == HEAD, so this covers up to
#                     the tag being formed), whatever tags sit in between.
#                     Long lists go to changes_from_<TAG>.txt. Overrides -D.
#                     TAG must exist, else warning + zip-only.
#   -T, --text TEXT   With --push: append TEXT to the zip message after the
#                     md5 line (e.g. -T "looks like OTG is fixed on P10").
#   -h, --help        Show this help.
# END HELP

# NOTE: errexit/pipefail apply to direct execution. When this file is
# sourced, shell options of the caller are left alone (see fox_sourced
# handling below) so a failure never kills the interactive terminal.
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
    set -eo pipefail
fi

# --- Source-safe exits (reference: evox make_n.sh safe_exit) ---
# In a sourced shell `exit` would kill the user's terminal. Every abort
# therefore ends in an inline return/exit block (return works ONLY at the
# top level of a sourced script — never hidden inside a function).
fox_sourced=false
if [[ "${BASH_SOURCE[0]}" != "${0}" ]]; then
    fox_sourced=true
fi
SAFE_EXIT_REQUESTED=false
SAFE_EXIT_CODE=0

fox_safe_exit() {
    SAFE_EXIT_CODE="${1:-1}"
    SAFE_EXIT_REQUESTED=true
    # Failed runs must not leave a version tag behind (sourced mode never
    # reaches the EXIT trap below, so clean up here too; idempotent).
    if [[ "$SAFE_EXIT_CODE" != 0 && "${FOX_TAG_CREATED:-}" == 1 && -n "${FOX_TAG_NAME:-}" && -n "${SCRIPT_DIR:-}" ]]; then
        git -C "$SCRIPT_DIR" tag -d "$FOX_TAG_NAME" >/dev/null 2>&1 \
            && echo "[build] Removed git tag $FOX_TAG_NAME (failed build)" || true
        FOX_TAG_CREATED=0
    fi
    # Ping admin on failure (text-only, never fatal: bot/network/config
    # issues only warn). Sent once: SAFE_EXIT_CODE sticks, repeat calls
    # with the same code are no-ops via the guard below.
    if [[ "$SAFE_EXIT_CODE" != 0 && "${FOX_ADMIN_NOTIFIED:-}" != 1 && -n "${SCRIPT_DIR:-}" ]]; then
        FOX_ADMIN_NOTIFIED=1
        _admin_notify "OrangeFox build FAILED: ${BUILD_NAME:-?} (exit $SAFE_EXIT_CODE, tag ${FOX_TAG_NAME:-none})" || true
    fi
}

# Best-effort admin DM ping via tg_push.py --notify (text-only, no zip).
# Never fatal: any failure only warns, the build result is unaffected.
# No-op when tg_push.py is missing (e.g. partial tree).
_admin_notify() {
    if [[ -z "${SCRIPT_DIR:-}" || ! -f "$SCRIPT_DIR/tools/tg_push.py" ]]; then
        echo "[build] WARNING: admin notify skipped (tg_push.py missing)"
        return 0
    fi
    python3 "$SCRIPT_DIR/tools/tg_push.py" --notify "$1" admin \
        || echo "[build] WARNING: admin notify failed (build result unaffected)"
}

# Stop the flow NOW: inline return/exit (see note above).
# Usage (top level ONLY):
#   if [[ "$SAFE_EXIT_REQUESTED" == true ]]; then
#       if [[ "$fox_sourced" == true ]]; then
#           return "$SAFE_EXIT_CODE"
#       else
#           exit "$SAFE_EXIT_CODE"
#       fi
#   fi

fox_on_interrupt() {
    echo ""
    echo "[build] Interrupted (SIGINT/SIGTERM)."
    trap - SIGINT SIGTERM
    # Sourced mode continues after the trap: stop the flow explicitly.
    SAFE_EXIT_REQUESTED=true
    SAFE_EXIT_CODE=130
}

trap fox_on_interrupt SIGINT SIGTERM

for var in ${!FOX_@} ${!OF_@} ${!TARGET_@} ${!TW_@}; do     unset "$var"; done
unset DEVICE_BUILD_FLAG

# --- Resolve paths ---
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SOURCE_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"

# nproc is banned in make recipe shells and may be missing elsewhere.
fox_nproc() {
    local n
    n=$(nproc 2>/dev/null) && [ "$n" -gt 0 ] 2>/dev/null && { echo "$n"; return 0; }
    n=$(getconf _NPROCESSORS_ONLN 2>/dev/null) && [ "$n" -gt 0 ] 2>/dev/null && { echo "$n"; return 0; }
    n=$(grep -c ^processor /proc/cpuinfo 2>/dev/null) && [ "$n" -gt 0 ] 2>/dev/null && { echo "$n"; return 0; }
    echo 8
}

# --- Device/family/kernel inventory (build.sh --list) ---
# Tree view over families/*/family.json + devices/*/device.conf:
# per family keymint/default kernels, per device effective kernel versions
# (family ∪ device overrides; "[override]" marks a devices/<dev>/pixel.json
# `kernels` entry). Read-only: prints and returns, never builds.
fox_print_tree() {
    local fam_json fam info dev_conf dev dfam klist over
    echo "SoC families (families/*/family.json) and devices (devices/*/device.conf):"
    echo ""
    for fam_json in "$SCRIPT_DIR"/families/*/family.json; do
        fam=$(basename "$(dirname "$fam_json")")
        info=$(python3 -c "import json,sys; f=json.load(open(sys.argv[1])); print('%s|%s|%s' % (f.get('keymint','?'), f.get('default_kernel','?'), ','.join(sorted((f.get('kernels') or {}).keys())) or '(none)'))" "$fam_json" 2>/dev/null) || {
            echo "  $fam: ERROR: unreadable family.json"
            continue
        }
        echo "$fam [keymint=$(echo "$info" | cut -d'|' -f1), default=$(echo "$info" | cut -d'|' -f2), kernels=$(echo "$info" | cut -d'|' -f3)]"
        local any_dev=false
        for dev_conf in "$SCRIPT_DIR"/devices/*/device.conf; do
            dev=$(basename "$(dirname "$dev_conf")")
            dfam=$(FAMILY=""; DEVICE=""; . "$dev_conf" 2>/dev/null; printf '%s' "$FAMILY")
            [ "$dfam" = "$fam" ] || continue
            any_dev=true
            klist=$(python3 "$SCRIPT_DIR/include/prebuilt/gen_kernel_mk.py" --list "$fam" "$dev" 2>/dev/null) || klist="(error)"
            over=""
            python3 -c "import json,sys; sys.exit(0 if 'kernels' in json.load(open(sys.argv[1])) else 1)" "$SCRIPT_DIR/devices/$dev/pixel.json" 2>/dev/null && over=" [override]"
            echo "  $dev [kernels=${klist}${over}]"
        done
        $any_dev || echo "  (no devices)"
    done
}

# --- Parse arguments ---
FAMILY=""
CLEAN=true
JOBS="$(fox_nproc)"
BUILD_NAME=""
PATCH_VERSION=""
LGZ_LEVEL="0"
KERNEL_VER=""
FOX_FORCE=false
FOX_BUILD_TYPE="Stable"
FOX_NO_FIRST_STAGE=""
CPIO_ONLY=false
FOX_GIT_TAG=""
FOX_DIFF_TAG=""
FOX_DIFF_FROM=""
FOX_PUSH_TEXT=""

while [[ $# -gt 0 ]] && [[ "$SAFE_EXIT_REQUESTED" == false ]]; do
    case "$1" in
        -f|--family)
            shift
            FAMILY="${1:-}"
            if [[ -z "$FAMILY" ]]; then
                echo "ERROR: --family requires an argument (device or family)"
                fox_safe_exit 1
            fi
            if [[ "$SAFE_EXIT_REQUESTED" == false ]]; then
                # families/common/ holds shared files, it is not buildable.
                if [[ "$FAMILY" != "common" && -d "$SCRIPT_DIR/families/$FAMILY" ]]; then
                    : # already a SoC family
                elif [[ -f "$SCRIPT_DIR/devices/$FAMILY/device.conf" ]]; then
                    _DEV="$FAMILY"
                    # shellcheck disable=SC1090
                    . "$SCRIPT_DIR/devices/$FAMILY/device.conf"  # sets FAMILY
                    echo "[build] Resolved device $_DEV -> family $FAMILY"
                else
                    echo "ERROR: unknown family/device '$FAMILY'."
                    echo "  Families: $(for d in "$SCRIPT_DIR"/families/*/; do b=$(basename "$d"); [ "$b" = "common" ] || printf '%s ' "$b"; done | sort | tr '\n' ' ')"
                    echo "  Devices:  $(for d in "$SCRIPT_DIR"/devices/*/; do basename "$d"; done | sort | tr '\n' ' ')"
                    fox_safe_exit 1
                fi
            fi
            shift
            ;;
        --notrm)
            CLEAN=false
            shift
            ;;
        -j)
            shift
            JOBS="${1:-$(fox_nproc)}"
            shift
            ;;
        -j[0-9]*)
            JOBS="${1#-j}"
            shift
            ;;
        -n|--name)
            shift
            BUILD_NAME="${1:-}"
            if [[ -z "$BUILD_NAME" ]]; then
                echo "ERROR: --name requires an argument"
                fox_safe_exit 1
            fi
            shift
            ;;
        -p|--patch)
            shift
            PATCH_VERSION="${1:-}"
            if [[ -z "$PATCH_VERSION" || ! "$PATCH_VERSION" =~ ^[0-9]+$ ]]; then
                echo "ERROR: --patch requires a numeric argument (e.g., 5)"
                fox_safe_exit 1
            fi
            export FOX_MAINTAINER_PATCH_VERSION="$PATCH_VERSION"
            shift
            ;;
        -l|--level)
            shift
            LGZ_LEVEL="${1:-}"
            if [[ -z "$LGZ_LEVEL" || ! "$LGZ_LEVEL" =~ ^[0-3]$ ]]; then
                echo "ERROR: --level requires 0, 1, 2 or 3"
                fox_safe_exit 1
            fi
            shift
            ;;
        -k|--kernel)
            shift
            KERNEL_VER="${1:-}"
            if [[ -z "$KERNEL_VER" ]]; then
                echo "ERROR: --kernel requires a version argument (e.g., 6.1)"
                fox_safe_exit 1
            fi
            shift
            ;;
        --force)
            FOX_FORCE=true
            shift
            ;;
        -N|--no-first-stage)
            FOX_NO_FIRST_STAGE=1
            shift
            ;;
        --platform-recovery)
            FOX_RECOVERY_IN_PLATFORM=1
            shift
            ;;
        --new-theme)
            FOX_REWORK_THEME=1
            shift
            ;;
        --push)
            shift
            FOX_PUSH_GROUP="${1:-}"
            if [[ -z "$FOX_PUSH_GROUP" ]]; then
                echo "ERROR: --push requires a group name (see .tg_push.json)"
                fox_safe_exit 1
            fi
            shift
            ;;
        -g|--git-tag)
            FOX_GIT_TAG=1
            shift
            ;;
        -D|--diff-tag)
            FOX_DIFF_TAG=1
            shift
            ;;
        --diff-from)
            shift
            FOX_DIFF_FROM="${1:-}"
            if [[ -z "$FOX_DIFF_FROM" ]]; then
                echo "ERROR: --diff-from requires a tag (e.g. --diff-from test8.9-20260924-0411)"
                fox_safe_exit 1
            fi
            shift
            ;;        -T|--text)
            shift
            FOX_PUSH_TEXT="${1:-}"
            if [[ -z "$FOX_PUSH_TEXT" ]]; then
                echo "ERROR: --text requires a message (e.g. -T \"OTG fixed on P10?\")"
                fox_safe_exit 1
            fi
            shift
            ;;
        --build-type)
            shift
            FOX_BUILD_TYPE="${1:-}"
            if [[ -z "$FOX_BUILD_TYPE" ]]; then
                echo "ERROR: --build-type requires a value (e.g., Stable, Beta)"
                fox_safe_exit 1
            fi
            shift
            ;;
        -c|--cpio-only)
            CPIO_ONLY=true
            shift
            ;;
        --list)
            fox_print_tree
            fox_safe_exit 0
            if [[ "$fox_sourced" == true ]]; then
                return "$SAFE_EXIT_CODE"
            else
                exit "$SAFE_EXIT_CODE"
            fi
            ;;
        -h|--help)
            sed -n '1,/^# END HELP$/p' "${BASH_SOURCE[0]}"
            fox_safe_exit 0
            ;;
        *)
            echo "Unknown option: $1"
            fox_safe_exit 1
            ;;
    esac
done

if [[ "$SAFE_EXIT_REQUESTED" == true ]]; then
    if [[ "$fox_sourced" == true ]]; then
        return "$SAFE_EXIT_CODE"
    else
        exit "$SAFE_EXIT_CODE"
    fi
fi

# --- Version tag (--git-tag): name + datetime, clean tree or no build ---
# Runs before anything mutates the tree (.gen_kernel.mk, .build_platform.conf
# are all generated later). Push stays manual; a failed build deletes the tag
# (via fox_safe_exit + the EXIT trap below), so a tag always means "built OK".
fox_tag_cleanup() {
    if [[ "${FOX_TAG_CREATED:-}" == 1 && "${FOX_BUILD_OK:-}" != 1 && -n "${FOX_TAG_NAME:-}" ]]; then
        git -C "$SCRIPT_DIR" tag -d "$FOX_TAG_NAME" >/dev/null 2>&1 \
            && echo "[build] Removed git tag $FOX_TAG_NAME (failed build)" || true
    fi
}
if [[ -n "$FOX_GIT_TAG" ]]; then
    command -v git >/dev/null 2>&1 || { echo "ERROR: --git-tag needs git in PATH"; fox_safe_exit 1; }
    if [[ "$SAFE_EXIT_REQUESTED" == false && -z "$BUILD_NAME" ]]; then
        echo "ERROR: --git-tag needs -n/--name (tag = <name>-<datetime>)"
        fox_safe_exit 1
    fi
    if [[ "$SAFE_EXIT_REQUESTED" == false ]]; then
        FOX_TAG_NAME="${BUILD_NAME}-$(date '+%Y%m%d-%H%M')"
        if git -C "$SCRIPT_DIR" rev-parse -q --verify "refs/tags/$FOX_TAG_NAME" >/dev/null 2>&1; then
            echo "ERROR: git tag $FOX_TAG_NAME already exists (built this minute?)."
            echo "  Delete it first: git -C $SCRIPT_DIR tag -d $FOX_TAG_NAME"
            fox_safe_exit 1
        fi
    fi
    if [[ "$SAFE_EXIT_REQUESTED" == false ]]; then
        _dirty=$(git -C "$SCRIPT_DIR" status --porcelain 2>&1) || { echo "ERROR: git status failed in $SCRIPT_DIR"; fox_safe_exit 1; }
    fi
    if [[ "$SAFE_EXIT_REQUESTED" == false && -n "$_dirty" ]]; then
        echo "ERROR: uncommitted changes in $SCRIPT_DIR — commit or stash first:"
        echo "$_dirty" | sed 's/^/  /'
        fox_safe_exit 1
    fi
    unset _dirty
    if [[ "$SAFE_EXIT_REQUESTED" == false ]]; then
        git -C "$SCRIPT_DIR" tag -a "$FOX_TAG_NAME" -m "OrangeFox $BUILD_NAME build ($FOX_TAG_NAME)" \
            || fox_safe_exit 1
    fi
    if [[ "$SAFE_EXIT_REQUESTED" == false ]]; then
        FOX_TAG_CREATED=1
        echo "[build] Git tag created: $FOX_TAG_NAME"
        if [[ "$fox_sourced" != true ]]; then
            trap fox_tag_cleanup EXIT
        fi
    fi
    if [[ "$SAFE_EXIT_REQUESTED" == true ]]; then
        if [[ "$fox_sourced" == true ]]; then
            return "$SAFE_EXIT_CODE"
        else
            exit "$SAFE_EXIT_CODE"
        fi
    fi
fi
export LGZ_LEVEL
export FOX_BUILD_TYPE
# First-stage kill-switch for device.mk (make imports env): set only when
# -N/--no-first-stage was passed, so normal builds see an empty var.
# Also persisted to .build_platform.conf: the callback must strip the
# first_stage_ramdisk copy out of the recovery root even after the
# RECOVERY_IN_PLATFORM merge below (that merge would otherwise resurrect
# ~9MB of stock first_stage the installer preserves anyway).
if [[ -n "$FOX_NO_FIRST_STAGE" ]]; then
    export FOX_NO_FIRST_STAGE
    echo "[build] First-stage components DISABLED (FOX_NO_FIRST_STAGE=1)"
    _plat_conf="$SCRIPT_DIR/.build_platform.conf"
    if [[ -f "$_plat_conf" ]]; then
        if grep -q '^NO_FIRST_STAGE=' "$_plat_conf" 2>/dev/null; then
            sed -i 's/^NO_FIRST_STAGE=.*/NO_FIRST_STAGE=1/' "$_plat_conf"
        else
            echo "NO_FIRST_STAGE=1" >> "$_plat_conf"
        fi
        echo "[build] .build_platform.conf updated: NO_FIRST_STAGE=1"
    fi
    unset _plat_conf
fi
# Recovery-in-platform test layout (var2-AIO): empty by default, so normal
# builds see an empty var. Exported for BoardConfig.mk (make imports env);
# .build_platform.conf (lunch-time file read by the callback in recipe
# shells, where custom env is stripped) is updated below at build time.
if [[ -n "${FOX_RECOVERY_IN_PLATFORM:-}" ]]; then
    export FOX_RECOVERY_IN_PLATFORM
    echo "[build] Recovery-in-platform layout ENABLED (FOX_RECOVERY_IN_PLATFORM=1)"
    _plat_conf="$SCRIPT_DIR/.build_platform.conf"
    if [[ -f "$_plat_conf" ]]; then
        if grep -q '^RECOVERY_IN_PLATFORM=' "$_plat_conf" 2>/dev/null; then
            sed -i 's/^RECOVERY_IN_PLATFORM=.*/RECOVERY_IN_PLATFORM=1/' "$_plat_conf"
        else
            echo "RECOVERY_IN_PLATFORM=1" >> "$_plat_conf"
        fi
        echo "[build] .build_platform.conf updated: RECOVERY_IN_PLATFORM=1"
    else
        echo "[build] WARNING: $_plat_conf missing, callback merge will be skipped"
    fi
fi
# Reworked (wide-variant) theme, test-gated: empty by default, so normal
# builds ship the stock base theme only. Exported for BoardConfig.mk (make
# re-exports it to Soong, which reads it via Getenv in
# orangefox_defaults.go); .build_platform.conf (lunch-time file read by
# the callback in recipe shells, where custom env is stripped) decides
# whether theme_wide.py variants are generated and packed below.
if [[ -n "${FOX_REWORK_THEME:-}" ]]; then
    export FOX_REWORK_THEME
    echo "[build] Reworked theme ENABLED (FOX_REWORK_THEME=1)"
    _plat_conf="$SCRIPT_DIR/.build_platform.conf"
    if [[ -f "$_plat_conf" ]]; then
        if grep -q '^REWORK_THEME=' "$_plat_conf" 2>/dev/null; then
            sed -i 's/^REWORK_THEME=.*/REWORK_THEME=1/' "$_plat_conf"
        else
            echo "REWORK_THEME=1" >> "$_plat_conf"
        fi
        echo "[build] .build_platform.conf updated: REWORK_THEME=1"
    else
        echo "[build] WARNING: $_plat_conf missing, theme variants will be skipped"
    fi
fi
if [[ "$CPIO_ONLY" == true ]]; then
    echo "[build] cpio-only delivery: .img/.zip will NOT be copied, only ramdisk cpio.lz4"
fi

# --- Kernel profile resolution (families/*/family.json `kernels`) ---
# Engages only with a known family context (-f). Without -f the legacy
# path is kept (interactive vendorsetup owns device selection).
# AIO (-f aio) skips this entirely: no kernel image is built, the stock
# kernel (and its cmdline) is kept, so no profile is needed and -k is
# rejected as meaningless.
# Result: KERNEL_GROUPS entries "tag|gen_dev", KERNEL_MK path (or empty
# for the legacy path), FOX_KERNEL_VER exported for the callback.
KERNEL_GROUPS=()
KERNEL_MK=""
if [[ "$FAMILY" == "aio" ]]; then
    if [[ -n "$KERNEL_VER" ]]; then
        echo "ERROR: -f aio builds no kernel image; -k/--kernel is meaningless here."
        fox_safe_exit 2
    fi
    echo "[build] AIO mode: stock kernel kept, kernel profiles skipped"
    FOX_KERNEL_VER="stock"
    export FOX_KERNEL_VER
elif [[ -n "$FAMILY" ]]; then
    KFAMILY="$FAMILY"
    KDEV="${_DEV:-}"
    if [[ -n "$KDEV" ]]; then
        KLIST_RAW=$(python3 "$SCRIPT_DIR/include/prebuilt/gen_kernel_mk.py" --list "$KFAMILY" "$KDEV" 2>&1) || {
            echo "ERROR: kernel profile list crashed for $KFAMILY/$KDEV (not 'no kernels' — the script itself failed):"
            echo "$KLIST_RAW" >&2
            fox_safe_exit 2
        }
    else
        KLIST_RAW=$(python3 "$SCRIPT_DIR/include/prebuilt/gen_kernel_mk.py" --list "$KFAMILY" 2>&1) || {
            echo "ERROR: kernel profile list crashed for $KFAMILY (not 'no kernels' — the script itself failed):"
            echo "$KLIST_RAW" >&2
            fox_safe_exit 2
        }
    fi
    if [[ -n "$KLIST_RAW" && "$KLIST_RAW" != "(none)"* ]]; then
        KDEFAULT=$(echo "$KLIST_RAW" | sed -n 's/.*(default: \([^)]*\)).*/\1/p')
        # shellcheck disable=SC2206
        KVERSIONS=($(echo "$KLIST_RAW" | sed 's/ (default:.*//'))
        if [[ -n "$KERNEL_VER" ]]; then
            FOX_KERNEL_VER="$KERNEL_VER"
        elif [[ "$FOX_FORCE" == true ]]; then
            echo "ERROR: kernel version not specified (use -k VER)."
            echo "  Available for $KFAMILY: $KLIST_RAW"
            fox_safe_exit 2
        elif [[ -t 0 ]]; then
            echo "[build] Select kernel profile for $KFAMILY (default: ${KDEFAULT:-${KVERSIONS[0]}}):"
            select FOX_KERNEL_VER in "${KVERSIONS[@]}"; do
                if [[ -z "$FOX_KERNEL_VER" ]]; then
                    FOX_KERNEL_VER="${KDEFAULT:-${KVERSIONS[0]}}"
                fi
                break
            done < /dev/tty
            echo "[build] Kernel profile: $FOX_KERNEL_VER"
        else
            echo "ERROR: kernel version not specified (use -k VER; no tty for picker)."
            echo "  Available for $KFAMILY: $KLIST_RAW"
            fox_safe_exit 2
        fi
        if [[ "$SAFE_EXIT_REQUESTED" == false ]]; then
            FP_JSON=$(python3 "$SCRIPT_DIR/include/prebuilt/gen_kernel_mk.py" --fingerprint "$KFAMILY" "$FOX_KERNEL_VER" 2>&1) || {
                echo "$FP_JSON" >&2
                fox_safe_exit 2
            }
        fi
        if [[ "$SAFE_EXIT_REQUESTED" == false ]]; then
            KERNEL_MK="$SCRIPT_DIR/families/$KFAMILY/.gen_kernel.mk"
            if [[ "$FP_JSON" == "[]" ]]; then
                # Family without pixel.json devices (e.g. gs101 WIP):
                # single group straight from the family profile.
                KERNEL_GROUPS=("$KFAMILY|-")
            else
            GROUP_LINES=$(FP_JSON="$FP_JSON" python3 -c '
import json, os
fps = json.loads(os.environ["FP_JSON"])
byhash = {}
for e in fps:
    byhash.setdefault(e["hash"], []).append(e["device"])
for h in sorted(byhash):
    print(h + "|" + ",".join(sorted(byhash[h])))' 2>/dev/null) || GROUP_LINES=""
            ALL_DEVS=$(FP_JSON="$FP_JSON" python3 -c '
import json, os
print(",".join(sorted(e["device"] for e in json.loads(os.environ["FP_JSON"]))))' 2>/dev/null) || ALL_DEVS=""
            while IFS='|' read -r _h _devs; do
                [ -n "$_devs" ] || continue
                if [[ -n "${KDEV:-}" ]]; then
                    case ",$_devs," in
                        *",$KDEV,"*) ;;
                        *) continue ;;
                    esac
                fi
                if [[ "$_devs" == "$ALL_DEVS" ]]; then
                    _tag="$KFAMILY"
                else
                    _tag="${KFAMILY}_$(echo "$_devs" | tr ',' '-')"
                fi
                _gen="${_devs%%,*}"
                KERNEL_GROUPS+=("$_tag|$_gen")
            done <<< "$GROUP_LINES"
            fi
            if [[ ${#KERNEL_GROUPS[@]} -eq 0 ]]; then
                echo "ERROR: no kernel group contains device '${KDEV:-?}'."
                fox_safe_exit 2
            fi
            export FOX_KERNEL_VER
            # Pre-generate for the FIRST group BEFORE lunch: dumpvars parses
            # BoardConfig.mk during lunch and would hit the empty-cmdline
            # guard otherwise. The per-group loop regenerates before each
            # mka (make re-reads BoardConfig every invocation).
            if [[ "$SAFE_EXIT_REQUESTED" == false && ${#KERNEL_GROUPS[@]} -gt 0 ]]; then
                _pre_gen="${KERNEL_GROUPS[0]#*|}"
                python3 "$SCRIPT_DIR/include/prebuilt/gen_kernel_mk.py" --generate \
                    "$KFAMILY" "${_pre_gen:-"-"}" "$FOX_KERNEL_VER" "$KERNEL_MK" \
                    || fox_safe_exit 2
            fi
        fi
    fi
fi
if [[ "$SAFE_EXIT_REQUESTED" == true ]]; then
    if [[ "$fox_sourced" == true ]]; then
        return "$SAFE_EXIT_CODE"
    else
        exit "$SAFE_EXIT_CODE"
    fi
fi

if [[ "$FAMILY" == "aio" ]]; then
    # All-in-one: both KeyMint HALs ship in one cpio (see device.mk);
    # the wrong one exits harmlessly at runtime.
    FOX_KEYMINT_TYPE="both"
    export FOX_KEYMINT_TYPE
elif [[ -n "$FAMILY" ]]; then
    # KeyMint HAL type for device.mk (config-parse env, like DEVICE_BUILD_FLAG):
    # families/<fam>/family.json `keymint` (rust|cpp). Loud fail — without it
    # device.mk falls back to the family-name mapping, which must never happen
    # silently on a -f build.
    FOX_KEYMINT_TYPE="$(python3 -c "import json; print(json.load(open('$SCRIPT_DIR/families/$FAMILY/family.json')).get('keymint',''))" 2>/dev/null)"
    case "$FOX_KEYMINT_TYPE" in
        rust|cpp) export FOX_KEYMINT_TYPE ;;
        *) echo "ERROR: families/$FAMILY/family.json needs keymint 'rust' or 'cpp'"; fox_safe_exit 2 ;;
    esac
fi
if [[ "$SAFE_EXIT_REQUESTED" == true ]]; then
    if [[ "$fox_sourced" == true ]]; then
        return "$SAFE_EXIT_CODE"
    else
        exit "$SAFE_EXIT_CODE"
    fi
fi

echo "=============================================="
echo "  OrangeFox Recovery Build Script"
echo "=============================================="
echo "  Source root:   $SOURCE_ROOT"
echo "  Family:        ${FAMILY:-<interactive>}"
echo "  Kernel:        ${FOX_KERNEL_VER:-<legacy default>}"
if [[ -n "$FAMILY" ]]; then
echo "  Keymint:       $FOX_KEYMINT_TYPE (from family.json)"
fi
if [[ ${#KERNEL_GROUPS[@]} -gt 0 ]]; then
    echo "  Groups:        $(printf '%s ' "${KERNEL_GROUPS[@]%%|*}")"
fi
echo "  Name:          ${BUILD_NAME:-<auto>}"
if [[ -n "${FOX_TAG_NAME:-}" ]]; then
echo "  Git tag:       $FOX_TAG_NAME"
fi
echo "  Patch Version: ${FOX_MAINTAINER_PATCH_VERSION:-<not set>}"
echo "  First-stage:   ${FOX_NO_FIRST_STAGE:+skipped (no-first-stage)}${FOX_NO_FIRST_STAGE:-included}"
echo "  Artifacts:     $([[ "$CPIO_ONLY" == true ]] && echo "cpio.lz4 only" || echo ".img/.zip")"
echo "  Clean:         $CLEAN"
echo "  Jobs:          $JOBS"
echo "=============================================="

cd "$SOURCE_ROOT"

# Apply maintainer micro-patches (PEP format: patches/files/{modified,original,new,patches}).
# Fails the build on conflict so a stale patch never ships silently.
python3 "$SCRIPT_DIR/patches/apply_patches.py" --apply --root "$SOURCE_ROOT" \
    || fox_safe_exit 1
if [[ "$SAFE_EXIT_REQUESTED" == true ]]; then
    if [[ "$fox_sourced" == true ]]; then
        return "$SAFE_EXIT_CODE"
    else
        exit "$SAFE_EXIT_CODE"
    fi
fi

if [ ! -f "external/guava/Android.bp" ] || [ ! -f "external/gflags/Android.bp" ]; then
    if ! repo sync -c -d --force-sync external/gflags external/guava; then
        echo "ERROR: repo sync failed. Please check your network connection and try again."
        fox_safe_exit 1
    fi
fi
if [[ "$SAFE_EXIT_REQUESTED" == true ]]; then
    if [[ "$fox_sourced" == true ]]; then
        return "$SAFE_EXIT_CODE"
    else
        exit "$SAFE_EXIT_CODE"
    fi
fi

PRODUCT_OUT="out/target/product/pixels"

if [[ -n "$FAMILY" ]]; then
    export DEVICE_BUILD_FLAG="$FAMILY"
    echo "[build] Pre-set DEVICE_BUILD_FLAG=$FAMILY"
fi

# Legacy path (no -f): the family is picked in the vendorsetup menu DURING
# lunch, so KERNEL_MK is still empty here — but dumpvars parses BoardConfig.mk
# during lunch and would hit the empty-cmdline guard for ANY family. Pre-generate
# default profiles for every family now (cheap python, no build); the post-lunch
# fallback below narrows to the selected family (honoring -k), and the end-of-build
# cleanup removes all generated files so no stale override survives.
if [[ -z "$KERNEL_MK" ]]; then
    for _fam_json in "$SCRIPT_DIR"/families/*/family.json; do
        _fam=$(basename "$(dirname "$_fam_json")")
        [ "$_fam" = "common" ] && continue
        _def=$(python3 -c "import json;print(json.load(open('$_fam_json')).get('default_kernel',''))" 2>/dev/null)
        [ -n "$_def" ] || continue
        python3 "$SCRIPT_DIR/include/prebuilt/gen_kernel_mk.py" --generate "$_fam" "-" "$_def" "$SCRIPT_DIR/families/$_fam/.gen_kernel.mk" >/dev/null 2>&1 || true
    done
    echo "[build] Pre-generated default kernel profiles for lunch (family selected interactively)"
fi

# Hard guarantee before lunch: dumpvars parses BoardConfig.mk during lunch
# and aborts the whole build on empty VENDOR_CMDLINE — and a failed lunch
# must never reach the destructive product-out clean below. Refuse early
# with an actionable message instead of cryptic dumpvars spam.
if [[ -n "${KERNEL_MK:-}" ]]; then
    if [[ ! -s "$KERNEL_MK" ]] || ! grep -q '^VENDOR_CMDLINE := "[^"]' "$KERNEL_MK"; then
        echo "ERROR: kernel profile missing/empty: $KERNEL_MK"
        echo "  Regenerate: python3 $SCRIPT_DIR/include/prebuilt/gen_kernel_mk.py --generate $KFAMILY ${KDEV:--} ${FOX_KERNEL_VER:-VER} $KERNEL_MK"
        fox_safe_exit 2
    fi
    if [[ "$SAFE_EXIT_REQUESTED" == true ]]; then
        if [[ "$fox_sourced" == true ]]; then
            return "$SAFE_EXIT_CODE"
        else
            exit "$SAFE_EXIT_CODE"
        fi
    fi
fi

echo "[build] Sourcing build/envsetup.sh ..."
set +e
source build/envsetup.sh

echo "[build] Running lunch twrp_pixels-ap2a-eng ..."
lunch twrp_pixels-ap2a-eng || fox_safe_exit $?
if [[ "$fox_sourced" != true ]]; then
    set -eo pipefail
fi
if [[ "$SAFE_EXIT_REQUESTED" == true ]]; then
    if [[ "$fox_sourced" == true ]]; then
        return "$SAFE_EXIT_CODE"
    else
        exit "$SAFE_EXIT_CODE"
    fi
fi

echo "[build] DEVICE_BUILD_FLAG=${DEVICE_BUILD_FLAG:-<not set>}"

# Legacy fallback (no -f): the vendorsetup menu picked the family during lunch,
# so KERNEL_MK is still empty. Resolve the kernel profile now: explicit -k wins
# (validated, loud on unknown), otherwise the family default with a loud notice
# (this is the interactive path; scripts/CI must pass -f/-k). Mirrors the -f
# resolution above, minus the device filter and the tty picker.
# AIO never enters here (stock kernel kept, no profiles exist).
if [[ -z "$KERNEL_MK" && -n "${DEVICE_BUILD_FLAG:-}" && "${DEVICE_BUILD_FLAG:-}" != "aio" ]]; then
    KFAMILY="$DEVICE_BUILD_FLAG"
    KLIST_RAW=$(python3 "$SCRIPT_DIR/include/prebuilt/gen_kernel_mk.py" --list "$KFAMILY" 2>&1) || {
        echo "ERROR: kernel profile list crashed for $KFAMILY (not 'no kernels' — the script itself failed):"
        echo "$KLIST_RAW" >&2
        fox_safe_exit 2
    }
    if [[ "$SAFE_EXIT_REQUESTED" == false ]]; then
        if [[ -z "$KLIST_RAW" || "$KLIST_RAW" == "(none)"* ]]; then
            echo "ERROR: no kernel profiles for family '$KFAMILY' (WIP?)."
            fox_safe_exit 2
        fi
    fi
    if [[ "$SAFE_EXIT_REQUESTED" == false ]]; then
        KDEFAULT=$(echo "$KLIST_RAW" | sed -n 's/.*(default: \([^)]*\)).*/\1/p')
        if [[ -n "$KERNEL_VER" ]]; then
            FOX_KERNEL_VER="$KERNEL_VER"
        elif [[ -n "$KDEFAULT" ]]; then
            FOX_KERNEL_VER="$KDEFAULT"
            echo "[build] No -f/-k: using default kernel profile $FOX_KERNEL_VER for $KFAMILY (override with -k VER)"
        else
            echo "ERROR: no default kernel for $KFAMILY and no -k given."
            fox_safe_exit 2
        fi
    fi
    if [[ "$SAFE_EXIT_REQUESTED" == false ]]; then
        FP_JSON=$(python3 "$SCRIPT_DIR/include/prebuilt/gen_kernel_mk.py" --fingerprint "$KFAMILY" "$FOX_KERNEL_VER" 2>&1) || {
            echo "$FP_JSON" >&2
            fox_safe_exit 2
        }
    fi
    if [[ "$SAFE_EXIT_REQUESTED" == false ]]; then
        KERNEL_MK="$SCRIPT_DIR/families/$KFAMILY/.gen_kernel.mk"
        if [[ "$FP_JSON" == "[]" ]]; then
            # Family without pixel.json devices: single group from family profile.
            KERNEL_GROUPS=("$KFAMILY|-")
        else
            GROUP_LINES=$(FP_JSON="$FP_JSON" python3 -c '
import json, os
fps = json.loads(os.environ["FP_JSON"])
byhash = {}
for e in fps:
    byhash.setdefault(e["hash"], []).append(e["device"])
for h in sorted(byhash):
    print(h + "|" + ",".join(sorted(byhash[h])))' 2>/dev/null) || GROUP_LINES=""
            ALL_DEVS=$(FP_JSON="$FP_JSON" python3 -c '
import json, os
print(",".join(sorted(e["device"] for e in json.loads(os.environ["FP_JSON"]))))' 2>/dev/null) || ALL_DEVS=""
            while IFS='|' read -r _h _devs; do
                [ -n "$_devs" ] || continue
                if [[ "$_devs" == "$ALL_DEVS" ]]; then
                    _tag="$KFAMILY"
                else
                    _tag="${KFAMILY}_$(echo "$_devs" | tr ',' '-')"
                fi
                _gen="${_devs%%,*}"
                KERNEL_GROUPS+=("$_tag|$_gen")
            done <<< "$GROUP_LINES"
        fi
        if [[ ${#KERNEL_GROUPS[@]} -eq 0 ]]; then
            echo "ERROR: no kernel groups for family '$KFAMILY'."
            fox_safe_exit 2
        fi
        export FOX_KERNEL_VER
        _pre_gen="${KERNEL_GROUPS[0]#*|}"
        python3 "$SCRIPT_DIR/include/prebuilt/gen_kernel_mk.py" --generate \
            "$KFAMILY" "${_pre_gen:-"-"}" "$FOX_KERNEL_VER" "$KERNEL_MK" \
            || fox_safe_exit 2
        # KeyMint HAL type for device.mk (same contract as the -f path above).
        FOX_KEYMINT_TYPE="$(python3 -c "import json; print(json.load(open('$SCRIPT_DIR/families/$KFAMILY/family.json')).get('keymint',''))" 2>/dev/null)"
        case "$FOX_KEYMINT_TYPE" in
            rust|cpp) export FOX_KEYMINT_TYPE ;;
            *) echo "ERROR: families/$KFAMILY/family.json needs keymint 'rust' or 'cpp'"; fox_safe_exit 2 ;;
        esac
    fi
    if [[ "$SAFE_EXIT_REQUESTED" == true ]]; then
        if [[ "$fox_sourced" == true ]]; then
            return "$SAFE_EXIT_CODE"
        else
            exit "$SAFE_EXIT_CODE"
        fi
    fi
fi

BUILD_TARGETS="adbd vendorbootimage"

# gs101 needs no special build-mode switch: family.mk already merges
# first-stage + recovery into a single platform fragment (no dtb/dlkm
# fragments, no modules inside). Post-processing below extracts that
# fragment for `fastboot flash vendor_boot: <ramdisk>` (empty name =
# platform; NOT :default, which would collapse the table and kill the
# stock dlkm fragment) after the image is copied.

# KeyMint HAL module must be built explicitly: vendorbootimage does not pull
# vendor/bin/hw binaries on its own (ninja graph entries exist via
# PRODUCT_PACKAGES but intermediates never compile, and the recovery callback
# finds nothing to copy). FOX_KEYMINT_TYPE arrives from family.json (see
# above); manual lunch without build.sh falls back to the family-name mapping
# (same default as device.mk).
case "${FOX_KEYMINT_TYPE:-}" in
    both)
        KEYMINT_MODULE="android.hardware.security.keymint-service.trusty android.hardware.security.keymint-service.rust.trusty" ;;
    cpp) KEYMINT_MODULE="android.hardware.security.keymint-service.trusty" ;;
    rust) KEYMINT_MODULE="android.hardware.security.keymint-service.rust.trusty" ;;
    *)
        if [[ "${DEVICE_BUILD_FLAG:-}" == "gs201" || "${DEVICE_BUILD_FLAG:-}" == "gs101" ]]; then
            KEYMINT_MODULE="android.hardware.security.keymint-service.trusty"
        else
            KEYMINT_MODULE="android.hardware.security.keymint-service.rust.trusty"
        fi
        ;;
esac
BUILD_TARGETS="$BUILD_TARGETS $KEYMINT_MODULE"
echo "[build] Keymint module: $KEYMINT_MODULE"

echo "=============================================="
echo "  Build targets: $BUILD_TARGETS"
echo "  Parallelism:   -j$JOBS"
echo "=============================================="

# --- Per-group build loop ---
# Legacy path (no kernel profiles): exactly one iteration with an empty tag.
# Kernel path: one iteration per config group; the product out is cleaned
# between groups (mandatory — the generated .gen_kernel.mk changes), and
# each group ships under its own tag (zuma.img / zuma_husky.img).
if [[ ${#KERNEL_GROUPS[@]} -eq 0 ]]; then
    KERNEL_GROUPS=("|")
fi
GROUP_FIRST=true
for GROUP_ENTRY in "${KERNEL_GROUPS[@]}"; do
    GROUP_TAG="${GROUP_ENTRY%%|*}"
    GROUP_GEN="${GROUP_ENTRY#*|}"
    if [[ -n "$KERNEL_MK" ]]; then
        python3 "$SCRIPT_DIR/include/prebuilt/gen_kernel_mk.py" --generate \
            "$KFAMILY" "${GROUP_GEN:-"-"}" "$FOX_KERNEL_VER" "$KERNEL_MK" \
            || fox_safe_exit 2
        if [[ "$SAFE_EXIT_REQUESTED" == true ]]; then
            break
        fi
    fi
    if [[ "$GROUP_FIRST" == true ]]; then
        GROUP_FIRST=false
        if [[ "$CLEAN" == "true" && -d "$PRODUCT_OUT" ]]; then
            echo "[build] Cleaning $PRODUCT_OUT ..."
            rm -rf "$PRODUCT_OUT"
            echo "[build] Clean done."
        fi
    else
        echo "[build] Cleaning $PRODUCT_OUT between kernel groups (mandatory) ..."
        rm -rf "$PRODUCT_OUT"
        echo "[build] Clean done."
    fi
    if [[ -n "$GROUP_TAG" ]]; then
        if [[ "$GROUP_TAG" == "$KFAMILY" ]]; then
            echo "[build] === Group $GROUP_TAG (kernel $FOX_KERNEL_VER, shared family profile) ==="
        else
            echo "[build] === Group $GROUP_TAG (kernel $FOX_KERNEL_VER, profile of $GROUP_GEN) ==="
        fi
    fi

    # Refresh OrangeFox env snapshot for the post-image hook (Fox_After_*).
    # The hook script (vendor/recovery/OrangeFox_A14.sh) reads OUT/FOX_BUILD_TYPE
    # from inherited env or /tmp/pixels/fox_env.sh, which lunch writes ONCE via
    # printconfig. If the file is gone (/tmp volatility, manual cleanup), group
    # 2+ recipes run with empty OUT (artifacts at /...) and default Unofficial
    # type. Re-materialize it from the current (post-lunch, proven-good) env
    # before every group; fail fast if the shell itself lost the critical vars.
    if [[ -z "${OUT:-}" || -z "${FOX_BUILD_TYPE:-}" ]]; then
        echo "ERROR: build env lost OUT/FOX_BUILD_TYPE before group build — re-run lunch"
        fox_safe_exit 2
    fi
    if [[ "$SAFE_EXIT_REQUESTED" == false ]]; then
        _fox_dev=$(cut -d'_' -f2 <<<"${TARGET_PRODUCT:-twrp_pixels}")
        mkdir -p "/tmp/$_fox_dev"
        export > "/tmp/$_fox_dev/fox_env.sh"
        echo "[build] Refreshed /tmp/$_fox_dev/fox_env.sh for post-image hook"
    fi
    if [[ "$SAFE_EXIT_REQUESTED" == true ]]; then
        break
    fi

    # KeyMint HAL first, alone: the recovery callback copies the vendor output
    # during vendorbootimage assembly and parallel ninja gives no ordering
    # guarantee. The follow-up full mka reuses it (no-op) and builds the rest.
    echo "[build] Building keymint HAL first ($KEYMINT_MODULE) ..."
    mka "$KEYMINT_MODULE" -j"$JOBS" || fox_safe_exit $?
    if [[ "$SAFE_EXIT_REQUESTED" == true ]]; then
        break
    fi
    mka $BUILD_TARGETS -j"$JOBS" || fox_safe_exit $?
    if [[ "$SAFE_EXIT_REQUESTED" == true ]]; then
        break
    fi
    # Success marker for the push/admin gates below: set HERE, right after
    # the build (a failed mka breaks out above before reaching this). It
    # must precede the Telegram push block — the old position after it
    # meant the gate never saw success and every push was skipped.
    FOX_BUILD_OK=1

    echo ""
    echo "=============================================="
    echo "  Build complete!"
    echo "  Output: $SOURCE_ROOT/$PRODUCT_OUT/"
    echo "=============================================="

    # --- Dynamic Artifact Naming & Copying ---
    BUILDS_DIR="$SOURCE_ROOT/builds"
    mkdir -p "$BUILDS_DIR"

    LATEST_IMG=$(find "$SOURCE_ROOT/$PRODUCT_OUT" -maxdepth 1 -name 'OrangeFox-*.img' -printf '%T@ %p\n' 2>/dev/null | sort -rn | head -1 | cut -d' ' -f2-)
    LATEST_ZIP=$(find "$SOURCE_ROOT/$PRODUCT_OUT" -maxdepth 1 -name 'OrangeFox-*.zip' -printf '%T@ %p\n' 2>/dev/null | sort -rn | head -1 | cut -d' ' -f2-)

    if [[ -n "$GROUP_TAG" ]]; then
        FAMILY_TAG="$GROUP_TAG"
    else
        FAMILY_TAG="${DEVICE_BUILD_FLAG:-unknown}"
    fi

    # Extract dynamic version prefix (e.g., "OrangeFox-R11.3" or "OrangeFox-R11.4_5")
    if [[ -n "$LATEST_IMG" ]]; then
        OFOX_PREFIX=$(basename "$LATEST_IMG" | cut -d'-' -f1,2)
    elif [[ -n "$LATEST_ZIP" ]]; then
        OFOX_PREFIX=$(basename "$LATEST_ZIP" | cut -d'-' -f1,2)
    else
        OFOX_PREFIX="OrangeFox-UnknownVersion"
    fi

    # Construct final filenames
    if [[ -n "$BUILD_NAME" ]]; then
        IMG_DEST="$BUILDS_DIR/${OFOX_PREFIX}-${BUILD_NAME}-${FAMILY_TAG}.img"
        ZIP_DEST="$BUILDS_DIR/${OFOX_PREFIX}-${BUILD_NAME}-${FAMILY_TAG}.zip"
    else
        IMG_DEST="$BUILDS_DIR/${OFOX_PREFIX}-${FAMILY_TAG}.img"
        ZIP_DEST="$BUILDS_DIR/${OFOX_PREFIX}-${FAMILY_TAG}.zip"
    fi

    # --- Artifact delivery: full image vs cpio-only ---
    # --cpio-only: .img/.zip are NOT copied to builds/; only the ramdisk
    # cpio.lz4 (extracted below) is delivered for fragment flashing.
    if [[ "$CPIO_ONLY" == true ]]; then
        echo "[build] --cpio-only: skipping .img/.zip copy"
    else
        # Copy files
        if [[ -n "$LATEST_IMG" ]]; then
            cp "$LATEST_IMG" "$IMG_DEST"
            echo "[build] Copied: $IMG_DEST"
        fi
        if [[ -n "$LATEST_ZIP" ]]; then
            cp "$LATEST_ZIP" "$ZIP_DEST"
            echo "[build] Copied: $ZIP_DEST"
        fi

        if [[ -z "$LATEST_IMG" && -z "$LATEST_ZIP" ]]; then
            echo "[build] WARNING: No OrangeFox artifacts found in $PRODUCT_OUT"
        fi
    fi

    # --- Ramdisk cpio extract (gs101 always; --platform-recovery always;
    # other families on --cpio-only) ---
    # Tensor G1 has no vendor_kernel_boot partition; the device keeps its own
    # dtb + dlkm + bootloader, we replace only the platform fragment (empty
    # name in the table). Other families replace only the recovery fragment.
    # --platform-recovery (var2-AIO test layout) merges first-stage into the
    # payload, so the payload cpio is needed for our universal installer
    # even without --cpio-only (full .img/.zip are still copied above).
    # Host fastboot fetches the on-device vendor_boot, swaps that one entry
    # and flashes back — so testers need just the ramdisk in stock
    # lz4_legacy format, not the whole image.
    #   gs101 → vendor_ramdisk/ramdisk.cpio,  flash: fastboot flash vendor_boot:
    #   other → vendor_ramdisk/recovery.cpio, flash: fastboot flash vendor_boot:recovery
    if [[ -n "$LATEST_IMG" ]] && [[ "${DEVICE_BUILD_FLAG:-}" == "gs101" || "$CPIO_ONLY" == true || -n "${FOX_RECOVERY_IN_PLATFORM:-}" ]]; then
        if [[ "${DEVICE_BUILD_FLAG:-}" == "gs101" || -n "${FOX_RECOVERY_IN_PLATFORM:-}" ]]; then
            FRAG_SRC="vendor_ramdisk/ramdisk.cpio"
            FRAG_FLASH="fastboot flash vendor_boot:"
        else
            FRAG_SRC="vendor_ramdisk/recovery.cpio"
            FRAG_FLASH="fastboot flash vendor_boot:recovery"
        fi
        FRAG_ALT="vendor_ramdisk_recovery.cpio"
        GS101_WORK="$SOURCE_ROOT/$PRODUCT_OUT/gs101_ramdisk"
        rm -rf "$GS101_WORK" && mkdir -p "$GS101_WORK"
        MAGISKBOOT_BIN="$SOURCE_ROOT/vendor/recovery/tools/magiskboot"
        if [[ ! -x "$MAGISKBOOT_BIN" ]]; then
            echo "[build] WARNING: magiskboot missing, skipping ramdisk extract"
        else
            "$MAGISKBOOT_BIN" unpack -h "$LATEST_IMG" | grep -E "VND_RAMDISK|DTB_SZ" || true
            (cd "$GS101_WORK" && "$MAGISKBOOT_BIN" unpack "$LATEST_IMG" >/dev/null 2>&1)
            # magiskboot versions disagree on the recovery fragment path.
            if [[ ! -f "$GS101_WORK/$FRAG_SRC" && -f "$GS101_WORK/$FRAG_ALT" ]]; then
                FRAG_SRC="$FRAG_ALT"
            fi
            if [[ -f "$GS101_WORK/$FRAG_SRC" ]]; then
                # magiskboot unpacks fragments decompressed; recompress to the
                # stock lz4_legacy format for the fragment flash path.
                lz4 -l -9 -f "$GS101_WORK/$FRAG_SRC" "$GS101_WORK/vendor_ramdisk.cpio.lz4"
                RAMDISK_DEST="${IMG_DEST%.img}.ramdisk.lz4"
                cp "$GS101_WORK/vendor_ramdisk.cpio.lz4" "$RAMDISK_DEST"
                echo "[build] ramdisk cpio: $RAMDISK_DEST (flash: $FRAG_FLASH $RAMDISK_DEST)"
                if [[ "$CPIO_ONLY" == true ]]; then
                    md5sum "$RAMDISK_DEST"
                else
                    md5sum "$RAMDISK_DEST" "$IMG_DEST"
                fi
            else
                echo "[build] WARNING: no $FRAG_SRC in $LATEST_IMG, skipping ramdisk extract"
            fi
        fi
        rm -rf "$GS101_WORK"
    fi

    # --- Installer zip (AIO): pack the fresh payload via installer/ ---
    # The universal installer bundles the just-built ramdisk payload;
    # OFOX_PAYLOAD refreshes installer/ + export.txt RECOVERY_IMG so the
    # tree tracks the latest payload. Tag/family mirror the ramdisk
    # artifact naming above. Non-fatal: a pack failure must never fail
    # the build (the payload itself is already delivered).
    if [[ "$FAMILY_TAG" == "aio" && -n "${RAMDISK_DEST:-}" && -f "$RAMDISK_DEST" ]]; then
        echo "[build] packing installer zip for $FAMILY_TAG ..."
        OFOX_PAYLOAD="$RAMDISK_DEST" \
        OFOX_NAME="OrangeFox" \
        OFOX_TYPE="$(echo "$OFOX_PREFIX" | cut -d'-' -f2)" \
        OFOX_TAG="${BUILD_NAME:-Beta}" \
        OFOX_FAMILY="$FAMILY_TAG" \
        OFOX_BUILDS_DIR="$BUILDS_DIR" \
        bash "$SCRIPT_DIR/installer/pack-module-zip.sh" \
        || echo "[build] WARNING: installer pack failed (payload is fine: $RAMDISK_DEST)"
    fi
done

# --- Telegram push (AIO installer zip only, --push GROUP) ---
# Non-fatal by design: network/API flakes must never fail a good build.
# Guarded on the post-process vars: if the pack loop never ran (failed or
# skipped build), there is no zip to push and no garbage path is built.
# Hard gate on FOX_BUILD_OK (set only after successful artifacts): without
# it a stale zip from a previous run would pass the -f check below and go
# to testers as if it were fresh (seen live: ninja failed, tag removed,
# yet the old test9 zip was pushed to all groups).
if [[ "${FOX_BUILD_OK:-}" != 1 ]]; then
    if [[ -n "${FOX_PUSH_GROUP:-}" ]]; then
        echo "[build] Push SKIPPED: build did not complete (no fresh zip; stale artifacts left untouched)"
    fi
elif [[ -n "${FOX_PUSH_GROUP:-}" && -n "${BUILDS_DIR:-}" && -n "${OFOX_PREFIX:-}" ]]; then
    _push_zip="$BUILDS_DIR/OrangeFox-$(echo "$OFOX_PREFIX" | cut -d'-' -f2)-${BUILD_NAME:-Beta}-aio.zip"
    # -D/--diff-tag: range = previous reachable tag .. fresh -g tag (or
    # HEAD when built without -g). A fresh tag sits exactly on HEAD, so
    # step past it to find the previous one. Warns (zip-only) when the
    # history has no earlier tag.
    # --diff-from TAG: forced range TAG..HEAD (fresh -g tag == HEAD, so
    # this covers up to the tag being formed). Overrides -D. TAG must
    # exist, else warning + zip-only.
    _diff_args=()
    if [[ -n "${FOX_DIFF_FROM:-}" ]]; then
        if git -C "$SCRIPT_DIR" rev-parse --verify --quiet "$FOX_DIFF_FROM^{commit}" >/dev/null 2>&1; then
            _diff_args=(--diff-from "$FOX_DIFF_FROM")
            echo "[build] Change list (forced base): $FOX_DIFF_FROM..HEAD"
        else
            echo "[build] WARNING: --diff-from tag not found: $FOX_DIFF_FROM, sending zip only"
        fi
    elif [[ -n "${FOX_DIFF_TAG:-}" ]]; then
        _diff_base="HEAD"
        _diff_cur="HEAD"
        if [[ -n "${FOX_TAG_NAME:-}" ]]; then
            _diff_base="HEAD~1"
            _diff_cur="$FOX_TAG_NAME"
        fi
        _prev_tag=$(git -C "$SCRIPT_DIR" describe --tags --abbrev=0 "$_diff_base" 2>/dev/null || true)
        if [[ -n "$_prev_tag" ]]; then
            _diff_args=(--diff "$_prev_tag..$_diff_cur")
            echo "[build] Change list: $_prev_tag..$_diff_cur"
        else
            echo "[build] WARNING: --diff-tag requested but no previous tag reachable, sending zip only"
        fi
        unset _diff_base _diff_cur _prev_tag
    fi
    # -T/--text: postscript after the md5 line of the zip message.
    _text_args=()
    if [[ -n "${FOX_PUSH_TEXT:-}" ]]; then
        _text_args=(--text "$FOX_PUSH_TEXT")
    fi
    if [[ -f "$_push_zip" ]]; then
        python3 "$SCRIPT_DIR/tools/tg_push.py" "$_push_zip" "$FOX_PUSH_GROUP" "${_diff_args[@]}" "${_text_args[@]}" \
        || echo "[build] WARNING: telegram push failed (zip is fine: $_push_zip)"
    else
        echo "[build] WARNING: --push requested but no AIO zip at $_push_zip (aio pack skipped?)"
    fi
    unset _push_zip _diff_args _text_args
fi
# Build-OK admin ping (text-only, never fatal): one line per finished
# build so completions and failures are equally visible in DMs.
if [[ "${FOX_BUILD_OK:-}" == 1 && "${FOX_ADMIN_NOTIFIED:-}" != 1 ]]; then
    FOX_ADMIN_NOTIFIED=1
    if [[ -n "${FOX_PUSH_GROUP:-}" ]]; then
        _admin_notify "OrangeFox build OK: ${BUILD_NAME:-?} pushed to ${FOX_PUSH_GROUP}" || true
    else
        _admin_notify "OrangeFox build OK: ${BUILD_NAME:-?} (no --push, artifacts in builds/)" || true
    fi
fi
if [[ -n "${FOX_DIFF_TAG:-}${FOX_DIFF_FROM:-}${FOX_PUSH_TEXT:-}" && -z "${FOX_PUSH_GROUP:-}" ]]; then
    echo "[build] WARNING: --diff-tag/--diff-from/--text have no effect without --push"
fi

# Stale generated overrides would silently reconfigure later manual builds
# (dumpvars parses BoardConfig.mk on every lunch). Remove everything this
# script may have generated — the pre-lunch sweep covers all families.
_removed=$(rm -fv "$SCRIPT_DIR"/families/*/.gen_kernel.mk 2>/dev/null)
if [[ -n "$_removed" ]]; then
    echo "[build] Removed generated kernel profiles (reproducible via build.sh -k $FOX_KERNEL_VER)"
fi
if [[ "$SAFE_EXIT_REQUESTED" == true ]]; then
    if [[ "$fox_sourced" == true ]]; then
        return "$SAFE_EXIT_CODE"
    else
        exit "$SAFE_EXIT_CODE"
    fi
fi

echo "=============================================="
echo "  Artifacts in: $BUILDS_DIR/"
echo "=============================================="
# NOTE: FOX_BUILD_OK is set right after mka succeeds (line ~980), not
# here — the Telegram push/admin gates above must see it.