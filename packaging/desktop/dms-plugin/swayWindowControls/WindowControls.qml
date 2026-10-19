import QtQuick
import qs.Common
import qs.Services
import qs.Widgets
import qs.Modules.Plugins

// Window buttons for the focused window in the Mac style (red closes, yellow minimizes, green maximizes), on top of
// sway-rdp-windows: sway's title bars have none. The small arrow brings the last minimized window back.
PluginComponent {
    id: root

    function run(action) {
        if (action === "close")
            Proc.runCommand("swayWindowControls.close", ["swaymsg", "kill"], function () {}, 0);
        else
            Proc.runCommand("swayWindowControls." + action, ["sway-rdp-windows", action], function () {}, 0);
    }

    readonly property var buttons: [
        { "icon": "close", "action": "close", "color": "#ff5f57" },
        { "icon": "remove", "action": "minimize", "color": "#febc2e" },
        { "icon": "add", "action": "maximize", "color": "#28c840" }
    ]

    horizontalBarPill: Component {
        Row {
            spacing: Theme.spacingS

            Repeater {
                model: root.buttons

                Rectangle {
                    required property var modelData
                    width: 14
                    height: 14
                    radius: 7
                    anchors.verticalCenter: parent.verticalCenter
                    color: modelData.color

                    DankIcon {
                        anchors.centerIn: parent
                        visible: area.containsMouse
                        name: modelData.icon
                        size: 12
                        color: "#4d0000"
                    }

                    MouseArea {
                        id: area
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: root.run(modelData.action)
                    }
                }
            }

            DankIcon {
                anchors.verticalCenter: parent.verticalCenter
                name: "unfold_more"
                size: 16
                color: restoreArea.containsMouse ? Theme.surfaceText : Theme.surfaceVariantText

                MouseArea {
                    id: restoreArea
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: root.run("restore")
                }
            }
        }
    }
}
