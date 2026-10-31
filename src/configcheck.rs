//! What an account's own compositor setup does inside a Waydesk session, as plain functions on text and paths (no file access, no processes), so that each rule
//! can be tried in a test. Used by the session worker (which sway configuration to start with) and by `waydesk-admin config check` (what to tell the administrator),
//! so that the two cannot disagree about which configuration is the one in use. Docs: docs/BRING-YOUR-OWN-CONFIG.md.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

/// Configs sway looks for itself (`/usr/lib/waydesk/etc` is where our private sway is built to look instead of `/etc`).
pub const SWAY_OWN_CONFIGS: [&str; 2] = ["/etc/sway/config", "/usr/lib/waydesk/etc/sway/config"];
/// Used when nothing else exists, so that a server without the desktop package still gets a session (sway refuses to start without a config).
pub const MINIMAL_CONFIG: &str = "/usr/share/waydesk/sway-minimal.conf";
/// The configuration of the `cosmic` desktop: sway starts COSMIC (`start-cosmic`) as one fullscreen window of its own headless output.
pub const COSMIC_CONFIG: &str = "/usr/share/waydesk/sway-cosmic.conf";
/// The same for `plasma`: KDE Plasma's own compositor (KWin) as the fullscreen window.
pub const PLASMA_CONFIG: &str = "/usr/share/waydesk/sway-plasma.conf";
/// The configuration of the Waydesk desktop package.
pub const SYSTEM_SWAY_CONFIG: &str = "/etc/xdg/waydesk/sway.conf";
/// Where an administrator's per-account settings are kept (read by the session unit after /etc/default/waydesk): `UID.env`. Next to that file, not under /etc/waydesk
/// (which only the daemon's group may enter): `waydesk-admin users list` and `status` are open to everybody and show what is set here.
pub const ACCOUNT_SETTINGS_DIR: &str = "/etc/default/waydesk.d";

// ---- which sway configuration a session uses ----

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SwaySource {
    /// `~/.config/waydesk/config`: the account's own file for Waydesk sessions.
    AccountWaydesk(PathBuf),
    /// The Waydesk desktop package's configuration.
    SystemWaydesk(PathBuf),
    /// Nothing of Waydesk's: sway's own lookup (the account's `~/.sway/config` or `~/.config/sway/config`, else the system's `/etc/sway/config`) decides.
    SwayLookup(PathBuf),
    /// Nothing at all was found: the minimal configuration of the package.
    Minimal(PathBuf),
    Nothing,
}

/// Which configuration a session starts sway with. The account's own file for these sessions wins, then the system's Waydesk one; then sway's usual lookup,
/// and if that would find nothing, the minimal one.
pub fn sway_source(home: Option<&Path>, is_file: impl Fn(&Path) -> bool) -> SwaySource {
    sway_source_for(home, "sway", is_file)
}

/// The same for an account's desktop: `cosmic` has the package's COSMIC configuration where `sway` has the Waydesk desktop's one (the account's own file still wins).
pub fn sway_source_for(home: Option<&Path>, desktop: &str, is_file: impl Fn(&Path) -> bool) -> SwaySource {
    let user = |rel: &str| home.map(|h| h.join(rel)).filter(|p| is_file(p));
    if let Some(p) = user(".config/waydesk/config") {
        return SwaySource::AccountWaydesk(p);
    }
    let nested = match desktop {
        "cosmic" => Some(PathBuf::from(COSMIC_CONFIG)),
        "plasma" => Some(PathBuf::from(PLASMA_CONFIG)),
        _ => None,
    };
    if let Some(path) = nested.filter(|p| is_file(p)) {
        return SwaySource::SystemWaydesk(path);
    }
    let system = PathBuf::from(SYSTEM_SWAY_CONFIG);
    if is_file(&system) {
        return SwaySource::SystemWaydesk(system);
    }
    if let Some(p) = user(".sway/config").or_else(|| user(".config/sway/config")).or_else(|| SWAY_OWN_CONFIGS.iter().map(PathBuf::from).find(|c| is_file(c))) {
        return SwaySource::SwayLookup(p);
    }
    let minimal = PathBuf::from(MINIMAL_CONFIG);
    if is_file(&minimal) {
        SwaySource::Minimal(minimal)
    } else {
        SwaySource::Nothing
    }
}

