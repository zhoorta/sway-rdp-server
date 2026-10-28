# waydesk: first security review (2026-10-16)

Who wrote it: the author of the code, with an AI assistant, walking `docs/MULTIUSER-DESIGN.md` section 7 against the code
and then testing what could be tested in a VM. **This is not an independent review and not an audit.** Nothing here
justifies exposing the server to the internet. The stated deployment is: loopback, an SSH tunnel or a VPN.

Trust model used: the *network* is hostile; the *daemon* (`waydesk`, unprivileged) is the network-facing part and is
assumed to be compromisable; the *helper* (root) and the packaged files are trusted; a *logged-in account* is trusted
with itself and with nothing else; its session process is treated as hostile by the daemon.

## Found and fixed in this review

| # | Finding | Severity | Fix | Checked |
|---|---|---|---|---|
| 1 | **Root could log in over RDP.** The rule "the account running the server may log in" matched `root` when the helper runs as root, so a correct root password gave a root session. | High | `candidate()` refuses uid 0 whoever runs the server. | Unit test; login with the real root password rejected in the VM. |
| 2 | **Sessions inherited the helper's sandbox**: `NoNewPrivileges` (no `sudo`, `su`, `pkexec`), a seccomp filter and `RestrictNamespaces` (no Flatpak, no browser sandboxes) in every user's terminal. Also all sessions sat in the helper's cgroup. | High (function and design) | Sessions are `waydesk-session@<uid>.service` units with no sandbox, in the account's own `user-<uid>.slice`. | `NoNewPrivs: 0`, `Seccomp: 0`, cgroup `user-1001.slice/waydesk-session@1001.service`. |
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
| 11 | **The root helper kept more than it needs**: network access, devices, the clock, the kernel log, writable homes, writable and executable memory, all capabilities. | `waydesk-helper.service`: `RestrictAddressFamilies=AF_UNIX`, `IPAddressDeny=any`, `PrivateDevices`, `ProtectClock`, `ProtectHostname`, `ProtectHome=read-only`, `ProtectKernelLogs`, `MemoryDenyWriteExecute`, and a capability bound of `CAP_CHOWN CAP_DAC_READ_SEARCH CAP_AUDIT_WRITE CAP_SETUID CAP_SETGID`. | `systemd-analyze security`: **7.2 MEDIUM to 4.0 OK** (the daemon stays at 1.8). After the change a login works and the helper starts a new session unit. |
| 12 | **The Flatpak polkit rule was wide**: members of `sudo` could run every system Flatpak action without a password in an active remote session, including sideloading bundles and changing remotes; weaker than `sudo` asking for a password. | `40-waydesk-flatpak.rules`: only members of the new group `waydesk-admins` (empty at first, made by the package; `adduser ACCOUNT waydesk-admins`), and only install, update, uninstall and metadata actions. Sideloading, remotes and configuration always ask. | `pkcheck` inside active sessions: a member of the group gets `app-install` without a password and still a password for `modify-repo` and `install-bundle`; a `sudo` member outside the group and an ordinary account get a password for all three. |
| 13 | **Every ordinary account with a login shell could log in**: the set of accounts that get a desktop was "whoever has an account", and an upgrade or a new account widened it without anyone deciding. | `--require-group NAME` for the helper (and `--auth pam`): the account must be in that group, and a group that does not exist lets nobody in (with a warning at start). The packaged unit defaults to the group `waydesk-users`, made by the package; a first install adds the account that ran it. `/etc/default/waydesk` can clear or change it (`WAYDESK_HELPER_ARGS=`). | Unit tests; in the VM a member was accepted and a valid account outside the group refused; with the setting cleared the outsider was accepted, with it restored refused again. |
| 14 | **The failure counters could be used against the people they protect, and could be wiped.** Counters were per account name and per address, a lockout was a flat minute, and one successful login reset the address counter. A stranger who knew an account name could keep its owner out; the flat minute allowed about 7,000 guesses a day per account; one login with an account of one's own cleared the brake on an address guessing at others. | Counters per (account, address) pair, so someone elsewhere cannot lock an owner out of their own address; lockouts that double each time (60 s up to one hour) and start again after an hour of quiet; a success eases the address counter by one failure and does not reset it; a warning in the log once per ten minutes when one account has 20 failures from any addresses. Keys carry the length of the name, so two pairs cannot be written the same way. | Five unit tests with time injected (exact escalation 60, 120, 240, 480 s; owner unaffected by a stranger; ease by one; the warning once per window); a login still works in the VM. Not tested live with a burst of failures. |
| 15 | **The root helper had no system call filter.** | `waydesk-helper.service`: `SystemCallFilter=@system-service` with `SystemCallErrorNumber=EPERM` (a refused call fails, it does not kill the helper). `@privileged` stays allowed because the helper needs chown and setuid/setgid. `systemd-analyze security` exposure 4.0 to 2.6. | On the dev VM (arm64): a wrong password is rejected through PAM, a right one is accepted and the session unit starts and streams; no seccomp record in the kernel log. Not tried with other PAM stacks (sssd, ldap) or on amd64. |
| 16 | **No resource limits.** A leak or flood in the daemon or helper, or a runaway program or fork bomb in one account, could take the whole host. | `waydesk.service`: `MemoryMax=1G`, `TasksMax=512`, `LimitNOFILE=8192`. `waydesk-helper.service`: `MemoryMax=256M`, `TasksMax=256`, `LimitNOFILE=4096`. New `user-.slice.d/50-waydesk.conf` (applies to every account on the machine, not only RDP ones): `TasksMax=8192`, `MemoryHigh=80%`, `MemoryMax=90%`. | Values read back with `systemctl show` on the dev VM. Daemon and helper use 4 MB and 1.5 MB idle; their limits are not measured under load with many clients. The user-slice values are static per account, so they do not share RAM fairly between accounts. |

