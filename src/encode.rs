//! H.264 encoding of captured frames (OpenH264, screen-content mode). Separate from the RDP graphics pipeline
//! (`egfx.rs`) so that it can run in the session process while the network daemon only forwards the result.

use anyhow::{Context as _, Result};
use openh264::encoder::{BitRate, Complexity, Encoder, EncoderConfig, FrameRate, FrameType, QpRange, RateControlMode, UsageType};
use openh264::formats::YUVSource;
use openh264::OpenH264API;
use yuv::{bgra_to_yuv420, BufferStoreMut, YuvConversionMode, YuvPlanarImageMut, YuvRange, YuvStandardMatrix};

#[derive(Clone, Copy)]
pub struct Tuning {
    pub bitrate: u32,
    /// Highest quantiser OpenH264 may use (lower = better picture, more bits).
    pub max_qp: u8,
    pub fps: u32,
}

/// Size of the encoder's picture and of the EGFX surface for a desktop of `width` x `height`: H.264 wants multiples of
/// 16, plus one more block because the destination rectangle of a frame is the region bounds plus one pixel, which
/// must still fit inside the surface.
pub fn surface_size(width: usize, height: usize) -> (usize, usize) {
    (((width + 15) & !15) + 16, ((height + 15) & !15) + 16)
}

/// A tuning switch of the encoder from the environment: `0` or `1` (anything else, or nothing: the default). For measuring (see
/// `waydesk-encbench`); the defaults are what was measured to be best.
fn env_flag(name: &str, default: bool) -> bool {
    match std::env::var(name).as_deref() {
        Ok("1") => true,
        Ok("0") => false,
        _ => default,
    }
}

pub struct H264 {
    encoder: Encoder,
    sw: usize,
    sh: usize,
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
    sps_pps: Option<Vec<u8>>,
}

impl H264 {
    pub fn new(width: u16, height: u16, t: Tuning) -> Result<Self> {
        let (sw, sh) = surface_size(usize::from(width), usize::from(height));
        let config = EncoderConfig::new()
            .bitrate(BitRate::from_bps(t.bitrate))
            .max_frame_rate(FrameRate::from_hz(t.fps as f32))
            // Screen content is the default: about half the bits of camera mode for moving pictures (measured), at about three times
            // the CPU. WAYDESK_H264_USAGE=camera trades that back for servers with little CPU (see /etc/default/waydesk).
            .usage_type(match std::env::var("WAYDESK_H264_USAGE").as_deref() {
                Ok("camera") => UsageType::CameraVideoRealTime,
                _ => UsageType::ScreenContentRealTime,
            })
            .complexity(match std::env::var("WAYDESK_H264_COMPLEXITY").as_deref() {
                Ok("low") => Complexity::Low,
                Ok("high") => Complexity::High,
                Ok("medium") => Complexity::Medium,
                // Low saves 15-36 % CPU under heavy motion at the same bits and PSNR (docs/MEASUREMENTS.md, encoder benchmark).
                _ => Complexity::Low,
            })
            .scene_change_detect(env_flag("WAYDESK_H264_SCENE_CHANGE", true))
            .background_detection(env_flag("WAYDESK_H264_BACKGROUND", true))
            .adaptive_quantization(env_flag("WAYDESK_H264_ADAPTIVE_QUANT", true))
            .long_term_reference(env_flag("WAYDESK_H264_LTR", false))
            .num_threads(std::env::var("WAYDESK_H264_THREADS").ok().and_then(|v| v.parse::<u16>().ok()).filter(|n| (1..=16).contains(n)).unwrap_or(4))
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
        })
    }

    /// Changes the bitrate the encoder aims for, while it runs (OpenH264 takes it for the next frames; it skips frames when it is over).
    pub fn set_bitrate(&mut self, bps: u32) {
        use openh264_sys2::{SBitrateInfo, ENCODER_OPTION_BITRATE, SPATIAL_LAYER_ALL};
        let mut info = SBitrateInfo { iLayer: SPATIAL_LAYER_ALL, iBitrate: bps.min(i32::MAX as u32) as i32 };
        // SAFETY: the option takes a pointer to an SBitrateInfo, which lives for the call.
        let result = unsafe { self.encoder.raw_api().set_option(ENCODER_OPTION_BITRATE, (&raw mut info).cast()) };
        if result != 0 {
            tracing::warn!(bps, result, "the encoder did not take the new bitrate");
        }
    }

    /// The next frame will be a keyframe: it carries the whole picture again, not only the difference to the last one.
    pub fn force_keyframe(&mut self) {
        self.encoder.force_intra_frame();
    }

    /// Encode a tight BGRA frame of `w` x `h` (the surface is `sw` x `sh`, the extra rows stay black).
    /// Returns `None` when the encoder decided to skip the frame.
    pub fn encode(&mut self, bgra: &[u8], w: usize, h: usize) -> Result<Option<Vec<u8>>> {
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

/// The encoder library of the system (Debian: `libopenh264-8`).
///
/// The crate's own `from_blob_path` only accepts Cisco's binaries (it compares a SHA-256 list), which would reject every
/// distribution build. The bindings were generated for OpenH264 2.6, so a system library is accepted when the file it
/// resolves to is a 2.6 or 2.7 release; anything else is refused instead of risking an ABI mismatch. 2.7.0 is accepted because
/// its public headers (codec_api.h, codec_app_def.h, codec_def.h) are identical to 2.6.0's and the soname is the same (.8);
/// Arch ships 2.7. Check the headers again before adding a later release.
/// `WAYDESK_OPENH264` names a library to use instead (for example a copy downloaded from Cisco); that one keeps the
/// crate's hash check.
pub fn load_openh264() -> anyhow::Result<OpenH264API> {
    if let Ok(p) = std::env::var("WAYDESK_OPENH264") {
        return OpenH264API::from_blob_path(&p).map_err(|e| anyhow::anyhow!("WAYDESK_OPENH264={p}: {e}"));
    }
    let mut problems = Vec::new();
    for dir in ["/usr/lib/aarch64-linux-gnu", "/usr/lib/x86_64-linux-gnu", "/usr/lib64", "/usr/lib"] {
        let link = std::path::Path::new(dir).join("libopenh264.so.8");
        let Ok(real) = std::fs::canonicalize(&link) else { continue };
        let name = real.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_owned();
        if !name.starts_with("libopenh264.so.2.6.") && !name.starts_with("libopenh264.so.2.7.") {
            problems.push(format!("{}: is {name}, need an OpenH264 2.6 or 2.7 release", link.display()));
            continue;
        }
        // SAFETY: the file is a 2.6 or 2.7 release, whose API is the one the bindings were generated for, as checked above.
        match unsafe { OpenH264API::from_blob_path_unchecked(&real) } {
            Ok(api) => return Ok(api),
            Err(e) => problems.push(format!("{}: {e}", real.display())),
        }
    }
    anyhow::bail!("no usable libopenh264 (install the libopenh264-8 package, version 2.6 or 2.7): {}", problems.join("; "))
}

