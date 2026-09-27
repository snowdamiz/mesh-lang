//! Mesh type checker: Hindley-Milner type inference with extensions.
//!
//! This crate implements type checking and inference for the Mesh language.
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

// A `TypeError` is built once per reported error while the checker's hot
// path returns `Ok`, so boxing it would cost every signature for nothing;
// several inference passes thread their state through many parameters.
#![allow(clippy::result_large_err, clippy::too_many_arguments)]

pub mod builtins;
pub mod diagnostics;
pub mod env;
pub mod error;
pub mod exhaustiveness;
pub mod infer;
mod ownership;
pub mod traits;
pub mod ty;
pub mod unify;

use rowan::TextRange;
use rustc_hash::{FxHashMap, FxHashSet};

use mesh_parser::ast::item::ParamOwnership;

use crate::diagnostics::DiagnosticOptions;
use crate::error::TypeError;
use crate::traits::{ImplDef as TraitImplDef, TraitDef};
use crate::ty::{Scheme, Ty};

// Re-export type registry types for downstream crate consumption (codegen).
pub use crate::infer::{
    default_schema_table, register_variant_constructors, FnConstraints, SchemaInfo, StructDefInfo,
    SumTypeDefInfo, TypeAliasInfo, TypeRegistry, VariantFieldInfo, VariantInfo,
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

    /// The name of the current module being type-checked (e.g., "Geometry").
    /// None for single-file mode (backward compat). Used to name clustered
    /// route handlers.
    pub current_module: Option<String>,

    /// Whether compiler-provided test-only builtins are available.
    pub test_builtins: bool,

    /// The full names of the project's modules, to tell a module used
    /// without its import from an unknown name.
    pub project_modules: Vec<String>,
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

    /// Service definitions exported by this module.
    pub service_defs: FxHashMap<String, ServiceExportInfo>,

    /// Actor definitions exported by this module (name -> type scheme).
    /// Actors are always exported (no `pub` prefix in grammar, same as services).
    pub actor_defs: FxHashMap<String, Scheme>,

    /// Names of private (non-pub) items, for distinguishing "private" from "nonexistent" in errors.
    pub private_names: FxHashSet<String>,

    /// Type aliases exported by this module (pub type only).
    pub type_aliases: FxHashMap<String, TypeAliasInfo>,

    /// Affine resource type names exported by this module.
    pub resource_types: FxHashSet<String>,

    /// Parameter ownership modes for exported functions.
    pub function_ownership: FxHashMap<String, Vec<ParamOwnership>>,

    /// What exported functions require of a call's arguments beyond their
    /// types: a where-clause, the bounds a body infers of them.
    pub function_constraints: FxHashMap<String, FnConstraints>,

    /// Names of the public interfaces this module declares.
    pub interfaces: FxHashSet<String>,
}

impl ModuleExports {
    /// What an importer of `module_name` sees of its `exports`.
    pub fn new(module_name: String, exports: &ExportedSymbols) -> Self {
        Self {
            module_name,
            functions: exports.functions.clone(),
            struct_defs: exports.struct_defs.clone(),
            sum_type_defs: exports.sum_type_defs.clone(),
            service_defs: exports.service_defs.clone(),
            actor_defs: exports.actor_defs.clone(),
            private_names: exports.private_names.clone(),
            type_aliases: exports.type_aliases.clone(),
            resource_types: exports.resource_types.clone(),
            function_ownership: exports.function_ownership.clone(),
            function_constraints: exports.function_constraints.clone(),
            interfaces: exports
                .trait_defs
                .iter()
                .map(|interface| interface.name.clone())
                .collect(),
        }
    }
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
    /// Service definitions with helper function info.
    pub service_defs: FxHashMap<String, ServiceExportInfo>,
    /// Actor definitions (name -> type scheme).
    pub actor_defs: FxHashMap<String, Scheme>,
    /// Trait definitions declared in this module.
    pub trait_defs: Vec<TraitDef>,
    /// Trait impls declared in this module.
    pub trait_impls: Vec<TraitImplDef>,
    /// Names of private (non-pub) items, for distinguishing "private" from "nonexistent" in errors.
    pub private_names: FxHashSet<String>,
    /// Type alias definitions exported by this module (pub type only).
    pub type_aliases: FxHashMap<String, TypeAliasInfo>,
    /// Affine resource type names exported by this module.
    pub resource_types: FxHashSet<String>,
    /// Parameter ownership modes for exported functions.
    pub function_ownership: FxHashMap<String, Vec<ParamOwnership>>,
    /// What exported functions require of a call's arguments beyond their
    /// types: a where-clause, the bounds a body infers of them.
    pub function_constraints: FxHashMap<String, FnConstraints>,
}

