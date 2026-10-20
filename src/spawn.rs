//! Starting a session process (`waydesk-server --session-worker`) with one end of a socket pair as its fd 3.

use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};

use anyhow::{bail, Context as _, Result};

use crate::auth::UserInfo;

/// Starts `exe --session-worker args... --workspace NAME [--own-sway]` and returns the process and the parent's end
/// of the socket pair.
///
/// `user`: the account the session belongs to. When it is not the account running this process, this process has to
/// be root: the session then starts as that account (groups, gid, uid, a clean environment). `own_sway`: the session
/// starts its own headless sway instead of using the one it was started in.
///
/// `register`: None for a session of one client, which gets one end of a socket pair as fd 3 (returned); Some(path) for a
/// persistent session, which registers at the helper's socket there and gets its clients from it (nothing returned).
pub fn spawn_worker(exe: &Path, args: &[String], workspace: &str, user: Option<&UserInfo>, own_sway: bool, register: Option<&Path>) -> Result<(Child, Option<UnixStream>)> {
    let switch = user.filter(|u| u.uid != unsafe { libc::getuid() });
    if switch.is_some() && unsafe { libc::getuid() } != 0 {
        bail!("starting a session as another account needs the daemon to run as root");
    }
    if own_sway {
        if let Some(u) = switch {
            let dir = format!("/run/user/{}", u.uid);
            if !Path::new(&dir).is_dir() {
                bail!("{dir} does not exist: the account needs a runtime directory (loginctl enable-linger {})", u.name);
            }
        }
    }
    let (parent, child_end) = match register {
        None => {
            let (p, c) = UnixStream::pair().context("socket pair")?;
            (Some(p), Some(c))
        }
        Some(_) => (None, None),
    };
    let child_fd = child_end.as_ref().map(AsRawFd::as_raw_fd);
    let mut cmd = Command::new(exe);
    cmd.arg("--session-worker").args(args).args(["--workspace", &workspace]).stdin(Stdio::null());
    if let Some(path) = register {
        cmd.arg("--register").arg(path);
    }
    if own_sway {
        cmd.arg("--own-sway");
    }
    let ids = switch.map(|u| (u.uid, u.gid, u.groups.clone()));
    if let Some(u) = switch {
        cmd.env_clear()
            .env("PATH", "/usr/local/bin:/usr/bin:/bin")
            .env("HOME", &u.home)
            .env("USER", &u.name)
            .env("LOGNAME", &u.name)
            .env("XDG_RUNTIME_DIR", format!("/run/user/{}", u.uid))
            .env("LANG", "C.UTF-8")
            .current_dir(&u.home);
        if let Ok(level) = std::env::var("WAYDESK_LOG") {
            cmd.env("WAYDESK_LOG", level);
        }
    }
    // SAFETY: only async-signal-safe calls (setgroups, setgid, setuid, dup2, fcntl) between fork and exec.
    unsafe {
        cmd.pre_exec(move || {
            if let Some((uid, gid, groups)) = &ids {
                if libc::setgroups(groups.len(), groups.as_ptr()) != 0 || libc::setgid(*gid) != 0 || libc::setuid(*uid) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            let ok = match child_fd {
                None => true,
                Some(3) => libc::fcntl(3, libc::F_SETFD, 0) >= 0,
                Some(fd) => libc::dup2(fd, 3) >= 0,
            };
            if ok {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        });
    }
    let child = cmd.spawn().with_context(|| format!("cannot start {}", exe.display()))?;
    drop(child_end);
    Ok((child, parent))
}
