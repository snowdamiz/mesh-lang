//! WebSocket server runtime: actor-per-connection over a shared I/O reactor.
//!
//! Integrates the Phase 59 WebSocket protocol layer (frame codec, handshake,
//! close) with Mesh's actor system. Each accepted WebSocket connection spawns
//! a dedicated actor with crash isolation via `catch_unwind`.
//!
//! Accepted sockets, TLS handshakes, HTTP upgrades, frame reads, and partial
//! writes are driven by one nonblocking readiness reactor. Actor workers only
//! exchange bounded in-memory messages with that reactor.
//!
//! ## Architecture
//!
//! ```text
//! TcpListener (accept-loop thread)
//!     |
//!     v  register socket with shared reactor
//! HTTP upgrade and frame I/O (reactor thread)
//!     |
//!     v  spawn actor after upgrade
//! ws_connection_entry (actor coroutine on scheduler worker)
//!     |
//!     +-- call on_connect (accept/reject)
//!     +-- attach bounded reactor event sink
//!     +-- actor_message_loop (receive -> dispatch)
//!     +-- cleanup (rooms, close frame, connection handle)
//! ```

use std::collections::VecDeque;
use std::net::TcpListener;
use std::sync::Arc;

use parking_lot::Mutex;
use rustls::{ServerConfig, ServerConnection, StreamOwned};

use super::close::WsCloseCode;
use super::frame::WsOpcode;
use super::reactor::{
    register_server, ReactorConfig, ReactorConnection, ReactorEvent, ReactorEventSink,
    ReactorTransport, ServerHandshakeHandler, SinkError,
};
use crate::actor::process::Process;
use crate::actor::stack;
use crate::actor::{
    global_scheduler, MailboxPushError, Message, MessageBuffer, ProcessId, ProcessState,
};
use crate::string::MeshString;

// ---------------------------------------------------------------------------
// Reserved type tags for WebSocket mailbox messages
// ---------------------------------------------------------------------------

/// Reserved type tag for WebSocket text frames.
pub const WS_TEXT_TAG: u64 = u64::MAX - 1;

/// Reserved type tag for WebSocket binary frames.
pub const WS_BINARY_TAG: u64 = u64::MAX - 2;

/// Reserved type tag for WebSocket disconnect (close/error from client).
pub const WS_DISCONNECT_TAG: u64 = u64::MAX - 3;

/// Reserved type tag for WebSocket connect notification.
pub const WS_CONNECT_TAG: u64 = u64::MAX - 4;

/// WebSocket close code 1008 (Policy Violation) for on_connect rejection.
const WS_POLICY_VIOLATION: u16 = 1008;

// ---------------------------------------------------------------------------
// Handler and connection structs
// ---------------------------------------------------------------------------

/// WebSocket handler containing three Mesh closure pairs (on_connect,
/// on_message, on_close). Each closure is a `{fn_ptr, env_ptr}` pair.
///
/// Passed from the Mesh-compiled program to `mesh_ws_serve`. The struct
/// is `#[repr(C)]` so Mesh's codegen can construct it directly.
#[repr(C)]
struct WsHandler {
    on_connect_fn: *mut u8,
    on_connect_env: *mut u8,
    on_message_fn: *mut u8,
    on_message_env: *mut u8,
    on_close_fn: *mut u8,
    on_close_env: *mut u8,
}

// WsHandler contains raw function pointers transferred between threads.
// The pointers are to compiled Mesh functions which are valid for the
// lifetime of the program.
unsafe impl Send for WsHandler {}

/// Connection handle for `Ws.send` -- stored on the Rust heap (not GC heap)
/// and backed by a bounded command path to the shared reactor.
pub(crate) struct WsConnection {
    pub(crate) io: ReactorConnection,
}

/// Arguments passed to the spawned WebSocket actor, following the HTTP
/// server's `ConnectionArgs` pattern.
#[repr(C)]
struct WsConnectionArgs {
    handler: WsHandler,
    connection: ReactorConnection,
    sink: Arc<ServerSink>,
    path: String,
    headers: Vec<(String, String)>,
}

// WsConnectionArgs contains raw pointers but is only used for transfer
// to the actor entry function.
unsafe impl Send for WsConnectionArgs {}

const SERVER_PENDING_ITEMS: usize = 256;
const SERVER_PENDING_BYTES: usize = 16 * 1024 * 1024;
const SERVER_MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

struct ServerSinkState {
    target: Option<(Arc<Mutex<Process>>, ProcessId)>,
    pending: VecDeque<ReactorEvent>,
    pending_bytes: usize,
    remote_close: bool,
    terminated: bool,
}

struct ServerSink {
    state: Mutex<ServerSinkState>,
}

impl ServerSink {
    fn new() -> Self {
        Self {
            state: Mutex::new(ServerSinkState {
                target: None,
                pending: VecDeque::new(),
                pending_bytes: 0,
                remote_close: false,
                terminated: false,
            }),
        }
    }

    fn attach(&self, process: Arc<Mutex<Process>>, pid: ProcessId) -> Result<(), SinkError> {
        let (terminated, remote_close) = {
            let mut state = self.state.lock();
            while let Some(event) = state.pending.pop_front() {
                state.pending_bytes -= reactor_event_bytes(&event);
                deliver_server_event(&process, pid, event)?;
            }
            state.target = Some((Arc::clone(&process), pid));
            (state.terminated, state.remote_close)
        };
        if terminated && !remote_close {
            push_disconnect(&process, pid, 1006, "WebSocket transport closed");
        }
        Ok(())
    }
}

impl ReactorEventSink for ServerSink {
    fn event(&self, event: ReactorEvent) -> Result<(), SinkError> {
        let mut state = self.state.lock();
        if state.terminated {
            return Err(SinkError::Closed);
        }
        if let Some((process, pid)) = &state.target {
            let process = Arc::clone(process);
            let pid = *pid;
            if matches!(&event, ReactorEvent::Close(_, _)) {
                deliver_server_event(&process, pid, event)?;
                state.remote_close = true;
                return Ok(());
            }
            drop(state);
            return deliver_server_event(&process, pid, event);
        }

        let is_close = matches!(&event, ReactorEvent::Close(_, _));
        let bytes = reactor_event_bytes(&event);
        if !is_close
            && (state.pending.len() >= SERVER_PENDING_ITEMS
                || state
                    .pending_bytes
                    .checked_add(bytes)
                    .is_none_or(|total| total > SERVER_PENDING_BYTES))
        {
            return Err(SinkError::Full);
        }
        state.pending_bytes += bytes;
        state.pending.push_back(event);
        state.remote_close |= is_close;
        Ok(())
    }

