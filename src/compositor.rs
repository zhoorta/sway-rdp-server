//! What a session asks of the compositor, and how sway is asked.
//!
//! A session is one output of the client's size with a workspace on it. Creating, sizing, focusing and removing those is
//! compositor-specific (sway: `swaymsg`; Hyprland: `hyprctl`), so the session code talks to the `Compositor` trait; `sway` and
//! `hyprland` are the implementations. Which one is used is chosen once, at start (`select`, `--compositor`). Capture (wlr-screencopy) and input (virtual keyboard and pointer) are Wayland protocols
//! and need no such layer.
//!
//! Starting the compositor itself (the private one of a session, its sockets, its configuration) is not part of this: see
//! `OwnCompositor` in worker.rs.

use std::process::{Command, Stdio};

use anyhow::{bail, Context as _, Result};

/// The compositor operations a session needs. All of them are blocking calls, made from the session's own thread.
pub trait Compositor: Send + Sync {
    /// Names of the outputs the compositor has.
    fn output_names(&self) -> Result<Vec<String>>;

    /// Makes a new (headless) output and returns its name.
    fn create_output(&self) -> Result<String>;

    /// Removes an output made by `create_output`.
    fn remove_output(&self, output: &str) -> Result<()>;

    /// Sets up a new output: the client's size at 60 Hz, scale 1.
    fn init_output(&self, output: &str, size: (u16, u16)) -> Result<()>;

    /// Changes the size of the output (a resize of the client's window).
    fn set_mode(&self, output: &str, size: (u16, u16)) -> Result<()>;

    /// Sets the output's scale (1, 1.5, 2, ...), to match the client's.
    fn set_scale(&self, output: &str, scale: &str) -> Result<()>;

    /// Gives the compositor's cursor an invisible theme: a headless compositor draws its cursor into the picture whatever
    /// the capture asks for, and the client has its own pointer.
    fn hide_cursor(&self) -> Result<()>;

    /// Puts the workspace `workspace` on `output` and focuses it there (a workspace of that name that exists elsewhere is
    /// brought along).
    fn show_workspace(&self, output: &str, workspace: &str) -> Result<()>;

    /// Opens a terminal on the focused workspace.
    fn spawn_terminal(&self) -> Result<()>;

    /// Closes what ran in a session: the windows of its workspace, and the ones it minimized (they wait out of sight, marked
    /// with the output they came from).
    fn close_windows(&self, workspace: &str, output: &str);

    /// Moves the focus to another output before `output` goes away: a workspace that is focused when its output goes away is
    /// moved to the remaining output and kept there, empty, under the same name.
    fn release_focus(&self, output: &str);
}

/// Which compositor a session runs on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Sway,
    /// Needs a GPU: Hyprland has no software renderer (see docs/HYPRLAND.md).
    Hyprland,
}

static KIND: std::sync::OnceLock<Kind> = std::sync::OnceLock::new();

static COSMIC: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether the desktop is COSMIC: the compositor is a sway all the same, and its configuration starts COSMIC inside it (docs/COSMIC.md).
pub fn is_cosmic() -> bool {
    COSMIC.load(std::sync::atomic::Ordering::Relaxed)
}

/// COSMIC's own keyboard layout. The session's virtual keyboard has the layout's keymap, but a COSMIC running as a window of sway gets raw key codes from
/// sway and reads them with the layout of its own settings: so they are made the same (COSMIC watches the file and applies it at once). No-op for other desktops.
pub fn set_cosmic_layout(layout: &str) {
    if !is_cosmic() {
        return;
    }
    let Some(home) = std::env::var_os("HOME") else { return };
    let dir = std::path::Path::new(&home).join(".config/cosmic/com.system76.CosmicComp/v1");
    let layout = layout.replace(['"', '\\'], "");
    let ron = format!("(rules: \"\", model: \"\", layout: \"{layout}\", variant: \"\", options: None, repeat_delay: 600, repeat_rate: 25)");
    let write = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(dir.join("xkb_config"), ron));
    match write {
        Ok(()) => tracing::info!(%layout, "COSMIC keyboard layout set"),
        Err(e) => tracing::warn!("cannot set COSMIC's keyboard layout: {e}"),
    }
}

