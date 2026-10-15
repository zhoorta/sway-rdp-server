//! Clipboard (CLIPRDR), text only, through the `wl-clipboard` tools.
//! Client copy: the client announces formats, we ask for the text and `wl-copy` it.
//! Session copy: `wl-paste --watch` notices a change, we announce it, and `wl-paste` supplies the text when asked.

use std::io::{BufRead as _, BufReader, Write as _};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

use ironrdp_cliprdr::backend::{CliprdrBackend, CliprdrBackendFactory, ClipboardMessage};
use ironrdp_cliprdr::pdu::{
    ClipboardFormat, ClipboardFormatId, ClipboardGeneralCapabilityFlags, FileContentsRequest, FileContentsResponse,
    FormatDataRequest, FormatDataResponse, LockDataId,
};
use ironrdp_server::tokio::sync::mpsc::UnboundedSender;
use ironrdp_server::{CliprdrServerFactory, ServerEvent, ServerEventSender};
use tracing::{debug, info, warn};

#[derive(Default)]
struct Shared {
    sender: Mutex<Option<UnboundedSender<ServerEvent>>>,
    /// Text we put on the Wayland clipboard for the client: its echo must not go back to the client.
    last_set: Mutex<Option<String>>,
}

impl Shared {
    fn send(&self, msg: ClipboardMessage) {
        if let Some(tx) = self.sender.lock().unwrap().as_ref() {
            if tx.send(ServerEvent::Clipboard(msg)).is_err() {
                debug!("clipboard message dropped: server event channel closed");
            }
        }
    }
}

pub struct Factory {
    st: Arc<Shared>,
}

impl Factory {
    pub fn new() -> Self {
        Self { st: Arc::new(Shared::default()) }
    }
}

impl ServerEventSender for Factory {
    fn set_sender(&mut self, sender: UnboundedSender<ServerEvent>) {
        *self.st.sender.lock().unwrap() = Some(sender);
    }
}

impl CliprdrBackendFactory for Factory {
    fn build_cliprdr_backend(&self) -> Box<dyn CliprdrBackend> {
        Box::new(Backend { st: self.st.clone(), watcher: None, ready_at: None })
    }
}

impl CliprdrServerFactory for Factory {}

#[derive(Debug)]
struct Backend {
    st: Arc<Shared>,
    watcher: Option<Child>,
    ready_at: Option<std::time::Instant>,
}

/// How long after the channel is ready the client's clipboard is left alone. The Windows App announces
/// the Mac clipboard at connection time and then took up to a minute to answer the request for its text
/// (probably macOS gating a programmatic pasteboard read), which kept the connection at "Configuring
/// remote PC". Asking later keeps a slow answer from holding up the connection.
const SETTLE: std::time::Duration = std::time::Duration::from_secs(5);

impl std::fmt::Debug for Shared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Shared")
    }
}

ironrdp_core::impl_as_any!(Backend);

impl Drop for Backend {
    fn drop(&mut self) {
        if let Some(mut c) = self.watcher.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

fn text_format() -> ClipboardFormat {
    ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)
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

/// CF_UNICODETEXT: UTF-16LE, CRLF line ends, NUL terminated.
fn to_unicode_text(text: &str) -> Vec<u8> {
    let crlf = text.replace("\r\n", "\n").replace('\n', "\r\n");
    crlf.encode_utf16().chain(std::iter::once(0)).flat_map(u16::to_le_bytes).collect()
}

fn from_unicode_text(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).take_while(|&u| u != 0).collect();
    String::from_utf16_lossy(&units).replace("\r\n", "\n")
}

impl CliprdrBackend for Backend {
    fn temporary_directory(&self) -> &str {
        "/tmp"
    }

    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        // The Windows App on macOS did not answer requests for the text without this one.
        ClipboardGeneralCapabilityFlags::USE_LONG_FORMAT_NAMES
    }

    fn on_ready(&mut self) {
        info!("clipboard channel ready");
        self.ready_at = Some(std::time::Instant::now());
        // One line on stdout per clipboard change in the session.
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
                let st = self.st.clone();
                std::thread::spawn(move || {
                    for line in BufReader::new(out).lines() {
                        if line.is_err() {
                            break;
                        }
                        let Some(text) = wayland_text() else { continue };
                        if st.last_set.lock().unwrap().as_deref() == Some(text.as_str()) {
                            continue; // this is the client's own text coming back
                        }
                        info!(len = text.len(), "session clipboard changed; announcing to the client");
                        st.send(ClipboardMessage::SendInitiateCopy(vec![text_format()]));
                    }
                });
            }
            Err(e) => warn!("clipboard from session to client is off (cannot run wl-paste --watch): {e}"),
        }
    }

    fn on_request_format_list(&mut self) {
        // Start of the session: offer what is already on the clipboard.
        if wayland_text().is_some() {
            self.st.send(ClipboardMessage::SendInitiateCopy(vec![text_format()]));
        }
    }

    fn on_process_negotiated_capabilities(&mut self, _capabilities: ClipboardGeneralCapabilityFlags) {}

    fn on_remote_copy(&mut self, available_formats: &[ClipboardFormat]) {
        if !available_formats.iter().any(|f| f.id == ClipboardFormatId::CF_UNICODETEXT) {
            debug!(formats = ?available_formats.iter().map(|f| f.id.0).collect::<Vec<_>>(), "client copied something other than text; ignored");
            return;
        }
        let wait = self.ready_at.map_or(SETTLE, |t| SETTLE.saturating_sub(t.elapsed()));
        if wait.is_zero() {
            info!("client copied text; asking for it");
            self.st.send(ClipboardMessage::SendInitiatePaste(ClipboardFormatId::CF_UNICODETEXT));
        } else {
            info!(?wait, "client copied text during connection setup; asking for it a little later");
            let st = self.st.clone();
            std::thread::spawn(move || {
                std::thread::sleep(wait);
                st.send(ClipboardMessage::SendInitiatePaste(ClipboardFormatId::CF_UNICODETEXT));
            });
        }
    }

    fn on_format_data_request(&mut self, request: FormatDataRequest) {
        let st = self.st.clone();
        info!(format = request.format.0, "client asks for the session clipboard");
        std::thread::spawn(move || {
            let msg = match (request.format == ClipboardFormatId::CF_UNICODETEXT, wayland_text()) {
                (true, Some(text)) => {
                    info!(chars = text.chars().count(), "sending the session clipboard to the client");
                    FormatDataResponse::new_data(to_unicode_text(&text))
                }
                (is_text, text) => {
                    info!(is_text, has_text = text.is_some(), "cannot provide that format");
                    FormatDataResponse::new_error()
                }
            };
            st.send(ClipboardMessage::SendFormatData(msg));
        });
    }

    fn on_format_data_response(&mut self, response: FormatDataResponse<'_>) {
        if response.is_error() {
            info!("the client could not provide the clipboard text");
            return;
        }
        let text = from_unicode_text(response.data());
        info!(bytes = response.data().len(), chars = text.chars().count(), "client clipboard text received");
        if text.is_empty() {
            return;
        }
        *self.st.last_set.lock().unwrap() = Some(text.clone());
        std::thread::spawn(move || {
            // wl-copy forks into the background to keep serving the selection; its parent returns at once.
            let child = Command::new("wl-copy").stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
            match child {
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

    fn on_file_contents_request(&mut self, _request: FileContentsRequest) {}

    fn on_file_contents_response(&mut self, _response: FileContentsResponse<'_>) {}

    fn on_lock(&mut self, _data_id: LockDataId) {}

    fn on_unlock(&mut self, _data_id: LockDataId) {}
}
