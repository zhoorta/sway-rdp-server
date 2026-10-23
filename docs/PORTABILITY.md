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

| Distribution | State |
|---|---|
| Debian 13 | tested |
| Ubuntu 26.04 LTS | server and a plain desktop; shell and browser not yet (see above) |
| Fedora | not yet |
