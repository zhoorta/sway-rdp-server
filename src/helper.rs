//! The root helper (`sway-rdp-server --helper`): the only part that runs with root rights, and the only part that
//! does what needs them: check a password for any account (PAM) and start a session process as that account.
//! It speaks the small protocol of `proto::HelperReq` on a Unix socket that only the daemon's account can open, and
//! never touches the network or the RDP protocol.
//!
//! Rules it enforces, whatever the daemon says:
//! - a session is started only for the account that authenticated on the same connection;
//! - one login and one session per connection;
//! - what is started and with which arguments is the helper's own configuration.

use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::Child;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{ensure, Context as _, Result};
use tracing::{info, warn};

use crate::auth::{lookup_user, PamPolicy, UserInfo};
use crate::fdpass;
use crate::proto::{self, HelperReq, HelperResp};
use crate::spawn::spawn_worker;

pub struct Config {
    pub socket: PathBuf,
    /// The only account that may connect.
    pub daemon_user: String,
    pub allow_users: Option<Vec<String>>,
    pub exe: PathBuf,
    pub worker_args: Vec<String>,
    pub workspace: String,
}

/// Connections served at once.
const MAX_CONNECTIONS: usize = 64;

pub fn run(cfg: Config) -> Result<()> {
    ensure!(unsafe { libc::geteuid() } == 0, "the helper has to run as root");
    let daemon = lookup_user(&cfg.daemon_user).with_context(|| format!("no account named {}", cfg.daemon_user))?;
    ensure!(daemon.uid != 0, "the daemon must not be root");
    let policy = PamPolicy::new("sway-rdp-server", cfg.allow_users.clone())?;

    if let Some(dir) = cfg.socket.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let _ = std::fs::remove_file(&cfg.socket);
    // Owner only from the first moment; then handed to the daemon's account.
    let old = unsafe { libc::umask(0o177) };
    let listener = UnixListener::bind(&cfg.socket).with_context(|| format!("cannot listen on {}", cfg.socket.display()));
    unsafe { libc::umask(old) };
    let listener = listener?;
    let path = std::ffi::CString::new(cfg.socket.as_os_str().as_encoded_bytes())?;
    ensure!(unsafe { libc::chown(path.as_ptr(), daemon.uid, daemon.gid) } == 0, "chown of the socket failed");
    info!(socket = %cfg.socket.display(), daemon = %daemon.name, "helper listening");

    let cfg = Arc::new(cfg);
    let active = Arc::new(AtomicUsize::new(0));
    for conn in listener.incoming() {
        let sock = match conn {
            Ok(s) => s,
            Err(e) => {
                warn!("accept failed: {e}");
                continue;
            }
        };
        match peer_uid(&sock) {
            Some(uid) if uid == daemon.uid => {}
            other => {
                warn!(?other, "refused a connection from another account");
                continue;
            }
        }
        if active.fetch_add(1, Ordering::AcqRel) >= MAX_CONNECTIONS {
            active.fetch_sub(1, Ordering::AcqRel);
            warn!("too many connections, refused");
            continue;
        }
        let (cfg, policy, active) = (cfg.clone(), policy.clone(), active.clone());
        std::thread::spawn(move || {
            serve(sock, &cfg, &policy);
            active.fetch_sub(1, Ordering::AcqRel);
        });
    }
    Ok(())
}

/// The uid of the process on the other end (SO_PEERCRED).
fn peer_uid(sock: &UnixStream) -> Option<u32> {
    let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let rc = unsafe { libc::getsockopt(sock.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PEERCRED, (&mut cred as *mut libc::ucred).cast(), &mut len) };
    (rc == 0).then_some(cred.uid)
}

fn reply(sock: &mut UnixStream, resp: &HelperResp) -> bool {
    proto::write_frame(sock, &resp.encode()).is_ok()
}

/// One daemon connection: one client of the daemon.
fn serve(mut sock: UnixStream, cfg: &Config, policy: &PamPolicy) {
    let mut who: Option<UserInfo> = None;
    let mut child: Option<Child> = None;
    loop {
        let body = match proto::read_frame(&mut sock) {
            Ok(Some(b)) => b,
            _ => break,
        };
        let req = match HelperReq::decode(&body) {
            Ok(r) => r,
            Err(e) => {
                warn!("bad request from the daemon: {e}");
                break;
            }
        };
        let ok = match req {
            HelperReq::Authenticate { user, password, source } => {
                if who.is_some() {
                    reply(&mut sock, &HelperResp::Error { message: "already logged in on this connection".into() })
                } else {
                    match policy.authenticate(&user, &password, &source) {
                        Some(info) => {
                            who = Some(info);
                            reply(&mut sock, &HelperResp::AuthOk)
                        }
                        None => reply(&mut sock, &HelperResp::AuthFail),
                    }
                }
            }
            HelperReq::StartSession => match (&who, &child) {
                (None, _) => reply(&mut sock, &HelperResp::Error { message: "not logged in".into() }),
                (_, Some(_)) => reply(&mut sock, &HelperResp::Error { message: "a session is already running".into() }),
                (Some(user), None) => match spawn_worker(&cfg.exe, &cfg.worker_args, &cfg.workspace, Some(user), true) {
                    Ok((c, ours)) => {
                        child = Some(c);
                        let sent = reply(&mut sock, &HelperResp::Started) && fdpass::send_fd(&sock, ours.as_raw_fd()).is_ok();
                        // Our copy must go: the session sees the end of the stream only when the daemon's is the last.
                        drop(ours);
                        sent
                    }
                    Err(e) => {
                        warn!(user = %user.name, "cannot start the session: {e:#}");
                        reply(&mut sock, &HelperResp::Error { message: format!("{e:#}") })
                    }
                },
            },
            HelperReq::Bye => false,
        };
        if !ok {
            break;
        }
    }
    // The daemon is gone (or finished): the session sees the end of its stream and cleans up; wait for it.
    if let Some(mut c) = child {
        for _ in 0..300 {
            if matches!(c.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        warn!(pid = c.id(), "session process did not end in 15 s; killing it");
        let _ = c.kill();
        let _ = c.wait();
    }
}
