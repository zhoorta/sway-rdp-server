# Capacity test: how many Omarchy users on one server (plan, 2026-10-31)

Driver: `ramp-desktops.sh`. Same machine, same procedure, three desktops, so the only variable is the desktop.

## Before anything: the criteria (also written in the header of every `out-*.tsv`)
Capacity is reported **per resource**, and the headline is the lowest of them:

| Resource | Stop when | Default |
|---|---|---|
| RAM | available memory under | `STOP_AVAIL_MB=1500` (the daemon's own admission rule may refuse earlier: reported as "refused") |
| CPU | busy share of all threads (clients and harness subtracted) over a 60 s sample at or over | `STOP_CPU=90` |
| Encoder / GPU | median fps of the sessions under (video profile only) | `STOP_FPS=12` |
| Validity | the client machine busier than | `STOP_CLIENT_CPU=60` (that step and later ones do not count) |

Capacity = the last step before the first stop. Also record `errors` and `probe_ms` (compositor answer time); a step with errors in the journal is not a clean step.

## Machines
- Server: the i7-7700 (62 GB, HD 630), **clean install** of the rebuilt Arch package (`packaging/arch/build.sh`), rebooted once (polkit socket), `free -m` noted. Omarchy installed as on 10-30; for the sway runs the Waydesk desktop (patched sway + DMS) installed too. Ideally a box without Omarchy's user units (fcitx5 ~37 MB and omarchy-crash-watch ~7 MB leak into sway sessions): note it if not.
- Client machine: Hetzner cloud, same data centre as the server, **8 dedicated vCPU, 16-32 GB** (CCX33 class), `freerdp3-sdl` (Arch: `freerdp`; check the binary name `sdl-freerdp3`). Direct connection, RDP allowed from the client only (`waydesk-admin access custom --bind SERVER:3389 --allow CLIENT`), never a tunnel. Resize if `client_vps_cpu_pct` passes 60.
- Driver: the Mac, plugged in, `caffeinate -dis`.

## Runs (each from the same starting state: no lt* accounts, `drop_caches`, same `free -m`)
1. `./ramp-desktops.sh 40 omarchy-mixed omarchy` : mixed profile (page that scrolls, typing terminal, every 4th a video loop, three with sound).
2. `./ramp-desktops.sh 40 waydesk-mixed waydesk-desktop`
3. `./ramp-desktops.sh 40 sway-min-mixed sway-minimal`
4. Phase 2 (encoder): at N = the capacity from run 1, `STOP_FPS=12 ./ramp-desktops.sh N omarchy-video omarchy video` (every user loops a 720p video), and the same for the two sway desktops. Reports how many streams the HD 630 encoder holds at steady fps; add a software-encoder comparison (`WAYDESK_ENCODER=software`) only if time allows.
Between runs: `./cleanup-remote.sh` (stops clients and workloads, removes the lt* accounts), `loginctl` shows no sessions, wait until `free -m` is back to the baseline.
The SSH key and known hosts: `SSH_KEY`, `KNOWN_HOSTS` (client), `SERVER_KNOWN_HOSTS` (server; the i7 IP had an old entry).

## Raw data
`out-LABEL.tsv` (one row per step, criteria in the header), `raw-LABEL/step-N.json` (`waydesk-admin status --json` after each step: per-session render, encoder, fps, kbps), `raw-LABEL/system.txt` (versions, CPU, config). Commit the tsv and json, with the scripts, next to the article.

## To state in the article
Experimental; Hyprland needs a GPU (no software renderer); `LIBSEAT_BACKEND=noop` and the GPU card node are open to every account (docs/HYPRLAND.md); no Waydesk shell on Hyprland; the two bugs the multi-user test found (monitor rule reset, polkit socket); iGPU memory is system memory (2.24 GB against 1.95 GB per user in the 10-28 run); clients on a separate machine; the OS of each number (10-28 numbers are Debian, these Arch); the workload is not "heavy" (no Calc/VS Code) because focus handling on Hyprland is not scripted: say so.

## Setup on the server (Arch), once
`pacman -S intel-gpu-tools wtype foot chromium python imagemagick ffmpeg`, then `/srv/load` (page.html, video.html, video-audio.html, term.sh, work.sh, sample.py, chmod 755), `/root/load` (cleanup.sh), `/srv/capacity` as in README.md only for the HEAVY profile (not used here). Check `/srv/load/clip.mp4` exists (README ffmpeg line).
