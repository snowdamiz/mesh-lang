//! Scheduler-aware TCP waits for PostgreSQL, including rustls's underlying I/O.
//!
//! One bounded readiness reactor owns socket references and reply channels.
//! A socket mutex protects only nonblocking I/O/registration, never a wait.
//! Using Mio for the actual I/O also rearms Windows readiness correctly.
//! No process, pool, connection, or TLS lock crosses a coroutine suspension.
use std::cell::Cell;
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use crate::actor::{
    cooperative_channel, cooperative_recv, cooperative_recv_timeout, CooperativeSender,
};
use mio::{Events, Interest, Poll, Token, Waker};

const MAX_WAITS: usize = 8192;
const WAKE: Token = Token(0);
struct Reactor {
    queue: crossbeam_channel::Sender<Wait>,
    wake: Arc<Waker>,
    count: Arc<AtomicUsize>,
}
struct Permit(Arc<AtomicUsize>);
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
struct Wait {
    socket: Arc<parking_lot::Mutex<mio::net::TcpStream>>,
    interest: Interest,
    deadline: Option<Instant>,
    cancelled: Arc<AtomicBool>,
    answer: Option<CooperativeSender<io::Result<()>>>,
    _permit: Permit,
}
impl Wait {
    fn finish(&mut self, result: io::Result<()>) {
        if let Some(answer) = self.answer.take() {
            let _ = answer.send(result);
        }
    }
}
impl Drop for Wait {
    fn drop(&mut self) {
        self.finish(Err(io::Error::other("database readiness reactor stopped")));
    }
}
struct CancelWait {
    cancelled: Arc<AtomicBool>,
    wake: Arc<Waker>,
}
impl Drop for CancelWait {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        let _ = self.wake.wake();
    }
}

fn reactor() -> io::Result<&'static Reactor> {
    static REACTOR: OnceLock<Result<Reactor, String>> = OnceLock::new();
    REACTOR
        .get_or_init(|| {
            let mut poll = Poll::new().map_err(|e| e.to_string())?;
            let wake = Arc::new(Waker::new(poll.registry(), WAKE).map_err(|e| e.to_string())?);
            let (queue, receiver) = crossbeam_channel::bounded::<Wait>(MAX_WAITS);
            std::thread::Builder::new()
                .name("mesh-pg-readiness".into())
                .spawn(move || {
                    let mut events = Events::with_capacity(1024);
                    let mut waits = HashMap::<Token, Wait>::new();
                    let mut sequence = 0usize;
                    loop {
                        while let Ok(mut wait) = receiver.try_recv() {
                            if wait.cancelled.load(Ordering::Acquire) {
                                continue;
                            }
                            // Tokens are never reused while registered, including integer wrap.
                            let token = loop {
                                sequence = sequence.wrapping_add(1).max(1);
                                let candidate = Token(sequence);
                                if !waits.contains_key(&candidate) {
                                    break candidate;
                                }
                            };
                            let registered = poll.registry().register(
                                &mut *wait.socket.lock(),
                                token,
                                wait.interest,
                            );
                            match registered {
                                Ok(()) => {
                                    waits.insert(token, wait);
                                }
                                Err(e) => wait.finish(Err(e)),
                            }
                        }
                        let now = Instant::now();
                        waits.retain(|_, wait| {
                            let cancelled = wait.cancelled.load(Ordering::Acquire);
                            let expired = wait.deadline.is_some_and(|deadline| now >= deadline);
                            if cancelled || expired {
                                let _ = poll.registry().deregister(&mut *wait.socket.lock());
                                wait.finish(if expired {
                                    Err(io::ErrorKind::TimedOut.into())
                                } else {
                                    Ok(())
                                });
                                false
                            } else {
                                true
                            }
                        });
                        let timeout = waits
                            .values()
                            .filter_map(|wait| wait.deadline)
                            .min()
                            .map(|deadline| deadline.saturating_duration_since(Instant::now()));
                        match poll.poll(&mut events, timeout) {
                            Ok(()) => {}
                            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                            Err(_) => break,
                        }
                        for event in events.iter() {
                            if let Some(mut wait) = waits.remove(&event.token()) {
                                let _ = poll.registry().deregister(&mut *wait.socket.lock());
                                wait.finish(Ok(()));
                            }
                        }
                    }
                    // Dropping each pending reply wakes its caller with an error.
                })
                .map_err(|e| e.to_string())?;
            Ok(Reactor {
                queue,
                wake,
                count: Arc::new(AtomicUsize::new(0)),
            })
        })
        .as_ref()
        .map_err(|error| io::Error::other(error.clone()))
}

