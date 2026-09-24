//! JIT compilation engine for REPL inputs.
//!
//! Uses the full Mesh compiler pipeline (parse -> typecheck -> MIR -> LLVM IR)
//! to compile expressions, then executes them via LLVM's JIT execution engine.
//! This ensures REPL behavior is identical to compiled code.
//!
//! The actor runtime (mesh-rt) is linked and its symbols are registered with
//! LLVM so that JIT-compiled code can call runtime functions like
//! `mesh_gc_alloc`, `mesh_actor_spawn`, etc.

use crate::session::ReplSession;
use std::sync::Once;

static RUNTIME_INIT: Once = Once::new();

/// Initialize the Mesh runtime (GC + actor scheduler) and register all
/// runtime symbols with LLVM so that JIT-compiled code can resolve them.
///
/// This is called once at REPL startup. Subsequent calls are no-ops.
pub fn init_runtime() {
    RUNTIME_INIT.call_once(|| {
        // Initialize the GC arena
        mesh_rt::mesh_rt_init();

        // Initialize the actor scheduler with default number of workers
        mesh_rt::mesh_rt_init_actor(0);

        // Register mesh-rt symbols with LLVM's global symbol table so the
        // JIT execution engine can resolve them. LLVM's MCJIT uses dlsym
        // on some platforms, but explicit registration is more reliable.
        register_runtime_symbols();
    });
}

/// Register all mesh-rt extern "C" symbols with LLVM's dynamic lookup.
///
/// This calls LLVMAddSymbol for each runtime function, making them
/// available to JIT-compiled code. Without this, the JIT engine cannot
/// resolve calls to runtime functions like mesh_gc_alloc, mesh_print, etc.
fn register_runtime_symbols() {
    // LLVMAddSymbol is the LLVM C API for registering symbols with the
    // JIT's symbol resolver. inkwell 0.8 doesn't expose it directly,
    // so we call into llvm-sys through the re-exported C bindings.
    extern "C" {
        fn LLVMAddSymbol(name: *const std::ffi::c_char, value: *mut std::ffi::c_void);
    }

    for (name, ptr) in runtime_symbols() {
        let c_name = std::ffi::CString::new(name).unwrap();
        unsafe {
            LLVMAddSymbol(c_name.as_ptr(), ptr as *mut std::ffi::c_void);
        }
    }
}

