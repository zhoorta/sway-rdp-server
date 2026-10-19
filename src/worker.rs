//! The session process (`sway-rdp-server --session-worker`): one user's output, capture, encoder and input.
//! It talks to the network daemon over the socket it inherits as fd 3, using `proto`. It never touches the
//! network: it encodes what the daemon has credits for and injects the input the daemon forwards.

use std::os::fd::FromRawFd;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context as _, Result};
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use crate::capture::Rect;
use crate::encode::{Tuning, H264};
use crate::input;
use crate::proto::{self, Down, FrameKind, Up};
use crate::session::{self, Session};

/// The daemon never grants more than this many at once (1000 when the client switched acknowledgements off).
const MAX_CREDITS: u32 = 1000;
/// How long the picture must stay still before the areas that moved are sent again as a keyframe.
const SETTLE: Duration = Duration::from_millis(300);

pub struct Config {
    pub opts: session::Options,
    /// None: no H.264, bitmaps only.
    pub tuning: Option<Tuning>,
    /// Start a headless sway of this session's own (see `OwnSway`).
    pub own_sway: bool,
    /// Keep the session across clients: register with the root helper at this socket and take the client sockets it
    /// sends. None: one client, on the socket inherited as fd 3, and the session ends with it.
    pub register: Option<std::path::PathBuf>,
    /// A persistent session without a client ends after this long (zero: never).
    pub idle_timeout: std::time::Duration,
    /// A persistent session ends when its client leaves.
    pub end_on_disconnect: bool,
}

/// Where clients come from.
enum Source {
    /// The daemon's socket pair, once.
    Single(Option<std::os::unix::net::UnixStream>),
    /// Sockets sent by the helper, one per client; `None` from `next` means the helper is gone.
    Registered(mpsc::UnboundedReceiver<std::os::unix::net::UnixStream>),
}

impl Source {
    async fn next(&mut self) -> Option<std::os::unix::net::UnixStream> {
        match self {
            Source::Single(s) => match s.take() {
                Some(stream) => Some(stream),
                None => std::future::pending().await,
            },
            Source::Registered(rx) => rx.recv().await,
        }
    }

    fn persistent(&self) -> bool {
        matches!(self, Source::Registered(_))
    }
}

