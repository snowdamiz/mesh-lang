//! Shared nonblocking WebSocket I/O reactor.
//!
//! One readiness thread owns every steady-state WebSocket transport, including
//! rustls state. Callers submit bounded, nonblocking commands and never perform
//! socket I/O on actor scheduler workers.

use std::collections::{HashMap, VecDeque};
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, TrySendError};
use mio::{Events, Interest, Poll, Token, Waker};
use parking_lot::Mutex;
use rustls::Connection;

use super::close::{
    build_close_payload, is_valid_close_code, is_valid_text_payload, parse_close_payload_strict,
    WsCloseCode,
};
use super::frame::{
    encode_frame, FrameDecoder, MessageAssembler, ReassembleResult, WsFrame, WsOpcode,
};
use super::handshake::{parse_upgrade_request_bytes, parse_upgrade_response_bytes};

const WAKE_TOKEN: Token = Token(0);
const COMMAND_QUEUE_ITEMS: usize = 8_192;
const READ_BUDGET_BYTES: usize = 64 * 1024;
const WRITE_BUDGET_BYTES: usize = 64 * 1024;
const FRAME_BUDGET_ITEMS: usize = 64;
const TLS_BUFFER_BYTES: usize = 64 * 1024;
const REACTOR_TICK: Duration = Duration::from_millis(25);
const CLOSE_DEADLINE: Duration = Duration::from_secs(2);

/// What one reactor admits across all its connections.
#[derive(Clone, Copy)]
struct ReactorLimits {
    connections: usize,
    tls_handshakes: usize,
    write_items: usize,
    write_bytes: usize,
    read_bytes: usize,
    inbound_items: usize,
    inbound_bytes: usize,
}

/// The process reactor's limits.
const PROCESS_LIMITS: ReactorLimits = ReactorLimits {
    connections: 16_384,
    tls_handshakes: 2_048,
    write_items: 65_536,
    write_bytes: 128 * 1024 * 1024,
    read_bytes: 128 * 1024 * 1024,
    inbound_items: 65_536,
    inbound_bytes: 128 * 1024 * 1024,
};

/// Connection IDs are tokens, after the waker's 0; a 64-bit count does not
/// wrap back to it.
static NEXT_CONNECTION_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug)]
pub(crate) enum ReactorEvent {
    Text(Vec<u8>, InboundPermit),
    Binary(Vec<u8>, InboundPermit),
    Close(u16, String),
}

/// A delivered message's share of its reactor's inbound budget, held until
/// the message is consumed.
#[derive(Debug)]
pub(crate) struct InboundPermit {
    bytes: usize,
    budget: Arc<QueueBudget>,
}

impl InboundPermit {
    fn reserve(budget: &Arc<QueueBudget>, bytes: usize) -> Option<Self> {
        budget.reserve(bytes).then(|| Self {
            bytes,
            budget: Arc::clone(budget),
        })
    }

    /// A permit of a budget of its own, for tests that make events by hand.
    #[cfg(test)]
    pub(crate) fn unbounded(bytes: usize) -> Self {
        Self::reserve(&Arc::new(QueueBudget::new(1, bytes)), bytes).unwrap()
    }
}

impl Drop for InboundPermit {
    fn drop(&mut self) {
        self.budget.release(self.bytes);
    }
}

/// A sink refused an event: its queue is full.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SinkFull;

pub(crate) trait ReactorEventSink: Send + Sync {
    /// The connection completed its opening handshake.
    fn opened(&self) {}
    fn event(&self, event: ReactorEvent) -> Result<(), SinkFull>;
    fn terminated(&self, reason: &str);
}

pub(crate) trait ServerHandshakeHandler: Send + Sync {
    fn opened(
        &self,
        connection: ReactorConnection,
        path: String,
        headers: Vec<(String, String)>,
    ) -> Arc<dyn ReactorEventSink>;

    fn failed(&self, reason: &str);
}

#[derive(Clone, Copy)]
enum PeerRole {
    Server,
    Client,
}

#[derive(Clone)]
pub(crate) struct ReactorConfig {
    role: PeerRole,
    pub(crate) max_message_bytes: usize,
    max_write_queue_bytes: usize,
    max_write_queue_items: usize,
    ping_interval: Duration,
    pong_timeout: Duration,
    handshake_timeout: Duration,
}

impl ReactorConfig {
    pub(crate) fn server(max_message_bytes: usize) -> Self {
        Self {
            role: PeerRole::Server,
            max_message_bytes,
            max_write_queue_bytes: 32 * 1024 * 1024,
            max_write_queue_items: 256,
            ping_interval: Duration::from_secs(30),
            pong_timeout: Duration::from_secs(10),
            handshake_timeout: Duration::from_secs(5),
        }
    }

    pub(crate) fn client(max_message_bytes: usize, heartbeat_timeout: Duration) -> Self {
        Self {
            role: PeerRole::Client,
            max_message_bytes,
            max_write_queue_bytes: max_message_bytes.saturating_mul(2).max(256 * 1024),
            max_write_queue_items: 256,
            ping_interval: heartbeat_timeout / 2,
            pong_timeout: heartbeat_timeout,
            handshake_timeout: Duration::from_secs(10),
        }
    }

    #[cfg(test)]
    fn with_write_limits(mut self, items: usize, bytes: usize) -> Self {
        self.max_write_queue_items = items;
        self.max_write_queue_bytes = bytes;
        self
    }

    pub(crate) fn with_handshake_timeout(mut self, timeout: Duration) -> Self {
        self.handshake_timeout = timeout;
        self
    }
}

/// A connected socket as the reactor drives it: in the clear, or under a
/// TLS session, client or server.
pub(crate) enum ReactorTransport {
    Plain(mio::net::TcpStream),
    Tls {
        session: Connection,
        socket: mio::net::TcpStream,
    },
}

/// A connected socket, nonblocking for the reactor. A socket this process
/// owns always takes the mode: only a bad descriptor refuses it.
fn nonblocking(socket: TcpStream) -> mio::net::TcpStream {
    socket
        .set_nonblocking(true)
        .expect("a connected socket accepts nonblocking mode");
    mio::net::TcpStream::from_std(socket)
}

impl ReactorTransport {
    pub(crate) fn plain(stream: TcpStream) -> Self {
        Self::Plain(nonblocking(stream))
    }

    pub(crate) fn tls(session: impl Into<Connection>, socket: TcpStream) -> Self {
        let mut session = session.into();
        session.set_buffer_limit(Some(TLS_BUFFER_BYTES));
        Self::Tls {
            session,
            socket: nonblocking(socket),
        }
    }

    fn source(&mut self) -> &mut mio::net::TcpStream {
        match self {
            Self::Plain(socket) | Self::Tls { socket, .. } => socket,
        }
    }

    fn session(&self) -> Option<&Connection> {
        match self {
            Self::Tls { session, .. } => Some(session),
            Self::Plain(_) => None,
        }
    }

    fn wants_write(&self) -> bool {
        self.session().is_some_and(|session| session.wants_write())
    }

    fn wants_read(&self) -> bool {
        self.session().is_some_and(|session| session.wants_read())
    }

    fn is_handshaking(&self) -> bool {
        self.session()
            .is_some_and(|session| session.is_handshaking())
    }

    /// Plaintext: from the socket, or what TLS records decrypted.
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Plain(socket) => socket.read(buffer),
            Self::Tls { session, .. } => session.reader().read(buffer),
        }
    }

    /// Plaintext: to the socket, or to TLS to encrypt.
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        match self {
            Self::Plain(socket) => socket.write(bytes),
            Self::Tls { session, .. } => session.writer().write(bytes),
        }
    }

    /// Writes TLS records waiting to go out: none when there are none.
    fn flush_tls(&mut self) -> Option<io::Result<usize>> {
        match self {
            Self::Tls { session, socket } if session.wants_write() => {
                Some(session.write_tls(socket))
            }
            _ => None,
        }
    }

    fn shutdown(&mut self) {
        let _ = self.source().shutdown(Shutdown::Both);
    }
}

#[derive(Debug)]
struct QueueBudget {
    bytes: AtomicUsize,
    items: AtomicUsize,
    max_bytes: usize,
    max_items: usize,
}

impl QueueBudget {
    fn new(max_items: usize, max_bytes: usize) -> Self {
        Self {
            bytes: AtomicUsize::new(0),
            items: AtomicUsize::new(0),
            max_bytes,
            max_items,
        }
    }

    fn reserve(&self, bytes: usize) -> bool {
        if !reserve_counter(&self.items, 1, self.max_items) {
            return false;
        }
        if !reserve_counter(&self.bytes, bytes, self.max_bytes) {
            self.items.fetch_sub(1, Ordering::AcqRel);
            return false;
        }
        true
    }

    fn release(&self, bytes: usize) {
        self.bytes.fetch_sub(bytes, Ordering::AcqRel);
        self.items.fetch_sub(1, Ordering::AcqRel);
    }
}

fn reserve_counter(counter: &AtomicUsize, amount: usize, maximum: usize) -> bool {
    counter
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            current.checked_add(amount).filter(|next| *next <= maximum)
        })
        .is_ok()
}

struct Reservation {
    bytes: usize,
    local: Arc<QueueBudget>,
    aggregate: Arc<QueueBudget>,
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.local.release(self.bytes);
        self.aggregate.release(self.bytes);
    }
}

