#!/bin/sh
# Builds waydesk-repo, the package that makes a server trust and use the Waydesk repository: the public key, the apt source and the pin.
#
#   packaging/repo/build-repo-package.sh PUBLIC-KEY.gpg [REPO-URL]     output: packaging/out/waydesk-repo_<version>_all.deb
#
# REPO-URL defaults to https://waydesk.dev/apt. The first install on a server (it cannot come from the repository itself):
#   curl -fsSLO https://waydesk.dev/apt/waydesk-repo.deb && sudo apt install ./waydesk-repo.deb && sudo apt update
# and check the key fingerprint against the one published with it.
set -eu
KEY=${1:?usage: build-repo-package.sh PUBLIC-KEY.gpg [REPO-URL]}
URL=${2:-https://waydesk.dev/apt}
VERSION=${WAYDESK_REPO_PACKAGE_VERSION:-1.3}
cd "$(dirname "$0")/../.."
OUT="$PWD/packaging/out"
mkdir -p "$OUT"
KEY=$(cd "$(dirname "$KEY")" && pwd)/$(basename "$KEY")
docker run --rm -v "$PWD/packaging/repo:/repo:ro" -v "$KEY:/key.gpg:ro" -v "$OUT:/out" -e URL="$URL" -e VERSION="$VERSION" waydesk-build-arm64 sh -c '
set -eu
P=/tmp/pkg; rm -rf $P
mkdir -p $P/DEBIAN $P/usr/share/keyrings $P/usr/share/waydesk-repo $P/etc/apt/preferences.d $P/usr/share/doc/waydesk-repo
install -m 644 /key.gpg $P/usr/share/keyrings/waydesk-archive-keyring.gpg
# the apt source depends on the system (Debian 13 is trixie, Ubuntu 26.04 is resolute): a template, written out by the postinst
sed "s|https://waydesk.dev/apt|$URL|; /^#/d" /repo/waydesk.sources.in > $P/usr/share/waydesk-repo/waydesk.sources.in
chmod 644 $P/usr/share/waydesk-repo/waydesk.sources.in
sed "/^#/d" /repo/waydesk.pref > $P/etc/apt/preferences.d/waydesk; chmod 644 $P/etc/apt/preferences.d/waydesk
printf "/etc/apt/preferences.d/waydesk\n" > $P/DEBIAN/conffiles
cat > $P/DEBIAN/postinst <<"POSTINST"
#!/bin/sh
set -e
if [ "$1" = configure ]; then
    f=/etc/apt/sources.list.d/waydesk.sources
    # An existing file stays (version 1.0 shipped it as a conffile, always for trixie), except for one change: a file that still has the old address of the
    # repository (waydesk.stackcare.pt, used until 2026-10-24) is moved to the address of the template of this package; any other edit is left alone.
    if [ -e "$f" ]; then
        new=$(sed -n "s/^URIs: //p" /usr/share/waydesk-repo/waydesk.sources.in)
        if [ -n "$new" ] && grep -qx "URIs: https://waydesk.stackcare.pt/apt" "$f"; then
            sed -i "s|^URIs: https://waydesk.stackcare.pt/apt\$|URIs: $new|" "$f"
        fi
    else
        suite=""
        [ -r /etc/os-release ] && . /etc/os-release
        case "${VERSION_CODENAME:-}" in
            trixie|resolute) suite=$VERSION_CODENAME ;;
            *) echo "waydesk-repo: Waydesk has no packages for ${PRETTY_NAME:-this system} (Debian 13 and Ubuntu 26.04 only): no apt source was written." >&2 ;;
        esac
        if [ -n "$suite" ]; then
            sed "s/@SUITE@/$suite/" /usr/share/waydesk-repo/waydesk.sources.in > "$f"
            chmod 644 "$f"
        fi
    fi
fi
exit 0
POSTINST
cat > $P/DEBIAN/postrm <<"POSTRM"
#!/bin/sh
set -e
if [ "$1" = purge ]; then rm -f /etc/apt/sources.list.d/waydesk.sources; fi
exit 0
POSTRM
chmod 755 $P/DEBIAN/postinst $P/DEBIAN/postrm
echo "The public key of the Waydesk repository, its apt source ($URL, suite trixie on Debian 13 and resolute on Ubuntu 26.04, written by the postinst) and the pin that makes our builds win over third-party repositories." > $P/usr/share/doc/waydesk-repo/README
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
