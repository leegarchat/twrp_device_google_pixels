#!/bin/bash
#
# build.sh — OrangeFox Recovery build script for all Tensor Pixel devices.
#
# Usage:
#   ./build.sh [--family DEV|FAMILY] [--notrm] [-j N] [--name TAG] [--patch N] [--level 0-3]
#              [-k|--kernel VER] [--force]
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
#   --notrm           Don't clean out/target/product/pixels before build.
#                     (Between kernel-profile groups a clean is mandatory
#                     and always performed, with a notice.)
#   -n, --name TAG    Name tag for output files. Copies final .img/.zip to
#                     builds/OrangeFox-<VERSION>-{TAG}-{family}.img/zip
#   -p, --patch N     Set FOX_MAINTAINER_PATCH_VERSION (numbers only).
#                     E.g., "--patch 5" will result in version R11.3_5.
#   -l, --level N     LGZ cluster compression level 0-3 (default 0=fast).
#                     Exported as LGZ_LEVEL for fox_build_callback.sh.
#   -h, --help        Show this help.

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

# --- Parse arguments ---
FAMILY=""
CLEAN=true
JOBS="$(fox_nproc)"
BUILD_NAME=""
PATCH_VERSION=""
LGZ_LEVEL="0"
KERNEL_VER=""
FOX_FORCE=false

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
        -h|--help)
            sed -n '2,30p' "${BASH_SOURCE[0]}"
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
export LGZ_LEVEL

# --- Kernel profile resolution (families/*/family.json `kernels`) ---
# Engages only with a known family context (-f). Without -f the legacy
# path is kept (interactive vendorsetup owns device selection).
# Result: KERNEL_GROUPS entries "tag|gen_dev", KERNEL_MK path (or empty
# for the legacy path), FOX_KERNEL_VER exported for the callback.
KERNEL_GROUPS=()
KERNEL_MK=""
if [[ -n "$FAMILY" ]]; then
    KFAMILY="$FAMILY"
    KDEV="${_DEV:-}"
    if [[ -n "$KDEV" ]]; then
        KLIST_RAW=$(python3 "$SCRIPT_DIR/gen_kernel_mk.py" --list "$KFAMILY" "$KDEV" 2>/dev/null) || KLIST_RAW=""
    else
        KLIST_RAW=$(python3 "$SCRIPT_DIR/gen_kernel_mk.py" --list "$KFAMILY" 2>/dev/null) || KLIST_RAW=""
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
            FP_JSON=$(python3 "$SCRIPT_DIR/gen_kernel_mk.py" --fingerprint "$KFAMILY" "$FOX_KERNEL_VER" 2>&1) || {
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
                python3 "$SCRIPT_DIR/gen_kernel_mk.py" --generate \
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

echo "=============================================="
echo "  OrangeFox Recovery Build Script"
echo "=============================================="
echo "  Source root:   $SOURCE_ROOT"
echo "  Family:        ${FAMILY:-<interactive>}"
echo "  Kernel:        ${FOX_KERNEL_VER:-<legacy default>}"
if [[ ${#KERNEL_GROUPS[@]} -gt 0 ]]; then
    echo "  Groups:        $(printf '%s ' "${KERNEL_GROUPS[@]%%|*}")"
fi
echo "  Name:          ${BUILD_NAME:-<auto>}"
echo "  Patch Version: ${FOX_MAINTAINER_PATCH_VERSION:-<not set>}"
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

echo "[build] Sourcing build/envsetup.sh ..."
set +e
source build/envsetup.sh

echo "[build] Running lunch twrp_pixels-ap2a-eng ..."
lunch twrp_pixels-ap2a-eng
if [[ "$fox_sourced" != true ]]; then
    set -eo pipefail
fi

echo "[build] DEVICE_BUILD_FLAG=${DEVICE_BUILD_FLAG:-<not set>}"

BUILD_TARGETS="adbd vendorbootimage"

if [[ "${DEVICE_BUILD_FLAG:-}" == "gs201" || "${DEVICE_BUILD_FLAG:-}" == "gs101" ]]; then
    if [[ "${DEVICE_BUILD_FLAG:-}" == "gs101" ]]; then
        export VENDOR_BOOT_PATCH_STOCK=true
        echo "[build] gs101: stock vendor_boot patch mode (VENDOR_BOOT_PATCH_STOCK=true)"
    fi
    BUILD_TARGETS="$BUILD_TARGETS android.hardware.security.keymint-service.trusty"
    echo "[build] ${DEVICE_BUILD_FLAG}: adding keymint-service.trusty to build targets"
fi

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
        python3 "$SCRIPT_DIR/gen_kernel_mk.py" --generate \
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

    mka $BUILD_TARGETS -j"$JOBS" || fox_safe_exit $?
    if [[ "$SAFE_EXIT_REQUESTED" == true ]]; then
        break
    fi

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
done

# A stale generated override would silently reconfigure later manual builds.
if [[ -n "$KERNEL_MK" && -f "$KERNEL_MK" ]]; then
    rm -f "$KERNEL_MK"
    echo "[build] Removed generated $KERNEL_MK (reproducible via -k $FOX_KERNEL_VER)"
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