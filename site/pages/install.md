title: Install and first login
nav: Install
description: Install Waydesk on Debian 13 or Ubuntu 26.04, make the first administrator, connect, and add more people. About ten minutes from a fresh server.
---
From a fresh server to a desktop in about ten minutes: one package, one command to make the first administrator, and an RDP client. Everything below was run on a clean Debian 13 server.

## See it first
```html
<figure class="demo">
<video muted loop playsinline preload="metadata" poster="/demo-poster.jpg" controls aria-label="A short recording of a Waydesk desktop: a terminal runs two commands, then the browser opens on the documentation and a window is dragged">
<source src="/demo.webm" type="video/webm">
<source src="/demo.mp4" type="video/mp4">
</video>
<figcaption>A 20-second silent loop, recorded inside a Waydesk session on a 4-vCPU cloud server with no GPU. The typing and window moves are scripted; the desktop is the real one.</figcaption>
</figure>
<script>(function () { var v = document.querySelector('video.demo, .demo video'); if (!v || matchMedia('(prefers-reduced-motion: reduce)').matches) return; v.controls = false; v.play().catch(function () { v.controls = true; }); v.addEventListener('click', function () { v.paused ? v.play() : v.pause(); }); })();</script>
```

## Install {bar: terminal}
Packages come from a signed apt repository. The first download cannot be verified by the repository itself, so compare its key with the fingerprint printed on the [home page](/#install) before you trust it.

```console
$ curl -fsSLO https://waydesk.dev/apt/waydesk-repo.deb
$ sudo apt install ./waydesk-repo.deb
$ gpg --show-keys --with-fingerprint /usr/share/keyrings/waydesk-archive-keyring.gpg
$ sudo apt update
$ sudo apt install waydesk
```

`waydesk` is the server and the desktop (the shell, the dock, a file manager and a terminal). On a server that should only run the RDP part, `waydesk-server` alone is enough. The RDP port is **closed to the network** until you open it.

## The first administrator {bar: terminal}
There is no default account or password. `waydesk-setup-admin` makes the first administrator, the account that connects over RDP and manages the others:

```console
$ sudo waydesk-setup-admin
```

It asks for an account name (lower case letters, digits, `-` and `_`), a password of at least ten characters (this is the RDP password), and then how you will connect:

1. From this machine, or through an SSH tunnel (the default: the port stays closed to the network).
2. From computers on this server's local network (the port opens to that network only).
3. Over a WireGuard VPN, made now (the port opens to the VPN only).

It also prints the **fingerprint of the server's RDP certificate**. Write it down: your client shows it on the first connection, and a client that does not compare can be fooled by a man in the middle. For scripts, `--name NAME --password-stdin --access local|lan|vpn` does the same without questions.

## Connect {bar: terminal}
The default needs one command on your own computer. It forwards a local port to the server's closed one:

```console
$ ssh -N -L 3389:127.0.0.1:3389 you@your-server  # keep this running
```

Then point the RDP client at `localhost` and log in as the account you made. If your computer already uses port 3389, forward `13389` instead and connect to `localhost:13389`. The [Connect](/docs/connect/) page has the other ways (a local network, a VPN, certificates you can verify).

Clients that were tried: the **Windows App on macOS**, **mstsc on Windows 11** (ARM, in a VM) and **FreeRDP** (Debian 13's `sdl-freerdp`, version 3.15). Ubuntu's own `xfreerdp3` 3.32 cannot decode the H.264 stream; use Remmina or Debian's client there.

## See what it does {bar: terminal}
```console
$ sudo waydesk-admin status
Memory    62.6 GB in all, 53.5 GB available
Swap      sda1 8.0 GB (0 MB used), zram0 16.0 GB (1.7 GB used)
Load      2.55 1.95 1.79 (8 cores)
GPU       Intel HD Graphics 630 (i915, renderD128): 61 % busy, 1100 MHz
          new sessions use: off
Sessions  2 running; a new one would be accepted (needs 9.4 GB available)

ACCOUNT  SESSION  MEMORY  SWAP  CPU  LIMIT    RENDER  ENCODER          PROGRAMS (resident)
lt1      running  860 MB  -     29%  31.3 GB  gpu     vaapi q40 20fps  chromium 1066 MB, qs 329 MB, sway 104 MB
lt2      running  736 MB  -     10%  31.3 GB  cpu     openh264 5fps    chromium 1109 MB, qs 399 MB, waydesk-server 77 MB
```

Real output, trimmed: Debian 13 on a Core i7-7700 with its HD 630, two accounts: one session on the GPU (VAAPI), one with the GPU off (OpenH264 on the CPU). The systems and clients that were tried are on [Limits and status](/docs/limits/).

## What you get {bar: desktop}
- **A desktop of your own for every account**, with a shell, a dock, a file manager and your applications. Reconnect and the windows are where you left them: closing the RDP window does not end the session.
- **The size follows the client.** A Retina Mac or a laptop at 200 % scaling gets a sharp picture, and the window resizes live.
- **Chromium** is installed with the desktop (from Flathub on Ubuntu). `waydesk-setup`, run as your own account, offers browsers, office, developer and creative applications.
- **Windows** are floating: drag a title bar, use the three dots for minimise, maximise and close. Super+Up maximises, Super+Down minimises, Super+Shift+Up brings every window entirely back on the screen.

## Add more people {bar: terminal}
Administrators make the other accounts:

```console
$ sudo waydesk-admin users add alice
made alice (uid 1000), groups: waydesk-users
temporary password (shown once): ************
The account should change it at once: Change password, in the desktop's power menu.
$ sudo waydesk-admin users limit alice --memory 4G
$ waydesk-admin status
```

A new session is refused when the machine would run out of memory, so one account cannot take everyone else down. The [command line](/docs/cli/) page shows every command with its real output, and the [GPU](/docs/gpu/) page what changes on a machine with a graphics card.
