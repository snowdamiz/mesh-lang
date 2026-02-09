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
pub mod diagnostics;
pub mod env;
pub mod error;
pub mod exhaustiveness;
pub mod infer;
pub mod traits;
pub mod ty;
pub mod unify;

use rustc_hash::FxHashMap;
use rowan::TextRange;

use crate::diagnostics::DiagnosticOptions;
use crate::error::TypeError;
use crate::traits::{TraitDef, ImplDef as TraitImplDef};
use crate::ty::{Scheme, Ty};

// Re-export type registry types for downstream crate consumption (codegen).
pub use crate::infer::{
    StructDefInfo, SumTypeDefInfo, TypeAliasInfo, TypeRegistry, VariantFieldInfo, VariantInfo,
    register_variant_constructors,
};
// Re-export trait registry for downstream trait resolution (codegen dispatch).
pub use crate::traits::TraitRegistry;

// ── Cross-Module Type Checking Types ────────────────────────────────────

/// Context built by the driver from already-checked dependency modules.
/// Pre-seeds the type checker's environments before inference begins.
#[derive(Debug, Default)]
pub struct ImportContext {
    /// Module namespace -> exported symbols.
    /// Key is the namespace name used for qualified access (last path segment
    /// for `import Math.Vector` -> key is "Vector").
    pub module_exports: FxHashMap<String, ModuleExports>,

    /// Trait definitions from ALL processed modules (globally visible).
    pub all_trait_defs: Vec<TraitDef>,

    /// Trait impls from ALL processed modules (globally visible, XMOD-05).
    pub all_trait_impls: Vec<TraitImplDef>,
}

impl ImportContext {
    /// Create an empty import context (for single-file / backward compat).
    pub fn empty() -> Self {
        Self::default()
    }
}

/// Exports from a single module.
#[derive(Debug, Default, Clone)]
pub struct ModuleExports {
    /// The full module name (e.g., "Math.Vector").
    pub module_name: String,

    /// Function/value type schemes, keyed by unqualified name.
    pub functions: FxHashMap<String, Scheme>,

    /// Struct definitions exported by this module.
    pub struct_defs: FxHashMap<String, StructDefInfo>,

    /// Sum type definitions exported by this module.
    pub sum_type_defs: FxHashMap<String, SumTypeDefInfo>,
}

/// Symbols exported by a module after type checking.
#[derive(Debug, Default, Clone)]
pub struct ExportedSymbols {
    /// Function type schemes (name -> scheme).
    pub functions: FxHashMap<String, Scheme>,
    /// Struct definitions.
    pub struct_defs: FxHashMap<String, StructDefInfo>,
    /// Sum type definitions.
    pub sum_type_defs: FxHashMap<String, SumTypeDefInfo>,
    /// Trait definitions declared in this module.
    pub trait_defs: Vec<TraitDef>,
    /// Trait impls declared in this module.
    pub trait_impls: Vec<TraitImplDef>,
}

// ── TypeckResult ────────────────────────────────────────────────────────

/// The result of type checking a Snow program.
///
/// Contains a mapping from source ranges to their inferred types, plus
/// any type errors encountered during checking. Also includes the type
/// registry with struct/sum type/alias definitions needed by codegen
/// to determine memory layouts, and the trait registry for trait method
/// dispatch resolution during MIR lowering.
pub struct TypeckResult {
    /// Map from source text ranges to their inferred types.
    pub types: FxHashMap<TextRange, Ty>,
    /// Type errors found during checking.
    pub errors: Vec<TypeError>,
    /// Warnings found during checking (e.g. redundant match arms).
    pub warnings: Vec<TypeError>,
    /// The inferred type of the last expression/item in the program.
    /// `None` if the program has no items or only produces errors.
    pub result_type: Option<Ty>,
    /// Registry of all struct, sum type, and type alias definitions.
    /// Used by codegen to determine memory layouts and variant tags.
    pub type_registry: TypeRegistry,
    /// Registry of all trait definitions and impl registrations.
    /// Used by codegen for trait method dispatch resolution.
    pub trait_registry: TraitRegistry,
    /// Default method bodies from interface definitions.
    /// Keyed by `(trait_name, method_name)`, value is the text range of the
    /// INTERFACE_METHOD node that contains the default body. The lowerer
    /// uses this range to find the method's AST node from the parse tree.
    pub default_method_bodies: FxHashMap<(String, String), TextRange>,
    /// Qualified module names used by this module via `import` declarations.
    /// Maps namespace name (e.g., "Math") to the list of exported function names.
    /// Used by the MIR lowerer to resolve qualified access (e.g., Math.add).
    pub qualified_modules: FxHashMap<String, Vec<String>>,
    /// Function names imported via `from Module import name1, name2` (selective imports).
    /// These names are directly callable without qualification.
    /// Used by the MIR lowerer to skip trait dispatch for imported functions.
    pub imported_functions: Vec<String>,
}

