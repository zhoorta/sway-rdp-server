# A patch of the DMS shell for waydesk, exit status 1 when the file is not what it expects (another DMS version): nothing is written then.
# The launcher lists no screensavers: xscreensaver-data (a dependency of the file manager's session package) installs about a hundred
# of them as applications (Categories=Screensaver, no icon, they draw on the root window and cannot run in a Wayland session).
# Usage: SRC DST (Services/AppSearchService.qml).
import os, sys
src, dst = sys.argv[1:]
text = open(src).read()
old = "        applications = DesktopEntries.applications.values;\n"
if old not in text:
    sys.exit(1)
new = """        const all = DesktopEntries.applications.values;
        const kept = [];
        for (let i = 0; i < all.length; i++) {
            const cats = all[i].categories;
            let screensaver = false;
            for (let j = 0; cats && j < cats.length; j++) {
                if (cats[j] === "Screensaver")
                    screensaver = true;
            }
            if (!screensaver)
                kept.push(all[i]);
        }
        applications = kept;
"""
text = text.replace(old, new, 1)
if os.path.lexists(dst):
    os.remove(dst)
with open(dst, "w") as f:
    f.write(text)
