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

# ---- bin.lgz first-run unpack (magisk branch: into the module dir) ----
# The zip ships every tool as one lgz cluster (bin.lgz) plus per-OS lean
# decoders in unpack_bin/. Unpack once into $MODPATH/bin (the marker
# pins the cluster byte size); install-recovery.sh below re-checks the
# same marker and skips when already unpacked.
_UP_LGZ="$MODPATH/bin.lgz"
_UP_DST="$MODPATH/bin"
_UP_ARCH=""
case "$(uname -m 2>/dev/null || echo unknown)" in
    x86_64|amd64) _UP_ARCH="x86_64";;
    aarch64|arm64) _UP_ARCH="arm64";;
esac
_UP_DEC="$MODPATH/unpack_bin/lgz-lean-linux-$_UP_ARCH"
if [ -f "$_UP_LGZ" ]; then
    _UP_NEED=1
    if [ "${FOX_UNPACK_FORCE:-0}" != 1 ] && [ -f "$_UP_DST/.lgz-cluster-size" ]; then
        _UP_WANT="$(wc -c < "$_UP_LGZ" | tr -d ' ')"
        _UP_HAVE="$(cat "$_UP_DST/.lgz-cluster-size" 2>/dev/null | tr -d ' \r\n')"
        if [ -n "$_UP_WANT" ] && [ "$_UP_WANT" = "$_UP_HAVE" ]; then
            _UP_NEED=0
        fi
    fi
    if [ "$_UP_NEED" = 1 ]; then
        if [ -z "$_UP_ARCH" ] || [ ! -f "$_UP_DEC" ]; then
            abort "! no lgz decoder for linux/$_UP_ARCH (want unpack_bin/lgz-lean-linux-$_UP_ARCH in the module)"
        fi
        chmod +x "$_UP_DEC" 2>/dev/null || true
        [ -x "$_UP_DEC" ] || abort "! lgz decoder not executable: $_UP_DEC"
        ui_print "- Unpacking tools (bin.lgz)..."
        "$_UP_DEC" decompress "$_UP_LGZ" "$_UP_DST" || abort "! bin.lgz unpack failed"
        wc -c < "$_UP_LGZ" | tr -d ' ' > "$_UP_DST/.lgz-cluster-size"
    fi
fi
unset _UP_LGZ _UP_DST _UP_ARCH _UP_DEC _UP_NEED _UP_WANT _UP_HAVE

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
    abort "! vendor_boot install failed (see install.log in /tmp/recovery_install, /data/local/tmp/recovery_install or /data/adb/recovery_install.tmp)"
fi

return 0
