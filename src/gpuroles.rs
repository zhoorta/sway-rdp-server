//! What each GPU of the machine is for (`WAYDESK_GPU_ROLES`, docs/GPU.md): sessions, a local model, or both. A card kept for a model is not chosen by `auto`, and NVENC stays off it
//! for sessions drawn on another card. The decisions are plain functions over plain values so that they can be tested without a GPU.
#![allow(dead_code)]

use std::collections::HashMap;

/// `renderD129` out of `/dev/dri/renderD129` (and out of `renderD129`).
pub fn short_name(node: &str) -> &str {
    node.rsplit('/').next().unwrap_or(node)
}

/// What a card is for. Cards that are not listed are `Render` (sessions), as before roles existed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Sessions only: `auto` may pick it, and a model is not pointed at it.
    Render,
    /// A model (or anything else that is not a session) only: `auto` skips it and NVENC stays off it for other cards' sessions.
    Inference,
    /// Both: sessions may draw on it and the model may use it.
    Both,
}

impl Role {
    pub fn word(self) -> &'static str {
        match self {
            Role::Render => "render",
            Role::Inference => "inference",
            Role::Both => "both",
        }
    }
}

fn parse_role(s: &str) -> Option<Role> {
    match s.trim().to_ascii_lowercase().as_str() {
        "render" | "rendering" | "sessions" | "desktops" => Some(Role::Render),
        "inference" | "ai" | "model" | "models" | "compute" | "reserved" => Some(Role::Inference),
        "both" | "shared" => Some(Role::Both),
        _ => None,
    }
}

/// `renderD128=render,renderD129=inference,renderD130=both` (`WAYDESK_GPU_ROLES`), by node name; entries that are not `name=role` are ignored.
pub fn parse_roles(text: &str) -> HashMap<String, Role> {
    text.split([',', ' ']).filter_map(|e| e.trim().split_once('=')).filter_map(|(n, r)| Some((short_name(n.trim()).to_owned(), parse_role(r)?))).collect()
}

/// The role of every card `WAYDESK_GPU_ROLES` names.
pub fn effective_roles(roles: Option<&str>) -> HashMap<String, Role> {
    roles.map(parse_roles).unwrap_or_default()
}

/// The nodes that take no sessions of their own accord (role `Inference`).
pub fn reserved_nodes(roles: Option<&str>) -> Vec<String> {
    let mut v: Vec<String> = effective_roles(roles).into_iter().filter(|(_, r)| *r == Role::Inference).map(|(n, _)| n).collect();
    v.sort();
    v
}

/// What `status` adds under a card: its role when roles are in use, who is on it, and how to point a model at it. `role` is `None` when no roles are configured
/// (then a card with a UUID is offered to a model, as before roles existed); a `Render` card is not.
pub fn card_note(role: Option<Role>, sessions: &[String], uuid: Option<&str>) -> String {
    let mut parts = Vec::new();
    match role {
        Some(Role::Inference) => parts.push("inference only (new sessions do not pick it)".to_owned()),
        Some(Role::Render) => parts.push("sessions only".to_owned()),
        Some(Role::Both) => parts.push("sessions and inference".to_owned()),
        None => {}
    }
    parts.push(match sessions {
        [] => "no sessions".to_owned(),
        [one] => format!("1 session: {one}"),
        many => format!("{} sessions: {}", many.len(), many.join(", ")),
    });
    if let (Some(u), true) = (uuid, role != Some(Role::Render)) {
        parts.push(format!("for a model: CUDA_VISIBLE_DEVICES={u}"));
    }
    parts.join("; ")
}

/// The line that names every card a model may use: `CUDA_VISIBLE_DEVICES=GPU-a,GPU-b`. `cards`: (node, role, uuid) of the NVIDIA cards. Only when roles are in use and
/// at least one card is meant for inference (`Inference` or `Both`); `None` otherwise.
pub fn inference_line(cards: &[(String, Option<Role>, Option<String>)]) -> Option<String> {
    let chosen: Vec<_> = cards.iter().filter(|(_, r, u)| matches!(r, Some(Role::Inference) | Some(Role::Both)) && u.is_some()).collect();
    if chosen.is_empty() {
        return None;
    }
    let uuids: Vec<&str> = chosen.iter().filter_map(|(_, _, u)| u.as_deref()).collect();
    let nodes: Vec<&str> = chosen.iter().map(|(n, _, _)| short_name(n)).collect();
    Some(format!("CUDA_VISIBLE_DEVICES={} ({})", uuids.join(","), nodes.join(", ")))
}

