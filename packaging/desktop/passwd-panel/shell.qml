// Change password: the account that is logged in changes its own password. Three fields (current, new, again) and the answer.
//
// The work is done by `waydesk-passwd`, which takes the current and the new password on stdin (never as arguments) and lets PAM
// change it, so the system's rules apply. It needs no administrator. Colours come from the Qt palette, which qt6ct keeps in step with DMS.

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Quickshell
import Quickshell.Io

ShellRoot {
    id: root

    property bool working: false
    property bool done: false
    // The last error, or nothing.
    property string notice: ""

    SystemPalette { id: pal }

    Process {
        id: job
        command: ["/usr/bin/waydesk-passwd"]
        stdinEnabled: true
        stderr: StdioCollector { id: err }
        // What goes in once the process is there (the two passwords, one per line).
        property string pending: ""
        onStarted: {
            write(pending);
            pending = "";
        }
        onExited: (code, status) => settle.start()
    }
    // The collector has its text a moment after the process ended.
    Timer {
        id: settle
        interval: 120
        onTriggered: {
            root.working = false;
            if (job.exitCode === 0) {
                root.done = true;
                current.text = newPassword.text = again.text = "";
            } else {
                root.notice = err.text.trim() || "The password was not changed.";
                current.forceActiveFocus();
            }
        }
    }

    function submit() {
        if (working || !canSubmit)
            return;
        notice = "";
        working = true;
        job.pending = current.text + "\n" + newPassword.text + "\n";
        job.running = true;
    }

    readonly property bool canSubmit: current.text.length > 0 && newPassword.text.length >= 8 && newPassword.text === again.text

    FloatingWindow {
        title: "Change password"
        implicitWidth: 440
        implicitHeight: 400
        minimumSize: Qt.size(400, 380)
        maximumSize: Qt.size(560, 520)
        color: pal.window
        visible: true
        onClosed: Qt.quit()

        ColumnLayout {
            anchors.fill: parent
            anchors.margins: 24
            spacing: 12

            Label {
                text: root.done ? "Password changed" : "Change password"
                font.pixelSize: 20
                font.weight: Font.DemiBold
                color: pal.windowText
            }

            // the form
            ColumnLayout {
                visible: !root.done
                Layout.fillWidth: true
                spacing: 10

                Rectangle {
                    visible: root.notice !== ""
                    Layout.fillWidth: true
                    implicitHeight: noticeText.implicitHeight + 20
                    radius: 6
                    color: Qt.rgba(0.9, 0.3, 0.3, 0.18)
                    border.color: Qt.rgba(0.9, 0.3, 0.3, 0.6)
                    Label {
                        id: noticeText
                        anchors.fill: parent
                        anchors.margins: 10
                        text: root.notice
                        wrapMode: Text.Wrap
                        color: pal.windowText
                    }
                }

                Label { text: "Current password"; color: pal.windowText }
                TextField {
                    id: current
                    Layout.fillWidth: true
                    echoMode: TextInput.Password
                    enabled: !root.working
                    focus: true
                    onAccepted: newPassword.forceActiveFocus()
                }
                Label { text: "New password"; color: pal.windowText }
                TextField {
                    id: newPassword
                    Layout.fillWidth: true
                    echoMode: TextInput.Password
                    enabled: !root.working
                    onAccepted: again.forceActiveFocus()
                }
                Label { text: "New password again"; color: pal.windowText }
                TextField {
                    id: again
                    Layout.fillWidth: true
                    echoMode: TextInput.Password
                    enabled: !root.working
                    onAccepted: root.submit()
                }
                Label {
                    text: newPassword.text.length > 0 && newPassword.text.length < 8 ? "At least 8 characters."
                        : (again.text.length > 0 && again.text !== newPassword.text ? "The two new passwords are not the same." : " ")
                    color: pal.placeholderText
                    font.pixelSize: 12
                }
            }

            // the answer
            Label {
                visible: root.done
                text: "Your new password is in use from now on. It applies to the next login over RDP; the session that is open keeps running."
                wrapMode: Text.Wrap
                color: pal.windowText
                Layout.fillWidth: true
            }

            Item { Layout.fillHeight: true }

            RowLayout {
                Layout.alignment: Qt.AlignRight
                Button { text: root.done ? "Close" : "Cancel"; onClicked: Qt.quit() }
                Button {
                    visible: !root.done
                    text: root.working ? "Changing…" : "Change password"
                    highlighted: true
                    enabled: root.canSubmit && !root.working
                    onClicked: root.submit()
                }
            }
        }
    }
}