struct ConnectionShared {
    accepting_writes: AtomicBool,
    cancelled: AtomicBool,
    cancel_reason: Mutex<Option<String>>,
    local_budget: Arc<QueueBudget>,
}

#[derive(Clone)]
pub(crate) struct ReactorConnection {
    id: u64,
    role: PeerRole,
    max_message_bytes: usize,
    shared: Arc<ConnectionShared>,
    control: Arc<ReactorControl>,
}

impl ReactorConnection {
    pub(crate) fn send(&self, opcode: WsOpcode, payload: &[u8]) -> Result<(), String> {
        if payload.len() > self.max_message_bytes {
            return Err("MESSAGE_TOO_BIG".to_string());
        }
        if !self.shared.accepting_writes.load(Ordering::Acquire) {
            return Err("WebSocket connection is closed".to_string());
        }
        let reservation = self.reserve_frame(payload.len())?;
        let encoded = encode_frame(opcode, payload, self.mask());
        self.submit(Command::Write {
            id: self.id,
            outbound: Outbound::new(encoded, Some(reservation)),
        })
    }

    pub(crate) fn graceful_close(&self, code: u16, reason: &str) -> Result<(), String> {
        if !is_valid_close_code(code) {
            return Err("invalid WebSocket close code".to_string());
        }
        let payload = build_close_payload(code, reason);
        let encoded = encode_frame(WsOpcode::Close, &payload, self.mask());
        let retained_reason = String::from_utf8(payload[2..].to_vec())
            .expect("build_close_payload preserves UTF-8 boundaries");
        if !self.shared.accepting_writes.swap(false, Ordering::AcqRel) {
            return Ok(());
        }
        let result = self.submit(Command::Close {
            id: self.id,
            outbound: Outbound::new(encoded, None),
            reason: retained_reason,
        });
        if let Err(reason) = &result {
            self.cancel(reason.clone());
        }
        result
    }

    pub(crate) fn cancel(&self, reason: impl Into<String>) {
        self.shared.accepting_writes.store(false, Ordering::Release);
        *self.shared.cancel_reason.lock() = Some(reason.into());
        self.shared.cancelled.store(true, Ordering::Release);
        let _ = self.control.waker.wake();
    }

    pub(crate) fn is_closed(&self) -> bool {
        !self.shared.accepting_writes.load(Ordering::Acquire)
    }

    /// A client masks every frame it sends; a server masks none.
    fn mask(&self) -> Option<[u8; 4]> {
        match self.role {
            PeerRole::Server => None,
            PeerRole::Client => Some(rand::random()),
        }
    }

    /// Room in the connection's and the reactor's write budgets for a frame
    /// of `payload_bytes` (a frame's header takes at most 14 more).
    fn reserve_frame(&self, payload_bytes: usize) -> Result<Reservation, String> {
        let bytes = payload_bytes + 14;
        if !self.shared.local_budget.reserve(bytes) {
            return Err("BACKPRESSURE: WebSocket outbound queue is full".to_string());
        }
        if !self.control.aggregate_write_budget.reserve(bytes) {
            self.shared.local_budget.release(bytes);
            return Err("BACKPRESSURE: aggregate WebSocket outbound queue is full".to_string());
        }
        Ok(Reservation {
            bytes,
            local: Arc::clone(&self.shared.local_budget),
            aggregate: Arc::clone(&self.control.aggregate_write_budget),
        })
    }

    fn submit(&self, command: Command) -> Result<(), String> {
        match self.control.commands.try_send(command) {
            Ok(()) => {
                let _ = self.control.waker.wake();
                Ok(())
            }
            Err(TrySendError::Full(_)) => {
                Err("BACKPRESSURE: WebSocket reactor command queue is full".to_string())
            }
            Err(TrySendError::Disconnected(_)) => {
                Err("WebSocket reactor is unavailable".to_string())
            }
        }
    }
}

/// A reactor's command queue and waker, and what its limits count.
struct ReactorControl {
    commands: Sender<Command>,
    waker: Waker,
    limits: ReactorLimits,
    connections: AtomicUsize,
    tls_handshakes: AtomicUsize,
    aggregate_write_budget: Arc<QueueBudget>,
    inbound_budget: Arc<QueueBudget>,
}

impl ReactorControl {
    fn new(commands: Sender<Command>, waker: Waker, limits: ReactorLimits) -> Self {
        Self {
            commands,
            waker,
            limits,
            connections: AtomicUsize::new(0),
            tls_handshakes: AtomicUsize::new(0),
            aggregate_write_budget: Arc::new(QueueBudget::new(
                limits.write_items,
                limits.write_bytes,
            )),
            inbound_budget: Arc::new(QueueBudget::new(limits.inbound_items, limits.inbound_bytes)),
        }
    }
}

/// A connection counted against its reactor's limit while it lives.
struct ConnectionSlot(Arc<ReactorControl>);

impl Drop for ConnectionSlot {
    fn drop(&mut self) {
        self.0.connections.fetch_sub(1, Ordering::AcqRel);
    }
}

/// A TLS handshake counted against its reactor's limit until it completes.
struct TlsHandshakeSlot(Arc<ReactorControl>);

impl TlsHandshakeSlot {
    fn reserve(
        control: &Arc<ReactorControl>,
        stream: &ReactorTransport,
    ) -> Result<Option<Self>, String> {
        if !stream.is_handshaking() {
            return Ok(None);
        }
        if !reserve_counter(&control.tls_handshakes, 1, control.limits.tls_handshakes) {
            return Err("WebSocket TLS handshake limit reached".to_string());
        }
        Ok(Some(Self(Arc::clone(control))))
    }
}

impl Drop for TlsHandshakeSlot {
    fn drop(&mut self) {
        self.0.tls_handshakes.fetch_sub(1, Ordering::AcqRel);
    }
}

enum Command {
    Register {
        entry: Box<Entry>,
    },
    Write {
        id: u64,
        outbound: Outbound,
    },
    Close {
        id: u64,
        outbound: Outbound,
        reason: String,
    },
}

struct Outbound {
    bytes: Vec<u8>,
    offset: usize,
    _reservation: Option<Reservation>,
}

impl Outbound {
    fn new(bytes: Vec<u8>, reservation: Option<Reservation>) -> Self {
        Self {
            bytes,
            offset: 0,
            _reservation: reservation,
        }
    }
}

fn prioritize_close(queue: &mut VecDeque<Outbound>, keep_front: bool, close: Outbound) {
    if keep_front {
        queue.truncate(1);
        queue.push_back(close);
    } else {
        queue.clear();
        queue.push_back(close);
    }
}

/// Where a connection is in its opening handshake, or open. Frames that
/// arrive before a server connection opens wait in the decoder.
enum Phase {
    ServerHandshake {
        handler: Arc<dyn ServerHandshakeHandler>,
        buffer: Vec<u8>,
    },
    /// The request is read and the 101 answer queued; the connection opens
    /// once the answer is flushed.
    ServerReply {
        handler: Arc<dyn ServerHandshakeHandler>,
        path: String,
        headers: Vec<(String, String)>,
    },
    ClientHandshake {
        sink: Arc<dyn ReactorEventSink>,
        client_key: String,
        buffer: Vec<u8>,
    },
    Open {
        sink: Arc<dyn ReactorEventSink>,
    },
}

struct Heartbeat {
    last_ping: Instant,
    pending: Option<([u8; 4], Instant)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CloseState {
    Open,
    AwaitingPeer,
    Replying,
}

struct Entry {
    id: u64,
    token: Token,
    connection: ReactorConnection,
    _slot: ConnectionSlot,
    tls_handshake_slot: Option<TlsHandshakeSlot>,
    stream: ReactorTransport,
    phase: Phase,
    /// When the opening handshake must be done by; none once open.
    handshake_deadline: Option<Instant>,
    config: ReactorConfig,
    decoder: FrameDecoder,
    assembler: MessageAssembler,
    outbound: VecDeque<Outbound>,
    heartbeat: Heartbeat,
    read_ready: bool,
    write_ready: bool,
    frames_ready: bool,
    tls_close_notify_sent: bool,
    close_state: CloseState,
    close_deadline: Option<Instant>,
    termination_reason: String,
    dead: bool,
}

impl Entry {
    fn interest(&self) -> Interest {
        if self.needs_write_interest() {
            Interest::READABLE.add(Interest::WRITABLE)
        } else {
            Interest::READABLE
        }
    }

    fn needs_write_interest(&self) -> bool {
        self.stream.wants_write() || (!self.outbound.is_empty() && !self.stream.is_handshaking())
    }

    /// Whether the connection still takes frames to send: open, not closing.
    fn accepts_frames(&self) -> bool {
        self.close_state == CloseState::Open && !self.dead
    }

    fn release_tls_handshake_slot(&mut self) {
        if !self.stream.is_handshaking() {
            self.tls_handshake_slot.take();
        }
    }

    fn buffered_bytes(&self) -> usize {
        let handshake = match &self.phase {
            Phase::ServerHandshake { buffer, .. } | Phase::ClientHandshake { buffer, .. } => {
                buffer.len()
            }
            Phase::ServerReply { .. } | Phase::Open { .. } => 0,
        };
        handshake + self.decoder.buffered_len() + self.assembler.buffered_len()
    }

