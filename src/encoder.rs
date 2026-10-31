//! The H.264 encoder of a session: OpenH264 on the CPU (`encode.rs`), or the GPU's fixed-function encoder: VAAPI (`encode_va.rs`) when the
//! session renders on a GPU (docs/GPU-PLAN.md, stage 2), NVENC on NVIDIA (`encode_nv.rs`). `WAYDESK_ENCODER=auto|vaapi|nvenc|software` (`auto` is the default).

use anyhow::Result;

use crate::capture::Pixels;
use crate::encode::{Tuning, H264};

pub trait Encode: Send {
    /// Encodes a tight BGRA frame of `w` x `h`. `None` when the encoder dropped the frame.
    fn encode(&mut self, bgra: &[u8], w: usize, h: usize) -> Result<Option<Vec<u8>>>;
    /// Encodes a captured picture, which may be in GPU memory. An encoder that cannot take it from there gets it read out.
    fn encode_frame(&mut self, frame: &Pixels, w: usize, h: usize) -> Result<Option<Vec<u8>>> {
        let bgra = frame.bgra()?;
        self.encode(&bgra, w, h)
    }
    /// True for an encoder that takes pictures in GPU memory as they are (the capture then renders into dmabufs and the CPU never sees them).
    fn takes_gpu_frames(&self) -> bool {
        false
    }
    /// Changes the bitrate the encoder aims for, while it runs.
    fn set_bitrate(&mut self, bps: u32);
    /// The next frame is a keyframe.
    fn force_keyframe(&mut self);
    /// The quantiser a constant-quantiser GPU encoder runs at now (what `waydesk-admin status` shows).
    fn quantiser(&self) -> Option<u32> {
        None
    }
}

impl Encode for H264 {
    fn encode(&mut self, bgra: &[u8], w: usize, h: usize) -> Result<Option<Vec<u8>>> {
        H264::encode(self, bgra, w, h)
    }
    fn set_bitrate(&mut self, bps: u32) {
        H264::set_bitrate(self, bps)
    }
    fn force_keyframe(&mut self) {
        H264::force_keyframe(self)
    }
}

/// Wraps an encoder and says in the log (`WAYDESK_LOG=debug`), every 5 s, how it does: frames made, time per frame and size per frame.
struct Timed {
    inner: Box<dyn Encode>,
    name: &'static str,
    since: std::time::Instant,
    frames: u32,
    dropped: u32,
    busy: std::time::Duration,
    bytes: usize,
}

impl Timed {
    fn new(inner: Box<dyn Encode>, name: &'static str) -> Self {
        Self { inner, name, since: std::time::Instant::now(), frames: 0, dropped: 0, busy: Default::default(), bytes: 0 }
    }

    /// Counts a frame that took since `start`, and reports every 5 s.
    fn account(&mut self, start: std::time::Instant, out: Option<Vec<u8>>) -> Result<Option<Vec<u8>>> {
        self.busy += start.elapsed();
        match &out {
            Some(d) => {
                self.frames += 1;
                self.bytes += d.len();
            }
            None => self.dropped += 1,
        }
        let elapsed = self.since.elapsed();
        if elapsed >= std::time::Duration::from_secs(5) {
            // For `waydesk-admin status`, also without a debug log: a screen with nothing on it makes no frames and no statistics.
            if self.frames > 0 {
                let secs = elapsed.as_secs_f64();
                crate::sessionstatus::set_stats(f64::from(self.frames) / secs, (self.bytes as f64 * 8.0 / secs / 1000.0) as u32, self.inner.quantiser());
            }
            if self.frames + self.dropped > 0 {
                tracing::debug!(
                    encoder = self.name,
                    fps = format!("{:.1}", f64::from(self.frames) / elapsed.as_secs_f64()),
                    dropped = self.dropped,
                    ms_per_frame = format!("{:.1}", self.busy.as_secs_f64() * 1000.0 / f64::from((self.frames + self.dropped).max(1))),
                    kb_per_frame = format!("{:.1}", self.bytes as f64 / 1024.0 / f64::from(self.frames.max(1))),
                    "encoder"
                );
            }
            *self = Self::new(std::mem::replace(&mut self.inner, Box::new(Gone)), self.name);
        }
        Ok(out)
    }
}

