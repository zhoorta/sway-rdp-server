//! What a session asks of the compositor, and how sway is asked.
//!
//! A session is one output of the client's size with a workspace on it. Creating, sizing, focusing and removing those is
//! compositor-specific (sway: `swaymsg`; Hyprland: `hyprctl`), so the session code talks to the `Compositor` trait and
//! `sway` is the implementation. Capture (wlr-screencopy) and input (virtual keyboard and pointer) are Wayland protocols
//! and need no such layer.
//!
//! Starting the compositor itself (the private sway of a session, its sockets, its configuration) is not part of this
//! yet: see `OwnSway` in worker.rs.

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

/// The compositor of this process. Always sway for now.
pub fn current() -> &'static dyn Compositor {
    static SWAY: sway::Sway = sway::Sway;
    &SWAY
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