fn wait_socket(
    socket: &Arc<parking_lot::Mutex<mio::net::TcpStream>>,
    interest: Interest,
    deadline: Option<Instant>,
) -> io::Result<()> {
    if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
        return Err(io::ErrorKind::TimedOut.into());
    }
    let reactor = reactor()?;
    let mut count = reactor.count.load(Ordering::Acquire);
    loop {
        if count >= MAX_WAITS {
            return Err(io::Error::other("too many database socket waits"));
        }
        match reactor.count.compare_exchange_weak(
            count,
            count + 1,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => break,
            Err(actual) => count = actual,
        }
    }
    let permit = Permit(reactor.count.clone());
    let cancelled = Arc::new(AtomicBool::new(false));
    let (answer, receiver) = cooperative_channel();
    let _cancel = CancelWait {
        cancelled: cancelled.clone(),
        wake: reactor.wake.clone(),
    };
    let wait = Wait {
        socket: socket.clone(),
        interest,
        deadline,
        cancelled,
        answer: Some(answer),
        _permit: permit,
    };
    reactor
        .queue
        .try_send(wait)
        .map_err(|_| io::Error::other("database readiness queue unavailable"))?;
    reactor.wake.wake()?;
    cooperative_recv(&receiver)
        .map_err(|_| io::Error::other("database readiness reply unavailable"))?
}

/// A nonblocking socket whose Read/Write contract remains synchronous. Its
/// waits suspend actors; native callers block on the same readiness channel.
/// Read_exact/write_all keep their own partial-byte positions across waits.
pub(super) struct CooperativeTcp {
    pub(super) socket: TcpStream,
    io: Arc<parking_lot::Mutex<mio::net::TcpStream>>,
    read_timeout: Cell<Option<Duration>>,
    write_timeout: Option<Duration>,
}
impl CooperativeTcp {
    pub(super) fn new(socket: TcpStream) -> io::Result<Self> {
        let read_timeout = Cell::new(socket.read_timeout()?);
        let write_timeout = socket.write_timeout()?;
        socket.set_nonblocking(true)?;
        let io = Arc::new(parking_lot::Mutex::new(mio::net::TcpStream::from_std(
            socket.try_clone()?,
        )));
        Ok(Self {
            socket,
            io,
            read_timeout,
            write_timeout,
        })
    }
    pub(super) fn set_read_timeout(&self, timeout: Option<Duration>) {
        self.read_timeout.set(timeout);
        // Keep introspection/legacy native socket settings consistent. Actual
        // waits use monotonic deadlines, since the socket is nonblocking.
        let _ = self.socket.set_read_timeout(timeout);
    }
}
impl Read for CooperativeTcp {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let deadline = self
            .read_timeout
            .get()
            .and_then(|duration| Instant::now().checked_add(duration));
        loop {
            let result = self.io.lock().read(bytes);
            match result {
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    wait_socket(&self.io, Interest::READABLE, deadline)?
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                result => return result,
            }
        }
    }
}
impl Write for CooperativeTcp {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let deadline = self
            .write_timeout
            .and_then(|duration| Instant::now().checked_add(duration));
        loop {
            let result = self.io.lock().write(bytes);
            match result {
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    wait_socket(&self.io, Interest::WRITABLE, deadline)?
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                result => return result,
            }
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        self.socket.flush()
    }
}

