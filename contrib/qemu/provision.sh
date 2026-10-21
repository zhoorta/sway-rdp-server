#!/bin/sh
# Installs the waydesk desktop into the development VM (see vm.sh), from the packages in packaging/out:
#
#   packaging/build-deb.sh arm64           builds the two .deb files (Docker)
#   contrib/qemu/provision.sh              pushes the newest ones, and sets the VM up
#
# What it does in the VM: installs both packages (and what they depend on), DankMaterialShell and quickshell through
# packaging/desktop/setup/dms.sh, Flatpak with Flathub, makes the RDP server listen on all interfaces (only the forward of the Mac
# reaches it, on 127.0.0.1:3395), and makes two accounts to log in with: rdptest and rdptest2 (an administrator). Their passwords
# are made here and kept in ~/vm/<name>/accounts.txt. Safe to run again: the passwords of existing accounts stay.
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
VM="$HERE/vm.sh"
OUT="$REPO/packaging/out"
ARCH=${VM_ARCH:-arm64}   # amd64: the x86_64 VM of vm.sh (VM_ARCH=amd64), with the amd64 packages
case "$ARCH" in amd64) NAME=${VM_NAME:-sway-rdp-dev-amd64}; RDP_PORT=3396 ;; *) NAME=${VM_NAME:-sway-rdp-dev}; RDP_PORT=3395 ;; esac
ACCOUNTS="${VM_DIR:-$HOME/vm/$NAME}/accounts.txt"

BASE=$(ls -t "$OUT"/waydesk-server_*_$ARCH.deb 2>/dev/null | head -1)
DESKTOP=$(ls -t "$OUT"/waydesk-desktop_*_all.deb 2>/dev/null | head -1)
META=$(ls -t "$OUT"/waydesk_*_all.deb 2>/dev/null | head -1)   # the metapackage: optional
FM=$(ls -t "$OUT"/pcmanfm-qt_*waydesk*_$ARCH.deb 2>/dev/null | head -1)   # optional: packaging/pcmanfm-qt/build.sh arm64
SWAY=$(ls -t "$OUT"/sway_*waydesk*_$ARCH.deb 2>/dev/null | head -1)   # optional: packaging/sway/build.sh arm64
[ -n "$BASE" ] && [ -n "$DESKTOP" ] || { echo "no packages in $OUT: run packaging/build-deb.sh arm64" >&2; exit 1; }
"$VM" status | grep -q running || { echo "the VM is not running: $VM start" >&2; exit 1; }

# Passwords of the two accounts: kept from the first run.
if [ ! -f "$ACCOUNTS" ]; then
    printf 'rdptest %s\nrdptest2 %s\n' "$(openssl rand -hex 6)" "$(openssl rand -hex 6)" > "$ACCOUNTS"
    chmod 600 "$ACCOUNTS"
fi
PW1=$(sed -n 's/^rdptest //p' "$ACCOUNTS")
PW2=$(sed -n 's/^rdptest2 //p' "$ACCOUNTS")

echo "pushing $(basename "$BASE") and $(basename "$DESKTOP")"
"$VM" ssh 'rm -rf ~/provision && mkdir -p ~/provision'
tar cf - -C "$OUT" "$(basename "$BASE")" "$(basename "$DESKTOP")" ${META:+"$(basename "$META")"} ${FM:+"$(basename "$FM")"} ${SWAY:+"$(basename "$SWAY")"} -C "$REPO/packaging/desktop/setup" dms.sh | "$VM" ssh 'tar xf - -C ~/provision'

{
    printf "PW1='%s'\nPW2='%s'\n" "$PW1" "$PW2"
    cat <<'REMOTE'
set -eu
export DEBIAN_FRONTEND=noninteractive
cd /home/jose/provision
apt-get update -qq
# Without the recommended packages (apt would add a screensaver, a second portal and a policy-kit agent of its own). The ones
# that matter are named here: audio, the clipboard, the picker of "Add app to desktop", the wallpaper, Flatpak, a browser (Chromium: the default one; Firefox is only offered by the wizard).
apt-get install -y --no-install-recommends -o Dpkg::Options::=--force-confold flatpak ./waydesk-server_*.deb ./waydesk-desktop_*.deb $(ls ./waydesk_*.deb 2>/dev/null) \
    pipewire pipewire-pulse pulseaudio-utils wireplumber wl-clipboard wmenu whiptail swaybg chromium
# The patched file manager (menus without Desktop Preferences, Create Launcher, Open in Terminal), when it was built; held so that
# Debian's own pcmanfm-qt does not replace it.
if ls ./pcmanfm-qt_*.deb >/dev/null 2>&1; then
    apt-mark unhold pcmanfm-qt >/dev/null 2>&1 || true
    apt-get install -y --reinstall --allow-downgrades --no-install-recommends -o Dpkg::Options::=--force-confold ./pcmanfm-qt_*.deb
    apt-mark hold pcmanfm-qt
fi
# The patched sway (window buttons in the title bar), held like the file manager. Sessions that run keep the sway they started with.
if ls ./sway_*.deb >/dev/null 2>&1; then
    apt-mark unhold sway >/dev/null 2>&1 || true
    apt-get install -y --reinstall --allow-downgrades --no-install-recommends -o Dpkg::Options::=--force-confold ./sway_*.deb
    apt-mark hold sway
fi
bash ./dms.sh
flatpak remote-add --system --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
# Only the forward of the Mac (127.0.0.1) reaches this address: listen on every interface, not on the loopback of the VM only.
sed -i 's|^WAYDESK_BIND=.*|WAYDESK_BIND=0.0.0.0:3389|' /etc/default/waydesk
systemctl restart waydesk.service
for acct in rdptest rdptest2; do
    id "$acct" >/dev/null 2>&1 && continue
    useradd -m -s /bin/bash "$acct"
    if [ "$acct" = rdptest ]; then echo "$acct:$PW1" | chpasswd; else echo "$acct:$PW2" | chpasswd; usermod -aG sudo,waydesk-admins "$acct"; fi
done
# Who may log in over RDP: the accounts of this VM (the group is made by the package).
for acct in rdptest rdptest2 rdptest3; do id "$acct" >/dev/null 2>&1 && usermod -aG waydesk-users "$acct"; done
# rdptest2 is the administrator (also on a rerun, and after the groups were remade by a reinstall)
id rdptest2 >/dev/null 2>&1 && usermod -aG sudo,waydesk-admins rdptest2
systemctl restart waydesk-helper.service waydesk.service
systemctl is-active waydesk-helper.service waydesk.service
ss -ltn | grep ':3389 ' || true
REMOTE
} | "$VM" ssh 'sudo sh -s'

echo
echo "RDP: connect the Windows App to 127.0.0.1:$RDP_PORT with one of (kept in $ACCOUNTS):"
sed 's/^/    /' "$ACCOUNTS"