    fn terminated(&self, reason: &str) {
        let target = {
            let mut state = self.state.lock();
            if state.terminated {
                return;
            }
            state.terminated = true;
            (!state.remote_close)
                .then(|| state.target.as_ref())
                .flatten()
                .map(|(process, pid)| (Arc::clone(process), *pid))
        };
        if let Some((process, pid)) = target {
            push_disconnect(&process, pid, 1006, reason);
        }
    }
}

fn reactor_event_bytes(event: &ReactorEvent) -> usize {
    match event {
        ReactorEvent::Text(bytes, _) | ReactorEvent::Binary(bytes, _) => bytes.len(),
        ReactorEvent::Close(_, reason) => reason.len(),
    }
}

fn deliver_server_event(
    process: &Arc<Mutex<Process>>,
    pid: ProcessId,
    event: ReactorEvent,
) -> Result<(), SinkError> {
    let (tag, payload, _permit) = match event {
        ReactorEvent::Text(payload, permit) => (WS_TEXT_TAG, payload, permit),
        ReactorEvent::Binary(payload, permit) => (WS_BINARY_TAG, payload, permit),
        ReactorEvent::Close(code, reason) => {
            push_disconnect(process, pid, code, &reason);
            return Ok(());
        }
    };
    let message = Message {
        buffer: MessageBuffer::new(payload, tag),
    };
    push_actor_message(process, pid, message).map_err(|error| match error {
        MailboxPushError::Full => SinkError::Full,
        MailboxPushError::MessageTooLarge => SinkError::TooLarge,
    })
}

// ---------------------------------------------------------------------------
// Public API: mesh_ws_serve, mesh_ws_send, mesh_ws_send_binary
// ---------------------------------------------------------------------------

/// Start a WebSocket server on the given port and return after spawning its accept loop.
///
/// Binds a TCP listener and registers each accepted connection with the shared
/// reactor. After the upgrade, each connection actor runs lifecycle callbacks
/// (on_connect, on_message, on_close).
///
/// # Arguments
///
/// Six function/env pointer pairs for the three callbacks, plus the port:
/// - `on_connect_fn/env`: Called after handshake with (conn, path, headers)
/// - `on_message_fn/env`: Called for each text/binary frame with (conn, msg)
/// - `on_close_fn/env`: Called when connection ends with (conn, code, reason)
/// - `port`: TCP port to listen on
#[no_mangle]
pub extern "C" fn mesh_ws_serve(
    on_connect_fn: *mut u8,
    on_connect_env: *mut u8,
    on_message_fn: *mut u8,
    on_message_env: *mut u8,
    on_close_fn: *mut u8,
    on_close_env: *mut u8,
    port: i64,
) {
    // Ensure the actor scheduler is initialized (idempotent).
    crate::actor::mesh_rt_init_actor(0);

    let addr = format!("0.0.0.0:{}", port);
    let listener = match TcpListener::bind(&addr) {
        Ok(l) => l,
        Err(e) => {
            eprintln!(
                "[mesh-rt] Failed to start WebSocket server on {}: {}",
                addr, e
            );
            return;
        }
    };

    eprintln!("[mesh-rt] WebSocket server listening on {}", addr);

    // Wrap raw pointers for Send (function pointers are valid for program lifetime).
    let handler = SendableHandler {
        on_connect_fn,
        on_connect_env,
        on_message_fn,
        on_message_env,
        on_close_fn,
        on_close_env,
    };

    // Spawn an OS thread for the accept loop so Ws.serve returns immediately.
    // This allows calling Ws.serve before HTTP.serve in the same function
    // without blocking (both are blocking accept loops).
    std::thread::Builder::new()
        .name(format!("ws-accept-{}", port))
        .spawn(move || {
            ws_accept_loop(listener, handler);
        })
        .expect("Failed to spawn WebSocket accept thread");
}

/// Wrapper for raw callback pointers to satisfy Send requirement.
/// Safe because these are function pointers (or null) that remain valid
/// for the entire program lifetime.
#[derive(Clone, Copy)]
struct SendableHandler {
    on_connect_fn: *mut u8,
    on_connect_env: *mut u8,
    on_message_fn: *mut u8,
    on_message_env: *mut u8,
    on_close_fn: *mut u8,
    on_close_env: *mut u8,
}
unsafe impl Send for SendableHandler {}
unsafe impl Sync for SendableHandler {}

impl SendableHandler {
    fn actor_handler(self) -> WsHandler {
        WsHandler {
            on_connect_fn: self.on_connect_fn,
            on_connect_env: self.on_connect_env,
            on_message_fn: self.on_message_fn,
            on_message_env: self.on_message_env,
            on_close_fn: self.on_close_fn,
            on_close_env: self.on_close_env,
        }
    }
}

struct ServerOpenHandler {
    callbacks: SendableHandler,
}

impl ServerHandshakeHandler for ServerOpenHandler {
    fn opened(
        &self,
        connection: ReactorConnection,
        path: String,
        headers: Vec<(String, String)>,
    ) -> Arc<dyn ReactorEventSink> {
        let sink = Arc::new(ServerSink::new());
        let args = WsConnectionArgs {
            handler: self.callbacks.actor_handler(),
            connection,
            sink: Arc::clone(&sink),
            path,
            headers,
        };
        let args_ptr = Box::into_raw(Box::new(args)) as *const u8;
        global_scheduler().spawn(
            ws_connection_entry as *const u8,
            args_ptr,
            std::mem::size_of::<WsConnectionArgs>() as u64,
            1,
        );
        sink
    }

    fn failed(&self, reason: &str) {
        eprintln!("[mesh-rt] WebSocket upgrade failed: {reason}");
    }
}

/// Accept loop for WebSocket connections. Runs on a dedicated OS thread,
/// dispatching each accepted connection to an actor on the Mesh scheduler.
fn ws_accept_loop(listener: TcpListener, callbacks: SendableHandler) {
    let handler: Arc<dyn ServerHandshakeHandler> = Arc::new(ServerOpenHandler { callbacks });
    for tcp_stream in listener.incoming() {
        let tcp_stream = match tcp_stream {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[mesh-rt] accept error: {}", e);
                continue;
            }
        };

        let _ = tcp_stream.set_nodelay(true);
        let transport = match ReactorTransport::plain(tcp_stream) {
            Ok(transport) => transport,
            Err(error) => {
                eprintln!("[mesh-rt] prepare WebSocket socket: {error}");
                continue;
            }
        };
        if let Err(error) = register_server(
            transport,
            Arc::clone(&handler),
            ReactorConfig::server(SERVER_MAX_MESSAGE_BYTES),
        ) {
            eprintln!("[mesh-rt] register WebSocket connection: {error}");
        }
    }
}

