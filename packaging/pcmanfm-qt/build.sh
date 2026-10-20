#!/bin/sh
# Builds Debian's pcmanfm-qt with the patches next to this script (no "Hide Desktop Items", "Create Launcher", "Desktop Preferences"
# or "Open in Terminal" in the menus, no underlined menu letters) in Docker (Debian 13). Usage: packaging/pcmanfm-qt/build.sh [arm64|amd64]   (default arm64)
# Output: packaging/out/pcmanfm-qt_<debian version>+waydesk1_<arch>.deb. Install it over Debian's package: it has a higher version,
# and `apt-mark hold pcmanfm-qt` keeps Debian's own updates from replacing it (the patch must be checked against a new upstream).
set -e
cd "$(dirname "$0")/../.."
ARCH=${1:-arm64}
mkdir -p packaging/out
docker run --rm --platform linux/$ARCH -v "$PWD/packaging/pcmanfm-qt:/patch:ro" -v "$PWD/packaging/out:/out" \
  -e UID_HOST="$(id -u)" -e GID_HOST="$(id -g)" debian:13 sh -c '
set -e
export DEBIAN_FRONTEND=noninteractive
echo "deb-src http://deb.debian.org/debian trixie main" > /etc/apt/sources.list.d/src.list
apt-get update -qq
apt-get install -y -qq dpkg-dev devscripts equivs fakeroot >/dev/null
mkdir /build && cd /build
apt-get source --download-only pcmanfm-qt >/dev/null 2>&1
dpkg-source -x pcmanfm-qt_*.dsc >/dev/null
cd pcmanfm-qt-*/
mk-build-deps -i -r -t "apt-get -y -qq --no-install-recommends" debian/control >/dev/null
for p in waydesk-menus waydesk-no-mnemonics; do
    cp /patch/$p.patch debian/patches/
    echo $p.patch >> debian/patches/series
done
DEBFULLNAME="José Mendonça" DEBEMAIL=jmendonca@gmail.com dch --local +waydesk "Remove desktop menu entries and Open in Terminal (waydesk)."
dpkg-buildpackage -b -uc -us >/dev/null
cd ..
cp pcmanfm-qt_*.deb /out/
chown $UID_HOST:$GID_HOST /out/pcmanfm-qt_*.deb'
ls -l packaging/out/pcmanfm-qt_*.deb
