#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

DIST_DIR="$SCRIPT_DIR/dist"
PUSH_DIR="$SCRIPT_DIR/target/push"
BIN_NAME="recovery-tensor-daemon"

TARGET_X64="x86_64-unknown-linux-musl"
TARGET_X86="i686-unknown-linux-musl"
TARGET_ARM64="aarch64-unknown-linux-musl"
TARGET_ARM32="armv7-unknown-linux-musleabihf"

ALL_TARGETS=("$TARGET_X64" "$TARGET_X86" "$TARGET_ARM64" "$TARGET_ARM32")

# Default parameters
METHOD="auto"         # auto | cargo | cross
SELECTED_ARCH="all"   # all | x64 | x86 | arm64 | arm32

usage() {
    cat <<EOF
Usage:
  $0 [OPTIONS]

Build method options:
  --cargo          Force local cargo (requires system gcc linkers)
  --cross          Force cross (requires Docker or Podman)
  --auto           Auto-select: cross (if containers available), else cargo (default)

Architecture selection:
  --arch <type>    Build a specific architecture:
                     all   - All 4 platforms (default)
                     x64   - x86_64-unknown-linux-musl
                     x86   - i686-unknown-linux-musl
                     arm64 - aarch64-unknown-linux-musl
                     arm32 - armv7-unknown-linux-musleabihf
  -h, --help       Show this message

Examples:
  $0 --cargo --arch x64
  $0 --cross --arch arm64
  $0 --arch all

Outputs:
  dist/                      Static binaries recovery-tensor-daemon-linux-*
  target/push/               Push-ready copies {name}_{arch} (x64, x86,
                             arm64, arm32), e.g. for: adb push
                             target/push/recovery-tensor-daemon_arm64 /data/local/

Note:
  Host cargo builds compile WITHOUT the Binder feature (headless GSC mode).
  The recovery image build goes through Soong (Android.bp), which enables
  features: ["binder"] and links the AIDL/Binder rustlibs.
EOF
    exit 0
}

# CLI argument parsing
while [[ $# -gt 0 ]]; do
    case "$1" in
        --cargo)
            METHOD="cargo"
            shift
            ;;
        --cross)
            METHOD="cross"
            shift
            ;;
        --auto)
            METHOD="auto"
            shift
            ;;
        --arch)
            SELECTED_ARCH="${2:-}"
            if [[ -z "$SELECTED_ARCH" ]]; then
                echo "Error: --arch requires an architecture argument"
                exit 1
            fi
            shift 2
            ;;
        -h|--help)
            usage
            ;;
        *)
            echo "Unknown parameter: $1"
            usage
            ;;
    esac
done

# Detect distro package manager
detect_pkg_manager() {
    if command -v apt-get &>/dev/null; then
        echo "apt"
    elif command -v dnf &>/dev/null; then
        echo "dnf"
    elif command -v pacman &>/dev/null; then
        echo "pacman"
    else
        echo "unknown"
    fi
}

# Install recommendations
suggest_install() {
    local missing_type="$1"
    local pm
    pm="$(detect_pkg_manager)"

    echo ""
    echo "============================================================"
    echo "Warning: missing required dependencies!"
    echo "============================================================"

    if [[ "$missing_type" == "container" ]]; then
        echo "Building with '--cross' requires Podman or Docker."
        echo "Recommended install command:"
        case "$pm" in
            apt)    echo "  sudo apt update && sudo apt install -y podman" ;;
            dnf)    echo "  sudo dnf install -y podman" ;;
            pacman) echo "  sudo pacman -S --needed podman" ;;
            *)      echo "  Install Podman or Docker with your package manager." ;;
        esac
        echo ""
        echo "Also make sure cross is installed: cargo install cross --git https://github.com/cross-rs/cross"
    elif [[ "$missing_type" == "cargo-tools" ]]; then
        echo "Building with '--cargo' requires cross linkers and musl toolchains."
        echo "Recommended install command:"
        case "$pm" in
            apt)
                echo "  sudo apt update && sudo apt install -y musl-tools gcc-i686-linux-gnu gcc-aarch64-linux-gnu gcc-arm-linux-gnueabihf"
                ;;
            dnf)
                echo "  sudo dnf install -y musl-gcc gcc-arm-linux-gnu gcc-aarch64-linux-gnu"
                ;;
            pacman)
                echo "  sudo pacman -S --needed musl aarch64-linux-gnu-gcc arm-linux-gnueabihf-gcc lib32-glibc"
                ;;
            *)
                echo "  Install cross compilers for x86, aarch64, armv7 and musl-tools."
                ;;
        esac
    fi
    echo "============================================================"
    echo ""
}

# Container engine check
has_container_engine() {
    command -v podman &>/dev/null || (command -v docker &>/dev/null && docker info &>/dev/null)
}

