//! H.264 encoding on the GPU's fixed-function encoder through VAAPI (cros-codecs' stateless encoder), for sessions that render on a GPU.
//! Same contract as `encode::H264`: a BGRA frame in, an Annex B access unit out (SPS and PPS inline at every keyframe), a bitrate that can
//! change while it runs. The colour conversion is still on the CPU, straight into the surface (docs/GPU-PLAN.md).
//!
//! The rate is controlled here, not by the chip: hardware CBR pads every frame up to the target (a static grey picture cost 30 KB a frame on
//! Intel), and it expects a steady frame rate while ours come only when something changes. The encoder runs at a constant quantiser instead,
//! and `QpControl` moves that quantiser between a floor (the picture quality wanted) and a ceiling (what a slow link may cost).

use std::collections::HashMap;
use std::os::fd::{AsRawFd, OwnedFd};
use std::path::Path;
use std::rc::Rc;
use std::sync::Mutex;
use std::time::Instant;

use anyhow::{anyhow, bail, Context as _, Result};
use cros_codecs::backend::vaapi::encoder::VaapiBackend;
use cros_codecs::codec::h264::parser::{Level, Profile};
use cros_codecs::encoder::h264::EncoderConfig;
use cros_codecs::encoder::stateless::h264::StatelessEncoder;
use cros_codecs::encoder::{FrameMetadata, RateControl, Tunings, VideoEncoder};
use cros_codecs::libva::{
    self, Display, ExternalBufferDescriptor, MemoryType, Surface, UsageHint, VADRMPRIMESurfaceDescriptor, VAEntrypoint, VAProfile, VA_FOURCC_BGRX, VA_FOURCC_NV12,
    VA_RT_FORMAT_RGB32, VA_RT_FORMAT_YUV420,
};
use cros_codecs::video_frame::{ReadMapping, VideoFrame, WriteMapping};
use cros_codecs::{BlockingMode, Fourcc, FrameLayout, PlaneLayout, Resolution};
use yuv::{bgra_to_yuv_nv12, BufferStoreMut, YuvBiPlanarImageMut, YuvConversionMode, YuvRange, YuvStandardMatrix};

use crate::capture::Pixels;
use crate::dmabuf::GpuFrame;
use crate::encode::{surface_size, Tuning};
use crate::vpp::Vpp;

/// A frame the encoder takes: a VA surface already filled with the picture, handed over once.
struct VaFrame {
    surface: Mutex<Option<Surface<()>>>,
    size: Resolution,
}

impl std::fmt::Debug for VaFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "VaFrame({}x{})", self.size.width, self.size.height)
    }
}

// SAFETY: a frame is made, given to the encoder and consumed on the one thread that owns the `VaH264` it belongs to; it never meets another.
unsafe impl Send for VaFrame {}
unsafe impl Sync for VaFrame {}

impl VideoFrame for VaFrame {
    type MemDescriptor = ();
    type NativeHandle = Surface<()>;

    fn fourcc(&self) -> Fourcc {
        Fourcc::from(b"NV12")
    }
    fn resolution(&self) -> Resolution {
        self.size
    }
    fn get_plane_size(&self) -> Vec<usize> {
        let (w, h) = (self.size.width as usize, self.size.height as usize);
        vec![w * h, w * h / 2]
    }
    fn get_plane_pitch(&self) -> Vec<usize> {
        vec![self.size.width as usize; 2]
    }
    fn map<'a>(&'a self) -> Result<Box<dyn ReadMapping<'a> + 'a>, String> {
        Err("a VA surface frame is not mapped by the encoder".into())
    }
    fn map_mut<'a>(&'a mut self) -> Result<Box<dyn WriteMapping<'a> + 'a>, String> {
        Err("a VA surface frame is not mapped by the encoder".into())
    }
    fn to_native_handle(&self, _display: &Rc<Display>) -> Result<Surface<()>, String> {
        self.surface.lock().map_err(|e| e.to_string())?.take().ok_or_else(|| "the surface was already handed over".to_string())
    }
}

