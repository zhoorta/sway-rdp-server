#!/bin/sh
# Builds the helpers of the DMS shell from the upstream source, as .deb files with the names of the third-party packages they replace
# (danksearch, dgop, matugen), in a clean Debian 13 container. See docs/DMS-PACKAGE.md, stage 3.
#
#   packaging/dms/build-helpers.sh [arm64|amd64]    output: packaging/out/{danksearch,dgop,matugen}_<version>_<arch>.deb
#
# The versions are the upstream ones plus +waydesk1, so they are newer than the OBS builds they replace (1.6.0, 1.6.2, 4.0.0) and
# apt picks ours when our repository is configured. Go: pinned toolchain with a checked sha256 (danksearch and dgop need 1.26/1.27 and
# Debian 13 has 1.24). Rust (matugen, needs rustc >= 1.88): pinned 1.90.0 through rustup-init with a checked sha256, `--locked` so the dependencies are exactly the ones of upstream's Cargo.lock.
# Licences: danksearch and dgop are MIT; matugen is GPL-2.0-or-later (a published repository must carry its source, see the doc).
set -eu
ARCH=${1:-arm64}
GO_VERSION=1.27.3
RUST_VERSION=1.90.0
case "$ARCH" in
    arm64) GO_SHA256=a06e99309dbcbdca1a8ccd2282c3ee146e55930886e26b4f1193269587165f49
           RUSTUP_TRIPLE=aarch64-unknown-linux-gnu; RUSTUP_SHA256=a9c168dbea81a84671c6b7a5c1795cb0eb699020110d107ddfca427bd797e697 ;;
    amd64) GO_SHA256=65efbc80fdc1fa2cd2631cad5b481777a68e8725f34e89f50fc6ac714e379b40
           RUSTUP_TRIPLE=x86_64-unknown-linux-gnu; RUSTUP_SHA256=5be191e8c70a5007074254cb74d78dbfdd25cd5a61c27de7b8e5410122ec24f4 ;;
    *) echo "arm64 or amd64" >&2; exit 1 ;;
esac
DANKSEARCH_TAG=v2.0.0
DGOP_TAG=v2.0.0
MATUGEN_TAG=v4.2.0
cd "$(dirname "$0")/../.."
OUT="$PWD/packaging/out"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
mkdir -p "$OUT"
cat > "$WORK/build.tmpl" <<'EOF'
set -eu
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq && apt-get install -y -qq --no-install-recommends git curl ca-certificates tar gzip gcc libc6-dev pkg-config >/dev/null
git config --global http.lowSpeedLimit 1000
git config --global http.lowSpeedTime 30
retry() { for i in 1 2 3 4 5; do "$@" && return 0; echo "failed ($i): $*; retrying" >&2; sleep 5; done; return 1; }
clone() { retry sh -c 'rm -rf /work/$1 && git clone -q --depth 1 --branch $2 $3 /work/$1' sh "$@"; }
cd /tmp
curl -fsSL --retry 5 --retry-delay 3 --retry-all-errors --connect-timeout 20 --max-time 600 -o go.tgz https://go.dev/dl/go@GO_VERSION@.linux-@ARCH@.tar.gz
echo "@GO_SHA256@  go.tgz" | sha256sum -c -
tar -C /usr/local -xzf go.tgz
export PATH=/usr/local/go/bin:$HOME/.cargo/bin:$PATH
# Rust: matugen's dependencies need rustc 1.88 or newer and Debian 13 has 1.85, so a pinned toolchain through rustup-init (checked sha256).
curl -fsSL --retry 5 --retry-delay 3 --retry-all-errors --connect-timeout 20 --max-time 600 -o /tmp/rustup-init https://static.rust-lang.org/rustup/dist/@RUSTUP_TRIPLE@/rustup-init
echo "@RUSTUP_SHA256@  /tmp/rustup-init" | sha256sum -c -
chmod +x /tmp/rustup-init
retry /tmp/rustup-init -y --profile minimal --default-toolchain @RUST_VERSION@ --no-modify-path

clone danksearch @DANKSEARCH_TAG@ https://github.com/AvengeMedia/danksearch
clone dgop @DGOP_TAG@ https://github.com/AvengeMedia/dgop
clone matugen @MATUGEN_TAG@ https://github.com/InioX/matugen