/// The config to start sway with (`-c`), or None to let sway look.
pub fn sway_config(home: Option<&Path>, is_file: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    sway_config_for(home, "sway", is_file)
}

pub fn sway_config_for(home: Option<&Path>, desktop: &str, is_file: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    match sway_source_for(home, desktop, is_file) {
        SwaySource::AccountWaydesk(p) | SwaySource::SystemWaydesk(p) | SwaySource::Minimal(p) => Some(p),
        SwaySource::SwayLookup(_) | SwaySource::Nothing => None,
    }
}

/// The account's own sway configuration, when the package's one is used instead (it is left unread, which surprises people).
pub fn ignored_account_sway_config(home: Option<&Path>, is_file: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    let user = |rel: &str| home.map(|h| h.join(rel)).filter(|p| is_file(p));
    match sway_source(home, &is_file) {
        SwaySource::SystemWaydesk(_) => user(".sway/config").or_else(|| user(".config/sway/config")),
        _ => None,
    }
}

// ---- findings ----

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Ok,
    Info,
    Warn,
    Fail,
}

impl Level {
    pub fn word(self) -> &'static str {
        match self {
            Level::Ok => "OK",
            Level::Info => "INFO",
            Level::Warn => "WARN",
            Level::Fail => "FAIL",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    pub level: Level,
    /// A stable name for scripts: `sway.output-disabled`, `hyprland.idle-screen-off`, ...
    pub id: &'static str,
    pub message: String,
    pub hint: Option<String>,
}

impl Finding {
    pub fn new(level: Level, id: &'static str, message: impl Into<String>) -> Self {
        Self { level, id, message: message.into(), hint: None }
    }
    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

// ---- idle and lock ----

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdleStep {
    pub seconds: u64,
    pub command: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdleKind {
    Lock,
    ScreenOff,
    Suspend,
    Dim,
    Other,
}

pub fn classify_idle(command: &str) -> IdleKind {
    let c = command.to_ascii_lowercase();
    if c.contains("dpms off") || c.contains("dpms disable") || (c.contains("dpms") && c.contains("disable")) {
        IdleKind::ScreenOff
    } else if c.contains("suspend") || c.contains("hibernate") {
        IdleKind::Suspend
    } else if c.contains("lock") {
        IdleKind::Lock
    } else if c.contains("brightnessctl") || c.contains("light -") {
        IdleKind::Dim
    } else {
        IdleKind::Other
    }
}

/// The steps of a hypridle configuration: each `listener { timeout = N  on-timeout = COMMAND }`.
pub fn parse_hypridle(text: &str) -> Vec<IdleStep> {
    let mut steps = Vec::new();
    let (mut in_listener, mut timeout, mut command) = (false, None::<u64>, String::new());
    for raw in text.lines() {
        let line = raw.split(" #").next().unwrap_or("").trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if line.starts_with("listener") && line.ends_with('{') {
            in_listener = true;
            timeout = None;
            command.clear();
        } else if in_listener && line == "}" {
            if let Some(seconds) = timeout.take() {
                steps.push(IdleStep { seconds, command: std::mem::take(&mut command) });
            }
            in_listener = false;
        } else if in_listener {
            if let Some((key, value)) = line.split_once('=') {
                match key.trim() {
                    "timeout" => timeout = value.trim().parse().ok(),
                    "on-timeout" | "on_timeout" => command = value.trim().to_owned(),
                    _ => {}
                }
            }
        }
    }
    steps
}

/// Splits a shell command line into words, honouring single and double quotes (no escapes beyond that: enough for a swayidle line).
pub fn shell_words(s: &str) -> Vec<String> {
    let (mut words, mut cur, mut quote, mut any) = (Vec::new(), String::new(), None::<char>, false);
    for c in s.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => cur.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                any = true;
            }
            (None, c) if c.is_whitespace() => {
                if any || !cur.is_empty() {
                    words.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            (None, c) => cur.push(c),
        }
    }
    if any || !cur.is_empty() {
        words.push(cur);
    }
    words
}

/// The `timeout N 'command'` steps of a swayidle command line (what follows `swayidle`).
pub fn parse_swayidle(line: &str) -> Vec<IdleStep> {
    let words = shell_words(line);
    let mut steps = Vec::new();
    let mut i = 0;
    while i + 2 < words.len() {
        if words[i] == "timeout" {
            if let Ok(seconds) = words[i + 1].parse::<u64>() {
                steps.push(IdleStep { seconds, command: words[i + 2].clone() });
                i += 3;
                continue;
            }
        }
        i += 1;
    }
    steps
}

fn minutes(seconds: u64) -> String {
    if seconds % 60 == 0 {
        format!("{} min", seconds / 60)
    } else {
        format!("{seconds} s")
    }
}

/// What the idle steps of an account do to a remote session. `from` says where they came from (a file).
pub fn idle_findings(steps: &[IdleStep], from: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    for s in steps {
        match classify_idle(&s.command) {
            IdleKind::Lock => out.push(
                Finding::new(Level::Info, "idle.lock", format!("{from}: locks the screen after {} without input; the account's password unlocks it over RDP", minutes(s.seconds)))
                    .hint("raise or remove the timeout there if sessions should not lock"),
            ),
            IdleKind::ScreenOff => out.push(
                Finding::new(Level::Warn, "idle.screen-off", format!("{from}: switches the display off after {} without input, and the remote picture goes black until the next key or mouse movement", minutes(s.seconds)))
                    .hint("remove that step, or raise its timeout, in the idle configuration"),
            ),
            IdleKind::Suspend => out.push(Finding::new(
                Level::Info,
                "idle.suspend",
                format!("{from}: tries to suspend the machine after {}; refused for accounts that are not administrators (the session is not affected)", minutes(s.seconds)),
            )),
            IdleKind::Dim | IdleKind::Other => {}
        }
    }
    out
}

// ---- power commands ----

/// The lines of a configuration that power off, restart, suspend or log out: `(line number, trimmed line)`, at most `limit`. Comments are skipped.
pub fn power_command_lines(text: &str, limit: usize) -> Vec<(usize, String)> {
    const WORDS: [&str; 10] = ["poweroff", "shutdown", "reboot", "hyprshutdown", "systemctl suspend", "systemctl hibernate", "systemctl halt", "loginctl terminate", "uwsm stop", "wlogout"];
    let mut out = Vec::new();
    for (n, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.starts_with('#') || line.starts_with("--") || line.starts_with("//") || line.is_empty() {
            continue;
        }
        if WORDS.iter().any(|w| line.contains(w)) {
            out.push((n + 1, line.chars().take(110).collect()));
            if out.len() >= limit {
                break;
            }
        }
    }
    out
}

pub fn power_findings(lines: &[(usize, String)], file: &str) -> Vec<Finding> {
    if lines.is_empty() {
        return Vec::new();
    }
    let sample = lines.iter().take(3).map(|(n, l)| format!("{file}:{n}: {l}")).collect::<Vec<_>>().join("; ");
    vec![Finding::new(Level::Info, "config.power-commands", format!("the configuration has power or logout commands ({sample})"))
        .hint("on a shared server restart, shutdown and suspend are refused for ordinary accounts; logout works; for hyprshutdown and Omarchy's menu Waydesk has stand-ins that say so")]
}

// ---- sway configurations ----

fn sway_lines(text: &str) -> impl Iterator<Item = (usize, &str)> {
    text.lines().enumerate().map(|(n, l)| (n + 1, l.trim())).filter(|(_, l)| !l.is_empty() && !l.starts_with('#'))
}

/// The files a sway configuration includes (plain paths; patterns and variables are not followed).
pub fn sway_includes(text: &str) -> Vec<String> {
    sway_lines(text)
        .filter_map(|(_, l)| l.strip_prefix("include ").map(|p| p.trim().to_owned()))
        .filter(|p| !p.contains('*') && !p.contains('$'))
        .collect()
}

/// What a sway configuration does that matters on a server whose sessions have one headless output of their own (named `HEADLESS-N`).
pub fn sway_config_findings(text: &str, file: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    for (n, line) in sway_lines(text) {
        let w = shell_words(line);
        match w.first().map(String::as_str) {
            Some("output") if w.len() >= 3 && w[2] == "disable" && (w[1] == "*" || w[1].starts_with("HEADLESS")) => out.push(
                Finding::new(Level::Fail, "sway.output-disabled", format!("{file}:{n}: `{line}` disables the session's own output (sessions use outputs named HEADLESS-N): the picture would be black"))
                    .hint("name the physical output instead of `*`, or remove the line"),
            ),
            Some("output") if w.len() >= 4 && w[2] == "dpms" && w[3] == "off" && w[1] == "*" => out.push(
                Finding::new(Level::Warn, "sway.dpms-off", format!("{file}:{n}: `{line}` switches the session's output off")).hint("remove it, or limit it to a physical output by name"),
            ),
            Some("workspace") if w.len() >= 4 && w[2] == "output" && !w[3].starts_with("HEADLESS") => out.push(
                Finding::new(Level::Warn, "sway.workspace-pinned", format!("{file}:{n}: workspace {} is pinned to the output {}, which this server's sessions do not have", w[1], w[3]))
                    .hint("sway will show it on the session's output anyway; remove the pin if workspaces behave oddly"),
            ),
            Some("exec" | "exec_always") if line.contains("swayidle") => {
                let rest = line.split("swayidle").nth(1).unwrap_or("");
                out.extend(idle_findings(&parse_swayidle(rest), &format!("{file}:{n} (swayidle)")));
            }
            Some("exec" | "exec_always") if line.contains("swaylock") => {
                out.push(Finding::new(Level::Info, "sway.lock-at-start", format!("{file}:{n}: `{line}` starts the screen locker")));
            }
            _ => {}
        }
    }
    out
}

// ---- Hyprland ----

/// `Hyprland 0.57.0 built from ...` or `v0.57.0`: (major, minor, patch).
pub fn parse_hyprland_version(text: &str) -> Option<(u32, u32, u32)> {
    let start = text.find("Hyprland ").map(|i| i + "Hyprland ".len()).or_else(|| text.find('v').map(|i| i + 1))?;
    let token: String = text[start..].chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    let mut parts = token.split('.').filter(|p| !p.is_empty()).map(|p| p.parse::<u32>().ok());
    Some((parts.next()??, parts.next()??, parts.next().flatten().unwrap_or(0)))
}

/// The Lua configuration (and `hl.exec_cmd`) needs Hyprland 0.55 or later.
pub fn hyprland_version_finding(version: Option<(u32, u32, u32)>) -> Finding {
    match version {
        Some((major, minor, patch)) if major > 0 || minor >= 55 => Finding::new(Level::Ok, "hyprland.version", format!("Hyprland {major}.{minor}.{patch}")),
        Some((major, minor, patch)) => Finding::new(Level::Fail, "hyprland.version", format!("Hyprland {major}.{minor}.{patch} is older than 0.55; Waydesk drives Hyprland with its Lua interface"))
            .hint("update Hyprland, or use sway for this account"),
        None => Finding::new(Level::Warn, "hyprland.version", "could not tell which version of Hyprland is installed"),
    }
}

pub fn hyprland_format_finding(has_lua: bool, has_old_conf: bool) -> Finding {
    match (has_lua, has_old_conf) {
        (true, _) => Finding::new(Level::Ok, "hyprland.config-format", "the account has a Lua configuration (~/.config/hypr/hyprland.lua)"),
        (false, true) => Finding::new(Level::Fail, "hyprland.config-format", "the account has only the old hyprland.conf: Waydesk's Hyprland support needs the Lua configuration (Hyprland 0.55 and later)")
            .hint("convert it (Hyprland's wiki describes the Lua format), or use sway for this account"),
        (false, false) => Finding::new(Level::Info, "hyprland.config-format", "the account has no Hyprland configuration of its own: Hyprland's defaults are used"),
    }
}

/// Removes terminal colour codes (`ESC [ ... letter`).
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for n in chars.by_ref() {
                if n.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// What `Hyprland --verify-config` printed: the part after its "Config parsing result" line is the answer, the debug lines before it are noise.
pub fn verify_config_finding(output: &str, exited_ok: bool) -> Finding {
    let clean = strip_ansi(output);
    let text = clean.trim();
    if exited_ok && text.contains("config ok") {
        return Finding::new(Level::Ok, "hyprland.config-valid", "Hyprland accepts the configuration (--verify-config)");
    }
    let answer = match text.find("Config parsing result") {
        Some(i) => &text[i..],
        None => text,
    };
    let detail: Vec<&str> = answer
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("====") && !l.contains("Config parsing result") && !l.contains("DEBUG") && !l.starts_with("no field") && !l.starts_with("no file"))
        .take(3)
        .collect();
    Finding::new(Level::Fail, "hyprland.config-valid", format!("Hyprland does not accept the configuration: {}", if detail.is_empty() { "no details (run `Hyprland --verify-config --config FILE` as the account)".to_owned() } else { detail.join(" | ") }))
        .hint("fix the error above; Waydesk would start the session without the account's configuration")
}

/// The first word of each `hl.exec_cmd("...")` (and old `exec-once = ...`) with a literal command: what the configuration starts.
pub fn hypr_exec_programs(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with("--") || line.starts_with('#') {
            continue;
        }
        let command = if let Some(i) = line.find("exec_cmd(") {
            let rest = line[i + "exec_cmd(".len()..].trim_start();
            match rest.chars().next() {
                Some(q @ ('"' | '\'')) => rest[1..].split(q).next().map(str::to_owned),
                _ => None,
            }
        } else {
            line.strip_prefix("exec-once").and_then(|r| r.split_once('=')).map(|(_, v)| v.trim().to_owned())
        };
        if let Some(c) = command {
            if let Some(first) = c.split_whitespace().next() {
                let first = first.trim_start_matches(['[', '{']);
                if !first.is_empty() && !out.iter().any(|o| o == first) {
                    out.push(first.to_owned());
                }
            }
        }
    }
    out
}

/// Programs that are not worth looking for (shell builtins and what is always there).
pub fn is_common_program(name: &str) -> bool {
    matches!(name, "sh" | "bash" | "env" | "sleep" | "systemctl" | "dbus-update-activation-environment" | "hyprctl" | "echo" | "true" | "false" | "[" | "test" | "cd" | "exec" | "pidof" | "notify-send")
}

// ---- the per-account settings file ----

/// `sway`, `cosmic`, `plasma`, `hyprland` or `default` (the server-wide setting), whatever the case.
pub fn normalize_desktop(s: &str) -> Option<&'static str> {
    match s.trim().to_ascii_lowercase().as_str() {
        "sway" => Some("sway"),
        "cosmic" => Some("cosmic"),
        "plasma" => Some("plasma"),
        "hyprland" => Some("hyprland"),
        "default" | "server" => Some("default"),
        _ => None,
    }
}

