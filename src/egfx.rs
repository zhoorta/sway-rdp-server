//! Graphics pipeline (EGFX) output: the captured frames go to the client as H.264 (AVC420).
//! The Windows App on macOS is built around this path and mishandles the older bitmap updates.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use ironrdp_egfx::pdu::{Avc420Region, CapabilitySet};
use ironrdp_egfx::server::{GraphicsPipelineHandler, GraphicsPipelineServer};
use ironrdp_server::tokio::sync::mpsc::UnboundedSender;
use ironrdp_server::{EgfxServerMessage, GfxDvcBridge, GfxServerFactory, GfxServerHandle, ServerEvent, ServerEventSender};
use tracing::{debug, info};

use crate::capture::Rect;
use crate::tcpinfo::Link;
use crate::encode::surface_size;


/// State shared between the DVC callbacks (which run with the server locked) and the display loop.
#[derive(Default)]
pub struct EgfxState {
    ready: AtomicBool,
    /// Where credits go: to the session process, which only encodes while it holds one.
    credit_sink: Mutex<Option<Box<dyn Fn(u16) + Send + Sync>>>,
    /// Where a new bitrate for the encoder goes (to the session process), and the highest one there may be (the configured one).
    bitrate_sink: Mutex<Option<Box<dyn Fn(u32) + Send + Sync>>>,
    max_bitrate: AtomicU32,
    /// The connection's socket (-1: none), to ask the kernel what the link delivers.
    socket: AtomicI32, // fd + 1, 0: none
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

/// The lowest bitrate the encoder is asked for, however slow the link: below it the picture is not worth sending.
const MIN_BITRATE: u32 = 300_000;
/// How long the frames waiting for the link may take to get there, at the delivery rate measured: the byte budget in flight.
const QUEUE_SECONDS: f64 = 0.4;
/// Share of the delivery rate the encoder aims for.
const BITRATE_SHARE: f64 = 0.8;
/// The link's capacity is the best delivery rate of this long: a single sample dips whenever the kernel recovers a lost packet.
const RATE_WINDOW: Duration = Duration::from_secs(10);
/// More than this in the socket's send queue and no new frame is let go: the path is slower than what we make, and the queue is delay.
const QUEUE_HOLD: u64 = 192_000;
/// More than this queued and the path is the limit (until it is under half of it again): the kernel's delivery rate is not trusted then, the
/// rate at which the peer acknowledges is.
const QUEUE_FULL: u64 = 96_000;
/// How long the acknowledged bytes are counted over for that rate.
const DRAIN_WINDOW: Duration = Duration::from_millis(1500);
/// A delivery rate measured while the link was full is trusted this long.
const CAPACITY_AGE: Duration = Duration::from_secs(30);
/// A frame tells the delay of the path only when it is this small: a big one takes seconds to send on a slow link, and that is not the path.
const DELAY_FRAME_MAX: u32 = 4096;

/// A frame in flight.
struct Sent {
    id: u32,
    at: Instant,
    size: u32,
    /// Bytes that were in flight when it was sent: nothing means it went into an empty pipe.
    queue_ahead: u64,
}

/// The flow control of one connection: how many frames and how many bytes may be on their way to the client.
///
/// `issued` counts the credits out there (held by the session or attached to a frame in flight); an acknowledgement ends a frame and
/// tops `issued` up to `window`, so a larger window hands out extra credits and a smaller one simply gives fewer back. The window follows the
/// delay of the path (see `sample`). On a link that cannot carry what the encoder makes that is not enough, because the frames then wait in
/// the queue for seconds: while acknowledgements arrive with frames queued behind them the delivery rate is measured, and then no credit is
/// given while more than `QUEUE_SECONDS` of it is in flight, and the encoder is asked for a bitrate that fits (`bitrate_for`).
struct Flow {
    window: u32,
    issued: u32,
    sent: VecDeque<Sent>,
    inflight_bytes: u64,
    period_start: Instant,
    period_min: Option<Duration>,
    /// The delay of the path itself: the lowest round trip of a frame that went into an empty pipe (it creeps up slowly, so that a longer
    /// route is followed in the end).
    base: Option<Duration>,
    /// Acknowledgements still to ignore: the first frame is the whole picture and takes far longer than any later one.
    skip: u32,
    /// What the link delivered when it was full, in bytes per second, and when that was measured.
    capacity: Option<(f64, Instant)>,
    /// The delivery rates the capacity is the highest of (within `RATE_WINDOW`).
    rates: VecDeque<(Instant, f64)>,
    /// Bytes in the socket's send queue at the last acknowledgement.
    queued: u64,
    /// The queue is deep: the path is the limit. The window the acknowledged bytes are counted over: when it began and how many there were.
    link_full: bool,
    drain: Option<(Instant, u64)>,
    last_bitrate: u32,
    last_bitrate_at: Instant,
}

impl Default for Flow {
    fn default() -> Self {
        Self::new()
    }
}

impl Flow {
    fn new() -> Self {
        let window = fixed_window().map_or(MIN_WINDOW + 1, u32::from);
        Self {
            window,
            issued: 0,
            sent: VecDeque::new(),
            inflight_bytes: 0,
            period_start: Instant::now(),
            period_min: None,
            base: None,
            skip: FIRST_FRAMES,
            capacity: None,
            rates: VecDeque::new(),
            queued: 0,
            link_full: false,
            drain: None,
            last_bitrate: 0,
            last_bitrate_at: Instant::now(),
        }
    }

