# Waydesk OS: a Debian-based image (design, 2026-10-22)

Not a new distribution: Debian 13 with Waydesk installed and configured, as an image that boots into a working server. Security updates keep
coming from Debian; Waydesk's own packages come from `https://waydesk.dev/apt` (signed, see below). Nothing here is built as an image yet:
the parts that exist and are tested are marked.

## The first administrator (a requirement of every install path)

RDP logs in with an account's own password (PAM), so a server where nobody has an account in the group `waydesk-users` cannot be used. Every way of
installing therefore ends with a first administrator, who gets: RDP login, the administrator panels of the desktop (group `sudo`), Flatpak installs
without a password (group `waydesk-admins`). There is no default account and no default password anywhere.

| Install path | How the administrator is made | Status |
|---|---|---|
| `apt install waydesk` on a Debian server | the account that ran the installation (`sudo`) joins `waydesk-users` automatically; if nobody can log in, the installer prints `sudo waydesk-setup-admin` | postinst hint written; the tool tested |
| Any running server, later | `sudo waydesk-setup-admin` asks for the name and the password twice (at least 10 characters), or `--name NAME --password-stdin` / `--password-hash HASH` for automation, or `--existing NAME` for an account that exists | **tested** on the dev VM: new account, existing account, refusals (short password, bad name, existing name), interactive flow through a terminal, and a real RDP login with the account it made |
| Cloud image (Hetzner and similar) | cloud-init `write_files` puts `/etc/waydesk/firstboot.conf` (name and a password *hash*, never clear text); `waydesk-firstboot` reads it at the first boot, makes the account, shreds the file | script **tested** (configured, and nothing configured); not yet in an image |
| ISO installer, VM console | the Debian installer makes the first user (sudo, with a password): it is added to `waydesk-users` by the postinst; `waydesk-firstboot` can also ask on the console | postinst path as above; console wizard not tested on a console |

`waydesk-firstboot` (`packaging/appliance/`) also makes this server's **own RDP certificate and key** at the first boot: an image is copied to many
servers and must not carry one shared private key. It marks itself done only once an administrator exists; until then it leaves a hint in the login
message (`/etc/motd.d/waydesk-first-admin`) and tries again at the next boot.

A cloud-init example (the hash comes from `mkpasswd -m yescrypt`, or `openssl passwd -6`):

```yaml
#cloud-config
write_files:
  - path: /etc/waydesk/firstboot.conf
    permissions: '0600'
    content: |
      WAYDESK_ADMIN_NAME=anna
      WAYDESK_ADMIN_PASSWORD_HASH='$6$...'
      WAYDESK_HOSTNAME=desk1
```

## What the image contains (planned)

- Debian 13 minimal + `waydesk` (server, desktop, shell), our repository preconfigured (`waydesk-repo`), `systemd-zram-generator`, `chromium`.
- A firewall that opens only SSH and the VPN port (UDP 51820), RDP bound to loopback or the VPN address (`docs/WIREGUARD.md`), password SSH off, automatic
  security updates (`unattended-upgrades`).
- `waydesk-firstboot.service` (first boot only).
- Build with `mkosi` or `debos` for arm64 and amd64; test by booting in QEMU on the Mac, then as a Hetzner snapshot. An installer ISO only if someone asks.

## The repository: waydesk.dev/apt (moved 2026-10-24)

- Served at `https://waydesk.dev/apt` (any static web server; the content is what `packaging/repo/make-repo.sh` writes). The URL is in `packaging/repo/waydesk.sources.in`.
  It was `https://waydesk.stackcare.pt/apt` until then; that copy is left as it was and is not updated any more, so servers that installed `waydesk-repo` 1.1 or earlier
  stay on it until they install `waydesk-repo` 1.2 by hand (there is no redirect).
- **The key is yours and stays with you.** `packaging/repo/make-key.sh` makes it (ed25519, signing only, 3 years, passphrase asked by gpg, a revocation certificate
  moved out of the key directory: keep that on a USB stick, offline). Nothing in this repository ever holds the real private key; the keys used in the tests
  were throwaway keys in `/tmp`.
- The server: the same nginx host as the other StackCare sites, one Unix account `waydesk` for the site and the repository (home `/home/waydesk`, `app` a link to the web root
  `/var/www/waydesk/app`; the site is in the root, the repository in `app/apt`). nginx never lets an index be cached (`dists/`, `waydesk-repo.deb` and the key answer `Cache-Control: no-cache`),
  packages are cached for 30 days (a package never changes under its file name), directory listing is off. Let's Encrypt through Certbot, the same access-log filter as the other sites.
  The ssh alias that `publish.sh` uses logs in as `waydesk` and needs `WAYDESK_PUBLISH_DIR=app/apt`.
- Publishing, in this order (the key stays with you):
  1. `packaging/repo/make-key.sh` (once), then `packaging/repo/build-repo-package.sh ~/.../waydesk-archive-keyring.gpg` (makes `waydesk-repo_<version>_all.deb` in `packaging/out`);
  2. `WAYDESK_REPO_KEY=<fingerprint> WAYDESK_GNUPGHOME=<dir> packaging/repo/make-repo.sh OUTDIR` (it also puts `waydesk-repo.deb` at the top);
  3. `packaging/repo/publish.sh OUTDIR` is a dry run; `packaging/repo/publish.sh OUTDIR --really --key <fingerprint>` publishes. It refuses unless `InRelease` verifies with the keyring, the
     `waydesk-repo` package carries the same key, and you typed that key's fingerprint (so a test key cannot go out by accident; tested, with a throwaway key and a mismatching package).
     It uses `tar` over ssh, so the server needs nothing but `tar`: new packages first, then the index files one by one with the signed ones last, then removes packages that are no longer named.
- A server joins with: `curl -fsSLO https://waydesk.dev/apt/waydesk-repo.deb && sudo apt install ./waydesk-repo.deb && sudo apt update`. The fingerprint of the key
  is published on the same site; check it before trusting (the first install cannot be verified by the repository itself).
- Tested with a throwaway key: signed repository accepted; tampered index, missing signature and a signature by another key all refused; the pin makes our
  `waydesk-shell`, `dms`, `danksearch`, `dgop` and `matugen` win over a third-party repository; `apt install ./waydesk-repo.deb` followed by `apt update` works.
- Key rotation: make a new key before the old one expires, ship it in a new `waydesk-repo` version signed by the old key, then switch the repository to the new key.
  A lost or stolen key: publish the revocation certificate, make a new key, and tell the servers to reinstall `waydesk-repo` (this needs a second channel: the website).
- `matugen` is GPL-2.0-or-later: the repository must offer its source (the upstream tag at least, ideally a `deb-src` entry) before it is published.

## Not decided yet

- The image's name and branding ("Waydesk" has no clearance yet, see the README); say "based on Debian" and use none of Debian's logos.
- Where the image files are hosted (1-2 GB each).
- Whether the ISO is worth making.
