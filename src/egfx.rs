//! Graphics pipeline (EGFX) output: the captured frames go to the client as H.264 (AVC420).
//! The Windows App on macOS is built around this path and mishandles the older bitmap updates.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

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
    /// Where credits go: to the session process, which only encodes while it holds one.
    credit_sink: Mutex<Option<Box<dyn Fn(u16) + Send + Sync>>>,
    sent: AtomicU64,
    acked: AtomicU64,
    handle: Mutex<Option<GfxServerHandle>>,
    sender: Mutex<Option<UnboundedSender<ServerEvent>>>,
    flow: Mutex<Flow>,
}

/// Frames in flight (sent, not acknowledged yet) are limited by credits. The frame rate cannot exceed the frames in flight
/// divided by the round trip time, so 3 (the pipeline's own default) suits a local network but a client 100 ms away needs
/// about 8. Unless `WAYDESK_FRAMES_IN_FLIGHT` (1 to 32) fixes the number, the window follows the round trip measured from the
/// acknowledgements (see `Flow`), between `MIN_WINDOW` and `MAX_WINDOW`.
const MIN_WINDOW: u32 = 3;
const MAX_WINDOW: u32 = 16;
/// The window aims at this many frames per second at the measured round trip.
const TARGET_FRAME_MS: u32 = 16;
/// How long the lowest round trip is collected before the window is reconsidered.
const PERIOD: Duration = Duration::from_secs(2);
/// Round trips of the first frames of a connection are not counted.
const FIRST_FRAMES: u32 = 5;

fn fixed_window() -> Option<u16> {
    static VALUE: OnceLock<Option<u16>> = OnceLock::new();
    *VALUE.get_or_init(|| std::env::var("WAYDESK_FRAMES_IN_FLIGHT").ok().and_then(|v| v.trim().parse::<u16>().ok()).map(|n| n.clamp(1, 32)))
}

/// The most frames the pipeline itself accepts in flight: the credits stay below it.
pub fn max_credits() -> u16 {
    fixed_window().unwrap_or(MAX_WINDOW as u16)
}

/// The adaptive credit window of one connection. `issued` counts the credits out there (held by the session or attached to a
/// frame in flight); an acknowledgement ends a frame and tops `issued` up to `window`, so a larger window hands out extra
/// credits and a smaller one simply gives fewer back.
struct Flow {
    window: u32,
    issued: u32,
    sent_at: VecDeque<(u32, Instant)>,
    period_start: Instant,
    period_min: Option<Duration>,
    /// Acknowledgements still to ignore: the first frame is the whole picture and takes far longer than any later one.
    skip: u32,
}

impl Default for Flow {
    fn default() -> Self {
        Self::new()
    }
}

impl Flow {
    fn new() -> Self {
        let window = fixed_window().map_or(MIN_WINDOW + 1, u32::from);
        Self { window, issued: 0, sent_at: VecDeque::new(), period_start: Instant::now(), period_min: None, skip: FIRST_FRAMES }
    }

    /// Notes a round trip sample; at the end of a period moves the window towards what the lowest round trip needs
    /// (the lowest, because the queueing a big window causes would otherwise make it ask for even more).
    fn sample(&mut self, rtt: Duration) {
        if fixed_window().is_some() {
            return;
        }
        if self.skip > 0 {
            self.skip -= 1;
            return;
        }
        self.period_min = Some(self.period_min.map_or(rtt, |m| m.min(rtt)));
        if self.period_start.elapsed() < PERIOD {
            return;
        }
        let wanted = (self.period_min.take().map_or(0, |m| m.as_millis() as u32).div_ceil(TARGET_FRAME_MS) + 2).clamp(MIN_WINDOW, MAX_WINDOW);
        self.period_start = Instant::now();
        // Up at once, down one step at a time.
        let next = if wanted > self.window { wanted } else { self.window.saturating_sub(1).max(wanted) };
        if next != self.window {
            info!(from = self.window, to = next, "frames in flight adjusted to the round trip");
            self.window = next;
        }
    }
}

/// The client's queue depth when it has switched acknowledgements off: it will never give a credit back.
const ACKS_SUSPENDED: u32 = 0xFFFF_FFFF;

