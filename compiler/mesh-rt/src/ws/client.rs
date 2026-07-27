//! Bounded, scheduler-aware WebSocket client.

use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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

use super::close::{build_close_payload, parse_close_payload, validate_text_payload};
use super::frame::{
    read_frame_with_mask, write_masked_frame, MessageAssembler, ReassembleResult, WsOpcode,
};
use super::handshake::compute_accept_key;

const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;
const MAX_QUEUE_CAPACITY: usize = 65_536;
const MAX_OPEN_HANDLES: usize = 4_096;
const IO_POLL_INTERVAL: Duration = Duration::from_millis(100);

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

enum ClientStream {
    Plain(TcpStream),
    Tls(StreamOwned<ClientConnection, TcpStream>),
}

impl ClientStream {
    fn set_read_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        match self {
            Self::Plain(stream) => stream.set_read_timeout(timeout),
            Self::Tls(stream) => stream.get_ref().set_read_timeout(timeout),
        }
    }

    fn set_write_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        match self {
            Self::Plain(stream) => stream.set_write_timeout(timeout),
            Self::Tls(stream) => stream.get_ref().set_write_timeout(timeout),
        }
    }

    fn shutdown(&self) {
        let _ = match self {
            Self::Plain(stream) => stream.shutdown(Shutdown::Both),
            Self::Tls(stream) => stream.get_ref().shutdown(Shutdown::Both),
        };
    }
}

impl Read for ClientStream {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.read(buffer),
            Self::Tls(stream) => stream.read(buffer),
        }
    }
}

impl Write for ClientStream {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.write(buffer),
            Self::Tls(stream) => stream.write(buffer),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Plain(stream) => stream.flush(),
            Self::Tls(stream) => stream.flush(),
        }
    }
}

enum ClientEvent {
    Text(Vec<u8>),
    Binary(Vec<u8>),
    Close(u16, String),
}

struct Waiter {
    id: u64,
    sender: CooperativeSender<ClientEvent>,
}

struct InboundState {
    queue: VecDeque<ClientEvent>,
    capacity: usize,
    waiter: Option<Waiter>,
    next_waiter_id: u64,
    terminal_error: Option<String>,
}

struct WsConnection {
    stream: Arc<Mutex<ClientStream>>,
    inbound: Arc<Mutex<InboundState>>,
    shutdown: Arc<AtomicBool>,
    max_message_bytes: usize,
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
        ClientEvent::Text(data) => ("text", data, 0, String::new()),
        ClientEvent::Binary(data) => ("binary", data, 0, String::new()),
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

    let url = unsafe { (*url).as_str().to_string() };
    let timeout = options.connect_timeout;
    let (sender, receiver) = cooperative_channel();
    std::thread::spawn(move || {
        send_connect_result(sender, connect(&url, options));
    });

    match cooperative_recv_timeout(&receiver, timeout) {
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
            connection.shutdown.store(true, Ordering::Release);
            connection.stream.lock().shutdown();
        }
    }
}

fn connect(url: &str, options: WsClientOptions) -> Result<WsConnection, String> {
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
    let tcp = (host.as_str(), port)
        .to_socket_addrs()
        .map_err(|reason| format!("DNS_FAILURE: {reason}"))?
        .find_map(|address| TcpStream::connect_timeout(&address, options.connect_timeout).ok())
        .ok_or_else(|| {
            "CONNECT_FAILURE: no resolved address accepted the connection".to_string()
        })?;
    tcp.set_nodelay(true).ok();
    tcp.set_read_timeout(Some(options.connect_timeout))
        .map_err(|reason| format!("set read timeout: {reason}"))?;
    tcp.set_write_timeout(Some(options.connect_timeout))
        .map_err(|reason| format!("set write timeout: {reason}"))?;

    let mut stream = if secure {
        let mut roots = RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let config = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let server_name = ServerName::try_from(host.clone())
            .map_err(|_| "TLS_ERROR: invalid certificate server name".to_string())?;
        let connection = ClientConnection::new(Arc::new(config), server_name)
            .map_err(|reason| format!("TLS_ERROR: {reason}"))?;
        ClientStream::Tls(StreamOwned::new(connection, tcp))
    } else {
        ClientStream::Plain(tcp)
    };

    perform_handshake(&mut stream, &parsed, &host, port, secure)?;
    stream.set_read_timeout(Some(IO_POLL_INTERVAL)).ok();
    stream.set_write_timeout(Some(IO_POLL_INTERVAL)).ok();

    let stream = Arc::new(Mutex::new(stream));
    let inbound = Arc::new(Mutex::new(InboundState {
        queue: VecDeque::with_capacity(options.queue_capacity),
        capacity: options.queue_capacity,
        waiter: None,
        next_waiter_id: 1,
        terminal_error: None,
    }));
    let shutdown = Arc::new(AtomicBool::new(false));
    let connection = WsConnection {
        stream: Arc::clone(&stream),
        inbound: Arc::clone(&inbound),
        shutdown: Arc::clone(&shutdown),
        max_message_bytes: options.max_message_bytes,
    };
    std::thread::spawn(move || {
        reader_loop(
            stream,
            inbound,
            shutdown,
            options.heartbeat_timeout,
            options.max_message_bytes,
        )
    });
    Ok(connection)
}

