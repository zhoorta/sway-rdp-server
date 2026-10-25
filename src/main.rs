//! waydesk-server: an RDP server on IronRDP for a sway desktop.
//! Each connection gets its own headless output (client size and scale), captured with wlr-screencopy,
//! sent as H.264 over the graphics pipeline (bitmap updates as a fallback), with keyboard and pointer injected.

mod allow;
mod audio;
mod auth;
mod capture;
mod base64;
mod clipboard;
mod cursors;
mod egfx;
mod encode;
mod fdpass;
mod helper;
mod helperclient;
mod input;
mod link;
mod memory;
mod proto;
mod session;
mod sessionaudio;
mod sessionclip;
mod spawn;
mod tcpinfo;
mod worker;

use std::num::{NonZeroU16, NonZeroUsize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Context as _;
use ironrdp_server::{
    BitmapUpdate, Credentials, DesktopSize, DisplayUpdate, KeyboardEvent, MouseEvent, PixelFormat, RGBAPointer, RdpServer, Refusal,
    RdpServerDisplay, RdpServerDisplayUpdates, RdpServerInputHandler, TlsIdentityCtx,
};
use tracing::{error, info, warn};

use link::{Event, Link};
use proto::{Down, FrameKind};

/// Frames the session may send before the client acknowledges anything, in bitmap mode (the pipeline hands out its
/// own credits in H.264 mode).
const BITMAP_CREDITS: u16 = 3;

struct Updates {
    link: Arc<Link>,
    egfx: Option<EgfxLink>,
    start: std::time::Instant,
    /// Picture size this object was made for.
    size: (u16, u16),
    mode: Mode,
    max_qp: u8,
    /// The client was told to hide its own pointer.
    pointer_hidden: bool,
    /// What the client's pointer looks like: the shape an application asked for (a cursor name, see `cursors`), or the picture it drew.
    pointer: PointerShape,
}

enum PointerShape {
    Named(String),
    Image { hot_x: u16, hot_y: u16, width: u16, height: u16, data: Vec<u8> },
}

impl PointerShape {
    fn pointer(&self) -> RGBAPointer {
        match self {
            PointerShape::Named(name) => cursors::pointer(name),
            PointerShape::Image { hot_x, hot_y, width, height, data } => cursors::pointer_from_image(*hot_x, *hot_y, *width, *height, data),
        }
    }
}

struct EgfxLink {
    st: Arc<egfx::EgfxState>,
    out: egfx::Out,
}

#[derive(PartialEq)]
enum Mode {
    /// Waiting to see whether the client opens the graphics pipeline.
    Undecided,
    H264,
    Bitmaps,
}

#[async_trait::async_trait]
impl RdpServerDisplayUpdates for Updates {
    async fn next_update(&mut self) -> anyhow::Result<Option<DisplayUpdate>> {
        loop {
            // By default the client draws the pointer (the picture has none): say which one, the default arrow. With
            // WAYDESK_CURSOR=server the cursor is in the picture, so the client's own is made invisible: with both there are
            // two, and the one in the picture is a network round trip behind.
            if !self.pointer_hidden {
                self.pointer_hidden = true;
                return Ok(Some(if capture::cursor_in_picture() {
                    // A pointer shape with nothing in it: the Windows App did not take the "hidden pointer" update.
                    DisplayUpdate::RGBAPointer(RGBAPointer { cache_index: 0, width: 32, height: 32, hot_x: 0, hot_y: 0, data: vec![0; 32 * 32 * 4] })
                } else {
                    // The default pointer is left to the client, and FreeRDP draws nothing for it: give a pointer to draw, in the shape asked for.
                    DisplayUpdate::RGBAPointer(self.pointer.pointer())
                }));
            }
            // Graphics pipeline first: it is what the Windows App is built around. Give the client a
            // few seconds to open it before falling back to the old bitmap updates.
            if self.mode == Mode::Undecided {
                match &self.egfx {
                    Some(l) if l.st.is_ready() => self.mode = Mode::H264, // credits come with the pipeline
                    Some(_) if self.start.elapsed() < std::time::Duration::from_secs(4) => {
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                        continue;
                    }
                    _ => {
                        self.mode = Mode::Bitmaps;
                        self.link.send(Down::UseBitmaps);
                        self.link.send(Down::Credit { frames: BITMAP_CREDITS });
                    }
                }
            }

            // The client forgets its pointer shape when it activates the display again (after a resize, for one): say it again
            // from time to time, it is a few KB.
            let received = tokio::time::timeout(std::time::Duration::from_secs(2), async { self.link.events.lock().await.recv().await }).await;
            let event = match received {
                Ok(event) => event.unwrap_or(Event::Closed),
                Err(_) => {
                    // The client forgets the shape too: repeat the one in use (the empty pointer, or the shape).
                    self.pointer_hidden = false;
                    continue;
                }
            };
            match event {
                // An error here would be logged and retried at once, for ever; ending the updates ends the connection.
                Event::Closed => {
                    info!("the session process ended; closing this connection");
                    return Ok(None);
                }
                Event::Cursor(name) => {
                    if !matches!(&self.pointer, PointerShape::Named(n) if *n == name) {
                        info!(cursor = %name, "pointer shape changed");
                        self.pointer = PointerShape::Named(name);
                        self.pointer_hidden = false; // the pointer is sent again, in the new shape
                    }
                }
                Event::CursorImage { hot_x, hot_y, width, height, data } => {
                    info!(width, height, "pointer picture changed");
                    self.pointer = PointerShape::Image { hot_x, hot_y, width, height, data };
                    self.pointer_hidden = false;
                }
                Event::Resized(width, height) => {
                    if (width, height) != self.size {
                        info!(width, height, "telling the client to resize");
                        self.pointer_hidden = false; // sent again after the resize
                        return Ok(Some(DisplayUpdate::Resize(DesktopSize { width, height })));
                    }
                }
                Event::Frame { x, y, w, h, kind: FrameKind::H264, data } => {
                    let Some(link) = self.egfx.as_mut().filter(|_| self.mode == Mode::H264) else {
                        // A frame that was already on its way when we switched to bitmaps.
                        self.link.send(Down::Credit { frames: 1 });
                        continue;
                    };
                    match link.out.prepare()? {
                        egfx::Prepare::Ready => link.out.send_encoded(&data, capture::Rect { x: x.into(), y: y.into(), w: w.into(), h: h.into() }, self.max_qp)?,
                        egfx::Prepare::Unsupported => {
                            warn!("client has no AVC420; using bitmap updates");
                            self.mode = Mode::Bitmaps;
                            self.link.send(Down::UseBitmaps);
                            self.link.send(Down::Credit { frames: BITMAP_CREDITS + 1 });
                        }
                    }
                }
                Event::Frame { x, y, w, h, kind: FrameKind::Bgra, data } => {
                    self.link.send(Down::Credit { frames: 1 });
                    if self.mode != Mode::Bitmaps {
                        continue;
                    }
                    return Ok(Some(DisplayUpdate::Bitmap(BitmapUpdate {
                        x,
                        y,
                        width: NonZeroU16::new(w).context("empty damage")?,
                        height: NonZeroU16::new(h).context("empty damage")?,
                        format: PixelFormat::BgrA32,
                        data: data.into(),
                        stride: NonZeroUsize::new(usize::from(w) * 4).context("empty damage")?,
                    })));
                }
            }
        }
    }
}

/// The state of one connection. Every connection has its own `Slot`.
struct Slot {
    link: Mutex<Option<Arc<Link>>>,
    klid: AtomicU32,
    /// Scale the client asked for last, in percent.
    scale: AtomicU32,
    /// Size and scale the client asked for in its latest display layout, not applied yet.
    wanted: Mutex<Option<(u16, u16, u32)>>,
    /// A worker is applying resizes.
    applying: AtomicBool,
    /// The account that logged in (PAM logins only).
    user: Arc<Mutex<Option<auth::UserInfo>>>,
    /// The connection to the root helper, when logins and sessions go through it.
    helper: Option<Arc<helperclient::HelperConn>>,
    /// Why the helper refused a new session (`proto::REFUSED_*`), 0 when it did not.
    refused: AtomicU8,
}

impl Slot {
    fn new(user: Arc<Mutex<Option<auth::UserInfo>>>, helper: Option<Arc<helperclient::HelperConn>>) -> Self {
        Self {
            user,
            helper,
            link: Mutex::new(None),
            klid: AtomicU32::new(0),
            scale: AtomicU32::new(100),
            wanted: Mutex::new(None),
            applying: AtomicBool::new(false),
            refused: AtomicU8::new(0),
        }
    }

    fn link(&self) -> Option<Arc<Link>> {
        self.link.lock().unwrap().clone()
    }
}

/// Applies the client's wanted sizes one at a time, waiting a moment so that dragging a window edge,
/// which sends a layout for every step, ends in one resize.
async fn resize_worker(slot: Arc<Slot>) {
    loop {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            let Some((w, h, scale)) = slot.wanted.lock().unwrap().take() else { break };
            let Some(link) = slot.link() else { continue };
            if link.size() == session::clamp_size(w, h) {
                continue;
            }
            let finished = link.resized.notified();
            link.send(Down::Resize { width: w, height: h, scale_percent: scale as u16 });
            // The session answers when the output has its new size; the running `Updates` then tells the client.
            if tokio::time::timeout(std::time::Duration::from_secs(5), finished).await.is_err() {
                error!("the session did not finish resizing in 5 s");
            }
            // Let the client finish reactivating before the next resize.
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        slot.applying.store(false, Ordering::Release);
        // A layout may have arrived between the last check and clearing the flag.
        if slot.wanted.lock().unwrap().is_some() && !slot.applying.swap(true, Ordering::AcqRel) {
            continue;
        }
        break;
    }
}

/// How to start a session process.
struct Config {
    exe: PathBuf,
    args: Vec<String>,
    /// Base name of the session workspaces.
    workspace: String,
    /// Sessions run as the account that logged in, each in a sway of its own.
    per_user: bool,
}

#[derive(Clone)]
struct Handler {
    slot: Arc<Slot>,
    cfg: Arc<Config>,
    egfx: Option<(Arc<egfx::EgfxState>, encode::Tuning)>,
}

impl Handler {
    fn send(&self, cmd: input::Cmd) {
        let Some(link) = self.slot.link() else { return };
        link.send(match cmd {
            input::Cmd::Key { code, pressed } => Down::Key { code, pressed },
            input::Cmd::Move { x, y } => Down::PointerMove { x, y },
            input::Cmd::Button { code, pressed } => Down::Button { code, pressed },
            input::Cmd::Wheel { vertical, horizontal } => Down::Wheel { vertical, horizontal },
            input::Cmd::SetExtent { .. } => return,
        });
    }
}

#[async_trait::async_trait]
impl RdpServerDisplay for Handler {
    async fn size(&mut self) -> DesktopSize {
        match self.slot.link() {
            Some(l) => {
                let (width, height) = l.size();
                DesktopSize { width, height }
            }
            None => DesktopSize { width: 1680, height: 1050 },
        }
    }

    /// A client is connecting with this desktop size: give it a session of exactly that size.
    async fn request_initial_size(&mut self, client_size: DesktopSize) -> DesktopSize {
        // Also called again when the client is reactivated after a resize: the session already exists then.
        if let Some(link) = self.slot.link() {
            let (width, height) = link.size();
            return DesktopSize { width, height };
        }
        let klid = self.slot.klid.load(Ordering::Relaxed);
        info!(?client_size, klid = format!("{klid:#06x}"), "starting a session process");
        if let Some(helper) = self.slot.helper.clone() {
            // The root helper starts the session as the account that logged in and hands us its socket.
            let started = tokio::task::spawn_blocking(move || helper.start_session()).await;
            let result = match started {
                Ok(Ok(stream)) => Link::from_helper(stream, (client_size.width, client_size.height), klid).await,
                Ok(Err(e)) => Err(e),
                Err(e) => Err(e.into()),
            };
            return match result {
                Ok(link) => {
                    if let Some((st, tuning)) = &self.egfx {
                        let to = link.clone();
                        st.set_credit_sink(Box::new(move |frames| to.send(Down::Credit { frames })));
                        let to = link.clone();
                        st.set_bitrate_sink(tuning.bitrate, Box::new(move |bps| to.send(Down::Bitrate { bps })));
                    }
                    let (width, height) = link.size();
                    *self.slot.link.lock().unwrap() = Some(link);
                    DesktopSize { width, height }
                }
                Err(e) => {
                    error!("cannot create session: {e:#}");
                    if let Some(r) = e.downcast_ref::<helperclient::SessionRefused>() {
                        self.slot.refused.store(r.reason, Ordering::Relaxed);
                    }
                    client_size
                }
            };
        }
        let user = if self.cfg.per_user { self.slot.user.lock().unwrap().clone() } else { None };
        if self.cfg.per_user && user.is_none() {
            error!("no account for this connection");
            return client_size;
        }
        match Link::spawn(&self.cfg.exe, &self.cfg.args, &self.cfg.workspace, (client_size.width, client_size.height), klid, user.as_ref(), self.cfg.per_user).await {
            Ok(link) => {
                if let Some((st, tuning)) = &self.egfx {
                    let to = link.clone();
                    st.set_credit_sink(Box::new(move |frames| to.send(Down::Credit { frames })));
                    let to = link.clone();
                    st.set_bitrate_sink(tuning.bitrate, Box::new(move |bps| to.send(Down::Bitrate { bps })));
                }
                let (width, height) = link.size();
                *self.slot.link.lock().unwrap() = Some(link);
                DesktopSize { width, height }
            }
            Err(e) => {
                error!("cannot create session: {e:#}");
                client_size
            }
        }
    }

    /// Why there is no session, when the helper refused one on purpose: the client is told with an error code.
    fn refusal(&mut self) -> Option<Refusal> {
        match self.slot.refused.load(Ordering::Relaxed) {
            proto::REFUSED_MEMORY => Some(Refusal::OutOfMemory),
            proto::REFUSED_USERS => Some(Refusal::Denied),
            _ => None,
        }
    }

    async fn updates(&mut self) -> anyhow::Result<Box<dyn RdpServerDisplayUpdates>> {
        let link = self.slot.link().context("no session for this connection")?;
        // New connection: the client has nothing yet, and drops what it is sent before it is ready.
        // Ask for the whole picture a few times while it comes up (the DisplayControl layout below is the real signal).
        link.send(Down::RequestFull);
        let timers = link.clone();
        tokio::spawn(async move {
            for secs in [1u64, 2, 4, 8] {
                tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
                timers.send(Down::RequestFull);
            }
        });
        let size = link.size();
        // After a resize the session starts again from zero credits (frames in flight died with the reactivation) and
        // the channel stays up, so nothing grants new ones: without this a still picture never gets going again.
        // A new connection gets its first credits when the channel reports ready.
        if let Some((st, _)) = &self.egfx {
            if st.is_ready() {
                st.reset_credits();
            }
        }
        let egfx = self.egfx.as_ref().map(|(st, _)| EgfxLink { st: st.clone(), out: egfx::Out::new(st.clone(), size.0, size.1) });
        let max_qp = self.egfx.as_ref().map_or(0, |(_, t)| t.max_qp.clamp(1, 51));
        Ok(Box::new(Updates { link, egfx, start: std::time::Instant::now(), size, mode: Mode::Undecided, max_qp, pointer_hidden: false, pointer: PointerShape::Named("default".into()) }))
    }

    fn request_layout(&mut self, layout: ironrdp_displaycontrol::pdu::DisplayControlMonitorLayout) {
        tracing::debug!(?layout, "client layout");
        let Some(m) = layout.monitors().iter().find(|m| m.is_primary()).or(layout.monitors().first()) else { return };
        let (w, h) = m.dimensions();
        let scale = m.desktop_scale_factor().unwrap_or(100);
        let Some(link) = self.slot.link() else { return };
        info!(w, h, scale, "client display layout");
        let current = link.size();
        // A new scale applies at once; a new size waits a moment (see `resize_worker`).
        if self.slot.scale.swap(scale, Ordering::AcqRel) != scale {
            link.send(Down::Resize { width: current.0, height: current.1, scale_percent: scale as u16 });
        }
        let (w, h) = session::clamp_size(w.min(u32::from(u16::MAX)) as u16, h.min(u32::from(u16::MAX)) as u16);
        if (w, h) != current {
            *self.slot.wanted.lock().unwrap() = Some((w, h, scale));
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
                if let Some(link) = self.slot.link() {
                    link.send(Down::RequestFull);
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
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_env("WAYDESK_LOG").unwrap_or_else(|_| "info".into()))
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
    // On by default (text only); `--no-clipboard` turns it off. It needs wl-clipboard in the session.
    let mut clipboard = true;
    let mut audio = true;
    let mut max_sessions = 64usize;
    // Internal: run as the session process of one connection (started by the daemon, with a socket as fd 3).
    let mut worker = false;
    let mut own_sway = false;
    // One sway per logged-in account, started as that account (needs --auth pam, and root for other accounts).
    let mut per_user = false;
    let mut allow_users: Option<Vec<String>> = None;
    let mut require_group: Option<String> = None;
    // Source addresses that may connect (see `allow`): none given = loopback allows all, any other address only local networks.
    let mut allow_from: Option<String> = std::env::var("WAYDESK_ALLOW_FROM").ok();
    // The root helper (`--helper`), or the daemon using one (`--auth helper`).
    let mut run_helper = false;
    let mut helper_socket = PathBuf::from("/run/waydesk/helper.sock");
    let mut daemon_user = "waydesk".to_string();
    let mut systemd_sessions = false;
    // A new session needs this much available memory (MB; automatic when not given, 0 = no check), and at most this many accounts
    // may have a session (0 = no limit). Accounts that already have one just reconnect.
    let mut min_available_mb: Option<u64> = None;
    let mut max_active_users = 0usize;
    // How long a session without a client lives (seconds, 0 = for ever), and whether it ends when the client leaves.
    let mut idle_timeout = 8 * 3600u64;
    let mut end_on_disconnect = false;
    // Internal: where a persistent session registers (set by the helper).
    let mut register: Option<PathBuf> = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--bind" => bind = args.next().context("--bind needs a value")?,
            "--cert" => cert = args.next().map(PathBuf::from),
            "--key" => key = args.next().map(PathBuf::from),
            "--kb-layout" => layout = args.next().context("--kb-layout needs a value (used when the client announces none)")?,
            "--no-terminal" => spawn_terminal = false,
            "--workspace" => workspace = args.next().context("--workspace needs a name")?,
            // Started per session with WAYDESK_OUTPUT, WAYDESK_WORKSPACE and WAYDESK_INDEX set; stopped when it ends.
            "--session-hook" => hook = args.next().map(PathBuf::from),
            "--no-egfx" => no_egfx = true,
            "--session-worker" => worker = true,
            "--own-sway" => own_sway = true,
            "--helper" => run_helper = true,
            "--systemd-sessions" => systemd_sessions = true,
            "--min-available-mb" => min_available_mb = Some(args.next().context("--min-available-mb needs a number (0 = no check)")?.parse()?),
            "--max-active-users" => max_active_users = args.next().context("--max-active-users needs a number (0 = no limit)")?.parse()?,
            "--idle-timeout" => idle_timeout = args.next().context("--idle-timeout needs seconds")?.parse()?,
            "--end-on-disconnect" => end_on_disconnect = true,
            "--register" => register = args.next().map(PathBuf::from),
            "--helper-socket" => helper_socket = PathBuf::from(args.next().context("--helper-socket needs a path")?),
            "--daemon-user" => daemon_user = args.next().context("--daemon-user needs an account name")?,
            "--per-user" => per_user = true,
            "--allow-from" => allow_from = Some(args.next().context("--allow-from needs a list of addresses, ranges, private, loopback or any")?),
            "--require-group" => require_group = Some(args.next().context("--require-group needs a group name")?),
            "--allow-users" => allow_users = Some(args.next().context("--allow-users needs a comma separated list")?.split(',').map(str::to_owned).collect()),
            // A new connection ends the open one. It does so before the new client has logged in, so use
            // it only where reaching the port already means trust (loopback, SSH tunnel, VPN).
            "--tls-only" => tls_only = true,
            "--auth" => auth_mode = args.next().context("--auth needs file or pam")?,
            "--no-replace" => replace = Some(false),
            "--replace" => replace = Some(true),
            // First line "user:password" (split at the first colon). No built-in default credential.
            "--credentials" => creds_file = args.next().map(PathBuf::from),
            "--clipboard" => clipboard = true,
            "--no-clipboard" => clipboard = false,
            "--no-audio" => audio = false,
            "--max-sessions" => max_sessions = args.next().context("--max-sessions needs a number")?.parse()?,
            "--bitrate" => bitrate = args.next().context("--bitrate needs a value")?.parse()?,
            "--quality" => max_qp = args.next().context("--quality needs a value (max QP, lower = better)")?.parse()?,
            other => anyhow::bail!("unknown argument {other}"),
        }
    }
    if worker {
        worker::prefer_private_bin();
        let tuning = (!no_egfx).then_some(encode::Tuning { bitrate, max_qp, fps: 30 });
        let opts = session::Options { default_layout: layout, spawn_terminal, workspace, hook };
        return worker::run(worker::Config { opts, tuning, own_sway, register, idle_timeout: std::time::Duration::from_secs(idle_timeout), end_on_disconnect }).await;
    }
    // What the session processes are started with.
    let mut worker_args = vec!["--kb-layout".to_string(), layout.clone(), "--workspace".into(), workspace.clone()];
    worker_args.extend(["--bitrate".to_string(), bitrate.to_string(), "--quality".into(), max_qp.to_string()]);
    if !spawn_terminal {
        worker_args.push("--no-terminal".into());
    }
    if no_egfx {
        worker_args.push("--no-egfx".into());
    }
    worker_args.extend(["--idle-timeout".to_string(), idle_timeout.to_string()]);
    if end_on_disconnect {
        worker_args.push("--end-on-disconnect".into());
    }
    if let Some(h) = &hook {
        worker_args.extend(["--session-hook".to_string(), h.display().to_string()]);
    }
    if run_helper {
        let exe = std::env::current_exe().context("path of this program")?;
        return helper::run(helper::Config { socket: helper_socket, daemon_user, allow_users, require_group, exe, worker_args, workspace, systemd_sessions, min_available_mb, max_active_users });
    }
    anyhow::ensure!(matches!(auth_mode.as_str(), "file" | "pam" | "helper"), "--auth must be file, pam or helper");
    anyhow::ensure!(!per_user || auth_mode == "pam", "--per-user needs --auth pam (the account comes from the login)");
    let via_helper = auth_mode == "helper";
    anyhow::ensure!(!via_helper || unsafe { libc::geteuid() } != 0, "--auth helper: the network daemon refuses to run as root (start waydesk-server --helper as root and this as an ordinary service account)");
    let pam = auth_mode == "pam";
    let tls_only = tls_only || pam || via_helper;
    let (username, password) = if pam || via_helper {
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
        cfg: Arc::new(Config { exe: std::env::current_exe().context("path of this program")?, args: worker_args, workspace: workspace.clone(), per_user }),
        acceptor,
        pub_key: identity.pub_key,
        tls_only,
        pam: if pam { Some(auth::PamPolicy::new("waydesk-server", allow_users, require_group)?) } else { None },
        credentials: (username, password),
        helper_socket: via_helper.then_some(helper_socket),
        egfx: (!no_egfx).then_some(encode::Tuning { bitrate, max_qp, fps: 30 }),
        clipboard,
        audio: audio && !per_user,
    });
    if let Some(p) = &common.pam {
        info!(user = p.running_user(), root = p.is_root(), per_user, "login: PAM service waydesk-server, plain TLS login (no NLA)");
        if p.is_root() {
            warn!("running as root: any ordinary account can log in and the whole daemon, including the network side, has root rights. Development only (see docs/MULTIUSER-DESIGN.md).");
        }
    }
    if per_user && audio {
        warn!("--per-user: sound is off (it needs the root helper: the sound is recorded in the account's own session)");
    }
    if replace == Some(true) {
        warn!("--replace is ignored: every connection now gets its own session");
    }

    let addr = bind.parse::<std::net::SocketAddr>().context("bad --bind address")?;
    let listener = tokio::net::TcpListener::bind(addr).await.with_context(|| format!("cannot listen on {addr}"))?;
    let allow = Arc::new(allow::Allow::from_config(allow_from.as_deref(), &addr).map_err(|e| anyhow::anyhow!("--allow-from / WAYDESK_ALLOW_FROM: {e}"))?);
    if !addr.ip().is_loopback() && allow.is_any() {
        warn!(%bind, "listening for any source address: the login is a password with no second factor, keep this port off the open internet (WAYDESK_ALLOW_FROM limits who may connect)");
    }
    let refusals = Arc::new(allow::RefusalLog::new());
    let permits = Arc::new(tokio::sync::Semaphore::new(max_sessions));
    info!(%bind, max_sessions, allowed = %allow_from.as_deref().unwrap_or(if addr.ip().is_loopback() { "any (loopback)" } else { "private" }), "waydesk-server listening");

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
                // Nothing is said to an address that is not allowed: no TLS, no login, no permit.
                if !allow.permits(peer.ip()) {
                    if let Some(more) = refusals.note() {
                        warn!(%peer, also_refused = more, "connection refused: the source address is not in the allowed list (WAYDESK_ALLOW_FROM)");
                    }
                    continue;
                }
                // Counts connections, not only logged-in ones: an unauthenticated flood can use up the permits.
                let Ok(permit) = permits.clone().try_acquire_owned() else {
                    warn!(%peer, "too many connections, refused");
                    continue;
                };
                limit_stall(&stream, peer);
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

/// A client that stops reading (frozen, or its network gone) leaves data in the send queue; the kernel's own limit for that is about
/// 15 minutes, and a client that still answers the zero-window probes keeps the connection for ever, with its task, buffers and slot.
/// This ends such a connection after STALL_LIMIT without progress, and probes idle ones so that a silent peer is noticed too.
const STALL_LIMIT: std::time::Duration = std::time::Duration::from_secs(60);

fn limit_stall(stream: &tokio::net::TcpStream, peer: std::net::SocketAddr) {
    use std::os::fd::AsRawFd;
    let fd = stream.as_raw_fd();
    let set = |level: libc::c_int, name: libc::c_int, value: libc::c_int| {
        let r = unsafe { libc::setsockopt(fd, level, name, (&value as *const libc::c_int).cast(), std::mem::size_of::<libc::c_int>() as libc::socklen_t) };
        if r != 0 {
            warn!(%peer, "cannot set socket option {name}: {}", std::io::Error::last_os_error());
        }
    };
    set(libc::IPPROTO_TCP, libc::TCP_USER_TIMEOUT, STALL_LIMIT.as_millis() as libc::c_int);
    set(libc::SOL_SOCKET, libc::SO_KEEPALIVE, 1);
    set(libc::IPPROTO_TCP, libc::TCP_KEEPIDLE, 30);
    set(libc::IPPROTO_TCP, libc::TCP_KEEPINTVL, 10);
    set(libc::IPPROTO_TCP, libc::TCP_KEEPCNT, 3);
    set_congestion_control(fd, peer);
}

/// The congestion control of the connection: BBR unless `WAYDESK_TCP_CONGESTION` names another one (empty: the system's default). On a link
/// that loses packets at random (wireless, mobile, a long way away) CUBIC takes every loss for congestion and ends up with a fraction of what
/// the link carries (measured: 0.3 Mbit/s against 2 to 4 with BBR, and stalls of a second after each loss that made typed letters arrive in
/// bursts); BBR does not. A process that is not root may only use what `net.ipv4.tcp_allowed_congestion_control` lists: the package adds bbr to it and
/// loads the module. Where it is not there the connection keeps the system's default.
fn set_congestion_control(fd: std::os::fd::RawFd, peer: std::net::SocketAddr) {
    static REPORTED: AtomicBool = AtomicBool::new(false);
    let name = std::env::var("WAYDESK_TCP_CONGESTION").unwrap_or_else(|_| "bbr".into());
    if name.is_empty() {
        return;
    }
    let r = unsafe { libc::setsockopt(fd, libc::IPPROTO_TCP, libc::TCP_CONGESTION, name.as_ptr().cast(), name.len() as libc::socklen_t) };
    if r != 0 && !REPORTED.swap(true, Ordering::Relaxed) {
        info!(%peer, "TCP congestion control {name} is not available ({}): connections keep the system's default", std::io::Error::last_os_error());
    }
}

/// How long a connection may take to log in.
const PRE_LOGIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// What every connection shares (read only).
struct Common {
    cfg: Arc<Config>,
    acceptor: ironrdp_server::tokio_rustls::TlsAcceptor,
    pub_key: Vec<u8>,
    tls_only: bool,
    pam: Option<Arc<auth::PamPolicy>>,
    /// Logins and sessions come from the root helper at this socket.
    helper_socket: Option<PathBuf>,
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
    let login = Arc::new(Mutex::new(None));
    let helper = match &common.helper_socket {
        Some(path) => {
            let path = path.clone();
            match tokio::task::spawn_blocking(move || helperclient::HelperConn::connect(&path)).await {
                Ok(Ok(conn)) => Some(Arc::new(conn)),
                other => {
                    error!(%peer, "cannot reach the helper: {:?}", other.map(|r| r.err()));
                    return;
                }
            }
        }
        None => None,
    };
    let slot = Arc::new(Slot::new(login.clone(), helper.clone()));
    // Set when a login was accepted, whichever way logins are checked.
    let authed = Arc::new(AtomicBool::new(false));
    let egfx_state = Arc::new(egfx::EgfxState::default());
    {
        use std::os::fd::AsRawFd;
        egfx_state.set_socket(stream.as_raw_fd()); // the kernel tells what the link delivers (see egfx::Flow)
    }
    let handler = Handler {
        slot: slot.clone(),
        cfg: common.cfg.clone(),
        egfx: common.egfx.map(|tuning| (egfx_state.clone(), tuning)),
    };
    // With the root helper every account records in its own session; without it the machine has one speaker.
    let audio_lease = if common.audio && helper.is_none() { AudioLease::take() } else { None };
    if common.audio && helper.is_none() && audio_lease.is_none() {
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
        .with_cliprdr_factory(common.clipboard.then(|| Box::new(clipboard::Factory::new(slot.clone())) as Box<dyn ironrdp_server::CliprdrServerFactory>))
        .with_sound_factory(if !common.audio {
            None
        } else if helper.is_some() {
            Some(Box::new(audio::Factory::remote(slot.clone())) as Box<dyn ironrdp_server::SoundServerFactory>)
        } else {
            audio_lease.is_some().then(|| Box::new(audio::Factory::new()) as Box<dyn ironrdp_server::SoundServerFactory>)
        })
        .with_honor_client_desktop_size(true)
        .with_replace_on_new_connection(false)
        .with_credential_validator(match (&common.pam, &helper) {
            (Some(p), _) => Some(Arc::new(auth::PamValidator::new(p.clone(), login.clone(), peer.ip().to_string())) as Arc<dyn ironrdp_server::CredentialValidator>),
            (None, Some(h)) => Some(Arc::new(helperclient::HelperValidator::new(h.clone(), peer.ip().to_string(), authed.clone())) as Arc<dyn ironrdp_server::CredentialValidator>),
            (None, None) => None,
        })
        .build();
    if common.pam.is_none() && helper.is_none() {
        let (username, password) = common.credentials.clone();
        server.set_credentials(Some(Credentials { username, password, domain: None }));
    }

    info!(%peer, "client connected");
    // A connection that does not log in soon is cut: otherwise a few idle connections would use up all the slots.
    let needs_login = common.pam.is_some() || helper.is_some();
    let login_deadline = {
        let (authed, login) = (authed.clone(), login.clone());
        async move {
            tokio::time::sleep(PRE_LOGIN_TIMEOUT).await;
            if needs_login && !authed.load(Ordering::Acquire) && login.lock().unwrap().is_none() {
                return;
            }
            std::future::pending::<()>().await
        }
    };
    let result = tokio::select! {
        r = server.run_connection(stream) => r,
        _ = login_deadline => Err(anyhow::anyhow!("no login within {} s", PRE_LOGIN_TIMEOUT.as_secs())),
    };
    egfx_state.clear_socket(); // the connection is closed
    match &result {
        Ok(()) => info!(%peer, duration = ?started.elapsed(), "client disconnected"),
        Err(e) => info!(%peer, duration = ?started.elapsed(), error = %format!("{e:#}"), "client disconnected"),
    }
    drop(server);

    // The session process removes its output and stops the hook; wait for it.
    slot.wanted.lock().unwrap().take();
    let link = slot.link.lock().unwrap().take();
    if let Some(link) = link {
        link.shutdown().await;
    }
    drop(audio_lease);
}