impl Encode for Timed {
    fn encode(&mut self, bgra: &[u8], w: usize, h: usize) -> Result<Option<Vec<u8>>> {
        let start = std::time::Instant::now();
        let out = self.inner.encode(bgra, w, h)?;
        self.account(start, out)
    }
    fn encode_frame(&mut self, frame: &Pixels, w: usize, h: usize) -> Result<Option<Vec<u8>>> {
        let start = std::time::Instant::now();
        let out = self.inner.encode_frame(frame, w, h)?;
        self.account(start, out)
    }
    fn takes_gpu_frames(&self) -> bool {
        self.inner.takes_gpu_frames()
    }
    fn set_bitrate(&mut self, bps: u32) {
        tracing::debug!(encoder = self.name, bps, "bitrate");
        self.inner.set_bitrate(bps)
    }
    fn force_keyframe(&mut self) {
        self.inner.force_keyframe()
    }
    fn quantiser(&self) -> Option<u32> {
        self.inner.quantiser()
    }
}

/// Stands in for the encoder for the moment `Timed` is rebuilt.
struct Gone;
impl Encode for Gone {
    fn encode(&mut self, _: &[u8], _: usize, _: usize) -> Result<Option<Vec<u8>>> {
        anyhow::bail!("encoder gone")
    }
    fn set_bitrate(&mut self, _: u32) {}
    fn force_keyframe(&mut self) {}
}

/// For diagnosing the picture: with `WAYDESK_DUMP_DIR` set, every frame is written there as it was given to the encoder (`src-NNNNN.bgra`) next to the
/// bitstream it became (`stream.h264`, one line per frame in `frames.txt`), for comparing what was decoded with what was captured. Off by default: it writes
/// the screen to disk, and a few hundred frames of it.
struct Dump {
    inner: Box<dyn Encode>,
    dir: std::path::PathBuf,
    n: u32,
    stream: std::fs::File,
    index: std::fs::File,
}

impl Dump {
    fn new(inner: Box<dyn Encode>, dir: std::path::PathBuf) -> Option<Self> {
        std::fs::create_dir_all(&dir).ok()?;
        let stream = std::fs::File::create(dir.join("stream.h264")).ok()?;
        let index = std::fs::File::create(dir.join("frames.txt")).ok()?;
        Some(Self { inner, dir, n: 0, stream, index })
    }
}

impl Encode for Dump {
    fn encode(&mut self, bgra: &[u8], w: usize, h: usize) -> Result<Option<Vec<u8>>> {
        use std::io::Write;
        let out = self.inner.encode(bgra, w, h)?;
        if let (Some(data), true) = (&out, self.n < 900) {
            let _ = std::fs::write(self.dir.join(format!("src-{:05}.bgra", self.n)), bgra);
            let _ = self.stream.write_all(data);
            let _ = writeln!(self.index, "{} {} {w} {h}", self.n, data.len());
            self.n += 1;
        }
        Ok(out)
    }
    fn encode_frame(&mut self, frame: &Pixels, w: usize, h: usize) -> Result<Option<Vec<u8>>> {
        let bgra = frame.bgra()?;
        // The encoder gets the picture as it is (zero-copy included) so that what is tested is what is used; the copy is only for the dump.
        use std::io::Write;
        let out = self.inner.encode_frame(frame, w, h)?;
        if let (Some(data), true) = (&out, self.n < 900) {
            let _ = std::fs::write(self.dir.join(format!("src-{:05}.bgra", self.n)), &*bgra);
            let _ = self.stream.write_all(data);
            let _ = writeln!(self.index, "{} {} {w} {h}", self.n, data.len());
            self.n += 1;
        }
        Ok(out)
    }
    fn takes_gpu_frames(&self) -> bool {
        self.inner.takes_gpu_frames()
    }
    fn set_bitrate(&mut self, bps: u32) {
        self.inner.set_bitrate(bps)
    }
    fn force_keyframe(&mut self) {
        self.inner.force_keyframe()
    }
}

/// With the dump on, the rectangle of the frame just encoded is added to `rects.txt` (what the client paints of it).
pub fn dump_rect(x: u32, y: u32, w: u32, h: u32) {
    use std::io::Write;
    if let Some(dir) = std::env::var_os("WAYDESK_DUMP_DIR") {
        if let Ok(mut f) = std::fs::OpenOptions::new().append(true).create(true).open(std::path::Path::new(&dir).join("rects.txt")) {
            let _ = writeln!(f, "{x} {y} {w} {h}");
        }
    }
}

/// Wraps `enc` in the dump when `WAYDESK_DUMP_DIR` asks for it.
fn maybe_dump(enc: Box<dyn Encode>) -> Box<dyn Encode> {
    match std::env::var_os("WAYDESK_DUMP_DIR") {
        Some(dir) => {
            tracing::warn!(dir = ?dir, "WAYDESK_DUMP_DIR: the encoder writes the screen to disk");
            // `Dump::new` needs the encoder by value and gives it back on failure through the Option: rebuild on error.
            let dir = std::path::PathBuf::from(dir);
            let takes = enc.takes_gpu_frames();
            match Dump::new(enc, dir) {
                Some(d) => Box::new(d),
                None => {
                    tracing::warn!("cannot write the dump directory");
                    let _ = takes;
                    Box::new(Gone)
                }
            }
        }
        None => enc,
    }
}

