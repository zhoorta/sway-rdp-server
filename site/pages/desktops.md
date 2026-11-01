title: Other desktops
nav: Desktops
seo_title: COSMIC, KDE Plasma and Hyprland over RDP on a Linux server · Waydesk
description: Waydesk gives every account the Waydesk desktop by default. An account can have COSMIC, KDE Plasma or Hyprland instead (all three tested on Arch): how each one runs, what you install, what was tested and what was not.
---
Every account gets the Waydesk desktop (sway with a shell, a launcher and a dock) unless you choose otherwise. Three other desktops can be given to an account with one command. All three were tested on Arch Linux with an Intel GPU (Hyprland with Omarchy 4.0.6 and ML4W; COSMIC and KDE Plasma with their lock screens, logout and reconnect), none of them under load. Plasma was also run on Debian 13 (four accounts at once on a 4-core server); COSMIC and Hyprland were not tested on Debian or Ubuntu, and other GPUs were not tried with Hyprland.

## The five desktops {bar: notes}
```html
<div class="gallery">
  <figure id="waydesk" class="win gtile" style="margin:0"><a href="/gallery/waydesk.webp"><img src="/gallery/waydesk.webp" width="1680" height="1050" loading="lazy" alt="Waydesk desktop with the shell bar, the dock, a browser showing this site and a terminal"></a><div class="body"><p><strong>Waydesk desktop.</strong> The default: a shell, a launcher and a dock. Debian 13 and Ubuntu 26.04 (amd64 and arm64) and Arch. An idle session takes about 0.7 GB.</p></div></figure>
  <figure id="plasma" class="win gtile" style="margin:0"><a href="/gallery/plasma.webp"><img src="/gallery/plasma.webp" width="1680" height="1050" loading="lazy" alt="KDE Plasma desktop with a terminal showing waydesk-admin status"></a><div class="body"><p><strong>KDE Plasma.</strong> Tested on Arch with an Intel GPU, and on Debian 13. You install Plasma yourself. An idle session takes about 0.65 GB.</p></div></figure>
  <figure id="cosmic" class="win gtile" style="margin:0"><a href="/gallery/cosmic.webp"><img src="/gallery/cosmic.webp" width="1680" height="1050" loading="lazy" alt="COSMIC desktop with a browser showing this site and the dock"></a><div class="body"><p><strong>COSMIC.</strong> Tested on Arch with an Intel GPU. You install COSMIC yourself. An idle session takes about 0.9 GB.</p></div></figure>
  <figure id="omarchy" class="win gtile" style="margin:0"><a href="/gallery/omarchy.webp"><img src="/gallery/omarchy.webp" width="1680" height="1050" loading="lazy" alt="Omarchy on Hyprland: a terminal showing waydesk-admin status next to a browser showing this site"></a><div class="body"><p><strong>Omarchy (Hyprland).</strong> Tested on Arch with an Intel GPU; Hyprland needs a GPU. An idle session takes about 0.75 GB.</p></div></figure>
  <figure id="ml4w" class="win gtile" style="margin:0"><a href="/gallery/ml4w.webp"><img src="/gallery/ml4w.webp" width="1680" height="1050" loading="lazy" alt="ML4W on Hyprland: a terminal and a browser showing this site, with the bar and the dock"></a><div class="body"><p><strong>ML4W (Hyprland).</strong> Tested on Arch with an Intel GPU; Hyprland needs a GPU. An idle session takes about 1.0 GB.</p></div></figure>
</div>
```

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
- COSMIC and Plasma under load, and on Debian or Ubuntu.
- Hyprland without a GPU (it has no software renderer and the session will not start).

## Arch Linux {bar: notes}
There is a package for Arch and a signed pacman repository at `https://waydesk.dev/arch` for x86_64 and Arch Linux ARM (aarch64); the steps are in `docs/ARCH.md`. It has the server and the session; the Waydesk desktop packages are for Debian and Ubuntu. On 2026-11-01 the package was installed from that repository on Arch (x86_64, in a container with systemd) and on Arch Linux ARM (aarch64), the services were started and an RDP login got a sway session. Hyprland, COSMIC and Plasma sessions were last run on a real Arch machine with the previous version (0.4.0).
