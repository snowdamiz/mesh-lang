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

use crossbeam_channel::{Receiver, Sender, TryRecvError, TrySendError};
use mio::{Events, Interest, Poll, Token, Waker};
use parking_lot::Mutex;
use rustls::{ClientConnection, ServerConnection, StreamOwned};

use super::close::{
    build_close_payload, is_valid_close_code, parse_close_payload_strict, validate_text_payload,
    WsCloseCode,
};
use super::frame::{
    encode_frame, FrameDecoder, MessageAssembler, ReassembleResult, WsFrame, WsOpcode,
};
use super::handshake::{parse_upgrade_request_bytes, parse_upgrade_response_bytes};

const WAKE_TOKEN: Token = Token(0);
const COMMAND_QUEUE_ITEMS: usize = 8_192;
const MAX_CONNECTIONS: usize = 16_384;
const MAX_TLS_HANDSHAKES: usize = 2_048;
const MAX_AGGREGATE_WRITE_BYTES: usize = 128 * 1024 * 1024;
const MAX_AGGREGATE_WRITE_ITEMS: usize = 65_536;
const MAX_AGGREGATE_READ_BYTES: usize = 128 * 1024 * 1024;
const MAX_AGGREGATE_INBOUND_EVENT_BYTES: usize = 128 * 1024 * 1024;
const MAX_AGGREGATE_INBOUND_EVENT_ITEMS: usize = 65_536;
const READ_BUDGET_BYTES: usize = 64 * 1024;
const WRITE_BUDGET_BYTES: usize = 64 * 1024;
const FRAME_BUDGET_ITEMS: usize = 64;
const TLS_BUFFER_BYTES: usize = 64 * 1024;
const REACTOR_TICK: Duration = Duration::from_millis(25);
const CLOSE_DEADLINE: Duration = Duration::from_secs(2);

static NEXT_CONNECTION_ID: AtomicU64 = AtomicU64::new(1);
static TLS_HANDSHAKES: AtomicUsize = AtomicUsize::new(0);
static AGGREGATE_INBOUND_EVENT_BYTES: AtomicUsize = AtomicUsize::new(0);
static AGGREGATE_INBOUND_EVENT_ITEMS: AtomicUsize = AtomicUsize::new(0);

#[cfg(test)]
static REACTOR_THREADS_STARTED: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug)]
pub(crate) enum ReactorEvent {
    Text(Vec<u8>, InboundPermit),
    Binary(Vec<u8>, InboundPermit),
    Close(u16, String),
}

#[derive(Debug)]
pub(crate) struct InboundPermit {
    bytes: usize,
}

impl InboundPermit {
    pub(crate) fn reserve(bytes: usize) -> Option<Self> {
        if !reserve_counter(
            &AGGREGATE_INBOUND_EVENT_ITEMS,
            1,
            MAX_AGGREGATE_INBOUND_EVENT_ITEMS,
        ) {
            return None;
        }
        if !reserve_counter(
            &AGGREGATE_INBOUND_EVENT_BYTES,
            bytes,
            MAX_AGGREGATE_INBOUND_EVENT_BYTES,
        ) {
            AGGREGATE_INBOUND_EVENT_ITEMS.fetch_sub(1, Ordering::AcqRel);
            return None;
        }
        Some(Self { bytes })
    }
}

impl Drop for InboundPermit {
    fn drop(&mut self) {
        AGGREGATE_INBOUND_EVENT_BYTES.fetch_sub(self.bytes, Ordering::AcqRel);
        AGGREGATE_INBOUND_EVENT_ITEMS.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SinkError {
    Full,
    TooLarge,
    Closed,
}

pub(crate) trait ReactorEventSink: Send + Sync {
    fn opened(&self) {}
    fn event(&self, event: ReactorEvent) -> Result<(), SinkError>;
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

pub(crate) enum ReactorTransport {
    Plain(mio::net::TcpStream),
    ServerTls(StreamOwned<ServerConnection, mio::net::TcpStream>),
    ClientTls(StreamOwned<ClientConnection, mio::net::TcpStream>),
}

struct BudgetedIo<'a> {
    socket: &'a mut mio::net::TcpStream,
    remaining: usize,
}

impl Read for BudgetedIo<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.remaining == 0 {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        let allowance = buffer.len().min(self.remaining);
        let count = self.socket.read(&mut buffer[..allowance])?;
        self.remaining -= count;
        Ok(count)
    }
}

impl Write for BudgetedIo<'_> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if self.remaining == 0 {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        let allowance = buffer.len().min(self.remaining);
        let count = self.socket.write(&buffer[..allowance])?;
        self.remaining -= count;
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.socket.flush()
    }
}

impl ReactorTransport {
    pub(crate) fn plain(stream: TcpStream) -> io::Result<Self> {
        stream.set_nonblocking(true)?;
        Ok(Self::Plain(mio::net::TcpStream::from_std(stream)))
    }

