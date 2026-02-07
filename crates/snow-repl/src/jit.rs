//! JIT compilation engine for REPL inputs.
//!
//! Uses the full Snow compiler pipeline (parse -> typecheck -> MIR -> LLVM IR)
//! to compile expressions, then executes them via LLVM's JIT execution engine.
//! This ensures REPL behavior is identical to compiled code.

use crate::session::ReplSession;

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
        let rendered = typeck.render_errors(&full_source, "<repl>");
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
        let rendered = typeck.render_errors(&full_source, "<repl>");
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
}
