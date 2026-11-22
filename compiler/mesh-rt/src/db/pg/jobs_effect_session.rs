//! Single-use-at-a-time transport for a scoped Jobs effect login.
use std::io::{self, Read, Write};
use std::net::{IpAddr, Shutdown, SocketAddr, TcpStream};
use std::os::fd::AsRawFd;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use super::{
    finish_startup, parse_error_response_full, parse_pg_url, read_message_with_limit, text_params,
    write_bind_values, write_describe_portal, write_execute, write_parse, write_sync, PgStream,
    SslMode,
};

pub(crate) const IO_TIMEOUT: Duration = Duration::from_secs(5);
const OPTIONS: &str = "-c statement_timeout=3000 -c lock_timeout=250 -c default_transaction_isolation=read\\ committed -c search_path=pg_catalog,pg_temp -c application_name=mesh_jobs_effect_v1";
const MAX_FRAME: usize = 4096;
const MAX_ASYNC: usize = 32;
const BOOL_COLUMN: &[u8] = b"\0\x01receipt\0\0\0\0\0\0\0\0\0\0\x10\0\x01\xff\xff\xff\xff\0\0";

pub(crate) struct EffectSession {
    tls: super::StreamOwned<super::ClientConnection, TcpStream>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum EffectFailure {
    BeforeSubmission,
    Unknown,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum EffectReceipt {
    Confirmed(bool),
    Rejected,
}

// A successful deadline check under this mutex admits one nonblocking
// send. Scheduling can delay an admitted send across the deadline. Keep the
// mutex through the send so stop and revocation serialize with it.
pub(crate) struct EffectAuthority {
    state: Mutex<AuthorityState>,
}

struct AuthorityState {
    deadline: Instant,
    expires: Instant,
    revoked: bool,
    interrupted: bool,
    socket: Option<TcpStream>,
}

impl AuthorityState {
    fn live(&self) -> bool {
        self.live_at(Instant::now())
    }

    fn live_at(&self, now: Instant) -> bool {
        !self.revoked && now < self.expires
    }
}

impl EffectAuthority {
    pub(crate) fn new(deadline: Instant, lease: Instant) -> Self {
        Self {
            state: Mutex::new(AuthorityState {
                deadline,
                expires: deadline.min(lease),
                revoked: false,
                interrupted: false,
                socket: None,
            }),
        }
    }

    pub(crate) fn renew(&self, expires: Instant) {
        let mut state = self.state.lock();
        if state.live() {
            state.expires = state.expires.max(expires.min(state.deadline));
        }
    }

    pub(crate) fn limit_deadline(&self, deadline: Instant) {
        // A zero-grace stop waits for an admitted send to exit this mutex.
        let mut state = self.state.lock();
        state.deadline = state.deadline.min(deadline);
        state.expires = state.expires.min(state.deadline);
    }

    pub(crate) fn live(&self) -> bool {
        self.state.lock().live()
    }

    pub(crate) fn revoke(&self) -> bool {
        let mut state = self.state.lock();
        state.revoked = true;
        if let Some(socket) = state.socket.take() {
            state.interrupted = true;
            let _ = socket.shutdown(Shutdown::Both);
        }
        state.interrupted
    }

    fn attach(&self, stream: &TcpStream) -> Result<(), EffectFailure> {
        let mut state = self.state.lock();
        if !state.live() || state.socket.is_some() {
            return Err(EffectFailure::BeforeSubmission);
        }
        state.socket = Some(
            stream
                .try_clone()
                .map_err(|_| EffectFailure::BeforeSubmission)?,
        );
        Ok(())
    }

    // Receipt admission, publication, and socket release share the mutex.
    pub(crate) fn release(
        &self,
        publish: impl FnOnce() -> io::Result<()>,
    ) -> Result<(), EffectFailure> {
        let mut state = self.state.lock();
        if state.socket.is_none() || !state.live() || publish().is_err() {
            return Err(EffectFailure::Unknown);
        }
        state.socket.take();
        Ok(())
    }

    pub(crate) fn publish_without_checkout(
        &self,
        publish: impl FnOnce() -> io::Result<()>,
    ) -> bool {
        let state = self.state.lock();
        state.socket.is_none() && state.live() && publish().is_ok()
    }
}

impl EffectSession {
    pub(crate) fn open(url: &str, jobs_url: &str) -> Result<Self, String> {
        let invalid = "Jobs effects require a separate verified TLS login at the Jobs database's numeric address";
        let app = parse_pg_url(url).map_err(|_| invalid)?;
        let jobs = parse_pg_url(jobs_url).map_err(|_| invalid)?;
        let ip = app.host.parse::<IpAddr>().map_err(|_| invalid)?;
        if app.host != jobs.host
            || app.port != jobs.port
            || app.database != jobs.database
            || app.user == jobs.user
            || app.user.is_empty()
            || app.database.is_empty()
            || app.password.is_empty()
            || app.user.len() > 63
            || app.database.len() > 63
            || app.user.contains('\0')
            || app.database.contains('\0')
            || app.sslmode != SslMode::Require
        {
            return Err(invalid.into());
        }

        let deadline = Instant::now() + IO_TIMEOUT;
        let failed = "could not open verified SCRAM Jobs effect session";
        let socket = TcpStream::connect_timeout(&SocketAddr::new(ip, app.port), IO_TIMEOUT)
            .map_err(|_| failed)?;
        socket.set_nodelay(true).map_err(|_| failed)?;
        let mut io = DeadlineSocket::new(&socket, deadline, None);
        let (PgStream::Tls(tls), b'I') =
            finish_startup(&mut io, &app, Some(OPTIONS)).map_err(|_| failed)?
        else {
            return Err(failed.into());
        };
        remaining(deadline).map_err(|_| failed)?;
        Ok(Self {
            tls: super::StreamOwned::new(tls.conn, socket),
        })
    }

    pub(crate) fn check(&mut self, sql: &str, params: &[&str]) -> bool {
        self.query(sql, params, Instant::now() + IO_TIMEOUT, None)
            == Ok(EffectReceipt::Confirmed(true))
    }

    pub(crate) fn checkout(&self, owner: &EffectAuthority) -> Result<(), EffectFailure> {
        owner.attach(&self.tls.sock)
    }

    // The caller owns this checkout until reset and release succeed. Any error
    // after attachment retires the connection and revokes the attempt channel.
    pub(crate) fn invoke(
        &mut self,
        sql: &str,
        params: &[&str],
        deadline: Instant,
        owner: &EffectAuthority,
    ) -> Result<EffectReceipt, EffectFailure> {
        let receipt = self.query(sql, params, deadline, Some(owner))?;
        if self.reset(deadline, Some(owner)) {
            Ok(receipt)
        } else {
            Err(EffectFailure::Unknown)
        }
    }

    pub(crate) fn reset(&mut self, deadline: Instant, owner: Option<&EffectAuthority>) -> bool {
        let mut io = DeadlineSocket::new(&self.tls.sock, deadline, owner);
        let mut tls = rustls::Stream::new(&mut self.tls.conn, &mut io);
        tls.write_all(b"Q\0\0\0\x10DISCARD ALL\0").is_ok()
            && tls.flush().is_ok()
            && Frames::new(&mut tls, deadline).reset()
    }

    fn query(
        &mut self,
        sql: &str,
        params: &[&str],
        deadline: Instant,
        owner: Option<&EffectAuthority>,
    ) -> Result<EffectReceipt, EffectFailure> {
        let mut request = Vec::new();
        write_parse(&mut request, sql);
        write_bind_values(&mut request, &text_params(params), &[])
            .map_err(|_| EffectFailure::BeforeSubmission)?;
        write_describe_portal(&mut request);
        write_execute(&mut request);
        write_sync(&mut request);

        let mut io = DeadlineSocket::new(&self.tls.sock, deadline, owner);
        let mut tls = rustls::Stream::new(&mut self.tls.conn, &mut io);
        if tls.write_all(&request).and_then(|()| tls.flush()).is_err() {
            return Err(if io.submitted {
                EffectFailure::Unknown
            } else {
                EffectFailure::BeforeSubmission
            });
        }
        Frames::new(&mut tls, deadline)
            .receipt()
            .ok_or(EffectFailure::Unknown)
    }
}

fn remaining(deadline: Instant) -> io::Result<Duration> {
    let duration = deadline.saturating_duration_since(Instant::now());
    if duration.is_zero() {
        Err(io::ErrorKind::TimedOut.into())
    } else {
        Ok(duration)
    }
}

struct DeadlineSocket<'a> {
    socket: &'a TcpStream,
    deadline: Instant,
    owner: Option<&'a EffectAuthority>,
    submitted: bool,
}

impl<'a> DeadlineSocket<'a> {
    fn new(socket: &'a TcpStream, deadline: Instant, owner: Option<&'a EffectAuthority>) -> Self {
        Self {
            socket,
            deadline,
            owner,
            submitted: false,
        }
    }

