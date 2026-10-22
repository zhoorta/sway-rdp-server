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
patched sway; RDP on a WireGuard address (`docs/WIREGUARD.md`), reached from a Mac in Portugal (ping 77 ms). `tester1` came in with the Windows App (1680x1050);
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
- Trap while measuring: logging in again right after `loginctl terminate-user` can time out on `user@UID.service` (90 s); wait until the old user manager has stopped.

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
