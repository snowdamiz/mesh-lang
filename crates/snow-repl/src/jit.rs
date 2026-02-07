//! JIT compilation engine for REPL inputs.
//!
//! Uses the full Snow compiler pipeline (parse -> typecheck -> MIR -> LLVM IR)
//! to compile expressions, then executes them via LLVM's JIT execution engine.
//! This ensures REPL behavior is identical to compiled code.
//!
//! The actor runtime (snow-rt) is linked and its symbols are registered with
//! LLVM so that JIT-compiled code can call runtime functions like
//! `snow_gc_alloc`, `snow_actor_spawn`, etc.

use crate::session::ReplSession;
use std::sync::Once;

static RUNTIME_INIT: Once = Once::new();

/// Initialize the Snow runtime (GC + actor scheduler) and register all
/// runtime symbols with LLVM so that JIT-compiled code can resolve them.
///
/// This is called once at REPL startup. Subsequent calls are no-ops.
pub fn init_runtime() {
    RUNTIME_INIT.call_once(|| {
        // Initialize the GC arena
        snow_rt::snow_rt_init();

        // Initialize the actor scheduler with default number of workers
        snow_rt::snow_rt_init_actor(0);

        // Register snow-rt symbols with LLVM's global symbol table so the
        // JIT execution engine can resolve them. LLVM's MCJIT uses dlsym
        // on some platforms, but explicit registration is more reliable.
        register_runtime_symbols();
    });
}

