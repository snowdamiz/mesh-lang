//! WebSocket server runtime: actor-per-connection with reader thread bridge.
//!
//! Integrates the Phase 59 WebSocket protocol layer (frame codec, handshake,
//! close) with Snow's actor system. Each accepted WebSocket connection spawns
//! a dedicated actor with crash isolation via `catch_unwind`.
//!
//! The **reader thread bridge** is the novel component: a dedicated OS thread
//! per connection reads WebSocket frames from the TCP stream and pushes them
//! into the actor's mailbox via reserved type tags, without blocking the M:N
//! scheduler's worker threads.
//!
//! ## Architecture
//!
//! ```text
//! TcpListener (accept loop on calling thread)
//!     |
//!     v  spawn actor per connection
//! ws_connection_entry (actor coroutine on scheduler worker)
//!     |
//!     +-- perform_upgrade (HTTP -> WebSocket)
//!     +-- call on_connect (accept/reject)
//!     +-- split stream: read clone -> reader thread, write -> Arc<Mutex>
//!     +-- spawn reader thread
//!     +-- actor_message_loop (receive -> dispatch)
//!     +-- cleanup (close frame, shutdown reader thread)
//! ```

use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;

use crate::actor::{self, global_scheduler, MessageBuffer, Message, ProcessId, ProcessState};
use crate::actor::process::Process;
use crate::actor::stack;
use crate::string::SnowString;
use super::frame::{read_frame, write_frame, WsOpcode};
use super::handshake::perform_upgrade;
use super::close::{process_frame, send_close, WsCloseCode};

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

/// WebSocket handler containing three Snow closure pairs (on_connect,
/// on_message, on_close). Each closure is a `{fn_ptr, env_ptr}` pair.
///
/// Passed from the Snow-compiled program to `snow_ws_serve`. The struct
/// is `#[repr(C)]` so Snow's codegen can construct it directly.
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
// The pointers are to compiled Snow functions which are valid for the
// lifetime of the program.
unsafe impl Send for WsHandler {}

/// Connection handle for `Ws.send` -- stored on the Rust heap (not GC heap)
/// because it contains an `Arc<Mutex<TcpStream>>`.
struct WsConnection {
    write_stream: Arc<Mutex<TcpStream>>,
    shutdown: Arc<AtomicBool>,
}

/// Arguments passed to the spawned WebSocket actor, following the HTTP
/// server's `ConnectionArgs` pattern.
#[repr(C)]
struct WsConnectionArgs {
    handler: WsHandler,
    stream_ptr: usize,
}

// WsConnectionArgs contains raw pointers but is only used for transfer
// to the actor entry function.
unsafe impl Send for WsConnectionArgs {}

// ---------------------------------------------------------------------------
// Public API: snow_ws_serve, snow_ws_send, snow_ws_send_binary
// ---------------------------------------------------------------------------

/// Start a WebSocket server on the given port, blocking the calling thread.
///
/// Binds a TCP listener and spawns one actor per accepted WebSocket
/// connection. Each connection actor runs the upgrade handshake, calls
/// lifecycle callbacks (on_connect, on_message, on_close), and uses a
/// reader thread bridge for non-blocking frame delivery.
///
/// # Arguments
///
/// Six function/env pointer pairs for the three callbacks, plus the port:
/// - `on_connect_fn/env`: Called after handshake with (conn, path, headers)
/// - `on_message_fn/env`: Called for each text/binary frame with (conn, msg)
/// - `on_close_fn/env`: Called when connection ends with (conn, code, reason)
/// - `port`: TCP port to listen on
#[no_mangle]
pub extern "C" fn snow_ws_serve(
    on_connect_fn: *mut u8,
    on_connect_env: *mut u8,
    on_message_fn: *mut u8,
    on_message_env: *mut u8,
    on_close_fn: *mut u8,
    on_close_env: *mut u8,
    port: i64,
) {
    // Ensure the actor scheduler is initialized (idempotent).
    crate::actor::snow_rt_init_actor(0);

    let addr = format!("0.0.0.0:{}", port);
    let listener = match TcpListener::bind(&addr) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[snow-rt] Failed to start WebSocket server on {}: {}", addr, e);
            return;
        }
    };

    eprintln!("[snow-rt] WebSocket server listening on {}", addr);

    for tcp_stream in listener.incoming() {
        let tcp_stream = match tcp_stream {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[snow-rt] accept error: {}", e);
                continue;
            }
        };

        // Set read timeout before passing to the actor.
        tcp_stream.set_read_timeout(Some(Duration::from_secs(30))).ok();

        // Pack handler (copy the 6 pointers) and stream into args.
        let handler = WsHandler {
            on_connect_fn,
            on_connect_env,
            on_message_fn,
            on_message_env,
            on_close_fn,
            on_close_env,
        };
        let stream_ptr = Box::into_raw(Box::new(tcp_stream)) as usize;
        let args = WsConnectionArgs {
            handler,
            stream_ptr,
        };
        let args_ptr = Box::into_raw(Box::new(args)) as *const u8;
        let args_size = std::mem::size_of::<WsConnectionArgs>() as u64;

        let sched = global_scheduler();
        sched.spawn(
            ws_connection_entry as *const u8,
            args_ptr,
            args_size,
            1, // Normal priority
        );
    }
}

