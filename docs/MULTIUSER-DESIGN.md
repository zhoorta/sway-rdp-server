# sway-rdp: multi-user design (draft, 2026-10-16)

Status: milestones 1 and 2 are built and tested in a VM (see section 8); the rest is design. It describes where `sway-rdp-server` (one user, one client at a time,
all sessions on one shared sway) goes to become an out-of-the-box multi-user remote desktop for Debian.
Written from reading the current code (`main.rs`, `session.rs`, `auth.rs`, `input.rs`, `capture.rs`) and the
vendored IronRDP patches. Statements about systemd, logind and PAM behaviour are from experience and have to be
tested on Debian 13 before they are relied on.

## 1. Goal and non-goals

Goal: `apt install sway-rdp` on a Debian 13 server; users log in over RDP with their normal account password and get
their own isolated Sway desktop; reconnecting returns to the same session.

Non-goals (on purpose): drive, printer and smartcard redirection, RD Gateway, load balancing, other compositors,
GPU sharing, running exposed on the open internet without a VPN or tunnel.

## 2. Problems in the current code that the design has to remove

1. `PamValidator` only accepts `$USER` of the server process: a non-root process can only verify its own password.
2. One `Slot` holds one `Session`; the server handles one client at a time.
3. All sessions are outputs of one sway, so one uid, one Wayland socket, one clipboard: any user can capture or inject
   into another user's session.
4. The lockout is global (5 failures lock everyone for 60 s): any stranger can lock all users out.
5. With `--replace`, a new connection drops the open one before it has authenticated.
6. The server process parses untrusted network input and also holds the login logic.

## 3. Components

```
 client ──TLS/RDP──> sway-rdpd  (user sway-rdp, no login, hardened)
                        │  unix socket, SO_PEERCRED checked
                        ├──> sway-rdp-helper  (root, tiny, no network)   PAM + start/find session
                        │
                        └──> sway-rdp-session (as the real user, in its own logind session)
                               ├─ sway (headless, own XDG_RUNTIME_DIR 0700)
                               ├─ capture + H.264 encode, input injection, audio, clipboard
                               └─ shell hook (DMS, foot ...)
```

| Part | Runs as | Does | Never does |
|---|---|---|---|
| `sway-rdpd` | `sway-rdp` | TLS, RDP protocol (IronRDP), EGFX/audio/clipboard channel framing, rate limiting per IP | touch user files, call PAM, hold passwords after the check |
| `sway-rdp-helper` | root | `authenticate(user, password, source_ip)` through PAM; `ensure_session(uid)` | parse RDP or TLS, listen on the network |
| `sway-rdp-session` | the user | runs the user's headless sway, capture, encoder, input, audio, clipboard, session hook | anything as another user |

Today's `sway-rdp-server` code maps almost entirely to `sway-rdp-session` (`session.rs`, `capture.rs`, `input.rs`,
`audio.rs`, `clipboard.rs`, the encoder half of `egfx.rs`) and to `sway-rdpd` (`main.rs`, the IronRDP side).

## 4. Connection flow

1. Client connects to `sway-rdpd`; TLS; RDP handshake up to the credentials (plain TLS login, as today with `--auth pam`).
2. `sway-rdpd` sends `Authenticate{user, password, source_ip}` to the helper. The helper rate limits per user and per
   source IP (not globally), runs PAM (`pam_authenticate`, `pam_acct_mgmt`), answers `Ok{uid}` or `Fail`.
   Failures are delayed. The password is zeroed in both processes right after.
3. On `Ok`, `sway-rdpd` sends `EnsureSession{uid, size, scale, layout}`.
4. The helper looks for a running session of that uid (control socket `/run/user/<uid>/sway-rdp/control.sock`).
   - Found: returns its path. This is a reconnect.
   - Not found: forks, `initgroups`/`setgid`/`setuid`, opens a PAM session through the `sway-rdp` PAM service (which
     includes `pam_systemd`, so logind creates the session and `/run/user/<uid>`), then execs `sway-rdp-session`,
     which starts the headless sway and the user's shell hook, and creates the control socket.
5. `sway-rdpd` connects to the control socket and speaks the stream protocol (section 5). Only then does it tell
   the RDP client the connection is ready, and only now may it replace an older connection of the same user.

## 5. Stream protocol (`sway-rdpd` <-> `sway-rdp-session`)

Length-prefixed messages on a Unix stream socket. The socket is owned by the user, mode 0600, with an ACL
(or a group) letting only `sway-rdp` connect, and the session checks `SO_PEERCRED` as well.

- To the session: `Hello{width, height, scale, klid}`, `Resize`, `Key`, `Pointer`, `Wheel`, `ClipboardData`, `Bye`.
- From the session: `Frame{rect, h264 | bitmap}`, `AudioChunk{pcm}`, `ClipboardData`, `Resized{w,h}`, `Bye`.

The encoder runs in the session, so `sway-rdpd` relays encoded frames and never sees raw pixels. This needs `egfx.rs`
to be split: encoding in the session, channel and surface bookkeeping in the daemon.

## 6. Session life

