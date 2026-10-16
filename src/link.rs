//! The network daemon's side of the stream protocol: starts a session process for a connection, sends it what the
//! client does (input, resizes, credits) and turns what it sends back into events for the RDP side.

use std::path::Path;
use std::process::Child;
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
        let (child, parent) = crate::spawn::spawn_worker(exe, args, &workspace, user, own_sway)?;
        Self::from_stream(parent, Some(child), slot, size, klid).await
    }

    /// Uses a session process that the root helper started: `stream` is the daemon's end of its socket.
    pub async fn from_helper(stream: std::os::unix::net::UnixStream, size: (u16, u16), klid: u32) -> Result<Arc<Link>> {
        Self::from_stream(stream, None, Slot::take(), size, klid).await
    }

    /// Takes over a session process that is already running: sends it the hello and waits until it reports its size.
    /// `child` is None when something else (the root helper) owns the process.
    async fn from_stream(parent: std::os::unix::net::UnixStream, child: Option<Child>, slot: Slot, size: (u16, u16), klid: u32) -> Result<Arc<Link>> {
        parent.set_nonblocking(true)?;
        let (mut rd, mut wr) = tokio::net::UnixStream::from_std(parent)?.into_split();

        let hello = Down::Hello { version: proto::VERSION, width: size.0, height: size.1, scale_percent: 100, klid };
        proto::write_frame_async(&mut wr, &hello.encode()).await?;
        let first = tokio::time::timeout(Duration::from_secs(30), proto::read_frame_async(&mut rd))
            .await
            .context("the session did not start in 30 s")??
            .context("the session process ended before it was ready")?;
        let Up::Ready { width, height } = Up::decode(&first)? else { bail!("first message of the session is not Ready") };
        info!(width, height, pid = child.as_ref().map(Child::id), "session process ready");

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
            child: Mutex::new(child),
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

    /// Tells the session to end and waits for it to clean up (ten seconds at most). Without a child process of our
    /// own (the helper owns it), the end of the stream is the sign that it is gone.
    pub async fn shutdown(self: Arc<Self>) {
        self.send(Down::Bye);
        let child = self.child.lock().unwrap().take();
        match child {
            Some(mut child) => {
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
            None => {
                let mut events = self.events.lock().await;
                let waiting = async {
                    while !matches!(events.recv().await, Some(Event::Closed) | None) {}
                };
                if tokio::time::timeout(Duration::from_secs(10), waiting).await.is_err() {
                    warn!("session process did not close its socket in time");
                }
            }
        }
    }
}
