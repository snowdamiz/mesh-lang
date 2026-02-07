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
//! - [`actor`]: Actor runtime -- PCB, M:N scheduler, corosensei coroutines
//!
//! ## ABI Contract
//!
//! All public `extern "C"` functions in this crate form the runtime ABI.
//! Compiled Snow programs call these functions directly via LLVM IR. The
//! function signatures must remain stable across Snow compiler versions
//! (or at least across a single phase).

pub mod actor;
pub mod env;
pub mod file;
pub mod gc;
pub mod io;
pub mod panic;
pub mod string;

// Re-export key functions for convenient Rust-side access and testing.
pub use actor::{
    snow_actor_link, snow_actor_receive, snow_actor_register, snow_actor_self, snow_actor_send,
    snow_actor_set_terminate, snow_actor_spawn, snow_actor_whereis, snow_reduction_check,
    snow_rt_init_actor, snow_rt_run_scheduler,
};
pub use env::{snow_env_args, snow_env_get};
pub use file::{
    snow_file_append, snow_file_delete, snow_file_exists, snow_file_read, snow_file_write,
};
pub use gc::{snow_gc_alloc, snow_gc_alloc_actor, snow_rt_init};
pub use io::{snow_io_eprintln, snow_io_read_line};
pub use panic::snow_panic;
pub use string::{
    snow_bool_to_string, snow_float_to_string, snow_int_to_string, snow_print, snow_println,
    snow_string_concat, snow_string_contains, snow_string_ends_with, snow_string_eq,
    snow_string_length, snow_string_new, snow_string_replace, snow_string_slice,
    snow_string_starts_with, snow_string_to_lower, snow_string_to_upper, snow_string_trim,
    SnowString,
};