/// What the account's settings file holds: a value per setting, `None` where the account follows the server.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AccountSettings {
    /// `sway` or `hyprland` (`WAYDESK_COMPOSITOR`).
    pub desktop: Option<String>,
    /// `auto`, `off` or a render node (`WAYDESK_GPU`).
    pub gpu: Option<String>,
}

impl AccountSettings {
    pub fn is_empty(&self) -> bool {
        self.desktop.is_none() && self.gpu.is_none()
    }
}

/// The text of the settings file for an account (`changed` is the command line that wrote it, for the comment).
pub fn render_account_env(name: &str, settings: &AccountSettings, changed: &str) -> String {
    let mut out = format!("# Written by `waydesk-admin users set {name} {changed}`; read by the session unit after /etc/default/waydesk.\n");
    if let Some(d) = &settings.desktop {
        out.push_str(&format!("WAYDESK_COMPOSITOR={d}\n"));
    }
    if let Some(g) = &settings.gpu {
        out.push_str(&format!("WAYDESK_GPU={g}\n"));
    }
    out
}

fn parse_account_value(text: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}=");
    text.lines().filter_map(|l| l.trim().strip_prefix(prefix.as_str())).map(|v| v.trim().trim_matches(['"', '\'']).to_owned()).filter(|v| !v.is_empty()).last()
}

