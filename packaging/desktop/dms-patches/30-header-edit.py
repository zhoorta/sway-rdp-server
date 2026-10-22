# A patch of the DMS shell for waydesk, exit status 1 when the file is not what it expects (another DMS version): nothing is written then.
# The control center header has an edit button (rearranging its tiles) that has no setting to hide it. Usage: SRC DST (Modules/ControlCenter/Components/HeaderPane.qml).
import os, sys
src, dst = sys.argv[1:]
text = open(src).read()
needle = 'iconName: editMode ? "done" : "edit"'
if needle not in text:
    sys.exit(1)
os.remove(dst)
with open(dst, "w") as f:
    f.write(text.replace(needle, "visible: false\n            " + needle, 1))