## Checked again against the current code

The claims of the first review about the helper, the registration socket and the login policy still hold: root is refused whoever
runs the server; a session is started only for the account that authenticated on the same connection and the helper takes no
program or arguments from the daemon; the identity switch is `setgroups`, `setgid`, `setuid`; registration is refused below uid
1000, counted per account and in all, and the account is the one the kernel reports; received descriptors are `CLOEXEC`; messages
have size limits and strict decoding. The daemon unit is sandboxed (no capabilities, a syscall filter, `ProtectSystem=strict`).

## Open, with the change that would close it

1. **Account-wide guessing from many addresses is only logged, not stopped.** Per (account, address) limits (finding 14) cannot see
   a guess spread over many addresses. A hard cap per account would again let a stranger lock the owner out, so the choice is
   left to the administrator: `pam_faillock` in the PAM stack of the service does it, with that trade-off. A second factor
   would close it properly.
2. (Closed: see finding 13.)
3. (Closed: see finding 12.)
4. (Closed: see finding 15. The capability set was already narrow; a system call filter is added.)
5. (Closed: see finding 16.)
6. **Clipboard messages from a session are limited in size but not in number.** A gate like the one for sound would close it.
7. **The installer adds third-party apt repositories** (DankMaterialShell from the openSUSE build service, trixie-backports) and
   fetches their keys over HTTPS without pinning a fingerprint. That is a supply chain decision to write down, or to pin.
8. **The development VM listens on `0.0.0.0` inside the VM** (only the loopback forward of the host reaches it). Not for production.
9. Still open from the first pass: plain TLS login, the daemon-supplied source address, passwords not wiped from memory, one binary
   for daemon and helper, no second factor, no deadline for slow clients after login, no fuzzing of the RDP side, amd64 untested, and
   no review by someone else. An independent review is the next real step: it should be given `src/helper.rs`, `src/auth.rs`,
   `src/link.rs`, `src/proto.rs` and `packaging/systemd/*.service`.

---

# Third pass (2026-10-21)

Who wrote it: the author of the code, with an AI assistant, reading what the first two passes did not: `waydesk-admin` and its
polkit action, the Users & Access panel, both sides of the clipboard, the three units, `postinst`, the PAM files, the Flatpak
rule and the dev VM files. **Still not an independent review**, and no active testing: code reading only. Nothing here changes
the stated deployment (loopback, an SSH tunnel or a VPN).

## Fixed in this pass

| # | Finding | Fix | Checked |
|---|---|---|---|
| 17 | **Clipboard texts from a session were limited in size but not in number.** Each one is announced to the client, so a session copying in a loop could flood the client and use daemon time that belongs to everybody (memory stays bounded: only the last text is kept). | `ClipboardGate` in `link.rs`: at most 10 texts a second, each at most `MAX_CLIPBOARD`; the rest are dropped. | Unit test `clipboard_texts_of_a_session_are_limited_in_size_and_speed`. **Not run yet**: the Mac's Cargo (1.76) cannot read the v4 lock file; run `cargo test link` in the VM. |