    /// Whether the connection takes more input now. Frames already read
    /// wait for their turn, and a server's 101 answer goes out before the
    /// frames after its request are read: until then, what the peer sends
    /// stays in the socket, rather than the decoder growing. A connection
    /// replying to a close, or failed, reads nothing more.
    fn takes_input(&self) -> bool {
        !self.frames_ready
            && !matches!(self.phase, Phase::ServerReply { .. })
            && self.close_state != CloseState::Replying
    }

    fn readable(&mut self) -> bool {
        let mut bytes_read = 0usize;
        let mut buffer = [0u8; 16 * 1024];
        while bytes_read < READ_BUDGET_BYTES && !self.dead && self.takes_input() {
            let allowance = (READ_BUDGET_BYTES - bytes_read).min(buffer.len());
            match self.stream.read(&mut buffer[..allowance]) {
                Ok(0) => {
                    self.fail("WebSocket peer disconnected");
                    return false;
                }
                Ok(count) => {
                    bytes_read += count;
                    if let Err(reason) = self.consume_bytes(&buffer[..count]) {
                        self.protocol_failure(&reason);
                        break;
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    // TLS decrypts what it reads next; a plain socket has
                    // nothing more.
                    let ReactorTransport::Tls { session, socket } = &mut self.stream else {
                        return false;
                    };
                    if !session.wants_read() {
                        return false;
                    }
                    match read_tls(session, socket) {
                        Ok(0) => {
                            self.fail("WebSocket peer disconnected");
                            return false;
                        }
                        Ok(count) => bytes_read += count,
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => return false,
                        Err(error) => {
                            self.fail(&format!("read WebSocket TLS transport: {error}"));
                            return false;
                        }
                    }
                }
                Err(error) => {
                    self.fail(&format!("read WebSocket transport: {error}"));
                    return false;
                }
            }
        }
        !self.dead
    }

    fn consume_bytes(&mut self, bytes: &[u8]) -> Result<(), String> {
        match &mut self.phase {
            Phase::ServerHandshake { buffer, .. } | Phase::ClientHandshake { buffer, .. } => {
                buffer.extend_from_slice(bytes)
            }
            Phase::ServerReply { .. } | Phase::Open { .. } => self.decoder.extend(bytes),
        }
        self.advance_handshake()?;
        self.process_frames()
    }

    /// Completes the opening handshake whose request or answer the buffer
    /// now holds whole; what follows it goes to the decoder.
    fn advance_handshake(&mut self) -> Result<(), String> {
        match &mut self.phase {
            Phase::ServerHandshake { handler, buffer } => {
                if let Some(parsed) = parse_upgrade_request_bytes(buffer)? {
                    let reply = Phase::ServerReply {
                        handler: Arc::clone(handler),
                        path: parsed.path,
                        headers: parsed.headers,
                    };
                    self.decoder.extend(&buffer[parsed.consumed..]);
                    self.outbound
                        .push_back(Outbound::new(parsed.response, None));
                    self.phase = reply;
                }
            }
            Phase::ClientHandshake {
                sink,
                client_key,
                buffer,
            } => {
                if let Some(consumed) = parse_upgrade_response_bytes(buffer, client_key)? {
                    let sink = Arc::clone(sink);
                    self.decoder.extend(&buffer[consumed..]);
                    self.open(sink);
                }
            }
            Phase::ServerReply { .. } | Phase::Open { .. } => {}
        }
        Ok(())
    }

    fn open(&mut self, sink: Arc<dyn ReactorEventSink>) {
        sink.opened();
        self.handshake_deadline = None;
        self.phase = Phase::Open { sink };
    }

    /// Once the 101 answer is flushed, the handler takes the connection and
    /// the frames that came with the request are read.
    fn finish_server_reply(&mut self) -> Result<(), String> {
        let flushed = self.outbound.is_empty() && !self.stream.wants_write();
        let (
            Phase::ServerReply {
                handler,
                path,
                headers,
            },
            true,
        ) = (&mut self.phase, flushed)
        else {
            return Ok(());
        };
        let sink = handler.opened(
            self.connection.clone(),
            std::mem::take(path),
            std::mem::take(headers),
        );
        self.open(sink);
        self.process_frames()
    }

    fn process_frames(&mut self) -> Result<(), String> {
        self.frames_ready = false;
        let Phase::Open { sink } = &self.phase else {
            return Ok(());
        };
        let sink = Arc::clone(sink);
        for _ in 0..FRAME_BUDGET_ITEMS {
            let Some((frame, masked)) = self.decoder.next_frame()? else {
                return Ok(());
            };
            match (self.config.role, masked) {
                (PeerRole::Server, false) => {
                    return Err("client sent an unmasked WebSocket frame".to_string());
                }
                (PeerRole::Client, true) => {
                    return Err("server sent a masked WebSocket frame".to_string());
                }
                _ => {}
            }
            self.process_frame(frame, &sink)?;
            if self.close_state == CloseState::Replying {
                return Ok(());
            }
        }
        self.frames_ready = true;
        Ok(())
    }

    fn process_frame(
        &mut self,
        frame: WsFrame,
        sink: &Arc<dyn ReactorEventSink>,
    ) -> Result<(), String> {
        if self.close_state != CloseState::Open && frame.opcode != WsOpcode::Close {
            return Ok(());
        }
        match frame.opcode {
            WsOpcode::Ping => self.queue_internal(WsOpcode::Pong, &frame.payload),
            WsOpcode::Pong => {
                if self
                    .heartbeat
                    .pending
                    .is_some_and(|(payload, _)| frame.payload == payload)
                {
                    self.heartbeat.pending = None;
                }
                Ok(())
            }
            WsOpcode::Close => {
                let (code, reason) = parse_close_payload_strict(&frame.payload)?;
                self.connection
                    .shared
                    .accepting_writes
                    .store(false, Ordering::Release);
                self.close_deadline = Some(Instant::now() + CLOSE_DEADLINE);
                self.termination_reason = "peer closed".to_string();
                if self.close_state == CloseState::Open {
                    let close = self.close_outbound(&frame.payload);
                    self.prioritize_close(close);
                }
                self.close_state = CloseState::Replying;
                self.finish_close_if_flushed();
                let _ = sink.event(ReactorEvent::Close(code, reason));
                Ok(())
            }
            WsOpcode::Text | WsOpcode::Binary | WsOpcode::Continuation => {
                match self.assembler.push(frame) {
                    ReassembleResult::Complete(message) => {
                        let Some(permit) = InboundPermit::reserve(
                            &self.connection.control.inbound_budget,
                            message.payload.len(),
                        ) else {
                            self.start_close(
                                WsCloseCode::TRY_AGAIN_LATER,
                                "aggregate inbound queue full",
                            );
                            return Ok(());
                        };
                        // A message is text or binary, as its first frame was.
                        let event = if message.opcode == WsOpcode::Text {
                            if !is_valid_text_payload(&message.payload) {
                                return Err("invalid UTF-8 in text message".to_string());
                            }
                            ReactorEvent::Text(message.payload, permit)
                        } else {
                            ReactorEvent::Binary(message.payload, permit)
                        };
                        if sink.event(event).is_err() {
                            self.start_close(WsCloseCode::TRY_AGAIN_LATER, "inbound queue full");
                        }
                        Ok(())
                    }
                    ReassembleResult::Accumulating => Ok(()),
                    ReassembleResult::TooLarge => {
                        self.start_close(WsCloseCode::MESSAGE_TOO_BIG, "message too big");
                        Ok(())
                    }
                    ReassembleResult::ProtocolError(reason) => Err(reason.to_string()),
                }
            }
        }
    }

    fn queue_internal(&mut self, opcode: WsOpcode, payload: &[u8]) -> Result<(), String> {
        let reservation = self.connection.reserve_frame(payload.len())?;
        let encoded = encode_frame(opcode, payload, self.connection.mask());
        self.outbound
            .push_back(Outbound::new(encoded, Some(reservation)));
        Ok(())
    }

    fn prioritize_close(&mut self, close: Outbound) {
        let keep_front = self.stream.wants_write()
            || self
                .outbound
                .front()
                .is_some_and(|outbound| outbound.offset > 0);
        prioritize_close(&mut self.outbound, keep_front, close);
    }

    fn close_outbound(&self, payload: &[u8]) -> Outbound {
        Outbound::new(
            encode_frame(WsOpcode::Close, payload, self.connection.mask()),
            None,
        )
    }

    fn start_close(&mut self, code: u16, reason: &str) {
        if self.close_state != CloseState::Open {
            return;
        }
        self.connection
            .shared
            .accepting_writes
            .store(false, Ordering::Release);
        let close = self.close_outbound(&build_close_payload(code, reason));
        self.prioritize_close(close);
        self.close_state = CloseState::AwaitingPeer;
        self.close_deadline = Some(Instant::now() + CLOSE_DEADLINE);
        self.termination_reason = reason.to_string();
    }

    fn finish_close_if_flushed(&mut self) {
        if self.close_state != CloseState::Replying
            || !self.outbound.is_empty()
            || self.stream.wants_write()
        {
            return;
        }
        match &mut self.stream {
            ReactorTransport::Tls { session, .. } if !self.tls_close_notify_sent => {
                session.send_close_notify();
                self.tls_close_notify_sent = true;
                self.write_ready = true;
            }
            _ => self.dead = true,
        }
    }

    /// Fails the connection on input it cannot take (RFC 6455 7.1.7): an
    /// open one sends the close its error calls for, unless it has sent
    /// one; it reads nothing more, and ends once its close is out.
    fn protocol_failure(&mut self, reason: &str) {
        if !matches!(self.phase, Phase::Open { .. }) {
            self.fail(reason);
            return;
        }
        let code = if reason.contains("UTF-8") {
            WsCloseCode::INVALID_DATA
        } else if reason.contains("maximum") {
            WsCloseCode::MESSAGE_TOO_BIG
        } else {
            WsCloseCode::PROTOCOL_ERROR
        };
        self.start_close(code, reason);
        self.termination_reason = reason.to_string();
        self.close_state = CloseState::Replying;
        self.finish_close_if_flushed();
    }

    fn writable(&mut self) -> bool {
        let mut written = 0usize;
        let mut network_written = false;
        let mut blocked = false;
        while written < WRITE_BUDGET_BYTES && !self.dead {
            // TLS records first, then the plaintext they carry.
            let progress = if let Some(flushed) = self.stream.flush_tls() {
                network_written = true;
                flushed
            } else {
                let Some(outbound) = self.outbound.front_mut() else {
                    break;
                };
                if outbound.offset == outbound.bytes.len() {
                    // TLS holds a handshake's plaintext until the handshake is done.
                    if self.stream.is_handshaking() {
                        blocked = true;
                        break;
                    }
                    self.outbound.pop_front();
                    continue;
                }
                let end =
                    (outbound.offset + WRITE_BUDGET_BYTES - written).min(outbound.bytes.len());
                self.stream
                    .write(&outbound.bytes[outbound.offset..end])
                    .inspect(|count| outbound.offset += count)
            };
            // A write that took nothing waits for writability, as one that
            // would block does: TLS takes nothing while its buffer is full.
            match progress.and_then(|count| {
                (count > 0)
                    .then_some(count)
                    .ok_or(io::ErrorKind::WouldBlock.into())
            }) {
                Ok(count) => written += count,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    blocked = true;
                    break;
                }
                Err(error) => {
                    self.fail(&format!("write WebSocket transport: {error}"));
                    break;
                }
            }
        }

        if let Err(reason) = self.finish_server_reply() {
            self.protocol_failure(&reason);
        }
        if network_written && self.stream.wants_read() {
            self.read_ready = true;
        }
        self.finish_close_if_flushed();
        !self.dead && !blocked && self.needs_write_interest()
    }

