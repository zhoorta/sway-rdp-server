//! Using a GPU for the session's compositor when the machine has one (docs/GPU-PLAN.md, stage 1). The session picks the render node
//! (the session does, from `WAYDESK_GPU`; the node is opened to `waydesk-users` by a udev rule); no node, or any step failing: software rendering as before.

use std::os::unix::fs::FileTypeExt;
use std::path::PathBuf;

/// What `WAYDESK_GPU` asks for.
#[derive(Debug, PartialEq)]
pub enum Setting {
    Off,
    Auto,
    Node(PathBuf),
}

/// `WAYDESK_GPU`: `off`, `auto` (also nothing set) or the path of a render node.
pub fn parse(value: Option<&str>) -> Setting {
    match value.map(str::trim) {
        None | Some("") | Some("auto") => Setting::Auto,
        Some("off") | Some("0") | Some("no") => Setting::Off,
        Some(path) => Setting::Node(PathBuf::from(path)),
    }
}

/// The render nodes of the machine, in name order.
pub fn render_nodes() -> Vec<PathBuf> {
    let mut nodes: Vec<PathBuf> = std::fs::read_dir("/dev/dri")
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("renderD"))
        .filter(|e| e.metadata().is_ok_and(|m| m.file_type().is_char_device()))
        .map(|e| e.path())
        .collect();
    nodes.sort();
    nodes
}

/// The node to use for a setting, given the nodes that exist.
pub fn choose(setting: &Setting, nodes: &[PathBuf]) -> Option<PathBuf> {
    match setting {
        Setting::Off => None,
        Setting::Auto => nodes.first().cloned(),
        Setting::Node(p) => nodes.contains(p).then(|| p.clone()),
    }
}

/// The node this session should render on, if the account can really open it. `WAYDESK_GPU` comes from `/etc/default/waydesk`.
pub fn usable_node() -> Option<PathBuf> {
    let path = choose(&parse(std::env::var("WAYDESK_GPU").ok().as_deref()), &render_nodes())?;
    match std::fs::OpenOptions::new().read(true).write(true).open(&path) {
        Ok(_) => Some(path),
        Err(e) => {
            tracing::warn!(node = %path.display(), "the GPU is not usable by this account ({e}): software rendering");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setting_from_text() {
        assert_eq!(parse(None), Setting::Auto);
        assert_eq!(parse(Some("")), Setting::Auto);
        assert_eq!(parse(Some(" auto ")), Setting::Auto);
        assert_eq!(parse(Some("off")), Setting::Off);
        assert_eq!(parse(Some("/dev/dri/renderD129")), Setting::Node("/dev/dri/renderD129".into()));
    }

    #[test]
    fn choice_of_node() {
        let nodes = vec![PathBuf::from("/dev/dri/renderD128"), PathBuf::from("/dev/dri/renderD129")];
        assert_eq!(choose(&Setting::Auto, &nodes), Some("/dev/dri/renderD128".into()));
        assert_eq!(choose(&Setting::Auto, &[]), None, "no GPU: software");
        assert_eq!(choose(&Setting::Off, &nodes), None);
        assert_eq!(choose(&Setting::Node("/dev/dri/renderD129".into()), &nodes), Some("/dev/dri/renderD129".into()));
        assert_eq!(choose(&Setting::Node("/dev/dri/renderD130".into()), &nodes), None, "a node that does not exist is not used");
    }
}
