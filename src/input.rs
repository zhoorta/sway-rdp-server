//! Input injection into sway: a virtual keyboard (zwp_virtual_keyboard_v1) and a virtual pointer
//! (zwlr_virtual_pointer_v1), driven from a command channel on their own Wayland connection.

use std::os::fd::{AsFd, FromRawFd};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Instant;

use anyhow::{Context as _, Result};
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_output, wl_pointer, wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1;
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1;
use wayland_protocols_wlr::virtual_pointer::v1::client::zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1;
use wayland_protocols_wlr::virtual_pointer::v1::client::zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1;
use xkbcommon::xkb;

pub const BTN_LEFT: u32 = 0x110;
pub const BTN_RIGHT: u32 = 0x111;
pub const BTN_MIDDLE: u32 = 0x112;
pub const BTN_SIDE: u32 = 0x113;
pub const BTN_EXTRA: u32 = 0x114;

#[derive(Debug)]
pub enum Cmd {
    Key { code: u32, pressed: bool },
    Move { x: u32, y: u32 },
    /// The captured output changed size: RDP coordinates now refer to this extent.
    SetExtent { w: u32, h: u32 },
    Button { code: u32, pressed: bool },
    /// Wheel notches, positive = away from the user (RDP convention); converted for Wayland here.
    Wheel { vertical: f64, horizontal: f64 },
}

/// RDP (XT set 1) scancode to Linux evdev keycode.
pub fn evdev_code(code: u8, extended: bool) -> Option<u32> {
    if !extended {
        return (1..=0x58).contains(&code).then_some(u32::from(code));
    }
    Some(match code {
        0x1c => 96,  // KP Enter
        0x1d => 97,  // Right Ctrl
        0x35 => 98,  // KP /
        0x37 => 99,  // Print / SysRq
        0x38 => 100, // Right Alt (AltGr)
        0x47 => 102, // Home
        0x48 => 103, // Up
        0x49 => 104, // Page Up
        0x4b => 105, // Left
        0x4d => 106, // Right
        0x4f => 107, // End
        0x50 => 108, // Down
        0x51 => 109, // Page Down
        0x52 => 110, // Insert
        0x53 => 111, // Delete
        0x5b => 125, // Left Super
        0x5c => 126, // Right Super
        0x5d => 127, // Menu
        _ => return None,
    })
}

#[derive(Default)]
struct State {
    outputs: Vec<(wl_output::WlOutput, Option<String>)>,
}

/// Starts the input thread. `extent` is the size (in output pixels) of the captured output that RDP coordinates refer to.
pub fn spawn(output_name: Option<String>, extent: (u32, u32), layout: String) -> Result<Sender<Cmd>> {
    let (tx, rx) = channel::<Cmd>();
    let (ready_tx, ready_rx) = channel::<Result<()>>();
    std::thread::spawn(move || match setup(output_name.as_deref(), &layout) {
        Ok(mut inj) => {
            let _ = ready_tx.send(Ok(()));
            inj.run(rx, extent);
        }
        Err(e) => {
            let _ = ready_tx.send(Err(e));
        }
    });
    ready_rx.recv().context("input thread died")??;
    Ok(tx)
}

struct Injector {
    conn: Connection,
    xkb_state: xkb::State,
    mods: (u32, u32, u32, u32),
    vk: ZwpVirtualKeyboardV1,
    ptr: ZwlrVirtualPointerV1,
    start: Instant,
    /// Wheel amounts that do not make a whole notch yet (vertical, horizontal).
    scroll: [f64; 2],
    /// Multiplies every wheel amount (`SWAY_RDP_SCROLL_SCALE`, 1 = a notch is a notch).
    scroll_scale: f64,
}

fn setup(want_output: Option<&str>, layout: &str) -> Result<Injector> {
    let conn = Connection::connect_to_env().context("cannot connect to the Wayland display")?;
    let (globals, mut queue) = registry_queue_init::<State>(&conn).context("registry")?;
    let qh = queue.handle();
    let mut state = State::default();

    let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=1, ()).context("no wl_seat")?;
    let vk_mgr: ZwpVirtualKeyboardManagerV1 =
        globals.bind(&qh, 1..=1, ()).context("compositor has no virtual-keyboard")?;
    let ptr_mgr: ZwlrVirtualPointerManagerV1 =
        globals.bind(&qh, 1..=2, ()).context("compositor has no wlr-virtual-pointer")?;

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
        .context("output for the pointer not found")?;

    // Keyboard: compile a keymap from the layout name and hand it to the compositor.
    let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    let keymap = xkb::Keymap::new_from_names(&ctx, "", "", layout, "", None, xkb::COMPILE_NO_FLAGS)
        .with_context(|| format!("cannot compile an xkb keymap for layout {layout:?}"))?;
    let mut text = keymap.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1).into_bytes();
    text.push(0);
    let raw = unsafe { libc::memfd_create(c"sway-rdp-server-keymap".as_ptr(), libc::MFD_CLOEXEC) };
    anyhow::ensure!(raw >= 0, "memfd_create failed: {}", std::io::Error::last_os_error());
    let file = unsafe { std::fs::File::from_raw_fd(raw) };
    file.set_len(text.len() as u64)?;
    {
        use std::io::Write as _;
        (&file).write_all(&text)?;
    }
    let vk = vk_mgr.create_virtual_keyboard(&seat, &qh, ());
    vk.keymap(1, file.as_fd(), text.len() as u32); // 1 = XKB_V1

    let ptr = ptr_mgr.create_virtual_pointer_with_output(Some(&seat), Some(&output), &qh, ());
    queue.roundtrip(&mut state).context("roundtrip after creating virtual devices")?;
    let xkb_state = xkb::State::new(&keymap);
    let scroll_scale = std::env::var("SWAY_RDP_SCROLL_SCALE").ok().and_then(|v| v.parse::<f64>().ok()).filter(|v| *v > 0.0).unwrap_or(1.0);
    Ok(Injector { conn, xkb_state, mods: (0, 0, 0, 0), vk, ptr, start: Instant::now(), scroll: [0.0; 2], scroll_scale })
}

