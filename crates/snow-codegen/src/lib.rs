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
    todo!("compile: MIR lowering and LLVM codegen will be implemented in subsequent plans")
}