/// Picks the quantiser from how many bits the last frames took against the bitrate the link allows. A bucket fills with the bits of each
/// frame and drains at the target rate (it cannot hold credit: a quiet second does not pay for a busy one).
///
/// The quantiser moves in tiers, from `floor` (the quality wanted, where a desktop stays almost always) to `ceiling` (what a slow link may
/// cost), and rarely: with this encoder a new quantiser starts a new sequence, which is a keyframe. One tier up after a second with the bucket
/// full, one down after five with it empty, and never twice within two seconds.
struct QpControl {
    floor: u32,
    ceiling: u32,
    target_bps: f64,
    bucket: f64,
    tier: u32,
    last: Instant,
    changed: Instant,
    full_since: Option<Instant>,
    empty_since: Option<Instant>,
}

const TIERS: u32 = 3;
const FULL_AFTER: std::time::Duration = std::time::Duration::from_secs(1);
const EMPTY_AFTER: std::time::Duration = std::time::Duration::from_secs(5);
const HOLD: std::time::Duration = std::time::Duration::from_secs(2);

impl QpControl {
    fn new(floor: u32, ceiling: u32, target_bps: u32) -> Self {
        let now = Instant::now();
        Self {
            floor,
            ceiling: ceiling.max(floor),
            target_bps: f64::from(target_bps.max(1)),
            bucket: 0.0,
            tier: 0,
            last: now,
            changed: now - HOLD,
            full_since: None,
            empty_since: None,
        }
    }

    /// The quantiser for the next frame, after one of `bytes` bytes.
    fn after_frame(&mut self, bytes: usize) -> u32 {
        self.after_frame_at(Instant::now(), bytes)
    }

    fn after_frame_at(&mut self, now: Instant, bytes: usize) -> u32 {
        let dt = now.saturating_duration_since(self.last).as_secs_f64();
        self.last = now;
        self.bucket = (self.bucket + bytes as f64 * 8.0 - self.target_bps * dt).max(0.0);
        // Full: half a second of target bits or more behind. Empty: less than a twentieth of a second.
        let (full, empty) = (self.bucket >= self.target_bps * 0.5, self.bucket <= self.target_bps * 0.05);
        self.full_since = if full { self.full_since.or(Some(now)) } else { None };
        self.empty_since = if empty { self.empty_since.or(Some(now)) } else { None };
        if now.saturating_duration_since(self.changed) >= HOLD {
            if self.full_since.is_some_and(|t| now.saturating_duration_since(t) >= FULL_AFTER) && self.tier < TIERS {
                self.tier += 1;
                self.changed = now;
                self.full_since = None;
            } else if self.empty_since.is_some_and(|t| now.saturating_duration_since(t) >= EMPTY_AFTER) && self.tier > 0 {
                self.tier -= 1;
                self.changed = now;
                self.empty_since = None;
            }
        }
        self.qp()
    }

    fn qp(&self) -> u32 {
        self.floor + self.tier * (self.ceiling - self.floor) / TIERS
    }
}

/// A captured dmabuf (linear XRGB8888) as a VA surface the video processor can read.
struct DmabufXrgb {
    fd: OwnedFd,
    width: u32,
    height: u32,
    stride: u32,
}

impl ExternalBufferDescriptor for DmabufXrgb {
    const MEMORY_TYPE: MemoryType = MemoryType::DrmPrime2;
    type DescriptorAttribute = VADRMPRIMESurfaceDescriptor;

    fn va_surface_attribute(&mut self) -> VADRMPRIMESurfaceDescriptor {
        let mut d = VADRMPRIMESurfaceDescriptor { width: self.width, height: self.height, fourcc: VA_FOURCC_BGRX, num_objects: 1, num_layers: 1, ..Default::default() };
        d.objects[0].fd = self.fd.as_raw_fd();
        d.objects[0].size = self.stride * self.height;
        d.objects[0].drm_format_modifier = 0; // linear
        d.layers[0].drm_format = crate::dmabuf::XRGB8888;
        d.layers[0].num_planes = 1;
        d.layers[0].pitch[0] = self.stride;
        d
    }
}

/// How many imported buffers the encoder keeps: the capture rotates through a few, so this is more than enough.
const MAX_IMPORTS: usize = 16;

type Encoder = StatelessEncoder<VaFrame, VaapiBackend<(), Surface<()>>>;

