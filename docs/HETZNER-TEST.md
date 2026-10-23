# Test on a Hetzner server (runbook, draft 2026-10-16)

Goal: connect to a headless Sway desktop on a real server over the internet, through a tunnel, and see how it feels over
a real network. One user, one or two clients. **Not tested yet**: the unit files and headless Sway below are written
from the VM setup that did work (console Sway, two concurrent sessions) and from experience; expect to fix things.

## 0. Rules for this test
- Never publish port 3389. The RDP port stays on loopback and you reach it through SSH.
- The Cloud Firewall allows only SSH (22), from your own IP if it is stable.
- Throwaway server, deleted afterwards. Test password is not reused anywhere.
- **Harden SSH before anything else (before the first apt command).** A new server on a public IP is probed within minutes, and Hetzner's Debian
  image ships with root password login on (cloud-init writes `PasswordAuthentication yes` into `/etc/ssh/sshd_config.d/50-cloud-init.conf`).
  On 2026-10-23 a test server left like that was taken over about 30 minutes after a clean test run: a brute-forced root login, a miner started
  from `/var/tmp/snap` by `/etc/cron.d/system-helper`, and then every command failed for root (exit status 126). It looked like a memory
  lock-up, which is why this rule is written down. Do it with the key you ticked at creation (no password), then check from outside:

```
printf 'PasswordAuthentication no\nKbdInteractiveAuthentication no\nPermitRootLogin prohibit-password\n' > /etc/ssh/sshd_config.d/10-hardening.conf
sshd -t && systemctl reload ssh      # the name sorts before 50-cloud-init.conf, and the first value wins
ssh -o PreferredAuthentications=password -o PubkeyAuthentication=no root@SERVER true    # must say: Permission denied (publickey)
```

  If the server was already reachable by password, put the Cloud Firewall (SSH from your IP only) on it first. If one is ever taken over: rebuild it,
  do not clean it, and read the old disk only from Hetzner's rescue system (mounted read-only).

## 1. Server
- Hetzner Cloud, arm64 **CAX11** (the binary is built for arm64; an x86 server needs an amd64 build first), Debian 13,
  your SSH key. Check the price in the console; it is a few euros a month and billed hourly.
- Attach a Firewall: inbound TCP 22 only.
- Log in as root once and create the user:

```
adduser --disabled-password --gecos "" jose
usermod -aG sudo jose && echo 'jose ALL=(ALL) NOPASSWD:ALL' > /etc/sudoers.d/jose
mkdir -p /home/jose/.ssh && cp ~/.ssh/authorized_keys /home/jose/.ssh/ && chown -R jose: /home/jose/.ssh
loginctl enable-linger jose        # user manager and /run/user/<uid> without a login
```

- `PermitRootLogin` and `PasswordAuthentication` were already closed in section 0; if you made the `jose` account, make it the one you use from here on.

## 2. Packages (as jose)
```
sudo apt update && sudo apt install -y sway foot libopenh264-8 libpam0g pulseaudio-utils \
  pipewire pipewire-pulse wireplumber openssl libxkbcommon0
```

## 3. Binary, certificate, login
From the Mac, after `cargo build --release` in the build container (see `release/build-binaries.sh`):
```
scp waydesk-server jose@SERVER:~/
ssh jose@SERVER 'mkdir -p ~/swrdp && mv ~/waydesk-server ~/swrdp/ && cd ~/swrdp \
  && openssl req -x509 -newkey rsa:2048 -nodes -keyout key.pem -out cert.pem -days 30 -subj "/CN=hetzner-test" \
  && chmod 600 key.pem && printf "jose:CHANGE-ME-LONG-RANDOM\n" > rdp.cred && chmod 600 rdp.cred'
```
Use `--auth pam` instead of `--credentials` if you prefer the account password (see the README for what that trades away).

## 4. Headless Sway and the server (user units)
`~/.config/systemd/user/sway-headless.service`:
```
[Unit]
Description=Headless sway for waydesk
[Service]
Environment=WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 WLR_RENDERER=pixman GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1
ExecStart=/usr/bin/sway
Restart=on-failure
[Install]
WantedBy=default.target
```
`~/.config/sway/config` (start from `/etc/sway/config`) ends with the server, so it inherits `SWAYSOCK` and `WAYLAND_DISPLAY`:
```
exec ~/swrdp/waydesk-server --bind 127.0.0.1:3389 --cert ~/swrdp/cert.pem --key ~/swrdp/key.pem \
     --credentials ~/swrdp/rdp.cred --bitrate 8000000
```
```
systemctl --user daemon-reload && systemctl --user enable --now sway-headless
systemctl --user enable --now pipewire.socket pipewire-pulse.socket wireplumber.service
journalctl --user -u sway-headless -f        # the server's log lines appear here
```
Open question: whether sway runs with no outputs at all on the headless backend before the first client; if it exits,
add `WLR_HEADLESS_OUTPUTS=1`.

## 5. Connect (from the Mac)
```
ssh -N -L 3395:127.0.0.1:3389 jose@SERVER
sdl-freerdp /v:127.0.0.1:3395 /u:jose /p:CHANGE-ME-LONG-RANDOM /cert:ignore /size:1600x900
```

## 6. What to look at
1. Connects, picture appears, a terminal opens.
2. Typing and pointer feel (latency over the real link); try `--bitrate` 4, 8 and 20 Mbit/s.
3. Resize the window several times; no hang.
4. Drop the connection (close the laptop lid or kill the tunnel), reconnect: new session, old one removed.
5. Two clients at once (the second is silent; focus is shared, so do not type in both).
6. Sound: play something in the session (needs PipeWire running; first test it with `pactl info`).
7. CPU on the server while scrolling a browser or video (`htop`): software rendering and software H.264 on 2 vCPUs.
8. `journalctl` and `server.log` free of errors after the session ends; `swaymsg -t get_outputs` shows no leftover outputs.

## 7. Tear down
Delete the server and the Firewall in the Hetzner console, and remove the host from `~/.ssh/known_hosts`.
