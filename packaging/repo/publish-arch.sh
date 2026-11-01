#!/bin/sh
# Uploads a repository made by make-arch-repo.sh to the web server (the folder behind https://waydesk.dev/arch).
#
#   packaging/repo/publish-arch.sh REPO-DIR                       dry run: reads from the server and says what it would do
#   packaging/repo/publish-arch.sh REPO-DIR --really --key FINGERPRINT
#
# --really needs the fingerprint of the key you mean to publish, typed by you; it must be the key in REPO-DIR/waydesk.gpg and every .sig must verify with it.
# Environment: WAYDESK_PUBLISH_HOST (required: ssh host or alias), WAYDESK_PUBLISH_ARCH_DIR (folder behind /arch, relative to the ssh account's home; default
# app/arch), WAYDESK_PUBLISH_ARCH_URL (default https://waydesk.dev/arch, for the final check).
# Order: packages and their signatures first (new files only; nothing already on the server is overwritten or deleted: an old package may still be named by a
# client's cached database), then the databases and their signatures, each replaced atomically, the .sig after its file.
set -eu
SRC=${1:?usage: publish-arch.sh REPO-DIR [--really --key FINGERPRINT]}
HOST=${WAYDESK_PUBLISH_HOST:?set WAYDESK_PUBLISH_HOST to the ssh host (or alias) of the web server}
DIR=${WAYDESK_PUBLISH_ARCH_DIR:-app/arch}
BASE=${WAYDESK_PUBLISH_ARCH_URL:-https://waydesk.dev/arch}
REAL=""; WANT=""
shift
while [ $# -gt 0 ]; do
    case "$1" in
        --really) REAL=1; shift ;;
        --key) WANT=$(printf '%s' "${2:?--key needs a fingerprint}" | tr -d ' ' | tr a-f A-F); shift 2 ;;
        *) echo "publish-arch.sh: unknown option $1" >&2; exit 2 ;;
    esac
done
SRC=$(cd "$SRC" && pwd)
[ -f "$SRC/waydesk.gpg" ] && ls "$SRC"/*/waydesk.db >/dev/null 2>&1 || { echo "publish-arch.sh: $SRC does not look like a repository from make-arch-repo.sh" >&2; exit 1; }
fp=$(gpg --show-keys --with-colons "$SRC/waydesk.gpg" 2>/dev/null | awk -F: '/^fpr/ {print $10; exit}')
echo "key in this repository: ${fp:-unknown}"
tmpk=$(mktemp -d); trap 'rm -rf "$tmpk"' EXIT
gpg --homedir "$tmpk" --batch --import "$SRC/waydesk.gpg" 2>/dev/null
gpg --homedir "$tmpk" --export > "$tmpk/k.gpg"
for s in "$SRC"/*/*.sig; do gpgv --keyring "$tmpk/k.gpg" "$s" "${s%.sig}" >/dev/null 2>&1 || { echo "publish-arch.sh: $s does not verify with waydesk.gpg: refusing" >&2; exit 1; }; done
if [ -n "$REAL" ] && [ "$WANT" != "$fp" ]; then echo "publish-arch.sh: --really needs --key $fp (the key you intend to publish, typed by you)" >&2; exit 1; fi
echo "target: $HOST:~/$DIR/   ${REAL:+(for real)}${REAL:-(dry run: nothing is changed)}"
(cd "$SRC" && find . -mindepth 2 -type f ! -name 'waydesk.db*' ! -name 'waydesk.files*' | sed 's|^\./||' | sort) > "$tmpk/pk.txt"
ssh "$HOST" "mkdir -p '$DIR' && cd '$DIR' && find . -mindepth 2 -type f | sed 's|^\./||' | sort" > "$tmpk/remote.txt" || true
awk 'FILENAME == ARGV[1] { seen[$0]; next } !($0 in seen)' "$tmpk/remote.txt" "$tmpk/pk.txt" > "$tmpk/new.txt"
echo "new package files: $(wc -l < "$tmpk/new.txt" | tr -d ' ')"; sed 's/^/  + /' "$tmpk/new.txt"
echo "databases replaced: $(cd "$SRC" && ls -d */ | tr -d / | tr '\n' ' ')(waydesk.db, waydesk.files and their .sig); also waydesk.gpg"
[ -n "$REAL" ] || { echo "dry run only. Add --really --key $fp to publish."; exit 0; }
{ cat "$tmpk/new.txt"; echo waydesk.gpg; } > "$tmpk/first.txt"
COPYFILE_DISABLE=1 tar --exclude="._*" --exclude=.DS_Store -C "$SRC" -cf - -T "$tmpk/first.txt" | ssh "$HOST" "cd '$DIR' && tar -xf - && find . -type d -exec chmod 755 {} + && find . -type f -exec chmod 644 {} +"
for d in $(cd "$SRC" && ls -d */ | tr -d /); do
    ssh "$HOST" "cd '$DIR/$d' && rm -rf .incoming && mkdir .incoming"
    (cd "$SRC/$d" && COPYFILE_DISABLE=1 tar --exclude="._*" -cf - waydesk.db waydesk.db.sig waydesk.files waydesk.files.sig) | ssh "$HOST" "cd '$DIR/$d/.incoming' && tar -xf - && chmod 644 * && cd .. && mv -f .incoming/waydesk.files .incoming/waydesk.files.sig . && mv -f .incoming/waydesk.db .incoming/waydesk.db.sig . && rmdir .incoming"
done
echo "checking every file at $BASE ..."
bad=0
(cd "$SRC" && find . -type f | sed 's|^\./||' | sort) | while read -r f; do
    code=$(curl -s -o /dev/null -w '%{http_code}' "$BASE/$(printf '%s' "$f" | sed 's/+/%2b/g')")
    [ "$code" = 200 ] || echo "  $code $f"
done > "$tmpk/bad.txt"
if [ -s "$tmpk/bad.txt" ]; then cat "$tmpk/bad.txt" >&2; echo "publish-arch.sh: the files above are not served" >&2; exit 1; fi
echo "published; every file answers 200"