pub struct VaH264 {
    display: Rc<Display>,
    encoder: Encoder,
    layout: FrameLayout,
    size: Resolution,
    framerate: u32,
    qp: QpControl,
    current_qp: u32,
    /// What makes the encoder again for a new quantiser (see `submit`).
    image_format: libva::VAImageFormat,
    /// Video processing (RGB to NV12 on the GPU) and the captured buffers imported for it, by buffer; created with the first GPU frame.
    vpp: Option<Vpp>,
    imports: HashMap<u64, Surface<DmabufXrgb>>,
    gpu_failed: bool,
    keyframe: bool,
    timestamp: u64,
}

// SAFETY: every `Rc` the VA objects share (the display, held by the encoder, its backend, the surfaces) lives inside this struct, so moving the
// whole struct to another thread moves all of them together; nothing else holds one, and it is used by one thread at a time.
unsafe impl Send for VaH264 {}

impl VaH264 {
    pub fn new(node: &Path, width: u16, height: u16, t: Tuning) -> Result<Self> {
        let (sw, sh) = surface_size(usize::from(width), usize::from(height));
        let size = Resolution { width: sw as u32, height: sh as u32 };
        let display = Display::open_drm_display(node).map_err(|e| anyhow!("VA display on {}: {e}", node.display()))?;
        let entrypoints = display.query_config_entrypoints(VAProfile::VAProfileH264High).map_err(|e| anyhow!("VA entrypoints: {e}"))?;
        // The fixed-function encoder (low power) is the cheaper one where the chip has it working; on Skylake to Coffee Lake it needs the GuC
        // firmware (i915.enable_guc=2) and fails without it, so a failure there is not the end: the shader-assisted encoder comes next.
        let mut modes = Vec::new();
        if entrypoints.contains(&VAEntrypoint::VAEntrypointEncSliceLP) {
            modes.push(true);
        }
        if entrypoints.contains(&VAEntrypoint::VAEntrypointEncSlice) {
            modes.push(false);
        }
        if modes.is_empty() {
            bail!("the GPU has no H.264 encoder (High profile)");
        }
        let image_format = display
            .query_image_formats()
            .map_err(|e| anyhow!("VA image formats: {e}"))?
            .into_iter()
            .find(|f| f.fourcc == VA_FOURCC_NV12)
            .context("the GPU has no NV12 image format")?;
        // Floor: the quality asked for (`--quality` is the highest quantiser the CPU encoder uses; two steps below it is as good as it looks).
        // Ceiling: what a slow link may cost. `WAYDESK_VA_MIN_QP` and `WAYDESK_VA_MAX_QP` change them (see /etc/default/waydesk).
        let env_qp = |name: &str| std::env::var(name).ok().and_then(|v| v.parse::<u32>().ok()).filter(|q| (1..=51).contains(q));
        let floor = env_qp("WAYDESK_VA_MIN_QP").unwrap_or_else(|| u32::from(t.max_qp.clamp(1, 51)).saturating_sub(2).max(1));
        let ceiling = env_qp("WAYDESK_VA_MAX_QP").unwrap_or_else(|| floor.max(40));
        let tunings = Tunings { rate_control: RateControl::ConstantQuality(floor), framerate: t.fps, min_quality: 1, max_quality: 51 };
        let config = EncoderConfig { resolution: size, profile: Profile::High, level: Level::L5_1, initial_tunings: tunings.clone(), ..Default::default() };
        let fourcc = Fourcc::from(b"NV12");
        let mut encoder = None;
        let mut last_error = String::new();
        for low_power in modes {
            match Encoder::new_vaapi(Rc::clone(&display), config.clone(), fourcc, size, low_power, BlockingMode::Blocking) {
                Ok(e) => {
                    tracing::info!(low_power, "VAAPI H.264 encoder opened");
                    encoder = Some(e);
                    break;
                }
                Err(e) => last_error = format!("{} encoder: {e:?}", if low_power { "low power" } else { "full" }),
            }
        }
        let encoder = encoder.ok_or_else(|| anyhow!("VAAPI H.264 encoder: {last_error}"))?;
        let layout = FrameLayout {
            format: (fourcc, 0),
            size,
            planes: vec![
                PlaneLayout { buffer_index: 0, offset: 0, stride: sw },
                PlaneLayout { buffer_index: 0, offset: sw * sh, stride: sw },
            ],
        };
        Ok(Self { display, encoder, layout, size, framerate: t.fps, qp: QpControl::new(floor, ceiling, t.bitrate), current_qp: floor, image_format, vpp: None, imports: HashMap::new(), gpu_failed: false, keyframe: true, timestamp: 0 })
    }