/// Register all snow-rt extern "C" symbols with LLVM's dynamic lookup.
///
/// This calls LLVMAddSymbol for each runtime function, making them
/// available to JIT-compiled code. Without this, the JIT engine cannot
/// resolve calls to runtime functions like snow_gc_alloc, snow_print, etc.
fn register_runtime_symbols() {
    // LLVMAddSymbol is the LLVM C API for registering symbols with the
    // JIT's symbol resolver. inkwell 0.8 doesn't expose it directly,
    // so we call into llvm-sys through the re-exported C bindings.
    extern "C" {
        fn LLVMAddSymbol(name: *const std::ffi::c_char, value: *mut std::ffi::c_void);
    }

    /// Register a single symbol with LLVM's JIT resolver.
    fn add_sym(name: &str, ptr: *const ()) {
        let c_name = std::ffi::CString::new(name).unwrap();
        unsafe {
            LLVMAddSymbol(c_name.as_ptr(), ptr as *mut std::ffi::c_void);
        }
    }

    // GC / runtime init
    add_sym("snow_rt_init", snow_rt::snow_rt_init as *const ());
    add_sym("snow_gc_alloc", snow_rt::snow_gc_alloc as *const ());
    add_sym("snow_gc_alloc_actor", snow_rt::snow_gc_alloc_actor as *const ());

    // IO
    add_sym("snow_print", snow_rt::snow_print as *const ());
    add_sym("snow_println", snow_rt::snow_println as *const ());
    add_sym("snow_io_read_line", snow_rt::snow_io_read_line as *const ());
    add_sym("snow_io_eprintln", snow_rt::snow_io_eprintln as *const ());

    // String operations
    add_sym("snow_string_new", snow_rt::snow_string_new as *const ());
    add_sym("snow_string_concat", snow_rt::snow_string_concat as *const ());
    add_sym("snow_string_length", snow_rt::snow_string_length as *const ());
    add_sym("snow_string_eq", snow_rt::snow_string_eq as *const ());
    add_sym("snow_string_contains", snow_rt::snow_string_contains as *const ());
    add_sym("snow_string_slice", snow_rt::snow_string_slice as *const ());
    add_sym("snow_string_replace", snow_rt::snow_string_replace as *const ());
    add_sym("snow_string_starts_with", snow_rt::snow_string_starts_with as *const ());
    add_sym("snow_string_ends_with", snow_rt::snow_string_ends_with as *const ());
    add_sym("snow_string_trim", snow_rt::snow_string_trim as *const ());
    add_sym("snow_string_to_upper", snow_rt::snow_string_to_upper as *const ());
    add_sym("snow_string_to_lower", snow_rt::snow_string_to_lower as *const ());
    add_sym("snow_int_to_string", snow_rt::snow_int_to_string as *const ());
    add_sym("snow_bool_to_string", snow_rt::snow_bool_to_string as *const ());
    add_sym("snow_float_to_string", snow_rt::snow_float_to_string as *const ());

    // Panic
    add_sym("snow_panic", snow_rt::snow_panic as *const ());

    // Actor runtime
    add_sym("snow_rt_init_actor", snow_rt::snow_rt_init_actor as *const ());
    add_sym("snow_rt_run_scheduler", snow_rt::snow_rt_run_scheduler as *const ());
    add_sym("snow_actor_spawn", snow_rt::snow_actor_spawn as *const ());
    add_sym("snow_actor_send", snow_rt::snow_actor_send as *const ());
    add_sym("snow_actor_receive", snow_rt::snow_actor_receive as *const ());
    add_sym("snow_actor_self", snow_rt::snow_actor_self as *const ());
    add_sym("snow_actor_link", snow_rt::snow_actor_link as *const ());
    add_sym("snow_actor_set_terminate", snow_rt::snow_actor_set_terminate as *const ());
    add_sym("snow_actor_register", snow_rt::snow_actor_register as *const ());
    add_sym("snow_actor_whereis", snow_rt::snow_actor_whereis as *const ());
    add_sym("snow_reduction_check", snow_rt::snow_reduction_check as *const ());

    // Services
    add_sym("snow_service_call", snow_rt::snow_service_call as *const ());
    add_sym("snow_service_reply", snow_rt::snow_service_reply as *const ());

    // Collections -- List
    add_sym("snow_list_new", snow_rt::snow_list_new as *const ());
    add_sym("snow_list_from_array", snow_rt::snow_list_from_array as *const ());
    add_sym("snow_list_get", snow_rt::snow_list_get as *const ());
    add_sym("snow_list_head", snow_rt::snow_list_head as *const ());
    add_sym("snow_list_tail", snow_rt::snow_list_tail as *const ());
    add_sym("snow_list_length", snow_rt::snow_list_length as *const ());
    add_sym("snow_list_append", snow_rt::snow_list_append as *const ());
    add_sym("snow_list_concat", snow_rt::snow_list_concat as *const ());
    add_sym("snow_list_map", snow_rt::snow_list_map as *const ());
    add_sym("snow_list_filter", snow_rt::snow_list_filter as *const ());
    add_sym("snow_list_reduce", snow_rt::snow_list_reduce as *const ());
    add_sym("snow_list_reverse", snow_rt::snow_list_reverse as *const ());

    // Collections -- Map
    add_sym("snow_map_new", snow_rt::snow_map_new as *const ());
    add_sym("snow_map_put", snow_rt::snow_map_put as *const ());
    add_sym("snow_map_get", snow_rt::snow_map_get as *const ());
    add_sym("snow_map_delete", snow_rt::snow_map_delete as *const ());
    add_sym("snow_map_has_key", snow_rt::snow_map_has_key as *const ());
    add_sym("snow_map_size", snow_rt::snow_map_size as *const ());
    add_sym("snow_map_keys", snow_rt::snow_map_keys as *const ());
    add_sym("snow_map_values", snow_rt::snow_map_values as *const ());

    // Collections -- Set
    add_sym("snow_set_new", snow_rt::snow_set_new as *const ());
    add_sym("snow_set_add", snow_rt::snow_set_add as *const ());
    add_sym("snow_set_remove", snow_rt::snow_set_remove as *const ());
    add_sym("snow_set_contains", snow_rt::snow_set_contains as *const ());
    add_sym("snow_set_size", snow_rt::snow_set_size as *const ());
    add_sym("snow_set_union", snow_rt::snow_set_union as *const ());
    add_sym("snow_set_intersection", snow_rt::snow_set_intersection as *const ());

    // Collections -- Queue
    add_sym("snow_queue_new", snow_rt::snow_queue_new as *const ());
    add_sym("snow_queue_push", snow_rt::snow_queue_push as *const ());
    add_sym("snow_queue_pop", snow_rt::snow_queue_pop as *const ());
    add_sym("snow_queue_peek", snow_rt::snow_queue_peek as *const ());
    add_sym("snow_queue_size", snow_rt::snow_queue_size as *const ());
    add_sym("snow_queue_is_empty", snow_rt::snow_queue_is_empty as *const ());

    // Collections -- Range
    add_sym("snow_range_new", snow_rt::snow_range_new as *const ());
    add_sym("snow_range_to_list", snow_rt::snow_range_to_list as *const ());
    add_sym("snow_range_length", snow_rt::snow_range_length as *const ());
    add_sym("snow_range_map", snow_rt::snow_range_map as *const ());
    add_sym("snow_range_filter", snow_rt::snow_range_filter as *const ());

    // Collections -- Tuple
    add_sym("snow_tuple_first", snow_rt::snow_tuple_first as *const ());
    add_sym("snow_tuple_second", snow_rt::snow_tuple_second as *const ());
    add_sym("snow_tuple_nth", snow_rt::snow_tuple_nth as *const ());
    add_sym("snow_tuple_size", snow_rt::snow_tuple_size as *const ());

    // File operations
    add_sym("snow_file_read", snow_rt::snow_file_read as *const ());
    add_sym("snow_file_write", snow_rt::snow_file_write as *const ());
    add_sym("snow_file_append", snow_rt::snow_file_append as *const ());
    add_sym("snow_file_exists", snow_rt::snow_file_exists as *const ());
    add_sym("snow_file_delete", snow_rt::snow_file_delete as *const ());

    // Environment
    add_sym("snow_env_get", snow_rt::snow_env_get as *const ());
    add_sym("snow_env_args", snow_rt::snow_env_args as *const ());

    // JSON
    add_sym("snow_json_parse", snow_rt::snow_json_parse as *const ());
    add_sym("snow_json_encode", snow_rt::snow_json_encode as *const ());
    add_sym("snow_json_from_string", snow_rt::snow_json_from_string as *const ());
    add_sym("snow_json_from_int", snow_rt::snow_json_from_int as *const ());
    add_sym("snow_json_from_float", snow_rt::snow_json_from_float as *const ());
    add_sym("snow_json_from_bool", snow_rt::snow_json_from_bool as *const ());
    add_sym("snow_json_encode_string", snow_rt::snow_json_encode_string as *const ());
    add_sym("snow_json_encode_int", snow_rt::snow_json_encode_int as *const ());
    add_sym("snow_json_encode_bool", snow_rt::snow_json_encode_bool as *const ());
    add_sym("snow_json_encode_list", snow_rt::snow_json_encode_list as *const ());
    add_sym("snow_json_encode_map", snow_rt::snow_json_encode_map as *const ());

    // HTTP
    add_sym("snow_http_get", snow_rt::snow_http_get as *const ());
    add_sym("snow_http_post", snow_rt::snow_http_post as *const ());
    add_sym("snow_http_router", snow_rt::snow_http_router as *const ());
    add_sym("snow_http_route", snow_rt::snow_http_route as *const ());
    add_sym("snow_http_serve", snow_rt::snow_http_serve as *const ());
    add_sym("snow_http_request_method", snow_rt::snow_http_request_method as *const ());
    add_sym("snow_http_request_path", snow_rt::snow_http_request_path as *const ());
    add_sym("snow_http_request_body", snow_rt::snow_http_request_body as *const ());
    add_sym("snow_http_request_header", snow_rt::snow_http_request_header as *const ());
    add_sym("snow_http_request_query", snow_rt::snow_http_request_query as *const ());
    add_sym("snow_http_response_new", snow_rt::snow_http_response_new as *const ());
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
    "fn", "def", "let", "type", "struct", "module", "actor", "service", "interface", "trait",
    "impl", "supervisor",
];

