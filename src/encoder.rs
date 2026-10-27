//! The H.264 encoder of a session: OpenH264 on the CPU (`encode.rs`), or the GPU's fixed-function encoder through VAAPI (`encode_va.rs`) when
//! the session renders on a GPU (docs/GPU-PLAN.md, stage 2). `WAYDESK_ENCODER=auto|vaapi|software` (`auto` is the default).

use anyhow::Result;

use crate::encode::{Tuning, H264};

pub trait Encode: Send {
    /// Encodes a tight BGRA frame of `w` x `h`. `None` when the encoder dropped the frame.
    fn encode(&mut self, bgra: &[u8], w: usize, h: usize) -> Result<Option<Vec<u8>>>;
    /// Changes the bitrate the encoder aims for, while it runs.
    fn set_bitrate(&mut self, bps: u32);
    /// The next frame is a keyframe.
    fn force_keyframe(&mut self);
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
}

impl Encode for Timed {
    fn encode(&mut self, bgra: &[u8], w: usize, h: usize) -> Result<Option<Vec<u8>>> {
        let start = std::time::Instant::now();
        let out = self.inner.encode(bgra, w, h)?;
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
    fn set_bitrate(&mut self, bps: u32) {
        tracing::debug!(encoder = self.name, bps, "bitrate");
        self.inner.set_bitrate(bps)
    }
    fn force_keyframe(&mut self) {
        self.inner.force_keyframe()
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

#[derive(Debug, PartialEq)]
enum Choice {
    Auto,
    Vaapi,
    Software,
}

fn choice(value: Option<&str>) -> Choice {
    match value.map(str::trim) {
        Some("vaapi") => Choice::Vaapi,
        Some("software") | Some("openh264") | Some("off") => Choice::Software,
        _ => Choice::Auto,
    }
}

/// The encoder for a desktop of `width` x `height`. With a GPU the session can use, VAAPI; if that does not start (no driver, no H.264
/// encoder on the chip), OpenH264 and a line in the log.
pub fn open(width: u16, height: u16, tuning: Tuning) -> Result<Box<dyn Encode>> {
    let choice = choice(std::env::var("WAYDESK_ENCODER").ok().as_deref());
    #[cfg(feature = "vaapi")]
    if choice != Choice::Software {
        if let Some(node) = crate::gpu::usable_node() {
            match crate::encode_va::VaH264::new(&node, width, height, tuning) {
                Ok(enc) => {
                    tracing::info!(node = %node.display(), "H.264 encoder: VAAPI");
                    return Ok(Box::new(Timed::new(Box::new(enc), "vaapi")));
                }
                Err(e) => tracing::warn!("VAAPI encoder not available ({e:#}): OpenH264 on the CPU"),
            }
        } else if choice == Choice::Vaapi {
            tracing::warn!("WAYDESK_ENCODER=vaapi, but the session has no usable GPU: OpenH264 on the CPU");
        }
    }
    #[cfg(not(feature = "vaapi"))]
    if choice == Choice::Vaapi {
        tracing::warn!("WAYDESK_ENCODER=vaapi, but this build has no VAAPI support: OpenH264 on the CPU");
    }
    Ok(Box::new(Timed::new(Box::new(H264::new(width, height, tuning)?), "openh264")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choice_from_text() {
        assert_eq!(choice(None), Choice::Auto);
        assert_eq!(choice(Some("auto")), Choice::Auto);
        assert_eq!(choice(Some("vaapi")), Choice::Vaapi);
        assert_eq!(choice(Some("software")), Choice::Software);
        assert_eq!(choice(Some("nonsense")), Choice::Auto);
    }
}
