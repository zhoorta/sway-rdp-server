#!/bin/sh
# Installs the sway-rdp desktop into the development VM (see vm.sh), from the packages in packaging/out:
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
NAME=${VM_NAME:-sway-rdp-dev}
ACCOUNTS="${VM_DIR:-$HOME/vm/$NAME}/accounts.txt"

BASE=$(ls -t "$OUT"/sway-rdp_*_arm64.deb 2>/dev/null | head -1)
DESKTOP=$(ls -t "$OUT"/sway-rdp-desktop_*_all.deb 2>/dev/null | head -1)
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
tar cf - -C "$OUT" "$(basename "$BASE")" "$(basename "$DESKTOP")" -C "$REPO/packaging/desktop/setup" dms.sh | "$VM" ssh 'tar xf - -C ~/provision'

{
    printf "PW1='%s'\nPW2='%s'\n" "$PW1" "$PW2"
    cat <<'REMOTE'
set -eu
export DEBIAN_FRONTEND=noninteractive
cd /home/jose/provision
apt-get update -qq
# Without the recommended packages (apt would add a screensaver, a second portal and a policy-kit agent of its own). The ones
# that matter are named here: audio, the clipboard, the picker of "Add app to desktop", the wallpaper, Flatpak, a browser.
apt-get install -y --no-install-recommends -o Dpkg::Options::=--force-confold flatpak ./sway-rdp_*.deb ./sway-rdp-desktop_*.deb \
    pipewire pipewire-pulse pulseaudio-utils wireplumber wl-clipboard wmenu whiptail swaybg firefox-esr
bash ./dms.sh
flatpak remote-add --system --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
# Only the forward of the Mac (127.0.0.1) reaches this address: listen on every interface, not on the loopback of the VM only.
sed -i 's|^SWAY_RDP_BIND=.*|SWAY_RDP_BIND=0.0.0.0:3389|' /etc/default/sway-rdp
systemctl restart sway-rdp.service
for acct in rdptest rdptest2; do
    id "$acct" >/dev/null 2>&1 && continue
    useradd -m -s /bin/bash "$acct"
    if [ "$acct" = rdptest ]; then echo "$acct:$PW1" | chpasswd; else echo "$acct:$PW2" | chpasswd; usermod -aG sudo,sway-rdp-admins "$acct"; fi
done
# Who may log in over RDP: the accounts of this VM (the group is made by the package).
for acct in rdptest rdptest2 rdptest3; do id "$acct" >/dev/null 2>&1 && usermod -aG sway-rdp-users "$acct"; done
systemctl restart sway-rdp-helper.service sway-rdp.service
systemctl is-active sway-rdp-helper.service sway-rdp.service
ss -ltn | grep ':3389 ' || true
REMOTE
} | "$VM" ssh 'sudo sh -s'

echo
echo "RDP: connect the Windows App to 127.0.0.1:3395 with one of (kept in $ACCOUNTS):"
sed 's/^/    /' "$ACCOUNTS"