/// Send a text frame to a WebSocket client.
///
/// `conn` is a pointer to a `WsConnection` (obtained from the on_connect
/// callback). `msg` is a pointer to a `SnowString` containing the text.
///
/// Returns 0 on success, -1 on error.
#[no_mangle]
pub extern "C" fn snow_ws_send(conn: *mut u8, msg: *const SnowString) -> i64 {
    if conn.is_null() || msg.is_null() {
        return -1;
    }
    let conn = unsafe { &*(conn as *const WsConnection) };
    let text = unsafe { (*msg).as_str() };
    let mut stream = conn.write_stream.lock();
    match write_frame(&mut *stream, WsOpcode::Text, text.as_bytes(), true) {
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
pub extern "C" fn snow_ws_send_binary(conn: *mut u8, data: *const u8, len: i64) -> i64 {
    if conn.is_null() || data.is_null() {
        return -1;
    }
    let conn = unsafe { &*(conn as *const WsConnection) };
    let bytes = unsafe { std::slice::from_raw_parts(data, len as usize) };
    let mut stream = conn.write_stream.lock();
    match write_frame(&mut *stream, WsOpcode::Binary, bytes, true) {
        Ok(()) => 0,
        Err(_) => -1,
    }
}

// ---------------------------------------------------------------------------
// Actor entry point
// ---------------------------------------------------------------------------

/// Actor entry function for a single WebSocket connection.
///
/// Performs the upgrade handshake, splits the stream, spawns the reader
/// thread, runs the message loop, and handles cleanup on exit or crash.
extern "C" fn ws_connection_entry(args: *const u8) {
    if args.is_null() {
        return;
    }

    let args = unsafe { Box::from_raw(args as *mut WsConnectionArgs) };
    let handler = args.handler;
    let mut stream = unsafe { *Box::from_raw(args.stream_ptr as *mut TcpStream) };

    // 1. Perform WebSocket upgrade handshake
    let (path, headers) = match perform_upgrade(&mut stream) {
        Ok(ph) => ph,
        Err(e) => {
            eprintln!("[snow-rt] WS upgrade failed: {}", e);
            return;
        }
    };

    // 2. Split stream: clone for reading, original for writing via Arc<Mutex>
    let read_stream = match stream.try_clone() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[snow-rt] WS stream clone failed: {}", e);
            return;
        }
    };
    let write_stream = Arc::new(Mutex::new(stream));
    let shutdown = Arc::new(AtomicBool::new(false));

    // 3. Create WsConnection handle (Rust heap, not GC heap)
    let conn = Box::into_raw(Box::new(WsConnection {
        write_stream: write_stream.clone(),
        shutdown: shutdown.clone(),
    }));
    let conn_ptr = conn as *mut u8;

    // 4. Get current actor's PID and process Arc for reader thread
    let my_pid = stack::get_current_pid().expect("ws_connection_entry: no PID");
    let sched = global_scheduler();
    let proc_arc = sched
        .get_process(my_pid)
        .expect("ws_connection_entry: no process");

    // 5. Call on_connect callback (LIFE-01, LIFE-02)
    let accepted = call_on_connect(&handler, conn_ptr, &path, &headers);
    if !accepted {
        // on_connect rejected -- send close 1008 (Policy Violation)
        let _ = send_close(&mut *write_stream.lock(), WS_POLICY_VIOLATION, "rejected");
        shutdown.store(true, Ordering::SeqCst);
        // Clean up connection handle
        unsafe {
            drop(Box::from_raw(conn));
        }
        return;
    }

    // 6. Set read timeout on the read stream clone for the reader thread
    //    (5 seconds for periodic shutdown check)
    read_stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .ok();

    // 7. Spawn reader thread
    let reader_shutdown = shutdown.clone();
    let reader_write = write_stream.clone();
    let reader_proc = proc_arc.clone();
    let reader_pid = my_pid;
    std::thread::spawn(move || {
        reader_thread_loop(
            read_stream,
            reader_write,
            reader_proc,
            reader_pid,
            reader_shutdown,
        );
    });

    // 8. Actor message loop with catch_unwind (ACTOR-01, ACTOR-05)
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        actor_message_loop(&handler, conn_ptr);
    }));

    // 9. Cleanup
    shutdown.store(true, Ordering::SeqCst);

    if result.is_err() {
        // Actor crashed (ACTOR-05): send close 1011
        let _ = send_close(
            &mut *write_stream.lock(),
            WsCloseCode::INTERNAL_ERROR,
            "internal error",
        );
        // Call on_close for crash case (best-effort -- GC may not be safe)
        call_on_close(&handler, conn_ptr, WsCloseCode::INTERNAL_ERROR, "internal error");
    } else {
        // Normal exit (disconnect): call on_close (LIFE-04)
        call_on_close(&handler, conn_ptr, WsCloseCode::NORMAL, "");
    }

    // Clean up connection handle
    unsafe {
        drop(Box::from_raw(conn));
    }
}

