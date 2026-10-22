//! Stream protocol between the network daemon and a user's session process (milestone 2 of docs/MULTIUSER-DESIGN.md).
//!
//! A message on the wire is `u32 length (little endian)`, `u8 tag`, payload; `length` counts the tag and the payload.
//! Integers are little endian, byte strings are `u32 length` + bytes. Both ends refuse a length above `MAX_MESSAGE`
//! and any message that does not decode exactly, so a broken or hostile peer ends the connection instead of
//! making the other side allocate or guess.
//!
//! Not used by the server yet: this is the contract the split will be built against.
#![allow(dead_code)]

use std::io::{self, Read, Write};

/// Protocol version, sent in `Down::Hello`.
pub const VERSION: u16 = 1;

/// Largest message accepted (a full 4096x2304 BGRA bitmap is 36 MiB).
pub const MAX_MESSAGE: u32 = 48 * 1024 * 1024;

/// Longest string in a helper message (account names, passwords, addresses).
pub const MAX_STRING: u32 = 1024;

#[derive(Debug, PartialEq)]
pub enum ProtoError {
    /// The message ended before all of its fields were read.
    Truncated,
    /// Bytes were left over after the last field.
    Trailing,
    UnknownTag(u8),
    /// A length above `MAX_MESSAGE`.
    TooLarge(u32),
    /// A string that is not valid UTF-8.
    BadString,
    Io(String),
}

impl std::fmt::Display for ProtoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProtoError::Truncated => write!(f, "message too short"),
            ProtoError::Trailing => write!(f, "unexpected bytes after message"),
            ProtoError::UnknownTag(t) => write!(f, "unknown message tag {t}"),
            ProtoError::TooLarge(n) => write!(f, "message of {n} bytes is too large"),
            ProtoError::BadString => write!(f, "string is not valid UTF-8"),
            ProtoError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ProtoError {}

impl From<io::Error> for ProtoError {
    fn from(e: io::Error) -> Self {
        ProtoError::Io(e.to_string())
    }
}

/// What the picture data in an `Up::Frame` is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FrameKind {
    /// H.264 (Annex B), for the graphics pipeline.
    H264 = 0,
    /// Raw BGRA of the rectangle, for the bitmap fallback.
    Bgra = 1,
}

/// Longest clipboard text either side relays (UTF-8 bytes).
pub const MAX_CLIPBOARD: usize = 1024 * 1024;

/// From the network daemon to the session.
#[derive(Debug, PartialEq)]
pub enum Down {
    Hello { version: u16, width: u16, height: u16, scale_percent: u16, klid: u32 },
    Resize { width: u16, height: u16, scale_percent: u16 },
    Key { code: u32, pressed: bool },
    PointerMove { x: u32, y: u32 },
    Button { code: u32, pressed: bool },
    Wheel { vertical: f64, horizontal: f64 },
    /// The daemon can take this many more frames (flow control; replaces the EGFX backpressure check).
    Credit { frames: u16 },
    /// Send the whole picture again.
    RequestFull,
    /// Prefer bitmaps: the client has no H.264.
    UseBitmaps,
    /// The client's clipboard text (UTF-8) for the session's clipboard.
    Clipboard { data: Vec<u8> },
    /// The client has a clipboard channel: start watching the session's clipboard.
    ClipboardOn,
    /// The client has a sound channel: record the session's speaker and send it as `Up::Audio`.
    AudioOn,
    /// The sound channel is closed: stop recording.
    AudioOff,
    Bye,
}

/// From the session to the network daemon.
#[derive(Debug, PartialEq)]
pub enum Up {
    /// Session is running; the picture is this size.
    Ready { width: u16, height: u16 },
    /// A resize finished; later frames have this size.
    Resized { width: u16, height: u16 },
    Frame { x: u16, y: u16, w: u16, h: u16, kind: FrameKind, data: Vec<u8> },
    /// 20 ms chunk of 16-bit stereo PCM at 44.1 kHz.
    Audio { pcm: Vec<u8> },
    Clipboard { data: Vec<u8> },
    Bye,
}

// ---- encoding helpers ----

#[derive(Default)]
struct Out(Vec<u8>);

impl Out {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn f64(&mut self, v: f64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn bool(&mut self, v: bool) {
        self.u8(v as u8);
    }
    fn bytes(&mut self, v: &[u8]) {
        self.u32(v.len() as u32);
        self.0.extend_from_slice(v);
    }
    fn str(&mut self, v: &str) {
        self.bytes(v.as_bytes());
    }
}

struct In<'a>(&'a [u8]);