    pub(crate) fn server_tls(
        mut stream: StreamOwned<ServerConnection, TcpStream>,
    ) -> io::Result<Self> {
        stream.conn.set_buffer_limit(Some(TLS_BUFFER_BYTES));
        let (connection, socket) = stream.into_parts();
        socket.set_nonblocking(true)?;
        Ok(Self::ServerTls(StreamOwned::new(
            connection,
            mio::net::TcpStream::from_std(socket),
        )))
    }

    pub(crate) fn client_tls(
        mut stream: StreamOwned<ClientConnection, TcpStream>,
    ) -> io::Result<Self> {
        stream.conn.set_buffer_limit(Some(TLS_BUFFER_BYTES));
        let (connection, socket) = stream.into_parts();
        socket.set_nonblocking(true)?;
        Ok(Self::ClientTls(StreamOwned::new(
            connection,
            mio::net::TcpStream::from_std(socket),
        )))
    }

    fn source(&mut self) -> &mut mio::net::TcpStream {
        match self {
            Self::Plain(stream) => stream,
            Self::ServerTls(stream) => &mut stream.sock,
            Self::ClientTls(stream) => &mut stream.sock,
        }
    }

    fn wants_write(&self) -> bool {
        match self {
            Self::Plain(_) => false,
            Self::ServerTls(stream) => stream.conn.wants_write(),
            Self::ClientTls(stream) => stream.conn.wants_write(),
        }
    }

    fn wants_read(&self) -> bool {
        match self {
            Self::Plain(_) => true,
            Self::ServerTls(stream) => stream.conn.wants_read(),
            Self::ClientTls(stream) => stream.conn.wants_read(),
        }
    }

    fn is_tls(&self) -> bool {
        !matches!(self, Self::Plain(_))
    }

    fn is_handshaking(&self) -> bool {
        match self {
            Self::Plain(_) => false,
            Self::ServerTls(stream) => stream.conn.is_handshaking(),
            Self::ClientTls(stream) => stream.conn.is_handshaking(),
        }
    }

    fn read_tls(&mut self, budget: usize) -> io::Result<Option<(usize, bool)>> {
        match self {
            Self::Plain(_) => Ok(None),
            Self::ServerTls(stream) => {
                let mut io = BudgetedIo {
                    socket: &mut stream.sock,
                    remaining: budget,
                };
                let count = stream.conn.read_tls(&mut io)?;
                if count == 0 {
                    return Ok(Some((0, true)));
                }
                let state = stream
                    .conn
                    .process_new_packets()
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
                Ok(Some((count, state.peer_has_closed())))
            }
            Self::ClientTls(stream) => {
                let mut io = BudgetedIo {
                    socket: &mut stream.sock,
                    remaining: budget,
                };
                let count = stream.conn.read_tls(&mut io)?;
                if count == 0 {
                    return Ok(Some((0, true)));
                }
                let state = stream
                    .conn
                    .process_new_packets()
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
                Ok(Some((count, state.peer_has_closed())))
            }
        }
    }

    fn write_tls(&mut self, budget: usize) -> io::Result<Option<usize>> {
        match self {
            Self::Plain(_) => Ok(None),
            Self::ServerTls(stream) => {
                let mut io = BudgetedIo {
                    socket: &mut stream.sock,
                    remaining: budget,
                };
                stream.conn.write_tls(&mut io).map(Some)
            }
            Self::ClientTls(stream) => {
                let mut io = BudgetedIo {
                    socket: &mut stream.sock,
                    remaining: budget,
                };
                stream.conn.write_tls(&mut io).map(Some)
            }
        }
    }

    fn send_close_notify(&mut self) {
        match self {
            Self::Plain(_) => {}
            Self::ServerTls(stream) => stream.conn.send_close_notify(),
            Self::ClientTls(stream) => stream.conn.send_close_notify(),
        }
    }

    fn shutdown(&self) {
        let result = match self {
            Self::Plain(stream) => stream.shutdown(Shutdown::Both),
            Self::ServerTls(stream) => stream.sock.shutdown(Shutdown::Both),
            Self::ClientTls(stream) => stream.sock.shutdown(Shutdown::Both),
        };
        let _ = result;
    }
}

impl Read for ReactorTransport {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.read(buffer),
            Self::ServerTls(stream) => stream.conn.reader().read(buffer),
            Self::ClientTls(stream) => stream.conn.reader().read(buffer),
        }
    }
}

