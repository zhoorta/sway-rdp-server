//! ext-image-copy-capture capture of one output: the standard protocol that replaces wlr-screencopy (sway 1.11+, Hyprland, COSMIC offer it; COSMIC
//! does not offer the wlr one). Same contract as the wlr capture (`Capturer::next_frame` blocks until the output has a changed frame and returns it
//! as tight BGRA), shm buffers only for now.

use std::os::fd::{AsFd, FromRawFd};

use anyhow::{bail, Context as _, Result};
use memmap2::MmapMut;
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_buffer, wl_output, wl_registry, wl_shm, wl_shm_pool};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, WEnum};
use wayland_protocols::ext::image_capture_source::v1::client::ext_image_capture_source_v1::ExtImageCaptureSourceV1;
use wayland_protocols::ext::image_capture_source::v1::client::ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1;
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_frame_v1::{self, ExtImageCopyCaptureFrameV1};
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_manager_v1::{ExtImageCopyCaptureManagerV1, Options};
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_session_v1::{self, ExtImageCopyCaptureSessionV1};

use crate::capture::{cursor_in_picture, Captured, Pixels, Rect};

#[derive(Default)]
struct State {
    outputs: Vec<(wl_output::WlOutput, Option<String>)>,
    /// What the session wants of a buffer; complete when `constraints_done`.
    size: Option<(u32, u32)>,
    formats: Vec<wl_shm::Format>,
    constraints_done: bool,
    stopped: bool,
    damage: Vec<Rect>,
    ready: bool,
    failed: Option<WEnum<ext_image_copy_capture_frame_v1::FailureReason>>,
}

impl State {
    fn new_constraints(&mut self) {
        if self.constraints_done {
            self.constraints_done = false;
            self.size = None;
            self.formats.clear();
        }
    }
}

struct Buffer {
    pool: wl_shm_pool::WlShmPool,
    buffer: wl_buffer::WlBuffer,
    map: MmapMut,
    width: u32,
    height: u32,
    format: wl_shm::Format,
}

pub struct ExtCapturer {
    queue: EventQueue<State>,
    qh: QueueHandle<State>,
    state: State,
    shm: wl_shm::WlShm,
    session: ExtImageCopyCaptureSessionV1,
    buffer: Option<Buffer>,
    /// The first frame is asked for in full; after it the buffer holds the picture and the compositor adds what changes.
    first: bool,
}

/// Whether the compositor offers the protocol (and so this capture can be used).
pub fn offered(globals: &GlobalListContents) -> bool {
    globals.with_list(|l| {
        ["ext_image_copy_capture_manager_v1", "ext_output_image_capture_source_manager_v1"].iter().all(|n| l.iter().any(|g| g.interface == *n))
    })
}

impl ExtCapturer {
    pub fn new(want_output: Option<&str>) -> Result<Self> {
        let conn = Connection::connect_to_env().context("cannot connect to the Wayland display")?;
        let (globals, mut queue) = registry_queue_init::<State>(&conn).context("registry")?;
        let qh = queue.handle();
        let mut state = State::default();

        let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ()).context("wl_shm missing")?;
        let manager: ExtImageCopyCaptureManagerV1 = globals.bind(&qh, 1..=1, ()).context("compositor has no ext-image-copy-capture")?;
        let sources: ExtOutputImageCaptureSourceManagerV1 = globals.bind(&qh, 1..=1, ()).context("compositor has no ext-image-capture-source")?;
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