/// A session whose encoder runs on another card than its compositor (Intel draws, NVENC encodes: legitimate, and the thing that held the model's card on 2026-10-31).
pub fn encoder_card_warning(name: &str, render: Option<&str>, encoder_node: Option<&str>) -> Option<String> {
    match (render, encoder_node) {
        (Some(r), Some(e)) if short_name(r) != short_name(e) => Some(format!("{name} renders on {} but encodes on {}", short_name(r), short_name(e))),
        _ => None,
    }
}

/// A session drawing on a card whose role is inference only.
pub fn role_warning(name: &str, render: Option<&str>, roles: &HashMap<String, Role>) -> Option<String> {
    let r = render?;
    (roles.get(short_name(r)) == Some(&Role::Inference)).then(|| format!("{name} renders on {}, which is kept for inference (the session started before the roles changed, or the account is pinned there)", short_name(r)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_card_is_described_with_its_role_and_sessions() {
        assert_eq!(card_note(None, &[], None), "no sessions");
        assert_eq!(card_note(Some(Role::Inference), &["alice".into()], Some("GPU-1")), "inference only (new sessions do not pick it); 1 session: alice; for a model: CUDA_VISIBLE_DEVICES=GPU-1");
        assert_eq!(card_note(Some(Role::Render), &["a".into(), "b".into()], Some("GPU-2")), "sessions only; 2 sessions: a, b", "a render card is not offered to a model");
        assert_eq!(card_note(Some(Role::Both), &[], Some("GPU-3")), "sessions and inference; no sessions; for a model: CUDA_VISIBLE_DEVICES=GPU-3");
        assert_eq!(card_note(None, &[], Some("GPU-4")), "no sessions; for a model: CUDA_VISIBLE_DEVICES=GPU-4", "no roles: as before");
    }

    #[test]
    fn roles_come_from_the_roles_setting() {
        let roles = parse_roles("renderD128=render, /dev/dri/renderD129=AI, renderD130=both, renderD131=nonsense, bad");
        assert_eq!(roles.get("renderD128"), Some(&Role::Render));
        assert_eq!(roles.get("renderD129"), Some(&Role::Inference));
        assert_eq!(roles.get("renderD130"), Some(&Role::Both));
        assert_eq!(roles.len(), 3);
        assert_eq!(reserved_nodes(Some("renderD131=inference, renderD129=ai, renderD130=both")), vec!["renderD129".to_owned(), "renderD131".to_owned()]);
        assert!(reserved_nodes(None).is_empty());
    }

    #[test]
    fn one_line_names_every_card_a_model_may_use() {
        let cards = vec![
            ("/dev/dri/renderD129".to_owned(), Some(Role::Inference), Some("GPU-a".to_owned())),
            ("/dev/dri/renderD130".to_owned(), Some(Role::Both), Some("GPU-b".to_owned())),
            ("/dev/dri/renderD131".to_owned(), Some(Role::Render), Some("GPU-c".to_owned())),
        ];
        assert_eq!(inference_line(&cards).as_deref(), Some("CUDA_VISIBLE_DEVICES=GPU-a,GPU-b (renderD129, renderD130)"));
        assert_eq!(inference_line(&[("/dev/dri/renderD131".to_owned(), None, Some("GPU-c".to_owned()))]), None, "no roles in use");
    }

    #[test]
    fn a_session_on_another_card_than_its_encoder_or_on_an_inference_card_is_warned_about() {
        assert!(encoder_card_warning("lt1", Some("/dev/dri/renderD128"), Some("/dev/dri/renderD129")).unwrap().contains("renders on renderD128 but encodes on renderD129"));
        assert_eq!(encoder_card_warning("lt1", Some("/dev/dri/renderD128"), Some("/dev/dri/renderD128")), None);
        assert_eq!(encoder_card_warning("lt1", None, Some("/dev/dri/renderD129")), None, "software render: nothing to compare");
        let roles = parse_roles("renderD129=inference");
        assert!(role_warning("lt2", Some("/dev/dri/renderD129"), &roles).is_some());
        assert_eq!(role_warning("lt2", Some("/dev/dri/renderD128"), &roles), None);
    }
}
