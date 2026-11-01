#!/bin/sh
# Builds Debian's sway with the patch next to this script (round window buttons in the title bar: close, minimize, maximize, set
# with `titlebar_button`; see waydesk-titlebar-buttons.patch) in Docker (Debian 13). Usage: packaging/sway/build.sh [arm64|amd64]   (default arm64)
# Output: packaging/out/sway_<debian version>+waydesk1_<arch>.deb. Install it over Debian's package: it has a higher version, and
# `apt-mark hold sway` keeps Debian's own updates from replacing it (the patch must be checked against a new upstream).
set -e
cd "$(dirname "$0")/../.."
ARCH=${1:-arm64}
mkdir -p packaging/out
docker run --rm --platform linux/$ARCH -v "$PWD/packaging/sway:/patch:ro" -v "$PWD/packaging/out:/out" \
  -e UID_HOST="$(id -u)" -e GID_HOST="$(id -g)" debian:13 sh -c '
set -e
export DEBIAN_FRONTEND=noninteractive
echo "deb-src http://deb.debian.org/debian trixie main" > /etc/apt/sources.list.d/src.list
apt-get update -qq
apt-get install -y -qq dpkg-dev devscripts equivs fakeroot >/dev/null
mkdir /build && cd /build
apt-get source --download-only sway >/dev/null 2>&1
dpkg-source -x sway_*.dsc >/dev/null
cd sway-*/
mk-build-deps -i -r -t "apt-get -y -qq --no-install-recommends" debian/control >/dev/null
cp /patch/waydesk-titlebar-buttons.patch debian/patches/
echo waydesk-titlebar-buttons.patch >> debian/patches/series
DEBFULLNAME="StackCare" DEBEMAIL=jose@stackcare.pt dch --local +waydesk "Window buttons in the title bar (waydesk)."
DEB_BUILD_OPTIONS=nocheck dpkg-buildpackage -b -uc -us >/dev/null
cd ..
cp sway_*.deb /out/
chown $UID_HOST:$GID_HOST /out/sway_*.deb'
