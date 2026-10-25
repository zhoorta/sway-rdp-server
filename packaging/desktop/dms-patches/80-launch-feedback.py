# A patch of the DMS shell for waydesk, exit status 1 when a file is not what it expects (another DMS version): nothing is written then.
# Feedback while an application starts (like the bouncing icon of a Mac): the dock shows the app, a pinned one or a new item for one that is not
# pinned, with an icon that bounces until the window is there (or 12 seconds have passed). Usage: SRC DST for Services/SessionService.qml,
# Modules/SurfaceWidgets/ApplicationStrip.qml and Modules/Dock/DockAppButton.qml, in that order.
import os, sys
pairs = list(zip(sys.argv[1::2], sys.argv[2::2]))
if len(pairs) != 3:
    sys.exit(1)
texts = [open(src).read() for src, _ in pairs]
session, strip, button = texts
if "noteLaunch" in session:
    sys.exit(1)

# 1. SessionService: remember what is starting.
a_old = "    function launchDesktopEntry(desktopEntry, useNvidia, args) {\n        if (!desktopEntry || !desktopEntry.command)\n            return;\n"
a_new = a_old + "        noteLaunch(desktopEntry.id ? Paths.moddedAppId(desktopEntry.id) : \"\");\n"
state = '''    // Applications that are starting (id -> when, and how many windows there were): the dock shows them with a bouncing icon.
    property var launching: ({})
    function noteLaunch(appId) {
        if (!appId)
            return;
        const next = Object.assign({}, launching);
        next[appId] = {
            time: Date.now(),
            windows: CompositorService.sortedToplevels.length
        };
        launching = next;
        launchExpiry.start();
    }
    Timer {
        id: launchExpiry
        interval: 500
        repeat: true
        onTriggered: {
            const now = Date.now();
            const windows = CompositorService.sortedToplevels.length;
            const next = {};
            let any = false;
            for (const id in launching) {
                if (now - launching[id].time < 12000 && windows <= launching[id].windows) {
                    next[id] = launching[id];
                    any = true;
                }
            }
            if (Object.keys(next).length !== Object.keys(launching).length)
                launching = next;
            if (!any)
                stop();
        }
    }

'''
if a_old not in session:
    sys.exit(1)
session = session.replace(a_old, state + a_new, 1)
if "import qs.Common" not in session:
    sys.exit(1)

# 2. ApplicationStrip: the item of a starting app.
b_old = "                    let finalItems = overflow.items;\n"
b_new = b_old + '''                    const launchingIds = Object.keys(SessionService.launching || {});
                    if (launchingIds.length) {
                        finalItems = finalItems.map(item => Object.assign({}, item));
                        launchingIds.forEach(id => {
                            const existing = finalItems.find(item => item.appId === id && item.type !== "separator");
                            if (existing) {
                                if (!existing.isRunning)
                                    existing.isLaunching = true;
                                return;
                            }
                            finalItems.push({
                                uniqueKey: "launching_" + id,
                                type: "pinned",
                                appId: id,
                                toplevel: null,
                                isPinned: false,
                                isRunning: false,
                                isLaunching: true,
                                isCoreApp: false,
                                coreAppData: null,
                                isInOverflow: false
                            });
                        });
                    }
'''
c_old = "    onPinnedAppsChanged: modelUpdate.schedule()\n"
c_new = c_old + '''    Connections {
        target: SessionService
        function onLaunchingChanged() {
            modelUpdate.schedule();
        }
    }
'''
if b_old not in strip or c_old not in strip:
    sys.exit(1)
strip = strip.replace(b_old, b_new, 1).replace(c_old, c_new, 1)
if "import qs.Services" not in strip:
    strip = strip.replace("import qs.Common\n", "import qs.Common\nimport qs.Services\n", 1)
    if "import qs.Services" not in strip:
        sys.exit(1)

# 3. DockAppButton: the bounce.
d_old = "    readonly property real hoverAnimOffset: hoverBounce.offset\n"
d_new = '''    property real launchOffset: 0
    readonly property bool launching: appData?.isLaunching === true
    readonly property real hoverAnimOffset: hoverBounce.offset + launchOffset

    SequentialAnimation {
        running: root.launching && !root.dragging
        loops: Animation.Infinite
        onRunningChanged: if (!running)
            root.launchOffset = 0
        NumberAnimation {
            target: root
            property: "launchOffset"
            to: hoverBounce.direction * root.actualIconSize * 0.35
            duration: 260
            easing.type: Easing.OutQuad
        }
        NumberAnimation {
            target: root
            property: "launchOffset"
            to: 0
            duration: 320
            easing.type: Easing.OutBounce
        }
        PauseAnimation {
            duration: 120
        }
    }
'''
if d_old not in button:
    sys.exit(1)
button = button.replace(d_old, d_new, 1)

for (src, dst), text in zip(pairs, (session, strip, button)):
    if os.path.lexists(dst):
        os.remove(dst)
    with open(dst, "w") as f:
        f.write(text)
