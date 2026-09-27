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