        let source: ExtImageCaptureSourceV1 = sources.create_source(&output, &qh, ());
        let options = if cursor_in_picture() { Options::PaintCursors } else { Options::empty() };
        let session = manager.create_session(&source, options, &qh, ());
        let mut me = Self { queue, qh, state, shm, session, buffer: None, first: true };
        me.wait_constraints()?;
        Ok(me)
    }

    fn wait_constraints(&mut self) -> Result<()> {
        while !self.state.constraints_done {
            if self.state.stopped {
                bail!("the capture session stopped before it announced its buffers");
            }
            self.queue.blocking_dispatch(&mut self.state)?;
        }
        Ok(())
    }

    /// A buffer that fits the session's constraints now (made again when they changed).
    fn ensure_buffer(&mut self) -> Result<()> {
        let (w, h) = self.state.size.context("the session announced no buffer size")?;
        let format = [wl_shm::Format::Xrgb8888, wl_shm::Format::Argb8888, wl_shm::Format::Xbgr8888, wl_shm::Format::Abgr8888]
            .into_iter()
            .find(|f| self.state.formats.contains(f))
            .with_context(|| format!("no shm format the capture takes in {:?}", self.state.formats))?;
        if self.buffer.as_ref().is_some_and(|b| b.width == w && b.height == h && b.format == format) {
            return Ok(());
        }
        if let Some(old) = self.buffer.take() {
            old.buffer.destroy();
            old.pool.destroy();
        }
        let stride = w * 4;
        let size = stride as usize * h as usize;
        let raw = unsafe { libc::memfd_create(c"waydesk-server-shm".as_ptr(), libc::MFD_CLOEXEC) };
        if raw < 0 {
            bail!("memfd_create failed: {}", std::io::Error::last_os_error());
        }
        let file = unsafe { std::fs::File::from_raw_fd(raw) };
        file.set_len(size as u64)?;
        let map = unsafe { MmapMut::map_mut(&file)? };
        let pool = self.shm.create_pool(file.as_fd(), size as i32, &self.qh, ());
        let buffer = pool.create_buffer(0, w as i32, h as i32, stride as i32, format, &self.qh, ());
        self.buffer = Some(Buffer { pool, buffer, map, width: w, height: h, format });
        self.first = true;
        Ok(())
    }

    /// Blocks until the compositor has a changed frame, then returns it as tight BGRA.
    pub fn next_frame(&mut self) -> Result<Captured> {
        for _ in 0..4 {
            self.wait_constraints()?;
            self.ensure_buffer()?;
            self.state.damage.clear();
            self.state.ready = false;
            self.state.failed = None;
            let buf = self.buffer.as_ref().expect("buffer");
            let (w, h) = (buf.width, buf.height);
            let frame = self.session.create_frame(&self.qh, ());
            frame.attach_buffer(&buf.buffer);
            if self.first {
                frame.damage_buffer(0, 0, w as i32, h as i32);
            }
            frame.capture();
            while !self.state.ready && self.state.failed.is_none() {
                if self.state.stopped {
                    frame.destroy();
                    bail!("the capture session stopped (the output went away)");
                }
                self.queue.blocking_dispatch(&mut self.state)?;
            }
            frame.destroy();
            if let Some(why) = self.state.failed.take() {
                match why {
                    // The output changed size or format: the session has announced (or is about to) new constraints; the frame is asked again with them.
                    WEnum::Value(ext_image_copy_capture_frame_v1::FailureReason::BufferConstraints) => {
                        self.queue.roundtrip(&mut self.state)?;
                        continue;
                    }
                    other => bail!("capture frame failed: {other:?}"),
                }
            }
            self.first = false;
            let buf = self.buffer.as_ref().expect("buffer");
            let swap_rb = matches!(buf.format, wl_shm::Format::Xbgr8888 | wl_shm::Format::Abgr8888);
            let mut data = buf.map[..(w * h * 4) as usize].to_vec();
            for px in data.chunks_exact_mut(4) {
                if swap_rb {
                    px.swap(0, 2);
                }
                px[3] = 255;
            }
            let full = Rect { x: 0, y: 0, w, h };
            let damage = self.state.damage.iter().copied().reduce(Rect::union).unwrap_or(full);
            return Ok(Captured { width: w as u16, height: h as u16, pixels: Pixels::Cpu(data), damage });
        }
        bail!("the capture session kept changing its buffer constraints")
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
impl Dispatch<ExtImageCopyCaptureManagerV1, ()> for State {
    fn event(_: &mut Self, _: &ExtImageCopyCaptureManagerV1, _: <ExtImageCopyCaptureManagerV1 as wayland_client::Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<ExtOutputImageCaptureSourceManagerV1, ()> for State {
    fn event(_: &mut Self, _: &ExtOutputImageCaptureSourceManagerV1, _: <ExtOutputImageCaptureSourceManagerV1 as wayland_client::Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<ExtImageCaptureSourceV1, ()> for State {
    fn event(_: &mut Self, _: &ExtImageCaptureSourceV1, _: <ExtImageCaptureSourceV1 as wayland_client::Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
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
impl Dispatch<ExtImageCopyCaptureSessionV1, ()> for State {
    fn event(state: &mut Self, _: &ExtImageCopyCaptureSessionV1, event: ext_image_copy_capture_session_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        use ext_image_copy_capture_session_v1::Event;
        match event {
            // The events of one set of constraints come in no fixed order, and `done` ends the set: the first one after a `done` starts a new set.
            Event::BufferSize { width, height } => {
                state.new_constraints();
                state.size = Some((width, height));
            }
            Event::ShmFormat { format } => {
                state.new_constraints();
                if let WEnum::Value(f) = format {
                    state.formats.push(f);
                }
            }
            Event::Done => state.constraints_done = true,
            Event::Stopped => state.stopped = true,
            _ => {}
        }
    }
}
impl Dispatch<ExtImageCopyCaptureFrameV1, ()> for State {
    fn event(state: &mut Self, _: &ExtImageCopyCaptureFrameV1, event: ext_image_copy_capture_frame_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        use ext_image_copy_capture_frame_v1::Event;
        match event {
            Event::Damage { x, y, width, height } => {
                state.damage.push(Rect { x: x.max(0) as u32, y: y.max(0) as u32, w: width.max(0) as u32, h: height.max(0) as u32 })
            }
            Event::Ready => state.ready = true,
            Event::Failed { reason } => state.failed = Some(reason),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_set_of_constraints_replaces_the_old_whichever_event_comes_first() {
        let mut s = State { size: Some((10, 10)), formats: vec![wl_shm::Format::Xrgb8888], constraints_done: true, ..State::default() };
        s.new_constraints();
        assert!(!s.constraints_done && s.size.is_none() && s.formats.is_empty());
        // events of the same set that follow do not clear what the first one set
        s.size = Some((20, 20));
        s.new_constraints();
        assert_eq!(s.size, Some((20, 20)));
    }
}