    /// An empty NV12 surface of the encoder's size.
    fn new_surface(&self) -> Result<Surface<()>> {
        let (sw, sh) = (self.size.width, self.size.height);
        let mut surfaces = self
            .display
            .create_surfaces(VA_RT_FORMAT_YUV420, Some(VA_FOURCC_NV12), sw, sh, Some(UsageHint::USAGE_HINT_ENCODER), vec![()])
            .map_err(|e| anyhow!("VA surface: {e}"))?;
        surfaces.pop().context("no VA surface")
    }

    /// A surface holding the picture in GPU memory: the captured buffer imported (once per buffer) and converted by the video processor.
    fn picture_from_gpu(&mut self, g: &GpuFrame, flip: bool, w: usize, h: usize) -> Result<Surface<()>> {
        if !self.imports.contains_key(&g.id) {
            if self.imports.len() >= MAX_IMPORTS {
                self.imports.clear();
            }
            let fd = g.dmabuf().try_clone().context("dup of the dmabuf")?;
            let desc = DmabufXrgb { fd, width: g.width, height: g.height, stride: g.stride };
            let surface = self
                .display
                .create_surfaces(VA_RT_FORMAT_RGB32, Some(VA_FOURCC_BGRX), g.width, g.height, Some(UsageHint::USAGE_HINT_ENCODER), vec![desc])
                .map_err(|e| anyhow!("VA import of the dmabuf: {e}"))?
                .pop()
                .context("no VA surface")?;
            self.imports.insert(g.id, surface);
        }
        if self.vpp.is_none() {
            self.vpp = Some(Vpp::new(&self.display, self.size.width, self.size.height)?);
        }
        let out = self.new_surface()?;
        self.vpp.as_ref().expect("vpp").convert(&self.imports[&g.id], &out, w as u32, h as u32, flip)?;
        Ok(out)
    }

    /// A surface holding the picture: `bgra` (`w` x `h`) converted into its top left corner, black around it.
    fn picture(&self, bgra: &[u8], w: usize, h: usize) -> Result<Surface<()>> {
        let (sw, sh) = (self.size.width, self.size.height);
        let surface = self.new_surface()?;
        {
            let mut image = libva::Image::create_from(&surface, self.image_format, (sw, sh), (sw, sh)).map_err(|e| anyhow!("VA image: {e}"))?;
            let va = *image.image();
            let (y_off, uv_off) = (va.offsets[0] as usize, va.offsets[1] as usize);
            let (y_pitch, uv_pitch) = (va.pitches[0] as usize, va.pitches[1] as usize);
            let data = image.as_mut();
            let (head, uv_all) = data.split_at_mut(uv_off);
            let y_all = &mut head[y_off..];
            // The surface is bigger than the picture (the encoder wants multiples of 16, plus a block): black around the picture.
            for row in y_all.chunks_mut(y_pitch).take(sh as usize) {
                row[w..sw as usize].fill(0);
            }
            y_all[h * y_pitch..sh as usize * y_pitch].fill(0);
            for row in uv_all.chunks_mut(uv_pitch).take(sh as usize / 2) {
                row[w..sw as usize].fill(128);
            }
            uv_all[(h / 2) * uv_pitch..(sh as usize / 2) * uv_pitch].fill(128);
            let mut nv12 = YuvBiPlanarImageMut {
                y_plane: BufferStoreMut::Borrowed(y_all),
                y_stride: y_pitch as u32,
                uv_plane: BufferStoreMut::Borrowed(uv_all),
                uv_stride: uv_pitch as u32,
                width: w as u32,
                height: h as u32,
            };
            bgra_to_yuv_nv12(&mut nv12, bgra, (w * 4) as u32, YuvRange::Full, YuvStandardMatrix::Bt709, YuvConversionMode::Balanced).context("BGRA to NV12")?;
        }
        Ok(surface)
    }
}

