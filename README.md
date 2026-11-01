<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="branding/waydesk-logo-dark.svg">
    <img src="branding/waydesk-logo.svg" alt="Waydesk" height="64">
  </picture>
</p>

<p align="center">
  <a href="https://github.com/zhoorta/waydesk/releases/latest"><img alt="Release" src="https://img.shields.io/github/v/release/zhoorta/waydesk?label=release"></a>
  <img alt="License: AGPL-3.0-or-later" src="https://img.shields.io/badge/license-AGPL--3.0--or--later-blue">
  <img alt="Debian 13 and Ubuntu 26.04, amd64 and arm64" src="https://img.shields.io/badge/Debian%2013%20%7C%20Ubuntu%2026.04-amd64%20%7C%20arm64-informational">
  <a href="https://waydesk.dev"><img alt="Website" src="https://img.shields.io/badge/website-waydesk.dev-orange"></a>
</p>

# Waydesk: a Linux desktop over RDP, out of the box

One `apt install` on Debian 13 or Ubuntu 26.04 gives every account on the server its own desktop (sway, a shell, a dock and apps) over RDP. Sessions are kept
alive between connections, the picture is H.264 and no GPU is needed (an Intel, AMD or NVIDIA one is used when there is one). Connect with the Windows App, Remmina or any RDP client: nothing to install on the client.

<p align="center">
  <img src="docs/img/desktop.png" alt="A Waydesk session in the Windows App: the bar, desktop icons, a browser, a terminal showing waydesk-admin status, and the dock" width="860">
</p>

- **One desktop per account.** Each login gets its own headless sway, sized and scaled from what the client asks for (a Retina Mac or a laptop at 200 % scaling gets a sharp picture, and the window resizes live). Reconnect and the windows are where you left them.
- **Your choice of desktop.** The Waydesk desktop (sway) is the default; `waydesk-admin users set NAME --desktop cosmic|plasma|hyprland` gives an account [COSMIC](docs/COSMIC.md), [KDE Plasma](docs/PLASMA.md) or [Hyprland](docs/HYPRLAND.md) instead (tested on Arch with an Intel GPU, Omarchy and ML4W for Hyprland, which needs a GPU; COSMIC and Plasma run inside the session's sway). An Arch Linux package is in `packaging/arch` ([notes](docs/ARCH.md)).
- **Memory is protected.** A new session is refused when the machine is full, every account has ceilings, and compressed swap absorbs spikes. A 12-vCPU, 23 GB server held 12 heavy users (Chromium, Calc and VS Code open) before it refused the 13th: [measurements and methods](https://waydesk.dev/#measurements).
- **Secure by default.** The RDP port listens on loopback only; a WireGuard VPN, a local certificate authority and an address allowlist are built in (`waydesk-admin vpn init`, `waydesk-admin access`). The daemon is unprivileged and sandboxed, and a small root helper only checks a password and starts a session. See the [threat model](docs/THREAT-MODEL.md).
- **Administration included.** `waydesk-admin` and a Users & Access panel manage accounts, limits and sessions, and a Task Manager shows what each account uses.
- **Free software.** AGPL-3.0-or-later, built on [IronRDP](https://github.com/Devolutions/IronRDP) and [sway](https://swaywm.org).

## Install

Debian 13 or Ubuntu 26.04 LTS, amd64 or arm64, as an administrator of the server:

```
curl -fsSLO https://waydesk.dev/apt/waydesk-repo.deb
sudo apt install ./waydesk-repo.deb
gpg --show-keys --with-fingerprint /usr/share/keyrings/waydesk-archive-keyring.gpg   # must be C646 72B2 5ECD 4773 196C 3958 9D8E 8874 E676 17F4
sudo apt update && sudo apt install waydesk
sudo waydesk-setup-admin      # the first administrator
waydesk-setup                 # optional: browsers, office, developer and creative apps (as the administrator account, not root)
```

The first download cannot be verified by the repository itself, so compare the fingerprint above with the one printed. The RDP port listens on `127.0.0.1:3389`:
reach it through an SSH tunnel or a VPN ([WireGuard](docs/WIREGUARD.md)), never straight from the internet. There is no default account and no default password.
Then connect an RDP client to the server (through the tunnel) with the administrator's name and password, and check the certificate fingerprint it prints.

## Status

Early. Tested with FreeRDP and the Windows App on macOS. Not independently reviewed: four internal security passes are in
[docs/SECURITY-REVIEW.md](docs/SECURITY-REVIEW.md), and the open items are in the [threat model](docs/THREAT-MODEL.md). Other distributions are not supported yet (the packages are `.deb` only).
The clipboard is text only, and there is no hardware encoding yet; the [roadmap](docs/ROADMAP.md) lists what is next. To report a vulnerability, see [SECURITY.md](SECURITY.md).

## Documentation

- [Website](https://waydesk.dev): architecture, measurements and their methods, security and a reference
- [Guide](docs/GUIDE.md): the long form of this page: server and desktop packages, memory protection, who may log in, account management, running the daemon by hand
- [Reference](docs/REFERENCE.md): packages, commands, units, ports and files
- [Access](docs/ACCESS.md) and [WireGuard](docs/WIREGUARD.md): who can reach the port
- [Measurements](docs/MEASUREMENTS.md), [Portability](docs/PORTABILITY.md), [Changelog](CHANGELOG.md)

## Names and licence

**Waydesk** is a working name; `apt install waydesk` installs everything (`waydesk-server` and `waydesk-desktop`). Waydesk builds on the sway compositor and is not part of the sway
project. No trademark or package-name clearance has been done.

Waydesk is licensed under the GNU Affero General Public License, version 3 or (at your option) any later version: [LICENSE](LICENSE). In short: you may use, study, change and share it, for any purpose, including at a company and for paid work of your own. If you give others a modified version, or let them use a modified version over a network, they must be able to get its source under the same licence. Charging for installing, hosting or supporting it is allowed. The text decides, not this summary; if the AGPL does not fit your case, write to [StackCare](https://stackcare.pt/en).

The third-party code in the repository keeps its own licences: `patches/ironrdp-server` is a modified copy of IronRDP's `ironrdp-server` crate (MIT or Apache-2.0, files included there), and the other patches belong to the projects they patch.
The repository also ships three copyleft programs as binaries (matugen, quickshell, a patched pcmanfm-qt); their source is at <https://waydesk.dev/apt/source/>, see [the guide](docs/GUIDE.md#source-of-the-copyleft-programs).
