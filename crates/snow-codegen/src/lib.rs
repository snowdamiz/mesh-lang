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

/// Lower a parsed and type-checked Snow program to MIR without monomorphization.
///
/// Use this when lowering multiple modules that will be merged before
/// monomorphization (which requires reachability analysis from the entry point).
///
/// # Errors
///
/// Returns an error string if MIR lowering fails.
pub fn lower_to_mir_raw(
    parse: &snow_parser::Parse,
    typeck: &snow_typeck::TypeckResult,
) -> Result<mir::MirModule, String> {
    let module = lower_to_mir(parse, typeck)?;
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

// ── Multi-Module Compilation (Phase 39) ────────────────────────────────

/// Compile a pre-built MIR module to a native binary.
///
/// This accepts a MIR module directly (already lowered and optionally merged
/// from multiple source modules) and produces a native executable.
pub fn compile_mir_to_binary(
    mir: &mir::MirModule,
    output: &Path,
    opt_level: u8,
    target_triple: Option<&str>,
    rt_lib_path: Option<&Path>,
) -> Result<(), String> {
    let obj_path = output.with_extension("o");

    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "snow_module", opt_level, target_triple)?;
    codegen.compile(mir)?;

    if opt_level > 0 {
        codegen.run_optimization_passes(opt_level)?;
    }

    codegen.emit_object(&obj_path)?;
    link::link(&obj_path, output, rt_lib_path)?;

    Ok(())
}

/// Compile a pre-built MIR module to LLVM IR text.
pub fn compile_mir_to_llvm_ir(
    mir: &mir::MirModule,
    output: &Path,
    target_triple: Option<&str>,
) -> Result<(), String> {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "snow_module", 0, target_triple)?;
    codegen.compile(mir)?;

    codegen.emit_llvm_ir(output)?;
    Ok(())
}

/// Merge multiple MIR modules into a single module.
///
/// Functions, struct definitions, and sum type definitions from all modules
/// are combined. The entry function is taken from the designated entry module.
/// Duplicate struct/sum type definitions (e.g., builtins registered in every
/// module) are deduplicated by name.
///
/// After merging, runs the monomorphization pass to eliminate unreachable
/// functions (which requires the entry point from the merged module).
pub fn merge_mir_modules(
    modules: Vec<mir::MirModule>,
    entry_module_idx: usize,
) -> mir::MirModule {
    use std::collections::HashSet;

    let mut merged = mir::MirModule {
        functions: Vec::new(),
        structs: Vec::new(),
        sum_types: Vec::new(),
        entry_function: None,
        service_dispatch: std::collections::HashMap::new(),
    };

    let mut seen_functions: HashSet<String> = HashSet::new();
    let mut seen_structs: HashSet<String> = HashSet::new();
    let mut seen_sum_types: HashSet<String> = HashSet::new();

    // Process entry module first (its main() takes priority)
    if let Some(entry) = modules.get(entry_module_idx) {
        merged.entry_function = entry.entry_function.clone();
    }

    for module in &modules {
        for func in &module.functions {
            if seen_functions.insert(func.name.clone()) {
                merged.functions.push(func.clone());
            }
        }
        for s in &module.structs {
            if seen_structs.insert(s.name.clone()) {
                merged.structs.push(s.clone());
            }
        }
        for st in &module.sum_types {
            if seen_sum_types.insert(st.name.clone()) {
                merged.sum_types.push(st.clone());
            }
        }
        for (key, value) in &module.service_dispatch {
            merged.service_dispatch.entry(key.clone()).or_insert_with(|| value.clone());
        }
    }

    // Run monomorphization on the merged module to eliminate unreachable
    // functions (builtins like Ord__compare__String that are generated in
    // every module but only used if referenced from main).
    monomorphize(&mut merged);

    merged
}
