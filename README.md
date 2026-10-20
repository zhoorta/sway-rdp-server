# sway-rdp-server

An RDP server for [sway](https://swaywm.org). Every client gets a headless sway output sized and scaled from what the
client asks for, so a Retina Mac or a laptop with 200 % scaling gets a sharp desktop at its own resolution and the
window can be resized live. Status: **early, one author, tested mostly with FreeRDP in a VM; the Windows App on macOS
was tested before the multi-user work and not since.**

## Multi-user (Debian 13)

`apt install ./sway-rdp_*.deb` (build it with `packaging/build-deb.sh [arm64|amd64]`, Docker needed) installs two
systemd services:

- `sway-rdp-helper`: runs as root and does only two things: check a password through PAM and start a session as that
  account. It never touches the network.
- `sway-rdp`: the network-facing RDP daemon, as the unprivileged system account `sway-rdp` (heavily sandboxed), on
  `127.0.0.1:3389` by default (`/etc/default/sway-rdp`).

An account logs in over RDP with its normal password (plain TLS login: use a VPN or an SSH tunnel) and gets its own
headless sway that **survives disconnects**: reconnecting returns to the same windows, and a second client of the same
account replaces the first, only after logging in. Sessions end after 8 hours without a client (`--idle-timeout`) or
when the account quits its sway. Accounts cannot reach each other's sockets. The self-signed certificate is made at
install time and its fingerprint is printed.

`sway-rdp-desktop` (second package, same build) makes those sessions a Windows-like desktop: a system sway config
(`/etc/xdg/sway-rdp/sway.conf`: floating windows with title bars, Alt+Tab, Alt+F4, `Super+d` launcher, screenshots;
an account's `~/.config/sway-rdp/config` replaces it), the glue that starts [DankMaterialShell](https://danklinux.com)
through the account's systemd user session when it is installed (it is not in Debian), and `sway-rdp-setup`, a
whiptail wizard for DMS and developer, creative and AI apps. Design, threat model and what is not done yet:
[docs/MULTIUSER-DESIGN.md](docs/MULTIUSER-DESIGN.md). The sections below describe the single-account form, which still
exists (run the binary inside your own sway).

- Capture: `wlr-screencopy` (frame diff). The pointer is drawn by the RDP client, and sway gets an invisible cursor theme (`SWAY_RDP_CURSOR=server` keeps the cursor in the picture instead, with the shapes of the apps, for clients that hide their own pointer).
- Video: H.264 (AVC420) over the RDP graphics pipeline; plain bitmaps as fallback. The encoder is the system's
  `libopenh264` (Debian: package `libopenh264-8`) loaded at run time; sway-rdp-server contains no encoder code.
- Input: virtual keyboard and pointer (layout taken from the client's keyboard id), Command works as Ctrl on Macs.
- Audio: RDPSND from a PipeWire/PulseAudio null sink (`sway-rdp-server`), PCM 44.1 kHz.
- Clipboard: text only (up to 1 MiB), both ways, **on by default** (`--no-clipboard` turns it off; it needs `wl-clipboard`). The session process does the `wl-copy`/`wl-paste` as the account; the daemon only relays text. Tried with FreeRDP and the macOS Windows App (2026-10-19). The server waits 5 s after connecting before it asks the client for its clipboard.
- Security: TLS always on. There is **no built-in credential**. Two ways to log in (one user, the one running the server):
  - `--auth file` (default): NLA with the `user:password` on the first line of `--credentials FILE` (password of at least 8
    characters). NLA protects the password even if you accept an unverified certificate.
  - `--auth pam`: the account's own password, checked through PAM (service `sway-rdp-server`, see `contrib/pam/`), so no
    password is stored anywhere. This needs plain TLS login without NLA, because NLA has to know the password itself.
    **The password then travels inside the TLS connection**: someone who intercepts it and shows another certificate could
    read it, so verify the certificate fingerprint on first use, and prefer loopback, a tunnel or a VPN on networks you do
    not control. Failures are delayed 2 s, and after 5 failed attempts in a row every login is refused for 60 s.
  - A self-signed certificate is fine for a start (`--cert`, `--key`). `--tls-only` forces plain TLS login with `--auth file` (testing).
- Built on [IronRDP](https://github.com/Devolutions/IronRDP) (`ironrdp-server` 0.13 with three small local patches in
  `patches/`, see `patches/ironrdp-server.diff`) and `ironrdp-egfx`.

## Run

```
sway-rdp-server --bind 127.0.0.1:3389 --cert cert.pem --key key.pem --credentials rdp.cred
# or, with your login password: sway-rdp-server --bind 127.0.0.1:3389 --cert cert.pem --key key.pem --auth pam
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
(SIGKILL after two seconds). Environment: `SWAY_RDP_OUTPUT` (the headless output), `SWAY_RDP_WORKSPACE`, `SWAY_RDP_INDEX`
(0 to 99, to derive a port or name). Use it to start a bar, a dock or a per-session shell on the new output. Note that new layer-shell surfaces open on the focused output, so the hook should `swaymsg focus output
"$SWAY_RDP_OUTPUT"` before starting each one.

## Known gaps

No dynamic hardware encoding (VA-API), no Opus, relative mouse and Unicode key events,
Caps Lock sync. In the multi-user form sound is off, and sessions share the helper's control group (no
per-account resource limits yet). See docs/MULTIUSER-DESIGN.md.

## Licence

Dual-licensed under either of [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your option. `patches/ironrdp-server` is a
modified copy of IronRDP's `ironrdp-server` crate and stays under its own licence (MIT or Apache-2.0, files included there).

## Who may log in

Members of the group `sway-rdp-users` (the package makes it, and on a first install adds the account that ran the installation):
`adduser ACCOUNT sway-rdp-users`. The account also needs a login shell listed in `/etc/shells`. To let every ordinary account in, put
`SWAY_RDP_HELPER_ARGS=` (nothing after the equals sign) in `/etc/default/sway-rdp` and restart `sway-rdp-helper`; for another group,
`SWAY_RDP_HELPER_ARGS=--require-group NAME`. Root is never let in.

## Managing accounts: `sway-rdp-admin`

```
sway-rdp-admin users list [--json]                      who may log in, who may install Flatpaks, who has a session
sudo sway-rdp-admin users add NAME [--admin] [--sudo] [--no-rdp]    makes the account and shows a temporary password once
sudo sway-rdp-admin users passwd NAME [--stdin]         a new random password (shown once), or one read from stdin
sudo sway-rdp-admin users enable NAME | disable NAME    log in over RDP or not (disable ends the session too; --keep-session)
sudo sway-rdp-admin users admin NAME on|off             Flatpak installs without a password (group sway-rdp-admins)
sudo sway-rdp-admin users sudo NAME on|off
sudo sway-rdp-admin users remove NAME [--delete-home]
sway-rdp-admin sessions list [--json]       |   sudo sway-rdp-admin sessions end NAME
```

Listing is open to everybody; everything else needs root. A new account's password does not expire: RDP cannot ask for a new one, so
the account changes it with `passwd` in a terminal on the desktop. The tool never touches root or accounts below uid 1000, and refuses to
remove or demote the account that runs it or the last administrator. Passwords go to `chpasswd` over a pipe, never as arguments.