    fn write_authorized(&self, owner: &EffectAuthority, bytes: &[u8]) -> io::Result<usize> {
        let state = owner.state.lock();
        let now = Instant::now();
        if state.socket.is_none() || !state.live_at(now) {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        if now >= self.deadline {
            return Err(io::ErrorKind::TimedOut.into());
        }
        // No blocking I/O is allowed while holding the authority lock.
        let sent = unsafe {
            libc::send(
                self.socket.as_raw_fd(),
                bytes.as_ptr().cast(),
                bytes.len().min(16 * 1024),
                libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL,
            )
        };
        if sent < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(sent as usize)
        }
    }
}

impl Read for DeadlineSocket<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.socket
            .set_read_timeout(Some(remaining(self.deadline)?))?;
        self.socket.read(bytes)
    }
}

impl Write for DeadlineSocket<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let written = if let Some(owner) = self.owner {
            loop {
                match self.write_authorized(owner, bytes) {
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        let left = remaining(self.deadline)?;
                        std::thread::sleep(left.min(Duration::from_millis(1)));
                    }
                    result => break result?,
                }
            }
        } else {
            self.socket
                .set_write_timeout(Some(remaining(self.deadline)?))?;
            self.socket.write(bytes)?
        };
        self.submitted |= written != 0;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        remaining(self.deadline).map(|_| ())
    }
}

