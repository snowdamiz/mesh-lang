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
use rustls::ServerConfig;

use super::close::WsCloseCode;
use super::frame::WsOpcode;
use super::reactor::{
    register_server, ReactorConfig, ReactorConnection, ReactorEvent, ReactorEventSink,
    ReactorTransport, ServerHandshakeHandler, SinkFull,
};
use crate::actor::process::Process;
use crate::actor::stack;
use crate::actor::{
    global_scheduler, MailboxPushError, Message, MessageBuffer, ProcessId, ProcessState,
};
use crate::callback::{call2, call3};
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
#[derive(Clone, Copy)]
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
unsafe impl Sync for WsHandler {}

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
const SERVER_MAX_MESSAGE_BYTES: usize = crate::actor::mailbox::DEFAULT_MAILBOX_MAX_BYTES;
const SERVER_PENDING_BYTES: usize = SERVER_MAX_MESSAGE_BYTES;

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

    fn attach(&self, process: Arc<Mutex<Process>>, pid: ProcessId) -> Result<(), SinkFull> {
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
    fn event(&self, event: ReactorEvent) -> Result<(), SinkFull> {
        let mut state = self.state.lock();
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
            return Err(SinkFull);
        }
        state.pending_bytes += bytes;
        state.pending.push_back(event);
        state.remote_close |= is_close;
        Ok(())
    }

    fn terminated(&self, reason: &str) {
        let target = {
            let mut state = self.state.lock();
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
) -> Result<(), SinkFull> {
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
    // A connection's actor has the default mailbox, which takes a message of
    // any size the reactor admits: a refusal is a full mailbox.
    push_actor_message(process, pid, message, false).map_err(|_| SinkFull)
}

// ---------------------------------------------------------------------------
// Public API: mesh_ws_serve, mesh_ws_serve_tls, mesh_ws_send
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
    let callbacks = WsHandler {
        on_connect_fn,
        on_connect_env,
        on_message_fn,
        on_message_env,
        on_close_fn,
        on_close_env,
    };
    ws_serve(callbacks, port, None);
}

/// Bind `port`, then accept on a thread of its own (so Ws.serve returns at
/// once, and HTTP.serve can follow it), in TLS when `tls` is given.
fn ws_serve(callbacks: WsHandler, port: i64, tls: Option<Arc<ServerConfig>>) {
    // Ensure the actor scheduler is initialized (idempotent).
    crate::actor::mesh_rt_init_actor(0);
    let kind = if tls.is_some() {
        "WebSocket TLS"
    } else {
        "WebSocket"
    };

    let addr = format!("0.0.0.0:{}", port);
    let listener = match TcpListener::bind(&addr) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[mesh-rt] Failed to start {kind} server on {addr}: {e}");
            return;
        }
    };

    eprintln!("[mesh-rt] {kind} server listening on {addr}");
    let thread = if tls.is_some() { "wss" } else { "ws" };
    if let Err(error) = std::thread::Builder::new()
        .name(format!("{thread}-accept-{port}"))
        .spawn(move || ws_accept_loop(listener, callbacks, tls))
    {
        eprintln!("[mesh-rt] Failed to spawn {kind} accept thread: {error}");
    }
}

