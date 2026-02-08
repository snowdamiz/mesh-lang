//! Runtime function declarations in the LLVM module.
//!
//! Declares all `snow-rt` extern "C" functions so that codegen can emit
//! calls to them. These declarations must match the signatures in the
//! `snow-rt` crate exactly.

use inkwell::module::Module;
use inkwell::types::BasicMetadataTypeEnum;
use inkwell::values::FunctionValue;
use inkwell::AddressSpace;

/// Declare all Snow runtime functions in the LLVM module.
///
/// This should be called once during module initialization, before any
/// codegen that might reference runtime functions.
pub fn declare_intrinsics<'ctx>(module: &Module<'ctx>) {
    let context = module.get_context();
    let void_type = context.void_type();
    let i8_type = context.i8_type();
    let i32_type = context.i32_type();
    let i64_type = context.i64_type();
    let f64_type = context.f64_type();
    let ptr_type = context.ptr_type(AddressSpace::default());

    // snow_rt_init() -> void
    let init_ty = void_type.fn_type(&[], false);
    module.add_function("snow_rt_init", init_ty, Some(inkwell::module::Linkage::External));

    // snow_gc_alloc_actor(size: u64, align: u64) -> ptr
    let gc_alloc_ty = ptr_type.fn_type(
        &[i64_type.into(), i64_type.into()],
        false,
    );
    module.add_function("snow_gc_alloc_actor", gc_alloc_ty, Some(inkwell::module::Linkage::External));

    // snow_string_new(data: ptr, len: u64) -> ptr
    let string_new_ty = ptr_type.fn_type(
        &[ptr_type.into(), i64_type.into()],
        false,
    );
    module.add_function("snow_string_new", string_new_ty, Some(inkwell::module::Linkage::External));

    // snow_string_concat(a: ptr, b: ptr) -> ptr
    let string_concat_ty = ptr_type.fn_type(
        &[ptr_type.into(), ptr_type.into()],
        false,
    );
    module.add_function("snow_string_concat", string_concat_ty, Some(inkwell::module::Linkage::External));

    // snow_int_to_string(val: i64) -> ptr
    let int_to_string_ty = ptr_type.fn_type(&[i64_type.into()], false);
    module.add_function("snow_int_to_string", int_to_string_ty, Some(inkwell::module::Linkage::External));

    // snow_float_to_string(val: f64) -> ptr
    let float_to_string_ty = ptr_type.fn_type(&[f64_type.into()], false);
    module.add_function("snow_float_to_string", float_to_string_ty, Some(inkwell::module::Linkage::External));

    // snow_bool_to_string(val: i8) -> ptr
    let bool_to_string_ty = ptr_type.fn_type(&[i8_type.into()], false);
    module.add_function("snow_bool_to_string", bool_to_string_ty, Some(inkwell::module::Linkage::External));

    // snow_print(s: ptr) -> void
    let print_ty = void_type.fn_type(&[ptr_type.into()], false);
    module.add_function("snow_print", print_ty, Some(inkwell::module::Linkage::External));

    // snow_println(s: ptr) -> void
    let println_ty = void_type.fn_type(&[ptr_type.into()], false);
    module.add_function("snow_println", println_ty, Some(inkwell::module::Linkage::External));

    // ── Actor runtime functions ──────────────────────────────────────────

    // snow_rt_init_actor(num_schedulers: i32) -> void
    let init_actor_ty = void_type.fn_type(&[i32_type.into()], false);
    module.add_function("snow_rt_init_actor", init_actor_ty, Some(inkwell::module::Linkage::External));

    // snow_actor_spawn(fn_ptr: ptr, args: ptr, args_size: i64, priority: i8) -> i64
    let spawn_ty = i64_type.fn_type(
        &[ptr_type.into(), ptr_type.into(), i64_type.into(), i8_type.into()],
        false,
    );
    module.add_function("snow_actor_spawn", spawn_ty, Some(inkwell::module::Linkage::External));

    // snow_actor_send(target_pid: i64, msg_ptr: ptr, msg_size: i64) -> void
    let send_ty = void_type.fn_type(
        &[i64_type.into(), ptr_type.into(), i64_type.into()],
        false,
    );
    module.add_function("snow_actor_send", send_ty, Some(inkwell::module::Linkage::External));

    // snow_actor_receive(timeout_ms: i64) -> ptr
    let receive_ty = ptr_type.fn_type(&[i64_type.into()], false);
    module.add_function("snow_actor_receive", receive_ty, Some(inkwell::module::Linkage::External));

    // snow_actor_self() -> i64
    let self_ty = i64_type.fn_type(&[], false);
    module.add_function("snow_actor_self", self_ty, Some(inkwell::module::Linkage::External));

    // snow_actor_link(target_pid: i64) -> void
    let link_ty = void_type.fn_type(&[i64_type.into()], false);
    module.add_function("snow_actor_link", link_ty, Some(inkwell::module::Linkage::External));

    // snow_reduction_check() -> void
    let reduction_ty = void_type.fn_type(&[], false);
    module.add_function("snow_reduction_check", reduction_ty, Some(inkwell::module::Linkage::External));

    // snow_actor_set_terminate(pid: i64, callback_fn_ptr: ptr) -> void
    let set_terminate_ty = void_type.fn_type(&[i64_type.into(), ptr_type.into()], false);
    module.add_function("snow_actor_set_terminate", set_terminate_ty, Some(inkwell::module::Linkage::External));

    // snow_rt_run_scheduler() -> void
    let run_scheduler_ty = void_type.fn_type(&[], false);
    module.add_function("snow_rt_run_scheduler", run_scheduler_ty, Some(inkwell::module::Linkage::External));

    // ── Supervisor runtime functions ─────────────────────────────────────

    // snow_supervisor_start(config_ptr: ptr, config_size: i64) -> i64 (PID)
    let sup_start_ty = i64_type.fn_type(&[ptr_type.into(), i64_type.into()], false);
    module.add_function("snow_supervisor_start", sup_start_ty, Some(inkwell::module::Linkage::External));

    // snow_supervisor_start_child(sup_pid: i64, args_ptr: ptr, args_size: i64) -> i64
    let sup_start_child_ty = i64_type.fn_type(&[i64_type.into(), ptr_type.into(), i64_type.into()], false);
    module.add_function("snow_supervisor_start_child", sup_start_child_ty, Some(inkwell::module::Linkage::External));

    // snow_supervisor_terminate_child(sup_pid: i64, child_pid: i64) -> i64
    let sup_term_child_ty = i64_type.fn_type(&[i64_type.into(), i64_type.into()], false);
    module.add_function("snow_supervisor_terminate_child", sup_term_child_ty, Some(inkwell::module::Linkage::External));

    // snow_supervisor_count_children(sup_pid: i64) -> i64
    let sup_count_ty = i64_type.fn_type(&[i64_type.into()], false);
    module.add_function("snow_supervisor_count_children", sup_count_ty, Some(inkwell::module::Linkage::External));

    // snow_actor_trap_exit() -> void
    let trap_exit_ty = void_type.fn_type(&[], false);
    module.add_function("snow_actor_trap_exit", trap_exit_ty, Some(inkwell::module::Linkage::External));

    // snow_actor_exit(target_pid: i64, reason_tag: i8) -> void
    let actor_exit_ty = void_type.fn_type(&[i64_type.into(), i8_type.into()], false);
    module.add_function("snow_actor_exit", actor_exit_ty, Some(inkwell::module::Linkage::External));

    // ── Standard library: String operations (Phase 8) ──────────────────

    // snow_string_length(s: ptr) -> i64
    let string_length_ty = i64_type.fn_type(&[ptr_type.into()], false);
    module.add_function("snow_string_length", string_length_ty, Some(inkwell::module::Linkage::External));

    // snow_string_slice(s: ptr, start: i64, end: i64) -> ptr
    let string_slice_ty = ptr_type.fn_type(
        &[ptr_type.into(), i64_type.into(), i64_type.into()],
        false,
    );
    module.add_function("snow_string_slice", string_slice_ty, Some(inkwell::module::Linkage::External));

    // snow_string_contains(haystack: ptr, needle: ptr) -> i8
    let string_contains_ty = i8_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
    module.add_function("snow_string_contains", string_contains_ty, Some(inkwell::module::Linkage::External));

    // snow_string_starts_with(s: ptr, prefix: ptr) -> i8
    let string_starts_with_ty = i8_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
    module.add_function("snow_string_starts_with", string_starts_with_ty, Some(inkwell::module::Linkage::External));

    // snow_string_ends_with(s: ptr, suffix: ptr) -> i8
    let string_ends_with_ty = i8_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
    module.add_function("snow_string_ends_with", string_ends_with_ty, Some(inkwell::module::Linkage::External));

    // snow_string_trim(s: ptr) -> ptr
    let string_trim_ty = ptr_type.fn_type(&[ptr_type.into()], false);
    module.add_function("snow_string_trim", string_trim_ty, Some(inkwell::module::Linkage::External));

    // snow_string_to_upper(s: ptr) -> ptr
    let string_to_upper_ty = ptr_type.fn_type(&[ptr_type.into()], false);
    module.add_function("snow_string_to_upper", string_to_upper_ty, Some(inkwell::module::Linkage::External));

    // snow_string_to_lower(s: ptr) -> ptr
    let string_to_lower_ty = ptr_type.fn_type(&[ptr_type.into()], false);
    module.add_function("snow_string_to_lower", string_to_lower_ty, Some(inkwell::module::Linkage::External));

    // snow_string_replace(s: ptr, from: ptr, to: ptr) -> ptr
    let string_replace_ty = ptr_type.fn_type(
        &[ptr_type.into(), ptr_type.into(), ptr_type.into()],
        false,
    );
    module.add_function("snow_string_replace", string_replace_ty, Some(inkwell::module::Linkage::External));

    // snow_string_eq(a: ptr, b: ptr) -> i8
    let string_eq_ty = i8_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
    module.add_function("snow_string_eq", string_eq_ty, Some(inkwell::module::Linkage::External));

    // ── Standard library: File I/O functions (Phase 8) ────────────────

    // snow_file_read(path: ptr) -> ptr (SnowResult)
    let file_read_ty = ptr_type.fn_type(&[ptr_type.into()], false);
    module.add_function("snow_file_read", file_read_ty, Some(inkwell::module::Linkage::External));

    // snow_file_write(path: ptr, content: ptr) -> ptr (SnowResult)
    let file_write_ty = ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
    module.add_function("snow_file_write", file_write_ty, Some(inkwell::module::Linkage::External));

    // snow_file_append(path: ptr, content: ptr) -> ptr (SnowResult)
    let file_append_ty = ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
    module.add_function("snow_file_append", file_append_ty, Some(inkwell::module::Linkage::External));

    // snow_file_exists(path: ptr) -> i8
    let file_exists_ty = i8_type.fn_type(&[ptr_type.into()], false);
    module.add_function("snow_file_exists", file_exists_ty, Some(inkwell::module::Linkage::External));

    // snow_file_delete(path: ptr) -> ptr (SnowResult)
    let file_delete_ty = ptr_type.fn_type(&[ptr_type.into()], false);
    module.add_function("snow_file_delete", file_delete_ty, Some(inkwell::module::Linkage::External));

    // ── Standard library: IO functions (Phase 8) ─────────────────────

    // snow_io_read_line() -> ptr (SnowResult)
    let io_read_line_ty = ptr_type.fn_type(&[], false);
    module.add_function("snow_io_read_line", io_read_line_ty, Some(inkwell::module::Linkage::External));

    // snow_io_eprintln(s: ptr) -> void
    let io_eprintln_ty = void_type.fn_type(&[ptr_type.into()], false);
    module.add_function("snow_io_eprintln", io_eprintln_ty, Some(inkwell::module::Linkage::External));

    // ── Standard library: Env functions (Phase 8) ────────────────────

    // snow_env_get(key: ptr) -> ptr (SnowOption)
    let env_get_ty = ptr_type.fn_type(&[ptr_type.into()], false);
    module.add_function("snow_env_get", env_get_ty, Some(inkwell::module::Linkage::External));

    // snow_env_args() -> ptr (packed array)
    let env_args_ty = ptr_type.fn_type(&[], false);
    module.add_function("snow_env_args", env_args_ty, Some(inkwell::module::Linkage::External));

    // ── Standard library: Collection functions (Phase 8 Plan 02) ──────────

    // List functions
    module.add_function("snow_list_new", ptr_type.fn_type(&[], false), Some(inkwell::module::Linkage::External));
    module.add_function("snow_list_length", i64_type.fn_type(&[ptr_type.into()], false), Some(inkwell::module::Linkage::External));
    module.add_function("snow_list_append", ptr_type.fn_type(&[ptr_type.into(), i64_type.into()], false), Some(inkwell::module::Linkage::External));
    module.add_function("snow_list_head", i64_type.fn_type(&[ptr_type.into()], false), Some(inkwell::module::Linkage::External));
    module.add_function("snow_list_tail", ptr_type.fn_type(&[ptr_type.into()], false), Some(inkwell::module::Linkage::External));
    module.add_function("snow_list_get", i64_type.fn_type(&[ptr_type.into(), i64_type.into()], false), Some(inkwell::module::Linkage::External));
    module.add_function("snow_list_concat", ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false), Some(inkwell::module::Linkage::External));
    module.add_function("snow_list_reverse", ptr_type.fn_type(&[ptr_type.into()], false), Some(inkwell::module::Linkage::External));
    module.add_function("snow_list_map", ptr_type.fn_type(&[ptr_type.into(), ptr_type.into(), ptr_type.into()], false), Some(inkwell::module::Linkage::External));
    module.add_function("snow_list_filter", ptr_type.fn_type(&[ptr_type.into(), ptr_type.into(), ptr_type.into()], false), Some(inkwell::module::Linkage::External));
    module.add_function("snow_list_reduce", i64_type.fn_type(&[ptr_type.into(), i64_type.into(), ptr_type.into(), ptr_type.into()], false), Some(inkwell::module::Linkage::External));
    module.add_function("snow_list_from_array", ptr_type.fn_type(&[ptr_type.into(), i64_type.into()], false), Some(inkwell::module::Linkage::External));

    // Map functions
    module.add_function("snow_map_new", ptr_type.fn_type(&[], false), Some(inkwell::module::Linkage::External));
    module.add_function("snow_map_new_typed", ptr_type.fn_type(&[i64_type.into()], false), Some(inkwell::module::Linkage::External));
    module.add_function("snow_map_tag_string", ptr_type.fn_type(&[ptr_type.into()], false), Some(inkwell::module::Linkage::External));
    module.add_function("snow_map_put", ptr_type.fn_type(&[ptr_type.into(), i64_type.into(), i64_type.into()], false), Some(inkwell::module::Linkage::External));
    module.add_function("snow_map_get", i64_type.fn_type(&[ptr_type.into(), i64_type.into()], false), Some(inkwell::module::Linkage::External));
    module.add_function("snow_map_has_key", i8_type.fn_type(&[ptr_type.into(), i64_type.into()], false), Some(inkwell::module::Linkage::External));
    module.add_function("snow_map_delete", ptr_type.fn_type(&[ptr_type.into(), i64_type.into()], false), Some(inkwell::module::Linkage::External));
    module.add_function("snow_map_size", i64_type.fn_type(&[ptr_type.into()], false), Some(inkwell::module::Linkage::External));
    module.add_function("snow_map_keys", ptr_type.fn_type(&[ptr_type.into()], false), Some(inkwell::module::Linkage::External));
    module.add_function("snow_map_values", ptr_type.fn_type(&[ptr_type.into()], false), Some(inkwell::module::Linkage::External));

    // Set functions
    module.add_function("snow_set_new", ptr_type.fn_type(&[], false), Some(inkwell::module::Linkage::External));
    module.add_function("snow_set_add", ptr_type.fn_type(&[ptr_type.into(), i64_type.into()], false), Some(inkwell::module::Linkage::External));
    module.add_function("snow_set_remove", ptr_type.fn_type(&[ptr_type.into(), i64_type.into()], false), Some(inkwell::module::Linkage::External));
    module.add_function("snow_set_contains", i8_type.fn_type(&[ptr_type.into(), i64_type.into()], false), Some(inkwell::module::Linkage::External));
    module.add_function("snow_set_size", i64_type.fn_type(&[ptr_type.into()], false), Some(inkwell::module::Linkage::External));
    module.add_function("snow_set_union", ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false), Some(inkwell::module::Linkage::External));
    module.add_function("snow_set_intersection", ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false), Some(inkwell::module::Linkage::External));

    // Tuple functions
    module.add_function("snow_tuple_nth", i64_type.fn_type(&[ptr_type.into(), i64_type.into()], false), Some(inkwell::module::Linkage::External));
    module.add_function("snow_tuple_first", i64_type.fn_type(&[ptr_type.into()], false), Some(inkwell::module::Linkage::External));
    module.add_function("snow_tuple_second", i64_type.fn_type(&[ptr_type.into()], false), Some(inkwell::module::Linkage::External));
    module.add_function("snow_tuple_size", i64_type.fn_type(&[ptr_type.into()], false), Some(inkwell::module::Linkage::External));

    // Range functions
    module.add_function("snow_range_new", ptr_type.fn_type(&[i64_type.into(), i64_type.into()], false), Some(inkwell::module::Linkage::External));
    module.add_function("snow_range_to_list", ptr_type.fn_type(&[ptr_type.into()], false), Some(inkwell::module::Linkage::External));
    module.add_function("snow_range_map", ptr_type.fn_type(&[ptr_type.into(), ptr_type.into(), ptr_type.into()], false), Some(inkwell::module::Linkage::External));
    module.add_function("snow_range_filter", ptr_type.fn_type(&[ptr_type.into(), ptr_type.into(), ptr_type.into()], false), Some(inkwell::module::Linkage::External));
    module.add_function("snow_range_length", i64_type.fn_type(&[ptr_type.into()], false), Some(inkwell::module::Linkage::External));

    // Queue functions
    module.add_function("snow_queue_new", ptr_type.fn_type(&[], false), Some(inkwell::module::Linkage::External));
    module.add_function("snow_queue_push", ptr_type.fn_type(&[ptr_type.into(), i64_type.into()], false), Some(inkwell::module::Linkage::External));
    module.add_function("snow_queue_pop", ptr_type.fn_type(&[ptr_type.into()], false), Some(inkwell::module::Linkage::External));
    module.add_function("snow_queue_peek", i64_type.fn_type(&[ptr_type.into()], false), Some(inkwell::module::Linkage::External));
    module.add_function("snow_queue_size", i64_type.fn_type(&[ptr_type.into()], false), Some(inkwell::module::Linkage::External));
    module.add_function("snow_queue_is_empty", i8_type.fn_type(&[ptr_type.into()], false), Some(inkwell::module::Linkage::External));

    // ── Standard library: JSON functions (Phase 8 Plan 04) ──────────────

    // snow_json_parse(input: ptr) -> ptr (SnowResult)
    module.add_function("snow_json_parse", ptr_type.fn_type(&[ptr_type.into()], false), Some(inkwell::module::Linkage::External));

    // snow_json_encode(json: ptr) -> ptr (SnowString)
    module.add_function("snow_json_encode", ptr_type.fn_type(&[ptr_type.into()], false), Some(inkwell::module::Linkage::External));

    // snow_json_encode_string(s: ptr) -> ptr (SnowString)
    module.add_function("snow_json_encode_string", ptr_type.fn_type(&[ptr_type.into()], false), Some(inkwell::module::Linkage::External));

    // snow_json_encode_int(val: i64) -> ptr (SnowString)
    module.add_function("snow_json_encode_int", ptr_type.fn_type(&[i64_type.into()], false), Some(inkwell::module::Linkage::External));

    // snow_json_encode_bool(val: i8) -> ptr (SnowString)
    module.add_function("snow_json_encode_bool", ptr_type.fn_type(&[i8_type.into()], false), Some(inkwell::module::Linkage::External));

    // snow_json_encode_map(map: ptr) -> ptr (SnowString)
    module.add_function("snow_json_encode_map", ptr_type.fn_type(&[ptr_type.into()], false), Some(inkwell::module::Linkage::External));

    // snow_json_encode_list(list: ptr) -> ptr (SnowString)
    module.add_function("snow_json_encode_list", ptr_type.fn_type(&[ptr_type.into()], false), Some(inkwell::module::Linkage::External));

    // snow_json_from_int(val: i64) -> ptr
    module.add_function("snow_json_from_int", ptr_type.fn_type(&[i64_type.into()], false), Some(inkwell::module::Linkage::External));

    // snow_json_from_float(val: f64) -> ptr
    module.add_function("snow_json_from_float", ptr_type.fn_type(&[f64_type.into()], false), Some(inkwell::module::Linkage::External));

    // snow_json_from_bool(val: i8) -> ptr
    module.add_function("snow_json_from_bool", ptr_type.fn_type(&[i8_type.into()], false), Some(inkwell::module::Linkage::External));

    // snow_json_from_string(s: ptr) -> ptr
    module.add_function("snow_json_from_string", ptr_type.fn_type(&[ptr_type.into()], false), Some(inkwell::module::Linkage::External));

    // ── Standard library: HTTP functions (Phase 8 Plan 05) ──────────────

    // snow_http_router() -> ptr
    module.add_function("snow_http_router", ptr_type.fn_type(&[], false), Some(inkwell::module::Linkage::External));

    // snow_http_route(router: ptr, pattern: ptr, handler_fn: ptr) -> ptr
    module.add_function("snow_http_route", ptr_type.fn_type(&[ptr_type.into(), ptr_type.into(), ptr_type.into()], false), Some(inkwell::module::Linkage::External));

    // snow_http_serve(router: ptr, port: i64) -> void
    module.add_function("snow_http_serve", void_type.fn_type(&[ptr_type.into(), i64_type.into()], false), Some(inkwell::module::Linkage::External));

    // snow_http_response_new(status: i64, body: ptr) -> ptr
    module.add_function("snow_http_response_new", ptr_type.fn_type(&[i64_type.into(), ptr_type.into()], false), Some(inkwell::module::Linkage::External));

    // snow_http_get(url: ptr) -> ptr (SnowResult)
    module.add_function("snow_http_get", ptr_type.fn_type(&[ptr_type.into()], false), Some(inkwell::module::Linkage::External));

    // snow_http_post(url: ptr, body: ptr) -> ptr (SnowResult)
    module.add_function("snow_http_post", ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false), Some(inkwell::module::Linkage::External));

    // snow_http_request_method(req: ptr) -> ptr
    module.add_function("snow_http_request_method", ptr_type.fn_type(&[ptr_type.into()], false), Some(inkwell::module::Linkage::External));

    // snow_http_request_path(req: ptr) -> ptr
    module.add_function("snow_http_request_path", ptr_type.fn_type(&[ptr_type.into()], false), Some(inkwell::module::Linkage::External));

    // snow_http_request_body(req: ptr) -> ptr
    module.add_function("snow_http_request_body", ptr_type.fn_type(&[ptr_type.into()], false), Some(inkwell::module::Linkage::External));

    // snow_http_request_header(req: ptr, name: ptr) -> ptr (SnowOption)
    module.add_function("snow_http_request_header", ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false), Some(inkwell::module::Linkage::External));

    // snow_http_request_query(req: ptr, name: ptr) -> ptr (SnowOption)
    module.add_function("snow_http_request_query", ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false), Some(inkwell::module::Linkage::External));

    // ── Service runtime functions (Phase 9 Plan 03) ──────────────────────

    // snow_service_call(target_pid: i64, msg_tag: i64, payload_ptr: ptr, payload_size: i64) -> ptr
    let service_call_ty = ptr_type.fn_type(
        &[i64_type.into(), i64_type.into(), ptr_type.into(), i64_type.into()],
        false,
    );
    module.add_function("snow_service_call", service_call_ty, Some(inkwell::module::Linkage::External));

    // snow_service_reply(caller_pid: i64, reply_ptr: ptr, reply_size: i64) -> void
    let service_reply_ty = void_type.fn_type(
        &[i64_type.into(), ptr_type.into(), i64_type.into()],
        false,
    );
    module.add_function("snow_service_reply", service_reply_ty, Some(inkwell::module::Linkage::External));

    // ── Job runtime functions (Phase 9 Plan 04) ──────────────────────────

    // snow_job_async(fn_ptr: ptr, env_ptr: ptr) -> i64 (PID)
    let job_async_ty = i64_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
    module.add_function("snow_job_async", job_async_ty, Some(inkwell::module::Linkage::External));

    // snow_job_await(job_pid: i64) -> ptr (SnowResult)
    let job_await_ty = ptr_type.fn_type(&[i64_type.into()], false);
    module.add_function("snow_job_await", job_await_ty, Some(inkwell::module::Linkage::External));

    // snow_job_await_timeout(job_pid: i64, timeout_ms: i64) -> ptr (SnowResult)
    let job_await_timeout_ty = ptr_type.fn_type(&[i64_type.into(), i64_type.into()], false);
    module.add_function("snow_job_await_timeout", job_await_timeout_ty, Some(inkwell::module::Linkage::External));

    // snow_job_map(list_ptr: ptr, fn_ptr: ptr, env_ptr: ptr) -> ptr (List of SnowResult)
    let job_map_ty = ptr_type.fn_type(&[ptr_type.into(), ptr_type.into(), ptr_type.into()], false);
    module.add_function("snow_job_map", job_map_ty, Some(inkwell::module::Linkage::External));

    // snow_panic(msg: ptr, msg_len: u64, file: ptr, file_len: u64, line: u32) -> void
    // (noreturn -- marked via attribute)
    let panic_params: Vec<BasicMetadataTypeEnum<'ctx>> = vec![
        ptr_type.into(),     // msg
        i64_type.into(),     // msg_len
        ptr_type.into(),     // file
        i64_type.into(),     // file_len
        i32_type.into(),     // line
    ];
    let panic_ty = void_type.fn_type(&panic_params, false);
    let panic_fn = module.add_function("snow_panic", panic_ty, Some(inkwell::module::Linkage::External));
    // Mark as noreturn
    panic_fn.add_attribute(
        inkwell::attributes::AttributeLoc::Function,
        context.create_enum_attribute(
            inkwell::attributes::Attribute::get_named_enum_kind_id("noreturn"),
            0,
        ),
    );
}

