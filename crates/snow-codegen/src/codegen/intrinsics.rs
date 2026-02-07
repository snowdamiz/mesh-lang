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

    // snow_gc_alloc(size: u64, align: u64) -> ptr
    let gc_alloc_ty = ptr_type.fn_type(
        &[i64_type.into(), i64_type.into()],
        false,
    );
    module.add_function("snow_gc_alloc", gc_alloc_ty, Some(inkwell::module::Linkage::External));

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
        assert!(module.get_function("snow_gc_alloc").is_some());
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
