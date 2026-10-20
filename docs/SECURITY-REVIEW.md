# sway-rdp: first security review (2026-10-16)

Who wrote it: the author of the code, with an AI assistant, walking `docs/MULTIUSER-DESIGN.md` section 7 against the code
and then testing what could be tested in a VM. **This is not an independent review and not an audit.** Nothing here
justifies exposing the server to the internet. The stated deployment is: loopback, an SSH tunnel or a VPN.

Trust model used: the *network* is hostile; the *daemon* (`sway-rdp`, unprivileged) is the network-facing part and is
assumed to be compromisable; the *helper* (root) and the packaged files are trusted; a *logged-in account* is trusted
with itself and with nothing else; its session process is treated as hostile by the daemon.

## Found and fixed in this review

| # | Finding | Severity | Fix | Checked |
|---|---|---|---|---|
| 1 | **Root could log in over RDP.** The rule "the account running the server may log in" matched `root` when the helper runs as root, so a correct root password gave a root session. | High | `candidate()` refuses uid 0 whoever runs the server. | Unit test; login with the real root password rejected in the VM. |
| 2 | **Sessions inherited the helper's sandbox**: `NoNewPrivileges` (no `sudo`, `su`, `pkexec`), a seccomp filter and `RestrictNamespaces` (no Flatpak, no browser sandboxes) in every user's terminal. Also all sessions sat in the helper's cgroup. | High (function and design) | Sessions are `sway-rdp-session@<uid>.service` units with no sandbox, in the account's own `user-<uid>.slice`. | `NoNewPrivs: 0`, `Seccomp: 0`, cgroup `user-1001.slice/sway-rdp-session@1001.service`. |
| 3 | **A session could crash the daemon.** A bitmap frame shorter than its rectangle, or a rectangle outside the picture, went straight to the RDP encoder. One hostile account could take the daemon (and so every user) down. | High | Every frame is checked (rectangle inside the current size, bitmap length exact, H.264 size bounded); sizes must be ones `clamp_size` can produce; anything else ends that session's link. | Unit tests of the checks. |
| 4 | **A session could exhaust the daemon's memory**: frames were queued without limit, and a session may send without credits. | High | Bounded queue (8 events): a fast session is slowed down by its own socket. | By construction; not load-tested. |
| 5 | **Idle connections could use up all slots** (8 by default) with no login. | High (network DoS) | A connection that has not logged in within 30 s is cut; default limit 64. | An idle TCP connection was closed after 30 s. |
| 6 | **Any local account could flood the helper's registration socket** with connections (a thread each, in a root process). | Medium (local DoS) | At most 4 per account and 256 in all. | 12 connections as one account: 9 refused. |
| 7 | Sessions were unreachable after a helper restart (upgrade). | Robustness | Sessions register again after the helper comes back. | Restarted the helper; same session picked up at the next login. |

## Checked and found as intended

- The helper's socket is created owner-only, handed to the daemon's account, and each connection is checked with
  `SO_PEERCRED`. Another account got "Permission denied".
- The helper starts sessions only for the account that authenticated on the same connection, and takes no program,
  path or arguments from the daemon. A session request before or after failed logins is refused.
- Identity switch order is `setgroups`, `setgid`, `setuid`, then a cleared environment.
- The helper never connects to a path an account controls (sessions connect to the helper), so a planted symlink cannot
  point root at another socket. A session can only register as the account the kernel says it is.
- Descriptors received are `CLOEXEC`. Messages have size limits and strict decoding (tests for truncation, trailing
  bytes, unknown tags, oversize and non-UTF-8 strings).
- Failures are counted per account name and per source address; unknown accounts and wrong passwords get the same answer
  and delay. Passwords are not logged.

## Open (not fixed)

1. **Plain TLS login.** The password reaches the daemon in clear inside TLS (PAM cannot do NLA). A compromised daemon
   sees passwords, can read the streams of live sessions and type into them. Mitigation is the network (VPN or tunnel) and
   the daemon sandbox, not the code. The certificate is self-signed; a client that does not check the fingerprint can
   be intercepted.
2. **The helper trusts the daemon's `source` address**, so a compromised daemon can dodge or poison the per-address
   counters. The per-account counter still holds.
3. **Failed logins tie up helper threads** (2 s delay each, 64 connections at once): many sources failing at the same time
   can starve legitimate logins. The per-address lockout only helps against few sources.
4. **No per-account resource limits by default.** Sessions are in `user-<uid>.slice`, so `user-.slice.d` drop-ins work,
   for example `MemoryMax=` and `TasksMax=`, but the package ships none.
5. **Passwords are not wiped from memory** in the daemon or the helper.
6. **The helper is the same binary as the daemon** (everything in it is mapped, only some of it runs as root); a separate
   small crate would be easier to audit.
7. **No second factor.** The RDP form has one password field and the PAM conversation answers every prompt with it.
8. **Slow clients after login** are not cut off (only the pre-login phase has a deadline), and there is no per-address
   connection limit.
9. **Keys held down when a client is replaced stay pressed.** Only a convenience bug.
10. **Not done:** fuzzing the RDP side (IronRDP) and the PAM conversation, `systemd-analyze security` scoring, Lintian,
    testing the amd64 build, any review by someone else.

## What would make a first external review useful

The three entry points: `src/helper.rs` and `src/auth.rs` (root), `src/link.rs` (the daemon's handling of an untrusted
session), and `packaging/systemd/*.service`. About 900 lines of Rust plus the units; the protocol is in `src/proto.rs`.

---

# Second pass (2026-10-20)

