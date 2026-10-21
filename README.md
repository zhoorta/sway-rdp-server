<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="branding/waydesk-logo-dark.svg">
    <img src="branding/waydesk-logo.svg" alt="Waydesk" height="64">
  </picture>
</p>

# Waydesk (waydesk-server)

An RDP server for [sway](https://swaywm.org). Every client gets a headless sway output sized and scaled from what the
client asks for, so a Retina Mac or a laptop with 200 % scaling gets a sharp desktop at its own resolution and the
window can be resized live. Status: **early, one author, tested mostly with FreeRDP in a VM; the Windows App on macOS
was tested before the multi-user work and not since.**

## Names

**Waydesk** is the name of the product (a Windows-like remote Linux desktop over RDP); `apt install waydesk` installs everything.
The packages are `waydesk-server` (the RDP server, the root helper and the `waydesk-admin` tool) and `waydesk-desktop` (the desktop for
the sessions); `waydesk` is an empty package that depends on both. Waydesk builds on the sway compositor and is not part of the sway
project. Waydesk is a working name: no domain, trademark or package-name clearance has been done.

## Multi-user (Debian 13)

`apt install ./waydesk-server_*.deb` (build it with `packaging/build-deb.sh [arm64|amd64]`, Docker needed) installs two
systemd services:

- `waydesk-helper`: runs as root and does only two things: check a password through PAM and start a session as that
  account. It never touches the network.
- `waydesk`: the network-facing RDP daemon, as the unprivileged system account `waydesk` (heavily sandboxed), on
  `127.0.0.1:3389` by default (`/etc/default/waydesk`).

An account logs in over RDP with its normal password (plain TLS login: use a VPN or an SSH tunnel) and gets its own
headless sway that **survives disconnects**: reconnecting returns to the same windows, and a second client of the same
account replaces the first, only after logging in. Sessions end after 8 hours without a client (`--idle-timeout`) or
when the account quits its sway. Accounts cannot reach each other's sockets. The self-signed certificate is made at
install time and its fingerprint is printed.

`waydesk-desktop` (second package, same build) makes those sessions a Windows-like desktop: a system sway config
(`/etc/xdg/waydesk/sway.conf`: floating windows with title bars, Alt+Tab, Alt+F4, `Super+d` launcher, screenshots;
an account's `~/.config/waydesk/config` replaces it), the glue that starts [DankMaterialShell](https://duxlabs.dev)
through the account's systemd user session when it is installed (it is not in Debian), and `waydesk-setup`, a
whiptail wizard for DMS and developer, creative and AI apps. Design, threat model and what is not done yet:
[docs/MULTIUSER-DESIGN.md](docs/MULTIUSER-DESIGN.md). The sections below describe the single-account form, which still
exists (run the binary inside your own sway).

Window buttons (red closes, yellow minimizes, green maximizes) in the title bar of every window need a patched sway, because sway's title
bars are text only: `packaging/sway/build.sh [arm64|amd64]` builds Debian's sway 1.10 with `packaging/sway/waydesk-titlebar-buttons.patch`
(a new command, `titlebar_button close|minimize|maximize COMMAND`, set by `waydesk-titlebar-buttons`). Install the `.deb` over Debian's
package and `apt-mark hold sway`; the patch has to be checked against each new Debian sway. With Debian's own sway nothing breaks: the
shell's bar shows the same three buttons for the focused window instead.

- Capture: `wlr-screencopy` (frame diff). The pointer is drawn by the RDP client, and sway gets an invisible cursor theme (`WAYDESK_CURSOR=server` keeps the cursor in the picture instead, with the shapes of the apps, for clients that hide their own pointer).
- Video: H.264 (AVC420) over the RDP graphics pipeline; plain bitmaps as fallback. The encoder is the system's
  `libopenh264` (Debian: package `libopenh264-8`) loaded at run time; waydesk-server contains no encoder code.
- Input: virtual keyboard and pointer (layout taken from the client's keyboard id), Command works as Ctrl on Macs.
- Audio: RDPSND from a PipeWire/PulseAudio null sink (`waydesk-server`), PCM 44.1 kHz.
- Clipboard: text only (up to 1 MiB), both ways, **on by default** (`--no-clipboard` turns it off; it needs `wl-clipboard`). The session process does the `wl-copy`/`wl-paste` as the account; the daemon only relays text. Tried with FreeRDP and the macOS Windows App (2026-10-19). The server waits 5 s after connecting before it asks the client for its clipboard.
- Security: TLS always on. There is **no built-in credential**. Two ways to log in (one user, the one running the server):
  - `--auth file` (default): NLA with the `user:password` on the first line of `--credentials FILE` (password of at least 8
    characters). NLA protects the password even if you accept an unverified certificate.
  - `--auth pam`: the account's own password, checked through PAM (service `waydesk-server`, see `contrib/pam/`), so no
    password is stored anywhere. This needs plain TLS login without NLA, because NLA has to know the password itself.
    **The password then travels inside the TLS connection**: someone who intercepts it and shows another certificate could
    read it, so verify the certificate fingerprint on first use, and prefer loopback, a tunnel or a VPN on networks you do
    not control. Failures are delayed 2 s, and after 5 failed attempts in a row every login is refused for 60 s.
  - A self-signed certificate is fine for a start (`--cert`, `--key`). `--tls-only` forces plain TLS login with `--auth file` (testing).
- Built on [IronRDP](https://github.com/Devolutions/IronRDP) (`ironrdp-server` 0.13 with three small local patches in
  `patches/`, see `patches/ironrdp-server.diff`) and `ironrdp-egfx`.

## Run

```
waydesk-server --bind 127.0.0.1:3389 --cert cert.pem --key key.pem --credentials rdp.cred
# or, with your login password: waydesk-server --bind 127.0.0.1:3389 --cert cert.pem --key key.pem --auth pam
```

Run it inside the sway session it should serve (it needs `SWAYSOCK` and `WAYLAND_DISPLAY`). Needs libpam (`libpam0g`) for `--auth pam`, and `swaymsg`, `foot` (only for
the first terminal, `--no-terminal` skips it), `libopenh264.so.8`, and `parec`/`pactl` for audio (`--no-audio` skips).

Useful options: `--bind ADDR`, `--kb-layout xkb` (fallback layout), `--workspace NAME` (default `Remote`),
`--session-hook PROGRAM`, `--bitrate`, `--quality`, `--no-egfx`, `--no-audio`, `--no-clipboard`, `--replace`/`--no-replace`.

**Exposure.** Binding to anything but loopback puts an RDP login on the network. Use a long random password, and prefer
loopback plus an SSH tunnel or a VPN. A new connection replaces the open one; that is on by default only for loopback,
because the old client is dropped before the new one has authenticated (so anyone who can reach the port could kick the user).

## Session hook

`--session-hook PROGRAM` starts a program when a session begins and sends SIGTERM to its process group when the session ends
(SIGKILL after two seconds). Environment: `WAYDESK_OUTPUT` (the headless output), `WAYDESK_WORKSPACE`, `WAYDESK_INDEX`
(0 to 99, to derive a port or name). Use it to start a bar, a dock or a per-session shell on the new output. Note that new layer-shell surfaces open on the focused output, so the hook should `swaymsg focus output
"$WAYDESK_OUTPUT"` before starting each one.

## Known gaps

No dynamic hardware encoding (VA-API), no Opus, relative mouse and Unicode key events,
Caps Lock sync. In the multi-user form sound is off, and sessions share the helper's control group (no
per-account resource limits yet). See docs/MULTIUSER-DESIGN.md.

## Licence

Dual-licensed under either of [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your option. `patches/ironrdp-server` is a
modified copy of IronRDP's `ironrdp-server` crate and stays under its own licence (MIT or Apache-2.0, files included there).

## Sizing

Measured in the 4 GB arm64 development VM (software rendering, 1280x800), desktop with the bar and the dock, nothing else open (2026-10-21):

| | Memory |
|---|---|
| First session, idle (PSS: shared libraries divided between the processes that map them) | about 610 MB: the shell (`qs`) 394, the session process 45, `dms` 39, the file manager 29, sway 23, portals 18, helpers 15, sound 11 |
| Each further idle session (drop in available RAM) | about 410 MB |
| An open terminal, or a file manager window | +20 MB, +5 MB |
| The daemon (`waydesk`, all connections) | about 5 MB |

Plan on about 0.5 GB per logged-in session for the desktop itself, plus what the programs use (a browser is several hundred MB more), plus about 1 GB
for the system. The 4 GB VM ran out of memory with five accounts logged in; when it does, the kernel kills the shell (`qs`) of an account first, and that
account is left with windows and no bar. Sessions stay until they time out (`--idle-timeout`, 8 h by default; `--end-on-disconnect` ends them when the
client leaves), so the number that counts is the accounts logged in, not the ones connected. `WAYDESK_DMS_SOFTWARE=1` in `/etc/default/waydesk` takes about
170 MB off each session by drawing the shell without OpenGL (the bar, the dock and the launcher look right; rounded corners in the settings window
and some previews do not). To keep one account from taking the rest, give the accounts a share: for example a drop-in `/etc/systemd/system/user-.slice.d/60-share.conf`
with `MemoryHigh=` and `MemoryMax=` (the packaged one only caps an account at 90 % of the machine).

## Who may log in

Members of the group `waydesk-users` (the package makes it, and on a first install adds the account that ran the installation):
`adduser ACCOUNT waydesk-users`. The account also needs a login shell listed in `/etc/shells`. To let every ordinary account in, put
`WAYDESK_HELPER_ARGS=` (nothing after the equals sign) in `/etc/default/waydesk` and restart `waydesk-helper`; for another group,
`WAYDESK_HELPER_ARGS=--require-group NAME`. Root is never let in.

## File manager menus

The desktop and file manager are Debian's pcmanfm-qt. `packaging/pcmanfm-qt/build.sh [arm64|amd64]` rebuilds it with a small patch that
removes Desktop Preferences, Hide Desktop Items, Create Launcher and Open in Terminal from the menus (DMS does wallpaper and settings).
Install the resulting `pcmanfm-qt_*+waydesk1_*.deb` over Debian's and `apt-mark hold pcmanfm-qt`, so Debian's updates do not replace it;
check the patch again when upstream changes. Without it everything works, with the stock menus.

## Managing accounts: `waydesk-admin`

```
waydesk-admin users list [--json]                      who may log in, who may install Flatpaks, who has a session
sudo waydesk-admin users add NAME [--admin] [--sudo] [--no-rdp]    makes the account and shows a temporary password once
sudo waydesk-admin users passwd NAME [--stdin]         a new random password (shown once), or one read from stdin
sudo waydesk-admin users enable NAME | disable NAME    log in over RDP or not (disable ends the session too; --keep-session)
sudo waydesk-admin users admin NAME on|off             Flatpak installs without a password (group waydesk-admins)
sudo waydesk-admin users sudo NAME on|off
sudo waydesk-admin users remove NAME [--delete-home]
waydesk-admin sessions list [--json]       |   sudo waydesk-admin sessions end NAME
```

Listing is open to everybody; everything else needs root. A new account's password does not expire: RDP cannot ask for a new one, so
the account changes it with `passwd` in a terminal on the desktop. The tool never touches root or accounts below uid 1000, and refuses to
remove or demote the account that runs it or the last administrator. Passwords go to `chpasswd` over a pipe, never as arguments.

On the desktop, **Users & Access** (start menu, or `waydesk-users`; only for administrators, the group `sudo`: others do not see it and it
does not open for them) does the same with switches and buttons: who can log in, who
installs Flatpaks without a password, who is an administrator, the running sessions, new passwords (shown once) and removal. Changes
go through `pkexec waydesk-admin` (polkit action `org.waydesk.admin`), so an administrator's password is asked for each time.