/// Start a WebSocket TLS server on the given port, blocking the calling thread.
///
/// Same as `mesh_ws_serve` but wraps each connection in TLS via rustls.
/// Certificate and private key are loaded from PEM files at the given paths.
#[no_mangle]
pub extern "C" fn mesh_ws_serve_tls(
    on_connect_fn: *mut u8,
    on_connect_env: *mut u8,
    on_message_fn: *mut u8,
    on_message_env: *mut u8,
    on_close_fn: *mut u8,
    on_close_env: *mut u8,
    port: i64,
    cert_path: *const MeshString,
    key_path: *const MeshString,
) {
    if cert_path.is_null() || key_path.is_null() {
        eprintln!("[mesh-rt] WebSocket TLS certificate and key paths must not be null");
        return;
    }
    crate::actor::mesh_rt_init_actor(0);

    let cert_str = unsafe { (*cert_path).as_str() };
    let key_str = unsafe { (*key_path).as_str() };

    let tls_config = match crate::http::server::build_server_config(cert_str, key_str) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[mesh-rt] Failed to load TLS certificates: {}", e);
            return;
        }
    };

    let addr = format!("0.0.0.0:{}", port);
    let listener = match TcpListener::bind(&addr) {
        Ok(l) => l,
        Err(e) => {
            eprintln!(
                "[mesh-rt] Failed to start WebSocket TLS server on {}: {}",
                addr, e
            );
            return;
        }
    };

    eprintln!("[mesh-rt] WebSocket TLS server listening on {}", addr);
    let callbacks = SendableHandler {
        on_connect_fn,
        on_connect_env,
        on_message_fn,
        on_message_env,
        on_close_fn,
        on_close_env,
    };
    if let Err(error) = std::thread::Builder::new()
        .name(format!("wss-accept-{port}"))
        .spawn(move || ws_tls_accept_loop(listener, callbacks, tls_config))
    {
        eprintln!("[mesh-rt] Failed to spawn WebSocket TLS accept thread: {error}");
    }
}

fn ws_tls_accept_loop(
    listener: TcpListener,
    callbacks: SendableHandler,
    tls_config: Arc<ServerConfig>,
) {
    let handler: Arc<dyn ServerHandshakeHandler> = Arc::new(ServerOpenHandler { callbacks });
    for tcp_stream in listener.incoming() {
        let tcp_stream = match tcp_stream {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[mesh-rt] accept error: {}", e);
                continue;
            }
        };

        let _ = tcp_stream.set_nodelay(true);
        let conn = match ServerConnection::new(Arc::clone(&tls_config)) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[mesh-rt] TLS connection setup failed: {}", e);
                continue;
            }
        };
        let transport = match ReactorTransport::server_tls(StreamOwned::new(conn, tcp_stream)) {
            Ok(transport) => transport,
            Err(error) => {
                eprintln!("[mesh-rt] prepare WebSocket TLS socket: {error}");
                continue;
            }
        };
        if let Err(error) = register_server(
            transport,
            Arc::clone(&handler),
            ReactorConfig::server(SERVER_MAX_MESSAGE_BYTES),
        ) {
            eprintln!("[mesh-rt] register WebSocket TLS connection: {error}");
        }
    }
}

/// Send a text frame to a WebSocket client.
///
/// `conn` is a pointer to a `WsConnection` (obtained from the on_connect
/// callback). `msg` is a pointer to a `MeshString` containing the text.
///
/// Returns 0 on success, -1 on error.
#[no_mangle]
pub extern "C" fn mesh_ws_send(conn: *mut u8, msg: *const MeshString) -> i64 {
    if conn.is_null() || msg.is_null() {
        return -1;
    }
    let conn = unsafe { &*(conn as *const WsConnection) };
    let text = unsafe { (*msg).as_str() };
    match conn.io.send(WsOpcode::Text, text.as_bytes()) {
        Ok(()) => 0,
        Err(_) => -1,
    }
}

/// Send a binary frame to a WebSocket client.
///
/// `conn` is a pointer to a `WsConnection`. `data` and `len` specify the
/// raw bytes to send.
///
/// Returns 0 on success, -1 on error.
#[no_mangle]
pub extern "C" fn mesh_ws_send_binary(conn: *mut u8, data: *const u8, len: i64) -> i64 {
    let Some(len) = binary_payload_len(len) else {
        return -1;
    };
    if conn.is_null() || data.is_null() {
        return -1;
    }
    let conn = unsafe { &*(conn as *const WsConnection) };
    let bytes = unsafe { std::slice::from_raw_parts(data, len) };
    match conn.io.send(WsOpcode::Binary, bytes) {
        Ok(()) => 0,
        Err(_) => -1,
    }
}

fn binary_payload_len(len: i64) -> Option<usize> {
    usize::try_from(len)
        .ok()
        .filter(|len| *len <= SERVER_MAX_MESSAGE_BYTES)
}

// ---------------------------------------------------------------------------
// Actor entry point
// ---------------------------------------------------------------------------

fn close_or_cancel(connection: &ReactorConnection, code: u16, reason: &str) {
    if let Err(error) = connection.graceful_close(code, reason) {
        connection.cancel(error);
    }
}

