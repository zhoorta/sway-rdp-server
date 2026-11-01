title: The command line
nav: Command line
seo_title: waydesk-admin command line: users, sessions, status, access · Waydesk
description: Every waydesk-admin command with its real output: accounts, sessions, limits, status and --watch, access, certificates and the VPN.
---
`waydesk-admin` is how an administrator looks after a server. Listing and `status` are open to every local account (an ordinary account sees its own row and the totals; administrators see every account); everything else needs root (`sudo`). Each command below was run on a real server, and the output is shown as it came, with only passwords and keys removed.

## At a glance {bar: table}
| command | what it does |
|---|---|
| `users list \| add \| remove \| passwd \| enable \| disable \| admin \| sudo \| limit \| set` | accounts, passwords, rights, per-account limits, and the desktop and GPU of an account |
| `config check NAME` | whether an account's own sway or Hyprland setup will work as a session |
| `sessions list \| end NAME` | who has a running session; end one |
| `status [--json] [--watch [SECONDS]]` | memory, swap, load, the GPUs, and what each account uses and runs |
| `gpu roles [show \| reset \| NODE=ROLE ...]` | what each GPU is for: sessions (`render`), a local model (`inference`) or `both` |
| `ai [show \| endpoint HOST:PORT \| endpoint off]` | the local model service that `status` reports on |
| `access [show \| local \| lan \| vpn \| custom]` | where the RDP port listens and who may connect |
| `cert [status \| local-ca \| reissue \| export-ca \| install \| self-signed]` | the certificate clients check |
| `vpn [status \| init \| add \| show \| list \| remove]` | a WireGuard VPN for the port (see [the VPN page](/docs/vpn/)) |
| `waydesk-setup-admin` | the first administrator, and how clients connect |
| `waydesk-setup` | as your own account: choose browsers, office, developer and creative applications |

## Accounts {bar: terminal}
```console
$ sudo waydesk-admin users add alice
made alice (uid 1021), groups: waydesk-users
temporary password (shown once): ************
The account should change it at once: Change password, in the desktop's power menu.
$ sudo waydesk-admin users add bob
made bob (uid 1022), groups: waydesk-users
temporary password (shown once): ************
The account should change it at once: Change password, in the desktop's power menu.
```

On a server with four accounts, each on a different desktop (the next section sets that):

```console
$ waydesk-admin users list
ACCOUNT                UID  RDP  FLATPAK  SUDO  SHELL   DESKTOP    GPU          SESSION
alice                 1021  yes  -        -     yes     sway       auto         running on sway
bob                   1022  yes  -        -     yes     plasma*    auto         running on plasma
carol                 1023  yes  -        -     yes     cosmic*    auto         running on cosmic
dave                  1024  yes  -        -     yes     hyprland*  auto         running on hyprland
```

`RDP` is the right to log in over RDP (the group `waydesk-users`), `FLATPAK` installing applications without a password (`waydesk-admins`), `SUDO` the administrator panels of the desktop. `DESKTOP` is the desktop the account's sessions start with (a `*` means it is set for this account; without it the server's setting applies), `GPU` the card they draw on, and `SESSION` what is running now. `--sudo` and `--admin` give them when the account is made; `--no-rdp` makes an account that cannot log in.

```console
$ sudo waydesk-admin users disable bob
bob may no longer log in, and the session was ended
$ waydesk-admin users list          # two of the four rows
ACCOUNT                UID  RDP  FLATPAK  SUDO  SHELL   DESKTOP    GPU          SESSION
alice                 1021  yes  -        -     yes     sway       auto         running on sway
bob                   1022  -    -        -     yes     plasma*    auto         none
```

`users enable NAME` gives the right back, `users disable NAME --keep-session` leaves a running session alone, `users passwd NAME` sets a new temporary password (`--stdin` reads it from a pipe, never from an argument), `users admin NAME on|off` and `users sudo NAME on|off` change the rights, and `users remove NAME [--delete-home]` removes the account. Accounts below uid 1000 and root are never touched.

## Desktop and GPU of an account {bar: terminal}
```console
$ sudo waydesk-admin users set bob --desktop plasma
bob: sessions start with plasma (set for this account)
bob: new sessions use the first usable GPU (the server's setting)
check the account's own configuration with: waydesk-admin config check bob
$ sudo waydesk-admin users set alice --gpu renderD128
alice: sessions start with sway (the server's setting)
alice: new sessions use /dev/dri/renderD128 (set for this account)
alice has a session running; its GPU changes when that session is restarted (waydesk-admin sessions end alice)
check the account's own configuration with: waydesk-admin config check alice
$ sudo waydesk-admin config check dave
waydesk config check for dave (hyprland)

INFO  sessions start with hyprland (set for this account)
OK    dave may log in over RDP (group waydesk-users)
OK    the account has a runtime directory (its user manager is running)
OK    Hyprland 0.57.0
OK    the account has a Lua configuration (~/.config/hypr/hyprland.lua)
OK    Hyprland accepts the configuration (--verify-config)
OK    the account can open /dev/dri/renderD128 and /dev/dri/card1 (Intel Kaby Lake-S GT2 [HD Graphics 630])

0 problems, 0 warnings
```

`--desktop` takes `sway` (the default), `plasma`, `cosmic` or `hyprland`, or `default` to follow the server; `--gpu` takes `auto`, `off`, a render node such as `renderD128`, or `default`. Both are read when a session starts, so end the account's session first (`sessions end`). `config check` only reads: it looks at the account's own configuration and exits with status 1 when something is a problem. More on each: [Other desktops](/docs/desktops/) and [Using a GPU](/docs/gpu/).