# --- Go
(cd /work/danksearch && retry go mod download && CGO_ENABLED=0 GOOS=linux GOARCH=@ARCH@ go build -trimpath -ldflags "-s -w -X main.Version=@DANKSEARCH_TAG@" -o /work/dsearch ./cmd/dsearch)
(cd /work/dgop && retry go mod download && CGO_ENABLED=0 GOOS=linux GOARCH=@ARCH@ go build -trimpath -ldflags "-s -w -X main.Version=@DGOP_TAG@" -o /work/dgop-bin ./cmd/dgop)
# --- Rust
(cd /work/matugen && retry cargo fetch --locked && cargo build --release --locked && cp target/release/matugen /work/matugen-bin)

mkdirs() { rm -rf $1; mkdir -p $1/DEBIAN $1/usr/bin $1/usr/share/doc/$2; }
control() { # dir package version depends description
    {
        echo "Package: $2"
        echo "Version: $3"
        echo "Architecture: @ARCH@"
        echo "Maintainer: StackCare <jose@stackcare.pt>"
        echo "Section: utils"
        echo "Priority: optional"
        echo "Installed-Size: $(du -sk $1 | cut -f1)"
        [ -n "$4" ] && echo "Depends: $4"
        echo "Description: $5"
        echo " Built by the waydesk project from the upstream source (see /usr/share/doc/$2/README.waydesk)."
    } > $1/DEBIAN/control
}
note() { # dir package sourcedir tag licence url
    cp /work/$3/LICENSE $1/usr/share/doc/$2/copyright
    printf '%s %s, licence %s, built unmodified from the upstream source:\n  %s  tag %s\nBuilt for the waydesk project (docs/DMS-PACKAGE.md).\n' "$2" "$4" "$5" "$6" "$4" > $1/usr/share/doc/$2/README.waydesk
}

P=/work/pkg-danksearch; mkdirs $P danksearch
install -m 755 /work/dsearch $P/usr/bin/dsearch; ln -s dsearch $P/usr/bin/danksearch
note $P danksearch danksearch @DANKSEARCH_TAG@ MIT https://github.com/AvengeMedia/danksearch
control $P danksearch @DANKSEARCH_VERSION@ "" "Fast file search utility for DMS"
dpkg-deb --root-owner-group -Zxz -b $P /out/ >/dev/null

P=/work/pkg-dgop; mkdirs $P dgop
install -m 755 /work/dgop-bin $P/usr/bin/dgop
note $P dgop dgop @DGOP_TAG@ MIT https://github.com/AvengeMedia/dgop
control $P dgop @DGOP_VERSION@ "" "Stateless CPU/GPU monitor for DankMaterialShell"
dpkg-deb --root-owner-group -Zxz -b $P /out/ >/dev/null

P=/work/pkg-matugen; mkdirs $P matugen
install -m 755 /work/matugen-bin $P/usr/bin/matugen
deps=$(ldd /work/matugen-bin | awk '/=>/ {print $3}' | xargs -r dpkg -S 2>/dev/null | cut -d: -f1 | sort -u | grep -v '^libc6$' | tr '\n' ',' | sed 's/,$//; s/,/, /g')
note $P matugen matugen @MATUGEN_TAG@ GPL-2.0-or-later https://github.com/InioX/matugen
control $P matugen @MATUGEN_VERSION@ "libc6${deps:+, $deps}" "Material You color generation tool"
dpkg-deb --root-owner-group -Zxz -b $P /out/ >/dev/null
ls -la /out/danksearch_* /out/dgop_* /out/matugen_*
EOF
sed -e "s|@ARCH@|$ARCH|g" -e "s|@GO_VERSION@|$GO_VERSION|g" -e "s|@RUST_VERSION@|$RUST_VERSION|g" -e "s|@RUSTUP_TRIPLE@|$RUSTUP_TRIPLE|g" -e "s|@RUSTUP_SHA256@|$RUSTUP_SHA256|g" -e "s|@GO_SHA256@|$GO_SHA256|g" \
    -e "s|@DANKSEARCH_TAG@|$DANKSEARCH_TAG|g" -e "s|@DGOP_TAG@|$DGOP_TAG|g" -e "s|@MATUGEN_TAG@|$MATUGEN_TAG|g" \
    -e "s|@DANKSEARCH_VERSION@|${DANKSEARCH_TAG#v}+waydesk1|g" -e "s|@DGOP_VERSION@|${DGOP_TAG#v}+waydesk1|g" -e "s|@MATUGEN_VERSION@|${MATUGEN_TAG#v}+waydesk1|g" \
    "$WORK/build.tmpl" > "$WORK/build.sh"
docker run --rm --platform linux/$ARCH -v "$WORK:/work" -v "$OUT:/out" debian:trixie-slim bash /work/build.sh