/// Every mesh-rt function compiled code may call, by symbol name. A call
/// to a symbol missing here jumps to null in the JIT; the
/// `every_declared_runtime_function_is_registered` test keeps it complete.
fn runtime_symbols() -> Vec<(&'static str, *const ())> {
    vec![
        (
            "mesh_actor_exit",
            mesh_rt::actor::mesh_actor_exit as *const (),
        ),
        ("mesh_actor_link", mesh_rt::mesh_actor_link as *const ()),
        (
            "mesh_actor_receive",
            mesh_rt::mesh_actor_receive as *const (),
        ),
        (
            "mesh_actor_register",
            mesh_rt::mesh_actor_register as *const (),
        ),
        ("mesh_actor_self", mesh_rt::mesh_actor_self as *const ()),
        ("mesh_actor_send", mesh_rt::mesh_actor_send as *const ()),
        (
            "mesh_actor_send_named",
            mesh_rt::actor::mesh_actor_send_named as *const (),
        ),
        (
            "mesh_actor_send_shaped",
            mesh_rt::mesh_actor_send_shaped as *const (),
        ),
        (
            "mesh_actor_set_terminate",
            mesh_rt::mesh_actor_set_terminate as *const (),
        ),
        ("mesh_actor_spawn", mesh_rt::mesh_actor_spawn as *const ()),
        (
            "mesh_actor_spawn_shaped",
            mesh_rt::mesh_actor_spawn_shaped as *const (),
        ),
        (
            "mesh_actor_stop",
            mesh_rt::actor::mesh_actor_stop as *const (),
        ),
        (
            "mesh_actor_trap_exit",
            mesh_rt::actor::mesh_actor_trap_exit as *const (),
        ),
        (
            "mesh_actor_whereis",
            mesh_rt::mesh_actor_whereis as *const (),
        ),
        (
            "mesh_alloc_result",
            mesh_rt::io::mesh_alloc_result as *const (),
        ),
        (
            "mesh_base64_decode",
            mesh_rt::crypto::mesh_base64_decode as *const (),
        ),
        (
            "mesh_base64_decode_url",
            mesh_rt::crypto::mesh_base64_decode_url as *const (),
        ),
        (
            "mesh_base64_encode",
            mesh_rt::crypto::mesh_base64_encode as *const (),
        ),
        (
            "mesh_base64_encode_url",
            mesh_rt::crypto::mesh_base64_encode_url as *const (),
        ),
        (
            "mesh_bool_to_string",
            mesh_rt::mesh_bool_to_string as *const (),
        ),
        (
            "mesh_bytes_builder_finish",
            mesh_rt::bytes::mesh_bytes_builder_finish as *const (),
        ),
        (
            "mesh_bytes_builder_new",
            mesh_rt::bytes::mesh_bytes_builder_new as *const (),
        ),
        (
            "mesh_bytes_builder_write_bytes",
            mesh_rt::bytes::mesh_bytes_builder_write_bytes as *const (),
        ),
        (
            "mesh_bytes_builder_write_u16_be",
            mesh_rt::bytes::mesh_bytes_builder_write_u16_be as *const (),
        ),
        (
            "mesh_bytes_builder_write_u32_be",
            mesh_rt::bytes::mesh_bytes_builder_write_u32_be as *const (),
        ),
        (
            "mesh_bytes_builder_write_u8",
            mesh_rt::bytes::mesh_bytes_builder_write_u8 as *const (),
        ),
        (
            "mesh_bytes_concat",
            mesh_rt::bytes::mesh_bytes_concat as *const (),
        ),
        (
            "mesh_bytes_empty",
            mesh_rt::bytes::mesh_bytes_empty as *const (),
        ),
        (
            "mesh_bytes_from_base58",
            mesh_rt::bytes::mesh_bytes_from_base58 as *const (),
        ),
        (
            "mesh_bytes_from_base64",
            mesh_rt::bytes::mesh_bytes_from_base64 as *const (),
        ),
        (
            "mesh_bytes_from_hex",
            mesh_rt::bytes::mesh_bytes_from_hex as *const (),
        ),
        (
            "mesh_bytes_from_list",
            mesh_rt::bytes::mesh_bytes_from_list as *const (),
        ),
        (
            "mesh_bytes_from_utf8",
            mesh_rt::bytes::mesh_bytes_from_utf8 as *const (),
        ),
        (
            "mesh_bytes_get",
            mesh_rt::bytes::mesh_bytes_get as *const (),
        ),
        (
            "mesh_bytes_length",
            mesh_rt::bytes::mesh_bytes_length as *const (),
        ),
        (
            "mesh_bytes_read_u16_be",
            mesh_rt::bytes::mesh_bytes_read_u16_be as *const (),
        ),
        (
            "mesh_bytes_read_u16_le",
            mesh_rt::bytes::mesh_bytes_read_u16_le as *const (),
        ),
        (
            "mesh_bytes_read_u32_be",
            mesh_rt::bytes::mesh_bytes_read_u32_be as *const (),
        ),
        (
            "mesh_bytes_read_u32_le",
            mesh_rt::bytes::mesh_bytes_read_u32_le as *const (),
        ),
        (
            "mesh_bytes_read_u64_be",
            mesh_rt::bytes::mesh_bytes_read_u64_be as *const (),
        ),
        (
            "mesh_bytes_read_u64_le",
            mesh_rt::bytes::mesh_bytes_read_u64_le as *const (),
        ),
        (
            "mesh_bytes_read_uint_le",
            mesh_rt::bytes::mesh_bytes_read_uint_le as *const (),
        ),
        (
            "mesh_bytes_repeat",
            mesh_rt::bytes::mesh_bytes_repeat as *const (),
        ),
        (
            "mesh_bytes_secure_equals",
            mesh_rt::bytes::mesh_bytes_secure_equals as *const (),
        ),
        (
            "mesh_bytes_slice",
            mesh_rt::bytes::mesh_bytes_slice as *const (),
        ),
        (
            "mesh_bytes_to_base58",
            mesh_rt::bytes::mesh_bytes_to_base58 as *const (),
        ),
        (
            "mesh_bytes_to_base64",
            mesh_rt::bytes::mesh_bytes_to_base64 as *const (),
        ),
        (
            "mesh_bytes_to_hex",
            mesh_rt::bytes::mesh_bytes_to_hex as *const (),
        ),
        (
            "mesh_bytes_to_list",
            mesh_rt::bytes::mesh_bytes_to_list as *const (),
        ),
        (
            "mesh_bytes_to_utf8",
            mesh_rt::bytes::mesh_bytes_to_utf8 as *const (),
        ),
        (
            "mesh_bytes_write_u16_be",
            mesh_rt::bytes::mesh_bytes_write_u16_be as *const (),
        ),
        (
            "mesh_bytes_write_u32_be",
            mesh_rt::bytes::mesh_bytes_write_u32_be as *const (),
        ),
        (
            "mesh_bytes_write_u64_be",
            mesh_rt::bytes::mesh_bytes_write_u64_be as *const (),
        ),
        (
            "mesh_bytes_write_uint_le",
            mesh_rt::bytes::mesh_bytes_write_uint_le as *const (),
        ),
        (
            "mesh_changeset_cast",
            mesh_rt::mesh_changeset_cast as *const (),
        ),
        (
            "mesh_changeset_cast_with_types",
            mesh_rt::mesh_changeset_cast_with_types as *const (),
        ),
        (
            "mesh_changeset_changes",
            mesh_rt::mesh_changeset_changes as *const (),
        ),
        (
            "mesh_changeset_errors",
            mesh_rt::mesh_changeset_errors as *const (),
        ),
        (
            "mesh_changeset_get_change",
            mesh_rt::mesh_changeset_get_change as *const (),
        ),
        (
            "mesh_changeset_get_error",
            mesh_rt::mesh_changeset_get_error as *const (),
        ),
        (
            "mesh_changeset_valid",
            mesh_rt::mesh_changeset_valid as *const (),
        ),
        (
            "mesh_changeset_validate_format",
            mesh_rt::mesh_changeset_validate_format as *const (),
        ),
        (
            "mesh_changeset_validate_inclusion",
            mesh_rt::mesh_changeset_validate_inclusion as *const (),
        ),
        (
            "mesh_changeset_validate_length",
            mesh_rt::mesh_changeset_validate_length as *const (),
        ),
        (
            "mesh_changeset_validate_number",
            mesh_rt::mesh_changeset_validate_number as *const (),
        ),
        (
            "mesh_changeset_validate_required",
            mesh_rt::mesh_changeset_validate_required as *const (),
        ),
        (
            "mesh_channel_bounded",
            mesh_rt::channel::mesh_channel_bounded as *const (),
        ),
        (
            "mesh_channel_bounded_bytes",
            mesh_rt::channel::mesh_channel_bounded_bytes as *const (),
        ),
        (
            "mesh_channel_byte_depth",
            mesh_rt::channel::mesh_channel_byte_depth as *const (),
        ),
        (
            "mesh_channel_depth",
            mesh_rt::channel::mesh_channel_depth as *const (),
        ),
        (
            "mesh_channel_dropped",
            mesh_rt::channel::mesh_channel_dropped as *const (),
        ),
        (
            "mesh_channel_recv",
            mesh_rt::channel::mesh_channel_recv as *const (),
        ),
        (
            "mesh_channel_try_send",
            mesh_rt::channel::mesh_channel_try_send as *const (),
        ),
        (
            "mesh_checked_abs",
            mesh_rt::finance::mesh_checked_abs as *const (),
        ),
        (
            "mesh_checked_add",
            mesh_rt::finance::mesh_checked_add as *const (),
        ),
        (
            "mesh_checked_div",
            mesh_rt::finance::mesh_checked_div as *const (),
        ),
        (
            "mesh_checked_mul",
            mesh_rt::finance::mesh_checked_mul as *const (),
        ),
        (
            "mesh_checked_mul_div",
            mesh_rt::finance::mesh_checked_mul_div as *const (),
        ),
        (
            "mesh_checked_rescale",
            mesh_rt::finance::mesh_checked_rescale as *const (),
        ),
        (
            "mesh_checked_sub",
            mesh_rt::finance::mesh_checked_sub as *const (),
        ),
        (
            "mesh_cluster_capacity",
            mesh_rt::dist::cluster_api::mesh_cluster_capacity as *const (),
        ),
        (
            "mesh_cluster_pressure",
            mesh_rt::dist::cluster_api::mesh_cluster_pressure as *const (),
        ),
        (
            "mesh_cluster_role",
            mesh_rt::dist::cluster_api::mesh_cluster_role as *const (),
        ),
        (
            "mesh_cluster_state",
            mesh_rt::dist::cluster_api::mesh_cluster_state as *const (),
        ),
        (
            "mesh_cluster_telemetry",
            mesh_rt::dist::cluster_api::mesh_cluster_telemetry as *const (),
        ),
        (
            "mesh_continuity_acknowledge_replica",
            mesh_rt::dist::continuity::mesh_continuity_acknowledge_replica as *const (),
        ),
        (
            "mesh_continuity_authority_status",
            mesh_rt::dist::continuity::mesh_continuity_authority_status as *const (),
        ),
        (
            "mesh_continuity_complete_declared_work",
            mesh_rt::dist::continuity::mesh_continuity_complete_declared_work as *const (),
        ),
        (
            "mesh_continuity_mark_completed",
            mesh_rt::dist::continuity::mesh_continuity_mark_completed as *const (),
        ),
        (
            "mesh_continuity_status",
            mesh_rt::dist::continuity::mesh_continuity_status as *const (),
        ),
        (
            "mesh_continuity_submit",
            mesh_rt::dist::continuity::mesh_continuity_submit as *const (),
        ),
        (
            "mesh_continuity_submit_declared_work",
            mesh_rt::dist::continuity::mesh_continuity_submit_declared_work as *const (),
        ),
        (
            "mesh_continuity_submit_with_durability",
            mesh_rt::dist::continuity::mesh_continuity_submit_with_durability as *const (),
        ),
        (
            "mesh_crypto_aead_key",
            mesh_rt::crypto::mesh_crypto_aead_key as *const (),
        ),
        (
            "mesh_crypto_aead_open",
            mesh_rt::crypto::mesh_crypto_aead_open as *const (),
        ),
        (
            "mesh_crypto_aead_seal",
            mesh_rt::crypto::mesh_crypto_aead_seal as *const (),
        ),
        (
            "mesh_crypto_argon2id",
            mesh_rt::crypto::mesh_crypto_argon2id as *const (),
        ),
        (
            "mesh_crypto_hkdf_sha256",
            mesh_rt::crypto::mesh_crypto_hkdf_sha256 as *const (),
        ),
        (
            "mesh_crypto_hmac_sha256",
            mesh_rt::crypto::mesh_crypto_hmac_sha256 as *const (),
        ),
        (
            "mesh_crypto_hmac_sha512",
            mesh_rt::crypto::mesh_crypto_hmac_sha512 as *const (),
        ),
        (
            "mesh_crypto_hpke_open",
            mesh_rt::crypto::mesh_crypto_hpke_open as *const (),
        ),
        (
            "mesh_crypto_hpke_open_secret",
            mesh_rt::crypto::mesh_crypto_hpke_open_secret as *const (),
        ),
        (
            "mesh_crypto_hpke_seal",
            mesh_rt::crypto::mesh_crypto_hpke_seal as *const (),
        ),
        (
            "mesh_crypto_hpke_seal_secret",
            mesh_rt::crypto::mesh_crypto_hpke_seal_secret as *const (),
        ),
        (
            "mesh_crypto_mlkem_decapsulate",
            mesh_rt::crypto::mesh_crypto_mlkem_decapsulate as *const (),
        ),
        (
            "mesh_crypto_mlkem_encapsulate",
            mesh_rt::crypto::mesh_crypto_mlkem_encapsulate as *const (),
        ),
        (
            "mesh_crypto_mlkem_from_secret",
            mesh_rt::crypto::mesh_crypto_mlkem_from_secret as *const (),
        ),
        (
            "mesh_crypto_mlkem_from_seed",
            mesh_rt::crypto::mesh_crypto_mlkem_from_seed as *const (),
        ),
        (
            "mesh_crypto_mlkem_generate",
            mesh_rt::crypto::mesh_crypto_mlkem_generate as *const (),
        ),
        (
            "mesh_crypto_random_bytes",
            mesh_rt::crypto::mesh_crypto_random_bytes as *const (),
        ),
        (
            "mesh_crypto_sha256",
            mesh_rt::crypto::mesh_crypto_sha256 as *const (),
        ),
        (
            "mesh_crypto_sha256_hex",
            mesh_rt::crypto::mesh_crypto_sha256_hex as *const (),
        ),
        (
            "mesh_crypto_sha512",
            mesh_rt::crypto::mesh_crypto_sha512 as *const (),
        ),
        (
            "mesh_crypto_sha512_hex",
            mesh_rt::crypto::mesh_crypto_sha512_hex as *const (),
        ),
        (
            "mesh_crypto_sign",
            mesh_rt::crypto::mesh_crypto_sign as *const (),
        ),
        (
            "mesh_crypto_signing_from_secret",
            mesh_rt::crypto::mesh_crypto_signing_from_secret as *const (),
        ),
        (
            "mesh_crypto_signing_from_seed",
            mesh_rt::crypto::mesh_crypto_signing_from_seed as *const (),
        ),
        (
            "mesh_crypto_signing_generate",
            mesh_rt::crypto::mesh_crypto_signing_generate as *const (),
        ),
        (
            "mesh_crypto_uuid4",
            mesh_rt::crypto::mesh_crypto_uuid4 as *const (),
        ),
        (
            "mesh_crypto_verify",
            mesh_rt::crypto::mesh_crypto_verify as *const (),
        ),
        (
            "mesh_crypto_x25519_from_secret",
            mesh_rt::crypto::mesh_crypto_x25519_from_secret as *const (),
        ),
        (
            "mesh_crypto_x25519_from_seed",
            mesh_rt::crypto::mesh_crypto_x25519_from_seed as *const (),
        ),
        (
            "mesh_crypto_x25519_generate",
            mesh_rt::crypto::mesh_crypto_x25519_generate as *const (),
        ),
        (
            "mesh_crypto_x25519_public",
            mesh_rt::crypto::mesh_crypto_x25519_public as *const (),
        ),
        (
            "mesh_crypto_x25519_shared",
            mesh_rt::crypto::mesh_crypto_x25519_shared as *const (),
        ),
        (
            "mesh_datetime_add",
            mesh_rt::datetime::mesh_datetime_add as *const (),
        ),
        (
            "mesh_datetime_after",
            mesh_rt::datetime::mesh_datetime_after as *const (),
        ),
        (
            "mesh_datetime_before",
            mesh_rt::datetime::mesh_datetime_before as *const (),
        ),
        (
            "mesh_datetime_diff",
            mesh_rt::datetime::mesh_datetime_diff as *const (),
        ),
        (
            "mesh_datetime_from_iso8601",
            mesh_rt::datetime::mesh_datetime_from_iso8601 as *const (),
        ),
        (
            "mesh_datetime_from_unix_ms",
            mesh_rt::datetime::mesh_datetime_from_unix_ms as *const (),
        ),
        (
            "mesh_datetime_from_unix_secs",
            mesh_rt::datetime::mesh_datetime_from_unix_secs as *const (),
        ),
        (
            "mesh_datetime_to_iso8601",
            mesh_rt::datetime::mesh_datetime_to_iso8601 as *const (),
        ),
        (
            "mesh_datetime_to_unix_ms",
            mesh_rt::datetime::mesh_datetime_to_unix_ms as *const (),
        ),
        (
            "mesh_datetime_to_unix_secs",
            mesh_rt::datetime::mesh_datetime_to_unix_secs as *const (),
        ),
        (
            "mesh_datetime_utc_now",
            mesh_rt::datetime::mesh_datetime_utc_now as *const (),
        ),
        (
            "mesh_duration_millis",
            mesh_rt::monotonic::mesh_duration_millis as *const (),
        ),
        (
            "mesh_duration_seconds",
            mesh_rt::monotonic::mesh_duration_seconds as *const (),
        ),
        ("mesh_env_args", mesh_rt::mesh_env_args as *const ()),
        ("mesh_env_get", mesh_rt::mesh_env_get as *const ()),
        ("mesh_env_get_int", mesh_rt::mesh_env_get_int as *const ()),
        (
            "mesh_env_get_secret_hex",
            mesh_rt::env::mesh_env_get_secret_hex as *const (),
        ),
        (
            "mesh_env_get_with_default",
            mesh_rt::mesh_env_get_with_default as *const (),
        ),
        (
            "mesh_expr_add",
            mesh_rt::db::expr::mesh_expr_add as *const (),
        ),
        (
            "mesh_expr_alias",
            mesh_rt::db::expr::mesh_expr_alias as *const (),
        ),
        (
            "mesh_expr_call",
            mesh_rt::db::expr::mesh_expr_call as *const (),
        ),
        (
            "mesh_expr_case",
            mesh_rt::db::expr::mesh_expr_case as *const (),
        ),
        (
            "mesh_expr_coalesce",
            mesh_rt::db::expr::mesh_expr_coalesce as *const (),
        ),
        (
            "mesh_expr_column",
            mesh_rt::db::expr::mesh_expr_column as *const (),
        ),
        (
            "mesh_expr_div",
            mesh_rt::db::expr::mesh_expr_div as *const (),
        ),
        ("mesh_expr_eq", mesh_rt::db::expr::mesh_expr_eq as *const ()),
        (
            "mesh_expr_excluded",
            mesh_rt::db::expr::mesh_expr_excluded as *const (),
        ),
        ("mesh_expr_gt", mesh_rt::db::expr::mesh_expr_gt as *const ()),
        (
            "mesh_expr_gte",
            mesh_rt::db::expr::mesh_expr_gte as *const (),
        ),
        ("mesh_expr_lt", mesh_rt::db::expr::mesh_expr_lt as *const ()),
        (
            "mesh_expr_lte",
            mesh_rt::db::expr::mesh_expr_lte as *const (),
        ),
        (
            "mesh_expr_mul",
            mesh_rt::db::expr::mesh_expr_mul as *const (),
        ),
        (
            "mesh_expr_neq",
            mesh_rt::db::expr::mesh_expr_neq as *const (),
        ),
        (
            "mesh_expr_null",
            mesh_rt::db::expr::mesh_expr_null as *const (),
        ),
        (
            "mesh_expr_sub",
            mesh_rt::db::expr::mesh_expr_sub as *const (),
        ),
        (
            "mesh_expr_value",
            mesh_rt::db::expr::mesh_expr_value as *const (),
        ),
        ("mesh_file_append", mesh_rt::mesh_file_append as *const ()),
        ("mesh_file_delete", mesh_rt::mesh_file_delete as *const ()),
        ("mesh_file_exists", mesh_rt::mesh_file_exists as *const ()),
        ("mesh_file_read", mesh_rt::mesh_file_read as *const ()),
        (
            "mesh_file_read_bytes",
            mesh_rt::file::mesh_file_read_bytes as *const (),
        ),
        ("mesh_file_size", mesh_rt::file::mesh_file_size as *const ()),
        ("mesh_file_write", mesh_rt::mesh_file_write as *const ()),
        (
            "mesh_file_write_bytes",
            mesh_rt::file::mesh_file_write_bytes as *const (),
        ),
        (
            "mesh_float_to_string",
            mesh_rt::mesh_float_to_string as *const (),
        ),
        ("mesh_gc_alloc", mesh_rt::mesh_gc_alloc as *const ()),
        (
            "mesh_gc_alloc_actor",
            mesh_rt::mesh_gc_alloc_actor as *const (),
        ),
        (
            "mesh_global_register",
            mesh_rt::actor::mesh_global_register as *const (),
        ),
        (
            "mesh_global_unregister",
            mesh_rt::actor::mesh_global_unregister as *const (),
        ),
        (
            "mesh_global_whereis",
            mesh_rt::actor::mesh_global_whereis as *const (),
        ),
        ("mesh_hash_bool", mesh_rt::mesh_hash_bool as *const ()),
        ("mesh_hash_combine", mesh_rt::mesh_hash_combine as *const ()),
        ("mesh_hash_float", mesh_rt::mesh_hash_float as *const ()),
        ("mesh_hash_int", mesh_rt::mesh_hash_int as *const ()),
        ("mesh_hash_string", mesh_rt::mesh_hash_string as *const ()),
        (
            "mesh_hex_decode",
            mesh_rt::crypto::mesh_hex_decode as *const (),
        ),
        (
            "mesh_hex_encode",
            mesh_rt::crypto::mesh_hex_encode as *const (),
        ),
        (
            "mesh_host_background_schedule",
            mesh_rt::library::mesh_host_background_schedule as *const (),
        ),
        (
            "mesh_host_log_redacted",
            mesh_rt::library::mesh_host_log_redacted as *const (),
        ),
        (
            "mesh_host_monotonic_clock",
            mesh_rt::library::mesh_host_monotonic_clock as *const (),
        ),
        (
            "mesh_host_network_state",
            mesh_rt::library::mesh_host_network_state as *const (),
        ),
        (
            "mesh_host_push_get_token",
            mesh_rt::library::mesh_host_push_get_token as *const (),
        ),
        (
            "mesh_host_secure_store_delete",
            mesh_rt::library::mesh_host_secure_store_delete as *const (),
        ),
        (
            "mesh_host_secure_store_get",
            mesh_rt::library::mesh_host_secure_store_get as *const (),
        ),
        (
            "mesh_host_secure_store_put",
            mesh_rt::library::mesh_host_secure_store_put as *const (),
        ),
        (
            "mesh_host_wall_clock",
            mesh_rt::library::mesh_host_wall_clock as *const (),
        ),
        (
            "mesh_http_body",
            mesh_rt::http::client::mesh_http_body as *const (),
        ),
        (
            "mesh_http_body_bytes",
            mesh_rt::http::client::mesh_http_body_bytes as *const (),
        ),
        (
            "mesh_http_build",
            mesh_rt::http::client::mesh_http_build as *const (),
        ),
        (
            "mesh_http_cancel",
            mesh_rt::http::client::mesh_http_cancel as *const (),
        ),
        (
            "mesh_http_client",
            mesh_rt::http::client::mesh_http_client as *const (),
        ),
        (
            "mesh_http_client_close",
            mesh_rt::http::client::mesh_http_client_close as *const (),
        ),
        (
            "mesh_http_header",
            mesh_rt::http::client::mesh_http_header as *const (),
        ),
        (
            "mesh_http_idempotency_key",
            mesh_rt::http::server::mesh_http_idempotency_key as *const (),
        ),
        (
            "mesh_http_json",
            mesh_rt::http::client::mesh_http_json as *const (),
        ),
        (
            "mesh_http_max_redirects",
            mesh_rt::http::client::mesh_http_max_redirects as *const (),
        ),
        (
            "mesh_http_max_response_bytes",
            mesh_rt::http::client::mesh_http_max_response_bytes as *const (),
        ),
        (
            "mesh_http_metrics",
            mesh_rt::http::client::mesh_http_metrics as *const (),
        ),
        (
            "mesh_http_query",
            mesh_rt::http::client::mesh_http_query as *const (),
        ),
        (
            "mesh_http_request_body",
            mesh_rt::mesh_http_request_body as *const (),
        ),
        (
            "mesh_http_request_body_bytes",
            mesh_rt::http::server::mesh_http_request_body_bytes as *const (),
        ),
        (
            "mesh_http_request_header",
            mesh_rt::mesh_http_request_header as *const (),
        ),
        (
            "mesh_http_request_id",
            mesh_rt::http::server::mesh_http_request_id as *const (),
        ),
        (
            "mesh_http_request_method",
            mesh_rt::mesh_http_request_method as *const (),
        ),
        (
            "mesh_http_request_param",
            mesh_rt::http::server::mesh_http_request_param as *const (),
        ),
        (
            "mesh_http_request_path",
            mesh_rt::mesh_http_request_path as *const (),
        ),
        (
            "mesh_http_request_query",
            mesh_rt::mesh_http_request_query as *const (),
        ),
        (
            "mesh_http_response_bytes_new",
            mesh_rt::http::server::mesh_http_response_bytes_new as *const (),
        ),
        (
            "mesh_http_response_bytes_with_headers",
            mesh_rt::http::server::mesh_http_response_bytes_with_headers as *const (),
        ),
        (
            "mesh_http_response_new",
            mesh_rt::mesh_http_response_new as *const (),
        ),
        (
            "mesh_http_response_with_headers",
            mesh_rt::http::server::mesh_http_response_with_headers as *const (),
        ),
        (
            "mesh_http_retry_class",
            mesh_rt::http::client::mesh_http_retry_class as *const (),
        ),
        ("mesh_http_route", mesh_rt::mesh_http_route as *const ()),
        (
            "mesh_http_route_delete",
            mesh_rt::http::router::mesh_http_route_delete as *const (),
        ),
        (
            "mesh_http_route_get",
            mesh_rt::http::router::mesh_http_route_get as *const (),
        ),
        (
            "mesh_http_route_post",
            mesh_rt::http::router::mesh_http_route_post as *const (),
        ),
        (
            "mesh_http_route_put",
            mesh_rt::http::router::mesh_http_route_put as *const (),
        ),
        ("mesh_http_router", mesh_rt::mesh_http_router as *const ()),
        (
            "mesh_http_send",
            mesh_rt::http::client::mesh_http_send as *const (),
        ),
        (
            "mesh_http_send_with",
            mesh_rt::http::client::mesh_http_send_with as *const (),
        ),
        ("mesh_http_serve", mesh_rt::mesh_http_serve as *const ()),
        (
            "mesh_http_serve_tls",
            mesh_rt::http::server::mesh_http_serve_tls as *const (),
        ),
        (
            "mesh_http_stage_timeout",
            mesh_rt::http::client::mesh_http_stage_timeout as *const (),
        ),
        (
            "mesh_http_stream",
            mesh_rt::http::client::mesh_http_stream as *const (),
        ),
        (
            "mesh_http_stream_bytes",
            mesh_rt::http::client::mesh_http_stream_bytes as *const (),
        ),
        (
            "mesh_http_timeout",
            mesh_rt::http::client::mesh_http_timeout as *const (),
        ),
        (
            "mesh_http_use_middleware",
            mesh_rt::http::router::mesh_http_use_middleware as *const (),
        ),
        (
            "mesh_i128_add",
            mesh_rt::wide_num::mesh_i128_add as *const (),
        ),
        (
            "mesh_i128_compare",
            mesh_rt::wide_num::mesh_i128_compare as *const (),
        ),
        (
            "mesh_i128_divide",
            mesh_rt::wide_num::mesh_i128_divide as *const (),
        ),
        (
            "mesh_i128_multiply",
            mesh_rt::wide_num::mesh_i128_multiply as *const (),
        ),
        (
            "mesh_i128_parse",
            mesh_rt::wide_num::mesh_i128_parse as *const (),
        ),
        (
            "mesh_i128_subtract",
            mesh_rt::wide_num::mesh_i128_subtract as *const (),
        ),
        (
            "mesh_i128_to_int",
            mesh_rt::wide_num::mesh_i128_to_int as *const (),
        ),
        (
            "mesh_i128_to_string",
            mesh_rt::wide_num::mesh_i128_to_string as *const (),
        ),
        (
            "mesh_int_to_string",
            mesh_rt::mesh_int_to_string as *const (),
        ),
        ("mesh_io_eprintln", mesh_rt::mesh_io_eprintln as *const ()),
        ("mesh_io_read_line", mesh_rt::mesh_io_read_line as *const ()),
        ("mesh_iter_all", mesh_rt::mesh_iter_all as *const ()),
        ("mesh_iter_any", mesh_rt::mesh_iter_any as *const ()),
        ("mesh_iter_count", mesh_rt::mesh_iter_count as *const ()),
        (
            "mesh_iter_enumerate",
            mesh_rt::mesh_iter_enumerate as *const (),
        ),
        (
            "mesh_iter_enumerate_next",
            mesh_rt::mesh_iter_enumerate_next as *const (),
        ),
        ("mesh_iter_filter", mesh_rt::mesh_iter_filter as *const ()),
        (
            "mesh_iter_filter_next",
            mesh_rt::mesh_iter_filter_next as *const (),
        ),
        ("mesh_iter_find", mesh_rt::mesh_iter_find as *const ()),
        (
            "mesh_iter_from",
            mesh_rt::collections::list::mesh_iter_from as *const (),
        ),
        (
            "mesh_iter_generic_next",
            mesh_rt::mesh_iter_generic_next as *const (),
        ),
        ("mesh_iter_map", mesh_rt::mesh_iter_map as *const ()),
        (
            "mesh_iter_map_next",
            mesh_rt::mesh_iter_map_next as *const (),
        ),
        ("mesh_iter_reduce", mesh_rt::mesh_iter_reduce as *const ()),
        ("mesh_iter_skip", mesh_rt::mesh_iter_skip as *const ()),
        (
            "mesh_iter_skip_next",
            mesh_rt::mesh_iter_skip_next as *const (),
        ),
        ("mesh_iter_sum", mesh_rt::mesh_iter_sum as *const ()),
        ("mesh_iter_take", mesh_rt::mesh_iter_take as *const ()),
        (
            "mesh_iter_take_next",
            mesh_rt::mesh_iter_take_next as *const (),
        ),
        ("mesh_iter_zip", mesh_rt::mesh_iter_zip as *const ()),
        (
            "mesh_iter_zip_next",
            mesh_rt::mesh_iter_zip_next as *const (),
        ),
        (
            "mesh_job_async",
            mesh_rt::actor::job::mesh_job_async as *const (),
        ),
        (
            "mesh_job_async_shaped",
            mesh_rt::actor::job::mesh_job_async_shaped as *const (),
        ),
        (
            "mesh_job_await",
            mesh_rt::actor::job::mesh_job_await as *const (),
        ),
        (
            "mesh_job_await_timeout",
            mesh_rt::actor::job::mesh_job_await_timeout as *const (),
        ),
        (
            "mesh_job_map",
            mesh_rt::actor::job::mesh_job_map as *const (),
        ),
        (
            "mesh_job_map_shaped",
            mesh_rt::actor::job::mesh_job_map_shaped as *const (),
        ),
        (
            "mesh_json_array_get",
            mesh_rt::json::mesh_json_array_get as *const (),
        ),
        (
            "mesh_json_array_length",
            mesh_rt::json::mesh_json_array_length as *const (),
        ),
        (
            "mesh_json_array_new",
            mesh_rt::json::mesh_json_array_new as *const (),
        ),
        (
            "mesh_json_array_push",
            mesh_rt::json::mesh_json_array_push as *const (),
        ),
        (
            "mesh_json_as_bool",
            mesh_rt::json::mesh_json_as_bool as *const (),
        ),
        (
            "mesh_json_as_float",
            mesh_rt::json::mesh_json_as_float as *const (),
        ),
        (
            "mesh_json_as_int",
            mesh_rt::json::mesh_json_as_int as *const (),
        ),
        (
            "mesh_json_as_string",
            mesh_rt::json::mesh_json_as_string as *const (),
        ),
        ("mesh_json_encode", mesh_rt::mesh_json_encode as *const ()),
        (
            "mesh_json_encode_bool",
            mesh_rt::mesh_json_encode_bool as *const (),
        ),
        (
            "mesh_json_encode_int",
            mesh_rt::mesh_json_encode_int as *const (),
        ),
        (
            "mesh_json_encode_list",
            mesh_rt::mesh_json_encode_list as *const (),
        ),
        (
            "mesh_json_encode_map",
            mesh_rt::mesh_json_encode_map as *const (),
        ),
        (
            "mesh_json_encode_string",
            mesh_rt::mesh_json_encode_string as *const (),
        ),
        (
            "mesh_json_from_bool",
            mesh_rt::mesh_json_from_bool as *const (),
        ),
        (
            "mesh_json_from_float",
            mesh_rt::mesh_json_from_float as *const (),
        ),
        (
            "mesh_json_from_int",
            mesh_rt::mesh_json_from_int as *const (),
        ),
        (
            "mesh_json_from_list",
            mesh_rt::json::mesh_json_from_list as *const (),
        ),
        (
            "mesh_json_from_map",
            mesh_rt::json::mesh_json_from_map as *const (),
        ),
        (
            "mesh_json_from_string",
            mesh_rt::mesh_json_from_string as *const (),
        ),
        ("mesh_json_get", mesh_rt::mesh_json_get as *const ()),
        (
            "mesh_json_get_nested",
            mesh_rt::mesh_json_get_nested as *const (),
        ),
        (
            "mesh_json_is_null",
            mesh_rt::json::mesh_json_is_null as *const (),
        ),
        (
            "mesh_json_is_string",
            mesh_rt::mesh_json_is_string as *const (),
        ),
        ("mesh_json_null", mesh_rt::json::mesh_json_null as *const ()),
        (
            "mesh_json_object_get",
            mesh_rt::json::mesh_json_object_get as *const (),
        ),
        (
            "mesh_json_object_new",
            mesh_rt::json::mesh_json_object_new as *const (),
        ),
        (
            "mesh_json_object_put",
            mesh_rt::json::mesh_json_object_put as *const (),
        ),
        ("mesh_json_parse", mesh_rt::mesh_json_parse as *const ()),
        (
            "mesh_json_parse_raw",
            mesh_rt::json::mesh_json_parse_raw as *const (),
        ),
        (
            "mesh_json_to_list",
            mesh_rt::json::mesh_json_to_list as *const (),
        ),
        (
            "mesh_json_to_map",
            mesh_rt::json::mesh_json_to_map as *const (),
        ),
        (
            "mesh_json_value_as_bool",
            mesh_rt::json::mesh_json_value_as_bool as *const (),
        ),
        (
            "mesh_json_value_as_float",
            mesh_rt::json::mesh_json_value_as_float as *const (),
        ),
        (
            "mesh_json_value_as_int",
            mesh_rt::json::mesh_json_value_as_int as *const (),
        ),
        (
            "mesh_library_invoke",
            mesh_rt::library::mesh_library_invoke as *const (),
        ),
        ("mesh_list_all", mesh_rt::mesh_list_all as *const ()),
        ("mesh_list_any", mesh_rt::mesh_list_any as *const ()),
        ("mesh_list_append", mesh_rt::mesh_list_append as *const ()),
        (
            "mesh_list_builder_new",
            mesh_rt::collections::list::mesh_list_builder_new as *const (),
        ),
        (
            "mesh_list_builder_push",
            mesh_rt::collections::list::mesh_list_builder_push as *const (),
        ),
        ("mesh_list_collect", mesh_rt::mesh_list_collect as *const ()),
        (
            "mesh_list_compare",
            mesh_rt::collections::list::mesh_list_compare as *const (),
        ),
        ("mesh_list_concat", mesh_rt::mesh_list_concat as *const ()),
        (
            "mesh_list_contains",
            mesh_rt::mesh_list_contains as *const (),
        ),
        (
            "mesh_list_contains_by",
            mesh_rt::collections::list::mesh_list_contains_by as *const (),
        ),
        (
            "mesh_list_contains_str",
            mesh_rt::mesh_list_contains_str as *const (),
        ),
        ("mesh_list_drop", mesh_rt::mesh_list_drop as *const ()),
        (
            "mesh_list_enumerate",
            mesh_rt::mesh_list_enumerate as *const (),
        ),
        (
            "mesh_list_eq",
            mesh_rt::collections::list::mesh_list_eq as *const (),
        ),
        ("mesh_list_filter", mesh_rt::mesh_list_filter as *const ()),
        ("mesh_list_find", mesh_rt::mesh_list_find as *const ()),
        (
            "mesh_list_flat_map",
            mesh_rt::mesh_list_flat_map as *const (),
        ),
        ("mesh_list_flatten", mesh_rt::mesh_list_flatten as *const ()),
        (
            "mesh_list_from_array",
            mesh_rt::mesh_list_from_array as *const (),
        ),
        ("mesh_list_get", mesh_rt::mesh_list_get as *const ()),
        (
            "mesh_list_hash_by",
            mesh_rt::collections::list::mesh_list_hash_by as *const (),
        ),
        ("mesh_list_head", mesh_rt::mesh_list_head as *const ()),
        (
            "mesh_list_iter_new",
            mesh_rt::collections::list::mesh_list_iter_new as *const (),
        ),
        (
            "mesh_list_iter_next",
            mesh_rt::collections::list::mesh_list_iter_next as *const (),
        ),
        ("mesh_list_last", mesh_rt::mesh_list_last as *const ()),
        ("mesh_list_length", mesh_rt::mesh_list_length as *const ()),
        ("mesh_list_map", mesh_rt::mesh_list_map as *const ()),
        ("mesh_list_new", mesh_rt::mesh_list_new as *const ()),
        ("mesh_list_nth", mesh_rt::mesh_list_nth as *const ()),
        ("mesh_list_reduce", mesh_rt::mesh_list_reduce as *const ()),
        ("mesh_list_reverse", mesh_rt::mesh_list_reverse as *const ()),
        ("mesh_list_sort", mesh_rt::mesh_list_sort as *const ()),
        ("mesh_list_tail", mesh_rt::mesh_list_tail as *const ()),
        ("mesh_list_take", mesh_rt::mesh_list_take as *const ()),
        (
            "mesh_list_to_string",
            mesh_rt::collections::list::mesh_list_to_string as *const (),
        ),
        ("mesh_list_zip", mesh_rt::mesh_list_zip as *const ()),
        ("mesh_map_collect", mesh_rt::mesh_map_collect as *const ()),
        (
            "mesh_map_collect_by",
            mesh_rt::iter::mesh_map_collect_by as *const (),
        ),
        (
            "mesh_map_collect_string_keys",
            mesh_rt::mesh_map_collect_string_keys as *const (),
        ),
        ("mesh_map_delete", mesh_rt::mesh_map_delete as *const ()),
        (
            "mesh_map_delete_by",
            mesh_rt::collections::map::mesh_map_delete_by as *const (),
        ),
        (
            "mesh_map_entry_key",
            mesh_rt::collections::map::mesh_map_entry_key as *const (),
        ),
        (
            "mesh_map_entry_value",
            mesh_rt::collections::map::mesh_map_entry_value as *const (),
        ),
        (
            "mesh_map_eq",
            mesh_rt::collections::map::mesh_map_eq as *const (),
        ),
        (
            "mesh_map_eq_by",
            mesh_rt::collections::map::mesh_map_eq_by as *const (),
        ),
        (
            "mesh_map_from_list",
            mesh_rt::mesh_map_from_list as *const (),
        ),
        (
            "mesh_map_from_list_by",
            mesh_rt::collections::map::mesh_map_from_list_by as *const (),
        ),
        (
            "mesh_map_fetch",
            mesh_rt::collections::map::mesh_map_fetch as *const (),
        ),
        (
            "mesh_map_fetch_by",
            mesh_rt::collections::map::mesh_map_fetch_by as *const (),
        ),
        ("mesh_map_get", mesh_rt::mesh_map_get as *const ()),
        (
            "mesh_map_get_by",
            mesh_rt::collections::map::mesh_map_get_by as *const (),
        ),
        ("mesh_map_has_key", mesh_rt::mesh_map_has_key as *const ()),
        (
            "mesh_map_has_key_by",
            mesh_rt::collections::map::mesh_map_has_key_by as *const (),
        ),
        (
            "mesh_map_hash_by",
            mesh_rt::collections::map::mesh_map_hash_by as *const (),
        ),
        (
            "mesh_map_iter_new",
            mesh_rt::collections::map::mesh_map_iter_new as *const (),
        ),
        (
            "mesh_map_iter_next",
            mesh_rt::collections::map::mesh_map_iter_next as *const (),
        ),
        ("mesh_map_keys", mesh_rt::mesh_map_keys as *const ()),
        ("mesh_map_merge", mesh_rt::mesh_map_merge as *const ()),
        (
            "mesh_map_merge_by",
            mesh_rt::collections::map::mesh_map_merge_by as *const (),
        ),
        ("mesh_map_new", mesh_rt::mesh_map_new as *const ()),
        (
            "mesh_map_new_typed",
            mesh_rt::collections::map::mesh_map_new_typed as *const (),
        ),
        ("mesh_map_put", mesh_rt::mesh_map_put as *const ()),
        (
            "mesh_map_put_by",
            mesh_rt::collections::map::mesh_map_put_by as *const (),
        ),
        ("mesh_map_size", mesh_rt::mesh_map_size as *const ()),
        (
            "mesh_map_tag_string",
            mesh_rt::collections::map::mesh_map_tag_string as *const (),
        ),
        ("mesh_map_to_list", mesh_rt::mesh_map_to_list as *const ()),
        (
            "mesh_map_to_string",
            mesh_rt::collections::map::mesh_map_to_string as *const (),
        ),
        ("mesh_map_values", mesh_rt::mesh_map_values as *const ()),
        (
            "mesh_migration_add_column",
            mesh_rt::mesh_migration_add_column as *const (),
        ),
        (
            "mesh_migration_create_index",
            mesh_rt::mesh_migration_create_index as *const (),
        ),
        (
            "mesh_migration_create_table",
            mesh_rt::mesh_migration_create_table as *const (),
        ),
        (
            "mesh_migration_drop_column",
            mesh_rt::mesh_migration_drop_column as *const (),
        ),
        (
            "mesh_migration_drop_index",
            mesh_rt::mesh_migration_drop_index as *const (),
        ),
        (
            "mesh_migration_drop_table",
            mesh_rt::mesh_migration_drop_table as *const (),
        ),
        (
            "mesh_migration_execute",
            mesh_rt::mesh_migration_execute as *const (),
        ),
        (
            "mesh_migration_rename_column",
            mesh_rt::mesh_migration_rename_column as *const (),
        ),
        (
            "mesh_mlkem_private_key_seal_for_storage",
            mesh_rt::storage_wrapping::mesh_mlkem_private_key_seal_for_storage as *const (),
        ),
        (
            "mesh_mlkem_private_key_unseal_from_storage",
            mesh_rt::storage_wrapping::mesh_mlkem_private_key_unseal_from_storage as *const (),
        ),
        (
            "mesh_monotonic_elapsed",
            mesh_rt::monotonic::mesh_monotonic_elapsed as *const (),
        ),
        (
            "mesh_monotonic_now_nanos",
            mesh_rt::monotonic::mesh_monotonic_now_nanos as *const (),
        ),
        (
            "mesh_node_connect",
            mesh_rt::dist::node::mesh_node_connect as *const (),
        ),
        (
            "mesh_node_list",
            mesh_rt::dist::node::mesh_node_list as *const (),
        ),
        (
            "mesh_node_monitor",
            mesh_rt::actor::mesh_node_monitor as *const (),
        ),
        (
            "mesh_node_self",
            mesh_rt::dist::node::mesh_node_self as *const (),
        ),
        (
            "mesh_node_spawn",
            mesh_rt::dist::node::mesh_node_spawn as *const (),
        ),
        (
            "mesh_node_start",
            mesh_rt::dist::node::mesh_node_start as *const (),
        ),
        (
            "mesh_node_start_from_env",
            mesh_rt::dist::node::mesh_node_start_from_env as *const (),
        ),
        (
            "mesh_option_box_scalar",
            mesh_rt::option::mesh_option_box_scalar as *const (),
        ),
        (
            "mesh_orm_build_delete",
            mesh_rt::mesh_orm_build_delete as *const (),
        ),
        (
            "mesh_orm_build_insert",
            mesh_rt::mesh_orm_build_insert as *const (),
        ),
        (
            "mesh_orm_build_select",
            mesh_rt::mesh_orm_build_select as *const (),
        ),
        (
            "mesh_orm_build_update",
            mesh_rt::mesh_orm_build_update as *const (),
        ),
        ("mesh_panic", mesh_rt::mesh_panic as *const ()),
        (
            "mesh_panic_str",
            mesh_rt::panic::mesh_panic_str as *const (),
        ),
        ("mesh_pg_begin", mesh_rt::db::pg::mesh_pg_begin as *const ()),
        ("mesh_pg_cast", mesh_rt::db::expr::mesh_pg_cast as *const ()),
        ("mesh_pg_close", mesh_rt::db::pg::mesh_pg_close as *const ()),
        (
            "mesh_pg_commit",
            mesh_rt::db::pg::mesh_pg_commit as *const (),
        ),
        (
            "mesh_pg_connect",
            mesh_rt::db::pg::mesh_pg_connect as *const (),
        ),
        (
            "mesh_pg_create_daily_partitions_ahead",
            mesh_rt::mesh_pg_create_daily_partitions_ahead as *const (),
        ),
        (
            "mesh_pg_create_extension",
            mesh_rt::mesh_pg_create_extension as *const (),
        ),
        (
            "mesh_pg_create_gin_index",
            mesh_rt::mesh_pg_create_gin_index as *const (),
        ),
        (
            "mesh_pg_create_range_partitioned_table",
            mesh_rt::mesh_pg_create_range_partitioned_table as *const (),
        ),
        (
            "mesh_pg_crypt",
            mesh_rt::db::expr::mesh_pg_crypt as *const (),
        ),
        (
            "mesh_pg_drop_partition",
            mesh_rt::mesh_pg_drop_partition as *const (),
        ),
        (
            "mesh_pg_execute",
            mesh_rt::db::pg::mesh_pg_execute as *const (),
        ),
        (
            "mesh_pg_execute_values",
            mesh_rt::db::pg::mesh_pg_execute_values as *const (),
        ),
        (
            "mesh_pg_gen_salt",
            mesh_rt::db::expr::mesh_pg_gen_salt as *const (),
        ),
        ("mesh_pg_int", mesh_rt::db::expr::mesh_pg_int as *const ()),
        (
            "mesh_pg_jsonb",
            mesh_rt::db::expr::mesh_pg_jsonb as *const (),
        ),
        (
            "mesh_pg_jsonb_contains",
            mesh_rt::db::expr::mesh_pg_jsonb_contains as *const (),
        ),
        (
            "mesh_pg_list_daily_partitions_before",
            mesh_rt::mesh_pg_list_daily_partitions_before as *const (),
        ),
        (
            "mesh_pg_plainto_tsquery",
            mesh_rt::db::expr::mesh_pg_plainto_tsquery as *const (),
        ),
        ("mesh_pg_query", mesh_rt::db::pg::mesh_pg_query as *const ()),
        (
            "mesh_pg_query_as",
            mesh_rt::db::pg::mesh_pg_query_as as *const (),
        ),
        (
            "mesh_pg_query_values",
            mesh_rt::db::pg::mesh_pg_query_values as *const (),
        ),
        (
            "mesh_pg_rollback",
            mesh_rt::db::pg::mesh_pg_rollback as *const (),
        ),
        ("mesh_pg_text", mesh_rt::db::expr::mesh_pg_text as *const ()),
        (
            "mesh_pg_timestamptz",
            mesh_rt::db::expr::mesh_pg_timestamptz as *const (),
        ),
        (
            "mesh_pg_to_tsvector",
            mesh_rt::db::expr::mesh_pg_to_tsvector as *const (),
        ),
        (
            "mesh_pg_transaction",
            mesh_rt::db::pg::mesh_pg_transaction as *const (),
        ),
        (
            "mesh_pg_ts_rank",
            mesh_rt::db::expr::mesh_pg_ts_rank as *const (),
        ),
        (
            "mesh_pg_tsvector_matches",
            mesh_rt::db::expr::mesh_pg_tsvector_matches as *const (),
        ),
        ("mesh_pg_uuid", mesh_rt::db::expr::mesh_pg_uuid as *const ()),
        (
            "mesh_pool_close",
            mesh_rt::db::pool::mesh_pool_close as *const (),
        ),
        (
            "mesh_pool_execute",
            mesh_rt::db::pool::mesh_pool_execute as *const (),
        ),
        (
            "mesh_pool_execute_values",
            mesh_rt::db::pool::mesh_pool_execute_values as *const (),
        ),
        (
            "mesh_pool_open",
            mesh_rt::db::pool::mesh_pool_open as *const (),
        ),
        (
            "mesh_pool_query",
            mesh_rt::db::pool::mesh_pool_query as *const (),
        ),
        (
            "mesh_pool_query_as",
            mesh_rt::db::pool::mesh_pool_query_as as *const (),
        ),
        (
            "mesh_pool_query_values",
            mesh_rt::db::pool::mesh_pool_query_values as *const (),
        ),
        ("mesh_print", mesh_rt::mesh_print as *const ()),
        ("mesh_println", mesh_rt::mesh_println as *const ()),
        (
            "mesh_process_demonitor",
            mesh_rt::mesh_process_demonitor as *const (),
        ),
        (
            "mesh_process_exit",
            mesh_rt::process_signal::mesh_process_exit as *const (),
        ),
        (
            "mesh_process_install_shutdown_signals",
            mesh_rt::process_signal::mesh_process_install_shutdown_signals as *const (),
        ),
        (
            "mesh_process_monitor",
            mesh_rt::mesh_process_monitor as *const (),
        ),
        (
            "mesh_process_register",
            mesh_rt::actor::mesh_process_register as *const (),
        ),
        (
            "mesh_process_request_shutdown",
            mesh_rt::process_signal::mesh_process_request_shutdown as *const (),
        ),
        (
            "mesh_process_shutdown_requested",
            mesh_rt::process_signal::mesh_process_shutdown_requested as *const (),
        ),
        (
            "mesh_process_whereis",
            mesh_rt::actor::mesh_process_whereis as *const (),
        ),
        (
            "mesh_query_fragment",
            mesh_rt::mesh_query_fragment as *const (),
        ),
        ("mesh_query_from", mesh_rt::mesh_query_from as *const ()),
        (
            "mesh_query_group_by",
            mesh_rt::mesh_query_group_by as *const (),
        ),
        (
            "mesh_query_group_by_raw",
            mesh_rt::mesh_query_group_by_raw as *const (),
        ),
        ("mesh_query_having", mesh_rt::mesh_query_having as *const ()),
        ("mesh_query_join", mesh_rt::mesh_query_join as *const ()),
        (
            "mesh_query_join_as",
            mesh_rt::mesh_query_join_as as *const (),
        ),
        ("mesh_query_limit", mesh_rt::mesh_query_limit as *const ()),
        ("mesh_query_offset", mesh_rt::mesh_query_offset as *const ()),
        (
            "mesh_query_order_by",
            mesh_rt::mesh_query_order_by as *const (),
        ),
        (
            "mesh_query_order_by_raw",
            mesh_rt::mesh_query_order_by_raw as *const (),
        ),
        ("mesh_query_select", mesh_rt::mesh_query_select as *const ()),
        (
            "mesh_query_select_avg",
            mesh_rt::mesh_query_select_avg as *const (),
        ),
        (
            "mesh_query_select_count",
            mesh_rt::mesh_query_select_count as *const (),
        ),
        (
            "mesh_query_select_count_field",
            mesh_rt::mesh_query_select_count_field as *const (),
        ),
        (
            "mesh_query_select_expr",
            mesh_rt::db::query::mesh_query_select_expr as *const (),
        ),
        (
            "mesh_query_select_exprs",
            mesh_rt::db::query::mesh_query_select_exprs as *const (),
        ),
        (
            "mesh_query_select_max",
            mesh_rt::mesh_query_select_max as *const (),
        ),
        (
            "mesh_query_select_min",
            mesh_rt::mesh_query_select_min as *const (),
        ),
        (
            "mesh_query_select_raw",
            mesh_rt::mesh_query_select_raw as *const (),
        ),
        (
            "mesh_query_select_sum",
            mesh_rt::mesh_query_select_sum as *const (),
        ),
        ("mesh_query_where", mesh_rt::mesh_query_where as *const ()),
        (
            "mesh_query_where_between",
            mesh_rt::mesh_query_where_between as *const (),
        ),
        (
            "mesh_query_where_expr",
            mesh_rt::db::query::mesh_query_where_expr as *const (),
        ),
        (
            "mesh_query_where_in",
            mesh_rt::mesh_query_where_in as *const (),
        ),
        (
            "mesh_query_where_not_in",
            mesh_rt::mesh_query_where_not_in as *const (),
        ),
        (
            "mesh_query_where_not_null",
            mesh_rt::mesh_query_where_not_null as *const (),
        ),
        (
            "mesh_query_where_null",
            mesh_rt::mesh_query_where_null as *const (),
        ),
        (
            "mesh_query_where_op",
            mesh_rt::mesh_query_where_op as *const (),
        ),
        (
            "mesh_query_where_or",
            mesh_rt::mesh_query_where_or as *const (),
        ),
        (
            "mesh_query_where_raw",
            mesh_rt::mesh_query_where_raw as *const (),
        ),
        (
            "mesh_query_where_sub",
            mesh_rt::mesh_query_where_sub as *const (),
        ),
        (
            "mesh_queue_is_empty",
            mesh_rt::mesh_queue_is_empty as *const (),
        ),
        ("mesh_queue_new", mesh_rt::mesh_queue_new as *const ()),
        ("mesh_queue_peek", mesh_rt::mesh_queue_peek as *const ()),
        ("mesh_queue_pop", mesh_rt::mesh_queue_pop as *const ()),
        ("mesh_queue_push", mesh_rt::mesh_queue_push as *const ()),
        ("mesh_queue_size", mesh_rt::mesh_queue_size as *const ()),
        (
            "mesh_random_next_int",
            mesh_rt::random::mesh_random_next_int as *const (),
        ),
        (
            "mesh_random_next_unit_ppm",
            mesh_rt::random::mesh_random_next_unit_ppm as *const (),
        ),
        (
            "mesh_random_seed",
            mesh_rt::random::mesh_random_seed as *const (),
        ),
        ("mesh_range_filter", mesh_rt::mesh_range_filter as *const ()),
        (
            "mesh_range_iter",
            mesh_rt::collections::range::mesh_range_iter as *const (),
        ),
        (
            "mesh_range_iter_new",
            mesh_rt::collections::range::mesh_range_iter_new as *const (),
        ),
        (
            "mesh_range_iter_next",
            mesh_rt::collections::range::mesh_range_iter_next as *const (),
        ),
        ("mesh_range_length", mesh_rt::mesh_range_length as *const ()),
        ("mesh_range_map", mesh_rt::mesh_range_map as *const ()),
        ("mesh_range_new", mesh_rt::mesh_range_new as *const ()),
        (
            "mesh_range_to_list",
            mesh_rt::mesh_range_to_list as *const (),
        ),
        (
            "mesh_reduction_check",
            mesh_rt::mesh_reduction_check as *const (),
        ),
        (
            "mesh_regex_captures",
            mesh_rt::mesh_regex_captures as *const (),
        ),
        (
            "mesh_regex_compile",
            mesh_rt::mesh_regex_compile as *const (),
        ),
        (
            "mesh_regex_from_literal",
            mesh_rt::mesh_regex_from_literal as *const (),
        ),
        ("mesh_regex_match", mesh_rt::mesh_regex_match as *const ()),
        (
            "mesh_regex_replace",
            mesh_rt::mesh_regex_replace as *const (),
        ),
        ("mesh_regex_split", mesh_rt::mesh_regex_split as *const ()),
        (
            "mesh_register_autonomous_config_json",
            mesh_rt::dist::autonomous::mesh_register_autonomous_config_json as *const (),
        ),
        (
            "mesh_register_declared_handler",
            mesh_rt::dist::node::mesh_register_declared_handler as *const (),
        ),
        (
            "mesh_register_function",
            mesh_rt::dist::node::mesh_register_function as *const (),
        ),
        (
            "mesh_register_startup_work",
            mesh_rt::dist::node::mesh_register_startup_work as *const (),
        ),
        ("mesh_repo_all", mesh_rt::mesh_repo_all as *const ()),
        ("mesh_repo_count", mesh_rt::mesh_repo_count as *const ()),
        ("mesh_repo_delete", mesh_rt::mesh_repo_delete as *const ()),
        (
            "mesh_repo_delete_where",
            mesh_rt::mesh_repo_delete_where as *const (),
        ),
        (
            "mesh_repo_delete_where_returning",
            mesh_rt::mesh_repo_delete_where_returning as *const (),
        ),
        (
            "mesh_repo_execute_raw",
            mesh_rt::mesh_repo_execute_raw as *const (),
        ),
        ("mesh_repo_exists", mesh_rt::mesh_repo_exists as *const ()),
        ("mesh_repo_get", mesh_rt::mesh_repo_get as *const ()),
        ("mesh_repo_get_by", mesh_rt::mesh_repo_get_by as *const ()),
        ("mesh_repo_insert", mesh_rt::mesh_repo_insert as *const ()),
        (
            "mesh_repo_insert_changeset",
            mesh_rt::mesh_repo_insert_changeset as *const (),
        ),
        (
            "mesh_repo_insert_expr",
            mesh_rt::db::repo::mesh_repo_insert_expr as *const (),
        ),
        (
            "mesh_repo_insert_or_update",
            mesh_rt::mesh_repo_insert_or_update as *const (),
        ),
        (
            "mesh_repo_insert_or_update_expr",
            mesh_rt::db::repo::mesh_repo_insert_or_update_expr as *const (),
        ),
        ("mesh_repo_one", mesh_rt::mesh_repo_one as *const ()),
        ("mesh_repo_preload", mesh_rt::mesh_repo_preload as *const ()),
        (
            "mesh_repo_query_raw",
            mesh_rt::mesh_repo_query_raw as *const (),
        ),
        (
            "mesh_repo_transaction",
            mesh_rt::mesh_repo_transaction as *const (),
        ),
        ("mesh_repo_update", mesh_rt::mesh_repo_update as *const ()),
        (
            "mesh_repo_update_changeset",
            mesh_rt::mesh_repo_update_changeset as *const (),
        ),
        (
            "mesh_repo_update_where",
            mesh_rt::mesh_repo_update_where as *const (),
        ),
        (
            "mesh_repo_update_where_expr",
            mesh_rt::db::repo::mesh_repo_update_where_expr as *const (),
        ),
        (
            "mesh_resource_destroy",
            mesh_rt::secret::mesh_resource_destroy as *const (),
        ),
        (
            "mesh_result_is_ok",
            mesh_rt::io::mesh_result_is_ok as *const (),
        ),
        (
            "mesh_result_unwrap",
            mesh_rt::io::mesh_result_unwrap as *const (),
        ),
        (
            "mesh_row_from_row_get",
            mesh_rt::db::row::mesh_row_from_row_get as *const (),
        ),
        (
            "mesh_row_parse_bool",
            mesh_rt::db::row::mesh_row_parse_bool as *const (),
        ),
        (
            "mesh_row_parse_float",
            mesh_rt::db::row::mesh_row_parse_float as *const (),
        ),
        (
            "mesh_row_parse_int",
            mesh_rt::db::row::mesh_row_parse_int as *const (),
        ),
        ("mesh_rt_init", mesh_rt::mesh_rt_init as *const ()),
        (
            "mesh_rt_init_actor",
            mesh_rt::mesh_rt_init_actor as *const (),
        ),
        (
            "mesh_rt_run_scheduler",
            mesh_rt::mesh_rt_run_scheduler as *const (),
        ),
        ("mesh_run_main", mesh_rt::panic::mesh_run_main as *const ()),
        (
            "mesh_secret_concat",
            mesh_rt::secret::mesh_secret_concat as *const (),
        ),
        (
            "mesh_secret_destroy",
            mesh_rt::secret::mesh_secret_destroy as *const (),
        ),
        (
            "mesh_secret_map_contains",
            mesh_rt::secret::mesh_secret_map_contains as *const (),
        ),
        (
            "mesh_secret_map_copy",
            mesh_rt::secret::mesh_secret_map_copy as *const (),
        ),
        (
            "mesh_secret_map_delete",
            mesh_rt::secret::mesh_secret_map_delete as *const (),
        ),
        (
            "mesh_secret_map_fork",
            mesh_rt::secret::mesh_secret_map_fork as *const (),
        ),
        (
            "mesh_secret_map_insert",
            mesh_rt::secret::mesh_secret_map_insert as *const (),
        ),
        (
            "mesh_secret_map_merge",
            mesh_rt::secret::mesh_secret_map_merge as *const (),
        ),
        (
            "mesh_secret_map_new",
            mesh_rt::secret::mesh_secret_map_new as *const (),
        ),
        (
            "mesh_secret_map_seal_for_storage",
            mesh_rt::storage_wrapping::mesh_secret_map_seal_for_storage as *const (),
        ),
        (
            "mesh_secret_map_unseal_from_storage",
            mesh_rt::storage_wrapping::mesh_secret_map_unseal_from_storage as *const (),
        ),
        (
            "mesh_secret_random",
            mesh_rt::secret::mesh_secret_random as *const (),
        ),
        (
            "mesh_secret_seal_for_storage",
            mesh_rt::storage_wrapping::mesh_secret_seal_for_storage as *const (),
        ),
        (
            "mesh_secret_unseal_from_storage",
            mesh_rt::storage_wrapping::mesh_secret_unseal_from_storage as *const (),
        ),
        ("mesh_service_call", mesh_rt::mesh_service_call as *const ()),
        (
            "mesh_service_call_shaped",
            mesh_rt::mesh_service_call_shaped as *const (),
        ),
        (
            "mesh_service_cast_shaped",
            mesh_rt::mesh_service_cast_shaped as *const (),
        ),
        (
            "mesh_service_reply",
            mesh_rt::mesh_service_reply as *const (),
        ),
        (
            "mesh_service_reply_shaped",
            mesh_rt::mesh_service_reply_shaped as *const (),
        ),
        ("mesh_set_add", mesh_rt::mesh_set_add as *const ()),
        ("mesh_set_collect", mesh_rt::mesh_set_collect as *const ()),
        ("mesh_set_contains", mesh_rt::mesh_set_contains as *const ()),
        (
            "mesh_set_difference",
            mesh_rt::mesh_set_difference as *const (),
        ),
        (
            "mesh_set_element_at",
            mesh_rt::collections::set::mesh_set_element_at as *const (),
        ),
        (
            "mesh_set_eq",
            mesh_rt::collections::set::mesh_set_eq as *const (),
        ),
        (
            "mesh_set_from_list",
            mesh_rt::mesh_set_from_list as *const (),
        ),
        (
            "mesh_set_hash_by",
            mesh_rt::collections::set::mesh_set_hash_by as *const (),
        ),
        (
            "mesh_set_intersection",
            mesh_rt::mesh_set_intersection as *const (),
        ),
        (
            "mesh_set_iter_new",
            mesh_rt::collections::set::mesh_set_iter_new as *const (),
        ),
        (
            "mesh_set_iter_next",
            mesh_rt::collections::set::mesh_set_iter_next as *const (),
        ),
        ("mesh_set_new", mesh_rt::mesh_set_new as *const ()),
        ("mesh_set_remove", mesh_rt::mesh_set_remove as *const ()),
        ("mesh_set_size", mesh_rt::mesh_set_size as *const ()),
        ("mesh_set_to_list", mesh_rt::mesh_set_to_list as *const ()),
        (
            "mesh_set_to_string",
            mesh_rt::collections::set::mesh_set_to_string as *const (),
        ),
        ("mesh_set_union", mesh_rt::mesh_set_union as *const ()),
        (
            "mesh_signing_private_key_seal_for_storage",
            mesh_rt::storage_wrapping::mesh_signing_private_key_seal_for_storage as *const (),
        ),
        (
            "mesh_signing_private_key_unseal_from_storage",
            mesh_rt::storage_wrapping::mesh_signing_private_key_unseal_from_storage as *const (),
        ),
        (
            "mesh_sqlite_begin",
            mesh_rt::db::sqlite::mesh_sqlite_begin as *const (),
        ),
        (
            "mesh_sqlite_close",
            mesh_rt::db::sqlite::mesh_sqlite_close as *const (),
        ),
        (
            "mesh_sqlite_commit",
            mesh_rt::db::sqlite::mesh_sqlite_commit as *const (),
        ),
        (
            "mesh_sqlite_execute",
            mesh_rt::db::sqlite::mesh_sqlite_execute as *const (),
        ),
        (
            "mesh_sqlite_execute_values",
            mesh_rt::db::sqlite::mesh_sqlite_execute_values as *const (),
        ),
        (
            "mesh_sqlite_open",
            mesh_rt::db::sqlite::mesh_sqlite_open as *const (),
        ),
        (
            "mesh_sqlite_query",
            mesh_rt::db::sqlite::mesh_sqlite_query as *const (),
        ),
        (
            "mesh_sqlite_query_values",
            mesh_rt::db::sqlite::mesh_sqlite_query_values as *const (),
        ),
        (
            "mesh_sqlite_rollback",
            mesh_rt::db::sqlite::mesh_sqlite_rollback as *const (),
        ),
        (
            "mesh_storage_key_ephemeral",
            mesh_rt::storage_wrapping::mesh_storage_key_ephemeral as *const (),
        ),
        (
            "mesh_storage_key_platform",
            mesh_rt::storage_wrapping::mesh_storage_key_platform as *const (),
        ),
        (
            "mesh_storage_key_seal_bytes",
            mesh_rt::storage_wrapping::mesh_storage_key_seal_bytes as *const (),
        ),
        (
            "mesh_storage_key_unseal_bytes",
            mesh_rt::storage_wrapping::mesh_storage_key_unseal_bytes as *const (),
        ),
        (
            "mesh_string_collect",
            mesh_rt::mesh_string_collect as *const (),
        ),
        (
            "mesh_string_compare",
            mesh_rt::mesh_string_compare as *const (),
        ),
        (
            "mesh_string_concat",
            mesh_rt::mesh_string_concat as *const (),
        ),
        (
            "mesh_string_contains",
            mesh_rt::mesh_string_contains as *const (),
        ),
        (
            "mesh_string_ends_with",
            mesh_rt::mesh_string_ends_with as *const (),
        ),
        ("mesh_string_eq", mesh_rt::mesh_string_eq as *const ()),
        ("mesh_string_join", mesh_rt::mesh_string_join as *const ()),
        (
            "mesh_string_length",
            mesh_rt::mesh_string_length as *const (),
        ),
        ("mesh_string_new", mesh_rt::mesh_string_new as *const ()),
        (
            "mesh_string_replace",
            mesh_rt::mesh_string_replace as *const (),
        ),
        ("mesh_string_slice", mesh_rt::mesh_string_slice as *const ()),
        ("mesh_string_split", mesh_rt::mesh_string_split as *const ()),
        (
            "mesh_string_starts_with",
            mesh_rt::mesh_string_starts_with as *const (),
        ),
        (
            "mesh_string_to_float",
            mesh_rt::mesh_string_to_float as *const (),
        ),
        (
            "mesh_string_to_int",
            mesh_rt::mesh_string_to_int as *const (),
        ),
        (
            "mesh_string_to_lower",
            mesh_rt::mesh_string_to_lower as *const (),
        ),
        (
            "mesh_string_inspect",
            mesh_rt::string::mesh_string_inspect as *const (),
        ),
        (
            "mesh_string_to_string",
            mesh_rt::string::mesh_string_to_string as *const (),
        ),
        (
            "mesh_string_to_upper",
            mesh_rt::mesh_string_to_upper as *const (),
        ),
        ("mesh_string_trim", mesh_rt::mesh_string_trim as *const ()),
        (
            "mesh_supervisor_count_children",
            mesh_rt::actor::mesh_supervisor_count_children as *const (),
        ),
        (
            "mesh_supervisor_start",
            mesh_rt::actor::mesh_supervisor_start as *const (),
        ),
        (
            "mesh_supervisor_start_child",
            mesh_rt::actor::mesh_supervisor_start_child as *const (),
        ),
        (
            "mesh_supervisor_terminate_child",
            mesh_rt::actor::mesh_supervisor_terminate_child as *const (),
        ),
        (
            "mesh_test_assert",
            mesh_rt::test::mesh_test_assert as *const (),
        ),
        (
            "mesh_test_assert_eq",
            mesh_rt::test::mesh_test_assert_eq as *const (),
        ),
        (
            "mesh_test_assert_ne",
            mesh_rt::test::mesh_test_assert_ne as *const (),
        ),
        (
            "mesh_test_assert_raises",
            mesh_rt::test::mesh_test_assert_raises as *const (),
        ),
        (
            "mesh_test_begin",
            mesh_rt::test::mesh_test_begin as *const (),
        ),
        (
            "mesh_test_cleanup_actors",
            mesh_rt::test::mesh_test_cleanup_actors as *const (),
        ),
        (
            "mesh_test_fail_count",
            mesh_rt::test::mesh_test_fail_count as *const (),
        ),
        ("mesh_test_end", mesh_rt::test::mesh_test_end as *const ()),
        (
            "mesh_test_fail_msg",
            mesh_rt::test::mesh_test_fail_msg as *const (),
        ),
        (
            "mesh_test_mock_actor",
            mesh_rt::test::mesh_test_mock_actor as *const (),
        ),
        ("mesh_test_pass", mesh_rt::test::mesh_test_pass as *const ()),
        (
            "mesh_test_pass_count",
            mesh_rt::test::mesh_test_pass_count as *const (),
        ),
        (
            "mesh_test_run_body",
            mesh_rt::test::mesh_test_run_body as *const (),
        ),
        (
            "mesh_test_summary",
            mesh_rt::test::mesh_test_summary as *const (),
        ),
        (
            "mesh_timer_apply_after",
            mesh_rt::mesh_timer_apply_after as *const (),
        ),
        (
            "mesh_timer_send_after",
            mesh_rt::mesh_timer_send_after as *const (),
        ),
        (
            "mesh_timer_send_after_shaped",
            mesh_rt::mesh_timer_send_after_shaped as *const (),
        ),
        ("mesh_timer_sleep", mesh_rt::mesh_timer_sleep as *const ()),
        (
            "mesh_trigger_startup_work",
            mesh_rt::dist::node::mesh_trigger_startup_work as *const (),
        ),
        ("mesh_tuple_first", mesh_rt::mesh_tuple_first as *const ()),
        ("mesh_tuple_nth", mesh_rt::mesh_tuple_nth as *const ()),
        ("mesh_tuple_second", mesh_rt::mesh_tuple_second as *const ()),
        ("mesh_tuple_size", mesh_rt::mesh_tuple_size as *const ()),
        (
            "mesh_u128_add",
            mesh_rt::wide_num::mesh_u128_add as *const (),
        ),
        (
            "mesh_u128_compare",
            mesh_rt::wide_num::mesh_u128_compare as *const (),
        ),
        (
            "mesh_u128_divide",
            mesh_rt::wide_num::mesh_u128_divide as *const (),
        ),
        (
            "mesh_u128_multiply",
            mesh_rt::wide_num::mesh_u128_multiply as *const (),
        ),
        (
            "mesh_u128_parse",
            mesh_rt::wide_num::mesh_u128_parse as *const (),
        ),
        (
            "mesh_u128_subtract",
            mesh_rt::wide_num::mesh_u128_subtract as *const (),
        ),
        (
            "mesh_u128_to_int",
            mesh_rt::wide_num::mesh_u128_to_int as *const (),
        ),
        (
            "mesh_u128_to_string",
            mesh_rt::wide_num::mesh_u128_to_string as *const (),
        ),
        ("mesh_u64_add", mesh_rt::wide_num::mesh_u64_add as *const ()),
        (
            "mesh_u64_compare",
            mesh_rt::wide_num::mesh_u64_compare as *const (),
        ),
        (
            "mesh_u64_divide",
            mesh_rt::wide_num::mesh_u64_divide as *const (),
        ),
        (
            "mesh_u64_multiply",
            mesh_rt::wide_num::mesh_u64_multiply as *const (),
        ),
        (
            "mesh_u64_parse",
            mesh_rt::wide_num::mesh_u64_parse as *const (),
        ),
        (
            "mesh_u64_subtract",
            mesh_rt::wide_num::mesh_u64_subtract as *const (),
        ),
        (
            "mesh_u64_to_int",
            mesh_rt::wide_num::mesh_u64_to_int as *const (),
        ),
        (
            "mesh_u64_to_string",
            mesh_rt::wide_num::mesh_u64_to_string as *const (),
        ),
        (
            "mesh_ws_broadcast",
            mesh_rt::ws::rooms::mesh_ws_broadcast as *const (),
        ),
        (
            "mesh_ws_broadcast_except",
            mesh_rt::ws::rooms::mesh_ws_broadcast_except as *const (),
        ),
        (
            "mesh_ws_client_close",
            mesh_rt::ws::client::mesh_ws_client_close as *const (),
        ),
        (
            "mesh_ws_client_connect",
            mesh_rt::ws::client::mesh_ws_client_connect as *const (),
        ),
        (
            "mesh_ws_client_connect_timeout",
            mesh_rt::ws::client::mesh_ws_client_connect_timeout as *const (),
        ),
        (
            "mesh_ws_client_heartbeat_timeout",
            mesh_rt::ws::client::mesh_ws_client_heartbeat_timeout as *const (),
        ),
        (
            "mesh_ws_client_max_message_bytes",
            mesh_rt::ws::client::mesh_ws_client_max_message_bytes as *const (),
        ),
        (
            "mesh_ws_client_options",
            mesh_rt::ws::client::mesh_ws_client_options as *const (),
        ),
        (
            "mesh_ws_client_queue_capacity",
            mesh_rt::ws::client::mesh_ws_client_queue_capacity as *const (),
        ),
        (
            "mesh_ws_client_reconnect_delay",
            mesh_rt::ws::client::mesh_ws_client_reconnect_delay as *const (),
        ),
        (
            "mesh_ws_client_recv",
            mesh_rt::ws::client::mesh_ws_client_recv as *const (),
        ),
        (
            "mesh_ws_client_send_bytes",
            mesh_rt::ws::client::mesh_ws_client_send_bytes as *const (),
        ),
        (
            "mesh_ws_client_send_text",
            mesh_rt::ws::client::mesh_ws_client_send_text as *const (),
        ),
        (
            "mesh_ws_join",
            mesh_rt::ws::rooms::mesh_ws_join as *const (),
        ),
        (
            "mesh_ws_leave",
            mesh_rt::ws::rooms::mesh_ws_leave as *const (),
        ),
        (
            "mesh_ws_send",
            mesh_rt::ws::server::mesh_ws_send as *const (),
        ),
        (
            "mesh_ws_send_binary",
            mesh_rt::ws::server::mesh_ws_send_binary as *const (),
        ),
        (
            "mesh_ws_serve",
            mesh_rt::ws::server::mesh_ws_serve as *const (),
        ),
        (
            "mesh_ws_serve_tls",
            mesh_rt::ws::server::mesh_ws_serve_tls as *const (),
        ),
        (
            "mesh_x25519_private_key_seal_for_storage",
            mesh_rt::storage_wrapping::mesh_x25519_private_key_seal_for_storage as *const (),
        ),
        (
            "mesh_x25519_private_key_unseal_from_storage",
            mesh_rt::storage_wrapping::mesh_x25519_private_key_unseal_from_storage as *const (),
        ),
    ]
}