/// The settings an account's file asks for (nothing set: no file, or no such line).
pub fn parse_account_env(text: &str) -> AccountSettings {
    AccountSettings {
        desktop: parse_account_value(text, "WAYDESK_COMPOSITOR").map(|v| v.to_ascii_lowercase()),
        gpu: parse_account_value(text, "WAYDESK_GPU"),
    }
}

/// What `--gpu` accepts, given the render nodes that exist: `auto`, `off`, `renderD129` or `/dev/dri/renderD129` (a node that exists), or `default`
/// (the server's setting, `Ok(None)`). Anything else is an error that says what is possible.
pub fn normalize_gpu(s: &str, nodes: &[String]) -> Result<Option<String>, String> {
    let t = s.trim();
    match t.to_ascii_lowercase().as_str() {
        "default" | "server" => return Ok(None),
        "auto" => return Ok(Some("auto".into())),
        "off" | "none" | "cpu" => return Ok(Some("off".into())),
        _ => {}
    }
    let path = if t.starts_with('/') { t.to_owned() } else { format!("/dev/dri/{t}") };
    if nodes.contains(&path) {
        Ok(Some(path))
    } else if nodes.is_empty() {
        Err(format!("{t:?} is not a render node of this machine (it has no GPU); use auto, off or default"))
    } else {
        Err(format!("{t:?} is not a render node of this machine; use auto, off, default or one of: {}", nodes.join(", ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has(files: &'static [&'static str]) -> impl Fn(&Path) -> bool {
        move |p: &Path| files.iter().any(|f| p == Path::new(f))
    }

    #[test]
    fn the_sway_source_follows_the_order_the_session_uses() {
        let home = Some(Path::new("/home/a"));
        assert_eq!(sway_source(home, has(&["/home/a/.config/waydesk/config", SYSTEM_SWAY_CONFIG])), SwaySource::AccountWaydesk("/home/a/.config/waydesk/config".into()));
        assert_eq!(sway_source(home, has(&[SYSTEM_SWAY_CONFIG, "/home/a/.config/sway/config"])), SwaySource::SystemWaydesk(SYSTEM_SWAY_CONFIG.into()));
        assert_eq!(sway_source(home, has(&["/home/a/.config/sway/config", "/etc/sway/config"])), SwaySource::SwayLookup("/home/a/.config/sway/config".into()));
        assert_eq!(sway_source(home, has(&["/etc/sway/config"])), SwaySource::SwayLookup("/etc/sway/config".into()));
        assert_eq!(sway_source(home, has(&[MINIMAL_CONFIG])), SwaySource::Minimal(MINIMAL_CONFIG.into()));
        assert_eq!(sway_source(home, has(&[])), SwaySource::Nothing);
        assert_eq!(sway_source(None, has(&["/etc/sway/config"])), SwaySource::SwayLookup("/etc/sway/config".into()));
    }

    #[test]
    fn the_cosmic_desktop_has_its_own_system_config() {
        let home = Some(Path::new("/home/a"));
        let has = |l: &'static [&'static str]| move |p: &Path| l.iter().any(|x| Path::new(x) == p);
        assert_eq!(sway_source_for(home, "cosmic", has(&[COSMIC_CONFIG, SYSTEM_SWAY_CONFIG])), SwaySource::SystemWaydesk(COSMIC_CONFIG.into()));
        assert_eq!(sway_source_for(home, "sway", has(&[COSMIC_CONFIG, SYSTEM_SWAY_CONFIG])), SwaySource::SystemWaydesk(SYSTEM_SWAY_CONFIG.into()));
        assert_eq!(sway_source_for(home, "cosmic", has(&[COSMIC_CONFIG, "/home/a/.config/waydesk/config"])), SwaySource::AccountWaydesk("/home/a/.config/waydesk/config".into()));
        assert_eq!(sway_source_for(home, "cosmic", has(&[SYSTEM_SWAY_CONFIG])), SwaySource::SystemWaydesk(SYSTEM_SWAY_CONFIG.into()), "without the COSMIC file: as for sway");
        assert_eq!(sway_source_for(home, "plasma", has(&[COSMIC_CONFIG, PLASMA_CONFIG])), SwaySource::SystemWaydesk(PLASMA_CONFIG.into()));
    }

    #[test]
    fn the_package_config_hides_the_accounts_own_sway_config() {
        let home = Some(Path::new("/home/a"));
        assert_eq!(ignored_account_sway_config(home, has(&[SYSTEM_SWAY_CONFIG, "/home/a/.config/sway/config"])), Some("/home/a/.config/sway/config".into()));
        assert_eq!(ignored_account_sway_config(home, has(&["/home/a/.config/sway/config"])), None, "used, not ignored");
        assert_eq!(ignored_account_sway_config(home, has(&[SYSTEM_SWAY_CONFIG])), None);
    }

    #[test]
    fn hypridle_steps_with_comments() {
        let text = "general {\n    lock_cmd = pidof hyprlock || hyprlock\n}\nlistener {\n    timeout = 480   # 8min.\n    on-timeout = brightnessctl -s set 10   # dim\n}\nlistener {\n    timeout = 600\n    on-timeout = loginctl lock-session\n}\nlistener {\n    timeout = 660\n    on-timeout = hyprctl dispatch 'hl.dsp.dpms({ action = \"disable\" })'\n}\nlistener {\n    timeout = 1800\n    on-timeout = systemctl suspend\n}\n";
        let steps = parse_hypridle(text);
        assert_eq!(steps.len(), 4);
        assert_eq!(steps[1], IdleStep { seconds: 600, command: "loginctl lock-session".into() });
        let kinds: Vec<_> = steps.iter().map(|s| classify_idle(&s.command)).collect();
        assert_eq!(kinds, [IdleKind::Dim, IdleKind::Lock, IdleKind::ScreenOff, IdleKind::Suspend]);
        let findings = idle_findings(&steps, "hypridle.conf");
        assert_eq!(findings.iter().map(|f| f.id).collect::<Vec<_>>(), ["idle.lock", "idle.screen-off", "idle.suspend"]);
        assert!(findings[1].message.contains("11 min") && findings[1].level == Level::Warn);
    }

    #[test]
    fn swayidle_command_lines() {
        let steps = parse_swayidle(" -w timeout 300 'swaylock -f' timeout 600 'swaymsg \"output * dpms off\"' resume 'swaymsg \"output * dpms on\"' before-sleep 'swaylock -f'");
        assert_eq!(steps, vec![IdleStep { seconds: 300, command: "swaylock -f".into() }, IdleStep { seconds: 600, command: "swaymsg \"output * dpms off\"".into() }]);
        assert_eq!(classify_idle(&steps[1].command), IdleKind::ScreenOff);
        assert!(parse_swayidle("-w").is_empty());
    }

    #[test]
    fn shell_words_keep_quoted_text_together() {
        assert_eq!(shell_words("a 'b c' \"d e\" f"), ["a", "b c", "d e", "f"]);
        assert_eq!(shell_words("x ''"), ["x", ""]);
    }

    #[test]
    fn sway_configurations_are_checked_for_what_breaks_a_headless_session() {
        let text = "# comment\nset $mod Mod4\noutput * disable\noutput HDMI-A-1 disable\noutput * dpms off\nworkspace 1 output DP-1\nworkspace 2 output HEADLESS-1\nexec swayidle -w timeout 300 'swaylock -f' timeout 600 'swaymsg \"output * dpms off\"'\nexec swaylock\nbindsym $mod+Shift+e exec systemctl poweroff\ninclude ~/.config/sway/extra\ninclude ~/.config/sway/conf.d/*\n";
        let ids: Vec<_> = sway_config_findings(text, "config").iter().map(|f| f.id).collect();
        assert_eq!(ids, ["sway.output-disabled", "sway.dpms-off", "sway.workspace-pinned", "idle.lock", "idle.screen-off", "sway.lock-at-start"]);
        assert_eq!(sway_includes(text), ["~/.config/sway/extra"], "a pattern is not followed");
        assert!(sway_config_findings("output HDMI-A-1 disable\n", "c").is_empty(), "a physical output may be disabled");
        assert!(sway_config_findings("# output * disable\n", "c").is_empty(), "comments are skipped");
    }

    #[test]
    fn power_commands_are_found_and_comments_skipped() {
        let text = "-- hl.exec_cmd(\"poweroff\")\nhl.bind(mod .. \" + SHIFT + E\", hl.dsp.exec_cmd(\"wlogout\"))\nlocal x = 1\nhl.exec_cmd(\"systemctl suspend\")\n";
        let lines = power_command_lines(text, 5);
        assert_eq!(lines.iter().map(|(n, _)| *n).collect::<Vec<_>>(), [2, 4]);
        assert_eq!(power_findings(&lines, "hyprland.lua")[0].id, "config.power-commands");
        assert!(power_findings(&[], "x").is_empty());
    }

    #[test]
    fn hyprland_versions() {
        assert_eq!(parse_hyprland_version("Hyprland 0.57.0 built from branch v0.57.0 at commit abc"), Some((0, 57, 0)));
        assert_eq!(parse_hyprland_version("v0.55.1"), Some((0, 55, 1)));
        assert_eq!(parse_hyprland_version("nothing here"), None);
        assert_eq!(hyprland_version_finding(Some((0, 57, 0))).level, Level::Ok);
        assert_eq!(hyprland_version_finding(Some((0, 54, 2))).level, Level::Fail);
        assert_eq!(hyprland_version_finding(Some((1, 0, 0))).level, Level::Ok);
        assert_eq!(hyprland_version_finding(None).level, Level::Warn);
    }

    #[test]
    fn hyprland_config_formats() {
        assert_eq!(hyprland_format_finding(true, true).level, Level::Ok);
        assert_eq!(hyprland_format_finding(false, true).level, Level::Fail);
        assert_eq!(hyprland_format_finding(false, false).level, Level::Info);
    }

    #[test]
    fn verify_config_output() {
        assert_eq!(verify_config_finding("\n======== Config parsing result:\n\nconfig ok\n", true).level, Level::Ok);
        let bad = verify_config_finding("======== Config parsing result:\n\nhyprland.lua:9: module 'functions' not found:\n\tno field package.preload['functions']\n", false);
        assert_eq!(bad.level, Level::Fail);
        assert!(bad.message.contains("module 'functions' not found"), "{}", bad.message);
        assert_eq!(verify_config_finding("config ok", false).level, Level::Fail, "a failing exit is not ok");
        let noisy = "\u{1b}[1;32mDEBUG \u{1b}[0m]: [main] User-specified config location\n\u{1b}[1;32mDEBUG \u{1b}[0m]: [Config] more noise\n======== Config parsing result:\n\nhyprland.lua:1: module 'x' not found:\n\tno field package.preload['x']\n\tno file '/usr/lib/lua/x.lua'\n";
        let f = verify_config_finding(noisy, false);
        assert!(f.message.contains("module 'x' not found") && !f.message.contains("DEBUG") && !f.message.contains('\u{1b}'), "{}", f.message);
    }

    #[test]
    fn colour_codes_are_removed() {
        assert_eq!(strip_ansi("\u{1b}[1;32mgreen\u{1b}[0m plain"), "green plain");
        assert_eq!(strip_ansi("no codes"), "no codes");
    }

    #[test]
    fn what_a_hyprland_configuration_starts() {
        let text = "-- hl.exec_cmd(\"commented\")\nhl.exec_cmd(\"waybar\")\nhl.exec_cmd('hypridle --quiet')\nhl.exec_cmd(HOME .. \"/x.sh\")\nhl.exec_cmd(\"waybar again\")\nexec-once = swaync\n";
        assert_eq!(hypr_exec_programs(text), ["waybar", "hypridle", "swaync"]);
        assert!(is_common_program("systemctl") && !is_common_program("waybar"));
    }

    #[test]
    fn the_account_settings_file() {
        assert_eq!(normalize_desktop(" Hyprland "), Some("hyprland"));
        assert_eq!(normalize_desktop("SWAY"), Some("sway"));
        assert_eq!(normalize_desktop("COSMIC"), Some("cosmic"));
        assert_eq!(normalize_desktop("Plasma"), Some("plasma"));
        assert_eq!(normalize_desktop("default"), Some("default"));
        assert_eq!(normalize_desktop("gnome"), None);
        let both = AccountSettings { desktop: Some("hyprland".into()), gpu: Some("/dev/dri/renderD129".into()) };
        assert_eq!(parse_account_env(&render_account_env("alice", &both, "--gpu renderD129")), both);
        assert_eq!(parse_account_env("# nothing\n"), AccountSettings::default());
        assert_eq!(parse_account_env("WAYDESK_COMPOSITOR=sway\nWAYDESK_COMPOSITOR=\"Hyprland\"\n").desktop, Some("hyprland".into()), "the last one wins");
        let nodes = vec!["/dev/dri/renderD128".to_owned(), "/dev/dri/renderD129".to_owned()];
        assert_eq!(normalize_gpu("renderD129", &nodes), Ok(Some("/dev/dri/renderD129".into())));
        assert_eq!(normalize_gpu("/dev/dri/renderD128", &nodes), Ok(Some("/dev/dri/renderD128".into())));
        assert_eq!(normalize_gpu("OFF", &nodes), Ok(Some("off".into())));
        assert_eq!(normalize_gpu("auto", &[]), Ok(Some("auto".into())));
        assert_eq!(normalize_gpu("default", &nodes), Ok(None));
        assert!(normalize_gpu("renderD130", &nodes).is_err(), "a node that does not exist is refused");
        assert!(normalize_gpu("renderD128", &[]).is_err());
    }
}
