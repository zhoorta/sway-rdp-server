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

**The server works on its own.** `waydesk-server` needs only sway, foot, PAM, openssl, systemd and `libopenh264`; it does not need
`waydesk-desktop`. Installed alone you get the RDP login with the account's password, persistent per-account sessions, the memory
protection (admission check, per-account caps, zram) and `waydesk-admin`; the session itself is plain sway: the account's `~/.config/waydesk/config` if it has one, otherwise sway's usual lookup
(`~/.config/sway/config`, then `/etc/sway/config`). Install `waydesk-desktop` (or `waydesk`, which pulls in both)
only if you want the Windows-like desktop below. Only Debian 13 (arm64 and amd64) is tested; other distros, Debian-based ones
included, have not been tried, and the packages are `.deb` only.

`waydesk-desktop` (second package, same build) makes those sessions a Windows-like desktop: a system sway config
(`/etc/xdg/waydesk/sway.conf`: floating windows with title bars, Alt+Tab, Alt+F4, `Super+d` launcher, screenshots;
an account's `~/.config/waydesk/config` replaces it), the glue that starts [DankMaterialShell](https://duxlabs.dev)
through the account's systemd user session when it is installed (it is not in Debian), and `waydesk-setup`, a
whiptail wizard for DMS and developer, creative and AI apps. Design, threat model and what is not done yet:
[docs/MULTIUSER-DESIGN.md](docs/MULTIUSER-DESIGN.md). The sections below describe the single-account form, which still
exists (run the binary inside your own sway).

**Task Manager** (the launcher, or `Ctrl+Shift+Esc`; administrators only, like Users & Access) opens the shell's System Monitor: processes (search, sort by CPU or memory, end one), CPU, memory and swap, network speeds,
disks and system information, for the machine's processes, as `/proc` shows them. It is hidden and refused for accounts outside the group `sudo`, but that is a convenience, not a barrier:
`ps` shows the same to every account, and so do the CPU and memory widgets the shell offers for its bar (the kernel's `hidepid`, see "Look at it", is the real one);
`waydesk-admin status` and the Users & Access panel are the administrator's view of every session.

Workspaces (virtual desktops): the first is called Remote, others are made on demand and go away when their last window does (four at most). With the mouse: the
**+** next to the workspace pill of the bar makes one and goes to it, and the arrow beside it lists the workspaces with their windows (click a window to go to it,
**Move to** sends it to another workspace or to a new one). With the keyboard: `Super+1..4` go to a workspace, `Super+Shift+1..4` send the focused window there,
`Super+Ctrl+Left/Right` step through them and `Super+Ctrl+D` makes a new one. The button is a bar plugin (`swayWorkspaces`) that only calls `waydesk-windows` (sway's IPC).

The default browser is Chromium (`waydesk-desktop` recommends it; `/etc/xdg/sway-mimeapps.list` makes it the default for links, and it is on a new
desktop). Chromium and Firefox ESR are set to use the title bar of the window manager (`/etc/chromium/initial_preferences`, which also carries Debian's own
defaults for a new profile, and a Firefox default preference), so they get the window buttons below like every other window; an existing Chromium profile
is changed once at the next start of the shell.

Window buttons (red closes, yellow minimizes, green maximizes) in the title bar of every window need a patched sway, because sway's title
bars are text only: `packaging/sway/build.sh [arm64|amd64]` builds Debian's sway 1.10 with `packaging/sway/waydesk-titlebar-buttons.patch`
(a new command, `titlebar_button close|minimize|maximize COMMAND`, set by `waydesk-titlebar-buttons`; a double click on a title bar runs the
maximize command). Install the `.deb` over Debian's
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

With Chromium (the default browser) and three light pages open, a user costs about 700 MB in all: about 430 MB for the session and about 270 MB for
Chromium. Four such users, logged in one after the other, left 830 MB of 3.75 GB available; a fifth would not fit, so a 4 GB machine serves about
**4 users working with a browser, or about 8 with nothing but the desktop open** (a few more with `WAYDESK_DMS_SOFTWARE=1`, below). Heavy sites (mail,
video calls, office suites) cost much more per user; count on 3 users per 4 GB for real work.

Plan on about 0.4-0.5 GB per logged-in session for the desktop itself, plus what the programs use, plus about 1 GB
for the system. Sessions stay until they time out (`--idle-timeout`, 8 h by default; `--end-on-disconnect` ends them when the
client leaves), so the number that counts is the accounts logged in, not the ones connected. `WAYDESK_DMS_SOFTWARE=1` in `/etc/default/waydesk` takes about
170 MB off each session by drawing the shell without OpenGL (the bar, the dock and the launcher look right; rounded corners in the settings window
and some previews do not).

### Running out of memory

With these four layers the same 4 GB VM took **five users with Chromium and three pages each** (the zram swap held 240-420 MB of idle pages), refused the
sixth and the seventh at login, and was never short enough for the kernel to kill anything (it was killed twice, with five accounts, before). Without zram the
fifth user is the one that does not fit. The 4 GB test VM was killed by the kernel's out-of-memory killer with five accounts logged in (it kills the shell, `qs`, of an account first, which leaves
that account with windows and no bar). The package prevents it in four layers, none of which asks anything at install time:

1. **A new session is refused when the machine cannot take one more.** The root helper checks the available memory before it starts a session for an account
   that has none: by default it needs the larger of 900 MB and 15 % of the memory (swap is not counted: an account that is swapping is not working). The
   account's running session is never refused, so nobody is thrown out for a newcomer. The refusal is in the journal (`session refused: the machine is short of
   memory`); the client is sent a reason before the connection closes ("The server ran out of available memory resources" for the memory check, "The server denied
the connection" for `--max-active-users`; FreeRDP shows it, tested; other clients show their own text for the same RDP error codes). Change it with `--min-available-mb N` (0 = off) and `--max-active-users N` in `WAYDESK_HELPER_ARGS`
   (see `/etc/default/waydesk`).
2. **Every account has ceilings** (`user-.slice.d/10-waydesk.conf`): at most 35 % of the machine's memory before the kernel reclaims from that account first, 50 % at
   the most (then only that account's biggest process is killed), no more than half of the memory in swap, 8192 tasks. They are ceilings, not shares: they do
   not hold anybody back while the machine has room, and CPU is not capped at all (every account has the same `CPUWeight`, so a busy one gets every idle core and
   gives way only when the others want theirs). An account that needs more, a developer with VS Code and builds for example, gets it with
   `sudo waydesk-admin users limit NAME --memory 12G [--cpu-weight 200] [--swap 4G]` (`reset` goes back to the defaults), or in the Users & Access panel, **Limits…**.
3. **Compressed swap in RAM.** `systemd-zram-generator` is recommended; Debian's file makes a zram swap of half the RAM (at most 4 GiB), and the package
   picks zstd for it and sets `vm.swappiness=100` and `vm.page-cluster=0` (`/usr/lib/sysctl.d/50-waydesk.conf`). A short spike is absorbed by compressing idle pages instead of
   killing a browser. A disk swap behind it works too (zram has the higher priority); disk swap alone is slow for a desktop, and on network storage worse.
4. **Look at it.** `waydesk-admin status` shows the memory, the swap, the load, whether a new session would be accepted, and for every
   account its memory, swap, CPU and the programs that hold the most memory (resident memory, so shared pages count for each process); `--json` for scripts. The same is in
   the **Users & Access** panel (administrators only): a bar for the machine, and memory, CPU and programs on each account's card.
   Administrators (the group `sudo`) see every account; any other account that runs `status`, `users list` or `sessions list` sees its own row and the totals of the
   machine. That only keeps the tool from collecting it: the same numbers are in `/proc` and the cgroup files, which every account can read on Debian. On a
   host whose users do not trust each other, consider mounting `/proc` with `hidepid=invisible` (and `gid=` of a group for the administrators), and test the
   desktop sessions (polkit, Flatpak, logind) thoroughly before relying on it: it hides processes from services that expect to see them.

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
the account changes it itself on the desktop: **Change password** in the power menu (next to log out; `waydesk-passwd-panel`). The tool never touches root or accounts below uid 1000, and refuses to
remove or demote the account that runs it or the last administrator. Passwords go to `chpasswd` over a pipe, never as arguments.

On the desktop, **Users & Access** (start menu, or `waydesk-users`; only for administrators, the group `sudo`: others do not see it and it
does not open for them) does the same with switches and buttons: who can log in, who
installs Flatpaks without a password, who is an administrator, the running sessions, new passwords (shown once) and removal. Changes
go through `pkexec waydesk-admin` (polkit action `org.waydesk.admin`), so an administrator's password is asked for each time.

Every account, administrator or not, changes its own password with **Change password** in the shell's power menu: the current password
and the new one twice. `waydesk-passwd` (current and new password on stdin, run as the account itself, no privileges) checks the current
one through PAM and then runs the system's `passwd`, so the system's password rules apply. Minimum 8 characters.

