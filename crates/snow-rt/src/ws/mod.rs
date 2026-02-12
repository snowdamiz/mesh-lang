//! WebSocket protocol layer (RFC 6455).
//!
//! Provides the complete low-level WebSocket wire protocol implementation:
//! - **Frame codec** (`frame`): Variable-length frame parsing and writing with XOR masking
//! - **Handshake** (`handshake`): HTTP upgrade with Sec-WebSocket-Accept validation
//! - **Close** (`close`): Close handshake, text UTF-8 validation, and protocol-level frame dispatch

pub mod frame;
pub mod handshake;
pub mod close;

pub use frame::{WsOpcode, WsFrame, read_frame, write_frame, apply_mask};
pub use handshake::{perform_upgrade, write_bad_request};
pub use close::{parse_close_payload, build_close_payload, send_close, validate_text_payload, process_frame, WsCloseCode};
