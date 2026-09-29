#!/bin/bash
#
# build.sh — OrangeFox Recovery build script (AIO-only).
#
# Concept: single universal installer for all Tensor Pixels. The stock
# kernel is always kept, only the recovery ramdisk cpio.lz4 payload is
# delivered and packed into the universal installer zip.
#
# Usage:
#   ./build.sh [--notrm] [-j N] [--name TAG] [--patch N] [--level 0-3]
#              [--list] [--build-type TYPE] [--new-theme]
#              [--push GROUP] [-g] [-D] [--diff-from TAG] [-T TEXT]
#   source ./build.sh [...]   # same, but runs in the current shell (env kept)
#
# Options:
#   -f, --family aio
#                     Accepted for backward compatibility only. The tree
#                     builds a single universal (aio) payload; any other
#                     value is rejected. May be omitted entirely.
#   --list            Print the family/device tree (families with keymint;
#                     devices with their family mapping) and exit.
#                     Read-only, builds nothing.
#   --build-type TYPE Build type string, default Stable. Only exact `Stable`
#                     enables OF_ADVANCED_SECURITY downstream (adbd stopped
#                     at boot, MTP autostart off); any other value builds a
#                     non-Secure image. Exported for vendorsetup.sh/lunch.
#   --notrm           Don't clean out/target/product/pixels before build.
#   -n, --name TAG    Name tag for output files:
#                     builds/OrangeFox-<VERSION>-{TAG}-aio.zip +
#                     OrangeFox-<VERSION>-{TAG}-aio.ramdisk.lz4
#   -p, --patch N     Set FOX_MAINTAINER_PATCH_VERSION (numbers only).
#                     E.g., "--patch 5" will result in version R11.3_5.
#   -l, --level N     LGZ cluster compression level 0-3 (default 0=fast).
#                     Exported as LGZ_LEVEL for fox_build_callback.sh.
#   -j N              Parallel build jobs (also as -jN). Default: nproc.
#                     Exported as JOBS for the Soong/make invocation.
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
#                     md5 / full-changes-link lines
#                     (e.g. -T "looks like OTG is fixed on P10").
#   -h, --help        Show this help.
#
# Removed in the AIO-only tree (rejected with an error, or accepted as a
# no-op for old command lines — see arg parsing below):
#   -k/--kernel, --force (per-family kernel profiles: gone, stock kept),
#   -c/--cpio-only, --platform-recovery, -N/--no-first-stage (now always on).
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
    if [[ "$SAFE_EXIT_CODE" != 0 ]]; then
        FOX_BUILD_OK=0
    fi
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

# --- Device/family inventory (build.sh --list) ---
# Tree view over families/*/family.json + devices/*/device.conf:
# per family keymint type, per device family mapping.
# Read-only: prints and returns, never builds.
fox_print_tree() {
    local fam_json fam km dev_conf dev dfam
    echo "SoC families (families/*/family.json) and devices (devices/*/device.conf):"
    echo ""
    echo "aio [universal payload: stock kernel kept, all devices in one cpio]"
    for fam_json in "$SCRIPT_DIR"/families/*/family.json; do
        fam=$(basename "$(dirname "$fam_json")")
        [ "$fam" = "common" ] && continue
        [ "$fam" = "aio" ] && continue
        km=$(python3 -c "import json,sys; print(json.load(open(sys.argv[1])).get('keymint','?'))" "$fam_json" 2>/dev/null) || km="?"
        echo "  $fam [keymint=$km]"
        local any_dev=false
        for dev_conf in "$SCRIPT_DIR"/devices/*/device.conf; do
            dev=$(basename "$(dirname "$dev_conf")")
            dfam=$(FAMILY=""; DEVICE=""; . "$dev_conf" 2>/dev/null; printf '%s' "$FAMILY")
            [ "$dfam" = "$fam" ] || continue
            any_dev=true
            echo "    $dev"
        done
        $any_dev || echo "    (no devices)"
    done
}

# --- Parse arguments (AIO-only) ---
FAMILY="aio"
CLEAN=true
JOBS="$(fox_nproc)"
BUILD_NAME=""
PATCH_VERSION=""
LGZ_LEVEL="0"
FOX_BUILD_TYPE="Stable"
FOX_GIT_TAG=""
FOX_DIFF_TAG=""
FOX_DIFF_FROM=""
FOX_PUSH_TEXT=""

