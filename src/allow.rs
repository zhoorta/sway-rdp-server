//! Which source addresses may open a connection at all. Checked as soon as a connection is accepted, before TLS and before any login: an
//! address that is not allowed gets nothing (the socket is closed), so scanners and brute force from elsewhere never reach the login.
//!
//! The list is made of CIDR ranges (`192.168.1.0/24`, `fd00::/8`), single addresses, and the words `private` (the addresses of local networks:
//! 10/8, 172.16/12, 192.168/16, 100.64/10 which Tailscale uses, link-local, `fc00::/7`, `fe80::/10` and loopback), `loopback` and `any`.
//! Without a list a daemon on loopback allows anything (only this machine can reach it) and a daemon on any other address allows `private`.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

#[derive(Debug, Clone, PartialEq)]
enum Net {
    V4(u32, u8),
    V6(u128, u8),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Allow {
    nets: Vec<Net>,
    any: bool,
}

fn v4(a: u8, b: u8, c: u8, d: u8) -> u32 {
    u32::from(Ipv4Addr::new(a, b, c, d))
}

impl Net {
    fn contains(&self, ip: IpAddr) -> bool {
        // A connection on a socket that listens on both families comes as an IPv4-mapped IPv6 address.
        let ip = match ip {
            IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(IpAddr::V6(v6), IpAddr::V4),
            v4 => v4,
        };
        match (self, ip) {
            (Net::V4(net, prefix), IpAddr::V4(a)) => {
                let mask = if *prefix == 0 { 0 } else { u32::MAX << (32 - prefix) };
                u32::from(a) & mask == net & mask
            }
            (Net::V6(net, prefix), IpAddr::V6(a)) => {
                let mask = if *prefix == 0 { 0 } else { u128::MAX << (128 - prefix) };
                u128::from(a) & mask == net & mask
            }
            _ => false,
        }
    }
}

impl Allow {
    pub fn any() -> Self {
        Allow { nets: Vec::new(), any: true }
    }

    /// The addresses of local networks and of this machine.
    fn private_nets() -> Vec<Net> {
        vec![
            Net::V4(v4(10, 0, 0, 0), 8),
            Net::V4(v4(172, 16, 0, 0), 12),
            Net::V4(v4(192, 168, 0, 0), 16),
            Net::V4(v4(100, 64, 0, 0), 10),
            Net::V4(v4(169, 254, 0, 0), 16),
            Net::V4(v4(127, 0, 0, 0), 8),
            Net::V6(u128::from(Ipv6Addr::new(0xfc00, 0, 0, 0, 0, 0, 0, 0)), 7),
            Net::V6(u128::from(Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 0)), 10),
            Net::V6(1, 128),
        ]
    }

    fn loopback_nets() -> Vec<Net> {
        vec![Net::V4(v4(127, 0, 0, 0), 8), Net::V6(1, 128)]
    }

    /// `192.168.1.0/24`, `10.8.0.2`, `fd00::/8`, `private`, `loopback` or `any`, separated by commas or spaces.
    pub fn parse(list: &str) -> Result<Self, String> {
        let mut allow = Allow { nets: Vec::new(), any: false };
        for item in list.split(|c: char| c == ',' || c.is_whitespace()).filter(|s| !s.is_empty()) {
            match item {
                "any" => allow.any = true,
                "private" => allow.nets.extend(Self::private_nets()),
                "loopback" => allow.nets.extend(Self::loopback_nets()),
                _ => allow.nets.push(parse_net(item)?),
            }
        }
        if !allow.any && allow.nets.is_empty() {
            return Err("the list of allowed addresses is empty (say `any` to allow every address)".into());
        }
        Ok(allow)
    }

    /// The list to use: the one given (a daemon that cannot read it must not start), or the default for the address it listens on.
    pub fn from_config(given: Option<&str>, bind: &SocketAddr) -> Result<Self, String> {
        match given.map(str::trim).filter(|s| !s.is_empty()) {
            Some(list) => Self::parse(list),
            None if bind.ip().is_loopback() => Ok(Self::any()),
            None => Self::parse("private"),
        }
    }

    pub fn permits(&self, ip: IpAddr) -> bool {
        self.any || self.nets.iter().any(|n| n.contains(ip))
    }

    pub fn is_any(&self) -> bool {
        self.any
    }
}

fn parse_net(item: &str) -> Result<Net, String> {
    let (addr, prefix) = match item.split_once('/') {
        Some((a, p)) => (a, Some(p.parse::<u8>().map_err(|_| format!("{item}: the prefix length is not a number"))?)),
        None => (item, None),
    };
    match addr.parse::<IpAddr>().map_err(|_| format!("{item}: not an address, a range, or one of private, loopback, any"))? {
        IpAddr::V4(a) => {
            let prefix = prefix.unwrap_or(32);
            if prefix > 32 {
                return Err(format!("{item}: an IPv4 prefix is at most 32"));
            }
            Ok(Net::V4(u32::from(a), prefix))
        }
        IpAddr::V6(a) => {
            let prefix = prefix.unwrap_or(128);
            if prefix > 128 {
                return Err(format!("{item}: an IPv6 prefix is at most 128"));
            }
            Ok(Net::V6(u128::from(a), prefix))
        }
    }
}