## Checked and found as intended

- `waydesk-admin`: every name is validated (nothing that could be an option or a path), tools are started by absolute path with
  no shell, passwords only travel over a pipe to `chpasswd`, root and uid below 1000 are never touched, one cannot remove or
  demote oneself or the last `sudo` member. The panel passes its arguments to `pkexec` as an array, and the tool validates
  them again. The polkit action is pinned to `/usr/bin/waydesk-admin`, `auth_admin`, no GUI.
- The first-install private key is not exposed: `openssl req -keyout` creates it with mode 0600.
- The units match what the first two passes say of them.

## Noted, not changed

1. A client that sends clipboard announcements during the first 5 seconds of a connection makes a sleeping thread for each
   (`on_remote_copy`). Only an authenticated client can, and it ends after 5 s.
2. `user-.slice.d/50-waydesk.conf` (`MemoryMax=90%`, `TasksMax=8192`) applies to every account, root's and administrators'
   SSH sessions too. An operational trade-off; worth a line in the README.
3. `waydesk-admin users add` gives the account a real random password, so with `PasswordAuthentication yes` in sshd it can
   also log in over SSH from the start. By design; the command's output could say so.
4. Still open from before: plain TLS login, unpinned third-party apt keys, no second factor, no fuzzing of the RDP side,
   amd64 untested, no review by someone else.

---

# Fourth pass (2026-10-21): the machine's memory

Who wrote it: the author of the code, with an AI assistant. Still not an independent review. Found while measuring: a 4 GB machine was taken down twice by
the kernel's out-of-memory killer with five accounts logged in; a user (or a handful of logins) could do the same to everybody.

| # | Finding | Fix | Checked |
|---|---|---|---|
| 18 | **Any set of logins, or one runaway account, could exhaust the memory of the machine** and with it every session (a denial of service by accounts that are allowed to log in). | The root helper refuses a new session when less than max(900 MB, 15 %) of the memory is available or the optional limit of active users is reached (an account with a running session just reconnects); every account's slice has ceilings of 35 % / 50 % of the memory, 50 % in swap and 8192 tasks; zram swap with zstd. | Unit tests of the rule; on the VM a new account was refused (journal: `session refused`) while the running one reconnected; five users fit and the sixth and seventh were refused with no kill. |

Noted: `waydesk-admin status` and the account list are open to every local account, like `/proc` is: they show the names of the programs and the memory of the other accounts
(not their files or what is in them). `users limit` and `users limit ... reset` need root; sizes, weights and task numbers are checked before they reach `systemctl`
(unit tests: no option, unit or shell syntax gets through). What a session runs (renderer, encoder, quantiser, frame rate; nothing about the client) is written by its worker to `/run/waydesk/status/UID.json`, in a directory the helper makes sticky and world-writable like `/tmp`: the file is mode 0600 and owned by the account, `status` takes only a file owned by the uid it is named for, and an account can neither read another's nor replace it. A refused login is only a failed login for the RDP client; nothing tells it why, which is also nothing for an attacker to learn.

Correction to finding 12 (second pass), found on 2026-10-21 when an administrator of the group `waydesk-admins` was asked for a password in the middle of installing an app: the
passwordless rule left out `org.freedesktop.Flatpak.modify-repo`, and the second pass described that action as "changing remotes". It is not: Flatpak's system helper asks for it
for the housekeeping of its own local repository during every install, update and uninstall (`EnsureRepo`, `GetRevokefsFd`, `RunTriggers`, `PruneLocalRepo`, `RemoveLocalRef`,
`CancelPull`; changing a remote is `configure-remote`). It is now in the rule (`40-waydesk-flatpak.rules`). Still asking for a password: `configure-remote`, `configure`,
`install-bundle`, and the parental-controls actions. The privilege of a member does not grow in a way that matters: a member could already deploy any app of the configured
remotes system-wide without a password, which is the larger power. Checked on the VM: an install of Sublime Text (and its runtime) from the member's own session finished
without a prompt (`flatpak install --noninteractive`, exit 0), where before it failed with "GetRevokefsFd not allowed for user".
