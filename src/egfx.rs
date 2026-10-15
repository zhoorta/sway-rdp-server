//! Graphics pipeline (EGFX) output: the captured frames go to the client as H.264 (AVC420).
//! The Windows App on macOS is built around this path and mishandles the older bitmap updates.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, Result};
use ironrdp_egfx::pdu::{Avc420Region, CapabilitySet};
use ironrdp_egfx::server::{GraphicsPipelineHandler, GraphicsPipelineServer};
use ironrdp_server::tokio::sync::mpsc::UnboundedSender;
use ironrdp_server::{EgfxServerMessage, GfxDvcBridge, GfxServerFactory, GfxServerHandle, ServerEvent, ServerEventSender};
use openh264::encoder::{BitRate, Complexity, Encoder, EncoderConfig, FrameRate, FrameType, QpRange, RateControlMode, UsageType};
use openh264::formats::YUVSource;
use openh264::OpenH264API;
use tracing::{debug, info, warn};
use yuv::{bgra_to_yuv420, BufferStoreMut, YuvConversionMode, YuvPlanarImageMut, YuvRange, YuvStandardMatrix};

use crate::capture::Rect;

#[derive(Clone, Copy)]
pub struct Tuning {
    pub bitrate: u32,
    /// Highest quantiser OpenH264 may use (lower = better picture, more bits).
    pub max_qp: u8,
    pub fps: u32,
}

/// State shared between the DVC callbacks (which run with the server locked) and the display loop.
#[derive(Default)]
pub struct EgfxState {
    ready: AtomicBool,
    sent: AtomicU64,
    acked: AtomicU64,
    handle: Mutex<Option<GfxServerHandle>>,
    sender: Mutex<Option<UnboundedSender<ServerEvent>>>,
}

impl EgfxState {
    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
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
        self.st.ready.store(true, Ordering::Release);
    }

    fn on_frame_ack(&mut self, frame_id: u32, queue_depth: u32, total_frames_decoded: u32) {
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
        // The desktop size is only known per session; `Out::send` sends ResetGraphics with it.
        let server = GraphicsPipelineServer::new(Box::new(Handler { st: self.st.clone() }));
        let handle: GfxServerHandle = Arc::new(Mutex::new(server));
        *self.st.handle.lock().unwrap() = Some(handle.clone());
        Some((GfxDvcBridge::new(handle.clone()), handle))
    }
}

/// Per-connection sender: owns the encoder and the surface.
pub struct Out {
    st: Arc<EgfxState>,
    width: usize,
    height: usize,
    /// Surface size: H.264 wants multiples of 16.
    sw: usize,
    sh: usize,
    surface: Option<u16>,
    enc: H264,
    start: std::time::Instant,
}

pub enum Sent {
    Done,
    /// Too many frames in flight (nothing was consumed): try again shortly.
    Backpressure,
    /// The encoder produced nothing for this frame: the area must be tried again.
    Skipped,
    /// The client cannot do AVC420: use the bitmap path instead.
    Unsupported,
}

impl Out {
    pub fn new(st: Arc<EgfxState>, width: u16, height: u16, tuning: Tuning) -> Result<Self> {
        let (width, height) = (usize::from(width), usize::from(height));
        // H.264 wants multiples of 16. Plus one more block: the destination rectangle of a frame is the
        // region bounds plus one pixel, which must still fit inside the surface.
        let (sw, sh) = (((width + 15) & !15) + 16, ((height + 15) & !15) + 16);
        Ok(Self { st, width, height, sw, sh, surface: None, enc: H264::new(sw, sh, tuning)?, start: std::time::Instant::now() })
    }

    pub fn needs_surface(&self) -> bool {
        self.surface.is_none()
    }

    pub fn send(&mut self, frame: &[u8], dirty: Rect) -> Result<Sent> {
        let handle = self.st.handle().context("no EGFX server handle")?;
        let mut server = handle.lock().unwrap();
        if !server.supports_avc420() {
            return Ok(Sent::Unsupported);
        }
        if server.should_backpressure() {
            return Ok(Sent::Backpressure);
        }
        let surface = match self.surface {
            Some(id) => id,
            None => {
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
                id
            }
        };

        let Some(data) = self.enc.encode(frame, self.width, self.height)? else {
            self.st.flush(&mut server)?;
            return Ok(Sent::Skipped);
        };
        // The crate documents these bounds as inclusive, but they go on the wire unchanged and the wire format
        // is exclusive on the right and bottom: inclusive values left a 1 px line stale along the edges.
        let region = Avc420Region {
            left: dirty.x as u16,
            top: dirty.y as u16,
            right: (dirty.x + dirty.w) as u16,
            bottom: (dirty.y + dirty.h) as u16,
            quantization_parameter: self.enc.max_qp,
            quality: 100,
        };
        let ts = self.start.elapsed().as_millis() as u32;
        let n = self.st.sent.fetch_add(1, Ordering::Relaxed) + 1;
        if n <= 3 {
            info!(n, bytes = data.len(), ?dirty, left = region.left, top = region.top, right = region.right, bottom = region.bottom, "sending first frames");
        }
        if server.send_avc420_frame(surface, &data, &[region], ts).is_none() {
            warn!("send_avc420_frame refused a frame after encoding it");
        }
        self.st.flush(&mut server)?;
        Ok(Sent::Done)
    }

    /// The surface has to be (re)sent in full the first time: caller marks everything dirty.
    pub fn force_keyframe(&mut self) {
        self.enc.encoder.force_intra_frame();
    }
}

