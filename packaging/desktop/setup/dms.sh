#!/usr/bin/env bash
# Installs DankMaterialShell on Debian 13 and starts it with sway. Steps follow danklinux.com (checked 2026-10-16).
# Needs sudo. Safe to run again.
set -euo pipefail
SUDO=""; [ "$(id -u)" -eq 0 ] || SUDO="sudo"

$SUDO apt-get install -y --no-install-recommends curl gnupg ca-certificates
$SUDO mkdir -p /etc/apt/keyrings

# quickshell must come from trixie-backports (the old DankLinux build is deprecated). The installed build is the one that counts: a
# backports version has "~bpo" in it. (Looking for the DankLinux repository in `apt-cache policy` removed a good, held build once that
# repository also listed a newer quickshell.)
if dpkg -s quickshell >/dev/null 2>&1 && ! dpkg-query -W -f='${Version}' quickshell | grep -q '~bpo'; then
  $SUDO apt-mark unhold quickshell >/dev/null 2>&1 || true
  $SUDO apt-get remove -y quickshell
fi
echo "deb http://deb.debian.org/debian trixie-backports main" | $SUDO tee /etc/apt/sources.list.d/trixie-backports.list >/dev/null

repo() { # name url
  local name="$1" url="$2"
  [ -f "/etc/apt/keyrings/$name.gpg" ] || curl -fsSL "${url}Release.key" | $SUDO gpg --dearmor -o "/etc/apt/keyrings/$name.gpg"
  echo "deb [signed-by=/etc/apt/keyrings/$name.gpg] $url /" | $SUDO tee "/etc/apt/sources.list.d/$name.list" >/dev/null
}
# With the waydesk repository configured (/etc/apt/sources.list.d/waydesk.sources, packaging/repo), the shell and its helpers (danksearch,
# dgop, matugen) come from there, built by us (docs/DMS-PACKAGE.md), and the third-party repositories are not added. Only quickshell still
# comes from Debian (trixie-backports).
OWN_REPO=
[ -f /etc/apt/sources.list.d/waydesk.sources ] && OWN_REPO=1
if [ -z "$OWN_REPO" ]; then
repo danklinux     https://download.opensuse.org/repositories/home:/AvengeMedia:/danklinux/Debian_13/
repo avengemedia-dms https://download.opensuse.org/repositories/home:/AvengeMedia:/dms/Debian_13/
fi

$SUDO apt-get update
# Only when it is missing: an installed quickshell is held with DMS (below) and moves with it (WAYDESK_DMS_UPDATE=1); asking for the
# newest one of trixie-backports on every run failed once backports had a newer build that needs other new libraries.
dpkg -s quickshell >/dev/null 2>&1 || $SUDO apt-get install -y -t trixie-backports quickshell
# qt6-wayland: without it the shell opens as plain windows instead of a bar and wallpaper
# DMS is pinned: its repository is not versioned by us and a new major release moves files the shell patches (waydesk-shell) and
# the plugins rely on, which broke running sessions once (1.6.3 -> 2.0.0). The tested version is installed and held; WAYDESK_DMS_UPDATE=1
# lifts the hold and installs the newest one (check the patches afterwards), WAYDESK_DMS_VERSION=x.y.z picks another version.
DMS_TESTED=2.0.0db1
DMS_WANT="${WAYDESK_DMS_VERSION:-$DMS_TESTED}"
$SUDO apt-get install -y qt6-wayland
if [ -n "$OWN_REPO" ]; then
  # Our own build (package waydesk-shell, Provides: dms): updated with the rest of waydesk, only quickshell stays held.
  $SUDO apt-mark unhold dms >/dev/null 2>&1 || true
  $SUDO apt-get install -y waydesk-shell
  $SUDO apt-mark hold quickshell >/dev/null
elif [ -n "${WAYDESK_DMS_UPDATE:-}" ]; then
  $SUDO apt-mark unhold dms quickshell >/dev/null 2>&1 || true
  $SUDO apt-get install -y dms
  $SUDO apt-get install -y -t trixie-backports quickshell
elif dpkg -s dms >/dev/null 2>&1; then
  echo "DMS $(dpkg-query -W -f='${Version}' dms) is installed and held (WAYDESK_DMS_UPDATE=1 to update)."
elif ! $SUDO apt-get install -y "dms=$DMS_WANT"; then
  echo "warning: DMS $DMS_WANT is not in the repository any more: installing the newest, which the shell patches may not match." >&2
  $SUDO apt-get install -y dms
fi
# quickshell (from trixie-backports) is what DMS runs on: held too, so it only changes together with DMS.
if [ -z "$OWN_REPO" ]; then $SUDO apt-mark hold dms quickshell >/dev/null; fi

# With waydesk-desktop the system config (/etc/xdg/waydesk/sway.conf) starts DMS through sway-session.target. Without it,
# set that up in the account's own sway config, as the DMS docs ask for sway.
if [ ! -f /etc/xdg/waydesk/sway.conf ]; then
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
