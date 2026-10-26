# Reference

Packages, commands, units, ports and files of Waydesk. The short version is on [waydesk.dev](https://waydesk.dev/#reference).


## Packages

- `waydesk`: metapackage: server and desktop
- `waydesk-server`: daemon, root helper, waydesk-admin, waydesk-setup-admin, PAM services, units, zram and sysctl drop-ins, minimal sway config
- `waydesk-desktop`: sway config, setup wizard (waydesk-setup), per-account shell setup, administrator panels
- `waydesk-shell, waydesk-quickshell`: DankMaterialShell 2.0.0 built by us with our patches; quickshell 0.3.0, built per distribution
- `waydesk-sway`: sway 1.11 with wlroots 0.19.3 linked statically and the title-bar button patch, under /usr/lib/waydesk/bin; built per distribution (+deb13, +ubuntu26.04)
- `pcmanfm-qt`: file manager with our patches, built per distribution
- `waydesk-repo`: the key, the apt source template and the pin

## Commands: administration

- `waydesk-admin status [--json]`: memory, swap, load, running sessions and what each account uses
- `waydesk-admin users list [--json]`: accounts that may log in
- `waydesk-admin users add NAME [--admin] [--sudo] [--no-rdp]`: create an account; prints a generated password
- `waydesk-admin users passwd NAME [--stdin]`: set a password (read from stdin, never from an argument)
- `waydesk-admin users remove NAME [--delete-home]`: remove an account
- `waydesk-admin users enable NAME | disable NAME [--keep-session]`: allow or stop RDP logins for an account
- `waydesk-admin users admin NAME on|off`: Flatpak installs without a password
- `waydesk-admin users sudo NAME on|off`: membership of the sudo group
- `waydesk-admin users limit NAME [show | reset | --memory SIZE] [--swap SIZE] [--cpu-weight N] [--tasks N]`: per-account ceilings
- `waydesk-admin sessions list [--json] | end NAME`: running sessions; ending one closes that account's desktop

## Commands: setup

- `waydesk-setup [--list | --check]`: choose and install shell, browsers, office, developer and creative apps
- `waydesk-setup-admin [--name NAME [--password-stdin]] [--existing NAME]`: the first administrator: RDP access, panels, Flatpak installs without a password

## Units and ports

- `waydesk.service`: the daemon: TLS, RDP, framing (user waydesk)
- `waydesk-helper.service`: the root helper: password check, session start, memory admission
- `waydesk-session@UID.service`: one per account: its sway, capture, encoder and shell
- `127.0.0.1:3389`: RDP over TLS; reach it through a tunnel or a VPN

## Files

- `/etc/default/waydesk`: daemon and session options
- `/etc/pam.d/waydesk-server`: PAM service for the login check
- `/etc/pam.d/waydesk-session`: PAM service that gives the session a logind session
- `/etc/xdg/waydesk/sway.conf`: the session's sway config (fallback: /usr/share/waydesk/sway-minimal.conf)
- `/usr/lib/waydesk/bin`: private sway, swaymsg and swaybar; first in the session's PATH
- `/run/waydesk/`: helper and session sockets
- `/etc/apt/sources.list.d/waydesk.sources`: the repository, written for your release
- `/etc/apt/preferences.d/waydesk`: the pin: these packages win over third-party builds of the same names
- `/usr/lib/systemd/zram-generator.conf.d/50-waydesk.conf`: zram as large as the RAM, at most 16 GiB

## Who can reach the port

- `waydesk-admin access [show | local | lan | vpn | custom ...]`: where the port listens and which addresses may connect (see `ACCESS.md`)
- `waydesk-admin cert [status | local-ca | reissue | export-ca | install | self-signed]`: the certificate clients check
- `waydesk-admin vpn [status | init | add NAME | show NAME | list | remove NAME]`: a WireGuard VPN for the port
- `waydesk-setup-admin ... --access local|lan|vpn`: choose how clients reach the port while making the first administrator
- `/etc/default/waydesk`: `WAYDESK_BIND`, `WAYDESK_ALLOW_FROM`, `WAYDESK_TCP_CONGESTION` and the other options
