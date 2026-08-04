//! Bounded, scheduler-aware WebSocket client.

use std::collections::{HashMap, VecDeque};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use parking_lot::Mutex;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
use rustls_pki_types::ServerName;
use url::Url;

use crate::actor::{cooperative_channel, cooperative_recv_timeout, CooperativeSender};
use crate::bytes::{mesh_bytes_new, MeshBytes};
use crate::gc::mesh_gc_alloc_actor;
use crate::io::{alloc_result, MeshResult};
use crate::string::{mesh_string_new, MeshString};

use super::close::is_valid_close_code;
use super::frame::WsOpcode;
use super::handshake::MAX_HANDSHAKE_BYTES;
use super::reactor::{
    register_client, InboundPermit, ReactorConfig, ReactorConnection, ReactorEvent,
    ReactorEventSink, ReactorTransport, SinkError,
};

const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;
const MAX_QUEUE_CAPACITY: usize = 65_536;
const MAX_INBOUND_QUEUE_BYTES: usize = 32 * 1024 * 1024;
const MAX_OPEN_HANDLES: usize = 4_096;
const MAX_CONNECT_WORKERS: usize = 64;

#[derive(Clone)]
struct WsClientOptions {
    connect_timeout: Duration,
    heartbeat_timeout: Duration,
    max_message_bytes: usize,
    queue_capacity: usize,
}

impl Default for WsClientOptions {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(10),
            heartbeat_timeout: Duration::from_secs(30),
            max_message_bytes: 1024 * 1024,
            queue_capacity: 256,
        }
    }
}

enum ClientEvent {
    Text(Vec<u8>, InboundPermit),
    Binary(Vec<u8>, InboundPermit),
    Close(u16, String),
}

impl ClientEvent {
    fn byte_len(&self) -> usize {
        match self {
            Self::Text(data, _) | Self::Binary(data, _) => data.len(),
            Self::Close(_, reason) => reason.len(),
        }
    }
}

struct Waiter {
    id: u64,
    sender: CooperativeSender<ClientEvent>,
}

struct InboundState {
    queue: VecDeque<ClientEvent>,
    queued_bytes: usize,
    capacity: usize,
    max_bytes: usize,
    waiter: Option<Waiter>,
    next_waiter_id: u64,
    terminal_error: Option<String>,
}

struct WsConnection {
    io: ReactorConnection,
    inbound: Arc<Mutex<InboundState>>,
}

struct ClientSink {
    inbound: Arc<Mutex<InboundState>>,
    ready: Mutex<Option<std::sync::mpsc::SyncSender<Result<(), String>>>>,
}

impl ReactorEventSink for ClientSink {
    fn opened(&self) {
        if let Some(ready) = self.ready.lock().take() {
            let _ = ready.send(Ok(()));
        }
    }

    fn event(&self, event: ReactorEvent) -> Result<(), SinkError> {
        let event = match event {
            ReactorEvent::Text(data, permit) => ClientEvent::Text(data, permit),
            ReactorEvent::Binary(data, permit) => ClientEvent::Binary(data, permit),
            ReactorEvent::Close(code, reason) => ClientEvent::Close(code, reason),
        };
        deliver(&self.inbound, event)
            .then_some(())
            .ok_or(SinkError::Full)
    }

    fn terminated(&self, reason: &str) {
        if let Some(ready) = self.ready.lock().take() {
            let _ = ready.send(Err(reason.to_string()));
        }
        terminate(&self.inbound, reason);
    }
}

#[repr(C)]
struct MeshWsMessage {
    kind: *mut MeshString,
    data: *mut MeshBytes,
    close_code: i64,
    close_reason: *mut MeshString,
}

static OPTIONS: OnceLock<Mutex<HashMap<u64, WsClientOptions>>> = OnceLock::new();
static CONNECTIONS: OnceLock<Mutex<HashMap<u64, Arc<WsConnection>>>> = OnceLock::new();
static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1);
static ACTIVE_CONNECT_WORKERS: AtomicUsize = AtomicUsize::new(0);

struct ConnectWorkerPermit;

impl ConnectWorkerPermit {
    fn reserve() -> Option<Self> {
        ACTIVE_CONNECT_WORKERS
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                (active < MAX_CONNECT_WORKERS).then_some(active + 1)
            })
            .ok()
            .map(|_| Self)
    }
}

impl Drop for ConnectWorkerPermit {
    fn drop(&mut self) {
        ACTIVE_CONNECT_WORKERS.fetch_sub(1, Ordering::AcqRel);
    }
}

fn options() -> &'static Mutex<HashMap<u64, WsClientOptions>> {
    OPTIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn connections() -> &'static Mutex<HashMap<u64, Arc<WsConnection>>> {
    CONNECTIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn next_handle() -> u64 {
    NEXT_HANDLE.fetch_add(1, Ordering::Relaxed)
}