/// Says "refused" for an address, at most once every ten seconds for the whole daemon (a scan would fill the log), with how many were
/// refused meanwhile.
pub struct RefusalLog {
    last: std::sync::Mutex<Option<std::time::Instant>>,
    suppressed: std::sync::atomic::AtomicU64,
}

impl RefusalLog {
    pub const fn new() -> Self {
        RefusalLog { last: std::sync::Mutex::new(None), suppressed: std::sync::atomic::AtomicU64::new(0) }
    }

    /// None when this refusal is not to be logged, else the number of refusals since the last line that was.
    pub fn note(&self) -> Option<u64> {
        let now = std::time::Instant::now();
        let mut last = self.last.lock().unwrap();
        if last.is_some_and(|t| now.duration_since(t) < std::time::Duration::from_secs(10)) {
            self.suppressed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return None;
        }
        *last = Some(now);
        Some(self.suppressed.swap(0, std::sync::atomic::Ordering::Relaxed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn ranges_and_single_addresses() {
        let a = Allow::parse("192.168.1.0/24, 10.8.0.2 fd00::/8").unwrap();
        assert!(a.permits(ip("192.168.1.77")));
        assert!(!a.permits(ip("192.168.2.1")));
        assert!(a.permits(ip("10.8.0.2")));
        assert!(!a.permits(ip("10.8.0.3")));
        assert!(a.permits(ip("fd12:3456::1")));
        assert!(!a.permits(ip("2001:db8::1")));
    }

    #[test]
    fn private_means_local_networks() {
        let a = Allow::parse("private").unwrap();
        for yes in ["10.1.2.3", "172.16.0.1", "172.31.255.255", "192.168.0.1", "100.100.1.1", "169.254.1.1", "127.0.0.1", "::1", "fe80::1", "fd00::5"] {
            assert!(a.permits(ip(yes)), "{yes}");
        }
        for no in ["8.8.8.8", "172.32.0.1", "100.128.0.1", "2001:db8::1", "192.169.0.1"] {
            assert!(!a.permits(ip(no)), "{no}");
        }
    }

    #[test]
    fn an_ipv4_address_on_a_dual_stack_socket_is_still_ipv4() {
        let a = Allow::parse("192.168.1.0/24").unwrap();
        assert!(a.permits(ip("::ffff:192.168.1.5")));
        assert!(!a.permits(ip("::ffff:192.168.2.5")));
    }

    #[test]
    fn the_default_depends_on_where_the_daemon_listens() {
        let lo: SocketAddr = "127.0.0.1:3389".parse().unwrap();
        let lan: SocketAddr = "192.168.1.10:3389".parse().unwrap();
        let all: SocketAddr = "0.0.0.0:3389".parse().unwrap();
        assert!(Allow::from_config(None, &lo).unwrap().is_any());
        for bind in [lan, all] {
            let a = Allow::from_config(None, &bind).unwrap();
            assert!(a.permits(ip("192.168.1.99")) && !a.permits(ip("8.8.8.8")));
        }
        assert!(Allow::from_config(Some("any"), &lan).unwrap().permits(ip("8.8.8.8")));
        assert!(Allow::from_config(Some("  "), &lan).unwrap().permits(ip("10.0.0.1")) && !Allow::from_config(Some(""), &lan).unwrap().permits(ip("8.8.8.8")));
    }

    #[test]
    fn bad_lists_are_refused_so_that_the_daemon_does_not_start_open() {
        for bad in ["not-an-address", "10.0.0.0/33", "10.0.0.0/x", "::1/129", "10.0.0.0/8 ,, bogus"] {
            assert!(Allow::parse(bad).is_err(), "{bad}");
        }
        assert!(Allow::parse(",").is_err());
    }

    #[test]
    fn the_prefix_zero_and_the_whole_address_work() {
        assert!(Allow::parse("0.0.0.0/0").unwrap().permits(ip("1.2.3.4")));
        assert!(Allow::parse("1.2.3.4/32").unwrap().permits(ip("1.2.3.4")));
        assert!(!Allow::parse("1.2.3.4/32").unwrap().permits(ip("1.2.3.5")));
    }

    #[test]
    fn refusals_are_logged_once_in_a_while() {
        let log = RefusalLog::new();
        assert_eq!(log.note(), Some(0));
        assert_eq!(log.note(), None);
        assert_eq!(log.note(), None);
    }
}
