import QtQuick
import Quickshell
import qs.Common
import qs.Services
import qs.Widgets
import qs.Modules.Plugins

// Window buttons for the focused window in the Mac style (red closes, yellow minimizes, green maximizes), on top of
// sway-rdp-windows: sway's title bars have none. The small arrow opens the list of minimized windows.
PluginComponent {
    id: root

    property var minimized: []

    function refresh() {
        Proc.runCommand("swayWindowControls.list", ["sway-rdp-windows", "list"], function (out, code) {
            if (code !== 0)
                return;
            try {
                root.minimized = JSON.parse(out);
            } catch (e) {
                root.minimized = [];
            }
        }, 0);
    }

    function restore(id) {
        Proc.runCommand("swayWindowControls.restore", ["sway-rdp-windows", "restore", String(id)], function () {
            root.refresh();
        }, 0);
        root.closePopout();
    }

    // The count on the arrow follows what is minimized.
    Timer {
        interval: 3000
        running: true
        repeat: true
        triggeredOnStart: true
        onTriggered: root.refresh()
    }

    function run(action) {
        if (action === "close")
            Proc.runCommand("swayWindowControls.close", ["swaymsg", "kill"], function () {}, 0);
        else
            Proc.runCommand("swayWindowControls." + action, ["sway-rdp-windows", action], function () {
                root.refresh();
            }, 0);
    }

    readonly property var buttons: [
        { "icon": "close", "action": "close", "tone": Theme.error },
        { "icon": "remove", "action": "minimize", "tone": Theme.warning },
        { "icon": "add", "action": "maximize", "tone": Theme.success }
    ]

    horizontalBarPill: Component {
        Row {
            spacing: Theme.spacingXS + 2

            Repeater {
                model: root.buttons

                Rectangle {
                    required property var modelData
                    width: 10
                    height: 10
                    radius: 5
                    anchors.verticalCenter: parent.verticalCenter
                    color: modelData.tone
                    opacity: area.containsMouse ? 1 : 0.7

                    DankIcon {
                        anchors.centerIn: parent
                        visible: area.containsMouse
                        name: modelData.icon
                        size: 10
                        color: Theme.surface
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

            Item {
                anchors.verticalCenter: parent.verticalCenter
                width: arrowRow.width
                height: arrowRow.height
                visible: root.minimized.length > 0

                Row {
                    id: arrowRow
                    spacing: 1

                    StyledText {
                        anchors.verticalCenter: parent.verticalCenter
                        text: root.minimized.length
                        font.pixelSize: Theme.fontSizeSmall
                        color: arrowArea.containsMouse ? Theme.surfaceText : Theme.surfaceVariantText
                    }

                    DankIcon {
                        anchors.verticalCenter: parent.verticalCenter
                        name: "expand_more"
                        size: 14
                        color: arrowArea.containsMouse ? Theme.surfaceText : Theme.surfaceVariantText
                    }
                }

                MouseArea {
                    id: arrowArea
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: {
                        root.refresh();
                        root.triggerPopout();
                    }
                }
            }
        }
    }

    popoutWidth: 320
    popoutHeight: 0

    popoutContent: Component {
        PopoutComponent {
            headerText: I18n.tr("Minimized")
            showCloseButton: true

            Column {
                width: parent.width
                spacing: Theme.spacingXS

                StyledText {
                    visible: root.minimized.length === 0
                    text: I18n.tr("No minimized windows")
                    color: Theme.surfaceVariantText
                    font.pixelSize: Theme.fontSizeMedium
                }

                Repeater {
                    model: root.minimized

                    Rectangle {
                        required property var modelData
                        width: parent.width
                        height: 40
                        radius: Theme.cornerRadius
                        color: rowArea.containsMouse ? Theme.surfaceContainerHighest : "transparent"

                        Row {
                            anchors.fill: parent
                            anchors.leftMargin: Theme.spacingS
                            spacing: Theme.spacingS

                            Item {
                                width: 24
                                height: 24
                                anchors.verticalCenter: parent.verticalCenter

                                Image {
                                    id: appIcon
                                    anchors.fill: parent
                                    source: modelData.app ? Quickshell.iconPath(modelData.app.toLowerCase(), true) : ""
                                    sourceSize: Qt.size(48, 48)
                                    fillMode: Image.PreserveAspectFit
                                }

                                DankIcon {
                                    anchors.centerIn: parent
                                    visible: appIcon.status !== Image.Ready
                                    name: "web_asset"
                                    size: 20
                                    color: Theme.surfaceVariantText
                                }
                            }

                            StyledText {
                                anchors.verticalCenter: parent.verticalCenter
                                width: parent.width - 24 - Theme.spacingS * 3
                                text: modelData.title || modelData.app || "?"
                                elide: Text.ElideRight
                                color: Theme.surfaceText
                                font.pixelSize: Theme.fontSizeMedium
                            }
                        }

                        MouseArea {
                            id: rowArea
                            anchors.fill: parent
                            hoverEnabled: true
                            cursorShape: Qt.PointingHandCursor
                            onClicked: root.restore(modelData.id)
                        }
                    }
                }
            }
        }
    }
}
