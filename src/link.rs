//! The network daemon's side of the stream protocol: starts a session process for a connection, sends it what the
//! client does (input, resizes, credits) and turns what it sends back into events for the RDP side.

use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{bail, Context as _, Result};
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::auth::UserInfo;
use crate::proto::{self, Down, FrameKind, Up};

/// Numbers of the sessions that exist (1, 2, ...). The lowest free one names the session's workspace, so two
/// concurrent sessions on one sway never share (and steal) a workspace.
static USED_SLOTS: Mutex<BTreeSet<u32>> = Mutex::new(BTreeSet::new());

struct Slot(u32);

impl Slot {
    fn take() -> Self {
        let mut used = USED_SLOTS.lock().unwrap();
        let n = (1..).find(|n| !used.contains(n)).expect("slot numbers are not exhausted");
        used.insert(n);
        Slot(n)
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        USED_SLOTS.lock().unwrap().remove(&self.0);
    }
}

pub enum Event {
    Frame { x: u16, y: u16, w: u16, h: u16, kind: FrameKind, data: Vec<u8> },
    /// The picture has a new size from now on; frames before this message have the old one.
    Resized(u16, u16),
    /// The session process is gone.
    Closed,
}

pub struct Link {
    tx: mpsc::UnboundedSender<Down>,
    pub events: tokio::sync::Mutex<mpsc::UnboundedReceiver<Event>>,
    size: Mutex<(u16, u16)>,
    /// Notified when a resize has finished.
    pub resized: tokio::sync::Notify,
    child: Mutex<Option<Child>>,
    _slot: Slot,
}

impl Link {
    /// Starts `exe --session-worker args...` with one end of a socket pair as fd 3 and waits until it has its
    /// output and reports the size.
    ///
    /// `user`: the account the session belongs to. When it is not the account running the daemon, the daemon has to be
    /// root: the process then starts as that account (groups, gid, uid, a clean environment). `own_sway`: the session
    /// starts its own headless sway instead of using the one the daemon runs in.
    pub async fn spawn(exe: &Path, args: &[String], workspace: &str, size: (u16, u16), klid: u32, user: Option<&UserInfo>, own_sway: bool) -> Result<Arc<Link>> {
        let slot = Slot::take();
        // On a shared sway the first session keeps the configured name and later ones get a number; with a sway
        // of its own there is nothing to keep apart.
        let workspace = if own_sway || slot.0 == 1 { workspace.to_owned() } else { format!("{workspace} {}", slot.0) };
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
        let (parent, child_end) = std::os::unix::net::UnixStream::pair().context("socket pair")?;
        let child_fd = child_end.as_raw_fd();
        let mut cmd = Command::new(exe);
        cmd.arg("--session-worker").args(args).args(["--workspace", &workspace]).stdin(Stdio::null());
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
            if let Ok(level) = std::env::var("SWAY_RDP_LOG") {
                cmd.env("SWAY_RDP_LOG", level);
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
                let ok = if child_fd == 3 { libc::fcntl(3, libc::F_SETFD, 0) >= 0 } else { libc::dup2(child_fd, 3) >= 0 };
                if ok {
                    Ok(())
                } else {
                    Err(std::io::Error::last_os_error())
                }
            });
        }
        let child = cmd.spawn().with_context(|| format!("cannot start {}", exe.display()))?;
        drop(child_end);
        parent.set_nonblocking(true)?;
        let (mut rd, mut wr) = tokio::net::UnixStream::from_std(parent)?.into_split();

        let hello = Down::Hello { version: proto::VERSION, width: size.0, height: size.1, scale_percent: 100, klid };
        proto::write_frame_async(&mut wr, &hello.encode()).await?;
        let first = tokio::time::timeout(Duration::from_secs(30), proto::read_frame_async(&mut rd))
            .await
            .context("the session did not start in 30 s")??
            .context("the session process ended before it was ready")?;
        let Up::Ready { width, height } = Up::decode(&first)? else { bail!("first message of the session is not Ready") };
        info!(width, height, pid = child.id(), "session process ready");

        let (tx, mut tx_rx) = mpsc::unbounded_channel::<Down>();
        tokio::spawn(async move {
            while let Some(msg) = tx_rx.recv().await {
                if proto::write_frame_async(&mut wr, &msg.encode()).await.is_err() {
                    break;
                }
            }
            // `wr` is dropped here: the session sees the end of the stream and shuts down.
        });
        let (ev_tx, ev_rx) = mpsc::unbounded_channel::<Event>();
        let link = Arc::new(Link {
            tx,
            events: tokio::sync::Mutex::new(ev_rx),
            size: Mutex::new((width, height)),
            resized: tokio::sync::Notify::new(),
            child: Mutex::new(Some(child)),
            _slot: slot,
        });
        let reader = link.clone();
        tokio::spawn(async move {
            loop {
                let body = match proto::read_frame_async(&mut rd).await {
                    Ok(Some(b)) => b,
                    Ok(None) => break,
                    Err(e) => {
                        warn!("session stream: {e}");
                        break;
                    }
                };
                match Up::decode(&body) {
                    Ok(Up::Frame { x, y, w, h, kind, data }) => {
                        let _ = ev_tx.send(Event::Frame { x, y, w, h, kind, data });
                    }
                    Ok(Up::Resized { width, height }) => {
                        *reader.size.lock().unwrap() = (width, height);
                        reader.resized.notify_waiters();
                        let _ = ev_tx.send(Event::Resized(width, height));
                    }
                    Ok(Up::Ready { .. }) | Ok(Up::Audio { .. }) | Ok(Up::Clipboard { .. }) => {}
                    Ok(Up::Bye) => break,
                    Err(e) => {
                        warn!("bad message from the session: {e}");
                        break;
                    }
                }
            }
            let _ = ev_tx.send(Event::Closed);
        });
        Ok(link)
    }

    pub fn send(&self, msg: Down) {
        let _ = self.tx.send(msg);
    }

    /// Size of the picture the session produces now.
    pub fn size(&self) -> (u16, u16) {
        *self.size.lock().unwrap()
    }

    /// Tells the session to end and waits for it to clean up (ten seconds at most).
    pub async fn shutdown(self: Arc<Self>) {
        self.send(Down::Bye);
        let child = self.child.lock().unwrap().take();
        if let Some(mut child) = child {
            let _ = tokio::task::spawn_blocking(move || {
                for _ in 0..200 {
                    if matches!(child.try_wait(), Ok(Some(_))) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                warn!("session process did not end in time; killing it");
                let _ = child.kill();
                let _ = child.wait();
            })
            .await;
        }
    }
}
