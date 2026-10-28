# Measurements of the multi-user build (2026-10-19)

Synthetic loads on one VM, measured on the server side only. No subjective latency was recorded in writing (the video played smoothly in the Windows App; the window-drag leftovers described in the git log were fixed afterwards). Treat the numbers as an order of magnitude, not a benchmark.

## One session, 1680x1050
VM burin-dev (4 vCPU arm64, 4 GB, software rendering, pixman), one session (rdptest) with DMS, LibreOffice Draw, Thunar and foot open, Mac Windows App via SSH tunnel, 1680x1050. Package build from a4aef6a + working tree (2026-10-16). 60 s per state; CPU = /proc utime+stime deltas, share of ONE core; network = loopback byte counters (sway-rdp -> sshd), includes protocol overhead; memory = RSS sum of the session user + sway-rdp daemon.

| State | sway-rdp-server | sway | Total session mem | Network |
|---|---|---|---|---|
| Idle | ~2% total (incl. DMS) | - | 949 MB | ~1 KB/s |
| Scrolling terminal (seq loop) | 53.6% | 3.2% | 1101 MB | ~292 KB/s |
| Test video 720p30 (mpv, lavfi testsrc2) | 75.7% | 3.4% | 1300 MB | ~1127 KB/s |
| Window moving (~26 moves/s) | 56.4% | 2.9% | 1101 MB | ~193 KB/s |

Largest idle consumer: DMS quickshell (qs) ~446 MB RSS.
Not measured: hardware encoding (none in the VM).
Sampler bugs fixed along the way: ps %cpu is a lifetime average (first idle run 15.7% was invalid); overlapping samplers; short-lived load generators (seq) missing from per-process tables.

## DMS animations on vs off (2026-10-19, one session at 1120x672, control centre + dash toggled every ~0.9 s, 45 s)
| | qs (DMS) | sway-rdp-server | sway | whole session | Network |
|---|---|---|---|---|---|
| Animations on (DMS default, "Short") | 89.2% | 13.2% | 6.2% | 109.2% | 132 KB/s |
| Animations off (settings.json animationSpeed 0 etc.) | 25.8% | 7.0% | 2.2% | 37.6% | 91 KB/s |
DMS defaults found in the code (v1.6.3): blur off (it is only available when the compositor offers `ext-background-effect-v1`; not checked for sway), popup and dock transparency opaque, animations Short. The package now writes a settings file without animations on an account's first start (commit 92cf4cf).

## Two sessions (2026-10-19, both clients visible, both 1120x672, 60 s)
| Run | sway-rdp-server | load generators | Network | Used RAM (free -m) |
|---|---|---|---|---|
| Video in session 1, session 2 idle | 32.9% | mpv 11.3% | 595 KB/s | 1663 MB |
| Video in session 1 + scrolling terminal in session 2 | 49.2% | mpv 9.1%, foot 33.9% | 738 KB/s | 1760 MB |
Whole VM: 99.8% of one core of 4 (including the load generators). Frames acked: video ~26/s, terminal ~14/s.
Control: single session at 1680x1050 video was 75.7% (see table above); cost follows pixel count.
An earlier two-session run with session 2 frozen is invalid (resize credit bug, fixed in 7f76ba6).

## Memory per session, current build (2026-10-21)
VM sway-rdp-dev (4 vCPU arm64, 4 GB, software rendering), 1280x800, desktop with bar and dock, package from 5642db1 with the patched sway. Memory = PSS (shared libraries divided between the processes that map them) summed per process name for the account, after 60-90 s of idle.

| State | Total (PSS) | Largest |
|---|---|---|
| First session, just logged in | 608 MB | qs 394, waydesk-server (session process) 45, dms 39, pcmanfm-qt 29, sway 23, xdg-desktop-portal 18, python3 helpers 15, pipewire-pulse 7, wireplumber 4 |
| Plus a terminal and a file manager window | 643 MB | qterminal +19, pcmanfm-qt +5 |
| cgroup memory.current of the account's slice (includes page cache) | 841 MB / 876 MB | |
| Each further idle session (drop in MemAvailable, caches dropped before and after) | about 410 MB (3195172 kB to 2780428 kB) | |
| Daemon (`waydesk`), one process for all connections | 5 MB RSS | |

