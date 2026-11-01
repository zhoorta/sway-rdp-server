title: Limits and status
nav: Limits and status
seo_title: Waydesk limits and status: tested systems, clients and capacity
description: What Waydesk is not yet: early software, the clients and GPUs it was tried with, what happens without a GPU, and how many users a server holds.
---
Waydesk is early, free software, written and tested by one person on real servers. This page says what was tried, what was not, and what to expect. Nothing here is hidden in a footnote elsewhere.

## Status {bar: notes}
- **Early software.** Version 0.5.0. Expect rough edges; [open an issue](https://github.com/zhoorta/waydesk/issues) when you find one.
- **Systems:** Debian 13 and Ubuntu 26.04, on amd64 and arm64 (the table below). Clean installs of the 0.5.0 packages from the repository passed on both systems and both architectures (the packages were installed in clean containers on 2026-11-01; the sessions were checked on the earlier versions). Arch Linux has a package and a pacman repository ([Other desktops](/docs/desktops/#arch-linux)). COSMIC, KDE Plasma and Hyprland sessions are experimental: tested on Arch, and Plasma also on Debian 13 ([Other desktops](/docs/desktops/)).
- **No second factor.** The login is the account's password inside TLS: keep the port behind an SSH tunnel or a [VPN](/docs/vpn/#security-what-protects-the-login).
- **Not an audited product.** Four internal review passes are written down, with what is still open.

## Systems {bar: table}
| System | amd64 | arm64 | Notes |
|---|---|---|---|
| Debian 13 (trixie) | tested | tested | clean install from the repository, a session with the shell, Chromium, LibreOffice, VS Code |
| Ubuntu 26.04 LTS | tested | tested | Chromium and Firefox come from Flathub (no Chromium deb) |
| Arch Linux | tested | tested (Arch Linux ARM) | the server and a sway session from the pacman repository; no desktop shell packages; Hyprland experimental |
| Ubuntu 24.04, Fedora | not yet | not yet | different Qt and wlroots; no rpm yet |

Tested means a clean server, SSH hardened first, the install steps, an RDP login from FreeRDP and a look at the session. Ubuntu's own `xfreerdp3` (3.32) cannot decode our H.264 stream; Debian 13's client (3.15) can. The Windows App on macOS was also used, and mstsc on Windows 11 (ARM, in a VM) with the GPU encoders; older Windows clients were not tested.

## Clients {bar: table}
| Client | Result |
|---|---|
| Windows App on macOS | tried, works |
| mstsc on Windows 11 (ARM, in a VM, with the GPU encoders) | tried, works |
| FreeRDP (Debian 13's `sdl-freerdp` 3.15) | tried, works |
| Ubuntu's own `xfreerdp3` 3.32 | cannot decode the H.264 stream: use Remmina or Debian's client |
| Older Windows clients, Linux and mobile clients other than the above | not tested |

## Without a GPU {bar: notes}
Everything runs on the CPU. OpenGL and Vulkan applications open and are slow: `glxgears`, `vkcube` and Zed opened on a server with no render node. A **WebGL page in the browser does not animate**, because Waydesk starts Chromium without its GPU process there (it saves memory); with SwiftShader it runs at about 8 fps for a full-screen page, using three of four cores. With a render node but the GPU switched off, `vkcube` failed to show its window. On a 4-core, 8 GB Debian 13 server with no render node, a session with the Waydesk desktop opened LibreOffice Calc, Blender 4.3.2, Kdenlive 24.12.3, darktable 5.0.1 and VS Code 1.144 one after the other, and each drew its window. Only that was looked at (a screenshot of each): their speed on the CPU was not measured. The idle session held 684 MB, and Blender alone about 1.2 GB. They were also tried with the GPU on and off on a machine that had one.

## With a GPU {bar: notes}
Tried on an Intel HD 630 and an NVIDIA RTX 4000 SFF Ada. NVIDIA needs the vendor's driver, 615 or newer (Debian's 550 cannot draw the desktop). AMD should work through VAAPI and was not tried. The GPU is shared by every session: ten users on full-screen WebGL get about 13 fps each on the HD 630, and an integrated GPU takes some system memory for each session. Not measured: several users on one NVIDIA card (GeForce cards limit simultaneous NVENC sessions; the encoder then falls back to OpenH264) and a card with its own memory under many users. More on the [GPU page](/docs/gpu/).

## How many users {bar: notes}
On an 8 GB, 4-core server **7 to 8 users** fit with Chromium, Calc and VS Code open and left idle (8 on Debian 13, 7 on Ubuntu 26.04). Users who are actually working cost more, and active users on an 8 GB server were not measured. With active users, a 12-vCPU, 23 GB server (Hetzner CPX52, 123.60 euros a month) held 12 of them before it refused the 13th: memory is the limit, about 1.6 GB per such user. Someone compiling or recalculating needs more, and the server refuses a new session instead of slowing everyone down. The figures and their method are on the [measurements](/docs/measurements/) page.
