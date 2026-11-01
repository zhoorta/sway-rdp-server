# Hyprland sessions (tested on Arch with an Intel GPU, 2026-11-01)

Waydesk can run each account's desktop on Hyprland instead of sway, for example the Omarchy desktop on Arch. It is off by default and
needs a GPU. This records how it works, what was verified and what was not.

## Turning it on

`WAYDESK_COMPOSITOR=hyprland` in `/etc/default/waydesk`, or `--compositor hyprland` on the daemon's command line (the daemon passes it on to
every session process). `sway` is the default. Hyprland 0.55 or later is needed: it is configured and driven with Lua, and Waydesk sends it Lua
(`hyprctl eval`).

## What a session is

One Hyprland per account, started as that account, on the GPU's card only (`AQ_DRM_DEVICES`, `LIBSEAT_BACKEND=noop`). It has no output
of its own. When a client connects the session makes a headless output with `hyprctl output create headless`, gives it the client's size
and scale (`hl.monitor`), captures it with wlr-screencopy and injects input with the virtual keyboard and pointer, as with sway. The
compositor-specific calls are in `src/compositor.rs` (the `Compositor` trait; `hyprland` and `sway` implement it), the start in
`OwnCompositor::start_hyprland` in `src/worker.rs`.

Nothing is set up as root per account: with no display connected to the GPU there is no exclusive access to fight over, so several accounts
run side by side on one GPU (two were tried at once, each with its own output size).

## As a service (checked on Arch, 2026-10-30)