fn error(message: impl AsRef<str>) -> *mut MeshResult {
    let message = message.as_ref();
    alloc_result(
        1,
        mesh_string_new(message.as_ptr(), message.len() as u64).cast(),
    )
}

fn ok_unit() -> *mut MeshResult {
    alloc_result(0, std::ptr::null_mut())
}

fn ok_int(value: i64) -> *mut MeshResult {
    unsafe {
        let boxed = mesh_gc_alloc_actor(std::mem::size_of::<i64>() as u64, 8) as *mut i64;
        boxed.write(value);
        alloc_result(0, boxed.cast())
    }
}

fn mesh_message(event: ClientEvent) -> *mut MeshResult {
    let (kind, data, close_code, close_reason) = match event {
        ClientEvent::Text(data, _permit) => ("text", data, 0, String::new()),
        ClientEvent::Binary(data, _permit) => ("binary", data, 0, String::new()),
        ClientEvent::Close(code, reason) => ("close", Vec::new(), code as i64, reason),
    };
    unsafe {
        let message = mesh_gc_alloc_actor(
            std::mem::size_of::<MeshWsMessage>() as u64,
            std::mem::align_of::<MeshWsMessage>() as u64,
        ) as *mut MeshWsMessage;
        (*message).kind = mesh_string_new(kind.as_ptr(), kind.len() as u64);
        (*message).data = mesh_bytes_new(data.as_ptr(), data.len() as u64);
        (*message).close_code = close_code;
        (*message).close_reason = mesh_string_new(close_reason.as_ptr(), close_reason.len() as u64);
        alloc_result(0, message.cast())
    }
}

fn update_options(handle: i64, update: impl FnOnce(&mut WsClientOptions)) -> i64 {
    if handle <= 0 {
        return handle;
    }
    if let Some(value) = options().lock().get_mut(&(handle as u64)) {
        update(value);
    }
    handle
}

#[no_mangle]
pub extern "C" fn mesh_ws_client_options() -> i64 {
    let mut registry = options().lock();
    if registry.len() >= MAX_OPEN_HANDLES {
        return 0;
    }
    let handle = next_handle();
    registry.insert(handle, WsClientOptions::default());
    handle as i64
}

#[no_mangle]
pub extern "C" fn mesh_ws_client_connect_timeout(handle: i64, timeout_ms: i64) -> i64 {
    update_options(handle, |options| {
        options.connect_timeout = duration_from_millis(timeout_ms)
    })
}

#[no_mangle]
pub extern "C" fn mesh_ws_client_heartbeat_timeout(handle: i64, timeout_ms: i64) -> i64 {
    update_options(handle, |options| {
        options.heartbeat_timeout = duration_from_millis(timeout_ms)
    })
}

#[no_mangle]
pub extern "C" fn mesh_ws_client_max_message_bytes(handle: i64, bytes: i64) -> i64 {
    update_options(handle, |options| {
        options.max_message_bytes = usize::try_from(bytes).unwrap_or(usize::MAX)
    })
}

#[no_mangle]
pub extern "C" fn mesh_ws_client_queue_capacity(handle: i64, capacity: i64) -> i64 {
    update_options(handle, |options| {
        options.queue_capacity = usize::try_from(capacity).unwrap_or(usize::MAX)
    })
}

fn duration_from_millis(value: i64) -> Duration {
    Duration::from_millis(u64::try_from(value).unwrap_or(u64::MAX))
}

fn validate_options(options: &WsClientOptions) -> Result<(), String> {
    if options.connect_timeout.is_zero() || options.connect_timeout > Duration::from_secs(120) {
        return Err("connect timeout must be between 1 and 120000 milliseconds".to_string());
    }
    if options.heartbeat_timeout < Duration::from_secs(1)
        || options.heartbeat_timeout > Duration::from_secs(300)
    {
        return Err("heartbeat timeout must be between 1000 and 300000 milliseconds".to_string());
    }
    if !(1..=MAX_MESSAGE_BYTES).contains(&options.max_message_bytes) {
        return Err(format!(
            "maximum message size must be between 1 and {MAX_MESSAGE_BYTES} bytes"
        ));
    }
    if !(1..=MAX_QUEUE_CAPACITY).contains(&options.queue_capacity) {
        return Err(format!(
            "queue capacity must be between 1 and {MAX_QUEUE_CAPACITY}"
        ));
    }
    Ok(())
}