struct ServerOpenHandler {
    callbacks: WsHandler,
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
            handler: self.callbacks,
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
/// dispatching each accepted connection (in TLS when `tls` is given) to an
/// actor on the Mesh scheduler.
fn ws_accept_loop(listener: TcpListener, callbacks: WsHandler, tls: Option<Arc<ServerConfig>>) {
    let handler: Arc<dyn ServerHandshakeHandler> = Arc::new(ServerOpenHandler { callbacks });
    for tcp_stream in listener.incoming() {
        let tcp_stream = match tcp_stream {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[mesh-rt] accept error: {}", e);
                std::thread::sleep(crate::http::server::ACCEPT_PAUSE);
                continue;
            }
        };

        let _ = tcp_stream.set_nodelay(true);
        let transport = match &tls {
            None => ReactorTransport::plain(tcp_stream),
            Some(config) => {
                ReactorTransport::tls(crate::http::server::tls_session(config), tcp_stream)
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

/// Start a WebSocket TLS server on the given port and return after spawning its accept loop.
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
    let (cert_path, key_path) = unsafe { ((*cert_path).as_str(), (*key_path).as_str()) };
    let tls = match crate::http::server::build_server_config(cert_path, key_path) {
        Ok(tls) => tls,
        Err(e) => {
            eprintln!("[mesh-rt] Failed to load TLS certificates: {}", e);
            return;
        }
    };
    let callbacks = WsHandler {
        on_connect_fn,
        on_connect_env,
        on_message_fn,
        on_message_env,
        on_close_fn,
        on_close_env,
    };
    ws_serve(callbacks, port, Some(tls));
}

/// Send a text frame to a WebSocket client.
///
/// `conn` is the connection handle the callbacks were given: a pointer to
/// its `WsConnection`. Mesh code may pass any Int, 0 among them, which is
/// refused.
///
/// Returns 0 on success, -1 on error.
#[no_mangle]
pub extern "C" fn mesh_ws_send(conn: *mut u8, msg: *const MeshString) -> i64 {
    if conn.is_null() {
        return -1;
    }
    let conn = unsafe { &*(conn as *const WsConnection) };
    let text = unsafe { (*msg).as_str() };
    match conn.io.send(WsOpcode::Text, text.as_bytes()) {
        Ok(()) => 0,
        Err(_) => -1,
    }
}

// ---------------------------------------------------------------------------
// Actor entry point
// ---------------------------------------------------------------------------

/// Actor entry function for a single WebSocket connection.
///
/// Attaches the reactor event sink, runs the callback loop, and handles
/// cleanup on exit or crash. It performs no socket I/O.
extern "C" fn ws_connection_entry(args: *const u8) {
    let WsConnectionArgs {
        handler,
        connection,
        sink,
        path,
        headers,
    } = *unsafe { Box::from_raw(args as *mut WsConnectionArgs) };
    let conn = Box::into_raw(Box::new(WsConnection {
        io: connection.clone(),
    }));
    let conn_ptr = conn as *mut u8;
    // The reactor's handler spawns this entry, and only as an actor.
    let pid = stack::get_current_pid().expect("a WebSocket connection runs as an actor");
    let process = global_scheduler()
        .get_process(pid)
        .expect("a running actor is in the process table");

    let refusal = if !call_on_connect(&handler, conn_ptr, &path, &headers) {
        Some((WS_POLICY_VIOLATION, "rejected"))
    } else if sink.attach(process, pid).is_err() {
        Some((WsCloseCode::TRY_AGAIN_LATER, "inbound queue full"))
    } else {
        None
    };
    // A close the reactor cannot take cancels the connection, and one
    // already closed takes none.
    if let Some((code, reason)) = refusal {
        let _ = connection.graceful_close(code, reason);
    } else {
        let (code, reason) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            actor_message_loop(&handler, conn_ptr)
        }))
        .unwrap_or_else(|_| (WsCloseCode::INTERNAL_ERROR, "internal error".to_string()));
        crate::ws::rooms::global_room_registry().cleanup_connection(conn as usize);
        let _ = connection.graceful_close(code, &reason);
        call_on_close(&handler, conn_ptr, code, &reason);
    }

    crate::ws::rooms::global_room_registry().cleanup_connection(conn as usize);
    unsafe {
        drop(Box::from_raw(conn));
    }
}

/// Queue `message` for the connection's actor and wake it; a disconnect,
/// `control`, takes the mailbox's slot for one.
fn push_actor_message(
    proc_arc: &Arc<Mutex<Process>>,
    actor_pid: ProcessId,
    message: Message,
    control: bool,
) -> Result<(), MailboxPushError> {
    let mut proc = proc_arc.lock();
    if control {
        proc.mailbox.try_push_control(message)?;
    } else {
        proc.mailbox.try_push(message)?;
    }
    if matches!(proc.state, ProcessState::Waiting) && proc.set_live_state(ProcessState::Ready) {
        drop(proc);
        global_scheduler().wake_process(actor_pid);
    }
    Ok(())
}

