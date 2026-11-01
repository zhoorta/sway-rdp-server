title: Other desktops
nav: Desktops
seo_title: COSMIC, KDE Plasma and Hyprland over RDP on a Linux server · Waydesk
description: Waydesk gives every account the Waydesk desktop by default. An account can have COSMIC, KDE Plasma or Hyprland instead (experimental): how each one runs, what you install, what was tested and what was not.
---
Every account gets the Waydesk desktop (sway with a shell, a launcher and a dock) unless you choose otherwise. Three other desktops can be given to an account with one command. All three are **experimental** and were tested on Arch Linux. Plasma was also run on Debian 13 (four accounts at once on a 4-core server); COSMIC and Hyprland were not tested on Debian or Ubuntu.

## Choose one {bar: terminal}
```console
$ sudo waydesk-admin users set anna --desktop cosmic     # or plasma, hyprland, sway, default
$ sudo waydesk-admin config check anna                   # will this account's setup work as a session?
```
The setting is read when the session starts: end the session or log out first. `default` follows the server's `WAYDESK_COMPOSITOR`, which is `sway` unless you changed it. You install the desktop yourself with the system's package manager; Waydesk does not install COSMIC, Plasma or Hyprland.

## How they run {bar: notes}
- **COSMIC and KDE Plasma** run as one fullscreen window of the session's headless sway. Their own compositors cannot make a headless output (COSMIC exits without a monitor; KDE's RDP server only shares a session that already exists), so sway stays underneath and the client sees only the inner desktop. Waydesk bridges the text clipboard to the inner desktop and sets its keyboard layout. An idle session takes about 940 MB with COSMIC and 750 MB with Plasma 6.8. Three accounts of each of Plasma and COSMIC ran at once, beside sway and Omarchy sessions (twelve sessions on the i7, idle, on the hardware encoder); capacity under load was not measured.
- **Hyprland** (0.55 or later) is a compositor of its own, one per account, and needs a GPU. Set it per account with `--desktop hyprland` or for the whole server with `WAYDESK_COMPOSITOR=hyprland`. It was tested with Omarchy's configuration: on an i7-7700 with an HD 630, 20 heavy users were admitted before the memory rule refused the 21st, with the render engine saturated from about 8 users.
- **The Waydesk desktop** (sway) is the default and the one that is measured most: 21 heavy users on the same i7-7700, limited by memory.

## What was not tested {bar: notes}
- The lock screen of COSMIC.
- COSMIC and Plasma under load, and on Debian or Ubuntu.
- Hyprland without a GPU (it has no software renderer and the session will not start).

## Arch Linux {bar: notes}
There is a package for Arch and a signed pacman repository at `https://waydesk.dev/arch` for x86_64 and Arch Linux ARM (aarch64); the steps are in `docs/ARCH.md`. It has the server and the session; the Waydesk desktop packages are for Debian and Ubuntu. On 2026-11-01 the package was installed from that repository on Arch (x86_64, in a container with systemd) and on Arch Linux ARM (aarch64), the services were started and an RDP login got a sway session. Hyprland, COSMIC and Plasma sessions were last run on a real Arch machine with the previous version (0.4.0).
