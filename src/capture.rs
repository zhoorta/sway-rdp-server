//! wlr-screencopy capture of one output, plus the shared "latest frame + dirty rectangle" the RDP side reads.

use std::os::fd::{AsFd, FromRawFd};
use std::sync::{Arc, Mutex};

use anyhow::{bail, Context as _, Result};
use memmap2::MmapMut;
use tokio::sync::Notify;
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_buffer, wl_output, wl_registry, wl_shm, wl_shm_pool};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};
#[cfg(feature = "vaapi")]
use wayland_protocols::wp::linux_dmabuf::zv1::client::{zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1, zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1};
use wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1};
use wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1;

/// Whether the mouse cursor is in the picture (`WAYDESK_CURSOR=server`). By default it is not: the client draws its own
/// pointer, instantly and without a second one a network round trip behind (the Windows App on a Mac did not hide its own
/// pointer when told to). A headless sway draws its cursor into every frame whatever screencopy is asked for, so in this
/// mode the session gives it an invisible cursor theme (waydesk-blank, see session.rs). With `server` the cursor has the shape
/// the app asked for, and the client's own pointer is made invisible, which only some clients honour. Set in
/// /etc/default/waydesk, which the daemon and the sessions both read.
pub fn cursor_in_picture() -> bool {
    static MODE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| std::env::var("WAYDESK_CURSOR").is_ok_and(|v| v.eq_ignore_ascii_case("server")))
}


#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    pub fn union(self, o: Rect) -> Rect {
        let x0 = self.x.min(o.x);
        let y0 = self.y.min(o.y);
        let x1 = (self.x + self.w).max(o.x + o.w);
        let y1 = (self.y + self.h).max(o.y + o.h);
        Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
    }
}

/// Latest captured frame (tight BGRA, stride = width * 4) and the area changed since the RDP side last took it.
pub struct Shared {
    pub width: u16,
    pub height: u16,
    state: Mutex<Pending>,
    notify: Notify,
}

struct Pending {
    latest: Arc<Pixels>,
    dirty: Option<Rect>,
    /// When the whole picture was last marked changed: frames that stay in GPU memory are not compared, they are trusted to the
    /// compositor's damage, and a full refresh now and then makes that safe against an under-reporting compositor.
    last_full: std::time::Instant,
}

/// How often a picture that is changing gets a full refresh, when the changes come from the compositor's damage alone.
const FULL_REFRESH: std::time::Duration = std::time::Duration::from_secs(10);

impl Shared {
    pub fn new(width: u16, height: u16, pixels: Pixels) -> Self {
        let full = Rect { x: 0, y: 0, w: width.into(), h: height.into() };
        Self {
            width,
            height,
            state: Mutex::new(Pending { latest: Arc::new(pixels), dirty: Some(full), last_full: std::time::Instant::now() }),
            notify: Notify::new(),
        }
    }

    /// Store a new frame. The area that really changed is found by comparing with the previous frame; the compositor's own damage is only
    /// logged (it proved unreliable on a scaled headless output). A frame in GPU memory cannot be compared without reading it, which is what
    /// keeping it there is for: it goes by the compositor's damage (checked against the real difference while dragging windows at scale 1,
    /// 1.5 and 2: never short), with a full refresh every `FULL_REFRESH`.
    pub fn push(&self, pixels: Pixels, reported: Rect) {
        let mut st = self.state.lock().unwrap();
        let full = Rect { x: 0, y: 0, w: self.width.into(), h: self.height.into() };
        let diff = match (&*st.latest, &pixels) {
            (Pixels::Cpu(old), Pixels::Cpu(new)) => diff_bbox(old, new, usize::from(self.width), usize::from(self.height)),
            #[cfg(feature = "vaapi")]
            (_, Pixels::Gpu(..)) => {
                if st.last_full.elapsed() >= FULL_REFRESH {
                    st.last_full = std::time::Instant::now();
                    Some(full)
                } else {
                    Some(reported)
                }
            }
            // The kind of capture changed: nothing to compare with.
            #[cfg(feature = "vaapi")]
            (_, Pixels::Cpu(_)) => Some(full),
        };
        tracing::debug!(?reported, ?diff, "frame");
        st.latest = Arc::new(pixels);
        if let Some(d) = diff {
            st.dirty = Some(st.dirty.map_or(d, |old| old.union(d)));
            drop(st);
            self.notify.notify_one();
        }
    }

