#!/system/bin/sh
#
# otg_patch.sh — Reads otg_host_shim module status, injects it if necessary,
# activates it, and sets the system property for USB routing.
#
# Called by runatboot: 
# 1. Checks if the otg_host_shim module is loaded.
# 2. Injects it from /system/lib64/modules if missing.
# 3. Activates the host_ready flag.
# 4. Sets sys.usb.patch_dwc3=1 so RC property triggers can handle OTG switching.
#
# The actual host <-> device switching is done by init.recovery.usb.rc triggers
# reacting to sys.usb.config changes when sys.usb.patch_dwc3=1.
#

MODULE_BASENAME="otg_host_shim"
PROC_SHIM="/proc/otg_host_shim"
PROC_READY="/proc/otg_host_ready"

log_info() {
    echo "otg_patch: INFO: $1"
    # Write to kernel log buffer for dmesg visibility
    echo "otg_patch: INFO: $1" > /dev/kmsg 2>/dev/null
}

log_error() {
    echo "otg_patch: ERROR: $1"
    echo "otg_patch: ERROR: $1" > /dev/kmsg 2>/dev/null
}

log_info "Starting OTG patch routine..."

mount -t debugfs none /sys/kernel/debug 2>/dev/null
if [ $? -eq 0 ]; then
    log_info "debugfs successfully mounted or already available."
else
    log_error "Failed to mount debugfs."
fi

# Monolithic kernels (CONFIG_MODULES=n) have no /proc/modules: nothing to inject.
if [ ! -e /proc/modules ]; then
    log_info "Monolithic kernel (/proc/modules absent), skipping module injection."
else

# _ko_try_load <mod> — pick the best .ko for this kernel and load it.
#
# Selection matrix:
#   - version:  uname -r major.minor (X.Y — patchlevel ignored),
#               e.g. 6.1 from "6.1.162-android14-11".
#   - pagesize: getconf PAGESIZE (4096 → <mod>_<ver>.ko,
#               16384 → <mod>_<ver>_16k.ko).
#   - exact match first; then brute force low→high (sort -V gives
#     5.15 < 5.15_16k < 6.1 < ...) staying within the detected pagesize.
#     Cross-pagesize candidates are tried only when PAGESIZE is unknown.
#   - insmod -f: our vermagic comes from reference kernel sources and never
#     matches stock Pixel kernels exactly; the X.Y + pagesize selection above
#     is the real compatibility gate, -f only bypasses the string check.
#
# Returns 0 if loaded or already loaded, 1 if nothing worked.
_ko_try_load() {
    local mod="$1"
    local dir="/system/lib64/modules"

    # Already loaded?
    if grep -q "^${mod} " /proc/modules 2>/dev/null; then
        return 0
    fi

    local ver pagesz wanted="" ordered="" cand
    ver=$(uname -r 2>/dev/null | cut -d. -f1,2)
    pagesz=$(getconf PAGESIZE 2>/dev/null)

    if [ -n "$ver" ] && [ -n "$pagesz" ]; then
        if [ "$pagesz" = "16384" ]; then
            wanted="$dir/${mod}_${ver}_16k.ko"
        elif [ "$pagesz" = "4096" ]; then
            wanted="$dir/${mod}_${ver}.ko"
        fi
    fi

    if [ -n "$wanted" ] && [ -f "$wanted" ]; then
        ordered="$wanted"
    fi

    for cand in $(ls "$dir"/${mod}_*.ko 2>/dev/null | sort -V); do
        [ "$cand" = "$wanted" ] && continue
        case "$cand" in
            *_16k.ko)
                [ "$pagesz" = "4096" ] && continue
                ;;
            *)
                [ "$pagesz" = "16384" ] && continue
                ;;
        esac
        ordered="$ordered $cand"
    done

    if [ -z "$ordered" ]; then
        log_error "No candidate files for $mod in $dir (uname=$(uname -r 2>/dev/null), pagesize=${pagesz:-unknown})."
        return 1
    fi

    for cand in $ordered; do
        log_info "Trying $cand (uname=$(uname -r 2>/dev/null), pagesize=${pagesz:-unknown})..."
        if insmod -f "$cand" 2>/dev/null; then
            log_info "Successfully injected $cand."
            return 0
        fi
    done
    log_error "All candidates failed for $mod."
    return 1
}

if [ ! -f "$PROC_SHIM" ]; then
    log_info "/proc/otg_host_shim not found. Module is not loaded."
    if ! _ko_try_load "$MODULE_BASENAME"; then
        log_error "Module injection failed."
        resetprop sys.usb.patch_dwc3 0
        exit 1
    fi
else
    log_info "Module is already loaded (/proc/otg_host_shim exists)."
fi

fi # /proc/modules present

if [ -f "$PROC_SHIM" ]; then
    STATUS=$(cat "$PROC_READY" 2>/dev/null)
    
    if [ "$STATUS" != "1" ]; then
        log_info "host_ready is inactive. Activating via $PROC_SHIM..."
        echo "1" > "$PROC_SHIM" 2>/dev/null
        STATUS=$(cat "$PROC_READY" 2>/dev/null)
    fi

    if [ "$STATUS" = "1" ]; then
        resetprop sys.usb.patch_dwc3 1
        log_info "host_ready is ACTIVE. Property sys.usb.patch_dwc3 set to 1."
    else
        resetprop sys.usb.patch_dwc3 0
        log_error "host_ready is NOT active despite activation attempt. Property sys.usb.patch_dwc3 set to 0."
    fi
else
    log_error "Critical error: Proc interfaces still missing after injection."
    resetprop sys.usb.patch_dwc3 0
    exit 1
fi

USBSW_CTRL_REG="0x93"
USBSW_CONNECT="0x09"

log_info "Searching for max77759 TCPC device in sysfs..."

TCPC_SYSFS_DIR=$(ls -d /sys/bus/i2c/drivers/max77759tcpc/*-* 2>/dev/null | head -n 1)

if [ -n "$TCPC_SYSFS_DIR" ]; then
    TCPC_DEV_NAME=$(basename "$TCPC_SYSFS_DIR")
    TCPC_I2C_BUS=$(echo "$TCPC_DEV_NAME" | cut -d'-' -f1)
    TCPC_I2C_ADDR_RAW=$(echo "$TCPC_DEV_NAME" | cut -d'-' -f2)
    TCPC_I2C_ADDR="0x${TCPC_I2C_ADDR_RAW}"
    log_info "Found max77759 TCPC on bus: $TCPC_I2C_BUS with address: $TCPC_I2C_ADDR"
    log_info "Connecting USB data path switches..."
    i2cset -fy "$TCPC_I2C_BUS" "$TCPC_I2C_ADDR" "$USBSW_CTRL_REG" "$USBSW_CONNECT" b 2>/dev/null
    if [ $? -eq 0 ]; then
        log_info "USB data path switches connected successfully."
    else
        log_error "Failed to write USBSW_CTRL via i2cset. OTG host may not enumerate devices."
    fi
else
    log_error "max77759 TCPC driver or device not found. Skipping switch configuration."
fi

log_info "OTG patch routine finished."
exit 0