/// Actor entry function for a single WebSocket connection.
///
/// Attaches the reactor event sink, runs the callback loop, and handles
/// cleanup on exit or crash. It performs no socket I/O.
extern "C" fn ws_connection_entry(args: *const u8) {
    if args.is_null() {
        return;
    }

    let args = unsafe { Box::from_raw(args as *mut WsConnectionArgs) };
    let WsConnectionArgs {
        handler,
        connection,
        sink,
        path,
        headers,
    } = *args;
    let conn = Box::into_raw(Box::new(WsConnection {
        io: connection.clone(),
    }));
    let conn_ptr = conn as *mut u8;

    let Some(my_pid) = stack::get_current_pid() else {
        connection.cancel("WebSocket actor has no process ID");
        unsafe {
            drop(Box::from_raw(conn));
        }
        return;
    };
    let sched = global_scheduler();
    let Some(proc_arc) = sched.get_process(my_pid) else {
        connection.cancel("WebSocket actor process is unavailable");
        unsafe {
            drop(Box::from_raw(conn));
        }
        return;
    };

    let accepted = call_on_connect(&handler, conn_ptr, &path, &headers);
    if !accepted {
        crate::ws::rooms::global_room_registry().cleanup_connection(conn as usize);
        close_or_cancel(&connection, WS_POLICY_VIOLATION, "rejected");
        unsafe {
            drop(Box::from_raw(conn));
        }
        return;
    }

    if sink.attach(proc_arc, my_pid).is_err() {
        crate::ws::rooms::global_room_registry().cleanup_connection(conn as usize);
        close_or_cancel(
            &connection,
            WsCloseCode::TRY_AGAIN_LATER,
            "inbound queue full",
        );
        unsafe {
            drop(Box::from_raw(conn));
        }
        return;
    }

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        actor_message_loop(&handler, conn_ptr)
    }));

    crate::ws::rooms::global_room_registry().cleanup_connection(conn as usize);

    match result {
        Err(_) => {
            close_or_cancel(&connection, WsCloseCode::INTERNAL_ERROR, "internal error");
            call_on_close(
                &handler,
                conn_ptr,
                WsCloseCode::INTERNAL_ERROR,
                "internal error",
            );
        }
        Ok((code, reason)) => {
            if !connection.is_closed() {
                if code == 1006 {
                    connection.cancel(reason.clone());
                } else {
                    close_or_cancel(&connection, code, &reason);
                }
            }
            call_on_close(&handler, conn_ptr, code, &reason);
        }
    }

    crate::ws::rooms::global_room_registry().cleanup_connection(conn as usize);
    unsafe {
        drop(Box::from_raw(conn));
    }
}

fn push_actor_message(
    proc_arc: &Arc<Mutex<Process>>,
    actor_pid: ProcessId,
    message: Message,
) -> Result<(), MailboxPushError> {
    let mut proc = proc_arc.lock();
    proc.mailbox.try_push(message)?;
    if matches!(proc.state, ProcessState::Waiting) && proc.set_live_state(ProcessState::Ready) {
        drop(proc);
        global_scheduler().wake_process(actor_pid);
    }
    Ok(())
}

/// Push a WS_DISCONNECT_TAG message to the actor's mailbox and wake it.
fn push_disconnect(proc_arc: &Arc<Mutex<Process>>, actor_pid: ProcessId, code: u16, reason: &str) {
    let mut proc = proc_arc.lock();
    let mut payload = Vec::with_capacity(2 + reason.len());
    payload.extend_from_slice(&code.to_be_bytes());
    payload.extend_from_slice(reason.as_bytes());
    let buffer = MessageBuffer::new(payload, WS_DISCONNECT_TAG);
    if proc.mailbox.try_push_control(Message { buffer }).is_err() {
        return;
    }
    if matches!(proc.state, ProcessState::Waiting) {
        if proc.set_live_state(ProcessState::Ready) {
            drop(proc);
            let sched = global_scheduler();
            sched.wake_process(actor_pid);
        }
    }
}

// ---------------------------------------------------------------------------
// Actor message loop
// ---------------------------------------------------------------------------

/// Main message loop for the WebSocket actor.
///
/// Blocks on `mesh_actor_receive(-1)` to get messages from the mailbox.
/// Dispatches based on the type tag:
/// - `WS_TEXT_TAG` / `WS_BINARY_TAG`: call on_message callback
/// - `WS_DISCONNECT_TAG`: client disconnected, exit loop
/// - `EXIT_SIGNAL_TAG`: exit signal from linked actor, exit loop
/// - Other: regular actor-to-actor message (ignored for now)
fn actor_message_loop(handler: &WsHandler, conn_ptr: *mut u8) -> (u16, String) {
    use crate::actor::mesh_actor_receive;

    loop {
        let msg_ptr = mesh_actor_receive(-1);
        if msg_ptr.is_null() {
            return (1006, "WebSocket actor receive stopped".to_string());
        }

        // Read type_tag from heap layout: [u64 type_tag, u64 data_len, u8... data]
        let type_tag = unsafe {
            let mut tag_bytes = [0u8; 8];
            std::ptr::copy_nonoverlapping(msg_ptr, tag_bytes.as_mut_ptr(), 8);
            u64::from_le_bytes(tag_bytes)
        };

        match type_tag {
            WS_TEXT_TAG | WS_BINARY_TAG => {
                // Read data_len and data pointer
                let (data_len, data_ptr) = unsafe {
                    let mut len_bytes = [0u8; 8];
                    std::ptr::copy_nonoverlapping(msg_ptr.add(8), len_bytes.as_mut_ptr(), 8);
                    let len = u64::from_le_bytes(len_bytes) as usize;
                    (len, msg_ptr.add(16))
                };
                // Call on_message (LIFE-03)
                call_on_message(
                    handler,
                    conn_ptr,
                    data_ptr,
                    data_len,
                    type_tag == WS_TEXT_TAG,
                );
            }
            WS_DISCONNECT_TAG => {
                // Client disconnected (ACTOR-06)
                let payload = unsafe {
                    let mut len_bytes = [0u8; 8];
                    std::ptr::copy_nonoverlapping(msg_ptr.add(8), len_bytes.as_mut_ptr(), 8);
                    let len = u64::from_le_bytes(len_bytes) as usize;
                    std::slice::from_raw_parts(msg_ptr.add(16), len)
                };
                return decode_disconnect(payload);
            }
            tag if tag == crate::actor::EXIT_SIGNAL_TAG => {
                // Exit signal from linked actor
                return (
                    WsCloseCode::GOING_AWAY,
                    "WebSocket actor exited".to_string(),
                );
            }
            _ => {
                // Regular actor-to-actor message -- ignore for now
            }
        }
    }
}

fn decode_disconnect(payload: &[u8]) -> (u16, String) {
    let Some(code) = payload
        .get(..2)
        .map(|bytes| u16::from_be_bytes([bytes[0], bytes[1]]))
    else {
        return (1006, "WebSocket transport closed".to_string());
    };
    (code, String::from_utf8_lossy(&payload[2..]).into_owned())
}

// ---------------------------------------------------------------------------
// Callback invocation helpers
// ---------------------------------------------------------------------------

