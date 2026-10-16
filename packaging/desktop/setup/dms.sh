#!/usr/bin/env bash
# Installs DankMaterialShell on Debian 13 and starts it with sway. Steps follow danklinux.com (checked 2026-10-16).
# Needs sudo. Safe to run again.
set -euo pipefail
SUDO=""; [ "$(id -u)" -eq 0 ] || SUDO="sudo"

$SUDO apt-get install -y --no-install-recommends curl gnupg ca-certificates
$SUDO mkdir -p /etc/apt/keyrings

# quickshell must come from trixie-backports (the old DankLinux build is deprecated).
if dpkg -s quickshell >/dev/null 2>&1 && apt-cache policy quickshell | grep -q 'opensuse'; then
  $SUDO apt-get remove -y quickshell
fi
echo "deb http://deb.debian.org/debian trixie-backports main" | $SUDO tee /etc/apt/sources.list.d/trixie-backports.list >/dev/null

repo() { # name url
  local name="$1" url="$2"
  [ -f "/etc/apt/keyrings/$name.gpg" ] || curl -fsSL "${url}Release.key" | $SUDO gpg --dearmor -o "/etc/apt/keyrings/$name.gpg"
  echo "deb [signed-by=/etc/apt/keyrings/$name.gpg] $url /" | $SUDO tee "/etc/apt/sources.list.d/$name.list" >/dev/null
}
repo danklinux     https://download.opensuse.org/repositories/home:/AvengeMedia:/danklinux/Debian_13/
repo avengemedia-dms https://download.opensuse.org/repositories/home:/AvengeMedia:/dms/Debian_13/

$SUDO apt-get update
$SUDO apt-get install -y -t trixie-backports quickshell
# qt6-wayland: without it the shell opens as plain windows instead of a bar and wallpaper
$SUDO apt-get install -y dms qt6-wayland

# With sway-rdp-desktop the system config (/etc/xdg/sway-rdp/sway.conf) starts DMS through sway-session.target. Without it,
# set that up in the account's own sway config, as the DMS docs ask for sway.
if [ ! -f /etc/xdg/sway-rdp/sway.conf ]; then
mkdir -p "$HOME/.config/systemd/user" "$HOME/.config/sway"
cat > "$HOME/.config/systemd/user/sway-session.target" <<'EOT'
[Unit]
Description=Sway Session Target
Requires=graphical-session.target
After=graphical-session.target
EOT
SWAYCONF="$HOME/.config/sway/config"
[ -f "$SWAYCONF" ] || cp /etc/sway/config "$SWAYCONF"
if ! grep -qF '# >>> desktop installer: dms' "$SWAYCONF"; then
  cat >> "$SWAYCONF" <<'EOT'

# >>> desktop installer: dms
exec dbus-update-activation-environment --systemd --all
exec systemctl --user start sway-session.target
# <<< desktop installer: dms
EOT
fi
# A plain `dms run` line would start it twice.
sed -i '/^exec dms run/d' "$SWAYCONF"
systemctl --user daemon-reload 2>/dev/null || true
systemctl --user add-wants sway-session.target dms 2>/dev/null || echo "note: run 'systemctl --user add-wants sway-session.target dms' inside your session"
fi
echo "DMS installed. Log out and in (restart sway) to see it."
