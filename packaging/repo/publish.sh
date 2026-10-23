#!/bin/sh
# Uploads a repository made by make-repo.sh to the web server (cPanel host "focinhos"; the folder behind https://waydesk.stackcare.pt/apt).
#
#   packaging/repo/publish.sh REPO-DIR                       dry run: reads from the server and says what it would do
#   packaging/repo/publish.sh REPO-DIR --really --key FINGERPRINT
#
# Publishing needs the fingerprint of the key you mean to publish, typed by you: it must be the key in the repository's keyring, the repository's
# InRelease must verify with it, and the waydesk-repo package inside must carry the same key. This is so that a test key can never be published by
# accident. REPO-DIR is the OUTDIR of make-repo.sh; put waydesk-repo.deb in it
# first (packaging/repo/build-repo-package.sh makes it) and check that waydesk-archive-keyring.gpg is the key you mean to publish.
# Environment: WAYDESK_PUBLISH_HOST (ssh host, default focinhos), WAYDESK_PUBLISH_DIR (default waydesk-apt, relative to the home directory).
# The server has tar but no rsync, so this uses tar over ssh (COPYFILE_DISABLE: macOS tar would otherwise add ._* metadata files to the upload).
#
# Order, for servers that update while this runs: 1. packages that are not on the server yet (and the key and waydesk-repo.deb); 2. the index
# files, one by one, each replaced atomically with the signature files (Release, InRelease, Release.gpg) last; 3. packages that the new indexes
# no longer name. Nothing outside that folder is touched.
set -eu
SRC=${1:?usage: publish.sh REPO-DIR [--really]}
HOST=${WAYDESK_PUBLISH_HOST:-focinhos}
DIR=${WAYDESK_PUBLISH_DIR:-waydesk-apt}
REAL=""; WANT=""
shift
while [ $# -gt 0 ]; do
    case "$1" in
        --really) REAL=1; shift ;;
        --key) WANT=$(printf '%s' "${2:?--key needs a fingerprint}" | tr -d ' ' | tr a-f A-F); shift 2 ;;
        *) echo "publish.sh: unknown option $1" >&2; exit 2 ;;
    esac
done
SRC=$(cd "$SRC" && pwd)
[ -f "$SRC/dists/trixie/InRelease" ] && [ -d "$SRC/pool" ] || { echo "publish.sh: $SRC does not look like a repository from make-repo.sh" >&2; exit 1; }
[ -f "$SRC/waydesk-repo.deb" ] || echo "note: $SRC/waydesk-repo.deb is missing (the install instructions on the page need it)" >&2
fp=$(gpg --show-keys --with-colons "$SRC/waydesk-archive-keyring.gpg" 2>/dev/null | awk -F: '/^fpr/ {print $10; exit}')
echo "key in this repository: ${fp:-unknown}"
# The repository must verify with the key it ships, and the repository package must carry that key too.
gpgv --keyring "$SRC/waydesk-archive-keyring.gpg" "$SRC/dists/trixie/InRelease" >/dev/null 2>&1 || { echo "publish.sh: InRelease does not verify with waydesk-archive-keyring.gpg: refusing" >&2; exit 1; }
if [ -f "$SRC/waydesk-repo.deb" ]; then
    pkgfp=$(tar -xOf "$SRC/waydesk-repo.deb" data.tar.xz 2>/dev/null | tar -xOf - ./usr/share/keyrings/waydesk-archive-keyring.gpg 2>/dev/null | gpg --show-keys --with-colons 2>/dev/null | awk -F: '/^fpr/ {print $10; exit}')
    [ "$pkgfp" = "$fp" ] || { echo "publish.sh: waydesk-repo.deb carries key ${pkgfp:-unreadable}, not $fp: rebuild it with build-repo-package.sh" >&2; exit 1; }
fi
if [ -n "$REAL" ] && [ "$WANT" != "$fp" ]; then echo "publish.sh: --really needs --key $fp (the key you intend to publish, typed by you)" >&2; exit 1; fi
echo "target: $HOST:~/$DIR/   ${REAL:+(for real)}${REAL:-(dry run: nothing is changed)}"

tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
# what is on the server: "size path" for every file under pool/
ssh "$HOST" "mkdir -p '$DIR' && cd '$DIR' && find pool -type f -printf '%s %p\n' 2>/dev/null | sort -k2" > "$tmp/remote.txt" || true
(cd "$SRC" && find pool -type f | while read -r f; do printf '%s %s\n' "$(wc -c < "$f" | tr -d ' ')" "$f"; done | sort -k2) > "$tmp/local.txt"
# to upload: local entries (same size and name) that are not on the server; to delete: server entries that are not local any more
# (set differences with awk, not comm: comm needs both lists sorted the same way, and the Mac and the server sort + ~ and _ differently, which made it
# report files that were there as missing, and then delete them by name after they had been uploaded: 2026-10-23)
notin() { awk 'NR==FNR { seen[$0]; next } !($0 in seen)' "$2" "$1"; }   # the lines of $1 that are not in $2
notin "$tmp/local.txt" "$tmp/remote.txt" | cut -d' ' -f2- > "$tmp/upload.txt"
notin "$tmp/remote.txt" "$tmp/local.txt" | cut -d' ' -f2- > "$tmp/delete.txt"
echo "packages to upload: $(wc -l < "$tmp/upload.txt" | tr -d ' ')   to remove from the server: $(wc -l < "$tmp/delete.txt" | tr -d ' ')"
sed 's/^/  + /' "$tmp/upload.txt"; sed 's/^/  - /' "$tmp/delete.txt"
if [ -z "$REAL" ]; then
    echo "indexes: $(cd "$SRC" && find dists -type f | wc -l | tr -d ' ') files would be replaced; plus waydesk-archive-keyring.gpg and waydesk-repo.deb"
    echo "dry run only. Add --really to publish."; exit 0
