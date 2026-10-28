# Roadmap

What is planned or open, in no particular order inside a group. Nothing here is promised for a date. Done work is in `git log` and the changelog of the site.

## Next

Small, already understood, in the order I would do them.

- **Measure active users on an 8 GB server.** The 7 to 8 users on 8 GB were idle ones; active users were measured on 23 GB and 64 GB machines only. One ramp with the current harness (`docs/measure-data/harness/ramp.sh`) on a rented 8 GB, 4-vCPU box closes it, and gives the sizing page one workload on one machine.
- **A bug to find: clicks that only respond to a drag after a resize.** A session made by one client at 1680x1050 and then taken over by another at 3360x2100 (Retina) answered only drags, not clicks, until a fresh session; suspected input scaling after a resumed session with a different size. Reproduce with `sdl-freerdp` and a second client of another size.
- **WebGL without a GPU.** Chromium is started with `--disable-gpu` when there is no render node, so a WebGL page does not animate; with `--use-angle=swiftshader --enable-unsafe-swiftshader` it runs at about 8 fps using three of four cores. Decide whether to offer it as a setting (default stays off: memory and CPU).
- **A browser crash after a screen recorder.** On Debian 13 a Chromium started in a session where `wf-recorder` or `grim` had run crashed (SIGSEGV) until a new session was made. Find out whether it is the recorder, sway or Chromium; it matters for anyone who records a Waydesk session.
- **Docs still to write:** a line about Software (gnome-software) being heavy on small servers; a THREAT-MODEL note for the per-account keyboard file; recipes (a team server, a company behind a VPN, a slow link, removal).
- **Release housekeeping:** back up the repository key (two USB sticks and the revocation certificate); a trademark search for the name; follow IronRDP issue #2224.

## Needs hardware I do not have now

- **Several users on one NVIDIA card.** GeForce cards limit simultaneous NVENC sessions; the encoder then falls back to OpenH264 (written, not measured).
- **Vulkan and XWayland applications on NVIDIA**, and zero-copy capture from NVIDIA (the GBM linear buffer is refused there today; it may never be possible).
- **AMD**: VAAPI should work and was not tried.
- **A card with its own memory under many users** (the HD 630 shares system memory, which is why the GPU did not add users).
- **A demo of the GPU path** (a recording like the one on the install page, on a GPU server).

## Site and documentation

- Refresh the hero screenshot (it still shows an older site inside the desktop).
- Per-client setup pages (Windows App, mstsc, Remmina, FreeRDP) with screenshots; more tutorials.
- A short recording per feature, if the first one is useful.
- A roadmap page on waydesk.dev, if you want these public (this file is the source).

## More systems

- **Ubuntu 24.04, Fedora, Arch:** different Qt and wlroots versions; no rpm yet. Not started.

## Planned

- **A CPU limit per account (`--cpu-quota`).** Accounts have the same `CPUWeight` and no CPU cap today, on purpose: a hard cap wastes idle cores, and the
  desktop itself (software rendering, software H.264) counts against it. For an account that keeps starving the others, `waydesk-admin users limit NAME
  --cpu-quota 100%` (a `CPUQuota=` on the account's slice, next to the existing `--memory`, `--swap` and `--cpu-weight`) and a field in the **Limits…**
  dialog of the Users & Access panel. Weights stay the default; the quota is the exception.

- **H.264 CPU on a lightly changing desktop.** Screen mode costs about 8 ms per frame at 720p whatever changed. Convert only the changed rectangles to YUV and take a
  cheaper path for tiny damage; compare real text legibility of screen and camera mode (PSNR says 12 dB lower at rest). Numbers in `docs/MEASUREMENTS.md`.

- **Access control and auditing.** Planned, in this order:
  - an audit log of logins, failures, session ends and administrator actions, with export to syslog;
  - per-group switches for the clipboard and file transfer;
  - idle and maximum session limits;
  - alerts on distributed password guessing (only logged today);
  - two-factor login (TOTP) enrolled from the Users & Access panel;
  - LDAP / Active Directory login through PAM.

- **Later, if there is demand.** OIDC single sign-on (Entra ID, Google, Keycloak), SCIM user provisioning, WebAuthn, an administrator-approved app catalogue, a
  per-group network egress allowlist, optional session recording (off by default), and a broker that sends people to the right server when there is more than one.

## In progress

- **A full login on arm64 hardware with 0.4.0.** The 0.4.0 arm64 packages are built (and earlier arm64 versions were installed on a real Ubuntu 26.04 server), but not yet installed and logged into on a machine. The GPU path on arm64 (VAAPI, NVENC) was not tried.

- **More keyboard layouts.** `waydeskmac` was measured on the built-in keyboard of a Portuguese MacBook through the Windows App only; its shifted and Option characters were checked by one person. Other Mac layouts and external keyboards need the same measurement (press each key, compare what arrives with the keycap): a layout is one short file in `/etc/xkb/symbols`.

## Maybe, if it turns out to be needed

- **An invisible resize margin** a few pixels outside a window, so that the border can stay thin (3 px today) and still be easy to grab over a slow link.
- **A browser comparison** on the test server: Chromium (today's default), Firefox ESR and Zen, idle memory and CPU with one and five tabs, and what their
  title bars look like in a Waydesk session (Firefox-based browsers draw their own unless told otherwise).
- **Unlocking the keyring with the account's password** (PAM) so that the browser's password store does not have to be the plain one. Needs the helper to
  hand the password to a one-shot `gnome-keyring-daemon --unlock` in the session, and a security review.
- **Measuring the CPU cost of the window shadows** while dragging windows, with and without them.
