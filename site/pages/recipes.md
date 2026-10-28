title: Recipes
nav: Recipes
seo_title: Waydesk recipes: a shared team server, a company VPN, a slow link
description: Three set-ups made from the documented commands: a shared server for a small team, a server behind a WireGuard VPN, and a long or lossy link.
---
Each recipe is the shortest path through commands that are explained elsewhere in the documentation.

## A shared server for a small team {bar: terminal}
One server, a few people, each with a desktop of their own and a memory ceiling.

```console
$ sudo apt install waydesk
$ sudo waydesk-setup-admin                  # the first administrator, and how clients connect
$ sudo waydesk-admin users add alice        # a temporary password, shown once
$ sudo waydesk-admin users add bob
$ sudo waydesk-admin users limit alice --memory 4G
$ waydesk-admin status                      # who is logged in and what each account uses
```

The server refuses a new session when memory would run out, so one account cannot take the others down. How many fit is on [Limits and status](/docs/limits/).

## A server behind a VPN {bar: terminal}
For several people, or anything reached over the internet: the RDP port exists only inside a WireGuard VPN.

```console
$ sudo waydesk-admin vpn init --endpoint vpn.example.com
$ sudo waydesk-admin vpn add alice-laptop   # a QR code for a phone, a text for a laptop
$ sudo waydesk-admin vpn list
```

Open UDP 51820 in the firewall (and the provider's security group), bring the VPN up on the client and connect the RDP client to `10.8.0.1`. [VPN and securing access](/docs/vpn/) has the steps and what it does not protect.

## A long or lossy link {bar: terminal}
A tunnel runs TCP inside TCP and stalls on a long round trip, so use the VPN above, and watch the picture adapt:

```console
$ waydesk-admin status --watch              # encoder, quantiser and frame rate per session
```

The picture follows what the link delivers, from 300 kbit/s to 10 Mbit/s, and gets blockier before it gets slower. The numbers are on [Measurements](/docs/measurements/).
