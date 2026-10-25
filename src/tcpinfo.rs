//! What the kernel knows about a TCP connection (`getsockopt(TCP_INFO)`): the rate it has been delivering, and whether that rate was held
//! down by the sender having nothing more to send (then it is not what the link can carry).

use std::os::fd::RawFd;

/// The start of `struct tcp_info` of Linux (include/uapi/linux/tcp.h), as far as `delivery_rate`. The kernel fills as much of the structure
/// as it knows, so a larger one is fine and an older kernel leaves the rest at zero.
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct TcpInfo {
    state: u8,
    ca_state: u8,
    retransmits: u8,
    probes: u8,
    backoff: u8,
    options: u8,
    wscale: u8,
    /// Bit 0: the delivery rate sample was limited by the application (it had nothing more to send).
    flags: u8,
    rto: u32,
    ato: u32,
    snd_mss: u32,
    rcv_mss: u32,
    unacked: u32,
    sacked: u32,
    lost: u32,
    retrans: u32,
    fackets: u32,
    last_data_sent: u32,
    last_ack_sent: u32,
    last_data_recv: u32,
    last_ack_recv: u32,
    pmtu: u32,
    rcv_ssthresh: u32,
    rtt: u32,
    rttvar: u32,
    snd_ssthresh: u32,
    snd_cwnd: u32,
    advmss: u32,
    reordering: u32,
    rcv_rtt: u32,
    rcv_space: u32,
    total_retrans: u32,
    pacing_rate: u64,
    max_pacing_rate: u64,
    bytes_acked: u64,
    bytes_received: u64,
    segs_out: u32,
    segs_in: u32,
    notsent_bytes: u32,
    min_rtt: u32,
    data_segs_in: u32,
    data_segs_out: u32,
    delivery_rate: u64,
}

/// What the connection has been delivering.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Link {
    /// Bytes per second.
    pub delivery_rate: f64,
    /// The sender had nothing more to send: the rate is a lower bound, not what the link carries.
    pub app_limited: bool,
}

/// The kernel's numbers for the connection on `fd`, None when it cannot say (not a TCP socket, an old kernel).
pub fn read(fd: RawFd) -> Option<Link> {
    let mut info = TcpInfo::default();
    let mut len = std::mem::size_of::<TcpInfo>() as libc::socklen_t;
    // SAFETY: the buffer is a TcpInfo of the length given.
    let r = unsafe { libc::getsockopt(fd, libc::IPPROTO_TCP, libc::TCP_INFO, (&raw mut info).cast(), &mut len) };
    if r != 0 || (len as usize) < std::mem::offset_of!(TcpInfo, delivery_rate) + 8 {
        return None;
    }
    Some(Link { delivery_rate: info.delivery_rate as f64, app_limited: info.flags & 1 != 0 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_layout_is_the_kernels() {
        assert_eq!(std::mem::offset_of!(TcpInfo, rto), 8);
        assert_eq!(std::mem::offset_of!(TcpInfo, pacing_rate), 104);
        assert_eq!(std::mem::offset_of!(TcpInfo, segs_out), 136);
        assert_eq!(std::mem::offset_of!(TcpInfo, notsent_bytes), 144);
        assert_eq!(std::mem::offset_of!(TcpInfo, delivery_rate), 160);
    }

    #[test]
    fn a_loopback_connection_can_be_read() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let client = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (_server, _) = listener.accept().unwrap();
        use std::os::fd::AsRawFd;
        assert!(read(client.as_raw_fd()).is_some());
        assert!(read(-1).is_none());
    }
}
