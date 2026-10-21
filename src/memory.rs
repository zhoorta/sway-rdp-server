//! The memory of the machine, and the rule that decides whether a new session may start. The helper enforces it (`helper.rs`) and
//! `waydesk-admin status` shows it, so both use this file.
#![allow(dead_code)]

/// What `/proc/meminfo` says, in kB.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Meminfo {
    pub total_kb: u64,
    pub available_kb: u64,
    pub swap_total_kb: u64,
    pub swap_free_kb: u64,
}

pub fn parse_meminfo(text: &str) -> Option<Meminfo> {
    let get = |key: &str| text.lines().find_map(|l| l.strip_prefix(key)?.trim_start_matches(':').split_whitespace().next()?.parse::<u64>().ok());
    Some(Meminfo {
        total_kb: get("MemTotal")?,
        available_kb: get("MemAvailable")?,
        swap_total_kb: get("SwapTotal").unwrap_or(0),
        swap_free_kb: get("SwapFree").unwrap_or(0),
    })
}

pub fn read_meminfo() -> Option<Meminfo> {
    parse_meminfo(&std::fs::read_to_string("/proc/meminfo").ok()?)
}

/// The least available memory (MB) a new session needs when nothing says otherwise: the larger of 900 MB and 15 % of the memory.
/// A session costs about 430 MB at once and a browser with a few pages another 270 MB (measured on a 4 GB machine, where this
/// stops the fifth user); the rest is room for the others to grow.
pub fn auto_min_available_mb(total_kb: u64) -> u64 {
    (total_kb / 1024 * 15 / 100).max(900)
}

#[derive(Debug, PartialEq)]
pub enum Admission {
    Open,
    /// Not enough memory is available for one more session.
    Full { available_mb: u64, needed_mb: u64 },
    /// The number of accounts with a session is at the limit.
    TooManyUsers { users: usize, max: usize },
}

/// Whether an account without a session may start one. `min_available_mb`: None is the automatic value, Some(0) switches the
/// memory check off; `max_users` 0 is no limit. Swap is not counted: it holds pages nobody uses, and a machine that swaps the
/// sessions in use is as good as down.
pub fn admission(info: &Meminfo, running_users: usize, min_available_mb: Option<u64>, max_users: usize) -> Admission {
    if max_users > 0 && running_users >= max_users {
        return Admission::TooManyUsers { users: running_users, max: max_users };
    }
    let needed_mb = min_available_mb.unwrap_or_else(|| auto_min_available_mb(info.total_kb));
    let available_mb = info.available_kb / 1024;
    if needed_mb > 0 && available_mb < needed_mb {
        return Admission::Full { available_mb, needed_mb };
    }
    Admission::Open
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(total_mb: u64, available_mb: u64) -> Meminfo {
        Meminfo { total_kb: total_mb * 1024, available_kb: available_mb * 1024, swap_total_kb: 0, swap_free_kb: 0 }
    }

    #[test]
    fn meminfo_is_read_from_the_text_of_proc() {
        let m = parse_meminfo("MemTotal:        4012792 kB\nMemFree:          108564 kB\nMemAvailable:     830936 kB\nSwapTotal:       2006396 kB\nSwapFree:        2006000 kB\n").unwrap();
        assert_eq!((m.total_kb, m.available_kb, m.swap_total_kb, m.swap_free_kb), (4012792, 830936, 2006396, 2006000));
        assert!(parse_meminfo("MemTotal: 1 kB\n").is_none(), "no MemAvailable: not trusted");
        assert_eq!(parse_meminfo("MemTotal: 10 kB\nMemAvailable: 5 kB\n").unwrap().swap_total_kb, 0);
    }

    #[test]
    fn the_automatic_minimum_is_900_mb_or_15_percent() {
        assert_eq!(auto_min_available_mb(2048 * 1024), 900);
        assert_eq!(auto_min_available_mb(4012792), 900);
        assert_eq!(auto_min_available_mb(16384 * 1024), 2457);
    }

    #[test]
    fn a_session_starts_only_with_enough_memory_and_room_for_the_user() {
        // what the 4 GB machine showed with three and with four users that had a browser open
        assert_eq!(admission(&info(3918, 1452), 3, None, 0), Admission::Open);
        assert_eq!(admission(&info(3918, 830), 4, None, 0), Admission::Full { available_mb: 830, needed_mb: 900 });
        assert_eq!(admission(&info(3918, 830), 4, Some(500), 0), Admission::Open);
        assert_eq!(admission(&info(3918, 10), 9, Some(0), 0), Admission::Open, "0 switches the check off");
        assert_eq!(admission(&info(3918, 3000), 4, None, 4), Admission::TooManyUsers { users: 4, max: 4 });
        assert_eq!(admission(&info(3918, 3000), 3, None, 4), Admission::Open);
    }
}
