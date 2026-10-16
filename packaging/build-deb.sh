#!/bin/sh
# Builds the sway-rdp package in Docker (Debian 13). Usage: packaging/build-deb.sh [arm64|amd64]   (default arm64)
# Output: packaging/out/sway-rdp_<version>_<arch>.deb
set -e
cd "$(dirname "$0")/.."
ARCH=${1:-arm64}
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)~git$(git rev-parse --short HEAD 2>/dev/null || echo local)"
mkdir -p packaging/out
docker build --platform linux/$ARCH -t sway-rdp-build-$ARCH -f packaging/Dockerfile.build packaging
docker run --rm --platform linux/$ARCH -v "$PWD:/src" -v sway-rdp-cargo:/opt/cargo/registry -e CARGO_TARGET_DIR=/tmp/target \
  -e VERSION="$VERSION" -e ARCH="$ARCH" -e UID_HOST="$(id -u)" -e GID_HOST="$(id -g)" sway-rdp-build-$ARCH sh -c '
set -e
cd /src && cargo build --release
P=/tmp/pkg; rm -rf $P; mkdir -p $P/DEBIAN $P/usr/bin $P/lib/systemd/system $P/etc/default $P/etc/pam.d $P/usr/share/doc/sway-rdp
install -m 755 /tmp/target/release/sway-rdp-server $P/usr/bin/sway-rdp-server
install -m 644 packaging/systemd/*.service $P/lib/systemd/system/
install -m 644 packaging/default-sway-rdp $P/etc/default/sway-rdp
install -m 644 packaging/pam-sway-rdp-server $P/etc/pam.d/sway-rdp-server
install -m 644 README.md $P/usr/share/doc/sway-rdp/README.md
install -m 644 docs/MULTIUSER-DESIGN.md $P/usr/share/doc/sway-rdp/MULTIUSER-DESIGN.md
cat LICENSE-MIT > $P/usr/share/doc/sway-rdp/copyright
for f in postinst prerm postrm conffiles; do install -m 755 packaging/debian/$f $P/DEBIAN/$f; done
chmod 644 $P/DEBIAN/conffiles
sed -e "s/@VERSION@/$VERSION/" -e "s/@ARCH@/$ARCH/" packaging/debian/control > $P/DEBIAN/control
echo "Installed-Size: $(du -sk $P | cut -f1)" >> $P/DEBIAN/control
dpkg-deb --root-owner-group -Zxz -b $P /src/packaging/out/ >/dev/null
chown -R $UID_HOST:$GID_HOST /src/packaging/out'
ls -l packaging/out