struct Frames<'a, R> {
    stream: &'a mut R,
    deadline: Instant,
    async_count: usize,
}

impl<'a, R: Read> Frames<'a, R> {
    fn new(stream: &'a mut R, deadline: Instant) -> Self {
        Self {
            stream,
            deadline,
            async_count: 0,
        }
    }

    fn next(&mut self) -> Option<(u8, Vec<u8>)> {
        loop {
            // Rustls may already have the next frame in its plaintext buffer.
            remaining(self.deadline).ok()?;
            match read_message_with_limit(self.stream, MAX_FRAME).ok()? {
                (b'N' | b'S', _) if self.async_count < MAX_ASYNC => self.async_count += 1,
                frame => return Some(frame),
            }
        }
    }

    fn expect(&mut self, tag: u8, bytes: &[u8]) -> bool {
        self.next()
            .is_some_and(|(actual_tag, body)| actual_tag == tag && body == bytes)
    }

    fn receipt(&mut self) -> Option<EffectReceipt> {
        if !(self.expect(b'1', b"") && self.expect(b'2', b"") && self.expect(b'T', BOOL_COLUMN)) {
            return None;
        }
        let (tag, body) = self.next()?;
        let receipt = match (tag, body.as_slice()) {
            (b'D', b"\0\x01\0\0\0\x01t") if self.expect(b'C', b"SELECT 1\0") => {
                EffectReceipt::Confirmed(true)
            }
            (b'D', b"\0\x01\0\0\0\x01f") if self.expect(b'C', b"SELECT 1\0") => {
                EffectReceipt::Confirmed(false)
            }
            (b'E', _)
                if matches!(
                    parse_error_response_full(&body).sqlstate.as_str(),
                    "55000" | "55P03" | "25000" | "57014"
                ) =>
            {
                EffectReceipt::Rejected
            }
            _ => return None,
        };
        self.expect(b'Z', b"I").then_some(receipt)
    }

