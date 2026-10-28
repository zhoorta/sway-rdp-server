# Changelog

What changed, newest first. The numbered releases are the ones in the apt repository.

## 2026-10-28 · 0.4.0

**A GPU, when there is one.** A session draws its desktop on the machine's GPU (a render node in `/dev/dri`) and encodes the H.264 picture with the chip's encoder through VAAPI; without a GPU, or when anything about it fails, it is software as before. On an Intel HD 630 (i7-7700, Debian 13): the OpenGL benchmark goes from 167 to 2840 points, the session's CPU for a full-screen animation from 107 % of a core to 21 %, and Chromium gets WebGL (it ran with `--disable-gpu` before, which also meant no WebGL at all). `WAYDESK_GPU=off` and `WAYDESK_ENCODER=software` in `/etc/default/waydesk` switch it off; the accounts get the render node through a udev rule for `waydesk-users` (`acl` is a new dependency, as are `libva2` and `libva-drm2`). The VAAPI encoder is `cros-codecs` (BSD-3, ChromiumOS). Notes and numbers in `docs/GPU-PLAN.md`.

**No copy to the CPU.** The compositor draws the capture into GPU buffers (dmabuf) and the GPU's video processor converts them to the encoder's format, so the picture no longer passes through the CPU: the session's CPU for the full-screen animation goes from about 21 % of a core to 2 to 3 %. `WAYDESK_DMABUF=0` goes back to the copy through shared memory.

**The quantiser moves without a new stream.** A quantiser change (the encoder follows the link) now only changes a number in the next picture's header; before, it sent new parameter sets and a new keyframe, and several in a row left the Windows App on a Mac showing the whole screen washed out until the next keyframe. The output buffer of the GPU encoder is now as large as a raw picture: a keyframe of full-screen noise at a low quantiser (1.8 MB) did not fit the fixed 1.5 MB and ended the connection. `cros-codecs` and `cros-libva` are vendored with small patches (`patches/*.diff`).

