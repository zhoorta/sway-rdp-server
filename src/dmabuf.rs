//! GPU buffers for screencopy (docs/GPU-PLAN.md, stage 3): the compositor renders the captured output straight into a dmabuf we allocated, so
//! the picture does not go through a shm buffer (a read-back on the compositor's side and a copy on ours) and, with the VAAPI encoder, never
//! reaches the CPU at all. The buffers are linear XRGB8888 (bytes B G R X, the order the encoder wants), made with GBM on the render node
//! and registered with the compositor through `zwp_linux_dmabuf_v1`.

use std::fs::File;
use std::os::fd::{AsFd, AsRawFd, OwnedFd};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, bail, Context as _, Result};
use gbm::{BufferObject, BufferObjectFlags, Device, Format};
use wayland_client::protocol::wl_buffer;
use wayland_client::{Dispatch, QueueHandle};
use wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_buffer_params_v1::{self, ZwpLinuxBufferParamsV1};
use wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1;

/// `DRM_FORMAT_XRGB8888`, what a GLES compositor offers for a screencopy dmabuf (and what the pixman one cannot).
pub const XRGB8888: u32 = 0x3432_5258;
/// `DRM_FORMAT_ARGB8888`, what Hyprland offers: the same bytes (B G R A). The pool then makes ARGB buffers (the compositor only renders into the format it
/// offered) and the encoder reads them as XRGB (`encode_va.rs`), so the alpha byte is ignored.
pub const ARGB8888: u32 = 0x3432_5241;

/// How many buffers the pool rotates through: the compositor renders into one, the encoder reads another, and the newest waits in between.
/// One more, the first, is the damage oracle (see `oracle`) and is never given out.
const BUFFERS: usize = 5;

/// Gives every pool (and so every buffer of it) a number nobody else has: the encoder keeps its imports by it.
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

struct Slot {
    #[allow(dead_code)] // keeps the buffer object, and so its memory, for as long as the pool lives
    bo: BufferObject<()>,
    wl: wl_buffer::WlBuffer,
    fd: Arc<OwnedFd>,
    busy: Arc<AtomicBool>,
}

pub struct DmaPool {
    // Fields drop in this order, and a buffer object must go before the device it was made on: `device` is last.
    slots: Vec<Slot>,
    next: usize,
    id: u64,
    pub width: u32,
    pub height: u32,
    /// `XRGB8888` or `ARGB8888`: what the compositor offered.
    pub format: u32,
    stride: u32,
    #[allow(dead_code)]
    device: Device<File>,
}

/// A captured picture in GPU memory. The buffer is the pool's until the last holder of the frame lets go of it.
pub struct GpuFrame {
    /// Unique per buffer (pool number and slot), for the encoder's cache of imports.
    pub id: u64,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    fd: Arc<OwnedFd>,
    busy: Arc<AtomicBool>,
}

impl Drop for GpuFrame {
    fn drop(&mut self) {
        self.busy.store(false, Ordering::Release);
    }
}

impl GpuFrame {
    /// The dmabuf, to import into another API.
    pub fn dmabuf(&self) -> &OwnedFd {
        &self.fd
    }

    /// The picture as tight BGRA rows with alpha set opaque, read through a CPU mapping: slow (GPU memory), for the encoders and paths that
    /// want bytes (OpenH264, raw bitmaps).
    pub fn read_bgra(&self, flip: bool) -> Result<Vec<u8>> {
        // DMA_BUF_IOCTL_SYNC: the CPU is about to read (START) or has read (END); _IOW('b', 0, u64).
        const SYNC: libc::c_ulong = 0x4008_6200;
        const READ: u64 = 1;
        const START: u64 = 0;
        const END: u64 = 4;
        let (w, h, stride) = (self.width as usize, self.height as usize, self.stride as usize);
        let sync = |flags: u64| {
            let arg = flags;
            // SAFETY: the request takes a pointer to a u64 of flags and the fd is a dmabuf of ours.
            unsafe { libc::ioctl(self.fd.as_raw_fd(), SYNC, &arg) }
        };
        // SAFETY: the mapping is read-only, of a buffer of at least stride * height bytes that nobody else resizes.
        let map = unsafe { memmap2::MmapOptions::new().len(stride * h).map(&*self.fd) }.context("mmap of the dmabuf")?;
        sync(START | READ);
        let mut out = Vec::with_capacity(w * h * 4);
        for row in 0..h {
            let src = if flip { h - 1 - row } else { row };
            out.extend_from_slice(&map[src * stride..src * stride + w * 4]);
        }
        sync(END | READ);
        for px in out.chunks_exact_mut(4) {
            px[3] = 255;
        }
        Ok(out)
    }
}