/// Push a WS_DISCONNECT_TAG message to the actor's mailbox and wake it: a
/// connection has one disconnect, which its control slot takes.
fn push_disconnect(proc_arc: &Arc<Mutex<Process>>, actor_pid: ProcessId, code: u16, reason: &str) {
    let mut payload = Vec::with_capacity(2 + reason.len());
    payload.extend_from_slice(&code.to_be_bytes());
    payload.extend_from_slice(reason.as_bytes());
    let buffer = MessageBuffer::new(payload, WS_DISCONNECT_TAG);
    let _ = push_actor_message(proc_arc, actor_pid, Message { buffer }, true);
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
        // An actor's receive without a timeout returns only a message.
        let msg_ptr = mesh_actor_receive(-1);

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
                call_on_message(handler, conn_ptr, data_ptr, data_len);
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

/// A disconnect's code and reason, as `push_disconnect` wrote them.
fn decode_disconnect(payload: &[u8]) -> (u16, String) {
    (
        u16::from_be_bytes([payload[0], payload[1]]),
        String::from_utf8_lossy(&payload[2..]).into_owned(),
    )
}

// ---------------------------------------------------------------------------
// Callback invocation helpers
// ---------------------------------------------------------------------------

/// Call the on_connect callback with the connection, its path and its
/// headers: whether it accepts the connection (a non-null result).
fn call_on_connect(
    handler: &WsHandler,
    conn_ptr: *mut u8,
    path: &str,
    headers: &[(String, String)],
) -> bool {
    let path = crate::string::mesh_str(path);
    let mut headers_map = crate::collections::map::mesh_map_new_typed(1);
    for (name, value) in headers {
        let key = crate::string::mesh_str(name);
        let val = crate::string::mesh_str(value);
        headers_map = crate::collections::map::mesh_map_put(headers_map, key as u64, val as u64);
    }
    let result = unsafe {
        call3(
            handler.on_connect_fn,
            handler.on_connect_env,
            conn_ptr as u64,
            path as u64,
            headers_map as u64,
        )
    };
    result != 0
}

/// Call the on_message callback with the message as a string.
fn call_on_message(handler: &WsHandler, conn_ptr: *mut u8, data_ptr: *const u8, data_len: usize) {
    let message = crate::string::mesh_string_new(data_ptr, data_len as u64);
    unsafe {
        call2(
            handler.on_message_fn,
            handler.on_message_env,
            conn_ptr as u64,
            message as u64,
        );
    }
}

/// Call the on_close callback: the connection ended (normal disconnect or
/// crash).
fn call_on_close(handler: &WsHandler, conn_ptr: *mut u8, code: u16, reason: &str) {
    let reason = crate::string::mesh_str(reason);
    unsafe {
        call3(
            handler.on_close_fn,
            handler.on_close_env,
            conn_ptr as u64,
            u64::from(code),
            reason as u64,
        );
    }
}

// The reactor must not admit a message its actor mailbox cannot hold, nor
// the pre-attach sink more bytes than attachment can deliver.
const _: () = assert!(SERVER_MAX_MESSAGE_BYTES <= crate::actor::mailbox::DEFAULT_MAILBOX_MAX_BYTES);
const _: () = assert!(SERVER_PENDING_BYTES <= crate::actor::mailbox::DEFAULT_MAILBOX_MAX_BYTES);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor::Priority;
    use crate::ws::close::parse_close_payload;
    use crate::ws::frame::{apply_mask, read_frame, write_masked_frame, WsOpcode};
    use crate::ws::reactor::InboundPermit;
    use rustls::{ClientConnection, ServerConnection, StreamOwned};
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
        std::ptr::dangling_mut::<u8>()
    }

    /// on_connect: accept without counting (env=null calling convention).
    extern "C" fn accept_on_connect(_conn: *mut u8, _path: *mut u8, _headers: *mut u8) -> *mut u8 {
        std::ptr::dangling_mut::<u8>()
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
    }

    fn start_rejecting_server(port: u16) {
        std::thread::spawn(move || {
            mesh_ws_serve(
                join_then_reject_on_connect as *mut u8,
                std::ptr::null_mut(),
                echo_on_message as *mut u8,
                std::ptr::null_mut(),
                noop_on_close as *mut u8,
                std::ptr::null_mut(),
                port as i64,
            );
        });
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
    }

    /// Connect to a WS server and complete the HTTP upgrade handshake.
    /// Reads the HTTP response byte-by-byte to avoid consuming frame data.
    /// A server started on a thread of its own may not listen yet (a loaded
    /// host is slow to run it): connecting is retried for a while.
    fn ws_connect(port: u16) -> TcpStream {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut stream = loop {
            match TcpStream::connect(("127.0.0.1", port)) {
                Ok(stream) => break stream,
                Err(error) if std::time::Instant::now() >= deadline => {
                    panic!("no WebSocket server on port {port}: {error}")
                }
                Err(_) => std::thread::sleep(Duration::from_millis(20)),
            }
        };
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

    /// Callbacks without environments.
    fn callbacks(on_connect: *mut u8, on_message: *mut u8, on_close: *mut u8) -> WsHandler {
        WsHandler {
            on_connect_fn: on_connect,
            on_connect_env: std::ptr::null_mut(),
            on_message_fn: on_message,
            on_message_env: std::ptr::null_mut(),
            on_close_fn: on_close,
            on_close_env: std::ptr::null_mut(),
        }
    }

    /// One connection to `handler` on the shared reactor: the raw client,
    /// upgraded.
    fn connect_to(handler: impl ServerHandshakeHandler + 'static) -> TcpStream {
        crate::actor::mesh_rt_init_actor(0);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let client = std::thread::spawn(move || ws_connect(port));
        let (tcp, _) = listener.accept().unwrap();
        register_server(
            ReactorTransport::plain(tcp),
            Arc::new(handler),
            ReactorConfig::server(SERVER_MAX_MESSAGE_BYTES),
        )
        .unwrap();
        client.join().unwrap()
    }

    /// The current actor's process.
    fn current_process() -> Arc<Mutex<Process>> {
        let pid = stack::get_current_pid().unwrap();
        global_scheduler().get_process(pid).unwrap()
    }

    fn message(tag: u64) -> Message {
        Message {
            buffer: MessageBuffer::new(Vec::new(), tag),
        }
    }

    /// on_connect: queue itself a message it ignores, then an exit signal.
    extern "C" fn exit_on_connect(_conn: *mut u8, _path: *mut u8, _headers: *mut u8) -> *mut u8 {
        let process = current_process();
        process.lock().mailbox.push(message(7));
        process
            .lock()
            .mailbox
            .push(message(crate::actor::EXIT_SIGNAL_TAG));
        std::ptr::dangling_mut::<u8>()
    }

    /// An exit signal ends a connection's actor: the client is told it is
    /// going away, and on_close hears why. Other actors' messages are
    /// ignored.
    #[test]
    fn an_exit_signal_closes_the_connection_as_going_away() {
        let record = Box::leak(Box::new(CloseRecord {
            code: AtomicU64::new(0),
            reason: Mutex::new(String::new()),
        }));
        let mut handler = callbacks(
            exit_on_connect as *mut u8,
            echo_on_message as *mut u8,
            recording_on_close as *mut u8,
        );
        handler.on_close_env = record as *const CloseRecord as *mut u8;
        let mut stream = connect_to(ServerOpenHandler { callbacks: handler });
        let close = read_frame(&mut stream).unwrap();
        assert_eq!(
            parse_close_payload(&close.payload).0,
            WsCloseCode::GOING_AWAY
        );
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while record.code.load(Ordering::SeqCst) == 0 {
            assert!(std::time::Instant::now() < deadline, "on_close never ran");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(record.code.load(Ordering::SeqCst), 1001);
        assert_eq!(&*record.reason.lock(), "WebSocket actor exited");
    }

    static RELEASED: AtomicBool = AtomicBool::new(false);

    /// on_connect: once released, fill its own mailbox.
    extern "C" fn fill_mailbox_on_connect(
        _conn: *mut u8,
        _path: *mut u8,
        _headers: *mut u8,
    ) -> *mut u8 {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !RELEASED.load(Ordering::SeqCst) {
            assert!(std::time::Instant::now() < deadline, "never released");
            std::thread::yield_now();
        }
        let process = current_process();
        while process.lock().mailbox.try_push(message(7)).is_ok() {}
        std::ptr::dangling_mut::<u8>()
    }

    /// The server's handler, with a message arriving before the actor
    /// attaches.
    struct MessageFirst(ServerOpenHandler);

    impl ServerHandshakeHandler for MessageFirst {
        fn opened(
            &self,
            connection: ReactorConnection,
            path: String,
            headers: Vec<(String, String)>,
        ) -> Arc<dyn ReactorEventSink> {
            let sink = self.0.opened(connection, path, headers);
            sink.event(ReactorEvent::Text(
                b"early".to_vec(),
                InboundPermit::unbounded(5),
            ))
            .unwrap();
            RELEASED.store(true, Ordering::SeqCst);
            sink
        }

        fn failed(&self, reason: &str) {
            self.0.failed(reason);
        }
    }

    /// A message that arrived before the actor could take it, when its
    /// mailbox is full by then, closes the connection as try-again-later.
    #[test]
    fn a_full_mailbox_at_attach_closes_the_connection() {
        let mut stream = connect_to(MessageFirst(ServerOpenHandler {
            callbacks: callbacks(
                fill_mailbox_on_connect as *mut u8,
                echo_on_message as *mut u8,
                noop_on_close as *mut u8,
            ),
        }));
        let close = read_frame(&mut stream).unwrap();
        assert_eq!(
            parse_close_payload(&close.payload),
            (
                WsCloseCode::TRY_AGAIN_LATER,
                "inbound queue full".to_string()
            )
        );
    }

    /// on_close: send on the closed connection, recording the result.
    extern "C" fn send_on_close(
        env: *mut u8,
        conn: *mut u8,
        _code: i64,
        _reason: *mut u8,
    ) -> *mut u8 {
        let sent = mesh_ws_send(conn, crate::string::mesh_str("late"));
        unsafe { &*(env as *const AtomicU64) }.store(sent as u64, Ordering::SeqCst);
        std::ptr::null_mut()
    }

    /// Ws.send refuses a null handle, and a connection that has closed. A
    /// binary message reaches on_message as a string.
    #[test]
    fn sends_to_a_null_or_closed_connection_are_refused() {
        crate::gc::mesh_rt_init();
        assert_eq!(
            mesh_ws_send(std::ptr::null_mut(), crate::string::mesh_str("x")),
            -1
        );
        let sent = Box::leak(Box::new(AtomicU64::new(0)));
        let mut handler = callbacks(
            accept_on_connect as *mut u8,
            echo_on_message as *mut u8,
            send_on_close as *mut u8,
        );
        handler.on_close_env = sent as *const AtomicU64 as *mut u8;
        let mut stream = connect_to(ServerOpenHandler { callbacks: handler });
        write_masked_frame(&mut stream, WsOpcode::Binary, b"bin", true, [1, 2, 3, 4]).unwrap();
        assert_eq!(read_frame(&mut stream).unwrap().payload, b"bin");
        ws_send_close(&mut stream, 1000);
        assert_eq!(read_frame(&mut stream).unwrap().opcode, WsOpcode::Close);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while sent.load(Ordering::SeqCst) == 0 {
            assert!(std::time::Instant::now() < deadline, "on_close never ran");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(sent.load(Ordering::SeqCst) as i64, -1);
    }

    /// A request that is no upgrade fails the connection: the server tells
    /// its log and closes the socket.
    #[test]
    fn a_failed_upgrade_closes_the_socket() {
        crate::actor::mesh_rt_init_actor(0);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let (tcp, _) = listener.accept().unwrap();
        register_server(
            ReactorTransport::plain(tcp),
            Arc::new(ServerOpenHandler {
                callbacks: callbacks(
                    accept_on_connect as *mut u8,
                    echo_on_message as *mut u8,
                    noop_on_close as *mut u8,
                ),
            }),
            ReactorConfig::server(SERVER_MAX_MESSAGE_BYTES),
        )
        .unwrap();
        client.write_all(b"BREW /pot\r\n\r\n").unwrap();
        assert_eq!(client.read(&mut [0u8; 1]).unwrap(), 0);
    }

    /// Ws.serve on a port another socket holds says so and returns.
    #[test]
    fn serving_on_a_taken_port_returns() {
        let taken = TcpListener::bind("0.0.0.0:0").unwrap();
        mesh_ws_serve(
            accept_on_connect as *mut u8,
            std::ptr::null_mut(),
            echo_on_message as *mut u8,
            std::ptr::null_mut(),
            noop_on_close as *mut u8,
            std::ptr::null_mut(),
            i64::from(taken.local_addr().unwrap().port()),
        );
    }

    /// A sink told its connection ended before the actor attached tells the
    /// actor at attachment; data past the pending bounds is refused.
    #[test]
    fn a_sink_holds_what_arrives_before_attachment_within_bounds() {
        let sink = ServerSink::new();
        sink.terminated("gone");
        let process = Arc::new(Mutex::new(Process::new(
            ProcessId(99_003),
            Priority::Normal,
        )));
        sink.attach(Arc::clone(&process), ProcessId(99_003))
            .unwrap();
        let disconnect = process.lock().mailbox.pop().unwrap().buffer;
        assert_eq!(disconnect.type_tag, WS_DISCONNECT_TAG);
        assert_eq!(
            decode_disconnect(&disconnect.data),
            (1006, "WebSocket transport closed".to_string())
        );

        let sink = ServerSink::new();
        for _ in 0..SERVER_PENDING_ITEMS {
            sink.event(ReactorEvent::Binary(
                Vec::new(),
                InboundPermit::unbounded(0),
            ))
            .unwrap();
        }
        assert_eq!(
            sink.event(ReactorEvent::Binary(
                Vec::new(),
                InboundPermit::unbounded(0)
            )),
            Err(SinkFull)
        );
    }

    #[test]
    fn attach_keeps_target_unpublished_until_pending_delivery_finishes() {
        let sink = Arc::new(ServerSink::new());
        sink.event(ReactorEvent::Text(
            b"first".to_vec(),
            InboundPermit::unbounded(5),
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
            sink.event(ReactorEvent::Text(Vec::new(), InboundPermit::unbounded(0)))
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
    fn failed_room_broadcast_disconnects_the_recipient() {
        extern "C" fn join_on_connect(conn: *mut u8, path: *mut u8, _headers: *mut u8) -> *mut u8 {
            crate::ws::rooms::mesh_ws_join(conn, path as *const MeshString);
            std::ptr::dangling_mut::<u8>()
        }

        crate::actor::mesh_rt_init_actor(0);
        for except in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let server = std::thread::spawn(move || {
                let (tcp, _) = listener.accept().unwrap();
                let handler: Arc<dyn ServerHandshakeHandler> = Arc::new(ServerOpenHandler {
                    callbacks: WsHandler {
                        on_connect_fn: join_on_connect as *mut u8,
                        on_connect_env: std::ptr::null_mut(),
                        on_message_fn: echo_on_message as *mut u8,
                        on_message_env: std::ptr::null_mut(),
                        on_close_fn: noop_on_close as *mut u8,
                        on_close_env: std::ptr::null_mut(),
                    },
                });
                register_server(
                    ReactorTransport::plain(tcp),
                    handler,
                    ReactorConfig::server(1),
                )
                .unwrap();
            });
            let mut stream = ws_connect(port);
            server.join().unwrap();
            ws_send_text(&mut stream, "r");
            assert_eq!(read_frame(&mut stream).unwrap().payload, b"r");

            let room = crate::string::mesh_string_new(b"/ws".as_ptr(), 3);
            let message = crate::string::mesh_string_new(b"too big".as_ptr(), 7);
            let failures = if except {
                crate::ws::rooms::mesh_ws_broadcast_except(room, message, std::ptr::null_mut())
            } else {
                crate::ws::rooms::mesh_ws_broadcast(room, message)
            };
            assert_eq!(failures, 1);
            let received = stream.read(&mut [0]);
            assert!(
                matches!(received, Ok(0))
                    || matches!(&received, Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset),
                "failed broadcast left the recipient connected: {received:?}"
            );
        }
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
            let transport = ReactorTransport::tls(connection, tcp);
            let handler: Arc<dyn ServerHandshakeHandler> = Arc::new(ServerOpenHandler {
                callbacks: WsHandler {
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