while [[ $# -gt 0 ]] && [[ "$SAFE_EXIT_REQUESTED" == false ]]; do
    case "$1" in
        -f|--family)
            shift
            _fam_arg="${1:-}"
            if [[ -z "$_fam_arg" ]]; then
                echo "ERROR: --family requires an argument (only 'aio' is supported)"
                fox_safe_exit 1
            elif [[ "$_fam_arg" != "aio" ]]; then
                echo "ERROR: only '-f aio' is supported in this tree (got '$_fam_arg')."
                echo "  Per-family images were removed: the universal AIO payload covers all devices."
                fox_safe_exit 2
            fi
            FAMILY="aio"
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
        -k|--kernel|--force)
            echo "ERROR: '$1' was removed in the AIO-only tree (stock kernel is always kept, no kernel profiles)."
            fox_safe_exit 2
            ;;
        -N|--no-first-stage|--platform-recovery|-c|--cpio-only)
            echo "[build] NOTE: '$1' is always on in the AIO-only tree (ignored, kept for old command lines)."
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
            ;;
        -T|--text)
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
# Runs before anything mutates the tree (.build_platform.conf is generated
# at lunch). Push stays manual; a failed build deletes the tag
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
# AIO-only fixed layout (previously -N/--no-first-stage/--platform-recovery/-c):
# stock kernel is kept, only the recovery ramdisk cpio is delivered;
# first-stage + recovery are merged into one payload cpio, stock first_stage
# is preserved by the installer, so first-stage components are skipped and
# the merged first_stage copy is stripped by the callback.
export FOX_NO_FIRST_STAGE=1
export FOX_RECOVERY_IN_PLATFORM=1
echo "[build] AIO layout: cpio-only, recovery-in-platform, no first-stage (fixed)"
FOX_KEYMINT_TYPE="both"
export FOX_KEYMINT_TYPE
FOX_KERNEL_VER="stock"
export FOX_KERNEL_VER
echo "[build] AIO mode: stock kernel kept, both KeyMint HALs ship"

if [[ "$SAFE_EXIT_REQUESTED" == true ]]; then
    if [[ "$fox_sourced" == true ]]; then
        return "$SAFE_EXIT_CODE"
    else
        exit "$SAFE_EXIT_CODE"
    fi
fi

echo "=============================================="
echo "  OrangeFox Recovery Build Script (AIO-only)"
echo "=============================================="
echo "  Source root:   $SOURCE_ROOT"
echo "  Family:        aio (universal)"
echo "  Kernel:        stock (kept)"
echo "  Keymint:       both (universal payload)"
echo "  Name:          ${BUILD_NAME:-<auto>}"
if [[ -n "${FOX_TAG_NAME:-}" ]]; then
echo "  Git tag:       $FOX_TAG_NAME"
fi
echo "  Patch Version: ${FOX_MAINTAINER_PATCH_VERSION:-<not set>}"
echo "  Artifacts:     cpio.lz4 payload + installer zip"
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

export DEVICE_BUILD_FLAG="aio"
echo "[build] DEVICE_BUILD_FLAG=aio (fixed)"

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

# KeyMint HAL modules must be built explicitly: vendorbootimage does not pull
# vendor/bin/hw binaries on its own (ninja graph entries exist via
# PRODUCT_PACKAGES but intermediates never compile, and the recovery callback
# finds nothing to copy). AIO always ships both HALs; the wrong one exits
# harmlessly at runtime.
KEYMINT_MODULE="android.hardware.security.keymint-service.trusty android.hardware.security.keymint-service.rust.trusty"
BUILD_TARGETS="adbd vendorbootimage $KEYMINT_MODULE"
echo "[build] Keymint modules: $KEYMINT_MODULE"

echo "=============================================="
echo "  Build targets: $BUILD_TARGETS"
echo "  Parallelism:   -j$JOBS"
echo "=============================================="

if [[ "$CLEAN" == "true" && -d "$PRODUCT_OUT" ]]; then
    echo "[build] Cleaning $PRODUCT_OUT ..."
    rm -rf "$PRODUCT_OUT"
    echo "[build] Clean done."
