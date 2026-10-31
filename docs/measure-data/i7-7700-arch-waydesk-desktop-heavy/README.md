# Waydesk desktop (sway) heavy ramp on the i7-7700, 2026-10-31

`../i7-7700-arch-waydesk-desktop-heavy.tsv` and `step-N.json`, from `harness/ramp-desktops.sh 40 waydesk-heavy waydesk-desktop heavy`. Same machine, workload and procedure as `../i7-7700-arch-omarchy-heavy-2-argb-fix/`, package 0.4.0-3; only the desktop differs (patched sway with the Waydesk desktop instead of Hyprland with Omarchy).
Clients on a Hetzner cloud server (8 dedicated vCPU, 1.7 ms); client CPU stayed under 23 %.

**21 users admitted, the 22nd refused by the memory rule.** No errors; command latency probe 72-124 ms.
- Memory: about 2.2 GB per heavy user (53.0 GB available with one user, 8.6 GB with twenty-one).
- CPU: 38 % busy at 21 users (46 % at 20); Waydesk's share 3.1 %. Omarchy was at 75 % with 20 users.
- iGPU render engine: 39 % with one user, 45-50 % from 2 to 10 users, 59-77 % at 11-21 (Omarchy: 98.5 % at 8 users). Video engine 4 % with one user, 27-32 % at 20-21.
- Per client (intel_gpu_top, column `gpu_top_clients`): VS Code about 24-36 %, sway 12-24 %, Chromium up to 10 %, waydesk-server 2-4 %.
- Median frame rate per session is low and erratic (0.4-19 fps): most sessions produce frames only when something changes, so the figure depends on what the workload was doing during the sample. Not comparable one to one with the Omarchy run, where the compositor kept redrawing.
- The run ended on the memory rule; no stop criterion for the render engine yet.
