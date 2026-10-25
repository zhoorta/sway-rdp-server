import QtQuick
import Quickshell
import qs.Common
import qs.Services
import qs.Widgets
import qs.DCommon.Widgets as DCommon
import qs.Modules.Plugins

// Workspaces of a waydesk session, next to the shell's own workspace switcher (which switches between the ones that exist but cannot
// add one on sway): "+" makes a new workspace and goes to it, the arrow opens the list of workspaces with their windows (minimized ones dimmed, a click brings one back). Click a window
// to go to it; the small buttons under it send it to another workspace (or to a new one). All of it goes through
// `waydesk-windows` (sway's own IPC), nothing of the shell's internals.
PluginComponent {
    id: root

    // [{name, focused, windows: [{id, title, app, focused}]}], read when the list is opened and after every action.
    property var workspaces: []

    function label(name) {
        return name === "Remote" ? "1" : name;
    }

    function refresh() {
        Proc.runCommand("swayWorkspaces.list", ["waydesk-windows", "workspaces"], function (out, code) {
            if (code !== 0)
                return;
            try {
                root.workspaces = JSON.parse(out);
            } catch (e) {
                root.workspaces = [];
            }
        }, 0);
    }

    function run(args, closeList) {
        Proc.runCommand("swayWorkspaces." + args[0], ["waydesk-windows"].concat(args), function () {
            root.refresh();
        }, 0);
        if (closeList)
            root.closePopout();
    }

    Component.onCompleted: refresh()

    horizontalBarPill: Component {
        Row {
            spacing: 2

            Item {
                width: 22
                height: 22
                anchors.verticalCenter: parent.verticalCenter

                DCommon.DIcon {
                    anchors.centerIn: parent
                    name: "add"
                    size: 18
                    color: addArea.containsMouse ? Theme.surfaceText : Theme.surfaceVariantText
                }

                MouseArea {
                    id: addArea
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: root.run(["workspace-new"], false)
                }
            }

            Item {
                width: 18
                height: 22
                anchors.verticalCenter: parent.verticalCenter

                DCommon.DIcon {
                    anchors.centerIn: parent
                    name: "expand_more"
                    size: 16
                    color: listArea.containsMouse ? Theme.surfaceText : Theme.surfaceVariantText
                }

                MouseArea {
                    id: listArea
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

    popoutWidth: 380
    popoutHeight: 0

    popoutContent: Component {
        PopoutComponent {
            headerText: I18n.tr("Workspaces")
            showCloseButton: true

            Column {
                width: parent.width
                spacing: Theme.spacingS

                Repeater {
                    model: root.workspaces

                    Column {
                        id: wsBlock
                        required property var modelData
                        width: parent.width
                        spacing: 2

                        // the workspace: click to go there
                        Rectangle {
                            width: parent.width
                            height: 32
                            radius: Theme.cornerRadius
                            color: wsArea.containsMouse ? Theme.surfaceContainerHighest : (wsBlock.modelData.focused ? Theme.surfaceContainerHigh : "transparent")

                            Row {
                                anchors.fill: parent
                                anchors.leftMargin: Theme.spacingS
                                spacing: Theme.spacingS

                                StyledText {
                                    anchors.verticalCenter: parent.verticalCenter
                                    text: I18n.tr("Workspace %1").arg(root.label(wsBlock.modelData.name))
                                    font.weight: wsBlock.modelData.focused ? Font.DemiBold : Font.Normal
                                    color: wsBlock.modelData.focused ? Theme.primary : Theme.surfaceText
                                    font.pixelSize: Theme.fontSizeMedium
                                }
                                StyledText {
                                    anchors.verticalCenter: parent.verticalCenter
                                    text: wsBlock.modelData.windows.length === 0 ? I18n.tr("empty") : wsBlock.modelData.windows.length + (wsBlock.modelData.windows.length === 1 ? " window" : " windows")
                                    color: Theme.surfaceVariantText
                                    font.pixelSize: Theme.fontSizeSmall
                                }
                            }

                            MouseArea {
                                id: wsArea
                                anchors.fill: parent
                                hoverEnabled: true
                                cursorShape: Qt.PointingHandCursor
                                onClicked: root.run(["workspace-go", wsBlock.modelData.name], true)
                            }
                        }

                        // its windows: click to go to one; the buttons send it to another workspace
                        Repeater {
                            model: wsBlock.modelData.windows

                            Column {
                                id: winBlock
                                required property var modelData
                                width: wsBlock.width
                                spacing: 0

                                Rectangle {
                                    width: parent.width
                                    height: 30
                                    radius: Theme.cornerRadius
                                    color: winArea.containsMouse ? Theme.surfaceContainerHighest : "transparent"

                                    Row {
                                        anchors.fill: parent
                                        anchors.leftMargin: Theme.spacingL
                                        spacing: Theme.spacingS

                                        Item {
                                            width: 18
                                            height: 18
                                            anchors.verticalCenter: parent.verticalCenter
                                            Image {
                                                id: winIcon
                                                anchors.fill: parent
                                                source: winBlock.modelData.app ? Quickshell.iconPath(DesktopEntries.heuristicLookup(winBlock.modelData.app)?.icon ?? winBlock.modelData.app.toLowerCase(), true) : ""
                                                sourceSize: Qt.size(36, 36)
                                                fillMode: Image.PreserveAspectFit
                                            }
                                            DCommon.DIcon {
                                                anchors.centerIn: parent
                                                visible: winIcon.status !== Image.Ready
                                                name: "web_asset"
                                                size: 16
                                                color: Theme.surfaceVariantText
                                            }
                                        }
                                        StyledText {
                                            anchors.verticalCenter: parent.verticalCenter
                                            width: parent.width - 18 - Theme.spacingL - Theme.spacingS * 2
                                            text: winBlock.modelData.title || winBlock.modelData.app || "?"
                                            elide: Text.ElideRight
                                            color: winBlock.modelData.focused ? Theme.primary : Theme.surfaceText
                                            opacity: winBlock.modelData.minimized ? 0.55 : 1
                                            font.italic: winBlock.modelData.minimized === true
                                            font.pixelSize: Theme.fontSizeMedium
                                        }
                                    }

                                    MouseArea {
                                        id: winArea
                                        anchors.fill: parent
                                        hoverEnabled: true
                                        cursorShape: Qt.PointingHandCursor
                                        onClicked: root.run([winBlock.modelData.minimized ? "restore" : "focus", String(winBlock.modelData.id)], true)
                                    }
                                }

                                Row {
                                    visible: winBlock.modelData.minimized !== true
                                    leftPadding: Theme.spacingL + 18 + Theme.spacingS
                                    spacing: 4
                                    bottomPadding: 4

                                    StyledText {
                                        anchors.verticalCenter: parent.verticalCenter
                                        text: I18n.tr("Move to")
                                        color: Theme.surfaceVariantText
                                        font.pixelSize: Theme.fontSizeSmall
                                    }

                                    Repeater {
                                        model: root.workspaces.filter(w => w.name !== wsBlock.modelData.name)

                                        Rectangle {
                                            id: chip
                                            required property var modelData
                                            width: chipText.implicitWidth + 14
                                            height: 22
                                            radius: 11
                                            color: chipArea.containsMouse ? Theme.primary : Theme.surfaceContainerHighest

                                            StyledText {
                                                id: chipText
                                                anchors.centerIn: parent
                                                text: root.label(chip.modelData.name)
                                                color: chipArea.containsMouse ? Theme.primaryText : Theme.surfaceText
                                                font.pixelSize: Theme.fontSizeSmall
                                            }
                                            MouseArea {
                                                id: chipArea
                                                anchors.fill: parent
                                                hoverEnabled: true
                                                cursorShape: Qt.PointingHandCursor
                                                onClicked: root.run(["move-to", String(winBlock.modelData.id), chip.modelData.name], false)
                                            }
                                        }
                                    }

                                    Rectangle {
                                        id: newChip
                                        visible: root.workspaces.length < 4
                                        width: newText.implicitWidth + 14
                                        height: 22
                                        radius: 11
                                        color: newArea.containsMouse ? Theme.primary : Theme.surfaceContainerHighest

                                        StyledText {
                                            id: newText
                                            anchors.centerIn: parent
                                            text: "+ " + I18n.tr("new")
                                            color: newArea.containsMouse ? Theme.primaryText : Theme.surfaceText
                                            font.pixelSize: Theme.fontSizeSmall
                                        }
                                        MouseArea {
                                            id: newArea
                                            anchors.fill: parent
                                            hoverEnabled: true
                                            cursorShape: Qt.PointingHandCursor
                                            onClicked: root.run(["move-to", String(winBlock.modelData.id), "new"], true)
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                Rectangle {
                    visible: root.workspaces.length < 4
                    width: parent.width
                    height: 34
                    radius: Theme.cornerRadius
                    color: addNewArea.containsMouse ? Theme.surfaceContainerHighest : Theme.surfaceContainerHigh

                    Row {
                        anchors.centerIn: parent
                        spacing: Theme.spacingS
                        DCommon.DIcon { anchors.verticalCenter: parent.verticalCenter; name: "add"; size: 18; color: Theme.surfaceText }
                        StyledText { anchors.verticalCenter: parent.verticalCenter; text: I18n.tr("New workspace"); color: Theme.surfaceText; font.pixelSize: Theme.fontSizeMedium }
                    }
                    MouseArea {
                        id: addNewArea
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: root.run(["workspace-new"], true)
                    }
                }
            }
        }
    }
}
