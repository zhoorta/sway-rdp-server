//! Sound (RDPSND): whatever plays in the session goes to a virtual speaker (a PipeWire null sink named
//! `sway-rdp-server`); its monitor is recorded with `parec` and streamed to the client as 20 ms PCM chunks.

use std::io::{self, Read as _};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

use ironrdp_rdpsnd::pdu::{AudioFormat, WaveFormat};
use ironrdp_rdpsnd::server::{NegotiatedFormat, RdpsndError, RdpsndServerHandler, RdpsndServerMessage};
use ironrdp_server::tokio::sync::mpsc::UnboundedSender;
use ironrdp_server::{ServerEvent, ServerEventSender, SoundServerFactory};
use tracing::info;

use crate::link::Link;
use crate::proto::Down;
use crate::Slot;

pub const SINK: &str = "sway-rdp-server";
pub const RATE: u32 = 44_100;
/// 20 ms of 16-bit stereo.
pub const CHUNK: usize = (RATE as usize / 50) * 4;

#[derive(Default)]
pub struct Factory {
    sender: Arc<Mutex<Option<UnboundedSender<ServerEvent>>>>,
    /// With sessions of their own (the root helper starts them as the account) the sound is recorded there, in the
    /// account's PipeWire (`sessionaudio.rs`), and only relayed here; without, this process records it itself.
    slot: Option<Arc<Slot>>,
}

impl Factory {
    /// Records in this process (one session, the whole machine's speaker).
    pub fn new() -> Self {
        Self::default()
    }

    /// Relays what the connection's own session records.
    pub fn remote(slot: Arc<Slot>) -> Self {
        Self { sender: Arc::default(), slot: Some(slot) }
    }
}

impl ServerEventSender for Factory {
    fn set_sender(&mut self, sender: UnboundedSender<ServerEvent>) {
        *self.sender.lock().unwrap() = Some(sender);
    }
}

impl SoundServerFactory for Factory {
    fn build_backend(&self) -> Box<dyn RdpsndServerHandler> {
        Box::new(Handler {
            sender: self.sender.clone(),
            formats: vec![AudioFormat {
                format: WaveFormat::PCM,
                n_channels: 2,
                n_samples_per_sec: RATE,
                n_avg_bytes_per_sec: RATE * 4,
                n_block_align: 4,
                bits_per_sample: 16,
                data: None,
            }],
            child: None,
            previous_sink: None,
            slot: self.slot.clone(),
            remote: None,
        })
    }
}

struct Handler {
    sender: Arc<Mutex<Option<UnboundedSender<ServerEvent>>>>,
    formats: Vec<AudioFormat>,
    child: Option<Child>,
    /// The default sink before the session took over, put back when the stream ends.
    previous_sink: Option<String>,
    slot: Option<Arc<Slot>>,
    /// The session whose sound is relayed.
    remote: Option<Arc<Link>>,
}

impl std::fmt::Debug for Handler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("audio::Handler")
    }
}

pub fn pactl(args: &[&str]) -> io::Result<String> {
    let out = Command::new("pactl").args(args).stderr(Stdio::null()).output()?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The virtual speaker apps play into, made the default so the session's apps use it.
pub fn ensure_sink() -> io::Result<()> {
    if !pactl(&["list", "short", "sinks"])?.lines().any(|l| l.split_whitespace().nth(1) == Some(SINK)) {
        pactl(&["load-module", "module-null-sink", &format!("sink_name={SINK}"), "sink_properties=device.description=sway-rdp-server"])?;
    }
    pactl(&["set-default-sink", SINK])?;
    Ok(())
}

impl RdpsndServerHandler for Handler {
    fn get_formats(&self) -> &[AudioFormat] {
        &self.formats
    }

    fn choose_format<'a>(&mut self, common: &'a [NegotiatedFormat]) -> Option<&'a NegotiatedFormat> {
        common.iter().find(|n| n.format().format == WaveFormat::PCM && n.format().n_channels == 2 && n.format().n_samples_per_sec == RATE && n.format().bits_per_sample == 16)
    }

    fn start(&mut self, _format: &NegotiatedFormat) -> Result<(), Box<dyn RdpsndError>> {
        if let Some(slot) = &self.slot {
            let Some(link) = slot.link() else {
                return Err(Box::new(io::Error::other("sound: no session yet")) as Box<dyn RdpsndError>);
            };
            let sender = self.sender.clone();
            let t0 = std::time::Instant::now();
            link.set_audio_sink(Some(Box::new(move |pcm| {
                if let Some(tx) = sender.lock().unwrap().as_ref() {
                    let _ = tx.send(ServerEvent::Rdpsnd(RdpsndServerMessage::Wave(pcm, t0.elapsed().as_millis() as u32)));
                }
            })));
            link.send(Down::AudioOn);
            info!("sound: relaying the session speaker");
            self.remote = Some(link);
            return Ok(());
        }
        let previous = pactl(&["get-default-sink"]).ok().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty() && s != SINK);
        if previous.is_some() {
            self.previous_sink = previous;
        }
        ensure_sink().map_err(|e| Box::new(io::Error::other(format!("no audio server: {e}"))) as Box<dyn RdpsndError>)?;
        let mut child = Command::new("parec")
            .args([&format!("--device={SINK}.monitor"), "--format=s16le", &format!("--rate={RATE}"), "--channels=2", "--latency-msec=20"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| Box::new(io::Error::other(format!("cannot run parec: {e}"))) as Box<dyn RdpsndError>)?;
        let mut out = child.stdout.take().expect("piped stdout");
        self.child = Some(child);
        let sender = self.sender.clone();
        info!("audio: streaming the session speaker");
        std::thread::spawn(move || {
            let start = std::time::Instant::now();
            let mut buf = vec![0u8; CHUNK];
            let mut sent = 0u64;
            while out.read_exact(&mut buf).is_ok() {
                let ts = start.elapsed().as_millis() as u32;
                let guard = sender.lock().unwrap();
                let Some(tx) = guard.as_ref() else { break };
                if tx.send(ServerEvent::Rdpsnd(RdpsndServerMessage::Wave(buf.clone(), ts))).is_err() {
                    break;
                }
                sent += 1;
                if sent == 1 {
                    info!("audio: first chunk sent");
                }
            }
            info!(chunks = sent, "audio: capture ended");
        });
        Ok(())
    }

    fn stop(&mut self) {
        if let Some(link) = self.remote.take() {
            link.send(Down::AudioOff);
            link.set_audio_sink(None);
        }
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        // Give the VM's own speakers back.
        if let Some(sink) = self.previous_sink.take() {
            let _ = pactl(&["set-default-sink", &sink]);
        }
    }
}

impl Drop for Handler {
    fn drop(&mut self) {
        self.stop();
    }
}
