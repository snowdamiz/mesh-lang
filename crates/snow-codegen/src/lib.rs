//! LLVM code generation for the Snow compiler.
//!
//! This crate transforms a typed Snow program (represented by the parser's
//! `Parse` and the type checker's `TypeckResult`) into native machine code
//! via LLVM, using the Inkwell safe bindings.
//!
//! ## Architecture
//!
//! - [`mir`]: Mid-level IR definitions and lowering from typed AST
//! - [`codegen`]: LLVM IR generation from MIR
//!
//! ## Pipeline
//!
//! ```text
//! Parse + TypeckResult -> MIR -> LLVM IR -> Object file -> Native binary
//! ```

pub mod codegen;
pub mod mir;

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

/// Compile a parsed and type-checked Snow program to a native binary.
///
/// This is the main entry point for code generation. It takes the parse tree
/// and type-check results, lowers them through MIR to LLVM IR, and produces
/// a native executable.
///
/// # Errors
///
/// Returns an error string if compilation fails at any stage (MIR lowering,
/// LLVM IR generation, object file emission, or linking).
pub fn compile(
    _parse: &snow_parser::Parse,
    _typeck: &snow_typeck::TypeckResult,
) -> Result<(), String> {
    todo!("compile: LLVM IR codegen will be implemented in Plan 05-04")
}