    fn preflight(&mut self, now: Instant) {
        if self.connection.shared.cancelled.load(Ordering::Acquire) {
            let reason = self.connection.shared.cancel_reason.lock().clone();
            self.fail(&reason.unwrap_or_default());
        } else if self
            .handshake_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            self.fail("TIMEOUT: WebSocket handshake");
        } else if self.close_deadline.is_some_and(|deadline| now >= deadline) {
            self.dead = true;
        }
    }

    fn tick(&mut self, now: Instant) {
        if self.dead
            || !matches!(self.phase, Phase::Open { .. })
            || self.close_state != CloseState::Open
        {
            return;
        }
        if self
            .heartbeat
            .pending
            .is_some_and(|(_, sent)| now.duration_since(sent) >= self.config.pong_timeout)
        {
            self.start_close(WsCloseCode::GOING_AWAY, "HEARTBEAT_TIMEOUT");
            return;
        }
        if self.heartbeat.pending.is_none()
            && now.duration_since(self.heartbeat.last_ping) >= self.config.ping_interval
        {
            let payload: [u8; 4] = rand::random();
            match self.queue_internal(WsOpcode::Ping, &payload) {
                Ok(()) => {
                    self.heartbeat.last_ping = now;
                    self.heartbeat.pending = Some((payload, now));
                }
                Err(reason) => self.fail(&reason),
            }
        }
    }

    fn fail(&mut self, reason: &str) {
        if !self.dead {
            self.termination_reason = reason.to_string();
            self.dead = true;
        }
    }

    fn notify_terminated(&self) {
        self.connection
            .shared
            .accepting_writes
            .store(false, Ordering::Release);
        match &self.phase {
            Phase::ServerHandshake { handler, .. } | Phase::ServerReply { handler, .. } => {
                handler.failed(&self.termination_reason)
            }
            Phase::ClientHandshake { sink, .. } | Phase::Open { sink } => {
                sink.terminated(&self.termination_reason)
            }
        }
    }
}

/// Reads TLS records from `socket` and decrypts them: the bytes read, none
/// at end of stream.
fn read_tls(session: &mut Connection, socket: &mut mio::net::TcpStream) -> io::Result<usize> {
    let count = session.read_tls(socket)?;
    if count > 0 {
        session
            .process_new_packets()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    }
    Ok(count)
}

fn reactor() -> Result<&'static Arc<ReactorControl>, String> {
    static REACTOR: OnceLock<Result<Arc<ReactorControl>, String>> = OnceLock::new();
    REACTOR
        .get_or_init(|| start_reactor(PROCESS_LIMITS))
        .as_ref()
        .map_err(Clone::clone)
}

fn start_reactor(limits: ReactorLimits) -> Result<Arc<ReactorControl>, String> {
    let poll = Poll::new().map_err(|error| format!("create WebSocket poller: {error}"))?;
    let waker = Waker::new(poll.registry(), WAKE_TOKEN)
        .map_err(|error| format!("create WebSocket reactor waker: {error}"))?;
    let (commands, receiver) = crossbeam_channel::bounded(COMMAND_QUEUE_ITEMS);
    let control = Arc::new(ReactorControl::new(commands, waker, limits));
    std::thread::Builder::new()
        .name("mesh-ws-reactor".to_string())
        .spawn(move || reactor_loop(poll, receiver, limits.read_bytes))
        .map_err(|error| format!("start WebSocket reactor: {error}"))?;
    Ok(control)
}

pub(crate) fn register_server(
    stream: ReactorTransport,
    handler: Arc<dyn ServerHandshakeHandler>,
    config: ReactorConfig,
) -> Result<ReactorConnection, String> {
    let phase = Phase::ServerHandshake {
        handler,
        buffer: Vec::new(),
    };
    register(reactor()?, stream, phase, VecDeque::new(), config)
}

pub(crate) fn register_client(
    stream: ReactorTransport,
    request: Vec<u8>,
    client_key: String,
    sink: Arc<dyn ReactorEventSink>,
    config: ReactorConfig,
) -> Result<ReactorConnection, String> {
    let phase = Phase::ClientHandshake {
        sink,
        client_key,
        buffer: Vec::new(),
    };
    let request = VecDeque::from([Outbound::new(request, None)]);
    register(reactor()?, stream, phase, request, config)
}

/// Hands `stream` to the reactor `control` drives, in its opening `phase`
/// with `outbound` queued.
fn register(
    control: &Arc<ReactorControl>,
    stream: ReactorTransport,
    phase: Phase,
    outbound: VecDeque<Outbound>,
    config: ReactorConfig,
) -> Result<ReactorConnection, String> {
    let (entry, connection) = new_entry(control, stream, phase, outbound, config)?;
    connection.submit(Command::Register {
        entry: Box::new(entry),
    })?;
    Ok(connection)
}

/// The reactor's entry for `stream`, and the connection its owner drives it
/// through, within `control`'s limits.
fn new_entry(
    control: &Arc<ReactorControl>,
    stream: ReactorTransport,
    phase: Phase,
    outbound: VecDeque<Outbound>,
    config: ReactorConfig,
) -> Result<(Entry, ReactorConnection), String> {
    let tls_handshake_slot = TlsHandshakeSlot::reserve(control, &stream)?;
    if !reserve_counter(&control.connections, 1, control.limits.connections) {
        return Err("WebSocket reactor connection limit reached".to_string());
    }
    let slot = ConnectionSlot(Arc::clone(control));
    let id = NEXT_CONNECTION_ID.fetch_add(1, Ordering::Relaxed);
    let connection = ReactorConnection {
        id,
        role: config.role,
        max_message_bytes: config.max_message_bytes,
        shared: Arc::new(ConnectionShared {
            accepting_writes: AtomicBool::new(true),
            cancelled: AtomicBool::new(false),
            cancel_reason: Mutex::new(None),
            local_budget: Arc::new(QueueBudget::new(
                config.max_write_queue_items,
                config.max_write_queue_bytes,
            )),
        }),
        control: Arc::clone(control),
    };
    let now = Instant::now();
    let entry = Entry {
        id,
        token: Token(id as usize),
        connection: connection.clone(),
        _slot: slot,
        tls_handshake_slot,
        stream,
        phase,
        handshake_deadline: Some(now + config.handshake_timeout),
        decoder: FrameDecoder::new(config.max_message_bytes),
        assembler: MessageAssembler::new(config.max_message_bytes),
        config,
        outbound,
        heartbeat: Heartbeat {
            last_ping: now,
            pending: None,
        },
        read_ready: false,
        write_ready: false,
        frames_ready: false,
        tls_close_notify_sent: false,
        close_state: CloseState::Open,
        close_deadline: None,
        termination_reason: "WebSocket connection closed".to_string(),
        dead: false,
    };
    Ok((entry, connection))
}

