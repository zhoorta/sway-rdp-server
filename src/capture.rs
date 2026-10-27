//! wlr-screencopy capture of one output, plus the shared "latest frame + dirty rectangle" the RDP side reads.

use std::os::fd::{AsFd, FromRawFd};
use std::sync::{Arc, Mutex};

use anyhow::{bail, Context as _, Result};
use memmap2::MmapMut;
use tokio::sync::Notify;
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_buffer, wl_output, wl_registry, wl_shm, wl_shm_pool};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};
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
    latest: Arc<Vec<u8>>,
    dirty: Option<Rect>,
}

impl Shared {
    pub fn new(width: u16, height: u16, data: Vec<u8>) -> Self {
        let full = Rect { x: 0, y: 0, w: width.into(), h: height.into() };
        Self {
            width,
            height,
            state: Mutex::new(Pending { latest: Arc::new(data), dirty: Some(full) }),
            notify: Notify::new(),
        }
    }

    /// Store a new frame. The area that really changed is found by comparing with the previous frame;
    /// the compositor's own damage is only logged (it proved unreliable on a scaled headless output).
    pub fn push(&self, data: Vec<u8>, reported: Rect) {
        let mut st = self.state.lock().unwrap();
        let diff = diff_bbox(&st.latest, &data, usize::from(self.width), usize::from(self.height));
        tracing::debug!(?reported, ?diff, "frame");
        st.latest = Arc::new(data);
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

    pub async fn next(&self) -> (Arc<Vec<u8>>, Rect) {
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

pub struct Captured {
    pub width: u16,
    pub height: u16,
    pub data: Vec<u8>,
    pub damage: Rect,
}

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
}

impl State {
    fn reset_frame(&mut self) {
        self.buffer = None;
        self.buffer_done = false;
        self.y_invert = false;
        self.damage.clear();
        self.ready = false;
        self.failed = false;
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

        Ok(Self { queue, qh, state, manager, manager_version, output, shm, pool: None })
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
        Ok(Captured { width: w as u16, height: h as u16, data, damage })
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
            Event::BufferDone => state.buffer_done = true,
            Event::Ready { .. } => state.ready = true,
            Event::Failed => state.failed = true,
            _ => {}
        }
    }
}
