//! Snow type checker: Hindley-Milner type inference with extensions.
//!
//! This crate implements type checking and inference for the Snow language.
//! It builds on the parser's CST/AST to assign types to all expressions,
//! detect type errors, and support features like:
//!
//! - Hindley-Milner type inference with let-polymorphism
//! - Unification with occurs check
//! - Type annotations (explicit and inferred)
//! - Generic functions and data types
//! - Option/Result sugar types
//!
//! # Architecture
//!
//! - [`ty`]: Core type representation (Ty, TyCon, TyVar, Scheme)
//! - [`unify`]: Unification engine with occurs check and level-based generalization
//! - [`env`]: Type environment with scope stack
//! - [`builtins`]: Built-in type and operator registration
//! - [`error`]: Type error types with provenance tracking
//! - [`infer`]: Algorithm J inference engine

pub mod builtins;
pub mod env;
pub mod error;
pub mod infer;
pub mod traits;
pub mod ty;
pub mod unify;

use rustc_hash::FxHashMap;
use rowan::TextRange;

use crate::error::TypeError;
use crate::ty::Ty;

/// The result of type checking a Snow program.
///
/// Contains a mapping from source ranges to their inferred types, plus
/// any type errors encountered during checking.
pub struct TypeckResult {
    /// Map from source text ranges to their inferred types.
    pub types: FxHashMap<TextRange, Ty>,
    /// Type errors found during checking.
    pub errors: Vec<TypeError>,
    /// The inferred type of the last expression/item in the program.
    /// `None` if the program has no items or only produces errors.
    pub result_type: Option<Ty>,
}

/// Type-check a parsed Snow program.
///
/// This is the main entry point for the type checker. It walks the AST,
/// infers types for all expressions, checks type annotations, and reports
/// errors.
pub fn check(parse: &snow_parser::Parse) -> TypeckResult {
    infer::infer(parse)
}
