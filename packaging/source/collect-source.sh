#!/bin/sh
# Collects the corresponding source of the copyleft programs that the Waydesk repository ships as binaries, for publication next to the packages
# (https://waydesk.dev/apt/source/): GPL-2.0 section 3 and LGPL-3.0 section 4 want the source offered from the same place as the binaries.
#
#   packaging/source/collect-source.sh OUTDIR     Needs Docker and network. Verifies what has a pinned checksum, writes SHA256SUMS and README.
#
# What is in it, and why (permissive programs we also build, sway, wlroots, DankMaterialShell, danksearch, dgop, need no source offer):
#   matugen 4.2.0       GPL-2.0-or-later   upstream tag v4.2.0, built unmodified (packaging/dms/build-helpers.sh); with the crates it is built from (cargo vendor, Cargo.lock)
#   quickshell 0.3.0    LGPL-3.0-only      Debian's orig tarball, built unmodified (packaging/quickshell/build-private.sh)
#   pcmanfm-qt          GPL-2.0-or-later   the Debian 13 and the Ubuntu 26.04 source packages, built with the two patches of packaging/pcmanfm-qt/ (modified)
set -eu
OUT=${1:?usage: collect-source.sh OUTDIR}
HERE=$(cd "$(dirname "$0")/../.." && pwd)
mkdir -p "$OUT"
OUT=$(cd "$OUT" && pwd)
MATUGEN_TAG=v4.2.0
QS_SHA256=205216575e948f23836917770473ae5fb3f980098cc706dd293833422c56b3f5

# quickshell: the very tarball the build verifies
mkdir -p "$OUT/quickshell"
curl -fsSL --retry 3 -o "$OUT/quickshell/quickshell_0.3.0.orig.tar.xz" http://deb.debian.org/debian/pool/main/q/quickshell/quickshell_0.3.0.orig.tar.xz
echo "$QS_SHA256  $OUT/quickshell/quickshell_0.3.0.orig.tar.xz" | shasum -a 256 -c - >/dev/null

# matugen: the tag, and the crates of its Cargo.lock so that it builds without crates.io
mkdir -p "$OUT/matugen"
docker run --rm -v "$OUT/matugen:/out" -e TAG="$MATUGEN_TAG" debian:13 sh -c '
set -eu
apt-get update -qq && apt-get install -y -qq --no-install-recommends git curl xz-utils ca-certificates gcc libc6-dev >/dev/null
# the same pinned Rust toolchain as packaging/dms/build-helpers.sh (checked sha256)
case "$(uname -m)" in
    aarch64) T=aarch64-unknown-linux-gnu; H=a9c168dbea81a84671c6b7a5c1795cb0eb699020110d107ddfca427bd797e697 ;;
    x86_64)  T=x86_64-unknown-linux-gnu;  H=5be191e8c70a5007074254cb74d78dbfdd25cd5a61c27de7b8e5410122ec24f4 ;;
esac
curl -fsSL --retry 5 --retry-all-errors -o /tmp/rustup-init https://static.rust-lang.org/rustup/dist/$T/rustup-init
echo "$H  /tmp/rustup-init" | sha256sum -c - >/dev/null
chmod +x /tmp/rustup-init && /tmp/rustup-init -y -q --profile minimal --default-toolchain 1.90.0 --no-modify-path >/dev/null 2>&1
export PATH=$HOME/.cargo/bin:$PATH
git clone -q --depth 1 --branch "$TAG" https://github.com/InioX/matugen /w
cd /w
git rev-parse HEAD > /out/matugen-${TAG#v}.commit
git archive --format=tar --prefix=matugen-${TAG#v}/ HEAD | xz -9 > /out/matugen-${TAG#v}.tar.xz
cargo vendor --locked /tmp/vendor 2>/dev/null | sed "s|/tmp/vendor|vendor|" > /tmp/cargo-config.toml
tar -C /tmp -cf - vendor | xz -9 > /out/matugen-${TAG#v}-vendored-crates.tar.xz
cp /tmp/cargo-config.toml /out/matugen-${TAG#v}-cargo-config.toml
'

# pcmanfm-qt: the distributions' own source packages, and our patches
for image in debian:13 ubuntu:26.04; do
    tag=$(printf '%s' "$image" | sed 's/^debian:/deb/; s/^ubuntu:/ubuntu/')
    mkdir -p "$OUT/pcmanfm-qt/$tag"
    docker run --rm -v "$OUT/pcmanfm-qt/$tag:/out" "$image" sh -c '
set -eu
for f in /etc/apt/sources.list.d/*.sources; do sed -i "s/^Types: deb\$/Types: deb deb-src/" "$f"; done
apt-get update -qq >/dev/null && apt-get install -y -qq --no-install-recommends dpkg-dev >/dev/null
cd /tmp && apt-get source --download-only pcmanfm-qt >/dev/null 2>&1
cp pcmanfm-qt_* /out/
'
done
mkdir -p "$OUT/pcmanfm-qt/waydesk-patches"
cp "$HERE"/packaging/pcmanfm-qt/*.patch "$OUT/pcmanfm-qt/waydesk-patches/"

cat > "$OUT/README" <<'EOF'
Source code of the copyleft programs in the Waydesk repository
================================================================

The binary packages at https://waydesk.dev/apt contain the programs below. Their licences ask for the source to be available from the
same place as the binaries; this folder is that place. It is offered to everybody who can download the packages, at no charge, for as long
as the packages are on this server.

  matugen 4.2.0 (GPL-2.0-or-later)       matugen/
      matugen-4.2.0.tar.xz               the upstream source of the tag v4.2.0 (commit in matugen-4.2.0.commit); the package is built from it unmodified
      matugen-4.2.0-vendored-crates.tar.xz   the Rust crates of its Cargo.lock (cargo vendor, unpacks to vendor/). To build without crates.io:
                                         tar xf matugen-4.2.0.tar.xz; cd matugen-4.2.0; tar xf ../matugen-4.2.0-vendored-crates.tar.xz;
                                         mkdir .cargo; cp ../matugen-4.2.0-cargo-config.toml .cargo/config.toml; cargo build --release --locked --offline
                                         (Rust 1.90.0 was used; matugen needs 1.88 or newer)
  quickshell 0.3.0 (LGPL-3.0-only)       quickshell/
      quickshell_0.3.0.orig.tar.xz       Debian's copy of the upstream release; the package is built from it unmodified
  pcmanfm-qt (GPL-2.0-or-later)          pcmanfm-qt/
      deb13/, ubuntu26.04/               the source packages of Debian 13 and Ubuntu 26.04 (.dsc, orig and debian archives)
      waydesk-patches/                   the two patches applied on top of them for the Waydesk packages (they remove menu entries)

How the packages are built from these: https://github.com/zhoorta/waydesk (packaging/dms/build-helpers.sh, packaging/quickshell/build-private.sh,
packaging/pcmanfm-qt/build.sh). SHA256SUMS has the checksums of every file here. The tool that made this folder is packaging/source/collect-source.sh.

The programs of Waydesk itself and the others it builds (sway, wlroots, DankMaterialShell, danksearch, dgop) are under permissive licences
(MIT or Apache-2.0); their source and our patches are in the same GitHub repository.

Questions: jose@stackcare.pt
EOF

(cd "$OUT" && find . -type f ! -name SHA256SUMS | sort | sed 's|^\./||' | xargs shasum -a 256 > SHA256SUMS)
echo "collected in $OUT:"; (cd "$OUT" && find . -type f | sort | sed 's|^\./||' | xargs ls -la | awk '{print $5, $9}')
