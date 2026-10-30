//! What a session says about itself for `waydesk-admin status`: whether it renders on a GPU, which encoder it runs (VAAPI, NVENC, or OpenH264 on the
//! CPU, which a failed GPU start falls back to with nothing but a line in the log), the quantiser and the frame rate it reaches.
//!
//! The session worker writes it to `/run/waydesk/status/UID.json`, a directory the helper makes sticky (like `/tmp`: an account can write its own
//! file and cannot replace another's). The file is readable by its account and by root, so `status` shows the rest of the accounts' sessions to
//! root only. It holds nothing about the client. Shared with the admin tool, which reads it.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

/// Where the files are.
pub const DIR: &str = "/run/waydesk/status";

/// Statistics older than this are not shown as current (a session with nothing on the screen makes no frames and so no statistics).
pub const FRESH_SECS: u64 = 20;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    /// The render node the session's sway draws on; `None`: software.
    pub render: Option<String>,
    /// `vaapi`, `nvenc` or `openh264`.
    pub encoder: Option<String>,
    /// The compositor the session runs on: `sway` or `hyprland` (a session keeps it until it restarts, whatever the setting says now).
    pub compositor: Option<String>,
    /// The quantiser of a GPU encoder.
    pub qp: Option<u32>,
    /// Frames made a second, and the bitrate they took, over the last few seconds.
    pub fps: Option<f64>,
    pub kbps: Option<u32>,
    /// When the statistics were taken (Unix seconds).
    pub stats_at: u64,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub fn path_for(dir: &Path, uid: u32) -> PathBuf {
    dir.join(format!("{uid}.json"))
}

pub fn to_json(s: &Status) -> Value {
    json!({ "render": s.render, "encoder": s.encoder, "compositor": s.compositor, "qp": s.qp, "fps": s.fps, "kbps": s.kbps, "stats_at": s.stats_at })
}

pub fn from_json(v: &Value) -> Status {
    Status {
        render: v.get("render").and_then(Value::as_str).map(str::to_owned),
        encoder: v.get("encoder").and_then(Value::as_str).map(str::to_owned),
        compositor: v.get("compositor").and_then(Value::as_str).map(str::to_owned),
        qp: v.get("qp").and_then(Value::as_u64).map(|q| q as u32),
        fps: v.get("fps").and_then(Value::as_f64),
        kbps: v.get("kbps").and_then(Value::as_u64).map(|k| k as u32),
        stats_at: v.get("stats_at").and_then(Value::as_u64).unwrap_or(0),
    }
}

/// The status of the account's session, if it wrote one that the account really owns.
pub fn read(dir: &Path, uid: u32) -> Option<Status> {
    use std::os::unix::fs::MetadataExt as _;
    let path = path_for(dir, uid);
    let meta = std::fs::metadata(&path).ok()?;
    // The directory is sticky and anyone can create files in it: only a file of the account itself counts.
    if meta.uid() != uid {
        return None;
    }
    let text = std::fs::read_to_string(&path).ok()?;
    Some(from_json(&serde_json::from_str::<Value>(&text).ok()?))
}

/// True while the statistics are recent enough to be called current.
pub fn is_fresh(s: &Status) -> bool {
    s.stats_at != 0 && now().saturating_sub(s.stats_at) <= FRESH_SECS
}

static CURRENT: Mutex<Status> = Mutex::new(Status { render: None, encoder: None, compositor: None, qp: None, fps: None, kbps: None, stats_at: 0 });

fn publish(s: &Status) {
    let dir = Path::new(DIR);
    // Not under the helper (a session started by hand, a test): nothing to write to, and nothing to complain about.
    if !dir.is_dir() {
        return;
    }
    let uid = unsafe { libc::geteuid() };
    let path = path_for(dir, uid);
    let tmp = dir.join(format!("{uid}.json.tmp"));
    let Ok(text) = serde_json::to_string(&to_json(s)) else { return };
    use std::os::unix::fs::PermissionsExt as _;
    if std::fs::write(&tmp, text).is_ok() {
        // Readable by the account and root only (the umask of a service may be looser).
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
        let _ = std::fs::rename(&tmp, &path);
    }
}

fn update(f: impl FnOnce(&mut Status)) {
    let mut s = CURRENT.lock().unwrap_or_else(|e| e.into_inner());
    f(&mut s);
    publish(&s);
}

/// The session's sway draws on `node`, or in software.
pub fn set_render(node: Option<&Path>) {
    update(|s| s.render = node.map(|n| n.display().to_string()));
}

/// The compositor the session runs on (`sway` or `hyprland`).
pub fn set_compositor(name: &str) {
    update(|s| s.compositor = Some(name.to_owned()));
}

/// The encoder the session settled on.
pub fn set_encoder(name: &str) {
    update(|s| {
        s.encoder = Some(name.to_owned());
        s.qp = None;
        s.fps = None;
        s.kbps = None;
        s.stats_at = 0;
    });
}

/// How the encoder does, every few seconds.
pub fn set_stats(fps: f64, kbps: u32, qp: Option<u32>) {
    update(|s| {
        s.fps = Some(fps);
        s.kbps = Some(kbps);
        s.qp = qp;
        s.stats_at = now();
    });
}

/// The session ends: its file goes too.
pub fn clear() {
    let uid = unsafe { libc::geteuid() };
    let _ = std::fs::remove_file(path_for(Path::new(DIR), uid));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_status_survives_its_json() {
        let s = Status { render: Some("/dev/dri/renderD129".into()), encoder: Some("nvenc".into()), compositor: Some("hyprland".into()), qp: Some(25), fps: Some(17.5), kbps: Some(30_000), stats_at: 1_700_000_000 };
        assert_eq!(from_json(&to_json(&s)), s);
        let soft = Status { render: None, encoder: Some("openh264".into()), ..Default::default() };
        assert_eq!(from_json(&to_json(&soft)), soft);
    }

    #[test]
    fn missing_fields_are_nothing_not_an_error() {
        assert_eq!(from_json(&json!({})), Status::default());
        assert_eq!(from_json(&json!({ "encoder": 7, "fps": "fast" })), Status::default());
    }

    #[test]
    fn statistics_go_stale() {
        let fresh = Status { stats_at: now(), ..Default::default() };
        let old = Status { stats_at: now() - FRESH_SECS - 5, ..Default::default() };
        assert!(is_fresh(&fresh));
        assert!(!is_fresh(&old));
        assert!(!is_fresh(&Status::default()), "no statistics yet is not current");
    }

    #[test]
    fn only_a_file_of_the_account_counts() {
        let dir = std::env::temp_dir().join(format!("waydesk-status-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mine = unsafe { libc::geteuid() };
        std::fs::write(path_for(&dir, mine), serde_json::to_string(&to_json(&Status { encoder: Some("vaapi".into()), ..Default::default() })).unwrap()).unwrap();
        assert_eq!(read(&dir, mine).and_then(|s| s.encoder), Some("vaapi".into()));
        // The same file, asked for as the account of another uid, is not that account's.
        std::fs::copy(path_for(&dir, mine), path_for(&dir, mine + 1)).unwrap();
        assert!(read(&dir, mine + 1).is_none(), "a file owned by someone else is not shown for this uid");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
