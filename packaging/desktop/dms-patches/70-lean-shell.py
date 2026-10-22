# A patch of the DMS shell for waydesk, exit status 1 when the file is not what it expects (another DMS version): nothing is written then.
# Leaves out what a remote session never uses, so it is not built at every account's start (about 20 MB of the shell's memory, measured):
#  - the desktop widget layer, its editor and its context menu: the desktop is drawn by the file manager, and the settings pages are hidden (40-settings-tabs);
#  - the clipboard history window: it needs ext-data-control, which sway does not have (the shell's own log says "Clipboard manager unavailable").
# Everything that refers to them is null-safe (PopoutService.clipboardHistoryModal?.show(), the IPC handlers check for null). Usage: SRC DST (DMSShell.qml).
import os, sys
src, dst = sys.argv[1:]
text = open(src).read()
for header in ("    DesktopWidgetLayer {", "    DesktopWidgetEditor {", "    DesktopContextMenu {", "    ClipboardHistoryModal {"):
    i = text.find("\n" + header)
    if i < 0:
        sys.exit(1)
    i += 1
    depth, j = 0, i
    while j < len(text):
        c = text[j]
        if c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
            if depth == 0:
                break
        j += 1
    if depth != 0:
        sys.exit(1)
    end = text.index("\n", j) + 1
    # also the blank line that separated it from the next object
    if text[end:end + 1] == "\n":
        end += 1
    text = text[:i] + text[end:]
os.remove(dst)
with open(dst, "w") as f:
    f.write(text)