impl VaH264 {
    /// Encodes the picture in `surface` and moves the quantiser if the link asks for it.
    fn submit(&mut self, surface: Surface<()>) -> Result<Option<Vec<u8>>> {
        let frame = VaFrame { surface: Mutex::new(Some(surface)), size: self.size };
        let meta = FrameMetadata { timestamp: self.timestamp, layout: self.layout.clone(), force_keyframe: std::mem::take(&mut self.keyframe) };
        self.timestamp += 1;
        self.encoder.encode(meta, frame).map_err(|e| anyhow!("VAAPI encode: {e:?}"))?;
        let Some(coded) = self.encoder.poll().map_err(|e| anyhow!("VAAPI poll: {e:?}"))? else { return Ok(None) };
        let qp = self.qp.after_frame(coded.bitstream.len());
        if qp != self.current_qp {
            // The quantiser travels in the slice headers (patches/cros-codecs.diff), so this sends no new SPS or PPS and starts no new sequence: a
            // client that rebuilt its decoder on every new parameter set lost the keyframe and showed a washed-out picture (Windows App).
            let tunings = Tunings { rate_control: RateControl::ConstantQuality(qp), framerate: self.framerate, min_quality: 1, max_quality: 51 };
            match self.encoder.tune(tunings) {
                Ok(()) => {
                    tracing::debug!(from = self.current_qp, to = qp, "quantiser");
                    self.current_qp = qp;
                }
                Err(e) => tracing::warn!(qp, "could not change the quantiser: {e:?}"),
            }
        }
        Ok(Some(declare_no_reordering(coded.bitstream)))
    }
}

/// cros-codecs writes an SPS without `bitstream_restriction` (the last bit of its VUI is 0). A decoder then has to assume the worst for the
/// level: reordering and a DPB of up to 16 frames, with POC type 0. We never reorder and keep one reference, and OpenH264's stream says so
/// (0 and 1). So the SPS at the front of a keyframe gets the restriction appended; any other input comes back unchanged.
fn declare_no_reordering(bs: Vec<u8>) -> Vec<u8> {
    let Some(p) = bs.iter().take(8).collect::<Vec<_>>().windows(3).position(|w| *w[0] == 0 && *w[1] == 0 && *w[2] == 1) else { return bs };
    if bs.get(p + 3).map(|b| b & 0x1f) != Some(7) {
        return bs;
    }
    let body = p + 4;
    let Some(mut end) = bs[body..].windows(3).position(|w| w == [0, 0, 1]).map(|i| body + i) else { return bs };
    while end > body && bs[end - 1] == 0 {
        end -= 1;
    }
    let Some(sps) = restrict_sps(&bs[body..end]) else { return bs };
    let mut out = Vec::with_capacity(bs.len() + 16);
    out.extend_from_slice(&bs[..body]);
    out.extend_from_slice(&sps);
    out.extend_from_slice(&bs[end..]);
    out
}

/// The SPS payload (after the NAL header byte, with emulation prevention) with `bitstream_restriction_flag` 0 replaced by the flag 1 and
/// its fields: vectors over the picture edge, no byte/bit limits, vector length 2^16, 0 frames reordered, 1 frame buffered.
fn restrict_sps(escaped: &[u8]) -> Option<Vec<u8>> {
    let mut raw = Vec::with_capacity(escaped.len());
    let mut zeros = 0;
    for &b in escaped {
        if zeros >= 2 && b == 3 {
            zeros = 0;
            continue;
        }
        zeros = if b == 0 { zeros + 1 } else { 0 };
        raw.push(b);
    }
    let last = *raw.last()?;
    if last == 0 {
        return None;
    }
    let total = raw.len() * 8 - last.trailing_zeros() as usize - 1;
    let bit = |i: usize| raw[i / 8] >> (7 - i % 8) & 1;
    if bit(total - 1) != 0 {
        return None;
    }
    let mut bits: Vec<u8> = (0..total - 1).map(bit).collect();
    let ue = |bits: &mut Vec<u8>, v: u32| {
        let n = 32 - (v + 1).leading_zeros() as usize;
        bits.extend(std::iter::repeat(0).take(n - 1));
        bits.extend((0..n).rev().map(|i| ((v + 1) >> i & 1) as u8));
    };
    bits.extend([1, 1]);
    for v in [0, 0, 16, 16, 0, 1] {
        ue(&mut bits, v);
    }
    bits.push(1);
    while bits.len() % 8 != 0 {
        bits.push(0);
    }
    let mut out = Vec::with_capacity(bits.len() / 8 + 2);
    let mut zeros = 0;
    for byte in bits.chunks(8).map(|c| c.iter().fold(0u8, |a, b| a << 1 | b)) {
        if zeros >= 2 && byte <= 3 {
            out.push(3);
            zeros = 0;
        }
        zeros = if byte == 0 { zeros + 1 } else { 0 };
        out.push(byte);
    }
    Some(out)
}