# Linker check for cargo builds
check_cargo_linker() {
    local target="$1"
    local linker=""

    case "$target" in
        "$TARGET_X86")
            linker="i686-linux-gnu-gcc"
            ;;
        "$TARGET_ARM64")
            linker="aarch64-linux-gnu-gcc"
            ;;
        "$TARGET_ARM32")
            linker="arm-linux-gnueabihf-gcc"
            ;;
        "$TARGET_X64")
            return 0
            ;;
    esac

    if ! command -v "$linker" &>/dev/null; then
        echo "Error: cross linker '$linker' for $target not found!"
        suggest_install "cargo-tools"
        return 1
    fi
    return 0
}

# Resolve final builder
BUILDER=""
if [[ "$METHOD" == "cross" ]]; then
    if ! command -v cross &>/dev/null || ! has_container_engine; then
        suggest_install "container"
        exit 1
    fi
    BUILDER="cross"
elif [[ "$METHOD" == "cargo" ]]; then
    BUILDER="cargo"
else
    # auto
    if command -v cross &>/dev/null && has_container_engine; then
        BUILDER="cross"
    else
        BUILDER="cargo"
    fi
fi

echo "==> Build mode: $BUILDER (selected by rule: $METHOD)"

# Prepare rustup targets for cargo
if [[ "$BUILDER" == "cargo" ]]; then
    echo "==> Checking rustup targets..."
    rustup target add "${ALL_TARGETS[@]}" >/dev/null 2>&1 || true
fi

rm -rf "$DIST_DIR"
mkdir -p "$DIST_DIR"

build_target() {
    local target="$1"
    local output_name="$2"
    local short_arch="$3"

    echo ""
    echo "------------------------------------------------------------"
    echo "Building [$output_name] -> $target"
    echo "------------------------------------------------------------"

    if [[ "$BUILDER" == "cross" ]]; then
        cross build --release --target "$target"
    else
        if ! check_cargo_linker "$target"; then
            exit 1
        fi
        case "$target" in
            "$TARGET_ARM64")
                export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER="aarch64-linux-gnu-gcc"
                ;;
            "$TARGET_ARM32")
                export CARGO_TARGET_ARMV7_UNKNOWN_LINUX_MUSLEABIHF_LINKER="arm-linux-gnueabihf-gcc"
                ;;
            "$TARGET_X86")
                export CARGO_TARGET_I686_UNKNOWN_LINUX_MUSL_LINKER="i686-linux-gnu-gcc"
                ;;
            "$TARGET_X64")
                if command -v musl-gcc &>/dev/null; then
                    export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER="musl-gcc"
                fi
                ;;
        esac

        cargo build --release --target "$target"
    fi

    local src_bin="$SCRIPT_DIR/target/$target/release/$BIN_NAME"
    local dst_bin="$DIST_DIR/${BIN_NAME}-${output_name}"

    if [[ -f "$src_bin" ]]; then
        cp "$src_bin" "$dst_bin"

        # Strip debug info
        if command -v strip &>/dev/null; then
            strip "$dst_bin" 2>/dev/null || true
        fi

        local size
        size=$(stat -c%s "$dst_bin" 2>/dev/null || stat -f%z "$dst_bin")
        echo "Success: $dst_bin ($size bytes)"

        # Post-process: adb-push-ready copy as target/push/{name}_{arch}
        # (e.g. target/push/recovery-tensor-daemon_arm64 for `adb push` to
        # /data/local on devices). target/ is gitignored; push/ is kept
        # across runs (only overwritten per built arch).
        mkdir -p "$PUSH_DIR"
        cp "$dst_bin" "$PUSH_DIR/${BIN_NAME}_${short_arch}"
        echo "Push copy: $PUSH_DIR/${BIN_NAME}_${short_arch}"
    else
        echo "Error: compiled binary not found: $src_bin"
        exit 1
    fi
}

# Build tasks (3rd arg = short arch for target/push/{name}_{arch})
case "$SELECTED_ARCH" in
    all)
        build_target "$TARGET_X64"   "linux-x86_64" "x64"
        build_target "$TARGET_X86"   "linux-x86"    "x86"
        build_target "$TARGET_ARM64" "linux-arm64"  "arm64"
        build_target "$TARGET_ARM32" "linux-arm32"  "arm32"
        ;;
    x64)
        build_target "$TARGET_X64"   "linux-x86_64" "x64"
        ;;
    x86)
        build_target "$TARGET_X86"   "linux-x86"    "x86"
        ;;
    arm64)
        build_target "$TARGET_ARM64" "linux-arm64"  "arm64"
        ;;
    arm32)
        build_target "$TARGET_ARM32" "linux-arm32"  "arm32"
        ;;
    *)
        echo "Error: unknown architecture '$SELECTED_ARCH'"
        usage
        ;;
esac

echo ""
echo "============================================================"
echo "Done! Generated binaries in dist/:"
ls -lh "$DIST_DIR"
echo "Push-ready copies in target/push/ ({name}_{arch} for adb push):"
ls -lh "$PUSH_DIR"
echo "============================================================"
