#!/system/bin/sh
PKG_NAME="com.android.pixeldisplayservice"
LOG_TAG="PixelDisplayServiceDisabler"

log() {
    /system/bin/log -p i -t "$LOG_TAG" "$1"
}

log "Waiting for PackageManager service to be ready."

# Wait for PackageManager to become ready
until pm path android; do
    log "Waiting for PackageManager to become ready..."
    sleep 1
done

log "PackageManager is ready. Checking if $PKG_NAME is installed."

# Check if the package is installed
if ! pm list packages | grep -q "$PKG_NAME"; then
    log "Package $PKG_NAME is not installed. No action required."
    exit 0
fi

log "Attempting to uninstall $PKG_NAME."

# Attempt to uninstall the package
if pm uninstall -k --user 0 "$PKG_NAME"; then
    log "Successfully uninstalled $PKG_NAME."
else
    log "Failed to uninstall $PKG_NAME. It may already be uninstalled for this user."
fi
