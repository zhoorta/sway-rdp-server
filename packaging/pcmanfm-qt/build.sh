#!/bin/sh
# Builds the distribution's pcmanfm-qt with the patches next to this script (no "Hide Desktop Items", "Create Launcher", "Desktop Preferences"
# or "Open in Terminal" in the menus, no underlined menu letters) in Docker. Usage: packaging/pcmanfm-qt/build.sh [arm64|amd64 [image]]   (arm64 and debian:13 by default;
# ubuntu:26.04 for Ubuntu)
# Output: packaging/out/pcmanfm-qt_<distribution version>+waydesk2+<tag>_<arch>.deb (tag deb13, ubuntu26.04: the repository gives it only to the suite of that distribution).
# It has a higher version than the distribution's own package, and waydesk.pref pins it, so the distribution's updates do not replace it (the patches must be checked
# against a new upstream).
set -e
cd "$(dirname "$0")/../.."
ARCH=${1:-arm64}
IMAGE=${2:-debian:13}
TAG=$(printf '%s' "$IMAGE" | sed 's/^debian:/deb/; s/^ubuntu:/ubuntu/')
mkdir -p packaging/out
docker run --rm --platform linux/$ARCH -v "$PWD/packaging/pcmanfm-qt:/patch:ro" -v "$PWD/packaging/out:/out" \
  -e UID_HOST="$(id -u)" -e GID_HOST="$(id -g)" -e TAG="$TAG" "$IMAGE" sh -c '
set -e
export DEBIAN_FRONTEND=noninteractive
# source packages: Debian images use debian.sources, Ubuntu images ubuntu.sources (deb822); both get deb-src
for f in /etc/apt/sources.list.d/*.sources; do sed -i "s/^Types: deb\$/Types: deb deb-src/" "$f"; done
apt-get update -qq
apt-get install -y -qq dpkg-dev devscripts equivs fakeroot >/dev/null
mkdir /build && cd /build
apt-get source --download-only pcmanfm-qt >/dev/null 2>&1
dpkg-source -x pcmanfm-qt_*.dsc >/dev/null
cd pcmanfm-qt-*/
# The file manager recommends lxqt-qtplugin and lxqt-policykit, and those pull in lxqt-session and with it xscreensaver (a hundred screensavers in the
# launcher), lxqt-config, qps and qlipper: a session of ours uses qt6ct for the Qt theme and the shell for the polkit prompt, so neither is needed.
perl -0pi -e "s/\\s*lxqt-(qtplugin|policykit)(\\s*\\([^)]*\\))?\\s*,?//g" debian/control
if grep -qE "lxqt-(qtplugin|policykit)" debian/control; then echo "pcmanfm-qt: debian/control still names lxqt-qtplugin or lxqt-policykit" >&2; exit 1; fi
mk-build-deps -i -r -t "apt-get -y -qq --no-install-recommends" debian/control >/dev/null
for p in waydesk-menus waydesk-no-mnemonics; do
    cp /patch/$p.patch debian/patches/
    echo $p.patch >> debian/patches/series
done
V=$(dpkg-parsechangelog -S Version)
DEBFULLNAME="José Mendonça" DEBEMAIL=jmendonca@gmail.com dch --newversion "${V}+waydesk2+${TAG}" -D UNRELEASED "Remove desktop menu entries and Open in Terminal (waydesk); no recommends of lxqt-qtplugin and lxqt-policykit."
dpkg-buildpackage -b -uc -us >/dev/null
cd ..
cp pcmanfm-qt_*.deb /out/
chown $UID_HOST:$GID_HOST /out/pcmanfm-qt_*.deb'
ls -l packaging/out/pcmanfm-qt_*.deb
