# Roadmap

What is planned or open, in no particular order inside a group. Nothing here is promised for a date. Done work is in `git log` and the changelog of the site.

## Planned

- **A CPU limit per account (`--cpu-quota`).** Accounts have the same `CPUWeight` and no CPU cap today, on purpose: a hard cap wastes idle cores, and the
  desktop itself (software rendering, software H.264) counts against it. For an account that keeps starving the others, `waydesk-admin users limit NAME
  --cpu-quota 100%` (a `CPUQuota=` on the account's slice, next to the existing `--memory`, `--swap` and `--cpu-weight`) and a field in the **Limits…**
  dialog of the Users & Access panel. Weights stay the default; the quota is the exception.

- **H.264 CPU on a lightly changing desktop.** Screen mode costs about 8 ms per frame at 720p whatever changed. Convert only the changed rectangles to YUV and take a
  cheaper path for tiny damage; compare real text legibility of screen and camera mode (PSNR says 12 dB lower at rest). Numbers in `docs/MEASUREMENTS.md`.

## In progress

- **A full login on arm64 hardware with 0.3.0.** The arm64 packages install cleanly from the repository (checked in containers), and a full login was last checked on real arm64 servers with 0.1.0.

- **More keyboard layouts.** `waydeskmac` was measured on the built-in keyboard of a Portuguese MacBook through the Windows App only; its shifted and Option characters were checked by one person. Other Mac layouts and external keyboards need the same measurement (press each key, compare what arrives with the keycap): a layout is one short file in `/etc/xkb/symbols`.

## Maybe, if it turns out to be needed

- **An invisible resize margin** a few pixels outside a window, so that the border can stay thin (3 px today) and still be easy to grab over a slow link.
- **A browser comparison** on the test server: Chromium (today's default), Firefox ESR and Zen, idle memory and CPU with one and five tabs, and what their
  title bars look like in a Waydesk session (Firefox-based browsers draw their own unless told otherwise).
- **Unlocking the keyring with the account's password** (PAM) so that the browser's password store does not have to be the plain one. Needs the helper to
  hand the password to a one-shot `gnome-keyring-daemon --unlock` in the session, and a security review.
- **Measuring the CPU cost of the window shadows** while dragging windows, with and without them.
