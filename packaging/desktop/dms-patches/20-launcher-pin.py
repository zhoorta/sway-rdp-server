# A patch of the DMS shell for waydesk, exit status 1 when the file is not what it expects (another DMS version): nothing is written then.
# The launcher menu (right click on an app) gets "Pin to desktop" next to "Pin to Dock" (waydesk-add-to-desktop). Usage: SRC DST (Modals/DLauncherV2/LauncherContextMenu.qml).
import os, sys
src, dst = sys.argv[1:]
text = open(src).read()
old = '''        if (item?.type === "app" && pinDockId)
            items.push({
                type: "item",
                icon: isPinned ? "keep_off" : "push_pin",
                text: isPinned ? I18n.tr("Unpin from Dock") : I18n.tr("Pin to Dock"),
                action: togglePin
            });
'''
new = '''        if (isRegularApp && desktopEntry?.id)
            items.push({
                type: "item",
                icon: "push_pin",
                text: "Pin to desktop",
                action: () => {
                    Quickshell.execDetached(["waydesk-add-to-desktop", desktopEntry.id]);
                    hide();
                }
            });
'''
if old not in text or "import Quickshell\n" in text:
    sys.exit(1)
text = text.replace("import QtQuick\n", "import QtQuick\nimport Quickshell\n", 1).replace(old, old + new, 1)
os.remove(dst)
with open(dst, "w") as f:
    f.write(text)
