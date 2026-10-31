# Omarchy (Hyprland) heavy ramp after the ARGB8888 capture fix, 2026-10-31

`../i7-7700-arch-omarchy-heavy-2-argb-fix.tsv` and `step-N.json`, from `harness/ramp-desktops.sh 40 omarchy-heavy-2 omarchy heavy`. Same machine and procedure as `../i7-7700-arch-omarchy-heavy/` (before the fix); package 0.4.0-3 (commit 5f2a5dd).
Clients on a Hetzner cloud server (8 dedicated vCPU, same data centre, 1.7 ms); client machine CPU stayed under 32 %.

**20 users admitted, the 21st refused by the memory rule** (9.6 GB available, 9.4 GB needed: 15 % of the RAM). Before the fix the run stopped at 9 users on CPU (92 %).
- Memory: about 2.4 GB per heavy user (55.8 GB available with one user, 9.4 GB with twenty); no swap in use.
- CPU: 75 % busy at 20 users; Waydesk's share of the machine 4.8 % (33 % at 9 users before the fix).
- The iGPU render engine was saturated from about 8 users (98.5 %) and stayed at 100 % from user 11; the video (encode) engine reached 34 %. The median frame rate per session fell from 5-8 fps to 3.3 fps as users were added, so the users past about 8-10 were admitted but the picture was updated less often. No errors in the journal; the command latency probe stayed between 70 and 194 ms.
- The test has no stop criterion for the render engine (added to the next version of the script); the run ended on the memory rule.
