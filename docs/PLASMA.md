# KDE Plasma sessions

`waydesk-admin users set NAME --desktop plasma` gives an account KDE Plasma (Wayland). Needs Plasma installed (Arch: `plasma-desktop`, plus apps such as `konsole`). Experimental.

Plasma's RDP server (KRdp) shares a session that already exists; it does not start one per user on a headless server. Waydesk does, the same way as for COSMIC (docs/COSMIC.md): the session stays a headless sway with one output of the client's size, and its configuration (`/usr/share/waydesk/sway-plasma.conf`) runs `startplasma-wayland`. KWin finds sway's display and runs as a fullscreen window of it; capture and input stay sway's.

What Waydesk does for it:
- **Clipboard** (text): the bridge uses KWin's own display (it offers ext-data-control).
- **Keyboard layout:** the layout Waydesk applies is also set in Plasma's own keyboard settings (`kxkbrc`, with `kwriteconfig6 --notify` so that KWin applies it at once), because KWin reads sway's raw key codes with its own layout. Needs `kwriteconfig6` (part of Plasma).
- **Cursor:** nothing special: KWin gives sway the cursor image and the client draws it.

Measured on the i7-7700 test server, Plasma 6.8.2: about 750 MB and 46 processes for an idle session. Not measured under load. Several accounts at once: four on a 4-core Debian 13 server (Plasma 6.3.6), and three on the i7 beside nine other sessions (sway, COSMIC, Omarchy), all drawing, on the hardware encoder (2026-11-01).