/// Connects to the helper's registration socket (the helper learns our account from the kernel) and turns every
/// descriptor it sends into a client socket. If the helper goes away (a restart, an upgrade) it connects again, so the
/// session stays reachable.
fn register(path: &std::path::Path) -> Result<Source> {
    let path = path.to_path_buf();
    // Fail early if the helper is not there at all.
    let first = std::os::unix::net::UnixStream::connect(&path).with_context(|| format!("cannot register at {}", path.display()))?;
    let (tx, rx) = mpsc::unbounded_channel();
    std::thread::spawn(move || {
        let mut reg = Some(first);
        loop {
            if let Some(sock) = reg.take().or_else(|| std::os::unix::net::UnixStream::connect(&path).ok()) {
                while let Ok(fd) = crate::fdpass::recv_fd(&sock) {
                    if tx.send(std::os::unix::net::UnixStream::from(fd)).is_err() {
                        return;
                    }
                }
            }
            if tx.is_closed() {
                return;
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    });
    Ok(Source::Registered(rx))
}

/// How a client's turn ended.
enum Outcome {
    /// The client left; the session goes on.
    Gone,
    /// Another client arrived: this is its socket.
    Replaced(std::os::unix::net::UnixStream),
    /// The session has to end (signal, sway gone, error).
    Stop,
}

struct Worker {
    session: Option<Session>,
    enc: Option<H264>,
    tuning: Option<Tuning>,
    /// Frames the daemon has room for.
    credits: u32,
    /// Send raw bitmaps instead of H.264.
    bitmaps: bool,
    up: mpsc::UnboundedSender<Up>,
    /// Area changed since the last settle refresh. Between keyframes the encoder keeps improving the picture in
    /// places the client was not told to redraw, so a region drawn roughly during motion would stay rough.
    settle: Option<Rect>,
    /// The next frame to go out is the settle refresh itself and does not start another one.
    settling: bool,
    last_frame: tokio::time::Instant,
    /// The rectangle of the latest frame, and when it last differed from the one before (a window moving or resizing).
    last_rect: Option<Rect>,
    last_change: tokio::time::Instant,
}

pub async fn run(cfg: Config) -> Result<()> {
    // A stop request (systemd stopping the service, Ctrl-C) must still remove the output and stop the hook.
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut int = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;

    let mut source = match &cfg.register {
        // The helper starts us and then sends clients; register before the (slow) sway start so that it can.
        Some(path) => register(path)?,
        // The daemon passes its end of a socket pair as fd 3.
        None => Source::Single(Some(unsafe { std::os::unix::net::UnixStream::from_raw_fd(3) })),
    };
    // Declared before the session so that it is dropped after it: the session's cleanup still needs the sway.
    let mut sway = if cfg.own_sway { Some(OwnSway::start().await?) } else { None };

    let mut session: Option<Session> = None;
    let mut pending: Option<std::os::unix::net::UnixStream> = None;
    loop {
        let conn = match pending.take() {
            Some(c) => c,
            None => {
                // Waiting for a client.
                let idle_for = (session.is_some() && !cfg.idle_timeout.is_zero()).then_some(cfg.idle_timeout);
                let idle = async {
                    match idle_for {
                        Some(d) => tokio::time::sleep(d).await,
                        None => std::future::pending().await,
                    }
                };
                tokio::pin!(idle);
                let mut tick = tokio::time::interval(Duration::from_secs(2));
                let next = loop {
                    tokio::select! {
                        c = source.next() => break c,
                        _ = &mut idle => {
                            info!("no client for {:?}: ending the session", cfg.idle_timeout);
                            break None;
                        }
                        _ = tick.tick() => if sway.as_mut().is_some_and(OwnSway::exited) {
                            info!("sway has ended: ending the session");
                            break None;
                        },
                        _ = term.recv() => break None,
                        _ = int.recv() => break None,
                    }
                };
                match next {
                    Some(c) => c,
                    None => break,
                }
            }
        };
        let (back, outcome) = serve_client(conn, session.take(), &cfg, &mut source, &mut sway, &mut term, &mut int).await;
        session = back;
        match outcome {
            Outcome::Gone if source.persistent() && !cfg.end_on_disconnect => info!("client left; the session stays"),
            Outcome::Gone => break,
            Outcome::Replaced(next) => {
                info!("a new client replaces the old one");
                pending = Some(next);
            }
            Outcome::Stop => break,
        }
    }
    info!("session worker ending");
    // Dropping the session removes the output and stops the hook.
    drop(session);
    Ok(())
}

/// Serves one client until it leaves, is replaced or the session has to end. Returns the session (if it exists).
async fn serve_client(
    conn: std::os::unix::net::UnixStream,
    session: Option<Session>,
    cfg: &Config,
    source: &mut Source,
    sway: &mut Option<OwnSway>,
    term: &mut tokio::signal::unix::Signal,
    int: &mut tokio::signal::unix::Signal,
) -> (Option<Session>, Outcome) {
    let (mut rd, mut wr) = match conn.set_nonblocking(true).and_then(|()| tokio::net::UnixStream::from_std(conn)) {
        Ok(s) => s.into_split(),
        Err(e) => {
            warn!("bad client socket: {e}");
            return (session, Outcome::Gone);
        }
    };
    // Writer and reader tasks: the main loop must never read a half message inside `select!`.
    let (up_tx, mut up_rx) = mpsc::unbounded_channel::<Up>();
    let writer = tokio::spawn(async move {
        while let Some(msg) = up_rx.recv().await {
            if proto::write_frame_async(&mut wr, &msg.encode()).await.is_err() {
                break;
            }
        }
    });
    let (down_tx, mut down_rx) = mpsc::unbounded_channel::<Down>();
    let reader = tokio::spawn(async move {
        loop {
            let msg = match proto::read_frame_async(&mut rd).await {
                Ok(Some(body)) => Down::decode(&body),
                Ok(None) => Ok(Down::Bye),
                Err(e) => Err(e),
            };
            match msg {
                Ok(m) => {
                    let bye = m == Down::Bye;
                    if down_tx.send(m).is_err() || bye {
                        break;
                    }
                }
                Err(e) => {
                    warn!("bad message from the daemon: {e}");
                    let _ = down_tx.send(Down::Bye);
                    break;
                }
            }
        }
    });
    let finish = |writer: tokio::task::JoinHandle<()>, reader: tokio::task::JoinHandle<()>| {
        // Dropping the halves closes the socket: the daemon sees the end of the stream.
        writer.abort();
        reader.abort();
    };

    let hello = match down_rx.recv().await {
        Some(Down::Hello { version, width, height, klid, .. }) if version == proto::VERSION => (width, height, klid),
        other => {
            warn!("first message of a client is not a usable hello: {other:?}");
            finish(writer, reader);
            return (session, Outcome::Gone);
        }
    };
    let (width, height, klid) = hello;

    // First client: make the session. Later clients: it exists; give it the new client's size.
    let session = match session {
        None => {
            let layout = if klid == 0 { cfg.opts.default_layout.clone() } else { session::xkb_layout_for_klid(klid).to_owned() };
            info!(width, height, klid = format!("{klid:#06x}"), %layout, "creating session");
            let opts = cfg.opts.clone();
            match tokio::task::spawn_blocking(move || Session::create((width, height), &layout, &opts)).await {
                Ok(Ok(s)) => s,
                Ok(Err(e)) => {
                    error!("cannot create the session: {e:#}");
                    finish(writer, reader);
                    return (None, Outcome::Stop);
                }
                Err(e) => {
                    error!("session task failed: {e}");
                    finish(writer, reader);
                    return (None, Outcome::Stop);
                }
            }
        }
        Some(mut s) => {
            info!(width, height, "a client reconnects to the session");
            if session::clamp_size(width, height) != (s.shared.width, s.shared.height) {
                let (s2, result) = tokio::task::spawn_blocking(move || {
                    let r = s.resize((width, height));
                    (s, r)
                })
                .await
                .expect("resize task");
                if let Err(e) = result {
                    error!("resize for the new client failed: {e:#}");
                }
                s = s2;
            }
            s
        }
    };
    let (w, h) = (session.shared.width, session.shared.height);
    session.shared.force_full();
    let mut me = Worker { session: Some(session), enc: None, tuning: cfg.tuning, credits: 0, bitmaps: cfg.tuning.is_none(), up: up_tx, settle: None, settling: false, last_frame: tokio::time::Instant::now(), last_rect: None, last_change: tokio::time::Instant::now() };
    let _ = me.up.send(Up::Ready { width: w, height: h });

    let mut tick = tokio::time::interval(Duration::from_secs(2));
    // The helper has gone away: no new clients can come, this one carries on.
    let mut helper_gone = false;
    let outcome = loop {
        let shared = me.session.as_ref().expect("session").shared.clone();
        tokio::select! {
            _ = term.recv() => break Outcome::Stop,
            _ = int.recv() => break Outcome::Stop,
            _ = tick.tick() => if sway.as_mut().is_some_and(OwnSway::exited) {
                info!("sway has ended");
                break Outcome::Stop;
            },
            replacement = source.next(), if source.persistent() && !helper_gone => match replacement {
                Some(next) => break Outcome::Replaced(next),
                None => helper_gone = true,
            },
            msg = down_rx.recv() => match msg {
                None | Some(Down::Bye) => {
                    let _ = me.up.send(Up::Bye);
                    break Outcome::Gone;
                }
                Some(m) => me.handle(m).await,
            },
            _ = tokio::time::sleep_until(me.settle_due()), if me.settle.is_some() && me.credits > 0 && !me.settling => me.start_settle(),
            // Only take a frame when there is room for it: changes keep merging into the dirty rectangle meanwhile.
            (frame, rect) = shared.next(), if me.credits > 0 => {
                if let Err(e) = me.send_frame(&frame, rect).await {
                    error!("sending a frame failed: {e:#}");
                    break Outcome::Gone;
                }
            }
        }
    };
    // Let the goodbye out before the socket closes.
    if matches!(outcome, Outcome::Gone | Outcome::Stop) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let session = me.session.take();
    drop(me);
    finish(writer, reader);
    (session, outcome)
}

impl Worker {
    /// When the settle refresh is due. A picture that stays the same, or whose changes stay inside the same rectangle
    /// (a video playing where a window was dragged to), counts as settled once that rectangle stopped changing.
    fn settle_due(&self) -> tokio::time::Instant {
        (if self.settle == self.last_rect { self.last_frame } else { self.last_change }) + SETTLE
    }

    /// Send what moved lately again as a keyframe, now that the picture is still.
    fn start_settle(&mut self) {
        let (Some(rect), Some(enc)) = (self.settle.take(), self.enc.as_mut()) else { return };
        // The area may be from before a resize: keep what still lies on the picture.
        let shared = self.session.as_ref().expect("session").shared.clone();
        let (x1, y1) = ((rect.x + rect.w).min(shared.width.into()), (rect.y + rect.h).min(shared.height.into()));
        if rect.x >= x1 || rect.y >= y1 {
            return;
        }
        enc.force_keyframe();
        self.settling = true;
        shared.mark_dirty(Rect { x: rect.x, y: rect.y, w: x1 - rect.x, h: y1 - rect.y });
    }

    async fn handle(&mut self, msg: Down) {
        let session = self.session.as_ref().expect("session");
        match msg {
            Down::Hello { .. } => warn!("second hello ignored"),
            Down::Key { code, pressed } => {
                let _ = session.input.send(input::Cmd::Key { code, pressed });
            }
            Down::PointerMove { x, y } => {
                let _ = session.input.send(input::Cmd::Move { x, y });
            }
            Down::Button { code, pressed } => {
                let _ = session.input.send(input::Cmd::Button { code, pressed });
            }
            Down::Wheel { vertical, horizontal } => {
                let _ = session.input.send(input::Cmd::Wheel { vertical, horizontal });
            }
            Down::Credit { frames } => self.credits = (self.credits + u32::from(frames)).min(MAX_CREDITS),
            Down::RequestFull => session.shared.force_full(),
            Down::UseBitmaps => {
                self.bitmaps = true;
                self.enc = None;
                session.shared.force_full();
            }
            Down::Clipboard { .. } => {}
            Down::Resize { width, height, scale_percent } => self.resize(width, height, scale_percent).await,
            Down::Bye => {}
        }
    }

    /// Sets the scale and, if the size changed, resizes the output. Frames in flight are lost with the client's
    /// reactivation, so the credits start again from zero (the daemon grants new ones).
    async fn resize(&mut self, width: u16, height: u16, scale: u16) {
        let mut session = self.session.take().expect("session");
        let (session, result) = tokio::task::spawn_blocking(move || {
            session.set_scale(u32::from(scale));
            let current = (session.shared.width, session.shared.height);
            let result = if session::clamp_size(width, height) != current { session.resize((width, height)).map(|()| true) } else { Ok(false) };
            (session, result)
        })
        .await
        .expect("resize task");
        let (w, h) = (session.shared.width, session.shared.height);
        self.session = Some(session);
        match result {
            Ok(true) => {
                self.enc = None;
                self.credits = 0;
                let _ = self.up.send(Up::Resized { width: w, height: h });
            }
            Ok(false) => {}
            Err(e) => error!("resize failed: {e:#}"),
        }
    }

    async fn send_frame(&mut self, frame: &Arc<Vec<u8>>, r: Rect) -> Result<()> {
        let shared = self.session.as_ref().expect("session").shared.clone();
        let (w, h) = (usize::from(shared.width), usize::from(shared.height));
        if self.bitmaps {
            let stride = w * 4;
            let mut data = Vec::with_capacity(r.w as usize * r.h as usize * 4);
            for row in r.y..r.y + r.h {
                let start = row as usize * stride + r.x as usize * 4;
                data.extend_from_slice(&frame[start..start + r.w as usize * 4]);
            }
            self.credits -= 1;
            let _ = self.up.send(Up::Frame { x: r.x as u16, y: r.y as u16, w: r.w as u16, h: r.h as u16, kind: FrameKind::Bgra, data });
            return Ok(());
        }
        if self.enc.is_none() {
            let tuning = self.tuning.context("no encoder settings")?;
            self.enc = Some(H264::new(shared.width, shared.height, tuning)?);
        }
        match self.enc.as_mut().expect("encoder").encode(frame, w, h)? {
            Some(data) => {
                self.credits -= 1;
                if self.settling {
                    self.settling = false;
                } else {
                    self.settle = Some(self.settle.map_or(r, |old| old.union(r)));
                }
                self.last_frame = tokio::time::Instant::now();
                if self.last_rect != Some(r) {
                    self.last_rect = Some(r);
                    self.last_change = self.last_frame;
                }
                let _ = self.up.send(Up::Frame { x: r.x as u16, y: r.y as u16, w: r.w as u16, h: r.h as u16, kind: FrameKind::H264, data });
            }
            None => {
                // The encoder dropped the frame: its area was never sent, so send it again.
                shared.mark_dirty(r);
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        }
        Ok(())
    }
}

/// A headless sway that belongs to this session (and so to its account): its Wayland and IPC sockets live in the
/// account's own runtime directory, which no other account can enter. Stopped when dropped.
struct OwnSway {
    child: std::process::Child,
}

impl OwnSway {
    /// True when sway has ended (the account logged out of it, or it crashed).
    fn exited(&mut self) -> bool {
        !matches!(self.child.try_wait(), Ok(None))
    }

    async fn start() -> Result<Self> {
        use std::os::unix::process::CommandExt;
        let runtime = std::path::PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").context("XDG_RUNTIME_DIR is not set")?);
        let uid = unsafe { libc::getuid() };
        // The account's own config for these sessions wins, then the system's; without either, sway's usual lookup.
        let config = std::env::var_os("HOME")
            .map(|h| std::path::PathBuf::from(h).join(".config/sway-rdp/config"))
            .filter(|p| p.is_file())
            .or_else(|| Some(std::path::PathBuf::from("/etc/xdg/sway-rdp/sway.conf")).filter(|p| p.is_file()));
        let mut cmd = std::process::Command::new("sway");
        if let Some(path) = &config {
            cmd.arg("-c").arg(path);
            info!(config = %path.display(), "sway config");
        }
        let child = cmd
            .env("WLR_BACKENDS", "headless")
            .env("WLR_LIBINPUT_NO_DEVICES", "1")
            // No default output: the session creates its own, at the client's size.
            .env("WLR_HEADLESS_OUTPUTS", "0")
            .env("WLR_RENDERER", "pixman")
            .env("GSETTINGS_BACKEND", "memory")
            .env("NO_AT_BRIDGE", "1")
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("SWAYSOCK")
            .env_remove("DISPLAY")
            .stdin(std::process::Stdio::null())
            .process_group(0)
            .spawn()
            .context("cannot start sway")?;
        let mut me = Self { child };
        let ipc = runtime.join(format!("sway-ipc.{uid}.{}.sock", me.child.id()));
        for _ in 0..100 {
            if ipc.exists() {
                break;
            }
            if let Ok(Some(status)) = me.child.try_wait() {
                bail!("sway ended right away ({status})");
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        anyhow::ensure!(ipc.exists(), "sway has no IPC socket after 10 s");

        // The Wayland socket has an automatic name: ask sway to tell us what its children inherit.
        let note = runtime.join(format!("sway-rdp-{}.wayland", me.child.id()));
        let out = std::process::Command::new("swaymsg")
            .arg("-s")
            .arg(&ipc)
            .arg("exec")
            .arg(format!("printenv WAYLAND_DISPLAY > '{}'", note.display()))
            .output()
            .context("cannot run swaymsg")?;
        anyhow::ensure!(out.status.success(), "swaymsg exec failed: {}", String::from_utf8_lossy(&out.stderr));
        let mut socket = String::new();
        for _ in 0..50 {
            socket = std::fs::read_to_string(&note).unwrap_or_default().trim().to_owned();
            if !socket.is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        let _ = std::fs::remove_file(&note);
        anyhow::ensure!(!socket.is_empty(), "could not find out the Wayland socket of sway");
        std::env::set_var("SWAYSOCK", &ipc);
        std::env::set_var("WAYLAND_DISPLAY", &socket);
        info!(pid = me.child.id(), wayland = %socket, "own headless sway started");
        Ok(me)
    }
}

impl Drop for OwnSway {
    fn drop(&mut self) {
        let pgid = self.child.id() as i32;
        unsafe { libc::kill(-pgid, libc::SIGTERM) };
        for _ in 0..60 {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        unsafe { libc::kill(-pgid, libc::SIGKILL) };
        let _ = self.child.wait();
    }
}