// ---------------------------------------------------------------------------
// Reader thread
// ---------------------------------------------------------------------------

/// Reader thread loop: reads WebSocket frames from the TCP stream and pushes
/// them into the actor's mailbox via reserved type tags.
///
/// Runs on a dedicated OS thread to avoid blocking the M:N scheduler's worker
/// threads. Handles control frames (ping/pong/close) via `process_frame` and
/// delivers data frames (text/binary) as mailbox messages.
fn reader_thread_loop(
    mut read_stream: TcpStream,
    write_stream: Arc<Mutex<TcpStream>>,
    proc_arc: Arc<Mutex<Process>>,
    actor_pid: ProcessId,
    shutdown: Arc<AtomicBool>,
) {
    loop {
        if shutdown.load(Ordering::SeqCst) {
            break;
        }

        match read_frame(&mut read_stream) {
            Ok(frame) => {
                let mut writer = write_stream.lock();
                match process_frame(&mut *writer, frame) {
                    Ok(Some(data_frame)) => {
                        drop(writer); // Release lock before mailbox push
                        let tag = match data_frame.opcode {
                            WsOpcode::Text => WS_TEXT_TAG,
                            WsOpcode::Binary => WS_BINARY_TAG,
                            _ => WS_TEXT_TAG, // Continuation treated as text for now
                        };
                        let buffer = MessageBuffer::new(data_frame.payload, tag);
                        let msg = Message { buffer };

                        // Push to mailbox and wake actor if Waiting (Pitfall 7)
                        {
                            let mut proc = proc_arc.lock();
                            proc.mailbox.push(msg);
                            if matches!(proc.state, ProcessState::Waiting) {
                                proc.state = ProcessState::Ready;
                                drop(proc);
                                let sched = global_scheduler();
                                sched.wake_process(actor_pid);
                            }
                        }
                    }
                    Ok(None) => { /* Control frame handled (ping->pong) */ }
                    Err(_) => {
                        drop(writer);
                        // Close or protocol error -- push disconnect
                        push_disconnect(&proc_arc, actor_pid);
                        break;
                    }
                }
            }
            Err(e) => {
                if shutdown.load(Ordering::SeqCst) {
                    break;
                }
                // Check if it's a timeout (not a real error)
                if e.contains("timed out") || e.contains("WouldBlock") {
                    continue; // Just a read timeout, check shutdown and loop
                }
                // Real I/O error -- push disconnect
                push_disconnect(&proc_arc, actor_pid);
                break;
            }
        }
    }
}