#[derive(Debug, PartialEq)]
enum Choice {
    Auto,
    Vaapi,
    Nvenc,
    Software,
}

fn choice(value: Option<&str>) -> Choice {
    match value.map(str::trim) {
        Some("vaapi") => Choice::Vaapi,
        Some("nvenc") => Choice::Nvenc,
        Some("software") | Some("openh264") | Some("off") => Choice::Software,
        _ => Choice::Auto,
    }
}

/// Whether the VAAPI encoder is tried: with `auto` and `vaapi`. `nvenc` and `software` skip it (before this, `nvenc` still started VAAPI first when the card had it).
fn tries_vaapi(choice: &Choice) -> bool {
    matches!(choice, Choice::Auto | Choice::Vaapi)
}

/// The encoder for a desktop of `width` x `height`. With a GPU the session can use, VAAPI; if that does not start (no driver, no H.264
/// encoder on the chip), OpenH264 and a line in the log.
pub fn open(width: u16, height: u16, tuning: Tuning) -> Result<Box<dyn Encode>> {
    let choice = choice(std::env::var("WAYDESK_ENCODER").ok().as_deref());
    #[cfg(feature = "vaapi")]
    if tries_vaapi(&choice) {
        if let Some(node) = crate::gpu::usable_node() {
            match crate::encode_va::VaH264::new(&node, width, height, tuning) {
                Ok(enc) => {
                    tracing::info!(node = %node.display(), "H.264 encoder: VAAPI");
                    crate::sessionstatus::set_encoder("vaapi");
                    return Ok(maybe_dump(Box::new(Timed::new(Box::new(enc), "vaapi"))));
                }
                Err(e) => tracing::warn!("VAAPI encoder not available ({e:#}): OpenH264 on the CPU"),
            }
        } else if choice == Choice::Vaapi {
            tracing::warn!("WAYDESK_ENCODER=vaapi, but the session has no usable GPU: OpenH264 on the CPU");
        }
    }
    // NVIDIA has no VAAPI encoder: its own, NVENC, takes the BGRA picture as it is. Machines without NVIDIA's library skip this without a word.
    #[cfg(feature = "nvenc")]
    if matches!(choice, Choice::Auto | Choice::Nvenc) {
        if nvidia_video_codec_sdk::nvenc_available() {
            match crate::encode_nv::NvH264::new(crate::gpu::usable_node().as_deref(), width, height, tuning) {
                Ok(enc) => {
                    tracing::info!("H.264 encoder: NVENC");
                    crate::sessionstatus::set_encoder("nvenc");
                    return Ok(maybe_dump(Box::new(Timed::new(Box::new(enc), "nvenc"))));
                }
                Err(e) => tracing::warn!("NVENC encoder not available ({e:#}): OpenH264 on the CPU"),
            }
        } else if choice == Choice::Nvenc {
            tracing::warn!("WAYDESK_ENCODER=nvenc, but libnvidia-encode is not installed: OpenH264 on the CPU");
        }
    }
    #[cfg(not(feature = "vaapi"))]
    if choice == Choice::Vaapi {
        tracing::warn!("WAYDESK_ENCODER=vaapi, but this build has no VAAPI support: OpenH264 on the CPU");
    }
    let enc = H264::new(width, height, tuning)?;
    crate::sessionstatus::set_encoder("openh264");
    Ok(maybe_dump(Box::new(Timed::new(Box::new(enc), "openh264"))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nvenc_and_software_do_not_start_with_vaapi() {
        assert!(tries_vaapi(&Choice::Auto) && tries_vaapi(&Choice::Vaapi));
        assert!(!tries_vaapi(&Choice::Nvenc) && !tries_vaapi(&Choice::Software));
    }

    #[test]
    fn choice_from_text() {
        assert_eq!(choice(None), Choice::Auto);
        assert_eq!(choice(Some("auto")), Choice::Auto);
        assert_eq!(choice(Some("vaapi")), Choice::Vaapi);
        assert_eq!(choice(Some("nvenc")), Choice::Nvenc);
        assert_eq!(choice(Some("software")), Choice::Software);
        assert_eq!(choice(Some("nonsense")), Choice::Auto);
    }
}
