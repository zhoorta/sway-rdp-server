# Omarchy (Hyprland) heavy ramp, 4 users, animations/blur/shadows switched off, 2026-10-31

Same machine, package (0.4.0-3) and procedure as `../i7-7700-arch-omarchy-heavy-2-argb-fix/`, first four steps; the wrapper configuration switched `animations`, `decoration.blur` and `decoration.shadow` off (checked with `hyprctl getoption`: all `false`).

| users | iGPU render engine, effects on | effects off | CPU busy, on | off |
|---|---|---|---|---|
| 1 | 44.8 % | 44.7 % | 11.2 % | 10.9 % |
| 2 | 21.6 % | 31.7 % | 8.3 % | 9.8 % |
| 3 | 47.7 % | 46.9 % | 16.0 % | 15.6 % |
| 4 | 65.3 % | 66.5 % | 21.3 % | 23.0 % |

**No difference**: the effects are not what uses the GPU in these sessions. A 12-second `intel_gpu_top` sample with the four sessions running attributes the render engine to the compositor: Hyprland 59 % (summed over the 4 sessions, about 15 % each), waydesk-server 2.6 %, Chromium 1.3 %, the shell 0.3 %, VS Code, the portal and Xwayland about 0. The 10-28 run with sway on Debian had the render engine at 15 % for 21 users, so a Hyprland compositor costs far more GPU per session than a sway one on the same chip. Why (per-frame composition, the two screencopy renders per captured frame, Omarchy's decoration settings) is not yet measured. The plan: the same ramp on the Waydesk desktop (sway) on this box, then per-client GPU sampling.