impl DmaPool {
    pub fn new<S: 'static>(node: &Path, dmabuf: &ZwpLinuxDmabufV1, qh: &QueueHandle<S>, width: u32, height: u32, format: u32) -> Result<Self>
    where
        S: Dispatch<ZwpLinuxBufferParamsV1, ()> + Dispatch<wl_buffer::WlBuffer, ()>,
    {
        let file = std::fs::OpenOptions::new().read(true).write(true).open(node).with_context(|| format!("open {}", node.display()))?;
        let device = Device::new(file).context("GBM device")?;
        let (mut slots, mut stride_of) = (Vec::with_capacity(BUFFERS), 0);
        for _ in 0..BUFFERS {
            let bo = device
                .create_buffer_object::<()>(width, height, if format == ARGB8888 { Format::Argb8888 } else { Format::Xrgb8888 }, BufferObjectFlags::RENDERING | BufferObjectFlags::LINEAR)
                .context("GBM buffer")?;
            let fd = bo.fd().map_err(|e| anyhow!("dmabuf fd: {e}"))?;
            let stride = bo.stride().map_err(|e| anyhow!("stride: {e}"))?;
            let offset = bo.offset(0).map_err(|e| anyhow!("offset: {e}"))?;
            let modifier: u64 = bo.modifier().map_err(|e| anyhow!("modifier: {e}"))?.into();
            if modifier != 0 {
                bail!("GBM made a buffer with modifier {modifier:#x}, not linear");
            }
            let params = dmabuf.create_params(qh, ());
            params.add(fd.as_fd(), 0, offset, stride, (modifier >> 32) as u32, modifier as u32);
            let wl = params.create_immed(width as i32, height as i32, format, zwp_linux_buffer_params_v1::Flags::empty(), qh, ());
            params.destroy();
            stride_of = stride;
            slots.push(Slot { bo, wl, fd: Arc::new(fd), busy: Arc::new(AtomicBool::new(false)) });
        }
        Ok(Self { slots, next: 0, id: NEXT_ID.fetch_add(1, Ordering::Relaxed), width, height, format, stride: stride_of, device })
    }

    /// The buffer the compositor copies into with `copy_with_damage` only to say what changed. That request writes just the changed areas, into
    /// a buffer that is expected to hold the rest from before: so it has to be the same buffer every time, and nobody reads it.
    pub fn oracle(&self) -> &wl_buffer::WlBuffer {
        &self.slots[0].wl
    }

    /// A buffer nobody holds, for the next capture; None when the encoder has kept all of them.
    pub fn free_slot(&mut self) -> Option<usize> {
        let others = self.slots.len() - 1;
        for _ in 0..others {
            let i = 1 + self.next % others;
            self.next = (self.next + 1) % others;
            if !self.slots[i].busy.load(Ordering::Acquire) {
                return Some(i);
            }
        }
        None
    }

    pub fn buffer(&self, i: usize) -> &wl_buffer::WlBuffer {
        &self.slots[i].wl
    }

    /// Takes slot `i` for a frame: it stays busy until the frame is dropped.
    pub fn frame(&self, i: usize) -> GpuFrame {
        let s = &self.slots[i];
        s.busy.store(true, Ordering::Release);
        GpuFrame { id: (self.id << 8) | i as u64, width: self.width, height: self.height, stride: self.stride, fd: s.fd.clone(), busy: s.busy.clone() }
    }
}

impl Drop for DmaPool {
    fn drop(&mut self) {
        for s in &self.slots {
            s.wl.destroy();
        }
    }
}
