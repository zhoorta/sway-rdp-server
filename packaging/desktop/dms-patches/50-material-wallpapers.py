# A patch of the DMS shell for waydesk, exit status 1 when the file is not what it expects (another DMS version): nothing is written then.
# The "Material" wallpapers are drawn by the shell's own wallpaper layer, which is off (the file manager draws the desktop): choosing one changes nothing, so the choice is hidden. Usage: SRC DST (Modules/Settings/WallpaperColorsTab.qml).
import os, sys
src, dst = sys.argv[1:]
text = open(src).read()
if "visible: root.materialWallpaper\n" not in text:
    sys.exit(1)
os.remove(dst)
with open(dst, "w") as f:
    f.write(text.replace("visible: root.materialWallpaper\n", "visible: false\n"))