#[no_mangle]
pub extern "C" fn mesh_ws_client_connect(
    url: *const MeshString,
    options_handle: i64,
) -> *mut MeshResult {
    if url.is_null() || options_handle <= 0 {
        return error("invalid WebSocket URL or options handle");
    }
    let Some(options) = options().lock().remove(&(options_handle as u64)) else {
        return error("invalid or already-consumed WebSocket options handle");
    };
    if let Err(reason) = validate_options(&options) {
        return error(reason);
    }
    if connections().lock().len() >= MAX_OPEN_HANDLES {
        return error("WebSocket connection limit reached");
    }
    let Some(worker_permit) = ConnectWorkerPermit::reserve() else {
        return error("WebSocket connect worker limit reached");
    };

    let url = unsafe { (*url).as_str().to_string() };
    let deadline = Instant::now() + options.connect_timeout;
    let (sender, receiver) = cooperative_channel();
    let worker = std::thread::Builder::new()
        .name("mesh-ws-connect".to_string())
        .spawn(move || {
            let _permit = worker_permit;
            send_connect_result(sender, connect_until(&url, options, deadline));
        });
    if let Err(reason) = worker {
        return error(format!("WebSocket connect worker spawn failed: {reason}"));
    }

    match cooperative_recv_timeout(
        &receiver,
        deadline.saturating_duration_since(Instant::now()),
    ) {
        Ok(Ok(connection)) => {
            let handle = next_handle();
            connections().lock().insert(handle, Arc::new(connection));
            ok_int(handle as i64)
        }
        Ok(Err(reason)) => error(reason),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => error("TIMEOUT: WebSocket connect"),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            error("WebSocket connect worker stopped")
        }
    }
}

fn send_connect_result(
    sender: CooperativeSender<Result<WsConnection, String>>,
    result: Result<WsConnection, String>,
) {
    if let Err(reason) = sender.send(result) {
        if let Ok(connection) = reason.0 {
            connection.io.cancel("abandoned WebSocket connect result");
        }
    }
}

#[cfg(test)]
fn connect(url: &str, options: WsClientOptions) -> Result<WsConnection, String> {
    let deadline = Instant::now() + options.connect_timeout;
    connect_until(url, options, deadline)
}

fn connect_until(
    url: &str,
    options: WsClientOptions,
    deadline: Instant,
) -> Result<WsConnection, String> {
    let parsed = Url::parse(url).map_err(|reason| format!("invalid WebSocket URL: {reason}"))?;
    let secure = match parsed.scheme() {
        "ws" => false,
        "wss" => true,
        _ => return Err("WebSocket URL must use ws:// or wss://".to_string()),
    };
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("WebSocket URL userinfo is not supported".to_string());
    }
    if parsed.fragment().is_some() {
        return Err("WebSocket URL fragments are not sent to servers".to_string());
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| "WebSocket URL is missing a host".to_string())?
        .to_string();
    let port = parsed
        .port_or_known_default()
        .ok_or_else(|| "WebSocket URL is missing a port".to_string())?;
    if Instant::now() >= deadline {
        return Err("TIMEOUT: WebSocket connect".to_string());
    }
    let addresses = (host.as_str(), port).to_socket_addrs();
    if Instant::now() >= deadline {
        return Err("TIMEOUT: WebSocket connect".to_string());
    }
    let addresses = addresses.map_err(|reason| format!("DNS_FAILURE: {reason}"))?;
    let mut tcp = None;
    for address in addresses {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return Err("TIMEOUT: WebSocket connect".to_string());
        };
        if let Ok(stream) = TcpStream::connect_timeout(&address, remaining) {
            tcp = Some(stream);
            break;
        }
    }
    let tcp = tcp.ok_or_else(|| {
        if Instant::now() >= deadline {
            "TIMEOUT: WebSocket connect".to_string()
        } else {
            "CONNECT_FAILURE: no resolved address accepted the connection".to_string()
        }
    })?;
    tcp.set_nodelay(true).ok();

    let stream = if secure {
        let mut roots = RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let config = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let server_name = ServerName::try_from(host.clone())
            .map_err(|_| "TLS_ERROR: invalid certificate server name".to_string())?;
        let connection = ClientConnection::new(Arc::new(config), server_name)
            .map_err(|reason| format!("TLS_ERROR: {reason}"))?;
        ReactorTransport::client_tls(StreamOwned::new(connection, tcp))
            .map_err(|reason| format!("configure WebSocket TLS transport: {reason}"))?
    } else {
        ReactorTransport::plain(tcp)
            .map_err(|reason| format!("configure WebSocket transport: {reason}"))?
    };
    let inbound = Arc::new(Mutex::new(InboundState {
        queue: VecDeque::new(),
        queued_bytes: 0,
        capacity: options.queue_capacity,
        max_bytes: options
            .max_message_bytes
            .saturating_mul(options.queue_capacity)
            .min(MAX_INBOUND_QUEUE_BYTES)
            .max(options.max_message_bytes),
        waiter: None,
        next_waiter_id: 1,
        terminal_error: None,
    }));
    let (ready_sender, ready_receiver) = std::sync::mpsc::sync_channel(1);
    let sink = Arc::new(ClientSink {
        inbound: Arc::clone(&inbound),
        ready: Mutex::new(Some(ready_sender)),
    });
    let (request, key) = handshake_request(&parsed, &host, port, secure)?;
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .ok_or_else(|| "TIMEOUT: WebSocket connect".to_string())?;
    let config = ReactorConfig::client(options.max_message_bytes, options.heartbeat_timeout)
        .with_handshake_timeout(remaining);
    let io = register_client(stream, request, key, sink, config)?;
    let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
        io.cancel("TIMEOUT: WebSocket handshake");
        return Err("TIMEOUT: WebSocket handshake".to_string());
    };
    match ready_receiver.recv_timeout(remaining) {
        Ok(Ok(())) => Ok(WsConnection { io, inbound }),
        Ok(Err(reason)) => Err(reason),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            io.cancel("TIMEOUT: WebSocket handshake");
            Err("TIMEOUT: WebSocket handshake".to_string())
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            io.cancel("WebSocket handshake worker stopped");
            Err("WebSocket handshake worker stopped".to_string())
        }
    }
}

