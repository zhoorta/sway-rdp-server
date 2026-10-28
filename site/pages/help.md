title: When something is wrong
nav: help
description: What to check when a client cannot connect, a session is refused, the picture is slow or wrong, or the GPU is not used. Where the logs are.
---
Most problems show in one of three places: `waydesk-admin status`, `waydesk-admin access show`, and the journal. Start there.

## Where the logs are {bar: terminal}
```console
$ journalctl -u waydesk -u waydesk-helper -u waydesk-session@1000 --since "-10 min"
```

`waydesk` is the network daemon, `waydesk-helper` checks passwords and starts sessions, and `waydesk-session@UID` is one account's session (the uid is in `waydesk-admin users list`). For more detail, put `WAYDESK_LOG=waydesk_server=debug,info` in `/etc/default/waydesk` and restart `waydesk`: the encoder then reports, every five seconds, its frame rate, milliseconds per frame and kilobytes per frame.

## The client cannot connect {bar: notes}
- **Is the port reachable at all?** `waydesk-admin access show` says where it listens and who may connect. The default is `127.0.0.1`: from another machine you need an SSH tunnel, or `access lan` or `access vpn`.
- **An address that is not allowed gets nothing**, before TLS: the journal says `connection refused: the source address is not in the allowed list`. Widen it with `access lan --allow "..."`.
- **The tunnel died.** An SSH tunnel lives only as long as its command; `ServerAliveInterval 30` in `~/.ssh/config` keeps it up across short drops. A laptop that sleeps drops it too.
- **The certificate warning.** The server's certificate is self-signed unless you made a local authority: compare the fingerprint the client shows with the one from `waydesk-admin cert status`. Do not accept one you cannot compare.
- **Five failed logins in a row** make every login refused for a minute; each failed one is delayed 2 seconds. Only members of `waydesk-users` may log in, never root: `waydesk-admin users list` shows the `RDP` column.
- **Forgotten password:** `sudo waydesk-admin users passwd NAME` sets a new temporary one.

## The login is accepted but there is no desktop {bar: notes}
- **The session was refused for memory.** `waydesk-admin status` ends its summary with "a new one would be REFUSED: 504 MB available, 1161 MB needed" when the machine is full (a new session needs the larger of 900 MB and 15 % of the memory). End an idle session (`sessions end NAME`), add memory, or lower another account's `users limit`. An account that already has a session just reconnects.
- **The GPU could not give the desktop its buffers.** On NVIDIA with Debian's driver 550 this used to leave no desktop; since 0.4.0 the session is made again in software. If you see `the session could not be made on the GPU (...): software rendering` in the journal, the GPU path failed and the session runs on the CPU: see [the GPU page](/gpu/).

## It is slow, or the picture is poor {bar: notes}
- **Check the link first.** The encoder follows what the link delivers, from 300 kbit/s to the maximum, and raises the quantiser (a blockier picture) when the link cannot carry the picture. A full-screen noise or video page asks for tens of Mbit/s; a mostly still desktop needs very little. A link of 4 Mbit/s will make a busy screen coarse, and it goes back to full quality within about half a minute of the screen settling.
- **An SSH tunnel on a long round trip** (about 100 ms, another country) stalls, because TCP runs inside TCP. A [WireGuard VPN](/vpn/) copes with it.
- **`waydesk-admin status --watch`** shows the encoder, its quantiser and frame rate per session while you look at the picture.

## The GPU is not used {bar: terminal}
```console
$ ls /dev/dri
$ waydesk-admin status
```

- **No `/dev/dri`:** the machine has no render node. On a dedicated server, the installer may have added `nomodeset` to the kernel line or blacklisted the driver; on a cloud server there is simply no GPU.
- **`status` says `cpu` or `openh264`:** the session fell back. The journal line before it says why (`sway did not start on the GPU`, `the session could not be made on the GPU`, `VAAPI encoder not available`, `NVENC encoder not available`).
- **The accounts cannot open the node.** The package grants `waydesk-users` access with a udev rule; `getfacl /dev/dri/renderD128` should list the group.
- **NVIDIA:** the driver must be NVIDIA's own, 615 or newer; `nvidia-smi` shows the version. See [the GPU page](/gpu/).
- **`WAYDESK_GPU=off`** in `/etc/default/waydesk` switches the GPU off on purpose; `status` shows what new sessions will use.

## A window is out of reach {bar: notes}
Windows can hang over the left and right edges (and below the screen), with 120 pixels left on the screen to grab. **Super+Shift+Up** brings every window entirely back. Super+Up maximises the focused window, Super+Down minimises it, Super+Shift+Down restores a minimised one.

## Still stuck {bar: notes}
Open an issue on [GitHub](https://github.com/zhoorta/waydesk/issues) with the output of `waydesk-admin status`, `waydesk-admin access show` and the last lines of the journal (passwords and keys never appear in them). For a commercial setup there is [StackCare](https://stackcare.pt/en).
