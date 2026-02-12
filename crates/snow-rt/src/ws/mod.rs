//! WebSocket protocol layer (RFC 6455).
//!
//! Provides the low-level WebSocket wire protocol implementation:
//! - **Frame codec**: Variable-length frame parsing and writing with XOR masking
//!
//! The handshake and close modules will be added in Plan 02.

pub mod frame;
