# A patch of the DMS shell for waydesk, exit status 1 when the file is not what it expects (another DMS version): nothing is written then.
# The focused-window menu of the bar (right click) shows the PID and has "Kill Process", but DMS finds the PID only on niri, Hyprland and Mango;
# on sway it said "Unavailable". Ask sway for it (waydesk-focused-pid). Usage: SRC DST (DMS 1.x: Modules/DankBar, 2.x: Modules/DBar).
import os, sys
src, dst = sys.argv[1:]
text = open(src).read()
old1 = "    function resolveActiveWindowPid() {\n        if (!activeWindow)\n            return 0;\n"
old2 = "        popout.processId = CompositorService.windowPid(activeWindow);\n"
if "import Quickshell.Io" in text or (old1 not in text and old2 not in text):
    sys.exit(1)
add = '''    property int swayFocusedPid: 0
    Process {
        id: swayPidProcess
        command: ["waydesk-focused-pid"]
        stdout: StdioCollector {
            onStreamFinished: root.swayFocusedPid = parseInt(text) || 0
        }
    }
    Connections {
        target: root
        function onActiveWindowChanged() {
            if (CompositorService.isSway)
                swayPidProcess.running = true;
        }
    }

'''
text = text.replace("import Quickshell\n", "import Quickshell\nimport Quickshell.Io\n", 1)
if old1 in text:
    text = text.replace(old1, add + old1 + "        if (CompositorService.isSway)\n            return swayFocusedPid;\n", 1)
else:
    text = text.replace(old2, "        popout.processId = CompositorService.windowPid(activeWindow) || (CompositorService.isSway ? swayFocusedPid : 0);\n", 1)
    text = text.replace("    function syncPopoutState() {", add + "    function syncPopoutState() {", 1)
os.remove(dst)
with open(dst, "w") as f:
    f.write(text)
