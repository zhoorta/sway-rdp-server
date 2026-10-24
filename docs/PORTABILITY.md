# Other distributions: Ubuntu, Fedora (findings and plan, 2026-10-23)

Today Waydesk is tested on Debian 13 only. This records what was found when the repository was installed on Ubuntu 26.04, and how Waydesk can
support Ubuntu and later Fedora without a separate fork for each.

## What was tried: Ubuntu 26.04.1 LTS ("resolute"), x86_64, 4 vCPU / 7.7 GB

The signed repository (suite `trixie`) was added with `waydesk-repo.deb`, then `apt install waydesk-desktop`.

| Step | Result |
|---|---|
| `waydesk-repo.deb`, key fingerprint, `apt update` | works; the fingerprint matches the one published on the site |
| `waydesk-server` and `waydesk-desktop` | install (218 and 679 new packages); the service starts |
| `waydesk-setup-admin`, RDP login (FreeRDP on the server itself) | works; a session starts with sway, swaybar, pipewire and pcmanfm-qt, about 110 to 145 MB |
| `waydesk-shell` | **cannot be installed**: needs `quickshell`, which Ubuntu does not have |
| sway | Ubuntu's own 1.11-3 is installed instead of our patched 1.10.1+waydesk1 (the repository pin does not cover it), so no title-bar buttons |
| Chromium | Ubuntu has no deb (it is a snap), so there is no default browser |

Result: the server works; the desktop is a black screen with the home and trash icons, sway's plain bar and no dock, launcher or panels.
A burst of H.264 decode errors (`rdpgfx_decode_AVC420 failed with error 50`) was logged by the test client when the session started and not
again; the client had its decoders installed. Not investigated further; it has not been checked whether Debian shows the same.

## Where it breaks, by layer

| Layer | Parts | Portability |
|---|---|---|
| Server | Rust RDP daemon, PAM login, root helper, systemd drop-ins, zram | mostly portable; PAM paths and zram setup differ a little; Fedora adds SELinux and firewalld |
| Session stack | patched sway, quickshell, DMS shell, helpers, pcmanfm-qt | depends on distro versions and on apt pins: this is the problem |
| Apps | browser, software centre | the browser is packaged differently on every distribution |

## Plan (decided 2026-10-23)

1. **Make the session stack Waydesk's own.** Ship the patched sway, quickshell, the shell and the helpers under a private prefix
   (`/usr/lib/waydesk` or `/opt/waydesk`) with their own package names (for example `waydesk-sway`, `waydesk-quickshell`). The session always
   starts those, whatever sway or quickshell the distribution has. This removes the pins and the version races, and one sway version (probably
   1.11) carries the title-bar patch everywhere. Cost: we follow those upstreams ourselves.
2. **Build per distribution in a container, from the same recipes.** Debian already builds in containers (`build-deb.sh`, `build-dms.sh`,
   `build-helpers.sh`). Add an Ubuntu 26.04 image (and 24.04), later Fedora. Only the base image and the packaging step change (`.deb` or `.rpm`).
   Debian and Ubuntu share the packaging: Ubuntu needs new containers and its own suite (`resolute`) in the apt repository.
3. **Apps from Flatpak where possible.** Chromium from Flathub gives one browser for every distribution (the software centre already uses
   Flatpak). To test first: the Flatpak sandbox inside an RDP session, and its memory cost against the measured 270 MB per Chromium user.
4. **Fedora last**, when there is demand: an rpm repository and its signing, SELinux policy for the helper and PAM, `firewalld`.

Order: finish the Debian 13 release test (clean install, app tests), then 1, then Ubuntu (2), then 3, then Fedora.

## Support matrix to publish

| Distribution | State (2026-10-23) |
|---|---|
| Debian 13 | tested; clean install from the published repository on x86 and arm64 passed |
| Ubuntu 26.04 LTS | packages built and installed in clean containers on x86 and arm64 from a rehearsal repository; a full RDP session with the shell, patched sway and Chromium from Flathub passed on x86; clean install from the published repository still to do |
| Fedora | not yet |

## Result of the sway experiment (2026-10-23, evening)

**Which sway.** Upstream is sway 1.12 (needs wlroots 0.20, which needs libwayland 1.24; Debian 13 has 1.23.1, so 1.12 would mean shipping libwayland as well). **Sway 1.11 with wlroots 0.19.3 builds on Debian 13 with Debian's own
libraries**, with wlroots linked in statically (a 2 MB `sway`, no wlroots package), and it is the version Ubuntu 26.04 ships. Decision: **1.11 + wlroots 0.19.3 for every distribution**; 1.12 later.

**The title-bar patch** (`packaging/sway/waydesk-titlebar-buttons.patch`, written against 1.10.1) applies to 1.11 and 1.12 with line offsets only (no conflicts, no changes) and compiles.