fn handshake_request(
    url: &Url,
    host: &str,
    port: u16,
    secure: bool,
) -> Result<(Vec<u8>, String), String> {
    let nonce: [u8; 16] = rand::random();
    let key = STANDARD.encode(nonce);
    let mut path = if url.path().is_empty() {
        "/".to_string()
    } else {
        url.path().to_string()
    };
    if let Some(query) = url.query() {
        path.push('?');
        path.push_str(query);
    }
    let host_header = host_header(host, port, secure);
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {host_header}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
    )
    .into_bytes();
    if request.len() > MAX_HANDSHAKE_BYTES {
        Err(format!(
            "WebSocket handshake request exceeds {MAX_HANDSHAKE_BYTES} bytes"
        ))
    } else {
        Ok((request, key))
    }
}

fn host_header(host: &str, port: u16, secure: bool) -> String {
    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    if port == if secure { 443 } else { 80 } {
        host
    } else {
        format!("{host}:{port}")
    }
}

fn deliver(inbound: &Arc<Mutex<InboundState>>, mut event: ClientEvent) -> bool {
    loop {
        let waiter = {
            let mut inbound = inbound.lock();
            if let Some(waiter) = inbound.waiter.take() {
                waiter
            } else if inbound.queue.len() >= inbound.capacity
                || inbound
                    .queued_bytes
                    .checked_add(event.byte_len())
                    .is_none_or(|bytes| bytes > inbound.max_bytes)
            {
                inbound.terminal_error = Some("BACKPRESSURE: inbound queue is full".to_string());
                return false;
            } else {
                inbound.queued_bytes += event.byte_len();
                inbound.queue.push_back(event);
                return true;
            }
        };
        match waiter.sender.send(event) {
            Ok(()) => return true,
            Err(reason) => event = reason.0,
        }
    }
}

fn terminate(inbound: &Arc<Mutex<InboundState>>, reason: &str) {
    let waiter = {
        let mut inbound = inbound.lock();
        inbound
            .terminal_error
            .get_or_insert_with(|| reason.to_string());
        inbound.waiter.take()
    };
    if let Some(waiter) = waiter {
        let _ = waiter
            .sender
            .send(ClientEvent::Close(1006, reason.to_string()));
    }
}

fn connection(handle: i64) -> Result<Arc<WsConnection>, String> {
    if handle <= 0 {
        return Err("invalid WebSocket connection handle".to_string());
    }
    connections()
        .lock()
        .get(&(handle as u64))
        .cloned()
        .ok_or_else(|| "closed or unknown WebSocket connection".to_string())
}

fn send_frame(handle: i64, opcode: WsOpcode, data: &[u8]) -> *mut MeshResult {
    let connection = match connection(handle) {
        Ok(connection) => connection,
        Err(reason) => return error(reason),
    };
    match connection.io.send(opcode, data) {
        Ok(()) => ok_unit(),
        Err(reason) => error(reason),
    }
}

#[no_mangle]
pub extern "C" fn mesh_ws_client_send_text(
    handle: i64,
    body: *const MeshString,
) -> *mut MeshResult {
    if body.is_null() {
        return error("WebSocket text body is null");
    }
    let body = unsafe { (*body).as_str().as_bytes() };
    send_frame(handle, WsOpcode::Text, body)
}

#[no_mangle]
pub extern "C" fn mesh_ws_client_send_bytes(
    handle: i64,
    body: *const MeshBytes,
) -> *mut MeshResult {
    if body.is_null() {
        return error("WebSocket binary body is null");
    }
    let body = unsafe { (*body).as_slice() };
    send_frame(handle, WsOpcode::Binary, body)
}