/// Chooses the compositor from its name (`sway`, `cosmic`, `hyprland`; empty is sway). Once, before any session is made.
pub fn select(name: &str) -> Result<Kind> {
    let kind = match name.trim().to_ascii_lowercase().as_str() {
        "" | "sway" => Kind::Sway,
        "cosmic" => {
            COSMIC.store(true, std::sync::atomic::Ordering::Relaxed);
            Kind::Sway
        }
        "hyprland" => Kind::Hyprland,
        other => bail!("unknown compositor {other:?} (sway, cosmic or hyprland)"),
    };
    let _ = KIND.set(kind);
    Ok(*KIND.get().unwrap_or(&kind))
}

/// The compositor of this process: sway unless `select` said otherwise.
pub fn kind() -> Kind {
    *KIND.get_or_init(|| Kind::Sway)
}

/// The command-line name of a compositor (what `select` takes).
pub fn name(kind: Kind) -> &'static str {
    match kind {
        Kind::Sway => "sway",
        Kind::Hyprland => "hyprland",
    }
}

/// The compositor of this process.
pub fn current() -> &'static dyn Compositor {
    static SWAY: sway::Sway = sway::Sway;
    static HYPRLAND: hyprland::Hyprland = hyprland::Hyprland;
    match kind() {
        Kind::Sway => &SWAY,
        Kind::Hyprland => &HYPRLAND,
    }
}

pub mod sway {
    use super::*;

    pub struct Sway;

