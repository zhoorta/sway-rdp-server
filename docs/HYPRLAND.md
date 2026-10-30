# Hyprland sessions (experimental, 2026-10-30)

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

- Audio (the per-user daemon mode has none; it needs the root helper), the mouse wheel, three or more accounts, real applications, the systemd units
  (`waydesk-session@`).
- The pointer: without the `waydesk-blank` cursor theme (the Waydesk package installs it) Hyprland draws its own cursor into the picture; the Arch package needs it too.
- The Waydesk desktop (shell, launcher, title-bar buttons) is sway-specific and is not part of a Hyprland session: the account gets
  whatever its own Hyprland configuration (`~/.config/hypr/hyprland.lua`) gives it.
- Minimizing: the patched sway's scratchpad minimize does not exist here.
- Security: `LIBSEAT_BACKEND=noop` means no seat management; the GPU's card is opened with the account's own permissions (groups `video` and
  `render`), so every account with a session has direct access to the GPU. That needs a review before it is used for accounts that do not trust
  each other (see `docs/THREAT-MODEL.md`).
- Hyprland is started without `start-hyprland` (its watchdog), so a crash is not restarted by it; the worker ends and the client reconnects.