    fn reset(&mut self) {
        self.issued = self.window;
        self.sent.clear();
        self.inflight_bytes = 0;
        self.last_bitrate = 0;
    }

    fn frame_sent(&mut self, id: u32, size: usize) {
        if self.sent.len() >= 128 {
            if let Some(old) = self.sent.pop_front() {
                self.inflight_bytes = self.inflight_bytes.saturating_sub(u64::from(old.size));
            }
        }
        let size = size.min(u32::MAX as usize) as u32;
        self.sent.push_back(Sent { id, at: Instant::now(), size, queue_ahead: self.inflight_bytes });
        self.inflight_bytes += u64::from(size);
    }

    /// A frame was acknowledged: what the flow control learns from it, how many credits to hand out, and a new bitrate for the encoder if
    /// it should change.
    fn frame_acked(&mut self, id: u32, max_bitrate: u32, link: Option<Link>) -> (u16, Option<u32>) {
        let now = Instant::now();
        // The frames before it that were never acknowledged are gone.
        let mut found = None;
        while let Some(f) = self.sent.pop_front() {
            self.inflight_bytes = self.inflight_bytes.saturating_sub(u64::from(f.size));
            if f.id == id {
                found = Some(f);
                break;
            }
        }
        if let Some(f) = found {
            // Only a frame that went into an empty pipe tells the delay of the path: the others waited behind ours.
            if f.queue_ahead == 0 && f.size <= DELAY_FRAME_MAX {
                self.sample(now.duration_since(f.at));
            }
        }
        self.note_link(now, link);
        self.issued = self.issued.saturating_sub(1);
        let mut give = self.window.saturating_sub(self.issued);
        // Too much on its way for the link: wait for acknowledgements (the next frame then carries the picture as it is by then). The last
        // one in flight is never held back, or nothing would come back to say it is time to go on.
        if !self.sent.is_empty() && (self.queued > QUEUE_HOLD || self.budget().is_some_and(|budget| self.inflight_bytes > budget)) {
            give = 0;
        }
        self.issued += give;
        (give as u16, self.bitrate_for(now, max_bitrate))
    }