impl crate::encoder::Encode for VaH264 {
    fn encode(&mut self, bgra: &[u8], w: usize, h: usize) -> Result<Option<Vec<u8>>> {
        let surface = self.picture(bgra, w, h)?;
        self.submit(surface)
    }

    fn takes_gpu_frames(&self) -> bool {
        !self.gpu_failed
    }

    fn encode_frame(&mut self, frame: &Pixels, w: usize, h: usize) -> Result<Option<Vec<u8>>> {
        match frame {
            Pixels::Cpu(bgra) => self.encode(bgra, w, h),
            Pixels::Gpu(g, flip) => {
                if !self.gpu_failed {
                    match self.picture_from_gpu(g, *flip, w, h) {
                        Ok(surface) => return self.submit(surface),
                        Err(e) => {
                            // Not every driver imports a dmabuf or converts it the way the Intel one does: from now on the picture comes through
                            // shm and the CPU conversion, as without the GPU capture.
                            tracing::warn!("VAAPI cannot take the captured buffer ({e:#}): reading pictures through shm");
                            self.gpu_failed = true;
                            self.imports.clear();
                            crate::capture::PREFER_SHM.store(true, std::sync::atomic::Ordering::Relaxed);
                        }
                    }
                }
                let bgra = g.read_bgra(*flip)?;
                self.encode(&bgra, w, h)
            }
        }
    }

    fn set_bitrate(&mut self, bps: u32) {
        self.qp.target_bps = f64::from(bps.max(1));
    }

    fn force_keyframe(&mut self) {
        self.keyframe = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn quantiser_steps_up_under_sustained_overload_and_back_down() {
        let t0 = Instant::now();
        let mut c = QpControl::new(18, 42, 8_000_000);
        assert_eq!(c.qp(), 18);
        // 100 KB every 20 ms is 40 Mbit/s against 8: the bucket is full within a second, and a second later the quantiser steps up.
        let mut now = t0;
        let mut seen = vec![];
        for _ in 0..300 {
            now += Duration::from_millis(20);
            seen.push(c.after_frame_at(now, 100_000));
        }
        assert_eq!(seen[0], 18, "one big frame is not a reason");
        assert!(seen.contains(&26), "first step: {seen:?}");
        assert_eq!(*seen.last().unwrap(), 42, "six seconds of overload reach the ceiling");
        // Quiet: tiny frames, the bucket drains, and after five seconds of it the quantiser steps down, one tier at a time.
        let mut down = vec![];
        for _ in 0..800 {
            now += Duration::from_millis(50);
            down.push(c.after_frame_at(now, 200));
        }
        assert_eq!(*down.last().unwrap(), 18, "back at the floor after a long quiet time");
        let changes = down.windows(2).filter(|w| w[0] != w[1]).count();
        assert!(changes <= 3, "one tier at a time: {changes} changes");
    }

    #[test]
    fn a_keyframe_burst_does_not_move_the_quantiser() {
        let t0 = Instant::now();
        let mut c = QpControl::new(18, 42, 8_000_000);
        let mut now = t0;
        c.after_frame_at(now, 600_000);
        for _ in 0..100 {
            now += Duration::from_millis(50);
            assert_eq!(c.after_frame_at(now, 5_000), 18);
        }
    }

    #[test]
    fn ceiling_is_never_under_floor() {
        let c = QpControl::new(30, 20, 1_000_000);
        assert_eq!(c.qp(), 30);
    }
}