fi

# Refresh OrangeFox env snapshot for the post-image hook (Fox_After_*).
# The hook script (vendor/recovery/OrangeFox_A14.sh) reads OUT/FOX_BUILD_TYPE
# from inherited env or /tmp/pixels/fox_env.sh, which lunch writes ONCE via
# printconfig. If the file is gone (/tmp volatility, manual cleanup),
# recipes run with empty OUT (artifacts at /...) and default Unofficial
# type. Re-materialize it from the current (post-lunch, proven-good) env
# before the build; fail fast if the shell itself lost the critical vars.
if [[ -z "${OUT:-}" || -z "${FOX_BUILD_TYPE:-}" ]]; then
    echo "ERROR: build env lost OUT/FOX_BUILD_TYPE before build — re-run lunch"
    fox_safe_exit 2
fi
if [[ "$SAFE_EXIT_REQUESTED" == false ]]; then
    _fox_dev=$(cut -d'_' -f2 <<<"${TARGET_PRODUCT:-twrp_pixels}")
    mkdir -p "/tmp/$_fox_dev"
    export > "/tmp/$_fox_dev/fox_env.sh"
    echo "[build] Refreshed /tmp/$_fox_dev/fox_env.sh for post-image hook"
fi
if [[ "$SAFE_EXIT_REQUESTED" == true ]]; then
    if [[ "$fox_sourced" == true ]]; then
        return "$SAFE_EXIT_CODE"
    else
        exit "$SAFE_EXIT_CODE"
    fi
fi

# KeyMint HAL first, alone: the recovery callback copies the vendor output
# during vendorbootimage assembly and parallel ninja gives no ordering
# guarantee. The follow-up full mka reuses it (no-op) and builds the rest.
echo "[build] Building keymint HALs first ($KEYMINT_MODULE) ..."
mka "$KEYMINT_MODULE" -j"$JOBS" || fox_safe_exit $?
if [[ "$SAFE_EXIT_REQUESTED" == true ]]; then
    if [[ "$fox_sourced" == true ]]; then
        return "$SAFE_EXIT_CODE"
    else
        exit "$SAFE_EXIT_CODE"
    fi
fi
mka $BUILD_TARGETS -j"$JOBS" || fox_safe_exit $?
if [[ "$SAFE_EXIT_REQUESTED" == true ]]; then
    if [[ "$fox_sourced" == true ]]; then
        return "$SAFE_EXIT_CODE"
    else
        exit "$SAFE_EXIT_CODE"
    fi
fi
# Success marker for the push/admin gates below: set HERE, right after
# the build (a failed mka breaks out above before reaching this).
FOX_BUILD_OK=1

echo ""
echo "=============================================="
echo "  Build complete!"
echo "  Output: $SOURCE_ROOT/$PRODUCT_OUT/"
echo "=============================================="

# --- Dynamic Artifact Naming & Copying (AIO-only: payload + installer) ---
BUILDS_DIR="$SOURCE_ROOT/builds"
mkdir -p "$BUILDS_DIR"

LATEST_IMG=$(find "$SOURCE_ROOT/$PRODUCT_OUT" -maxdepth 1 -name 'OrangeFox-*.img' -printf '%T@ %p\n' 2>/dev/null | sort -rn | head -1 | cut -d' ' -f2-)

FAMILY_TAG="aio"

# Extract dynamic version prefix (e.g., "OrangeFox-R11.3" or "OrangeFox-R11.4_5")
if [[ -n "$LATEST_IMG" ]]; then
    OFOX_PREFIX=$(basename "$LATEST_IMG" | cut -d'-' -f1,2)
else
    OFOX_PREFIX="OrangeFox-UnknownVersion"
fi

if [[ -z "$LATEST_IMG" ]]; then
    echo "[build] WARNING: No OrangeFox image found in $PRODUCT_OUT (cpio extract skipped)"
fi

