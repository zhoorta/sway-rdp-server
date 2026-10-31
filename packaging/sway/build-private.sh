#!/bin/sh
# Builds waydesk-sway: sway 1.11 with the patch next to this script (round window buttons in the title bar, see waydesk-titlebar-buttons.patch) and wlroots 0.19.3 linked
# in statically, installed under /usr/lib/waydesk/bin (sway, swaymsg, swaybar, swaynag). The session worker puts that directory first in its PATH (src/worker.rs), so the
# same compositor runs on every distribution and the distribution's own sway never decides: no pin, no `apt-mark hold`. Debian 13 has wlroots 0.18 and Ubuntu 26.04 has
# 0.19, and sway 1.12 would need libwayland 1.24, which Debian 13 does not have: that is why 1.11 with its own wlroots.
#
#   packaging/sway/build-private.sh [arch [image]]      arch: arm64 (default) or amd64; image: debian:13 (default) or ubuntu:26.04
# Output: packaging/out/waydesk-sway_1.11+waydesk3+<tag>_<arch>.deb  (tag: deb13, ubuntu26.04, from the image). The dependencies come from the binaries (dpkg-shlibdeps).
set -e
cd "$(dirname "$0")/../.."
ARCH=${1:-arm64}
IMAGE=${2:-debian:13}
TAG=$(printf '%s' "$IMAGE" | sed 's/^debian:/deb/; s/^ubuntu:/ubuntu/')
mkdir -p packaging/out
docker run --rm --platform linux/$ARCH -v "$PWD/packaging/sway:/patch:ro" -v "$PWD/packaging/out:/out" \
  -e UID_HOST="$(id -u)" -e GID_HOST="$(id -g)" -e ARCH="$ARCH" -e TAG="$TAG" "$IMAGE" sh -c '
set -e
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq --no-install-recommends build-essential meson ninja-build pkg-config patch scdoc wayland-protocols curl ca-certificates dpkg-dev \
  libwayland-dev libxkbcommon-dev libpixman-1-dev libinput-dev libdrm-dev libudev-dev libseat-dev libvulkan-dev glslang-tools \
  libegl-dev libgles-dev libgbm-dev libdisplay-info-dev liblcms2-dev libjson-c-dev libpcre2-dev libcairo2-dev libpango1.0-dev \
  libgdk-pixbuf-2.0-dev libevdev-dev hwdata libliftoff-dev libsystemd-dev libxcb1-dev libxcb-icccm4-dev libxcb-errors-dev \
  xwayland libxcb-composite0-dev libxcb-ewmh-dev libxcb-render0-dev libxcb-res0-dev libxcb-xfixes0-dev >/dev/null
mkdir /build && cd /build
fetch() { curl -fsSL --retry 3 -o "$2" "$1" && echo "$3  $2" | sha256sum -c - >/dev/null; }
fetch https://github.com/swaywm/sway/releases/download/1.11/sway-1.11.tar.gz sway.tar.gz 0e37a55b7c3379230e97e1ad982542b75016a0c7d6676198604e557f9b373dae
fetch https://gitlab.freedesktop.org/wlroots/wlroots/-/releases/0.19.3/downloads/wlroots-0.19.3.tar.gz wlroots.tar.gz 5d02693175e5afd9af5f10e3e4976d6e9249dc39a90eb17d23fa5f54b125ccc5
tar -xzf sway.tar.gz && cd sway-1.11
mkdir -p subprojects && tar -xzf ../wlroots.tar.gz -C subprojects && mv subprojects/wlroots-0.19.3 subprojects/wlroots
patch -p1 < /patch/waydesk-titlebar-buttons.patch >/dev/null
meson setup build -Dwlroots:xwayland=enabled -Dwlroots:examples=false -Dwlroots:default_library=static -Ddefault-wallpaper=false -Dman-pages=disabled \
  -Dzsh-completions=false -Dbash-completions=false -Dfish-completions=false --buildtype=release --prefix=/usr/lib/waydesk >/dev/null
ninja -C build >/tmp/ninja.log 2>&1 || { tail -40 /tmp/ninja.log; exit 1; }
P=/build/pkg; B=$P/usr/lib/waydesk/bin
mkdir -p $B $P/DEBIAN $P/usr/share/doc/waydesk-sway
for f in sway/sway swaymsg/swaymsg swaybar/swaybar swaynag/swaynag; do install -s -m 755 build/$f $B/; done
cp LICENSE $P/usr/share/doc/waydesk-sway/copyright-sway; cp subprojects/wlroots/LICENSE $P/usr/share/doc/waydesk-sway/copyright-wlroots
cp /patch/waydesk-titlebar-buttons.patch $P/usr/share/doc/waydesk-sway/
cat > $P/usr/share/doc/waydesk-sway/README <<README
sway 1.11 (https://github.com/swaywm/sway, MIT) with wlroots 0.19.3 (https://gitlab.freedesktop.org/wlroots/wlroots, MIT) linked in statically, and
waydesk-titlebar-buttons.patch (this directory). Built by packaging/sway/build-private.sh of https://github.com/zhoorta/waydesk.
The binaries are in /usr/lib/waydesk/bin; the Waydesk session puts that directory first in its PATH.
README
# the dependencies, from the binaries (dpkg-shlibdeps wants a debian/control next to it)
mkdir -p /tmp/dh/debian && cd /tmp/dh && printf "Source: waydesk-sway\n\nPackage: waydesk-sway\nArchitecture: any\n" > debian/control
DEPS=$(dpkg-shlibdeps -O $B/sway $B/swaymsg $B/swaybar $B/swaynag 2>/dev/null | sed "s/^shlibs:Depends=//")
cat > $P/DEBIAN/control <<CTL
Package: waydesk-sway
Version: 1.11+waydesk3+$TAG
Architecture: $ARCH
Maintainer: José Mendonça <jmendonca@gmail.com>
Section: x11
Priority: optional
Depends: $DEPS, xwayland
Description: sway 1.11 with window buttons in the title bar, for Waydesk sessions
 sway with wlroots 0.19.3 linked in (with XWayland support, so that applications that only speak
 X11 run: the Xwayland server of the distribution starts when the first one opens), plus Waydesk'"'"'s title-bar patch, installed under
 /usr/lib/waydesk/bin so that every distribution runs the same compositor in the
 Waydesk session. It does not replace the distribution'"'"'s sway.
CTL
dpkg-deb --root-owner-group -b $P /out/waydesk-sway_1.11+waydesk3+${TAG}_${ARCH}.deb >/dev/null
chown $UID_HOST:$GID_HOST /out/waydesk-sway_*.deb
echo "Depends: $DEPS"'
