//! Private durable job state. The only mutation entry point is `JobStore::apply`.
#![allow(dead_code, unused_imports)] // The Raft and worker adapters are separate implementation units.

mod model;
mod store;

pub(crate) use model::*;
pub(crate) use store::{DueCursor, DuePage, JobStore};

#[cfg(test)]
mod tests;
