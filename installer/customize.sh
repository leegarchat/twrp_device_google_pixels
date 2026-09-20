# OrangeFox vendor_boot installer (Magisk / KernelSU).
#
# Installer-only module: the manager is just a root runner from the
# booted system. The engine (install-recovery.sh) flashes both slots
# (or SLOT= from export.txt) with userdata backup, then this script
# drops the payload, keeps module.prop for the manager's own
# bookkeeping, and flags the module for removal: after the merge it
# uninstalls itself, no persistent module stays behind.
#
# Sourced (not executed) by the manager installer: no `exit` here,
# finish with return/abort only.
MODID="ofx_vb_installer"
NVBASE="${NVBASE:-/data/adb}"

ui_print "- OrangeFox vendor_boot installer"

# Drop leftovers of previous runs (persistent copy, if one exists).
rm -rf "$NVBASE/modules/$MODID" 2>/dev/null || true

chmod +x "$MODPATH/install-recovery.sh" "$MODPATH"/bin/linux/* 2>/dev/null || true
if sh "$MODPATH/install-recovery.sh"; then
    ui_print "- install OK, removing installer payload"
    # Installer only, but the manager still needs module.prop right
    # after this script returns (it copies it for bookkeeping), so
    # keep that one file, drop everything else, and flag the module
    # for removal: the manager uninstalls it, nothing stays behind.
    for f in "$MODPATH"/*; do
        [ "$f" = "$MODPATH/module.prop" ] || rm -rf "$f" 2>/dev/null || true
    done
    touch "$MODPATH/remove" 2>/dev/null || true
else
    ui_print "! install FAILED, removing installer module"
    rm -rf "$MODPATH" "$NVBASE/modules/$MODID" 2>/dev/null || true
    abort "! vendor_boot install failed (see /tmp/recovery_install/install.log)"
fi

return 0
