#!/usr/bin/env bash
# Installs the Dolphin right-click action "Install with OrangeFox
# recovery installer" for the current user (KDE Plasma: Dolphin has no
# drag-and-drop onto executables, the context menu is the idiom).
#
# Usage: ./install.sh   (run from this directory, or anywhere: the
# installer dir is resolved from the script path)
#
# Copies install-with-orangefox.desktop to the user ServiceMenus dirs
# (both kservices5 and kservices6 locations; the running Plasma reads
# whichever it uses, the other is harmless) with @INSTALLER_DIR@
# replaced by the absolute installer path. Overwrites a previous copy.
# Uninstall: delete the two installed files (paths are printed).
set -u

SRC_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INSTALLER_DIR="$(cd "$SRC_DIR/.." && pwd)"
TPL="$SRC_DIR/install-with-orangefox.desktop"
NAME="install-with-orangefox.desktop"

[ -f "$TPL" ] || { echo "template missing: $TPL" >&2; exit 1; }
[ -x "$INSTALLER_DIR/install-desktop.sh" ] \
    || { echo "install-desktop.sh missing in $INSTALLER_DIR" >&2; exit 1; }

installed=0
for dest in "$HOME/.local/share/kservices5/ServiceMenus" \
            "$HOME/.local/share/kservices6/ServiceMenus"; do
    mkdir -p "$dest" 2>/dev/null || continue
    sed "s|@INSTALLER_DIR@|$INSTALLER_DIR|g" "$TPL" > "$dest/$NAME" 2>/dev/null \
        || { echo "cannot write $dest/$NAME" >&2; continue; }
    echo "installed: $dest/$NAME"
    installed=1
done
[ "$installed" = 1 ] || { echo "nothing installed" >&2; exit 1; }

if command -v kbuildsycoca6 >/dev/null 2>&1; then
    kbuildsycoca6 >/dev/null 2>&1 || true
elif command -v kbuildsycoca5 >/dev/null 2>&1; then
    kbuildsycoca5 >/dev/null 2>&1 || true
fi
echo "done: right-click any payload file -> Actions -> Install with OrangeFox recovery installer"