fn reactor_loop(mut poll: Poll, receiver: Receiver<Command>, read_limit: usize) {
    let mut events = Events::with_capacity(1024);
    let mut entries = HashMap::<u64, Box<Entry>>::new();
    loop {
        // Input a connection does not take yet waits in its socket, and
        // read_ready remembers it (readiness is edge-triggered): it makes no
        // turn immediate.
        let immediate = entries.values().any(|entry| {
            (entry.read_ready && entry.takes_input()) || entry.write_ready || entry.frames_ready
        });
        let timeout = if immediate {
            Duration::ZERO
        } else {
            REACTOR_TICK
        };
        // A poll fails only when a signal interrupts it; the next turn polls
        // again.
        let _ = poll.poll(&mut events, Some(timeout));

        // The waker's token is no connection's: a wake only drains commands.
        for event in &events {
            if let Some(entry) = entries.get_mut(&(event.token().0 as u64)) {
                entry.read_ready |=
                    event.is_readable() || event.is_read_closed() || event.is_error();
                entry.write_ready |=
                    event.is_writable() || event.is_write_closed() || event.is_error();
            }
        }

        drain_commands(&poll, &receiver, &mut entries);
        let now = Instant::now();
        let mut aggregate_read_bytes: usize =
            entries.values().map(|entry| entry.buffered_bytes()).sum();
        for entry in entries.values_mut() {
            let buffered_before = entry.buffered_bytes();
            let had_pending_write = entry.needs_write_interest();
            entry.preflight(now);
            if entry.read_ready && !entry.dead && aggregate_read_bytes < read_limit {
                entry.read_ready = entry.readable();
                if entry.needs_write_interest() {
                    entry.write_ready = true;
                }
            }
            if entry.write_ready && !entry.dead {
                entry.write_ready = entry.writable();
            }
            entry.release_tls_handshake_slot();
            entry.tick(now);
            if !entry.dead && entry.frames_ready {
                if let Err(reason) = entry.process_frames() {
                    entry.protocol_failure(&reason);
                }
            }
            aggregate_read_bytes = aggregate_read_bytes
                .saturating_sub(buffered_before)
                .saturating_add(entry.buffered_bytes());
            let has_pending_write = entry.needs_write_interest();
            if !entry.dead && had_pending_write != has_pending_write {
                reregister_writable(&poll, entry);
            }
        }

        // Over the read limit, the connections holding the most unread bytes
        // go first.
        while aggregate_read_bytes >= read_limit {
            let Some(entry) = entries
                .values_mut()
                .filter(|entry| !entry.dead && entry.buffered_bytes() > 0)
                .max_by_key(|entry| entry.buffered_bytes())
            else {
                break;
            };
            aggregate_read_bytes = aggregate_read_bytes.saturating_sub(entry.buffered_bytes());
            entry.fail("BACKPRESSURE: aggregate WebSocket read queue is full");
        }

        entries.retain(|_, entry| {
            if entry.dead {
                let _ = poll.registry().deregister(entry.stream.source());
                entry.stream.shutdown();
                entry.notify_terminated();
            }
            !entry.dead
        });
    }
}

fn drain_commands(
    poll: &Poll,
    receiver: &Receiver<Command>,
    entries: &mut HashMap<u64, Box<Entry>>,
) {
    while let Ok(command) = receiver.try_recv() {
        match command {
            Command::Register { mut entry } => {
                let (token, interest) = (entry.token, entry.interest());
                let registered = poll
                    .registry()
                    .register(entry.stream.source(), token, interest);
                // An entry the poller refuses is dropped at the end of the
                // turn, as a dead one is.
                if let Err(error) = registered {
                    entry.fail(&format!("register WebSocket transport: {error}"));
                }
                entries.insert(entry.id, entry);
            }
            Command::Write { id, outbound } => {
                if let Some(entry) = entries.get_mut(&id).filter(|entry| entry.accepts_frames()) {
                    entry.outbound.push_back(outbound);
                    reregister_writable(poll, entry);
                }
            }
            Command::Close {
                id,
                outbound,
                reason,
            } => {
                if let Some(entry) = entries.get_mut(&id).filter(|entry| entry.accepts_frames()) {
                    entry.prioritize_close(outbound);
                    entry.close_state = CloseState::AwaitingPeer;
                    entry.close_deadline = Some(Instant::now() + CLOSE_DEADLINE);
                    entry.termination_reason = reason;
                    reregister_writable(poll, entry);
                }
            }
        }
    }
}

