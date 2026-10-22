#!/bin/sh
# Builds waydesk-shell: our own build of DankMaterialShell (Dux Shell) 2.x, with the waydesk patches applied, as a .deb. It replaces the
# `dms` package of the third-party OBS repository (Provides/Conflicts/Replaces: dms). Everything is built from the upstream tag in a clean
# Debian 13 container, with a pinned and checksum-verified Go toolchain. See docs/DMS-PACKAGE.md.
#
#   packaging/dms/build-dms.sh [arm64|amd64]     output: packaging/out/waydesk-shell_<version>_<arch>.deb
#
# Upstream's own Debian package does not build from source: it downloads the binary of their GitHub release. The build needs the network
# (Go modules, pinned by go.sum; the shared QML library is a git submodule).
set -eu
ARCH=${1:-arm64}
DMS_TAG=v2.0.0
GO_VERSION=1.27.3   # go.mod of DMS 2.0.0 asks for 1.27.1 or newer; Debian 13 has 1.24
case "$ARCH" in
    arm64) GO_SHA256=a06e99309dbcbdca1a8ccd2282c3ee146e55930886e26b4f1193269587165f49 ;;
    amd64) echo "amd64: put the sha256 of go$GO_VERSION.linux-amd64.tar.gz from https://go.dev/dl/ here first" >&2; exit 1 ;;
    *) echo "arm64 or amd64" >&2; exit 1 ;;
esac
cd "$(dirname "$0")/../.."
REPO="$PWD"
OUT="$REPO/packaging/out"
VERSION="${DMS_TAG#v}+waydesk1"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
mkdir -p "$OUT"
cat > "$WORK/build.sh" <<EOF
set -eu
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq && apt-get install -y -qq --no-install-recommends git curl ca-certificates make tar gzip python3 >/dev/null
cd /tmp
curl -fsSL --retry 5 --retry-delay 3 --retry-all-errors --connect-timeout 20 --max-time 300 -o go.tgz https://go.dev/dl/go$GO_VERSION.linux-$ARCH.tar.gz
echo "$GO_SHA256  go.tgz" | sha256sum -c -
tar -C /usr/local -xzf go.tgz
export PATH=/usr/local/go/bin:\$PATH
git config --global --add safe.directory '*'
# A stalled connection must fail and be retried, not hang the build.
git config --global http.lowSpeedLimit 1000
git config --global http.lowSpeedTime 30
retry() { for i in 1 2 3 4 5; do "\$@" && return 0; echo "failed (\$i): \$*; retrying" >&2; sleep 5; done; return 1; }
retry sh -c 'rm -rf /work/src && git clone -q --depth 1 --branch $DMS_TAG https://github.com/AvengeMedia/DankMaterialShell /work/src'
cd /work/src && retry git submodule update --init --depth 1
cd core && make sync-shell
for i in 1 2 3 4 5; do go mod download && break; echo "go mod download failed (\$i), retrying" >&2; sleep 5; done
(cd cmd/dms && CGO_ENABLED=0 GOOS=linux GOARCH=$ARCH go build -trimpath -tags 'distro_binary withshell' -ldflags "-s -w -X main.Version=$DMS_TAG" -o /work/dms)

# The package tree: what upstream's debian/rules installs.
P=/work/pkg
mkdir -p \$P/DEBIAN \$P/usr/bin \$P/usr/share/quickshell/dms \$P/usr/lib/systemd/user \$P/usr/share/applications \$P/usr/share/icons/hicolor/scalable/apps \$P/usr/share/doc/waydesk-shell
install -m 755 /work/dms \$P/usr/bin/dms
cp -rL /work/src/quickshell/. \$P/usr/share/quickshell/dms/
rm -rf \$P/usr/share/quickshell/dms/core \$P/usr/share/quickshell/dms/distro \$P/usr/share/quickshell/dms/tests
install -m 644 /work/src/assets/systemd/dms.service \$P/usr/lib/systemd/user/dms.service
# The binary carries its own copy of the UI and runs that unless DMS_SHELL_DIR names another: ours is the installed, patched tree.
sed -i 's|^Environment=QS_DISABLE_CRASH_HANDLER=1|&\\nEnvironment=DMS_SHELL_DIR=/usr/share/quickshell/dms|' \$P/usr/lib/systemd/user/dms.service
grep -q '^Environment=DMS_SHELL_DIR=' \$P/usr/lib/systemd/user/dms.service
install -m 644 /work/src/assets/dms-open.desktop /work/src/assets/dev.duxlabs.shell.desktop /work/src/assets/dev.duxlabs.shell.notepad.desktop \$P/usr/share/applications/
install -m 644 /work/src/assets/dev.duxlabs.shell.svg \$P/usr/share/icons/hicolor/scalable/apps/
# The waydesk patches, applied here, once: a patch that does not match this DMS version is reported (the build goes on, the line is in the log).
S=\$P/usr/share/quickshell/dms
/repo/packaging/desktop/dms-patches/apply.sh \$S \$S | tee /work/patches.log
test "\$(grep -c '^patched:' /work/patches.log)" -ge 5 || { echo "fewer than 5 patches applied: DMS changed, check the patches" >&2; exit 1; }
printf '%s\n' "$VERSION" > \$S/.waydesk-patched
find \$P -type d -exec chmod 755 {} + && find \$P -type f ! -perm -u+x -exec chmod 644 {} +
cp /work/src/LICENSE \$P/usr/share/doc/waydesk-shell/copyright
printf 'waydesk-shell is DankMaterialShell (Dux Shell) %s, MIT licence (above), built by the waydesk project with the patches of\npackaging/desktop/dms-patches applied. Source: https://github.com/AvengeMedia/DankMaterialShell\n' "$DMS_TAG" > \$P/usr/share/doc/waydesk-shell/README.waydesk
cat > \$P/DEBIAN/control <<C
Package: waydesk-shell
Version: $VERSION
Architecture: $ARCH
Maintainer: José Mendonça <jmendonca@gmail.com>
Section: x11
Priority: optional
Installed-Size: \$(du -sk \$P | cut -f1)
Depends: quickshell | quickshell-git, accountsservice, cava, danksearch, dgop, matugen, qml6-module-qtcore, qml6-module-qtmultimedia, qml6-module-qtqml, qml6-module-qtquick, qml6-module-qtquick-controls, qml6-module-qtquick-dialogs, qml6-module-qtquick-effects, qml6-module-qtquick-layouts, qml6-module-qtquick-templates, qml6-module-qtquick-window, qt6ct, python3
Suggests: cups-pk-helper
Provides: dms (= ${DMS_TAG#v})
Conflicts: dms, dms-git
Replaces: dms, dms-git
Description: DankMaterialShell for waydesk sessions, with the waydesk patches
 DankMaterialShell (Dux Shell), built from the upstream source with the changes waydesk needs on sway
 (window PID, pin to desktop, fewer settings pages, power menu) already applied, so that no account has to patch a copy at start.
C
dpkg-deb --root-owner-group -Zxz -b \$P /out/ >/dev/null
EOF
docker run --rm --platform linux/$ARCH -v "$WORK:/work" -v "$OUT:/out" -v "$REPO:/repo:ro" debian:trixie-slim bash /work/build.sh
ls -la "$OUT"/waydesk-shell_"$VERSION"_"$ARCH".deb
