title: Limits and status
nav: Limits and status
description: What Waydesk is not yet: early software, the clients and GPUs it was tried with, what happens without a GPU, and how many users a server holds.
---
Waydesk is early, free software, written and tested by one person on real servers. This page says what was tried, what was not, and what to expect. Nothing here is hidden in a footnote elsewhere.

## Status {bar: notes}
- **Early software.** Version 0.4.0. Expect rough edges; [open an issue](https://github.com/zhoorta/waydesk/issues) when you find one.
- **Systems:** Debian 13 and Ubuntu 26.04 on amd64 (arm64 packages are built; they have not been installed on a machine yet).
- **No second factor.** The login is the account's password inside TLS: keep the port behind an SSH tunnel or a [VPN](/docs/vpn/#security-what-protects-the-login).
- **Not an audited product.** Four internal review passes are written down, with what is still open.

## Clients {bar: table}
| Client | Result |
|---|---|
| Windows App on macOS | tried, works |
| mstsc on Windows 11 (ARM, in a VM, with the GPU encoders) | tried, works |
| FreeRDP (Debian 13's `sdl-freerdp` 3.15) | tried, works |
| Ubuntu's own `xfreerdp3` 3.32 | cannot decode the H.264 stream: use Remmina or Debian's client |
| Older Windows clients, Linux and mobile clients other than the above | not tested |

## Without a GPU {bar: notes}
Everything runs on the CPU. OpenGL and Vulkan applications open and are slow: `glxgears`, `vkcube` and Zed opened on a server with no render node. A **WebGL page in the browser does not animate**, because Waydesk starts Chromium without its GPU process there (it saves memory); with SwiftShader it runs at about 8 fps for a full-screen page, using three of four cores. With a render node but the GPU switched off, `vkcube` failed to show its window. Blender, Kdenlive, darktable, VS Code and LibreOffice were tried with the GPU on and off on a machine that had one, not on a machine without a render node.

## With a GPU {bar: notes}
Tried on an Intel HD 630 and an NVIDIA RTX 4000 SFF Ada. NVIDIA needs the vendor's driver, 615 or newer (Debian's 550 cannot draw the desktop). AMD should work through VAAPI and was not tried. The GPU is shared by every session: ten users on full-screen WebGL get about 13 fps each on the HD 630, and an integrated GPU takes some system memory for each session. Not measured: several users on one NVIDIA card (GeForce cards limit simultaneous NVENC sessions; the encoder then falls back to OpenH264) and a card with its own memory under many users. More on the [GPU page](/docs/gpu/).

## How many users {bar: notes}
On an 8 GB, 4-core server count **7 to 8 users** with Chromium, Calc and VS Code open if most are idle. A 12-vCPU, 23 GB server (Hetzner CPX52, 123.60 euros a month) held 12 of them before it refused the 13th: memory is the limit, about 1.6 GB per such user. Someone compiling or recalculating needs more, and the server refuses a new session instead of slowing everyone down. The figures and their method are on the [measurements](/docs/measurements/) page.