struct H264 {
    encoder: Encoder,
    sw: usize,
    sh: usize,
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
    sps_pps: Option<Vec<u8>>,
    max_qp: u8,
}

impl H264 {
    fn new(sw: usize, sh: usize, t: Tuning) -> Result<Self> {
        let config = EncoderConfig::new()
            .bitrate(BitRate::from_bps(t.bitrate))
            .max_frame_rate(FrameRate::from_hz(t.fps as f32))
            .usage_type(UsageType::ScreenContentRealTime)
            .complexity(Complexity::Medium)
            .scene_change_detect(true)
            .num_threads(4)
            .rate_control_mode(RateControlMode::Bitrate)
            .qp(QpRange::new(0, t.max_qp.clamp(1, 51)))
            // Skipping a frame would lose its changed area (only the changed rectangle is sent each time).
            .skip_frames(false);
        let encoder = Encoder::with_api_config(load_openh264()?, config).context("OpenH264 encoder")?;
        Ok(Self {
            encoder,
            sw,
            sh,
            y: vec![0u8; sw * sh],
            u: vec![128u8; (sw / 2) * (sh / 2)],
            v: vec![128u8; (sw / 2) * (sh / 2)],
            sps_pps: None,
            max_qp: t.max_qp.clamp(1, 51),
        })
    }

    /// Encode a tight BGRA frame of `w` x `h` (the surface is `sw` x `sh`, the extra rows stay black).
    /// Returns `None` when the encoder decided to skip the frame.
    fn encode(&mut self, bgra: &[u8], w: usize, h: usize) -> Result<Option<Vec<u8>>> {
        let (y_len, uv_len) = (self.sw * h, (self.sw / 2) * (h / 2));
        let mut img = YuvPlanarImageMut {
            y_plane: BufferStoreMut::Borrowed(&mut self.y[..y_len]),
            y_stride: self.sw as u32,
            u_plane: BufferStoreMut::Borrowed(&mut self.u[..uv_len]),
            u_stride: (self.sw / 2) as u32,
            v_plane: BufferStoreMut::Borrowed(&mut self.v[..uv_len]),
            v_stride: (self.sw / 2) as u32,
            width: w as u32,
            height: h as u32,
        };
        bgra_to_yuv420(&mut img, bgra, (w * 4) as u32, YuvRange::Full, YuvStandardMatrix::Bt709, YuvConversionMode::Balanced)
            .context("BGRA to YUV420")?;

        let src = Planes { y: &self.y, u: &self.u, v: &self.v, w: self.sw, h: self.sh };
        let bits = self.encoder.encode(&src).context("OpenH264 encode")?;
        let key = matches!(bits.frame_type(), FrameType::IDR | FrameType::I);
        if matches!(bits.frame_type(), FrameType::Skip | FrameType::Invalid) {
            return Ok(None);
        }
        let data = bits.to_vec();
        if key {
            if let Some(ps) = extract_sps_pps(&data) {
                self.sps_pps = Some(ps);
            }
        }
        Ok(Some(data))
    }
}

struct Planes<'a> {
    y: &'a [u8],
    u: &'a [u8],
    v: &'a [u8],
    w: usize,
    h: usize,
}

impl YUVSource for Planes<'_> {
    fn dimensions(&self) -> (usize, usize) {
        (self.w, self.h)
    }
    fn strides(&self) -> (usize, usize, usize) {
        (self.w, self.w / 2, self.w / 2)
    }
    fn y(&self) -> &[u8] {
        self.y
    }
    fn u(&self) -> &[u8] {
        self.u
    }
    fn v(&self) -> &[u8] {
        self.v
    }
}

/// The SPS and PPS NAL units (with start codes) of an Annex B stream, if both are present.
fn extract_sps_pps(data: &[u8]) -> Option<Vec<u8>> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        if data[i..i + 3] == [0, 0, 1] {
            starts.push(i);
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut out = Vec::new();
    let (mut sps, mut pps) = (false, false);
    for (n, &s) in starts.iter().enumerate() {
        let end = starts.get(n + 1).copied().unwrap_or(data.len());
        let nal_type = data.get(s + 3).map(|b| b & 0x1f);
        if matches!(nal_type, Some(7) | Some(8)) {
            // Keep a leading zero if the start code was four bytes long.
            let from = if s > 0 && data[s - 1] == 0 { s - 1 } else { s };
            let to = if end < data.len() && end > 0 && data[end - 1] == 0 { end - 1 } else { end };
            out.extend_from_slice(&data[from..to]);
            sps |= nal_type == Some(7);
            pps |= nal_type == Some(8);
        }
    }
    (sps && pps).then_some(out)
}

/// The encoder library of the system (Debian: `libopenh264-8`), checked for the exact version the crate was built for.
/// `SWAY_RDP_OPENH264` overrides the path (for example a copy downloaded from Cisco).
fn load_openh264() -> anyhow::Result<OpenH264API> {
    let paths: Vec<String> = std::env::var("SWAY_RDP_OPENH264").into_iter().chain(
        ["/usr/lib/aarch64-linux-gnu/libopenh264.so.8", "/usr/lib/x86_64-linux-gnu/libopenh264.so.8", "libopenh264.so.8"].map(String::from),
    ).collect();
    let mut last = None;
    for p in &paths {
        match OpenH264API::from_blob_path(p) {
            Ok(api) => return Ok(api),
            Err(e) => last = Some(format!("{p}: {e}")),
        }
    }
    anyhow::bail!("no usable libopenh264 (install the libopenh264-8 package): {}", last.unwrap_or_default())
}
