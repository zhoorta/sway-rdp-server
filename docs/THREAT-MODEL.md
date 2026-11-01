# Threat model and open items

Who we assume is hostile, what each part can and cannot do, and what is known to be missing. The five internal review passes are in
[`SECURITY-REVIEW.md`](SECURITY-REVIEW.md). How the port is reached (and who may connect) is in [`ACCESS.md`](ACCESS.md).

## Threat model

- **Attacker on the network:** sees only TLS. Can guess passwords; failures are counted per account and per address, delayed, and answered identically and at the same moment (4 s after the attempt began) for unknown and known accounts.
- **Compromised daemon:** runs as an unprivileged user with a system call filter and a narrow capability set. It can read the streams of live sessions and type into them, and sees passwords in transit. It cannot start a session for an account that did not authenticate on the same connection, and the helper takes no program, path or arguments from it.
- **Malicious account:** has its own sway, its own runtime directory (0700), its own clipboard and its own status folder; it cannot capture or inject into another account's session. Memory, tasks and CPU ceilings apply per account. It can use every GPU, the cards kept for inference too: GPU roles place sessions, they do not isolate them.
- **Out of scope:** drive, printer and smartcard redirection, RD Gateway, running exposed to the internet, GPU sharing.

## Open items

- Plain TLS login (above); a second factor is not available.
- The helper trusts the daemon's claim of the client's source address, so a compromised daemon can poison the per-address counters (the per-account counter still holds).
- Passwords are not wiped from memory; daemon and helper are one binary.
- Many failed logins from many addresses are logged, not stopped. `pam_faillock` in the PAM stack of the service does it, with the trade-off that a stranger can then lock the owner out.
- `waydesk-admin config check` runs an account's Hyprland check in the administrator's terminal (fifth pass, open item 1).
- Chromium from Flathub runs with `--password-store=basic`: nothing in a Waydesk session unlocks a keyring, so saved passwords and cookies sit under a fixed key, protected by the home directory's permissions only.
- No fuzzing of the RDP side, no independent review. Five internal passes are written down in [`docs/SECURITY-REVIEW.md`](https://github.com/zhoorta/waydesk/blob/main/docs/SECURITY-REVIEW.md); an outside review should start at `src/helper.rs`, `src/auth.rs`, `src/link.rs`, `src/proto.rs` and the unit files.