    /// Runs `swaymsg` (on the sway of the environment: `SWAYSOCK`) and returns what it printed.
    pub fn swaymsg(args: &[&str]) -> Result<String> {
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

    /// Runs a shell command in sway of the given IPC socket (`swaymsg -s SOCKET exec COMMAND`).
    pub fn exec_on(ipc: &std::path::Path, command: &str) -> Result<()> {
        let out = Command::new("swaymsg").arg("-s").arg(ipc).arg("exec").arg(command).output().context("cannot run swaymsg")?;
        anyhow::ensure!(out.status.success(), "swaymsg exec failed: {}", String::from_utf8_lossy(&out.stderr));
        Ok(())
    }

    /// `swaymsg -r -t subscribe -m '["tick"]'`: sway's tick events, one JSON line each, until sway ends. None when swaymsg cannot run.
    pub fn subscribe_ticks() -> Option<std::process::Child> {
        Command::new("swaymsg").args(["-r", "-t", "subscribe", "-m", "[\"tick\"]"]).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()
    }

    fn custom_mode(size: (u16, u16)) -> String {
        format!("{}x{}@60Hz", size.0, size.1)
    }

    // The commands, as plain data so that a test can check them without a sway.

    pub(super) fn init_output_args(output: &str, size: (u16, u16)) -> Vec<String> {
        ["output", output, "mode", "--custom", &custom_mode(size), "scale", "1"].map(String::from).to_vec()
    }

    pub(super) fn set_mode_args(output: &str, size: (u16, u16)) -> Vec<String> {
        ["output", output, "mode", "--custom", &custom_mode(size)].map(String::from).to_vec()
    }

    pub(super) fn workspace_commands(output: &str, workspace: &str) -> Vec<(Vec<String>, bool)> {
        // A workspace is created on the focused output, so the session output is focused first. The second field says
        // whether a failure of that command ends the setup.
        let c = |a: &[&str], must: bool| (a.iter().map(|s| s.to_string()).collect::<Vec<_>>(), must);
        vec![
            c(&["focus", "output", output], true),
            c(&["workspace", workspace], true),
            // If a workspace of that name already existed elsewhere, `workspace` jumped to it: bring it here.
            c(&["move", "workspace", "to", "output", output], false),
            c(&["focus", "output", output], true),
        ]
    }

    pub(super) fn close_windows_args(workspace: &str, output: &str) -> [Vec<String>; 2] {
        [vec![format!("[workspace=\"{workspace}\"]"), "kill".into()], vec![format!("[con_mark=\"^from:{output}$\"]"), "kill".into()]]
    }

    fn run(args: &[String]) -> Result<String> {
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        swaymsg(&refs)
    }

    impl Compositor for Sway {
        fn output_names(&self) -> Result<Vec<String>> {
            let v: serde_json::Value = serde_json::from_str(&swaymsg(&["-t", "get_outputs", "-r"])?)?;
            Ok(v.as_array().context("outputs not an array")?.iter().filter_map(|o| o["name"].as_str().map(str::to_owned)).collect())
        }

        fn create_output(&self) -> Result<String> {
            let before = self.output_names()?;
            swaymsg(&["create_output"])?;
            self.output_names()?.into_iter().find(|n| !before.contains(n)).context("sway created no new output")
        }

        fn remove_output(&self, output: &str) -> Result<()> {
            swaymsg(&["output", output, "unplug"]).map(|_| ())
        }

        fn init_output(&self, output: &str, size: (u16, u16)) -> Result<()> {
            run(&init_output_args(output, size)).map(|_| ())
        }

        fn set_mode(&self, output: &str, size: (u16, u16)) -> Result<()> {
            run(&set_mode_args(output, size)).map(|_| ())
        }

        fn set_scale(&self, output: &str, scale: &str) -> Result<()> {
            swaymsg(&["output", output, "scale", scale]).map(|_| ())
        }

        fn hide_cursor(&self) -> Result<()> {
            swaymsg(&["seat", "*", "xcursor_theme", "waydesk-blank", "24"]).map(|_| ())
        }

        fn show_workspace(&self, output: &str, workspace: &str) -> Result<()> {
            for (args, must) in workspace_commands(output, workspace) {
                match run(&args) {
                    Err(e) if must => return Err(e),
                    _ => {}
                }
            }
            Ok(())
        }

        fn spawn_terminal(&self) -> Result<()> {
            swaymsg(&["exec", "foot -T waydesk-server-session"]).map(|_| ())
        }

        fn close_windows(&self, workspace: &str, output: &str) {
            for args in close_windows_args(workspace, output) {
                let _ = run(&args);
            }
        }

        fn release_focus(&self, output: &str) {
            if let Some(other) = self.output_names().ok().and_then(|v| v.into_iter().find(|n| n != output)) {
                let _ = swaymsg(&["focus", "output", &other]);
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn commands_are_the_ones_the_session_always_sent() {
            assert_eq!(init_output_args("HEADLESS-2", (1280, 720)), ["output", "HEADLESS-2", "mode", "--custom", "1280x720@60Hz", "scale", "1"]);
            assert_eq!(set_mode_args("HEADLESS-2", (1920, 1080)), ["output", "HEADLESS-2", "mode", "--custom", "1920x1080@60Hz"]);
            let ws = workspace_commands("HEADLESS-2", "Remote");
            assert_eq!(ws.len(), 4);
            assert_eq!(ws[0], (vec!["focus".to_string(), "output".into(), "HEADLESS-2".into()], true));
            assert_eq!(ws[1].0, ["workspace", "Remote"]);
            assert!(!ws[2].1, "moving a workspace that was elsewhere is allowed to fail");
            assert_eq!(ws[2].0, ["move", "workspace", "to", "output", "HEADLESS-2"]);
            assert_eq!(ws[3], ws[0]);
            let [a, b] = close_windows_args("Remote", "HEADLESS-2");
            assert_eq!(a, ["[workspace=\"Remote\"]", "kill"]);
            assert_eq!(b, ["[con_mark=\"^from:HEADLESS-2$\"]", "kill"]);
        }
    }
}

pub mod hyprland {
    use super::*;

    /// Hyprland 0.55 and later: configured and driven with Lua (`hyprctl eval`), outputs made with `hyprctl output create headless`.
    /// A headless output can have any size and scale, and a capture of it has exactly that size.
    pub struct Hyprland;

    /// A Lua string literal for `s`.
    pub(super) fn lua_str(s: &str) -> String {
        let mut out = String::from("\"");
        for c in s.chars() {
            match c {
                '\\' => out.push_str("\\\\"),
                '"' => out.push_str("\\\""),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                c if c.is_control() => out.push_str(&format!("\\{:03}", c as u32)),
                c => out.push(c),
            }
        }
        out.push('"');
        out
    }

    // The Lua of each operation, as plain data so that a test can check it without a Hyprland.

    pub(super) fn monitor_lua(output: &str, size: (u32, u32), scale: f64) -> String {
        format!("hl.monitor({{ output = {}, mode = {}, position = 'auto', scale = {scale} }})", lua_str(output), lua_str(&format!("{}x{}@60", size.0, size.1)))
    }

    pub(super) fn focus_monitor_lua(output: &str) -> String {
        format!("hl.dispatch(hl.dsp.focus({{ monitor = {} }}))", lua_str(output))
    }

    pub(super) fn focus_workspace_lua(workspace: &str) -> String {
        format!("hl.dispatch(hl.dsp.focus({{ workspace = {} }}))", lua_str(&format!("name:{workspace}")))
    }

    pub(super) fn move_workspace_lua(workspace: &str, output: &str) -> String {
        format!("hl.dispatch(hl.dsp.workspace.move({{ workspace = {}, monitor = {} }}))", lua_str(&format!("name:{workspace}")), lua_str(output))
    }

    pub(super) fn exec_lua(command: &str) -> String {
        format!("hl.exec_cmd({})", lua_str(command))
    }

    /// Closes every window on the output, whatever workspace it is on (the account may have used others than ours).
    pub(super) fn close_windows_lua(output: &str) -> String {
        format!(
            "for _, w in ipairs(hl.get_windows()) do if w.monitor and w.monitor.name == {} then hl.dispatch(hl.dsp.window.close({{ window = w }})) end end",
            lua_str(output)
        )
    }

    /// Hyprland started without its `start-hyprland` wrapper shows a warning in the picture, on every start and again later. Waydesk starts it itself
    /// (it needs the process, not a supervisor), so the warning is switched off. Run once Hyprland answers.
    pub const QUIET_WATCHDOG_LUA: &str = "hl.config({ misc = { disable_watchdog_warning = true } })";

    /// Where the rule of the session's output is kept for the wrapper configuration (packaging/hyprland/waydesk.lua) to load after the account's own.
    pub const RULE_FILE: &str = "waydesk-monitor.lua";
    /// The file the wrapper writes when it has loaded the configuration.
    pub const READY_FILE: &str = "waydesk-hyprland-ready";

    /// Keeps `lua` (the `hl.monitor` call for the session's output) where the wrapper configuration finds it on the next load.
    pub(super) fn persist_rule(runtime: &std::path::Path, lua: &str) {
        let tmp = runtime.join(format!("{RULE_FILE}.tmp"));
        if std::fs::write(&tmp, format!("{lua}\n")).is_ok() {
            let _ = std::fs::rename(&tmp, runtime.join(RULE_FILE));
        }
    }

    fn runtime_dir() -> Option<std::path::PathBuf> {
        std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from)
    }

    /// Sets the output's rule in Hyprland, remembers it for reloads, and waits until the compositor reports the size.
    fn apply_monitor(output: &str, size: (u16, u16), scale: f64) -> Result<()> {
        let lua = monitor_lua(output, (size.0.into(), size.1.into()), scale);
        eval(&lua)?;
        if let Some(dir) = runtime_dir() {
            persist_rule(&dir, &lua);
        }
        wait_for_size(output, size)
    }

    /// `hyprctl ARGS` on the Hyprland of the environment (`HYPRLAND_INSTANCE_SIGNATURE`), and what it printed.
    pub fn hyprctl(args: &[&str]) -> Result<String> {
        let out = Command::new("hyprctl").args(args).output().context("cannot run hyprctl")?;
        let text = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        if !out.status.success() {
            bail!("hyprctl {args:?} failed: {text} {}", String::from_utf8_lossy(&out.stderr));
        }
        Ok(text)
    }

    /// Runs Lua in Hyprland. It answers `ok`, or says what is wrong with the code.
    fn eval(lua: &str) -> Result<()> {
        let answer = hyprctl(&["eval", lua])?;
        anyhow::ensure!(answer == "ok", "hyprctl eval failed: {answer} (in: {lua})");
        Ok(())
    }

    /// For `hyprctl output create/remove`, which answer `ok`.
    fn expect_ok(args: &[&str]) -> Result<()> {
        let answer = hyprctl(args)?;
        anyhow::ensure!(answer == "ok", "hyprctl {args:?} failed: {answer}");
        Ok(())
    }

    fn monitors() -> Result<Vec<serde_json::Value>> {
        match serde_json::from_str::<serde_json::Value>(&hyprctl(&["-j", "monitors", "all"])?)? {
            serde_json::Value::Array(v) => Ok(v),
            _ => bail!("monitors is not an array"),
        }
    }

    fn monitor(output: &str) -> Result<serde_json::Value> {
        monitors()?.into_iter().find(|m| m["name"].as_str() == Some(output)).with_context(|| format!("Hyprland has no output {output}"))
    }

    /// The Wayland socket and the instance signature of the Hyprland with this process id: what `hyprctl instances` says.
    pub fn instance_of(pid: u32) -> Option<(String, String)> {
        let text = hyprctl(&["instances", "-j"]).ok()?;
        let list: serde_json::Value = serde_json::from_str(&text).ok()?;
        let i = list.as_array()?.iter().find(|i| i["pid"].as_u64() == Some(u64::from(pid)))?;
        Some((i["instance"].as_str()?.to_owned(), i["wl_socket"].as_str()?.to_owned()))
    }

    /// `hyprctl eval` answers before the new mode is in force, and a capture started in between gets a picture of the old size (the session
    /// then reports the old size to the client and stops the capture when the real one shows up). So wait until Hyprland says the output has it.
    fn wait_for_size(output: &str, size: (u16, u16)) -> Result<()> {
        let want = (u64::from(size.0), u64::from(size.1));
        for _ in 0..100 {
            let m = monitor(output)?;
            if (m["width"].as_u64(), m["height"].as_u64()) == (Some(want.0), Some(want.1)) {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        bail!("Hyprland did not give {output} the size {}x{} within 2 s", size.0, size.1)
    }

    /// The DRM card (`/dev/dri/cardN`) of a render node (`/dev/dri/renderDN`): Hyprland is given the card.
    pub fn card_of_render_node(node: &std::path::Path) -> Option<std::path::PathBuf> {
        let name = node.file_name()?.to_str()?;
        let dir = std::fs::read_dir(format!("/sys/class/drm/{name}/device/drm")).ok()?;
        dir.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with("card") && n[4..].chars().all(|c| c.is_ascii_digit())).min().map(|n| std::path::PathBuf::from("/dev/dri").join(n))
    }

    impl Compositor for Hyprland {
        fn output_names(&self) -> Result<Vec<String>> {
            Ok(monitors()?.iter().filter_map(|m| m["name"].as_str().map(str::to_owned)).collect())
        }

        fn create_output(&self) -> Result<String> {
            let before = self.output_names()?;
            expect_ok(&["output", "create", "headless"])?;
            self.output_names()?.into_iter().find(|n| !before.contains(n)).context("Hyprland created no new output")
        }

        fn remove_output(&self, output: &str) -> Result<()> {
            expect_ok(&["output", "remove", output])
        }

        fn init_output(&self, output: &str, size: (u16, u16)) -> Result<()> {
            apply_monitor(output, size, 1.0)
        }

        fn set_mode(&self, output: &str, size: (u16, u16)) -> Result<()> {
            let scale = monitor(output)?["scale"].as_f64().unwrap_or(1.0);
            apply_monitor(output, size, scale)
        }

        fn set_scale(&self, output: &str, scale: &str) -> Result<()> {
            let m = monitor(output)?;
            let size = (m["width"].as_u64().unwrap_or(1024) as u16, m["height"].as_u64().unwrap_or(768) as u16);
            apply_monitor(output, size, scale.parse().context("scale is not a number")?)
        }

        fn hide_cursor(&self) -> Result<()> {
            expect_ok(&["setcursor", "waydesk-blank", "24"])
        }

        fn show_workspace(&self, output: &str, workspace: &str) -> Result<()> {
            eval(&focus_monitor_lua(output))?;
            eval(&focus_workspace_lua(workspace))?;
            // A workspace of that name that already existed elsewhere was jumped to: bring it here.
            let _ = eval(&move_workspace_lua(workspace, output));
            eval(&focus_monitor_lua(output))
        }

        fn spawn_terminal(&self) -> Result<()> {
            eval(&exec_lua("foot -T waydesk-server-session"))
        }

        fn close_windows(&self, _workspace: &str, output: &str) {
            let _ = eval(&close_windows_lua(output));
        }

        fn release_focus(&self, output: &str) {
            if let Some(other) = self.output_names().ok().and_then(|v| v.into_iter().find(|n| n != output)) {
                let _ = eval(&focus_monitor_lua(&other));
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn lua_strings_are_quoted() {
            assert_eq!(lua_str("HEADLESS-1"), "\"HEADLESS-1\"");
            assert_eq!(lua_str("a\"b\\c"), "\"a\\\"b\\\\c\"");
            assert_eq!(lua_str("x\ny"), "\"x\\ny\"");
            assert_eq!(lua_str("\u{1}"), "\"\\001\"");
        }

        #[test]
        fn lua_of_each_operation() {
            assert_eq!(monitor_lua("HEADLESS-1", (1280, 720), 1.0), "hl.monitor({ output = \"HEADLESS-1\", mode = \"1280x720@60\", position = 'auto', scale = 1 })");
            assert_eq!(monitor_lua("HEADLESS-1", (1600, 900), 1.5), "hl.monitor({ output = \"HEADLESS-1\", mode = \"1600x900@60\", position = 'auto', scale = 1.5 })");
            assert_eq!(focus_monitor_lua("HEADLESS-1"), "hl.dispatch(hl.dsp.focus({ monitor = \"HEADLESS-1\" }))");
            assert_eq!(focus_workspace_lua("Remote"), "hl.dispatch(hl.dsp.focus({ workspace = \"name:Remote\" }))");
            assert_eq!(move_workspace_lua("Remote", "HEADLESS-1"), "hl.dispatch(hl.dsp.workspace.move({ workspace = \"name:Remote\", monitor = \"HEADLESS-1\" }))");
            assert_eq!(exec_lua("foot -T x"), "hl.exec_cmd(\"foot -T x\")");
            assert!(close_windows_lua("HEADLESS-1").contains("w.monitor.name == \"HEADLESS-1\""));
        }

        #[test]
        fn a_hostile_name_cannot_leave_its_string() {
            let lua = focus_workspace_lua("x\" }))\nos.execute(\"id\")--");
            assert!(!lua.contains('\n'), "{lua}");
            assert!(lua.contains("\\\" }))\\nos.execute"), "{lua}");
        }

        #[test]
        fn the_start_warning_is_switched_off_by_a_misc_option() {
            assert!(QUIET_WATCHDOG_LUA.contains("misc") && QUIET_WATCHDOG_LUA.contains("disable_watchdog_warning = true"));
        }

        #[test]
        fn the_rule_of_the_output_is_kept_for_the_next_load() {
            let dir = std::env::temp_dir().join(format!("waydesk-rule-test-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            persist_rule(&dir, "hl.monitor({ output = \"HEADLESS-1\" })");
            persist_rule(&dir, &monitor_lua("HEADLESS-1", (1280, 720), 2.0));
            let text = std::fs::read_to_string(dir.join(RULE_FILE)).unwrap();
            assert_eq!(text, "hl.monitor({ output = \"HEADLESS-1\", mode = \"1280x720@60\", position = 'auto', scale = 2 })\n", "the latest rule replaces the earlier one");
            assert!(!dir.join(format!("{RULE_FILE}.tmp")).exists());
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn compositor_names() {
            assert_eq!(select("").unwrap(), Kind::Sway);
            assert!(select("compiz").is_err());
            assert_eq!(name(Kind::Hyprland), "hyprland");
        }
    }
}
