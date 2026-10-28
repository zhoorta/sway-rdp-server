title: A VPN with WireGuard
nav: vpn
description: Put the RDP port behind a WireGuard VPN with three commands: init, add a client, scan the QR code. A key per device and one UDP port open.
---
For several people or devices, or anything over the internet, do not publish the RDP port: put it behind a VPN. `waydesk-admin vpn` makes the WireGuard server side, one client at a time, and a QR code for a phone. The RDP port then listens on the VPN address only, so without a key it does not exist for anyone.

## Make the VPN {bar: terminal}
`vpn init` installs `wireguard-tools` if needed, makes the server side on its own interface (`waydesk0`, UDP 51820, `10.8.0.0/24`; change them with `--interface`, `--port`, `--subnet`), makes RDP listen on the VPN address only, starts the daemon after the VPN, and opens the port in `ufw` if it is on. The endpoint is the name or public address clients will connect to.

```console
$ sudo waydesk-admin vpn init --endpoint vpn.example.com
Open UDP 51820 to the internet in this machine's firewall and in the provider's security group, if there is one.
Done.
Listening on     10.8.0.1:3389  (vpn)
Who may connect   10.8.0.0/24
Signed by        CN=your-server
Fingerprint      6F:E1:32:12:8C:64:71:A3:C0:51:C6:BB:80:37:64:B9:C4:E4:C7:1E:C7:C2:B8:93:E3:48:42:A5:C9:A4:FD:10

Connect: bring the VPN up on the client, then point the RDP client at 10.8.0.1. `waydesk-admin vpn add NAME` makes a client.

The VPN is ready. Add a client with:  sudo waydesk-admin vpn add NAME
```

**Open UDP 51820** in the provider's firewall too (a cloud security group is outside the machine). Do not open 3389: the RDP port exists only inside the tunnel. `vpn init` refuses to replace an existing configuration without `--force` (it keeps a backup) and uses an interface name of its own, so a `wg0` you made by hand is left alone.

## Add a client {bar: terminal}
One client per device, so that a lost laptop is one entry to remove. Install `qrencode` first if you want the QR code for a phone:

```console
$ sudo apt install qrencode
$ sudo waydesk-admin vpn add phone
Client phone (10.8.0.2) added. Import this in the WireGuard app (or scan the QR code), then connect the RDP client to 10.8.0.1:

[Interface]
PrivateKey = <the client's private key>
Address = 10.8.0.2/32

[Peer]
PublicKey = <the server's public key>
PresharedKey = <a preshared key, on top of the keys>
Endpoint = vpn.example.com:51820
AllowedIPs = 10.8.0.1/32
PersistentKeepalive = 25

(the QR code of the same configuration follows)
```

On the phone or laptop, install the WireGuard app, scan the code (or import the text), and switch the tunnel on. `AllowedIPs = 10.8.0.1/32` sends only the traffic for the server through the tunnel; the rest of the client's traffic is not touched. Then connect the RDP client to **`10.8.0.1`**.

The configurations stay in `/etc/waydesk/vpn/peers` (root only), so `vpn show NAME` prints one again.

## See and remove clients {bar: terminal}
```console
$ sudo waydesk-admin vpn list
Interface    waydesk0  (10.8.0.1, subnet 10.8.0.0/24, UDP port 51820)
Endpoint     vpn.example.com:51820
State        up

CLIENT               ADDRESS         LAST CONNECTED
phone                10.8.0.2        never
$ sudo waydesk-admin vpn remove phone        # stops its access at once
```

`vpn status` shows the same listing. After the phone connects, `LAST CONNECTED` says when.

## What it gives, and what it does not {bar: notes}
- The RDP port is invisible to anyone without a peer key, and the password login only travels inside the tunnel.
- **A peer key is not a login.** The account's password is still asked, and the memory and user limits still apply.
- Everyone with a peer can try passwords for every account, so give peers only to people you would let near the login.
- There is still no second factor: long, random passwords matter.

## VPN or SSH tunnel {bar: table}
| | WireGuard VPN | SSH tunnel |
|---|---|---|
| Server setup | one command | none (sshd is usually there) |
| Many users and devices | yes, a key per device | an SSH account each |
| Always on | yes, in the background | only while the command runs |
| Long round trips | handles them well | TCP inside TCP stalls |
| Port exposed | UDP 51820 | TCP 22 |

The manual way, without `waydesk-admin`, is in [`docs/WIREGUARD.md`](https://github.com/zhoorta/waydesk/blob/main/docs/WIREGUARD.md), and the whole access model in [`docs/ACCESS.md`](https://github.com/zhoorta/waydesk/blob/main/docs/ACCESS.md).