/// Check whether the input appears to be a definition rather than an expression.
///
/// Definitions start with specific keywords (fn, let, type, struct, etc.)
/// and are added to the session context rather than evaluated for a result.
pub fn is_definition(input: &str) -> bool {
    let trimmed = input.trim();
    DEFINITION_KEYWORDS
        .iter()
        .any(|kw| {
            trimmed.starts_with(kw)
                && trimmed[kw.len()..]
                    .starts_with(|c: char| c.is_whitespace() || c == '(')
        })
}

/// Evaluate a Snow expression or definition using the full compiler pipeline.
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
    let parse = snow_parser::parse(&full_source);
    if !parse.ok() {
        let errors: Vec<String> = parse.errors().iter().map(|e| format!("{}", e)).collect();
        return Err(format!("Parse error: {}", errors.join(", ")));
    }

    // Type check to validate the definition
    let typeck = snow_typeck::check(&parse);
    if !typeck.errors.is_empty() {
        let rendered = typeck.render_errors(&full_source, "<repl>", &snow_typeck::diagnostics::DiagnosticOptions::colorless());
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
    let parse = snow_parser::parse(&full_source);
    if !parse.ok() {
        let errors: Vec<String> = parse.errors().iter().map(|e| format!("{}", e)).collect();
        return Err(format!("Parse error: {}", errors.join(", ")));
    }

    // Step 2: Type check
    let typeck = snow_typeck::check(&parse);
    if !typeck.errors.is_empty() {
        let rendered = typeck.render_errors(&full_source, "<repl>", &snow_typeck::diagnostics::DiagnosticOptions::colorless());
        return Err(rendered.join("\n"));
    }

    // Get the result type from the type checker
    let result_type_name = if let Some(ref ty) = typeck.result_type {
        format!("{}", ty)
    } else {
        "Unit".to_string()
    };

    // Step 3: Lower to MIR
    let mir = snow_codegen::lower_to_mir_module(&parse, &typeck)?;

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
    mir: &snow_codegen::mir::MirModule,
    wrapper_fn_name: &str,
    result_type: &str,
) -> Result<String, String> {
    use inkwell::context::Context;
    use inkwell::targets::{InitializationConfig, Target};
    use snow_codegen::codegen::CodeGen;

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

/// Format a raw JIT result value based on its Snow type.
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
        "Float" => {
            let f = f64::from_bits(raw as u64);
            format!("{}", f)
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

    #[test]
    fn test_init_runtime_is_idempotent() {
        // Should not panic when called multiple times
        init_runtime();
        init_runtime();
    }

    #[test]
    fn test_repl_config_default() {
        let config = crate::ReplConfig::default();
        assert_eq!(config.prompt, "snow> ");
        assert_eq!(config.continuation, "  ... ");
    }
}
