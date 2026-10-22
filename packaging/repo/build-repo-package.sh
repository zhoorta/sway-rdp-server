#!/bin/sh
# Builds waydesk-repo, the package that makes a server trust and use the Waydesk repository: the public key, the apt source and the pin.
#
#   packaging/repo/build-repo-package.sh PUBLIC-KEY.gpg [REPO-URL]     output: packaging/out/waydesk-repo_<version>_all.deb
#
# REPO-URL defaults to https://waydesk.stackcare.pt/apt. The first install on a server (it cannot come from the repository itself):
#   curl -fsSLO https://waydesk.stackcare.pt/apt/waydesk-repo.deb && sudo apt install ./waydesk-repo.deb && sudo apt update
# and check the key fingerprint against the one published with it.
set -eu
KEY=${1:?usage: build-repo-package.sh PUBLIC-KEY.gpg [REPO-URL]}
URL=${2:-https://waydesk.stackcare.pt/apt}
VERSION=${WAYDESK_REPO_PACKAGE_VERSION:-1.0}
cd "$(dirname "$0")/../.."
OUT="$PWD/packaging/out"
mkdir -p "$OUT"
KEY=$(cd "$(dirname "$KEY")" && pwd)/$(basename "$KEY")
docker run --rm -v "$PWD/packaging/repo:/repo:ro" -v "$KEY:/key.gpg:ro" -v "$OUT:/out" -e URL="$URL" -e VERSION="$VERSION" waydesk-build-arm64 sh -c '
set -eu
P=/tmp/pkg; rm -rf $P
mkdir -p $P/DEBIAN $P/usr/share/keyrings $P/etc/apt/sources.list.d $P/etc/apt/preferences.d $P/usr/share/doc/waydesk-repo
install -m 644 /key.gpg $P/usr/share/keyrings/waydesk-archive-keyring.gpg
sed "s|https://waydesk.stackcare.pt/apt|$URL|; /^#/d" /repo/waydesk.sources.in > $P/etc/apt/sources.list.d/waydesk.sources
chmod 644 $P/etc/apt/sources.list.d/waydesk.sources
sed "/^#/d" /repo/waydesk.pref > $P/etc/apt/preferences.d/waydesk; chmod 644 $P/etc/apt/preferences.d/waydesk
printf "/etc/apt/sources.list.d/waydesk.sources\n/etc/apt/preferences.d/waydesk\n" > $P/DEBIAN/conffiles
echo "The public key of the Waydesk repository, its apt source ($URL) and the pin that makes our builds win over third-party repositories." > $P/usr/share/doc/waydesk-repo/README
cat > $P/DEBIAN/control <<C
Package: waydesk-repo
Version: $VERSION
Architecture: all
Maintainer: José Mendonça <jmendonca@gmail.com>
Section: admin
Priority: optional
Installed-Size: 8
Description: Waydesk apt repository: key, source and pin
 Lets apt install and update waydesk-server, waydesk-desktop, waydesk-shell and their helpers from
 the repository, which is signed. Check the key fingerprint against the one published with the repository.
C
dpkg-deb --root-owner-group -Zxz -b $P /out/ >/dev/null
ls -la /out/waydesk-repo_*
'