- Disconnect: the session and its programs keep running; `sway-rdpd` just drops its stream. Setting
  `idle_timeout` (default 8 h) or `end_on_disconnect = true` ends it.
- Reconnect: step 4 returns the existing control socket; the session sends a full frame.
- Second client for the same user: replaces the first (after authentication succeeded).
- Logout in the shell: `sway exit` ends the session, the control socket disappears, the helper cleans up.
- The headless sway uses `WLR_BACKENDS=headless` and a config from `/etc/sway-rdp/sway.conf` plus an optional
  `~/.config/sway-rdp/config`. The output is created at session start (replaces `create_output` on a shared sway);
  workspace and focus handling in `session.rs` mostly goes away, since the sway is the user's own.

## 7. Threat model

| Threat | Mitigation |
|---|---|
| Bug in RDP/TLS parsing (pre-auth) | Runs as `sway-rdp`, not root. systemd: `NoNewPrivileges`, `ProtectSystem=strict`, `PrivateTmp`, `CapabilityBoundingSet=` empty, `SystemCallFilter=@system-service`, `RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX`. Memory-safe language. |
| Compromised `sway-rdpd` | Sees encoded streams of live sessions and can inject input as those users, but is not root and cannot read their files. This is weaker than sshd's model; it is accepted for now and written down (see 9). |
| Bug in the helper | Small (a few hundred lines), no network, no RDP. Accepts only `sway-rdp`'s uid (`SO_PEERCRED`). Fuzz the message parser. |
| Password guessing | Per-user and per-IP counters in the helper, delay on failure, `pam_faillock` in the PAM stack, audit log. |
| User enumeration | Same answer and same delay for "unknown user" and "wrong password". |
| One user attacks another | Separate uids, separate sway, `XDG_RUNTIME_DIR` 0700, no shared Wayland socket or clipboard. |
| Resource exhaustion by a user | Session runs in the user's logind slice: `MemoryMax`, `TasksMax`, `CPUQuota` through a `user-.slice.d` drop-in; max sessions setting. |
| Password on the wire | Plain TLS login (PAM cannot do NLA). Default bind is loopback or the VPN interface; certificate fingerprint shown at install; document SSH tunnel or VPN. |
| Password in memory | Zeroise buffers; no logging of credentials; core dumps off for both processes. |

## 8. Milestones (each ends with something testable)

1. **Two sessions, one user, concurrently.** Remove the single `Slot`; `Session` per connection. Test: two FreeRDP
   clients, two outputs, independent input.
2. **Split `egfx.rs`, define the stream protocol**, still one process pair on the same uid. Test: same behaviour as today
   through the socket.
3. **Per-user sway.** The session starts its own headless sway and shell. Test: two different uids on one VM, neither can
   capture or type into the other (`grim` and `wtype` against the other's socket fail).
4. **Helper and PAM for any user.** Root helper, dedicated `sway-rdp` user, systemd units, per-user and per-IP limits.
   Test: both users log in with their own password; wrong passwords are throttled per user, and one user's failures
   do not lock out the other.
5. **Reconnect and session management.** Existing session found, replace-after-auth, idle timeout, logout cleanup.
   Test: drop the connection, reconnect, same windows.
6. **Package and wizard.** `.deb` for `sway-rdp` with the units, PAM file and sway template; the installer wizard
   as an optional post-install step.
7. **Review.** Threat-model walk-through with someone else before any non-VPN exposure is documented.

### Where it stands (2026-10-16)

- 1 done: one `RdpServer` per connection; tested with up to three FreeRDP clients.
- 2 done, with two deviations from section 3: the session process (`--session-worker`, started by the daemon with a
  socket pair as fd 3) runs as the same user and owns output, capture, encoder and input; **audio and clipboard stay in
  the daemon** for now (they need the user's own PipeWire and Wayland socket, which is milestone 3). The daemon forwards
  input and resize requests and sends credits; frames are encoded in the session. Killing the daemon (even with
  SIGKILL) or sending SIGTERM to the session makes the session remove its output and end. Not tested: window resize
  under load, the Windows App on macOS after the split, bitmap fallback.
- The worker is spawned by the daemon here; in the final design the root helper starts it as the user (milestone 4).

## 9. Open questions

- The listener sees all sessions' encoded streams and can inject input. The stronger design is RDP server
  redirection (as gnome-remote-desktop does with its handover): `sway-rdpd` only authenticates, then redirects the
  client to a per-user endpoint. Worth a spike after milestone 5; it costs a second TLS handshake and a token scheme.
- Does IronRDP's acceptor accept the client's monitor data in the handshake? Needed for multi-monitor, which is
  not in these milestones.
- PAM stacks with 2FA modules (TOTP) need a multi-prompt conversation; the RDP login form has one password field.
- Lock screen and logind idle handling in a headless sway; whether `swaylock` works without a seat.
- Xwayland: enable per session or not at all.
- Is `pam_systemd` in the helper's fork enough to get `/run/user/<uid>` and the user manager when nobody is logged in
  on a seat? (Expected yes; verify on Debian 13.)
- Supported compositor stays Sway only. The `ext-image-copy-capture` protocol may replace `wlr-screencopy` later.
