# Our own DMS package (scope and first result, 2026-10-22)

Today `waydesk-setup` (`packaging/desktop/setup/dms.sh`) adds two third-party OBS repositories and Debian's trixie-backports, installs
DankMaterialShell (DMS) and its helpers from there, holds them, and every account's first start (`waydesk-shell`) makes a patched copy of the
QML shell. This page says what it would take to ship the shell ourselves, and what was tried.

## Why

- The shell is the biggest memory item of a session: `qs` is about 360-400 MB of the 410-610 MB of an idle session (`docs/MEASUREMENTS.md`).
  Only a shell we build can leave out the parts nobody in a remote session uses.
- Six runtime patches (below) are applied by Python scripts at each account's start. A new DMS version can break them without anyone
  noticing until a session looks wrong. Applied at build time they fail the build instead.
- No third-party apt sources on the hosts, versions pinned by us, and quickshell/DMS built for the target distro (Ubuntu 26.04 was blocked by
  Qt: the prebuilt quickshell wants exactly Qt 6.8.2).

## What is in the box today (Debian 13, arm64)

| Package | Version | Installed size | From | Notes |
|---|---|---|---|---|
| `dms` | 2.0.0db1 | 125 MB: 79 MB Go binary `/usr/bin/dms` + 47 MB QML in `/usr/share/quickshell/dms` | OBS `home:AvengeMedia:dms` | upstream tag `v2.0.0`, MIT |
| `quickshell` | 0.3.0-1~bpo13+1 | 7 MB | **Debian trixie-backports** | already Debian's own package; held |
| `danksearch` | 1.6.0 | 19 MB | OBS `home:AvengeMedia:danklinux` | file search for the launcher; licence and language not checked |
| `dgop` | 1.6.2 | 9 MB | OBS (apt no longer lists a source for it) | system monitor backend; licence and language not checked |
| `matugen` | 4.0.0 | 5 MB | OBS `danklinux` | colour themes from the wallpaper; Rust, GPL-2.0 (upstream `InioX/matugen`) |
| `cava`, `qt6ct`, `accountsservice`, the `qml6-module-*` | | | Debian | no work |

## What was tried

`packaging/dms/build-dms.sh` builds the `dms` binary (with the QML shell embedded) from the upstream tag in a clean Debian 13 container:
Go 1.27.3 (pinned, checksum checked; the 2.0.0 `go.mod` needs 1.27.1 and Debian 13 has 1.24), the shared QML library as a git submodule,
`go build -trimpath -tags 'distro_binary withshell'`, no cgo.

- It builds in 41 s on the Mac (arm64 container) to an 82 MB binary; the binary starts and reports `dms v2.0.0`.
- Not shown yet: that it runs a session. The installed shell tree and the embedded one do not hash the same (different file sets), so nothing here
  claims that our binary equals the one OBS ships.
- Upstream's own Debian packaging (`distro/debian/dms`) does not build from source either: it downloads the release binary from GitHub.
- The build needs the network (Go modules, pinned by `go.sum`; they are not vendored).

## Stage 1 status (2026-10-22): built and running on the dev VM

`packaging/dms/build-dms.sh` now builds the package `waydesk-shell` (Provides/Conflicts/Replaces `dms`): our `dms` binary, the QML tree with the
patches applied (`.waydesk-patched` marks it), upstream's service, desktop files and icon. The patches are files now
(`packaging/desktop/dms-patches/`, one driver `apply.sh`), used both by the build and, for a stock DMS, by `waydesk-shell` at an account's start.

Tested on the arm64 dev VM (one account, restarted after the install): the old `dms` is replaced, `qs` runs from `/usr/share/quickshell/dms`, the four
bar plugins load, the power menu has the patched layout (hint under "Log out", a line, "Change password"), and there is no per-account copy any more.
Idle `qs` was 347-353 MB, against 361-362 MB with the OBS package: no real change, because nothing is trimmed yet.

Things found on the way:
- The binary carries its own copy of the UI and runs that unless `DMS_SHELL_DIR` (or `-c`) names another directory. The first package build
  therefore ran the unpatched embedded shell: the patched tree in `/usr/share/quickshell/dms` was never used. The package now ships
  `dms.service` with `Environment=DMS_SHELL_DIR=/usr/share/quickshell/dms`, so every way of starting the shell uses the patched tree.
- Not verified: the package built by the script *with* that unit line. It was tested as the first build repacked offline with the same one-line change
  (the network was too slow to rebuild, about 55 KB/s); the script's change is the same `sed`, but a clean rebuild from the script is still to do.
- The build script retries downloads and fails stalled ones (a stalled git download hung it for 20 minutes once).
- A long-lived user manager can still hold a stale `DMS_SHELL_DIR` from the old per-account copy: the first start after the switch logged "Failed to open
  .../dms-shell/shell.qml" once and then recovered (the unit's own setting wins).
- Debian's `quickshell`, `cava`, the Qt modules and the other helpers (`danksearch`, `dgop`, `matugen`) are still installed from where they were.

## Runtime patches that would move into the build

`waydesk-shell` makes a per-account copy of the shell (`~/.cache/waydesk/dms-shell`, a tree of symlinks to `/usr/share/quickshell/dms` in which
only the patched files are real copies) and patches it with Python, at every shell start, for: the "focused app" bar widget (asks sway through
`waydesk-focused-pid`), "Pin to desktop" in the launcher menu, the edit button of the control centre header, the settings pages that mean nothing on
a remote desktop and the Material wallpapers, and the power menu ("Change password", the hold-to-confirm hint). They would become patch files
applied to the source tree before the build: a build with fewer than five patches applied fails, and the per-start copy and the Python step go away.

## Stages

1. **`dms` from source, patches applied, as a `.deb`** (`waydesk-shell`, replacing the OBS `dms`), same behaviour as today. Check: a session
   starts, the click-test list of the previous sessions still passes, memory per idle session is not worse.
2. **Trim**: remove modules and services that are loaded but not useful in a remote session (candidates from the file list: Bluetooth, Tailscale,
   lyrics, media widgets, battery/power-profile pieces); measure `qs` against today's 360-400 MB after each cut. Nothing is promised until it is measured.
3. **Helpers**: `danksearch`, `dgop`, `matugen` from source in the same pipeline, or drop the ones a remote session does not need (to be
   checked: what each is used for in our sessions).
4. **Repository**: our signing key, the repository layout, the `waydesk-setup` change from "adds OBS sources" to "adds ours". The key and the
   hosting need to be decided first, because root installs from this repository.
5. **Other targets**: quickshell built against the target's Qt (Ubuntu 26.04 ships Qt 6.10), then amd64 and Fedora.

## Decisions for José

- Name and licence handling: the package would be ours (`waydesk-shell`) and carry DMS's MIT licence text and the licences of what it bundles.
- Where to host the repository and who holds the signing key.
- How far to trim: every cut makes a later DMS update harder to merge.
- amd64: the script is arm64 only until the Go checksum for amd64 is filled in.