/// The result of evaluating an expression in the REPL.
#[derive(Debug, Clone)]
pub struct EvalResult {
    /// String representation of the evaluated value.
    pub value: String,
    /// Type name of the result.
    pub ty: String,
}

/// Keywords that indicate a definition (not an expression to evaluate).
const DEFINITION_KEYWORDS: &[&str] = &[
    "fn",
    "def",
    "let",
    "type",
    "struct",
    "module",
    "actor",
    "service",
    "interface",
    "trait",
    "impl",
    "supervisor",
];

/// Check whether the input appears to be a definition rather than an expression.
///
/// Definitions start with specific keywords (fn, let, type, struct, etc.)
/// and are added to the session context rather than evaluated for a result.
pub fn is_definition(input: &str) -> bool {
    let trimmed = input.trim();
    DEFINITION_KEYWORDS.iter().any(|kw| {
        trimmed.starts_with(kw)
            && trimmed[kw.len()..].starts_with(|c: char| c.is_whitespace() || c == '(')
    })
}

/// Evaluate a Mesh expression or definition using the full compiler pipeline.
///
/// For expressions: wraps in a function, compiles via LLVM JIT, executes, and
/// returns the result value with its type.
///
/// For definitions: adds to the session context and returns a "Defined" result.
///
/// The LLVM Context is created per-evaluation. In a future optimization, the
/// context could be persisted across evaluations for better performance.
pub fn jit_eval(source: &str, session: &mut ReplSession) -> Result<EvalResult, String> {
    let trimmed = source.trim();

    if trimmed.is_empty() {
        return Ok(EvalResult {
            value: String::new(),
            ty: "Unit".to_string(),
        });
    }

    // Detect whether this is a definition or an expression
    if is_definition(trimmed) {
        return eval_definition(trimmed, session);
    }

    eval_expression(trimmed, session)
}

