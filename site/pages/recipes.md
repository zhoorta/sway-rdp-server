title: Recipes
nav: Recipes
seo_title: Waydesk recipes: a team server, a VPN, a slow link, a shared local AI model, hosted AI with one key per account
description: Six set-ups made from the documented commands: a shared server for a small team, a server behind a WireGuard VPN, a long or lossy link, a GPU server that also runs a shared model, hosted AI with one key per account, and workstations that use that model.
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
$ sudo waydesk-admin gpu roles renderD128=render renderD129=inference   # what each card is for
$ sudo waydesk-admin ai endpoint 127.0.0.1:11434                        # where the model listens, for the AI line and column
$ sudo systemctl edit ollama                # Environment="CUDA_VISIBLE_DEVICES=GPU-..." (the value from status)
$ sudo waydesk-admin users set alice --gpu renderD128   # optional: pin an account to the Intel card; auto already skips the inference card
$ sudo waydesk-admin status --watch         # the card's role, the model and its VRAM, an AI column per account
```

Sessions that start afterwards draw on the Intel card, and the NVIDIA card holds the model alone. On a test server (Intel UHD 770 plus an RTX 4000 SFF Ada with a 70 W limit) the model's speed did not change with 1, 5 or 10 sessions on the Intel card, and the sessions took no video memory on the other. The same sessions on the model's own card cost about 0.5 GB of video memory each and slowed the model by up to a quarter. The numbers are on [Measurements](/docs/measurements/) and the settings on [Using a GPU](/docs/gpu/).

Two things to know. Ollama on `localhost` has no authentication, so every account on the server can use it, and there are no per-account limits on it; the `AI` column shows who has requests open, not how many tokens. And a card with the role `inference` only keeps sessions off it when they have another card to go to.

## Hosted AI for every account, one key each {bar: terminal}
Claude, GPT and the other hosted models can be used by every account of the server without anyone sharing a login. The organisation's key stays in a gateway on the server, each account gets a key of its own from the gateway, and usage, budgets and revocation are per account. Waydesk does not install or run the gateway: this is the pattern the vendors document, checked against their pages on 2026-11-01 and **not yet tried on a Waydesk server**.

What not to do: copy one person's login (`~/.claude`, a browser session) into the other accounts, or put one API key in a file every account can read. A subscription login belongs to one person (read the terms of your plan), and a shared file means nobody can be cut off or told apart. ChatGPT's apps and GitHub Copilot are licensed per seat: each person needs their own, and no gateway changes that.

The gateway is a program on `127.0.0.1` that holds the real key and hands out virtual ones. [LiteLLM](https://docs.litellm.ai/docs/proxy/virtual_keys) is an example (virtual keys need a Postgres database); Anthropic also documents [gateways for Claude Code](https://code.claude.com/docs/en/llm-gateway) and a self-hosted one with single sign-on, and says it does not endorse or audit third-party gateway products. With LiteLLM:

```console
$ sudo litellm --config /etc/litellm/config.yaml    # the models, and DATABASE_URL and LITELLM_MASTER_KEY from root-only files
$ curl -s http://127.0.0.1:4000/key/generate -H "Authorization: Bearer $LITELLM_MASTER_KEY" -H 'Content-Type: application/json' \
    -d '{"key_alias": "alice", "max_budget": 50, "budget_duration": "30d"}'      # one key per account, with a monthly budget
$ sudo install -D -o alice -m 0400 /dev/stdin /etc/waydesk/ai-keys/alice       # paste her key; only she (and root) can read it
$ sudo waydesk-admin ai endpoint 127.0.0.1:4000                                 # status then shows who has requests open
```

**Claude Code** takes the address and the key from managed settings, so the accounts configure nothing. One file for all of them, `/etc/claude-code/managed-settings.json`, and a helper that prints the key of whoever runs it (the helper must print the key and nothing else; Claude Code keeps its answer for five minutes):

```console
$ cat /etc/claude-code/managed-settings.json
{ "env": { "ANTHROPIC_BASE_URL": "http://127.0.0.1:4000" }, "apiKeyHelper": "/usr/local/bin/waydesk-ai-key" }
$ cat /usr/local/bin/waydesk-ai-key
#!/bin/sh
exec tr -d '\n' < "/etc/waydesk/ai-keys/$(id -un)"
```

While a gateway key is active, Claude Code sends it instead of any claude.ai login, and the usage is billed per token to whoever owns the key the gateway forwards (the organisation's account), not to a subscription. Inside the session, `/status` shows the base URL and the credential source. Remote Control and voice dictation need a claude.ai login and are unavailable with a gateway key. Version 2.1.285 or later can refuse any other destination with `allowedProviders: ["customEndpoint"]` in the same file.

**Codex** takes a custom provider in each account's `~/.codex/config.toml` (not in a project's file, which it ignores): `model_provider = "gateway"` and a `[model_providers.gateway]` table with `base_url` (the gateway's OpenAI-compatible address) and `env_key = "OPENAI_API_KEY"`, with that variable set from the account's key file. Other tools that accept an OpenAI-compatible base URL and a key are set up the same way.

To cut an account off, block its key (`/key/block` in LiteLLM) and the next request fails at once, whatever the tool; the real key never reached the account. The `AI` column of `waydesk-admin status` shows which accounts have a connection open to the gateway; the tokens and the cost per key are the gateway's own report.

Things to know. The gateway on `localhost` is reachable by every account, which is why each one needs its own key; keep the master key and the real provider key in files only root can read. A budget is only enforced when you set one (LiteLLM's is empty by default). The gateway is one more service to keep running and updated, and a gateway that does not forward what a new Claude Code release sends breaks the matching feature. And the answer to "can all my users share one subscription?" depends on your plan's terms, which are the vendor's, not ours.

## Workstations that use the server's model {bar: terminal}
The desktops stay local and only the model is shared, so no Waydesk session is needed. This uses other tools and is not something Waydesk configures or has tested: the model service listens on the VPN address only, and the workstations point their editor or chat application at it.

```console
$ sudo systemctl edit ollama                # Environment="OLLAMA_HOST=10.8.0.1:11434" (the VPN address of the server)
$ sudo systemctl restart ollama
```

The port is reachable only through the [VPN](/docs/vpn/), as the RDP port is. The model service has no accounts, so for several people, separate keys or budgets, put a gateway such as LiteLLM or Open WebUI in front of it; that is their documentation, not Waydesk's.