impl TypeckResult {
    /// Render all type errors as formatted diagnostic strings.
    ///
    /// Accepts `DiagnosticOptions` to control color and output format.
    /// Each error is rendered with labeled source spans, error codes, and
    /// fix suggestions when applicable.
    pub fn render_errors(&self, source: &str, filename: &str, options: &DiagnosticOptions) -> Vec<String> {
        self.errors
            .iter()
            .map(|err| diagnostics::render_diagnostic(err, source, filename, options, None))
            .collect()
    }
}

/// Type-check a parsed Snow program.
///
/// This is the main entry point for the type checker. It walks the AST,
/// infers types for all expressions, checks type annotations, and reports
/// errors.
pub fn check(parse: &snow_parser::Parse) -> TypeckResult {
    infer::infer(parse)
}

/// Type-check a parsed Snow program with pre-resolved imports.
///
/// This is the multi-module entry point. The ImportContext contains
/// symbols from already-type-checked dependency modules.
pub fn check_with_imports(parse: &snow_parser::Parse, import_ctx: &ImportContext) -> TypeckResult {
    infer::infer_with_imports(parse, import_ctx)
}

/// Collect exported symbols from a type-checked module.
///
/// Currently exports ALL top-level definitions (Phase 40 adds pub filtering).
/// Extracts function schemes from the typeck types map by scanning the parse
/// tree for FnDef items, struct/sum type defs from TypeRegistry, and
/// trait defs/impls from TraitRegistry.
pub fn collect_exports(
    parse: &snow_parser::Parse,
    typeck: &TypeckResult,
) -> ExportedSymbols {
    use snow_parser::ast::item::Item;
    use snow_parser::ast::AstNode;
    use snow_parser::syntax_kind::SyntaxKind;

    let tree = parse.tree();
    let mut exports = ExportedSymbols::default();

    for item in tree.items() {
        match item {
            Item::FnDef(fn_def) => {
                if let Some(name) = fn_def.name().and_then(|n| n.text()) {
                    // Look up the function's inferred type from the typeck result
                    let range = fn_def.syntax().text_range();
                    if let Some(ty) = typeck.types.get(&range) {
                        exports.functions.insert(
                            name,
                            Scheme::mono(ty.clone()),
                        );
                    }
                }
            }
            _ => {}
        }
    }

    // Copy struct and sum type defs from type_registry
    // (filter out builtins: Option, Result, Ordering are built-in)
    let builtin_sum_types = ["Option", "Result", "Ordering"];
    for (name, def) in &typeck.type_registry.struct_defs {
        exports.struct_defs.insert(name.clone(), def.clone());
    }
    for (name, def) in &typeck.type_registry.sum_type_defs {
        if !builtin_sum_types.contains(&name.as_str()) {
            exports.sum_type_defs.insert(name.clone(), def.clone());
        }
    }

    // Extract trait defs from AST InterfaceDef items, then look up in registry.
    for item in tree.items() {
        if let Item::InterfaceDef(iface) = item {
            if let Some(name) = iface.name().and_then(|n| n.text()) {
                if let Some(trait_def) = typeck.trait_registry.get_trait(&name) {
                    exports.trait_defs.push(trait_def.clone());
                }
            }
        }
    }

    // For trait impls: scan AST ImplDef items for trait names,
    // then collect matching impls from the registry.
    let mut local_impl_traits: Vec<(String, String)> = Vec::new(); // (trait_name, type_name)
    for item in tree.items() {
        if let Item::ImplDef(ref impl_def) = item {
            // Extract trait name from the first PATH child.
            let paths: Vec<_> = impl_def
                .syntax()
                .children()
                .filter(|n| n.kind() == SyntaxKind::PATH)
                .collect();

            let trait_name = paths
                .first()
                .and_then(|path| {
                    path.children_with_tokens()
                        .filter_map(|t| t.into_token())
                        .find(|t| t.kind() == SyntaxKind::IDENT)
                        .map(|t| t.text().to_string())
                });

            // Extract type name from the second PATH child (after `for`).
            let type_name = paths
                .get(1)
                .and_then(|path| {
                    path.children_with_tokens()
                        .filter_map(|t| t.into_token())
                        .find(|t| t.kind() == SyntaxKind::IDENT)
                        .map(|t| t.text().to_string())
                });

            if let (Some(tn), Some(ty)) = (trait_name, type_name) {
                local_impl_traits.push((tn, ty));
            }
        }
    }
    for impl_def in typeck.trait_registry.all_impls() {
        for (tn, ty) in &local_impl_traits {
            if impl_def.trait_name == *tn && impl_def.impl_type_name == *ty {
                exports.trait_impls.push(impl_def.clone());
            }
        }
    }

    exports
}
