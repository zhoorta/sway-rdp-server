#!/bin/sh
# Builds a signed apt repository from the packages in packaging/out (docs/DMS-PACKAGE.md, stage 4).
#
#   packaging/repo/make-repo.sh OUTDIR
#
# Needs: the packages (build-deb.sh, dms/build-dms.sh, dms/build-helpers.sh), Docker (dpkg-scanpackages from the build image), gpg.
# Environment:
#   WAYDESK_REPO_KEY      id of the signing key (required). WAYDESK_GNUPGHOME: its keyring (default ~/.gnupg).
#   WAYDESK_REPO_SUITE    suite and codename of the repository (default trixie: the packages are built for Debian 13).
# Order: make-key.sh, build-repo-package.sh with the public key, then this, then publish.sh. Layout of OUTDIR (serve it as it is, any static web
# server): waydesk-repo.deb, pool/main/*.deb, dists/<suite>/main/binary-<arch>/Packages{,.gz,.xz},
# dists/<suite>/{Release,InRelease,Release.gpg}, waydesk-archive-keyring.gpg (the public key, binary).
# Only the newest version of each package and architecture goes in.
set -eu
OUT=${1:?usage: make-repo.sh OUTDIR}
KEY=${WAYDESK_REPO_KEY:?set WAYDESK_REPO_KEY to the id of the signing key}
SUITE=${WAYDESK_REPO_SUITE:-trixie}
export GNUPGHOME=${WAYDESK_GNUPGHOME:-$HOME/.gnupg}
cd "$(dirname "$0")/../.."
SRC="$PWD/packaging/out"
OUT=$(mkdir -p "$OUT" && cd "$OUT" && pwd)
rm -rf "$OUT/pool" "$OUT/dists"
mkdir -p "$OUT/pool/main"

# newest version of each package and architecture (by the version, not the date: dpkg --compare-versions in the image)
docker run --rm -v "$SRC:/in:ro" -v "$OUT:/out" waydesk-build-arm64 sh -c '
set -eu
cd /in
for f in *.deb; do
    printf "%s %s %s %s\n" "$(dpkg-deb -f "$f" Package)" "$(dpkg-deb -f "$f" Architecture)" "$(dpkg-deb -f "$f" Version)" "$f"
done | sort -k1,1 -k2,2 > /tmp/all.txt
prev=""
best=""
pick() { if [ -n "$best" ]; then cp "/in/$bestfile" /out/pool/main/; fi; }
while read pkg arch ver file; do
    key="$pkg $arch"
    if [ "$key" != "$prev" ]; then
        pick
        prev="$key"; best="$ver"; bestfile="$file"
    elif dpkg --compare-versions "$ver" gt "$best"; then
        best="$ver"; bestfile="$file"
    fi
done < /tmp/all.txt
pick
cd /out
# the repository package also goes at the top, where the install instructions download it
ls pool/main/waydesk-repo_*_all.deb >/dev/null 2>&1 && cp "$(ls pool/main/waydesk-repo_*_all.deb | tail -1)" /out/waydesk-repo.deb
# only the architectures that have packages of their own ("all" packages do not make an architecture)
for arch in $(ls pool/main | sed -n "s/.*_\\([a-z0-9-]*\\)\\.deb\$/\\1/p" | grep -v "^all\$" | sort -u); do
    d=dists/'"$SUITE"'/main/binary-$arch
    mkdir -p $d
    # packages of this architecture and the architecture-independent ones
    mkdir -p /tmp/scan && rm -rf /tmp/scan/* && cp pool/main/*_${arch}.deb pool/main/*_all.deb /tmp/scan/ 2>/dev/null || true
    (cd /tmp/scan && dpkg-scanpackages --multiversion . 2>/dev/null) | sed "s|^Filename: ./|Filename: pool/main/|" > $d/Packages
    gzip -9kf $d/Packages; xz -9kf $d/Packages
done
'
# the Release file: hashes of the index files
python3 - "$OUT" "$SUITE" <<'PY'
import hashlib, os, sys, time
out, suite = sys.argv[1:]
base = os.path.join(out, "dists", suite)
files = []
for root, _, names in os.walk(base):
    for n in sorted(names):
        if n in ("Release", "InRelease", "Release.gpg"):
            continue
        p = os.path.join(root, n)
        files.append((os.path.relpath(p, base), p))
lines = [
    "Origin: Waydesk", "Label: Waydesk", "Suite: " + suite, "Codename: " + suite,
    "Date: " + time.strftime("%a, %d %b %Y %H:%M:%S UTC", time.gmtime()),
    "Architectures: " + " ".join(sorted(d.replace("binary-", "") for d in os.listdir(os.path.join(base, "main")) if d.startswith("binary-"))), "Components: main",
    "Description: Waydesk packages (server, desktop, shell and helpers)",
    "SHA256:",
]
for rel, p in sorted(files):
    data = open(p, "rb").read()
    lines.append(" %s %16d %s" % (hashlib.sha256(data).hexdigest(), len(data), rel))
open(os.path.join(base, "Release"), "w").write("\n".join(lines) + "\n")
PY
gpg --batch --yes --local-user "$KEY" --clearsign -o "$OUT/dists/$SUITE/InRelease" "$OUT/dists/$SUITE/Release"
gpg --batch --yes --local-user "$KEY" --armor --detach-sign -o "$OUT/dists/$SUITE/Release.gpg" "$OUT/dists/$SUITE/Release"
gpg --batch --yes --export "$KEY" > "$OUT/waydesk-archive-keyring.gpg"
echo "repository in $OUT:"; (cd "$OUT" && find pool dists -type f | sort | sed 's/^/  /'); du -sh "$OUT" | sed 's/^/size: /'