/// Process a definition: validate it parses and type-checks, then store it.
fn eval_definition(input: &str, session: &mut ReplSession) -> Result<EvalResult, String> {
    // Build full source with existing definitions + new one
    let mut full_source = session.definitions_source();
    if !full_source.is_empty() {
        full_source.push('\n');
    }
    full_source.push_str(input);

    // Parse to check for syntax errors
    let parse = mesh_parser::parse(&full_source);
    if !parse.ok() {
        let errors: Vec<String> = parse.errors().iter().map(|e| format!("{}", e)).collect();
        return Err(format!("Parse error: {}", errors.join(", ")));
    }

    // Type check to validate the definition
    let typeck = mesh_typeck::check(&parse);
    if !typeck.errors.is_empty() {
        let rendered = typeck.render_errors(
            &full_source,
            "<repl>",
            &mesh_typeck::diagnostics::DiagnosticOptions::colorless(),
        );
        return Err(rendered.join("\n"));
    }

    // Extract the definition name for display
    let def_name = extract_definition_name(input);

    // Extract the type of the definition for display
    let type_info = if let Some(ref result_ty) = typeck.result_type {
        format!("{}", result_ty)
    } else {
        String::new()
    };

    // Store the definition for future inputs
    session.add_definition(input);

    let display = if !type_info.is_empty() {
        format!("Defined: {} :: {}", def_name, type_info)
    } else {
        format!("Defined: {}", def_name)
    };

    Ok(EvalResult {
        value: display,
        ty: "Definition".to_string(),
    })
}

