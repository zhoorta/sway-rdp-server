import QtQuick
import qs.Common
import qs.Services
import qs.Widgets
import qs.Modules.Plugins

// The session's volume without a panel: click mutes, the wheel changes it.
PluginComponent {
    id: root

    horizontalBarPill: Component {
        DankIcon {
            name: AudioService.sinkVolumeIconName
            size: Theme.iconSize - 4
            color: AudioService.sinkSilent ? Theme.surfaceVariantText : Theme.surfaceText

            MouseArea {
                anchors.fill: parent
                anchors.margins: -4
                cursorShape: Qt.PointingHandCursor
                onClicked: AudioService.toggleMute()
                onWheel: function (wheel) {
                    if (wheel.angleDelta.y !== 0 && AudioService.sink?.audio)
                        AudioService.adjustDefaultSinkVolume(5, wheel.angleDelta.y > 0 ? 1 : -1);
                }
            }
        }
    }
}