// System DNS resolution and TCP establishment can block before a socket is
// available to the reactor. Four bounded workers own only copied addresses
// and socket replies; cancellation cannot leave a pool or actor reference.
struct ConnectJob {
    host: String,
    port: u16,
    answer: CooperativeSender<Result<TcpStream, String>>,
    cancelled: Arc<AtomicBool>,
    deadline: Instant,
}
struct CancelConnect(Arc<AtomicBool>);
impl Drop for CancelConnect {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}
pub(super) fn connect_tcp(host: &str, port: u16) -> Result<TcpStream, String> {
    static CONNECTORS: OnceLock<Result<crossbeam_channel::Sender<ConnectJob>, String>> =
        OnceLock::new();
    let queue = CONNECTORS
        .get_or_init(|| {
            let (queue, receiver) = crossbeam_channel::bounded::<ConnectJob>(64);
            for index in 0..4 {
                let receiver = receiver.clone();
                std::thread::Builder::new()
                    .name(format!("mesh-pg-connect-{index}"))
                    .spawn(move || {
                        while let Ok(job) = receiver.recv() {
                            let ConnectJob {
                                host,
                                port,
                                answer,
                                cancelled,
                                deadline,
                            } = job;
                            let result = (|| {
                                if cancelled.load(Ordering::Acquire) || Instant::now() >= deadline {
                                    return Err("database connection request expired".into());
                                }
                                let addrs = (host.as_str(), port)
                                    .to_socket_addrs()
                                    .map_err(|e| format!("DNS resolution failed: {e}"))?;
                                let mut failure = "could not resolve host".to_string();
                                for addr in addrs {
                                    let remaining =
                                        deadline.saturating_duration_since(Instant::now());
                                    if cancelled.load(Ordering::Acquire) || remaining.is_zero() {
                                        break;
                                    }
                                    match TcpStream::connect_timeout(
                                        &addr,
                                        remaining.min(Duration::from_secs(10)),
                                    ) {
                                        Ok(socket) => return Ok(socket),
                                        Err(e) => failure = format!("connection failed: {e}"),
                                    }
                                }
                                Err(failure)
                            })();
                            let _ = answer.send(result);
                        }
                    })
                    .map_err(|e| e.to_string())?;
            }
            Ok(queue)
        })
        .as_ref()
        .map_err(Clone::clone)?;
    let (answer, receiver) = cooperative_channel();
    let cancelled = Arc::new(AtomicBool::new(false));
    let _cancel = CancelConnect(cancelled.clone());
    queue
        .try_send(ConnectJob {
            host: host.to_owned(),
            port,
            answer,
            cancelled,
            deadline: Instant::now() + super::pg::ANSWER_TIMEOUT,
        })
        .map_err(|_| "database connection queue unavailable".to_string())?;
    cooperative_recv_timeout(&receiver, super::pg::ANSWER_TIMEOUT)
        .map_err(|_| "database connection establishment timeout".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    fn pair() -> (CooperativeTcp, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        (
            CooperativeTcp::new(client).unwrap(),
            listener.accept().unwrap().0,
        )
    }
    #[test]
    fn fragmented_read_preserves_bytes_and_eof() {
        let (mut socket, mut peer) = pair();
        let writer = std::thread::spawn(move || {
            for byte in b"fragmented" {
                peer.write_all(&[*byte]).unwrap();
                std::thread::sleep(Duration::from_millis(2));
            }
        });
        let mut bytes = [0; 10];
        socket.read_exact(&mut bytes).unwrap();
        assert_eq!(&bytes, b"fragmented");
        assert_eq!(socket.read(&mut [0]).unwrap(), 0);
        writer.join().unwrap();
    }
    #[test]
    fn readiness_deadline_expires_without_peer_activity() {
        let (mut socket, _peer) = pair();
        socket.set_read_timeout(Some(Duration::from_millis(20)));
        let start = Instant::now();
        assert_eq!(
            socket.read(&mut [0]).unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
        assert!(start.elapsed() >= Duration::from_millis(20));
    }

    #[test]
    fn write_all_preserves_position_across_backpressure() {
        let (mut socket, mut peer) = pair();
        let expected = vec![0x5a; 8 * 1024 * 1024];
        let reader = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            let mut received = Vec::new();
            peer.read_to_end(&mut received).unwrap();
            received
        });
        socket.write_all(&expected).unwrap();
        socket.socket.shutdown(std::net::Shutdown::Write).unwrap();
        assert_eq!(reader.join().unwrap(), expected);
    }
    #[test]
    fn actor_read_yields_until_another_actor_writes() {
        let (mut socket, mut peer) = pair();
        socket.set_read_timeout(Some(Duration::from_secs(2)));
        let answer = crate::actor::in_actor(move || {
            type Writer = Box<dyn FnOnce() + Send>;
            extern "C-unwind" fn write(args: *const u8) {
                let body = unsafe { Box::from_raw(args as *mut Writer) };
                crate::actor::mesh_timer_sleep(10);
                body();
            }
            let body: Writer = Box::new(move || peer.write_all(b"ready").unwrap());
            crate::actor::global_scheduler().spawn(
                write as *const u8,
                Box::into_raw(Box::new(body)).cast(),
                0,
                1,
            );
            let mut bytes = [0; 5];
            socket.read_exact(&mut bytes).unwrap();
            bytes
        });
        assert_eq!(&answer, b"ready");
    }
}