**A GPU that fails does not leave you without a desktop.** If a session cannot be made on the GPU (Debian's NVIDIA driver 550 lets sway start and then gives it no buffers: the first capture failed and the account got nothing), the session is made again once, with sway in software, and the log says so. On a machine with several NVIDIA cards the NVENC encoder runs on the card the session draws on (`WAYDESK_NVENC_DEVICE` chooses another).

**Windows may hang over the sides.** A window can be dragged past the left and right edges of the screen, as it already could past the bottom, as long as 120 pixels of it stay on the screen to grab it by (the title bar stays inside at the top, and above the dock at the bottom); before, the sides pulled the whole window back in. Super+Shift+Up (`waydesk-windows fit`) still brings every window entirely back.

**What runs, in `waydesk-admin status`.** The GPUs of the machine (model, driver, how busy: NVIDIA's card and its encoder through NVML, AMD from sysfs, Intel by the time out of its idle state), what new sessions will use (`WAYDESK_GPU`), and for every session whether it renders on a GPU or on the CPU and which encoder it runs, with its quantiser and frame rate: `vaapi q18 12fps`, `nvenc q25 17fps`, `openh264 9fps`. A GPU encoder that cannot start falls back to OpenH264 on the CPU, which used to show only as a line in the journal. `waydesk-admin status --watch [SECONDS]` keeps it on the screen, and `--json --watch` writes one line of JSON per round. A session says what it runs in `/run/waydesk/status/UID.json`, readable by its account and by root.

**NVIDIA (first version).** On a machine with an NVIDIA card the session can render on it (needs NVIDIA's driver 615 or newer, not Debian's 550, which cannot allocate the compositor's buffers) and encode with NVENC, for which `WAYDESK_ENCODER=auto` falls back when VAAPI has no encoder. Tested on an RTX 4000 SFF Ada; the NVIDIA libraries are loaded only when there is an NVIDIA card, so nothing changes elsewhere. The capture takes the 24-bit pixels NVIDIA's GL renderer reads back. Notes in `docs/GPU-PLAN.md`.

## 2026-10-26 · 0.3.1

**Through an SSH tunnel.** The flow control looks at the socket's send queue: while more than about 190 kB is queued the next frame waits, and while the queue is deep the capacity is how fast the peer acknowledges (the kernel's delivery rate is the first hop then, the loopback to `sshd`, which says nothing about the real link). Before, an SSH tunnel in front of a slow link let 3 MB build up, which is seconds between a click and its picture, while the encoder aimed for 10 to 40 Mbit/s. The log shows the queue next to the bitrate.

A browser by default: `waydesk-desktop` installs Chromium from Flathub (about 190 MB) where the archive has no Chromium package, which is Ubuntu: its recommend of `chromium` is now versioned, so that Ubuntu's stub (it installs the snap, which the shell neither tunes nor pins) is not taken for it. Debian keeps the Chromium deb. The browser is pinned first in the dock.

`waydesk-desktop` also adds the Flathub remote when it is installed (it needs the network; without it the install goes on and says how to add it later). Before, the Explore page of the Software app was empty on a fresh install, because there was no remote to read apps from.

`waydesk-setup` finds a program that a vendor's script has just installed in the home directory (`~/.opencode/bin`, `~/.local/bin`, `~/.cargo/bin`, `~/.bun/bin`): before, OpenCode got no launcher entry ("not installed", because the folder is only in `.bashrc`) and was installed again at every run, and the launcher entries of such programs use the full path. `waydesk-setup` runs as the account that typed it, also under `sudo waydesk-setup`, and refuses to run as plain root: Claude Code, OpenCode, mise, uv and the other tools of the wizard install into the home of whoever runs it, and as root they ended up in `/root`. Run it as the administrator account, without sudo.

## 2026-10-26 · 0.3.0

**Keyboard.** A keyboard whose keys reach the server at other places than the layout of its locale expects gets a layout of its own: the new **Keyboard** window in the launcher (or `~/.config/waydesk/keyboard`, one xkb layout name) picks it, and the session follows within a second, with no reconnect. `waydeskmac` is for the built-in keyboard of a Portuguese MacBook through the Windows App; a layout of your own goes in `/etc/xkb/symbols`.

**Fewer things you did not ask for.** The file manager (`pcmanfm-qt` `+waydesk2`) no longer recommends `lxqt-qtplugin` and `lxqt-policykit`: they pulled in `lxqt-session` and with it about a hundred xscreensaver screensavers in the launcher (no icon, none of them can run in a session), `lxqt-config`, `qps`, `qlipper` and the Hibernate, Leave and Logout entries, about 60 MB. A server that already has them keeps them (the LXQt packages recommend each other, so `apt autoremove` alone does not remove them): `sudo apt remove lxqt-session lxqt-qtplugin lxqt-policykit lxqt-config lxqt-notificationd lxqt-powermanagement xdg-desktop-portal-lxqt xscreensaver xscreensaver-data xscreensaver-gl qps qlipper && sudo apt autoremove` does, and nothing of Waydesk or the file manager goes with them (checked on Ubuntu 26.04). `waydesk-setup` no longer offers the shell (it comes with `waydesk`), and every account gets the shell conveniences of `waydesk-setup` (aliases, fzf, zoxide, each only when installed) in its `.bashrc`, once.

**A desktop that starts ready.** The dock starts with a browser, the file manager and the terminal pinned (each once, when it is installed), and the Chromium icon is no longer put on the desktop, which keeps Home and Trash. A shortcut added with `waydesk-add-to-desktop` is marked trusted with the right key (`metadata::trusted`), so the file manager shows no "!" on it. Chromium no longer shows "Continue where you left off" on a new profile.

**Smaller.** `waydesk-server --version` and `waydesk-admin --version`; a frame that the flow control holds back is a debug line, not a warning in the log.

## 2026-10-26 · 0.2.1

`waydesk-desktop` depends on `waydesk-shell`: `apt install waydesk` gives the desktop with its shell, bar and dock at the first login (it was a bare sway until `waydesk-setup` had installed the shell). `waydesk-setup` is for the apps only. Everything else is as in 0.2.0.

## 2026-10-26 · 0.2.0

Who can reach the port is set with one command each: `waydesk-admin access` (this machine, a local network or a VPN; the daemon closes source addresses that are not allowed before TLS and before any login), `cert local-ca` (a certificate that clients can verify), `vpn init` and `vpn add` (WireGuard), and `waydesk-setup-admin` asks how you will connect. X11 applications run (XWayland, one server per account). The mouse pointer has its real shapes (text cursor, resize arrows, hand), also for applications that draw their own cursor. TCP BBR for the connection, and flow control that follows the link: frames and bytes in flight, and a video bitrate that follows the best rate the link delivered in the last 10 seconds (it aims for 80 % of it: on a lossy 5 Mbit/s link it went from 1-2.7 to 3.4-4.7 Mbit/s). A keyboard that sends its keys to other places than the layout of its locale expects gets its own layout per account (`~/.config/waydesk/keyboard`; `waydeskmac` for a Portuguese MacBook keyboard). The H.264 encoder runs at low complexity by default (15-36 % less CPU under motion), with environment overrides and `waydesk-encbench` to measure them. Title bars and borders in the colours of the theme (dark or light), window buttons as a capsule, soft shadows and rounded title-bar corners. The dock floats over the bottom of the screen and bounces while an application starts. Flatpak and GTK applications no longer wait minutes for a dead portal after a login.

## 2026-10-24 · 0.1.0

First numbered release: `waydesk-server`, `waydesk-desktop` and `waydesk` 0.1.0 for Debian 13 and Ubuntu 26.04, amd64 and arm64. `waydesk-repo` 1.3 moves servers that still use the old repository address to waydesk.dev. The source of the copyleft programs in the repository (matugen, quickshell, pcmanfm-qt) is offered at [/apt/source/](/apt/source/).

## 2026-10-24

Ubuntu 26.04 checked from the repository; `dms.sh` no longer adds Debian backports when the Waydesk repository is configured (it broke `apt update` on Ubuntu); a client that stops reading is dropped after 60 s; `waydesk-server` alone starts a session with a minimal sway config; the Flathub Chromium opens without its terms window and keyring prompt; seven heavy users fit on 8 GB.

## 2026-10-23

Two suites, `trixie` and `resolute`, from one pool; private sway and quickshell under `/usr/lib/waydesk`; Flathub fallback for browsers; heavy-user capacity on Debian; fix for a publishing tool that deleted live packages.

## 2026-10-22

Signed apt repository, `waydesk-repo` and `waydesk-setup-admin`; our own build of the shell as a package; refused logins tell the client why.

## 2026-10-21

Memory admission, account ceilings and zram; patched sway with title-bar buttons; dock; Chromium as the default browser; third internal security pass.

## 2026-10-20

Renamed to Waydesk. Desktop icons, themed Qt and Flatpak apps, logind session per account.

## 2026-10-15 to 19

RDP server for sway, then multi-user daemon, helper and sessions; DankMaterialShell on sway.
