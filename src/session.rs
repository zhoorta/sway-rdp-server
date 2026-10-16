//! One remote session = one headless sway output of the client's size, with capture and input on it.
//! Created when a client connects, removed when it leaves.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use anyhow::{bail, Context as _, Result};
use tracing::{error, info, warn};

use crate::capture::{Capturer, Shared};
use crate::input;

/// Counts sessions so a hook can derive a unique port or name from `SWAY_RDP_INDEX` (0..99, then it wraps).
static NEXT_INDEX: AtomicU32 = AtomicU32::new(0);

/// Sessions are created one at a time: setting one up moves focus between outputs, which must not interleave.
static CREATE_LOCK: Mutex<()> = Mutex::new(());

/// Numbers of the sessions that exist (1, 2, ...). The lowest free one names the session's workspace, so two
/// concurrent sessions never share (and steal) a workspace.
static USED_SLOTS: Mutex<BTreeSet<u32>> = Mutex::new(BTreeSet::new());

fn take_slot() -> u32 {
    let mut used = USED_SLOTS.lock().unwrap();
    let n = (1..).find(|n| !used.contains(n)).expect("slot numbers are not exhausted");
    used.insert(n);
    n
}

fn release_slot(n: u32) {
    USED_SLOTS.lock().unwrap().remove(&n);
}

/// What a new session is made of; set once from the command line.
pub struct Options {
    /// Keyboard layout used when the client announces none.
    pub default_layout: String,
    /// Open a terminal on the new workspace.
    pub spawn_terminal: bool,
    /// Name of the workspace created on the session output.
    pub workspace: String,
    /// Program started when a session begins and stopped (SIGTERM to its process group) when it ends. It gets
    /// `SWAY_RDP_OUTPUT`, `SWAY_RDP_WORKSPACE` and `SWAY_RDP_INDEX` in its environment.
    pub hook: Option<PathBuf>,
}

pub struct Session {
    pub output: String,
    workspace: String,
    slot: u32,
    /// The session hook, if any (runs in its own process group).
    hook: Option<Child>,
    pub shared: Arc<Shared>,
    pub input: Sender<input::Cmd>,
    stop: Arc<AtomicBool>,
}

fn swaymsg(args: &[&str]) -> Result<String> {
    // Commands go after `--`, or swaymsg parses options such as `--custom` itself.
    let mut cmd = Command::new("swaymsg");
    if args.first().is_some_and(|a| !a.starts_with('-')) {
        cmd.arg("--");
    }
    let out = cmd.args(args).output().context("cannot run swaymsg")?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    if !out.status.success() || text.contains("\"success\": false") || text.contains("\"success\":false") {
        bail!("swaymsg {args:?} failed: {text} {}", String::from_utf8_lossy(&out.stderr));
    }
    Ok(text)
}

fn output_names() -> Result<Vec<String>> {
    let v: serde_json::Value = serde_json::from_str(&swaymsg(&["-t", "get_outputs", "-r"])?)?;
    Ok(v.as_array()
        .context("outputs not an array")?
        .iter()
        .filter_map(|o| o["name"].as_str().map(str::to_owned))
        .collect())
}