# --- Ramdisk cpio extract (AIO recovery-in-platform: platform fragment) ---
# The payload merges first-stage + recovery into one cpio for the universal
# installer. Host fastboot fetches the on-device vendor_boot, swaps the
# platform entry and flashes back — testers need just the ramdisk in stock
# lz4_legacy format, not the whole image.
#   flash: fastboot flash vendor_boot: <payload>
if [[ -n "$LATEST_IMG" ]]; then
    FRAG_SRC="vendor_ramdisk/ramdisk.cpio"
    FRAG_FLASH="fastboot flash vendor_boot:"
    FRAG_ALT="vendor_ramdisk_recovery.cpio"
    AIO_WORK="$SOURCE_ROOT/$PRODUCT_OUT/aio_ramdisk"
    rm -rf "$AIO_WORK" && mkdir -p "$AIO_WORK"
    MAGISKBOOT_BIN="$SOURCE_ROOT/vendor/recovery/tools/magiskboot"
    if [[ ! -x "$MAGISKBOOT_BIN" ]]; then
        echo "[build] WARNING: magiskboot missing, skipping ramdisk extract"
    else
        "$MAGISKBOOT_BIN" unpack -h "$LATEST_IMG" | grep -E "VND_RAMDISK|DTB_SZ" || true
        (cd "$AIO_WORK" && "$MAGISKBOOT_BIN" unpack "$LATEST_IMG" >/dev/null 2>&1)
        # magiskboot versions disagree on the recovery fragment path.
        if [[ ! -f "$AIO_WORK/$FRAG_SRC" && -f "$AIO_WORK/$FRAG_ALT" ]]; then
            FRAG_SRC="$FRAG_ALT"
        fi
        if [[ -f "$AIO_WORK/$FRAG_SRC" ]]; then
            # magiskboot unpacks fragments decompressed; recompress to the
            # stock lz4_legacy format for the fragment flash path.
            lz4 -l -9 -f "$AIO_WORK/$FRAG_SRC" "$AIO_WORK/vendor_ramdisk.cpio.lz4"
            if [[ -n "$BUILD_NAME" ]]; then
                RAMDISK_DEST="$BUILDS_DIR/${OFOX_PREFIX}-${BUILD_NAME}-${FAMILY_TAG}.ramdisk.lz4"
            else
                RAMDISK_DEST="$BUILDS_DIR/${OFOX_PREFIX}-${FAMILY_TAG}.ramdisk.lz4"
            fi
            cp "$AIO_WORK/vendor_ramdisk.cpio.lz4" "$RAMDISK_DEST"
            echo "[build] ramdisk cpio: $RAMDISK_DEST (flash: $FRAG_FLASH $RAMDISK_DEST)"
            md5sum "$RAMDISK_DEST"
        else
            echo "[build] WARNING: no $FRAG_SRC in $LATEST_IMG, skipping ramdisk extract"
        fi
    fi
    rm -rf "$AIO_WORK"
fi

# --- Installer zip (AIO): pack the fresh payload via installer/ ---
# The universal installer bundles the just-built ramdisk payload;
# OFOX_PAYLOAD refreshes installer/ + export.txt RECOVERY_IMG so the
# tree tracks the latest payload. Non-fatal: a pack failure must never fail
# the build (the payload itself is already delivered).
if [[ -n "${RAMDISK_DEST:-}" && -f "$RAMDISK_DEST" ]]; then
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

# --- Telegram push (AIO installer zip only, --push GROUP) ---
# Non-fatal by design: network/API flakes must never fail a good build.
# Guarded on the post-process vars: if the build never ran (failed or
# skipped), there is no zip to push and no garbage path is built.
# Hard gate on FOX_BUILD_OK (set only after successful artifacts): without
# it a stale zip from a previous run would be pushed to testers as if it
# were fresh.
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

# Migration: drop stale per-family kernel overrides generated by the old
# per-family flow (dumpvars parsed BoardConfig.mk on every lunch). The
# AIO-only tree builds no kernel image, so these files must never exist.
_stale_gen=$(rm -fv "$SCRIPT_DIR"/families/*/.gen_kernel.mk 2>/dev/null || true)
if [[ -n "$_stale_gen" ]]; then
    echo "[build] Removed stale per-family kernel profiles (AIO keeps stock kernel):"
    echo "$_stale_gen" | sed 's/^/  /'
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
# NOTE: FOX_BUILD_OK is set right after mka succeeds (above), not
# here — the Telegram push/admin gates above must see it.