qs (DankMaterialShell) inside: 307 MB anonymous, 97 MB file-backed; libLLVM 51 MB and libgallium 10 MB (Mesa's software OpenGL, shared between sessions), the JS heap 26 MB.
`QT_QUICK_BACKEND=software` (no Mesa/LLVM): qs 398 MB to 227 MB; with `WAYDESK_DMS_SOFTWARE=1` from /etc/default/waydesk 212 MB. Look: the bar, dock and launcher draw correctly; the settings window has square corners with small artifacts and the wallpaper preview stays blank (no rounded clipping or shader effects in Qt's software renderer). Not measured: CPU with it, popups other than settings, launcher and dash.
The VM was killed by the out-of-memory killer twice on 2026-10-21 with five accounts logged in (kernel log: "Out of memory: Killed process ... (qs)", oom_score_adj 200, UID of the account that was using the VM): the victim is the shell of an account, not sway.

## Users per 4 GB, with Chromium (2026-10-21)
Same VM (4 GB, no swap, 3748 MB available with no session after dropping caches), package from 0c5b764. Accounts logged in one after the other over RDP (1280x800), 60 s after login the session is idle; then `chromium` (default profile, first launch) with three pages (example.org, debian.org, en.wikipedia.org/wiki/Linux), 55 s later memory = drop in MemAvailable (real memory, shared pages counted once).

| User | Session, idle | Chromium + 3 pages | Total | Available afterwards |
|---|---|---|---|---|
| 1 (fresh4) | 503 MB | 260 MB | 763 MB | 2871 MB |
| 2 (fresh5) | 452 MB | 315 MB | 767 MB | 2104 MB |
| 3 (fresh6) | 407 MB | 245 MB | 652 MB | 1452 MB |
| 4 (fresh7) | 364 MB | 269 MB | 633 MB | 810 MB (830 MB after a later reading) |

Average 704 MB per user (session 432, Chromium 272). The session cost falls with each user (shared libraries and caches are loaded once). No out-of-memory kill; a fifth user (about 700 MB) would leave about 100 MB, so it was not tried (the VM was killed by the OOM killer twice earlier today with five accounts). Chromium with the same three pages measured as PSS (the table above, 2026-10-21 earlier): +450 MB for 15 processes; Firefox ESR with four tabs +779 MB for 16 processes (one run each).
Capacity: 4 users with a browser (or 3 with real work), about 8 idle sessions; WAYDESK_DMS_SOFTWARE=1 takes about 170 MB off each session.

## Users per 4 GB with the memory protection (2026-10-21, package from the commit "Memory protection ...")
Same VM and procedure as above (accounts logged in one after the other, Chromium with three pages, 55 s after each step), now with the zram swap
(zram0, 1.9 GB, zstd, priority 100), `vm.swappiness=100`, the per-account ceilings (35 % / 50 %) and the admission rule (900 MB available needed).

| User | Available after Chromium opened | Swap used | Next login |
|---|---|---|---|
| 1 (fresh4) | 2855 MB | 0 | accepted |
| 2 (fresh5) | 2199 MB | 0 | accepted |
| 3 (fresh6) | 1552 MB | 0 | accepted |
| 4 (fresh7) | 917 MB | 0 | accepted (917 > 900) |
| 5 (rdptest) | 537 MB | 238 MB (417 MB a few minutes later) | refused (535 < 900) |
| 6 (rdptest2) | 600 MB | | REFUSED at login |
| 7 (rdptest3) | 611 MB | | REFUSED at login |

Five users fit, no out-of-memory kill (kernel log: 0 in the 30 minutes of the run), the machine stayed usable at about 650 MB available. Without zram the same
VM had 830 MB available after four users and no room for a fifth. At the end `waydesk-admin status` showed per account 490-780 MB of its cgroup, 70-130 MB of it in swap,
and Chromium as the largest program (about 1.2 GB resident, shared pages counted for each process).

## Hetzner CAX11, over the internet (2026-10-22, commit 6dac08c)
Hetzner Cloud CAX11 (2 vCPU Ampere, 3.8 GB, Debian 13.6 arm64, zram 1.9 GB), the `.deb` packages of that commit plus DMS 2.0.0db1, quickshell 0.3.0 and the
patched sway; RDP on a WireGuard address (`docs/WIREGUARD.md`), reached from a Mac (ping 77 ms). `tester1` came in with the Windows App (1680x1050);
`tester2`-`tester4` with `sdl-freerdp` at 1280x720, one after the other, Chromium with three real pages (Wikipedia, Hacker News, debian.org) opened in each from the
server, 65 s after each login.

| Sessions | Available after Chromium opened | Swap used | Next login |
|---|---|---|---|
| 1 | 2.8 GB | 653 MB | accepted |
| 2 | 2.0 GB | 598 MB | accepted |
| 3 | 1.3 GB | 597 MB | accepted |
| 4 | 699 MB | 736 MB | REFUSED (699 < 900) |

- The fifth account (`tester5`) logged in (PAM), then the helper refused the session: "short of memory (700 MB available, 900 MB needed), running=4". No out-of-memory kill
  (kernel log 0), the four sessions went on. The RDP client only shows "Network disconnect": the reason is in the server log, not on the user's screen.
- An account with a running session reconnected while the server was full (`tester3`: "reconnecting to the running session").
- A session with the desktop idle: 581 MB (earlier VM figure: about 610). With Chromium and the three pages: 750 MB to 1.1 GB per account. One fewer user than the
  VM run (five), with more swap in use from the start: real pages, a long-lived first session and the package cache of the server in memory.
- CPU: a session whose screen keeps changing (`tester1`) kept the session process (H.264 encoder) at 70-90 % of a core and Chromium at about 27 %; the idle ones were at 3-7 %.
  On two cores the encoder, not memory, is likely to be the limit with several active users. Not measured yet with several busy sessions.
- Found on the way: Software (gnome-software) crashed at start because `librsvg2-common` (the SVG loader of GTK) was not installed after a
  `--no-install-recommends` install; fixed by making it a dependency of `waydesk-desktop` (4960315).

### CPU with moving screens (same server, same day)
Load: `mpv --vo=wlshm --fs` playing a generated 720p30 test pattern (`av://lavfi:testsrc2`) in each session (alone it runs in real time on this machine), clients
`sdl-freerdp` on the Mac over the tunnel; `tester1` had Software open at the same time (flatpak work), which adds some load. Frame rate counted from the server's
acknowledgement log (one line per 200 frames).

| Sessions playing the pattern | Machine busy (both cores) | Session process (capture + H.264) per session | Frame rate per session |
|---|---|---|---|
| 1 | 62 % | about 36 % of a core | 14 fps |
| 2 | 89 % | 20-30 % | 15 fps each |
| 3 | 99 % | 20-23 % | about 5 fps each (4 sessions open) |

- With three moving screens the two cores are used up and the frame rate falls to about a third; the frames still arrive (acknowledged = sent, queue depth 0): the picture gets
  choppy, it does not lag behind.
- One or two moving screens give 14-15 fps, well under the 30 fps of the capture loop (`session.rs`: 30 ms sleep after each frame). Not profiled; a likely cause is the time one
  capture takes (sway renders without a GPU) on top of that sleep. This is the first thing to look at if smoothness for video matters.
- Window work, scrolling and typing were fine in the desktop session over the same link (the session on `tester1`, "looks great").
- Rule of thumb for 2 vCPU: memory allows 4 browser users; two of them with continuous motion already use about 90 % of the CPU.

## Where the CPU of a moving screen goes (2026-10-22, dev VM, arm64, perf)
The Hetzner run (above) gave 14-15 fps for one moving 720p screen and 5 fps for three. To see why, the session process of one account (capture, colour
conversion, H.264) was profiled with `perf record -F 999 -g` while a full-screen 720p30 test pattern (`mpv av://lavfi:testsrc2`) played, client `sdl-freerdp`.

- **89 % of the session process is inside libopenh264** (motion search: `SumOf8x8BlockOfFrame`, `FillQpelLocationByFeatureValue`, `WelsSampleSad*`). Our own code is
  about 9 % (capture and the frame comparison in `Shared::push` about 2 %, colour conversion 2.6 %). The 30 ms sleep of the capture loop is not the limit: the VM
  delivers 30.3 fps with it. The 14 fps of the CAX11 were the encoder on slower, busy cores.
- Same load, session process CPU and bytes sent (30 s each, 30 fps in every row):

| Encoder setting | CPU of the session process | Sent to the client |
|---|---|---|
| screen content, medium, 4 threads (the default) | 43 % of a core | 6.4 Mbit/s |
| screen content, low complexity | 43 % | not measured |
| screen content, medium, 1 thread | 44 % | not measured |
| **camera video** (`WAYDESK_H264_USAGE=camera`) | **15 %** | **12.7 Mbit/s** |

- Complexity and the number of threads change nothing measurable. The usage type is the lever: camera mode is about three times cheaper in CPU and needs twice the
  bandwidth. It stays an option, not the default (bandwidth over the internet matters more for most users); a CPU-starved server that plays video can set it in
  `/etc/default/waydesk`. Scrolling text in a terminal (4 % CPU, 1.5 against 1.7 Mbit/s) is the same in both modes, so ordinary work does not care.
- Not done: the same comparison on the CAX11 itself (the server was deleted before this), the picture quality of camera mode on text (not looked at), a switch
  that picks the mode by itself when the CPU is short.
- Correction (2026-10-26, encoder benchmark): the "same in both modes" line above holds for a terminal, not for every desktop. See the next section.
- Trap while measuring: logging in again right after `loginctl terminate-user` can time out on `user@UID.service` (90 s); wait until the old user manager has stopped.

## H.264 encoder benchmark (2026-10-26, Nuremberg test server, x86, 2 vCPU)
`waydesk-encbench` (src/bin) encodes synthetic scenes with the same OpenH264 settings as the server, 120 frames at 30 fps, bitrate cap 10 Mbit/s, CPU as a
share of one core. Scenes: `pattern` (worst case, everything moves), `video` (moving gradients), `desktop` (mostly still, one area scrolls), `settle` (the desktop at rest).

| Scene, size | Camera | Screen, medium | Screen, low |
|---|---|---|---|
| pattern 720p | 42-45 %, 10 Mbit/s | 41-43 %, 6.7 Mbit/s | 26-29 %, 6.9 Mbit/s |
| video 720p | 22 %, 3.7 Mbit/s | 33 %, 0.71 Mbit/s | 28-30 %, 0.60 Mbit/s |
| desktop 720p | 9-10 %, 1.7-2.0 Mbit/s | 30 %, 0.93-1.06 Mbit/s | 30 % |
| desktop 1680x1050 | 20 %, 2.7 Mbit/s | 54 %, 1.4 Mbit/s | 54 % |
| at rest 720p | 8.6 %, 1.08 Mbit/s, PSNR 42.3 dB | 24.7 %, 0.56 Mbit/s, PSNR 30.6 dB | 25.2 % |
| at rest 1680x1050 | 17 %, 1.73 Mbit/s, PSNR 42.0 dB | 46 %, 0.86 Mbit/s, PSNR 30.0 dB | 45 % |

- Screen mode has an almost constant cost per frame (about 8 ms at 720p, 15 ms at 1680x1050) whatever the content; camera mode is very cheap on pictures that hardly change.
  The "three times" comes from there, not from motion.
- Low complexity saves 15-36 % under heavy motion at the same bits and PSNR, and nothing on a still desktop. It is the default since this run.
- Scene-change detection, background detection, adaptive quantisation, long-term references and the thread count change almost nothing (OpenH264 turns the
  background and adaptive quantisation off in screen mode by itself).
- PSNR on screen mode is about 12 dB lower at rest. Not yet judged on real text: PSNR on synthetic text may not show legibility.
- The benchmark encodes every frame; the server only encodes when something changed, so an idle desktop costs nothing. The numbers apply to scrolling and animation.
- Idea, not done: convert only the changed rectangles to YUV and use a cheaper path for tiny damage.

## Chromium memory flags (2026-10-22, dev VM, one account, fresh profile, Wikipedia + Hacker News + debian.org, PSS 75 s after start)
Where the 396 MB went (default flags): renderers 5 processes 146 MB, browser 116 MB, GPU process 60 MB, utility 36 MB, zygotes 26 MB.

| Flags | Processes | PSS |
|---|---|---|
| none (default) | 14 | 396 MB, 399 MB (two runs) |
| `--enable-low-end-device-mode` | 13 | 384 MB (window hidden, see below) |
| `--enable-low-end-device-mode --renderer-process-limit=2` | 13 | 396 MB (window hidden) |
| `--in-process-gpu` | 13 | 401 MB (window hidden) |
| **`--disable-gpu`** | 15 | **357 MB, 363 MB (two runs)** |

- Only `--disable-gpu` helps: about 38 MB (10 %) per account that browses. On an animated page (canvas plus a CSS animation) Chromium used 16 % of a core
  and the encoder 14-16 % with or without it, and a screenshot of the session shows the same picture. The hosts have no GPU (headless sway), so the GPU process
  only did software rendering. It is shipped as `/etc/chromium.d/waydesk` (a conffile of `waydesk-desktop`), which Debian's wrapper reads on every start.
- The first rows were measured while a fullscreen terminal from an earlier test covered the browser window (Chromium does not paint a covered window), so they
  are indicative; the default and `--disable-gpu` rows were repeated with the window visible. The low-end and process-limit flags were not repeated.
- Not tried: `--disable-site-per-process` (fewer renderers, but it weakens the isolation between sites: not recommended), Firefox ESR.

## Where the shell's memory goes, and what trimming can save (2026-10-22, dev VM, one account, DMS 2.0.0 as package waydesk-shell, 45 s after start)
Idle `qs` (the shell), RSS / PSS / anonymous memory:

| Variant | RSS | PSS | anon |
|---|---|---|---|
| installed shell (baseline, three runs) | 360 / 360 / 363 MB | 321-326 MB | 234-237 MB |
| `MALLOC_ARENA_MAX=1` | 357 | 317 | 231 |
| `LP_NUM_THREADS=1` (Mesa's software rasteriser threads) | 354 | 317 | 233 |
| `QSG_RENDER_LOOP=basic` | 360 | 323 | 235 |
| `GALLIUM_DRIVER=softpipe` | **530** | 495 | 415 |
| without the desktop widget layer, its editor and menu | 353 | 316 | 228 |
| without the lock screen | 361 | 325 | 236 |
| without desktop widgets + MuxModal + ClipboardHistoryModal + ColorPickerModal | 338 | 303 | 215 |
| **lean patch** (`70-lean-shell.py`: desktop widgets + clipboard history) | **341** | 306 | 217 |
| Qt software renderer (`WAYDESK_DMS_SOFTWARE=1`) | **239** | 208 | 171 |
| software renderer + square corners | **249** | | |
| **floor**: bare quickshell, one 32 px bar window, no DMS | **283** | 220 | 179 |

- Of the shell's 360 MB, 283 MB is the floor (Qt Quick plus Mesa's software OpenGL, with its 48 MB `libLLVM`, shared between accounts). Everything DMS adds
  is about 77 MB; the cuts that are free of feature loss save 19 MB of it. The environment knobs change nothing (4-6 MB, noise); `softpipe` is much worse.
- The one big lever is the renderer: the Qt software backend saves about 110 MB per account, with a flat look (rounded corners and shadows cannot be drawn: with
  rounded corners it leaves dark square halos behind the bar's pills and the clock; a screenshot of both is why new accounts get square corners in that mode).
- `MALLOC_ARENA_MAX`, `LP_NUM_THREADS` and the render loop were tried by restarting only `dms.service` of a running session; "PSS" counts shared library pages
  divided among the processes that map them, so with several accounts the 48 MB `libLLVM` costs each of them less than here.

## Hetzner 8 GB / 4 vCPU x86 (AMD EPYC Genoa), Debian 13 amd64 (2026-10-22)
The amd64 packages built natively on this server (server and desktop 2 min, patched sway 1.5 min, file manager 1.5 min, shell 1 min, the three helpers 2.5 min: 9 minutes in all, against hours under
emulation on the Mac), installed from their build folder (quickshell from Debian's backports), first administrator made with `waydesk-setup-admin`, RDP reached through an SSH tunnel.

- One session, idle desktop: 600 MB (the 4 GB arm64 server: about 520 MB), `qs` 438 MB, the four bar plugins loaded, the screenshot looks like the arm64 one.
- Starting a session takes 0.7-1.4 s from the login (0.25-0.34 s inside the helper once the login is accepted).
- **Browser users on 8 GB: 9** (Chromium with three pages each). The tenth login was refused: "1046 MB available, 1162 MB needed" (15 % of the memory). No out-of-memory kill; zram (3.8 GB) held 584 MB.
  Per account at the end: 600-790 MB of the account's cgroup, Chromium about 1.2-1.3 GB resident (shared pages counted for each process). Load average 0.4 on 4 cores with everyone idle after loading.
- Expected from the 4 GB arm64 run (4 users): about 8; measured: 9.

How it was measured (a trap): the first attempt logged in through an SSH tunnel from the Mac with `sdl-freerdp`. Five of ten logins failed with "no login within 30 s" or "failed to accept client", even with the
Mac idle: all the clients' video streams share one tunnel over the Mac's connection (55-450 KB/s measured that day), so a new connection cannot finish its handshake once five or six Chromium sessions are
streaming. The server was fine (the same accounts logged in within 1.4 s from the server's own loopback). The test was redone with a short-lived `xfreerdp3` on the server itself under Xvfb, one per account:
the session stays after the client leaves, exactly as for a user who closes the window. Use that for capacity tests, not a tunnel.
- Not measured on x86: CPU with moving screens (video), and the frame rate.

## Heavy users on the 8 GB / 4 vCPU x86 server: Chromium, LibreOffice Calc, VS Code, and the size of the zram (2026-10-23)
Available memory is `free`'s figure in GiB (as `waydesk-admin status` prints it). Debian 13 amd64 installed from the signed repository (`waydesk-repo.deb`, key checked), then `waydesk-setup` (shell, Chromium, LibreOffice, VS Code from Flathub, git). Every account
logs in over RDP from a short-lived `xfreerdp3` on the server (Xvfb) and then opens: Chromium with three pages (Wikipedia, BBC News, debian.org), LibreOffice Calc with a CSV of 300,000 rows by
10 columns (23 MB), and VS Code (Flatpak) on a copy of this repository (11 MB). 70 s after the last application started, the numbers below were taken. The applications are left idle: this is an upper bound.

- **One user with all of it open: about 1.85 GB** (cgroup of the account; the shell 420 MB, Chromium about 1 GB resident, VS Code about 1.1 GB resident, Calc 250-330 MB). CPU about 2 %, load 0.4.
- **Three users: 1.1-1.4 GB each** in their cgroups, 2.4 GB available, 4-83 MB in swap, CPU 86 % idle, no memory pressure.

| Users (all heavy) | Available with zram 3.8 GB (Debian's size) | Available with zram 7.6 GB (`zram-size = ram`) |
|---|---|---|
| 1 | 5.4 GB | 5.4 GB |
| 2 | 3.9 GB | 3.9 GB |
| 3 | 2.4 GB | 2.4 GB |
| 4 | 1.8 GB | 1.6 GB |
| 5 | 1.7 GB | 1.7 GB |
| 6 | 1.1 GB (a new session would be refused) | 1.6 GB |
| 7 | refused (`ERRINFO_OUT_OF_MEMORY` on the client) | 1.6 GB |
| 8 | | 1.1 GB (1165 MB, just above the 1162 MB a new session needs) |
| 9 | | refused |

- **The zram compresses about 4.5 to 1** with zstd on this load: 3.4 GB of data in 780 MB of RAM at six users (3.8 GB device), 6.8 GB in 1.5 GB at eight users (7.6 GB device). So a device as large as the
  RAM costs about a fifth of the RAM when it is full, and lets the kernel swap out idle anonymous pages instead of dropping the file cache. Six users fit with Debian's size, eight with the larger one:
  the package now makes the device as large as the RAM (at most 16 GiB).
- No out-of-memory kill in either run (kernel log), memory pressure ("some", 60 s average) between 0.4 and 2 %, and the server still answered SSH in 1.3 s with six heavy users in the first run. The refusal at the
  last step reached the client as "the server ran out of available memory resources" (`ERRINFO_OUT_OF_MEMORY`), as designed.
- **Zed does not run on a server without a GPU**: "Failed to create surface for any enabled backend", also with `ZED_ALLOW_EMULATED_GPU=1`. VS Code (Electron, Wayland) and Calc are fine.
- **LibreOffice did not start at first**: without `libreoffice-gtk3` (only a Recommends of `libreoffice`, skipped on an image that does not install Recommends) `soffice` looks for an X server ("Set DISPLAY").
  The setup manifest now installs it. The same reason made `systemd-zram-generator`, pipewire, `whiptail` and `wmenu` hard dependencies: a fresh Hetzner Debian image installs without Recommends, and the first
  clean install ended with no compressed swap and no audio.
- Not measured: CPU with active work (a build, a recalculation), typing and scrolling latency, and the frame rate with moving screens on this server.
- An earlier run of the same procedure on another server ended with that server taken over (SSH root password login, see `HETZNER-TEST.md` section 0); the capacity figures were taken before that and agree with
  the clean rerun (six users fit, the seventh refused). The lock-up that followed was the intruder's miner, not memory.

## Clean installs from the published repository, x86 and arm64 (2026-10-23)
Two fresh Hetzner servers, Debian 13: **x86_64, 4 vCPU, 7.6 GB** and **arm64 (CAX), 2 vCPU, 3.7 GB**. SSH hardened first (key only, checked from outside). Then exactly what the site tells a user, with nothing built locally:
`waydesk-repo.deb` from `https://waydesk.stackcare.pt/apt`, fingerprint compared (C646 72B2 5ECD 4773 196C 3958 9D8E 8874 E676 17F4), `apt update`, `apt install waydesk`,
`waydesk-setup` (shell and Chromium, through the setup engine with a selection file), `waydesk-setup-admin`, an RDP login from `xfreerdp3` on the server.

| | x86_64 | arm64 |
|---|---|---|
| `apt install waydesk` | exit 0 | exit 0 |
| zram at first boot | 7.6 GB zstd | 3.7 GB zstd |
| both services | active | active |
| session | shell, dock (appears with the first window), Chromium with title-bar buttons | same |
| audio | PulseAudio on PipeWire 1.4.2, default sink `waydesk-server`; the bar shows the speaker as on | same |
| idle session with Chromium on one page | 629 MB (cgroup), 6.2 GB available | 788 MB, 2.6 GB available |

**A failed first attempt that was a real bug:** the first install of the day ended with `404` for `sway` and `pcmanfm-qt`: the live repository's index named packages that were not on the server. `publish.sh` compared its file lists with `comm`
after sorting them on two machines (the Mac and the server sort `+`, `~` and `_` differently), so it reported files that were there as missing and as extra, and its last step deleted them by name after the upload. Fixed
(`c988000`): set differences with awk, no change to the indexes unless every package is on the server, and every package requested over HTTP at the end ("all 17 packages answer 200").

**Capacity with the published packages** (clients: `xfreerdp3` on the server, one account at a time, 70 s or 55 s after the last application started, apps left idle):
- **x86, 8 GB, heavy users (Chromium with Wikipedia, BBC News and debian.org; LibreOffice Calc with 300,000 rows; VS Code on a 25 MB source tree): 8 fit, the ninth was refused** ("994 MB available, 1162 MB needed" at the end).
  The zram, at the new default size (7.6 GB), held 6.8 GB of data in 1.5 GB of RAM (4.5 to 1). No out-of-memory kill, memory pressure ("some", 60 s) about 1.2 %. The same result as the run with local packages.
- **arm64, 3.7 GB, browser users (the same three pages): 4 fit, the fifth was refused** ("754 MB available, 900 MB needed"). No out-of-memory kill; zram held 871 MB of data in 182 MB. Not comparable with the earlier five users
  on a 4 GB virtual machine, which opened lighter pages; BBC News alone is several hundred MB.
- Observation, not changed: the admission rule looks at available memory, which does not count what the kernel could still free by moving idle pages into the (now large) zram. With 3 GB of zram free, the arm64 server
  refused the fifth user at 754 MB available. Whether the rule should count some of that is open; it errs on the safe side.
- Not measured: CPU with active work, latency, moving screens on these two servers.

## Clean install and heavy users on Ubuntu 26.04 x86 (2026-10-24)
Fresh Hetzner server, **Ubuntu 26.04.1, x86_64, 4 vCPU, 7.6 GB**. SSH hardened first (key only, checked from outside). Then what the site tells a user: `waydesk-repo.deb` from `https://waydesk.stackcare.pt/apt` (suite `resolute` chosen from
`VERSION_CODENAME`, fingerprint C646 72B2 5ECD 4773 196C 3958 9D8E 8874 E676 17F4 compared), `apt install waydesk` (exit 0, zram 7.6 GB zstd, `waydesk` and `waydesk-helper` active), `waydesk-setup-admin`, and the setup engine
with `vscode` and `dms` selected. Clients: a Debian 13 container with `xfreerdp3` and Xvfb on the server (Ubuntu's own `xfreerdp3` cannot decode our H.264).

- **Session**: the shell (bar, clock, wallpaper), desktop icons, LibreOffice Calc (apt), VS Code and Chromium (Flathub). Chromium has the title-bar buttons and a correct dock icon once its first-run dialogs are out of the way.
- **Bug found by this run, fixed in `4639c24` (not yet published)**: `dms.sh` added Debian's `trixie-backports` and installed `quickshell` from it even with the Waydesk repository configured. On Ubuntu that source is unsigned, so `apt update` failed
  and `waydesk-shell` was never installed (the session had a desktop but no shell). With the repository configured the script now installs only `waydesk-shell`, which brings `waydesk-quickshell`.
- **Two first-run dialogs of the Flathub Chromium (fixed the same day in `waydesk-shell`, checked with a new account)**: "Chromium Additional Terms of Service" (a placeholder text in this build; the window has an empty app id, so the dock showed "?" and
  the browser did nothing until it was accepted) and gnome-keyring's "Choose password for new keyring" (the page stayed on "Loading..." behind it). `waydesk-shell` now writes `First Run` and `EULA Accepted` (what the accept button writes) and
  `--password-store=basic` for the Flatpak build. The basic store keeps cookies and saved passwords under a fixed key, protected only by the home directory's permissions. A keyring that unlocks with the login password is not possible
  with the present design: the password is checked by the helper (PAM service `waydesk-server`, authentication and account only), the session starts from another unit with its own PAM service, so `pam_gnome_keyring` never sees the password.
  The capacity accounts below were seeded the same way by hand. The "!" badge on the desktop icon of the Flatpak Chromium is still there.
  A session that was started before the shell was installed keeps running without it until `waydesk-admin sessions end`.

**Capacity, heavy users** (Chromium with Wikipedia "Linux", BBC News, debian.org; Calc with a 300,000 x 10 CSV, 23 MB; VS Code on a 30 MB copy of the Python standard library, 1308 files; one account at a time, 70 to 100 s after the last
application started, applications left idle, so an upper bound for idle and a lower bound for active work; CPU of a user was still 140 % at the first reading because VS Code was indexing and its welcome screen animates):

| Users (all heavy) | Available | zram data / compressed |
|---|---|---|
| 1 | 5.2 GB | 0.7 MB |
| 2 | 3.4 GB | 0.7 MB |
| 3 | 2.1 GB | 472 MB / 113 MB |
| 4 | 2.2 GB | 2.2 GB / 471 MB |
| 5 | 2.0 GB | 3.9 GB / 816 MB |
| 6 | 1.7 GB | 5.6 GB / 1.1 GB |
| 7 | 810 MB (a new session is refused: 1161 MB needed) | 6.6 GB / 1.3 GB |
| 8 | refused (`the server is short of memory (472 MB available, 1161 MB needed)`, the client was disconnected after 2 s) | |

- **7 heavy users fit, the 8th was refused**; on Debian 13 (same hardware class, same procedure) 8 fit. One user cost 2.0 GB here (cgroup: Chromium 1.6-1.9 GB, VS Code 1.4 GB, shell 400 MB; Calc about 210 MB) against 1.85 GB on Debian; the Flatpak
  builds of Chromium and VS Code and the 30 MB tree (25 MB on Debian) explain part of it, the rest was not separated.
- Compression about 5 to 1 (6.6 GB of data in 1.3 GB). With seven users the zram device was full (7.5 GB of 7.6 GB in use as swap), no out-of-memory kill, memory pressure ("some", 60 s) 0.3 to 2.4 %.
- Not measured: CPU under active work, latency, moving screens; Ubuntu arm64; Debian with the corrected `dms.sh`.

## Clean installs from the repository republished on 2026-10-24 (version 0.0.1~202610241746.git7cc4c91)
Two freshly rebuilt Hetzner servers, SSH hardened first (key only, checked from outside): **Debian 13 x86_64** (4 vCPU, 7.6 GB) and **Ubuntu 26.04 arm64** (2 vCPU, 3.7 GB). The four steps of the site (`waydesk-repo.deb`, key fingerprint
compared, `apt install waydesk`, `waydesk-setup-admin`), then the setup engine with `dms` and `vscode` selected, then an RDP login with Debian 13's `xfreerdp3` and Chromium, Calc and VS Code started in the session.

| | Debian 13 x86_64 | Ubuntu 26.04 arm64 |
|---|---|---|
| suite chosen by `waydesk-repo` | trixie | resolute |
| `apt install waydesk`, setup engine | exit 0, exit 0 | exit 0, exit 0 |
| apt sources afterwards | Debian's own and Waydesk's (no backports) | Ubuntu's own and Waydesk's |
| shell | waydesk-shell 2.0.0+waydesk2, waydesk-quickshell 0.3.0+deb13 | same, +ubuntu26.04 |
| Chromium | apt package, no dialog | Flathub, opens with no terms window and no keyring prompt, page loads |
| Calc, VS Code (Flathub) | run | run |
| session with Chromium, Calc and VS Code open | 2.3 GB | 1.2 GB in RAM and 880 MB in zram (the account's ceiling is 1.9 GB on this 3.7 GB server) |

- The Ubuntu x86 and Debian arm64 combinations were exercised earlier the same day from locally built packages of the same sources (`...1657`, one commit older: the Chromium seeding was added after), Debian arm64 also from the repository on 2026-10-23. They were not reinstalled from the final repository.
- The arm64 server's stream was decoded with the x86 Debian client through two ssh tunnels: the arm64 `xfreerdp3` on Xvfb shows a shifted double image of the same correct stream (it does the same with the previous server build), so pictures of arm64 servers are taken from an x86 client.

## From the new address, waydesk.dev/apt (2026-10-24, evening)
- **Clean install on Debian 13 x86_64, 2 vCPU, 3.7 GB**, a freshly created server (SSH hardened first): `waydesk-repo.deb` from `https://waydesk.dev/apt`, key fingerprint compared, `apt install waydesk` and the setup engine
  (`dms`, `vscode`) both exit 0, suite `trixie`, no backports, shell 2.0.0+waydesk2 with quickshell 0.3.0+deb13; RDP login with the shell, bar and desktop icons (idle session 593 MB with the shell).
- **Upgrade path on Ubuntu 26.04 arm64**: a server with `waydesk-repo` 1.1 (old address) installs the package from the new address. 1.2 kept the old address in `waydesk.sources` (the postinst never touched an existing
  file); 1.3 moves a file that still has exactly the old address to the new one and leaves every other local edit alone (checked with an extra `Architectures:` line). `apt update` then fetches from waydesk.dev.

## TCP congestion control: BBR against CUBIC on a link that loses packets (2026-10-25)
One link, one afternoon, so read it as "what happens on a lossy long link", not as a general factor.
- **Server**: Debian 13 x86_64, 2 vCPU, kernel 6.12.107+deb13-cloud, Hetzner Nuremberg. **Client**: the Windows App on macOS, over WireGuard (the RDP connection runs inside it). Video at the configured 10 Mbit/s, graphics pipeline with H.264.
- **Method**: `ss -ti dst CLIENT` on the server for the RDP connection, read every 4 to 5 seconds while the picture was moving (scrolling a page, dragging a window), and `nstat` for the whole machine. CUBIC was the system default; for the second run only the default was changed
  (`sysctl net.ipv4.tcp_congestion_control=bbr`, `net.core.default_qdisc=fq`) and the client reconnected (a running session survives that); nothing else changed.

| | CUBIC (default) | BBR |
|---|---|---|
| round trip, as TCP sees it | 142 to 160 ms | 110 to 119 ms |
| what the connection delivered while the picture moved | 20 kbit/s to 1.5 Mbit/s, typically 200 to 400 | BBR's own estimate 1.9 to 4.3 Mbit/s (it sent 2.7 MB in 17 s, about 1.3 Mbit/s: the picture needed no more) |
| congestion window (segments) | 4 to 22 (slow start threshold 3 in one sample) | 15 to 90 |
| bytes waiting unsent on the server | 50 to 120 kB, up to 317 kB | 1 to 13 kB |
| retransmitted since the connection began | 3.9 % of the segments (808 of 20 604), 4.5 % of the bytes | 7 to 9 % of the bytes; the retransmitted segments per interval went 45, 36, 14, 3 as BBR probed (CUBIC: 10 to 13 per interval at a tenth of the traffic) |
| delay of the path the flow control measured | not usable: frames waited 1 to 9 seconds in the queue | 120 to 130 ms, no queue reported (frame acknowledgement times were not captured in this run) |

- **Why**: with a loss rate of a few percent CUBIC cannot get far: the Mathis formula, MSS / RTT x 1.22 / sqrt(loss) = 1368 x 8 / 0.15 x 1.22 / sqrt(0.039), gives about 0.45 Mbit/s, the order of what was seen; BBR does not take a random loss for congestion.
- **Typing**: with CUBIC the keystrokes reached the server in bursts. `xev` in the session (58 events for the sentence "the quick brown fox"): four groups of keys with the same arrival time (for example `e`, space, `q`, `u` at 1483 ms, `space`, `f`, `o` at 4823 ms), and two keys whose release arrived 600 ms or more after the
  press, so that key repeat started (`c` repeated nine times, `o` once). The stalls were about a second, which fits a retransmission timeout (344 ms) doubling after a loss (that is an inference: the cause of each stall was not traced). With BBR the typing was reported as "much much better"; it was **not** captured with `xev` again.
- **What the server now does**: the daemon asks for BBR on each RDP connection (`WAYDESK_TCP_CONGESTION`, empty for the system's default); the package loads `tcp_bbr` and adds `bbr` to `net.ipv4.tcp_allowed_congestion_control`, which is what lets a process that is not root use it. The system's default stays CUBIC.
- **Not measured**: a link without loss (then BBR and CUBIC should be close), the effect on a LAN, other kernels.

## Heavy users on a 12-vCPU / 23 GB server: Hetzner CPX52, Debian 13 x86 (2026-10-26)
Debian 13 amd64 from the published repository (version 0.3.1, key checked), `waydesk-setup` for the shell, Chromium, LibreOffice and VS Code (Flathub). The server cost 123.60 euros a month at the time (hourly billing up to that). Two accounts of the administrator
were open and in use (one playing a video with sound) while the test ran. The test (`docs/measure-data/capacity.sh`, results in `docs/measure-data/capacity-cpx52.tsv`) adds one account at a time. Each logs in over RDP from a short-lived `xfreerdp3` on the
server (never through a tunnel) and opens Chromium with three pages (Wikipedia, BBC News, debian.org), LibreOffice Calc with a CSV of 300,000 rows by 10 columns (23 MB) and VS Code (Flatpak) on a copy of the Python 3 library (28 MB); 75 s after the last application
started the numbers were taken. The applications are left idle: an upper bound. The test stops at the first refusal.

| Heavy accounts | Available memory | Swap in use | Load (12 cores) | CPU idle | Memory of the new account |
|---|---|---|---|---|---|
| 0 (two administrator sessions) | 15.8 GB | 1.1 GB | 2.5 | | |
| 1 | 14.8 GB | 1.0 GB | 1.2 | 94 % | 2.1 GB |
| 2 | 13.4 GB | 1.0 GB | 0.7 | 96 % | 1.6 GB |
| 4 | 14.5 GB | 0.6 GB | 1.6 | 93 % | 1.6 GB |
| 6 | 11.1 GB | 0.5 GB | 2.0 | 78 % | 1.5 GB |
| 8 | 7.7 GB | 0.5 GB | 2.0 | 86 % | 1.6 GB |
| 10 | 5.5 GB | 0.5 GB | 5.5 | 71 % | 1.6 GB |
| 12 | 3.4 GB | 1.1 GB | 2.5 | 87 % | 1.7 GB |
| 13 | refused: 3466 MB available, 3519 MB needed (`ERRINFO_OUT_OF_MEMORY` on the client) | | | | |

- **12 heavy accounts fit, the 13th was refused**, with the two administrator sessions still open (14 sessions at the end). The refusal is the daemon's rule that a new session needs 15 % of the RAM available (3.4 GB here).
- **Memory is the limit, not the CPU**: CPU idle stayed between 71 % and 96 % (the lowest values come while an account's applications were starting), the load peaked at 5.8 on 12 cores, and the CPU pressure average (`avg300`) was 2.8 %.
- **No out-of-memory kill** in the kernel log, memory pressure `some` and `full` 0.07 % (`avg300`) at the end, 1.1 GB of swap in use. The zram held 1 GB of data in 217 MB (about 4.7 to 1).
- **One heavy account costs about 1.6 GB** (1.3 to 2.1 GB in its cgroup; the first one more, with the shared files not yet in the cache). On 8 GB the earlier test measured about 1.85 GB per account and eight accounts.
- **The administrator's session stayed smooth** (video with sound in Chromium, pointer and windows) through all twelve additions: nothing was held back (send queue 0 bytes on every connection, encoder bitrate 23 to 35 Mbit/s).
- **Cost**: 123.60 euros a month for 12 heavy accounts is about 10.30 euros per account and month; with the browser-and-shell use of an ordinary account (about 1 GB) roughly twice as many fit.
- **Not measured**: applications that are busy (the numbers are for idle ones), a second round of the same accounts, and a server with a GPU.


## A GPU: heavy active users, WebGL, applications, the fallback, and NVIDIA (2026-10-28)
Hetzner server auction, dedicated: Intel Core i7-7700 (4 cores, 8 threads), its integrated HD Graphics 630, 64 GB, Debian 13 (kernel 6.12), installed with `installimage` with kernel mode setting on (Hetzner's image
adds `nomodeset` and blacklists `i915` by default, and `/dev/dri` does not appear until both are undone). The 0.3.1 packages with the server binary of the commits of the day (`2082cd5` and after: the GPU for the desktop,
the VAAPI encoder, zero-copy capture, the quantiser in the slice headers), about 0.12 euros an hour. The users are accounts made by `waydesk-admin users add`; the raw tables are in `docs/measure-data/` and the scripts that
produced them in `docs/measure-data/harness/`.

**What a user does** (the *heavy* workload, each user active, not idle): Chromium with a page that scrolls by itself every 2 to 5 seconds, a terminal that types at 5 to 12 characters a second and prints bursts of output,
LibreOffice Calc with a 300,000 x 10 sheet and VS Code (Flatpak) on a 28 MB source tree, both with keystrokes sent into them (Page Down, typing; `wtype`) every few seconds, a switch of window every 15 to 30 seconds; every fourth
user also has GIMP on a 4000 x 3000 image (zoom in and out); every fourth a muted 720p video looping in a second Chromium; three users watch the same video with sound (the audio is uncompressed 48 kHz stereo, 1.5 Mbit/s each).
Users are added one at a time (about 3.5 minutes each), each with its own RDP client, until the machine refuses one. The clients are `sdl-freerdp3` on **a separate cloud server in the same data centre** (8 vCPU, 0.5 ms
away), so that the decoding of 20 pictures does not take the CPU of the machine measured (they took about 2 % of the machine per user when they were on it). The CPU is the busy share of all 8 threads (`/proc/stat`) over a
60-second sample taken 100 seconds after the user's applications started; "workers" is what the session processes (capture and encoding) used, as a share of the whole machine; the "probe" is how long `swaymsg -t get_version`
took in the newest user's session.

| Active heavy users | CPU, GPU | workers, GPU | free memory, GPU | CPU, software | workers, software | free memory, software |
|---|---|---|---|---|---|---|
| 1 | 23 % | 1.9 % | 53.4 GB | 21 % | 3.7 % | 53.6 GB |
| 5 | 45 % | 3.7 % | 44.2 GB | 48 % | 9.3 % | 45.6 GB |
| 10 | 47 % | 4.2 % | 32.7 GB | 45 % | 8.7 % | 35.8 GB |
| 15 | 49 % | 4.6 % | 21.6 GB | 60 % | 11.7 % | 26.3 GB |
| 20 | 50 % | 4.8 % | 10.7 GB | 69 % | 15.4 % | 16.6 GB |
| 21 | the 22nd was refused | | | 78 % | 16.4 % | 14.9 GB |
| 24 | | | | 94 % | 21.1 % | 8.5 GB |
| 25 | | | | refused | | |

- **The GPU takes the load off the CPU, but on this machine it did not raise the number of users.** With the GPU (sway on the HD 630, the VAAPI encoder, zero-copy capture) 21 users fit and the 22nd was refused (8789 MB
  available, 9613 MB needed: the daemon's rule of 15 % of the RAM). With the GPU off (pixman, OpenH264) 24 fit and the 25th was refused (9157 MB available). What ended both runs was memory.
- **Memory per user is higher with the GPU**: 2.24 GB against 1.95 GB (from the first to the 20th user), because an integrated GPU draws its buffers from the system memory. A card with its own memory would not do this.
- **CPU stays flat with the GPU and does not without**: from 15 users on the GPU run stayed near 50 % (the average of users 16 to 20: 49 %) while the software run climbed 3 to 4 points per user (65 % on average for users
  16 to 20, 94 % at 24). The capture and encoding share is about 70 % lower: 4.8 % against 15.4 % at 20 users. With the memory of a bigger machine the GPU run had CPU for more users; that is an estimate, it was not measured.
- **Nothing failed in either run** (no error in the journal, no frame error, no out-of-memory kill). The probe median was 21 ms with the GPU (largest 45 ms) and 9 ms without (largest 33 ms); the GPU is shared by all sessions, which
  may be why it answers a little later, but this was not looked into.
- **How busy the GPU was**, from a 90-second sample with all 21 users active: its render engine 15 % busy, its video engine 25 %, at about 690 MHz, 18 W for the whole package (the CPU 45 % in that window). (The sampler lost the GPU columns for
  users 14 to 21 during the run, a bug of the sampler: its output pipe filled up with many clients; the table keeps the CPU and memory figures, which were not affected, and the row of user 21 of the GPU run is missing because the
  connection of the controlling machine dropped during it.)
- **A lighter first run**, the same users doing only the browsing page, the typing terminal and the video, with the clients on the same machine: 20 users fit with the GPU on before the CPU reached 98 % (the clients themselves
  took 36 % of the machine at 20 users, which is why the later runs put them elsewhere). `docs/measure-data/gpu-i7-7700-light-clients-local.tsv`.

### Full-screen WebGL, one page per user
Each user has a Chromium window full screen on a WebGL shader that draws noise at every pixel (the heaviest picture to draw and to encode), the frame rate being what the page itself reports (`fps N` in its title).

| Users | frame rate of each page | median | GPU (render engine) | CPU |
|---|---|---|---|---|
| 1 | 60 | 60 | 53 % | 23 % |
| 2 | 60, 60 | 60 | 93 % | 34 % |
| 3 | 50, 47, 36 | 47 | 100 % | 32 % |
| 4 | 27, 33, 33, 37 | 33 | 100 % | 34 % |
| 6 | 27, 17, 23, 22, 19, 15 | 20.5 | 100 % | 36 % |
| 8 | between 11 and 20 | 17 | 100 % | 37 % |
| 10 | between 6 and 19 | 13 | 100 % | 39 % |
| 12 | between 10 and 24 | 17 | 100 % | 39 % |
| GPU off, 1 or 2 | the page does not animate (0 fps) | 0 | 0 % | 14 to 15 % |

- **The HD 630 gives 60 fps to two full-screen WebGL users, and then the render engine is the limit**: it is at 100 % from the third user on, the frames are shared about evenly (each of ten users gets 6 to 19 fps), and the CPU
  stays low (39 % with twelve). This is a deliberately heavy page; a page that draws less shares among more users.
- **Without the GPU the page does not animate at all** in the browser as the desktop starts it (`--disable-gpu`, where there is no usable render node).

### Applications, with and without the GPU
Started in a session, 35 seconds each, as the account (Zed 1.25 from Flathub; Blender, Kdenlive, darktable, `vkcube` and `glxgears` from Debian). "Opens" means a window appeared and the program kept running (for Blender, whose process my check did not match by name, only the window).

| | with the GPU | with the GPU off |
|---|---|---|
| Zed, Blender, Kdenlive, darktable | open | open |
| `glxgears` (OpenGL) | opens, the card 17 % busy | opens, on the CPU (0 % on the card) |
| `vkcube` (Vulkan) | opens, the card 21 % busy | **fails**: `demo_init_vk_swapchain: Assertion '!err' failed` |

With a render node present and the GPU switched off, the software compositor takes no GPU buffers and a Vulkan application cannot present its window; everything that uses OpenGL falls back to the CPU. **The failure the site used to list for Zed ("Failed to create surface") did not reproduce** with Zed 1.25 in either mode.

### A server with no render node at all (2026-10-28)
A cloud server with no `/dev/dri` (Debian 13, 4 vCPU, 8 GB), one session, a local `sdl-freerdp3` client, Mesa's software drivers (`llvmpipe`, and lavapipe for Vulkan):

| | result |
|---|---|
| `glxgears` (OpenGL) | opens |
| `vkcube --wsi wayland` (Vulkan) | **opens** (the `vkcube` failure above only happened with a render node present and the GPU switched off) |
| Zed 1.25 | opens (its window appears); no error |
| Chromium, WebGL page, as Waydesk starts it | does not animate (the page's title never shows a frame rate): `/etc/chromium.d/waydesk` adds `--disable-gpu` |
| Chromium with `--use-angle=swiftshader --enable-unsafe-swiftshader` | 8 fps, the browser using about 310 % CPU of 400 % |

"Opens" is a window and a running process, looked at through sway, not at the picture. Blender, Kdenlive, darktable, VS Code and LibreOffice were tried only on the machine with the Intel device, with the GPU on and off, not here. These applications were opened, not used: how fast they are with and
without the GPU was not measured.

### When the GPU cannot encode
The desktop and the capture on the GPU, but `LIBVA_DRIVER_NAME` pointing at a driver that does not exist (what a machine looks like when VAAPI has no encoder, as NVIDIA's before NVENC), one user on the full-screen WebGL page:
the session starts, the client stays connected, the page runs at 62 fps, and the log says `VAAPI encoder not available (...): OpenH264 on the CPU`. The session worker then takes **12.3 % of the machine, against 2.9 % with VAAPI** (OpenH264
at about 42 ms a frame, the picture read back from the GPU). `waydesk-admin status` now shows this fallback per session.

### An NVIDIA card (RTX 4000 SFF Ada, driver 615)
A Hetzner auction server with an Intel Core i5-13500 (and its UHD 770) and the RTX 4000 SFF Ada, 62 GB, about 0.38 euros an hour. Details in `docs/GPU-PLAN.md`. In short: with Debian's driver (550) a headless sway cannot allocate its buffers on the card;
with NVIDIA's own 615.71.10 it can, and a full session runs on the card. The VAAPI encoder does not exist on NVIDIA, so there is an NVENC one (`src/encode_nv.rs`): for one user on the full-screen WebGL page, **61 fps, 4 to 9 ms
an encoded frame, the card's encoder engine 4 % busy, the session worker 8 % of a core** (13 % with OpenH264). `ffmpeg h264_nvenc` encodes a 1680x1050 test pattern at 527 fps; the Intel iGPU of the same machine through VAAPI at 446
fps. The stream was looked at in the Windows App on a Mac and in mstsc on Windows 11 (ARM, in a VM): no washed-out picture, sharp text, smooth dragging. A full-screen noise page ends up coarse on a slow link, as designed (the quantiser
rises to 40 and is back at 18 within 20 seconds of the noise stopping). Several users on one card were not measured.

### Not measured
Other GPUs than these two (AMD through VAAPI should work and was not tried; Intel Arc was not seen, and the UHD 770 only in a short test); a card with its own memory under many users; how fast applications are with and without the GPU, apart from
WebGL; the delay between a click and its picture and the frame rate each user really receives (the page's own frame rate was read, not what arrived at the client); runs longer than the 45 minutes each ramp took; clients far away
over a slow link with many users (a link of 4 Mbit/s was met by accident and the encoder followed it down to 1 Mbit/s); more than one data centre.