impl Write for ReactorTransport {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.write(buffer),
            Self::ServerTls(stream) => stream.conn.writer().write(buffer),
            Self::ClientTls(stream) => stream.conn.writer().write(buffer),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Plain(stream) => stream.flush(),
            Self::ServerTls(stream) => stream.conn.writer().flush(),
            Self::ClientTls(stream) => stream.conn.writer().flush(),
        }
    }
}

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
    let mut current = counter.load(Ordering::Acquire);
    loop {
        let Some(next) = current.checked_add(amount) else {
            return false;
        };
        if next > maximum {
            return false;
        }
        match counter.compare_exchange_weak(current, next, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return true,
            Err(observed) => current = observed,
        }
    }
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
        let mask = match self.role {
            PeerRole::Server => None,
            PeerRole::Client => Some(rand::random()),
        };
        let reservation = self.reserve_frame(payload.len())?;
        let encoded = encode_frame(opcode, payload, true, mask)?;
        self.submit(Command::Write {
            id: self.id,
            outbound: Outbound::new(encoded, Some(reservation)),
        })
    }

    pub(crate) fn graceful_close(&self, code: u16, reason: &str) -> Result<(), String> {
        if !is_valid_close_code(code) {
            return Err("invalid WebSocket close code".to_string());
        }
        if !self.shared.accepting_writes.load(Ordering::Acquire) {
            return Ok(());
        }
        let payload = build_close_payload(code, reason);
        let mask = match self.role {
            PeerRole::Server => None,
            PeerRole::Client => Some(rand::random()),
        };
        let encoded = encode_frame(WsOpcode::Close, &payload, true, mask)?;
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

    fn reserve(&self, bytes: usize) -> Result<Reservation, String> {
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

    fn reserve_frame(&self, payload_bytes: usize) -> Result<Reservation, String> {
        let bytes = payload_bytes
            .checked_add(14)
            .ok_or_else(|| "WebSocket frame length overflow".to_string())?;
        self.reserve(bytes)
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

struct ReactorControl {
    commands: Sender<Command>,
    waker: Arc<Waker>,
    aggregate_write_budget: Arc<QueueBudget>,
    connection_count: Arc<AtomicUsize>,
}

struct ConnectionSlot(Arc<AtomicUsize>);

impl Drop for ConnectionSlot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

struct TlsHandshakeSlot;

impl TlsHandshakeSlot {
    fn reserve(stream: &ReactorTransport) -> Result<Option<Self>, String> {
        if !stream.is_tls() || !stream.is_handshaking() {
            return Ok(None);
        }
        if !reserve_counter(&TLS_HANDSHAKES, 1, MAX_TLS_HANDSHAKES) {
            return Err("WebSocket TLS handshake limit reached".to_string());
        }
        Ok(Some(Self))
    }
}

impl Drop for TlsHandshakeSlot {
    fn drop(&mut self) {
        TLS_HANDSHAKES.fetch_sub(1, Ordering::AcqRel);
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

enum Phase {
    ServerHandshake {
        handler: Arc<dyn ServerHandshakeHandler>,
        buffer: Vec<u8>,
        deadline: Instant,
    },
    ServerReply {
        handler: Arc<dyn ServerHandshakeHandler>,
        path: String,
        headers: Vec<(String, String)>,
        remainder: Vec<u8>,
        deadline: Instant,
    },
    ClientHandshake {
        sink: Arc<dyn ReactorEventSink>,
        client_key: String,
        buffer: Vec<u8>,
        deadline: Instant,
    },
    Open {
        sink: Arc<dyn ReactorEventSink>,
    },
    Transitioning,
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
    config: ReactorConfig,
    decoder: FrameDecoder,
    assembler: MessageAssembler,
    outbound: VecDeque<Outbound>,
    heartbeat: Heartbeat,
    read_ready: bool,
    write_ready: bool,
    frames_ready: bool,
    transport_eof: bool,
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
        self.stream.wants_write()
            || (!self.outbound.is_empty()
                && (!self.stream.is_tls() || !self.stream.is_handshaking()))
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
            Phase::ServerReply { remainder, .. } => remainder.len(),
            Phase::Open { .. } | Phase::Transitioning => 0,
        };
        handshake + self.decoder.buffered_len() + self.assembler.buffered_len()
    }

    fn readable(&mut self) -> bool {
        let mut bytes_read = 0usize;
        let mut buffer = [0u8; 16 * 1024];
        while bytes_read < READ_BUDGET_BYTES && !self.dead {
            let allowance = (READ_BUDGET_BYTES - bytes_read).min(buffer.len());
            match self.stream.read(&mut buffer[..allowance]) {
                Ok(0) => {
                    self.handle_transport_eof();
                    return false;
                }
                Ok(count) => {
                    bytes_read += count;
                    if let Err(reason) = self.consume_bytes(&buffer[..count]) {
                        self.protocol_failure(&reason);
                        break;
                    }
                    if self.close_state == CloseState::Replying {
                        return false;
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    if !self.stream.is_tls() || self.transport_eof || !self.stream.wants_read() {
                        if self.transport_eof {
                            self.handle_transport_eof();
                        }
                        return false;
                    }
                    match self.stream.read_tls(READ_BUDGET_BYTES - bytes_read) {
                        Ok(Some((0, _))) => {
                            self.handle_transport_eof();
                            return false;
                        }
                        Ok(Some((count, peer_closed))) => {
                            bytes_read += count;
                            self.transport_eof |= peer_closed;
                        }
                        Ok(None) => return false,
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => return false,
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                        Err(error) => {
                            self.fail(&format!("read WebSocket TLS transport: {error}"));
                            return false;
                        }
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => {
                    self.fail(&format!("read WebSocket transport: {error}"));
                    return false;
                }
            }
        }
        !self.dead
    }

    fn handle_transport_eof(&mut self) {
        if self.close_state == CloseState::Replying {
            self.finish_close_if_flushed();
        } else {
            self.fail("WebSocket peer disconnected");
        }
    }

    fn consume_bytes(&mut self, bytes: &[u8]) -> Result<(), String> {
        match &mut self.phase {
            Phase::ServerHandshake { buffer, .. } | Phase::ClientHandshake { buffer, .. } => {
                buffer.extend_from_slice(bytes)
            }
            Phase::ServerReply { remainder, .. } => remainder.extend_from_slice(bytes),
            Phase::Open { .. } => self.decoder.extend(bytes)?,
            Phase::Transitioning => return Err("invalid WebSocket reactor transition".to_string()),
        }
        self.advance_handshake()?;
        self.process_frames()
    }

    fn advance_handshake(&mut self) -> Result<(), String> {
        let server_parsed = match &self.phase {
            Phase::ServerHandshake { buffer, .. } => parse_upgrade_request_bytes(buffer)?,
            _ => None,
        };
        if let Some(parsed) = server_parsed {
            let Phase::ServerHandshake {
                handler,
                mut buffer,
                deadline,
            } = std::mem::replace(&mut self.phase, Phase::Transitioning)
            else {
                return Err("invalid server handshake transition".to_string());
            };
            let remainder = buffer.split_off(parsed.consumed);
            self.outbound
                .push_back(Outbound::new(parsed.response, None));
            self.phase = Phase::ServerReply {
                handler,
                path: parsed.path,
                headers: parsed.headers,
                remainder,
                deadline,
            };
            return Ok(());
        }

        let client_consumed = match &self.phase {
            Phase::ClientHandshake {
                buffer, client_key, ..
            } => parse_upgrade_response_bytes(buffer, client_key)?,
            _ => None,
        };
        if let Some(consumed) = client_consumed {
            let Phase::ClientHandshake {
                sink, mut buffer, ..
            } = std::mem::replace(&mut self.phase, Phase::Transitioning)
            else {
                return Err("invalid client handshake transition".to_string());
            };
            let remainder = buffer.split_off(consumed);
            sink.opened();
            self.phase = Phase::Open { sink };
            if !remainder.is_empty() {
                self.decoder.extend(&remainder)?;
            }
        }
        Ok(())
    }

    fn finish_server_reply(&mut self) -> Result<(), String> {
        if !self.outbound.is_empty() || self.stream.wants_write() {
            return Ok(());
        }
        let Phase::ServerReply {
            handler,
            path,
            headers,
            remainder,
            ..
        } = std::mem::replace(&mut self.phase, Phase::Transitioning)
        else {
            return Ok(());
        };
        let sink = handler.opened(self.connection.clone(), path, headers);
        self.phase = Phase::Open { sink };
        if !remainder.is_empty() {
            self.decoder.extend(&remainder)?;
            self.process_frames()?;
        }
        Ok(())
    }

    fn process_frames(&mut self) -> Result<(), String> {
        self.frames_ready = false;
        if !matches!(self.phase, Phase::Open { .. }) {
            return Ok(());
        }
        for index in 0..FRAME_BUDGET_ITEMS {
            let Some((frame, masked)) = self.decoder.next_frame()? else {
                return Ok(());
            };
            let valid_mask = match self.config.role {
                PeerRole::Server => masked,
                PeerRole::Client => !masked,
            };
            if !valid_mask {
                return Err(match self.config.role {
                    PeerRole::Server => "client sent an unmasked WebSocket frame".to_string(),
                    PeerRole::Client => "server sent a masked WebSocket frame".to_string(),
                });
            }
            self.process_frame(frame)?;
            if self.close_state == CloseState::Replying || self.dead {
                return Ok(());
            }
            if index + 1 == FRAME_BUDGET_ITEMS {
                self.frames_ready = true;
            }
        }
        Ok(())
    }

    fn process_frame(&mut self, frame: WsFrame) -> Result<(), String> {
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
                    let close = self.close_outbound(&frame.payload)?;
                    self.prioritize_close(close);
                }
                self.close_state = CloseState::Replying;
                self.finish_close_if_flushed();
                let _ = self.deliver(ReactorEvent::Close(code, reason));
                Ok(())
            }
            WsOpcode::Text | WsOpcode::Binary | WsOpcode::Continuation => {
                if self.close_state != CloseState::Open {
                    return Ok(());
                }
                match self.assembler.push(frame) {
                    ReassembleResult::Complete(message) => {
                        let Some(permit) = InboundPermit::reserve(message.payload.len()) else {
                            return self.start_close(
                                WsCloseCode::TRY_AGAIN_LATER,
                                "aggregate inbound queue full",
                            );
                        };
                        let event = match message.opcode {
                            WsOpcode::Text => {
                                if validate_text_payload(&message.payload).is_err() {
                                    return Err("invalid UTF-8 in text message".to_string());
                                }
                                ReactorEvent::Text(message.payload, permit)
                            }
                            WsOpcode::Binary => ReactorEvent::Binary(message.payload, permit),
                            _ => return Err("invalid reassembled WebSocket opcode".to_string()),
                        };
                        match self.deliver(event) {
                            Ok(()) => Ok(()),
                            Err(SinkError::Full) => {
                                self.start_close(WsCloseCode::TRY_AGAIN_LATER, "inbound queue full")
                            }
                            Err(SinkError::TooLarge) => {
                                self.start_close(WsCloseCode::MESSAGE_TOO_BIG, "message too big")
                            }
                            Err(SinkError::Closed) => {
                                self.fail("WebSocket event sink is closed");
                                Ok(())
                            }
                        }
                    }
                    ReassembleResult::Accumulating => Ok(()),
                    ReassembleResult::TooLarge => {
                        self.start_close(WsCloseCode::MESSAGE_TOO_BIG, "message too big")
                    }
                    ReassembleResult::ProtocolError(reason) => Err(reason.to_string()),
                }
            }
        }
    }

    fn deliver(&self, event: ReactorEvent) -> Result<(), SinkError> {
        match &self.phase {
            Phase::Open { sink } => sink.event(event),
            _ => Err(SinkError::Closed),
        }
    }

    fn queue_internal(&mut self, opcode: WsOpcode, payload: &[u8]) -> Result<(), String> {
        let outbound = self.internal_outbound(opcode, payload)?;
        self.outbound.push_back(outbound);
        Ok(())
    }

    fn internal_outbound(&self, opcode: WsOpcode, payload: &[u8]) -> Result<Outbound, String> {
        let mask = match self.config.role {
            PeerRole::Server => None,
            PeerRole::Client => Some(rand::random()),
        };
        let reservation = self.connection.reserve_frame(payload.len())?;
        let encoded = encode_frame(opcode, payload, true, mask)?;
        Ok(Outbound::new(encoded, Some(reservation)))
    }

    fn prioritize_close(&mut self, close: Outbound) {
        let keep_front = self.stream.wants_write()
            || self
                .outbound
                .front()
                .is_some_and(|outbound| outbound.offset > 0);
        prioritize_close(&mut self.outbound, keep_front, close);
    }

    fn close_outbound(&self, payload: &[u8]) -> Result<Outbound, String> {
        let mask = match self.config.role {
            PeerRole::Server => None,
            PeerRole::Client => Some(rand::random()),
        };
        let encoded = encode_frame(WsOpcode::Close, payload, true, mask)?;
        Ok(Outbound::new(encoded, None))
    }

    fn start_close(&mut self, code: u16, reason: &str) -> Result<(), String> {
        if self.close_state != CloseState::Open {
            return Ok(());
        }
        self.connection
            .shared
            .accepting_writes
            .store(false, Ordering::Release);
        let payload = build_close_payload(code, reason);
        let close = self.close_outbound(&payload)?;
        self.prioritize_close(close);
        self.close_state = CloseState::AwaitingPeer;
        self.close_deadline = Some(Instant::now() + CLOSE_DEADLINE);
        self.termination_reason = reason.to_string();
        Ok(())
    }

    fn finish_close_if_flushed(&mut self) {
        if self.close_state == CloseState::Replying
            && self.outbound.is_empty()
            && !self.stream.wants_write()
        {
            if self.stream.is_tls() && !self.tls_close_notify_sent {
                self.stream.send_close_notify();
                self.tls_close_notify_sent = true;
                self.write_ready = true;
            } else {
                self.dead = true;
            }
        }
    }

    fn protocol_failure(&mut self, reason: &str) {
        if !matches!(self.phase, Phase::Open { .. }) {
            self.fail(reason);
            return;
        }
        let code = if reason.contains("UTF-8") {
            WsCloseCode::INVALID_DATA
        } else if reason.contains("maximum") || reason.contains("buffer is full") {
            WsCloseCode::MESSAGE_TOO_BIG
        } else {
            WsCloseCode::PROTOCOL_ERROR
        };
        if self.start_close(code, reason).is_err() {
            self.fail(reason);
        }
    }

    fn writable(&mut self) -> bool {
        let mut written = 0usize;
        let mut network_written = false;
        let mut blocked = false;
        while written < WRITE_BUDGET_BYTES && !self.dead {
            if self.stream.wants_write() {
                match self.stream.write_tls(WRITE_BUDGET_BYTES - written) {
                    Ok(Some(0)) => {
                        self.fail("write WebSocket TLS transport returned zero bytes");
                        break;
                    }
                    Ok(Some(count)) => {
                        written += count;
                        network_written = true;
                        continue;
                    }
                    Ok(None) => {}
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        blocked = true;
                        break;
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) => {
                        self.fail(&format!("flush WebSocket TLS transport: {error}"));
                        break;
                    }
                }
            }

            let Some(outbound) = self.outbound.front_mut() else {
                break;
            };
            if outbound.offset == outbound.bytes.len() {
                if self.stream.is_tls() && self.stream.is_handshaking() {
                    blocked = true;
                    break;
                }
                self.outbound.pop_front();
                continue;
            }
            let remaining_budget = WRITE_BUDGET_BYTES - written;
            let end = (outbound.offset + remaining_budget).min(outbound.bytes.len());
            match self.stream.write(&outbound.bytes[outbound.offset..end]) {
                Ok(0) if self.stream.is_tls() => {
                    blocked = true;
                    break;
                }
                Ok(0) => {
                    self.fail("write WebSocket transport returned zero bytes");
                    break;
                }
                Ok(count) => {
                    outbound.offset += count;
                    written += count;
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    blocked = true;
                    break;
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => {
                    self.fail(&format!("write WebSocket transport: {error}"));
                    break;
                }
            }
        }

        if (!self.stream.is_tls() || !self.stream.is_handshaking())
            && !self.stream.wants_write()
            && self
                .outbound
                .front()
                .is_some_and(|outbound| outbound.offset == outbound.bytes.len())
        {
            self.outbound.pop_front();
        }

        if matches!(self.phase, Phase::ServerReply { .. }) {
            if let Err(reason) = self.finish_server_reply() {
                self.fail(&reason);
            }
        }
        if network_written && self.stream.wants_read() {
            self.read_ready = true;
        }
        self.finish_close_if_flushed();
        !self.dead && !blocked && self.needs_write_interest()
    }

    fn preflight(&mut self, now: Instant) {
        if self.dead {
            return;
        }
        if self.connection.shared.cancelled.load(Ordering::Acquire) {
            let reason = self
                .connection
                .shared
                .cancel_reason
                .lock()
                .clone()
                .unwrap_or_else(|| "WebSocket connection cancelled".to_string());
            self.fail(&reason);
            return;
        }
        let handshake_deadline = match &self.phase {
            Phase::ServerHandshake { deadline, .. }
            | Phase::ServerReply { deadline, .. }
            | Phase::ClientHandshake { deadline, .. } => Some(*deadline),
            Phase::Open { .. } | Phase::Transitioning => None,
        };
        if handshake_deadline.is_some_and(|deadline| now >= deadline) {
            self.fail("TIMEOUT: WebSocket handshake");
            return;
        }
        if self.close_deadline.is_some_and(|deadline| now >= deadline) {
            self.dead = true;
        }
    }

    fn tick(&mut self, now: Instant) {
        if self.dead {
            return;
        }
        if !matches!(self.phase, Phase::Open { .. }) || self.close_state != CloseState::Open {
            return;
        }
        if self
            .heartbeat
            .pending
            .is_some_and(|(_, sent)| now.duration_since(sent) >= self.config.pong_timeout)
        {
            if self
                .start_close(WsCloseCode::GOING_AWAY, "HEARTBEAT_TIMEOUT")
                .is_err()
            {
                self.fail("HEARTBEAT_TIMEOUT");
            }
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
            Phase::Transitioning => {}
        }
    }
}

fn reactor() -> Result<&'static Arc<ReactorControl>, String> {
    static REACTOR: OnceLock<Result<Arc<ReactorControl>, String>> = OnceLock::new();
    REACTOR
        .get_or_init(start_reactor)
        .as_ref()
        .map_err(Clone::clone)
}

fn start_reactor() -> Result<Arc<ReactorControl>, String> {
    let poll = Poll::new().map_err(|error| format!("create WebSocket poller: {error}"))?;
    let waker = Arc::new(
        Waker::new(poll.registry(), WAKE_TOKEN)
            .map_err(|error| format!("create WebSocket reactor waker: {error}"))?,
    );
    let (commands, receiver) = crossbeam_channel::bounded(COMMAND_QUEUE_ITEMS);
    let control = Arc::new(ReactorControl {
        commands,
        waker,
        aggregate_write_budget: Arc::new(QueueBudget::new(
            MAX_AGGREGATE_WRITE_ITEMS,
            MAX_AGGREGATE_WRITE_BYTES,
        )),
        connection_count: Arc::new(AtomicUsize::new(0)),
    });
    std::thread::Builder::new()
        .name("mesh-ws-reactor".to_string())
        .spawn(move || reactor_loop(poll, receiver))
        .map_err(|error| format!("start WebSocket reactor: {error}"))?;
    #[cfg(test)]
    REACTOR_THREADS_STARTED.fetch_add(1, Ordering::AcqRel);
    Ok(control)
}

fn new_connection(config: ReactorConfig) -> Result<(ReactorConnection, ConnectionSlot), String> {
    let control = Arc::clone(reactor()?);
    if !reserve_counter(&control.connection_count, 1, MAX_CONNECTIONS) {
        return Err("WebSocket reactor connection limit reached".to_string());
    }
    let id = NEXT_CONNECTION_ID.fetch_add(1, Ordering::Relaxed);
    let token_value = usize::try_from(id).map_err(|_| "WebSocket connection ID overflow")?;
    if token_value == WAKE_TOKEN.0 {
        control.connection_count.fetch_sub(1, Ordering::AcqRel);
        return Err("WebSocket connection ID exhausted".to_string());
    }
    let shared = Arc::new(ConnectionShared {
        accepting_writes: AtomicBool::new(true),
        cancelled: AtomicBool::new(false),
        cancel_reason: Mutex::new(None),
        local_budget: Arc::new(QueueBudget::new(
            config.max_write_queue_items,
            config.max_write_queue_bytes,
        )),
    });
    let connection = ReactorConnection {
        id,
        role: config.role,
        max_message_bytes: config.max_message_bytes,
        shared,
        control: Arc::clone(&control),
    };
    Ok((connection, ConnectionSlot(control.connection_count.clone())))
}

pub(crate) fn register_server(
    stream: ReactorTransport,
    handler: Arc<dyn ServerHandshakeHandler>,
    config: ReactorConfig,
) -> Result<ReactorConnection, String> {
    let tls_handshake_slot = TlsHandshakeSlot::reserve(&stream)?;
    let (connection, slot) = new_connection(config.clone())?;
    let now = Instant::now();
    let entry = Entry {
        id: connection.id,
        token: Token(connection.id as usize),
        connection: connection.clone(),
        _slot: slot,
        tls_handshake_slot,
        stream,
        phase: Phase::ServerHandshake {
            handler,
            buffer: Vec::new(),
            deadline: now + config.handshake_timeout,
        },
        decoder: FrameDecoder::new(config.max_message_bytes),
        assembler: MessageAssembler::new(config.max_message_bytes),
        config,
        outbound: VecDeque::new(),
        heartbeat: Heartbeat {
            last_ping: now,
            pending: None,
        },
        read_ready: false,
        write_ready: false,
        frames_ready: false,
        transport_eof: false,
        tls_close_notify_sent: false,
        close_state: CloseState::Open,
        close_deadline: None,
        termination_reason: "WebSocket connection closed".to_string(),
        dead: false,
    };
    connection.submit(Command::Register {
        entry: Box::new(entry),
    })?;
    Ok(connection)
}

pub(crate) fn register_client(
    stream: ReactorTransport,
    request: Vec<u8>,
    client_key: String,
    sink: Arc<dyn ReactorEventSink>,
    config: ReactorConfig,
) -> Result<ReactorConnection, String> {
    let tls_handshake_slot = TlsHandshakeSlot::reserve(&stream)?;
    let (connection, slot) = new_connection(config.clone())?;
    let now = Instant::now();
    let entry = Entry {
        id: connection.id,
        token: Token(connection.id as usize),
        connection: connection.clone(),
        _slot: slot,
        tls_handshake_slot,
        stream,
        phase: Phase::ClientHandshake {
            sink,
            client_key,
            buffer: Vec::new(),
            deadline: now + config.handshake_timeout,
        },
        decoder: FrameDecoder::new(config.max_message_bytes),
        assembler: MessageAssembler::new(config.max_message_bytes),
        config,
        outbound: VecDeque::from([Outbound::new(request, None)]),
        heartbeat: Heartbeat {
            last_ping: now,
            pending: None,
        },
        read_ready: false,
        write_ready: false,
        frames_ready: false,
        transport_eof: false,
        tls_close_notify_sent: false,
        close_state: CloseState::Open,
        close_deadline: None,
        termination_reason: "WebSocket connection closed".to_string(),
        dead: false,
    };
    connection.submit(Command::Register {
        entry: Box::new(entry),
    })?;
    Ok(connection)
}

fn reactor_loop(mut poll: Poll, receiver: Receiver<Command>) {
    let mut events = Events::with_capacity(1024);
    let mut entries = HashMap::<u64, Box<Entry>>::new();
    loop {
        let immediate = entries
            .values()
            .any(|entry| entry.read_ready || entry.write_ready || entry.frames_ready);
        let timeout = if immediate {
            Duration::ZERO
        } else {
            REACTOR_TICK
        };
        if let Err(error) = poll.poll(&mut events, Some(timeout)) {
            if error.kind() != io::ErrorKind::Interrupted {
                eprintln!("[mesh-rt] WebSocket reactor poll failed: {error}");
            }
        }

        for event in &events {
            if event.token() == WAKE_TOKEN {
                drain_commands(&poll, &receiver, &mut entries);
                continue;
            }
            let id = event.token().0 as u64;
            let Some(entry) = entries.get_mut(&id) else {
                continue;
            };
            entry.read_ready |= event.is_readable() || event.is_read_closed() || event.is_error();
            entry.write_ready |= event.is_writable() || event.is_write_closed() || event.is_error();
        }

        drain_commands(&poll, &receiver, &mut entries);
        let now = Instant::now();
        let mut aggregate_read_bytes: usize =
            entries.values().map(|entry| entry.buffered_bytes()).sum();
        for entry in entries.values_mut() {
            let buffered_before = entry.buffered_bytes();
            let had_pending_write = entry.needs_write_interest();
            entry.preflight(now);
            if entry.read_ready && !entry.dead && aggregate_read_bytes < MAX_AGGREGATE_READ_BYTES {
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

        while aggregate_read_bytes >= MAX_AGGREGATE_READ_BYTES {
            let Some(entry) = entries
                .values_mut()
                .filter(|entry| !entry.dead)
                .max_by_key(|entry| entry.buffered_bytes())
            else {
                break;
            };
            let bytes = entry.buffered_bytes();
            if bytes == 0 {
                break;
            }
            entry.fail("BACKPRESSURE: aggregate WebSocket read queue is full");
            aggregate_read_bytes = aggregate_read_bytes.saturating_sub(bytes);
        }

        let dead = entries
            .iter()
            .filter_map(|(id, entry)| entry.dead.then_some(*id))
            .collect::<Vec<_>>();
        for id in dead {
            if let Some(mut entry) = entries.remove(&id) {
                let _ = poll.registry().deregister(entry.stream.source());
                entry.stream.shutdown();
                entry.notify_terminated();
            }
        }
    }
}

fn drain_commands(
    poll: &Poll,
    receiver: &Receiver<Command>,
    entries: &mut HashMap<u64, Box<Entry>>,
) {
    loop {
        match receiver.try_recv() {
            Ok(Command::Register { mut entry }) => {
                let interest = entry.interest();
                match poll
                    .registry()
                    .register(entry.stream.source(), entry.token, interest)
                {
                    Ok(()) => {
                        entries.insert(entry.id, entry);
                    }
                    Err(error) => {
                        entry.fail(&format!("register WebSocket transport: {error}"));
                        entry.stream.shutdown();
                        entry.notify_terminated();
                    }
                }
            }
            Ok(Command::Write { id, outbound }) => {
                if let Some(entry) = entries.get_mut(&id) {
                    if entry.close_state == CloseState::Open && !entry.dead {
                        entry.outbound.push_back(outbound);
                        reregister_writable(poll, entry);
                    }
                }
            }
            Ok(Command::Close {
                id,
                outbound,
                reason,
            }) => {
                if let Some(entry) = entries.get_mut(&id) {
                    if entry.close_state == CloseState::Open && !entry.dead {
                        entry.prioritize_close(outbound);
                        entry.close_state = CloseState::AwaitingPeer;
                        entry.close_deadline = Some(Instant::now() + CLOSE_DEADLINE);
                        entry.termination_reason = reason;
                        reregister_writable(poll, entry);
                    }
                }
            }
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => break,
        }
    }
}

fn reregister_writable(poll: &Poll, entry: &mut Entry) {
    let interest = entry.interest();
    if let Err(error) = poll
        .registry()
        .reregister(entry.stream.source(), entry.token, interest)
    {
        entry.fail(&format!("reregister WebSocket writer: {error}"));
    }
}

#[cfg(test)]
pub(crate) fn reactor_threads_started() -> usize {
    REACTOR_THREADS_STARTED.load(Ordering::Acquire)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graceful_close_retains_only_the_protocol_encoded_reason() {
        let poll = Poll::new().unwrap();
        let waker = Arc::new(Waker::new(poll.registry(), WAKE_TOKEN).unwrap());
        let (commands, receiver) = crossbeam_channel::bounded(1);
        let control = Arc::new(ReactorControl {
            commands,
            waker,
            aggregate_write_budget: Arc::new(QueueBudget::new(1, 1024)),
            connection_count: Arc::new(AtomicUsize::new(0)),
        });
        let connection = ReactorConnection {
            id: 1,
            role: PeerRole::Server,
            max_message_bytes: 1024,
            shared: Arc::new(ConnectionShared {
                accepting_writes: AtomicBool::new(true),
                cancelled: AtomicBool::new(false),
                cancel_reason: Mutex::new(None),
                local_budget: Arc::new(QueueBudget::new(1, 1024)),
            }),
            control,
        };
        let supplied = "reason".repeat(1024);

        connection.graceful_close(1000, &supplied).unwrap();

        let Command::Close { reason, .. } = receiver.recv().unwrap() else {
            panic!("graceful close queued the wrong command");
        };
        let payload = build_close_payload(1000, &supplied);
        assert_eq!(reason.as_bytes(), &payload[2..]);
        assert!(reason.len() <= 123);
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
}