impl<'a> In<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], ProtoError> {
        if self.0.len() < n {
            return Err(ProtoError::Truncated);
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Ok(head)
    }
    fn u8(&mut self) -> Result<u8, ProtoError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, ProtoError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, ProtoError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn f64(&mut self) -> Result<f64, ProtoError> {
        Ok(f64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn bool(&mut self) -> Result<bool, ProtoError> {
        Ok(self.u8()? != 0)
    }
    fn bytes(&mut self) -> Result<Vec<u8>, ProtoError> {
        let n = self.u32()?;
        if n > MAX_MESSAGE {
            return Err(ProtoError::TooLarge(n));
        }
        Ok(self.take(n as usize)?.to_vec())
    }
    /// A UTF-8 string of at most `MAX_STRING` bytes.
    fn str(&mut self) -> Result<String, ProtoError> {
        let n = self.u32()?;
        if n > MAX_STRING {
            return Err(ProtoError::TooLarge(n));
        }
        String::from_utf8(self.take(n as usize)?.to_vec()).map_err(|_| ProtoError::BadString)
    }
    fn finish(self) -> Result<(), ProtoError> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(ProtoError::Trailing)
        }
    }
}

impl Down {
    /// Tag byte followed by the payload (without the length prefix).
    pub fn encode(&self) -> Vec<u8> {
        let mut o = Out::default();
        match self {
            Down::Hello { version, width, height, scale_percent, klid } => {
                o.u8(1);
                o.u16(*version);
                o.u16(*width);
                o.u16(*height);
                o.u16(*scale_percent);
                o.u32(*klid);
            }
            Down::Resize { width, height, scale_percent } => {
                o.u8(2);
                o.u16(*width);
                o.u16(*height);
                o.u16(*scale_percent);
            }
            Down::Key { code, pressed } => {
                o.u8(3);
                o.u32(*code);
                o.bool(*pressed);
            }
            Down::PointerMove { x, y } => {
                o.u8(4);
                o.u32(*x);
                o.u32(*y);
            }
            Down::Button { code, pressed } => {
                o.u8(5);
                o.u32(*code);
                o.bool(*pressed);
            }
            Down::Wheel { vertical, horizontal } => {
                o.u8(6);
                o.f64(*vertical);
                o.f64(*horizontal);
            }
            Down::Credit { frames } => {
                o.u8(7);
                o.u16(*frames);
            }
            Down::RequestFull => o.u8(8),
            Down::UseBitmaps => o.u8(9),
            Down::ClipboardOn => o.u8(11),
            Down::AudioOn => o.u8(12),
            Down::AudioOff => o.u8(13),
            Down::Clipboard { data } => {
                o.u8(10);
                o.bytes(data);
            }
            Down::Bye => o.u8(255),
        }
        o.0
    }

    pub fn decode(buf: &[u8]) -> Result<Self, ProtoError> {
        let mut i = In(buf);
        let msg = match i.u8()? {
            1 => Down::Hello { version: i.u16()?, width: i.u16()?, height: i.u16()?, scale_percent: i.u16()?, klid: i.u32()? },
            2 => Down::Resize { width: i.u16()?, height: i.u16()?, scale_percent: i.u16()? },
            3 => Down::Key { code: i.u32()?, pressed: i.bool()? },
            4 => Down::PointerMove { x: i.u32()?, y: i.u32()? },
            5 => Down::Button { code: i.u32()?, pressed: i.bool()? },
            6 => Down::Wheel { vertical: i.f64()?, horizontal: i.f64()? },
            7 => Down::Credit { frames: i.u16()? },
            8 => Down::RequestFull,
            9 => Down::UseBitmaps,
            10 => Down::Clipboard { data: i.bytes()? },
            11 => Down::ClipboardOn,
            12 => Down::AudioOn,
            13 => Down::AudioOff,
            255 => Down::Bye,
            t => return Err(ProtoError::UnknownTag(t)),
        };
        i.finish()?;
        Ok(msg)
    }
}

impl Up {
    pub fn encode(&self) -> Vec<u8> {
        let mut o = Out::default();
        match self {
            Up::Ready { width, height } => {
                o.u8(1);
                o.u16(*width);
                o.u16(*height);
            }
            Up::Resized { width, height } => {
                o.u8(2);
                o.u16(*width);
                o.u16(*height);
            }
            Up::Frame { x, y, w, h, kind, data } => {
                o.u8(3);
                o.u16(*x);
                o.u16(*y);
                o.u16(*w);
                o.u16(*h);
                o.u8(*kind as u8);
                o.bytes(data);
            }
            Up::Audio { pcm } => {
                o.u8(4);
                o.bytes(pcm);
            }
            Up::Clipboard { data } => {
                o.u8(5);
                o.bytes(data);
            }
            Up::Bye => o.u8(255),
        }
        o.0
    }