fn reregister_writable(poll: &Poll, entry: &mut Entry) {
    let (token, interest) = (entry.token, entry.interest());
    let reregistered = poll
        .registry()
        .reregister(entry.stream.source(), token, interest);
    if let Err(error) = reregistered {
        entry.fail(&format!("reregister WebSocket writer: {error}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ws::close::parse_close_payload;
    use crate::ws::frame::{read_frame, read_frame_with_mask, write_frame, write_masked_frame};
    use crate::ws::handshake::compute_accept_key;
    use rustls::{ClientConnection, ServerConnection};
    use std::net::TcpListener;
    use std::sync::mpsc;

    const TIMEOUT: Duration = Duration::from_secs(5);

    /// What a connection's handler and sink were told, in order.
    #[derive(Debug, PartialEq)]
    enum Seen {
        Opened(String),
        Text(Vec<u8>),
        Binary(Vec<u8>),
        Close(u16, String),
        Terminated(String),
        Failed(String),
    }

    /// A handler and sink that report to a channel; the server's opened
    /// connection is kept for the test to send on.
    struct Recorder {
        seen: Mutex<mpsc::Sender<Seen>>,
        connection: Mutex<Option<ReactorConnection>>,
    }

    impl Recorder {
        fn new() -> (Arc<Self>, mpsc::Receiver<Seen>) {
            let (sender, receiver) = mpsc::channel();
            let recorder = Arc::new(Self {
                seen: Mutex::new(sender),
                connection: Mutex::new(None),
            });
            (recorder, receiver)
        }

        fn saw(&self, seen: Seen) {
            let _ = self.seen.lock().send(seen);
        }
    }

    impl ReactorEventSink for Recorder {
        fn opened(&self) {
            self.saw(Seen::Opened(String::new()));
        }

        fn event(&self, event: ReactorEvent) -> Result<(), SinkFull> {
            self.saw(match event {
                ReactorEvent::Text(data, _) => Seen::Text(data),
                ReactorEvent::Binary(data, _) => Seen::Binary(data),
                ReactorEvent::Close(code, reason) => Seen::Close(code, reason),
            });
            Ok(())
        }

        fn terminated(&self, reason: &str) {
            self.saw(Seen::Terminated(reason.to_string()));
        }
    }

    impl ServerHandshakeHandler for Arc<Recorder> {
        fn opened(
            &self,
            connection: ReactorConnection,
            path: String,
            _headers: Vec<(String, String)>,
        ) -> Arc<dyn ReactorEventSink> {
            *self.connection.lock() = Some(connection);
            self.saw(Seen::Opened(path));
            Arc::clone(self) as Arc<dyn ReactorEventSink>
        }

        fn failed(&self, reason: &str) {
            self.saw(Seen::Failed(reason.to_string()));
        }
    }

    fn next(seen: &mpsc::Receiver<Seen>) -> Seen {
        seen.recv_timeout(TIMEOUT).unwrap()
    }

    /// A connected pair of sockets: (ours, the reactor's).
    fn socket_pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let ours = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        ours.set_read_timeout(Some(TIMEOUT)).unwrap();
        (ours, listener.accept().unwrap().0)
    }

    /// A reactor of the test's own, with `limits`.
    fn reactor_with(limits: ReactorLimits) -> Arc<ReactorControl> {
        start_reactor(limits).unwrap()
    }

    /// A server connection on `control`'s reactor under `config`, and the
    /// raw client connected to it.
    fn server_on(
        control: &Arc<ReactorControl>,
        config: ReactorConfig,
    ) -> (TcpStream, Arc<Recorder>, mpsc::Receiver<Seen>) {
        let (client, tcp) = socket_pair();
        let (recorder, seen) = Recorder::new();
        let phase = Phase::ServerHandshake {
            handler: Arc::new(Arc::clone(&recorder)),
            buffer: Vec::new(),
        };
        register(
            control,
            ReactorTransport::plain(tcp),
            phase,
            VecDeque::new(),
            config,
        )
        .unwrap();
        (client, recorder, seen)
    }

    fn server(config: ReactorConfig) -> (TcpStream, Arc<Recorder>, mpsc::Receiver<Seen>) {
        server_on(reactor().unwrap(), config)
    }

    /// An entry for a descriptor no poller takes (a directory), with a
    /// frame queued so it asks to write.
    fn unpollable_entry() -> Entry {
        use std::os::fd::{FromRawFd, IntoRawFd};
        let directory = std::fs::File::open(std::env::temp_dir()).unwrap();
        let socket = unsafe { TcpStream::from_raw_fd(directory.into_raw_fd()) };
        let (recorder, _seen) = Recorder::new();
        let phase = Phase::Open {
            sink: Arc::clone(&recorder) as Arc<dyn ReactorEventSink>,
        };
        let outbound = VecDeque::from([Outbound::new(b"frame".to_vec(), None)]);
        let transport = ReactorTransport::Plain(mio::net::TcpStream::from_std(socket));
        let config = ReactorConfig::server(1024);
        new_entry(reactor().unwrap(), transport, phase, outbound, config)
            .unwrap()
            .0
    }

    /// A connection the poller refuses to watch, as it registers or as it
    /// asks to write, fails with the poller's reason.
    #[test]
    fn a_connection_the_poller_refuses_fails() {
        let poll = Poll::new().unwrap();
        let (commands, receiver) = crossbeam_channel::unbounded();
        let entry = unpollable_entry();
        let id = entry.id;
        commands
            .send(Command::Register {
                entry: Box::new(entry),
            })
            .unwrap();
        let mut entries = HashMap::new();
        drain_commands(&poll, &receiver, &mut entries);
        let registered = &entries[&id];
        assert!(registered.dead);
        assert!(registered
            .termination_reason
            .starts_with("register WebSocket transport: "));

        let mut entry = unpollable_entry();
        reregister_writable(&poll, &mut entry);
        assert!(entry.dead);
        assert!(entry
            .termination_reason
            .starts_with("reregister WebSocket writer: "));
    }

    const UPGRADE: &[u8] = b"GET /feed HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\n\
        Connection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
        Sec-WebSocket-Version: 13\r\n\r\n";

    /// Read an HTTP head, through its blank line.
    fn read_head(stream: &mut impl Read) -> String {
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).unwrap();
            head.push(byte[0]);
        }
        String::from_utf8(head).unwrap()
    }

    /// Send the upgrade request with `then` in the same write, and read the
    /// server's 101 answer and its opening.
    fn upgrade(client: &mut TcpStream, then: &[u8], seen: &mpsc::Receiver<Seen>) {
        client.write_all(&[UPGRADE, then].concat()).unwrap();
        assert!(read_head(client).starts_with("HTTP/1.1 101"));
        assert_eq!(next(seen), Seen::Opened("/feed".to_string()));
        assert_eq!(next(seen), Seen::Opened(String::new()));
    }

    /// A server connection on the process reactor, upgraded.
    fn open_server(config: ReactorConfig) -> (TcpStream, Arc<Recorder>, mpsc::Receiver<Seen>) {
        let (mut client, recorder, seen) = server(config);
        upgrade(&mut client, &[], &seen);
        (client, recorder, seen)
    }

    /// A masked frame, as a client sends.
    fn masked(opcode: WsOpcode, payload: &[u8], fin: bool) -> Vec<u8> {
        let mut frame = Vec::new();
        write_masked_frame(&mut frame, opcode, payload, fin, [1, 2, 3, 4]).unwrap();
        frame
    }

    /// The close frame the peer reads next: its code.
    fn close_code(peer: &mut impl Read) -> u16 {
        let frame = read_frame(peer).unwrap();
        assert_eq!(frame.opcode, WsOpcode::Close);
        parse_close_payload(&frame.payload).0
    }

    /// A client connection on the process reactor under `config` over
    /// `transport`, its upgrade request `request`; the server side answers.
    fn client(
        transport: ReactorTransport,
        request: Vec<u8>,
        config: ReactorConfig,
    ) -> (ReactorConnection, mpsc::Receiver<Seen>) {
        let (recorder, seen) = Recorder::new();
        let connection = register_client(
            transport,
            request,
            "dGhlIHNhbXBsZSBub25jZQ==".to_string(),
            recorder,
            config,
        )
        .unwrap();
        (connection, seen)
    }

    /// Answer a client's upgrade request as a server does.
    fn accept_upgrade(peer: &mut (impl Read + Write)) {
        read_head(peer);
        write!(
            peer,
            "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
             Sec-WebSocket-Accept: {}\r\n\r\n",
            compute_accept_key("dGhlIHNhbXBsZSBub25jZQ==")
        )
        .unwrap();
        peer.flush().unwrap();
    }

    /// A plain client connection, open: the raw server side and the
    /// connection's events.
    fn open_client(config: ReactorConfig) -> (TcpStream, ReactorConnection, mpsc::Receiver<Seen>) {
        let (mut peer, tcp) = socket_pair();
        let (connection, seen) = client(ReactorTransport::plain(tcp), UPGRADE.to_vec(), config);
        accept_upgrade(&mut peer);
        assert_eq!(next(&seen), Seen::Opened(String::new()));
        (peer, connection, seen)
    }

    /// A connection on a hand-made control no reactor drains, its commands
    /// left in a queue of `commands`.
    fn unattended(
        commands: usize,
        limits: ReactorLimits,
        local: QueueBudget,
    ) -> (ReactorConnection, crossbeam_channel::Receiver<Command>) {
        let poll = Poll::new().unwrap();
        let waker = Waker::new(poll.registry(), WAKE_TOKEN).unwrap();
        let (sender, receiver) = crossbeam_channel::bounded(commands);
        let connection = ReactorConnection {
            id: 1,
            role: PeerRole::Server,
            max_message_bytes: 1024,
            shared: Arc::new(ConnectionShared {
                accepting_writes: AtomicBool::new(true),
                cancelled: AtomicBool::new(false),
                cancel_reason: Mutex::new(None),
                local_budget: Arc::new(local),
            }),
            control: Arc::new(ReactorControl::new(sender, waker, limits)),
        };
        (connection, receiver)
    }

    #[test]
    fn graceful_close_retains_only_the_protocol_encoded_reason() {
        let (connection, receiver) = unattended(1, PROCESS_LIMITS, QueueBudget::new(1, 1024));
        let supplied = "reason".repeat(1024);

        connection.graceful_close(1000, &supplied).unwrap();

        let Command::Close { reason, .. } = receiver.recv().unwrap() else {
            panic!("graceful close queued the wrong command");
        };
        let payload = build_close_payload(1000, &supplied);
        assert_eq!(reason.as_bytes(), &payload[2..]);
        assert!(reason.len() <= 123);
    }

    /// What a connection refuses to send, before and after it closes, and a
    /// reactor that cannot take a command.
    #[test]
    fn sends_are_refused_when_they_cannot_be_queued() {
        let (connection, receiver) = unattended(1, PROCESS_LIMITS, QueueBudget::new(4, 1024));
        assert_eq!(
            connection.send(WsOpcode::Text, &[0; 1025]),
            Err("MESSAGE_TOO_BIG".to_string())
        );
        assert_eq!(connection.send(WsOpcode::Text, b"one"), Ok(()));
        assert_eq!(
            connection.send(WsOpcode::Text, b"two"),
            Err("BACKPRESSURE: WebSocket reactor command queue is full".to_string())
        );
        assert_eq!(
            connection.graceful_close(1005, ""),
            Err("invalid WebSocket close code".to_string())
        );
        assert_eq!(
            connection.graceful_close(1000, ""),
            Err("BACKPRESSURE: WebSocket reactor command queue is full".to_string())
        );
        assert!(connection.is_closed());
        assert_eq!(connection.graceful_close(1000, ""), Ok(()));
        assert_eq!(
            connection.send(WsOpcode::Text, b"three"),
            Err("WebSocket connection is closed".to_string())
        );

        drop(receiver);
        let (connection, receiver) = unattended(1, PROCESS_LIMITS, QueueBudget::new(4, 1024));
        drop(receiver);
        assert_eq!(
            connection.send(WsOpcode::Text, b"one"),
            Err("WebSocket reactor is unavailable".to_string())
        );
    }

    /// A frame needs room in its connection's write budget and then its
    /// reactor's; refused by the second, it gives the first back.
    #[test]
    fn write_budgets_refuse_frames_they_cannot_hold() {
        let (connection, _receiver) = unattended(4, PROCESS_LIMITS, QueueBudget::new(1, 1024));
        connection.send(WsOpcode::Text, b"one").unwrap();
        assert_eq!(
            connection.send(WsOpcode::Text, b"two"),
            Err("BACKPRESSURE: WebSocket outbound queue is full".to_string())
        );

        let limits = ReactorLimits {
            write_items: 0,
            ..PROCESS_LIMITS
        };
        let (connection, _receiver) = unattended(4, limits, QueueBudget::new(1, 1024));
        assert_eq!(
            connection.send(WsOpcode::Text, b"one"),
            Err("BACKPRESSURE: aggregate WebSocket outbound queue is full".to_string())
        );
        assert_eq!(
            connection.shared.local_budget.items.load(Ordering::Acquire),
            0
        );
    }

    #[test]
    fn write_budget_rejects_items_and_releases_on_drop() {
        let local = Arc::new(QueueBudget::new(1, 8));
        let aggregate = Arc::new(QueueBudget::new(2, 16));
        assert!(local.reserve(8));
        assert!(aggregate.reserve(8));
        let reservation = Reservation {
            bytes: 8,
            local: Arc::clone(&local),
            aggregate: Arc::clone(&aggregate),
        };
        assert!(!local.reserve(1));
        drop(reservation);
        assert!(local.reserve(8));
        local.release(8);
    }

    /// An inbound permit takes an item and its bytes; refused the bytes, it
    /// gives the item back.
    #[test]
    fn inbound_permits_are_bounded_by_items_and_bytes() {
        let budget = Arc::new(QueueBudget::new(2, 8));
        let permit = InboundPermit::reserve(&budget, 8).unwrap();
        assert!(InboundPermit::reserve(&budget, 1).is_none());
        assert_eq!(budget.items.load(Ordering::Acquire), 1);
        drop(permit);
        assert_eq!(budget.bytes.load(Ordering::Acquire), 0);
    }

    #[test]
    fn test_config_can_exercise_tiny_outbound_limits() {
        let config = ReactorConfig::server(1024).with_write_limits(1, 8);
        assert_eq!(config.max_write_queue_items, 1);
        assert_eq!(config.max_write_queue_bytes, 8);
    }

    #[test]
    fn close_drops_untouched_frames_but_finishes_a_partial_frame() {
        let mut untouched =
            VecDeque::from([Outbound::new(vec![1], None), Outbound::new(vec![2], None)]);
        prioritize_close(&mut untouched, false, Outbound::new(vec![8], None));
        assert_eq!(untouched.len(), 1);
        assert_eq!(untouched[0].bytes, [8]);

        let mut partial = Outbound::new(vec![1, 2], None);
        partial.offset = 1;
        let mut started = VecDeque::from([partial, Outbound::new(vec![3], None)]);
        prioritize_close(&mut started, true, Outbound::new(vec![8], None));
        assert_eq!(started.len(), 2);
        assert_eq!(started[0].offset, 1);
        assert_eq!(started[1].bytes, [8]);
    }

    /// A bad frame that arrived with the upgrade request fails the opened
    /// connection, as one arriving later does: a close with a protocol
    /// error, then the end of the connection, without waiting for the
    /// peer's close.
    #[test]
    fn a_bad_frame_sent_with_the_upgrade_closes_with_a_protocol_error() {
        let (mut client, _recorder, seen) = server(ReactorConfig::server(1024));
        let mut unmasked = Vec::new();
        write_frame(&mut unmasked, WsOpcode::Text, b"hi", true).unwrap();
        upgrade(&mut client, &unmasked, &seen);
        assert_eq!(close_code(&mut client), WsCloseCode::PROTOCOL_ERROR);
        assert_eq!(client.read(&mut [0u8; 1]).unwrap(), 0);
        assert_eq!(
            next(&seen),
            Seen::Terminated("client sent an unmasked WebSocket frame".to_string())
        );
    }

    /// A reactor admits connections, TLS handshakes, unread bytes and
    /// undelivered messages up to its limits.
    #[test]
    fn a_reactor_holds_to_its_limits() {
        let control = reactor_with(ReactorLimits {
            connections: 1,
            ..PROCESS_LIMITS
        });
        let (_client, _recorder, _seen) = server_on(&control, ReactorConfig::server(1024));
        let (_other, tcp) = socket_pair();
        let (recorder, _) = Recorder::new();
        let phase = Phase::ServerHandshake {
            handler: Arc::new(recorder),
            buffer: Vec::new(),
        };
        assert_eq!(
            register(
                &control,
                ReactorTransport::plain(tcp),
                phase,
                VecDeque::new(),
                ReactorConfig::server(1024)
            )
            .err(),
            Some("WebSocket reactor connection limit reached".to_string())
        );

        let _ = rustls::crypto::ring::default_provider().install_default();
        let (server_config, _) = crate::dist::node::ws_test_tls_configs();
        let control = reactor_with(ReactorLimits {
            tls_handshakes: 0,
            ..PROCESS_LIMITS
        });
        let (_other, tcp) = socket_pair();
        let (recorder, _) = Recorder::new();
        let phase = Phase::ServerHandshake {
            handler: Arc::new(recorder),
            buffer: Vec::new(),
        };
        assert_eq!(
            register(
                &control,
                ReactorTransport::tls(ServerConnection::new(server_config).unwrap(), tcp),
                phase,
                VecDeque::new(),
                ReactorConfig::server(1024)
            )
            .err(),
            Some("WebSocket TLS handshake limit reached".to_string())
        );

        let control = reactor_with(ReactorLimits {
            read_bytes: 16,
            ..PROCESS_LIMITS
        });
        let (mut client, _recorder, seen) = server_on(&control, ReactorConfig::server(1024));
        client.write_all(&UPGRADE[..20]).unwrap();
        assert_eq!(
            next(&seen),
            Seen::Failed("BACKPRESSURE: aggregate WebSocket read queue is full".to_string())
        );

        let control = reactor_with(ReactorLimits {
            inbound_items: 0,
            ..PROCESS_LIMITS
        });
        let (mut client, _recorder, seen) = server_on(&control, ReactorConfig::server(1024));
        upgrade(&mut client, &masked(WsOpcode::Text, b"hi", true), &seen);
        assert_eq!(close_code(&mut client), WsCloseCode::TRY_AGAIN_LATER);
    }

    /// A server answers a ping with its payload, delivers messages whole,
    /// and closes on each kind of bad message with its code.
    #[test]
    fn a_server_answers_pings_and_closes_on_bad_messages() {
        let (mut client, _recorder, seen) = open_server(ReactorConfig::server(8));
        client
            .write_all(
                &[
                    masked(WsOpcode::Ping, b"beat", true),
                    masked(WsOpcode::Binary, &[1, 2], false),
                    masked(WsOpcode::Continuation, &[3], true),
                ]
                .concat(),
            )
            .unwrap();
        let (pong, masked_pong) = read_frame_with_mask(&mut client).unwrap();
        assert_eq!(
            (pong.opcode, pong.payload, masked_pong),
            (WsOpcode::Pong, b"beat".to_vec(), false)
        );
        assert_eq!(next(&seen), Seen::Binary(vec![1, 2, 3]));

        for (bad, code) in [
            (
                masked(WsOpcode::Text, &[0xff], true),
                WsCloseCode::INVALID_DATA,
            ),
            (
                masked(WsOpcode::Binary, &[0; 9], true),
                WsCloseCode::MESSAGE_TOO_BIG,
            ),
            (
                [
                    masked(WsOpcode::Binary, &[0; 5], false),
                    masked(WsOpcode::Continuation, &[0; 5], true),
                ]
                .concat(),
                WsCloseCode::MESSAGE_TOO_BIG,
            ),
            (
                masked(WsOpcode::Continuation, &[0], true),
                WsCloseCode::PROTOCOL_ERROR,
            ),
        ] {
            let (mut client, _recorder, _seen) = open_server(ReactorConfig::server(8));
            client.write_all(&bad).unwrap();
            assert_eq!(close_code(&mut client), code);
        }
    }

    /// More frames than one turn processes wait for the next turn, whose
    /// bad frame still closes the connection.
    #[test]
    fn frames_past_a_turns_budget_are_processed_next_turn() {
        let (mut client, _recorder, seen) = open_server(ReactorConfig::server(8));
        let mut unmasked = Vec::new();
        write_frame(&mut unmasked, WsOpcode::Binary, &[], true).unwrap();
        let frames = [
            masked(WsOpcode::Binary, &[], true).repeat(FRAME_BUDGET_ITEMS),
            unmasked,
        ]
        .concat();
        client.write_all(&frames).unwrap();
        for _ in 0..FRAME_BUDGET_ITEMS {
            assert_eq!(next(&seen), Seen::Binary(Vec::new()));
        }
        assert_eq!(close_code(&mut client), WsCloseCode::PROTOCOL_ERROR);
    }

    /// A server connection whose handshake fails tells its handler: a
    /// request that is no upgrade, and one that never finishes.
    #[test]
    fn a_failed_server_handshake_tells_the_handler() {
        let (mut client, _recorder, seen) = server(ReactorConfig::server(8));
        client.write_all(b"BREW /pot\r\n\r\n").unwrap();
        assert!(
            matches!(next(&seen), Seen::Failed(reason) if reason.starts_with("malformed WebSocket request line"))
        );

        let (_client, _recorder, seen) =
            server(ReactorConfig::server(8).with_handshake_timeout(Duration::from_millis(50)));
        assert_eq!(
            next(&seen),
            Seen::Failed("TIMEOUT: WebSocket handshake".to_string())
        );
    }

    /// Set socket option `option` of `socket` to `value`.
    #[cfg(unix)]
    fn set_option<T>(socket: &TcpStream, option: libc::c_int, value: T) {
        use std::os::unix::io::AsRawFd;
        let rc = unsafe {
            libc::setsockopt(
                socket.as_raw_fd(),
                libc::SOL_SOCKET,
                option,
                (&value as *const T).cast(),
                std::mem::size_of::<T>() as libc::socklen_t,
            )
        };
        assert_eq!(rc, 0, "setsockopt: {}", std::io::Error::last_os_error());
    }

    /// Close `socket` with a reset rather than a FIN.
    #[cfg(unix)]
    fn reset(socket: TcpStream) {
        let linger = libc::linger {
            l_onoff: 1,
            l_linger: 0,
        };
        set_option(&socket, libc::SO_LINGER, linger);
    }

    /// A peer that resets the connection ends it with the read error.
    #[cfg(unix)]
    #[test]
    fn a_reset_ends_the_connection() {
        let (client, _recorder, seen) = open_server(ReactorConfig::server(8));
        reset(client);
        assert!(
            matches!(next(&seen), Seen::Terminated(reason) if reason.starts_with("read WebSocket transport: ")),
        );
    }

    /// A write to a peer that reset the connection ends it with the write
    /// error, where the reactor has not read the reset first: this one reads
    /// nothing, its read limit 0.
    #[cfg(unix)]
    #[test]
    fn a_write_to_a_reset_peer_ends_the_connection() {
        let control = reactor_with(ReactorLimits {
            read_bytes: 0,
            ..PROCESS_LIMITS
        });
        let (peer, tcp) = socket_pair();
        let (recorder, seen) = Recorder::new();
        let phase = Phase::ClientHandshake {
            sink: recorder,
            client_key: String::new(),
            buffer: Vec::new(),
        };
        let connection = register(
            &control,
            ReactorTransport::plain(tcp),
            phase,
            VecDeque::new(),
            ReactorConfig::client(8, Duration::from_secs(30)),
        )
        .unwrap();
        reset(peer);
        // The reset reaches the reactor's socket a moment after the close.
        let deadline = Instant::now() + TIMEOUT;
        let reason = loop {
            assert!(Instant::now() < deadline, "no write failed");
            let _ = connection.send(WsOpcode::Text, b"hi");
            if let Ok(Seen::Terminated(reason)) = seen.recv_timeout(Duration::from_millis(20)) {
                break reason;
            }
        };
        assert!(
            reason.starts_with("write WebSocket transport: "),
            "{reason}"
        );
    }

    /// A peer that stops reading holds the connection's writes back until
    /// it reads again; then everything sent arrives, in order.
    #[cfg(unix)]
    #[test]
    fn writes_wait_for_a_slow_reader() {
        let (mut client, tcp) = socket_pair();
        for socket in [&client, &tcp] {
            set_option(socket, libc::SO_SNDBUF, 4096 as libc::c_int);
            set_option(socket, libc::SO_RCVBUF, 4096 as libc::c_int);
        }
        let (recorder, seen) = Recorder::new();
        let phase = Phase::ServerHandshake {
            handler: Arc::new(Arc::clone(&recorder)),
            buffer: Vec::new(),
        };
        register(
            reactor().unwrap(),
            ReactorTransport::plain(tcp),
            phase,
            VecDeque::new(),
            ReactorConfig::server(64 * 1024),
        )
        .unwrap();
        upgrade(&mut client, &[], &seen);
        let connection = recorder.connection.lock().clone().unwrap();
        let message = vec![7u8; 60 * 1024];
        // Far more than the sockets buffer: the reactor's writes block.
        for _ in 0..16 {
            connection.send(WsOpcode::Binary, &message).unwrap();
        }
        for _ in 0..16 {
            assert_eq!(read_frame(&mut client).unwrap().payload, message);
        }
    }

    /// A peer that closes and then shuts its side, while the close reply
    /// still waits behind data it has not read, gets both once it reads:
    /// the end of its stream does not cut them off.
    #[cfg(unix)]
    #[test]
    fn a_peer_that_closes_and_leaves_still_gets_the_reply() {
        let (mut client, tcp) = socket_pair();
        for socket in [&client, &tcp] {
            set_option(socket, libc::SO_SNDBUF, 4096 as libc::c_int);
            set_option(socket, libc::SO_RCVBUF, 4096 as libc::c_int);
        }
        let (recorder, seen) = Recorder::new();
        let phase = Phase::ServerHandshake {
            handler: Arc::new(Arc::clone(&recorder)),
            buffer: Vec::new(),
        };
        register(
            reactor().unwrap(),
            ReactorTransport::plain(tcp),
            phase,
            VecDeque::new(),
            ReactorConfig::server(64 * 1024),
        )
        .unwrap();
        upgrade(&mut client, &[], &seen);
        let message = vec![7u8; 60 * 1024];
        let connection = recorder.connection.lock().clone().unwrap();
        connection.send(WsOpcode::Binary, &message).unwrap();
        // The message's header: it is on its way, and the close queues
        // behind it.
        let mut head = [0u8; 4];
        client.read_exact(&mut head).unwrap();
        assert_eq!(head, [0x82, 126, 0xf0, 0x00]);
        client
            .write_all(&masked(WsOpcode::Close, &1000u16.to_be_bytes(), true))
            .unwrap();
        assert_eq!(next(&seen), Seen::Close(1000, String::new()));
        client.shutdown(Shutdown::Write).unwrap();
        let mut payload = vec![0u8; message.len()];
        client.read_exact(&mut payload).unwrap();
        assert_eq!(payload, message);
        assert_eq!(close_code(&mut client), 1000);
        assert_eq!(next(&seen), Seen::Terminated("peer closed".to_string()));
    }

    /// A burst of small messages, far more than one turn processes and
    /// more bytes than the decoder buffers beyond one message, all arrive:
    /// the reactor reads no more than it has processed.
    #[test]
    fn a_burst_of_small_messages_all_arrive() {
        let (mut peer, _connection, seen) =
            open_client(ReactorConfig::client(16, Duration::from_secs(30)));
        let mut burst = Vec::new();
        for index in 0..20_000u32 {
            write_frame(&mut burst, WsOpcode::Binary, &index.to_be_bytes(), true).unwrap();
        }
        let writer = std::thread::spawn(move || {
            peer.write_all(&burst).unwrap();
            peer
        });
        for index in 0..20_000u32 {
            assert_eq!(next(&seen), Seen::Binary(index.to_be_bytes().to_vec()));
        }
        let _peer = writer.join().unwrap();
    }

    /// A client closes on a masked frame from its server.
    #[test]
    fn a_client_closes_on_a_masked_frame() {
        let (mut peer, _connection, _seen) =
            open_client(ReactorConfig::client(8, Duration::from_secs(30)));
        peer.write_all(&masked(WsOpcode::Text, b"hi", true))
            .unwrap();
        assert_eq!(close_code(&mut peer), WsCloseCode::PROTOCOL_ERROR);
    }

    /// A client pings on its interval; the answering pong keeps it open,
    /// and an unanswered ping closes it.
    #[test]
    fn a_client_heartbeat_is_kept_by_pongs() {
        let (mut peer, _connection, _seen) =
            open_client(ReactorConfig::client(8, Duration::from_millis(200)));
        let ping = read_frame(&mut peer).unwrap();
        assert_eq!(ping.opcode, WsOpcode::Ping);
        write_frame(&mut peer, WsOpcode::Pong, &ping.payload, true).unwrap();
        assert_eq!(read_frame(&mut peer).unwrap().opcode, WsOpcode::Ping);
        assert_eq!(close_code(&mut peer), WsCloseCode::GOING_AWAY);
    }

    /// A ping the write budget cannot hold ends the connection.
    #[test]
    fn a_ping_without_room_ends_the_connection() {
        let (_peer, _connection, seen) = open_client(
            ReactorConfig::client(8, Duration::from_millis(100)).with_write_limits(0, 0),
        );
        assert_eq!(
            next(&seen),
            Seen::Terminated("BACKPRESSURE: WebSocket outbound queue is full".to_string())
        );
    }

    /// Once a connection closes, data it receives is dropped: the peer's
    /// close is what it delivers next.
    #[test]
    fn a_closing_connection_ignores_data() {
        let (mut peer, connection, seen) =
            open_client(ReactorConfig::client(8, Duration::from_secs(30)));
        connection.graceful_close(1000, "bye").unwrap();
        assert_eq!(close_code(&mut peer), 1000);
        write_frame(&mut peer, WsOpcode::Text, b"late", true).unwrap();
        write_frame(
            &mut peer,
            WsOpcode::Close,
            &build_close_payload(1000, "ok"),
            true,
        )
        .unwrap();
        assert_eq!(next(&seen), Seen::Close(1000, "ok".to_string()));
        assert_eq!(next(&seen), Seen::Terminated("peer closed".to_string()));
    }

    /// A bad frame while a connection closes sends no second close: the
    /// connection, its close out, ends.
    #[test]
    fn a_closing_connection_does_not_close_twice() {
        let (mut peer, connection, seen) =
            open_client(ReactorConfig::client(8, Duration::from_secs(30)));
        connection.graceful_close(1000, "bye").unwrap();
        assert_eq!(close_code(&mut peer), 1000);
        peer.write_all(&[0x83, 0x00]).unwrap();
        assert_eq!(
            next(&seen),
            Seen::Terminated("unknown opcode: 0x3".to_string())
        );
        assert_eq!(peer.read(&mut [0u8; 1]).unwrap(), 0);
    }

    /// TLS: a client whose upgrade request is more than TLS buffers during
    /// the handshake sends the rest after it, and a peer that drops the
    /// connection without a close_notify ends it.
    #[test]
    fn tls_buffers_a_large_request_and_notices_a_dropped_peer() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let (server_config, client_config) = crate::dist::node::ws_test_tls_configs();
        let (tcp, reactor_side) = socket_pair();
        let request = [
            b"GET / HTTP/1.1\r\nX-Pad: ".as_slice(),
            &vec![b'a'; 2 * TLS_BUFFER_BYTES],
            b"\r\n\r\n",
        ]
        .concat();
        let session = ClientConnection::new(
            client_config,
            rustls_pki_types::ServerName::try_from("localhost").unwrap(),
        )
        .unwrap();
        let (_connection, seen) = client(
            ReactorTransport::tls(session, reactor_side),
            request.clone(),
            ReactorConfig::client(8, Duration::from_secs(30)),
        );
        let mut peer = rustls::StreamOwned::new(ServerConnection::new(server_config).unwrap(), tcp);
        assert_eq!(read_head(&mut peer).len(), request.len());
        write!(
            peer,
            "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
             Sec-WebSocket-Accept: {}\r\n\r\n",
            compute_accept_key("dGhlIHNhbXBsZSBub25jZQ==")
        )
        .unwrap();
        peer.flush().unwrap();
        assert_eq!(next(&seen), Seen::Opened(String::new()));
        drop(peer);
        assert_eq!(
            next(&seen),
            Seen::Terminated("WebSocket peer disconnected".to_string())
        );
    }
}
