//! Passing one file descriptor over a Unix stream socket (SCM_RIGHTS). The helper hands the daemon its end of a
//! session socket this way.
//!
//! The descriptor always travels on a message of its own: one byte with the descriptor attached, sent right after
//! the ordinary framed answer. A stream socket may split or join ordinary bytes, but ancillary data stays with the
//! byte it was sent with, so reading exactly one byte gets exactly the descriptor.

use std::io;
use std::mem::{size_of, zeroed};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;

/// Space for the control message of one descriptor, aligned for `cmsghdr`.
#[repr(C)]
union Control {
    buf: [u8; 64],
    _align: libc::cmsghdr,
}

pub fn send_fd(sock: &UnixStream, fd: RawFd) -> io::Result<()> {
    let mut byte = [0u8; 1];
    let mut iov = libc::iovec { iov_base: byte.as_mut_ptr().cast(), iov_len: 1 };
    let mut control: Control = unsafe { zeroed() };
    let mut msg: libc::msghdr = unsafe { zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = unsafe { control.buf.as_mut_ptr() }.cast();
    msg.msg_controllen = unsafe { libc::CMSG_SPACE(size_of::<RawFd>() as u32) } as _;
    unsafe {
        let cmsg = libc::CMSG_FIRSTHDR(&msg);
        (*cmsg).cmsg_level = libc::SOL_SOCKET;
        (*cmsg).cmsg_type = libc::SCM_RIGHTS;
        (*cmsg).cmsg_len = libc::CMSG_LEN(size_of::<RawFd>() as u32) as _;
        std::ptr::copy_nonoverlapping(&fd as *const RawFd as *const u8, libc::CMSG_DATA(cmsg), size_of::<RawFd>());
        loop {
            let n = libc::sendmsg(sock.as_raw_fd(), &msg, libc::MSG_NOSIGNAL);
            if n == 1 {
                return Ok(());
            }
            let err = io::Error::last_os_error();
            if n < 0 && err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(if n < 0 { err } else { io::Error::other("descriptor message not sent") });
        }
    }
}

pub fn recv_fd(sock: &UnixStream) -> io::Result<OwnedFd> {
    let mut byte = [0u8; 1];
    let mut iov = libc::iovec { iov_base: byte.as_mut_ptr().cast(), iov_len: 1 };
    let mut control: Control = unsafe { zeroed() };
    let mut msg: libc::msghdr = unsafe { zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = unsafe { control.buf.as_mut_ptr() }.cast();
    msg.msg_controllen = unsafe { libc::CMSG_SPACE(size_of::<RawFd>() as u32) } as _;
    unsafe {
        // MSG_CMSG_CLOEXEC: the descriptor must not leak into programs started later.
        let n = loop {
            let n = libc::recvmsg(sock.as_raw_fd(), &mut msg, libc::MSG_CMSG_CLOEXEC);
            if n < 0 && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                continue;
            }
            break n;
        };
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        if n == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "peer closed before sending a descriptor"));
        }
        let cmsg = libc::CMSG_FIRSTHDR(&msg);
        if cmsg.is_null() || (*cmsg).cmsg_level != libc::SOL_SOCKET || (*cmsg).cmsg_type != libc::SCM_RIGHTS {
            return Err(io::Error::other("no descriptor in the message"));
        }
        let mut fd: RawFd = -1;
        std::ptr::copy_nonoverlapping(libc::CMSG_DATA(cmsg), &mut fd as *mut RawFd as *mut u8, size_of::<RawFd>());
        if fd < 0 {
            return Err(io::Error::other("bad descriptor"));
        }
        Ok(OwnedFd::from_raw_fd(fd))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    #[test]
    fn a_descriptor_arrives_and_works() {
        let (a, b) = UnixStream::pair().unwrap();
        // The descriptor to pass: one end of another socket pair.
        let (x, mut y) = UnixStream::pair().unwrap();
        send_fd(&a, x.as_raw_fd()).unwrap();
        drop(x);
        let got = recv_fd(&b).unwrap();
        let mut received = UnixStream::from(got);
        y.write_all(b"hi").unwrap();
        let mut buf = [0u8; 2];
        received.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"hi");
    }

    #[test]
    fn a_plain_byte_without_descriptor_is_an_error() {
        let (mut a, b) = UnixStream::pair().unwrap();
        a.write_all(&[1]).unwrap();
        assert!(recv_fd(&b).is_err());
    }

    #[test]
    fn closed_peer_is_an_error() {
        let (a, b) = UnixStream::pair().unwrap();
        drop(a);
        assert!(recv_fd(&b).is_err());
    }
}