fn perform_handshake(
    stream: &mut ClientStream,
    url: &Url,
    host: &str,
    port: u16,
    secure: bool,
) -> Result<(), String> {
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
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {host_header}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
    )
    .map_err(|reason| format!("write WebSocket handshake: {reason}"))?;
    stream
        .flush()
        .map_err(|reason| format!("flush WebSocket handshake: {reason}"))?;

    let mut response = Vec::with_capacity(1024);
    let mut byte = [0u8; 1];
    while !response.ends_with(b"\r\n\r\n") {
        if response.len() >= 16 * 1024 {
            return Err("WebSocket handshake headers exceed 16384 bytes".to_string());
        }
        stream
            .read_exact(&mut byte)
            .map_err(|reason| format!("read WebSocket handshake: {reason}"))?;
        response.push(byte[0]);
    }
    let response = std::str::from_utf8(&response)
        .map_err(|_| "WebSocket handshake is not valid UTF-8".to_string())?;
    validate_handshake_response(response, &key)
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

fn validate_handshake_response(response: &str, key: &str) -> Result<(), String> {
    let mut lines = response.split("\r\n");
    let status = lines.next().unwrap_or_default();
    let mut status_parts = status.split_ascii_whitespace();
    if !matches!(status_parts.next(), Some("HTTP/1.1" | "HTTP/1.0"))
        || status_parts.next() != Some("101")
    {
        return Err(format!("WebSocket upgrade rejected: {status}"));
    }
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_string()))
        .collect::<HashMap<_, _>>();
    let token = |name: &str, expected: &str| {
        headers.get(name).is_some_and(|value| {
            value
                .split(',')
                .any(|token| token.trim().eq_ignore_ascii_case(expected))
        })
    };
    if !token("upgrade", "websocket") || !token("connection", "upgrade") {
        return Err("WebSocket upgrade response is missing required headers".to_string());
    }
    if headers.get("sec-websocket-accept").map(String::as_str) != Some(&compute_accept_key(key)) {
        return Err("WebSocket upgrade response has an invalid accept key".to_string());
    }
    Ok(())
}