    /// What the kernel says the connection has been delivering. While the sender had always more to send (not application limited) that is what
    /// the link carries; when the picture was the limit it says nothing about the link, and the last measurement is kept (it expires).
    ///
    /// A deep send queue says the path is slower than the first hop of the connection (an SSH tunnel: the loopback to sshd, which takes data far
    /// faster than the real link): then the kernel's rate is dropped and the rate at which the acknowledged bytes grow is the capacity.
    fn note_link(&mut self, now: Instant, link: Option<Link>) {
        let Some(link) = link else { return };
        self.queued = link.queued;
        let was_full = self.link_full;
        if link.queued > QUEUE_FULL {
            self.link_full = true;
        } else if link.queued < QUEUE_FULL / 2 {
            self.link_full = false;
        }
        if self.link_full {
            if !was_full {
                self.rates.clear();
                self.drain = Some((now, link.bytes_acked));
            } else if let Some((since, acked)) = self.drain {
                let span = now.duration_since(since);
                if span >= DRAIN_WINDOW {
                    let rate = link.bytes_acked.saturating_sub(acked) as f64 / span.as_secs_f64();
                    self.drain = Some((now, link.bytes_acked));
                    self.add_rate(now, rate);
                }
            }
            return;
        }
        self.drain = None;
        if !link.app_limited {
            self.add_rate(now, link.delivery_rate);
        }
    }

    /// A rate that the path carried: the capacity is the best of those of the last `RATE_WINDOW`.
    fn add_rate(&mut self, now: Instant, rate: f64) {
        if rate > 0.0 {
            self.rates.push_back((now, rate));
            while self.rates.front().is_some_and(|(at, _)| now.duration_since(*at) > RATE_WINDOW) {
                self.rates.pop_front();
            }
            let best = self.rates.iter().map(|(_, r)| *r).fold(0.0, f64::max);
            self.capacity = Some((best, now));
        }
    }

    /// The most bytes there may be in flight, None while the link has not been seen full (then only the window limits).
    fn budget(&self) -> Option<u64> {
        let (rate, at) = self.capacity?;
        (at.elapsed() < CAPACITY_AGE).then(|| (rate * QUEUE_SECONDS).clamp(30_000.0, 2_000_000.0) as u64)
    }

    /// The bitrate for the encoder when it differs enough from the last one: what the link delivers when it is full, or the configured
    /// one again, in steps, when it has not been full for a while.
    fn bitrate_for(&mut self, now: Instant, max_bitrate: u32) -> Option<u32> {
        let max = max_bitrate.max(MIN_BITRATE);
        let target = match self.capacity {
            Some((rate, at)) if at.elapsed() < CAPACITY_AGE => ((rate * 8.0 * BITRATE_SHARE) as u32).clamp(MIN_BITRATE, max),
            // not seen full for a while: back up towards the configured bitrate in steps, the link may be slow still
            _ if self.last_bitrate == 0 => max,
            _ => (self.last_bitrate / 2 * 3).clamp(MIN_BITRATE, max),
        };
        let last = if self.last_bitrate == 0 { max } else { self.last_bitrate };
        if target.abs_diff(last) * 5 > last && now.duration_since(self.last_bitrate_at) >= Duration::from_millis(1500) {
            self.last_bitrate = target;
            self.last_bitrate_at = now;
            return Some(target);
        }
        None
    }

    /// Notes a round trip of a frame that went into an empty pipe; at the end of a period moves the window towards what the delay of the
    /// path needs (the lowest of the period, and the lowest ever for the path itself).
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
        let Some(lowest) = self.period_min.take() else { return };
        self.period_start = Instant::now();
        let base = self.base.map_or(lowest, |b| lowest.min(b + b / 50));
        self.base = Some(base);
        let wanted = ((base.as_millis() as u32).div_ceil(TARGET_FRAME_MS) + 2).clamp(MIN_WINDOW, MAX_WINDOW);
        let next = if wanted > self.window {
            wanted // up at once
        } else {
            self.window.saturating_sub(1).max(wanted) // down one step at a time
        };
        if next != self.window {
            info!(from = self.window, to = next, base_ms = base.as_millis() as u64, "frames in flight adjusted to the round trip");
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
        let (window, unmeasured) = {
            let mut flow = self.flow.lock().unwrap();
            flow.reset();
            (flow.window, flow.capacity.is_none())
        };
        self.grant(window as u16);
        // A session that outlives its clients may still have the low bitrate of the last one: the link of this one has not been measured.
        let max = self.max_bitrate.load(Ordering::Relaxed);
        if unmeasured && max > 0 {
            self.send_bitrate(max);
        }
    }

    fn frame_sent(&self, frame_id: u32, size: usize) {
        self.flow.lock().unwrap().frame_sent(frame_id, size);
    }