/// Process an expression: wrap it, compile via full pipeline, and execute via JIT.
fn eval_expression(input: &str, session: &mut ReplSession) -> Result<EvalResult, String> {
    let (full_source, wrapper_fn) = session.wrap_expression(input);

    // Step 1: Parse
    let parse = mesh_parser::parse(&full_source);
    if !parse.ok() {
        let errors: Vec<String> = parse.errors().iter().map(|e| format!("{}", e)).collect();
        return Err(format!("Parse error: {}", errors.join(", ")));
    }

    // Step 2: Type check
    let typeck = mesh_typeck::check(&parse);
    if !typeck.errors.is_empty() {
        let rendered = typeck.render_errors(
            &full_source,
            "<repl>",
            &mesh_typeck::diagnostics::DiagnosticOptions::colorless(),
        );
        return Err(rendered.join("\n"));
    }

    // The wrapper is the last item, so the checker's result type is the
    // wrapper's, `() -> T`; the expression's type is what it returns.
    let result_type_name = match &typeck.result_type {
        Some(mesh_typeck::ty::Ty::Fun(_, ret)) => format!("{}", ret),
        Some(ty) => format!("{}", ty),
        None => "Unit".to_string(),
    };

    // Step 3: Lower to MIR
    let mir = mesh_codegen::lower_to_mir_module(&parse, &typeck)?;

    // Step 4: Generate LLVM IR and execute via JIT
    let value = jit_execute(&mir, &wrapper_fn, &result_type_name)?;

    // Record the result in session history
    session.record_result(value.clone(), result_type_name.clone());

    Ok(EvalResult {
        value,
        ty: result_type_name,
    })
}

