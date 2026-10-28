title: Connect
nav: connect
description: The ways an RDP client can reach a Waydesk server: an SSH tunnel, the local network, a WireGuard VPN, and certificates a client can verify.
---
The RDP login is a TLS login with the account's password, and there is no second factor. So the port is **closed to the network until you open it**. Four things are set, each with one command: where it listens, which addresses may connect, what certificate clients check, and, if you want one, a VPN.

## Where it listens, and who may connect {bar: table}
| mode | the port listens on | addresses that may connect |
|---|---|---|
| `local` (the default) | `127.0.0.1:3389` | this machine; reach it with an SSH tunnel |
| `lan` | this server's address on the local network | that network only (for example `192.168.1.0/24`) |
| `vpn` | the VPN address (`10.8.0.1`) | the VPN's addresses only |
| `custom` | what you choose with `--bind` | what you choose with `--allow` (default: private networks) |

The daemon checks the source address as soon as it accepts a connection, before TLS and before any login: an address that is not allowed gets nothing, so scans and password guessing from elsewhere never reach the login. Failed logins are delayed 2 seconds, and after five in a row every login is refused for a minute. Only members of `waydesk-users` may log in, and never root.

```console
$ waydesk-admin access show
Listening on     127.0.0.1:3389  (local)
Who may connect   this machine only
Signed by        CN=your-server
Fingerprint      6F:E1:32:12:8C:64:71:A3:C0:51:C6:BB:80:37:64:B9:C4:E4:C7:1E:C7:C2:B8:93:E3:48:42:A5:C9:A4:FD:10

Connect: from your computer run  ssh -N -L 3389:127.0.0.1:3389 ACCOUNT@THIS-SERVER  and point the RDP client at localhost.
$ sudo waydesk-admin access lan                   # this server's local network
$ sudo waydesk-admin access lan --allow "192.168.1.0/24 192.168.7.5"
$ sudo waydesk-admin access vpn                   # the WireGuard VPN made by `vpn init`
$ sudo waydesk-admin access local                 # back to this machine only
```

A list the daemon cannot read makes it refuse to start, so a typo never opens the port, and `access` puts the old settings back if the daemon does not start with the new ones.

## An SSH tunnel {bar: terminal}
One user, no setup on the server. Leave the port on `127.0.0.1` and, from your computer:

```console
$ ssh -N -L 3389:127.0.0.1:3389 you@your-server
```

Connect the RDP client to `localhost`. The tunnel lives only as long as that command runs; `ServerAliveInterval 30` in `~/.ssh/config`, or `autossh`, keeps it up across short drops.

> A tunnel runs TCP inside TCP, which stalls on a link with a long round trip (about 100 ms, another country): the picture follows the link down as far as it must, but a VPN is better there.

## The local network {bar: terminal}
```console
$ sudo waydesk-admin access lan
```

opens the port to this server's network only (other addresses are closed before TLS). `--allow` narrows or widens it. Do not use `--allow any` without a firewall you trust.

## A VPN {bar: notes}
The recommendation for several people or devices, and for anything over the internet: a key per device, nothing exposed but one UDP port. [The VPN page](/vpn/) walks through it.

## A certificate you can verify {bar: terminal}
At install the server makes a self-signed certificate, and the client has to compare its fingerprint on the first connection (`waydesk-admin cert status` prints it). A local certificate authority removes the comparison:

```console
$ sudo waydesk-admin cert local-ca          # also --san NAME_OR_IP for more names
$ sudo waydesk-admin cert export-ca         # prints the path of the authority's certificate
```

Copy that file to each client and import it **once** (macOS: add it to the login keychain and set Trust to Always Trust; Windows: `certmgr.msc`, Trusted Root Certification Authorities; Linux: `/usr/local/share/ca-certificates/` and `update-ca-certificates`). From then on every connection is checked and nothing is compared by hand. A timer renews the certificate before it ends. Where the server has a public name, `cert install --cert FULLCHAIN.pem --key KEY.pem` puts a Let's Encrypt certificate in place instead.

## Which client {bar: table}
| client | tried | notes |
|---|---|---|
| Windows App (macOS) | yes | add the PC as `localhost` through the tunnel; compare the certificate's fingerprint with the one the server printed |
| mstsc (Windows 11) | yes, on ARM in a VM | through an SSH tunnel; the VPN was not tried with it |
| FreeRDP / Remmina (Linux) | yes, Debian 13's FreeRDP 3.15 | Ubuntu's own `xfreerdp3` 3.32 cannot decode the H.264 stream |
| older Windows clients, iOS, Android | not tried | |
