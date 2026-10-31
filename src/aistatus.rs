//! What `waydesk-admin status` says about a local inference service (Ollama, vLLM, llama.cpp: anything that listens on a local port), when `WAYDESK_AI_ENDPOINT`
//! (`127.0.0.1:11434`) names one. Two things, both read from the machine and nothing proxied:
//!   - the connections each account has open to the service's port, from the kernel's TCP table (`/proc/net/tcp`: the owner of a client socket is the account that
//!     made the request; a streaming request keeps its connection open until the answer ends, so the count is "requests running or connections idle", not tokens);
//!   - what the service says about itself, if it is Ollama (`/api/ps`: the model that is loaded and how much of it is in video memory).
//! No token counts or history: that needs requests to pass through something that counts them. Plain functions over plain text so that it can be tested.
#![allow(dead_code)]

use std::collections::HashMap;
use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs as _};
use std::time::Duration;

/// One line of `/proc/net/tcp`.
#[derive(Clone, Debug, PartialEq)]
pub struct Conn {
    pub local_port: u16,
    pub remote_addr: u32,
    pub remote_port: u16,
    /// `01` is ESTABLISHED.
    pub state: u8,
    pub uid: u32,
}

/// `0100007F:2CAA 0100007F:D3E2 01 ... 1000 ...` -> the connection. The address is the kernel's little-endian hex of the IPv4 address.
pub fn parse_proc_net_tcp(text: &str) -> Vec<Conn> {
    let hex_port = |s: &str| u16::from_str_radix(s.split(':').nth(1)?, 16).ok();
    text.lines()
        .skip(1)
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            // sl local rem st tx:rx tr:when retrnsmt uid ...
            let remote = *f.get(2)?;
            Some(Conn {
                local_port: hex_port(f.get(1)?)?,
                remote_addr: u32::from_str_radix(remote.split(':').next()?, 16).ok()?,
                remote_port: hex_port(remote)?,
                state: u8::from_str_radix(f.get(3)?, 16).ok()?,
                uid: f.get(7)?.parse().ok()?,
            })
        })
        .collect()
}

/// `127.0.0.1:11434` -> (the IPv4 address as the kernel prints it, the port). A host that is not an IPv4 literal gives `None` for the address (any address matches).
pub fn parse_endpoint(s: &str) -> Option<(Option<u32>, u16)> {
    let (host, port) = s.trim().rsplit_once(':')?;
    let port: u16 = port.trim().parse().ok()?;
    let addr = host.trim().parse::<std::net::Ipv4Addr>().ok().map(|a| u32::from_le_bytes(a.octets()));
    Some((addr, port))
}

/// Connections per account to the service: established sockets whose far end is the service (the far end of a client's socket; the service's own sockets have it the
/// other way round and are not counted).
pub fn connections_by_uid(conns: &[Conn], addr: Option<u32>, port: u16) -> HashMap<u32, usize> {
    let mut m = HashMap::new();
    for c in conns.iter().filter(|c| c.state == 1 && c.remote_port == port && c.local_port != port && addr.map_or(true, |a| a == c.remote_addr)) {
        *m.entry(c.uid).or_insert(0) += 1;
    }
    m
}

/// A model the service has loaded.
#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    pub name: String,
    /// Its whole size and the part of it in video memory, MB.
    pub size_mb: u64,
    pub vram_mb: u64,
}

