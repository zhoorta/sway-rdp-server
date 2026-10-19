import QtQuick
import qs.Common
import qs.Services
import qs.Widgets
import qs.Modules.Plugins

// Buttons for the focused window, on top of sway-rdp-windows (sway's title bars have none).
PluginComponent {
    id: root

    function run(action) {
        if (action === "close")
            Proc.runCommand("swayWindowControls.close", ["swaymsg", "kill"], function () {}, 0);
        else
            Proc.runCommand("swayWindowControls." + action, ["sway-rdp-windows", action], function () {}, 0);
    }

    readonly property var buttons: [
        { "icon": "unfold_more", "action": "restore" },
        { "icon": "minimize", "action": "minimize" },
        { "icon": "crop_square", "action": "maximize" },
        { "icon": "close", "action": "close" }
    ]

    horizontalBarPill: Component {
        Row {
            spacing: Theme.spacingXS

            Repeater {
                model: root.buttons

                Rectangle {
                    required property var modelData
                    width: Theme.iconSize + 4
                    height: Theme.iconSize + 4
                    radius: Theme.cornerRadius
                    color: area.containsMouse ? (modelData.action === "close" ? Theme.error : Theme.surfaceContainerHighest) : "transparent"

                    DankIcon {
                        anchors.centerIn: parent
                        name: modelData.icon
                        size: Theme.iconSize - 6
                        color: Theme.surfaceText
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
        }
    }
}
