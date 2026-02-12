//! WebSocket protocol layer (RFC 6455).
//!
//! Provides the low-level WebSocket wire protocol implementation:
//! - **Frame codec**: Variable-length frame parsing and writing with XOR masking
//! - **Handshake**: HTTP upgrade with Sec-WebSocket-Accept validation
//!
//! The close module will be added next.

pub mod frame;
pub mod handshake;

pub use frame::{WsOpcode, WsFrame, read_frame, write_frame, apply_mask};
pub use handshake::{perform_upgrade, write_bad_request};
