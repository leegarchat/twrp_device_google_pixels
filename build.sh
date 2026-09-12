#!/bin/bash
#
# build.sh — OrangeFox Recovery build script for all Tensor Pixel devices.
#
# Usage:
#   ./build.sh [--family DEV|FAMILY] [--notrm] [-j N] [--name TAG] [--patch N]
#
# Options:
#   --family TARGET   Device codename (husky, shiba, ...) or SoC family
#                     (gs201/zuma/zumapro/gs101). Families and devices are
#                     discovered from families/ and devices/ — no hardcoded list.
#                     If omitted, vendorsetup.sh interactive menu is used.
#   --notrm           Don't clean out/target/product/pixels before build.
#   -j N              Parallelism for make (default: $(nproc)).
#   --name TAG        Name tag for output files. Copies final .img/.zip to
#                     builds/OrangeFox-<VERSION>-{TAG}-{family}.img/zip
#   --patch N         Set FOX_MAINTAINER_PATCH_VERSION (numbers only).
#                     E.g., "--patch 5" will result in version R11.3_5.

set -eo pipefail

for var in ${!FOX_@} ${!OF_@} ${!TARGET_@} ${!TW_@}; do     unset "$var"; done
unset DEVICE_BUILD_FLAG

# --- Resolve paths ---
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SOURCE_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"

# --- Parse arguments ---
FAMILY=""
CLEAN=true
JOBS="$(nproc)"
BUILD_NAME=""
PATCH_VERSION=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --family)
            shift
            FAMILY="${1:-}"
            if [[ -z "$FAMILY" ]]; then
                echo "ERROR: --family requires an argument (device or family)"
                exit 1
            fi
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
                exit 1
            fi
            shift
            ;;
        --notrm)
            CLEAN=false
            shift
            ;;
        -j)
            shift
            JOBS="${1:-$(nproc)}"
            shift
            ;;
        -j[0-9]*)
            JOBS="${1#-j}"
            shift
            ;;
        --name)
            shift
            BUILD_NAME="${1:-}"
            if [[ -z "$BUILD_NAME" ]]; then
                echo "ERROR: --name requires an argument"
                exit 1
            fi
            shift
            ;;
        --patch)
            shift
            PATCH_VERSION="${1:-}"
            if [[ -z "$PATCH_VERSION" || ! "$PATCH_VERSION" =~ ^[0-9]+$ ]]; then
                echo "ERROR: --patch requires a numeric argument (e.g., 5)"
                exit 1
            fi
            export FOX_MAINTAINER_PATCH_VERSION="$PATCH_VERSION"
            shift
            ;;
        -h|--help)
            sed -n '2,17p' "$0"
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

echo "=============================================="
echo "  OrangeFox Recovery Build Script"
echo "=============================================="
echo "  Source root:   $SOURCE_ROOT"
echo "  Family:        ${FAMILY:-<interactive>}"
echo "  Name:          ${BUILD_NAME:-<auto>}"
echo "  Patch Version: ${FOX_MAINTAINER_PATCH_VERSION:-<not set>}"
echo "  Clean:         $CLEAN"
echo "  Jobs:          $JOBS"
echo "=============================================="

cd "$SOURCE_ROOT"

# Apply maintainer micro-patches (PEP format: patches/files/{modified,original,new,patches}).
# Fails the build on conflict so a stale patch never ships silently.
python3 "$SCRIPT_DIR/patches/apply_patches.py" --apply --root "$SOURCE_ROOT"

if [ ! -f "external/guava/Android.bp" ] || [ ! -f "external/gflags/Android.bp" ]; then
    if ! repo sync -c -d --force-sync external/gflags external/guava; then
        echo "ERROR: repo sync failed. Please check your network connection and try again."
        exit 1
    fi
fi

PRODUCT_OUT="out/target/product/pixels"
if [[ "$CLEAN" == "true" && -d "$PRODUCT_OUT" ]]; then
    echo "[build] Cleaning $PRODUCT_OUT ..."
    rm -rf "$PRODUCT_OUT"
    echo "[build] Clean done."
fi

if [[ -n "$FAMILY" ]]; then
    export DEVICE_BUILD_FLAG="$FAMILY"
    echo "[build] Pre-set DEVICE_BUILD_FLAG=$FAMILY"
fi

echo "[build] Sourcing build/envsetup.sh ..."
set +e
source build/envsetup.sh

echo "[build] Running lunch twrp_pixels-ap2a-eng ..."
lunch twrp_pixels-ap2a-eng
set -eo pipefail

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

mka $BUILD_TARGETS -j"$JOBS"

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

FAMILY_TAG="${DEVICE_BUILD_FLAG:-unknown}"

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

echo "=============================================="
echo "  Artifacts in: $BUILDS_DIR/"
echo "=============================================="