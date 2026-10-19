//! The session side of the clipboard (text only): the session process runs as the account and so can reach its own
//! Wayland clipboard through the `wl-clipboard` tools. The network daemon, which has no access to it, only relays text.
//! Client copy: the daemon sends the text, `wl-copy` puts it on the clipboard.
//! Session copy: `wl-paste --watch` notices a change and the text goes to the daemon.

use std::io::{BufRead as _, BufReader, Write as _};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::proto::{Up, MAX_CLIPBOARD};

/// Watches the session's clipboard while the client's channel is open. Stopped when dropped.
pub struct Clipboard {
    watcher: Option<Child>,
    up: mpsc::UnboundedSender<Up>,
    /// Text the client put on the clipboard: its echo must not go back to the client.
    last_set: Arc<Mutex<Option<String>>>,
}

impl Clipboard {
    pub fn new(up: mpsc::UnboundedSender<Up>) -> Self {
        Self { watcher: None, up, last_set: Arc::new(Mutex::new(None)) }
    }

    /// (Re)starts the watch. `wl-paste --watch` runs its command once at once and then on every change, so what is
    /// on the clipboard now is offered to the client too.
    pub fn watch(&mut self) {
        self.stop();
        let child = Command::new("wl-paste")
            .args(["--watch", "sh", "-c", "echo c"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();
        match child {
            Ok(mut c) => {
                let out = c.stdout.take().expect("piped stdout");
                self.watcher = Some(c);
                let (up, last_set) = (self.up.clone(), self.last_set.clone());
                std::thread::spawn(move || {
                    for line in BufReader::new(out).lines() {
                        if line.is_err() {
                            break;
                        }
                        let Some(text) = wayland_text() else { continue };
                        if last_set.lock().unwrap().as_deref() == Some(text.as_str()) {
                            continue;
                        }
                        if text.len() > MAX_CLIPBOARD {
                            info!(len = text.len(), "session clipboard text is too long to send");
                            continue;
                        }
                        info!(len = text.len(), "session clipboard changed");
                        if up.send(Up::Clipboard { data: text.into_bytes() }).is_err() {
                            break;
                        }
                    }
                });
            }
            Err(e) => warn!("clipboard from session to client is off (cannot run wl-paste --watch): {e}"),
        }
    }

    pub fn stop(&mut self) {
        if let Some(mut c) = self.watcher.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }

    /// Puts the client's text on the session clipboard.
    pub fn set(&self, data: Vec<u8>) {
        if data.len() > MAX_CLIPBOARD {
            warn!(len = data.len(), "client clipboard text is too long; ignored");
            return;
        }
        let Ok(text) = String::from_utf8(data) else {
            warn!("client clipboard text is not UTF-8; ignored");
            return;
        };
        if text.is_empty() {
            return;
        }
        *self.last_set.lock().unwrap() = Some(text.clone());
        std::thread::spawn(move || {
            // wl-copy forks into the background to keep serving the selection; its parent returns at once.
            match Command::new("wl-copy").stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
                Ok(mut c) => {
                    if let Some(mut stdin) = c.stdin.take() {
                        let _ = stdin.write_all(text.as_bytes());
                    }
                    let status = c.wait();
                    info!(?status, "wl-copy done");
                }
                Err(e) => warn!("cannot run wl-copy: {e}"),
            }
        });
    }
}

impl Drop for Clipboard {
    fn drop(&mut self) {
        self.stop();
    }
}

/// What the Wayland clipboard holds as text, if anything.
fn wayland_text() -> Option<String> {
    let out = Command::new("wl-paste").args(["--no-newline", "--type", "text"]).stderr(Stdio::null()).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?;
    (!text.is_empty()).then_some(text)
}
