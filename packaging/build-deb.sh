#!/bin/sh
# Builds the sway-rdp package in Docker (Debian 13). Usage: packaging/build-deb.sh [arm64|amd64]   (default arm64)
# Output: packaging/out/sway-rdp_<version>_<arch>.deb
set -e
cd "$(dirname "$0")/.."
ARCH=${1:-arm64}
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)~$(date +%Y%m%d%H%M).git$(git rev-parse --short HEAD 2>/dev/null || echo local)"
mkdir -p packaging/out
docker build --platform linux/$ARCH -t sway-rdp-build-$ARCH -f packaging/Dockerfile.build packaging
docker run --rm --platform linux/$ARCH -v "$PWD:/src" -v sway-rdp-cargo:/opt/cargo/registry -e CARGO_TARGET_DIR=/tmp/target \
  -e VERSION="$VERSION" -e ARCH="$ARCH" -e UID_HOST="$(id -u)" -e GID_HOST="$(id -g)" sway-rdp-build-$ARCH sh -c '
set -e
cd /src && cargo build --release
P=/tmp/pkg; rm -rf $P; mkdir -p $P/DEBIAN $P/usr/bin $P/lib/systemd/system $P/etc/default $P/etc/pam.d $P/usr/share/doc/sway-rdp
install -m 755 /tmp/target/release/sway-rdp-server $P/usr/bin/sway-rdp-server
install -m 644 packaging/systemd/*.service $P/lib/systemd/system/   # includes sway-rdp-session@.service
install -m 644 packaging/default-sway-rdp $P/etc/default/sway-rdp
install -m 644 packaging/pam-sway-rdp-server $P/etc/pam.d/sway-rdp-server
install -m 644 packaging/pam-sway-rdp-session $P/etc/pam.d/sway-rdp-session
install -m 644 README.md $P/usr/share/doc/sway-rdp/README.md
install -m 644 docs/MULTIUSER-DESIGN.md $P/usr/share/doc/sway-rdp/MULTIUSER-DESIGN.md
cat LICENSE-MIT > $P/usr/share/doc/sway-rdp/copyright
for f in postinst prerm postrm conffiles; do install -m 755 packaging/debian/$f $P/DEBIAN/$f; done
chmod 644 $P/DEBIAN/conffiles
sed -e "s/@VERSION@/$VERSION/" -e "s/@ARCH@/$ARCH/" packaging/debian/control > $P/DEBIAN/control
echo "Installed-Size: $(du -sk $P | cut -f1)" >> $P/DEBIAN/control
dpkg-deb --root-owner-group -Zxz -b $P /src/packaging/out/ >/dev/null

# the desktop metapackage (architecture independent; built here only so that both packages share a version)
D=/tmp/pkgd; rm -rf $D; mkdir -p $D/DEBIAN $D/usr/bin $D/etc/xdg/sway-rdp $D/usr/share/sway-rdp/setup $D/usr/lib/systemd/user/sway-session.target.wants $D/usr/share/doc/sway-rdp-desktop
install -m 755 packaging/desktop/bin/* $D/usr/bin/
install -m 644 packaging/desktop/etc-sway-rdp/sway.conf $D/etc/xdg/sway-rdp/sway.conf
install -m 755 packaging/desktop/setup/*.sh $D/usr/share/sway-rdp/setup/
install -m 644 packaging/desktop/setup/*.tsv $D/usr/share/sway-rdp/setup/
mkdir -p $D/usr/share/sway-rdp/gtk $D/usr/lib/environment.d && install -m 644 packaging/desktop/gtk/*.css $D/usr/share/sway-rdp/gtk/
install -m 644 packaging/desktop/environment.d/*.conf $D/usr/lib/environment.d/
mkdir -p $D/usr/share/polkit-1/rules.d && install -m 644 packaging/desktop/polkit/*.rules $D/usr/share/polkit-1/rules.d/
mkdir -p $D/usr/lib/systemd/system/sway-rdp-session@.service.d && install -m 644 packaging/desktop/systemd-session/*.conf $D/usr/lib/systemd/system/sway-rdp-session@.service.d/
mkdir -p $D/usr/share/sway-rdp/qt6ct && install -m 644 packaging/desktop/qt6ct/* $D/usr/share/sway-rdp/qt6ct/
# /etc, not /usr/share: the Debian sway package owns /usr/share/xdg-desktop-portal/sway-portals.conf, and /etc wins over it
mkdir -p $D/etc/xdg-desktop-portal && install -m 644 packaging/desktop/xdg-desktop-portal/*.conf $D/etc/xdg-desktop-portal/
mkdir -p $D/usr/share/applications && install -m 644 packaging/desktop/applications/*.desktop $D/usr/share/applications/
install -m 755 packaging/debian/postinst-desktop $D/DEBIAN/postinst
mkdir -p $D/usr/share/sway-rdp/dms-plugin && cp -r packaging/desktop/dms-plugin/. $D/usr/share/sway-rdp/dms-plugin/ && chmod -R go-w,a+rX $D/usr/share/sway-rdp/dms-plugin
install -m 644 packaging/desktop/systemd-user/sway-session.target $D/usr/lib/systemd/user/sway-session.target
ln -s ../dms.service $D/usr/lib/systemd/user/sway-session.target.wants/dms.service
cat LICENSE-MIT > $D/usr/share/doc/sway-rdp-desktop/copyright
echo /etc/xdg/sway-rdp/sway.conf > $D/DEBIAN/conffiles
sed -e "s/@VERSION@/$VERSION/g" packaging/debian/control-desktop > $D/DEBIAN/control
echo "Installed-Size: $(du -sk $D | cut -f1)" >> $D/DEBIAN/control
dpkg-deb --root-owner-group -Zxz -b $D /src/packaging/out/ >/dev/null
chown -R $UID_HOST:$GID_HOST /src/packaging/out'
ls -l packaging/out
