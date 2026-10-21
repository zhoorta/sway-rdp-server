#!/bin/sh
# Builds the waydesk, waydesk-desktop and waydesk (metapackage) packages in Docker (Debian 13). Usage: packaging/build-deb.sh [arm64|amd64]   (default arm64)
# Output: packaging/out/waydesk-server_<version>_<arch>.deb
set -e
cd "$(dirname "$0")/.."
ARCH=${1:-arm64}
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)~$(date +%Y%m%d%H%M).git$(git rev-parse --short HEAD 2>/dev/null || echo local)"
mkdir -p packaging/out
docker build --platform linux/$ARCH -t waydesk-build-$ARCH -f packaging/Dockerfile.build packaging
docker run --rm --platform linux/$ARCH -v "$PWD:/src" -v waydesk-cargo:/opt/cargo/registry -e CARGO_TARGET_DIR=/tmp/target \
  -e VERSION="$VERSION" -e ARCH="$ARCH" -e UID_HOST="$(id -u)" -e GID_HOST="$(id -g)" waydesk-build-$ARCH sh -c '
set -e
cd /src && cargo build --release
P=/tmp/pkg; rm -rf $P; mkdir -p $P/DEBIAN $P/usr/bin $P/lib/systemd/system $P/etc/default $P/etc/pam.d $P/usr/share/doc/waydesk-server
install -m 755 /tmp/target/release/waydesk-server $P/usr/bin/waydesk-server
install -m 755 /tmp/target/release/waydesk-admin $P/usr/bin/waydesk-admin
mkdir -p $P/usr/share/polkit-1/actions && install -m 644 packaging/org.waydesk.admin.policy $P/usr/share/polkit-1/actions/
install -m 644 packaging/systemd/*.service $P/lib/systemd/system/   # includes waydesk-session@.service
mkdir -p $P/lib/systemd/system/user-.slice.d && install -m 644 packaging/systemd/user-slice/50-waydesk.conf $P/lib/systemd/system/user-.slice.d/
install -m 644 packaging/default-waydesk $P/etc/default/waydesk
install -m 644 packaging/pam-waydesk-server $P/etc/pam.d/waydesk-server
install -m 644 packaging/pam-waydesk-session $P/etc/pam.d/waydesk-session
mkdir -p $P/usr/share/icons && cp -a packaging/cursors/waydesk-blank $P/usr/share/icons/   # cursors with nothing in them
install -m 644 README.md $P/usr/share/doc/waydesk-server/README.md
install -m 644 docs/MULTIUSER-DESIGN.md $P/usr/share/doc/waydesk-server/MULTIUSER-DESIGN.md
cat LICENSE-MIT > $P/usr/share/doc/waydesk-server/copyright
for f in postinst prerm postrm conffiles; do install -m 755 packaging/debian/$f $P/DEBIAN/$f; done
chmod 644 $P/DEBIAN/conffiles
sed -e "s/@VERSION@/$VERSION/" -e "s/@ARCH@/$ARCH/" packaging/debian/control > $P/DEBIAN/control
echo "Installed-Size: $(du -sk $P | cut -f1)" >> $P/DEBIAN/control
dpkg-deb --root-owner-group -Zxz -b $P /src/packaging/out/ >/dev/null

# the desktop metapackage (architecture independent; built here only so that both packages share a version)
D=/tmp/pkgd; rm -rf $D; mkdir -p $D/DEBIAN $D/usr/bin $D/etc/xdg/waydesk $D/usr/share/waydesk/setup $D/usr/lib/systemd/user/sway-session.target.wants $D/usr/share/doc/waydesk-desktop
install -m 755 packaging/desktop/bin/* $D/usr/bin/
install -m 644 packaging/desktop/etc-waydesk/sway.conf $D/etc/xdg/waydesk/sway.conf
install -m 755 packaging/desktop/setup/*.sh $D/usr/share/waydesk/setup/
install -m 644 packaging/desktop/setup/*.tsv $D/usr/share/waydesk/setup/
mkdir -p $D/usr/share/waydesk/gtk $D/usr/lib/environment.d && install -m 644 packaging/desktop/gtk/*.css $D/usr/share/waydesk/gtk/
mkdir -p $D/usr/share/icons/hicolor/scalable/apps && install -m 644 packaging/desktop/icons/*.svg $D/usr/share/icons/hicolor/scalable/apps/
install -m 644 packaging/desktop/environment.d/*.conf $D/usr/lib/environment.d/
mkdir -p $D/usr/share/polkit-1/rules.d && install -m 644 packaging/desktop/polkit/*.rules $D/usr/share/polkit-1/rules.d/
mkdir -p $D/usr/lib/systemd/system/waydesk-session@.service.d && install -m 644 packaging/desktop/systemd-session/*.conf $D/usr/lib/systemd/system/waydesk-session@.service.d/
mkdir -p $D/usr/share/waydesk/qt6ct && install -m 644 packaging/desktop/qt6ct/* $D/usr/share/waydesk/qt6ct/
# /etc, not /usr/share: the Debian sway package owns /usr/share/xdg-desktop-portal/sway-portals.conf, and /etc wins over it
mkdir -p $D/etc/xdg-desktop-portal && install -m 644 packaging/desktop/xdg-desktop-portal/*.conf $D/etc/xdg-desktop-portal/
mkdir -p $D/usr/share/waydesk/users-panel && install -m 644 packaging/desktop/users-panel/*.qml $D/usr/share/waydesk/users-panel/
mkdir -p $D/usr/share/applications && install -m 644 packaging/desktop/applications/*.desktop $D/usr/share/applications/
install -m 755 packaging/debian/postinst-desktop $D/DEBIAN/postinst
mkdir -p $D/usr/share/waydesk/dms-plugin && cp -r packaging/desktop/dms-plugin/. $D/usr/share/waydesk/dms-plugin/ && chmod -R go-w,a+rX $D/usr/share/waydesk/dms-plugin
install -m 644 packaging/desktop/systemd-user/sway-session.target $D/usr/lib/systemd/user/sway-session.target
ln -s ../dms.service $D/usr/lib/systemd/user/sway-session.target.wants/dms.service
cat LICENSE-MIT > $D/usr/share/doc/waydesk-desktop/copyright
echo /etc/xdg/waydesk/sway.conf > $D/DEBIAN/conffiles
sed -e "s/@VERSION@/$VERSION/g" packaging/debian/control-desktop > $D/DEBIAN/control
echo "Installed-Size: $(du -sk $D | cut -f1)" >> $D/DEBIAN/control
dpkg-deb --root-owner-group -Zxz -b $D /src/packaging/out/ >/dev/null

# the Waydesk metapackage: apt install waydesk gets the server and the desktop (same version, so they stay together)
M=/tmp/pkgm; rm -rf $M; mkdir -p $M/DEBIAN $M/usr/share/doc/waydesk
cat LICENSE-MIT > $M/usr/share/doc/waydesk/copyright
sed -e "s/@VERSION@/$VERSION/g" packaging/debian/control-waydesk > $M/DEBIAN/control
echo "Installed-Size: $(du -sk $M | cut -f1)" >> $M/DEBIAN/control
dpkg-deb --root-owner-group -Zxz -b $M /src/packaging/out/ >/dev/null
chown -R $UID_HOST:$GID_HOST /src/packaging/out'
ls -l packaging/out
