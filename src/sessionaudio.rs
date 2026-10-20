//! The session side of the sound: the session process runs as the account and so reaches its own PipeWire. It makes the virtual
//! speaker (a null sink the apps play into), records its monitor with `parec` and sends 20 ms chunks to the network daemon, which
//! only relays them to the client (`audio.rs`). Recording runs while the client's sound channel is open.

use std::io::Read as _;
use std::process::{Child, Command, Stdio};

use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::audio::{ensure_sink, pactl, CHUNK, RATE, SINK};
use crate::proto::Up;

pub struct Audio {
    child: Option<Child>,
    up: mpsc::UnboundedSender<Up>,
    /// The default sink before the session took over, put back when the recording ends.
    previous_sink: Option<String>,
}

impl Audio {
    pub fn new(up: mpsc::UnboundedSender<Up>) -> Self {
        Self { child: None, up, previous_sink: None }
    }

    pub fn start(&mut self) {
        self.stop();
        let previous = pactl(&["get-default-sink"]).ok().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty() && s != SINK);
        if previous.is_some() {
            self.previous_sink = previous;
        }
        if let Err(e) = ensure_sink() {
            warn!("sound: no audio server in this session ({e}); is pipewire-pulse running?");
            return;
        }
        let spawned = Command::new("parec")
            .args([&format!("--device={SINK}.monitor"), "--format=s16le", &format!("--rate={RATE}"), "--channels=2", "--latency-msec=20"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();
        let mut child = match spawned {
            Ok(c) => c,
            Err(e) => {
                warn!("sound: cannot run parec ({e}); is pulseaudio-utils installed?");
                return;
            }
        };
        let mut out = child.stdout.take().expect("piped stdout");
        self.child = Some(child);
        let up = self.up.clone();
        info!("sound: recording the session speaker");
        std::thread::spawn(move || {
            let mut buf = vec![0u8; CHUNK];
            let mut sent = 0u64;
            while out.read_exact(&mut buf).is_ok() {
                if up.send(Up::Audio { pcm: buf.clone() }).is_err() {
                    break;
                }
                sent += 1;
            }
            info!(chunks = sent, "sound: recording ended");
        });
    }

    pub fn stop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        // Give the account's own speakers back.
        if let Some(sink) = self.previous_sink.take() {
            let _ = pactl(&["set-default-sink", &sink]);
        }
    }
}

impl Drop for Audio {
    fn drop(&mut self) {
        self.stop();
    }
}
