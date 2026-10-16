//! The root helper (`sway-rdp-server --helper`): the only part that runs with root rights, and the only part that
//! does what needs them: check a password for any account (PAM) and start a session process as that account.
//! It speaks the small protocol of `proto::HelperReq` on a Unix socket that only the daemon's account can open, and
//! never touches the network or the RDP protocol.
//!
//! Rules it enforces, whatever the daemon says:
//! - a session is started only for the account that authenticated on the same connection;
//! - one login per connection;
//! - what is started and with which arguments is the helper's own configuration.
//!
//! Sessions outlive connections: a session process registers at the helper's second socket (`sessions.sock`, which any
//! local account may open) right after it starts; the kernel tells the helper which account is on the other end, so a
//! session can only ever register as its own account. A reconnecting client gets a new socket pair: one end goes to
//! the session process over its registration, the other to the daemon. The helper never connects to a path inside an
//! account's directories (a symlink there could point root at something else).

use std::collections::HashMap;
use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::Child;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

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
/// How long a new session process may take to register.
const REGISTER_TIMEOUT: Duration = Duration::from_secs(20);

/// A running session process, as it registered.
struct Registered {
    /// Our end of its registration socket: client sockets are sent through it.
    sock: UnixStream,
    generation: u64,
}

/// The sessions that exist, by account, and the processes started for them.
struct Sessions {
    by_uid: Mutex<HashMap<u32, Registered>>,
    registered: Condvar,
    next_generation: AtomicU64,
    /// One at a time per account while a session is looked up or started.
    gates: Mutex<HashMap<u32, Arc<Mutex<()>>>>,
    /// Session processes, to be reaped when they end.
    children: Mutex<Vec<Child>>,
}

impl Sessions {
    fn gate(&self, uid: u32) -> Arc<Mutex<()>> {
        self.gates.lock().unwrap().entry(uid).or_default().clone()
    }

    /// A socket to the running session of `uid`, if there is one that still answers.
    fn attach(&self, uid: u32) -> Option<UnixStream> {
        let mut map = self.by_uid.lock().unwrap();
        let reg = map.get(&uid)?;
        let (ours, theirs) = UnixStream::pair().ok()?;
        if fdpass::send_fd(&reg.sock, theirs.as_raw_fd()).is_ok() {
            return Some(ours);
        }
        // The process behind the registration is gone.
        map.remove(&uid);
        None
    }

    fn reap(&self) {
        self.children.lock().unwrap().retain_mut(|c| matches!(c.try_wait(), Ok(None)));
    }
}

pub fn run(cfg: Config) -> Result<()> {
    ensure!(unsafe { libc::geteuid() } == 0, "the helper has to run as root");
    let daemon = lookup_user(&cfg.daemon_user).with_context(|| format!("no account named {}", cfg.daemon_user))?;
    ensure!(daemon.uid != 0, "the daemon must not be root");
    let policy = PamPolicy::new("sway-rdp-server", cfg.allow_users.clone())?;

    let dir = cfg.socket.parent().context("--helper-socket needs a directory")?.to_path_buf();
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let register_path = dir.join("sessions.sock");

    // The daemon's socket: owner only from the first moment, then handed to the daemon's account.
    let listener = bind(&cfg.socket, 0o177)?;
    let path = std::ffi::CString::new(cfg.socket.as_os_str().as_encoded_bytes())?;
    ensure!(unsafe { libc::chown(path.as_ptr(), daemon.uid, daemon.gid) } == 0, "chown of the socket failed");
    // The registration socket: any local account may open it (session processes run as their accounts).
    let registrations = bind(&register_path, 0o000)?;
    info!(socket = %cfg.socket.display(), registrations = %register_path.display(), daemon = %daemon.name, "helper listening");

    let sessions = Arc::new(Sessions {
        by_uid: Mutex::new(HashMap::new()),
        registered: Condvar::new(),
        next_generation: AtomicU64::new(1),
        gates: Mutex::new(HashMap::new()),
        children: Mutex::new(Vec::new()),
    });

    {
        let sessions = sessions.clone();
        std::thread::spawn(move || {
            for conn in registrations.incoming().flatten() {
                let sessions = sessions.clone();
                std::thread::spawn(move || registration(conn, &sessions));
            }
        });
    }
    {
        let sessions = sessions.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_secs(1));
            sessions.reap();
        });
    }

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
        let (cfg, policy, sessions, active, register_path) = (cfg.clone(), policy.clone(), sessions.clone(), active.clone(), register_path.clone());
        std::thread::spawn(move || {
            serve(sock, &cfg, &policy, &sessions, &register_path);
            active.fetch_sub(1, Ordering::AcqRel);
        });
    }
    Ok(())
}