    /// Bytes in the socket's send queue at the last acknowledgement.
    fn queued(&self) -> u64 {
        self.flow.lock().unwrap().queued
    }

    /// A frame was acknowledged: the credits to hand out, and a new bitrate for the encoder when the link needs one.
    fn frame_acked(&self, frame_id: u32) -> (u16, Option<u32>) {
        let max = self.max_bitrate.load(Ordering::Relaxed);
        let link = match self.socket.load(Ordering::Relaxed) {
            0 => None,
            fd => crate::tcpinfo::read(fd - 1),
        };
        self.flow.lock().unwrap().frame_acked(frame_id, max, link)
    }

    /// The socket of the connection, for the kernel's numbers; `clear_socket` before it is closed.
    pub fn set_socket(&self, fd: std::os::fd::RawFd) {
        self.socket.store(fd + 1, Ordering::Relaxed);
    }

    pub fn clear_socket(&self) {
        self.socket.store(0, Ordering::Relaxed);
    }

    /// Sets where the new bitrate goes, and the highest bitrate (the configured one).
    pub fn set_bitrate_sink(&self, max_bitrate: u32, sink: Box<dyn Fn(u32) + Send + Sync>) {
        self.max_bitrate.store(max_bitrate, Ordering::Relaxed);
        *self.bitrate_sink.lock().unwrap() = Some(sink);
    }

    fn send_bitrate(&self, bps: u32) {
        if let Some(sink) = self.bitrate_sink.lock().unwrap().as_ref() {
            sink(bps);
        }
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
        let (give, bitrate) = if queue_depth == ACKS_SUSPENDED { (1000, None) } else { self.st.frame_acked(frame_id) };
        self.st.grant(give);
        if let Some(bps) = bitrate {
            info!(bitrate = bps, queued_kb = self.st.queued() / 1000, "the encoder's bitrate follows what the link delivers");
            self.st.send_bitrate(bps);
        }
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
            Some(frame_id) => self.st.frame_sent(frame_id, data.len()),
            None => {
                // Refused frames are never acknowledged, so the credit would be lost for good.
                self.st.grant(1);
                debug!("send_avc420_frame refused a frame after encoding it (no credit: the link is the limit)");
            }
        }
        self.st.flush(&mut server)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the kernel says about a connection with an empty send queue.
    fn link(delivery_rate: f64, app_limited: bool) -> Link {
        Link { delivery_rate, app_limited, queued: 0, bytes_acked: 0 }
    }

    /// And one with a queue, `acked` bytes acknowledged so far.
    fn queued_link(queued: u64, acked: u64) -> Link {
        Link { delivery_rate: 5_000_000.0, app_limited: false, queued, bytes_acked: acked }
    }

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
    }

    #[test]
    fn a_full_link_limits_the_bytes_in_flight_and_the_bitrate() {
        let mut flow = Flow::new();
        flow.skip = 100;
        flow.last_bitrate_at = Instant::now() - Duration::from_secs(5);
        // the kernel says the link delivers 200 kB/s and the sender was not the limit
        for _ in 0..4 {
            flow.note_link(Instant::now(), Some(link(200_000.0, false)));
        }
        let (rate, _) = flow.capacity.expect("the delivery rate was taken");
        assert!((190_000.0..210_000.0).contains(&rate), "{rate}");
        assert_eq!(flow.budget(), Some((rate * QUEUE_SECONDS) as u64)); // 0.4 s of it: 80 kB
        let bitrate = flow.bitrate_for(Instant::now(), 10_000_000).expect("a new bitrate");
        assert!((1_200_000..1_400_000).contains(&bitrate), "{bitrate}"); // 0.8 of 1.6 Mbit/s
    }

