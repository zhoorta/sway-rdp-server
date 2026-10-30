//! One remote session = one headless sway output of the client's size, with capture and input on it.
//! Created when a client connects, removed when it leaves.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use anyhow::{bail, Context as _, Result};
use tracing::{error, info, warn};

use crate::capture::{CaptureEnd, Capturer, Shared};
use crate::compositor;
use crate::input;

/// Counts sessions so a hook can derive a unique port or name from `WAYDESK_INDEX` (0..99, then it wraps).
static NEXT_INDEX: AtomicU32 = AtomicU32::new(0);

/// Sessions are created one at a time, across all session processes: setting one up moves focus between outputs,
/// which must not interleave. A lock file in the runtime directory serves as the lock (released when dropped).
struct CreateLock(#[allow(dead_code)] std::fs::File);

fn create_lock() -> Result<CreateLock> {
    use std::os::fd::AsRawFd;
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("waydesk-create.lock"))
        .context("creation lock file")?;
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
        bail!("cannot lock {}: {}", dir.display(), std::io::Error::last_os_error());
    }
    Ok(CreateLock(file))
}

/// What a new session is made of; set once from the command line.
#[derive(Clone)]
pub struct Options {
    /// Keyboard layout used when the client announces none.
    pub default_layout: String,
    /// Open a terminal on the new workspace.
    pub spawn_terminal: bool,
    /// Name of the workspace created on the session output.
    pub workspace: String,
    /// Program started when a session begins and stopped (SIGTERM to its process group) when it ends. It gets
    /// `WAYDESK_OUTPUT`, `WAYDESK_WORKSPACE` and `WAYDESK_INDEX` in its environment.
    pub hook: Option<PathBuf>,
}

pub struct Session {
    pub output: String,
    workspace: String,
    /// The session hook, if any (runs in its own process group).
    hook: Option<Child>,
    pub shared: Arc<Shared>,
    pub input: Sender<input::Cmd>,
    stop: Arc<AtomicBool>,
}

/// Starts the session hook in its own process group, so everything it starts can be stopped together.
fn start_hook(hook: &std::path::Path, output: &str, workspace: &str, index: u32) -> Option<Child> {
    use std::os::unix::process::CommandExt;
    match Command::new(hook)
        .env("WAYDESK_OUTPUT", output)
        .env("WAYDESK_WORKSPACE", workspace)
        .env("WAYDESK_INDEX", index.to_string())
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
    let shared = Arc::new(Shared::new(first.width, first.height, first.pixels));
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
                    pusher.push(f.pixels, f.damage);
                    // About 30 fps at most; damage that arrives meanwhile is merged.
                    std::thread::sleep(std::time::Duration::from_millis(30));
                }
                Ok(f) => {
                    warn!(w = f.width, h = f.height, "capture size changed without a resize: the session adopts the new size");
                    pusher.end(CaptureEnd::SizeChanged { width: f.width, height: f.height });
                    break;
                }
                Err(e) => {
                    if !flag.load(Ordering::Relaxed) {
                        error!("capture stopped: {e:#}");
                        pusher.end(CaptureEnd::Failed(format!("{e:#}")));
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
        let _creating = create_lock()?;
        let output = compositor::current().create_output()?;
        // Everything after this point has to remove the output again on failure.
        let index = NEXT_INDEX.fetch_add(1, Ordering::Relaxed) % 100;
        // The caller names the workspace (it is the daemon that keeps concurrent sessions apart).
        let workspace = opts.workspace.clone();
        match Self::attach(&output, index, &workspace, (w, h), layout, opts) {
            Ok(s) => Ok(s),
            Err(e) => {
                let _ = compositor::current().remove_output(&output);
                Err(e)
            }
        }
    }

    fn attach(output: &str, index: u32, workspace: &str, (w, h): (u16, u16), layout: &str, opts: &Options) -> Result<Self> {
        let comp = compositor::current();
        comp.init_output(output, (w, h))?;
        if !crate::capture::cursor_in_picture() && !crate::cursors::sway_reports_cursor() {
            // A headless sway has no hardware cursor, so it draws the cursor into the picture whatever screencopy is asked
            // for: with the client's own pointer that makes two. Give sway a theme whose cursors have nothing in them.
            let _ = comp.hide_cursor();
        }
        comp.show_workspace(output, workspace)?;
        if opts.spawn_terminal {
            comp.spawn_terminal()?;
        }

        let (shared, stop) = start_capture(output)?;
        let input = input::spawn(Some(output.to_owned()), (shared.width.into(), shared.height.into()), layout.to_owned())
            .context("input setup")?;
        let hook = opts.hook.as_deref().and_then(|h| start_hook(h, output, workspace, index));
        Ok(Self { output: output.to_owned(), workspace: workspace.to_owned(), hook, shared, input, stop })
    }

    /// Change the output size and capture it again at that size. The pointer extent follows.
    /// The caller has to tell the RDP client (`DisplayUpdate::Resize`) and build a new encoder.
    pub fn resize(&mut self, size: (u16, u16)) -> Result<()> {
        let (w, h) = clamp_size(size.0, size.1);
        // The old capture wakes up on the mode change, sees the flag and ends.
        self.stop.store(true, Ordering::Relaxed);
        compositor::current().set_mode(&self.output, (w, h))?;
        let (shared, stop) = start_capture(&self.output).context("capture after resize")?;
        let _ = self.input.send(input::Cmd::SetExtent { w: shared.width.into(), h: shared.height.into() });
        info!(output = %self.output, width = shared.width, height = shared.height, "session resized");
        self.shared = shared;
        self.stop = stop;
        Ok(())
    }

    /// Starts the capture of the output again, at the size the output has now (after the capture stopped by itself: the compositor changed the size, or the
    /// capture failed). The caller tells the client when the size is not the one it has.
    pub fn restart_capture(&mut self) -> Result<()> {
        self.stop.store(true, Ordering::Relaxed);
        let (shared, stop) = start_capture(&self.output).context("capture restart")?;
        let _ = self.input.send(input::Cmd::SetExtent { w: shared.width.into(), h: shared.height.into() });
        info!(output = %self.output, width = shared.width, height = shared.height, "capture restarted");
        self.shared = shared;
        self.stop = stop;
        Ok(())
    }

    /// Match sway's scale to the client's (200% in the display layout means scale 2).
    pub fn set_scale(&self, percent: u32) {
        let scale = (f64::from(percent.clamp(100, 400)) / 100.0).to_string();
        if let Err(e) = compositor::current().set_scale(&self.output, &scale) {
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
        let comp = compositor::current();
        // Close what ran in this session so it does not pile up on the console output.
        comp.close_windows(&self.workspace, &self.output);
        comp.release_focus(&self.output);
        if let Err(e) = comp.remove_output(&self.output) {
            warn!("cannot remove output {}: {e:#}", self.output);
        }
        info!(output = %self.output, workspace = %self.workspace, "session removed");
    }
}
