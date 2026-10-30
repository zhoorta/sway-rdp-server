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

## Not verified

- Resizing a live session and changing its scale from the client (the Lua is tested as text, not against the running compositor from a client).
- Pointer clicks and drags, clipboard, audio, three or more accounts, real applications, the systemd units (`waydesk-session@`).
- The Waydesk desktop (shell, launcher, title-bar buttons) is sway-specific and is not part of a Hyprland session: the account gets
  whatever its own Hyprland configuration (`~/.config/hypr/hyprland.lua`) gives it.
- Minimizing: the patched sway's scratchpad minimize does not exist here.
- Security: `LIBSEAT_BACKEND=noop` means no seat management; the GPU's card is opened with the account's own permissions (groups `video` and
  `render`), so every account with a session has direct access to the GPU. That needs a review before it is used for accounts that do not trust
  each other (see `docs/THREAT-MODEL.md`).
- Hyprland is started without `start-hyprland` (its watchdog), so a crash is not restarted by it; the worker ends and the client reconnects.