/// Call the on_connect callback.
///
/// Builds Mesh-level path string and headers map, invokes the callback.
/// Returns `true` if the connection is accepted, `false` if rejected.
///
/// If no on_connect callback is set (null fn pointer), accepts by default.
fn call_on_connect(
    handler: &WsHandler,
    conn_ptr: *mut u8,
    path: &str,
    headers: &[(String, String)],
) -> bool {
    if handler.on_connect_fn.is_null() {
        return true; // No callback = accept
    }

    unsafe {
        // Build Mesh-level path string
        let path_mesh = crate::string::mesh_string_new(path.as_ptr(), path.len() as u64) as *mut u8;

        // Build headers map
        let mut headers_map = crate::collections::map::mesh_map_new_typed(1);
        for (name, value) in headers {
            let key = crate::string::mesh_string_new(name.as_ptr(), name.len() as u64);
            let val = crate::string::mesh_string_new(value.as_ptr(), value.len() as u64);
            headers_map =
                crate::collections::map::mesh_map_put(headers_map, key as u64, val as u64);
        }

        // Call the closure: if env is null, bare function; if non-null, closure
        let result = if handler.on_connect_env.is_null() {
            let f: fn(*mut u8, *mut u8, *mut u8) -> *mut u8 =
                std::mem::transmute(handler.on_connect_fn);
            f(conn_ptr, path_mesh, headers_map)
        } else {
            let f: fn(*mut u8, *mut u8, *mut u8, *mut u8) -> *mut u8 =
                std::mem::transmute(handler.on_connect_fn);
            f(handler.on_connect_env, conn_ptr, path_mesh, headers_map)
        };

        // Convention: non-null = accepted, null = rejected
        !result.is_null()
    }
}

/// Call the on_message callback.
///
/// Converts the frame payload to a MeshString and invokes the callback.
fn call_on_message(
    handler: &WsHandler,
    conn_ptr: *mut u8,
    data_ptr: *const u8,
    data_len: usize,
    _is_text: bool,
) {
    if handler.on_message_fn.is_null() {
        return;
    }

    unsafe {
        // Build a Mesh string from the frame payload
        let msg_mesh = crate::string::mesh_string_new(data_ptr, data_len as u64) as *mut u8;

        if handler.on_message_env.is_null() {
            let f: fn(*mut u8, *mut u8) -> *mut u8 = std::mem::transmute(handler.on_message_fn);
            f(conn_ptr, msg_mesh);
        } else {
            let f: fn(*mut u8, *mut u8, *mut u8) -> *mut u8 =
                std::mem::transmute(handler.on_message_fn);
            f(handler.on_message_env, conn_ptr, msg_mesh);
        }
    }
}