fi

# 1. packages, the key and the repository package
{ cat "$tmp/upload.txt"; for f in waydesk-archive-keyring.gpg waydesk-repo.deb; do [ -f "$SRC/$f" ] && echo "$f"; done; } > "$tmp/first.txt"
if [ -s "$tmp/first.txt" ]; then COPYFILE_DISABLE=1 tar --exclude="._*" --exclude=.DS_Store -C "$SRC" -cf - -T "$tmp/first.txt" | ssh "$HOST" "cd '$DIR' && tar -xf - && find pool -type d -exec chmod 755 {} + && find pool -type f -exec chmod 644 {} +; chmod 644 waydesk-* 2>/dev/null; true"; fi
# The packages must really be on the server before any index names them (the upload above cannot report a cut-short transfer): compare the
# server's pool with the local one, by name and size. If anything is missing nothing else is changed; run this again and it uploads what is missing.
ssh "$HOST" "cd '$DIR' && find pool -type f -printf '%s %p\n' | sort -k2" > "$tmp/remote2.txt"
notin "$tmp/local.txt" "$tmp/remote2.txt" > "$tmp/missing.txt"
if [ -s "$tmp/missing.txt" ]; then
    echo "publish.sh: $(wc -l < "$tmp/missing.txt" | tr -d ' ') package(s) are not on the server after the upload. The indexes were NOT changed:" >&2
    sed 's/^/  ! /' "$tmp/missing.txt" >&2; echo "Run it again: it uploads what is missing." >&2; exit 1
fi
# 2. the indexes: unpack next to the real ones, then move each into place; the signed files last
ssh "$HOST" "cd '$DIR' && rm -rf .incoming && mkdir .incoming"
COPYFILE_DISABLE=1 tar --exclude="._*" --exclude=.DS_Store -C "$SRC" -cf - dists | ssh "$HOST" "cd '$DIR/.incoming' && tar -xf -"
ssh "$HOST" "cd '$DIR/.incoming' && find dists -type d -exec chmod 755 {} + && find dists -type f -exec chmod 644 {} + && cd '$DIR' 2>/dev/null || cd ~/'$DIR'
cd ~/'$DIR' && find .incoming/dists -type d | sed 's|^.incoming/||' | while read -r d; do mkdir -p \"\$d\"; done
for pass in index signed; do
  find .incoming/dists -type f | sed 's|^.incoming/||' | sort | while read -r f; do
    case \"\$f\" in */Release|*/InRelease|*/Release.gpg) [ \$pass = signed ] || continue ;; *) [ \$pass = index ] || continue ;; esac
    mv -f \".incoming/\$f\" \"\$f\"
  done
done
rm -rf .incoming
true"
# index files that the new repository does not have any more (an architecture that was dropped): everything under dists/ that is not in the new set
(cd "$SRC" && find dists -type f) > "$tmp/dists-wanted.txt"
[ -s "$tmp/dists-wanted.txt" ] || { echo "publish.sh: the list of index files is empty: refusing to clean up dists/" >&2; exit 1; }
ssh "$HOST" "cd ~/'$DIR' && find dists -type f > .dists-have && grep -vxFf - .dists-have | xargs -r rm -f -- ; rm -f .dists-have; find dists -type d -empty -delete" < "$tmp/dists-wanted.txt"
# 3. packages the new indexes no longer name
if [ -s "$tmp/delete.txt" ]; then ssh "$HOST" "cd ~/'$DIR' && xargs rm -f --" < "$tmp/delete.txt"; fi
# 4. what a client will do: ask the public URL for every package in the pool (apt writes + as %2b)
BASE=${WAYDESK_PUBLISH_URL:-https://waydesk.stackcare.pt/apt}
echo "checking every package at $BASE ..."
(cd "$SRC" && find pool -type f | sort) | while read -r f; do
    code=$(curl -s -o /dev/null -w '%{http_code}' "$BASE/$(printf '%s' "$f" | sed 's/+/%2b/g')")
    [ "$code" = 200 ] || echo "  $code $f"
done > "$tmp/httpbad.txt"
if [ -s "$tmp/httpbad.txt" ]; then cat "$tmp/httpbad.txt" >&2; echo "publish.sh: the packages above are not served: clients cannot install them (run publish.sh again)" >&2; exit 1; fi
echo "all $(wc -l < "$tmp/local.txt" | tr -d ' ') packages answer 200"
echo "published. Check: curl -s https://waydesk.stackcare.pt/apt/dists/trixie/InRelease | head -3"
