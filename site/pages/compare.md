title: How it compares
nav: How it compares
seo_title: Waydesk compared with xrdp, GNOME Remote Desktop, X2Go, NoMachine, VNC
description: Waydesk next to xrdp, the GNOME remote desktop, X2Go, NoMachine and VNC: what each is for, and when to choose Waydesk.
---
A short, honest map. We have measured only Waydesk: nothing here claims that it is faster than the others, only what each tool is built around.

## The tools {bar: table}
| Tool | Built around | A good fit when |
|---|---|---|
| **Waydesk** | one Wayland session per account over standard RDP, H.264 tuned for screens; the Waydesk desktop (Sway) by default, or COSMIC, KDE Plasma or Hyprland for an account | you want several people on one Debian, Ubuntu or Arch server, with their own session and any RDP client, and a package to install |
| **xrdp** | RDP in front of an X11 session (Xorg or Xvnc) | you want the long-established option, with many desktop environments and distributions |
| **GNOME Remote Desktop** | the GNOME desktop's own RDP server | you already run GNOME and want to share or reach that desktop |
| **X2Go** | NX-style remote X11 sessions over SSH | you want everything inside SSH and use X11 applications |
| **NoMachine** | its own protocol and clients | you want its clients and features, and accept a product with commercial editions |
| **VNC** | sharing a framebuffer | you want the simplest thing, and bandwidth and per-user sessions matter less |

## When to choose Waydesk {bar: notes}
- You want **standard RDP** (the Windows App, mstsc, FreeRDP) and no Waydesk client to install.
- You want **one isolated session per account** with memory limits and new logins refused when the server is full, not a shared screen.
- You want **a desktop chosen per account**: the Waydesk desktop for most, KDE Plasma, COSMIC or Hyprland for those who ask, on the same server ([Other desktops](/docs/desktops/)).
- You want it **in one package**: `apt install waydesk` (or a pacman repository on Arch), with a desktop, a shell and a dock already set up on Debian and Ubuntu.
- You care about **slow links**: the encoder follows what the link delivers and the pointer is drawn by the client.

## When not to {bar: notes}
- You need an **X11 desktop environment** such as XFCE, MATE or KDE on X11: Waydesk sessions are Wayland. X11 applications run inside them through XWayland, but an X11 desktop does not.
- You need **KDE Plasma or COSMIC tested under load, or on Debian and Ubuntu**: they run inside the session's Sway and were tried on Arch (Plasma also on Debian 13), not under load. GNOME is not offered.
- You need **Windows-style features** beyond what was tested (see [Limits and status](/docs/limits/)).
- You need something **mature and widely deployed today**: xrdp has years of use behind it, and Waydesk is early.
