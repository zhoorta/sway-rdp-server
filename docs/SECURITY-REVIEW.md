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
