//! Graphics pipeline (EGFX) output: the captured frames go to the client as H.264 (AVC420).
//! The Windows App on macOS is built around this path and mishandles the older bitmap updates.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, Result};
use ironrdp_egfx::pdu::{Avc420Region, CapabilitySet};
use ironrdp_egfx::server::{GraphicsPipelineHandler, GraphicsPipelineServer};
use ironrdp_server::tokio::sync::mpsc::UnboundedSender;
use ironrdp_server::{EgfxServerMessage, GfxDvcBridge, GfxServerFactory, GfxServerHandle, ServerEvent, ServerEventSender};
use tracing::{debug, info, warn};

use crate::capture::Rect;
use crate::encode::surface_size;


/// State shared between the DVC callbacks (which run with the server locked) and the display loop.
#[derive(Default)]
pub struct EgfxState {
    ready: AtomicBool,
    /// Frames that may still be sent. The client's acknowledgements give them back; see `Credits` below.
    credits: AtomicU32,
    credit_notify: tokio::sync::Notify,
    sent: AtomicU64,
    acked: AtomicU64,
    handle: Mutex<Option<GfxServerHandle>>,
    sender: Mutex<Option<UnboundedSender<ServerEvent>>>,
}

/// Frames that may be in flight (sent, not acknowledged yet). Matches the pipeline's own default
/// (`ironrdp_egfx` DEFAULT_MAX_FRAMES_IN_FLIGHT), which our handler does not change.
const MAX_CREDITS: u32 = 3;
/// The client's queue depth when it has switched acknowledgements off: it will never give a credit back.
const ACKS_SUSPENDED: u32 = 0xFFFF_FFFF;

// Credits are the flow control between whoever encodes and whoever talks to the client. The sender may only
// encode and send while it holds one; each frame uses one, each acknowledgement returns one. With the encoder in a
// separate process the same numbers travel as `proto::Down::Credit` messages.
impl EgfxState {
    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }

    pub fn credits(&self) -> u32 {
        self.credits.load(Ordering::Acquire)
    }

    /// Waits until at least one frame may be sent.
    pub async fn wait_credit(&self) {
        loop {
            let notified = self.credit_notify.notified();
            if self.credits() > 0 {
                return;
            }
            notified.await;
        }
    }

    /// Atomically replaces the credits with `f(credits)`.
    fn update_credits(&self, f: impl Fn(u32) -> u32) {
        let mut cur = self.credits.load(Ordering::Acquire);
        while let Err(seen) = self.credits.compare_exchange_weak(cur, f(cur), Ordering::AcqRel, Ordering::Acquire) {
            cur = seen;
        }
    }

    fn take_credit(&self) {
        self.update_credits(|c| c.saturating_sub(1));
    }

    fn give_credit(&self) {
        self.update_credits(|c| (c + 1).min(MAX_CREDITS));
        self.credit_notify.notify_one();
    }

    fn set_credits(&self, n: u32) {
        self.credits.store(n, Ordering::Release);
        self.credit_notify.notify_one();
    }

    fn handle(&self) -> Option<GfxServerHandle> {
        self.handle.lock().unwrap().clone()
    }

    /// Drain what the pipeline queued and push it to the client.
    fn flush(&self, server: &mut GraphicsPipelineServer) -> Result<()> {
        let Some(channel_id) = server.channel_id() else { return Ok(()) };
        let messages = server.drain_output();
        if messages.is_empty() {
            return Ok(());
        }
        let svc = ironrdp_dvc::encode_dvc_messages(channel_id, messages, ironrdp_svc::ChannelFlags::SHOW_PROTOCOL)
            .map_err(|e| anyhow::anyhow!("encode dvc messages: {e:?}"))?;
        let sender = self.sender.lock().unwrap();
        let sender = sender.as_ref().context("no server event sender")?;
        sender
            .send(ServerEvent::Egfx(EgfxServerMessage::SendMessages { messages: svc }))
            .map_err(|_| anyhow::anyhow!("server event channel closed"))
    }
}

struct Handler {
    st: Arc<EgfxState>,
}

impl GraphicsPipelineHandler for Handler {
    fn capabilities_advertise(&mut self, pdu: &ironrdp_egfx::pdu::CapabilitiesAdvertisePdu) {
        debug!(?pdu, "client EGFX capabilities");
    }

    fn on_ready(&mut self, negotiated: &CapabilitySet) {
        info!(?negotiated, "EGFX ready");
        self.st.set_credits(MAX_CREDITS);
        self.st.ready.store(true, Ordering::Release);
    }

    fn on_frame_ack(&mut self, frame_id: u32, queue_depth: u32, total_frames_decoded: u32) {
        if queue_depth == ACKS_SUSPENDED {
            self.st.set_credits(1000);
        } else {
            self.st.give_credit();
        }
        let n = self.st.acked.fetch_add(1, Ordering::Relaxed) + 1;
        if n <= 3 || n % 200 == 0 {
            info!(frame_id, queue_depth, total_frames_decoded, acked = n, sent = self.st.sent.load(Ordering::Relaxed), "client acknowledged frame");
        }
    }