/// Compile MIR to LLVM IR and execute the wrapper function via JIT.
///
/// Uses LLVM's JIT execution engine to call the generated wrapper function
/// and capture its return value.
fn jit_execute(
    mir: &mesh_codegen::mir::MirModule,
    wrapper_fn_name: &str,
    result_type: &str,
) -> Result<String, String> {
    use inkwell::context::Context;
    use inkwell::targets::{InitializationConfig, Target};
    use mesh_codegen::codegen::CodeGen;

    // Initialize native target for JIT
    Target::initialize_native(&InitializationConfig::default())
        .map_err(|e| format!("Failed to initialize native target: {}", e))?;

    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "repl_jit", 0, None)?;
    codegen.compile(mir)?;

    // Extract the LLVM module and create a JIT execution engine
    let module = codegen.into_module();
    let ee = module
        .create_jit_execution_engine(inkwell::OptimizationLevel::None)
        .map_err(|e| format!("Failed to create JIT engine: {}", e))?;

    // A Float comes back in a floating-point register, not where an i64 does.
    if result_type == "Float" {
        let jit_fn =
            unsafe { ee.get_function::<unsafe extern "C" fn() -> f64>(wrapper_fn_name) }
                .map_err(|e| format!("Failed to find JIT function '{}': {}", wrapper_fn_name, e))?;
        return Ok(format!("{:?}", unsafe { jit_fn.call() }));
    }

    // Look up the wrapper function
    let maybe_fn = unsafe { ee.get_function::<unsafe extern "C" fn() -> i64>(wrapper_fn_name) };

    match maybe_fn {
        Ok(jit_fn) => {
            let result = unsafe { jit_fn.call() };
            let formatted = format_jit_result(result, result_type);
            Ok(formatted)
        }
        Err(_) => {
            // Function might return void (Unit type)
            let maybe_void_fn =
                unsafe { ee.get_function::<unsafe extern "C" fn()>(wrapper_fn_name) };
            match maybe_void_fn {
                Ok(jit_fn) => {
                    unsafe { jit_fn.call() };
                    Ok("()".to_string())
                }
                Err(e) => Err(format!(
                    "Failed to find JIT function '{}': {}",
                    wrapper_fn_name, e
                )),
            }
        }
    }
}

