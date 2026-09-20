//! Distribution subsystem for Mesh.
//!
//! Provides PID bit-packing helpers, the Mesh Term Format (STF) binary
//! serializer/deserializer, and the node identity/connection layer for
//! inter-node message transport.

pub mod autonomous;
pub mod bootstrap;
pub mod cluster_api;
pub mod consensus;
pub mod consensus_store;
pub mod continuity;
pub mod continuity_store;
pub mod discovery;
pub mod driver_service;
pub mod global;
pub mod identity;
pub mod identity_claim;
pub mod node;
pub mod operator;
pub mod protocol;
pub mod readiness;
pub mod routing;
pub mod scaling;
pub mod telemetry;
pub mod wire;

#[cfg(test)]
mod autonomous_model_tests;
