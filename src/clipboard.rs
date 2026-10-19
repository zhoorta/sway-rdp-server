//! Clipboard (CLIPRDR), text only. The RDP channel lives here, in the network daemon; the Wayland clipboard belongs to
//! the account and is reached through the session process (`sessionclip.rs`), which this side talks to over the link.
//! Client copy: the client announces formats, we ask for the text and send it to the session.
//! Session copy: the session sends its text when the clipboard changes, we announce it and answer the client's request.

use std::sync::{Arc, Mutex};

use ironrdp_cliprdr::backend::{CliprdrBackend, CliprdrBackendFactory, ClipboardMessage};
use ironrdp_cliprdr::pdu::{
    ClipboardFormat, ClipboardFormatId, ClipboardGeneralCapabilityFlags, FileContentsRequest, FileContentsResponse,
    FormatDataRequest, FormatDataResponse, LockDataId,
};
use ironrdp_server::tokio::sync::mpsc::UnboundedSender;
use ironrdp_server::{CliprdrServerFactory, ServerEvent, ServerEventSender};
use tracing::{debug, info};

use crate::proto::{Down, MAX_CLIPBOARD};
use crate::Slot;

struct Shared {
    sender: Mutex<Option<UnboundedSender<ServerEvent>>>,
    /// The connection's session (its link appears once the session is up).
    slot: Arc<Slot>,
    /// The text the session's clipboard holds, as the session last reported it.
    session_text: Mutex<Option<String>>,
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
    pub fn new(slot: Arc<Slot>) -> Self {
        Self { st: Arc::new(Shared { sender: Mutex::new(None), slot, session_text: Mutex::new(None) }) }
    }
}

impl ServerEventSender for Factory {
    fn set_sender(&mut self, sender: UnboundedSender<ServerEvent>) {
        *self.st.sender.lock().unwrap() = Some(sender);
    }
}

impl CliprdrBackendFactory for Factory {
    fn build_cliprdr_backend(&self) -> Box<dyn CliprdrBackend> {
        Box::new(Backend { st: self.st.clone(), ready_at: None })
    }
}

impl CliprdrServerFactory for Factory {}

#[derive(Debug)]
struct Backend {
    st: Arc<Shared>,
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

fn text_format() -> ClipboardFormat {
    ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)
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
        let Some(link) = self.st.slot.link() else {
            info!("clipboard: no session yet; the clipboard stays off for this connection");
            return;
        };
        let st = self.st.clone();
        link.set_clipboard_sink(Box::new(move |data| {
            let Ok(text) = String::from_utf8(data) else { return };
            info!(len = text.len(), "session clipboard changed; announcing to the client");
            *st.session_text.lock().unwrap() = Some(text);
            st.send(ClipboardMessage::SendInitiateCopy(vec![text_format()]));
        }));
        // The session offers what is on its clipboard now, then every change.
        link.send(Down::ClipboardOn);
    }

    fn on_request_format_list(&mut self) {
        // Start of the session: offer what is already on the clipboard.
        if self.st.session_text.lock().unwrap().is_some() {
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
        info!(format = request.format.0, "client asks for the session clipboard");
        let text = self.st.session_text.lock().unwrap().clone();
        let msg = match (request.format == ClipboardFormatId::CF_UNICODETEXT, text) {
            (true, Some(text)) => {
                info!(chars = text.chars().count(), "sending the session clipboard to the client");
                FormatDataResponse::new_data(to_unicode_text(&text))
            }
            (is_text, text) => {
                info!(is_text, has_text = text.is_some(), "cannot provide that format");
                FormatDataResponse::new_error()
            }
        };
        self.st.send(ClipboardMessage::SendFormatData(msg));
    }

    fn on_format_data_response(&mut self, response: FormatDataResponse<'_>) {
        if response.is_error() {
            info!("the client could not provide the clipboard text");
            return;
        }
        let text = from_unicode_text(response.data());
        info!(bytes = response.data().len(), chars = text.chars().count(), "client clipboard text received");
        if text.is_empty() || text.len() > MAX_CLIPBOARD {
            return;
        }
        match self.st.slot.link() {
            Some(link) => link.send(Down::Clipboard { data: text.into_bytes() }),
            None => debug!("client clipboard text dropped: no session"),
        }
    }

    fn on_file_contents_request(&mut self, _request: FileContentsRequest) {}

    fn on_file_contents_response(&mut self, _response: FileContentsResponse<'_>) {}

    fn on_lock(&mut self, _data_id: LockDataId) {}

    fn on_unlock(&mut self, _data_id: LockDataId) {}
}
