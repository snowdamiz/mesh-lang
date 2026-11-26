use std::io;
use std::mem::{size_of, size_of_val, zeroed};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;
use std::ptr;
use std::time::Duration;

use crate::db::pg::jobs_effect_session::IO_TIMEOUT;

pub(in crate::jobs::worker) const WORKER_ARG: &str = "--mesh-internal-job-attempt-effect-v1";
const PRELUDE: u8 = 1;
const FD_SIZE: usize = size_of::<RawFd>();
const RECEIVE_SPACE: usize = unsafe { libc::CMSG_SPACE((2 * FD_SIZE) as u32) as usize };
const SEND_SPACE: usize = unsafe { libc::CMSG_SPACE((4 * FD_SIZE) as u32) as usize };

#[repr(C)]
struct Ancillary<const N: usize> {
    align: [libc::cmsghdr; 0],
    bytes: [u8; N],
}

impl<const N: usize> Ancillary<N> {
    fn new() -> Self {
        Self {
            align: [],
            bytes: [0; N],
        }
    }
}

pub(in crate::jobs::worker) fn send(control: &UnixStream, endpoint: &OwnedFd) -> io::Result<()> {
    send_frame(control, PRELUDE, &[endpoint.as_raw_fd()])
}

fn send_frame(control: &UnixStream, mut byte: u8, fds: &[RawFd]) -> io::Result<()> {
    let bytes = size_of_val(fds);
    let space = unsafe { libc::CMSG_SPACE(bytes as u32) } as usize;
    let mut ancillary = Ancillary::<SEND_SPACE>::new();
    assert!(space <= ancillary.bytes.len() && !fds.is_empty());
    let mut iov = libc::iovec {
        iov_base: ptr::from_mut(&mut byte).cast(),
        iov_len: 1,
    };
    let mut message: libc::msghdr = unsafe { zeroed() };
    message.msg_iov = &mut iov;
    message.msg_iovlen = 1;
    message.msg_control = ancillary.bytes.as_mut_ptr().cast();
    message.msg_controllen = space as _;
    // The aligned buffer has room for the header and all the supplied FDs.
    let sent = unsafe {
        let header = libc::CMSG_FIRSTHDR(&message);
        (*header).cmsg_len = libc::CMSG_LEN(bytes as u32) as _;
        (*header).cmsg_level = libc::SOL_SOCKET;
        (*header).cmsg_type = libc::SCM_RIGHTS;
        ptr::copy_nonoverlapping(fds.as_ptr().cast::<u8>(), libc::CMSG_DATA(header), bytes);
        libc::sendmsg(
            control.as_raw_fd(),
            &message,
            libc::MSG_NOSIGNAL | libc::MSG_DONTWAIT,
        )
    };
    match sent {
        1 => Ok(()),
        -1 => Err(io::Error::last_os_error()),
        _ => Err(io::ErrorKind::WriteZero.into()),
    }
}

pub(in crate::jobs::worker) fn receive_stdin() -> io::Result<UnixStream> {
    let stdin = libc::STDIN_FILENO;
    // The spawn file action clears CLOEXEC on fd 0. Keep descendants from
    // retaining a copy across exec and hiding the runner's eventual EOF.
    if unsafe { libc::fcntl(stdin, libc::F_SETFD, libc::FD_CLOEXEC) } == -1 {
        return Err(io::Error::last_os_error());
    }
    receive(stdin, IO_TIMEOUT)
}

fn receive(control: RawFd, timeout: Duration) -> io::Result<UnixStream> {
    let mut waiting = libc::pollfd {
        fd: control,
        events: libc::POLLIN,
        revents: 0,
    };
    match unsafe { libc::poll(&mut waiting, 1, timeout.as_millis() as i32) } {
        -1 => return Err(io::Error::last_os_error()),
        0 => return Err(io::ErrorKind::TimedOut.into()),
        _ => {}
    }

    let mut byte = 0u8;
    let mut ancillary = Ancillary::<RECEIVE_SPACE>::new();
    let mut iov = libc::iovec {
        iov_base: ptr::from_mut(&mut byte).cast(),
        iov_len: 1,
    };
    let mut message: libc::msghdr = unsafe { zeroed() };
    message.msg_iov = &mut iov;
    message.msg_iovlen = 1;
    message.msg_control = ancillary.bytes.as_mut_ptr().cast();
    message.msg_controllen = ancillary.bytes.len() as _;
    let received = unsafe {
        libc::recvmsg(
            control,
            &mut message,
            libc::MSG_CMSG_CLOEXEC | libc::MSG_DONTWAIT,
        )
    };
    if received == -1 {
        return Err(io::Error::last_os_error());
    }
    if received == 0 && message.msg_controllen == 0 {
        return Err(io::ErrorKind::UnexpectedEof.into());
    }

    let mut valid = received == 1
        && byte == PRELUDE
        && message.msg_flags & (libc::MSG_CTRUNC | libc::MSG_TRUNC) == 0;
    let mut endpoint = None;
    // Even when the byte or ancillary data is invalid, take ownership of every
    // FD installed by recvmsg. Linux closes rights that did not fit the buffer.
    unsafe {
        let mut header = libc::CMSG_FIRSTHDR(&message);
        while !header.is_null() {
            let offset = header.cast::<u8>().offset_from(ancillary.bytes.as_ptr()) as usize;
            let len = (*header).cmsg_len as usize;
            let base = libc::CMSG_LEN(0) as usize;
            if len < base || len > message.msg_controllen as usize - offset {
                return Err(invalid_prelude());
            }
            if (*header).cmsg_level == libc::SOL_SOCKET && (*header).cmsg_type == libc::SCM_RIGHTS {
                let bytes = len - base;
                valid &= bytes == FD_SIZE;
                let fds = libc::CMSG_DATA(header).cast::<RawFd>();
                for index in 0..bytes / FD_SIZE {
                    let fd = OwnedFd::from_raw_fd(fds.add(index).read_unaligned());
                    if endpoint.replace(fd).is_some() {
                        valid = false;
                    }
                }
            } else {
                valid = false;
            }
            header = libc::CMSG_NXTHDR(&message, header);
        }
    }
    let endpoint = endpoint.filter(|_| valid).ok_or_else(invalid_prelude)?;
    let socket = UnixStream::from(endpoint);
    validate_stream(&socket)?;
    Ok(socket)
}