fn reader_loop(
    stream: Arc<Mutex<ClientStream>>,
    inbound: Arc<Mutex<InboundState>>,
    shutdown: Arc<AtomicBool>,
    heartbeat_timeout: Duration,
    max_message_bytes: usize,
) {
    let mut assembler = MessageAssembler::new(max_message_bytes);
    let ping_interval = heartbeat_timeout / 2;
    let mut last_ping = Instant::now();
    let mut pending_ping: Option<([u8; 4], Instant)> = None;

    while !shutdown.load(Ordering::Acquire) {
        if pending_ping.is_some_and(|(_, sent)| sent.elapsed() >= heartbeat_timeout) {
            terminate(&stream, &inbound, "HEARTBEAT_TIMEOUT");
            break;
        }
        if pending_ping.is_none() && last_ping.elapsed() >= ping_interval {
            let payload: [u8; 4] = rand::random();
            let sent = {
                let mut stream = stream.lock();
                write_masked_frame(&mut *stream, WsOpcode::Ping, &payload, true, rand::random())
            };
            if let Err(reason) = sent {
                terminate(&stream, &inbound, &reason);
                break;
            }
            last_ping = Instant::now();
            pending_ping = Some((payload, last_ping));
        }

        let frame = {
            let mut stream = stream.lock();
            read_frame_with_mask(&mut *stream)
        };
        let (frame, masked) = match frame {
            Ok(frame) => frame,
            Err(reason) if is_timeout(&reason) => continue,
            Err(reason) => {
                terminate(&stream, &inbound, &reason);
                break;
            }
        };
        if masked {
            terminate(&stream, &inbound, "server sent a masked WebSocket frame");
            break;
        }

        match frame.opcode {
            WsOpcode::Ping => {
                let result = {
                    let mut stream = stream.lock();
                    write_masked_frame(
                        &mut *stream,
                        WsOpcode::Pong,
                        &frame.payload,
                        true,
                        rand::random(),
                    )
                };
                if let Err(reason) = result {
                    terminate(&stream, &inbound, &reason);
                    break;
                }
            }
            WsOpcode::Pong => {
                if pending_ping.is_some_and(|(payload, _)| frame.payload == payload) {
                    pending_ping = None;
                }
            }
            WsOpcode::Close => {
                let (code, reason) = parse_close_payload(&frame.payload);
                {
                    let mut stream = stream.lock();
                    let payload = build_close_payload(code, "");
                    let _ = write_masked_frame(
                        &mut *stream,
                        WsOpcode::Close,
                        &payload,
                        true,
                        rand::random(),
                    );
                }
                deliver(&inbound, ClientEvent::Close(code, reason));
                terminate(&stream, &inbound, "WebSocket peer closed the connection");
                break;
            }
            WsOpcode::Text | WsOpcode::Binary | WsOpcode::Continuation => {
                match assembler.push(frame) {
                    ReassembleResult::Complete(message) => {
                        let event = match message.opcode {
                            WsOpcode::Text if validate_text_payload(&message.payload).is_ok() => {
                                ClientEvent::Text(message.payload)
                            }
                            WsOpcode::Text => {
                                terminate(&stream, &inbound, "invalid UTF-8 in text message");
                                break;
                            }
                            WsOpcode::Binary => ClientEvent::Binary(message.payload),
                            _ => unreachable!(),
                        };
                        if !deliver(&inbound, event) {
                            terminate(&stream, &inbound, "BACKPRESSURE: inbound queue is full");
                            break;
                        }
                    }
                    ReassembleResult::Accumulating => {}
                    ReassembleResult::TooLarge => {
                        terminate(&stream, &inbound, "MESSAGE_TOO_BIG");
                        break;
                    }
                    ReassembleResult::ProtocolError(reason) => {
                        terminate(&stream, &inbound, reason);
                        break;
                    }
                }
            }
        }
    }
    shutdown.store(true, Ordering::Release);
}

fn is_timeout(reason: &str) -> bool {
    reason.contains("timed out")
        || reason.contains("would block")
        || reason.contains("Resource temporarily unavailable")
}

