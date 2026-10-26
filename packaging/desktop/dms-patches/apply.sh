#!/bin/sh
# Applies the waydesk patches to a DMS shell tree.
#
#   apply.sh SRC DST    SRC: the shell tree to read (/usr/share/quickshell/dms); DST: where the patched files are written. DST may be
#                       SRC itself (at package build time) or a copy of links to SRC (at an account's start). Prints one line per patch.
#                       Exit status 0 when at least one patch applied, 1 when none did (another DMS version).
#
# Each patch checks that the file is what it expects and leaves it alone when it is not, so a DMS update that moves things
# makes the patch skip instead of breaking the shell.
src=$1 dst=$2
here=$(cd "$(dirname "$0")" && pwd)
applied=0
try() { # patch file, then SRC DST pairs (relative to the trees)
    patch=$1; shift
    args=""
    for rel in "$@"; do
        [ -f "$src/$rel" ] || return 0
        args="$args $src/$rel $dst/$rel"
    done
    # shellcheck disable=SC2086
    if python3 "$here/$patch" $args; then echo "patched: $patch"; applied=$((applied + 1)); else echo "skipped: $patch ($*)"; fi
}
# DMS 1.x keeps the bar widgets in Modules/DankBar, DMS 2.x in Modules/DBar.
try 10-focused-app.py Modules/DankBar/Widgets/FocusedApp.qml
try 10-focused-app.py Modules/DBar/Widgets/FocusedApp.qml
try 20-launcher-pin.py Modals/DLauncherV2/LauncherContextMenu.qml
try 30-header-edit.py Modules/ControlCenter/Components/HeaderPane.qml
try 40-settings-tabs.py Common/SettingsTabs.qml
try 50-material-wallpapers.py Modules/Settings/WallpaperColorsTab.qml
try 70-lean-shell.py DMSShell.qml
try 80-launch-feedback.py Services/SessionService.qml Modules/SurfaceWidgets/ApplicationStrip.qml Modules/Dock/DockAppButton.qml
try 90-dock-overlay.py Modules/Dock/DockBody.qml
try 60-power-menu.py Modules/PowerMenu/PowerMenuContent.qml DCommon/Session/PowerMenuView.qml
[ "$applied" -gt 0 ]
