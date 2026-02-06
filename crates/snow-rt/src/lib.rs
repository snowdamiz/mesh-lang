//! Snow runtime library.
//!
//! This crate provides the runtime support functions that compiled Snow
//! programs call at runtime. It is compiled as both:
//!
//! - A static library (`libsnow_rt.a`) for linking into Snow binaries
//! - A Rust library (`lib`) for unit testing
//!
//! ## Modules
//!
//! - [`gc`]: Arena/bump allocator for GC-managed memory (Phase 5: no collection)
//! - [`string`]: GC-managed string operations (create, concat, format, print)
//! - [`panic`]: Runtime panic handler with source locations
//!
//! ## ABI Contract
//!
//! All public `extern "C"` functions in this crate form the runtime ABI.
//! Compiled Snow programs call these functions directly via LLVM IR. The
//! function signatures must remain stable across Snow compiler versions
//! (or at least across a single phase).

pub mod gc;
pub mod panic;
pub mod string;

// Re-export key functions for convenient Rust-side access and testing.
pub use gc::{snow_gc_alloc, snow_rt_init};
pub use panic::snow_panic;
pub use string::{
    snow_bool_to_string, snow_float_to_string, snow_int_to_string, snow_print, snow_println,
    snow_string_concat, snow_string_new, SnowString,
};
