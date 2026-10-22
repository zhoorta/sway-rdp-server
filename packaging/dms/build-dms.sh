#!/bin/sh
# Builds the dms binary (Dux Shell / DankMaterialShell core, with the QML shell embedded) from the upstream source, in a clean Debian 13
# container, with a pinned and checksum-verified Go toolchain. First step of our own DMS package (docs/DMS-PACKAGE.md); not part of
# build-deb.sh yet.
#
#   packaging/dms/build-dms.sh [arm64|amd64]     output: packaging/out/dms-<version>-<arch>
#
# Upstream's own Debian package does not build from source: it downloads this binary from their GitHub release. The build needs the network
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
OUT="$PWD/packaging/out"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
mkdir -p "$OUT"
cat > "$WORK/build.sh" <<EOF
set -eu
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq && apt-get install -y -qq --no-install-recommends git curl ca-certificates make tar gzip >/dev/null
cd /tmp
curl -fsSL --retry 5 --retry-delay 3 --retry-all-errors -o go.tgz https://go.dev/dl/go$GO_VERSION.linux-$ARCH.tar.gz
echo "$GO_SHA256  go.tgz" | sha256sum -c -
tar -C /usr/local -xzf go.tgz
export PATH=/usr/local/go/bin:\$PATH
git config --global --add safe.directory '*'
git clone -q --depth 1 --branch $DMS_TAG https://github.com/AvengeMedia/DankMaterialShell /work/src
cd /work/src && git submodule update --init --depth 1
cd core && make sync-shell
cd cmd/dms
CGO_ENABLED=0 GOOS=linux GOARCH=$ARCH go build -trimpath -tags 'distro_binary withshell' -ldflags "-s -w -X main.Version=$DMS_TAG" -o /out/dms-$DMS_TAG-$ARCH
EOF
docker run --rm --platform linux/$ARCH -v "$WORK:/work" -v "$OUT:/out" debian:trixie-slim bash /work/build.sh
ls -la "$OUT/dms-$DMS_TAG-$ARCH"
