//! H.264 encoding on the GPU's fixed-function encoder through VAAPI (cros-codecs' stateless encoder), for sessions that render on a GPU.
//! Same contract as `encode::H264`: a BGRA frame in, an Annex B access unit out (SPS and PPS inline at every keyframe), a bitrate that can
//! change while it runs. The colour conversion is still on the CPU, straight into the surface (docs/GPU-PLAN.md).
//!
//! The rate is controlled here, not by the chip: hardware CBR pads every frame up to the target (a static grey picture cost 30 KB a frame on
//! Intel), and it expects a steady frame rate while ours come only when something changes. The encoder runs at a constant quantiser instead,
//! and `QpControl` moves that quantiser between a floor (the picture quality wanted) and a ceiling (what a slow link may cost).

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
use cros_codecs::libva::{self, Display, Surface, UsageHint, VAEntrypoint, VAProfile, VA_FOURCC_NV12, VA_RT_FORMAT_YUV420};
use cros_codecs::video_frame::{ReadMapping, VideoFrame, WriteMapping};
use cros_codecs::{BlockingMode, Fourcc, FrameLayout, PlaneLayout, Resolution};
use yuv::{bgra_to_yuv_nv12, BufferStoreMut, YuvBiPlanarImageMut, YuvConversionMode, YuvRange, YuvStandardMatrix};

use crate::encode::{surface_size, Tuning};

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

type Encoder = StatelessEncoder<VaFrame, VaapiBackend<(), Surface<()>>>;

pub struct VaH264 {
    display: Rc<Display>,
    encoder: Encoder,
    layout: FrameLayout,
    size: Resolution,
    framerate: u32,
    qp: QpControl,
    current_qp: u32,
    image_format: libva::VAImageFormat,
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
        Ok(Self { display, encoder, layout, size, framerate: t.fps, qp: QpControl::new(floor, ceiling, t.bitrate), current_qp: floor, image_format, keyframe: true, timestamp: 0 })
    }

    /// A surface holding the picture: `bgra` (`w` x `h`) converted into its top left corner, black around it.
    fn picture(&self, bgra: &[u8], w: usize, h: usize) -> Result<Surface<()>> {
        let (sw, sh) = (self.size.width, self.size.height);
        let mut surfaces = self
            .display
            .create_surfaces(VA_RT_FORMAT_YUV420, Some(VA_FOURCC_NV12), sw, sh, Some(UsageHint::USAGE_HINT_ENCODER), vec![()])
            .map_err(|e| anyhow!("VA surface: {e}"))?;
        let surface = surfaces.pop().context("no VA surface")?;
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

impl crate::encoder::Encode for VaH264 {
    fn encode(&mut self, bgra: &[u8], w: usize, h: usize) -> Result<Option<Vec<u8>>> {
        let surface = self.picture(bgra, w, h)?;
        let frame = VaFrame { surface: Mutex::new(Some(surface)), size: self.size };
        let meta = FrameMetadata { timestamp: self.timestamp, layout: self.layout.clone(), force_keyframe: std::mem::take(&mut self.keyframe) };
        self.timestamp += 1;
        self.encoder.encode(meta, frame).map_err(|e| anyhow!("VAAPI encode: {e:?}"))?;
        let Some(coded) = self.encoder.poll().map_err(|e| anyhow!("VAAPI poll: {e:?}"))? else { return Ok(None) };
        let qp = self.qp.after_frame(coded.bitstream.len());
        if qp != self.current_qp {
            let tunings = Tunings { rate_control: RateControl::ConstantQuality(qp), framerate: self.framerate, min_quality: 1, max_quality: 51 };
            match self.encoder.tune(tunings) {
                Ok(()) => {
                    tracing::debug!(from = self.current_qp, to = qp, "quantiser");
                    self.current_qp = qp;
                }
                Err(e) => tracing::warn!(qp, "the encoder did not take the new quantiser: {e:?}"),
            }
        }
        Ok(Some(coded.bitstream))
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