#[no_mangle]
pub extern "C" fn mesh_ws_client_recv(handle: i64, timeout_ms: i64) -> *mut MeshResult {
    if timeout_ms < 0 {
        return error("WebSocket receive timeout must be non-negative");
    }
    let connection = match connection(handle) {
        Ok(connection) => connection,
        Err(reason) => return error(reason),
    };
    let (sender, receiver) = cooperative_channel();
    let waiter_id = {
        let mut inbound = connection.inbound.lock();
        if let Some(event) = inbound.queue.pop_front() {
            inbound.queued_bytes -= event.byte_len();
            return mesh_message(event);
        }
        if let Some(reason) = &inbound.terminal_error {
            return error(reason);
        }
        if inbound.waiter.is_some() {
            return error("only one concurrent receiver is allowed per WebSocket connection");
        }
        let id = inbound.next_waiter_id;
        inbound.next_waiter_id = inbound.next_waiter_id.wrapping_add(1);
        inbound.waiter = Some(Waiter { id, sender });
        id
    };

    let timeout = Duration::from_millis(timeout_ms as u64);
    match cooperative_recv_timeout(&receiver, timeout) {
        Ok(event) => mesh_message(event),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            let mut inbound = connection.inbound.lock();
            if inbound
                .waiter
                .as_ref()
                .is_some_and(|waiter| waiter.id == waiter_id)
            {
                inbound.waiter = None;
            }
            drop(inbound);
            match receiver.try_recv() {
                Ok(event) => mesh_message(event),
                Err(_) => error("TIMEOUT: WebSocket receive"),
            }
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            error("WebSocket receive channel disconnected")
        }
    }
}

#[no_mangle]
pub extern "C" fn mesh_ws_client_close(
    handle: i64,
    code: i64,
    reason: *const MeshString,
) -> *mut MeshResult {
    let Ok(code) = u16::try_from(code) else {
        return error("invalid WebSocket close code");
    };
    if !is_valid_close_code(code) {
        return error("invalid WebSocket close code");
    }
    if reason.is_null() {
        return error("WebSocket close reason is null");
    }
    let Some(connection) = connections().lock().remove(&(handle as u64)) else {
        return error("closed or unknown WebSocket connection");
    };
    let reason = unsafe { (*reason).as_str() };
    match connection.io.graceful_close(code, reason) {
        Ok(()) => ok_unit(),
        Err(reason) => {
            connection.io.cancel(reason.clone());
            error(reason)
        }
    }
}

#[no_mangle]
pub extern "C" fn mesh_ws_client_reconnect_delay(
    attempt: i64,
    base_ms: i64,
    max_ms: i64,
    jitter_ppm: i64,
) -> *mut MeshResult {
    match reconnect_delay(attempt, base_ms, max_ms, jitter_ppm) {
        Ok(delay) => ok_int(delay),
        Err(reason) => error(reason),
    }
}