/// Format a raw JIT result value based on its Mesh type.
fn format_jit_result(raw: i64, type_name: &str) -> String {
    match type_name {
        "Int" => format!("{}", raw),
        "Bool" => {
            if raw != 0 {
                "true".to_string()
            } else {
                "false".to_string()
            }
        }
        "String" if raw != 0 => {
            // The value is a pointer to a runtime string.
            let text = unsafe { (*(raw as *const mesh_rt::MeshString)).as_str() };
            format!("{:?}", text)
        }
        "Unit" | "()" => "()".to_string(),
        _ => format!("<{} at 0x{:x}>", type_name, raw),
    }
}

/// Extract the name from a definition for display.
fn extract_definition_name(input: &str) -> String {
    let trimmed = input.trim();
    let tokens: Vec<&str> = trimmed.split_whitespace().collect();
    if tokens.len() >= 2 {
        // Handle "fn name(...)" by stripping parens
        let name = tokens[1];
        if let Some(paren_pos) = name.find('(') {
            name[..paren_pos].to_string()
        } else {
            name.to_string()
        }
    } else {
        "<anonymous>".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_declared_runtime_function_is_registered() {
        let context = inkwell::context::Context::create();
        let module = context.create_module("symbols");
        mesh_codegen::codegen::intrinsics::declare_intrinsics(&module);
        let registered: std::collections::HashSet<&str> = runtime_symbols()
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        // Provided by mesh-test-rt, which only `meshc test` links.
        let test_only = [
            "mesh_test_install_in_memory_secure_store",
            "mesh_test_set_push_token",
        ];
        let missing: Vec<String> = module
            .get_functions()
            .map(|f| f.get_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with("mesh_"))
            .filter(|name| !registered.contains(name.as_str()))
            .filter(|name| !test_only.contains(&name.as_str()))
            .collect();
        assert!(
            missing.is_empty(),
            "not registered with the JIT: {missing:?}"
        );
    }

    #[test]
    fn test_is_definition_fn() {
        assert!(is_definition("fn foo() do 1 end"));
        assert!(is_definition("fn bar(x :: Int) :: Int do x end"));
        assert!(is_definition("  fn indented() do 1 end"));
    }

    #[test]
    fn test_is_definition_let() {
        assert!(is_definition("let x = 42"));
        assert!(is_definition("let (a, b) = (1, 2)"));
    }

    #[test]
    fn test_is_definition_type() {
        assert!(is_definition("type Color do Red | Green | Blue end"));
        assert!(is_definition("struct Point do x :: Int y :: Int end"));
    }

    #[test]
    fn test_is_definition_others() {
        assert!(is_definition("module Foo do end"));
        assert!(is_definition("actor Counter do end"));
        assert!(is_definition("service Cache do end"));
        assert!(is_definition("interface Printable do end"));
    }

    #[test]
    fn test_is_not_definition() {
        assert!(!is_definition("1 + 2"));
        assert!(!is_definition("foo()"));
        assert!(!is_definition("if true do 1 else 2 end"));
        assert!(!is_definition("x"));
    }

    #[test]
    fn test_extract_definition_name() {
        assert_eq!(extract_definition_name("fn add(a, b) do a + b end"), "add");
        assert_eq!(extract_definition_name("let x = 42"), "x");
        assert_eq!(extract_definition_name("type Color do end"), "Color");
        assert_eq!(extract_definition_name("struct Point do end"), "Point");
    }

    #[test]
    fn test_format_jit_result_int() {
        assert_eq!(format_jit_result(42, "Int"), "42");
        assert_eq!(format_jit_result(-1, "Int"), "-1");
        assert_eq!(format_jit_result(0, "Int"), "0");
    }

    #[test]
    fn test_format_jit_result_bool() {
        assert_eq!(format_jit_result(1, "Bool"), "true");
        assert_eq!(format_jit_result(0, "Bool"), "false");
    }

    #[test]
    fn test_format_jit_result_unit() {
        assert_eq!(format_jit_result(0, "Unit"), "()");
        assert_eq!(format_jit_result(0, "()"), "()");
    }

    #[test]
    fn test_eval_empty_input() {
        let mut session = ReplSession::new();
        let result = jit_eval("", &mut session).unwrap();
        assert_eq!(result.ty, "Unit");
    }

    #[test]
    fn test_eval_whitespace_input() {
        let mut session = ReplSession::new();
        let result = jit_eval("   ", &mut session).unwrap();
        assert_eq!(result.ty, "Unit");
    }

    /// Evaluates real input end to end. Nothing did, and every evaluation
    /// failed in codegen (`<` on strings in a generated helper the REPL, having
    /// no entry point, never prunes), results were labelled with the wrapper's
    /// type, floats were read from the wrong register and `let` bindings were
    /// invisible to later lines.
    #[test]
    fn test_eval_expressions_through_the_jit() {
        init_runtime();
        let mut session = ReplSession::new();
        let mut eval = |input: &str| {
            let result = jit_eval(input, &mut session).unwrap_or_else(|e| panic!("{input}: {e}"));
            format!("{} :: {}", result.value, result.ty)
        };
        assert_eq!(eval("1 + 41"), "42 :: Int");
        assert_eq!(eval("2.5 * 2.0"), "5.0 :: Float");
        assert_eq!(eval("\"apple\" < \"banana\""), "true :: Bool");
        assert_eq!(eval("\"a\" <> \"b-${1 + 1}\""), "\"ab-2\" :: String");
        eval("fn double(n :: Int) -> Int do n * 2 end");
        eval("let twice = fn (n :: Int) -> n * 2 end");
        eval("let x = 4");
        assert_eq!(eval("double(x) + twice(x)"), "16 :: Int");
    }

    #[test]
    fn test_init_runtime_is_idempotent() {
        // Should not panic when called multiple times
        init_runtime();
        init_runtime();
    }

    #[test]
    fn test_repl_config_default() {
        let config = crate::ReplConfig::default();
        assert_eq!(config.prompt, "mesh> ");
        assert_eq!(config.continuation, "  ... ");
    }
}
