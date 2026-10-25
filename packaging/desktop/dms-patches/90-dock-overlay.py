# A patch of the DMS shell for waydesk, exit status 1 when the file is not what it expects (another DMS version): nothing is written then.
# The dock stays where it is but reserves no space: windows (a maximized one too) go behind it, like the bar's space is the only one kept free.
# Usage: SRC DST (Modules/Dock/DockBody.qml).
import os, sys
src, dst = sys.argv[1:]
text = open(src).read()
old = "    readonly property bool shouldReserveDockSpace: dock.config.enabled && dockGeometry.shouldReserveSpace\n"
if old not in text:
    sys.exit(1)
text = text.replace(old, "    readonly property bool shouldReserveDockSpace: false\n", 1)
if os.path.lexists(dst):
    os.remove(dst)
with open(dst, "w") as f:
    f.write(text)