**On a real session (Ubuntu 26.04 x86_64, our build from an `ubuntu:26.04` container, started before Ubuntu's own sway in PATH):** the RDP login works, the picture reaches the client, the compositor offers `zwlr_screencopy_manager_v1` (v3, what the
server captures with), `ext_image_copy_capture_manager_v1`, `zwlr_data_control_manager_v1` and `ext_data_control_manager_v1` (the one DMS asked for and Debian's 1.10.1 did not have), and a window shows the three title-bar dots.
Stock Ubuntu sway 1.11-3 gave the same picture without the dots (the baseline).

**A trap for tests on Ubuntu:** `xfreerdp3` from Ubuntu 26.04 (FreeRDP 3.32.1) cannot decode the H.264 stream (`SurfaceCommand failed with error 50`, blank picture), with stock or patched sway; Debian 13's client can. Test with a
Debian client (a `debian:13` container with `freerdp3-x11`, `--network host`, Xvfb, `xwd` + `convert` for the client's view).

**Still to do for Ubuntu:** a private install path (the worker starts `sway` and `swaymsg` by name, `src/worker.rs` and `src/session.rs`, so a `waydesk-sway` package under `/usr/lib/waydesk/bin` and a lookup there first), `quickshell` and the shell
(DMS, helpers) built in the same container, `pcmanfm-qt`, Chromium from Flatpak, a `resolute` suite in the apt repository.

## What was built for Ubuntu (2026-10-23, night)

- **A private directory, `/usr/lib/waydesk`.** `waydesk-sway` (sway 1.11 + wlroots 0.19.3 static + the title-bar patch: `packaging/sway/build-private.sh`) and `waydesk-quickshell` (Quickshell 0.3.0 from Debian's orig tarball,
  with `wayland-protocols` 1.47 built for the build; `packaging/quickshell/build-private.sh`) install there. The session worker (`src/worker.rs`, `prefer_private_bin`) puts `bin/` first in `PATH` and `qml/` in `QML_IMPORT_PATH`; the shell script pushes the
  environment to the user's systemd manager, so `dms.service` finds them too. Checked in a real Ubuntu 26.04 session: `sway` and `qs` run from `/usr/lib/waydesk/bin`.
  `waydesk-server` depends on `waydesk-sway | sway`, `waydesk-shell` on `waydesk-quickshell | quickshell | quickshell-git`.
- **Built per distribution, in a container, tagged.** Versions end in `+deb13` or `+ubuntu26.04` (sway, quickshell, pcmanfm-qt); the dependencies come from the binaries (`dpkg-shlibdeps`: Debian needs `libdisplay-info2`, Ubuntu `libdisplay-info3`),
  and Quickshell is tied to the exact Qt (`qt6-*-private-abi`). The server, the desktop, the shell and the helpers are the same package for both (the shell and helpers are static Go binaries plus QML).
  `pcmanfm-qt` is rebuilt from Ubuntu's 2.3.0 with the same two patches (they applied), and pinned by `waydesk.pref`.
- **One repository, two suites.** `make-repo.sh` builds `trixie` and `resolute` from one pool: a package whose version ends in `+<tag>` goes only into its suite. `waydesk-repo` 1.1 writes `/etc/apt/sources.list.d/waydesk.sources` from the system's `VERSION_CODENAME` in its postinst
  (an existing file stays; an unsupported system gets a message and no source).
- **Chromium and Firefox on Ubuntu from Flathub.** A manifest row can name a fallback (`flatpak:<id>`, eighth field) for a package the archive does not have; `waydesk-setup` checks apt's *candidate* (not `apt-cache show`: Firefox ESR is "referred to by another package" there, which broke the install); `a|b` names
  alternatives (`docker-compose-v2`). `waydesk-shell` seeds the Flatpak Chromium per account (no GPU process, the window manager's title bar, the desktop icon, default browser).
  Flathub's Chromium shows its terms dialog on the first start of each account.
- **Found on the way:** two builds writing one file left a `.deb` with a wrong ar member size that `dpkg-deb` accepts and `ar`/apt reject (`make-repo.sh` now reads every package with both); a copy still in progress passed the control-data check (now read completely).
  `waydesk-admin` was also fixed (SIGPIPE).

**Still open for Ubuntu:** clean install from the published repository on fresh Ubuntu servers; the app tests (LibreOffice, VS Code) and capacity on Ubuntu; Ubuntu 24.04 (not built: a different Qt and wlroots); the Flatpak Chromium first-run dialog and its dock icon; `docker-compose` alternatives in other rows if more turn up.

## Arch and Omarchy (candidate, decided 2026-10-24: not now)

Omarchy (Arch + Hyprland, single-user local workstation) has a large community, but it is a poor fit for a multi-user headless RDP server: it owns the session (Hyprland, its own menus and themes), and a rolling release moves sway, wlroots, Qt and
quickshell under us, so the private-stack approach would become a PKGBUILD/AUR package that can break on any upgrade. Not worth starting before the remaining work (clean installs from the published repository, site, waydesk.dev migration, Ubuntu app and capacity tests, Fedora, Ubuntu 24.04).

If it is done, target generic **Arch**, not Omarchy-specific (Omarchy users can then use it; nobody has to run Omarchy). Cheap first step, a few hours, in the "other distributions" round after the Fedora probe: an `archlinux` container on the x86 test
server to check that the Rust server builds, that Arch's sway and wlroots work with the title-bar patch, what a PKGBUILD needs (sway, quickshell, pcmanfm-qt, freerdp), and how a rolling release should be pinned.