/// Get a runtime function by name from the module.
///
/// Panics if the function was not declared (call `declare_intrinsics` first).
pub fn get_intrinsic<'ctx>(module: &Module<'ctx>, name: &str) -> FunctionValue<'ctx> {
    module
        .get_function(name)
        .unwrap_or_else(|| panic!("Runtime function '{}' not declared", name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use inkwell::context::Context;

    #[test]
    fn test_declare_all_intrinsics() {
        let context = Context::create();
        let module = context.create_module("test");
        declare_intrinsics(&module);

        // Verify all expected functions exist
        assert!(module.get_function("snow_rt_init").is_some());
        assert!(module.get_function("snow_gc_alloc_actor").is_some());
        assert!(module.get_function("snow_string_new").is_some());
        assert!(module.get_function("snow_string_concat").is_some());
        assert!(module.get_function("snow_int_to_string").is_some());
        assert!(module.get_function("snow_float_to_string").is_some());
        assert!(module.get_function("snow_bool_to_string").is_some());
        assert!(module.get_function("snow_print").is_some());
        assert!(module.get_function("snow_println").is_some());
        assert!(module.get_function("snow_panic").is_some());

        // Actor runtime functions
        assert!(module.get_function("snow_rt_init_actor").is_some());
        assert!(module.get_function("snow_actor_spawn").is_some());
        assert!(module.get_function("snow_actor_send").is_some());
        assert!(module.get_function("snow_actor_receive").is_some());
        assert!(module.get_function("snow_actor_self").is_some());
        assert!(module.get_function("snow_actor_link").is_some());
        assert!(module.get_function("snow_reduction_check").is_some());
        assert!(module.get_function("snow_actor_set_terminate").is_some());
        assert!(module.get_function("snow_rt_run_scheduler").is_some());

        // Supervisor runtime functions
        assert!(module.get_function("snow_supervisor_start").is_some());
        assert!(module.get_function("snow_supervisor_start_child").is_some());
        assert!(module.get_function("snow_supervisor_terminate_child").is_some());
        assert!(module.get_function("snow_supervisor_count_children").is_some());
        assert!(module.get_function("snow_actor_trap_exit").is_some());
        assert!(module.get_function("snow_actor_exit").is_some());

        // Standard library functions (Phase 8)
        assert!(module.get_function("snow_file_read").is_some());
        assert!(module.get_function("snow_file_write").is_some());
        assert!(module.get_function("snow_file_append").is_some());
        assert!(module.get_function("snow_file_exists").is_some());
        assert!(module.get_function("snow_file_delete").is_some());
        assert!(module.get_function("snow_string_length").is_some());
        assert!(module.get_function("snow_string_slice").is_some());
        assert!(module.get_function("snow_string_contains").is_some());
        assert!(module.get_function("snow_string_starts_with").is_some());
        assert!(module.get_function("snow_string_ends_with").is_some());
        assert!(module.get_function("snow_string_trim").is_some());
        assert!(module.get_function("snow_string_to_upper").is_some());
        assert!(module.get_function("snow_string_to_lower").is_some());
        assert!(module.get_function("snow_string_replace").is_some());
        assert!(module.get_function("snow_string_eq").is_some());
        assert!(module.get_function("snow_io_read_line").is_some());
        assert!(module.get_function("snow_io_eprintln").is_some());
        assert!(module.get_function("snow_env_get").is_some());
        assert!(module.get_function("snow_env_args").is_some());

        // Collection functions (Phase 8 Plan 02)
        assert!(module.get_function("snow_list_new").is_some());
        assert!(module.get_function("snow_list_length").is_some());
        assert!(module.get_function("snow_list_append").is_some());
        assert!(module.get_function("snow_list_head").is_some());
        assert!(module.get_function("snow_list_tail").is_some());
        assert!(module.get_function("snow_list_get").is_some());
        assert!(module.get_function("snow_list_concat").is_some());
        assert!(module.get_function("snow_list_reverse").is_some());
        assert!(module.get_function("snow_list_map").is_some());
        assert!(module.get_function("snow_list_filter").is_some());
        assert!(module.get_function("snow_list_reduce").is_some());
        assert!(module.get_function("snow_list_from_array").is_some());
        assert!(module.get_function("snow_map_new").is_some());
        assert!(module.get_function("snow_map_new_typed").is_some());
        assert!(module.get_function("snow_map_put").is_some());
        assert!(module.get_function("snow_map_get").is_some());
        assert!(module.get_function("snow_map_has_key").is_some());
        assert!(module.get_function("snow_map_delete").is_some());
        assert!(module.get_function("snow_map_size").is_some());
        assert!(module.get_function("snow_map_keys").is_some());
        assert!(module.get_function("snow_map_values").is_some());
        assert!(module.get_function("snow_set_new").is_some());
        assert!(module.get_function("snow_set_add").is_some());
        assert!(module.get_function("snow_set_remove").is_some());
        assert!(module.get_function("snow_set_contains").is_some());
        assert!(module.get_function("snow_set_size").is_some());
        assert!(module.get_function("snow_set_union").is_some());
        assert!(module.get_function("snow_set_intersection").is_some());
        assert!(module.get_function("snow_tuple_nth").is_some());
        assert!(module.get_function("snow_tuple_first").is_some());
        assert!(module.get_function("snow_tuple_second").is_some());
        assert!(module.get_function("snow_tuple_size").is_some());
        assert!(module.get_function("snow_range_new").is_some());
        assert!(module.get_function("snow_range_to_list").is_some());
        assert!(module.get_function("snow_range_map").is_some());
        assert!(module.get_function("snow_range_filter").is_some());
        assert!(module.get_function("snow_range_length").is_some());
        assert!(module.get_function("snow_queue_new").is_some());
        assert!(module.get_function("snow_queue_push").is_some());
        assert!(module.get_function("snow_queue_pop").is_some());
        assert!(module.get_function("snow_queue_peek").is_some());
        assert!(module.get_function("snow_queue_size").is_some());
        assert!(module.get_function("snow_queue_is_empty").is_some());

        // HTTP functions (Phase 8 Plan 05)
        assert!(module.get_function("snow_http_router").is_some());
        assert!(module.get_function("snow_http_route").is_some());
        assert!(module.get_function("snow_http_serve").is_some());
        assert!(module.get_function("snow_http_response_new").is_some());
        assert!(module.get_function("snow_http_get").is_some());
        assert!(module.get_function("snow_http_post").is_some());
        assert!(module.get_function("snow_http_request_method").is_some());
        assert!(module.get_function("snow_http_request_path").is_some());
        assert!(module.get_function("snow_http_request_body").is_some());
        assert!(module.get_function("snow_http_request_header").is_some());
        assert!(module.get_function("snow_http_request_query").is_some());

        // Service runtime functions (Phase 9 Plan 03)
        assert!(module.get_function("snow_service_call").is_some());
        assert!(module.get_function("snow_service_reply").is_some());

        // Job runtime functions (Phase 9 Plan 04)
        assert!(module.get_function("snow_job_async").is_some());
        assert!(module.get_function("snow_job_await").is_some());
        assert!(module.get_function("snow_job_await_timeout").is_some());
        assert!(module.get_function("snow_job_map").is_some());

        // JSON functions (Phase 8 Plan 04)
        assert!(module.get_function("snow_json_parse").is_some());
        assert!(module.get_function("snow_json_encode").is_some());
        assert!(module.get_function("snow_json_encode_string").is_some());
        assert!(module.get_function("snow_json_encode_int").is_some());
        assert!(module.get_function("snow_json_encode_bool").is_some());
        assert!(module.get_function("snow_json_encode_map").is_some());
        assert!(module.get_function("snow_json_encode_list").is_some());
        assert!(module.get_function("snow_json_from_int").is_some());
        assert!(module.get_function("snow_json_from_float").is_some());
        assert!(module.get_function("snow_json_from_bool").is_some());
        assert!(module.get_function("snow_json_from_string").is_some());
    }

    #[test]
    fn test_get_intrinsic() {
        let context = Context::create();
        let module = context.create_module("test");
        declare_intrinsics(&module);

        let init_fn = get_intrinsic(&module, "snow_rt_init");
        assert_eq!(init_fn.get_name().to_str().unwrap(), "snow_rt_init");
    }

    #[test]
    fn test_panic_is_noreturn() {
        let context = Context::create();
        let module = context.create_module("test");
        declare_intrinsics(&module);

        let panic_fn = get_intrinsic(&module, "snow_panic");
        // Check that noreturn attribute is present
        let noreturn_id = inkwell::attributes::Attribute::get_named_enum_kind_id("noreturn");
        let attr = panic_fn.get_enum_attribute(inkwell::attributes::AttributeLoc::Function, noreturn_id);
        assert!(attr.is_some(), "snow_panic should have noreturn attribute");
    }
}
