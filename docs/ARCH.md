# Waydesk on Arch Linux (2026-10-30)

Waydesk builds and runs on Arch. There is a `PKGBUILD` in `packaging/arch/`; there is no repository (AUR or other) yet.

## Build and install

On an Arch machine, as an ordinary user, with `base-devel`, `cargo`, `clang` and `git`:

```
packaging/arch/build.sh            # makes packaging/out/arch/waydesk-server-<version>-<rel>-<arch>.pkg.tar.zst
sudo pacman -U packaging/out/arch/waydesk-server-*.pkg.tar.zst
sudo systemctl enable --now waydesk-helper.service waydesk.service
sudo waydesk-setup-admin           # the first account that may log in over RDP
```

`build.sh` packs the committed tree (not uncommitted changes) and runs `makepkg`, which also runs the test suite (`check()`).

The package installs the server, the root helper, the admin tools (`waydesk-admin`, `waydesk-passwd`), the services, the PAM files, the GPU udev rule, the
sysusers and tmpfiles definitions and the docs. Installing does not start anything; it makes the account `waydesk`, the group `waydesk-users` (members may
log in) and a self-signed certificate, and prints the fingerprint to check in your RDP client.

## What a session is

Sway (Arch's own, with `/usr/share/waydesk/sway-minimal.conf`: a bar, a terminal and nothing else), or Hyprland with `WAYDESK_COMPOSITOR=hyprland` in
`/etc/default/waydesk` (experimental, needs a GPU: [HYPRLAND.md](HYPRLAND.md)). The desktop of the Debian packages (the shell, launcher, title-bar
buttons, the patched sway) is not packaged for Arch.

## What each account needs

- A sound server in its session: `systemctl --user enable --now pipewire.socket pipewire-pulse.socket wireplumber.service` (as that account, or a preset).
  A session that starts without one has no sound until the next one.
- A GPU is optional for sway (without one the encoder runs on the CPU with OpenH264). For hardware encoding install the VA-API driver for the chip:
  `intel-media-driver` (Intel, Broadwell and later) or `libva-mesa-driver` (AMD). Hyprland sessions need a GPU.

## Differences from Debian that the package deals with

- PAM: Arch has no `common-auth`, so the server's PAM service is `packaging/arch/pam-waydesk-server` (includes `system-auth`). The session unit's PAM service
  has an `account` line, which Arch requires.
- OpenH264: Arch ships 2.7; Waydesk accepts 2.6 and 2.7 (their public headers are identical).
- libva: the bundled bindings are built against the system's libva (2.24 builds).
- Users and the certificate directory come from `sysusers.d` and `tmpfiles.d`, the certificate from the install script.

## Checked

Built with `makepkg` and run on Arch x86_64 (kernel 7.2, Intel HD 630, sway 1.12, Hyprland 0.57): login through the helper and the daemon, a session unit per account,
resize and scale, clipboard, sound, hardware H.264 (VAAPI), the same with Hyprland. Not checked: aarch64, AMD and NVIDIA GPUs on Arch, upgrades from one package version
to the next, `waydesk-admin vpn` (it installs `wireguard-tools` with `apt-get`; install it with pacman first), the certificate renewal timer.