/// Starts the session hook in its own process group, so everything it starts can be stopped together.
fn start_hook(hook: &std::path::Path, output: &str, workspace: &str, index: u32) -> Option<Child> {
    use std::os::unix::process::CommandExt;
    match Command::new(hook)
        .env("SWAY_RDP_OUTPUT", output)
        .env("SWAY_RDP_WORKSPACE", workspace)
        .env("SWAY_RDP_INDEX", index.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .process_group(0)
        .spawn()
    {
        Ok(c) => {
            info!(%output, hook = %hook.display(), "session hook started");
            Some(c)
        }
        Err(e) => {
            error!("cannot start session hook {}: {e}", hook.display());
            None
        }
    }
}

/// SIGTERM to the hook's process group, SIGKILL if it is still there after two seconds.
fn stop_hook(mut child: Child) {
    let pgid = child.id() as i32;
    unsafe { libc::kill(-pgid, libc::SIGTERM) };
    for _ in 0..40 {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    unsafe { libc::kill(-pgid, libc::SIGKILL) };
    let _ = child.wait();
}

/// Capture `output` on a thread of its own. The thread ends when `stop` is set, when the output goes away,
/// or when the output changes size (a resize makes a new capture).
fn start_capture(output: &str) -> Result<(Arc<Shared>, Arc<AtomicBool>)> {
    let mut cap = Capturer::new(Some(output)).context("capture setup")?;
    let first = cap.next_frame().context("first capture")?;
    info!(%output, width = first.width, height = first.height, "capturing");
    let shared = Arc::new(Shared::new(first.width, first.height, first.data));
    let stop = Arc::new(AtomicBool::new(false));
    let (pusher, flag) = (shared.clone(), stop.clone());
    std::thread::spawn(move || {
        while !flag.load(Ordering::Relaxed) {
            match cap.next_frame() {
                Ok(f) if flag.load(Ordering::Relaxed) => {
                    let _ = f; // stopped while waiting for this frame
                    break;
                }
                Ok(f) if f.width == pusher.width && f.height == pusher.height => {
                    pusher.push(f.data, f.damage);
                    // About 30 fps at most; damage that arrives meanwhile is merged.
                    std::thread::sleep(std::time::Duration::from_millis(30));
                }
                Ok(f) => {
                    warn!(w = f.width, h = f.height, "capture size changed without a resize; stopping this capture");
                    break;
                }
                Err(e) => {
                    if !flag.load(Ordering::Relaxed) {
                        error!("capture stopped: {e:#}");
                    }
                    break;
                }
            }
        }
    });
    Ok((shared, stop))
}

/// Client sizes the encoder and the virtual GPU can take: even, within H.264 level 5.1.
pub fn clamp_size(w: u16, h: u16) -> (u16, u16) {
    (w.clamp(640, 4096) & !1, h.clamp(480, 2304) & !1)
}

/// xkb layout for a Windows keyboard layout id (low word of the locale).
pub fn xkb_layout_for_klid(klid: u32) -> &'static str {
    match klid & 0xffff {
        0x0409 | 0x1009 => "us",
        0x0809 => "gb",
        0x0816 => "pt",
        0x0416 => "br",
        0x040c | 0x080c => "fr",
        0x0407 | 0x0807 => "de",
        0x040a | 0x0c0a => "es",
        0x0410 => "it",
        0x0413 | 0x0813 => "us",
        0x0406 => "dk",
        0x041d => "se",
        0x0414 | 0x0814 => "no",
        0x040b => "fi",
        _ => "us",
    }
}

impl Session {
    pub fn create(size: (u16, u16), layout: &str, opts: &Options) -> Result<Self> {
        let (w, h) = clamp_size(size.0, size.1);
        let _creating = CREATE_LOCK.lock().unwrap();
        let before = output_names()?;
        swaymsg(&["create_output"])?;
        let output = output_names()?
            .into_iter()
            .find(|n| !before.contains(n))
            .context("sway created no new output")?;
        // Everything after this point has to remove the output again on failure.
        let index = NEXT_INDEX.fetch_add(1, Ordering::Relaxed) % 100;
        let slot = take_slot();
        // The first session keeps the configured name; later ones get a number.
        let workspace = if slot == 1 { opts.workspace.clone() } else { format!("{} {slot}", opts.workspace) };
        match Self::attach(&output, index, slot, &workspace, (w, h), layout, opts) {
            Ok(s) => Ok(s),
            Err(e) => {
                release_slot(slot);
                let _ = swaymsg(&["output", &output, "unplug"]);
                Err(e)
            }
        }
    }

    fn attach(output: &str, index: u32, slot: u32, workspace: &str, (w, h): (u16, u16), layout: &str, opts: &Options) -> Result<Self> {
        swaymsg(&["output", output, "mode", "--custom", &format!("{w}x{h}@60Hz"), "scale", "1"])?;
        // A workspace is created on the focused output, so focus the session output first.
        swaymsg(&["focus", "output", output])?;
        swaymsg(&["workspace", workspace])?;
        // If a workspace of that name already existed elsewhere, `workspace` jumped to it: bring it here.
        let _ = swaymsg(&["move", "workspace", "to", "output", output]);
        swaymsg(&["focus", "output", output])?;
        if opts.spawn_terminal {
            swaymsg(&["exec", "foot -T sway-rdp-server-session"])?;
        }

        let (shared, stop) = start_capture(output)?;
        let input = input::spawn(Some(output.to_owned()), (shared.width.into(), shared.height.into()), layout.to_owned())
            .context("input setup")?;
        let hook = opts.hook.as_deref().and_then(|h| start_hook(h, output, workspace, index));
        Ok(Self { output: output.to_owned(), workspace: workspace.to_owned(), slot, hook, shared, input, stop })
    }

    /// Change the output size and capture it again at that size. The pointer extent follows.
    /// The caller has to tell the RDP client (`DisplayUpdate::Resize`) and build a new encoder.
    pub fn resize(&mut self, size: (u16, u16)) -> Result<()> {
        let (w, h) = clamp_size(size.0, size.1);
        // The old capture wakes up on the mode change, sees the flag and ends.
        self.stop.store(true, Ordering::Relaxed);
        swaymsg(&["output", &self.output, "mode", "--custom", &format!("{w}x{h}@60Hz")])?;
        let (shared, stop) = start_capture(&self.output).context("capture after resize")?;
        let _ = self.input.send(input::Cmd::SetExtent { w: shared.width.into(), h: shared.height.into() });
        info!(output = %self.output, width = shared.width, height = shared.height, "session resized");
        self.shared = shared;
        self.stop = stop;
        Ok(())
    }

    /// Match sway's scale to the client's (200% in the display layout means scale 2).
    pub fn set_scale(&self, percent: u32) {
        let scale = (f64::from(percent.clamp(100, 400)) / 100.0).to_string();
        if let Err(e) = swaymsg(&["output", &self.output, "scale", &scale]) {
            warn!("cannot set scale: {e:#}");
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.hook.take() {
            stop_hook(h);
        }
        // Close what ran in this session so it does not pile up on the console output.
        let _ = swaymsg(&[&format!("[workspace=\"{}\"]", self.workspace), "kill"]);
        // Windows minimized from this session sit in the scratchpad, marked with the output they came from.
        let _ = swaymsg(&[&format!("[con_mark=\"^from:{}$\"]", self.output), "kill"]);
        // Hand the focus back to another output first: a workspace that is focused when its output goes
        // away is moved to the remaining output and kept there, empty, under the same name.
        if let Some(other) = output_names().ok().and_then(|v| v.into_iter().find(|n| *n != self.output)) {
            let _ = swaymsg(&["focus", "output", &other]);
        }
        if let Err(e) = swaymsg(&["output", &self.output, "unplug"]) {
            warn!("cannot remove output {}: {e:#}", self.output);
        }
        release_slot(self.slot);
        info!(output = %self.output, workspace = %self.workspace, "session removed");
    }
}
