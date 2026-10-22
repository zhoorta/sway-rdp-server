# Secure access with WireGuard

The RDP login is a plain TLS login with the account's password, and the port listens on `127.0.0.1:3389` by default. To reach it from
elsewhere, do not publish the port: put it behind a tunnel. This page shows two ways, WireGuard (a VPN, for several users or devices)
and an SSH tunnel (one user, no setup on the server).

> Status: written from the configuration of the services and WireGuard's documentation; **not yet tested end to end** on a Waydesk
> host. Try it on a throwaway machine first.

## WireGuard

The server gets the address `10.8.0.1` in the VPN, each client one more (`10.8.0.2`, ...). RDP listens only on `10.8.0.1`, so
without the tunnel the port does not exist for anyone. Examples use `wg0` and UDP port 51820; change them if they clash.

### Server

```sh
sudo apt install wireguard
umask 077
wg genkey | sudo tee /etc/wireguard/server.key | wg pubkey | sudo tee /etc/wireguard/server.pub
```

`/etc/wireguard/wg0.conf`:

```ini
[Interface]
Address = 10.8.0.1/24
ListenPort = 51820
PrivateKey = CONTENTS_OF_server.key

# One block per client (see "Adding a client").
[Peer]
PublicKey = CLIENT_PUBLIC_KEY
AllowedIPs = 10.8.0.2/32
```

The file and the key must belong to root and not be readable by others (`chmod 600`).

Make RDP listen on the VPN address and start after the interface is up. In `/etc/default/waydesk`:

```sh
WAYDESK_BIND=10.8.0.1:3389
```

and a drop-in so the service waits for WireGuard (`sudo systemctl edit waydesk.service`):

```ini
[Unit]
Requires=wg-quick@wg0.service
After=wg-quick@wg0.service
```

Then:

```sh
sudo systemctl enable --now wg-quick@wg0
sudo systemctl restart waydesk
```

Open only the WireGuard port to the internet (UDP 51820), in the host's firewall and in the cloud provider's security group. Do **not**
open 3389. With ufw: `sudo ufw allow 51820/udp`. The RDP port is reachable only inside the tunnel, so no rule for it is needed
unless the firewall also filters the `wg0` interface (then allow `3389/tcp` on `wg0` only).

### Adding a client

On the client (a laptop, here Linux; the WireGuard apps for Windows, macOS, iOS and Android make the same key pair):

```sh
wg genkey | tee client.key | wg pubkey
```

Send the public key (not the private one) to the admin, who adds a `[Peer]` block to `wg0.conf` with a free address and runs
`sudo wg syncconf wg0 <(sudo wg-quick strip wg0)`.

The client's configuration:

```ini
[Interface]
Address = 10.8.0.2/24
PrivateKey = CONTENTS_OF_client.key

[Peer]
PublicKey = CONTENTS_OF_server.pub
Endpoint = SERVER_NAME_OR_ADDRESS:51820
AllowedIPs = 10.8.0.1/32
PersistentKeepalive = 25
```

`AllowedIPs = 10.8.0.1/32` sends only the traffic for the server through the tunnel; the rest of the client's traffic is not touched.
Connect the RDP client to `10.8.0.1`. The certificate is self-signed: check the fingerprint printed at install time (or
`openssl x509 -in /etc/waydesk/cert.pem -noout -fingerprint -sha256`) the first time.

Remove a client by deleting its `[Peer]` block and syncing again. A key per device is better than one shared key, so that one lost
laptop is one block to remove.

### What this does and does not give

- The RDP port is invisible to anyone without a peer key, and the password login only travels inside the tunnel.
- A peer key is not a login: the account's password is still asked, and the memory and user limits still apply.
- Everyone with a peer can try passwords for every account, so give peers only to people you would let near the login.

## SSH tunnel (one user, no server setup)

Leave `WAYDESK_BIND` at `127.0.0.1:3389` and, from the client:

```sh
ssh -N -L 3389:127.0.0.1:3389 ACCOUNT@SERVER
```

then connect the RDP client to `localhost` (if the client machine has a local RDP port 3389 in use, forward `13389` and connect to
`localhost:13389`). Needs an SSH login on the server, and the tunnel lives only as long as that command runs. `autossh` or a
`ServerAliveInterval 30` in `~/.ssh/config` keeps it up across short drops.

## Which one

| | WireGuard | SSH tunnel |
|---|---|---|
| Server setup | one package and a few lines | none (sshd is usually there) |
| Many users and devices | yes, a key per device | an SSH account each |
| Always on | yes, in the background | only while the command runs |
| Port exposed | UDP 51820 | TCP 22 |
