import QtQuick
import qs.Common
import qs.Services
import qs.Widgets
import qs.DCommon.Widgets as DCommon
import qs.Modules.Plugins

// The shell's settings, one click from the bar.
PluginComponent {
    id: root

    horizontalBarPill: Component {
        DCommon.DIcon {
            name: "settings"
            size: Theme.iconSize - 4
            color: Theme.surfaceText

            MouseArea {
                anchors.fill: parent
                anchors.margins: -4
                cursorShape: Qt.PointingHandCursor
                onClicked: PopoutService.focusOrToggleSettings()
            }
        }
    }
}