    /// Put an area back on the dirty list (it was taken but could not be sent).
    pub fn mark_dirty(&self, r: Rect) {
        let mut st = self.state.lock().unwrap();
        st.dirty = Some(st.dirty.map_or(r, |old| old.union(r)));
        drop(st);
        self.notify.notify_one();
    }

    /// Mark the whole screen dirty (new connection, client synchronise, resize).
    pub fn force_full(&self) {
        let full = Rect { x: 0, y: 0, w: self.width.into(), h: self.height.into() };
        self.state.lock().unwrap().dirty = Some(full);
        self.notify.notify_one();
    }

    pub async fn next(&self) -> (Arc<Pixels>, Rect) {
        loop {
            {
                let mut st = self.state.lock().unwrap();
                if let Some(r) = st.dirty.take() {
                    return (st.latest.clone(), r);
                }
            }
            self.notify.notified().await;
        }
    }
}

/// Bounding box of the pixels that differ between two tight BGRA frames of the same size.
fn diff_bbox(a: &[u8], b: &[u8], w: usize, h: usize) -> Option<Rect> {
    if a.len() != b.len() {
        return Some(Rect { x: 0, y: 0, w: w as u32, h: h as u32 });
    }
    let stride = w * 4;
    let (mut y0, mut y1, mut x0, mut x1) = (None::<usize>, 0usize, w, 0usize);
    for y in 0..h {
        let (ra, rb) = (&a[y * stride..(y + 1) * stride], &b[y * stride..(y + 1) * stride]);
        if ra == rb {
            continue;
        }
        y0.get_or_insert(y);
        y1 = y;
        let first = ra.chunks_exact(4).zip(rb.chunks_exact(4)).position(|(p, q)| p != q).unwrap_or(0);
        let last = w - 1 - ra.chunks_exact(4).rev().zip(rb.chunks_exact(4).rev()).position(|(p, q)| p != q).unwrap_or(0);
        x0 = x0.min(first);
        x1 = x1.max(last);
    }
    y0.map(|y0| Rect { x: x0 as u32, y: y0 as u32, w: (x1 - x0 + 1) as u32, h: (y1 - y0 + 1) as u32 })
}

/// A picture: bytes in memory, or (with a GPU) a buffer in GPU memory that never has to reach the CPU.
pub enum Pixels {
    /// Tight BGRA, stride = width * 4.
    Cpu(Vec<u8>),
    #[cfg(feature = "vaapi")]
    Gpu(crate::dmabuf::GpuFrame, bool),
}

impl Pixels {
    /// The picture as tight BGRA bytes: borrowed when it is in memory already, read out of the GPU buffer (slowly) when not.
    pub fn bgra(&self) -> Result<std::borrow::Cow<'_, [u8]>> {
        match self {
            Pixels::Cpu(v) => Ok(std::borrow::Cow::Borrowed(v)),
            #[cfg(feature = "vaapi")]
            Pixels::Gpu(g, flip) => Ok(std::borrow::Cow::Owned(g.read_bgra(*flip)?)),
        }
    }
}

pub struct Captured {
    pub width: u16,
    pub height: u16,
    pub pixels: Pixels,
    pub damage: Rect,
}

/// Set when nothing consumes GPU frames (the encoder is not the VAAPI one, or the client wants bitmaps): the capture goes back to shm.
pub static PREFER_SHM: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[derive(Clone)]
struct BufInfo {
    format: wl_shm::Format,
    width: u32,
    height: u32,
    stride: u32,
}

#[derive(Default)]
struct State {
    outputs: Vec<(wl_output::WlOutput, Option<String>)>,
    buffer: Option<BufInfo>,
    buffer_done: bool,
    y_invert: bool,
    damage: Vec<Rect>,
    ready: bool,
    failed: bool,
    /// The dmabuf the compositor offers for this frame: DRM format, width, height.
    dmabuf: Option<(u32, u32, u32)>,
}

impl State {
    fn reset_frame(&mut self) {
        self.buffer = None;
        self.buffer_done = false;
        self.y_invert = false;
        self.damage.clear();
        self.ready = false;
        self.failed = false;
        self.dmabuf = None;
    }
}

struct Pool {
    pool: wl_shm_pool::WlShmPool,
    map: MmapMut,
    size: usize,
}