fn reconnect_delay(
    attempt: i64,
    base_ms: i64,
    max_ms: i64,
    jitter_ppm: i64,
) -> Result<i64, String> {
    if !(0..=62).contains(&attempt)
        || base_ms <= 0
        || max_ms < base_ms
        || !(0..=1_000_000).contains(&jitter_ppm)
    {
        return Err(
            "invalid reconnect policy: require attempt 0..62, 0 < base <= max, jitter 0..1000000"
                .to_string(),
        );
    }
    let delay = base_ms.saturating_mul(1i64 << attempt).min(max_ms);
    let spread = ((delay as i128) * (jitter_ppm as i128) / 1_000_000) as i64;
    let lower = delay.saturating_sub(spread).max(0);
    let upper = delay.saturating_add(spread).min(max_ms);
    let width = (upper - lower) as u64 + 1;
    Ok(lower + (rand::random::<u64>() % width) as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ws::close::{build_close_payload, parse_close_payload};
    use crate::ws::frame::{read_frame_with_mask, write_frame};
    use crate::ws::handshake::compute_accept_key;
    use rustls::ServerConnection;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::Instant;

    fn accept_handshake(listener: TcpListener) -> TcpStream {
        let (mut stream, _) = listener.accept().unwrap();
        complete_handshake(&mut stream).unwrap();
        stream
    }

    fn complete_handshake(stream: &mut (impl Read + Write)) -> std::io::Result<()> {
        let key = read_handshake_key(stream)?;
        write_handshake_response(stream, &key)
    }

    fn read_handshake_key(stream: &mut impl Read) -> std::io::Result<String> {
        let mut request = Vec::new();
        let mut byte = [0u8; 1];
        while !request.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte)?;
            request.push(byte[0]);
        }
        let request = String::from_utf8(request).unwrap();
        Ok(request
            .lines()
            .find_map(|line| {
                line.split_once(':')
                    .filter(|(name, _)| name.eq_ignore_ascii_case("Sec-WebSocket-Key"))
                    .map(|(_, value)| value.trim())
            })
            .unwrap()
            .to_string())
    }

    fn write_handshake_response(stream: &mut impl Write, key: &str) -> std::io::Result<()> {
        write!(
            stream,
            "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
            compute_accept_key(key)
        )?;
        stream.flush()
    }

    fn register_plain_test_client(
        port: u16,
        handshake_timeout: Duration,
    ) -> (
        ReactorConnection,
        std::sync::mpsc::Receiver<Result<(), String>>,
    ) {
        let tcp = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let stream = ReactorTransport::plain(tcp).unwrap();
        let inbound = Arc::new(Mutex::new(InboundState {
            queue: VecDeque::new(),
            queued_bytes: 0,
            capacity: 1,
            max_bytes: 1024,
            waiter: None,
            next_waiter_id: 1,
            terminal_error: None,
        }));
        let (ready_sender, ready_receiver) = std::sync::mpsc::sync_channel(1);
        let sink = Arc::new(ClientSink {
            inbound,
            ready: Mutex::new(Some(ready_sender)),
        });
        let url = Url::parse(&format!("ws://127.0.0.1:{port}/feed")).unwrap();
        let (request, key) = handshake_request(&url, "127.0.0.1", port, false).unwrap();
        let io = register_client(
            stream,
            request,
            key,
            sink,
            ReactorConfig::client(1024, Duration::from_secs(30))
                .with_handshake_timeout(handshake_timeout),
        )
        .unwrap();
        (io, ready_receiver)
    }

    fn binary_event(data: Vec<u8>) -> ClientEvent {
        let permit = InboundPermit::reserve(data.len()).unwrap();
        ClientEvent::Binary(data, permit)
    }

    #[test]
    fn ipv6_host_header_keeps_required_brackets() {
        assert_eq!(host_header("::1", 80, false), "[::1]");
        assert_eq!(host_header("::1", 8080, false), "[::1]:8080");
    }

    #[test]
    fn handshake_request_rejects_an_oversized_path() {
        let url = Url::parse(&format!("ws://example.test/{}", "x".repeat(16 * 1024))).unwrap();
        assert!(handshake_request(&url, "example.test", 80, false).is_err());
    }

    #[test]
    fn connect_worker_limit_is_released_only_when_a_worker_finishes() {
        let mut permits = (0..MAX_CONNECT_WORKERS)
            .map(|_| ConnectWorkerPermit::reserve().unwrap())
            .collect::<Vec<_>>();
        assert!(ConnectWorkerPermit::reserve().is_none());

        let permit = permits.pop().unwrap();
        let (started_tx, started_rx) = std::sync::mpsc::sync_channel(1);
        let (finish_tx, finish_rx) = std::sync::mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || {
            let _permit = permit;
            started_tx.send(()).unwrap();
            finish_rx.recv().unwrap();
        });
        started_rx.recv().unwrap();
        assert!(ConnectWorkerPermit::reserve().is_none());

        finish_tx.send(()).unwrap();
        worker.join().unwrap();
        assert!(ConnectWorkerPermit::reserve().is_some());
    }

    #[test]
    fn rejected_handshake_closes_tcp_without_sending_a_websocket_frame() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut byte = [0u8; 1];
            while !request.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            stream
                .write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n")
                .unwrap();
            stream.flush().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            assert_eq!(stream.read(&mut byte).unwrap(), 0);
        });

        let result = connect(
            &format!("ws://127.0.0.1:{port}/"),
            WsClientOptions::default(),
        );
        assert!(matches!(result, Err(reason) if reason.contains("403 Forbidden")));
        server.join().unwrap();
    }

    #[test]
    fn a_response_after_the_handshake_deadline_cannot_open_the_connection() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let key = read_handshake_key(&mut stream).unwrap();
            std::thread::sleep(Duration::from_millis(150));
            let _ = write_handshake_response(&mut stream, &key);
        });

        let (_io, ready) = register_plain_test_client(port, Duration::from_millis(100));
        let reason = ready
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .unwrap_err();
        assert!(reason.contains("TIMEOUT"));
        server.join().unwrap();
    }

    #[test]
    fn cancellation_wins_over_a_ready_handshake_response() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (request_seen, request_ready) = std::sync::mpsc::sync_channel(1);
        let (release, released) = std::sync::mpsc::sync_channel(1);
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let key = read_handshake_key(&mut stream).unwrap();
            request_seen.send(()).unwrap();
            released.recv().unwrap();
            let _ = write_handshake_response(&mut stream, &key);
        });

        let (io, ready) = register_plain_test_client(port, Duration::from_secs(1));
        request_ready.recv().unwrap();
        io.cancel("test cancellation");
        release.send(()).unwrap();
        assert_eq!(
            ready.recv_timeout(Duration::from_secs(1)).unwrap(),
            Err("test cancellation".to_string())
        );
        server.join().unwrap();
    }

    #[test]
    fn delivery_requeues_after_receiver_timeout_race() {
        let (sender, receiver) = cooperative_channel();
        drop(receiver);
        let inbound = Arc::new(Mutex::new(InboundState {
            queue: VecDeque::new(),
            queued_bytes: 0,
            capacity: 1,
            max_bytes: 1,
            waiter: Some(Waiter { id: 1, sender }),
            next_waiter_id: 2,
            terminal_error: None,
        }));

        assert!(deliver(&inbound, binary_event(vec![1])));
        assert_eq!(inbound.lock().queue.len(), 1);
    }

    #[test]
    fn delivery_fails_closed_when_the_queue_is_full() {
        let inbound = Arc::new(Mutex::new(InboundState {
            queue: VecDeque::from([binary_event(vec![1])]),
            queued_bytes: 1,
            capacity: 1,
            max_bytes: 2,
            waiter: None,
            next_waiter_id: 1,
            terminal_error: None,
        }));

        assert!(!deliver(&inbound, binary_event(vec![2])));
        assert_eq!(
            inbound.lock().terminal_error.as_deref(),
            Some("BACKPRESSURE: inbound queue is full")
        );
    }

    #[test]
    fn delivery_fails_closed_when_the_queue_byte_budget_is_full() {
        let inbound = Arc::new(Mutex::new(InboundState {
            queue: VecDeque::from([binary_event(vec![1])]),
            queued_bytes: 1,
            capacity: 2,
            max_bytes: 1,
            waiter: None,
            next_waiter_id: 1,
            terminal_error: None,
        }));

        assert!(!deliver(&inbound, binary_event(vec![2])));
        assert_eq!(
            inbound.lock().terminal_error.as_deref(),
            Some("BACKPRESSURE: inbound queue is full")
        );
    }

    #[test]
    fn abandoned_connect_result_closes_the_socket() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let mut peer = accept_handshake(listener);
            peer.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
            assert_eq!(peer.read(&mut [0u8; 1]).unwrap(), 0);
        });
        let connection = connect(
            &format!("ws://127.0.0.1:{port}/"),
            WsClientOptions::default(),
        )
        .unwrap();
        let (sender, receiver) = cooperative_channel();
        drop(receiver);

        send_connect_result(sender, Ok(connection));
        server.join().unwrap();
    }

    #[test]
    fn reconnect_delay_is_bounded_and_rejects_invalid_policy() {
        for _ in 0..100 {
            let delay = reconnect_delay(3, 100, 1_000, 100_000).unwrap();
            assert!((720..=880).contains(&delay));
        }
        assert!(reconnect_delay(-1, 100, 1_000, 0).is_err());
        assert!(reconnect_delay(1, 1_000, 100, 0).is_err());
    }

    #[test]
    fn heartbeat_timeout_closes_an_unresponsive_peer() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let mut stream = accept_handshake(listener);
            let (ping, masked) = read_frame_with_mask(&mut stream).unwrap();
            assert!(masked);
            assert_eq!(ping.opcode, WsOpcode::Ping);
            let (close, masked) = read_frame_with_mask(&mut stream).unwrap();
            assert!(masked);
            assert_eq!(close.opcode, WsOpcode::Close);
            assert_eq!(parse_close_payload(&close.payload).0, 1001);
            assert_eq!(stream.read(&mut [0u8; 1]).unwrap(), 0);
        });

        let connection = connect(
            &format!("ws://127.0.0.1:{port}/feed"),
            WsClientOptions {
                heartbeat_timeout: Duration::from_secs(1),
                ..WsClientOptions::default()
            },
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while connection.inbound.lock().terminal_error.is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            connection.inbound.lock().terminal_error.as_deref(),
            Some("HEARTBEAT_TIMEOUT")
        );
        server.join().unwrap();
    }

    #[test]
    fn local_close_waits_for_the_peer_close() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let mut stream = accept_handshake(listener);
            let (close, masked) = read_frame_with_mask(&mut stream).unwrap();
            assert!(masked);
            assert_eq!(close.opcode, WsOpcode::Close);

            stream
                .set_read_timeout(Some(Duration::from_millis(200)))
                .unwrap();
            let mut byte = [0u8; 1];
            assert!(matches!(
                stream.read(&mut byte),
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    )
            ));

            write_frame(&mut stream, WsOpcode::Close, &close.payload, true).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            assert_eq!(stream.read(&mut byte).unwrap(), 0);
        });

        let connection = connect(
            &format!("ws://127.0.0.1:{port}/feed"),
            WsClientOptions::default(),
        )
        .unwrap();
        connection.io.graceful_close(1000, "done").unwrap();
        server.join().unwrap();
    }

    #[test]
    fn local_client_masks_writes_and_reassembles_bounded_messages() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let mut stream = accept_handshake(listener);

            let (frame, masked) = read_frame_with_mask(&mut stream).unwrap();
            assert!(masked);
            assert_eq!(frame.opcode, WsOpcode::Text);
            assert_eq!(frame.payload, b"subscribe");

            write_frame(&mut stream, WsOpcode::Text, b"ready", true).unwrap();
            write_frame(&mut stream, WsOpcode::Binary, &[1], false).unwrap();
            write_frame(&mut stream, WsOpcode::Continuation, &[2, 3], true).unwrap();
            let close = build_close_payload(1000, "done");
            write_frame(&mut stream, WsOpcode::Close, &close, true).unwrap();
        });

        let connection = connect(
            &format!("ws://127.0.0.1:{port}/feed"),
            WsClientOptions {
                queue_capacity: 3,
                ..WsClientOptions::default()
            },
        )
        .unwrap();
        connection.io.send(WsOpcode::Text, b"subscribe").unwrap();

        let deadline = Instant::now() + Duration::from_secs(2);
        while connection.inbound.lock().queue.len() < 3 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut inbound = connection.inbound.lock();
        assert!(
            matches!(inbound.queue.pop_front(), Some(ClientEvent::Text(data, _)) if data == b"ready")
        );
        assert!(
            matches!(inbound.queue.pop_front(), Some(ClientEvent::Binary(data, _)) if data == [1, 2, 3])
        );
        assert!(
            matches!(inbound.queue.pop_front(), Some(ClientEvent::Close(1000, reason)) if reason == "done")
        );
        drop(inbound);
        connection.io.cancel("test complete");
        assert_eq!(crate::ws::reactor::reactor_threads_started(), 1);
        server.join().unwrap();
    }

    #[test]
    fn shared_reactor_drains_a_large_final_message_before_tcp_close() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let expected = vec![0x5a; 256 * 1024];
        let payload = expected.clone();
        let server = std::thread::spawn(move || {
            let mut stream = accept_handshake(listener);
            write_frame(&mut stream, WsOpcode::Binary, &payload, true).unwrap();
        });

        let connection = connect(
            &format!("ws://127.0.0.1:{port}/feed"),
            WsClientOptions {
                max_message_bytes: expected.len(),
                queue_capacity: 1,
                ..WsClientOptions::default()
            },
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while connection.inbound.lock().queue.is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut inbound = connection.inbound.lock();
        assert!(
            matches!(inbound.queue.pop_front(), Some(ClientEvent::Binary(data, _)) if data == expected)
        );
        connection.io.cancel("test complete");
        server.join().unwrap();
    }

    #[test]
    fn shared_reactor_drives_large_wss_messages_in_both_directions() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let (server_config, client_config) = crate::dist::node::ws_test_tls_configs();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let sent = vec![0x2a; 128 * 1024];
        let expected = vec![0x5a; 256 * 1024];
        let server_expected = expected.clone();
        let server_sent = sent.clone();
        let server = std::thread::spawn(move || {
            let (tcp, _) = listener.accept().unwrap();
            tcp.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            tcp.set_write_timeout(Some(Duration::from_secs(3))).unwrap();
            let connection = ServerConnection::new(server_config).unwrap();
            let mut stream = StreamOwned::new(connection, tcp);
            complete_handshake(&mut stream).unwrap();

            let (frame, masked) = read_frame_with_mask(&mut stream).unwrap();
            assert!(masked);
            assert_eq!(frame.payload, server_sent);
            write_frame(&mut stream, WsOpcode::Binary, &server_expected, true).unwrap();
            let close = build_close_payload(1000, "done");
            write_frame(&mut stream, WsOpcode::Close, &close, true).unwrap();
            let (reply, masked) = read_frame_with_mask(&mut stream).unwrap();
            assert!(masked);
            assert_eq!(reply.opcode, WsOpcode::Close);
            assert_eq!(stream.read(&mut [0u8; 1]).unwrap(), 0);
        });

        let tcp = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let server_name = ServerName::try_from("localhost".to_string()).unwrap();
        let connection = ClientConnection::new(client_config, server_name).unwrap();
        let stream = ReactorTransport::client_tls(StreamOwned::new(connection, tcp)).unwrap();
        let inbound = Arc::new(Mutex::new(InboundState {
            queue: VecDeque::new(),
            queued_bytes: 0,
            capacity: 2,
            max_bytes: expected.len() + "done".len(),
            waiter: None,
            next_waiter_id: 1,
            terminal_error: None,
        }));
        let (ready_sender, ready_receiver) = std::sync::mpsc::sync_channel(1);
        let sink = Arc::new(ClientSink {
            inbound: Arc::clone(&inbound),
            ready: Mutex::new(Some(ready_sender)),
        });
        let url = Url::parse(&format!("wss://localhost:{port}/feed")).unwrap();
        let (request, key) = handshake_request(&url, "localhost", port, true).unwrap();
        let io = register_client(
            stream,
            request,
            key,
            sink,
            ReactorConfig::client(expected.len(), Duration::from_secs(30))
                .with_handshake_timeout(Duration::from_secs(2)),
        )
        .unwrap();
        ready_receiver
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap();
        io.send(WsOpcode::Binary, &sent).unwrap();

        let deadline = Instant::now() + Duration::from_secs(3);
        while inbound.lock().queue.len() < 2 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut inbound = inbound.lock();
        assert!(
            matches!(inbound.queue.pop_front(), Some(ClientEvent::Binary(data, _)) if data == expected)
        );
        assert!(
            matches!(inbound.queue.pop_front(), Some(ClientEvent::Close(1000, reason)) if reason == "done")
        );
        drop(inbound);
        server.join().unwrap();
    }
}