// Credits are the flow control between whoever encodes (the session process) and whoever talks to the client
// (this side). The session may only encode and send while it holds one; each frame uses one and each
// acknowledgement gives one back, as a `proto::Down::Credit` message.
impl EgfxState {
    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }

    /// Sets where credits are delivered.
    pub fn set_credit_sink(&self, sink: Box<dyn Fn(u16) + Send + Sync>) {
        *self.credit_sink.lock().unwrap() = Some(sink);
    }

    /// A new connection, or one that restarted from zero credits (after a resize): hands out a whole window.
    pub fn reset_credits(&self) {
        let window = {
            let mut flow = self.flow.lock().unwrap();
            flow.issued = flow.window;
            flow.sent_at.clear();
            flow.window
        };
        self.grant(window as u16);
    }

    fn frame_sent(&self, frame_id: u32) {
        let mut flow = self.flow.lock().unwrap();
        if flow.sent_at.len() >= 64 {
            flow.sent_at.pop_front();
        }
        flow.sent_at.push_back((frame_id, Instant::now()));
    }

    /// A frame was acknowledged: measures its round trip and returns how many credits to hand out.
    fn frame_acked(&self, frame_id: u32) -> u16 {
        let mut flow = self.flow.lock().unwrap();
        while let Some((id, at)) = flow.sent_at.pop_front() {
            if id == frame_id {
                flow.sample(at.elapsed());
                break;
            }
        }
        flow.issued = flow.issued.saturating_sub(1);
        let give = flow.window.saturating_sub(flow.issued);
        flow.issued += give;
        give as u16
    }

    pub fn grant(&self, frames: u16) {
        if let Some(sink) = self.credit_sink.lock().unwrap().as_ref() {
            sink(frames);
        }
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

    fn max_frames_in_flight(&self) -> u32 {
        u32::from(max_credits())
    }

    fn on_ready(&mut self, negotiated: &CapabilitySet) {
        info!(?negotiated, frames_in_flight = fixed_window(), "EGFX ready");
        self.st.ready.store(true, Ordering::Release);
        self.st.reset_credits();
    }

    fn on_frame_ack(&mut self, frame_id: u32, queue_depth: u32, total_frames_decoded: u32) {
        let give = if queue_depth == ACKS_SUSPENDED { 1000 } else { self.st.frame_acked(frame_id) };
        self.st.grant(give);
        let n = self.st.acked.fetch_add(1, Ordering::Relaxed) + 1;
        if n <= 3 || n % 200 == 0 {
            info!(frame_id, queue_depth, total_frames_decoded, acked = n, sent = self.st.sent.load(Ordering::Relaxed), "client acknowledged frame");
        }
    }

    fn on_close(&mut self) {
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
        *self.st.flow.lock().unwrap() = Flow::new();
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
        match server.send_avc420_frame(surface, data, &[region], ts) {
            Some(frame_id) => self.st.frame_sent(frame_id),
            None => {
                // Refused frames are never acknowledged, so the credit would be lost for good.
                self.st.grant(1);
                warn!("send_avc420_frame refused a frame after encoding it");
            }
        }
        self.st.flush(&mut server)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn period(flow: &mut Flow, rtt_ms: u64) {
        flow.period_start = Instant::now() - PERIOD;
        flow.sample(Duration::from_millis(rtt_ms));
    }

    #[test]
    fn window_follows_round_trip() {
        let mut flow = Flow::new();
        flow.skip = 0;
        period(&mut flow, 100);
        assert_eq!(flow.window, 9); // 100 ms / 16 ms = 7, plus 2
        period(&mut flow, 1);
        assert_eq!(flow.window, 8); // down one step at a time
        for _ in 0..10 {
            period(&mut flow, 1);
        }
        assert_eq!(flow.window, MIN_WINDOW);
        period(&mut flow, 5000);
        assert_eq!(flow.window, MAX_WINDOW);
    }

    #[test]
    fn first_frames_are_ignored() {
        let mut flow = Flow::new();
        for _ in 0..FIRST_FRAMES {
            period(&mut flow, 5000);
        }
        assert_eq!(flow.window, MIN_WINDOW + 1);
    }
}