/// Push a WS_DISCONNECT_TAG message to the actor's mailbox and wake it.
fn push_disconnect(proc_arc: &Arc<Mutex<Process>>, actor_pid: ProcessId) {
    let buffer = MessageBuffer::new(Vec::new(), WS_DISCONNECT_TAG);
    let msg = Message { buffer };
    let mut proc = proc_arc.lock();
    proc.mailbox.push(msg);
    if matches!(proc.state, ProcessState::Waiting) {
        proc.state = ProcessState::Ready;
        drop(proc);
        let sched = global_scheduler();
        sched.wake_process(actor_pid);
    }
}

// ---------------------------------------------------------------------------
// Actor message loop
// ---------------------------------------------------------------------------

/// Main message loop for the WebSocket actor.
///
/// Blocks on `snow_actor_receive(-1)` to get messages from the mailbox.
/// Dispatches based on the type tag:
/// - `WS_TEXT_TAG` / `WS_BINARY_TAG`: call on_message callback
/// - `WS_DISCONNECT_TAG`: client disconnected, exit loop
/// - `EXIT_SIGNAL_TAG`: exit signal from linked actor, exit loop
/// - Other: regular actor-to-actor message (ignored for now)
fn actor_message_loop(handler: &WsHandler, conn_ptr: *mut u8) {
    use crate::actor::snow_actor_receive;

    loop {
        let msg_ptr = snow_actor_receive(-1);
        if msg_ptr.is_null() {
            break;
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
                call_on_message(handler, conn_ptr, data_ptr, data_len, type_tag == WS_TEXT_TAG);
            }
            WS_DISCONNECT_TAG => {
                // Client disconnected (ACTOR-06)
                break;
            }
            tag if tag == crate::actor::EXIT_SIGNAL_TAG => {
                // Exit signal from linked actor
                break;
            }
            _ => {
                // Regular actor-to-actor message -- ignore for now
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Callback invocation helpers
// ---------------------------------------------------------------------------

/// Call the on_connect callback.
///
/// Builds Snow-level path string and headers map, invokes the callback.
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
        // Build Snow-level path string
        let path_snow =
            crate::string::snow_string_new(path.as_ptr(), path.len() as u64) as *mut u8;

        // Build headers map
        let mut headers_map = crate::collections::map::snow_map_new_typed(1);
        for (name, value) in headers {
            let key = crate::string::snow_string_new(name.as_ptr(), name.len() as u64);
            let val = crate::string::snow_string_new(value.as_ptr(), value.len() as u64);
            headers_map = crate::collections::map::snow_map_put(headers_map, key as u64, val as u64);
        }

        // Call the closure: if env is null, bare function; if non-null, closure
        let result = if handler.on_connect_env.is_null() {
            let f: fn(*mut u8, *mut u8, *mut u8) -> *mut u8 =
                std::mem::transmute(handler.on_connect_fn);
            f(conn_ptr, path_snow, headers_map)
        } else {
            let f: fn(*mut u8, *mut u8, *mut u8, *mut u8) -> *mut u8 =
                std::mem::transmute(handler.on_connect_fn);
            f(handler.on_connect_env, conn_ptr, path_snow, headers_map)
        };

        // Convention: non-null = accepted, null = rejected
        !result.is_null()
    }
}

/// Call the on_message callback.
///
/// Converts the frame payload to a SnowString and invokes the callback.
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
        // Build a Snow string from the frame payload
        let msg_snow =
            crate::string::snow_string_new(data_ptr, data_len as u64) as *mut u8;

        if handler.on_message_env.is_null() {
            let f: fn(*mut u8, *mut u8) -> *mut u8 =
                std::mem::transmute(handler.on_message_fn);
            f(conn_ptr, msg_snow);
        } else {
            let f: fn(*mut u8, *mut u8, *mut u8) -> *mut u8 =
                std::mem::transmute(handler.on_message_fn);
            f(handler.on_message_env, conn_ptr, msg_snow);
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
        let reason_snow =
            crate::string::snow_string_new(reason.as_ptr(), reason.len() as u64) as *mut u8;

        if handler.on_close_env.is_null() {
            let f: fn(*mut u8, i64, *mut u8) -> *mut u8 =
                std::mem::transmute(handler.on_close_fn);
            f(conn_ptr, code_i64, reason_snow);
        } else {
            let f: fn(*mut u8, *mut u8, i64, *mut u8) -> *mut u8 =
                std::mem::transmute(handler.on_close_fn);
            f(handler.on_close_env, conn_ptr, code_i64, reason_snow);
        }
    }
}
