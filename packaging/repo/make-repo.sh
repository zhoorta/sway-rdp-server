#!/bin/sh
# Builds a signed apt repository from the packages in packaging/out (docs/DMS-PACKAGE.md, stage 4).
#
#   packaging/repo/make-repo.sh OUTDIR
#
# Needs: the packages (build-deb.sh, dms/build-dms.sh, dms/build-helpers.sh), Docker (dpkg-scanpackages from the build image), gpg.
# Environment:
#   WAYDESK_REPO_KEY      id of the signing key (required). WAYDESK_GNUPGHOME: its keyring (default ~/.gnupg).
#   WAYDESK_REPO_SUITES   the suites, as suite:tag pairs (default "trixie:deb13 resolute:ubuntu26.04": Debian 13 and Ubuntu 26.04). A package whose version ends in
#                         +<tag> (waydesk-sway_1.11+waydesk1+deb13, built per distribution) goes only into the suite with that tag; every other package (the server,
#                         the desktop, the shell and the helpers, which do not depend on the distribution) goes into all of them. All suites share one pool.
# Order: make-key.sh, build-repo-package.sh with the public key, then this, then publish.sh. Layout of OUTDIR (serve it as it is, any static web
# server): waydesk-repo.deb, pool/main/*.deb, dists/<suite>/main/binary-<arch>/Packages{,.gz,.xz},
# dists/<suite>/{Release,InRelease,Release.gpg}, waydesk-archive-keyring.gpg (the public key, binary).
# Only the newest version of each package and architecture goes in.
set -eu
OUT=${1:?usage: make-repo.sh OUTDIR}
KEY=${WAYDESK_REPO_KEY:?set WAYDESK_REPO_KEY to the id of the signing key}
SUITES=${WAYDESK_REPO_SUITES:-"trixie:deb13 resolute:ubuntu26.04"}
export GNUPGHOME=${WAYDESK_GNUPGHOME:-$HOME/.gnupg}
cd "$(dirname "$0")/../.."
SRC="$PWD/packaging/out"
OUT=$(mkdir -p "$OUT" && cd "$OUT" && pwd)
rm -rf "$OUT/pool" "$OUT/dists"
mkdir -p "$OUT/pool/main"

# newest version of each package and architecture, for every suite (by the version, not the date: dpkg --compare-versions in the image)
docker run --rm -v "$SRC:/in:ro" -v "$OUT:/out" -e SUITES="$SUITES" waydesk-build-arm64 sh -c '
set -eu
cd /in
for f in *.deb; do
    # read the whole archive: the control data is at the start, so a file that is still being copied (or was cut short) looks fine without this
    # (dpkg-deb tolerates an ar member whose declared size is wrong, as when two builds write the same file; `ar` and apt do not: 2026-10-23)
    dpkg-deb -c "$f" >/dev/null 2>&1 && ar t "$f" >/dev/null 2>&1 || { echo "make-repo.sh: $f is not a valid, complete .deb (still being copied? written by two builds at once?): nothing was made" >&2; exit 1; }
    printf "%s %s %s %s\n" "$(dpkg-deb -f "$f" Package)" "$(dpkg-deb -f "$f" Architecture)" "$(dpkg-deb -f "$f" Version)" "$f"
done | sort -k1,1 -k2,2 > /tmp/all.txt
TAGS=""; for e in $SUITES; do TAGS="$TAGS ${e##*:}"; done
for entry in $SUITES; do
    suite=${entry%%:*}; tag=${entry##*:}
    # eligible for this suite: no distribution tag in the version, or this suite'"'"'s tag
    while read pkg arch ver file; do
        skip=0
        for t in $TAGS; do case "$ver" in *+"$t") [ "$t" = "$tag" ] || skip=1 ;; esac; done
        [ $skip = 0 ] && printf "%s %s %s %s\n" "$pkg" "$arch" "$ver" "$file"
    done < /tmp/all.txt > /tmp/elig-$suite.txt
    # the newest of each package and architecture
    : > /tmp/win-$suite.txt
    prev=""; best=""; bestfile=""; bestarch=""
    flush() { [ -n "$bestfile" ] && printf "%s %s\n" "$bestarch" "$bestfile" >> /tmp/win-$suite.txt; return 0; }
    while read pkg arch ver file; do
        key="$pkg $arch"
        if [ "$key" != "$prev" ]; then
            flush; prev="$key"; best="$ver"; bestfile="$file"; bestarch="$arch"
        elif dpkg --compare-versions "$ver" gt "$best"; then
            best="$ver"; bestfile="$file"
        fi
    done < /tmp/elig-$suite.txt
    flush
    while read arch file; do cp "/in/$file" /out/pool/main/; done < /tmp/win-$suite.txt
    # only the architectures that have packages of their own ("all" packages do not make an architecture)
    for arch in $(cut -d" " -f1 /tmp/win-$suite.txt | grep -v "^all\$" | sort -u); do
        d=/out/dists/$suite/main/binary-$arch
        mkdir -p $d
        rm -rf /tmp/scan && mkdir -p /tmp/scan
        # packages of this architecture and the architecture-independent ones, of this suite only
        while read a file; do [ "$a" = "$arch" ] || [ "$a" = all ] && cp "/in/$file" /tmp/scan/; done < /tmp/win-$suite.txt
        (cd /tmp/scan && dpkg-scanpackages --multiversion . 2>/dev/null) | sed "s|^Filename: ./|Filename: pool/main/|" > $d/Packages
        gzip -9kf $d/Packages; xz -9kf $d/Packages
    done
    echo "suite $suite ($tag): $(wc -l < /tmp/win-$suite.txt) packages"
done
cd /out
# the repository package also goes at the top, where the install instructions download it
if ls pool/main/waydesk-repo_*_all.deb >/dev/null 2>&1; then cp "$(ls pool/main/waydesk-repo_*_all.deb | sort -V | tail -1)" /out/waydesk-repo.deb; fi
'
# the Release file of every suite: hashes of the index files
python3 - "$OUT" <<'PY'
import hashlib, os, sys, time
out = sys.argv[1]
for suite in sorted(os.listdir(os.path.join(out, "dists"))):
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
for suite in $(ls "$OUT/dists"); do
    gpg --batch --yes --local-user "$KEY" --clearsign -o "$OUT/dists/$suite/InRelease" "$OUT/dists/$suite/Release"
    gpg --batch --yes --local-user "$KEY" --armor --detach-sign -o "$OUT/dists/$suite/Release.gpg" "$OUT/dists/$suite/Release"
done
gpg --batch --yes --export "$KEY" > "$OUT/waydesk-archive-keyring.gpg"
echo "repository in $OUT:"; (cd "$OUT" && find pool dists -type f | sort | sed 's/^/  /'); du -sh "$OUT" | sed 's/^/size: /'