    fn reset(&mut self) -> bool {
        self.expect(b'C', b"DISCARD ALL\0") && self.expect(b'Z', b"I")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let socket = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (peer, _) = listener.accept().unwrap();
        peer.set_read_timeout(Some(IO_TIMEOUT)).unwrap();
        (socket, peer)
    }

    fn append(bytes: &mut Vec<u8>, tag: u8, body: &[u8]) {
        bytes.push(tag);
        bytes.extend(((body.len() + 4) as u32).to_be_bytes());
        bytes.extend(body);
    }

    #[test]
    fn requires_one_boolean_and_idle_ready_before_receipt() {
        let mut frames = Vec::new();
        for (tag, body) in [
            (b'1', b"".as_slice()),
            (b'2', b""),
            (b'T', BOOL_COLUMN),
            (b'D', b"\0\x01\0\0\0\x01t"),
            (b'C', b"SELECT 1\0"),
            (b'Z', b"I"),
        ] {
            append(&mut frames, tag, body);
        }
        let deadline = Instant::now() + IO_TIMEOUT;
        assert_eq!(
            Frames::new(&mut frames.as_slice(), deadline).receipt(),
            Some(EffectReceipt::Confirmed(true))
        );
        assert_eq!(
            Frames::new(&mut &frames[..frames.len() - 1], deadline).receipt(),
            None
        );
        *frames.last_mut().unwrap() = b'T';
        assert_eq!(
            Frames::new(&mut frames.as_slice(), deadline).receipt(),
            None
        );
    }

    #[test]
    fn known_fence_rejection_requires_ready_and_other_errors_remain_unknown() {
        let mut prefix = Vec::new();
        append(&mut prefix, b'1', b"");
        append(&mut prefix, b'2', b"");
        append(&mut prefix, b'T', BOOL_COLUMN);
        let deadline = Instant::now() + IO_TIMEOUT;
        let mut rejected = prefix.clone();
        append(&mut rejected, b'E', b"SERROR\0C55P03\0Mlocked\0\0");
        assert_eq!(
            Frames::new(&mut rejected.as_slice(), deadline).receipt(),
            None
        );
        append(&mut rejected, b'Z', b"I");
        assert_eq!(
            Frames::new(&mut rejected.as_slice(), deadline).receipt(),
            Some(EffectReceipt::Rejected)
        );
        append(&mut prefix, b'E', b"SERROR\0C40003\0Munknown\0\0");
        append(&mut prefix, b'Z', b"I");
        assert_eq!(
            Frames::new(&mut prefix.as_slice(), deadline).receipt(),
            None
        );
    }

    #[test]
    fn reset_requires_full_discard_and_caps_async_frames() {
        let mut frames = Vec::new();
        append(&mut frames, b'C', b"DISCARD ALL\0");
        append(&mut frames, b'Z', b"I");
        let deadline = Instant::now() + IO_TIMEOUT;
        assert!(Frames::new(&mut frames.as_slice(), deadline).reset());
        let mut noise = Vec::new();
        for _ in 0..=MAX_ASYNC {
            append(&mut noise, b'N', b"");
        }
        noise.extend(frames);
        assert!(!Frames::new(&mut noise.as_slice(), deadline).reset());
        assert!(!Frames::new(&mut &b"C\0\0\xff\xff"[..], deadline).reset());
    }