    pub fn decode(buf: &[u8]) -> Result<Self, ProtoError> {
        let mut i = In(buf);
        let msg = match i.u8()? {
            1 => Up::Ready { width: i.u16()?, height: i.u16()? },
            2 => Up::Resized { width: i.u16()?, height: i.u16()? },
            3 => {
                let (x, y, w, h) = (i.u16()?, i.u16()?, i.u16()?, i.u16()?);
                let kind = match i.u8()? {
                    0 => FrameKind::H264,
                    1 => FrameKind::Bgra,
                    t => return Err(ProtoError::UnknownTag(t)),
                };
                Up::Frame { x, y, w, h, kind, data: i.bytes()? }
            }
            4 => Up::Audio { pcm: i.bytes()? },
            5 => Up::Clipboard { data: i.bytes()? },
            255 => Up::Bye,
            t => return Err(ProtoError::UnknownTag(t)),
        };
        i.finish()?;
        Ok(msg)
    }
}

// ---- daemon <-> root helper ----
//
// The network daemon runs without privileges and asks the helper for the two things only root can do: check a
// password (any account) and start a session process as that account. One helper connection serves one client
// connection: the helper remembers who authenticated on it and starts sessions only for that account, so the daemon
// cannot ask for a session of an account whose password it did not present.

/// From the daemon to the helper.
#[derive(Debug, PartialEq)]
pub enum HelperReq {
    Authenticate { user: String, password: String, source: String },
    /// Start the session of the account that authenticated on this connection. The answer carries the daemon's end of
    /// the session socket as a file descriptor (see `fdpass`). Nothing else is asked for: what runs, with which
    /// arguments, is the helper's own configuration, never the daemon's.
    StartSession,
    Bye,
}

/// From the helper to the daemon.
#[derive(Debug, PartialEq)]
pub enum HelperResp {
    AuthOk,
    AuthFail,
    /// The session is running; a file descriptor follows (see `fdpass`).
    Started,
    Error { message: String },
    /// The machine will not take a new session now; `reason` is one of the `REFUSED_*` values, `message` is for the log.
    Refused { reason: u8, message: String },
}

/// Why a new session was refused (`HelperResp::Refused`).
pub const REFUSED_MEMORY: u8 = 1;
pub const REFUSED_USERS: u8 = 2;

impl HelperReq {
    pub fn encode(&self) -> Vec<u8> {
        let mut o = Out::default();
        match self {
            HelperReq::Authenticate { user, password, source } => {
                o.u8(1);
                o.str(user);
                o.str(password);
                o.str(source);
            }
            HelperReq::StartSession => o.u8(2),
            HelperReq::Bye => o.u8(255),
        }
        o.0
    }

    pub fn decode(buf: &[u8]) -> Result<Self, ProtoError> {
        let mut i = In(buf);
        let msg = match i.u8()? {
            1 => HelperReq::Authenticate { user: i.str()?, password: i.str()?, source: i.str()? },
            2 => HelperReq::StartSession,
            255 => HelperReq::Bye,
            t => return Err(ProtoError::UnknownTag(t)),
        };
        i.finish()?;
        Ok(msg)
    }
}

impl HelperResp {
    pub fn encode(&self) -> Vec<u8> {
        let mut o = Out::default();
        match self {
            HelperResp::AuthOk => o.u8(1),
            HelperResp::AuthFail => o.u8(2),
            HelperResp::Started => o.u8(3),
            HelperResp::Error { message } => {
                o.u8(4);
                // Never longer than a string may be.
                o.str(&message.chars().take(200).collect::<String>());
            }
            HelperResp::Refused { reason, message } => {
                o.u8(5);
                o.u8(*reason);
                o.str(&message.chars().take(200).collect::<String>());
            }
        }
        o.0
    }

    pub fn decode(buf: &[u8]) -> Result<Self, ProtoError> {
        let mut i = In(buf);
        let msg = match i.u8()? {
            1 => HelperResp::AuthOk,
            2 => HelperResp::AuthFail,
            3 => HelperResp::Started,
            4 => HelperResp::Error { message: i.str()? },
            5 => HelperResp::Refused { reason: i.u8()?, message: i.str()? },
            t => return Err(ProtoError::UnknownTag(t)),
        };
        i.finish()?;
        Ok(msg)
    }
}

// ---- framing ----

