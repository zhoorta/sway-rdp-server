//! The session process (`sway-rdp-server --session-worker`): one user's output, capture, encoder and input.
//! It talks to the network daemon over the socket it inherits as fd 3, using `proto`. It never touches the
//! network: it encodes what the daemon has credits for and injects the input the daemon forwards.

use std::os::fd::FromRawFd;
use std::sync::Arc;

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

pub struct Config {
    pub opts: session::Options,
    /// None: no H.264, bitmaps only.
    pub tuning: Option<Tuning>,
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
}

pub async fn run(cfg: Config) -> Result<()> {
    // The daemon passes its end of a socket pair as fd 3.
    let std_stream = unsafe { std::os::unix::net::UnixStream::from_raw_fd(3) };
    std_stream.set_nonblocking(true).context("socket")?;
    let (mut rd, mut wr) = tokio::net::UnixStream::from_std(std_stream)?.into_split();

    let hello = proto::read_frame_async(&mut rd).await?.context("daemon closed before hello")?;
    let Down::Hello { version, width, height, klid, .. } = Down::decode(&hello)? else { bail!("first message is not a hello") };
    if version != proto::VERSION {
        bail!("daemon speaks protocol {version}, this is {}", proto::VERSION);
    }

    // Writer and reader tasks: the main loop must never read a half message inside `select!`.
    let (up_tx, mut up_rx) = mpsc::unbounded_channel::<Up>();
    tokio::spawn(async move {
        while let Some(msg) = up_rx.recv().await {
            if proto::write_frame_async(&mut wr, &msg.encode()).await.is_err() {
                break;
            }
        }
    });
    let (down_tx, mut down_rx) = mpsc::unbounded_channel::<Down>();
    tokio::spawn(async move {
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

    let layout = if klid == 0 { cfg.opts.default_layout.clone() } else { session::xkb_layout_for_klid(klid).to_owned() };
    info!(width, height, klid = format!("{klid:#06x}"), %layout, "creating session");
    let opts = cfg.opts;
    let session = tokio::task::spawn_blocking(move || Session::create((width, height), &layout, &opts))
        .await
        .context("session task")??;
    let (w, h) = (session.shared.width, session.shared.height);
    let mut me = Worker { session: Some(session), enc: None, tuning: cfg.tuning, credits: 0, bitmaps: cfg.tuning.is_none(), up: up_tx };
    let _ = me.up.send(Up::Ready { width: w, height: h });

    // A stop request (systemd stopping the service, Ctrl-C) must still remove the output and stop the hook.
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut int = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    loop {
        let shared = me.session.as_ref().expect("session").shared.clone();
        tokio::select! {
            _ = term.recv() => break,
            _ = int.recv() => break,
            msg = down_rx.recv() => match msg {
                None | Some(Down::Bye) => break,
                Some(m) => me.handle(m).await,
            },
            // Only take a frame when there is room for it: changes keep merging into the dirty rectangle meanwhile.
            (frame, rect) = shared.next(), if me.credits > 0 => {
                if let Err(e) = me.send_frame(&frame, rect).await {
                    error!("sending a frame failed: {e:#}");
                    break;
                }
            }
        }
    }
    let _ = me.up.send(Up::Bye);
    info!("session worker ending");
    // Dropping the session removes the output and stops the hook.
    drop(me);
    Ok(())
}

impl Worker {
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
