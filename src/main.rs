//! sway-rdp-server: an RDP server on IronRDP for a sway desktop.
//! Each connection gets its own headless output (client size and scale), captured with wlr-screencopy,
//! sent as H.264 over the graphics pipeline (bitmap updates as a fallback), with keyboard and pointer injected.

mod audio;
mod auth;
mod capture;
mod clipboard;
mod egfx;
mod encode;
mod input;
mod proto;
mod session;

use std::num::{NonZeroU16, NonZeroUsize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Context as _;
use ironrdp_server::{
    BitmapUpdate, Credentials, DesktopSize, DisplayUpdate, KeyboardEvent, MouseEvent, PixelFormat, RdpServer,
    RdpServerDisplay, RdpServerDisplayUpdates, RdpServerInputHandler, TlsIdentityCtx,
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
    enc: encode::H264,
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
                // Wait for room before taking a frame: changes keep merging into the dirty rectangle meanwhile.
                let st = self.egfx.as_ref().unwrap().st.clone();
                tokio::select! {
                    _ = st.wait_credit() => {}
                    _ = self.slot.resize_notify.notified() => continue,
                }
                let (frame, r) = tokio::select! {
                    next = self.shared.next() => next,
                    _ = self.slot.resize_notify.notified() => continue,
                };
                let link = self.egfx.as_mut().unwrap();
                match link.out.prepare()? {
                    egfx::Prepare::Ready => {}
                    egfx::Prepare::Unsupported => {
                        warn!("client has no AVC420; using bitmap updates");
                        link.unsupported = true;
                        self.shared.mark_dirty(r);
                        continue;
                    }
                }
                match link.enc.encode(&frame, usize::from(self.shared.width), usize::from(self.shared.height))? {
                    Some(data) => link.out.send_encoded(&data, r, link.enc.max_qp())?,
                    None => {
                        // The encoder dropped the frame: its area was never sent, so send it again.
                        link.out.flush()?;
                        self.shared.mark_dirty(r);
                        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                    }
                }
                continue;
            }

            let (frame, r) = tokio::select! {
                next = self.shared.next() => next,
                _ = self.slot.resize_notify.notified() => continue,
            };
            return Ok(Some(self.bitmap(&frame, r)?));
        }
    }
}

/// The session of one connection. Every connection has its own `Slot`.
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
    egfx: Option<(Arc<egfx::EgfxState>, encode::Tuning)>,
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
                out: egfx::Out::new(st.clone(), shared.width, shared.height),
                enc: encode::H264::new(shared.width, shared.height, *tuning)?,
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
                // Some clients (FreeRDP SDL) send 16.16 fixed-point amounts here; plain wheel units are 120 per notch.
                let unit = |v: i32| if v.unsigned_abs() > 32768 { f64::from(v) / 65536.0 / 40.0 } else { f64::from(v) / 120.0 };
                self.send(Cmd::Wheel { vertical: unit(y), horizontal: unit(x) })
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
    // Kept for old command lines: every connection has its own session now, so nothing is replaced.
    let mut replace: Option<bool> = None;
    // Experiment: plain TLS login (the password travels inside TLS, no CredSSP/NLA). Off by default.
    let mut tls_only = false;
    // "file": NLA with the user:password of --credentials. "pam": the account's own password, checked through PAM,
    // over plain TLS login (see auth.rs for what that trades away).
    let mut auth_mode = "file".to_string();
    let mut creds_file: Option<PathBuf> = None;
    // Off by default: the Windows App on macOS hangs the session when it has to hand over its clipboard
    // (see docs/NOTES.md), while FreeRDP is fine.
    let mut clipboard = false;
    let mut audio = true;
    let mut max_sessions = 8usize;
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
            "--tls-only" => tls_only = true,
            "--auth" => auth_mode = args.next().context("--auth needs file or pam")?,
            "--no-replace" => replace = Some(false),
            "--replace" => replace = Some(true),
            // First line "user:password" (split at the first colon). No built-in default credential.
            "--credentials" => creds_file = args.next().map(PathBuf::from),
            "--clipboard" => clipboard = true,
            "--no-audio" => audio = false,
            "--max-sessions" => max_sessions = args.next().context("--max-sessions needs a number")?.parse()?,
            "--bitrate" => bitrate = args.next().context("--bitrate needs a value")?.parse()?,
            "--quality" => max_qp = args.next().context("--quality needs a value (max QP, lower = better)")?.parse()?,
            other => anyhow::bail!("unknown argument {other}"),
        }
    }
    anyhow::ensure!(matches!(auth_mode.as_str(), "file" | "pam"), "--auth must be file or pam");
    let pam = auth_mode == "pam";
    let tls_only = tls_only || pam;
    let (username, password) = if pam {
        (String::new(), String::new())
    } else {
        let f = creds_file.context("--credentials FILE is required with --auth file (first line user:password)")?;
        let text = std::fs::read_to_string(&f).with_context(|| format!("reading {}", f.display()))?;
        let line = text.lines().next().unwrap_or("");
        let (u, p) = line.split_once(':').context("credentials file must start with user:password")?;
        anyhow::ensure!(!u.is_empty() && p.len() >= 8, "credentials: empty user or password shorter than 8 characters");
        (u.to_string(), p.to_string())
    };
    let (cert, key) = cert.zip(key).context("--cert and --key are required (TLS is always on)")?;
    let identity = TlsIdentityCtx::init_from_paths(&cert, &key).context("TLS identity")?;
    let acceptor = identity.make_acceptor().context("TLS acceptor")?;

    let common = Arc::new(Common {
        cfg: Arc::new(Config { opts: session::Options { default_layout: layout, spawn_terminal, workspace, hook } }),
        acceptor,
        pub_key: identity.pub_key,
        tls_only,
        pam_validator: if pam { Some(Arc::new(auth::PamValidator::new("sway-rdp-server")?)) } else { None },
        credentials: (username, password),
        egfx: (!no_egfx).then_some(encode::Tuning { bitrate, max_qp, fps: 30 }),
        clipboard,
        audio,
    });
    if let Some(v) = &common.pam_validator {
        info!(user = v.user(), "login: PAM service sway-rdp-server, plain TLS login (no NLA)");
    }
    if replace == Some(true) {
        warn!("--replace is ignored: every connection now gets its own session");
    }

    let addr = bind.parse::<std::net::SocketAddr>().context("bad --bind address")?;
    let listener = tokio::net::TcpListener::bind(addr).await.with_context(|| format!("cannot listen on {addr}"))?;
    let permits = Arc::new(tokio::sync::Semaphore::new(max_sessions));
    info!(%bind, max_sessions, "sway-rdp-server listening");

    // One RDP server per connection, all on this thread: connections are independent and each owns a session.
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async move {
            loop {
                let (stream, peer) = match listener.accept().await {
                    Ok(v) => v,
                    Err(e) => {
                        warn!("accept failed: {e}");
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                        continue;
                    }
                };
                // Counts connections, not only logged-in ones: an unauthenticated flood can use up the permits.
                let Ok(permit) = permits.clone().try_acquire_owned() else {
                    warn!(%peer, "too many connections, refused");
                    continue;
                };
                let common = common.clone();
                tokio::task::spawn_local(async move {
                    serve(stream, peer, common).await;
                    drop(permit);
                });
            }
        })
        .await;
    Ok(())
}

