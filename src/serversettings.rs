//! Editing the settings file of the server (`/etc/default/waydesk`) the way `waydesk-admin gpu roles` and `waydesk-admin ai endpoint` do: one `KEY=value` line is
//! replaced or removed, every other line (and every comment, including the commented examples) stays as it is.

pub const FILE: &str = "/etc/default/waydesk";

/// `text` without the active lines of `key` (`KEY=...`; a line that starts with `#` is a comment and stays) and, when `value` is given, with `KEY=value` as the last line:
/// the file is read with the last value winning, so the new one is the one that counts however many lines the key had before.
pub fn edit(text: &str, key: &str, value: Option<&str>) -> String {
    let prefix = format!("{key}=");
    let mut out = String::with_capacity(text.len() + 64);
    for line in text.lines() {
        if !line.trim_start().starts_with(&prefix) {
            out.push_str(line);
            out.push('\n');
        }
    }
    if let Some(v) = value {
        out.push_str(&format!("{key}={v}\n"));
    }
    out
}

/// Whether `value` is safe to write on a line of the file: printable ASCII without spaces, quotes, `#`, `$`, backslashes or backticks (the file is read by systemd and by shells).
pub fn is_plain_value(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|b| b.is_ascii_graphic() && !matches!(b, b'"' | b'\'' | b'#' | b'$' | b'\\' | b'`'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_the_line_and_keeps_the_rest() {
        let text = "# settings\nWAYDESK_GPU=auto\nWAYDESK_GPU_ROLES=renderD128=render\n#WAYDESK_GPU_ROLES=renderD128=render,renderD129=inference\nOTHER=1\n";
        let new = edit(text, "WAYDESK_GPU_ROLES", Some("renderD129=inference"));
        assert_eq!(new, "# settings\nWAYDESK_GPU=auto\n#WAYDESK_GPU_ROLES=renderD128=render,renderD129=inference\nOTHER=1\nWAYDESK_GPU_ROLES=renderD129=inference\n");
    }

    #[test]
    fn removes_every_active_line_of_the_key_only() {
        let text = "A=1\nWAYDESK_AI_ENDPOINT=127.0.0.1:1\n  WAYDESK_AI_ENDPOINT=127.0.0.1:2\n#WAYDESK_AI_ENDPOINT=127.0.0.1:11434\nWAYDESK_AI_ENDPOINT_OTHER=x\n";
        assert_eq!(edit(text, "WAYDESK_AI_ENDPOINT", None), "A=1\n#WAYDESK_AI_ENDPOINT=127.0.0.1:11434\nWAYDESK_AI_ENDPOINT_OTHER=x\n");
    }

    #[test]
    fn adds_to_a_file_without_the_key_and_to_an_empty_one() {
        assert_eq!(edit("A=1\n", "K", Some("v")), "A=1\nK=v\n");
        assert_eq!(edit("", "K", Some("v")), "K=v\n");
        assert_eq!(edit("A=1", "K", Some("v")), "A=1\nK=v\n");
        assert_eq!(edit("", "K", None), "");
    }

    #[test]
    fn a_key_that_is_a_prefix_of_another_is_not_touched() {
        // `WAYDESK_GPU=` must not take `WAYDESK_GPU_ROLES=` with it
        assert_eq!(edit("WAYDESK_GPU=off\nWAYDESK_GPU_ROLES=x=y\n", "WAYDESK_GPU", None), "WAYDESK_GPU_ROLES=x=y\n");
    }

    #[test]
    fn plain_values() {
        assert!(is_plain_value("renderD128=render,renderD129=inference"));
        assert!(is_plain_value("127.0.0.1:11434"));
        for bad in ["", "a b", "a\nb", "a\"b", "a'b", "a#b", "a$b", "a`b", "a\\b", "é"] {
            assert!(!is_plain_value(bad), "{bad:?}");
        }
    }
}