    #[test]
    fn revocation_cannot_shut_down_a_released_socket_or_send_again() {
        let (socket, mut peer) = pair();
        let deadline = Instant::now() + IO_TIMEOUT;
        let owner = EffectAuthority::new(deadline, deadline);
        owner.attach(&socket).unwrap();
        owner.renew(deadline);
        owner.release(|| Ok(())).unwrap();
        owner.revoke();
        assert!(!owner.live());
        assert_eq!(owner.attach(&socket), Err(EffectFailure::BeforeSubmission));
        assert!(DeadlineSocket::new(&socket, deadline, Some(&owner))
            .write_all(b"x")
            .is_err());
        (&socket).write_all(b"y").unwrap();
        let mut byte = [0];
        peer.read_exact(&mut byte).unwrap();
        assert_eq!(byte, *b"y");
    }

    #[test]
    fn revocation_interrupts_the_attached_socket() {
        let (socket, mut peer) = pair();
        let deadline = Instant::now() + IO_TIMEOUT;
        let owner = EffectAuthority::new(deadline, deadline);
        owner.attach(&socket).unwrap();
        owner.revoke();
        assert_eq!(peer.read(&mut [0]).unwrap(), 0);
        assert_eq!(
            owner.release(|| panic!("revoked receipt")),
            Err(EffectFailure::Unknown)
        );
    }

    #[test]
    fn stop_cutoff_denies_new_write_and_receipt_admissions() {
        let (socket, mut peer) = pair();
        let now = Instant::now();
        let deadline = now + IO_TIMEOUT;
        let grace = now + IO_TIMEOUT / 2;
        let owner = EffectAuthority::new(deadline, now + IO_TIMEOUT / 4);
        owner.limit_deadline(grace);
        owner.renew(deadline);
        assert_eq!(owner.state.lock().expires, grace);
        assert!(owner.publish_without_checkout(|| Ok(())));
        owner.attach(&socket).unwrap();
        let mut io = DeadlineSocket::new(&socket, deadline, Some(&owner));
        io.write_all(b"g").unwrap();

        owner.limit_deadline(Instant::now());
        owner.limit_deadline(grace);
        owner.renew(deadline);
        assert_eq!(
            io.write_all(b"x").unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        assert_eq!(
            owner.release(|| panic!("receipt after zero grace")),
            Err(EffectFailure::Unknown)
        );
        (&socket).write_all(b"y").unwrap();
        let mut bytes = [0; 2];
        peer.read_exact(&mut bytes).unwrap();
        assert_eq!(&bytes, b"gy");
        owner.revoke();
        assert!(!owner.publish_without_checkout(|| panic!("receipt after cutoff")));
    }

    #[test]
    fn startup_rejects_unscoped_connections_before_dialing() {
        let jobs = "postgres://jobs@127.0.0.1/jobs";
        for url in [
            "postgres://jobs:secret@127.0.0.1/jobs?sslmode=verify-full",
            "postgres://app:secret@127.0.0.1/jobs?sslmode=prefer",
            "postgres://app:secret@127.0.0.1/other?sslmode=verify-full",
            "postgres://app:secret@localhost/jobs?sslmode=verify-full",
        ] {
            let error = EffectSession::open(url, jobs).err().unwrap();
            assert!(error.contains("separate verified TLS login"));
        }
    }

    #[test]
    fn one_deadline_covers_progress() {
        let (socket, mut peer) = pair();
        peer.write_all(b"ab").unwrap();
        let deadline = Instant::now() + Duration::from_millis(100);
        let mut io = DeadlineSocket::new(&socket, deadline, None);
        io.read_exact(&mut [0]).unwrap();
        std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
        assert_eq!(
            io.read_exact(&mut [0]).unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
        assert_eq!(
            io.write_all(b"x").unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
    }
}
