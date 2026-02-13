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
pub mod collections;
pub mod db;
pub mod env;
pub mod file;
pub mod gc;
pub mod hash;
pub mod http;
pub mod io;
pub mod ws;
pub mod dist;
pub mod json;
pub mod option;
pub mod panic;
pub mod string;

// Re-export key functions for convenient Rust-side access and testing.
pub use actor::{
    snow_actor_link, snow_actor_receive, snow_actor_register, snow_actor_self, snow_actor_send,
    snow_actor_send_named,
    snow_actor_set_terminate, snow_actor_spawn, snow_actor_whereis, snow_reduction_check,
    snow_rt_init_actor, snow_rt_run_scheduler,
    snow_timer_sleep, snow_timer_send_after,
    snow_process_monitor, snow_process_demonitor,
    snow_node_monitor,
};
pub use actor::service::{snow_service_call, snow_service_reply};
pub use db::pg::{
    snow_pg_connect, snow_pg_close, snow_pg_execute, snow_pg_query,
};
pub use db::pool::{
    snow_pool_open, snow_pool_checkout, snow_pool_checkin, snow_pool_query,
    snow_pool_execute, snow_pool_close,
};
pub use db::sqlite::{
    snow_sqlite_open, snow_sqlite_close, snow_sqlite_execute, snow_sqlite_query,
};
pub use collections::list::{
    snow_list_all, snow_list_any, snow_list_append, snow_list_concat, snow_list_contains,
    snow_list_drop, snow_list_enumerate, snow_list_filter, snow_list_find, snow_list_flat_map,
    snow_list_flatten, snow_list_from_array, snow_list_get, snow_list_head, snow_list_last,
    snow_list_length, snow_list_map, snow_list_new, snow_list_nth, snow_list_reduce,
    snow_list_reverse, snow_list_sort, snow_list_tail, snow_list_take, snow_list_zip,
};
pub use collections::map::{
    snow_map_delete, snow_map_from_list, snow_map_get, snow_map_has_key, snow_map_keys,
    snow_map_merge, snow_map_new, snow_map_put, snow_map_size, snow_map_to_list, snow_map_values,
};
pub use collections::queue::{
    snow_queue_is_empty, snow_queue_new, snow_queue_peek, snow_queue_pop, snow_queue_push,
    snow_queue_size,
};
pub use collections::range::{
    snow_range_filter, snow_range_length, snow_range_map, snow_range_new, snow_range_to_list,
};
pub use collections::set::{
    snow_set_add, snow_set_contains, snow_set_difference, snow_set_from_list,
    snow_set_intersection, snow_set_new, snow_set_remove, snow_set_size, snow_set_to_list,
    snow_set_union,
};
pub use collections::tuple::{snow_tuple_first, snow_tuple_nth, snow_tuple_second, snow_tuple_size};
pub use env::{snow_env_args, snow_env_get};
pub use option::{SnowOption, alloc_option};
pub use file::{
    snow_file_append, snow_file_delete, snow_file_exists, snow_file_read, snow_file_write,
};
pub use gc::{snow_gc_alloc, snow_gc_alloc_actor, snow_rt_init};
pub use hash::{snow_hash_bool, snow_hash_combine, snow_hash_float, snow_hash_int, snow_hash_string};
pub use http::{
    snow_http_get, snow_http_post, snow_http_request_body, snow_http_request_header,
    snow_http_request_method, snow_http_request_path, snow_http_request_query, snow_http_response_new,
    snow_http_route, snow_http_router, snow_http_serve,
};
pub use io::{snow_io_eprintln, snow_io_read_line};
pub use json::{
    snow_json_encode, snow_json_encode_bool, snow_json_encode_int, snow_json_encode_list,
    snow_json_encode_map, snow_json_encode_string, snow_json_from_bool, snow_json_from_float,
    snow_json_from_int, snow_json_from_string, snow_json_parse,
};
pub use dist::node::{snow_node_self, snow_node_list, snow_node_start, snow_node_connect, snow_register_function, snow_node_spawn};
pub use panic::snow_panic;
pub use string::{
    snow_bool_to_string, snow_float_to_string, snow_int_to_string, snow_print, snow_println,
    snow_string_concat, snow_string_contains, snow_string_ends_with, snow_string_eq,
    snow_string_join, snow_string_length, snow_string_new, snow_string_replace, snow_string_slice,
    snow_string_split, snow_string_starts_with, snow_string_to_float, snow_string_to_int,
    snow_string_to_lower, snow_string_to_upper, snow_string_trim,
    SnowString,
};
