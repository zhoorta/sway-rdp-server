//! sway-rdp-server: an RDP server on IronRDP for a sway desktop.
//! Each connection gets its own headless output (client size and scale), captured with wlr-screencopy,
//! sent as H.264 over the graphics pipeline (bitmap updates as a fallback), with keyboard and pointer injected.

mod audio;
mod capture;
mod clipboard;
mod egfx;
mod input;
mod session;

use std::num::{NonZeroU16, NonZeroUsize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Context as _;
use ironrdp_server::{
    BitmapUpdate, ConnectionHandler, Credentials, DesktopSize, DisplayUpdate, KeyboardEvent, MouseEvent,
    PixelFormat, PostConnectionAction, RdpServer, RdpServerDisplay, RdpServerDisplayUpdates,
    RdpServerInputHandler, TlsIdentityCtx,
};
use tracing::{error, info, warn};

use capture::Shared;
use session::Session;

struct Updates {
    shared: Arc<Shared>,
    slot: Arc<Slot>,
    egfx: Option<EgfxLink>,
    start: std::time::Instant,
}

struct EgfxLink {
    st: Arc<egfx::EgfxState>,
    out: egfx::Out,
    /// Set once the client turned out not to do AVC420: stay on bitmaps.
    unsupported: bool,
}

impl Updates {
    fn bitmap(&self, frame: &[u8], r: capture::Rect) -> anyhow::Result<DisplayUpdate> {
        let stride = usize::from(self.shared.width) * 4;
        let mut data = Vec::with_capacity(r.w as usize * r.h as usize * 4);
        for row in r.y..r.y + r.h {
            let start = row as usize * stride + r.x as usize * 4;
            data.extend_from_slice(&frame[start..start + r.w as usize * 4]);
        }
        Ok(DisplayUpdate::Bitmap(BitmapUpdate {
            x: r.x as u16,
            y: r.y as u16,
            width: NonZeroU16::new(r.w as u16).context("empty damage")?,
            height: NonZeroU16::new(r.h as u16).context("empty damage")?,
            format: PixelFormat::BgrA32,
            data: data.into(),
            stride: NonZeroUsize::new(r.w as usize * 4).unwrap(),
        }))
    }
}

#[async_trait::async_trait]
impl RdpServerDisplayUpdates for Updates {
    async fn next_update(&mut self) -> anyhow::Result<Option<DisplayUpdate>> {
        loop {
            // A resize is ready (output and capture are already at the new size): have the server
            // deactivate and reactivate the client. It then asks for a new `updates()`.
            if let Some(size) = self.slot.resize_to.lock().unwrap().take() {
                if (size.width, size.height) != (self.shared.width, self.shared.height) {
                    info!(width = size.width, height = size.height, "telling the client to resize");
                    return Ok(Some(DisplayUpdate::Resize(size)));
                }
            }

            // Graphics pipeline first: it is what the Windows App is built around. Give the client a
            // few seconds to open it before falling back to the old bitmap updates.
            let use_egfx = match &self.egfx {
                Some(l) if !l.unsupported => {
                    if l.st.is_ready() {
                        true
                    } else if self.start.elapsed() < std::time::Duration::from_secs(4) {
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                        continue;
                    } else {
                        false
                    }
                }
                _ => false,
            };

            if use_egfx {
                // A fresh surface needs the whole picture.
                if self.egfx.as_ref().is_some_and(|l| l.out.needs_surface()) {
                    self.shared.force_full();
                }
                let (frame, r) = tokio::select! {
                    next = self.shared.next() => next,
                    _ = self.slot.resize_notify.notified() => continue,
                };
                let link = self.egfx.as_mut().unwrap();
                match link.out.send(&frame, r)? {
                    egfx::Sent::Done => continue,
                    egfx::Sent::Backpressure => {
                        // Nothing was consumed by the encoder: put the area back and wait a moment.
                        self.shared.mark_dirty(r);
                        tokio::time::sleep(std::time::Duration::from_millis(8)).await;
                        continue;
                    }
                    egfx::Sent::Skipped => {
                        // The encoder dropped the frame: its area was never sent, so send it again.
                        self.shared.mark_dirty(r);
                        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                        continue;
                    }
                    egfx::Sent::Unsupported => {
                        warn!("client has no AVC420; using bitmap updates");
                        link.unsupported = true;
                        self.shared.mark_dirty(r);
                        continue;
                    }
                }
            }

            let (frame, r) = tokio::select! {
                next = self.shared.next() => next,
                _ = self.slot.resize_notify.notified() => continue,
            };
            return Ok(Some(self.bitmap(&frame, r)?));
        }
    }
}

/// The session of the connection that is currently open (the server handles one client at a time).
struct Slot {
    session: Mutex<Option<Session>>,
    klid: AtomicU32,
    /// Size the client asked for in its latest display layout, not applied yet.
    wanted: Mutex<Option<(u16, u16)>>,
    /// A worker is applying resizes.
    applying: AtomicBool,
    /// Set once the output is at the new size; the running `Updates` turns it into `DisplayUpdate::Resize`.
    resize_to: Mutex<Option<DesktopSize>>,
    resize_notify: tokio::sync::Notify,
}

impl Slot {
    fn new() -> Self {
        Self {
            session: Mutex::new(None),
            klid: AtomicU32::new(0),
            wanted: Mutex::new(None),
            applying: AtomicBool::new(false),
            resize_to: Mutex::new(None),
            resize_notify: tokio::sync::Notify::new(),
        }
    }
}

/// Applies the client's wanted sizes one at a time, waiting a moment so that dragging a window edge,
/// which sends a layout for every step, ends in one resize.
async fn resize_worker(slot: Arc<Slot>) {
    loop {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            let Some(want) = slot.wanted.lock().unwrap().take() else { break };
            let s2 = slot.clone();
            let done = tokio::task::spawn_blocking(move || {
                let mut guard = s2.session.lock().unwrap();
                let Some(session) = guard.as_mut() else { return anyhow::Ok(None) };
                if (session.shared.width, session.shared.height) == session::clamp_size(want.0, want.1) {
                    return Ok(None);
                }
                session.resize(want)?;
                Ok(Some(DesktopSize { width: session.shared.width, height: session.shared.height }))
            })
            .await;
            match done {
                Ok(Ok(Some(size))) => {
                    *slot.resize_to.lock().unwrap() = Some(size);
                    slot.resize_notify.notify_one();
                    // Let the client finish reactivating before the next resize.
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
                Ok(Ok(None)) => {}
                Ok(Err(e)) => error!("resize failed: {e:#}"),
                Err(e) => error!("resize task failed: {e}"),
            }
        }
        slot.applying.store(false, Ordering::Release);
        // A layout may have arrived between the last check and clearing the flag.
        if slot.wanted.lock().unwrap().is_some() && !slot.applying.swap(true, Ordering::AcqRel) {
            continue;
        }
        break;
    }
}

struct Config {
    opts: session::Options,
}

#[derive(Clone)]
struct Handler {
    slot: Arc<Slot>,
    cfg: Arc<Config>,
    egfx: Option<(Arc<egfx::EgfxState>, egfx::Tuning)>,
}

impl Handler {
    fn send(&self, cmd: input::Cmd) {
        if let Some(s) = self.slot.session.lock().unwrap().as_ref() {
            let _ = s.input.send(cmd);
        }
    }

    fn shared(&self) -> Option<Arc<Shared>> {
        self.slot.session.lock().unwrap().as_ref().map(|s| s.shared.clone())
    }
}

/// Ends the session when the client leaves.
struct Conn {
    slot: Arc<Slot>,
}

impl ConnectionHandler for Conn {
    fn on_disconnected(&mut self, peer: std::net::SocketAddr, duration: std::time::Duration, error: Option<&anyhow::Error>) -> PostConnectionAction {
        info!(%peer, ?duration, error = ?error.map(|e| e.to_string()), "client disconnected");
        self.slot.session.lock().unwrap().take();
        self.slot.wanted.lock().unwrap().take();
        self.slot.resize_to.lock().unwrap().take();
        PostConnectionAction::Continue
    }
}

#[async_trait::async_trait]
impl RdpServerDisplay for Handler {
    async fn size(&mut self) -> DesktopSize {
        match self.shared() {
            Some(s) => DesktopSize { width: s.width, height: s.height },
            None => DesktopSize { width: 1680, height: 1050 },
        }
    }

    /// A client is connecting with this desktop size: give it a session of exactly that size.
    async fn request_initial_size(&mut self, client_size: DesktopSize) -> DesktopSize {
        // Also called again when the client is reactivated after a resize: the session already exists then.
        if let Some(shared) = self.shared() {
            return DesktopSize { width: shared.width, height: shared.height };
        }
        let (slot, cfg) = (self.slot.clone(), self.cfg.clone());
        let klid = slot.klid.load(Ordering::Relaxed);
        let layout = if klid == 0 { cfg.opts.default_layout.clone() } else { session::xkb_layout_for_klid(klid).to_owned() };
        info!(?client_size, klid = format!("{klid:#06x}"), %layout, "creating session");
        let created = tokio::task::spawn_blocking(move || {
            let mut guard = slot.session.lock().unwrap();
            let s = Session::create((client_size.width, client_size.height), &layout, &cfg.opts)?;
            let size = DesktopSize { width: s.shared.width, height: s.shared.height };
            *guard = Some(s);
            anyhow::Ok(size)
        })
        .await;
        match created {
            Ok(Ok(size)) => size,
            Ok(Err(e)) => {
                error!("cannot create session: {e:#}");
                client_size
            }
            Err(e) => {
                error!("session task failed: {e}");
                client_size
            }
        }
    }

    async fn updates(&mut self) -> anyhow::Result<Box<dyn RdpServerDisplayUpdates>> {
        let shared = self.shared().context("no session for this connection")?;
        // New connection: the client has nothing yet, and drops what it is sent before it is ready.
        // Repaint a few times while it comes up (the DisplayControl layout below is the real signal).
        shared.force_full();
        let timers = shared.clone();
        tokio::spawn(async move {
            for secs in [1u64, 2, 4, 8] {
                tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
                timers.force_full();
            }
        });
        let egfx = match &self.egfx {
            Some((st, tuning)) => Some(EgfxLink {
                st: st.clone(),
                out: egfx::Out::new(st.clone(), shared.width, shared.height, *tuning)?,
                unsupported: false,
            }),
            None => None,
        };
        Ok(Box::new(Updates { shared, slot: self.slot.clone(), egfx, start: std::time::Instant::now() }))
    }

    fn request_layout(&mut self, layout: ironrdp_displaycontrol::pdu::DisplayControlMonitorLayout) {
        tracing::debug!(?layout, "client layout");
        let Some(m) = layout.monitors().iter().find(|m| m.is_primary()).or(layout.monitors().first()) else { return };
        let (w, h) = m.dimensions();
        let scale = m.desktop_scale_factor().unwrap_or(100);
        let current = {
            let guard = self.slot.session.lock().unwrap();
            let Some(s) = guard.as_ref() else { return };
            info!(w, h, scale, "client display layout");
            s.set_scale(scale);
            (s.shared.width, s.shared.height)
        };
        let (w, h) = session::clamp_size(w.min(u32::from(u16::MAX)) as u16, h.min(u32::from(u16::MAX)) as u16);
        if (w, h) != current {
            *self.slot.wanted.lock().unwrap() = Some((w, h));
            if !self.slot.applying.swap(true, Ordering::AcqRel) {
                tokio::spawn(resize_worker(self.slot.clone()));
            }
        }
    }
}

impl RdpServerInputHandler for Handler {
    fn keyboard(&mut self, event: KeyboardEvent) {
        match event {
            KeyboardEvent::Pressed { code, extended } | KeyboardEvent::Released { code, extended } => {
                let pressed = matches!(event, KeyboardEvent::Pressed { .. });
                match input::evdev_code(code, extended) {
                    Some(code) => self.send(input::Cmd::Key { code, pressed }),
                    None => tracing::debug!(code, extended, "unmapped scancode"),
                }
            }
            // The Windows App drops updates sent before it is ready; its first synchronise means it is.
            KeyboardEvent::Synchronize(_) => {
                if let Some(s) = self.shared() {
                    s.force_full();
                }
            }
            other => tracing::debug!(?other, "keyboard event not handled"),
        }
    }

    fn keyboard_layout(&mut self, klid: u32) {
        self.slot.klid.store(klid, Ordering::Relaxed);
    }

    fn mouse(&mut self, event: MouseEvent) {
        use input::{Cmd, BTN_EXTRA, BTN_LEFT, BTN_MIDDLE, BTN_RIGHT, BTN_SIDE};
        let button = |code, pressed| Cmd::Button { code, pressed };
        match event {
            MouseEvent::Move { x, y } => self.send(Cmd::Move { x: x.into(), y: y.into() }),
            MouseEvent::LeftPressed => self.send(button(BTN_LEFT, true)),
            MouseEvent::LeftReleased => self.send(button(BTN_LEFT, false)),
            MouseEvent::RightPressed => self.send(button(BTN_RIGHT, true)),
            MouseEvent::RightReleased => self.send(button(BTN_RIGHT, false)),
            MouseEvent::MiddlePressed => self.send(button(BTN_MIDDLE, true)),
            MouseEvent::MiddleReleased => self.send(button(BTN_MIDDLE, false)),
            MouseEvent::Button4Pressed => self.send(button(BTN_SIDE, true)),
            MouseEvent::Button4Released => self.send(button(BTN_SIDE, false)),
            MouseEvent::Button5Pressed => self.send(button(BTN_EXTRA, true)),
            MouseEvent::Button5Released => self.send(button(BTN_EXTRA, false)),
            // 120 units per wheel notch.
            MouseEvent::VerticalScroll { value } => self.send(Cmd::Wheel { vertical: f64::from(value) / 120.0, horizontal: 0.0 }),
            MouseEvent::Scroll { x, y } => {
                self.send(Cmd::Wheel { vertical: f64::from(y) / 120.0, horizontal: f64::from(x) / 120.0 })
            }
            MouseEvent::RelMove { .. } => tracing::debug!("relative mouse move not handled"),
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_env("SWAY_RDP_LOG").unwrap_or_else(|_| "info".into()))
        .init();

    let mut args = std::env::args().skip(1);
    let (mut bind, mut cert, mut key) = ("127.0.0.1:3389".to_string(), None::<PathBuf>, None::<PathBuf>);
    let (mut layout, mut spawn_terminal) = ("us".to_string(), true);
    let (mut workspace, mut hook) = ("Remote".to_string(), None::<PathBuf>);
    let (mut no_egfx, mut bitrate, mut max_qp) = (false, 40_000_000u32, 20u8);
    // None = decide from the bind address: replacing is only safe where reaching the port already means trust.
    let mut replace: Option<bool> = None;
    let mut creds_file: Option<PathBuf> = None;
    // Off by default: the Windows App on macOS hangs the session when it has to hand over its clipboard
    // (see docs/NOTES.md), while FreeRDP is fine.
    let mut clipboard = false;
    let mut audio = true;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--bind" => bind = args.next().context("--bind needs a value")?,
            "--cert" => cert = args.next().map(PathBuf::from),
            "--key" => key = args.next().map(PathBuf::from),
            "--kb-layout" => layout = args.next().context("--kb-layout needs a value (used when the client announces none)")?,
            "--no-terminal" => spawn_terminal = false,
            "--workspace" => workspace = args.next().context("--workspace needs a name")?,
            // Started per session with SWAY_RDP_OUTPUT, SWAY_RDP_WORKSPACE and SWAY_RDP_INDEX set; stopped when it ends.
            "--session-hook" => hook = args.next().map(PathBuf::from),
            "--no-egfx" => no_egfx = true,
            // A new connection ends the open one. It does so before the new client has logged in, so use
            // it only where reaching the port already means trust (loopback, SSH tunnel, VPN).
            "--no-replace" => replace = Some(false),
            "--replace" => replace = Some(true),
            // First line "user:password" (split at the first colon). No built-in default credential.
            "--credentials" => creds_file = args.next().map(PathBuf::from),
            "--clipboard" => clipboard = true,
            "--no-audio" => audio = false,
            "--bitrate" => bitrate = args.next().context("--bitrate needs a value")?.parse()?,
            "--quality" => max_qp = args.next().context("--quality needs a value (max QP, lower = better)")?.parse()?,
            other => anyhow::bail!("unknown argument {other}"),
        }
    }
    let (username, password) = {
        let f = creds_file.context("--credentials FILE is required (first line user:password)")?;
        let text = std::fs::read_to_string(&f).with_context(|| format!("reading {}", f.display()))?;
        let line = text.lines().next().unwrap_or("");
        let (u, p) = line.split_once(':').context("credentials file must start with user:password")?;
        anyhow::ensure!(!u.is_empty() && p.len() >= 8, "credentials: empty user or password shorter than 8 characters");
        (u.to_string(), p.to_string())
    };
    let loopback = bind.parse::<std::net::SocketAddr>().map(|a| a.ip().is_loopback()).unwrap_or(false);
    let replace = replace.unwrap_or(loopback);
    let (cert, key) = cert.zip(key).context("--cert and --key are required (TLS is always on)")?;
    let identity = TlsIdentityCtx::init_from_paths(&cert, &key).context("TLS identity")?;
    let acceptor = identity.make_acceptor().context("TLS acceptor")?;

    let slot = Arc::new(Slot::new());
    let egfx_state = Arc::new(egfx::EgfxState::default());
    let handler = Handler {
        slot: slot.clone(),
        cfg: Arc::new(Config { opts: session::Options { default_layout: layout, spawn_terminal, workspace, hook } }),
        egfx: (!no_egfx).then(|| (egfx_state.clone(), egfx::Tuning { bitrate, max_qp, fps: 30 })),
    };
    let mut server = RdpServer::builder()
        .with_addr(bind.parse::<std::net::SocketAddr>().context("bad --bind address")?)
        .with_hybrid(acceptor, identity.pub_key)
        .with_input_handler(handler.clone())
        .with_display_handler(handler)
        .with_gfx_factory((!no_egfx).then(|| {
            Box::new(egfx::Factory::new(egfx_state.clone())) as Box<dyn ironrdp_server::GfxServerFactory>
        }))
        .with_cliprdr_factory(clipboard.then(|| Box::new(clipboard::Factory::new()) as Box<dyn ironrdp_server::CliprdrServerFactory>))
        .with_sound_factory(audio.then(|| Box::new(audio::Factory::new()) as Box<dyn ironrdp_server::SoundServerFactory>))
        .with_honor_client_desktop_size(true)
        .with_replace_on_new_connection(replace)
        .with_connection_handler(Some(Box::new(Conn { slot })))
        .build();
    server.set_credentials(Some(Credentials {
        username,
        password,
        domain: None,
    }));
    info!(%bind, "sway-rdp-server listening (NLA)");
    server.run().await.context("server stopped")
}