pub struct Capturer {
    queue: EventQueue<State>,
    qh: QueueHandle<State>,
    state: State,
    manager: ZwlrScreencopyManagerV1,
    manager_version: u32,
    output: wl_output::WlOutput,
    shm: wl_shm::WlShm,
    pool: Option<Pool>,
    /// GPU capture: the compositor renders into our dmabufs (see dmabuf.rs). None: shm, as without a GPU.
    #[cfg(feature = "vaapi")]
    gpu: Gpu,
}

#[cfg(feature = "vaapi")]
struct Gpu {
    node: Option<std::path::PathBuf>,
    dmabuf: Option<ZwpLinuxDmabufV1>,
    pool: Option<crate::dmabuf::DmaPool>,
}

impl Capturer {
    pub fn new(want_output: Option<&str>) -> Result<Self> {
        let conn = Connection::connect_to_env().context("cannot connect to the Wayland display")?;
        let (globals, mut queue) = registry_queue_init::<State>(&conn).context("registry")?;
        let qh = queue.handle();
        let mut state = State::default();

        let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ()).context("wl_shm missing")?;
        let manager: ZwlrScreencopyManagerV1 =
            globals.bind(&qh, 1..=3, ()).context("compositor has no wlr-screencopy")?;
        let manager_version = manager.version();

        globals.contents().with_list(|list| {
            for g in list.iter().filter(|g| g.interface == "wl_output") {
                let out: wl_output::WlOutput = globals.registry().bind(g.name, g.version.min(4), &qh, ());
                state.outputs.push((out, None));
            }
        });
        queue.roundtrip(&mut state).context("roundtrip for output names")?;

        let output = state
            .outputs
            .iter()
            .find(|(_, n)| want_output.is_none_or(|w| n.as_deref() == Some(w)))
            .map(|(o, _)| o.clone())
            .with_context(|| {
                let names: Vec<_> = state.outputs.iter().map(|(_, n)| n.clone().unwrap_or_default()).collect();
                format!("output {want_output:?} not found, have {names:?}")
            })?;