fn validate_stream(socket: &UnixStream) -> io::Result<()> {
    let mut kind = 0i32;
    let mut len = size_of::<i32>() as libc::socklen_t;
    let fd = socket.as_raw_fd();
    let status = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_TYPE,
            ptr::from_mut(&mut kind).cast(),
            &mut len,
        )
    };
    if status != 0 || kind != libc::SOCK_STREAM {
        return Err(invalid_prelude());
    }
    let peer = socket.peer_addr().map_err(|_| invalid_prelude())?;
    // The socket pair is unnamed. Checking the address family also excludes
    // connected TCP streams that happen to have the right SO_TYPE.
    if !peer.is_unnamed() {
        return Err(invalid_prelude());
    }
    Ok(())
}

fn invalid_prelude() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid job effect FD prelude")
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::io::{Read, Write};
    use std::os::unix::net::UnixDatagram;

    use super::*;

    const TEST_TIMEOUT: Duration = Duration::from_secs(1);

    fn assert_eof(mut socket: UnixStream) {
        socket.set_read_timeout(Some(TEST_TIMEOUT)).unwrap();
        assert_eq!(socket.read(&mut [0]).unwrap(), 0);
    }

    #[test]
    fn hands_off_exact_socket_and_leaves_control_bytes_and_eof() {
        let (mut parent_a, input_a) = UnixStream::pair().unwrap();
        let (mut parent_b, input_b) = UnixStream::pair().unwrap();
        let (mut owner_a, effect_a) = UnixStream::pair().unwrap();
        let (mut owner_b, effect_b) = UnixStream::pair().unwrap();

        send(&parent_a, &effect_a.into()).unwrap();
        send(&parent_b, &effect_b.into()).unwrap();
        parent_a.write_all(b"bootstrap A").unwrap();
        parent_b.write_all(b"bootstrap B").unwrap();
        let worker_b = receive(input_b.as_raw_fd(), TEST_TIMEOUT).unwrap();
        let worker_a = receive(input_a.as_raw_fd(), TEST_TIMEOUT).unwrap();

        for (mut worker, owner, byte) in [
            (worker_a, &mut owner_a, b'A'),
            (worker_b, &mut owner_b, b'B'),
        ] {
            let flags = unsafe { libc::fcntl(worker.as_raw_fd(), libc::F_GETFD) };
            assert_eq!(flags, libc::FD_CLOEXEC);
            owner.set_read_timeout(Some(TEST_TIMEOUT)).unwrap();
            worker.write_all(&[byte]).unwrap();
            let mut received = [0];
            owner.read_exact(&mut received).unwrap();
            assert_eq!(received, [byte]);
            drop(worker);
            assert_eq!(owner.read(&mut received).unwrap(), 0);
        }

        for (mut input, parent, expected) in [(input_a, parent_a, 'A'), (input_b, parent_b, 'B')] {
            drop(parent);
            input.set_read_timeout(Some(TEST_TIMEOUT)).unwrap();
            let mut tail = Vec::new();
            input.read_to_end(&mut tail).unwrap();
            assert_eq!(tail, format!("bootstrap {expected}").as_bytes());
        }
    }

    #[test]
    fn closes_all_received_rights_on_invalid_and_truncated_preludes() {
        for (marker, count) in [(0, 1), (PRELUDE, 2), (PRELUDE, 4)] {
            let (parent, input) = UnixStream::pair().unwrap();
            let (owners, effects): (Vec<_>, Vec<_>) =
                (0..count).map(|_| UnixStream::pair().unwrap()).unzip();
            let fds: Vec<_> = effects.iter().map(AsRawFd::as_raw_fd).collect();
            send_frame(&parent, marker, &fds).unwrap();
            drop(effects);

            let error = receive(input.as_raw_fd(), TEST_TIMEOUT).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            for owner in owners {
                assert_eof(owner);
            }
        }
    }

    #[test]
    fn rejects_missing_rights_and_wrong_fd_types() {
        let (mut parent, input) = UnixStream::pair().unwrap();
        parent.write_all(&[PRELUDE]).unwrap();
        let error = receive(input.as_raw_fd(), TEST_TIMEOUT).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);

        let file = File::open("/dev/null").unwrap();
        let (datagram, _) = UnixDatagram::pair().unwrap();
        let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
        assert!(fd >= 0, "{}", io::Error::last_os_error());
        let unconnected = unsafe { OwnedFd::from_raw_fd(fd) };
        for fd in [
            file.as_raw_fd(),
            datagram.as_raw_fd(),
            unconnected.as_raw_fd(),
        ] {
            send_frame(&parent, PRELUDE, &[fd]).unwrap();
            let error = receive(input.as_raw_fd(), TEST_TIMEOUT).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        }
    }

    #[test]
    fn prelude_wait_is_bounded_and_observes_parent_close() {
        let (parent, input) = UnixStream::pair().unwrap();
        let error = receive(input.as_raw_fd(), Duration::ZERO).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        drop(parent);
        let error = receive(input.as_raw_fd(), TEST_TIMEOUT).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
    }
}