    fn on_close(&mut self) {
        self.st.credits.store(0, Ordering::Release);
        self.st.ready.store(false, Ordering::Release);
    }
}

pub struct Factory {
    st: Arc<EgfxState>,
}

impl Factory {
    pub fn new(st: Arc<EgfxState>) -> Self {
        Self { st }
    }
}

impl ServerEventSender for Factory {
    fn set_sender(&mut self, sender: UnboundedSender<ServerEvent>) {
        *self.st.sender.lock().unwrap() = Some(sender);
    }
}

impl GfxServerFactory for Factory {
    fn build_gfx_handler(&self) -> Box<dyn GraphicsPipelineHandler> {
        Box::new(Handler { st: self.st.clone() })
    }

    fn build_server_with_handle(&self) -> Option<(GfxDvcBridge, GfxServerHandle)> {
        // New client: nothing is ready until its capabilities arrive.
        self.st.ready.store(false, Ordering::Release);
        self.st.credits.store(0, Ordering::Release);
        // The desktop size is only known per session; `Out::send` sends ResetGraphics with it.
        let server = GraphicsPipelineServer::new(Box::new(Handler { st: self.st.clone() }));
        let handle: GfxServerHandle = Arc::new(Mutex::new(server));
        *self.st.handle.lock().unwrap() = Some(handle.clone());
        Some((GfxDvcBridge::new(handle.clone()), handle))
    }
}

/// Per-connection sender of encoded frames: owns the surface. The encoder lives in `encode.rs`.
pub struct Out {
    st: Arc<EgfxState>,
    width: usize,
    height: usize,
    /// Surface size: see `encode::surface_size`.
    sw: usize,
    sh: usize,
    surface: Option<u16>,
    start: std::time::Instant,
}

/// Whether the client can take frames at all.
pub enum Prepare {
    /// Yes: the surface exists. (Whether there is room right now is a matter of credits.)
    Ready,
    /// The client cannot do AVC420: use the bitmap path instead.
    Unsupported,
}

impl Out {
    pub fn new(st: Arc<EgfxState>, width: u16, height: u16) -> Self {
        let (width, height) = (usize::from(width), usize::from(height));
        let (sw, sh) = surface_size(width, height);
        Self { st, width, height, sw, sh, surface: None, start: std::time::Instant::now() }
    }

    pub fn needs_surface(&self) -> bool {
        self.surface.is_none()
    }

    /// Checks the client and creates the surface the first time. Call before encoding a frame, after a credit is there.
    pub fn prepare(&mut self) -> Result<Prepare> {
        let handle = self.st.handle().context("no EGFX server handle")?;
        let mut server = handle.lock().unwrap();
        if !server.supports_avc420() {
            return Ok(Prepare::Unsupported);
        }
        if self.surface.is_none() {
            let monitor = ironrdp_pdu::gcc::Monitor {
                left: 0,
                top: 0,
                right: self.width as i32 - 1,
                bottom: self.height as i32 - 1,
                flags: ironrdp_pdu::gcc::MonitorFlags::PRIMARY,
            };
            server.resize_with_monitors(self.width as u16, self.height as u16, vec![monitor]);
            let id = server.create_surface(self.sw as u16, self.sh as u16).context("create_surface")?;
            server.map_surface_to_output(id, 0, 0);
            self.surface = Some(id);
            info!(surface = id, "EGFX surface created {}x{}", self.sw, self.sh);
        }
        Ok(Prepare::Ready)
    }

    /// Pushes out what the pipeline queued without sending a frame (the encoder produced nothing).
    pub fn flush(&mut self) -> Result<()> {
        let handle = self.st.handle().context("no EGFX server handle")?;
        let mut server = handle.lock().unwrap();
        self.st.flush(&mut server)
    }

    /// Sends one encoded frame for the changed rectangle `dirty`. `prepare` must have returned `Ready`.
    pub fn send_encoded(&mut self, data: &[u8], dirty: Rect, max_qp: u8) -> Result<()> {
        let handle = self.st.handle().context("no EGFX server handle")?;
        let mut server = handle.lock().unwrap();
        let surface = self.surface.context("send_encoded before prepare")?;
        // The crate documents these bounds as inclusive, but they go on the wire unchanged and the wire format
        // is exclusive on the right and bottom: inclusive values left a 1 px line stale along the edges.
        let region = Avc420Region {
            left: dirty.x as u16,
            top: dirty.y as u16,
            right: (dirty.x + dirty.w) as u16,
            bottom: (dirty.y + dirty.h) as u16,
            quantization_parameter: max_qp,
            quality: 100,
        };
        let ts = self.start.elapsed().as_millis() as u32;
        let n = self.st.sent.fetch_add(1, Ordering::Relaxed) + 1;
        if n <= 3 {
            info!(n, bytes = data.len(), ?dirty, left = region.left, top = region.top, right = region.right, bottom = region.bottom, "sending first frames");
        }
        self.st.take_credit();
        if server.send_avc420_frame(surface, data, &[region], ts).is_none() {
            // Refused frames are never acknowledged, so the credit would be lost for good.
            self.st.give_credit();
            warn!("send_avc420_frame refused a frame after encoding it");
        }
        self.st.flush(&mut server)
    }
}
