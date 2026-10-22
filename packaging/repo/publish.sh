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
# The server has tar but no rsync, so this uses tar over ssh.
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
comm -23 "$tmp/local.txt" "$tmp/remote.txt" | cut -d' ' -f2- > "$tmp/upload.txt"
comm -13 "$tmp/local.txt" "$tmp/remote.txt" | cut -d' ' -f2- > "$tmp/delete.txt"
echo "packages to upload: $(wc -l < "$tmp/upload.txt" | tr -d ' ')   to remove from the server: $(wc -l < "$tmp/delete.txt" | tr -d ' ')"
sed 's/^/  + /' "$tmp/upload.txt"; sed 's/^/  - /' "$tmp/delete.txt"
if [ -z "$REAL" ]; then
    echo "indexes: $(cd "$SRC" && find dists -type f | wc -l | tr -d ' ') files would be replaced; plus waydesk-archive-keyring.gpg and waydesk-repo.deb"
    echo "dry run only. Add --really to publish."; exit 0
fi

# 1. packages, the key and the repository package
{ cat "$tmp/upload.txt"; for f in waydesk-archive-keyring.gpg waydesk-repo.deb; do [ -f "$SRC/$f" ] && echo "$f"; done; } > "$tmp/first.txt"
if [ -s "$tmp/first.txt" ]; then tar -C "$SRC" -cf - -T "$tmp/first.txt" | ssh "$HOST" "cd '$DIR' && tar -xf - && find pool -type d -exec chmod 755 {} + && find pool -type f -exec chmod 644 {} +; chmod 644 waydesk-* 2>/dev/null; true"; fi
# 2. the indexes: unpack next to the real ones, then move each into place; the signed files last
ssh "$HOST" "cd '$DIR' && rm -rf .incoming && mkdir .incoming"
tar -C "$SRC" -cf - dists | ssh "$HOST" "cd '$DIR/.incoming' && tar -xf -"
ssh "$HOST" "cd '$DIR/.incoming' && find dists -type d -exec chmod 755 {} + && find dists -type f -exec chmod 644 {} + && cd '$DIR' 2>/dev/null || cd ~/'$DIR'
cd ~/'$DIR' && find .incoming/dists -type d | sed 's|^.incoming/||' | while read -r d; do mkdir -p \"\$d\"; done
for pass in index signed; do
  find .incoming/dists -type f | sed 's|^.incoming/||' | sort | while read -r f; do
    case \"\$f\" in */Release|*/InRelease|*/Release.gpg) [ \$pass = signed ] || continue ;; *) [ \$pass = index ] || continue ;; esac
    mv -f \".incoming/\$f\" \"\$f\"
  done
done
rm -rf .incoming
# indexes of architectures or suites that are not in the new repository any more
find dists -type f | while read -r f; do :; done"
# 3. packages the new indexes no longer name
if [ -s "$tmp/delete.txt" ]; then ssh "$HOST" "cd ~/'$DIR' && xargs rm -f --" < "$tmp/delete.txt"; fi
echo "published. Check: curl -s https://waydesk.stackcare.pt/apt/dists/trixie/InRelease | head -3"
