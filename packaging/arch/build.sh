#!/bin/sh
# Builds the Arch package from the committed tree. Run it on an Arch machine as an ordinary user (needs base-devel, cargo, clang, git).
# Output: packaging/out/arch/waydesk-server-<version>-<rel>-<arch>.pkg.tar.zst   Usage: packaging/arch/build.sh [makepkg options]
set -e
cd "$(dirname "$0")/../.."
ver=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
out=packaging/out/arch
mkdir -p "$out"
git archive --format=tar.gz --prefix="waydesk-server-$ver/" -o "$out/waydesk-server-$ver.tar.gz" HEAD -- . ':(exclude)site' ':(exclude)branding' ':(exclude)packaging/out'
cp packaging/arch/PKGBUILD packaging/arch/waydesk-server.install packaging/arch/waydesk.sysusers packaging/arch/waydesk.tmpfiles "$out/"
cd "$out"
makepkg -f "$@"