/// Ollama's `/api/ps`: `{"models":[{"name":"...","size":N,"size_vram":N}]}`. Anything else is an empty list.
pub fn parse_ollama_ps(body: &str) -> Vec<Model> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(body) else { return vec![] };
    v.get("models")
        .and_then(|m| m.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|m| {
                    Some(Model {
                        name: m.get("name")?.as_str()?.to_owned(),
                        size_mb: m.get("size").and_then(|x| x.as_u64()).unwrap_or(0) / 1_048_576,
                        vram_mb: m.get("size_vram").and_then(|x| x.as_u64()).unwrap_or(0) / 1_048_576,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// What the service said when asked.
#[derive(Clone, Debug, PartialEq)]
pub enum Probe {
    /// Nothing answered on the port.
    Down,
    /// Something answered, but not with Ollama's model list (vLLM, llama.cpp, or another program).
    Up,
    Models(Vec<Model>),
}

/// A plain HTTP GET with short timeouts: the body of a 200 answer.
fn http_get(addr: SocketAddr, path: &str) -> Option<(bool, String)> {
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_millis(500)).ok()?;
    s.set_read_timeout(Some(Duration::from_millis(1500))).ok()?;
    s.set_write_timeout(Some(Duration::from_millis(500))).ok()?;
    s.write_all(format!("GET {path} HTTP/1.0\r\nHost: {}\r\nConnection: close\r\n\r\n", addr.ip()).as_bytes()).ok()?;
    let mut out = Vec::new();
    let _ = s.take(1 << 20).read_to_end(&mut out);
    let text = String::from_utf8_lossy(&out).into_owned();
    let ok = text.starts_with("HTTP/1.0 200") || text.starts_with("HTTP/1.1 200");
    Some((ok, text.split_once("\r\n\r\n").map(|(_, b)| b.to_owned()).unwrap_or_default()))
}

pub fn probe(endpoint: &str) -> Probe {
    let Some(addr) = endpoint.trim().to_socket_addrs().ok().and_then(|mut a| a.next()) else { return Probe::Down };
    match http_get(addr, "/api/ps") {
        None => Probe::Down,
        Some((true, body)) if body.contains("\"models\"") => Probe::Models(parse_ollama_ps(&body)),
        Some(_) => Probe::Up,
    }
}

/// The machine line: `127.0.0.1:11434: llama3.1:8b 6.7 GB in video memory of 6.7 GB, 2 connections open`.
pub fn describe(endpoint: &str, probe: &Probe, open: usize) -> String {
    let gb = |mb: u64| if mb >= 1024 { format!("{:.1} GB", mb as f64 / 1024.0) } else { format!("{mb} MB") };
    let what = match probe {
        Probe::Down => return format!("{endpoint}: not answering"),
        Probe::Up => "answering (no model list: not Ollama)".to_owned(),
        Probe::Models(m) if m.is_empty() => "answering, no model loaded".to_owned(),
        Probe::Models(m) => m
            .iter()
            .map(|m| if m.vram_mb >= m.size_mb { format!("{} {} in video memory", m.name, gb(m.vram_mb)) } else { format!("{} {} of {} in video memory (the rest on the CPU)", m.name, gb(m.vram_mb), gb(m.size_mb)) })
            .collect::<Vec<_>>()
            .join(", "),
    };
    format!("{endpoint}: {what}; {open} connection{} open", if open == 1 { "" } else { "s" })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TCP: &str = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n\
   0: 0100007F:2CAA 00000000:0000 0A 00000000:00000000 00:00000000 00000000   999        0 11111 1 0000 100 0 0 10 0\n\
   1: 0100007F:D3E2 0100007F:2CAA 01 00000000:00000000 00:00000000 00000000  1000        0 22222 1 0000 20 4 20 10 -1\n\
   2: 0100007F:2CAA 0100007F:D3E2 01 00000000:00000000 00:00000000 00000000   999        0 11112 1 0000 20 4 20 10 -1\n\
   3: 0100007F:D3E4 0100007F:2CAA 01 00000000:00000000 00:00000000 00000000  1000        0 22223 1 0000 20 4 20 10 -1\n\
   4: 0100007F:D3E6 0100007F:2CAA 01 00000000:00000000 00:00000000 00000000  1001        0 22224 1 0000 20 4 20 10 -1\n\
   5: 0100007F:D3E8 0100007F:2CAA 06 00000000:00000000 00:00000000 00000000  1001        0 22225 1 0000 20 4 20 10 -1\n\
   6: 0100007F:D3EA 0100007F:0050 01 00000000:00000000 00:00000000 00000000  1001        0 22226 1 0000 20 4 20 10 -1\n\
   7: 0100007F:D3EC 0200A8C0:2CAA 01 00000000:00000000 00:00000000 00000000     0        0 22227 1 0000 20 4 20 10 -1\n";

    #[test]
    fn the_tcp_table_is_read() {
        let c = parse_proc_net_tcp(TCP);
        assert_eq!(c.len(), 8);
        assert_eq!((c[1].local_port, c[1].remote_port, c[1].state, c[1].uid, c[1].remote_addr), (0xD3E2, 0x2CAA, 1, 1000, 0x0100007F));
        assert!(parse_proc_net_tcp("").is_empty());
        assert!(parse_proc_net_tcp("header\ngarbage line\n").is_empty());
    }

    #[test]
    fn the_endpoint_is_read_as_the_kernel_prints_it() {
        assert_eq!(parse_endpoint("127.0.0.1:11434"), Some((Some(0x0100007F), 11434)));
        assert_eq!(parse_endpoint(" localhost:8000 "), Some((None, 8000)));
        assert_eq!(parse_endpoint("nonsense"), None);
        assert_eq!(parse_endpoint("127.0.0.1:99999"), None);
    }

    #[test]
    fn connections_are_counted_per_account_on_the_client_side_only() {
        let c = parse_proc_net_tcp(TCP);
        let m = connections_by_uid(&c, Some(0x0100007F), 0x2CAA);
        // uid 1000 has two, 1001 one (the other of its sockets is closing: state 06; the one to port 80 is not the service), the service's own sockets (uid 999) are not counted,
        // and the connection to another address (192.168.0.2) is not the service
        assert_eq!(m.get(&1000), Some(&2));
        assert_eq!(m.get(&1001), Some(&1));
        assert_eq!(m.get(&999), None);
        assert_eq!(m.get(&0), None);
        // no address given: any address with that port counts, so the 192.168.0.2 connection of root does
        assert_eq!(connections_by_uid(&c, None, 0x2CAA).get(&0), Some(&1));
    }

    #[test]
    fn ollamas_model_list_is_read() {
        let body = r#"{"models":[{"name":"llama3.1:8b","size":7516192768,"size_vram":7516192768},{"name":"big","size":21474836480,"size_vram":10737418240}]}"#;
        let m = parse_ollama_ps(body);
        assert_eq!(m, vec![Model { name: "llama3.1:8b".into(), size_mb: 7168, vram_mb: 7168 }, Model { name: "big".into(), size_mb: 20480, vram_mb: 10240 }]);
        assert!(parse_ollama_ps("{}").is_empty() && parse_ollama_ps("not json").is_empty() && parse_ollama_ps(r#"{"models":[]}"#).is_empty());
    }

    #[test]
    fn the_line_says_what_is_loaded_and_how_many_are_connected() {
        let ok = Probe::Models(vec![Model { name: "m".into(), size_mb: 7168, vram_mb: 7168 }]);
        assert_eq!(describe("127.0.0.1:11434", &ok, 2), "127.0.0.1:11434: m 7.0 GB in video memory; 2 connections open");
        let spill = Probe::Models(vec![Model { name: "big".into(), size_mb: 20480, vram_mb: 10240 }]);
        assert!(describe("e", &spill, 1).contains("big 10.0 GB of 20.0 GB in video memory (the rest on the CPU); 1 connection open"));
        assert_eq!(describe("e", &Probe::Down, 0), "e: not answering");
        assert!(describe("e", &Probe::Up, 0).contains("not Ollama"));
        assert!(describe("e", &Probe::Models(vec![]), 0).contains("no model loaded"));
    }

    #[test]
    fn a_service_that_answers_is_probed_over_http() {
        use std::io::{Read as _, Write as _};
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let t = std::thread::spawn(move || {
            for body in [r#"{"models":[{"name":"m","size":1048576,"size_vram":1048576}]}"#, "<html>nope</html>"] {
                let (mut s, _) = l.accept().unwrap();
                let mut buf = [0u8; 512];
                let _ = s.read(&mut buf);
                let _ = s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}", body.len()).as_bytes());
            }
        });
        assert_eq!(probe(&addr.to_string()), Probe::Models(vec![Model { name: "m".into(), size_mb: 1, vram_mb: 1 }]));
        assert_eq!(probe(&addr.to_string()), Probe::Up, "answers, but not with a model list");
        t.join().unwrap();
        assert_eq!(probe("127.0.0.1:1"), Probe::Down);
    }
}
