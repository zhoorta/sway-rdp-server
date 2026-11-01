# Reference

Packages, commands, units, ports and files of Waydesk. The short version is on [waydesk.dev](https://waydesk.dev/#reference).


## Packages

- `waydesk`: metapackage: server and desktop
- `waydesk-server`: daemon, root helper, waydesk-admin, waydesk-setup-admin, PAM services, units, zram and sysctl drop-ins, minimal sway config
- `waydesk-desktop`: sway config, the shell it depends on, setup wizard (waydesk-setup), per-account shell setup, the Keyboard window, administrator panels
- `waydesk-shell, waydesk-quickshell`: DankMaterialShell 2.0.0 built by us with our patches; quickshell 0.3.0, built per distribution
- `waydesk-sway`: sway 1.11 with wlroots 0.19.3 linked statically and the title-bar button patch, under /usr/lib/waydesk/bin; built per distribution (+deb13, +ubuntu26.04)
- `pcmanfm-qt`: file manager with our patches, built per distribution
- `waydesk-repo`: the key, the apt source template and the pin

## Commands: administration

- `waydesk-admin status [--json] [--watch [SECONDS]]`: memory, swap, load, the GPUs and how busy they are, running sessions and what each account uses and runs (GPU or CPU rendering, the video encoder: `vaapi`, `nvenc` or `openh264`, with its quantiser and frame rate); `--watch` repeats it (2 s by default), `--json --watch` prints one line of JSON each time With `WAYDESK_AI_ENDPOINT` set (a local inference service, `127.0.0.1:11434`), an `AI` line shows what the service holds (for Ollama: the model and how much of it is in video memory, from `/api/ps`) and how many connections are open to it, and the table gets an `AI` column: the connections each account has open to the service (the owner of a client socket in the kernel's TCP table; a streaming request keeps its connection open until the answer ends, so it counts requests running and connections left idle, not tokens). Connections of root or other non-account users are counted in the line, not the column. JSON: `ai` (endpoint, state, models, connections, connections_other) and per user `ai_connections`.
- `waydesk-admin users list [--json]`: accounts that may log in
- `waydesk-admin users add NAME [--admin] [--sudo] [--no-rdp]`: create an account; prints a generated password
- `waydesk-admin users passwd NAME [--stdin]`: set a password (read from stdin, never from an argument)
- `waydesk-admin users remove NAME [--delete-home]`: remove an account
- `waydesk-admin users enable NAME | disable NAME [--keep-session]`: allow or stop RDP logins for an account
- `waydesk-admin users admin NAME on|off`: Flatpak installs without a password
- `waydesk-admin users sudo NAME on|off`: membership of the sudo group
- `waydesk-admin users limit NAME [show | reset | --memory SIZE] [--swap SIZE] [--cpu-weight N] [--tasks N]`: per-account ceilings
- `waydesk-admin users set NAME [--desktop sway|hyprland|default] [--gpu auto|off|renderDN|default]`: the compositor and the GPU of one account's sessions (`default`: the server's `WAYDESK_COMPOSITOR` / `WAYDESK_GPU`; a render node must exist on this machine, e.g. `renderD129`; the encoder follows the GPU); written to `/etc/default/waydesk.d/UID.env`, taken up when the session next starts ([BRING-YOUR-OWN-CONFIG.md](BRING-YOUR-OWN-CONFIG.md))
- `waydesk-admin gpu roles [show | reset | NODE=ROLE ...]`: what each GPU is for (`render`, `inference` or `both`); shows the cards and their roles, or writes `WAYDESK_GPU_ROLES` in `/etc/default/waydesk` (root; only the named cards change, `reset` removes the setting; it applies to sessions that start afterwards)
- `waydesk-admin ai [show | endpoint HOST:PORT | endpoint off]`: the local inference service `status` reports on (Ollama, vLLM, llama.cpp); writes `WAYDESK_AI_ENDPOINT` (root) and says what answers there. Waydesk neither installs nor starts the service
- `waydesk-admin config check NAME [--json]`: whether the account's own sway or Hyprland setup will work as a session (exit status 1 when something is a problem)
- `waydesk-admin sessions list [--json] | end NAME`: running sessions; ending one closes that account's desktop

## Commands: setup

- `waydesk-setup [--list | --check]`: choose and install browsers, office, developer and creative apps (the shell comes with `waydesk-desktop`)
- `waydesk-keyboard-panel`: the **Keyboard** window in the launcher: the layout of the account's keys, written to `~/.config/waydesk/keyboard` (an xkb layout name; `waydeskmac` is in `/etc/xkb/symbols`); the session follows the file within a second
- `waydesk-server --version`, `waydesk-admin --version`: the version of the package
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