    #[test]
    fn a_deep_send_queue_holds_back_the_credits_but_not_for_the_last_frame() {
        let mut flow = Flow::new();
        flow.skip = 100;
        flow.window = 8;
        flow.issued = 8;
        for i in 0..4 {
            flow.frame_sent(i, 1_000);
        }
        // the kernel says: 3 MB queued, as with an SSH tunnel in front of a slow link
        let (give, _) = flow.frame_acked(0, 10_000_000, Some(queued_link(3_000_000, 0)));
        assert_eq!(give, 0);
        let (give, _) = flow.frame_acked(1, 10_000_000, Some(queued_link(3_000_000, 0)));
        assert_eq!(give, 0);
        flow.frame_acked(2, 10_000_000, Some(queued_link(3_000_000, 0)));
        let (give, _) = flow.frame_acked(3, 10_000_000, Some(queued_link(3_000_000, 0))); // the last one in flight
        assert!(give >= 1);
        // the queue is gone: credits flow again
        flow.frame_sent(4, 1_000);
        flow.frame_sent(5, 1_000);
        let (give, _) = flow.frame_acked(4, 10_000_000, Some(link(1_000_000.0, false)));
        assert!(give >= 1);
    }

    #[test]
    fn with_a_deep_queue_the_capacity_is_what_the_peer_acknowledges_not_the_kernels_rate() {
        let mut flow = Flow::new();
        let t0 = Instant::now();
        // before the queue: the first hop (loopback to sshd) looks like 5 MB/s
        flow.note_link(t0, Some(link(5_000_000.0, false)));
        assert_eq!(flow.capacity.map(|(r, _)| r), Some(5_000_000.0));
        // the queue builds up: that rate is dropped, and 600 kB acknowledged in two seconds make 300 kB/s
        flow.note_link(t0 + Duration::from_millis(100), Some(queued_link(300_000, 1_000_000)));
        assert_eq!(flow.capacity.map(|(r, _)| r), Some(5_000_000.0)); // nothing new yet (the window has not run its length), the old one stays
        flow.note_link(t0 + Duration::from_millis(2_100), Some(queued_link(300_000, 1_600_000)));
        let (rate, _) = flow.capacity.expect("a capacity from the drain");
        assert!((290_000.0..310_000.0).contains(&rate), "{rate}");
        assert!(flow.link_full);
        // and the queue is empty again
        flow.note_link(t0 + Duration::from_millis(2_200), Some(link(200_000.0, true)));
        assert!(!flow.link_full);
    }

    #[test]
    fn a_dip_in_the_delivery_rate_does_not_lower_the_capacity() {
        let mut flow = Flow::new();
        for rate in [400_000.0, 150_000.0, 250_000.0] {
            flow.note_link(Instant::now(), Some(link(rate, false)));
        }
        assert_eq!(flow.capacity.map(|(r, _)| r), Some(400_000.0));
    }

    #[test]
    fn a_rate_held_down_by_the_picture_says_nothing_about_the_link() {
        let mut flow = Flow::new();
        flow.note_link(Instant::now(), Some(link(50_000.0, true)));
        flow.note_link(Instant::now(), None);
        assert!(flow.capacity.is_none());
        assert_eq!(flow.budget(), None);
        flow.last_bitrate_at = Instant::now() - Duration::from_secs(5);
        assert_eq!(flow.bitrate_for(Instant::now(), 10_000_000), None); // stays at the configured bitrate
    }

    #[test]
    fn the_bitrate_comes_back_in_steps_when_the_link_is_not_full_any_more() {
        let mut flow = Flow::new();
        flow.last_bitrate = 400_000;
        flow.last_bitrate_at = Instant::now() - Duration::from_secs(5);
        assert_eq!(flow.bitrate_for(Instant::now(), 10_000_000), Some(600_000)); // 1.5 times
    }

    #[test]
    fn credits_are_held_back_over_the_budget_but_not_for_the_last_frame() {
        let mut flow = Flow::new();
        flow.skip = 100;
        flow.window = 8;
        flow.issued = 8;
        flow.capacity = Some((100_000.0, Instant::now())); // budget 40 kB
        for i in 0..6 {
            flow.frame_sent(i, 20_000); // 120 kB in flight
        }
        let (give, _) = flow.frame_acked(0, 10_000_000, None);
        assert_eq!(give, 0); // 100 kB still in flight, over the budget
        for i in 1..5 {
            flow.frame_acked(i, 10_000_000, None);
        }
        let (give, _) = flow.frame_acked(5, 10_000_000, None); // the last one
        assert!(give >= 1);
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