impl Injector {
    fn run(&mut self, rx: Receiver<Cmd>, mut extent: (u32, u32)) {
        while let Ok(cmd) = rx.recv() {
            let t = self.start.elapsed().as_millis() as u32;
            match cmd {
                Cmd::Key { code, pressed } => {
                    self.vk.key(t, code, u32::from(pressed));
                    // The compositor does not derive modifiers from virtual key events; track and send them.
                    let dir = if pressed { xkb::KeyDirection::Down } else { xkb::KeyDirection::Up };
                    self.xkb_state.update_key(xkb::Keycode::new(code + 8), dir);
                    let mods = (
                        self.xkb_state.serialize_mods(xkb::STATE_MODS_DEPRESSED),
                        self.xkb_state.serialize_mods(xkb::STATE_MODS_LATCHED),
                        self.xkb_state.serialize_mods(xkb::STATE_MODS_LOCKED),
                        self.xkb_state.serialize_layout(xkb::STATE_LAYOUT_EFFECTIVE),
                    );
                    if mods != self.mods {
                        self.mods = mods;
                        self.vk.modifiers(mods.0, mods.1, mods.2, mods.3);
                    }
                }
                Cmd::SetExtent { w, h } => extent = (w.max(1), h.max(1)),
                Cmd::Move { x, y } => {
                    self.ptr.motion_absolute(t, x.min(extent.0 - 1), y.min(extent.1 - 1), extent.0, extent.1);
                    self.ptr.frame();
                }
                Cmd::Button { code, pressed } => {
                    let st = if pressed { wl_pointer::ButtonState::Pressed } else { wl_pointer::ButtonState::Released };
                    self.ptr.button(t, code, st);
                    self.ptr.frame();
                }
                Cmd::Wheel { vertical, horizontal } => {
                    let mut sent = false;
                    for (i, (axis, notches)) in [(wl_pointer::Axis::VerticalScroll, vertical), (wl_pointer::Axis::HorizontalScroll, horizontal)].into_iter().enumerate() {
                        if notches == 0.0 {
                            continue;
                        }
                        // Wayland: positive = down/right; RDP: positive = up.
                        let signed = if i == 0 { -notches } else { notches } * self.scroll_scale;
                        // A trackpad sends many small amounts. Each one used to go out as a whole step, so a gentle swipe
                        // scrolled pages: collect them, and a step goes out when a whole notch has come together.
                        if self.scroll[i] * signed < 0.0 {
                            self.scroll[i] = 0.0; // the other way: start again
                        }
                        self.scroll[i] += signed;
                        let steps = self.scroll[i].trunc();
                        if steps != 0.0 {
                            self.scroll[i] -= steps;
                            if !sent {
                                self.ptr.axis_source(wl_pointer::AxisSource::Wheel);
                                sent = true;
                            }
                            self.ptr.axis(t, axis, steps * 15.0);
                            self.ptr.axis_discrete(t, axis, steps * 15.0, steps as i32);
                        }
                    }
                    if sent {
                        self.ptr.frame();
                    }
                }
            }
            if let Err(e) = self.conn.flush() {
                tracing::warn!("wayland flush failed: {e}");
            }
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(_: &mut Self, _: &wl_registry::WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(_: &mut Self, _: &wl_seat::WlSeat, _: wl_seat::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
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
impl Dispatch<ZwpVirtualKeyboardManagerV1, ()> for State {
    fn event(_: &mut Self, _: &ZwpVirtualKeyboardManagerV1, _: <ZwpVirtualKeyboardManagerV1 as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<ZwpVirtualKeyboardV1, ()> for State {
    fn event(_: &mut Self, _: &ZwpVirtualKeyboardV1, _: <ZwpVirtualKeyboardV1 as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<ZwlrVirtualPointerManagerV1, ()> for State {
    fn event(_: &mut Self, _: &ZwlrVirtualPointerManagerV1, _: <ZwlrVirtualPointerManagerV1 as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<ZwlrVirtualPointerV1, ()> for State {
    fn event(_: &mut Self, _: &ZwlrVirtualPointerV1, _: <ZwlrVirtualPointerV1 as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