/// Writes one framed message (`body` is what `encode` returned).
pub fn write_frame<W: Write>(w: &mut W, body: &[u8]) -> Result<(), ProtoError> {
    let len = u32::try_from(body.len()).map_err(|_| ProtoError::TooLarge(u32::MAX))?;
    if len > MAX_MESSAGE {
        return Err(ProtoError::TooLarge(len));
    }
    w.write_all(&len.to_le_bytes())?;
    w.write_all(body)?;
    Ok(())
}

/// Reads one framed message and returns its body (tag and payload). `Ok(None)` is a clean end of stream
/// before the first byte of a message.
pub fn read_frame<R: Read>(r: &mut R) -> Result<Option<Vec<u8>>, ProtoError> {
    let mut len = [0u8; 4];
    match r.read(&mut len[..1])? {
        0 => return Ok(None),
        _ => r.read_exact(&mut len[1..])?,
    }
    let len = u32::from_le_bytes(len);
    if len > MAX_MESSAGE {
        return Err(ProtoError::TooLarge(len));
    }
    if len == 0 {
        return Err(ProtoError::Truncated);
    }
    let mut body = vec![0u8; len as usize];
    r.read_exact(&mut body)?;
    Ok(Some(body))
}

/// `write_frame` for tokio streams.
pub async fn write_frame_async<W: tokio::io::AsyncWrite + Unpin>(w: &mut W, body: &[u8]) -> Result<(), ProtoError> {
    use tokio::io::AsyncWriteExt;
    let len = u32::try_from(body.len()).map_err(|_| ProtoError::TooLarge(u32::MAX))?;
    if len > MAX_MESSAGE {
        return Err(ProtoError::TooLarge(len));
    }
    w.write_all(&len.to_le_bytes()).await?;
    w.write_all(body).await?;
    w.flush().await?;
    Ok(())
}