Who wrote it: the author of the code, with an AI assistant, reading the code again after the changes of that day (sound for
sessions of their own, the pointer, the desktop packaging). **Still not an independent review.** No active penetration testing
was done in this pass: the checks below are code reading, unit tests, `systemd-analyze security` and logins in the development
VM. Nothing here changes the stated deployment: loopback, an SSH tunnel or a VPN.

## Fixed in this pass

| # | Finding | Fix | Checked |
|---|---|---|---|
| 8 | **Sound from a session was passed on unchecked.** The new sound relay forwarded every chunk a session sent to the client's channel; a session is not trusted, and the channel's queue has no limit, so sound could have been queued in the daemon without bound (the same class as finding 4). | `AudioGate` in `link.rs`: only chunks of exactly 20 ms, at most 100 a second (a real stream sends 50). Everything else is dropped. | Unit test `sound_chunks_of_a_session_are_limited_in_size_and_speed`. |
| 9 | **Any ordinary account got a desktop, whatever its login shell.** A desktop session gives the account a terminal and a file manager, so an account that sshd or the console would refuse (a file-transfer or service account with `nologin`) was given more than its administrator meant. | `shell_allows_login` in `auth.rs`: the shell must be listed in `/etc/shells` and must not be `nologin`, `false` and the like. Applies to every account, also with `--allow-users`. | Unit test; in the VM a correct password of an account with `/usr/sbin/nologin` was rejected (`login rejected`), a normal account accepted. |
| 10 | **Names from the network were logged as typed**, so a name with a line break could look like a second log line. | The account name and source address are logged with `Debug` formatting (quoted and escaped). | Journal lines now read `user="rdptest3" source="10.0.2.2"`. |
| 11 | **The root helper kept more than it needs**: network access, devices, the clock, the kernel log, writable homes, writable and executable memory, all capabilities. | `sway-rdp-helper.service`: `RestrictAddressFamilies=AF_UNIX`, `IPAddressDeny=any`, `PrivateDevices`, `ProtectClock`, `ProtectHostname`, `ProtectHome=read-only`, `ProtectKernelLogs`, `MemoryDenyWriteExecute`, and a capability bound of `CAP_CHOWN CAP_DAC_READ_SEARCH CAP_AUDIT_WRITE CAP_SETUID CAP_SETGID`. | `systemd-analyze security`: **7.2 MEDIUM to 4.0 OK** (the daemon stays at 1.8). After the change a login works and the helper starts a new session unit. |
| 12 | **The Flatpak polkit rule was wide**: members of `sudo` could run every system Flatpak action without a password in an active remote session, including sideloading bundles and changing remotes; weaker than `sudo` asking for a password. | `40-sway-rdp-flatpak.rules`: only members of the new group `sway-rdp-admins` (empty at first, made by the package; `adduser ACCOUNT sway-rdp-admins`), and only install, update, uninstall and metadata actions. Sideloading, remotes and configuration always ask. | `pkcheck` inside active sessions: a member of the group gets `app-install` without a password and still a password for `modify-repo` and `install-bundle`; a `sudo` member outside the group and an ordinary account get a password for all three. |

## Checked again against the current code

The claims of the first review about the helper, the registration socket and the login policy still hold: root is refused whoever
runs the server; a session is started only for the account that authenticated on the same connection and the helper takes no
program or arguments from the daemon; the identity switch is `setgroups`, `setgid`, `setuid`; registration is refused below uid
1000, counted per account and in all, and the account is the one the kernel reports; received descriptors are `CLOEXEC`; messages
have size limits and strict decoding. The daemon unit is sandboxed (no capabilities, a syscall filter, `ProtectSystem=strict`).

## Open, with the change that would close it

1. **Counters are per account name and per source address.** Both are easy to share (a known account name; one VPN or NAT address
   for a whole office), so the limits protect passwords but can also keep a real user out for a minute at a time. Options:
   count per (account, source) pair as well as per account, back off longer after repeated lockouts, and add `pam_faillock` to the
   PAM stack of the service. Needs a decision on what to protect first: availability or guessing.
2. **Every ordinary account with a login shell may log in.** An opt-in group (for example `sway-rdp-users`) as the default would
   make the set of accounts explicit; `--allow-users` exists but is a list of names.
3. (Closed: see finding 12.)
4. **The helper unit can go further**: a system call filter and a narrower capability set, checked with a real PAM stack. A PAM stack
   that needs the network by itself (`pam_ldap`, not `sssd`) needs `IPAddressDeny=any` and `RestrictAddressFamilies` lifted in a drop-in.
5. **No resource limits per account or for the daemon by default** (`MemoryMax`, `TasksMax`, `LimitNOFILE`): a `user-.slice.d` drop-in
   and limits on `sway-rdp.service` should be shipped or documented.
6. **Clipboard messages from a session are limited in size but not in number.** A gate like the one for sound would close it.
7. **The installer adds third-party apt repositories** (DankMaterialShell from the openSUSE build service, trixie-backports) and
   fetches their keys over HTTPS without pinning a fingerprint. That is a supply chain decision to write down, or to pin.
8. **The development VM listens on `0.0.0.0` inside the VM** (only the loopback forward of the host reaches it). Not for production.
9. Still open from the first pass: plain TLS login, the daemon-supplied source address, passwords not wiped from memory, one binary
   for daemon and helper, no second factor, no deadline for slow clients after login, no fuzzing of the RDP side, amd64 untested, and
   no review by someone else. An independent review is the next real step: it should be given `src/helper.rs`, `src/auth.rs`,
   `src/link.rs`, `src/proto.rs` and `packaging/systemd/*.service`.
