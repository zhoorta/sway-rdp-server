#!/bin/sh
# Makes the signing key of the Waydesk apt repository. Run it yourself, on a machine you trust: the private key signs everything the
# servers install as root, so whoever has it can put code on every Waydesk server.
#
#   packaging/repo/make-key.sh [GNUPGHOME-directory]
#
# It asks for a passphrase (pinentry). Make a strong one and keep it in a password manager. The key is ed25519, signing only, valid for
# 3 years (renew before it ends: the keyring package carries the public key). It writes, next to the key directory:
#   waydesk-archive-keyring.gpg   the public key, binary: what apt trusts (packaging/repo/build-repo-package.sh puts it in waydesk-repo)
#   waydesk-archive-key.asc       the same, armoured: to publish next to the repository and to compare fingerprints
#   revoke-<fingerprint>.asc      the revocation certificate: keep it OFFLINE (a USB stick); it is how you cancel a lost or stolen key
# Then: WAYDESK_REPO_KEY=<fingerprint> WAYDESK_GNUPGHOME=<directory> packaging/repo/make-repo.sh OUTDIR
set -eu
home=${1:-$HOME/.waydesk-repo-gnupg}
uid=${WAYDESK_KEY_UID:-Waydesk repository (waydesk.stackcare.pt)}
mkdir -p "$home" && chmod 700 "$home"
export GNUPGHOME="$home"
if gpg --list-secret-keys --with-colons 2>/dev/null | grep -q '^sec'; then
    echo "make-key.sh: $home has a secret key already; refusing to make a second one" >&2; exit 1
fi
if [ -n "${WAYDESK_KEY_NOPASS:-}" ]; then   # for tests only
    gpg --batch --pinentry-mode loopback --passphrase '' --quick-generate-key "$uid" ed25519 sign 3y
else
    echo "Making the key. gpg will ask for a passphrase: choose a long one."
    gpg --quick-generate-key "$uid" ed25519 sign 3y
fi
fpr=$(gpg --list-keys --with-colons | awk -F: '/^fpr/ {print $10; exit}')
out=$(dirname "$home")
gpg --export "$fpr" > "$out/waydesk-archive-keyring.gpg"
gpg --armor --export "$fpr" > "$out/waydesk-archive-key.asc"
# gpg made a revocation certificate when it made the key: move it out of the key directory
rev="$home/openpgp-revocs.d/$fpr.rev"
[ -f "$rev" ] && mv "$rev" "$out/revoke-$fpr.asc"
echo
echo "Fingerprint (publish it, and check it on the servers): $fpr"
echo "Public key:  $out/waydesk-archive-keyring.gpg  and  $out/waydesk-archive-key.asc"
echo "Revocation:  $out/revoke-$fpr.asc   <- copy to a USB stick and remove it from this machine"
echo "Next:        WAYDESK_REPO_KEY=$fpr WAYDESK_GNUPGHOME=$home packaging/repo/make-repo.sh OUTDIR"
