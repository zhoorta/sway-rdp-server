// Users & Access: who may log in over RDP, who is an administrator, the sessions that are running and what they use.
//
// The list is read with `waydesk-admin users list --json` and the machine and the sessions with `waydesk-admin status --json`
// (both open to everybody; this window is for administrators: the launcher entry is hidden from the others); every change goes through
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
    // Memory, swap, load, whether a new session would be accepted, and what each account uses (waydesk-admin status --json).
    property var status: ({})
    property string sheetLimitMemory: ""
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

    function refreshStatus() {
        run(["/usr/bin/waydesk-admin", "status", "--json"], (code, out, err) => {
            if (code !== 0)
                return;
            try {
                root.status = JSON.parse(out);
            } catch (e) {
            }
        });
    }

    function usageOf(name) {
        const list = root.status.users || [];
        for (let i = 0; i < list.length; i++)
            if (list[i].name === name)
                return list[i];
        return null;
    }

    function mb(n) {
        return n >= 1024 ? (n / 1024).toFixed(1) + " GB" : n + " MB";
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

    Component.onCompleted: { refresh(); refreshStatus(); }

    Timer {
        interval: 8000
        repeat: true
        running: root.sheet === "" && !root.working
        onTriggered: { root.refresh(); root.refreshStatus(); }
    }

    // ---- the window ----

    FloatingWindow {
        id: window
        title: "Users & Access"
        implicitWidth: 760
        implicitHeight: 520
        minimumSize: Qt.size(560, 360)
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
                Button { text: "Refresh"; onClicked: { root.refresh(); root.refreshStatus(); } }
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

            // the machine in one line: memory, and whether a new session would be accepted (waydesk-admin status has the rest)
            RowLayout {
                id: machine
                readonly property var m: root.status.machine || null
                readonly property var a: root.status.admission || null
                readonly property real used: m ? (m.mem_total_mb - m.mem_available_mb) / m.mem_total_mb : 0
                visible: m !== null
                Layout.fillWidth: true
                spacing: 12

                Label {
                    text: machine.m ? "Memory  " + root.mb(machine.m.mem_total_mb - machine.m.mem_available_mb) + " of " + root.mb(machine.m.mem_total_mb) + " in use" : ""
                    color: pal.text
                }
                Rectangle {
                    Layout.preferredWidth: 120
                    implicitHeight: 6
                    radius: 3
                    color: pal.mid
                    Rectangle {
                        width: parent.width * Math.min(1, machine.used)
                        height: parent.height
                        radius: 3
                        color: machine.used > 0.85 ? "#e07a5f" : (machine.used > 0.7 ? "#f2cc8f" : pal.highlight)
                    }
                }
                Item { Layout.fillWidth: true }
                Label {
                    text: machine.a ? (machine.a.open ? "New sessions: accepted" : "New sessions: refused (" + machine.a.reason + ")") : ""
                    color: machine.a && !machine.a.open ? "#e07a5f" : pal.placeholderText
                }
            }

            ListView {
                id: list
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                spacing: 6
                model: root.users

                // one line per account: who, what it is doing, and one menu with everything that can be done to it
                delegate: Rectangle {
                    id: card
                    required property var modelData
                    readonly property bool mine: modelData.name === root.me
                    readonly property bool running: modelData.session === "running"
                    // what the account uses now (null when it has nothing running)
                    readonly property var use: root.usageOf(modelData.name)
                    width: list.width
                    height: 62
                    radius: 10
                    color: rowHover.hovered ? Qt.lighter(pal.base, 1.07) : pal.base
                    border.color: pal.mid

                    HoverHandler { id: rowHover }

                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: 14
                        anchors.rightMargin: 10
                        spacing: 12

                        Rectangle {
                            Layout.preferredWidth: 36
                            Layout.preferredHeight: 36
                            radius: 18
                            color: card.modelData.sudo ? pal.highlight : pal.mid
                            Label {
                                anchors.centerIn: parent
                                text: card.modelData.name.charAt(0).toUpperCase()
                                color: card.modelData.sudo ? pal.highlightedText : pal.text
                                font.weight: Font.DemiBold
                                font.pixelSize: 15
                            }
                        }

                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 1

                            RowLayout {
                                spacing: 8
                                Label {
                                    text: card.modelData.name + (card.mine ? "  (you)" : "")
                                    font.weight: Font.DemiBold
                                    font.pixelSize: 14
                                    color: pal.text
                                    elide: Text.ElideRight
                                }
                                Rectangle {
                                    visible: card.modelData.sudo
                                    implicitWidth: adminChip.implicitWidth + 12
                                    implicitHeight: 18
                                    radius: 9
                                    color: Qt.rgba(pal.highlight.r, pal.highlight.g, pal.highlight.b, 0.22)
                                    Label { id: adminChip; anchors.centerIn: parent; text: "Administrator"; font.pixelSize: 10; color: pal.text }
                                }
                                Rectangle {
                                    visible: !card.modelData.rdp || !card.modelData.login_shell
                                    implicitWidth: blockedChip.implicitWidth + 12
                                    implicitHeight: 18
                                    radius: 9
                                    color: Qt.rgba(0.88, 0.48, 0.37, 0.25)
                                    Label { id: blockedChip; anchors.centerIn: parent; text: "Cannot log in"; font.pixelSize: 10; color: pal.text }
                                }
                            }

                            Label {
                                id: statusLine
                                Layout.fillWidth: true
                                elide: Text.ElideRight
                                font.pixelSize: 11
                                color: card.running ? pal.highlight : pal.placeholderText
                                text: !card.modelData.login_shell ? "No login shell"
                                    : card.running ? "Session running" + (card.use ? "  ·  " + root.mb(card.use.mem_mb) + "  ·  CPU " + Math.round(card.use.cpu_percent) + "%" : "")
                                    : "No session"
                                HoverHandler { id: statusHover }
                                ToolTip.visible: statusHover.hovered && card.use !== null && card.use.top.length > 0
                                ToolTip.delay: 400
                                ToolTip.text: card.use ? "Most memory: " + card.use.top.map(t => t.name + " " + root.mb(t.mb)).join(", ") + (card.use.cap_mb ? "\nLimit: " + root.mb(card.use.cap_mb) : "") : ""
                            }
                        }

                        Button {
                            id: optionsButton
                            text: "Options  \u25BE"
                            enabled: !root.working
                            onClicked: rowMenu.popup(optionsButton, optionsButton.width - rowMenu.width, optionsButton.height)

                            Menu {
                                id: rowMenu
                                // wide enough for the longest entry, and opened under the button with its right edge on the button's
                                width: 290

                                MenuItem {
                                    text: "End session"
                                    visible: card.running
                                    height: visible ? implicitHeight : 0
                                    onTriggered: root.change(["sessions", "end", card.modelData.name])
                                }
                                MenuItem {
                                    text: "New password"
                                    onTriggered: root.change(["users", "passwd", card.modelData.name, "--json"], (out) => root.showPassword("New password for " + card.modelData.name, out))
                                }
                                MenuItem {
                                    text: "Limits\u2026"
                                    onTriggered: { root.sheetUser = card.modelData.name; limitMemory.text = ""; limitCpu.value = 100; root.sheet = "limit"; }
                                }
                                MenuSeparator {}
                                MenuItem {
                                    text: "Can log in"
                                    checkable: true
                                    checked: card.modelData.rdp
                                    enabled: !card.mine
                                    onTriggered: root.change(["users", checked ? "enable" : "disable", card.modelData.name])
                                }
                                MenuItem {
                                    text: "Administrator"
                                    checkable: true
                                    checked: card.modelData.sudo
                                    enabled: !card.mine
                                    onTriggered: root.change(["users", "sudo", card.modelData.name, checked ? "on" : "off"])
                                }
                                MenuItem {
                                    text: "Install apps without a password"
                                    checkable: true
                                    checked: card.modelData.flatpak_admin
                                    onTriggered: root.change(["users", "admin", card.modelData.name, checked ? "on" : "off"])
                                }
                                MenuSeparator {}
                                MenuItem {
                                    text: "Remove\u2026"
                                    enabled: !card.mine
                                    onTriggered: { root.sheetUser = card.modelData.name; removeHome.checked = false; root.sheet = "remove"; }
                                }
                            }
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

                    // limits of one account
                    ColumnLayout {
                        visible: root.sheet === "limit"
                        spacing: 10
                        Label { text: "Limits of " + root.sheetUser; font.pixelSize: 18; font.weight: Font.DemiBold; color: pal.windowText }
                        Label {
                            text: "Ceilings, not reservations: they only matter when the account asks for more. By default no account can take more than half of the machine's memory; give one that needs more (VS Code with builds, big documents) a higher ceiling."
                            wrapMode: Text.Wrap
                            color: pal.placeholderText
                            Layout.fillWidth: true
                        }
                        TextField {
                            id: limitMemory
                            Layout.fillWidth: true
                            placeholderText: "memory ceiling, for example 6G or 800M (empty: leave as it is)"
                            validator: RegularExpressionValidator { regularExpression: /^[1-9][0-9]{0,5}[MGT]$/ }
                        }
                        RowLayout {
                            Label { text: "CPU weight"; color: pal.windowText }
                            SpinBox { id: limitCpu; from: 1; to: 10000; value: 100; editable: true }
                            Label { text: "100 is the same as everybody; 200 gets twice the share when the cores are busy"; color: pal.placeholderText; font.pixelSize: 11; wrapMode: Text.Wrap; Layout.fillWidth: true }
                        }
                        RowLayout {
                            Layout.alignment: Qt.AlignRight
                            Button { text: "Cancel"; onClicked: root.sheet = "" }
                            Button {
                                text: "Reset to default"
                                onClicked: { const u = root.sheetUser; root.sheet = ""; root.change(["users", "limit", u, "reset"]); }
                            }
                            Button {
                                text: "Apply"
                                highlighted: true
                                enabled: (limitMemory.text === "" || limitMemory.acceptableInput) && !root.working
                                onClicked: {
                                    const args = ["users", "limit", root.sheetUser];
                                    if (limitMemory.text !== "") { args.push("--memory"); args.push(limitMemory.text); }
                                    args.push("--cpu-weight"); args.push(String(limitCpu.value));
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
