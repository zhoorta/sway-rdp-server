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

## Stage 2 status (2026-10-22): trimming measured, mostly not worth cutting

Measured on the dev VM (tables in `docs/MEASUREMENTS.md`, "Where the shell's memory goes"):
- The shell is 360 MB resident when idle; a bare quickshell with one empty bar window is already **283 MB**. The renderer (Qt Quick plus Mesa's
  software OpenGL) is the floor; everything DMS adds on top is about 77 MB.
- Free cuts: a new patch (`70-lean-shell.py`) leaves out the desktop widget layer (settings already hidden) and the clipboard history window
  (unavailable on sway): **-19 MB per session**, nothing lost. The lock screen is 1 MB, not worth it. MuxModal and the colour picker add a few MB
  each and would take features away: left in.
- Environment knobs (`MALLOC_ARENA_MAX`, `LP_NUM_THREADS`, the Qt render loop) change nothing; `GALLIUM_DRIVER=softpipe` is much worse (530 MB).
- The big lever is the Qt software renderer (`WAYDESK_DMS_SOFTWARE=1`): **-110 MB per session**, flat look. With rounded corners it leaves square halos, so
  `waydesk-shell` now gives new accounts square corners in that mode. End to end on a new account: whole session 355 MB, against about 520 MB
  for a default one.
- The 48 MB `libLLVM` is shared between accounts: a second account does not pay for it again.

So the realistic shell budget is about 340 MB (default) or about 230 MB (low-memory mode). Going below needs a shell that does not use OpenGL
at all (a different shell, not a trimmed DMS). Not done: patch 70 is in the repository but the `waydesk-shell` `.deb` has not been rebuilt with it (the
network was too slow); it was applied in place on the VM for the test above.

## Stage 3 (helpers) and stage 4 (repository), status 2026-10-22

**Helpers.** None of `danksearch`, `dgop`, `matugen` runs while a session is idle (no process in any account); DMS calls `dsearch`, `dgop` and `matugen` on
demand. Upstream has `v2.0.0` for `danksearch` and `dgop` (we ran 1.6.0 and 1.6.2) and `v4.2.0` for `matugen` (we ran 4.0.0).
`packaging/dms/build-helpers.sh` (built and tested on the dev VM, see below) builds all three from those tags in a Debian 13 container (Go 1.27.3 pinned and checksummed; Rust from Debian with
`cargo build --locked`) and packs them under the **same package names** with the version `<upstream>+waydesk1`, so they replace the OBS packages
by version and need no change in the `Depends` of `waydesk-shell`. Licences: `danksearch` and `dgop` MIT; `matugen` GPL-2.0-or-later.

Built and tested (arm64): `danksearch` 2.0.0, `dgop` 2.0.0 and `matugen` 4.2.0 replaced the OBS 1.6.0 / 1.6.2 / 4.0.0 with a plain `apt install`, all three run, and a
restarted shell connected with `dgop`, ran the theming through `matugen` ("worker completed successfully"), loaded all four plugins and logged no errors about them.
`matugen` needs rustc 1.88 (Debian 13 has 1.85), so the script installs a pinned Rust 1.90.0 with `rustup-init`, checked against its published sha256 (the first
attempt with Debian's compiler failed on the `image` crate). The new `matugen` is 11.7 MB against 5.4 MB: upstream's default features (`jxl-image`, `dump-json`),
which the OBS build probably left out. It links only `libc` and `libgcc`, no X11.

**Repository.** `packaging/repo/make-repo.sh OUTDIR` makes a signed apt repository from `packaging/out` (newest version of each package and
architecture; `Packages` gz/xz; `Release` with SHA-256 hashes; `InRelease` and `Release.gpg`; the public key as `waydesk-archive-keyring.gpg`).
Tested with a throwaway **test key** (not the real one) served over HTTP to the dev VM:
- the signed repository is accepted, and its packages appear in `apt-cache policy`;
- a tampered index is refused (hash mismatch on `Packages`); a repository with no signature is refused (`The repository ... is not signed`, no
  package becomes visible); a repository signed by another key is refused (`Missing key ... needed to verify signature`);
- `packaging/repo/waydesk.pref` pins our names (`waydesk-shell`, `dms`, `danksearch`, `dgop`, `matugen`) to priority 990 for `Origin: Waydesk`, so a
  third-party repository cannot override them with a newer version; verified (`*** 2.0.0+waydesk1 990`).
`waydesk-setup` (`dms.sh`) now installs `waydesk-shell` from our repository when `/etc/apt/sources.list.d/waydesk.sources` exists, and does not add the
OBS repositories then (quickshell still comes from Debian's trixie-backports). Without that file it behaves as before.

Not decided, and not done on purpose:
- **Where the repository lives** (`packaging/repo/waydesk.sources.in` has the URL as a placeholder) and **who holds the signing key**. The key used in the test
  is a throwaway in `/tmp`. The real key should be made on a machine you trust, with a passphrase, and its fingerprint published next to the repository.
- **matugen is GPL.** A published repository must offer its source: at least `deb-src` / the source archive for the tag, or the written offer. The package
  says where the source is (`README.waydesk`), which is not enough for a public repository.
- A `waydesk-repo` package (keyring + `waydesk.sources` + `waydesk.pref`) so that one `apt install ./waydesk-repo.deb` sets it up: needs the URL.

## Stage 5 (other targets): what is known and what it needs

Known from today (2026-10-22):
- `waydesk-server` and `waydesk-desktop` install and run on **Ubuntu 26.04** (arm64, Hetzner); the shell does not, because the prebuilt `quickshell` wants exactly
  Qt 6.8.2 and Ubuntu has 6.10. Everything else in the shell stack (`dms`, `danksearch`, `dgop`, `matugen`) is a static Go or Rust binary that does not care.
- `waydesk-shell` as built here is `Architecture: arm64` only: `build-dms.sh` and `build-helpers.sh` need the sha256 of the amd64 Go toolchain
  (a one-line change each, from https://go.dev/dl/), and an amd64 build has never been run on real hardware by these scripts. The amd64 dev VM was deleted
  on 2026-10-22 (disk space); recreate it with `VM_ARCH=amd64 contrib/qemu/vm.sh create` (slow: emulated).
- The sway title-bar patch (`packaging/sway`) is for Debian's sway 1.10.1; Ubuntu 26.04 ships sway 1.11, which it was never tried against.

Order I would do it in, each step small and testable:
1. **amd64 on Debian 13**: fill in the two checksums, build, install on a rebuilt amd64 VM or a Hetzner CPX server, run one session. Smallest step and the most likely
   real target (most servers are amd64).
2. **quickshell from source** in the same container pipeline (a CMake build against the target's Qt): first on Debian 13 (to compare with the backports package), then on
   Ubuntu 26.04 with Qt 6.10. This is what unblocks Ubuntu; the build recipe and its dependency list are not written yet.
3. **The sway patch on 1.11** (or a decision to drop the title-bar buttons for sway >= 1.11 and use the bar plugin only).
4. **Fedora**: an RPM spec per package, the dependency names, SELinux policy for the root helper and the session units (the biggest part), `libopenh264` from the
   Cisco repository, the `wheel` group in place of `sudo` in the administrator checks. Only worth starting when someone asks for it.

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

Since 0.2.1 `waydesk-desktop` depends on `waydesk-shell`: `apt install waydesk` gives a session with its shell at the first login, and `waydesk-setup` is only for apps.