        #[cfg(feature = "vaapi")]
        let gpu = {
            // Only with a GPU session, and `WAYDESK_DMABUF=0` switches it off (it is for measuring and for a driver that does not like it).
            let wanted = std::env::var("WAYDESK_DMABUF").map_or(true, |v| v != "0");
            let node = if wanted { crate::gpu::usable_node() } else { None };
            let dmabuf = node.as_ref().and_then(|_| globals.bind::<ZwpLinuxDmabufV1, _, _>(&qh, 3..=3, ()).ok());
            Gpu { node: node.filter(|_| dmabuf.is_some()), dmabuf, pool: None }
        };
        Ok(Self {
            queue,
            qh,
            state,
            manager,
            manager_version,
            output,
            shm,
            pool: None,
            #[cfg(feature = "vaapi")]
            gpu,
        })
    }

    fn ensure_pool(&mut self, size: usize) -> Result<()> {
        if self.pool.as_ref().is_some_and(|p| p.size >= size) {
            return Ok(());
        }
        if let Some(old) = self.pool.take() {
            old.pool.destroy();
        }
        let raw = unsafe { libc::memfd_create(c"waydesk-server-shm".as_ptr(), libc::MFD_CLOEXEC) };
        if raw < 0 {
            bail!("memfd_create failed: {}", std::io::Error::last_os_error());
        }
        let file = unsafe { std::fs::File::from_raw_fd(raw) };
        file.set_len(size as u64)?;
        let map = unsafe { MmapMut::map_mut(&file)? };
        let pool = self.shm.create_pool(file.as_fd(), size as i32, &self.qh, ());
        self.pool = Some(Pool { pool, map, size });
        Ok(())
    }

    /// The frame into a dmabuf, when the session has a GPU and the compositor offers one. `None`: the shm path takes this frame (and the next ones,
    /// if something failed here).
    #[cfg(feature = "vaapi")]
    fn next_frame_dmabuf(&mut self, frame: &ZwlrScreencopyFrameV1) -> Result<Option<Captured>> {
        if PREFER_SHM.load(std::sync::atomic::Ordering::Relaxed) {
            self.gpu.node = None;
            self.gpu.pool = None;
        }
        let (Some(node), Some(dmabuf)) = (self.gpu.node.clone(), self.gpu.dmabuf.clone()) else { return Ok(None) };
        let Some((format, w, h)) = self.state.dmabuf else {
            tracing::info!("the compositor offers no dmabuf for the screen capture: shm");
            self.gpu.node = None;
            return Ok(None);
        };
        if format != crate::dmabuf::XRGB8888 {
            tracing::info!(format = format!("{format:#x}"), "the compositor offers a dmabuf format the capture does not take: shm");
            self.gpu.node = None;
            return Ok(None);
        }
        if self.gpu.pool.as_ref().is_none_or(|p| p.width != w || p.height != h) {
            self.gpu.pool = None;
            match crate::dmabuf::DmaPool::new(&node, &dmabuf, &self.qh, w, h) {
                Ok(pool) => {
                    tracing::info!(w, h, "screen capture into dmabufs");
                    self.gpu.pool = Some(pool);
                }
                Err(e) => {
                    tracing::warn!("no dmabuf capture ({e:#}): shm");
                    self.gpu.node = None;
                    return Ok(None);
                }
            }
        }
        // 1. What changed: `copy_with_damage` into the pool's oracle buffer, which waits for the next change. It writes only the changed areas,
        //    so the buffer is no picture; the picture comes from the second request.
        let oracle = self.gpu.pool.as_ref().expect("pool").oracle().clone();
        if self.manager_version >= 2 {
            frame.copy_with_damage(&oracle);
        } else {
            frame.copy(&oracle);
        }
        while !self.state.ready {
            if self.state.failed {
                return self.dmabuf_failed();
            }
            self.queue.blocking_dispatch(&mut self.state)?;
        }
        let full = Rect { x: 0, y: 0, w, h };
        let damage = self.state.damage.iter().copied().reduce(Rect::union).unwrap_or(full);

        // 2. The picture: a plain copy of the output as it is now into a buffer the encoder is not holding. It lets go of its frames as it goes,
        //    so a short wait is normal and a long one is a stuck encoder.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let slot = loop {
            if let Some(i) = self.gpu.pool.as_mut().expect("pool").free_slot() {
                break i;
            }
            if std::time::Instant::now() > deadline {
                bail!("every dmabuf of the capture is still held by the encoder");
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        };
        self.state.reset_frame();
        let snap = self.manager.capture_output(i32::from(cursor_in_picture()), &self.output, &self.qh, ());
        while !self.state.buffer_done && self.manager_version >= 3 || self.state.dmabuf.is_none() {
            if self.state.failed {
                snap.destroy();
                return self.dmabuf_failed();
            }
            self.queue.blocking_dispatch(&mut self.state)?;
        }
        if self.state.dmabuf != Some((format, w, h)) {
            // The output changed size between the two requests: this one is lost, the next capture builds a new pool.
            snap.destroy();
            self.gpu.pool = None;
            return self.shm_retry();
        }
        snap.copy(self.gpu.pool.as_ref().expect("pool").buffer(slot));
        while !self.state.ready {
            if self.state.failed {
                snap.destroy();
                return self.dmabuf_failed();
            }
            self.queue.blocking_dispatch(&mut self.state)?;
        }
        snap.destroy();
        let gpu = self.gpu.pool.as_ref().expect("pool").frame(slot);
        Ok(Some(Captured { width: w as u16, height: h as u16, pixels: Pixels::Gpu(gpu, self.state.y_invert), damage }))
    }

    /// The compositor could not render into a dmabuf: the shm path takes over for good.
    #[cfg(feature = "vaapi")]
    fn dmabuf_failed(&mut self) -> Result<Option<Captured>> {
        tracing::warn!("the compositor could not render into the dmabuf: shm");
        self.gpu.node = None;
        self.gpu.pool = None;
        self.state.reset_frame();
        self.shm_retry()
    }

    #[cfg(feature = "vaapi")]
    fn shm_retry(&mut self) -> Result<Option<Captured>> {
        self.next_frame().map(Some)
    }

    /// Blocks until the compositor has a changed frame (copy_with_damage), then returns it as tight BGRA.
    pub fn next_frame(&mut self) -> Result<Captured> {
        self.state.reset_frame();
        let frame = self.manager.capture_output(i32::from(cursor_in_picture()), &self.output, &self.qh, ());

        while self.state.buffer.is_none() || (self.manager_version >= 3 && !self.state.buffer_done) {
            if self.state.failed {
                bail!("screencopy failed before the buffer was announced");
            }
            self.queue.blocking_dispatch(&mut self.state)?;
        }
        #[cfg(feature = "vaapi")]
        if let Some(captured) = self.next_frame_dmabuf(&frame)? {
            frame.destroy();
            return Ok(captured);
        }
        let info = self.state.buffer.clone().context("no buffer info")?;
        // The pixman renderer gives Xrgb8888 (bytes B G R X in memory, what the encoder takes); a GPU renderer gives Xbgr8888 (R G B X).
        let swap_rb = match info.format {
            wl_shm::Format::Xrgb8888 | wl_shm::Format::Argb8888 => false,
            wl_shm::Format::Xbgr8888 | wl_shm::Format::Abgr8888 => true,
            other => bail!("unsupported shm format {other:?}"),
        };
        let size = info.stride as usize * info.height as usize;
        self.ensure_pool(size)?;
        let buffer = self.pool.as_ref().unwrap().pool.create_buffer(
            0,
            info.width as i32,
            info.height as i32,
            info.stride as i32,
            info.format,
            &self.qh,
            (),
        );
        if self.manager_version >= 2 {
            frame.copy_with_damage(&buffer);
        } else {
            frame.copy(&buffer);
        }
        while !self.state.ready {
            if self.state.failed {
                bail!("screencopy frame failed");
            }
            self.queue.blocking_dispatch(&mut self.state)?;
        }

        let (w, h, stride) = (info.width as usize, info.height as usize, info.stride as usize);
        let map = &self.pool.as_ref().unwrap().map;
        let mut data = Vec::with_capacity(w * h * 4);
        for row in 0..h {
            let src_row = if self.state.y_invert { h - 1 - row } else { row };
            data.extend_from_slice(&map[src_row * stride..src_row * stride + w * 4]);
        }
        for px in data.chunks_exact_mut(4) {
            if swap_rb {
                px.swap(0, 2);
            }
            px[3] = 255;
        }
        let full = Rect { x: 0, y: 0, w: w as u32, h: h as u32 };
        let damage = self.state.damage.iter().copied().reduce(Rect::union).unwrap_or(full);
        frame.destroy();
        buffer.destroy();
        Ok(Captured { width: w as u16, height: h as u16, pixels: Pixels::Cpu(data), damage })
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(_: &mut Self, _: &wl_registry::WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<wl_shm::WlShm, ()> for State {
    fn event(_: &mut Self, _: &wl_shm::WlShm, _: wl_shm::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<wl_shm_pool::WlShmPool, ()> for State {
    fn event(_: &mut Self, _: &wl_shm_pool::WlShmPool, _: wl_shm_pool::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<wl_buffer::WlBuffer, ()> for State {
    fn event(_: &mut Self, _: &wl_buffer::WlBuffer, _: wl_buffer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<ZwlrScreencopyManagerV1, ()> for State {
    fn event(_: &mut Self, _: &ZwlrScreencopyManagerV1, _: <ZwlrScreencopyManagerV1 as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<wl_output::WlOutput, ()> for State {
    fn event(state: &mut Self, proxy: &wl_output::WlOutput, event: wl_output::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wl_output::Event::Name { name } = event {
            if let Some(entry) = state.outputs.iter_mut().find(|(o, _)| o == proxy) {
                entry.1 = Some(name);
            }
        }
    }
}
impl Dispatch<ZwlrScreencopyFrameV1, ()> for State {
    fn event(state: &mut Self, _: &ZwlrScreencopyFrameV1, event: zwlr_screencopy_frame_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        use zwlr_screencopy_frame_v1::Event;
        match event {
            Event::Buffer { format, width, height, stride } => {
                if let WEnum::Value(format) = format {
                    state.buffer = Some(BufInfo { format, width, height, stride });
                }
            }
            Event::Flags { flags } => {
                state.y_invert = matches!(flags, WEnum::Value(f) if f.contains(zwlr_screencopy_frame_v1::Flags::YInvert));
            }
            Event::Damage { x, y, width, height } => state.damage.push(Rect { x, y, w: width, h: height }),
            Event::LinuxDmabuf { format, width, height } => state.dmabuf = Some((format, width, height)),
            Event::BufferDone => state.buffer_done = true,
            Event::Ready { .. } => state.ready = true,
            Event::Failed => state.failed = true,
            _ => {}
        }
    }
}

#[cfg(feature = "vaapi")]
impl Dispatch<ZwpLinuxDmabufV1, ()> for State {
    fn event(_: &mut Self, _: &ZwpLinuxDmabufV1, _: <ZwpLinuxDmabufV1 as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
#[cfg(feature = "vaapi")]
impl Dispatch<ZwpLinuxBufferParamsV1, ()> for State {
    fn event(_: &mut Self, _: &ZwpLinuxBufferParamsV1, _: <ZwpLinuxBufferParamsV1 as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
