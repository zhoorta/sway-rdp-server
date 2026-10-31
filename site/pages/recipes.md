title: Recipes
nav: Recipes
seo_title: Waydesk recipes: a team server, a VPN, a slow link, a shared local AI model
description: Five set-ups made from the documented commands: a shared server for a small team, a server behind a WireGuard VPN, a long or lossy link, a GPU server that also runs a shared model, and workstations that use that model.
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

## A GPU server that also runs a shared model {bar: terminal}
Desktops on the Intel card, the NVIDIA card kept for a local model (Ollama, vLLM or another service) that every session reaches on `localhost`. Waydesk does not install or start the model; it keeps the sessions off its card and shows who uses it.

```console
$ sudo waydesk-admin status                 # note the card names and the Inference line (CUDA_VISIBLE_DEVICES=GPU-...)
$ sudoedit /etc/default/waydesk             # WAYDESK_GPU_ROLES=renderD128=render,renderD129=inference
                                            # WAYDESK_AI_ENDPOINT=127.0.0.1:11434
$ sudo systemctl edit ollama                # Environment="CUDA_VISIBLE_DEVICES=GPU-..." (the value from status)
$ sudo waydesk-admin users set alice --gpu renderD128   # optional: pin an account to the Intel card; auto already skips the inference card
$ sudo waydesk-admin status --watch         # the card's role, the model and its VRAM, an AI column per account
```

Sessions that start afterwards draw on the Intel card, and the NVIDIA card holds the model alone. On a test server (Intel UHD 770 plus an RTX 4000 SFF Ada with a 70 W limit) the model's speed did not change with 1, 5 or 10 sessions on the Intel card, and the sessions took no video memory on the other. The same sessions on the model's own card cost about 0.5 GB of video memory each and slowed the model by up to a quarter. The numbers are on [Measurements](/docs/measurements/) and the settings on [Using a GPU](/docs/gpu/).

Two things to know. Ollama on `localhost` has no authentication, so every account on the server can use it, and there are no per-account limits on it; the `AI` column shows who has requests open, not how many tokens. And a card with the role `inference` only keeps sessions off it when they have another card to go to.

## Workstations that use the server's model {bar: terminal}
The desktops stay local and only the model is shared, so no Waydesk session is needed. This uses other tools and is not something Waydesk configures or has tested: the model service listens on the VPN address only, and the workstations point their editor or chat application at it.

```console
$ sudo systemctl edit ollama                # Environment="OLLAMA_HOST=10.8.0.1:11434" (the VPN address of the server)
$ sudo systemctl restart ollama
```

The port is reachable only through the [VPN](/docs/vpn/), as the RDP port is. The model service has no accounts, so for several people, separate keys or budgets, put a gateway such as LiteLLM or Open WebUI in front of it; that is their documentation, not Waydesk's.