/// Information about an exported service, containing the helper function
/// signatures and method mappings needed by importing modules.
#[derive(Debug, Default, Clone)]
pub struct ServiceExportInfo {
    /// Service name (e.g., "Counter").
    pub name: String,
    /// Helper functions: maps unqualified name (e.g., "start", "increment")
    /// to their type scheme. These are registered as ServiceName.method in
    /// the importing module's type environment.
    pub helpers: FxHashMap<String, Scheme>,
    /// Method names with their generated function names for MIR resolution.
    /// Maps (method_name, generated_fn_name), e.g., ("start", "__service_counter_start").
    pub methods: Vec<(String, String)>,
}

pub const DEFAULT_CLUSTERED_ROUTE_REPLICATION_COUNT: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClusteredRouteReplicationCountSource {
    Default,
    Explicit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClusteredRouteReplicationCount {
    pub value: u32,
    pub source: ClusteredRouteReplicationCountSource,
}

impl ClusteredRouteReplicationCount {
    pub fn defaulted() -> Self {
        Self {
            value: DEFAULT_CLUSTERED_ROUTE_REPLICATION_COUNT,
            source: ClusteredRouteReplicationCountSource::Default,
        }
    }

    pub fn explicit(value: u32) -> Self {
        Self {
            value,
            source: ClusteredRouteReplicationCountSource::Explicit,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClusteredRouteWrapperMetadata {
    pub handler_name: String,
    pub defining_module: Option<String>,
    pub runtime_name: String,
    pub handler_span: TextRange,
    pub replication_count: ClusteredRouteReplicationCount,
}

// ── TypeckResult ────────────────────────────────────────────────────────

/// The result of type checking a Mesh program.
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
    /// Names imported from a standard module, each with its module
    /// (`sqrt` -> `Math`).
    pub stdlib_imports: FxHashMap<String, String>,
    /// Service method mappings imported from other modules.
    /// Maps service_name -> Vec<(method_name, generated_fn_name)>.
    /// Used by the MIR lowerer to populate service_modules for cross-module calls.
    pub imported_service_methods: FxHashMap<String, Vec<(String, String)>>,
    /// Locally-defined service export info (for collect_exports).
    /// Maps service_name -> ServiceExportInfo with resolved helper types.
    /// Populated during infer_service_def, consumed by collect_exports.
    pub local_service_exports: FxHashMap<String, ServiceExportInfo>,
    /// Maps call-site TextRange -> mangled callee name (e.g. "slugify__2").
    /// Non-empty only when a call reaches an arity-overloaded fn.
    /// Consumed by the MIR lowerer to emit the correct mangled function reference.
    pub overloaded_call_targets: FxHashMap<TextRange, String>,
    /// Top-level fn names this module defines at more than one arity: each
    /// arity is its own function, named `name__N`.
    pub overloaded_fn_names: FxHashSet<String>,
    /// Metadata for `HTTP.clustered(...)` wrappers keyed by wrapper call range.
    /// Consumed by later lowering so clustered routes reuse declared-handler
    /// runtime-name/count truth instead of inventing an HTTP-only path.
    pub clustered_route_wrappers: FxHashMap<TextRange, ClusteredRouteWrapperMetadata>,
    /// Ranges of function arguments passed where a callback returning `()` is
    /// expected, whose own non-unit result is discarded. Lowering wraps each in
    /// an adapter that calls it and returns `()`.
    pub discarded_callback_results: FxHashSet<TextRange>,
    /// Ownership modes keyed by the direct callee spelling/symbol used by lowering.
    pub function_ownership: FxHashMap<String, Vec<ParamOwnership>>,
    /// What each function requires of a call's arguments beyond their types
    /// (its where-clause, the bounds its body infers of them), by the name
    /// `collect_exports` exports it under.
    pub fn_constraints: FxHashMap<String, FnConstraints>,
    /// Associated types reached through a type parameter: (the variable
    /// standing for the type, trait, associated type name, receiver type).
    /// A specialization knows the receiver, and so the associated type.
    pub assoc_projections: Vec<(Ty, String, String, Ty)>,
}

impl TypeckResult {
    /// Render all type errors as formatted diagnostic strings.
    ///
    /// Accepts `DiagnosticOptions` to control color and output format.
    /// Each error is rendered with labeled source spans, error codes, and
    /// fix suggestions when applicable.
    pub fn render_errors(
        &self,
        source: &str,
        filename: &str,
        options: &DiagnosticOptions,
    ) -> Vec<String> {
        self.errors
            .iter()
            .map(|err| diagnostics::render_diagnostic(err, source, filename, options, None))
            .collect()
    }
}

/// Type-check a parsed Mesh program.
///
/// This is the main entry point for the type checker. It walks the AST,
/// infers types for all expressions, checks type annotations, and reports
/// errors.
pub fn check(parse: &mesh_parser::Parse) -> TypeckResult {
    infer::infer(parse)
}

/// Type-check a parsed Mesh program with pre-resolved imports.
///
/// This is the multi-module entry point. The ImportContext contains
/// symbols from already-type-checked dependency modules.
pub fn check_with_imports(parse: &mesh_parser::Parse, import_ctx: &ImportContext) -> TypeckResult {
    infer::infer_with_imports(parse, import_ctx)
}

/// Collect what a type-checked module exports: its public functions (each
/// arity of an overloaded one), structs, sum types, aliases and interfaces,
/// its actors and services, and the impls it declares or derives, with the
/// names of the definitions it keeps private.
pub fn collect_exports(parse: &mesh_parser::Parse, typeck: &TypeckResult) -> ExportedSymbols {
    use mesh_parser::ast::item::Item;

    let tree = parse.tree();
    let mut exports = ExportedSymbols::default();

    // What the module makes public, and the names of what it keeps private
    // (an import of one is "private", not "not found"). Actors and services
    // have no `pub` and are always exported; `Option`, `Result` and
    // `Ordering` are the built-in types, whatever a module says.
    let registry = &typeck.type_registry;
    for item in tree.items() {
        let (name, public) = match &item {
            Item::FnDef(def) => (def.name(), def.visibility().is_some()),
            Item::StructDef(def) => (def.name(), def.visibility().is_some()),
            Item::SumTypeDef(def) => (def.name(), def.visibility().is_some()),
            Item::TypeAliasDef(def) => (def.name(), def.visibility().is_some()),
            Item::InterfaceDef(def) => (def.name(), def.visibility().is_some()),
            Item::ActorDef(def) => (def.name(), true),
            _ => continue,
        };
        let Some(name) = name.and_then(|n| n.text()) else {
            continue;
        };
        if matches!(item, Item::SumTypeDef(_))
            && ["Option", "Result", "Ordering"].contains(&name.as_str())
        {
            continue;
        }
        if !public {
            exports.private_names.insert(name);
            continue;
        }
        let ty = typeck.types.get(&item.syntax().text_range());
        match item {
            Item::FnDef(fn_def) => {
                let Some(ty) = ty else {
                    continue;
                };
                // Each arity of an overloaded name is its own function.
                let params = fn_def.param_list();
                let export_name = if typeck.overloaded_fn_names.contains(&name) {
                    let arity = params.iter().flat_map(|list| list.params()).count();
                    format!("{name}__{arity}")
                } else {
                    name
                };
                if let Some(constraints) = typeck.fn_constraints.get(&export_name) {
                    exports
                        .function_constraints
                        .insert(export_name.clone(), constraints.clone());
                }
                exports
                    .functions
                    .insert(export_name.clone(), Scheme::normalize_from_ty(ty.clone()));
                let ownership = params
                    .iter()
                    .flat_map(|list| list.params())
                    .map(|parameter| parameter.ownership())
                    .collect();
                exports.function_ownership.insert(export_name, ownership);
            }
            Item::StructDef(_) => {
                if registry.is_resource_name(&name) {
                    exports.resource_types.insert(name.clone());
                }
                let def = registry.struct_defs.get(&name).cloned();
                exports.struct_defs.extend(def.map(|def| (name, def)));
            }
            Item::SumTypeDef(_) => {
                let def = registry.sum_type_defs.get(&name).cloned();
                exports.sum_type_defs.extend(def.map(|def| (name, def)));
            }
            Item::TypeAliasDef(_) => {
                let def = registry.type_aliases.get(&name).cloned();
                exports.type_aliases.extend(def.map(|def| (name, def)));
            }
            Item::InterfaceDef(_) => {
                let def = typeck.trait_registry.get_trait(&name).cloned();
                exports.trait_defs.extend(def);
            }
            _ => {
                let scheme = ty.map(|ty| (name, Scheme::normalize_from_ty(ty.clone())));
                exports.actor_defs.extend(scheme);
            }
        }
    }
    for (name, info) in &typeck.local_service_exports {
        exports.service_defs.insert(name.clone(), info.clone());
    }

    // For trait impls: collect from explicit `impl Trait for Type` AST nodes,
    // plus impls generated by `deriving(...)` clauses on structs/sum types.
    let mut local_impl_traits: Vec<(String, String)> = Vec::new(); // (trait_name, type_name)

    // 1. Explicit impl blocks in the AST.
    for item in tree.items() {
        if let Item::ImplDef(ref impl_def) = item {
            let trait_name = impl_def.interface_name().map(|t| t.text().to_string());
            let type_name = impl_def.type_name().map(|t| t.text().to_string());

            if let (Some(tn), Some(ty)) = (trait_name, type_name) {
                local_impl_traits.push((tn, ty));
            }
        }
    }

    // 2. Impls generated by deriving(...) clauses on structs and sum types.
    //    These don't have explicit ImplDef AST nodes but are registered in
    //    the trait registry during struct/sum type processing.
    for item in tree.items() {
        // Without a deriving clause a type gets the default derives (the
        // loop below exports only those the registry actually holds).
        let defaults = |traits: &[&str]| traits.iter().map(|t| t.to_string()).collect();
        let (type_name, derive_traits) = match &item {
            Item::StructDef(struct_def) => {
                let name = struct_def.name().and_then(|n| n.text());
                let traits = if struct_def.has_deriving_clause() {
                    struct_def.deriving_traits()
                } else {
                    defaults(&["Debug", "Eq", "Ord", "Hash"])
                };
                (name, traits)
            }
            Item::SumTypeDef(sum_def) => {
                let name = sum_def.name().and_then(|n| n.text());
                let traits = if sum_def.has_deriving_clause() {
                    sum_def.deriving_traits()
                } else {
                    defaults(&["Debug", "Eq", "Ord"])
                };
                (name, traits)
            }
            _ => (None, vec![]),
        };
        if let Some(type_name) = type_name {
            for trait_name in derive_traits {
                // Map user-facing derive names to internal trait names.
                // "Json" derives both ToJson and FromJson.
                let internal_traits: &[&str] = match trait_name.as_str() {
                    "Json" => &["ToJson", "FromJson"],
                    "Row" => &["FromRow"],
                    _ => {
                        // For Eq, Ord, Display, Debug, Hash, Schema etc.
                        // store the name directly -- we'll match below.
                        &[] // handled by pushing single name
                    }
                };
                if internal_traits.is_empty() {
                    local_impl_traits.push((trait_name, type_name.clone()));
                } else {
                    for &t in internal_traits {
                        local_impl_traits.push((t.to_string(), type_name.clone()));
                    }
                }
            }
        }
    }

    for impl_def in typeck.trait_registry.all_impls() {
        for (tn, ty) in &local_impl_traits {
            if impl_def.trait_name == *tn && impl_def.impl_type_name == *ty {
                // Avoid duplicates (explicit impl + deriving could overlap)
                if !exports.trait_impls.iter().any(|i| {
                    i.trait_name == impl_def.trait_name
                        && i.impl_type_name == impl_def.impl_type_name
                }) {
                    exports.trait_impls.push(impl_def.clone());
                }
            }
        }
    }

    exports
}
