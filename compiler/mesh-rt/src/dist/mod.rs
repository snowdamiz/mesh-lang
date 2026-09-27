//! Distribution subsystem for Mesh.
//!
//! The node identity/connection layer for inter-node message transport, and
//! the cluster services built on it.

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

#[cfg(test)]
mod autonomous_model_tests;
#[cfg(test)]
mod consensus_testing;
