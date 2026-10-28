title: The command line
nav: Command line
description: Every waydesk-admin command with its real output: accounts, sessions, limits, status and --watch, access, certificates and the VPN.
---
`waydesk-admin` is how an administrator looks after a server. Listing and `status` are open to every local account (an ordinary account sees its own row and the totals; administrators see every account); everything else needs root (`sudo`). Each command below was run on a real server, and the output is shown as it came, with only passwords and keys removed.

## At a glance {bar: table}
| command | what it does |
|---|---|
| `users list \| add \| remove \| passwd \| enable \| disable \| admin \| sudo \| limit` | accounts, passwords, rights and per-account limits |
| `sessions list \| end NAME` | who has a running session; end one |
| `status [--json] [--watch [SECONDS]]` | memory, swap, load, the GPUs, and what each account uses and runs |
| `access [show \| local \| lan \| vpn \| custom]` | where the RDP port listens and who may connect |
| `cert [status \| local-ca \| reissue \| export-ca \| install \| self-signed]` | the certificate clients check |
| `vpn [status \| init \| add \| show \| list \| remove]` | a WireGuard VPN for the port (see [the VPN page](/docs/vpn/)) |
| `waydesk-setup-admin` | the first administrator, and how clients connect |
| `waydesk-setup` | as your own account: choose browsers, office, developer and creative applications |

## Accounts {bar: terminal}
```console
$ sudo waydesk-admin users add alice
made alice (uid 1000), groups: waydesk-users
temporary password (shown once): ************
The account should change it at once: Change password, in the desktop's power menu.
$ sudo waydesk-admin users add bob --admin
made bob (uid 1001), groups: waydesk-users, waydesk-admins
temporary password (shown once): ************
The account should change it at once: Change password, in the desktop's power menu.
$ waydesk-admin users list
ACCOUNT                UID  RDP  FLATPAK  SUDO  SHELL   SESSION
alice                 1000  yes  -        -     yes     none
bob                   1001  yes  yes      -     yes     none
```

`RDP` is the right to log in over RDP (the group `waydesk-users`), `FLATPAK` installing applications without a password (`waydesk-admins`), `SUDO` the administrator panels of the desktop. `--sudo` and `--admin` give them when the account is made; `--no-rdp` makes an account that cannot log in.

```console
$ sudo waydesk-admin users disable bob
bob may no longer log in, and the session was ended
$ waydesk-admin users list
ACCOUNT                UID  RDP  FLATPAK  SUDO  SHELL   SESSION
alice                 1000  yes  -        -     yes     none
bob                   1001  -    yes      -     yes     none
```

`users enable NAME` gives the right back, `users disable NAME --keep-session` leaves a running session alone, `users passwd NAME` sets a new temporary password (`--stdin` reads it from a pipe, never from an argument), `users admin NAME on|off` and `users sudo NAME on|off` change the rights, and `users remove NAME [--delete-home]` removes the account. Accounts below uid 1000 and root are never touched.

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
`status` shows the machine and every session. On a server without a GPU:

```console
$ waydesk-admin status
Memory    7.6 GB in all, 7.0 GB available
Swap      zram0 7.6 GB (0 MB used)
Load      0.33 0.28 0.12 (4 cores)
GPU       none (no render node: sessions render and encode on the CPU)
Sessions  1 running; a new one would be accepted (needs 1.1 GB available)

ACCOUNT            SESSION     MEMORY    SWAP    CPU    LIMIT RENDER ENCODER            PROGRAMS (resident)
lt1                running     570 MB       -     3%   3.8 GB cpu    openh264 0fps      qs 481 MB, sway 73 MB, waydesk-server 70 MB
```

`RENDER` says whether the session's desktop is drawn on a GPU or on the CPU, and `ENCODER` which video encoder it runs (`vaapi`, `nvenc` or `openh264`), with the quantiser and the frame rate once frames flow: `vaapi q18 12fps`, `nvenc q25 17fps`. A GPU encoder that cannot start falls back to OpenH264 on the CPU, and this is where you see it. The [GPU page](/docs/gpu/) has a status from a machine with a card.

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
