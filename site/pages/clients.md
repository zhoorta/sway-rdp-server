title: Set up your RDP client
nav: RDP clients
seo_title: Connect to a Linux RDP desktop from macOS, Windows or Linux: Waydesk clients
description: Step by step for the Windows App on macOS, mstsc on Windows and FreeRDP on Linux: the tunnel, the account, the certificate and the first connection.
---
Waydesk speaks standard RDP, so any RDP client can connect. This page shows the ones tried so far, in the order a first connection goes. Remmina, iOS and Android clients were not tried.

## What you need first {bar: notes}
- **An account.** The one `waydesk-setup-admin` made, or one an administrator made with `waydesk-admin users add NAME`.
- **A way to the port.** By default it listens on the server's own `127.0.0.1` only, so you reach it through an SSH tunnel (below), or the server was opened to a local network or a VPN, see [Connect](/docs/connect/).
- **The certificate fingerprint**, to compare on the first connection: `sudo waydesk-admin cert status` on the server prints it. A [local certificate authority](/docs/connect/#a-certificate-you-can-verify) makes the comparison unnecessary.

The tunnel, from your own computer (macOS, Linux, or PowerShell on Windows 10 and 11, which has an OpenSSH client):

```console
$ ssh -N -L 3389:127.0.0.1:3389 you@your-server  # keep this running
```

If your computer already uses port 3389 (a Windows PC with Remote Desktop switched on does), forward `13389` instead (`-L 13389:127.0.0.1:3389`) and connect to `localhost:13389`.

## Windows App on macOS {bar: notes}
Tried, works, with the Retina display sharp and the window resizing live.

1. Start the tunnel and leave it running.
2. In the Windows App, add a PC and give it the name `localhost` (or `localhost:13389`).
3. Give it the Waydesk account's name and password.
4. Connect. The first time, the app shows the server's certificate: compare its fingerprint with the one from `waydesk-admin cert status`, then accept.

A Mac keeps some shortcuts for itself: **Cmd+Space opens Spotlight** and never reaches the session, so a desktop that uses Super+Space (Omarchy's menu or launcher) cannot be opened that way. Turn Spotlight's shortcut off or move it (System Settings, Keyboard, Keyboard Shortcuts, Spotlight), or use another binding of the desktop. A MacBook's keyboard sends some keys to other places than a PC's. Waydesk follows the client's layout, and there is a Keyboard window in the desktop if a key arrives wrong.

## mstsc on Windows {bar: notes}
Tried on Windows 11 for ARM in a virtual machine, through an SSH tunnel; the VPN was not tried with it.

1. Start the tunnel in PowerShell (`ssh -N -L 13389:127.0.0.1:3389 you@your-server`).
2. Press Win+R, type `mstsc`, and enter `localhost:13389` as the computer. Under *Show options*, enter the account's name.
3. Connect and enter the password.
4. The certificate warning is expected with the server's own certificate. Windows shows a SHA-1 thumbprint where Waydesk prints a SHA-256 fingerprint, so the two cannot be compared by eye: for Windows clients the practical way is the [local certificate authority](/docs/connect/#a-certificate-you-can-verify), imported once with `certmgr.msc` into Trusted Root Certification Authorities.

## FreeRDP on Linux {bar: terminal}
Tried with Debian 13's `sdl-freerdp3` (FreeRDP 3.15).

```console
$ sudo apt install freerdp3-sdl
$ sdl-freerdp3 /v:localhost /u:alice /size:1680x1050 /gfx:AVC420
```

`/gfx:AVC420` asks for the H.264 stream Waydesk sends, and `/size` sets the window. It asks for the password and shows the certificate: compare the fingerprint with `waydesk-admin cert status`. **On Ubuntu**, the packaged `xfreerdp3` (3.32) cannot decode that stream: use Debian's client, or another client from the list above.

## If it does not connect {bar: notes}
Is the tunnel still running? Is the port the one you forwarded? `waydesk-admin access show` on the server says where it listens and who may connect, and [troubleshooting](/docs/help/) lists the rest in the order to check them.
