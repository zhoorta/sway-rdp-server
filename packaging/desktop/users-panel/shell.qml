// Users & Access: who may log in over RDP, who is an administrator, and the sessions that are running.
//
// The list is read with `waydesk-admin users list --json` (open to everybody); every change goes through
// `pkexec waydesk-admin ...`, which asks an administrator's password in the shell's own dialog (polkit action
// org.waydesk.admin). All the rules (who may be touched, nobody removing themselves or the last administrator) are in that
// command, not here: this window only shows and asks. Colours come from the Qt palette, which qt6ct keeps in step with DMS.

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Quickshell
import Quickshell.Io

ShellRoot {
    id: root

    property var users: []
    // What the window shows at the top: an error of the last action, or nothing.
    property string notice: ""
    property bool working: false
    readonly property string me: Quickshell.env("USER") || ""

    // The sheet that is open (an overlay inside the window): "", "add", "remove", "password".
    property string sheet: ""
    property string sheetUser: ""
    property string sheetPassword: ""
    property string sheetPasswordLabel: ""

    SystemPalette { id: pal }

    // ---- running commands ----

    Component {
        id: jobComponent
        Process {
            id: job
            property var finished
            property int resultCode: 0
            stdout: StdioCollector { id: so }
            stderr: StdioCollector { id: se }
            onExited: (code, status) => {
                job.resultCode = code;
                settle.start();
            }
            // The collectors have their text a moment after the process ended.
            property Timer settle: Timer {
                interval: 120
                onTriggered: {
                    job.finished(job.resultCode, so.text, se.text);
                    job.destroy();
                }
            }
            Component.onCompleted: running = true
        }
    }

    function run(args, done) {
        jobComponent.createObject(root, { command: args, finished: done });
    }

    // Runs a change as an administrator and refreshes the list. `then` gets the output when it worked.
    function change(args, then) {
        if (root.working)
            return;
        root.working = true;
        root.notice = "";
        run(["pkexec", "/usr/bin/waydesk-admin"].concat(args), (code, out, err) => {
            root.working = false;
            if (code !== 0) {
                // 126 and 127 are pkexec itself: the dialog was cancelled or the password was wrong.
                root.notice = code === 126 || code === 127 ? "Cancelled: no administrator password was given." : (err.trim() || out.trim() || ("failed with code " + code));
            } else if (then) {
                then(out);
            }
            refresh();
        });
    }

    function refresh() {
        run(["/usr/bin/waydesk-admin", "users", "list", "--json"], (code, out, err) => {
            if (code !== 0) {
                root.notice = err.trim() || "Cannot read the list of accounts";
                return;
            }
            try {
                root.users = JSON.parse(out);
            } catch (e) {
                root.notice = "Unexpected answer from waydesk-admin";
            }
        });
    }

    function showPassword(label, json) {
        try {
            root.sheetPassword = JSON.parse(json).password;
            root.sheetPasswordLabel = label;
            root.sheet = "password";
        } catch (e) {
            root.notice = "Unexpected answer from waydesk-admin";
        }
    }

    Component.onCompleted: refresh()

    Timer {
        interval: 8000
        repeat: true
        running: root.sheet === "" && !root.working
        onTriggered: root.refresh()
    }

    // ---- the window ----

    FloatingWindow {
        id: window
        title: "Users & Access"
        implicitWidth: 880
        implicitHeight: 560
        minimumSize: Qt.size(700, 400)
        color: pal.window
        visible: true
        onClosed: Qt.quit()

        ColumnLayout {
            anchors.fill: parent
            anchors.margins: 18
            spacing: 12

            RowLayout {
                Layout.fillWidth: true
                Label {
                    text: "Users & Access"
                    font.pixelSize: 22
                    font.weight: Font.DemiBold
                    color: pal.windowText
                    Layout.fillWidth: true
                }
                Button { text: "Refresh"; onClicked: root.refresh() }
                Button {
                    text: "Add account…"
                    highlighted: true
                    onClicked: { addName.text = ""; addRdp.checked = true; addFlatpak.checked = false; addSudo.checked = false; root.sheet = "add"; addName.forceActiveFocus(); }
                }
            }

            Label {
                text: "Changes ask for an administrator's password."
                color: pal.placeholderText
                Layout.fillWidth: true
            }

            Rectangle {
                visible: root.notice !== ""
                Layout.fillWidth: true
                implicitHeight: noticeText.implicitHeight + 20
                radius: 10
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

            ListView {
                id: list
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                spacing: 8
                model: root.users

                delegate: Rectangle {
                    id: card
                    required property var modelData
                    readonly property bool mine: modelData.name === root.me
                    width: list.width
                    height: 104
                    radius: 12
                    color: pal.base
                    border.color: pal.mid

                    ColumnLayout {
                        anchors.fill: parent
                        anchors.leftMargin: 16
                        anchors.rightMargin: 12
                        anchors.topMargin: 8
                        anchors.bottomMargin: 4
                        spacing: 0

                        // who, the session, and what can be done to the account
                        RowLayout {
                            Layout.fillWidth: true
                            spacing: 10

                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: 0
                                Label {
                                    text: card.modelData.name + (card.mine ? "  (you)" : "")
                                    font.weight: Font.DemiBold
                                    font.pixelSize: 15
                                    color: pal.text
                                    elide: Text.ElideRight
                                    Layout.fillWidth: true
                                }
                                Label {
                                    text: card.modelData.login_shell ? "uid " + card.modelData.uid : "no login shell: cannot log in"
                                    color: card.modelData.login_shell ? pal.placeholderText : "#e07a5f"
                                    font.pixelSize: 11
                                }
                            }

                            ColumnLayout {
                                Layout.preferredWidth: 170
                                spacing: 0
                                Label {
                                    text: card.modelData.session === "running" ? "Session running" : "No session"
                                    color: card.modelData.session === "running" ? pal.highlight : pal.placeholderText
                                    font.weight: card.modelData.session === "running" ? Font.DemiBold : Font.Normal
                                }
                                Label {
                                    visible: card.modelData.session === "running"
                                    text: "since " + card.modelData.session_since
                                    color: pal.placeholderText
                                    font.pixelSize: 10
                                    elide: Text.ElideRight
                                    Layout.fillWidth: true
                                }
                            }

                            Button {
                                text: "End session"
                                visible: card.modelData.session === "running"
                                enabled: !root.working
                                onClicked: root.change(["sessions", "end", card.modelData.name])
                            }
                            Button {
                                text: "New password"
                                enabled: !root.working
                                onClicked: root.change(["users", "passwd", card.modelData.name, "--json"], (out) => root.showPassword("New password for " + card.modelData.name, out))
                            }
                            Button {
                                text: "Remove…"
                                enabled: !root.working && !card.mine
                                onClicked: { root.sheetUser = card.modelData.name; removeHome.checked = false; root.sheet = "remove"; }
                            }
                        }

                        // what the account may do
                        RowLayout {
                            Layout.fillWidth: true
                            spacing: 24
                            Switch {
                                text: "Can log in"
                                checked: card.modelData.rdp
                                enabled: !root.working && !card.mine
                                onToggled: root.change(["users", checked ? "enable" : "disable", card.modelData.name])
                            }
                            Switch {
                                text: "Flatpak installs without a password"
                                checked: card.modelData.flatpak_admin
                                enabled: !root.working
                                onToggled: root.change(["users", "admin", card.modelData.name, checked ? "on" : "off"])
                            }
                            Switch {
                                text: "Administrator"
                                checked: card.modelData.sudo
                                enabled: !root.working && !card.mine
                                onToggled: root.change(["users", "sudo", card.modelData.name, checked ? "on" : "off"])
                            }
                            Item { Layout.fillWidth: true }
                        }
                    }
                }

                Label {
                    anchors.centerIn: parent
                    visible: root.users.length === 0
                    text: "No accounts yet. Add one."
                    color: pal.placeholderText
                }
            }
        }

        // ---- sheets: one question at a time, over the window ----

        Item {
            anchors.fill: parent
            visible: root.sheet !== ""

            Rectangle { anchors.fill: parent; color: "#000000"; opacity: 0.5 }
            MouseArea { anchors.fill: parent; onClicked: {} }

            Rectangle {
                anchors.centerIn: parent
                width: 460
                height: sheetColumn.implicitHeight + 40
                radius: 14
                color: pal.window
                border.color: pal.mid

                ColumnLayout {
                    id: sheetColumn
                    anchors.fill: parent
                    anchors.margins: 20
                    spacing: 12

                    // add an account
                    ColumnLayout {
                        visible: root.sheet === "add"
                        spacing: 10
                        Label { text: "Add account"; font.pixelSize: 18; font.weight: Font.DemiBold; color: pal.windowText }
                        TextField {
                            id: addName
                            Layout.fillWidth: true
                            placeholderText: "name (lower case letters, digits, _ and -)"
                            validator: RegularExpressionValidator { regularExpression: /^[a-z_][a-z0-9_-]{0,31}$/ }
                            onAccepted: createButton.clicked()
                        }
                        CheckBox { id: addRdp; text: "Can log in over RDP"; checked: true }
                        CheckBox { id: addFlatpak; text: "Can install Flatpaks without a password" }
                        CheckBox { id: addSudo; text: "Administrator (sudo)" }
                        RowLayout {
                            Layout.alignment: Qt.AlignRight
                            Button { text: "Cancel"; onClicked: root.sheet = "" }
                            Button {
                                id: createButton
                                text: "Create"
                                highlighted: true
                                enabled: addName.acceptableInput && !root.working
                                onClicked: {
                                    if (!enabled)
                                        return;
                                    const name = addName.text;
                                    const args = ["users", "add", name, "--json"];
                                    if (!addRdp.checked) args.push("--no-rdp");
                                    if (addFlatpak.checked) args.push("--admin");
                                    if (addSudo.checked) args.push("--sudo");
                                    root.sheet = "";
                                    root.change(args, (out) => root.showPassword("Temporary password for " + name, out));
                                }
                            }
                        }
                    }

                    // remove an account
                    ColumnLayout {
                        visible: root.sheet === "remove"
                        spacing: 10
                        Label { text: "Remove " + root.sheetUser + "?"; font.pixelSize: 18; font.weight: Font.DemiBold; color: pal.windowText }
                        Label {
                            text: "Its session is ended and the account is deleted. Unsaved work in it is lost."
                            wrapMode: Text.Wrap
                            color: pal.windowText
                            Layout.fillWidth: true
                        }
                        CheckBox { id: removeHome; text: "Also delete its home folder (all its files)" }
                        RowLayout {
                            Layout.alignment: Qt.AlignRight
                            Button { text: "Cancel"; onClicked: root.sheet = "" }
                            Button {
                                text: "Remove"
                                highlighted: true
                                onClicked: {
                                    const args = ["users", "remove", root.sheetUser];
                                    if (removeHome.checked) args.push("--delete-home");
                                    root.sheet = "";
                                    root.change(args);
                                }
                            }
                        }
                    }

                    // a password, shown once
                    ColumnLayout {
                        visible: root.sheet === "password"
                        spacing: 10
                        Label { text: root.sheetPasswordLabel; font.pixelSize: 18; font.weight: Font.DemiBold; color: pal.windowText; wrapMode: Text.Wrap; Layout.fillWidth: true }
                        TextField {
                            id: passwordText
                            Layout.fillWidth: true
                            readOnly: true
                            text: root.sheetPassword
                            font.family: "monospace"
                            font.pixelSize: 17
                            horizontalAlignment: TextInput.AlignHCenter
                        }
                        Label {
                            text: "Shown once. The account can change it in a terminal on the desktop with: passwd"
                            wrapMode: Text.Wrap
                            color: pal.placeholderText
                            Layout.fillWidth: true
                        }
                        RowLayout {
                            Layout.alignment: Qt.AlignRight
                            Button { text: "Copy"; onClicked: { passwordText.selectAll(); passwordText.copy(); } }
                            Button { text: "Done"; highlighted: true; onClicked: { root.sheet = ""; root.sheetPassword = ""; } }
                        }
                    }
                }
            }
        }
    }
}