/// Call the on_close callback.
///
/// Invoked when the connection ends (normal disconnect or crash).
fn call_on_close(handler: &WsHandler, conn_ptr: *mut u8, code: u16, reason: &str) {
    if handler.on_close_fn.is_null() {
        return;
    }

    unsafe {
        let code_i64 = code as i64;
        let reason_mesh =
            crate::string::mesh_string_new(reason.as_ptr(), reason.len() as u64) as *mut u8;

        if handler.on_close_env.is_null() {
            let f: fn(*mut u8, i64, *mut u8) -> *mut u8 = std::mem::transmute(handler.on_close_fn);
            f(conn_ptr, code_i64, reason_mesh);
        } else {
            let f: fn(*mut u8, *mut u8, i64, *mut u8) -> *mut u8 =
                std::mem::transmute(handler.on_close_fn);
            f(handler.on_close_env, conn_ptr, code_i64, reason_mesh);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor::Priority;
    use crate::ws::close::parse_close_payload;
    use crate::ws::frame::{apply_mask, read_frame, write_masked_frame, WsOpcode};
    use crate::ws::reactor::InboundPermit;
    use rustls::ClientConnection;
    use rustls_pki_types::ServerName;
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::Barrier;
    use std::time::Duration;

    // ── Callback functions ───────────────────────────────────────────

    /// on_connect with per-test counter via env pointer. Returns non-null (accept).
    extern "C" fn counting_on_connect(
        env: *mut u8,
        _conn: *mut u8,
        _path: *mut u8,
        _headers: *mut u8,
    ) -> *mut u8 {
        if !env.is_null() {
            unsafe {
                (*(env as *const AtomicU64)).fetch_add(1, Ordering::SeqCst);
            }
        }
        1 as *mut u8
    }

    /// on_connect: accept without counting (env=null calling convention).
    extern "C" fn accept_on_connect(_conn: *mut u8, _path: *mut u8, _headers: *mut u8) -> *mut u8 {
        1 as *mut u8
    }

    extern "C" fn join_then_reject_on_connect(
        conn: *mut u8,
        _path: *mut u8,
        _headers: *mut u8,
    ) -> *mut u8 {
        crate::ws::rooms::global_room_registry()
            .join(conn as usize, "server-reject-cleanup".to_string());
        std::ptr::null_mut()
    }

    /// on_message: echo the message back to the client (env=null).
    extern "C" fn echo_on_message(conn: *mut u8, msg: *mut u8) -> *mut u8 {
        mesh_ws_send(conn, msg as *const MeshString);
        std::ptr::null_mut()
    }

    /// on_message: always panic to test crash isolation (env=null).
    /// NOT extern "C" -- Rust ABI allows panic to unwind through catch_unwind.
    /// (extern "C" panics abort the process since Rust 1.71.)
    fn crash_on_message(_conn: *mut u8, _msg: *mut u8) -> *mut u8 {
        panic!("intentional test crash");
    }

    /// on_close with per-test counter via env pointer.
    extern "C" fn counting_on_close(
        env: *mut u8,
        _conn: *mut u8,
        _code: i64,
        _reason: *mut u8,
    ) -> *mut u8 {
        if !env.is_null() {
            unsafe {
                (*(env as *const AtomicU64)).fetch_add(1, Ordering::SeqCst);
            }
        }
        std::ptr::null_mut()
    }

    struct CloseRecord {
        code: AtomicU64,
        reason: Mutex<String>,
    }

    extern "C" fn recording_on_close(
        env: *mut u8,
        _conn: *mut u8,
        code: i64,
        reason: *mut u8,
    ) -> *mut u8 {
        let record = unsafe { &*(env as *const CloseRecord) };
        record.code.store(code as u64, Ordering::SeqCst);
        *record.reason.lock() = unsafe { (*(reason as *const MeshString)).as_str().to_string() };
        std::ptr::null_mut()
    }

    const REJOIN_ON_CLOSE_ROOM: &str = "server-close-cleanup";

    extern "C" fn rejoining_on_close(
        env: *mut u8,
        conn: *mut u8,
        _code: i64,
        _reason: *mut u8,
    ) -> *mut u8 {
        crate::ws::rooms::global_room_registry()
            .join(conn as usize, REJOIN_ON_CLOSE_ROOM.to_string());
        unsafe { &*(env as *const AtomicBool) }.store(true, Ordering::SeqCst);
        std::ptr::null_mut()
    }

    /// on_close: no-op (env=null calling convention).
    extern "C" fn noop_on_close(_conn: *mut u8, _code: i64, _reason: *mut u8) -> *mut u8 {
        std::ptr::null_mut()
    }

    extern "C" fn blocking_on_message(env: *mut u8, _conn: *mut u8, _msg: *mut u8) -> *mut u8 {
        let blocked = unsafe { &*(env as *const AtomicBool) };
        while blocked.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(1));
        }
        std::ptr::null_mut()
    }

    // ── Helpers ──────────────────────────────────────────────────────

    /// Get a free port by binding to port 0 and releasing.
    fn free_port() -> u16 {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    }

    /// Start a WS server that echoes messages back (no per-test counters).
    fn start_echo_server(port: u16) {
        std::thread::spawn(move || {
            mesh_ws_serve(
                accept_on_connect as *mut u8,
                std::ptr::null_mut(),
                echo_on_message as *mut u8,
                std::ptr::null_mut(),
                noop_on_close as *mut u8,
                std::ptr::null_mut(),
                port as i64,
            );
        });
        std::thread::sleep(Duration::from_millis(200));
    }

    fn start_rejecting_server(port: u16) {
        std::thread::spawn(move || {
            mesh_ws_serve(
                join_then_reject_on_connect as *mut u8,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                port as i64,
            );
        });
        std::thread::sleep(Duration::from_millis(200));
    }

    /// Start a WS server with per-test connect/close counters.
    fn start_counting_server(
        port: u16,
        connect_ctr: &'static AtomicU64,
        close_ctr: &'static AtomicU64,
    ) {
        // Cast to usize to cross thread boundary (*mut u8 is !Send).
        let connect_env = connect_ctr as *const AtomicU64 as usize;
        let close_env = close_ctr as *const AtomicU64 as usize;
        std::thread::spawn(move || {
            mesh_ws_serve(
                counting_on_connect as *mut u8,
                connect_env as *mut u8,
                echo_on_message as *mut u8,
                std::ptr::null_mut(),
                counting_on_close as *mut u8,
                close_env as *mut u8,
                port as i64,
            );
        });
        std::thread::sleep(Duration::from_millis(200));
    }

    fn start_close_recording_server(port: u16, record: &'static CloseRecord) {
        let close_env = record as *const CloseRecord as usize;
        std::thread::spawn(move || {
            mesh_ws_serve(
                accept_on_connect as *mut u8,
                std::ptr::null_mut(),
                echo_on_message as *mut u8,
                std::ptr::null_mut(),
                recording_on_close as *mut u8,
                close_env as *mut u8,
                port as i64,
            );
        });
        std::thread::sleep(Duration::from_millis(200));
    }

    /// Start a WS server where on_message always panics.
    fn start_crash_server(port: u16) {
        std::thread::spawn(move || {
            mesh_ws_serve(
                accept_on_connect as *mut u8,
                std::ptr::null_mut(),
                crash_on_message as *mut u8,
                std::ptr::null_mut(),
                noop_on_close as *mut u8,
                std::ptr::null_mut(),
                port as i64,
            );
        });
        std::thread::sleep(Duration::from_millis(200));
    }

    fn start_blocked_server(port: u16, blocked: &'static AtomicBool) {
        let blocked_env = blocked as *const AtomicBool as usize;
        std::thread::spawn(move || {
            mesh_ws_serve(
                accept_on_connect as *mut u8,
                std::ptr::null_mut(),
                blocking_on_message as *mut u8,
                blocked_env as *mut u8,
                noop_on_close as *mut u8,
                std::ptr::null_mut(),
                port as i64,
            );
        });
        std::thread::sleep(Duration::from_millis(200));
    }

    /// Connect to a WS server and complete the HTTP upgrade handshake.
    /// Reads the HTTP response byte-by-byte to avoid consuming frame data.
    fn ws_connect(port: u16) -> TcpStream {
        let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();

        let key = "dGhlIHNhbXBsZSBub25jZQ==";
        write!(
            stream,
            "GET /ws HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
        ).unwrap();
        stream.flush().unwrap();

        // Read HTTP response byte-by-byte until \r\n\r\n to avoid
        // consuming any WebSocket frame bytes that follow.
        let mut resp = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            stream.read_exact(&mut byte).unwrap();
            resp.push(byte[0]);
            if resp.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        let resp_str = String::from_utf8_lossy(&resp);
        assert!(
            resp_str.contains("101"),
            "Expected 101 Switching Protocols, got: {}",
            resp_str
        );
        stream
    }

    /// Send a masked text frame (client-to-server must be masked per RFC 6455).
    fn ws_send_text(stream: &mut TcpStream, text: &str) {
        let mask_key = [0x12, 0x34, 0x56, 0x78];
        let mut payload = text.as_bytes().to_vec();
        apply_mask(&mut payload, &mask_key);

        let len = text.len();
        let mut frame = vec![0x81u8]; // FIN=1, opcode=Text
        if len <= 125 {
            frame.push(0x80 | len as u8); // MASK=1
        } else {
            frame.push(0xFE); // MASK=1, 126
            frame.extend_from_slice(&(len as u16).to_be_bytes());
        }
        frame.extend_from_slice(&mask_key);
        frame.extend_from_slice(&payload);

        stream.write_all(&frame).unwrap();
        stream.flush().unwrap();
    }

    /// Send a masked close frame with the given status code.
    fn ws_send_close(stream: &mut TcpStream, code: u16) {
        ws_send_close_reason(stream, code, "");
    }

    fn ws_send_close_reason(stream: &mut TcpStream, code: u16, reason: &str) {
        let mask_key = [0xAA, 0xBB, 0xCC, 0xDD];
        let mut payload = code.to_be_bytes().to_vec();
        payload.extend_from_slice(reason.as_bytes());
        apply_mask(&mut payload, &mask_key);

        let mut frame = vec![0x88u8, 0x80 | payload.len() as u8];
        frame.extend_from_slice(&mask_key);
        frame.extend_from_slice(&payload);

        stream.write_all(&frame).unwrap();
        stream.flush().unwrap();
    }

    // ── Tests ────────────────────────────────────────────────────────

    #[test]
    fn binary_payload_length_rejects_negative_and_oversized_values() {
        assert_eq!(binary_payload_len(-1), None);
        assert_eq!(
            binary_payload_len(SERVER_MAX_MESSAGE_BYTES as i64 + 1),
            None
        );
        assert_eq!(binary_payload_len(0), Some(0));
        assert_eq!(
            binary_payload_len(SERVER_MAX_MESSAGE_BYTES as i64),
            Some(SERVER_MAX_MESSAGE_BYTES)
        );
    }

    #[test]
    fn tls_server_rejects_null_certificate_paths() {
        mesh_ws_serve_tls(
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
            std::ptr::null(),
            std::ptr::null(),
        );
    }

    #[test]
    fn attach_keeps_target_unpublished_until_pending_delivery_finishes() {
        let sink = Arc::new(ServerSink::new());
        sink.event(ReactorEvent::Text(
            b"first".to_vec(),
            InboundPermit::reserve(5).unwrap(),
        ))
        .unwrap();
        let process = Arc::new(Mutex::new(Process::new(
            ProcessId(99_001),
            Priority::Normal,
        )));
        let process_guard = process.lock();
        let start = Arc::new(Barrier::new(2));
        let attach_sink = Arc::clone(&sink);
        let attach_process = Arc::clone(&process);
        let attach_start = Arc::clone(&start);
        let attach = std::thread::spawn(move || {
            attach_start.wait();
            attach_sink.attach(attach_process, ProcessId(99_001))
        });

        start.wait();
        std::thread::sleep(Duration::from_millis(50));
        assert!(
            sink.state.try_lock().is_none(),
            "attach must serialize target publication with pending delivery"
        );

        drop(process_guard);
        assert!(attach.join().unwrap().is_ok());
    }

    #[test]
    fn terminal_close_uses_reserved_pending_slot_after_data_limit() {
        let sink = ServerSink::new();
        for _ in 0..SERVER_PENDING_ITEMS {
            sink.event(ReactorEvent::Text(
                Vec::new(),
                InboundPermit::reserve(0).unwrap(),
            ))
            .unwrap();
        }
        sink.event(ReactorEvent::Close(1001, "leaving".to_string()))
            .unwrap();

        let process = Arc::new(Mutex::new(Process::new(
            ProcessId(99_002),
            Priority::Normal,
        )));
        sink.attach(Arc::clone(&process), ProcessId(99_002))
            .unwrap();
        let process = process.lock();
        for _ in 0..SERVER_PENDING_ITEMS {
            assert_eq!(process.mailbox.pop().unwrap().buffer.type_tag, WS_TEXT_TAG);
        }
        let close = process.mailbox.pop().unwrap().buffer;
        assert_eq!(close.type_tag, WS_DISCONNECT_TAG);
        assert_eq!(
            decode_disconnect(&close.data),
            (1001, "leaving".to_string())
        );
    }

    #[test]
    fn rejected_connection_is_removed_from_rooms_before_drop() {
        let port = free_port();
        start_rejecting_server(port);
        let mut stream = ws_connect(port);
        let close = read_frame(&mut stream).unwrap();
        assert_eq!(parse_close_payload(&close.payload).0, WS_POLICY_VIOLATION);

        for _ in 0..50 {
            if crate::ws::rooms::global_room_registry()
                .members("server-reject-cleanup")
                .is_empty()
            {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("rejected connection remained registered in a room");
    }

    #[test]
    fn on_close_cannot_leave_a_dangling_room_member() {
        let port = free_port();
        let called = Box::leak(Box::new(AtomicBool::new(false)));
        let close_env = called as *const AtomicBool as usize;
        std::thread::spawn(move || {
            mesh_ws_serve(
                accept_on_connect as *mut u8,
                std::ptr::null_mut(),
                echo_on_message as *mut u8,
                std::ptr::null_mut(),
                rejoining_on_close as *mut u8,
                close_env as *mut u8,
                port as i64,
            );
        });
        std::thread::sleep(Duration::from_millis(200));
        let mut stream = ws_connect(port);
        ws_send_close(&mut stream, 1000);
        let _ = read_frame(&mut stream).unwrap();

        for _ in 0..100 {
            if called.load(Ordering::SeqCst) {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(called.load(Ordering::SeqCst));
        for _ in 0..50 {
            if crate::ws::rooms::global_room_registry()
                .members(REJOIN_ON_CLOSE_ROOM)
                .is_empty()
            {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("on_close reinserted a freed connection into a room");
    }

    /// End-to-end: connect, send text, get echo, close cleanly.
    #[test]
    fn test_ws_server_end_to_end_echo() {
        let port = free_port();
        start_echo_server(port);

        let mut stream = ws_connect(port);

        // Send text, expect echo back
        ws_send_text(&mut stream, "Hello WebSocket");
        let frame = read_frame(&mut stream).unwrap();
        assert_eq!(frame.opcode, WsOpcode::Text);
        assert_eq!(String::from_utf8_lossy(&frame.payload), "Hello WebSocket");

        // Clean close handshake
        ws_send_close(&mut stream, 1000);
        let close = read_frame(&mut stream).unwrap();
        assert_eq!(close.opcode, WsOpcode::Close);
        let (code, _) = parse_close_payload(&close.payload);
        assert_eq!(code, 1000);
    }

    #[test]
    fn server_tls_reactor_echoes_a_large_message() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        crate::actor::mesh_rt_init_actor(0);
        let (server_config, client_config) = crate::dist::node::ws_test_tls_configs();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (tcp, _) = listener.accept().unwrap();
            let connection = ServerConnection::new(server_config).unwrap();
            let transport =
                ReactorTransport::server_tls(StreamOwned::new(connection, tcp)).unwrap();
            let handler: Arc<dyn ServerHandshakeHandler> = Arc::new(ServerOpenHandler {
                callbacks: SendableHandler {
                    on_connect_fn: accept_on_connect as *mut u8,
                    on_connect_env: std::ptr::null_mut(),
                    on_message_fn: echo_on_message as *mut u8,
                    on_message_env: std::ptr::null_mut(),
                    on_close_fn: noop_on_close as *mut u8,
                    on_close_env: std::ptr::null_mut(),
                },
            });
            register_server(
                transport,
                handler,
                ReactorConfig::server(SERVER_MAX_MESSAGE_BYTES),
            )
            .unwrap();
        });

        let tcp = TcpStream::connect(("127.0.0.1", port)).unwrap();
        tcp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        tcp.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
        let connection = ClientConnection::new(
            client_config,
            ServerName::try_from("localhost".to_string()).unwrap(),
        )
        .unwrap();
        let mut stream = StreamOwned::new(connection, tcp);
        write!(
            stream,
            "GET /ws HTTP/1.1\r\nHost: localhost:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
        )
        .unwrap();
        stream.flush().unwrap();
        let mut response = Vec::new();
        let mut byte = [0u8; 1];
        while !response.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).unwrap();
            response.push(byte[0]);
        }
        assert!(String::from_utf8_lossy(&response).contains("101"));

        let payload = vec![b'x'; 128 * 1024];
        write_masked_frame(&mut stream, WsOpcode::Text, &payload, true, [1, 2, 3, 4]).unwrap();
        let echoed = read_frame(&mut stream).unwrap();
        assert_eq!(echoed.payload, payload);
        write_masked_frame(
            &mut stream,
            WsOpcode::Close,
            &1000u16.to_be_bytes(),
            true,
            [4, 3, 2, 1],
        )
        .unwrap();
        assert_eq!(read_frame(&mut stream).unwrap().opcode, WsOpcode::Close);
        assert_eq!(stream.read(&mut [0u8; 1]).unwrap(), 0);
        server.join().unwrap();
    }

    /// Lifecycle: on_connect fires on handshake, on_close fires on close.
    #[test]
    fn test_ws_server_lifecycle_callbacks() {
        let port = free_port();
        let connect_ctr: &'static AtomicU64 = Box::leak(Box::new(AtomicU64::new(0)));
        let close_ctr: &'static AtomicU64 = Box::leak(Box::new(AtomicU64::new(0)));
        start_counting_server(port, connect_ctr, close_ctr);

        // Before connect
        assert_eq!(connect_ctr.load(Ordering::SeqCst), 0);
        assert_eq!(close_ctr.load(Ordering::SeqCst), 0);

        let mut stream = ws_connect(port);
        for _ in 0..50 {
            if connect_ctr.load(Ordering::SeqCst) >= 1 {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            connect_ctr.load(Ordering::SeqCst),
            1,
            "on_connect should fire"
        );

        // Send close -> on_close should fire
        ws_send_close(&mut stream, 1000);
        let _ = read_frame(&mut stream); // consume close echo
        for _ in 0..100 {
            if close_ctr.load(Ordering::SeqCst) >= 1 {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(close_ctr.load(Ordering::SeqCst), 1, "on_close should fire");
    }

    #[test]
    fn on_close_receives_the_remote_code_and_reason() {
        let port = free_port();
        let record = Box::leak(Box::new(CloseRecord {
            code: AtomicU64::new(0),
            reason: Mutex::new(String::new()),
        }));
        start_close_recording_server(port, record);
        let mut stream = ws_connect(port);

        ws_send_close_reason(&mut stream, 1001, "leaving");
        let _ = read_frame(&mut stream).unwrap();
        for _ in 0..100 {
            if record.code.load(Ordering::SeqCst) != 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(record.code.load(Ordering::SeqCst), 1001);
        assert_eq!(&*record.reason.lock(), "leaving");
    }

    /// Crash isolation: actor panic sends close 1011, server keeps running.
    #[test]
    fn test_ws_server_crash_sends_1011() {
        let port = free_port();
        start_crash_server(port);

        // First connection: any message triggers panic
        let mut stream = ws_connect(port);
        ws_send_text(&mut stream, "trigger crash");

        let frame = read_frame(&mut stream).unwrap();
        assert_eq!(frame.opcode, WsOpcode::Close);
        let (code, _) = parse_close_payload(&frame.payload);
        assert_eq!(code, 1011, "actor crash should send close code 1011");

        // Second connection: server should still be accepting
        std::thread::sleep(Duration::from_millis(200));
        let _stream2 = ws_connect(port); // panics if server is dead
    }

    /// Shared reactor delivers multiple rapid messages in FIFO order.
    #[test]
    fn test_ws_server_shared_reactor_delivers_messages() {
        let port = free_port();
        start_echo_server(port);

        let mut stream = ws_connect(port);

        // Send 5 messages rapidly
        for i in 0..5 {
            ws_send_text(&mut stream, &format!("msg-{}", i));
        }

        // All should be echoed back in FIFO order
        for i in 0..5 {
            let frame = read_frame(&mut stream).unwrap();
            assert_eq!(frame.opcode, WsOpcode::Text);
            assert_eq!(
                String::from_utf8_lossy(&frame.payload),
                format!("msg-{}", i),
                "messages should be delivered in FIFO order"
            );
        }
    }

    #[test]
    fn inbound_mailbox_overflow_closes_instead_of_dropping_frames() {
        let port = free_port();
        let blocked: &'static AtomicBool = Box::leak(Box::new(AtomicBool::new(true)));
        start_blocked_server(port, blocked);
        let mut stream = ws_connect(port);
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();

        let mut frames = Vec::new();
        for _ in 0..1_030 {
            let mask_key = [0x12, 0x34, 0x56, 0x78];
            let mut payload = vec![b'x'];
            apply_mask(&mut payload, &mask_key);
            frames.extend_from_slice(&[0x81, 0x81]);
            frames.extend_from_slice(&mask_key);
            frames.extend_from_slice(&payload);
        }
        stream.write_all(&frames).unwrap();
        stream.flush().unwrap();

        let close = read_frame(&mut stream).unwrap();
        blocked.store(false, Ordering::SeqCst);
        assert_eq!(close.opcode, WsOpcode::Close);
        assert_eq!(parse_close_payload(&close.payload).0, 1013);
    }

    /// Client disconnect (TCP drop) triggers on_close and server keeps running.
    #[test]
    fn test_ws_server_client_disconnect_cleanup() {
        let port = free_port();
        let connect_ctr: &'static AtomicU64 = Box::leak(Box::new(AtomicU64::new(0)));
        let close_ctr: &'static AtomicU64 = Box::leak(Box::new(AtomicU64::new(0)));
        start_counting_server(port, connect_ctr, close_ctr);

        {
            let mut stream = ws_connect(port);
            ws_send_text(&mut stream, "hello");
            let _ = read_frame(&mut stream).unwrap(); // consume echo
                                                      // stream dropped -> TCP FIN, simulating client disconnect
        }

        // Wait for the reactor to detect disconnect and on_close to fire
        std::thread::sleep(Duration::from_secs(2));
        assert!(
            close_ctr.load(Ordering::SeqCst) >= 1,
            "on_close should fire on client disconnect"
        );

        // Server should still accept new connections
        let _stream2 = ws_connect(port);
    }
}
