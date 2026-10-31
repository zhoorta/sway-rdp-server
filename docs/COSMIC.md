# COSMIC sessions

`waydesk-admin users set NAME --desktop cosmic` gives an account the COSMIC desktop (System76's, in Rust). Needs the COSMIC packages (Arch: the `cosmic` group; `start-cosmic`, `cosmic-comp` and the applets must be installed). Experimental.

## How it works

COSMIC's compositor, `cosmic-comp`, cannot be the session's own compositor on a server:
- On its DRM backend it exits with `Backend initialized without output` when no monitor is connected, and it has no virtual or headless output.
- It offers no `wlr-screencopy` and no virtual pointer (capture is `ext-image-copy-capture`, input goes through libei).

So the session stays what it always was, a headless sway with one output of the client's size, and its configuration (`/usr/share/waydesk/sway-cosmic.conf`) runs `start-cosmic`. `cosmic-session` starts `cosmic-comp`, which finds sway's display and runs as a window of it; the config makes that window fullscreen. Waydesk captures sway's output and sends the keyboard and pointer to sway as before, and they reach COSMIC as the events of a fullscreen window.

An account's own `~/.config/waydesk/config` replaces the package's file, as for sway.

## Cost and limits

- Two compositors per session: memory and render time are those of sway plus COSMIC's (see `docs/measure-data/` for the measured capacity).
- Not tried: a client that changes its window size during the session, the clipboard between the client and COSMIC apps, locking the screen, `Super` shortcuts (sway has no bindings in this configuration, so the keys go to COSMIC).
