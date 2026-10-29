#!/bin/sh
# macOS double-click entry point: Finder runs .command files in
# Terminal (a bare .sh would open in an editor instead). Forwards
# drops and arguments to install-desktop.sh; keep this file next to
# export.txt and bin/. Drag-and-drop onto Terminal works too.
cd "$(dirname "$0")" || exit 1
exec ./install-desktop.sh "$@"