## Sessions {bar: terminal}
```console
$ waydesk-admin sessions list
no sessions running
$ sudo waydesk-admin sessions end alice
ended the session of alice
```

A session outlives the RDP connection: the windows are where you left them when you reconnect. Ending it closes everything the account had open.

## Limits {bar: terminal}
Every account lives in its own cgroup slice, with ceilings of 35 % and 50 % of the memory, 50 % of it in swap, and 8192 tasks. To change them for one account:

```console
$ sudo waydesk-admin users limit alice show
limits of alice (user-1000.slice):
CPUWeight=[not set]
MemoryHigh=2845147136
MemoryMax=4064497664
MemorySwapMax=4064497664
TasksMax=8192
$ sudo waydesk-admin users limit alice --memory 4G --swap 1G
limits of alice set: MemoryHigh=3865470561 MemoryMax=4294967296 MemorySwapMax=1073741824
```

`--cpu-weight N` and `--tasks N` work the same way, and `users limit NAME reset` puts the defaults back. A new session is refused when less than the larger of 900 MB and 15 % of the memory is available (or the optional limit of active users is reached); an account that already has a session just reconnects.

## Status {bar: terminal}
`status` shows the machine and every session. On an Intel machine with one account on each desktop (the machine runs Omarchy, which names itself that way in `/etc/os-release`):

```console
$ waydesk-admin status
Memory    62.6 GB in all, 54.6 GB available
Swap      none
Load      1.00 1.41 1.45 (8 cores)
System    Omarchy, kernel 7.2.11-arch1-1
GPU       Intel Kaby Lake-S GT2 [HD Graphics 630] (i915, renderD128): 2 % busy
            sessions and inference; 4 sessions: alice, bob, carol, dave
          new sessions use: auto
AI        127.0.0.1:11434: answering, no model loaded; 0 connections open
Desktop   sway for new sessions (WAYDESK_COMPOSITOR: sway, or hyprland, which needs a GPU)
Sessions  4 running; a new one would be accepted (needs 9.4 GB available)

ACCOUNT            DESKTOP   SESSION     MEMORY    SWAP    CPU    LIMIT RENDER     ENCODER                      AI   PROGRAMS (resident)
alice              sway      running     230 MB       -     4%  31.3 GB renderD128 vaapi@renderD128 q18 2fps    -    sway 273 MB, waydesk-server 172 MB, foot 32 MB
bob                plasma    running     660 MB       -     3%  31.3 GB renderD128 vaapi@renderD128             -    Xwayland 506 MB, plasmashell 473 MB, kwin_wayland 409 MB
carol              cosmic    running     920 MB       -     3%  31.3 GB renderD128 vaapi@renderD128             -    Xwayland 508 MB, cosmic-app-libr 313 MB, cosmic-launcher 313 MB
dave               hyprland  running     866 MB       -     6%  31.3 GB renderD128 vaapi@renderD128             -    quickshell 567 MB, Hyprland 363 MB, Xwayland 253 MB
```

`RENDER` says whether the session's desktop is drawn on a GPU (the card's node) or on the CPU, and `ENCODER` which video encoder it runs (`vaapi`, `nvenc` or `openh264`), with the quantiser and the frame rate once frames flow: `vaapi@renderD128 q18 2fps`. A GPU encoder that cannot start falls back to OpenH264 on the CPU, and this is where you see it. `DESKTOP` is the desktop each session runs: `sway`, `plasma`, `cosmic` or `hyprland`. `AI` counts the connections each account has open to a local model, and shows only when `WAYDESK_AI_ENDPOINT` names one ([the GPU page](/docs/gpu/)). `PROGRAMS` lists the biggest programs of the session. On a server without a GPU the `GPU` line reads `none (no render node: sessions render and encode on the CPU)`, `RENDER` says `cpu` and `ENCODER` says `openh264`.

Two more forms, for watching a machine under load and for scripts:

```console
$ waydesk-admin status --watch        # redraws every 2 seconds; --watch 5 for 5 seconds
$ waydesk-admin status --json         # the same as JSON
$ waydesk-admin status --json --watch 1 | jq .machine.load   # one line of JSON per second
```

A session's renderer, encoder and frame rate are readable by that account and by root, so another administrator's row shows a dash.

## Access and certificates {bar: terminal}
```console
$ waydesk-admin access show
Listening on     127.0.0.1:3389  (local)
Who may connect   this machine only
Signed by        CN=your-server
Fingerprint      6F:E1:32:12:8C:64:71:A3:C0:51:C6:BB:80:37:64:B9:C4:E4:C7:1E:C7:C2:B8:93:E3:48:42:A5:C9:A4:FD:10
$ waydesk-admin cert status
Subject          CN=your-server
Signed by        itself (self-signed): clients have to compare the fingerprint
Valid until      Oct 25 19:19:21 2036 GMT
Names            
Fingerprint      6F:E1:32:12:8C:64:71:A3:C0:51:C6:BB:80:37:64:B9:C4:E4:C7:1E:C7:C2:B8:93:E3:48:42:A5:C9:A4:FD:10
```

`access local | lan | vpn | custom` change where the port listens; `cert local-ca`, `cert install`, `cert reissue` and `cert export-ca` manage what clients check. The [Connect page](/docs/connect/) explains each.

## Everything is scriptable {bar: notes}
`users list`, `users add`, `users passwd`, `sessions list` and `status` take `--json`. `waydesk-setup-admin --name NAME --password-stdin --access local` makes the first administrator without questions. `waydesk-admin --version` prints the version.