fn deliver(inbound: &Arc<Mutex<InboundState>>, mut event: ClientEvent) -> bool {
    loop {
        let waiter = {
            let mut inbound = inbound.lock();
            if let Some(waiter) = inbound.waiter.take() {
                waiter
            } else if inbound.queue.len() >= inbound.capacity {
                inbound.terminal_error = Some("BACKPRESSURE: inbound queue is full".to_string());
                return false;
            } else {
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

fn terminate(stream: &Arc<Mutex<ClientStream>>, inbound: &Arc<Mutex<InboundState>>, reason: &str) {
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
    stream.lock().shutdown();
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
    if connection.shutdown.load(Ordering::Acquire) {
        return error("WebSocket connection is closed");
    }
    if data.len() > connection.max_message_bytes {
        return error("MESSAGE_TOO_BIG");
    }
    let result = {
        let mut stream = connection.stream.lock();
        write_masked_frame(&mut *stream, opcode, data, true, rand::random())
    };
    match result {
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
    if !(1000..=4999).contains(&code) || matches!(code, 1004 | 1005 | 1006 | 1015) {
        return error("invalid WebSocket close code");
    }
    if reason.is_null() {
        return error("WebSocket close reason is null");
    }
    let Some(connection) = connections().lock().remove(&(handle as u64)) else {
        return error("closed or unknown WebSocket connection");
    };
    let reason = unsafe { (*reason).as_str() };
    let payload = build_close_payload(code as u16, reason);
    let result = {
        let mut stream = connection.stream.lock();
        let result = write_masked_frame(
            &mut *stream,
            WsOpcode::Close,
            &payload,
            true,
            rand::random(),
        );
        stream.shutdown();
        result
    };
    connection.shutdown.store(true, Ordering::Release);
    match result {
        Ok(()) => ok_unit(),
        Err(reason) => error(reason),
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
    use crate::ws::frame::write_frame;
    use std::net::TcpListener;

    #[test]
    fn handshake_response_requires_the_rfc_accept_key() {
        let key = "dGhlIHNhbXBsZSBub25jZQ==";
        let response = format!(
            "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
            compute_accept_key(key)
        );
        assert!(validate_handshake_response(&response, key).is_ok());
        assert!(validate_handshake_response(
            "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: wrong\r\n\r\n",
            key
        )
        .is_err());
        let wrong_status = format!(
            "HTTP/1.1 1010 Not Switching\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
            compute_accept_key(key)
        );
        assert!(validate_handshake_response(&wrong_status, key).is_err());
    }

    #[test]
    fn ipv6_host_header_keeps_required_brackets() {
        assert_eq!(host_header("::1", 80, false), "[::1]");
        assert_eq!(host_header("::1", 8080, false), "[::1]:8080");
    }

    #[test]
    fn delivery_requeues_after_receiver_timeout_race() {
        let (sender, receiver) = cooperative_channel();
        drop(receiver);
        let inbound = Arc::new(Mutex::new(InboundState {
            queue: VecDeque::new(),
            capacity: 1,
            waiter: Some(Waiter { id: 1, sender }),
            next_waiter_id: 2,
            terminal_error: None,
        }));

        assert!(deliver(&inbound, ClientEvent::Binary(vec![1])));
        assert_eq!(inbound.lock().queue.len(), 1);
    }

    #[test]
    fn delivery_fails_closed_when_the_queue_is_full() {
        let inbound = Arc::new(Mutex::new(InboundState {
            queue: VecDeque::from([ClientEvent::Binary(vec![1])]),
            capacity: 1,
            waiter: None,
            next_waiter_id: 1,
            terminal_error: None,
        }));

        assert!(!deliver(&inbound, ClientEvent::Binary(vec![2])));
        assert_eq!(
            inbound.lock().terminal_error.as_deref(),
            Some("BACKPRESSURE: inbound queue is full")
        );
    }

    #[test]
    fn abandoned_connect_result_closes_the_socket() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let tcp = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut peer, _) = listener.accept().unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        let shutdown = Arc::new(AtomicBool::new(false));
        let connection = WsConnection {
            stream: Arc::new(Mutex::new(ClientStream::Plain(tcp))),
            inbound: Arc::new(Mutex::new(InboundState {
                queue: VecDeque::new(),
                capacity: 1,
                waiter: None,
                next_waiter_id: 1,
                terminal_error: None,
            })),
            shutdown: Arc::clone(&shutdown),
            max_message_bytes: 1,
        };
        let (sender, receiver) = cooperative_channel();
        drop(receiver);

        send_connect_result(sender, Ok(connection));

        assert!(shutdown.load(Ordering::Acquire));
        assert_eq!(peer.read(&mut [0u8; 1]).unwrap(), 0);
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
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut byte = [0u8; 1];
            while !request.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            let request = String::from_utf8(request).unwrap();
            let key = request
                .lines()
                .find_map(|line| {
                    line.split_once(':')
                        .filter(|(name, _)| name.eq_ignore_ascii_case("Sec-WebSocket-Key"))
                        .map(|(_, value)| value.trim())
                })
                .unwrap();
            write!(
                stream,
                "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
                compute_accept_key(key)
            )
            .unwrap();
            stream.flush().unwrap();
            let (ping, masked) = read_frame_with_mask(&mut stream).unwrap();
            assert!(masked);
            assert_eq!(ping.opcode, WsOpcode::Ping);
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
        let deadline = Instant::now() + Duration::from_secs(3);
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
    fn local_client_masks_writes_and_reassembles_bounded_messages() {
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
            let request = String::from_utf8(request).unwrap();
            let key = request
                .lines()
                .find_map(|line| {
                    line.split_once(':')
                        .filter(|(name, _)| name.eq_ignore_ascii_case("Sec-WebSocket-Key"))
                        .map(|(_, value)| value.trim())
                })
                .unwrap();
            write!(
                stream,
                "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
                compute_accept_key(key)
            )
            .unwrap();
            stream.flush().unwrap();

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
        {
            let mut stream = connection.stream.lock();
            write_masked_frame(
                &mut *stream,
                WsOpcode::Text,
                b"subscribe",
                true,
                [1, 2, 3, 4],
            )
            .unwrap();
        }

        let deadline = Instant::now() + Duration::from_secs(2);
        while connection.inbound.lock().queue.len() < 3 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut inbound = connection.inbound.lock();
        assert!(
            matches!(inbound.queue.pop_front(), Some(ClientEvent::Text(data)) if data == b"ready")
        );
        assert!(
            matches!(inbound.queue.pop_front(), Some(ClientEvent::Binary(data)) if data == [1, 2, 3])
        );
        assert!(
            matches!(inbound.queue.pop_front(), Some(ClientEvent::Close(1000, reason)) if reason == "done")
        );
        drop(inbound);
        connection.shutdown.store(true, Ordering::Release);
        connection.stream.lock().shutdown();
        server.join().unwrap();
    }
}
