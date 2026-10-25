# Changelog

What changed, newest first. The numbered releases are the ones in the apt repository; the first block is in the repository but not released yet.

## In the repository, not released yet

Who can reach the port is set with one command each: `waydesk-admin access` (this machine, a local network or a VPN; the daemon closes source addresses that are not allowed before TLS and before any login), `cert local-ca` (a certificate that clients can verify), `vpn init` and `vpn add` (WireGuard), and `waydesk-setup-admin` asks how you will connect. X11 applications run (XWayland, one server per account). The mouse pointer has its real shapes (text cursor, resize arrows, hand), also for applications that draw their own cursor. TCP BBR for the connection, and flow control that follows the link: frames and bytes in flight, and a video bitrate that follows what the link delivers. Title bars and borders in the colours of the theme (dark or light), window buttons as a capsule, soft shadows and rounded title-bar corners. The dock floats over the bottom of the screen and bounces while an application starts. Flatpak and GTK applications no longer wait minutes for a dead portal after a login.

## 2026-10-24 · 0.1.0

First numbered release: `waydesk-server`, `waydesk-desktop` and `waydesk` 0.1.0 for Debian 13 and Ubuntu 26.04, amd64 and arm64. `waydesk-repo` 1.3 moves servers that still use the old repository address to waydesk.dev. The source of the copyleft programs in the repository (matugen, quickshell, pcmanfm-qt) is offered at [/apt/source/](/apt/source/).

## 2026-10-24

Ubuntu 26.04 checked from the repository; `dms.sh` no longer adds Debian backports when the Waydesk repository is configured (it broke `apt update` on Ubuntu); a client that stops reading is dropped after 60 s; `waydesk-server` alone starts a session with a minimal sway config; the Flathub Chromium opens without its terms window and keyring prompt; seven heavy users fit on 8 GB.

## 2026-10-23

Two suites, `trixie` and `resolute`, from one pool; private sway and quickshell under `/usr/lib/waydesk`; Flathub fallback for browsers; heavy-user capacity on Debian; fix for a publishing tool that deleted live packages.

## 2026-10-22

Signed apt repository, `waydesk-repo` and `waydesk-setup-admin`; our own build of the shell as a package; refused logins tell the client why.

## 2026-10-21

Memory admission, account ceilings and zram; patched sway with title-bar buttons; dock; Chromium as the default browser; third internal security pass.

## 2026-10-20

Renamed to Waydesk. Desktop icons, themed Qt and Flatpak apps, logind session per account.

## 2026-10-15 to 19

RDP server for sway, then multi-user daemon, helper and sessions; DankMaterialShell on sway.
