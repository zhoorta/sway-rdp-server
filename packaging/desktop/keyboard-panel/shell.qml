// Keyboard: the layout of the keys this account types with.
//
// The session follows the layout the client reports (Windows locale) unless the account names one in ~/.config/waydesk/keyboard, a line with an
// xkb layout name. This window writes that file (or removes it for "Automatic") and the session picks the change up within a second, so there is
// nothing to restart. Colours come from the Qt palette, which qt6ct keeps in step with DMS.

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Quickshell
import Quickshell.Io

ShellRoot {
    id: root

    // What the list offers: the label, the xkb layout name that goes in the file ("" = no file: follow the client) and a word of help.
    readonly property var layouts: [
        { text: "Automatic (the keyboard of the computer you connect from)", value: "", help: "The layout your RDP client reports. Right for a PC or a Mac with the layout that matches its keys." },
        { text: "Portuguese, MacBook built-in keyboard", value: "waydeskmac", help: "For a MacBook's own keyboard through the Windows App: its keys reach the server at other places than a PC's, and the left Option key is AltGr." },
        { text: "Portuguese", value: "pt", help: "" },
        { text: "Portuguese (Brazil)", value: "br", help: "" },
        { text: "English (US)", value: "us", help: "" },
        { text: "English (UK)", value: "gb", help: "" },
        { text: "French", value: "fr", help: "" },
        { text: "German", value: "de", help: "" },
        { text: "Spanish", value: "es", help: "" },
        { text: "Italian", value: "it", help: "" },
        { text: "Danish", value: "dk", help: "" },
        { text: "Swedish", value: "se", help: "" },
        { text: "Norwegian", value: "no", help: "" },
        { text: "Finnish", value: "fi", help: "" }
    ]
    property bool applied: false

    SystemPalette { id: pal }

    // The layout in the file now.
    Process {
        id: reader
        command: ["sh", "-c", "grep -vE '^[[:space:]]*(#|$)' \"$HOME/.config/waydesk/keyboard\" 2>/dev/null | head -1"]
        stdout: StdioCollector { id: current }
        running: true
        onExited: {
            const name = current.text.trim();
            let at = 0;
            for (let i = 0; i < root.layouts.length; i++) {
                if (root.layouts[i].value === name)
                    at = i;
            }
            choice.currentIndex = at;
        }
    }

    Process { id: writer }

    function apply(value) {
        writer.command = value === ""
            ? ["sh", "-c", "rm -f \"$HOME/.config/waydesk/keyboard\""]
            : ["sh", "-c", "mkdir -p \"$HOME/.config/waydesk\" && printf '# set in Keyboard settings\\n%s\\n' \"$1\" > \"$HOME/.config/waydesk/keyboard\"", "sh", value];
        writer.running = true;
        applied = true;
    }

    FloatingWindow {
        title: "Keyboard"
        implicitWidth: 480
        implicitHeight: 400
        minimumSize: Qt.size(440, 380)
        maximumSize: Qt.size(620, 520)
        color: pal.window
        visible: true
        onClosed: Qt.quit()

        ColumnLayout {
            anchors.fill: parent
            anchors.margins: 24
            spacing: 12

            Label {
                text: "Keyboard"
                font.pixelSize: 20
                font.weight: Font.DemiBold
                color: pal.windowText
            }
            Label {
                text: "The layout of the keys you type with. It applies to this session at once."
                wrapMode: Text.Wrap
                color: pal.windowText
                Layout.fillWidth: true
            }

            ComboBox {
                id: choice
                Layout.fillWidth: true
                model: root.layouts
                textRole: "text"
                onActivated: index => root.apply(root.layouts[index].value)
            }
            Label {
                text: root.layouts[choice.currentIndex >= 0 ? choice.currentIndex : 0].help
                visible: text !== ""
                wrapMode: Text.Wrap
                color: pal.placeholderText
                font.pixelSize: 12
                Layout.fillWidth: true
            }
            Label {
                visible: root.applied
                text: "Changed. Try your keys below."
                color: pal.windowText
                font.pixelSize: 12
            }

            Label { text: "Try your keys here"; color: pal.windowText }
            TextField {
                Layout.fillWidth: true
                placeholderText: "~ ^ \\ | { } [ ] @ € º ª ´ `"
            }

            Item { Layout.fillHeight: true }

            RowLayout {
                Layout.alignment: Qt.AlignRight
                Button { text: "Close"; onClicked: Qt.quit() }
            }
        }
    }
}
