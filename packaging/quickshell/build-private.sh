#!/bin/sh
# Builds waydesk-quickshell: Quickshell 0.3.0 (https://quickshell.org, LGPL-3.0, the toolkit DankMaterialShell runs on) from Debian's orig tarball, installed under
# /usr/lib/waydesk (the binaries in bin/, its QML modules in qml/). The session worker puts bin/ first in PATH and qml/ in QML_IMPORT_PATH (src/worker.rs), so the same
# Quickshell runs on every distribution: Ubuntu has none, Debian 13 has it only in backports. The package Provides: quickshell, which is what waydesk-shell asks for.
# Quickshell uses Qt's private headers: the package is tied to the exact Qt it was built against (qt6-*-private-abi), like Debian's own.
# wayland-protocols >= 1.47 is built from source for the build (Debian 13 has 1.44). The crash handler (cpptrace) is left out.
#
#   packaging/quickshell/build-private.sh [arch [image]]     arch: arm64 (default) or amd64; image: debian:13 (default) or ubuntu:26.04
# Output: packaging/out/waydesk-quickshell_0.3.0+waydesk1+<tag>_<arch>.deb
set -e
cd "$(dirname "$0")/../.."
ARCH=${1:-arm64}
IMAGE=${2:-debian:13}
TAG=$(printf '%s' "$IMAGE" | sed 's/^debian:/deb/; s/^ubuntu:/ubuntu/')
mkdir -p packaging/out
docker run --rm --platform linux/$ARCH -v "$PWD/packaging/out:/out" -e UID_HOST="$(id -u)" -e GID_HOST="$(id -g)" -e ARCH="$ARCH" -e TAG="$TAG" "$IMAGE" sh -c '
set -e
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq --no-install-recommends build-essential cmake ninja-build pkgconf meson curl ca-certificates xz-utils dpkg-dev \
  qt6-base-dev qt6-declarative-dev qt6-wayland-dev qt6-shadertools-dev qt6-base-private-dev qt6-declarative-private-dev qt6-wayland-private-dev \
  libcli11-dev libwayland-dev libpipewire-0.3-dev libpam0g-dev libpolkit-agent-1-dev libpolkit-gobject-1-dev libglib2.0-dev libdrm-dev libgbm-dev \
  libjemalloc-dev libegl-dev libgles-dev libvulkan-dev libxcb1-dev spirv-tools >/dev/null
mkdir /build && cd /build
fetch() { curl -fsSL --retry 3 -o "$2" "$1" && echo "$3  $2" | sha256sum -c - >/dev/null; }
fetch http://deb.debian.org/debian/pool/main/q/quickshell/quickshell_0.3.0.orig.tar.xz qs.tar.xz 205216575e948f23836917770473ae5fb3f980098cc706dd293833422c56b3f5
fetch https://gitlab.freedesktop.org/wayland/wayland-protocols/-/releases/1.47/downloads/wayland-protocols-1.47.tar.xz wp.tar.xz 5fd4349bcbc9bab9a46f8cf77d1f434296a7a052c87440a094f63fcf62a58e20
tar -xJf wp.tar.xz && cd wayland-protocols-1.47 && meson setup build --prefix=/usr/local -Dtests=false >/dev/null && ninja -C build install >/dev/null && cd ..
export PKG_CONFIG_PATH=/usr/local/share/pkgconfig
tar -xJf qs.tar.xz && cd quickshell-0.3.0
cmake -GNinja -B build -DCMAKE_BUILD_TYPE=Release -DNO_PCH=ON -DCRASH_HANDLER=OFF -DDISTRIBUTOR=Waydesk \
  -DCMAKE_INSTALL_PREFIX=/usr/lib/waydesk -DINSTALL_QML_PREFIX=qml >/dev/null
cmake --build build >/dev/null
P=/build/pkg
DESTDIR=$P cmake --install build >/dev/null
find $P -type f -perm -u+x -exec strip --strip-unneeded {} + 2>/dev/null || true
mkdir -p $P/DEBIAN $P/usr/share/doc/waydesk-quickshell
cp LICENSE* $P/usr/share/doc/waydesk-quickshell/ 2>/dev/null || true
printf "Quickshell 0.3.0 (https://quickshell.org, LGPL-3.0-only), built from Debian'"'"'s orig tarball by packaging/quickshell/build-private.sh of https://github.com/zhoorta/waydesk.\nThe binaries are in /usr/lib/waydesk/bin, its QML modules in /usr/lib/waydesk/qml.\n" > $P/usr/share/doc/waydesk-quickshell/README
mkdir -p /tmp/dh/debian && cd /tmp/dh && printf "Source: waydesk-quickshell\n\nPackage: waydesk-quickshell\nArchitecture: any\n" > debian/control
BIN=$(find $P/usr/lib/waydesk/bin -type f | tr "\n" " ")
SO=$(find $P/usr/lib/waydesk/qml -name "*.so" | tr "\n" " ")
DEPS=$(dpkg-shlibdeps -O $BIN $SO 2>/dev/null | sed "s/^shlibs:Depends=//")
for abi in qt6-base-private-abi qt6-declarative-private-abi qt6-wayland-private-abi; do
  v=$(dpkg-query -W -f="\${Version}" $abi 2>/dev/null || true); [ -n "$v" ] && DEPS="$DEPS, $abi (= $v)"
done
cat > $P/DEBIAN/control <<CTL
Package: waydesk-quickshell
Version: 0.3.0+waydesk1+$TAG
Architecture: $ARCH
Maintainer: José Mendonça <jmendonca@gmail.com>
Section: x11
Priority: optional
Depends: $DEPS, hicolor-icon-theme, qt6-svg-plugins, qml6-module-qtquick, qml6-module-qtquick-controls, qml6-module-qtquick-layouts
Provides: quickshell (= 0.3.0)
Description: Quickshell, the QtQuick toolkit that DankMaterialShell runs on (Waydesk copy)
 Quickshell 0.3.0 installed under /usr/lib/waydesk so that every distribution
 runs the same version in the Waydesk session. It does not replace the
 distribution'"'"'s own quickshell, if there is one.
CTL
dpkg-deb --root-owner-group -b $P /out/waydesk-quickshell_0.3.0+waydesk1+${TAG}_${ARCH}.deb >/dev/null
chown $UID_HOST:$GID_HOST /out/waydesk-quickshell_*.deb
echo "Depends: $DEPS"
find $P/usr/lib/waydesk -maxdepth 2 | head -12'