/// `read_frame` for tokio streams. Not cancel safe: read it in a task of its own, never inside `select!`.
pub async fn read_frame_async<R: tokio::io::AsyncRead + Unpin>(r: &mut R) -> Result<Option<Vec<u8>>, ProtoError> {
    use tokio::io::AsyncReadExt;
    let mut len = [0u8; 4];
    if r.read(&mut len[..1]).await? == 0 {
        return Ok(None);
    }
    r.read_exact(&mut len[1..]).await?;
    let len = u32::from_le_bytes(len);
    if len > MAX_MESSAGE {
        return Err(ProtoError::TooLarge(len));
    }
    if len == 0 {
        return Err(ProtoError::Truncated);
    }
    let mut body = vec![0u8; len as usize];
    r.read_exact(&mut body).await?;
    Ok(Some(body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn async_framing_matches_sync_framing() {
        let (mut a, mut b) = tokio::io::duplex(1024);
        let msg = Up::Frame { x: 1, y: 2, w: 3, h: 4, kind: FrameKind::H264, data: vec![9; 5000] };
        let body = msg.encode();
        let writer = tokio::spawn(async move {
            write_frame_async(&mut a, &body).await.unwrap();
        });
        let got = read_frame_async(&mut b).await.unwrap().unwrap();
        writer.await.unwrap();
        assert_eq!(Up::decode(&got).unwrap(), msg);
        assert_eq!(read_frame_async(&mut b).await.unwrap(), None);
    }

    fn down_samples() -> Vec<Down> {
        vec![
            Down::Hello { version: VERSION, width: 1680, height: 1050, scale_percent: 200, klid: 0x0816 },
            Down::Resize { width: 640, height: 480, scale_percent: 100 },
            Down::Key { code: 30, pressed: true },
            Down::PointerMove { x: 1, y: 2 },
            Down::Button { code: 0x110, pressed: false },
            Down::Wheel { vertical: -1.5, horizontal: 0.25 },
            Down::Credit { frames: 4 },
            Down::RequestFull,
            Down::UseBitmaps,
            Down::Clipboard { data: b"hello".to_vec() },
            Down::ClipboardOn,
            Down::AudioOn,
            Down::AudioOff,
            Down::Bye,
        ]
    }

    fn up_samples() -> Vec<Up> {
        vec![
            Up::Ready { width: 1280, height: 800 },
            Up::Resized { width: 1920, height: 1080 },
            Up::Frame { x: 0, y: 0, w: 16, h: 16, kind: FrameKind::H264, data: vec![0, 0, 0, 1, 0x67] },
            Up::Frame { x: 3, y: 4, w: 1, h: 1, kind: FrameKind::Bgra, data: vec![1, 2, 3, 4] },
            Up::Audio { pcm: vec![0; 3528] },
            Up::Clipboard { data: Vec::new() },
            Up::Bye,
        ]
    }

    #[test]
    fn every_message_round_trips() {
        for m in down_samples() {
            assert_eq!(Down::decode(&m.encode()).unwrap(), m);
        }
        for m in up_samples() {
            assert_eq!(Up::decode(&m.encode()).unwrap(), m);
        }
    }

    #[test]
    fn framing_round_trips_over_a_stream() {
        let mut wire = Vec::new();
        for m in up_samples() {
            write_frame(&mut wire, &m.encode()).unwrap();
        }
        let mut cur = io::Cursor::new(wire);
        for m in up_samples() {
            let body = read_frame(&mut cur).unwrap().unwrap();
            assert_eq!(Up::decode(&body).unwrap(), m);
        }
        assert_eq!(read_frame(&mut cur).unwrap(), None);
    }

    #[test]
    fn truncated_messages_are_refused() {
        for m in down_samples() {
            let b = m.encode();
            for cut in 0..b.len() {
                assert!(Down::decode(&b[..cut]).is_err(), "cut {cut} of {m:?}");
            }
        }
        for m in up_samples() {
            let b = m.encode();
            for cut in 0..b.len() {
                assert!(Up::decode(&b[..cut]).is_err(), "cut {cut} of {m:?}");
            }
        }
    }

    #[test]
    fn helper_messages_round_trip_and_refuse_bad_input() {
        let reqs = [
            HelperReq::Authenticate { user: "alice".into(), password: "pä ss".into(), source: "10.0.0.1".into() },
            HelperReq::StartSession,
            HelperReq::Bye,
        ];
        for r in reqs {
            assert_eq!(HelperReq::decode(&r.encode()).unwrap(), r);
            let b = r.encode();
            for cut in 0..b.len() {
                assert!(HelperReq::decode(&b[..cut]).is_err());
            }
        }
        for r in [HelperResp::AuthOk, HelperResp::AuthFail, HelperResp::Started, HelperResp::Error { message: "no".into() }, HelperResp::Refused { reason: REFUSED_MEMORY, message: "full".into() }] {
            assert_eq!(HelperResp::decode(&r.encode()).unwrap(), r);
        }
        // a string longer than allowed
        let mut b = vec![1u8];
        b.extend_from_slice(&(MAX_STRING + 1).to_le_bytes());
        assert_eq!(HelperReq::decode(&b), Err(ProtoError::TooLarge(MAX_STRING + 1)));
        // not UTF-8
        let mut b = vec![1u8];
        b.extend_from_slice(&1u32.to_le_bytes());
        b.push(0xff);
        assert_eq!(HelperReq::decode(&b), Err(ProtoError::BadString));
        // a long error message is cut, not refused
        let long = HelperResp::Error { message: "x".repeat(5000) };
        assert!(matches!(HelperResp::decode(&long.encode()), Ok(HelperResp::Error { .. })));
    }

    #[test]
    fn trailing_bytes_are_refused() {
        let mut b = Down::Credit { frames: 1 }.encode();
        b.push(0);
        assert_eq!(Down::decode(&b), Err(ProtoError::Trailing));
    }

    #[test]
    fn unknown_tags_are_refused() {
        assert_eq!(Down::decode(&[200]), Err(ProtoError::UnknownTag(200)));
        assert_eq!(Up::decode(&[9]), Err(ProtoError::UnknownTag(9)));
        // frame kind
        let mut b = Up::Frame { x: 0, y: 0, w: 1, h: 1, kind: FrameKind::Bgra, data: vec![] }.encode();
        b[9] = 7;
        assert_eq!(Up::decode(&b), Err(ProtoError::UnknownTag(7)));
    }

    #[test]
    fn oversized_lengths_are_refused_without_allocating() {
        // frame header claiming 2 GiB
        let mut cur = io::Cursor::new(0x8000_0000u32.to_le_bytes().to_vec());
        assert_eq!(read_frame(&mut cur), Err(ProtoError::TooLarge(0x8000_0000)));
        // byte string inside a message claiming more than the limit
        let mut b = vec![4u8];
        b.extend_from_slice(&(MAX_MESSAGE + 1).to_le_bytes());
        assert_eq!(Up::decode(&b), Err(ProtoError::TooLarge(MAX_MESSAGE + 1)));
    }

    #[test]
    fn zero_length_and_short_streams_are_errors() {
        let mut cur = io::Cursor::new(0u32.to_le_bytes().to_vec());
        assert_eq!(read_frame(&mut cur), Err(ProtoError::Truncated));
        let mut cur = io::Cursor::new(vec![5, 0, 0, 0, 1, 2]);
        assert!(matches!(read_frame(&mut cur), Err(ProtoError::Io(_))));
    }
}
