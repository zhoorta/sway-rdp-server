# sway-rdp-server

An RDP server for [sway](https://swaywm.org). Every connection gets its **own headless sway output**, sized and scaled
from what the client asks for, so a Retina Mac or a laptop with 200 % scaling gets a sharp desktop at its own resolution
and the window can be resized live. Status: **early, one author, tested mostly with the Windows App on macOS and FreeRDP.**

- Capture: `wlr-screencopy` (frame diff), cursor included.
- Video: H.264 (AVC420) over the RDP graphics pipeline; plain bitmaps as fallback. The encoder is the system's
  `libopenh264` (Debian: package `libopenh264-8`) loaded at run time; sway-rdp-server contains no encoder code.
- Input: virtual keyboard and pointer (layout taken from the client's keyboard id), Command works as Ctrl on Macs.
- Audio: RDPSND from a PipeWire/PulseAudio null sink (`sway-rdp-server`), PCM 44.1 kHz.
- Clipboard: text only, **off by default** (`--clipboard`); it works with FreeRDP but hangs the macOS Windows App.
- Security: TLS and NLA always on. There is **no built-in credential**: `--credentials FILE` with `user:password` on the first
  line (password of at least 8 characters). One user. A self-signed certificate is fine for a start (`--cert`, `--key`).
- Built on [IronRDP](https://github.com/Devolutions/IronRDP) (`ironrdp-server` 0.13 with three small local patches in
  `patches/`, see `patches/ironrdp-server.diff`) and `ironrdp-egfx`.

## Run

```
sway-rdp-server --bind 127.0.0.1:3389 --cert cert.pem --key key.pem --credentials rdp.cred
```

Run it inside the sway session it should serve (it needs `SWAYSOCK` and `WAYLAND_DISPLAY`). Needs `swaymsg`, `foot` (only for
the first terminal, `--no-terminal` skips it), `libopenh264.so.8`, and `parec`/`pactl` for audio (`--no-audio` skips).

Useful options: `--bind ADDR`, `--kb-layout xkb` (fallback layout), `--workspace NAME` (default `Remote`),
`--session-hook PROGRAM`, `--bitrate`, `--quality`, `--no-egfx`, `--no-audio`, `--clipboard`, `--replace`/`--no-replace`.

**Exposure.** Binding to anything but loopback puts an RDP login on the network. Use a long random password, and prefer
loopback plus an SSH tunnel or a VPN. A new connection replaces the open one; that is on by default only for loopback,
because the old client is dropped before the new one has authenticated (so anyone who can reach the port could kick the user).

## Session hook

`--session-hook PROGRAM` starts a program when a session begins and sends SIGTERM to its process group when the session ends
(SIGKILL after two seconds). Environment: `SWAY_RDP_OUTPUT` (the headless output), `SWAY_RDP_WORKSPACE`, `SWAY_RDP_INDEX`
(0 to 99, to derive a port or name). Use it to start a bar, a dock or a per-session shell on the new output. Note that new layer-shell surfaces open on the focused output, so the hook should `swaymsg focus output
"$SWAY_RDP_OUTPUT"` before starting each one.

## Known gaps

No dynamic hardware encoding (VA-API), no Opus, one client at a time, clipboard with the Windows App, relative mouse and
Unicode key events, Caps Lock sync.

## Licence

Dual-licensed under either of [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your option. `patches/ironrdp-server` is a
modified copy of IronRDP's `ironrdp-server` crate and stays under its own licence (MIT or Apache-2.0, files included there).
