# A patch of the DMS shell for waydesk, exit status 1 when the file is not what it expects (another DMS version): nothing is written then.
# The power menu: the custom button ("Change password") acts at once, without the hold to confirm, and sits in a group of its own below "Log out". Usage: MENU_SRC MENU_DST VIEW_SRC VIEW_DST (Modules/PowerMenu/PowerMenuContent.qml, DCommon/Session/PowerMenuView.qml).
import os, sys
menu_src, menu_dst, view_src, view_dst = sys.argv[1:]
menu = open(menu_src).read()
view = open(view_src).read()
confirm = 'return action !== "lock" && action !== "restart";'
first = "firstInGroup: root.gridLayout || index === 0\n"
last = "lastInGroup: root.gridLayout || index === root.actions.length - 1\n"
anchor = "                focusPolicy: Qt.NoFocus\n"
if confirm not in menu or first not in view or last not in view or anchor not in view:
    sys.exit(1)
menu = menu.replace(confirm, 'return action !== "lock" && action !== "restart" && !String(action).startsWith("custom:");', 1)
is_custom = 'modelData.startsWith("custom:")'
view = view.replace(first, "firstInGroup: root.gridLayout || index === 0 || (" + is_custom + " && !root.actions[index - 1].startsWith(\"custom:\"))\n", 1)
view = view.replace(last, "lastInGroup: root.gridLayout || index === root.actions.length - 1 || (index + 1 < root.actions.length && root.actions[index + 1].startsWith(\"custom:\") && !" + is_custom + ")\n", 1)
line = anchor + """                // The "Change password" group sits a little lower, with a line in the space between it and "Log out".
                transform: Translate { y: !root.gridLayout && button.modelData.startsWith("custom:") ? root.groupGap : 0 }
"""
view = view.replace(anchor, line, 1)
rule = """    // The line between "Log out" and the custom group (in the menu itself: the list items clip what they draw outside themselves).
    Rectangle {
        readonly property int first: root.actions.findIndex(a => a.startsWith("custom:"))
        visible: root.groupGap > 0 && first > 0
        x: Style.spacingL + Style.spacingM
        y: Style.spacingL + first * (Style.listItemHeight + root.itemGap) - root.itemGap + (root.showHint ? root.hintPad * 2 + hint.implicitHeight : root.groupGap / 2) - 0.5
        width: root.width - (Style.spacingL + Style.spacingM) * 2
        height: 1
        color: Style.withAlpha(Style.outline, 0.4)
    }

"""
grid = "    Grid {\n        id: buttons\n"
if grid not in view:
    sys.exit(1)
view = view.replace(grid, rule + grid, 1)
gap = "    readonly property real itemGap:"
height = "    implicitHeight: buttons.implicitHeight + Style.spacingL * 2 + (showHint"
margin = "        anchors.top: buttons.bottom\n        anchors.topMargin: Style.spacingM\n"
if gap not in view or height not in view or margin not in view:
    sys.exit(1)
props = """    readonly property bool grouped: !gridLayout && actions.some(a => a.startsWith("custom:"))
    readonly property int firstCustom: actions.findIndex(a => a.startsWith("custom:"))
    readonly property real hintPad: Style.spacingS
    readonly property real groupGap: !grouped ? 0 : showHint ? hint.implicitHeight + hintPad * 3 : Style.spacingM
"""
view = view.replace(gap, props + gap, 1)
view = view.replace(height, "    implicitHeight: buttons.implicitHeight + groupGap + Style.spacingL * 2 + (showHint && !grouped", 1)
view = view.replace(margin, "        y: grouped ? Style.spacingL + firstCustom * (Style.listItemHeight + itemGap) - itemGap + hintPad : buttons.y + buttons.height + Style.spacingM\n", 1)
for dst, text in ((menu_dst, menu), (view_dst, view)):
    os.remove(dst)
    with open(dst, "w") as f:
        f.write(text)
