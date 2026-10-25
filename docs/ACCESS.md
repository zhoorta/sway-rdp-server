# Who can reach the RDP port

The RDP login is a TLS login with the account's password (PAM cannot do NLA), and there is no second factor. So the port is closed to the
network until you open it, and there are four things to set: where it listens, which addresses may connect, what certificate clients
check, and, if you want one, a VPN. `waydesk-admin` does each in one command, and `waydesk-setup-admin` asks the first question for you.

```
sudo waydesk-admin access            # what it is now, and how to connect
sudo waydesk-admin access local      # this machine only (the default): an SSH tunnel
sudo waydesk-admin access lan        # this server's local network
sudo waydesk-admin access vpn        # the WireGuard VPN made by `vpn init`
sudo waydesk-admin cert local-ca     # a certificate that clients can verify
sudo waydesk-admin vpn init          # a WireGuard VPN, then: vpn add NAME
```

## Where it listens, and who may connect

| mode | the port listens on | addresses that may connect |
|---|---|---|
| `local` (default) | `127.0.0.1:3389` | this machine; reach it with `ssh -N -L 3389:127.0.0.1:3389 ACCOUNT@SERVER` |
| `lan` | this server's address on the local network | that network only (for example `192.168.1.0/24`) |
| `vpn` | the VPN address (`10.8.0.1`) | the VPN's addresses only |
| `custom` | what you choose with `--bind` | what you choose with `--allow` (default: private networks) |

The daemon checks the source address as soon as it accepts a connection, before TLS and before any login: an address that is not allowed gets
nothing, so scans and password guessing from elsewhere never reach the login. It logs this at most once every ten seconds (with how many were
refused meanwhile). Without a list, a daemon on loopback allows anything (only this machine can reach it) and a daemon on any other address
allows only **private** addresses: 10/8, 172.16/12, 192.168/16, 100.64/10 (Tailscale), link-local, `fc00::/7`, and this machine. To
allow more, or less, set `WAYDESK_ALLOW_FROM` in `/etc/default/waydesk` (`access ... --allow LIST` does it): ranges, single addresses, and the
words `private`, `loopback` and `any`:

```
sudo waydesk-admin access lan --allow "192.168.1.0/24 192.168.7.5"
sudo waydesk-admin access custom --bind 0.0.0.0:3389 --allow any      # every address: only behind a firewall you trust
```

A list the daemon cannot read makes it refuse to start, so a typo never opens the port. `access` puts the old settings back if the daemon does not
start with the new ones. Failed logins are delayed 2 s, and after five in a row every login is refused for a minute (see `docs/SECURITY-REVIEW.md`
for what that does and does not protect against). Only members of `waydesk-users` may log in, and never root.

## A certificate clients can verify

At install the server makes a self-signed certificate, and the client has to compare its fingerprint on the first connection (printed at install,
and by `waydesk-admin cert status`). A client that does not compare can be fooled by a man in the middle, who would then see the password. A local
certificate authority removes that:

```
sudo waydesk-admin cert local-ca                   # also --san NAME_OR_IP for more names
sudo waydesk-admin cert export-ca                  # prints the path of the authority's certificate: copy it to each client
```

This makes an authority of your own (its key stays in `/etc/waydesk/ca`, root only), and a certificate for this server's names and addresses
signed by it (valid 800 days; macOS refuses longer). Each client imports the authority's certificate **once** (macOS: double-click it, add it to
the login keychain, set Trust to Always Trust; Windows: `certmgr.msc`, Trusted Root Certification Authorities, Import; Linux: `/usr/local/share/ca-certificates/`
and `update-ca-certificates`), and from then on checks every connection; there is no fingerprint to compare and no warning. A timer renews the certificate
before it ends. After a new address or name, `waydesk-admin cert reissue --san NAME`. `waydesk-admin cert self-signed` goes back to the first kind.

**Let's Encrypt.** Where the server has a public name, a certificate from elsewhere works the same way: `sudo waydesk-admin cert install --cert FULLCHAIN.pem --key KEY.pem`
checks that the key belongs to the certificate and that it has not expired, puts it in place and restarts the daemon (the old one comes back if the daemon does
not start). To renew it with certbot, add a deploy hook, for example `--deploy-hook 'waydesk-admin cert install --cert $RENEWED_LINEAGE/fullchain.pem --key $RENEWED_LINEAGE/privkey.pem'`.

## A VPN, in one command

```
sudo waydesk-admin vpn init --endpoint vpn.example.org     # the name or public address clients connect to
sudo waydesk-admin vpn add laptop                          # prints the client's configuration, and a QR code when qrencode is installed
sudo waydesk-admin vpn list                                # the clients, and when each last connected
sudo waydesk-admin vpn remove laptop                       # stops its access at once
```

`vpn init` installs `wireguard-tools` if needed, makes the server side on its own interface (`waydesk0`, UDP 51820, `10.8.0.0/24`; change them with `--interface`, `--port`,
`--subnet`), makes RDP listen on the VPN address only for the VPN's addresses (`access vpn`), starts the daemon after the VPN, and opens the port in `ufw` if it is on.
Open UDP 51820 in the provider's firewall too. It refuses to replace an existing configuration without `--force` (a backup is kept) and uses an interface name of its own, so a
`wg0` you made by hand is left alone. A client's configuration routes only this server's VPN address through the tunnel (not the client's other traffic), and has a preshared
key on top of the keys. The configurations stay in `/etc/waydesk/vpn/peers` (root only), so `vpn show NAME` prints one again. The manual way is in `docs/WIREGUARD.md`.

## Over the internet

A VPN is still the recommendation: there is no second factor, so a guessed or leaked password is enough. If you open the port to the internet anyway (`--allow any`),
use long random passwords, a certificate from the local authority or Let's Encrypt, and a firewall that rate-limits the port.