/// What every connection shares (read only).
struct Common {
    cfg: Arc<Config>,
    acceptor: ironrdp_server::tokio_rustls::TlsAcceptor,
    pub_key: Vec<u8>,
    tls_only: bool,
    pam_validator: Option<Arc<auth::PamValidator>>,
    credentials: (String, String),
    /// None = no graphics pipeline, bitmaps only.
    egfx: Option<encode::Tuning>,
    clipboard: bool,
    audio: bool,
}

/// Only one connection at a time can have sound: the virtual speaker is shared by the whole machine.
static AUDIO_BUSY: AtomicBool = AtomicBool::new(false);

struct AudioLease;

impl AudioLease {
    fn take() -> Option<Self> {
        (!AUDIO_BUSY.swap(true, Ordering::AcqRel)).then_some(Self)
    }
}

impl Drop for AudioLease {
    fn drop(&mut self) {
        AUDIO_BUSY.store(false, Ordering::Release);
    }
}

/// Runs one client from accept to disconnect, then removes its session.
async fn serve(stream: tokio::net::TcpStream, peer: std::net::SocketAddr, common: Arc<Common>) {
    let started = std::time::Instant::now();
    let slot = Arc::new(Slot::new());
    let egfx_state = Arc::new(egfx::EgfxState::default());
    let handler = Handler {
        slot: slot.clone(),
        cfg: common.cfg.clone(),
        egfx: common.egfx.map(|tuning| (egfx_state.clone(), tuning)),
    };
    let audio_lease = if common.audio { AudioLease::take() } else { None };
    if common.audio && audio_lease.is_none() {
        info!(%peer, "another session has the sound; this one is silent");
    }

    let builder = RdpServer::builder().with_addr(peer);
    let builder = if common.tls_only {
        builder.with_tls(common.acceptor.clone())
    } else {
        builder.with_hybrid(common.acceptor.clone(), common.pub_key.clone())
    };
    let mut server = builder
        .with_input_handler(handler.clone())
        .with_display_handler(handler)
        .with_gfx_factory(common.egfx.map(|_| Box::new(egfx::Factory::new(egfx_state.clone())) as Box<dyn ironrdp_server::GfxServerFactory>))
        .with_cliprdr_factory(common.clipboard.then(|| Box::new(clipboard::Factory::new()) as Box<dyn ironrdp_server::CliprdrServerFactory>))
        .with_sound_factory(audio_lease.is_some().then(|| Box::new(audio::Factory::new()) as Box<dyn ironrdp_server::SoundServerFactory>))
        .with_honor_client_desktop_size(true)
        .with_replace_on_new_connection(false)
        .with_credential_validator(common.pam_validator.clone().map(|v| v as Arc<dyn ironrdp_server::CredentialValidator>))
        .build();
    if common.pam_validator.is_none() {
        let (username, password) = common.credentials.clone();
        server.set_credentials(Some(Credentials { username, password, domain: None }));
    }

    info!(%peer, "client connected");
    let result = server.run_connection(stream).await;
    match &result {
        Ok(()) => info!(%peer, duration = ?started.elapsed(), "client disconnected"),
        Err(e) => info!(%peer, duration = ?started.elapsed(), error = %format!("{e:#}"), "client disconnected"),
    }
    drop(server);

    // Removing the session runs swaymsg and waits for the hook: not on the async thread.
    let _ = tokio::task::spawn_blocking(move || {
        slot.wanted.lock().unwrap().take();
        slot.resize_to.lock().unwrap().take();
        slot.session.lock().unwrap().take();
    })
    .await;
    drop(audio_lease);
}
