//! Distribution subsystem for Snow.
//!
//! Provides PID bit-packing helpers, the Snow Term Format (STF) binary
//! serializer/deserializer, and the node identity/connection layer for
//! inter-node message transport.

pub mod global;
pub mod node;
pub mod wire;
