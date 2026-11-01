#!/bin/sh
# Builds a signed pacman repository from the Arch packages in packaging/out/arch (packaging/arch/build.sh makes them; docs/ARCH.md).
#
#   packaging/repo/make-arch-repo.sh OUTDIR
#
# Needs: Docker (repo-add, from an Arch Linux ARM image: it is a shell script and does not care for the architecture), gpg.
# Environment: WAYDESK_REPO_KEY  id of the signing key (required; the key of the apt repository). WAYDESK_GNUPGHOME: its keyring (default ~/.gnupg).
# Layout of OUTDIR (serve it as it is, any static web server): waydesk.gpg (public key, armored),
# <arch>/waydesk.db, waydesk.files (the databases, real files: pacman asks for these names) and their .sig,
# <arch>/<package>.pkg.tar.{zst,xz} and .sig. <arch> is x86_64 or aarch64 (the architecture is read from the package's file name).
# Packages: every package in packaging/out/arch except the -debug ones. Order: make-arch-repo.sh, then publish-arch.sh.
set -eu
OUT=${1:?usage: make-arch-repo.sh OUTDIR}
KEY=${WAYDESK_REPO_KEY:?set WAYDESK_REPO_KEY to the id of the signing key}
export GNUPGHOME=${WAYDESK_GNUPGHOME:-$HOME/.gnupg}
cd "$(dirname "$0")/../.."
SRC="$PWD/packaging/out/arch"
OUT=$(mkdir -p "$OUT" && cd "$OUT" && pwd)
set -- "$SRC"/*.pkg.tar.zst "$SRC"/*.pkg.tar.xz
list=""
for f in "$@"; do [ -f "$f" ] || continue; case "$f" in *-debug-*) continue ;; esac; list="$list $f"; done
[ -n "$list" ] || { echo "make-arch-repo.sh: no packages in $SRC" >&2; exit 1; }
gpg --list-secret-keys "$KEY" >/dev/null 2>&1 || { echo "make-arch-repo.sh: the secret key $KEY is not in $GNUPGHOME" >&2; exit 1; }
rm -rf "$OUT/x86_64" "$OUT/aarch64" "$OUT/waydesk.gpg"
for f in $list; do
    b=$(basename "$f")
    case "$b" in *-x86_64.pkg.tar.*) arch=x86_64 ;; *-aarch64.pkg.tar.*) arch=aarch64 ;; *) echo "make-arch-repo.sh: cannot read the architecture of $b" >&2; exit 1 ;; esac
    mkdir -p "$OUT/$arch"; cp "$f" "$OUT/$arch/$b"
    gpg --batch --yes -u "$KEY" --detach-sign --no-armor -o "$OUT/$arch/$b.sig" "$OUT/$arch/$b"
done
for d in "$OUT"/x86_64 "$OUT"/aarch64; do
    [ -d "$d" ] || continue
    docker run --rm -v "$d:/r" menci/archlinuxarm:latest sh -c 'cd /r && repo-add -q waydesk.db.tar.zst $(ls *.pkg.tar.zst *.pkg.tar.xz 2>/dev/null) >/dev/null && rm -f waydesk.db.tar.zst.old waydesk.files.tar.zst.old'
    # repo-add makes symlinks (waydesk.db -> waydesk.db.tar.zst); a static server and tar over ssh do better with real files under the names pacman asks for
    ( cd "$d" && rm -f waydesk.db waydesk.files && cp waydesk.db.tar.zst waydesk.db && cp waydesk.files.tar.zst waydesk.files && rm -f waydesk.db.tar.zst waydesk.files.tar.zst
      for x in waydesk.db waydesk.files; do gpg --batch --yes -u "$KEY" --detach-sign --no-armor -o "$x.sig" "$x"; done
      for s in *.sig; do gpg --batch --verify "$s" "${s%.sig}" 2>/dev/null || { echo "make-arch-repo.sh: $s does not verify" >&2; exit 1; }; done )
done
gpg --armor --export "$KEY" > "$OUT/waydesk.gpg"
find "$OUT" -name '._*' -delete
echo "made $OUT (key $(gpg --with-colons --fingerprint "$KEY" | awk -F: '/^fpr/ {print $10; exit}'))"
( cd "$OUT" && find . -type f | sort | sed 's|^\./|  |' )