/// Binds a Unix socket created with the given umask (so it is never briefly more open than intended); a mask of 0
/// leaves it world-connectable.
fn bind(path: &std::path::Path, umask: libc::mode_t) -> Result<UnixListener> {
    let _ = std::fs::remove_file(path);
    let old = unsafe { libc::umask(umask) };
    let listener = UnixListener::bind(path).with_context(|| format!("cannot listen on {}", path.display()));
    unsafe { libc::umask(old) };
    listener
}

/// The uid of the process on the other end (SO_PEERCRED).
fn peer_uid(sock: &UnixStream) -> Option<u32> {
    let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let rc = unsafe { libc::getsockopt(sock.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PEERCRED, (&mut cred as *mut libc::ucred).cast(), &mut len) };
    (rc == 0).then_some(cred.uid)
}

/// A session process says it is up. It never sends anything: it stays connected until it ends, and the end of
/// the connection is how we learn that it is gone.
fn registration(mut conn: UnixStream, sessions: &Sessions) {
    use std::io::Read;
    let Some(uid) = peer_uid(&conn) else { return };
    // Only ordinary accounts have sessions; root and system accounts have no business here.
    if uid < 1000 || uid == 65534 {
        warn!(uid, "registration refused");
        return;
    }
    let Ok(sock) = conn.try_clone() else { return };
    let generation = sessions.next_generation.fetch_add(1, Ordering::Relaxed);
    sessions.by_uid.lock().unwrap().insert(uid, Registered { sock, generation });
    sessions.registered.notify_all();
    info!(uid, "session registered");
    let mut buf = [0u8; 64];
    while matches!(conn.read(&mut buf), Ok(n) if n > 0) {}
    let mut map = sessions.by_uid.lock().unwrap();
    if map.get(&uid).is_some_and(|r| r.generation == generation) {
        map.remove(&uid);
    }
    info!(uid, "session ended");
}

fn reply(sock: &mut UnixStream, resp: &HelperResp) -> bool {
    proto::write_frame(sock, &resp.encode()).is_ok()
}

/// The running session of the account, or a new one (started and waited for until it registers).
fn session_for(user: &UserInfo, cfg: &Config, sessions: &Sessions, register_path: &std::path::Path) -> Result<UnixStream> {
    let gate = sessions.gate(user.uid);
    let _one_at_a_time = gate.lock().unwrap();
    if let Some(s) = sessions.attach(user.uid) {
        info!(user = %user.name, "reconnecting to the running session");
        return Ok(s);
    }
    let (child, _) = spawn_worker(&cfg.exe, &cfg.worker_args, &cfg.workspace, Some(user), true, Some(register_path))?;
    info!(user = %user.name, pid = child.id(), "session process started");
    sessions.children.lock().unwrap().push(child);

    let deadline = Instant::now() + REGISTER_TIMEOUT;
    let mut map = sessions.by_uid.lock().unwrap();
    while !map.contains_key(&user.uid) {
        let left = deadline.saturating_duration_since(Instant::now());
        ensure!(!left.is_zero(), "the session process did not register in {} s", REGISTER_TIMEOUT.as_secs());
        map = sessions.registered.wait_timeout(map, left).unwrap().0;
    }
    drop(map);
    sessions.attach(user.uid).context("the new session process is already gone")
}

/// One daemon connection: one client of the daemon.
fn serve(mut sock: UnixStream, cfg: &Config, policy: &PamPolicy, sessions: &Sessions, register_path: &std::path::Path) {
    let mut who: Option<UserInfo> = None;
    let mut started = false;
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
            HelperReq::StartSession => match &who {
                None => reply(&mut sock, &HelperResp::Error { message: "not logged in".into() }),
                Some(_) if started => reply(&mut sock, &HelperResp::Error { message: "a session was already handed out on this connection".into() }),
                Some(user) => match session_for(user, cfg, sessions, register_path) {
                    Ok(ours) => {
                        started = true;
                        // The session process holds its end; ours goes to the daemon, and then we let go of it.
                        reply(&mut sock, &HelperResp::Started) && fdpass::send_fd(&sock, ours.as_raw_fd()).is_ok()
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
    // Nothing to clean up here: the session belongs to the account and goes on without this connection.
}
