# A patch of the DMS shell for waydesk, exit status 1 when the file is not what it expects (another DMS version): nothing is written then.
# Settings pages with nothing to set on a remote desktop (displays, network, power, input, users, lock screen, plugins ...) are left out of the sidebar. Usage: SRC DST (Common/SettingsTabs.qml).
import json, os, sys
src, dst = sys.argv[1:]
text = open(src).read()
needle = "    function isVisible(entry) {\n        if (!entry)\n            return false;\n"
if needle not in text:
    sys.exit(1)
hidden = [
    "desktop_widgets", "desktop_widget", "wellbeing",
    "displays", "display_config", "display_gamma", "display_widgets",
    "input", "keybinds", "keyboard", "mouse_touchpad",
    "power_battery", "power_sleep", "battery",
    "network", "network_wifi", "network_ethernet", "network_vpn", "network_cellular", "network_status",
    "user_accounts", "users", "user_create", "security_accounts", "lock_screen", "greeter", "greeter_auth",
    "window_rules", "compositor_layout", "multiplexers", "printers", "updater", "updater_changelog",
    "plugins", "plugins_manage",
]
add = "        if (%s.indexOf(entry.id) >= 0)\n            return false;\n" % json.dumps(hidden)
os.remove(dst)
with open(dst, "w") as f:
    f.write(text.replace(needle, needle + add, 1))
