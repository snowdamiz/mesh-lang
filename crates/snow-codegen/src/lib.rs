//! LLVM code generation for the Snow compiler.
//!
//! This crate transforms a typed Snow program (represented by the parser's
//! `Parse` and the type checker's `TypeckResult`) into native machine code
//! via LLVM, using the Inkwell safe bindings.
//!
//! ## Architecture
//!
//! - [`mir`]: Mid-level IR definitions and lowering from typed AST
//! - [`pattern`]: Pattern match compilation to decision trees
//! - [`codegen`]: LLVM IR generation from MIR
//!
//! ## Pipeline
//!
//! ```text
//! Parse + TypeckResult -> MIR -> DecisionTree -> LLVM IR -> Object file -> Native binary
//! ```

pub mod codegen;
pub mod link;
pub mod mir;
pub mod pattern;

use std::path::Path;

use inkwell::context::Context;

use codegen::CodeGen;
use mir::lower::lower_to_mir;
use mir::mono::monomorphize;

/// Lower a parsed and type-checked Snow program to MIR.
///
/// This runs the full MIR lowering pipeline: AST-to-MIR conversion (with
/// pipe desugaring, string interpolation compilation, and closure conversion),
/// followed by the monomorphization pass.
///
/// # Errors
///
/// Returns an error string if MIR lowering fails.
pub fn lower_to_mir_module(
    parse: &snow_parser::Parse,
    typeck: &snow_typeck::TypeckResult,
) -> Result<mir::MirModule, String> {
    let mut module = lower_to_mir(parse, typeck)?;
    monomorphize(&mut module);
    Ok(module)
}

/// Compile a parsed and type-checked Snow program to an object file.
///
/// This is the main entry point for code generation. It:
/// 1. Lowers the AST to MIR
/// 2. Monomorphizes generic code
/// 3. Generates LLVM IR
/// 4. Optionally optimizes
/// 5. Emits an object file
///
/// # Arguments
///
/// * `parse` - The parsed Snow source
/// * `typeck` - The type-checked results
/// * `output` - Path to write the object file
/// * `opt_level` - Optimization level (0 = none, 2 = default)
/// * `target_triple` - Optional target triple; None = host default
///
/// # Errors
///
/// Returns an error string if compilation fails at any stage.
pub fn compile_to_object(
    parse: &snow_parser::Parse,
    typeck: &snow_typeck::TypeckResult,
    output: &Path,
    opt_level: u8,
    target_triple: Option<&str>,
) -> Result<(), String> {
    let mir = lower_to_mir_module(parse, typeck)?;

    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "snow_module", opt_level, target_triple)?;
    codegen.compile(&mir)?;

    if opt_level > 0 {
        codegen.run_optimization_passes(opt_level)?;
    }

    codegen.emit_object(output)?;
    Ok(())
}

/// Compile a parsed and type-checked Snow program to LLVM IR text.
///
/// Similar to `compile_to_object` but emits human-readable LLVM IR (.ll file)
/// instead of a binary object file. Useful for debugging and inspection.
///
/// # Arguments
///
/// * `parse` - The parsed Snow source
/// * `typeck` - The type-checked results
/// * `output` - Path to write the .ll file
/// * `target_triple` - Optional target triple; None = host default
///
/// # Errors
///
/// Returns an error string if compilation fails at any stage.
pub fn compile_to_llvm_ir(
    parse: &snow_parser::Parse,
    typeck: &snow_typeck::TypeckResult,
    output: &Path,
    target_triple: Option<&str>,
) -> Result<(), String> {
    let mir = lower_to_mir_module(parse, typeck)?;

    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "snow_module", 0, target_triple)?;
    codegen.compile(&mir)?;

    codegen.emit_llvm_ir(output)?;
    Ok(())
}

/// Compile a parsed and type-checked Snow program to a native binary.
///
/// This is the full compilation pipeline: lower to MIR, generate LLVM IR,
/// optimize, emit object file, and link with snow-rt to produce a native
/// executable.
///
/// # Arguments
///
/// * `parse` - The parsed Snow source
/// * `typeck` - The type-checked results
/// * `output` - Path to write the final executable
/// * `opt_level` - Optimization level (0 = none, 2 = default)
/// * `target_triple` - Optional target triple; None = host default
/// * `rt_lib_path` - Optional path to `libsnow_rt.a`; None = auto-detect
///
/// # Errors
///
/// Returns an error string if compilation or linking fails.
pub fn compile_to_binary(
    parse: &snow_parser::Parse,
    typeck: &snow_typeck::TypeckResult,
    output: &Path,
    opt_level: u8,
    target_triple: Option<&str>,
    rt_lib_path: Option<&Path>,
) -> Result<(), String> {
    // Generate object file to a temporary path
    let obj_path = output.with_extension("o");
    compile_to_object(parse, typeck, &obj_path, opt_level, target_triple)?;

    // Link with snow-rt
    link::link(&obj_path, output, rt_lib_path)?;

    Ok(())
}

/// Compile a parsed and type-checked Snow program (verify-only pipeline).
///
/// Compiles through the full LLVM IR generation to verify correctness, but
/// does not emit any files. Useful for testing.
///
/// # Errors
///
/// Returns an error string if compilation fails at any stage.
pub fn compile(
    parse: &snow_parser::Parse,
    typeck: &snow_typeck::TypeckResult,
) -> Result<(), String> {
    let mir = lower_to_mir_module(parse, typeck)?;

    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "snow_module", 0, None)?;
    codegen.compile(&mir)?;

    Ok(())
}