With `WAYDESK_COMPOSITOR=hyprland` in `/etc/default/waydesk` the packaged units work unchanged (`waydesk-helper`, `waydesk`, `waydesk-session@UID`; the
session unit's `--own-sway` only means "start the compositor of this session"). A login through the real helper and daemon started the session unit,
Hyprland ran inside it, a terminal opened on the `Remote` workspace, and the session speaker reached the client as PCM blocks over the sound channel.
What a package has to provide for it:

- **The GPU's display node.** `packaging/udev/70-waydesk-gpu.rules` gives `waydesk-users` only the render node, on purpose. Hyprland opens `card*` too:
  `packaging/udev/71-waydesk-hyprland-card.rules` does that, and is not installed by default (see Security below).
- **PAM.** Debian's `packaging/pam-waydesk-server` includes `common-auth`, which Arch lacks: use `packaging/arch/pam-waydesk-server`. The session unit's
  `waydesk-session` service needs an `account` line (added: `pam_permit`), or Arch refuses it (`status=224/PAM`, session never starts). It still logs a
  `pam_warn` for `setcred`; that is harmless and left alone (an `auth` line with `pam_permit` could let anything authenticate against that service).
- **A sound server in the account's session.** `pipewire`, `pipewire-pulse` and `wireplumber` enabled as user units (`systemctl --user enable --now
  pipewire.socket pipewire-pulse.socket wireplumber.service`, or a preset), and `libpulse` for `parec` and `pactl`. A session that starts without it has no sound
  until the next one.
- **Omarchy's theme switch.** It writes the theme colour into the system-wide managed policy of Chromium, Chrome, Edge and Brave as root; only `wheel` gets that without
  a password, anyone else gets a polkit dialog ("Authorize running ...") for a password they do not have, and one account's theme would recolour every account's browser.
  `packaging/hyprland/bin/omarchy-theme-set-browser-policy` does nothing instead; installed in `/usr/lib/waydesk/bin`, which is first on the session's `PATH`, it
  affects Waydesk sessions only (a session that was already running keeps asking until it is restarted). The theme itself (bar, terminal, wallpaper) still changes.
  **That was not enough** (found 2026-11-01): Omarchy's own first start (`omarchy-provision-first-run`, then `install/user/theme.sh`, then `omarchy-theme-set`) calls the real helper, because its install scripts put
  `$OMARCHY_PATH/bin` first on their own `PATH`, and it waited on the password dialog for good: the account never got a theme (no Style or Background menu, an include error for `foot.ini` in the terminal),
  and sessions of other accounts sat on the dialog for hours. `packaging/polkit/50-waydesk-no-power.rules` now also answers no, at once, to `pkexec /usr/bin/omarchy-theme-set-browser-policy` for members of
  `waydesk-users` who are not in `wheel` (both packages). Checked on a new account: the first start sets Omarchy's default theme by itself, with no dialog.
- **The stand-ins need the right `PATH`.** Omarchy's Hyprland configuration (`default/hypr/envs.lua`) puts `$OMARCHY_PATH/bin` first on `PATH` for everything Hyprland starts, so
  the stand-ins below, in `/usr/lib/waydesk/bin`, were never reached from the menu (found when logout did nothing; a first test had looked at Hyprland's own `PATH`, not the menu's).
  Waydesk therefore starts Hyprland with `--config /usr/lib/waydesk/hyprland.lua` (`packaging/hyprland/waydesk.lua`), which loads the account's own configuration unchanged and
  then puts `/usr/lib/waydesk/bin` back in front. Only used when the account has `~/.config/hypr/hyprland.lua` to load. Checked through the real services: the menu process's
  `PATH` starts with Waydesk's directory, Omarchy's other commands still come from Omarchy's, and logout ended the session.
- **Logout.** Omarchy logs out with `uwsm stop`, which stops a session that its session manager started; Waydesk starts Hyprland itself, so the menu entry did
  nothing. `packaging/hyprland/bin/omarchy-system-logout` (also in `/usr/lib/waydesk/bin`) closes the windows as Omarchy's does and then exits Hyprland: the session
  worker sees that, ends the session and the RDP client is disconnected (tried: windows closed, unit stopped, client gone). Disconnecting without logging out still
  leaves the session running, as before.
- **Reboot, shutdown, suspend.** The server is shared. An ordinary account was already refused (logind answers "challenge": an administrator's authentication, and says that other users are
  logged in), but got a password dialog that a known administrator password would satisfy, and Omarchy's menu announced "Rebooting" and failed two seconds later.
  `packaging/polkit/50-waydesk-no-power.rules` now answers no, at once, to reboot, power-off, halt, suspend, hibernate and hybrid-sleep for members of `waydesk-users` who are
  not in `wheel` (checked: `alice` and `bob` get `no`, an account outside the group and an administrator in `wheel` are unchanged, `systemctl reboot` as `alice` says
  "Access denied"). It is not Hyprland-specific and is in both packages. The menu entries are still listed; `omarchy-system-reboot` and `omarchy-system-shutdown` in
  `/usr/lib/waydesk/bin` say "disabled on this server" instead of failing, and hand over to Omarchy's own command for `wheel`. Not run: that last path (it would restart the
  server). To hide the entries, an account can add `"system.reboot": {"label":"Reboot","when":"false"}` (same for shutdown and suspend) to
  `~/.config/omarchy/extensions/omarchy-menu.jsonc`; the server does not edit that file.
- The `waydesk-blank` cursor theme (from `packaging/cursors`), or Hyprland draws its own cursor into the picture.

## Why a GPU is needed

Hyprland has no software renderer and no headless-only mode: on a machine without a GPU its backend does not start (`CBackend::create()
failed`, or "no renderer"). A virtual display device (`vkms`) does not help, because frames cannot be imported without a GPU that renders.
Nesting it in a wlroots compositor fails too (it wants `xdg_wm_base` version 6, sway offers 5). So: a render node the account can open
(`WAYDESK_GPU`), or no session.

## Verified (Arch, Intel HD 630, Hyprland 0.57, Omarchy 4.0.6)

- A session through the daemon's `--auth pam --per-user` mode: Hyprland started as the account, output made at the client's size, the first
  terminal opened on the workspace, frames sent (VAAPI H.264 on the Intel GPU), keyboard input arrived, and on disconnect the output was removed.
- Size and scale of a headless output set through `hl.monitor` (1280x720, 1600x900 at 1.5, 1024x768), and a capture has exactly that size.
- Workspace focus and move, `hl.exec_cmd`, closing the windows of an output, `hyprctl setcursor`, `output remove`.
- Two accounts, each with their own Hyprland and output, at the same time.
- Pointer: a click reaches the window under the pointer (4 of 4), and a drag across a word in the terminal selects exactly that word. Clipboard text goes both
  ways (client to session and back). Testing trap: FreeRDP under Xvfb, driven with `xdotool`, reports a pointer move one step late (the server receives the
  position from before); nothing is wrong with the compositor. Follow a move with `mousemove_relative -- 1 0` and back to get the true position.
- Live resize and scale from a client (FreeRDP with dynamic resolution, 1280x720 to 1600x900, 1024x700, 1400x800, at 100% and 200%): the output and the
  client's picture follow every size, as on sway. `hyprctl eval` answers before the mode is in force, so `init_output` and `set_mode` wait until Hyprland
  reports the size; without that the capture started on the old size and the session ended up with a picture that did not match the output.

## Not verified

- Audio in the per-user daemon mode: it has none by design (it needs the root helper). Sound through the helper, and real applications under Omarchy (tried by hand from the Windows App on macOS, 2026-11-01: they worked), were checked since this list was written, with: the mouse wheel
  (2026-11-01, Windows App on macOS), nine sessions at once (below), twelve sessions of four kinds at once (below), the systemd units (above), sound through the helper (above).
- The pointer: without the `waydesk-blank` cursor theme (the Waydesk package installs it) Hyprland draws its own cursor into the picture; the Arch package needs it too.
- The Waydesk desktop (shell, launcher, title-bar buttons) is sway-specific and is not part of a Hyprland session: the account gets
  whatever its own Hyprland configuration (`~/.config/hypr/hyprland.lua`) gives it.
- Minimizing: the patched sway's scratchpad minimize does not exist here.
- Hyprland is started without `start-hyprland` (its watchdog), so a crash is not restarted by it; the worker ends and the client reconnects.

## Security notes (reviewed 2026-10-30, Intel GPU, kernel 7.2)

`LIBSEAT_BACKEND=noop` means no seat management: nothing hands out or takes back devices, and a session opens the GPU's card with the account's own
permissions (here an ACL for `waydesk-users`, `packaging/udev/71-waydesk-hyprland-card.rules`). For a headless server that is what is wanted (there is no
VT or login screen to switch to). What it costs, with what was measured and what was only reasoned:

**Measured**

- Hyprland cannot start on the render node alone (`CBackend::create() failed`), so the card node cannot be left out.
- Two accounts opened the card at once. The first opener became DRM master. The second could not open one of the first's GPU buffers by its global name
  (`GEM_OPEN` and `GEM_FLINK` both gave `Permission denied`: they need the master's authentication), in either order. The old trick of reading another
  client's buffer through a primary node by guessing its name therefore does not work between accounts. (Test: a small program using the i915 ioctls.)
- Both Hyprlands ran side by side with only the first one master: with no display connected the second does not need it.

**Reasoned, not tested**

- The first account to open the card holds DRM master: it could change the mode of, or blank, a display attached to that GPU, and read its connector and
  EDID data. With no display attached to the GPU (a headless server, or a display on the BMC's own chip) there is nothing to change. A server with a monitor on
  the iGPU should not use this rule.
- GPU command submission and buffer sharing through file descriptors are the same as through the render node that sessions already get, so the card node
  adds the master's abilities, not new ways to reach another account's data. A GPU hang or heavy load by one account affects all (as with the render node).
- Accounts of one session group trust each other no more than before; nothing here changes the Unix permissions between accounts.

Use it on a GPU server without displays, install the card rule only there, and keep sway for everything else.

## Checked again with the package (2026-11-01, i7-7700 with an HD 630, 62 GB, Arch 7.2.11, `waydesk-server` 0.5.0-1 installed from the repository)

The services and the polkit socket came up by themselves after a reboot. An Omarchy 4.0.6 account (Hyprland 0.57) was used from the Windows App on macOS: the bar and notifications, clipboard both ways, live resize, the mouse wheel, Logout (ends the session and disconnects the client) all worked; reboot and shutdown were refused by the system (`Access denied`), and the stand-ins' `PATH` order held. Two Omarchy accounts ran together with ten other sessions (below). Found: the account's Omarchy theme was never set (see BRING-YOUR-OWN-CONFIG.md), and the scale cannot be changed from Omarchy's display settings.

Twelve sessions at once, all on the one GPU with the VAAPI encoder: four sway, three KDE Plasma, three COSMIC and two Omarchy. All were drawing, each account's compositor ran as that account, the log had no warnings, and 53 GB of 62 GB were still free. The sessions were idle apart from what the test clients did at login; this is not a load test.

## Several accounts (measured 2026-10-30, Intel HD 630, 4 cores / 8 threads, 62 GB, Arch, Hyprland 0.57)

Nine sessions at once (eight test accounts and one in real use), all on the one GPU with the VAAPI encoder. Memory: about 0.9 GB for an idle Omarchy session (Quickshell
480 MB, Hyprland 260 MB, the portals 190 MB), 1.1 to 1.4 GB with a busy terminal, 2.4 GB for the account in real use with a browser; 14 GB in all. Idle CPU, about 5 % of a core each;
a terminal printing text as fast as it can, about 30 %. Four such terminals gave about 20 fps and 1.7 Mbit/s each at 1280x720 and the GPU 82 % busy. The RDP clients ran on the same
machine (eight H.264 decoders), so the load average (14 on 8 threads) overstates what the server needs. Not found: the limit. The admission rule (a new session needs 9.4 GB
available) did not come into play.

Two things the test found, both fixed:

- **A session could stall for good when many started together.** Hyprland answers `hyprctl` before it has loaded its configuration. Omarchy's configuration has a rule for every
  monitor (`output = ""`, `mode = "preferred"`); when the load finished after the session had made its output, that rule put it back to 1920x1080, the capture saw the change
  and stopped, and the client's picture froze while the server went on. The wrapper configuration now writes `waydesk-hyprland-ready` when it has loaded, the worker waits for it,
  and the output's rule is kept in `waydesk-monitor.lua`, which the wrapper loads after the account's own, so a reload (a theme change runs one) keeps the size. Checked: eight sessions
  started within eight seconds all came up at 1280x720 with no stopped capture, and `hyprctl reload` left the size alone. A capture that stops because the compositor changed the output's size (or fails) is now restarted by the session: it adopts the new size and tells the client, as for a resize the client asked for (checked on
  Hyprland and on sway by changing the output's size from outside: the client window followed and the picture kept updating); a failed capture is tried three times, then the connection ends and the client reconnects.
- **70 % of a core per idle session went to a failing helper.** polkit's agent helper is not setuid any more; agents reach it through `polkit-agent-helper.socket`. On a machine
  where that socket is not running (here: polkit installed as a dependency and the machine not restarted since), Omarchy's shell tried to authenticate about 20 times a second, each
  time starting the helper, which said "needs to be setuid root". Starting the socket brought an idle session from 73 % to 5 % of a core and the GPU from 66 % to 3 % busy. The Arch
  package depends on polkit and starts the socket when it is installed; the socket is started at boot anyway. (A `status` of an account's CPU counts the short-lived processes that
  are gone by the time you look, which is why `top` showed only 30 %.)

## Other people's Hyprland setups: ML4W (tried 2026-10-30)

ML4W (My Linux for Work, dotfiles for Hyprland, version 2.16.1: its configuration is Lua since 2.13) was installed for one account and run as a Waydesk session: its Welcome app, its top
bar and dock, the wallpaper daemon, the notification daemon and the launcher all came up, with the VAAPI encoder; about 1.0 GB for the session after 75 s (the shell, `qs`, is the
largest part). Packages: 84 names in ML4W's two lists, 49 already present, 34 installed, none missing from the Arch repositories. Its installer was run as the account with the
packages already installed (it asks for confirmation, and for `sudo` for its package step).

What it showed about running someone else's configuration:

- **The wrapper broke relative `require`s.** Hyprland looks for the `require`s of a Lua configuration next to the main configuration file; that is the wrapper (`/usr/lib/waydesk`), so
  ML4W's `require("functions")` failed, the wrapper stopped before it wrote its marker, and the client saw a black screen after 30 s. Omarchy had worked only because it sets its own
  `package.path`. The wrapper now adds `~/.config/hypr` to `package.path` first, loads the account's configuration with `pcall`, and leaves any error in `waydesk-config-error.txt`, which the
  session logs as a warning: a broken configuration gives a session without it, not a black screen.
- **Idle management reaches a remote session.** ML4W's `hypridle` locks the screen after 10 minutes without input, switches the display off after 11 and suspends after 30 (refused by the
  polkit rule). Measured on the session: **display off turns the client's picture completely black** (brightness 0.24 to 0.00) until it is switched on again; the **lock screen** (`hyprlock`) shows
  in the RDP picture and **typing the account password through the RDP client unlocks it**. Not measured: how long a reconnecting client has to wait, or whether its first mouse movement
  wakes a display that was switched off while nobody was connected. An administrator who does not want this edits `~/.config/hypr/hypridle.conf` of the account.
- **Two ML4W accounts at once** (ML4W 2.16.1 on the i7-7700, 2026-11-01): both sessions came up with the Welcome app, the bar, the dock and the wallpaper, on the VAAPI encoder, about 1.0 GB each, with no configuration error. `config check` warns, as before, that `hypridle.conf` switches the display off after 11 minutes. Not checked by hand: ML4W's settings app, its updater, the browser it expects, the AUR packages it offers.
- **ML4W's Restart and Shutdown logged the account out.** Its power menu, its keybind and its `shutdown` alias run `hyprshutdown -t 'Shutting down...' --post-cmd 'shutdown -P 0'` (or `reboot`): that
  closes the apps and exits Hyprland, which ends the Waydesk session, and only then asks the system to power off, which the polkit rule refuses. The server stayed up (checked: up since
  the boot of that day, `CanPowerOff` and `CanReboot` answer `no` for the account) but the session was gone. `packaging/hyprland/bin/hyprshutdown`, first on the session's `PATH`, says
  "Not available on this server" and leaves the session alone when the `--post-cmd` is a power-off, reboot, halt or `systemctl` command; a plain `hyprshutdown` (logout) and any other
  command go to the real one, and so does everything for an administrator (`wheel`). Checked with ML4W's own `ml4w-power`: poweroff and reboot left the same Hyprland running, logout still
  ended the session. The suspend entry (`systemctl suspend`) is refused by polkit without touching the session. It is generic: any Hyprland setup that shuts down through `hyprshutdown`
  gets the same behaviour.
