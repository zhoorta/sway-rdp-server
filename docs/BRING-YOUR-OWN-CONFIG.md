# Bring your own configuration (2026-10-30)

An account can run its own sway or Hyprland setup in a Waydesk session: the dotfiles in its home are used. This page says what Waydesk needs from a compositor, which
configuration is picked, how an administrator chooses the compositor per account, what `waydesk-admin config check` looks at, and what went wrong with real setups.

## What Waydesk needs

- **Capture and input** through Wayland protocols: `wlr-screencopy` (the picture), `zwp_virtual_keyboard` and `wlr-virtual-pointer` (input). sway and Hyprland have them.
- **An output of its own.** The session makes one headless output per client, named `HEADLESS-N`, and sizes it to the client's window. So rules about physical outputs do nothing, and a
  rule that disables *every* output, or moves everything off unknown ones, empties the session's.
- **Control of that output** through `swaymsg` (sway) or `hyprctl` and Lua (`hyprctl eval`, Hyprland 0.55 or later).
- `foot` for the first terminal of a session (start the sessions with `--no-terminal` to go without).
- For Hyprland, **a GPU** the account can open, both the render node and the display node ([HYPRLAND.md](HYPRLAND.md), Security notes). sway runs without one.

## Which desktop an account gets

`WAYDESK_COMPOSITOR=sway|hyprland` in `/etc/default/waydesk` is the server's setting (sway by default). An administrator can give one account another:

    waydesk-admin users set alice --desktop hyprland     # or sway; `default` removes it
    waydesk-admin users list                             # a star after the desktop: set for this account

The setting is a file, `/etc/default/waydesk.d/UID.env`, that the session unit reads after the server's file. A running session keeps its compositor until it is restarted
(`waydesk-admin sessions end NAME`); `users list` says so. Several accounts with different compositors run side by side (tried: one on sway with the Waydesk desktop and one on Hyprland
with ML4W, at the same time).

## Which configuration is used

**sway.** The first of these that exists: `~/.config/waydesk/config` (the account's file for Waydesk sessions), `/etc/xdg/waydesk/sway.conf` (the Waydesk desktop package), then sway's own
lookup (`~/.sway/config`, `~/.config/sway/config`, `/etc/sway/config`), and the package's minimal configuration if there is nothing at all. **With the Waydesk desktop package installed the
account's own `~/.config/sway/config` is therefore not used**; put it (or a link to it) at `~/.config/waydesk/config`, or leave the desktop package out.

**Hyprland.** The account's `~/.config/hypr/hyprland.lua`. Waydesk starts Hyprland with a small wrapper (`/usr/lib/waydesk/hyprland.lua`) that loads that file unchanged, as Hyprland would
have, and then adds three things: Waydesk's `bin` first on `PATH`, the size rule of the session's output, and a marker that tells the session the configuration has loaded. The old
`hyprland.conf` format is not supported. A configuration that fails to load does not stop the session: it starts without it and the error is in the session's log.

## Stand-ins

Some programs of well-known setups would end the session or ask for rights an account does not have. Waydesk puts programs of the same name first on the session's `PATH`
(`/usr/lib/waydesk/bin`):

| Program | What the stand-in does |
|---|---|
| `omarchy-system-logout` | closes the windows and exits Hyprland (Omarchy's `uwsm stop` has no session to stop) |
| `omarchy-system-reboot`, `omarchy-system-shutdown` | say "disabled on this server" (administrators get the real one) |
| `omarchy-theme-set-browser-policy` | does nothing (the real one writes a system-wide browser policy as root) |
| `hyprshutdown` | for a power-off or reboot command (ML4W's Restart and Shutdown) says so and leaves the session alone; logout goes to the real one |

Reboot, shutdown and suspend are refused for ordinary accounts by a polkit rule whatever program asks.

## The check

    waydesk-admin config check NAME [--json]       (as root; the account's files are read as the account, never as root)

It reports OK, INFO, WARN and FAIL lines with a hint each, and exits with status 1 when something is a FAIL. It looks at:

- the account (may it log in, does it have a runtime directory), the desktop it gets, `foot`;
- **sway:** which configuration is used, and when the account's own is ignored what *would* be in it (outputs disabled, workspaces pinned to outputs this server does not have,
  `swayidle` timers, a screen locker, power commands in bindings), following plain `include` lines;
- **Hyprland:** the version, the configuration format, Hyprland's own `--verify-config` on the account's file (it caught a failed `require` that left a black screen), the programs the
  configuration starts (and which are not installed, counting the account's `~/.local/bin`), `hypridle` timers, power commands, and whether the account can open the GPU's render node
  and display node.

Not built yet: a `--try` that starts a real short session to prove the configuration works, and profiles by group.

## What went wrong with real setups

- **ML4W: relative `require`s failed** because the wrapper became Hyprland's main configuration file and Hyprland looks for them next to it. Fixed in the wrapper; the check's
  `--verify-config` run would have shown it.
- **Idle management reaches a remote session.** `hypridle` (ML4W: lock at 10 min, display off at 11, suspend at 30) and `swayidle` act on a session nobody may be looking at. Display off
  turns the client's picture black; a lock screen can be unlocked by typing the account's password in the RDP client.
- **Restart and Shutdown ended the session** before the system refused them (`hyprshutdown`; Omarchy's menu announced a reboot and failed two seconds later). The stand-ins above.
- **Omarchy's theme switch asked for an administrator password** (it writes a shared browser policy). The stand-in above.
- Omarchy's own configuration puts its `bin` first on `PATH`, behind which a stand-in is never found; the wrapper puts Waydesk's directory back in front.

Tried and working: Omarchy 4.0.6 and ML4W 2.16.1 on Hyprland 0.57, the default configurations of sway on Arch, Debian and Ubuntu, and the Waydesk desktop. Not tried: a sway setup
with Waybar and a launcher, other Hyprland dotfiles, other desktops (GNOME, KDE, COSMIC).
