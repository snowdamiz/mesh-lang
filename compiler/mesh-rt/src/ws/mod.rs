//! WebSocket protocol layer (RFC 6455).
//!
//! Provides the complete low-level WebSocket wire protocol implementation:
//! - **Frame codec** (`frame`): Variable-length frame parsing and writing with XOR masking
//! - **Handshake** (`handshake`): HTTP upgrade with Sec-WebSocket-Accept validation
//! - **Close** (`close`): Close payloads and text UTF-8 validation

pub mod client;
pub mod close;
pub mod frame;
pub mod handshake;
pub(crate) mod reactor;
pub mod rooms;
pub mod server;

pub use close::{build_close_payload, is_valid_text_payload, parse_close_payload, WsCloseCode};
pub use frame::{apply_mask, read_frame, write_frame, WsFrame, WsOpcode};
pub use rooms::global_room_registry;
pub use server::{WS_BINARY_TAG, WS_CONNECT_TAG, WS_DISCONNECT_TAG, WS_TEXT_TAG};

/// Starts a named thread running `body`: `spawn_thread`, or in a test one
/// that fails as a system out of threads does.
pub(crate) type SpawnThread = fn(&str, Box<dyn FnOnce() + Send>) -> std::io::Result<()>;

pub(crate) fn spawn_thread(name: &str, body: Box<dyn FnOnce() + Send>) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name(name.to_string())
        .spawn(body)
        .map(drop)
}

#[cfg(test)]
pub(crate) fn no_threads(_name: &str, _body: Box<dyn FnOnce() + Send>) -> std::io::Result<()> {
    Err(std::io::Error::other("no threads left"))
}
