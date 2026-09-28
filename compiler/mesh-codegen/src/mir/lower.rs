//! AST-to-MIR lowering.
//!
//! Converts the typed Rowan CST (Parse + TypeckResult) to the MIR representation.
//! Handles desugaring of pipe operators, string interpolation, and closure conversion.

use std::collections::{HashMap, HashSet};

use mesh_parser::ast::expr::{
    BinaryExpr, CallExpr, CaseExpr, ClosureExpr, Expr, FieldAccess, ForInExpr, IfExpr, JsonExpr,
    LinkExpr, ListLiteral, Literal, MapLiteral, MatchArm, NameRef, PipeExpr, ReceiveExpr,
    ReturnExpr, SendExpr, SlotPipeExpr, SpawnExpr, StringExpr, StructLiteral, StructUpdate,
    TryExpr, TupleExpr, UnaryExpr, WhileExpr,
};
use mesh_parser::ast::item::{
    ActorDef, Block, FnDef, GuardClause, ImplDef, InterfaceMethod, Item, LetBinding, ParamList,
    ParamOwnership, RelationshipDecl, ServiceDef, SourceFile, StructDef, SumTypeDef, SupervisorDef,
};
use mesh_parser::ast::pat::Pattern;
use mesh_parser::ast::AstNode;
use mesh_parser::syntax_kind::SyntaxKind;
use mesh_parser::Parse;
use mesh_typeck::error::TypeError;
use mesh_typeck::ty::Ty;
use mesh_typeck::{ClusteredRouteWrapperMetadata, TraitRegistry, TypeckResult};
use rowan::TextRange;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::declared::declared_route_wrapper_name;

use super::types::{mangle_type_name, mir_type_to_impl_name, mir_type_to_ty, resolve_type};
use super::{
    sum_type_base, BinOp, MirChildSpec, MirExpr, MirFunction, MirLiteral, MirMatchArm, MirModule,
    MirNativeFunction, MirPattern, MirResourceDestructor, MirResourceField, MirResourceMoveSource,
    MirResourceVariant, MirStructDef, MirSumTypeDef, MirType, MirVariantDef, MsgShape,
    ServiceDispatch, UnaryOp,
};

// ── Helpers ──────────────────────────────────────────────────────────

/// Return true if `ty` is the `Json` newtype introduced in Phase 132.
///
/// Json is represented as `Ty::Con(TyCon { name: "Json", .. })` and resolves
/// to `MirType::Ptr` at the LLVM level (see types.rs resolve_con).
/// Codegen uses this predicate to detect Json-typed values and pass them
/// through as raw opaque pointers instead of re-encoding them as strings.
fn ty_is_json(ty: &Ty) -> bool {
    matches!(ty, Ty::Con(con) if con.name == "Json")
}

/// `ty`'s constructor name and arguments (`List<Int>`: `List`, `[Int]`).
pub(crate) fn ty_head(ty: &Ty) -> Option<(&str, &[Ty])> {
    match ty {
        Ty::Con(tc) => Some((tc.name.as_str(), &[])),
        Ty::App(con, args) => Some((ty_head(con)?.0, args.as_slice())),
        _ => None,
    }
}

/// The element types of `ty` when it is the collection `name`: `[T]` for a
/// `List<T>` or `Set<T>`, `[K, V]` for a `Map<K, V>`. The checker gives a
/// collection its element types, a bare annotation's included.
fn collection_elems(ty: &Ty, name: &str) -> Option<Vec<Ty>> {
    let (_, args) = ty_head(ty).filter(|(head, _)| *head == name)?;
    Some(args.to_vec())
}

/// The element type of a `List<T>`.
fn list_elem(ty: &Ty) -> Option<Ty> {
    collection_elems(ty, "List").map(|mut elems| elems.swap_remove(0))
}

/// The name prefix of the temporaries resource scopes bind their values to
/// (see `Lowerer::wrap_resource_scope`).
const RESOURCE_TEMP_PREFIX: &str = "__resource_value_";

/// How a call of `Base.method(...)` whose base names an interface or a type
/// is lowered (see `Lowerer::qualified_route`).
enum QualifiedRoute {
    /// `Iface.method(value, ...)`: the interface's impl for the value's type.
    Interface(String, String),
    /// `Type.method(value, ...)`: `value.method(...)`.
    TypeMethod(String),
    /// A static method: the impl function called.
    Static(String),
    /// `Type.from(value)` or `Type.try_from(value)` (the interface and the
    /// method, then the type): the impl for the value's type.
    Conversion(String, String, String),
}

/// A trait's type argument as the impl's mangled name spells it: a named
/// type by its bare name, any other in full (`From<List<Int>>` is
/// `From_List_of_Int_end`).
fn trait_arg_name(ty: &Ty) -> String {
    match ty_head(ty) {
        Some((name, [])) => name.to_string(),
        _ => Lowerer::ty_specialization_component(ty),
    }
}

/// The runtime function a built-in impl's method is (`Hash__hash__Int` is
/// `mesh_hash_int`), else `mangled` itself.
fn builtin_trait_redirect(mangled: String) -> String {
    match mangled.as_str() {
        "Display__to_string__Int" | "Debug__inspect__Int" => "mesh_int_to_string".to_string(),
        "Display__to_string__Float" | "Debug__inspect__Float" => "mesh_float_to_string".to_string(),
        "Display__to_string__Bool" | "Debug__inspect__Bool" => "mesh_bool_to_string".to_string(),
        "Hash__hash__Int" => "mesh_hash_int".to_string(),
        "Hash__hash__Float" => "mesh_hash_float".to_string(),
        "Hash__hash__Bool" => "mesh_hash_bool".to_string(),
        "Hash__hash__String" => "mesh_hash_string".to_string(),
        // Built-in From dispatch (Phase 77)
        "From_Int__from__Float" => "mesh_int_to_float".to_string(),
        "From_Int__from__String" => "mesh_int_to_string".to_string(),
        "From_Float__from__String" => "mesh_float_to_string".to_string(),
        "From_Bool__from__String" => "mesh_bool_to_string".to_string(),
        _ => mangled,
    }
}

/// Build a mangled trait method name, incorporating trait type args when present.
/// Non-parameterized: `Trait__method__Type` (e.g., `Display__to_string__Int`)
/// Parameterized: `Trait_TypeArg__method__ImplType` (e.g., `From_Int__from__Float`)
fn mangle_trait_method(
    trait_name: &str,
    trait_type_args: &[String],
    method_name: &str,
    impl_type_name: &str,
) -> String {
    if trait_type_args.is_empty() {
        format!("{}__{}__{}", trait_name, method_name, impl_type_name)
    } else {
        let args_str = trait_type_args.join("_");
        format!(
            "{}_{}__{}__{}",
            trait_name, args_str, method_name, impl_type_name
        )
    }
}

/// For every sum type, the sum types it reaches through payloads held by
/// value (directly, or through such payloads of those types). A payload
/// whose type reaches back to the owner would need the owner's layout inside
/// its own; it is stored boxed instead, which is what makes a self- or
/// mutually-recursive type finite.
fn sum_type_reach(registry: &mesh_typeck::TypeRegistry) -> HashMap<String, HashSet<String>> {
    let direct = |ty: &Ty| -> Option<String> {
        let (name, _) = ty_head(ty)?;
        registry
            .sum_type_defs
            .contains_key(name)
            .then(|| name.to_string())
    };
    let mut reach: HashMap<String, HashSet<String>> = registry
        .sum_type_defs
        .iter()
        .map(|(name, info)| {
            let targets = info
                .variants
                .iter()
                .flat_map(|v| v.fields.iter())
                .filter_map(|f| match f {
                    mesh_typeck::VariantFieldInfo::Positional(ty)
                    | mesh_typeck::VariantFieldInfo::Named(_, ty) => direct(ty),
                })
                .collect();
            (name.clone(), targets)
        })
        .collect();
    loop {
        let mut grew = false;
        for name in registry.sum_type_defs.keys() {
            let closure: HashSet<String> = reach[name]
                .iter()
                .flat_map(|target| reach.get(target).into_iter().flatten().cloned())
                .collect();
            let set = reach.get_mut(name).unwrap();
            for target in closure {
                grew |= set.insert(target);
            }
        }
        if !grew {
            return reach;
        }
    }
}

/// `ty` with each type parameter `subst` names (`Ty::Con("T")`) replaced by
/// its type.
fn substitute_type_params(ty: &Ty, subst: &HashMap<String, &Ty>) -> Ty {
    ty.replace_cons(&mut |con| subst.get(&con.name).map(|ty| (*ty).clone()))
}

/// Whether a value of this representation is a word of plain bits, which a
/// `Some` payload holds boxed: the runtime hands such an element back raw.
fn is_scalar_word(ty: &MirType) -> bool {
    matches!(
        ty,
        MirType::Int | MirType::Float | MirType::Bool | MirType::Pid(_)
    )
}

/// `call`, an `Option` whose `Some` payload the runtime set to an element's
/// raw scalar word, with that payload boxed, as a `Some` payload is read
/// through a pointer.
fn boxed_scalar_option(call: MirExpr) -> MirExpr {
    let ty = call.ty().clone();
    MirExpr::Call {
        func: Box::new(MirExpr::Var(
            "mesh_option_box_scalar".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(ty.clone())),
        )),
        args: vec![call],
        ty,
    }
}

/// Service helper arguments as MIR variables, marked with the shape of the
/// heap references they carry to the service actor.
fn shaped_params(names: &[String], types: &[MirType], shapes: &[MsgShape]) -> Vec<MirExpr> {
    names
        .iter()
        .zip(types)
        .zip(shapes)
        .map(|((name, ty), shape)| {
            let value = MirExpr::Var(name.clone(), ty.clone());
            if shape.is_scalar() {
                value
            } else {
                MirExpr::Shaped {
                    value: Box::new(value),
                    shape: shape.clone(),
                }
            }
        })
        .collect()
}

// ── Lowerer ──────────────────────────────────────────────────────────

/// The AST-to-MIR lowering context.
struct Lowerer<'a> {
    /// Type map from typeck: TextRange -> Ty.
    types: &'a FxHashMap<TextRange, Ty>,
    /// Associated types reached through type parameters (see `TypeckResult`).
    assoc_projections: &'a [(Ty, String, String, Ty)],
    /// Types of the function being lowered, with its type variables replaced by
    /// what this specialization was called with. A function with unannotated
    /// parameters is checked once, generically, and lowered once per usage
    /// type; without this, an inner expression typed by one of its variables
    /// (`Tuple.first(p)` in `fn head(p)`) had no concrete type to lower with.
    spec_types: FxHashMap<TextRange, Ty>,
    /// Type registry for struct/sum type lookups.
    registry: &'a mesh_typeck::TypeRegistry,
    /// Which sum types each sum type reaches through by-value payloads; a
    /// payload naming a type on a cycle with its owner is stored boxed.
    sum_reach: HashMap<String, HashSet<String>>,
    /// Trait registry for trait method dispatch resolution.
    trait_registry: &'a TraitRegistry,
    /// Default method body text ranges from interface definitions.
    /// Keyed by `(trait_name, method_name)`, value is the TextRange of
    /// the INTERFACE_METHOD node containing the default body.
    default_method_bodies: &'a FxHashMap<(String, String), TextRange>,
    /// The parse tree, used for looking up default method body AST nodes.
    parse: &'a Parse,
    /// Default method bodies of interfaces declared in other modules, by
    /// (interface, method): that module's syntax tree, its types, and the
    /// method's range there.
    foreign_defaults: HashMap<(String, String), ForeignDefault<'a>>,
    /// Functions being built.
    functions: Vec<MirFunction>,
    /// Bodyless native archive functions.
    native_functions: Vec<MirNativeFunction>,
    /// Struct definitions.
    structs: Vec<MirStructDef>,
    /// Sum type definitions.
    sum_types: Vec<MirSumTypeDef>,
    /// Scope stack for local variable types.
    scopes: Vec<HashMap<String, MirType>>,
    /// Counter for generating unique lifted closure function names.
    closure_counter: u32,
    /// Names of known functions (for distinguishing direct calls from closure calls).
    known_functions: HashMap<String, MirType>,
    /// Source-declared ownership mode for each direct function parameter.
    ownership_signatures: HashMap<String, Vec<ParamOwnership>>,
    /// Entry function name, if found.
    entry_function: Option<String>,
    /// Service module names (for field access resolution).
    /// Maps service name -> list of (method_name, generated_fn_name) pairs.
    service_modules: HashMap<String, Vec<(String, String)>>,
    /// Tracks which monomorphized trait functions have been generated for generic types.
    /// Prevents duplicate generation when the same generic struct is instantiated
    /// multiple times (e.g., Box<Int> used in multiple places).
    monomorphized_trait_fns: HashSet<String>,
    /// Let-bound polymorphic closures: for each name, the compiled copies
    /// keyed by the use type they were specialized for (`poly_closure_specs`).
    poly_closure_specs: HashMap<String, Vec<(Ty, String)>>,
    /// User-defined module namespaces for qualified access (Phase 39).
    /// Maps module namespace name (e.g., "Math") to list of exported function names.
    user_modules: HashMap<String, Vec<String>>,
    /// Function names imported via `from Module import name1, name2` (Phase 39).
    /// These are directly callable without qualification and must not go through
    /// trait dispatch.
    imported_functions: HashSet<String>,
    /// Names imported from a standard module, each with its module
    /// (`sqrt` -> `Math`): the name lowers as the qualified function does.
    stdlib_imports: &'a FxHashMap<String, String>,
    /// Module name for name-mangling private functions (Phase 41).
    /// Empty string means single-file mode (no prefix applied).
    module_name: String,
    /// Set of pub function names that should NOT be module-prefixed (Phase 41).
    pub_functions: HashSet<String>,
    /// Names of user-defined functions from FnDef items (Phase 41).
    /// Used to distinguish actual function definitions from variant constructors,
    /// actors, etc. when applying module-qualified naming at call sites.
    user_fn_defs: HashSet<String>,
    /// Maps user-defined function name → all concrete function types observed at
    /// call sites where the function was passed as a value argument (not called directly).
    ///
    /// Example: `fn pass(req, next) do next(req) end` used in `HTTP.use(r, pass)`.
    /// At the usage site, the typeck resolves `pass` to `Fn(Request, ...) -> Response`.
    /// This map lets the lowerer recover the correct parameter types for functions whose
    /// parameters were generalized (as Ty::Var) before the call site could constrain them.
    fn_value_usage_types: HashMap<String, Vec<Ty>>,
    /// Inferred functions whose definitions still contain TyVar placeholders but whose
    /// call sites expose one or more concrete function signatures.
    ///
    /// Single-signature entries let the lowerer repair the base ABI directly.
    /// Multi-signature entries require per-signature MIR clones so each call site can
    /// reference a concrete symbol instead of collapsing to the first observed ABI.
    inferred_fn_specializations: HashMap<String, Vec<Ty>>,
    /// The concrete types this module's generic functions call other
    /// modules' generic functions at, in their specializations: the
    /// defining modules must emit those (`imported_specializations`).
    imported_specializations: HashMap<String, Vec<Ty>>,
    /// Current enclosing function's return type (Phase 45).
    /// Set when entering a function body, used by lower_try_expr for early-return
    /// variant construction. Save/restore pattern for nested functions and closures.
    current_fn_return_type: Option<MirType>,
    /// Type-checker form of the current return type. Unlike MIR names, this
    /// preserves nested generic boundaries needed to compare `?` error types.
    current_fn_return_typeck: Option<Ty>,
    /// Counter for generating unique try binding names (Phase 45).
    /// Incremented per `?` usage to avoid shadowing in nested `?` expressions.
    try_counter: u32,
    /// Counter for compiler-generated resource cleanup result temporaries.
    resource_temp_counter: u32,
    /// Resources a pattern's `_` stands for, bound to names of their own
    /// (see `lower_pattern_with_expected`), for the arm, `let` or clause the
    /// pattern belongs to to destroy as it destroys named ones.
    discarded_resources: Vec<(String, Ty)>,
    /// Numbers the bindings of generated Json code (`json_fresh`).
    json_counter: u32,
    /// While lowering an actor with parameters: its name, its body
    /// function's name, and the parameter types, for self-calls.
    actor_body_target: Option<(String, String, Vec<MirType>)>,
    /// Enables special lowering of test DSL constructs (assert, assert_eq, assert_ne,
    /// assert_raises). Detected in lower_source_file's pre-scan pass by looking
    /// for `fn __test_body_*` or `fn __test_describe_*` function definitions
    /// (injected by the preprocessor).
    is_test_mode: bool,
    /// Maps call-site TextRange -> mangled callee name (e.g. "slugify__2").
    /// Populated by the typechecker for arity-overloaded calls; used here to
    /// emit the correct mangled function reference in lower_call_expr.
    overloaded_call_targets: HashMap<rowan::TextRange, String>,
    /// Top-level fn names this module defines at more than one arity: each
    /// arity is its own function, named `name__N` (see `fn_def_name`).
    overloaded_fn_names: &'a FxHashSet<String>,
    /// Metadata for `HTTP.clustered(...)` wrappers keyed by wrapper call range.
    clustered_route_wrappers: &'a FxHashMap<TextRange, ClusteredRouteWrapperMetadata>,
    /// Wrapper spans that successfully lowered to a concrete bare route shim.
    consumed_clustered_route_wrappers: HashSet<TextRange>,
    /// Callback arguments whose result is discarded (see `discard_callback_result`).
    discarded_callback_results: &'a FxHashSet<TextRange>,
    /// `Json` arguments passed where a `String` is expected (see `json_text`).
    json_text_arguments: &'a FxHashSet<TextRange>,
    /// Fail-closed lowering errors gathered while rewriting clustered routes.
    lowering_errors: Vec<String>,
    /// Each service loop's handlers, calls and casts: tag, function and
    /// argument count.
    service_dispatch: ServiceDispatch,
    /// Entry functions of the actors lowered so far; see `MirModule::actors`.
    actors: Vec<String>,
    /// While lowering a supervisor child's start: the `spawn` that ends it,
    /// which runs the spawned actor in place (`supervisor_child_entry`).
    supervised_spawn: Option<TextRange>,
}

/// Walk through Let/Block wrappers to find the effective return type of a MIR expression.
/// Let { ty, body, .. } has `ty` as the binding's value type, but the effective type is body's type.
/// Block(exprs, ty) already stores the last expression's type as `ty`.
fn effective_return_type(expr: &MirExpr) -> MirType {
    match expr {
        MirExpr::Let { body, .. } => effective_return_type(body),
        MirExpr::Block(_, ty) => ty.clone(),
        other => other.ty().clone(),
    }
}

fn runtime_value_type(ty: MirType) -> MirType {
    if matches!(ty, MirType::Tuple(_)) {
        MirType::Ptr
    } else {
        ty
    }
}

/// The parameter and result types of `ty` when it is a function type.
fn fn_type_parts(ty: &Ty) -> Option<(&[Ty], &Ty)> {
    match ty {
        Ty::Fun(params, ret) => Some((params, ret)),
        _ => None,
    }
}

/// The parameter and result types of a function's type (the type checker
/// gives every function one).
fn fun_parts(ty: &Ty) -> (&[Ty], &Ty) {
    fn_type_parts(ty).expect("the type checker gives a function a function type")
}

/// What a function's clauses match: its one parameter, several as a tuple,
/// or nothing.
fn clause_scrutinee(params: &[(String, MirType)]) -> MirExpr {
    let mut vars: Vec<MirExpr> = params
        .iter()
        .map(|(name, ty)| MirExpr::Var(name.clone(), ty.clone()))
        .collect();
    match vars.len() {
        0 => MirExpr::Unit,
        1 => vars.pop().unwrap(),
        _ => MirExpr::Call {
            func: Box::new(MirExpr::Var(
                "__mesh_make_tuple".to_string(),
                MirType::FnPtr(
                    vars.iter().map(|var| var.ty().clone()).collect(),
                    Box::new(MirType::Ptr),
                ),
            )),
            args: vars,
            ty: MirType::Ptr,
        },
    }
}

/// A service's new state, `body`, and its type, as the service loop keeps
/// it: a Unit state is the Int 0.
fn service_state(body: MirExpr) -> (MirExpr, MirType) {
    match effective_return_type(&body) {
        MirType::Unit => (
            MirExpr::Block(vec![body, MirExpr::IntLit(0, MirType::Int)], MirType::Int),
            MirType::Int,
        ),
        ty => (body, ty),
    }
}

/// `ty` with every type variable left open taken as Unit.
fn apply_default_unit(ty: &Ty) -> Ty {
    match ty {
        Ty::Var(_) => Ty::Tuple(vec![]),
        Ty::Con(_) | Ty::Never => ty.clone(),
        Ty::Fun(params, ret) => Ty::Fun(
            params.iter().map(apply_default_unit).collect(),
            Box::new(apply_default_unit(ret)),
        ),
        Ty::App(con, args) => Ty::App(
            Box::new(apply_default_unit(con)),
            args.iter().map(apply_default_unit).collect(),
        ),
        Ty::Tuple(elems) => Ty::Tuple(elems.iter().map(apply_default_unit).collect()),
    }
}

/// Read off which type each variable of `generic` stands for in `concrete`.
fn bind_type_vars(generic: &Ty, concrete: &Ty, bindings: &mut Vec<(mesh_typeck::ty::TyVar, Ty)>) {
    let bind_all = |generic: &[Ty], concrete: &[Ty], bindings: &mut Vec<_>| {
        for (generic, concrete) in generic.iter().zip(concrete) {
            bind_type_vars(generic, concrete, bindings);
        }
    };
    match (generic, concrete) {
        (Ty::Var(var), concrete) => {
            if !bindings.iter().any(|(bound, _)| bound == var) {
                bindings.push((*var, concrete.clone()));
            }
        }
        (Ty::Fun(params, ret), Ty::Fun(concrete_params, concrete_ret)) => {
            bind_all(params, concrete_params, bindings);
            bind_type_vars(ret, concrete_ret, bindings);
        }
        (Ty::Tuple(elems), Ty::Tuple(concrete_elems)) => bind_all(elems, concrete_elems, bindings),
        // A tuple known by its first elements, against the whole tuple.
        (row, Ty::Tuple(all)) if row.as_tuple_row().is_some() => {
            let (elems, tail) = row.as_tuple_row().unwrap();
            if all.len() >= elems.len() {
                let (named, rest) = all.split_at(elems.len());
                bind_all(elems, named, bindings);
                bind_type_vars(tail, &Ty::Tuple(rest.to_vec()), bindings);
            }
        }
        (Ty::App(_, args), Ty::App(_, concrete_args)) => bind_all(args, concrete_args, bindings),
        _ => {}
    }
}

/// `ty` with the variables in `bindings` replaced.
fn apply_type_vars(ty: &Ty, bindings: &[(mesh_typeck::ty::TyVar, Ty)]) -> Ty {
    let all = |tys: &[Ty]| tys.iter().map(|ty| apply_type_vars(ty, bindings)).collect();
    match ty {
        Ty::Var(var) => bindings
            .iter()
            .find(|(bound, _)| bound == var)
            .map_or_else(|| ty.clone(), |(_, concrete)| concrete.clone()),
        Ty::Fun(params, ret) => Ty::Fun(all(params), Box::new(apply_type_vars(ret, bindings))),
        Ty::Tuple(elems) => Ty::Tuple(all(elems)),
        Ty::App(con, args) => Ty::App(con.clone(), all(args)).normalize_tuple_row(),
        Ty::Con(_) | Ty::Never => ty.clone(),
    }
}

fn uniform_callback_index(name: &str) -> Option<usize> {
    match name {
        "mesh_list_map" | "mesh_list_filter" | "mesh_list_sort" | "mesh_list_find"
        | "mesh_list_any" | "mesh_list_all" | "mesh_list_flat_map" | "mesh_range_map"
        | "mesh_range_filter" | "mesh_job_map" | "mesh_iter_map" | "mesh_iter_filter"
        | "mesh_iter_any" | "mesh_iter_all" | "mesh_iter_find" => Some(1),
        "mesh_list_reduce" | "mesh_iter_reduce" => Some(2),
        // A row decoder returns a `Result`, which the list of rows holds as a slot.
        "mesh_pg_query_as" | "mesh_pool_query_as" => Some(3),
        // The runtime calls a job as `fn(env) -> i64`, whatever it returns.
        "mesh_job_async" => Some(0),
        "mesh_timer_apply_after" => Some(1),
        _ => None,
    }
}

/// Map a MIR type to its PostgreSQL SQL type string.
///
/// Used by `generate_schema_metadata` to produce `__field_types__()` entries.
fn mir_type_to_sql_type(ty: &MirType) -> &'static str {
    match ty {
        MirType::Int => "BIGINT",
        MirType::Float => "DOUBLE PRECISION",
        MirType::Bool => "BOOLEAN",
        _ => "TEXT",
    }
}

impl<'a> Lowerer<'a> {
    fn new(
        typeck: &'a TypeckResult,
        parse: &'a Parse,
        module_name: &str,
        pub_fns: &HashSet<String>,
        inferred_fn_usage_types: &HashMap<String, Vec<Ty>>,
    ) -> Self {
        let mut ownership_signatures: HashMap<String, Vec<ParamOwnership>> = typeck
            .function_ownership
            .iter()
            .map(|(name, modes)| (name.clone(), modes.clone()))
            .collect();
        for (name, modes) in typeck.function_ownership.iter() {
            if name.starts_with("crypto_") || name.starts_with("bytes_builder_") {
                ownership_signatures
                    .entry(format!("mesh_{name}"))
                    .or_insert_with(|| modes.clone());
            }
        }
        for alias in ["Secret.destroy", "secret_destroy", "mesh_secret_destroy"] {
            ownership_signatures
                .entry(alias.to_string())
                .or_insert_with(|| vec![ParamOwnership::Consume]);
        }
        for alias in ["Secret.concat", "secret_concat", "mesh_secret_concat"] {
            ownership_signatures
                .entry(alias.to_string())
                .or_insert_with(|| vec![ParamOwnership::Consume, ParamOwnership::Consume]);
        }
        for operation in ["insert", "contains", "copy", "delete", "fork"] {
            let mut modes = if operation == "fork" {
                vec![ParamOwnership::Borrow]
            } else {
                vec![ParamOwnership::Borrow, ParamOwnership::Move]
            };
            if operation == "insert" {
                modes.push(ParamOwnership::Consume);
            }
            for alias in [
                format!("SecretMap.{operation}"),
                format!("secret_map_{operation}"),
                format!("mesh_secret_map_{operation}"),
            ] {
                ownership_signatures
                    .entry(alias)
                    .or_insert_with(|| modes.clone());
            }
        }
        for alias in [
            "SecretMap.merge",
            "secret_map_merge",
            "mesh_secret_map_merge",
        ] {
            ownership_signatures
                .entry(alias.to_string())
                .or_insert_with(|| vec![ParamOwnership::Borrow, ParamOwnership::Consume]);
        }
        for (module, prefix) in [
            ("Secret", "secret"),
            ("SecretMap", "secret_map"),
            ("X25519PrivateKey", "x25519_private_key"),
            ("SigningPrivateKey", "signing_private_key"),
            ("MlKemPrivateKey", "mlkem_private_key"),
        ] {
            for alias in [
                format!("{module}.seal_for_storage"),
                format!("{prefix}_seal_for_storage"),
                format!("mesh_{prefix}_seal_for_storage"),
            ] {
                ownership_signatures.entry(alias).or_insert_with(|| {
                    vec![
                        ParamOwnership::Borrow,
                        ParamOwnership::Borrow,
                        ParamOwnership::Move,
                    ]
                });
            }
            for alias in [
                format!("{module}.unseal_from_storage"),
                format!("{prefix}_unseal_from_storage"),
                format!("mesh_{prefix}_unseal_from_storage"),
            ] {
                ownership_signatures.entry(alias).or_insert_with(|| {
                    vec![
                        ParamOwnership::Move,
                        ParamOwnership::Borrow,
                        ParamOwnership::Move,
                    ]
                });
            }
        }
        for name in ["seal_bytes", "unseal_bytes"] {
            for alias in [
                format!("StorageKey.{name}"),
                format!("storage_key_{name}"),
                format!("mesh_storage_key_{name}"),
            ] {
                ownership_signatures.entry(alias).or_insert_with(|| {
                    vec![
                        ParamOwnership::Move,
                        ParamOwnership::Borrow,
                        ParamOwnership::Move,
                    ]
                });
            }
        }

        Lowerer {
            types: &typeck.types,
            assoc_projections: &typeck.assoc_projections,
            spec_types: FxHashMap::default(),
            registry: &typeck.type_registry,
            sum_reach: sum_type_reach(&typeck.type_registry),
            trait_registry: &typeck.trait_registry,
            default_method_bodies: &typeck.default_method_bodies,
            parse,
            foreign_defaults: HashMap::new(),
            functions: Vec::new(),
            native_functions: Vec::new(),
            structs: Vec::new(),
            sum_types: Vec::new(),
            scopes: vec![HashMap::new()],
            closure_counter: 0,
            known_functions: HashMap::new(),
            ownership_signatures,
            entry_function: None,
            service_modules: typeck
                .imported_service_methods
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            monomorphized_trait_fns: HashSet::new(),
            poly_closure_specs: HashMap::new(),
            user_modules: typeck
                .qualified_modules
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            imported_functions: typeck.imported_functions.iter().cloned().collect(),
            stdlib_imports: &typeck.stdlib_imports,
            module_name: module_name.to_string(),
            pub_functions: pub_fns.clone(),
            user_fn_defs: HashSet::new(),
            fn_value_usage_types: inferred_fn_usage_types.clone(),
            inferred_fn_specializations: inferred_fn_usage_types.clone(),
            imported_specializations: HashMap::new(),
            current_fn_return_type: None,
            current_fn_return_typeck: None,
            try_counter: 0,
            resource_temp_counter: 0,
            discarded_resources: Vec::new(),
            json_counter: 0,
            actor_body_target: None,
            is_test_mode: false,
            overloaded_call_targets: typeck
                .overloaded_call_targets
                .iter()
                .map(|(k, v)| (*k, v.clone()))
                .collect(),
            overloaded_fn_names: &typeck.overloaded_fn_names,
            clustered_route_wrappers: &typeck.clustered_route_wrappers,
            consumed_clustered_route_wrappers: HashSet::new(),
            discarded_callback_results: &typeck.discarded_callback_results,
            json_text_arguments: &typeck.json_text_arguments,
            lowering_errors: Vec::new(),
            supervised_spawn: None,
            service_dispatch: HashMap::new(),
            actors: Vec::new(),
        }
    }

    // ── Scope management ─────────────────────────────────────────────

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn insert_var(&mut self, name: String, ty: MirType) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name, ty);
        }
    }

    fn lookup_var(&self, name: &str) -> Option<MirType> {
        for scope in self.scopes.iter().rev() {
            if let Some(ty) = scope.get(name) {
                return Some(ty.clone());
            }
        }
        None
    }

    /// The variables a closure lowered here may capture: those of the
    /// enclosing functions' scopes, an inner one's shadowing an outer one's.
    /// The global scope holds top-level functions, which a closure calls by
    /// name.
    fn capturable_vars(&self) -> HashMap<String, MirType> {
        self.scopes
            .iter()
            .skip(1)
            .flat_map(|scope| scope.iter())
            .map(|(name, ty)| (name.clone(), ty.clone()))
            .collect()
    }

    fn lookup_non_global_var(&self, name: &str) -> Option<MirType> {
        for scope in self.scopes.iter().skip(1).rev() {
            if let Some(ty) = scope.get(name) {
                return Some(ty.clone());
            }
        }
        None
    }

    /// A fresh name for a generated function of `kind`: `__closure_3`, and
    /// in a project the module's own (`Utils__closure_3`), so two modules'
    /// generated functions never meet when their MIR is merged.
    fn generated_fn_name(&mut self, kind: &str) -> String {
        self.closure_counter += 1;
        if self.module_name.is_empty() {
            format!("__{kind}_{}", self.closure_counter)
        } else {
            format!(
                "{}__{kind}_{}",
                self.module_name.replace('.', "_"),
                self.closure_counter
            )
        }
    }

    fn next_resource_temp(&mut self) -> String {
        let name = format!("{RESOURCE_TEMP_PREFIX}{}", self.resource_temp_counter);
        self.resource_temp_counter += 1;
        name
    }

    fn resource_destructor(&self, ty: &Ty) -> Option<MirResourceDestructor> {
        self.resource_destructor_inner(ty, &mut HashSet::new())
    }

    fn resource_destructor_inner(
        &self,
        ty: &Ty,
        visiting: &mut HashSet<String>,
    ) -> Option<MirResourceDestructor> {
        if !self.registry.is_resource_type(ty) {
            return None;
        }

        if let Ty::Tuple(elements) = ty {
            return Some(MirResourceDestructor::Aggregate(
                elements
                    .iter()
                    .enumerate()
                    .filter_map(|(index, element)| {
                        self.resource_destructor_inner(element, visiting)
                            .map(|destructor| MirResourceField {
                                index: index as u32,
                                ty: resolve_type(element, self.registry),
                                destructor,
                            })
                    })
                    .collect(),
            ));
        }
        // A resource type is a tuple or a named type.
        let (name, arguments) = ty_head(ty)?;
        // A pid names an actor, which owns the messages its type names.
        if name == "Pid" {
            return None;
        }
        if name == "PgConn" {
            return Some(MirResourceDestructor::PgConnection);
        }
        if self.registry.sum_type_defs.contains_key(name) {
            return self.resource_sum_destructor_inner(name, arguments, visiting);
        }
        // A resource without fields (a builtin handle, or an opaque one a
        // package declares) is destroyed by the runtime.
        let Some(definition) = self
            .registry
            .struct_defs
            .get(name)
            .filter(|definition| !definition.fields.is_empty())
        else {
            return Some(MirResourceDestructor::Opaque);
        };
        if !visiting.insert(name.to_string()) {
            return None;
        }
        let substitutions: HashMap<String, &Ty> = definition
            .generic_params
            .iter()
            .cloned()
            .zip(arguments.iter())
            .collect();
        let fields = definition
            .fields
            .iter()
            .enumerate()
            .filter_map(|(index, (_, field_ty))| {
                let field_ty = substitute_type_params(field_ty, &substitutions);
                self.resource_destructor_inner(&field_ty, visiting)
                    .map(|destructor| MirResourceField {
                        index: index as u32,
                        ty: resolve_type(&field_ty, self.registry),
                        destructor,
                    })
            })
            .collect();
        visiting.remove(name);
        Some(MirResourceDestructor::Aggregate(fields))
    }

    fn resource_sum_destructor_inner(
        &self,
        name: &str,
        arguments: &[Ty],
        visiting: &mut HashSet<String>,
    ) -> Option<MirResourceDestructor> {
        let definition = self.registry.sum_type_defs.get(name)?;
        let visit_key = format!("sum:{name}");
        if !visiting.insert(visit_key.clone()) {
            return None;
        }
        let substitutions: HashMap<String, &Ty> = definition
            .generic_params
            .iter()
            .cloned()
            .zip(arguments.iter())
            .collect();
        // Generic sum payloads use the base MIR definition's storage layout.
        // In particular, Result<T, E> stores T/E behind a pointer even when the
        // concrete semantic type is an unboxed integer handle such as PgConn.
        let storage_variants = &self
            .sum_types
            .iter()
            .find(|sum| sum.name == name)
            .expect("every sum type is registered before lowering")
            .variants;
        let variants = definition
            .variants
            .iter()
            .enumerate()
            .filter_map(|(tag, variant)| {
                let concrete_fields = variant
                    .fields
                    .iter()
                    .map(|field| match field {
                        mesh_typeck::VariantFieldInfo::Positional(ty)
                        | mesh_typeck::VariantFieldInfo::Named(_, ty) => {
                            substitute_type_params(ty, &substitutions)
                        }
                    })
                    .collect::<Vec<_>>();
                let field_types = storage_variants[tag].fields.clone();
                let resource_fields = concrete_fields
                    .iter()
                    .enumerate()
                    .filter_map(|(index, field_ty)| {
                        self.resource_destructor_inner(field_ty, visiting)
                            .map(|destructor| MirResourceField {
                                index: index as u32,
                                ty: resolve_type(field_ty, self.registry),
                                destructor,
                            })
                    })
                    .collect::<Vec<_>>();
                (!resource_fields.is_empty()).then_some(MirResourceVariant {
                    tag: tag as u8,
                    field_types,
                    resource_fields,
                })
            })
            .collect();
        visiting.remove(&visit_key);
        Some(MirResourceDestructor::SumVariants(variants))
    }

    fn resource_drop(
        name: &str,
        resource_ty: &MirType,
        destructor: MirResourceDestructor,
    ) -> MirExpr {
        MirExpr::ResourceDrop {
            value: Box::new(MirExpr::Var(
                name.to_string(),
                runtime_value_type(resource_ty.clone()),
            )),
            resource_ty: resource_ty.clone(),
            destructor,
        }
    }

    /// `expression` with `cleanup` run before each way out of it: a return,
    /// a panic, and a `break` or `continue` of a loop outside it
    /// (`loop_depth` counts the loops inside it around the current point).
    fn cleanup_before_exits(
        &mut self,
        expression: MirExpr,
        cleanup: &MirExpr,
        loop_depth: usize,
    ) -> MirExpr {
        match expression {
            MirExpr::Return(value) => {
                let value = self.cleanup_before_exits(*value, cleanup, loop_depth);
                let ty = effective_return_type(&value);
                let name = self.next_resource_temp();
                MirExpr::Let {
                    name: name.clone(),
                    ty: ty.clone(),
                    value: Box::new(value),
                    body: Box::new(MirExpr::Block(
                        vec![
                            cleanup.clone(),
                            MirExpr::Return(Box::new(MirExpr::Var(name, ty))),
                        ],
                        MirType::Never,
                    )),
                }
            }
            panic @ MirExpr::Panic { .. } => {
                MirExpr::Block(vec![cleanup.clone(), panic], MirType::Never)
            }
            MirExpr::Break if loop_depth == 0 => {
                MirExpr::Block(vec![cleanup.clone(), MirExpr::Break], MirType::Never)
            }
            MirExpr::Continue if loop_depth == 0 => {
                MirExpr::Block(vec![cleanup.clone(), MirExpr::Continue], MirType::Never)
            }
            mut other => {
                // A loop's condition, filter and body run inside it; a range's
                // bounds and a collection before it.
                let inside = loop_depth + 1;
                let mut rewrite = |child: &mut MirExpr, depth: usize| {
                    let taken = std::mem::replace(child, MirExpr::Unit);
                    *child = self.cleanup_before_exits(taken, cleanup, depth);
                };
                match &mut other {
                    MirExpr::While { cond, body, .. } => {
                        rewrite(cond, inside);
                        rewrite(body, inside);
                    }
                    MirExpr::ForInRange {
                        start,
                        end,
                        filter,
                        body,
                        ..
                    } => {
                        rewrite(start, loop_depth);
                        rewrite(end, loop_depth);
                        if let Some(filter) = filter {
                            rewrite(filter, inside);
                        }
                        rewrite(body, inside);
                    }
                    MirExpr::ForInList {
                        collection,
                        filter,
                        body,
                        ..
                    }
                    | MirExpr::ForInMap {
                        collection,
                        filter,
                        body,
                        ..
                    }
                    | MirExpr::ForInSet {
                        collection,
                        filter,
                        body,
                        ..
                    }
                    | MirExpr::ForInIterator {
                        iterator: collection,
                        filter,
                        body,
                        ..
                    } => {
                        rewrite(collection, loop_depth);
                        if let Some(filter) = filter {
                            rewrite(filter, inside);
                        }
                        rewrite(body, inside);
                    }
                    _ => {
                        for child in other.children_mut() {
                            rewrite(child, loop_depth);
                        }
                    }
                }
                other
            }
        }
    }

    fn can_fall_through(expression: &MirExpr) -> bool {
        match expression {
            MirExpr::Return(_)
            | MirExpr::Panic { .. }
            | MirExpr::Break
            | MirExpr::Continue
            | MirExpr::TailCall { .. } => false,
            MirExpr::Let { body, .. } => Self::can_fall_through(body),
            MirExpr::Block(expressions, _) => expressions.iter().all(Self::can_fall_through),
            MirExpr::If {
                then_body,
                else_body,
                ..
            } => Self::can_fall_through(then_body) || Self::can_fall_through(else_body),
            MirExpr::Match { arms, .. } => arms.iter().any(|arm| Self::can_fall_through(&arm.body)),
            MirExpr::ActorReceive {
                handler,
                timeout_body,
                ..
            } => {
                handler
                    .as_ref()
                    .is_some_and(|(_, _, body)| Self::can_fall_through(body))
                    || timeout_body.as_deref().is_some_and(Self::can_fall_through)
            }
            _ => true,
        }
    }

    /// The type of `param`, of type `ty`, when the function owns it and it
    /// is a resource, which the function then drops as it ends: any
    /// resource parameter a function, method or default method is given,
    /// unless it is borrowed. A method's `self` is always borrowed from its
    /// caller (see the ownership check's signatures).
    fn owned_resource(&self, param: &mesh_parser::ast::item::Param, ty: Option<&Ty>) -> Option<Ty> {
        let ty = ty?;
        (param.ownership() != ParamOwnership::Borrow
            && !param.is_self()
            && self.registry.is_resource_type(ty))
        .then(|| ty.clone())
    }

    /// `body` inside a resource scope for each of the `owned` resources.
    fn wrap_resource_scopes(&mut self, mut body: MirExpr, owned: Vec<(String, Ty)>) -> MirExpr {
        for (name, ty) in owned.into_iter().rev() {
            body = self.wrap_resource_scope(body, &name, &ty);
        }
        body
    }

    fn wrap_resource_scope(&mut self, body: MirExpr, name: &str, typeck_ty: &Ty) -> MirExpr {
        let resource_ty = resolve_type(typeck_ty, self.registry);
        let Some(destructor) = self.resource_destructor(typeck_ty) else {
            return body;
        };
        let cleanup = Self::resource_drop(name, &resource_ty, destructor);
        let body = self.cleanup_before_exits(body, &cleanup, 0);
        if !Self::can_fall_through(&body) {
            return body;
        }

        let result_ty = effective_return_type(&body);
        let result_name = self.next_resource_temp();
        MirExpr::Let {
            name: result_name.clone(),
            ty: result_ty.clone(),
            value: Box::new(body),
            body: Box::new(MirExpr::Block(
                vec![cleanup, MirExpr::Var(result_name, result_ty.clone())],
                result_ty,
            )),
        }
    }

    // ── Module-qualified naming (Phase 41) ──────────────────────────

    /// Apply module prefix to a private function name.
    ///
    /// Rules:
    /// - Empty module_name (single-file mode): return name unchanged
    /// - "main": unchanged (handled separately as mesh_main)
    /// - Pub functions: unchanged (cross-module references use unqualified name)
    /// - Otherwise: `ModuleName__name` (dots replaced with underscores)
    ///
    /// Every caller names a function the program defines, so a name that
    /// looks like the runtime's (`mesh_`) or an impl's is qualified too.
    fn qualify_name(&self, name: &str) -> String {
        // Single-file mode: no prefix
        if self.module_name.is_empty() {
            return name.to_string();
        }
        // main is handled separately (renamed to mesh_main)
        if name == "main" {
            return name.to_string();
        }
        // Pub functions keep unqualified names for cross-module references
        if self.pub_functions.contains(name) {
            return name.to_string();
        }
        // Apply module prefix: ModuleName__function_name
        format!("{}__{}", self.module_name.replace('.', "_"), name)
    }

    // ── Type resolution helper ───────────────────────────────────────

    /// The MIR type of the value at `range`. A tuple value is a pointer to its
    /// heap block (`runtime_value_type`); `resolve_range_structure` keeps the
    /// element types.
    fn resolve_range(&self, range: TextRange) -> MirType {
        runtime_value_type(self.resolve_range_structure(range))
    }

    fn resolve_range_structure(&self, range: TextRange) -> MirType {
        if let Some(ty) = self.get_ty(range) {
            resolve_type(ty, self.registry)
        } else {
            MirType::Unit
        }
    }

    fn get_ty(&self, range: TextRange) -> Option<&Ty> {
        self.spec_types
            .get(&range)
            .or_else(|| self.types.get(&range))
    }

    /// An impl's interface, the interface's type arguments as its mangled
    /// names spell them (as the type checker read them, where they are
    /// written), and the implementing type: `("From", ["Int"], "Float")`.
    fn impl_names(&self, impl_def: &ImplDef) -> (String, Vec<String>, String) {
        let trait_name = impl_def
            .interface_name()
            .map(|t| t.text().to_string())
            .unwrap_or_else(|| "<unknown>".to_string());
        let trait_type_args = impl_def
            .syntax()
            .children()
            .find(|n| n.kind() == SyntaxKind::GENERIC_ARG_LIST)
            .and_then(|written| self.get_ty(written.text_range())?.args_of(&trait_name))
            .map(|args| args.iter().map(trait_arg_name).collect())
            .unwrap_or_default();
        let type_name = impl_def
            .type_name()
            .map(|t| t.text().to_string())
            .unwrap_or_else(|| "<unknown>".to_string());
        (trait_name, trait_type_args, type_name)
    }

    /// Fill `spec_types` for the function at `fn_range`, checked as `generic`
    /// and lowered here as `concrete`. Returns what was there before, for the
    /// caller to put back.
    fn specialize_types(
        &mut self,
        fn_range: TextRange,
        generic: &Ty,
        concrete: &Ty,
    ) -> FxHashMap<TextRange, Ty> {
        let mut bindings = Vec::new();
        bind_type_vars(generic, concrete, &mut bindings);
        // An associated type reached through a type parameter is known once
        // the parameter is: `Self.Item` of `StrBox` is String.
        for (var, trait_name, assoc, receiver) in self.assoc_projections {
            let Ty::Var(var) = var else { continue };
            if bindings.iter().any(|(bound, _)| bound == var) {
                continue;
            }
            let receiver = apply_type_vars(receiver, &bindings);
            if Self::ty_contains_var(&receiver) {
                continue;
            }
            if let Some(assoc_ty) = self
                .trait_registry
                .resolve_associated_type(trait_name, assoc, &receiver)
            {
                bindings.push((*var, assoc_ty));
            }
        }
        let specialized = if bindings.is_empty() {
            FxHashMap::default()
        } else {
            self.types
                .iter()
                .filter(|(range, ty)| fn_range.contains_range(**range) && Self::ty_contains_var(ty))
                .map(|(range, ty)| (*range, apply_type_vars(ty, &bindings)))
                .collect()
        };
        std::mem::replace(&mut self.spec_types, specialized)
    }

    /// `expr` typed as the bare function it names, when it names one.
    ///
    /// A function used as a value is a closure (`resolve_type`), but one that
    /// is called, or handed to the runtime by name, is just its code pointer.
    /// Anything that is not a local is a function: user-defined, imported,
    /// a trait method or a runtime intrinsic.
    fn as_fn_item(&self, expr: MirExpr) -> MirExpr {
        match expr {
            MirExpr::Var(name, MirType::Closure(params, ret))
                if self.lookup_non_global_var(&name).is_none() =>
            {
                MirExpr::Var(name, MirType::FnPtr(params, ret))
            }
            other => other,
        }
    }

    fn lower_callee(&mut self, callee: &Expr) -> MirExpr {
        let lowered = self.lower_expr(callee);
        self.as_fn_item(lowered)
    }

    /// The function the call at `call_range` runs: `callee`, or the arity
    /// of an overloaded fn the type checker chose for it (`name__N`).
    fn lower_call_target(&mut self, call_range: TextRange, callee: &Expr) -> MirExpr {
        let Some(target) = self.overloaded_call_targets.get(&call_range).cloned() else {
            return self.lower_callee(callee);
        };
        let range = callee.syntax().text_range();
        let ty = self.resolve_range(range);
        let symbol = if self.user_fn_defs.contains(&target) {
            let qualified = self.qualify_name(&target);
            self.lowered_fn_symbol_name(&target, &qualified, range)
        } else {
            self.lowered_fn_symbol_name(&target, &target, range)
        };
        MirExpr::Var(symbol, ty)
    }

    /// A top-level fn's name: `name__N` when the module defines the name at
    /// more than one arity, each arity being its own function.
    fn fn_def_name(&self, fn_def: &FnDef) -> String {
        let name = fn_def
            .name()
            .and_then(|name| name.text())
            .expect("the parser names every fn");
        let top_level = fn_def
            .syntax()
            .parent()
            .is_some_and(|parent| parent.kind() == SyntaxKind::SOURCE_FILE);
        if top_level && self.overloaded_fn_names.contains(&name) {
            let arity = fn_def.param_list().map_or(0, |pl| pl.params().count());
            format!("{name}__{arity}")
        } else {
            name
        }
    }

    /// The fn `name_ref` names: its text, or, as the callee of a call to an
    /// overloaded fn, the arity the call runs (`name__N`).
    fn name_ref_fn_name(&self, name_ref: &NameRef) -> String {
        let range = name_ref.syntax().text_range();
        let call_range = name_ref
            .syntax()
            .parent()
            .and_then(CallExpr::cast)
            .filter(|call| {
                call.callee()
                    .is_some_and(|c| c.syntax().text_range() == range)
            })
            .map_or(range, |call| call.syntax().text_range());
        self.overloaded_call_targets
            .get(&call_range)
            .cloned()
            .unwrap_or_else(|| {
                name_ref
                    .text()
                    .expect("the parser makes a name reference of an identifier")
            })
    }

    // ── Message shapes ───────────────────────────────────────────────

    /// The shape of the value the syntax at `range` evaluates to.
    fn msg_shape_at(&self, range: TextRange) -> MsgShape {
        self.get_ty(range)
            .map_or(MsgShape::Shared, |ty| self.msg_shape(ty, &mut Vec::new()))
    }

    /// The variable `name` of type `ty`, which a closure captures, with the
    /// shape of what it holds. The shape goes into the environment's own
    /// shape table, which is what lets a closure be copied to another actor.
    /// The variable's type is read off a typed use of it inside the closure
    /// (a keyword key of the same name, `f(name: 1)`, has none); `Shared`
    /// stands for a type no use gives.
    fn shaped_capture(
        &self,
        closure: &mesh_parser::SyntaxNode,
        name: &str,
        ty: &MirType,
    ) -> MirExpr {
        let shape = closure
            .descendants()
            .filter_map(NameRef::cast)
            .filter(|name_ref| name_ref.text().as_deref() == Some(name))
            .find_map(|name_ref| self.get_ty(name_ref.syntax().text_range()))
            .map_or(MsgShape::Shared, |ty| self.msg_shape(ty, &mut Vec::new()));
        MirExpr::Shaped {
            shape,
            value: Box::new(MirExpr::Var(name.to_string(), ty.clone())),
        }
    }

    /// Mark `value`, the expression at `range`, as about to cross to another
    /// actor. Scalars need nothing and stay as they are.
    fn shaped(&self, value: MirExpr, range: TextRange) -> MirExpr {
        let shape = self.msg_shape_at(range);
        if shape.is_scalar() {
            return value;
        }
        MirExpr::Shaped {
            value: Box::new(value),
            shape,
        }
    }

    /// Where the heap references are in a value of type `ty`.
    ///
    /// `open` holds the named types currently being described, so a recursive
    /// type refers back to itself instead of unfolding forever. Anything not
    /// known to be copyable is `Shared`, which is always safe: the owning heap
    /// keeps it alive for the receiver.
    fn msg_shape(&self, ty: &Ty, open: &mut Vec<String>) -> MsgShape {
        let (name, args): (&str, &[Ty]) = match ty {
            Ty::Never => return MsgShape::Scalar,
            // An unresolved type has an unknown representation.
            Ty::Var(_) => return MsgShape::Shared,
            Ty::Fun(..) => return MsgShape::Closure,
            Ty::Tuple(elems) if elems.is_empty() => return MsgShape::Scalar,
            Ty::Tuple(elems) => {
                return MsgShape::Tuple(elems.iter().map(|e| self.msg_shape(e, open)).collect())
            }
            Ty::Con(_) | Ty::App(..) => ty_head(ty).expect("a named type has a head"),
        };
        let mut arg = |index: usize| {
            Box::new(
                args.get(index)
                    .map_or(MsgShape::Shared, |a| self.msg_shape(a, open)),
            )
        };
        match name {
            "Int" | "Float" | "Bool" | "Unit" | "()" | "DateTime" | "SqliteConn" | "PgConn"
            | "PoolHandle" => return MsgShape::Scalar,
            "Pid" => return MsgShape::Pid,
            "String" | "Atom" => return MsgShape::String,
            "Bytes" | "U64" | "U128" | "I128" | "Range" => return MsgShape::Leaf,
            "List" | "Set" => return MsgShape::List(arg(0)),
            "Map" => return MsgShape::Map(arg(0), arg(1)),
            "Queue" => return MsgShape::Queue(arg(0)),
            "Json" => return MsgShape::Json,
            _ => {}
        }

        // User-defined and builtin generic types. Resources are owner-bound
        // handles, never duplicated.
        if self.registry.is_resource_name(name) {
            return MsgShape::Shared;
        }
        let (MirType::Struct(mir_name) | MirType::SumType(mir_name)) =
            resolve_type(ty, self.registry)
        else {
            return MsgShape::Shared;
        };
        if open.contains(&mir_name) {
            return MsgShape::Recur(mir_name);
        }
        let (params, variants): (&[String], Vec<(Option<&str>, Vec<&Ty>)>) =
            if let Some(info) = self.registry.struct_defs.get(name) {
                let fields = info.fields.iter().map(|(_, field)| field).collect();
                (&info.generic_params, vec![(None, fields)])
            } else if let Some(info) = self.registry.sum_type_defs.get(name) {
                let variants = info
                    .variants
                    .iter()
                    .map(|variant| {
                        let fields = variant
                            .fields
                            .iter()
                            .map(|field| match field {
                                mesh_typeck::VariantFieldInfo::Positional(ty)
                                | mesh_typeck::VariantFieldInfo::Named(_, ty) => ty,
                            })
                            .collect();
                        (Some(variant.name.as_str()), fields)
                    })
                    .collect();
                (&info.generic_params, variants)
            } else {
                return MsgShape::Shared;
            };

        let subst: HashMap<String, &Ty> = params.iter().cloned().zip(args).collect();
        open.push(mir_name.clone());
        let mut described: Vec<(Option<&str>, Vec<MsgShape>)> = variants
            .into_iter()
            .map(|(variant, fields)| {
                let shapes = fields
                    .into_iter()
                    .map(|field| self.msg_shape(&substitute_type_params(field, &subst), open))
                    .collect();
                (variant, shapes)
            })
            .collect();
        open.pop();

        match described.first() {
            Some((None, _)) => MsgShape::Struct(mir_name, described.remove(0).1),
            _ => MsgShape::Sum(
                mir_name,
                described
                    .into_iter()
                    .map(|(variant, shapes)| (variant.unwrap_or_default().to_string(), shapes))
                    .collect(),
            ),
        }
    }

    /// The runtime key type tag of the map literal at `range`: 1 for String
    /// keys, 0 (compared as words) for any other.
    fn infer_map_key_type(&self, range: TextRange) -> i64 {
        match self.types.get(&range).and_then(ty_head) {
            Some(("Map", [key, ..])) if *key == Ty::string() => 1,
            _ => 0,
        }
    }

    // ── Function value usage type recovery ───────────────────────────

    /// Scan every NAME_REF node in the source AST and, for each node that refers
    /// to a user-defined function and has a concrete (non-Var) function type in the
    /// typeck map, record all observed types for that function name.
    ///
    /// At call sites like `HTTP.use(r, pass)` the typeck resolves the `pass`
    /// identifier to the instantiated concrete type (e.g. `Fn(Request, …)->Response`)
    /// even when the function definition's own parameter types were generalized away
    /// as Ty::Var before the call site was processed. A generic function is
    /// specialized at each of these types.
    fn build_fn_value_usage_types(
        &self,
        root: &mesh_parser::SyntaxNode,
    ) -> HashMap<String, Vec<Ty>> {
        let mut map: HashMap<String, Vec<Ty>> = HashMap::new();
        for name_ref in root.descendants().filter_map(NameRef::cast) {
            let name = self.name_ref_fn_name(&name_ref);
            if !self.user_fn_defs.contains(&name) {
                continue;
            }
            // Only record concrete function types — skip Ty::Var results.
            if let Some(ty @ Ty::Fun(..)) = self.types.get(&name_ref.syntax().text_range()) {
                map.entry(name.clone()).or_default().push(ty.clone());
            }
            // `let g = f`: `g`'s uses are uses of `f`.
            let uses = self.alias_use_types(&name_ref);
            map.entry(name).or_default().extend(uses);
        }
        map
    }

    /// When `name_ref` is the whole initializer of `let g = name_ref`, the
    /// types `g` is used at after it (through further aliases too). A name
    /// reference right under a `let` is its initializer; a `let` that takes
    /// it apart with a pattern (a local tuple named like a function) aliases
    /// nothing.
    fn alias_use_types(&self, name_ref: &NameRef) -> Vec<Ty> {
        let Some(alias) = name_ref.syntax().parent().and_then(LetBinding::cast) else {
            return Vec::new();
        };
        let Some(alias_name) = alias.name().and_then(|name| name.text()) else {
            return Vec::new();
        };
        let scope = alias.syntax().parent().expect("a `let` is inside a block");
        let after = alias.syntax().text_range().end();
        let mut types = Vec::new();
        for use_ref in scope.descendants().filter_map(NameRef::cast) {
            if use_ref.text().as_deref() != Some(alias_name.as_str())
                || use_ref.syntax().text_range().start() < after
            {
                continue;
            }
            if let Some(ty @ Ty::Fun(..)) = self.types.get(&use_ref.syntax().text_range()) {
                types.push(ty.clone());
            }
            types.extend(self.alias_use_types(&use_ref));
        }
        types
    }

    /// Add the specializations generic functions need because other code
    /// calls them: a call inside generic `g` has, in each of `g`'s
    /// specializations, the concrete type `g`'s bindings give it, and a call
    /// whose type is still open (`size([])`) is taken with Unit for what
    /// nothing fixed. Without this, `fn wrap(a) = ident(a)` called with a
    /// String ran `ident`'s Int version, and `size([])` beside two other
    /// uses called a function that was never emitted. A generic function
    /// of another module called here at an open type is recorded in
    /// `imported_specializations` at the types that call takes.
    fn close_specializations(&mut self, sf: &SourceFile) {
        // Each top-level function: its checked type and the calls it makes
        // to generic functions (every clause of a group).
        let mut fns: Vec<(String, Ty, Vec<(String, TextRange)>)> = Vec::new();
        let mut bodies: Vec<(usize, mesh_parser::SyntaxNode)> = Vec::new();
        for item in sf.items() {
            let Item::FnDef(fn_def) = item else { continue };
            let name = self.fn_def_name(&fn_def);
            let index = match fns.iter().position(|(fn_name, _, _)| *fn_name == name) {
                Some(index) => index,
                None => {
                    let ty = self
                        .get_ty(fn_def.syntax().text_range())
                        .cloned()
                        .expect("the type checker types every function");
                    fns.push((name, ty, Vec::new()));
                    fns.len() - 1
                }
            };
            bodies.push((index, fn_def.syntax().clone()));
        }
        for (index, body) in bodies {
            let calls: Vec<(String, TextRange)> = body
                .descendants()
                .filter_map(|node| self.generic_callee(&node, &fns))
                .collect();
            fns[index].2.extend(calls);
        }

        let mut worklist: Vec<(usize, Option<Ty>)> = Vec::new();
        for (index, (name, ty, _)) in fns.iter().enumerate() {
            if Self::ty_contains_var(ty) {
                for spec in self
                    .inferred_fn_specializations
                    .get(name)
                    .cloned()
                    .unwrap_or_default()
                {
                    worklist.push((index, Some(spec)));
                }
            } else {
                worklist.push((index, None));
            }
        }
        while let Some((index, spec)) = worklist.pop() {
            let (_, generic_ty, calls) = &fns[index];
            let mut bindings = Vec::new();
            if let Some(spec) = &spec {
                bind_type_vars(generic_ty, spec, &mut bindings);
            }
            let mut found = Vec::new();
            for (callee, range) in calls {
                // A name that is no call of the function: a keyword key
                // (`f(name: 1)`, untyped) or a local of the same name.
                let Some(call_ty) = self
                    .types
                    .get(range)
                    .filter(|ty| fn_type_parts(ty).is_some())
                else {
                    continue;
                };
                let concrete = apply_default_unit(&apply_type_vars(call_ty, &bindings));
                let known = self
                    .inferred_fn_specializations
                    .get(callee)
                    .is_some_and(|specs| specs.contains(&concrete));
                if !known {
                    found.push((callee.clone(), concrete));
                }
            }
            for (callee, concrete) in found {
                Self::push_usage_type(&mut self.inferred_fn_specializations, &callee, &concrete);
                match fns.iter().position(|(name, _, _)| *name == callee) {
                    Some(callee_index) => worklist.push((callee_index, Some(concrete))),
                    None => Self::push_usage_type(
                        &mut self.imported_specializations,
                        &callee,
                        &concrete,
                    ),
                }
            }
        }
    }

    /// The generic function `node` names, with the range its type is at:
    /// one of this module's `fns`, or another module's reached through an
    /// import (`ident(x)`) or its module (`Utils.ident(x)`) at an open type.
    fn generic_callee(
        &self,
        node: &mesh_parser::SyntaxNode,
        fns: &[(String, Ty, Vec<(String, TextRange)>)],
    ) -> Option<(String, TextRange)> {
        let open_at = |range: TextRange| self.types.get(&range).is_some_and(Self::ty_contains_var);
        if let Some(name_ref) = NameRef::cast(node.clone()) {
            let callee = self.name_ref_fn_name(&name_ref);
            let range = name_ref.syntax().text_range();
            let generic = match fns.iter().find(|(name, _, _)| *name == callee) {
                Some((_, ty, _)) => Self::ty_contains_var(ty),
                None => {
                    name_ref
                        .text()
                        .is_some_and(|name| self.imported_functions.contains(&name))
                        && open_at(range)
                }
            };
            return generic.then_some((callee, range));
        }
        let field_access = FieldAccess::cast(node.clone())?;
        let Some(Expr::NameRef(base)) = field_access.base() else {
            return None;
        };
        let exports = self.user_modules.get(&base.text()?)?;
        let range = field_access.syntax().text_range();
        let field = field_access.field()?.text().to_string();
        if !exports.contains(&field) || !open_at(range) {
            return None;
        }
        // A call to an overloaded function names its arity (`name__N`).
        let callee = field_access
            .syntax()
            .parent()
            .and_then(CallExpr::cast)
            .and_then(|call| {
                self.overloaded_call_targets
                    .get(&call.syntax().text_range())
            })
            .cloned()
            .unwrap_or(field);
        Some((callee, range))
    }

    /// The specializations generic functions need, before any is lowered:
    /// the types functions are used at as values, the concrete types
    /// generic functions are called at, and what those calls need in turn.
    fn prepare_specializations(&mut self, sf: &SourceFile) {
        // `build_fn_value_usage_types` counts only calls of functions defined
        // here (lowering registers them again as it declares them).
        for item in sf.items() {
            if let Item::FnDef(fn_def) = item {
                self.user_fn_defs.insert(self.fn_def_name(&fn_def));
            }
        }

        // Function value usage types, so that lower_fn_def can recover
        // concrete parameter types for functions whose params were
        // generalized away (Ty::Var) before call sites like
        // `HTTP.use(r, pass)` constrained them.
        let syntax = self.parse.syntax();
        let usage_types = self.build_fn_value_usage_types(&syntax);
        self.merge_usage_types(usage_types);

        // Identify locally-defined inferred functions whose definition type still
        // contains TyVars. These need concrete call-site evidence to repair their
        // ABI, and multi-signature cases need per-signature MIR clones.
        for item in sf.items() {
            if let Item::FnDef(fn_def) = item {
                let name = self.fn_def_name(&fn_def);
                let range = fn_def.syntax().text_range();
                if self.get_ty(range).is_some_and(Self::ty_contains_var) {
                    if let Some(usage_tys) = self.fn_value_usage_types.get(&name).cloned() {
                        for usage_ty in usage_tys {
                            Self::push_usage_type(
                                &mut self.inferred_fn_specializations,
                                &name,
                                &usage_ty,
                            );
                        }
                    }
                }
            }
        }

        self.close_specializations(sf);
    }

    fn ty_contains_var(ty: &Ty) -> bool {
        match ty {
            Ty::Var(_) => true,
            Ty::Con(_) | Ty::Never => false,
            Ty::Fun(params, ret) => {
                params.iter().any(Self::ty_contains_var) || Self::ty_contains_var(ret)
            }
            Ty::App(con, args) => {
                Self::ty_contains_var(con) || args.iter().any(Self::ty_contains_var)
            }
            Ty::Tuple(elems) => elems.iter().any(Self::ty_contains_var),
        }
    }

    fn is_concrete_fun_ty(ty: &Ty) -> bool {
        matches!(ty, Ty::Fun(..)) && !Self::ty_contains_var(ty)
    }

    fn push_usage_type(map: &mut HashMap<String, Vec<Ty>>, name: &str, ty: &Ty) {
        if !Self::is_concrete_fun_ty(ty) {
            return;
        }
        let entry = map.entry(name.to_string()).or_default();
        if !entry.contains(ty) {
            entry.push(ty.clone());
        }
    }

    fn merge_usage_types(&mut self, usage_types: HashMap<String, Vec<Ty>>) {
        for (name, tys) in usage_types {
            for ty in tys {
                Self::push_usage_type(&mut self.fn_value_usage_types, &name, &ty);
            }
        }
    }

    /// A name component for a concrete source type. Distinct source types must
    /// give distinct components: `List<Int>` and `List<String>` share the MIR
    /// type `Ptr`, yet each specialization dispatches its own methods.
    fn ty_specialization_component(ty: &Ty) -> String {
        let sanitize = |name: &str| {
            name.chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                .collect::<String>()
        };
        match ty {
            Ty::Con(tc) => sanitize(&tc.name),
            Ty::App(con, args) => format!(
                "{}_of_{}_end",
                Self::ty_specialization_component(con),
                args.iter()
                    .map(Self::ty_specialization_component)
                    .collect::<Vec<_>>()
                    .join("_")
            ),
            Ty::Tuple(elems) => format!(
                "Tuple{}_{}_end",
                elems.len(),
                elems
                    .iter()
                    .map(Self::ty_specialization_component)
                    .collect::<Vec<_>>()
                    .join("_")
            ),
            Ty::Fun(params, ret) => format!(
                "Fun_{}_to_{}_end",
                params
                    .iter()
                    .map(Self::ty_specialization_component)
                    .collect::<Vec<_>>()
                    .join("_"),
                Self::ty_specialization_component(ret)
            ),
            Ty::Var(v) => format!("Var{}", v.0),
            Ty::Never => "Never".to_string(),
        }
    }

    /// The name of `base_name`'s specialization at `fun_ty`, a concrete
    /// function type (`push_usage_type` keeps no other).
    fn mangle_inferred_fn_name(&self, base_name: &str, fun_ty: &Ty) -> String {
        let (params, ret) = fun_parts(fun_ty);
        let mut parts: Vec<String> = params
            .iter()
            .map(Self::ty_specialization_component)
            .collect();
        parts.push("ret".to_string());
        parts.push(Self::ty_specialization_component(ret));
        format!("{}__spec__{}", base_name, parts.join("__"))
    }

    /// The function a use of `original_name` at `range` runs: a function
    /// specialized at several types, the specialization at the use's type
    /// (what nothing fixed is Unit, as `close_specializations` took it);
    /// any other, `base_name`.
    fn lowered_fn_symbol_name(
        &self,
        original_name: &str,
        base_name: &str,
        range: TextRange,
    ) -> String {
        let specialization = self
            .inferred_fn_specializations
            .get(original_name)
            .filter(|variants| variants.len() > 1)
            .and_then(|variants| {
                let ty = apply_default_unit(self.get_ty(range)?);
                variants.contains(&ty).then_some(ty)
            });
        match specialization {
            Some(fun_ty) => self.mangle_inferred_fn_name(base_name, &fun_ty),
            None => base_name.to_string(),
        }
    }

    /// `HTTP.clustered(handler)` or `HTTP.clustered(<count>, handler)`: a
    /// shim calling the handler, which the type checker requires to be a
    /// `Request -> Response` function named at the top level.
    fn lower_clustered_route_wrapper(
        &mut self,
        call: &CallExpr,
        metadata: &ClusteredRouteWrapperMetadata,
    ) -> MirExpr {
        let handler_expr = call
            .arg_list()
            .and_then(|list| list.args().last())
            .expect("the type checker gives a clustered route its handler");
        let lowered_handler = self.lower_callee(&handler_expr);
        let shim_name = declared_route_wrapper_name(&metadata.runtime_name);
        let shim_ty = MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr));
        if !self.known_functions.contains_key(&shim_name) {
            let request = "__request".to_string();
            let body = MirExpr::Call {
                func: Box::new(lowered_handler),
                args: vec![MirExpr::Var(request.clone(), MirType::Ptr)],
                ty: MirType::Ptr,
            };
            self.push_helper_fn(
                &shim_name,
                vec![(request, MirType::Ptr)],
                MirType::Ptr,
                body,
            );
            self.known_functions
                .insert(shim_name.clone(), shim_ty.clone());
        }

        self.consumed_clustered_route_wrappers
            .insert(call.syntax().text_range());
        MirExpr::Var(shim_name, shim_ty)
    }

    fn is_inferred_specialization_name(&self, name: &str) -> bool {
        self.inferred_fn_specializations.keys().any(|base| {
            name.starts_with(&format!("{}__spec__", base))
                || name.starts_with(&format!("{}__spec__", self.qualify_name(base)))
        })
    }

    // ── Top-level lowering ───────────────────────────────────────────

    fn lower_source_file(&mut self, sf: SourceFile) {
        for node in sf.syntax().descendants() {
            let Some(function) = FnDef::cast(node) else {
                continue;
            };
            let name = self.fn_def_name(&function);
            let modes = function
                .param_list()
                .map(|parameters| {
                    parameters
                        .params()
                        .map(|parameter| parameter.ownership())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let qualified_name = self.qualify_name(&name);
            self.ownership_signatures
                .entry(name.clone())
                .or_insert_with(|| modes.clone());
            self.ownership_signatures
                .entry(qualified_name)
                .or_insert(modes);
        }

        // First pass: register all function names so we know which are direct calls.
        // For multi-clause functions, only register the FIRST clause (which has the type).
        for item in sf.items() {
            match &item {
                Item::FnDef(fn_def) => {
                    let name = self.fn_def_name(fn_def);
                    // Skip if already registered (subsequent clause of a multi-clause fn).
                    if !self.known_functions.contains_key(&name) {
                        let fn_ty = self.resolve_range(fn_def.syntax().text_range());
                        self.known_functions.insert(name.clone(), fn_ty.clone());
                        self.user_fn_defs.insert(name.clone());
                        self.insert_var(name, fn_ty);
                    }
                }
                Item::ActorDef(actor_def) => {
                    if let Some(name) = actor_def.name().and_then(|n| n.text()) {
                        // Actor definitions produce a function with the actor name
                        let fn_ty = self.resolve_range(actor_def.syntax().text_range());
                        self.known_functions.insert(name.clone(), fn_ty.clone());
                        self.insert_var(name, fn_ty);
                    }
                }
                Item::SupervisorDef(sup_def) => {
                    if let Some(name) = sup_def.name().and_then(|n| n.text()) {
                        // Supervisor definitions produce a function that returns Pid
                        let fn_ty = self.resolve_range(sup_def.syntax().text_range());
                        self.known_functions.insert(name.clone(), fn_ty.clone());
                        self.insert_var(name, fn_ty);
                    }
                }
                Item::ServiceDef(service_def) => {
                    if let Some(name) = service_def.name().and_then(|n| n.text()) {
                        // Pre-register the service start function.
                        let start_fn_name = format!("__service_{}_start", name.to_lowercase());
                        self.known_functions.insert(
                            start_fn_name.clone(),
                            MirType::FnPtr(vec![], Box::new(MirType::Pid(None))),
                        );
                    }
                }
                Item::ImplDef(impl_def) => {
                    let (trait_name, trait_type_args, type_name) = self.impl_names(impl_def);
                    let mut provided_methods = std::collections::HashSet::new();
                    for method in impl_def.methods() {
                        if let Some(method_name) = method.name().and_then(|n| n.text()) {
                            provided_methods.insert(method_name.clone());
                            let mangled = mangle_trait_method(
                                &trait_name,
                                &trait_type_args,
                                &method_name,
                                &type_name,
                            );
                            let fn_ty = self.resolve_range(method.syntax().text_range());
                            self.known_functions.insert(mangled.clone(), fn_ty);
                        }
                    }
                    // Pre-register default method bodies for missing methods.
                    let trait_def = self
                        .trait_registry
                        .get_trait(&trait_name)
                        .expect("the type checker knows every implemented interface");
                    for trait_method in &trait_def.methods {
                        if trait_method.has_default_body
                            && !provided_methods.contains(&trait_method.name)
                        {
                            let mangled = mangle_trait_method(
                                &trait_name,
                                &trait_type_args,
                                &trait_method.name,
                                &type_name,
                            );
                            // A method written without a result type returns Unit.
                            let fn_ty = match &trait_method.return_type {
                                Some(ret_ty) => resolve_type(ret_ty, self.registry),
                                None => MirType::Unit,
                            };
                            self.known_functions.insert(mangled, fn_ty);
                        }
                    }
                }
                _ => {}
            }
        }

        // Detect test mode: scan for the `fn __test_body_*` and
        // `fn __test_describe_*` functions injected by the test preprocessor.
        // When found, enable special DSL lowering for assert/assert_raises.
        self.is_test_mode = sf.items().any(|item| match item {
            Item::FnDef(fn_def) => {
                let name = self.fn_def_name(&fn_def);
                name.starts_with("__test_body_") || name.starts_with("__test_describe_")
            }
            _ => false,
        });

        // Register builtin I/O functions as known functions.
        self.known_functions.insert(
            "println".to_string(),
            MirType::FnPtr(vec![MirType::String], Box::new(MirType::Unit)),
        );
        self.known_functions.insert(
            "print".to_string(),
            MirType::FnPtr(vec![MirType::String], Box::new(MirType::Unit)),
        );

        // Register stdlib functions as known functions (Phase 8).
        // String operations
        self.known_functions.insert(
            "mesh_string_length".to_string(),
            MirType::FnPtr(vec![MirType::String], Box::new(MirType::Int)),
        );
        self.known_functions.insert(
            "mesh_string_slice".to_string(),
            MirType::FnPtr(
                vec![MirType::String, MirType::Int, MirType::Int],
                Box::new(MirType::String),
            ),
        );
        self.known_functions.insert(
            "mesh_string_contains".to_string(),
            MirType::FnPtr(
                vec![MirType::String, MirType::String],
                Box::new(MirType::Bool),
            ),
        );
        self.known_functions.insert(
            "mesh_string_starts_with".to_string(),
            MirType::FnPtr(
                vec![MirType::String, MirType::String],
                Box::new(MirType::Bool),
            ),
        );
        self.known_functions.insert(
            "mesh_string_ends_with".to_string(),
            MirType::FnPtr(
                vec![MirType::String, MirType::String],
                Box::new(MirType::Bool),
            ),
        );
        self.known_functions.insert(
            "mesh_string_trim".to_string(),
            MirType::FnPtr(vec![MirType::String], Box::new(MirType::String)),
        );
        for trim in ["mesh_string_trim_start", "mesh_string_trim_end"] {
            self.known_functions.insert(
                trim.to_string(),
                MirType::FnPtr(vec![MirType::String], Box::new(MirType::String)),
            );
        }
        self.known_functions.insert(
            "mesh_string_repeat".to_string(),
            MirType::FnPtr(
                vec![MirType::String, MirType::Int],
                Box::new(MirType::String),
            ),
        );
        self.known_functions.insert(
            "mesh_string_to_upper".to_string(),
            MirType::FnPtr(vec![MirType::String], Box::new(MirType::String)),
        );
        self.known_functions.insert(
            "mesh_string_to_lower".to_string(),
            MirType::FnPtr(vec![MirType::String], Box::new(MirType::String)),
        );
        self.known_functions.insert(
            "mesh_string_replace".to_string(),
            MirType::FnPtr(
                vec![MirType::String, MirType::String, MirType::String],
                Box::new(MirType::String),
            ),
        );
        // Phase 46: String split/join/to_int/to_float
        self.known_functions.insert(
            "mesh_string_split".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_string_join".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_string_to_int".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_string_to_float".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        // File I/O functions
        self.known_functions.insert(
            "mesh_file_read".to_string(),
            MirType::FnPtr(vec![MirType::String], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_file_read_bytes".to_string(),
            MirType::FnPtr(
                vec![MirType::String, MirType::Int, MirType::Int],
                Box::new(MirType::Ptr),
            ),
        );
        self.known_functions.insert(
            "mesh_file_write_bytes".to_string(),
            MirType::FnPtr(
                vec![MirType::String, MirType::Int, MirType::Ptr, MirType::Bool],
                Box::new(MirType::Ptr),
            ),
        );
        self.known_functions.insert(
            "mesh_file_size".to_string(),
            MirType::FnPtr(vec![MirType::String], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_file_write".to_string(),
            MirType::FnPtr(
                vec![MirType::String, MirType::String],
                Box::new(MirType::Ptr),
            ),
        );
        self.known_functions.insert(
            "mesh_file_append".to_string(),
            MirType::FnPtr(
                vec![MirType::String, MirType::String],
                Box::new(MirType::Ptr),
            ),
        );
        self.known_functions.insert(
            "mesh_file_exists".to_string(),
            MirType::FnPtr(vec![MirType::String], Box::new(MirType::Bool)),
        );
        self.known_functions.insert(
            "mesh_file_delete".to_string(),
            MirType::FnPtr(vec![MirType::String], Box::new(MirType::Ptr)),
        );
        // IO functions
        self.known_functions.insert(
            "mesh_io_read_line".to_string(),
            MirType::FnPtr(vec![], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_io_eprintln".to_string(),
            MirType::FnPtr(vec![MirType::String], Box::new(MirType::Unit)),
        );
        // Env functions
        self.known_functions.insert(
            "mesh_env_get".to_string(),
            MirType::FnPtr(vec![MirType::String], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_env_args".to_string(),
            MirType::FnPtr(vec![], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_env_get_with_default".to_string(),
            MirType::FnPtr(
                vec![MirType::String, MirType::String],
                Box::new(MirType::String),
            ),
        );
        self.known_functions.insert(
            "mesh_env_get_int".to_string(),
            MirType::FnPtr(vec![MirType::String, MirType::Int], Box::new(MirType::Int)),
        );
        self.known_functions.insert(
            "mesh_env_get_secret_hex".to_string(),
            MirType::FnPtr(vec![MirType::String], Box::new(MirType::Ptr)),
        );
        // Regex runtime functions (Phase 119)
        self.known_functions.insert(
            "mesh_regex_from_literal".to_string(),
            MirType::FnPtr(vec![MirType::String, MirType::Int], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_regex_compile".to_string(),
            MirType::FnPtr(vec![MirType::String], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_regex_match".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::String], Box::new(MirType::Bool)),
        );
        self.known_functions.insert(
            "mesh_regex_captures".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::String], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_regex_replace".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::String, MirType::String],
                Box::new(MirType::String),
            ),
        );
        self.known_functions.insert(
            "mesh_regex_split".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::String], Box::new(MirType::Ptr)),
        );
        // Binary-first Crypto V2 runtime functions. Fallible calls return an
        // ABI pointer to MeshResult; the call-site's typeck MIR type retains the
        // concrete nominal `Result<T, CryptoError>` identity.
        self.known_functions.insert(
            "mesh_crypto_sha256".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_crypto_sha512".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        for name in ["mesh_crypto_sha256_hex", "mesh_crypto_sha512_hex"] {
            self.known_functions.insert(
                name.to_string(),
                MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::String)),
            );
        }
        self.known_functions.insert(
            "mesh_crypto_random_bytes".to_string(),
            MirType::FnPtr(vec![MirType::Int], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_crypto_hmac_sha256".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_crypto_hkdf_sha256".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::Ptr, MirType::Ptr, MirType::Int],
                Box::new(MirType::Ptr),
            ),
        );
        self.known_functions.insert(
            "mesh_crypto_argon2id".to_string(),
            MirType::FnPtr(
                vec![
                    MirType::Ptr,
                    MirType::Ptr,
                    MirType::Int,
                    MirType::Int,
                    MirType::Int,
                    MirType::Int,
                ],
                Box::new(MirType::Ptr),
            ),
        );
        for name in [
            "mesh_crypto_x25519_generate",
            "mesh_crypto_mlkem_generate",
            "mesh_crypto_signing_generate",
        ] {
            self.known_functions.insert(
                name.to_string(),
                MirType::FnPtr(vec![], Box::new(MirType::Ptr)),
            );
        }
        self.known_functions.insert(
            "mesh_crypto_signing_from_seed".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_crypto_signing_from_secret".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_crypto_x25519_from_seed".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_crypto_x25519_from_secret".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_crypto_mlkem_from_seed".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_crypto_mlkem_from_secret".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_crypto_x25519_public".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_crypto_x25519_shared".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::Struct("X25519PublicKey".to_string())],
                Box::new(MirType::Ptr),
            ),
        );
        self.known_functions.insert(
            "mesh_crypto_hpke_seal".to_string(),
            MirType::FnPtr(
                vec![
                    MirType::Struct("X25519PublicKey".to_string()),
                    MirType::Ptr,
                    MirType::Ptr,
                    MirType::Ptr,
                ],
                Box::new(MirType::Ptr),
            ),
        );
        self.known_functions.insert(
            "mesh_crypto_hpke_open".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::Ptr, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        self.known_functions.insert(
            "mesh_crypto_hpke_seal_secret".to_string(),
            MirType::FnPtr(
                vec![
                    MirType::Struct("X25519PublicKey".to_string()),
                    MirType::Ptr,
                    MirType::Ptr,
                    MirType::Ptr,
                ],
                Box::new(MirType::Ptr),
            ),
        );
        self.known_functions.insert(
            "mesh_crypto_hpke_open_secret".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::Ptr, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        self.known_functions.insert(
            "mesh_crypto_mlkem_encapsulate".to_string(),
            MirType::FnPtr(
                vec![MirType::Struct("MlKemPublicKey".to_string())],
                Box::new(MirType::Ptr),
            ),
        );
        self.known_functions.insert(
            "mesh_crypto_mlkem_decapsulate".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::Struct("MlKemCiphertext".to_string())],
                Box::new(MirType::Ptr),
            ),
        );
        self.known_functions.insert(
            "mesh_crypto_sign".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_crypto_verify".to_string(),
            MirType::FnPtr(
                vec![
                    MirType::Struct("SigningPublicKey".to_string()),
                    MirType::Ptr,
                    MirType::Struct("Signature".to_string()),
                ],
                Box::new(MirType::Ptr),
            ),
        );
        self.known_functions.insert(
            "mesh_crypto_aead_key".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        for name in ["mesh_crypto_aead_seal", "mesh_crypto_aead_open"] {
            self.known_functions.insert(
                name.to_string(),
                MirType::FnPtr(
                    vec![MirType::Ptr, MirType::Ptr, MirType::Ptr, MirType::Ptr],
                    Box::new(MirType::Ptr),
                ),
            );
        }

        // Legacy non-colliding Phase 135 functions.
        self.known_functions.insert(
            "mesh_crypto_hmac_sha512".to_string(),
            MirType::FnPtr(
                vec![MirType::String, MirType::String],
                Box::new(MirType::String),
            ),
        );
        self.known_functions.insert(
            "mesh_crypto_uuid4".to_string(),
            MirType::FnPtr(vec![], Box::new(MirType::String)),
        );
        // Base64: String -> String (encode functions)
        self.known_functions.insert(
            "mesh_base64_encode".to_string(),
            MirType::FnPtr(vec![MirType::String], Box::new(MirType::String)),
        );
        self.known_functions.insert(
            "mesh_base64_encode_url".to_string(),
            MirType::FnPtr(vec![MirType::String], Box::new(MirType::String)),
        );
        // Base64: String -> Ptr/Result (decode functions)
        self.known_functions.insert(
            "mesh_base64_decode".to_string(),
            MirType::FnPtr(vec![MirType::String], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_base64_decode_url".to_string(),
            MirType::FnPtr(vec![MirType::String], Box::new(MirType::Ptr)),
        );
        // Hex: String -> String (encode)
        self.known_functions.insert(
            "mesh_hex_encode".to_string(),
            MirType::FnPtr(vec![MirType::String], Box::new(MirType::String)),
        );
        // Hex: String -> Ptr/Result (decode)
        self.known_functions.insert(
            "mesh_hex_decode".to_string(),
            MirType::FnPtr(vec![MirType::String], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_bytes_empty".to_string(),
            MirType::FnPtr(vec![], Box::new(MirType::Ptr)),
        );
        for name in ["mesh_bytes_from_list", "mesh_bytes_to_list"] {
            self.known_functions.insert(
                name.to_string(),
                MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
            );
        }
        self.known_functions.insert(
            "mesh_bytes_repeat".to_string(),
            MirType::FnPtr(vec![MirType::Int, MirType::Int], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_bytes_length".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Int)),
        );
        self.known_functions.insert(
            "mesh_bytes_get".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Int], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_bytes_slice".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::Int, MirType::Int],
                Box::new(MirType::Ptr),
            ),
        );
        self.known_functions.insert(
            "mesh_bytes_concat".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_bytes_secure_equals".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Bool)),
        );
        self.known_functions.insert(
            "mesh_bytes_from_utf8".to_string(),
            MirType::FnPtr(vec![MirType::String], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_bytes_to_utf8".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        for name in [
            "mesh_bytes_to_base64",
            "mesh_bytes_to_base58",
            "mesh_bytes_to_hex",
        ] {
            self.known_functions.insert(
                name.to_string(),
                MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::String)),
            );
        }
        self.known_functions.insert(
            "mesh_json_array_length".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_json_is_null".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Bool)),
        );
        for name in [
            "mesh_bytes_from_base64",
            "mesh_bytes_from_base58",
            "mesh_bytes_from_hex",
        ] {
            self.known_functions.insert(
                name.to_string(),
                MirType::FnPtr(vec![MirType::String], Box::new(MirType::Ptr)),
            );
        }
        self.known_functions.insert(
            "mesh_bytes_read_uint_le".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::Int, MirType::Int],
                Box::new(MirType::Ptr),
            ),
        );
        self.known_functions.insert(
            "mesh_bytes_write_uint_le".to_string(),
            MirType::FnPtr(vec![MirType::String, MirType::Int], Box::new(MirType::Ptr)),
        );
        for name in [
            "mesh_bytes_read_u16_be",
            "mesh_bytes_read_u16_le",
            "mesh_bytes_read_u32_be",
            "mesh_bytes_read_u32_le",
            "mesh_bytes_read_u64_be",
            "mesh_bytes_read_u64_le",
        ] {
            self.known_functions.insert(
                name.to_string(),
                MirType::FnPtr(vec![MirType::Ptr, MirType::Int], Box::new(MirType::Ptr)),
            );
        }
        self.known_functions.insert(
            "mesh_bytes_write_u16_be".to_string(),
            MirType::FnPtr(vec![MirType::Int], Box::new(MirType::Ptr)),
        );
        for name in ["mesh_bytes_write_u32_be", "mesh_bytes_write_u64_be"] {
            self.known_functions.insert(
                name.to_string(),
                MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
            );
        }
        self.known_functions.insert(
            "mesh_bytes_builder_new".to_string(),
            MirType::FnPtr(vec![MirType::Int], Box::new(MirType::Ptr)),
        );
        for name in [
            "mesh_bytes_builder_write_u8",
            "mesh_bytes_builder_write_u16_be",
            "mesh_bytes_builder_write_u32_be",
        ] {
            self.known_functions.insert(
                name.to_string(),
                MirType::FnPtr(vec![MirType::Ptr, MirType::Int], Box::new(MirType::Ptr)),
            );
        }
        self.known_functions.insert(
            "mesh_bytes_builder_write_bytes".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_bytes_builder_finish".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        for name in [
            "mesh_host_secure_store_put",
            "mesh_host_secure_store_get",
            "mesh_host_secure_store_delete",
            "mesh_host_push_get_token",
            "mesh_host_background_schedule",
            "mesh_host_network_state",
            "mesh_host_monotonic_clock",
            "mesh_host_wall_clock",
            "mesh_host_log_redacted",
        ] {
            self.known_functions.insert(
                name.to_string(),
                MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
            );
        }
        self.known_functions.insert(
            "mesh_secret_random".to_string(),
            MirType::FnPtr(vec![MirType::Int], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_storage_key_ephemeral".to_string(),
            MirType::FnPtr(vec![], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_storage_key_platform".to_string(),
            MirType::FnPtr(vec![], Box::new(MirType::Ptr)),
        );
        for name in [
            "mesh_storage_key_seal_bytes",
            "mesh_storage_key_unseal_bytes",
        ] {
            self.known_functions.insert(
                name.to_string(),
                MirType::FnPtr(
                    vec![MirType::Ptr, MirType::Ptr, MirType::Ptr],
                    Box::new(MirType::Ptr),
                ),
            );
        }
        self.known_functions.insert(
            "mesh_secret_concat".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_secret_destroy".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Unit)),
        );
        self.known_functions.insert(
            "mesh_secret_map_new".to_string(),
            MirType::FnPtr(vec![MirType::Int], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_secret_map_insert".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        self.known_functions.insert(
            "mesh_secret_map_contains".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Bool)),
        );
        for operation in ["copy", "delete", "merge"] {
            self.known_functions.insert(
                format!("mesh_secret_map_{operation}"),
                MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
            );
        }
        for prefix in [
            "secret",
            "secret_map",
            "x25519_private_key",
            "signing_private_key",
            "mlkem_private_key",
        ] {
            for operation in ["seal_for_storage", "unseal_from_storage"] {
                self.known_functions.insert(
                    format!("mesh_{prefix}_{operation}"),
                    MirType::FnPtr(
                        vec![MirType::Ptr, MirType::Ptr, MirType::Ptr],
                        Box::new(MirType::Ptr),
                    ),
                );
            }
        }
        for prefix in ["u64", "u128", "i128"] {
            self.known_functions.insert(
                format!("mesh_{prefix}_parse"),
                MirType::FnPtr(vec![MirType::String], Box::new(MirType::Ptr)),
            );
            self.known_functions.insert(
                format!("mesh_{prefix}_compare"),
                MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Int)),
            );
            for operation in ["add", "subtract"] {
                self.known_functions.insert(
                    format!("mesh_{prefix}_{operation}"),
                    MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
                );
            }
            self.known_functions.insert(
                format!("mesh_{prefix}_to_int"),
                MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
            );
            self.known_functions.insert(
                format!("mesh_{prefix}_to_string"),
                MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::String)),
            );
        }
        // DateTime functions (Phase 136)
        // utc_now() -> DateTime (i64)
        self.known_functions.insert(
            "mesh_datetime_utc_now".to_string(),
            MirType::FnPtr(vec![], Box::new(MirType::Int)),
        );
        // from_iso8601(s: String) -> Result (Ptr)
        self.known_functions.insert(
            "mesh_datetime_from_iso8601".to_string(),
            MirType::FnPtr(vec![MirType::String], Box::new(MirType::Ptr)),
        );
        // to_iso8601(dt: i64) -> String
        self.known_functions.insert(
            "mesh_datetime_to_iso8601".to_string(),
            MirType::FnPtr(vec![MirType::Int], Box::new(MirType::String)),
        );
        // from_unix_ms(ms: i64) -> Result (Ptr)
        self.known_functions.insert(
            "mesh_datetime_from_unix_ms".to_string(),
            MirType::FnPtr(vec![MirType::Int], Box::new(MirType::Ptr)),
        );
        // to_unix_ms(dt: i64) -> Int
        self.known_functions.insert(
            "mesh_datetime_to_unix_ms".to_string(),
            MirType::FnPtr(vec![MirType::Int], Box::new(MirType::Int)),
        );
        // from_unix_secs(s: i64) -> Result (Ptr)
        self.known_functions.insert(
            "mesh_datetime_from_unix_secs".to_string(),
            MirType::FnPtr(vec![MirType::Int], Box::new(MirType::Ptr)),
        );
        // to_unix_secs(dt: i64) -> Int
        self.known_functions.insert(
            "mesh_datetime_to_unix_secs".to_string(),
            MirType::FnPtr(vec![MirType::Int], Box::new(MirType::Int)),
        );
        // add(dt: i64, n: i64, unit: String) -> DateTime (i64)
        self.known_functions.insert(
            "mesh_datetime_add".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::Int, MirType::String],
                Box::new(MirType::Int),
            ),
        );
        // diff(dt1: i64, dt2: i64, unit: String) -> Float  (CRITICAL: Float, not Int)
        self.known_functions.insert(
            "mesh_datetime_diff".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::Int, MirType::String],
                Box::new(MirType::Float),
            ),
        );
        // before(dt1: i64, dt2: i64) -> Bool (i8)
        self.known_functions.insert(
            "mesh_datetime_before".to_string(),
            MirType::FnPtr(vec![MirType::Int, MirType::Int], Box::new(MirType::Bool)),
        );
        // after(dt1: i64, dt2: i64) -> Bool (i8)
        self.known_functions.insert(
            "mesh_datetime_after".to_string(),
            MirType::FnPtr(vec![MirType::Int, MirType::Int], Box::new(MirType::Bool)),
        );
        for name in [
            "mesh_checked_add",
            "mesh_checked_sub",
            "mesh_checked_mul",
            "mesh_checked_div",
        ] {
            self.known_functions.insert(
                name.to_string(),
                MirType::FnPtr(vec![MirType::Int, MirType::Int], Box::new(MirType::Ptr)),
            );
        }
        self.known_functions.insert(
            "mesh_checked_abs".to_string(),
            MirType::FnPtr(vec![MirType::Int], Box::new(MirType::Ptr)),
        );
        for name in ["mesh_checked_mul_div", "mesh_checked_rescale"] {
            self.known_functions.insert(
                name.to_string(),
                MirType::FnPtr(
                    vec![MirType::Int, MirType::Int, MirType::Int, MirType::String],
                    Box::new(MirType::Ptr),
                ),
            );
        }
        self.known_functions.insert(
            "mesh_monotonic_now_nanos".to_string(),
            MirType::FnPtr(vec![], Box::new(MirType::Int)),
        );
        self.known_functions.insert(
            "mesh_monotonic_elapsed".to_string(),
            MirType::FnPtr(vec![MirType::Int, MirType::Int], Box::new(MirType::Ptr)),
        );
        for name in ["mesh_duration_millis", "mesh_duration_seconds"] {
            self.known_functions.insert(
                name.to_string(),
                MirType::FnPtr(vec![MirType::Int], Box::new(MirType::Ptr)),
            );
        }
        self.known_functions.insert(
            "mesh_channel_bounded".to_string(),
            MirType::FnPtr(vec![MirType::Int, MirType::String], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_channel_bounded_bytes".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::Int, MirType::String],
                Box::new(MirType::Ptr),
            ),
        );
        for name in ["mesh_channel_try_send", "mesh_channel_recv"] {
            self.known_functions.insert(
                name.to_string(),
                MirType::FnPtr(vec![MirType::Int, MirType::Int], Box::new(MirType::Ptr)),
            );
        }
        for name in [
            "mesh_channel_depth",
            "mesh_channel_byte_depth",
            "mesh_channel_dropped",
        ] {
            self.known_functions.insert(
                name.to_string(),
                MirType::FnPtr(vec![MirType::Int], Box::new(MirType::Int)),
            );
        }
        self.known_functions.insert(
            "mesh_random_seed".to_string(),
            MirType::FnPtr(vec![MirType::Int], Box::new(MirType::Int)),
        );
        self.known_functions.insert(
            "mesh_random_next_int".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::Int, MirType::Int],
                Box::new(MirType::Ptr),
            ),
        );
        self.known_functions.insert(
            "mesh_random_next_unit_ppm".to_string(),
            MirType::FnPtr(vec![MirType::Int], Box::new(MirType::Ptr)),
        );
        // Http client functions (Phase 137)
        // MeshRequest handle is u64 -> MirType::Int
        self.known_functions.insert(
            "mesh_http_build".to_string(),
            MirType::FnPtr(
                vec![MirType::String, MirType::String],
                Box::new(MirType::Int),
            ),
        );
        self.known_functions.insert(
            "mesh_http_header".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::String, MirType::String],
                Box::new(MirType::Int),
            ),
        );
        self.known_functions.insert(
            "mesh_http_body".to_string(),
            MirType::FnPtr(vec![MirType::Int, MirType::String], Box::new(MirType::Int)),
        );
        self.known_functions.insert(
            "mesh_http_body_bytes".to_string(),
            MirType::FnPtr(vec![MirType::Int, MirType::Ptr], Box::new(MirType::Int)),
        );
        self.known_functions.insert(
            "mesh_http_timeout".to_string(),
            MirType::FnPtr(vec![MirType::Int, MirType::Int], Box::new(MirType::Int)),
        );
        self.known_functions.insert(
            "mesh_http_stage_timeout".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::String, MirType::Int],
                Box::new(MirType::Int),
            ),
        );
        self.known_functions.insert(
            "mesh_http_max_redirects".to_string(),
            MirType::FnPtr(vec![MirType::Int, MirType::Int], Box::new(MirType::Int)),
        );
        self.known_functions.insert(
            "mesh_http_max_response_bytes".to_string(),
            MirType::FnPtr(vec![MirType::Int, MirType::Int], Box::new(MirType::Int)),
        );
        self.known_functions.insert(
            "mesh_http_query".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::String, MirType::String],
                Box::new(MirType::Int),
            ),
        );
        self.known_functions.insert(
            "mesh_http_json".to_string(),
            MirType::FnPtr(vec![MirType::Int, MirType::String], Box::new(MirType::Int)),
        );
        self.known_functions.insert(
            "mesh_http_send".to_string(),
            MirType::FnPtr(vec![MirType::Int], Box::new(MirType::Ptr)),
        );
        // Http streaming + cancel + keep-alive (Phase 137 Plan 02)
        // mesh_http_stream(req: i64, fn_ptr: ptr, env_ptr: ptr) -> i64 (cancel handle)
        self.known_functions.insert(
            "mesh_http_stream".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Int),
            ),
        );
        self.known_functions.insert(
            "mesh_http_stream_bytes".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Int),
            ),
        );
        // mesh_http_cancel(cancel_handle: i64) -> unit
        self.known_functions.insert(
            "mesh_http_cancel".to_string(),
            MirType::FnPtr(vec![MirType::Int], Box::new(MirType::Unit)),
        );
        // mesh_http_client() -> i64 (Agent handle)
        self.known_functions.insert(
            "mesh_http_client".to_string(),
            MirType::FnPtr(vec![], Box::new(MirType::Int)),
        );
        // mesh_http_send_with(client: i64, req: i64) -> ptr
        self.known_functions.insert(
            "mesh_http_send_with".to_string(),
            MirType::FnPtr(vec![MirType::Int, MirType::Int], Box::new(MirType::Ptr)),
        );
        // mesh_http_client_close(client: i64) -> unit
        self.known_functions.insert(
            "mesh_http_client_close".to_string(),
            MirType::FnPtr(vec![MirType::Int], Box::new(MirType::Unit)),
        );
        self.known_functions.insert(
            "mesh_http_retry_class".to_string(),
            MirType::FnPtr(
                vec![MirType::String, MirType::String],
                Box::new(MirType::String),
            ),
        );
        self.known_functions.insert(
            "mesh_http_metrics".to_string(),
            MirType::FnPtr(vec![], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_ws_client_options".to_string(),
            MirType::FnPtr(vec![], Box::new(MirType::Int)),
        );
        for name in [
            "mesh_ws_client_connect_timeout",
            "mesh_ws_client_heartbeat_timeout",
            "mesh_ws_client_max_message_bytes",
            "mesh_ws_client_queue_capacity",
        ] {
            self.known_functions.insert(
                name.to_string(),
                MirType::FnPtr(vec![MirType::Int, MirType::Int], Box::new(MirType::Int)),
            );
        }
        self.known_functions.insert(
            "mesh_ws_client_connect".to_string(),
            MirType::FnPtr(vec![MirType::String, MirType::Int], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_ws_client_send_text".to_string(),
            MirType::FnPtr(vec![MirType::Int, MirType::String], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_ws_client_send_bytes".to_string(),
            MirType::FnPtr(vec![MirType::Int, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_ws_client_recv".to_string(),
            MirType::FnPtr(vec![MirType::Int, MirType::Int], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_ws_client_close".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::Int, MirType::String],
                Box::new(MirType::Ptr),
            ),
        );
        self.known_functions.insert(
            "mesh_ws_client_reconnect_delay".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::Int, MirType::Int, MirType::Int],
                Box::new(MirType::Ptr),
            ),
        );
        // ── Test runtime functions (Phase 138) ─────────────────────────
        // mesh_test_begin(name: ptr) -> void
        self.known_functions.insert(
            "mesh_test_begin".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Unit)),
        );
        // mesh_test_pass() -> void
        self.known_functions.insert(
            "mesh_test_pass".to_string(),
            MirType::FnPtr(vec![], Box::new(MirType::Unit)),
        );
        // mesh_test_fail_msg(msg: ptr) -> void
        self.known_functions.insert(
            "mesh_test_fail_msg".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Unit)),
        );
        // mesh_test_assert(cond: i8, expr_src: ptr, file: ptr, file_len: i64, line: i64) -> void
        self.known_functions.insert(
            "mesh_test_assert".to_string(),
            MirType::FnPtr(
                vec![
                    MirType::Bool,
                    MirType::Ptr,
                    MirType::Ptr,
                    MirType::Int,
                    MirType::Int,
                ],
                Box::new(MirType::Unit),
            ),
        );
        // mesh_test_assert_eq(lhs: ptr, rhs: ptr, expr_src: ptr, file: ptr, file_len: i64, line: i64) -> void
        self.known_functions.insert(
            "mesh_test_assert_eq".to_string(),
            MirType::FnPtr(
                vec![
                    MirType::Ptr,
                    MirType::Ptr,
                    MirType::Ptr,
                    MirType::Ptr,
                    MirType::Int,
                    MirType::Int,
                ],
                Box::new(MirType::Unit),
            ),
        );
        // mesh_test_assert_ne — same signature as assert_eq
        self.known_functions.insert(
            "mesh_test_assert_ne".to_string(),
            MirType::FnPtr(
                vec![
                    MirType::Ptr,
                    MirType::Ptr,
                    MirType::Ptr,
                    MirType::Ptr,
                    MirType::Int,
                    MirType::Int,
                ],
                Box::new(MirType::Unit),
            ),
        );
        // mesh_test_assert_raises(fn_ptr: ptr, env_ptr: ptr, file: ptr, file_len: i64, line: i64) -> void
        self.known_functions.insert(
            "mesh_test_assert_raises".to_string(),
            MirType::FnPtr(
                vec![
                    MirType::Ptr,
                    MirType::Ptr,
                    MirType::Ptr,
                    MirType::Int,
                    MirType::Int,
                ],
                Box::new(MirType::Unit),
            ),
        );
        // mesh_test_summary(passed: i64, failed: i64, elapsed_ms: i64) -> void
        self.known_functions.insert(
            "mesh_test_summary".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::Int, MirType::Int],
                Box::new(MirType::Unit),
            ),
        );
        // mesh_test_cleanup_actors() -> void
        self.known_functions.insert(
            "mesh_test_cleanup_actors".to_string(),
            MirType::FnPtr(vec![], Box::new(MirType::Unit)),
        );
        // mesh_test_run_body(fn_ptr: ptr, env_ptr: ptr) -> void
        self.known_functions.insert(
            "mesh_test_run_body".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Unit)),
        );
        self.known_functions.insert(
            "mesh_test_end".to_string(),
            MirType::FnPtr(vec![], Box::new(MirType::Unit)),
        );
        // mesh_test_mock_actor(fn_ptr: ptr, env_ptr: ptr) -> i64
        self.known_functions.insert(
            "mesh_test_mock_actor".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Int)),
        );
        self.known_functions.insert(
            "mesh_test_install_in_memory_secure_store".to_string(),
            MirType::FnPtr(vec![], Box::new(MirType::Bool)),
        );
        self.known_functions.insert(
            "mesh_test_set_push_token".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Bool)),
        );
        // mesh_test_pass_count() -> i64
        self.known_functions.insert(
            "mesh_test_pass_count".to_string(),
            MirType::FnPtr(vec![], Box::new(MirType::Int)),
        );
        // mesh_test_fail_count() -> i64
        self.known_functions.insert(
            "mesh_test_fail_count".to_string(),
            MirType::FnPtr(vec![], Box::new(MirType::Int)),
        );
        // ── Collection functions (Phase 8 Plan 02) ─────────────────────
        // List
        self.known_functions.insert(
            "mesh_list_new".to_string(),
            MirType::FnPtr(vec![], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_list_length".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Int)),
        );
        self.known_functions.insert(
            "mesh_list_append".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Int], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_list_head".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_list_tail".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_list_get".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Int], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_list_concat".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_list_reverse".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_list_map".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        self.known_functions.insert(
            "mesh_list_filter".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        self.known_functions.insert(
            "mesh_list_reduce".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::Int, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        self.known_functions.insert(
            "mesh_list_from_array".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Int], Box::new(MirType::Ptr)),
        );
        // Phase 46: sort, find, any, all, contains
        self.known_functions.insert(
            "mesh_list_sort".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        self.known_functions.insert(
            "mesh_list_find".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        // `Iter.next(iter)`: a pointer to an Option (see `box_next_scalar`).
        self.known_functions.insert(
            "mesh_iter_generic_next".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_list_any".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Bool),
            ),
        );
        self.known_functions.insert(
            "mesh_list_all".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Bool),
            ),
        );
        self.known_functions.insert(
            "mesh_list_contains".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Int], Box::new(MirType::Bool)),
        );
        // Phase 47: zip, flat_map, flatten, enumerate, take, drop, last, nth
        self.known_functions.insert(
            "mesh_list_zip".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_list_flat_map".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        self.known_functions.insert(
            "mesh_list_flatten".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_list_enumerate".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_list_take".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Int], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_list_drop".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Int], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_list_last".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_list_nth".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Int], Box::new(MirType::Ptr)),
        );
        // Map
        self.known_functions.insert(
            "mesh_map_new".to_string(),
            MirType::FnPtr(vec![], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_map_new_typed".to_string(),
            MirType::FnPtr(vec![MirType::Int], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_map_tag_string".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_map_put".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::Int, MirType::Int],
                Box::new(MirType::Ptr),
            ),
        );
        self.known_functions.insert(
            "mesh_map_get".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Int], Box::new(MirType::Int)),
        );
        self.known_functions.insert(
            "mesh_map_fetch".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Int], Box::new(MirType::Int)),
        );
        self.known_functions.insert(
            "mesh_map_has_key".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Int], Box::new(MirType::Bool)),
        );
        self.known_functions.insert(
            "mesh_map_delete".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Int], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_map_size".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Int)),
        );
        self.known_functions.insert(
            "mesh_map_keys".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_map_values".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        // Phase 47: Map merge/to_list/from_list
        self.known_functions.insert(
            "mesh_map_merge".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_map_to_list".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_map_from_list".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        // Set
        self.known_functions.insert(
            "mesh_set_new".to_string(),
            MirType::FnPtr(vec![], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_set_add".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Int], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_set_remove".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Int], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_set_contains".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Int], Box::new(MirType::Bool)),
        );
        self.known_functions.insert(
            "mesh_set_size".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Int)),
        );
        self.known_functions.insert(
            "mesh_set_union".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_set_intersection".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        // Phase 47: Set difference/to_list/from_list
        self.known_functions.insert(
            "mesh_set_difference".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_set_to_list".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_set_from_list".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        // Collection Display (Phase 21 Plan 04)
        self.known_functions.insert(
            "mesh_list_to_string".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_map_to_string".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        self.known_functions.insert(
            "mesh_set_to_string".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_string_to_string".to_string(),
            MirType::FnPtr(vec![MirType::Int], Box::new(MirType::Ptr)),
        );
        // List Eq/Ord (Phase 27)
        self.known_functions.insert(
            "mesh_list_eq".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Bool),
            ),
        );
        self.known_functions.insert(
            "mesh_list_compare".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Int),
            ),
        );
        // Tuple
        self.known_functions.insert(
            "mesh_tuple_nth".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Int], Box::new(MirType::Int)),
        );
        self.known_functions.insert(
            "mesh_tuple_first".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Int)),
        );
        self.known_functions.insert(
            "mesh_tuple_second".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Int)),
        );
        self.known_functions.insert(
            "mesh_tuple_size".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Int)),
        );
        // Range
        self.known_functions.insert(
            "mesh_range_new".to_string(),
            MirType::FnPtr(vec![MirType::Int, MirType::Int], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_range_to_list".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_range_map".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        self.known_functions.insert(
            "mesh_range_filter".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        self.known_functions.insert(
            "mesh_range_length".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Int)),
        );
        // Queue
        self.known_functions.insert(
            "mesh_queue_new".to_string(),
            MirType::FnPtr(vec![], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_queue_push".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Int], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_queue_pop".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_queue_peek".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Int)),
        );
        self.known_functions.insert(
            "mesh_queue_size".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Int)),
        );
        self.known_functions.insert(
            "mesh_queue_is_empty".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Bool)),
        );
        // JSON functions (Phase 8 Plan 04)
        self.known_functions.insert(
            "mesh_json_parse".to_string(),
            MirType::FnPtr(vec![MirType::String], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_json_encode".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::String)),
        );
        self.known_functions.insert(
            "mesh_json_encode_string".to_string(),
            MirType::FnPtr(vec![MirType::String], Box::new(MirType::String)),
        );
        self.known_functions.insert(
            "mesh_json_encode_int".to_string(),
            MirType::FnPtr(vec![MirType::Int], Box::new(MirType::String)),
        );
        self.known_functions.insert(
            "mesh_json_encode_bool".to_string(),
            MirType::FnPtr(vec![MirType::Bool], Box::new(MirType::String)),
        );
        self.known_functions.insert(
            "mesh_json_encode_map".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::String)),
        );
        self.known_functions.insert(
            "mesh_json_encode_list".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::String)),
        );
        self.known_functions.insert(
            "mesh_json_from_int".to_string(),
            MirType::FnPtr(vec![MirType::Int], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_json_from_float".to_string(),
            MirType::FnPtr(vec![MirType::Float], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_json_from_bool".to_string(),
            MirType::FnPtr(vec![MirType::Bool], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_json_from_string".to_string(),
            MirType::FnPtr(vec![MirType::String], Box::new(MirType::Ptr)),
        );
        // Phase 103: JSON field extraction (no DB roundtrip)
        // mesh_json_get(json: String, key: String) -> String
        self.known_functions.insert(
            "mesh_json_get".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        // mesh_json_get_nested(json: String, path1: String, path2: String) -> String
        self.known_functions.insert(
            "mesh_json_get_nested".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        self.known_functions.insert(
            "mesh_json_is_string".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Bool)),
        );
        // JSON structured object/array functions (Phase 49)
        self.known_functions.insert(
            "mesh_json_object_new".to_string(),
            MirType::FnPtr(vec![], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_json_object_put".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        self.known_functions.insert(
            "mesh_json_object_get".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_json_array_new".to_string(),
            MirType::FnPtr(vec![], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_json_array_push".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_json_array_get".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Int], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_json_as_int".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_json_as_float".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_json_as_string".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_json_as_bool".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        for name in [
            "mesh_json_value_as_int",
            "mesh_json_value_as_float",
            "mesh_json_value_as_bool",
        ] {
            self.known_functions.insert(
                name.to_string(),
                MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
            );
        }
        self.known_functions.insert(
            "mesh_json_null".to_string(),
            MirType::FnPtr(vec![], Box::new(MirType::Ptr)),
        );
        // JSON collection helpers (callback-based, for List<T> and Map<String, V> fields)
        self.known_functions.insert(
            "mesh_json_from_list".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_json_from_map".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_json_to_list".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_json_to_map".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        // Result helpers (for from_json Result propagation)
        self.known_functions.insert(
            "mesh_alloc_result".to_string(),
            MirType::FnPtr(vec![MirType::Int, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_result_is_ok".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Int)),
        );
        self.known_functions.insert(
            "mesh_result_unwrap".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        // HTTP functions (Phase 8 Plan 05)
        self.known_functions.insert(
            "mesh_http_router".to_string(),
            MirType::FnPtr(vec![], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_http_route".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::String, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        self.known_functions.insert(
            "mesh_http_serve".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Int], Box::new(MirType::Unit)),
        );
        self.known_functions.insert(
            "mesh_http_serve_tls".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::Int, MirType::String, MirType::String],
                Box::new(MirType::Unit),
            ),
        );
        self.known_functions.insert(
            "mesh_http_response_new".to_string(),
            MirType::FnPtr(vec![MirType::Int, MirType::String], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_http_response_bytes_new".to_string(),
            MirType::FnPtr(vec![MirType::Int, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_http_response_bytes_with_headers".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        self.known_functions.insert(
            "mesh_http_response_with_headers".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::String, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        self.known_functions.insert(
            "mesh_http_request_method".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::String)),
        );
        self.known_functions.insert(
            "mesh_http_request_path".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::String)),
        );
        self.known_functions.insert(
            "mesh_http_request_body".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::String)),
        );
        self.known_functions.insert(
            "mesh_http_request_body_bytes".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_http_request_header".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::String], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_http_request_query".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::String], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_http_request_id".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::String)),
        );
        self.known_functions.insert(
            "mesh_http_idempotency_key".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_cluster_capacity".to_string(),
            MirType::FnPtr(vec![], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_cluster_pressure".to_string(),
            MirType::FnPtr(vec![], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_cluster_telemetry".to_string(),
            MirType::FnPtr(vec![], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_cluster_role".to_string(),
            MirType::FnPtr(vec![], Box::new(MirType::String)),
        );
        self.known_functions.insert(
            "mesh_cluster_state".to_string(),
            MirType::FnPtr(vec![], Box::new(MirType::String)),
        );
        // Phase 51: Method-specific routing and path parameter extraction
        self.known_functions.insert(
            "mesh_http_route_get".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::String, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        self.known_functions.insert(
            "mesh_http_route_post".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::String, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        self.known_functions.insert(
            "mesh_http_route_put".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::String, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        self.known_functions.insert(
            "mesh_http_route_delete".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::String, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        self.known_functions.insert(
            "mesh_http_request_param".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::String], Box::new(MirType::Ptr)),
        );
        // Phase 52: Middleware
        self.known_functions.insert(
            "mesh_http_use_middleware".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        // ── WebSocket functions (Phase 60) ──────────────────────────────
        // mesh_ws_serve(on_connect_fn: ptr, on_connect_env: ptr, on_message_fn: ptr, on_message_env: ptr, on_close_fn: ptr, on_close_env: ptr, port: i64) -> void
        self.known_functions.insert(
            "mesh_ws_serve".to_string(),
            MirType::FnPtr(
                vec![
                    MirType::Ptr,
                    MirType::Ptr,
                    MirType::Ptr,
                    MirType::Ptr,
                    MirType::Ptr,
                    MirType::Ptr,
                    MirType::Int,
                ],
                Box::new(MirType::Unit),
            ),
        );
        // mesh_ws_send(conn: ptr, msg: ptr) -> i64
        self.known_functions.insert(
            "mesh_ws_send".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Int)),
        );
        // mesh_ws_serve_tls(on_connect_fn: ptr, on_connect_env: ptr, on_message_fn: ptr, on_message_env: ptr, on_close_fn: ptr, on_close_env: ptr, port: i64, cert_path: ptr, key_path: ptr) -> void
        self.known_functions.insert(
            "mesh_ws_serve_tls".to_string(),
            MirType::FnPtr(
                vec![
                    MirType::Ptr,
                    MirType::Ptr,
                    MirType::Ptr,
                    MirType::Ptr,
                    MirType::Ptr,
                    MirType::Ptr,
                    MirType::Int,
                    MirType::Ptr,
                    MirType::Ptr,
                ],
                Box::new(MirType::Unit),
            ),
        );
        // ── WebSocket Room functions (Phase 62) ──────────────────────────
        // mesh_ws_join(conn: ptr, room: ptr) -> i64
        self.known_functions.insert(
            "mesh_ws_join".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Int)),
        );
        // mesh_ws_leave(conn: ptr, room: ptr) -> i64
        self.known_functions.insert(
            "mesh_ws_leave".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Int)),
        );
        // mesh_ws_broadcast(room: ptr, msg: ptr) -> i64
        self.known_functions.insert(
            "mesh_ws_broadcast".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Int)),
        );
        // mesh_ws_broadcast_except(room: ptr, msg: ptr, except_conn: ptr) -> i64
        self.known_functions.insert(
            "mesh_ws_broadcast_except".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Int),
            ),
        );
        // ── SQLite functions (Phase 53) ──────────────────────────────────
        // Connection handle is MirType::Int (i64) for GC safety (SQLT-07).
        self.known_functions.insert(
            "mesh_sqlite_open".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_sqlite_close".to_string(),
            MirType::FnPtr(vec![MirType::Int], Box::new(MirType::Unit)),
        );
        self.known_functions.insert(
            "mesh_sqlite_execute".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        self.known_functions.insert(
            "mesh_sqlite_query".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        for name in ["mesh_sqlite_execute_values", "mesh_sqlite_query_values"] {
            self.known_functions.insert(
                name.to_string(),
                MirType::FnPtr(
                    vec![MirType::Int, MirType::Ptr, MirType::Ptr],
                    Box::new(MirType::Ptr),
                ),
            );
        }
        // ── PostgreSQL functions (Phase 54) ──────────────────────────────
        // Connection handle is MirType::Int (i64) for GC safety (same as SQLite).
        self.known_functions.insert(
            "mesh_pg_connect".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_pg_close".to_string(),
            MirType::FnPtr(vec![MirType::Int], Box::new(MirType::Unit)),
        );
        self.known_functions.insert(
            "mesh_pg_execute".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        self.known_functions.insert(
            "mesh_pg_query".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        for name in ["mesh_pg_execute_values", "mesh_pg_query_values"] {
            self.known_functions.insert(
                name.to_string(),
                MirType::FnPtr(
                    vec![MirType::Int, MirType::Ptr, MirType::Ptr],
                    Box::new(MirType::Ptr),
                ),
            );
        }
        // ── Phase 57: PG Transaction functions ──────────────────────────
        // mesh_pg_begin(conn: i64) -> ptr (Result)
        self.known_functions.insert(
            "mesh_pg_begin".to_string(),
            MirType::FnPtr(vec![MirType::Int], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_pg_commit".to_string(),
            MirType::FnPtr(vec![MirType::Int], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_pg_rollback".to_string(),
            MirType::FnPtr(vec![MirType::Int], Box::new(MirType::Ptr)),
        );
        // mesh_pg_transaction(conn: i64, fn_ptr: ptr, env_ptr: ptr) -> ptr
        self.known_functions.insert(
            "mesh_pg_transaction".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        // ── PostgreSQL expression helpers ───────────────────────────────
        self.known_functions.insert(
            "mesh_pg_cast".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        for name in [
            "mesh_pg_jsonb",
            "mesh_pg_int",
            "mesh_pg_text",
            "mesh_pg_uuid",
            "mesh_pg_timestamptz",
        ] {
            self.known_functions.insert(
                name.to_string(),
                MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
            );
        }
        self.known_functions.insert(
            "mesh_pg_gen_salt".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Int], Box::new(MirType::Ptr)),
        );
        for name in [
            "mesh_pg_crypt",
            "mesh_pg_ts_rank",
            "mesh_pg_tsvector_matches",
            "mesh_pg_jsonb_contains",
        ] {
            self.known_functions.insert(
                name.to_string(),
                MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
            );
        }
        for name in ["mesh_pg_to_tsvector", "mesh_pg_plainto_tsquery"] {
            self.known_functions.insert(
                name.to_string(),
                MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
            );
        }
        // ── PostgreSQL schema helpers ─────────────────────────────────
        self.known_functions.insert(
            "mesh_pg_create_extension".to_string(),
            MirType::FnPtr(vec![MirType::Int, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_pg_create_range_partitioned_table".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::Ptr, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        self.known_functions.insert(
            "mesh_pg_create_gin_index".to_string(),
            MirType::FnPtr(
                vec![
                    MirType::Int,
                    MirType::Ptr,
                    MirType::Ptr,
                    MirType::Ptr,
                    MirType::Ptr,
                ],
                Box::new(MirType::Ptr),
            ),
        );
        self.known_functions.insert(
            "mesh_pg_create_daily_partitions_ahead".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::Ptr, MirType::Int],
                Box::new(MirType::Ptr),
            ),
        );
        self.known_functions.insert(
            "mesh_pg_list_daily_partitions_before".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::Ptr, MirType::Int],
                Box::new(MirType::Ptr),
            ),
        );
        self.known_functions.insert(
            "mesh_pg_drop_partition".to_string(),
            MirType::FnPtr(vec![MirType::Int, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        // ── Phase 57: SQLite Transaction functions ──────────────────────
        self.known_functions.insert(
            "mesh_sqlite_begin".to_string(),
            MirType::FnPtr(vec![MirType::Int], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_sqlite_commit".to_string(),
            MirType::FnPtr(vec![MirType::Int], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_sqlite_rollback".to_string(),
            MirType::FnPtr(vec![MirType::Int], Box::new(MirType::Ptr)),
        );
        // ── Phase 57: Connection Pool functions ─────────────────────────
        // mesh_pool_open(url: ptr, min: i64, max: i64, timeout: i64) -> ptr
        self.known_functions.insert(
            "mesh_pool_open".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::Int, MirType::Int, MirType::Int],
                Box::new(MirType::Ptr),
            ),
        );
        self.known_functions.insert(
            "mesh_pool_close".to_string(),
            MirType::FnPtr(vec![MirType::Int], Box::new(MirType::Unit)),
        );
        self.known_functions.insert(
            "mesh_pool_query".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        self.known_functions.insert(
            "mesh_pool_execute".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        for name in ["mesh_pool_execute_values", "mesh_pool_query_values"] {
            self.known_functions.insert(
                name.to_string(),
                MirType::FnPtr(
                    vec![MirType::Int, MirType::Ptr, MirType::Ptr],
                    Box::new(MirType::Ptr),
                ),
            );
        }
        // ── Phase 58: Row Parsing & Struct-to-Row Mapping ─────────────────
        self.known_functions.insert(
            "mesh_row_from_row_get".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_row_parse_int".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_row_parse_float".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_row_parse_bool".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        for name in ["mesh_pg_query_as", "mesh_pool_query_as"] {
            self.known_functions.insert(
                name.to_string(),
                MirType::FnPtr(
                    vec![
                        MirType::Int,
                        MirType::Ptr,
                        MirType::Ptr,
                        MirType::Ptr,
                        MirType::Ptr,
                    ],
                    Box::new(MirType::Ptr),
                ),
            );
        }
        // ── Phase 97: ORM SQL Generation ─────────────────────────────────
        // mesh_orm_build_select(table: ptr, columns: ptr, where_clauses: ptr, order_by: ptr, limit: i64, offset: i64) -> ptr
        self.known_functions.insert(
            "mesh_orm_build_select".to_string(),
            MirType::FnPtr(
                vec![
                    MirType::Ptr,
                    MirType::Ptr,
                    MirType::Ptr,
                    MirType::Ptr,
                    MirType::Int,
                    MirType::Int,
                ],
                Box::new(MirType::Ptr),
            ),
        );
        // mesh_orm_build_insert(table: ptr, columns: ptr, returning: ptr) -> ptr
        self.known_functions.insert(
            "mesh_orm_build_insert".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        // mesh_orm_build_update(table: ptr, set_columns: ptr, where_clauses: ptr, returning: ptr) -> ptr
        self.known_functions.insert(
            "mesh_orm_build_update".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::Ptr, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        // mesh_orm_build_delete(table: ptr, where_clauses: ptr, returning: ptr) -> ptr
        self.known_functions.insert(
            "mesh_orm_build_delete".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        // ── Neutral SQL expression builder ────────────────────────────
        self.known_functions.insert(
            "mesh_expr_column".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_expr_value".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_expr_null".to_string(),
            MirType::FnPtr(vec![], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_expr_call".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_expr_add".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_expr_sub".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_expr_mul".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_expr_div".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_expr_eq".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_expr_neq".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_expr_lt".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_expr_lte".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_expr_gt".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_expr_gte".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_expr_case".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        self.known_functions.insert(
            "mesh_expr_coalesce".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_expr_excluded".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_expr_alias".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        // ── Phase 98: Query Builder ─────────────────────────────────────
        // mesh_query_from(table: ptr) -> ptr
        self.known_functions.insert(
            "mesh_query_from".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        // mesh_query_where(q: ptr, field: ptr, value: ptr) -> ptr
        self.known_functions.insert(
            "mesh_query_where".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        // mesh_query_where_op(q: ptr, field: ptr, op: ptr, value: ptr) -> ptr
        self.known_functions.insert(
            "mesh_query_where_op".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::Ptr, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        // mesh_query_where_in(q: ptr, field: ptr, values: ptr) -> ptr
        self.known_functions.insert(
            "mesh_query_where_in".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        // mesh_query_where_null(q: ptr, field: ptr) -> ptr
        self.known_functions.insert(
            "mesh_query_where_null".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        // mesh_query_where_not_null(q: ptr, field: ptr) -> ptr
        self.known_functions.insert(
            "mesh_query_where_not_null".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        // mesh_query_where_not_in(q: ptr, field: ptr, values: ptr) -> ptr
        self.known_functions.insert(
            "mesh_query_where_not_in".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        // mesh_query_where_between(q: ptr, field: ptr, low: ptr, high: ptr) -> ptr
        self.known_functions.insert(
            "mesh_query_where_between".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::Ptr, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        // mesh_query_where_or(q: ptr, fields: ptr, values: ptr) -> ptr
        self.known_functions.insert(
            "mesh_query_where_or".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        // mesh_query_where_expr(q: ptr, expr: ptr) -> ptr
        self.known_functions.insert(
            "mesh_query_where_expr".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        // mesh_query_select(q: ptr, fields: ptr) -> ptr
        self.known_functions.insert(
            "mesh_query_select".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        // mesh_query_select_expr(q: ptr, expr: ptr) -> ptr
        self.known_functions.insert(
            "mesh_query_select_expr".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        // mesh_query_select_exprs(q: ptr, exprs: ptr) -> ptr
        self.known_functions.insert(
            "mesh_query_select_exprs".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        // mesh_query_order_by(q: ptr, field: ptr, direction: ptr) -> ptr
        self.known_functions.insert(
            "mesh_query_order_by".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        // mesh_query_limit(q: ptr, n: i64) -> ptr
        self.known_functions.insert(
            "mesh_query_limit".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Int], Box::new(MirType::Ptr)),
        );
        // mesh_query_offset(q: ptr, n: i64) -> ptr
        self.known_functions.insert(
            "mesh_query_offset".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Int], Box::new(MirType::Ptr)),
        );
        // mesh_query_join(q: ptr, type: ptr, table: ptr, on_clause: ptr) -> ptr
        self.known_functions.insert(
            "mesh_query_join".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::Ptr, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        // mesh_query_join_as(q: ptr, type: ptr, table: ptr, alias: ptr, on_clause: ptr) -> ptr
        self.known_functions.insert(
            "mesh_query_join_as".to_string(),
            MirType::FnPtr(
                vec![
                    MirType::Ptr,
                    MirType::Ptr,
                    MirType::Ptr,
                    MirType::Ptr,
                    MirType::Ptr,
                ],
                Box::new(MirType::Ptr),
            ),
        );
        // mesh_query_group_by(q: ptr, field: ptr) -> ptr
        self.known_functions.insert(
            "mesh_query_group_by".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        // mesh_query_having(q: ptr, clause: ptr, value: ptr) -> ptr
        self.known_functions.insert(
            "mesh_query_having".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        // ── Phase 108: Aggregate SELECT functions ─────────────────────────
        // mesh_query_select_count(q: ptr) -> ptr
        self.known_functions.insert(
            "mesh_query_select_count".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        // mesh_query_select_count_field(q: ptr, field: ptr) -> ptr
        self.known_functions.insert(
            "mesh_query_select_count_field".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        // mesh_query_select_sum(q: ptr, field: ptr) -> ptr
        self.known_functions.insert(
            "mesh_query_select_sum".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        // mesh_query_select_avg(q: ptr, field: ptr) -> ptr
        self.known_functions.insert(
            "mesh_query_select_avg".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        // mesh_query_select_min(q: ptr, field: ptr) -> ptr
        self.known_functions.insert(
            "mesh_query_select_min".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        // mesh_query_select_max(q: ptr, field: ptr) -> ptr
        self.known_functions.insert(
            "mesh_query_select_max".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        // mesh_query_fragment(q: ptr, sql: ptr, params: ptr) -> ptr
        self.known_functions.insert(
            "mesh_query_fragment".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        // ── Phase 103: Query Builder Raw Extensions ─────────────────────
        // mesh_query_order_by_raw(q: ptr, expression: ptr) -> ptr
        self.known_functions.insert(
            "mesh_query_order_by_raw".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        // mesh_query_group_by_raw(q: ptr, expression: ptr) -> ptr
        self.known_functions.insert(
            "mesh_query_group_by_raw".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        // mesh_query_select_raw(q: ptr, expressions: ptr) -> ptr
        self.known_functions.insert(
            "mesh_query_select_raw".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        // mesh_query_where_raw(q: ptr, clause: ptr, params: ptr) -> ptr
        self.known_functions.insert(
            "mesh_query_where_raw".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        // ── Phase 109: Subquery WHERE ─────────────────────────────────────
        // mesh_query_where_sub(q: ptr, field: ptr, sub_query: ptr) -> ptr
        self.known_functions.insert(
            "mesh_query_where_sub".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        // ── Phase 98: Repo Read Operations ───────────────────────────────
        // mesh_repo_all(pool: i64, query: ptr) -> ptr
        self.known_functions.insert(
            "mesh_repo_all".to_string(),
            MirType::FnPtr(vec![MirType::Int, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        // mesh_repo_one(pool: i64, query: ptr) -> ptr
        self.known_functions.insert(
            "mesh_repo_one".to_string(),
            MirType::FnPtr(vec![MirType::Int, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        // mesh_repo_get(pool: i64, table: ptr, id: ptr) -> ptr
        self.known_functions.insert(
            "mesh_repo_get".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        // mesh_repo_get_by(pool: i64, table: ptr, field: ptr, value: ptr) -> ptr
        self.known_functions.insert(
            "mesh_repo_get_by".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::Ptr, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        // mesh_repo_count(pool: i64, query: ptr) -> ptr
        self.known_functions.insert(
            "mesh_repo_count".to_string(),
            MirType::FnPtr(vec![MirType::Int, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        // mesh_repo_exists(pool: i64, query: ptr) -> ptr
        self.known_functions.insert(
            "mesh_repo_exists".to_string(),
            MirType::FnPtr(vec![MirType::Int, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        // ── Phase 98: Repo Write Operations ─────────────────────────────
        // mesh_repo_insert(pool: i64, table: ptr, fields: ptr) -> ptr
        self.known_functions.insert(
            "mesh_repo_insert".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        // mesh_repo_insert_expr(pool: i64, table: ptr, expr_fields: ptr) -> ptr
        self.known_functions.insert(
            "mesh_repo_insert_expr".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        // mesh_repo_update(pool: i64, table: ptr, id: ptr, fields: ptr) -> ptr
        self.known_functions.insert(
            "mesh_repo_update".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::Ptr, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        // mesh_repo_delete(pool: i64, table: ptr, id: ptr) -> ptr
        self.known_functions.insert(
            "mesh_repo_delete".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        // mesh_repo_transaction(pool: i64, fn_ptr: ptr, env_ptr: ptr) -> ptr
        self.known_functions.insert(
            "mesh_repo_transaction".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        // ── Phase 103: Extended Repo Write Operations ────────────────────
        // mesh_repo_update_where(pool: i64, table: ptr, fields: ptr, query: ptr) -> ptr
        self.known_functions.insert(
            "mesh_repo_update_where".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::Ptr, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        // mesh_repo_update_where_expr(pool: i64, table: ptr, expr_fields: ptr, query: ptr) -> ptr
        self.known_functions.insert(
            "mesh_repo_update_where_expr".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::Ptr, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        // mesh_repo_delete_where(pool: i64, table: ptr, query: ptr) -> ptr
        self.known_functions.insert(
            "mesh_repo_delete_where".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        // mesh_repo_query_raw(pool: i64, sql: ptr, params: ptr) -> ptr
        self.known_functions.insert(
            "mesh_repo_query_raw".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        // mesh_repo_execute_raw(pool: i64, sql: ptr, params: ptr) -> ptr
        self.known_functions.insert(
            "mesh_repo_execute_raw".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        // ── Phase 109: Upsert, RETURNING, Subquery ────────────────────────
        // mesh_repo_insert_or_update(pool: i64, table: ptr, fields: ptr, conflict_targets: ptr, update_fields: ptr) -> ptr
        self.known_functions.insert(
            "mesh_repo_insert_or_update".to_string(),
            MirType::FnPtr(
                vec![
                    MirType::Int,
                    MirType::Ptr,
                    MirType::Ptr,
                    MirType::Ptr,
                    MirType::Ptr,
                ],
                Box::new(MirType::Ptr),
            ),
        );
        // mesh_repo_insert_or_update_expr(pool: i64, table: ptr, fields: ptr, conflict_targets: ptr, expr_fields: ptr) -> ptr
        self.known_functions.insert(
            "mesh_repo_insert_or_update_expr".to_string(),
            MirType::FnPtr(
                vec![
                    MirType::Int,
                    MirType::Ptr,
                    MirType::Ptr,
                    MirType::Ptr,
                    MirType::Ptr,
                ],
                Box::new(MirType::Ptr),
            ),
        );
        // mesh_repo_delete_where_returning(pool: i64, table: ptr, query: ptr) -> ptr
        self.known_functions.insert(
            "mesh_repo_delete_where_returning".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        // ── Phase 99: Repo Changeset Operations ─────────────────────────
        // mesh_repo_insert_changeset(pool: i64, table: ptr, changeset: ptr) -> ptr
        self.known_functions.insert(
            "mesh_repo_insert_changeset".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        // mesh_repo_update_changeset(pool: i64, table: ptr, id: ptr, changeset: ptr) -> ptr
        self.known_functions.insert(
            "mesh_repo_update_changeset".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::Ptr, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        // ── Phase 100: Repo Preloading ──────────────────────────────────
        // mesh_repo_preload(pool: i64, rows: ptr, associations: ptr, rel_meta: ptr) -> ptr
        self.known_functions.insert(
            "mesh_repo_preload".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::Ptr, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        // ── Phase 99: Changeset Operations ──────────────────────────────
        // mesh_changeset_cast(data: ptr, params: ptr, allowed: ptr) -> ptr
        self.known_functions.insert(
            "mesh_changeset_cast".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        // mesh_changeset_cast_with_types(data: ptr, params: ptr, allowed: ptr, field_types: ptr) -> ptr
        self.known_functions.insert(
            "mesh_changeset_cast_with_types".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::Ptr, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        // mesh_changeset_validate_required(cs: ptr, fields: ptr) -> ptr
        self.known_functions.insert(
            "mesh_changeset_validate_required".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        // mesh_changeset_validate_length(cs: ptr, field: ptr, min: ptr, max: ptr) -> ptr
        self.known_functions.insert(
            "mesh_changeset_validate_length".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::Ptr, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        // mesh_changeset_validate_format(cs: ptr, field: ptr, pattern: ptr) -> ptr
        self.known_functions.insert(
            "mesh_changeset_validate_format".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        // mesh_changeset_validate_inclusion(cs: ptr, field: ptr, allowed_values: ptr) -> ptr
        self.known_functions.insert(
            "mesh_changeset_validate_inclusion".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        // mesh_changeset_validate_number(cs: ptr, field: ptr, gt: ptr, lt: ptr, gte: ptr, lte: ptr) -> ptr
        self.known_functions.insert(
            "mesh_changeset_validate_number".to_string(),
            MirType::FnPtr(
                vec![
                    MirType::Ptr,
                    MirType::Ptr,
                    MirType::Ptr,
                    MirType::Ptr,
                    MirType::Ptr,
                    MirType::Ptr,
                ],
                Box::new(MirType::Ptr),
            ),
        );
        // mesh_changeset_valid(cs: ptr) -> ptr
        self.known_functions.insert(
            "mesh_changeset_valid".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        // mesh_changeset_errors(cs: ptr) -> ptr
        self.known_functions.insert(
            "mesh_changeset_errors".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        // mesh_changeset_changes(cs: ptr) -> ptr
        self.known_functions.insert(
            "mesh_changeset_changes".to_string(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        // mesh_changeset_get_change(cs: ptr, field: ptr) -> ptr
        self.known_functions.insert(
            "mesh_changeset_get_change".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        // mesh_changeset_get_error(cs: ptr, field: ptr) -> ptr
        self.known_functions.insert(
            "mesh_changeset_get_error".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        // ── Phase 101: Migration DDL Operations ─────────────────────────
        // mesh_migration_create_table(pool: i64, table: ptr, columns: ptr) -> ptr
        self.known_functions.insert(
            "mesh_migration_create_table".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        // mesh_migration_drop_table(pool: i64, table: ptr) -> ptr
        self.known_functions.insert(
            "mesh_migration_drop_table".to_string(),
            MirType::FnPtr(vec![MirType::Int, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        // mesh_migration_add_column(pool: i64, table: ptr, col_def: ptr) -> ptr
        self.known_functions.insert(
            "mesh_migration_add_column".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        // mesh_migration_drop_column(pool: i64, table: ptr, col: ptr) -> ptr
        self.known_functions.insert(
            "mesh_migration_drop_column".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        // mesh_migration_rename_column(pool: i64, table: ptr, old: ptr, new: ptr) -> ptr
        self.known_functions.insert(
            "mesh_migration_rename_column".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::Ptr, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        // mesh_migration_create_index(pool: i64, table: ptr, cols: ptr, opts: ptr) -> ptr
        self.known_functions.insert(
            "mesh_migration_create_index".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::Ptr, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        // mesh_migration_drop_index(pool: i64, table: ptr, cols: ptr) -> ptr
        self.known_functions.insert(
            "mesh_migration_drop_index".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        // mesh_migration_execute(pool: i64, sql: ptr) -> ptr
        self.known_functions.insert(
            "mesh_migration_execute".to_string(),
            MirType::FnPtr(vec![MirType::Int, MirType::Ptr], Box::new(MirType::Ptr)),
        );
        // ── Job functions (Phase 9 Plan 04) ──────────────────────────────
        // mesh_job_async takes (fn_ptr, env_ptr) -> i64 (PID)
        // But the closure splitting at codegen will expand the closure arg into (fn_ptr, env_ptr)
        self.known_functions.insert(
            "mesh_job_async".to_string(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Int)),
        );
        self.known_functions.insert(
            "mesh_job_await".to_string(),
            MirType::FnPtr(vec![MirType::Int], Box::new(MirType::Ptr)),
        );
        self.known_functions.insert(
            "mesh_job_await_timeout".to_string(),
            MirType::FnPtr(vec![MirType::Int, MirType::Int], Box::new(MirType::Ptr)),
        );
        // mesh_job_map takes (list_ptr, fn_ptr, env_ptr) -> ptr
        // Closure splitting expands the closure arg into (fn_ptr, env_ptr)
        self.known_functions.insert(
            "mesh_job_map".to_string(),
            MirType::FnPtr(
                vec![MirType::Ptr, MirType::Ptr, MirType::Ptr],
                Box::new(MirType::Ptr),
            ),
        );
        // ── Timer functions (Phase 44 Plan 02) ──────────────────────────────
        // mesh_timer_sleep(ms: i64) -> void (Unit)
        self.known_functions.insert(
            "mesh_timer_sleep".to_string(),
            MirType::FnPtr(vec![MirType::Int], Box::new(MirType::Unit)),
        );
        // mesh_timer_send_after(pid: i64, ms: i64, msg_ptr: ptr, msg_size: i64) -> void (Unit)
        self.known_functions.insert(
            "mesh_timer_send_after".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::Int, MirType::Ptr, MirType::Int],
                Box::new(MirType::Unit),
            ),
        );
        // mesh_timer_apply_after(ms: i64, fn_ptr: ptr, env_ptr: ptr) -> void (Unit)
        self.known_functions.insert(
            "mesh_timer_apply_after".to_string(),
            MirType::FnPtr(
                vec![
                    MirType::Int,
                    MirType::Closure(vec![], Box::new(MirType::Int)),
                ],
                Box::new(MirType::Unit),
            ),
        );
        // ── Service runtime functions (Phase 9 Plan 03) ─────────────────
        self.known_functions.insert(
            "mesh_service_call".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::Int, MirType::Ptr, MirType::Int],
                Box::new(MirType::Ptr),
            ),
        );
        self.known_functions.insert(
            "mesh_service_reply".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::Ptr, MirType::Int],
                Box::new(MirType::Unit),
            ),
        );
        self.known_functions.insert(
            "mesh_actor_send".to_string(),
            MirType::FnPtr(
                vec![MirType::Int, MirType::Ptr, MirType::Int],
                Box::new(MirType::Int),
            ),
        );

        // Also register variant constructors as known functions.
        for sum_info in self.registry.sum_type_defs.values() {
            for variant in &sum_info.variants {
                if !variant.fields.is_empty() {
                    // Variant constructor is a function
                    let name = variant.name.clone();
                    let qualified = format!("{}.{}", sum_info.name, variant.name);
                    // We don't have exact types here; mark as known for call dispatch.
                    self.known_functions
                        .insert(name, MirType::FnPtr(vec![], Box::new(MirType::Unit)));
                    self.known_functions
                        .insert(qualified, MirType::FnPtr(vec![], Box::new(MirType::Unit)));
                }
            }
        }

        self.prepare_specializations(&sf);

        // Second pass: lower all items. Consecutive FnDefs with one name and
        // arity are the clauses of one function, as the type checker groups
        // them.
        let items: Vec<Item> = sf.items().collect();
        // Services first: calls to them (`Counter.get(pid)`) anywhere in the
        // module lower to the helper functions a service definition creates.
        for item in &items {
            if let Item::ServiceDef(service_def) = item {
                self.lower_service_def(service_def);
            }
        }
        let mut i = 0;
        while i < items.len() {
            if matches!(items[i], Item::ServiceDef(_)) {
                i += 1;
                continue;
            }
            if let Item::FnDef(ref fn_def) = items[i] {
                let key = |f: &FnDef| {
                    (
                        f.name().and_then(|n| n.text()),
                        f.param_list().map(|pl| pl.params().count()).unwrap_or(0),
                    )
                };
                let mut group: Vec<&FnDef> = vec![fn_def];
                let mut j = i + 1;
                while let Some(Item::FnDef(next_fn)) = items.get(j) {
                    if key(next_fn) != key(fn_def) {
                        break;
                    }
                    group.push(next_fn);
                    j += 1;
                }
                if group.len() > 1 {
                    self.lower_fn_clauses(&group);
                    i = j;
                    continue;
                }
            }
            self.lower_item(items[i].clone());
            i += 1;
        }
    }

    fn lower_item(&mut self, item: Item) {
        match item {
            Item::FnDef(fn_def) => self.lower_fn_def(&fn_def),
            Item::StructDef(struct_def) => self.lower_struct_def(&struct_def),
            Item::SumTypeDef(sum_def) => self.lower_sum_type_def(&sum_def),
            Item::ImplDef(impl_def) => {
                let (trait_name, trait_type_args, type_name) = self.impl_names(&impl_def);

                // Collect names of methods explicitly provided in this impl.
                let mut provided_methods = std::collections::HashSet::new();
                for method in impl_def.methods() {
                    let method_name = method
                        .name()
                        .and_then(|n| n.text())
                        .unwrap_or_else(|| "<unnamed>".to_string());
                    provided_methods.insert(method_name.clone());
                    let mangled = mangle_trait_method(
                        &trait_name,
                        &trait_type_args,
                        &method_name,
                        &type_name,
                    );
                    self.lower_impl_method(&method, &mangled);
                }

                // Lower default method bodies for methods not provided by the impl.
                let trait_def = self
                    .trait_registry
                    .get_trait(&trait_name)
                    .expect("the type checker knows every implemented interface");
                for trait_method in &trait_def.methods {
                    if trait_method.has_default_body
                        && !provided_methods.contains(&trait_method.name)
                    {
                        let key = (trait_name.clone(), trait_method.name.clone());
                        if let Some(&range) = self.default_method_bodies.get(&key) {
                            self.lower_default_method(
                                range,
                                &trait_name,
                                &trait_type_args,
                                &trait_method.name,
                                &type_name,
                            );
                        } else if let Some(foreign) = self.foreign_defaults.get(&key).copied() {
                            // Lowered from the declaring module's syntax
                            // and types, for this module's type.
                            let parse = std::mem::replace(&mut self.parse, foreign.parse);
                            let types = std::mem::replace(&mut self.types, foreign.types);
                            self.lower_default_method(
                                foreign.range,
                                &trait_name,
                                &trait_type_args,
                                &trait_method.name,
                                &type_name,
                            );
                            self.parse = parse;
                            self.types = types;
                        }
                    }
                }
            }
            Item::ActorDef(actor_def) => self.lower_actor_def(&actor_def),
            Item::SupervisorDef(sup_def) => self.lower_supervisor_def(&sup_def),
            // Nothing to lower: interfaces are erased, type aliases resolved
            // and imports settled by the project; services were lowered
            // before any other item; and a module has no global bindings
            // (a project build rejects them, E0080, and the REPL moves them
            // into what it evaluates).
            Item::InterfaceDef(_)
            | Item::TypeAliasDef(_)
            | Item::ModuleDef(_)
            | Item::ImportDecl(_)
            | Item::FromImportDecl(_)
            | Item::ServiceDef(_)
            | Item::LetBinding(_) => {}
        }
    }

    // ── Function lowering ────────────────────────────────────────────

    fn lower_fn_def(&mut self, fn_def: &FnDef) {
        self.lower_fn_clauses(&[fn_def]);
    }

    /// Lower a function written as one or more clauses: consecutive `fn`s
    /// with one name and arity. Types come from the first clause.
    fn lower_fn_clauses(&mut self, clauses: &[&FnDef]) {
        let fn_def = clauses[0];
        let original_name = self.fn_def_name(fn_def);

        let fn_ty_raw = self
            .get_ty(fn_def.syntax().text_range())
            .cloned()
            .expect("the type checker types every function");

        let base_name = if original_name == "main" {
            self.entry_function = Some("mesh_main".to_string());
            "mesh_main".to_string()
        } else {
            self.qualify_name(&original_name)
        };

        if let Some(native) = fn_def.native_decl() {
            let (param_tys, return_ty) = fun_parts(&fn_ty_raw);
            let source_params = fn_def
                .param_list()
                .map(|params| params.params().collect::<Vec<_>>())
                .unwrap_or_default();
            let params = source_params
                .into_iter()
                .zip(param_tys)
                .map(|(param, ty)| {
                    (
                        param
                            .name()
                            .map(|name| name.text().to_string())
                            .unwrap_or_else(|| "_".to_string()),
                        runtime_value_type(resolve_type(ty, self.registry)),
                    )
                })
                .collect();
            self.native_functions.push(MirNativeFunction {
                name: base_name,
                symbol: native.symbol().unwrap_or_default(),
                params,
                return_type: runtime_value_type(resolve_type(return_ty, self.registry)),
            });
            return;
        }

        let specialization_tys = self
            .inferred_fn_specializations
            .get(&original_name)
            .cloned()
            .unwrap_or_default();

        if specialization_tys.len() > 1 {
            for usage_ty in specialization_tys {
                let emitted_name = self.mangle_inferred_fn_name(&base_name, &usage_ty);
                self.lower_fn_def_variant(
                    clauses,
                    &original_name,
                    &fn_ty_raw,
                    &usage_ty,
                    emitted_name,
                    false,
                );
            }
            return;
        }

        let concrete_fn_ty = specialization_tys.first().unwrap_or(&fn_ty_raw);
        self.lower_fn_def_variant(
            clauses,
            &original_name,
            &fn_ty_raw,
            concrete_fn_ty,
            base_name,
            true,
        );
    }

    fn lower_fn_def_variant(
        &mut self,
        clauses: &[&FnDef],
        original_name: &str,
        fn_ty_raw: &Ty,
        concrete_fn_ty: &Ty,
        emitted_name: String,
        update_original_name: bool,
    ) {
        let fn_def = clauses[0];
        let mut params = Vec::new();
        let mut owned_resource_params = Vec::new();
        self.push_scope();
        let outer_spec_types =
            self.specialize_types(fn_def.syntax().text_range(), fn_ty_raw, concrete_fn_ty);
        // Several clauses, a guard, or a parameter written as a pattern: the
        // body matches the arguments against each clause in turn.
        let matched = clauses.len() > 1
            || fn_def.guard().is_some()
            || fn_def
                .param_list()
                .is_some_and(|pl| pl.params().any(|param| param.pattern().is_some()));
        let (param_srcs, ret) = fun_parts(concrete_fn_ty);

        if matched {
            for (param_idx, param_ty) in param_srcs.iter().enumerate() {
                let param_name = format!("__param_{param_idx}");
                let mir_ty = runtime_value_type(resolve_type(param_ty, self.registry));
                self.insert_var(param_name.clone(), mir_ty.clone());
                params.push((param_name, mir_ty));
            }
        } else if let Some(param_list) = fn_def.param_list() {
            for (param, param_ty) in param_list.params().zip(param_srcs) {
                let param_name = param
                    .name()
                    .map(|t| t.text().to_string())
                    .unwrap_or_else(|| "_".to_string());
                let mir_ty = runtime_value_type(resolve_type(param_ty, self.registry));
                self.insert_var(param_name.clone(), mir_ty.clone());
                if let Some(ty) = self.owned_resource(&param, Some(param_ty)) {
                    owned_resource_params.push((param_name.clone(), ty));
                }
                params.push((param_name, mir_ty));
            }
        }

        let return_type = runtime_value_type(resolve_type(ret, self.registry));
        let return_typeck = Some(ret.clone());

        let prev_fn_return_type = self.current_fn_return_type.take();
        let prev_fn_return_typeck = self.current_fn_return_typeck.take();
        self.current_fn_return_type = Some(return_type.clone());
        self.current_fn_return_typeck = return_typeck;

        let mut body = if matched {
            self.lower_clause_match(clauses, &params, param_srcs, &return_type, &emitted_name)
        } else {
            self.lower_fn_body(fn_def)
        };

        body = self.wrap_resource_scopes(body, owned_resource_params);

        self.current_fn_return_type = prev_fn_return_type;
        self.current_fn_return_typeck = prev_fn_return_typeck;
        self.spec_types = outer_spec_types;
        self.pop_scope();

        let fn_ty = MirType::FnPtr(
            params.iter().map(|(_, t)| t.clone()).collect(),
            Box::new(return_type.clone()),
        );
        self.known_functions
            .insert(emitted_name.clone(), fn_ty.clone());
        if update_original_name && emitted_name != original_name {
            self.known_functions
                .insert(original_name.to_string(), fn_ty.clone());
        }

        let has_tail_calls = rewrite_tail_calls(&mut body, &emitted_name);

        self.functions.push(MirFunction {
            name: emitted_name,
            params,
            return_type,
            body,
            is_closure_fn: false,
            captures: Vec::new(),
            has_tail_calls,
        });
    }

    /// A function's body: its block, or the expression after `=`.
    fn lower_fn_body(&mut self, fn_def: &FnDef) -> MirExpr {
        match fn_def.body() {
            Some(block) => self.lower_block(&block),
            None => {
                let expr = fn_def
                    .expr_body()
                    .expect("the parser gives every function a body");
                self.lower_expr(&expr)
            }
        }
    }

    // ── Impl method lowering ───────────────────────────────────────

    /// Lower a single impl method to a MirFunction with a mangled name.
    /// The `self` parameter is named "self", with the concrete implementing
    /// struct type.
    fn lower_impl_method(&mut self, method: &FnDef, mangled_name: &str) {
        let fn_ty = self
            .get_ty(method.syntax().text_range())
            .cloned()
            .expect("the type checker types every method");
        let (param_tys, ret) = fun_parts(&fn_ty);

        // Extract parameter names and types.
        let mut params = Vec::new();
        let mut owned = Vec::new();
        self.push_scope();

        if let Some(param_list) = method.param_list() {
            for (param, param_ty) in param_list.params().zip(param_tys) {
                let param_name = if param.is_self() {
                    "self".to_string()
                } else {
                    param
                        .name()
                        .map(|t| t.text().to_string())
                        .unwrap_or_else(|| "_".to_string())
                };

                // Use the Ty::Fun param type for all params (including self).
                // The type checker stores the impl type as the first param type.
                // A tuple arrives as the pointer to its heap block, as it does
                // in any other function.
                let mir_ty = runtime_value_type(resolve_type(param_ty, self.registry));
                self.insert_var(param_name.clone(), mir_ty.clone());
                if let Some(ty) = self.owned_resource(&param, Some(param_ty)) {
                    owned.push((param_name.clone(), ty));
                }
                params.push((param_name, mir_ty));
            }
        }

        let return_type = runtime_value_type(resolve_type(ret, self.registry));
        let return_typeck = Some(ret.clone());

        // Track current function return type for ? operator desugaring (Phase 45).
        let prev_fn_return_type = self.current_fn_return_type.take();
        let prev_fn_return_typeck = self.current_fn_return_typeck.take();
        self.current_fn_return_type = Some(return_type.clone());
        self.current_fn_return_typeck = return_typeck;

        let body = self.lower_fn_body(method);
        let mut body = self.wrap_resource_scopes(body, owned);

        // Restore previous function return type.
        self.current_fn_return_type = prev_fn_return_type;
        self.current_fn_return_typeck = prev_fn_return_typeck;

        self.pop_scope();

        // TCE: Rewrite self-recursive tail calls to TailCall nodes (Phase 48).
        let has_tail_calls = rewrite_tail_calls(&mut body, mangled_name);

        self.functions.push(MirFunction {
            name: mangled_name.to_string(),
            params,
            return_type,
            body,
            is_closure_fn: false,
            captures: Vec::new(),
            has_tail_calls,
        });
    }

    // ── Default method body lowering ─────────────────────────────────

    /// Lower a default method body from an interface definition for a concrete type.
    ///
    /// The default body is re-lowered per concrete type (monomorphization model).
    /// The `self` parameter is bound to the concrete impl type.
    fn lower_default_method(
        &mut self,
        method_range: TextRange,
        trait_name: &str,
        trait_type_args: &[String],
        method_name: &str,
        type_name: &str,
    ) {
        // The type checker found the default body at this range.
        let interface_method = self
            .parse
            .syntax()
            .descendants()
            .filter_map(InterfaceMethod::cast)
            .find(|method| method.syntax().text_range() == method_range)
            .expect("an interface method's default body is where the type checker found it");
        let body_block = interface_method
            .body()
            .expect("an interface method with a default body has one");

        let mangled = mangle_trait_method(trait_name, trait_type_args, method_name, type_name);

        // The body was type-checked once with `self :: Self`; every type
        // recorded inside it is re-read with `Self` as this implementing type,
        // the way a generic function's body is specialized per instantiation.
        let self_ty = Ty::Con(mesh_typeck::ty::TyCon::new(type_name));
        let substitutions: HashMap<String, &Ty> = [("Self".to_string(), &self_ty)].into();
        let specialized: FxHashMap<TextRange, Ty> = self
            .types
            .iter()
            .filter(|(range, _)| method_range.contains_range(**range))
            .map(|(range, ty)| (*range, substitute_type_params(ty, &substitutions)))
            .collect();
        let outer_spec_types = std::mem::replace(&mut self.spec_types, specialized);
        let checked = self
            .get_ty(method_range)
            .cloned()
            .expect("the type checker types every interface method");
        let (checked_params, checked_return) = fun_parts(&checked);

        // Build parameters: bind `self` to the concrete type.
        let mut params = Vec::new();
        let mut owned = Vec::new();
        self.push_scope();

        if let Some(param_list) = interface_method.param_list() {
            for param in param_list.params() {
                let is_self = param.is_self();

                let param_name = if is_self {
                    "self".to_string()
                } else {
                    param
                        .name()
                        .map(|t| t.text().to_string())
                        .unwrap_or_else(|| "_".to_string())
                };

                // The checked signature (with `Self` already this type).
                let (typeck_ty, mir_ty) = if is_self {
                    (&self_ty, resolve_type(&self_ty, self.registry))
                } else {
                    let ty = &checked_params[params.len()];
                    (ty, runtime_value_type(resolve_type(ty, self.registry)))
                };

                self.insert_var(param_name.clone(), mir_ty.clone());
                if let Some(ty) = self.owned_resource(&param, Some(typeck_ty)) {
                    owned.push((param_name.clone(), ty));
                }
                params.push((param_name, mir_ty));
            }
        }

        let return_type = runtime_value_type(resolve_type(checked_return, self.registry));

        // Lower the default body.
        let body = self.lower_block(&body_block);
        let mut body = self.wrap_resource_scopes(body, owned);

        self.pop_scope();
        self.spec_types = outer_spec_types;

        // TCE: Rewrite self-recursive tail calls to TailCall nodes (Phase 48).
        let has_tail_calls = rewrite_tail_calls(&mut body, &mangled);

        self.functions.push(MirFunction {
            name: mangled,
            params,
            return_type,
            body,
            is_closure_fn: false,
            captures: Vec::new(),
            has_tail_calls,
        });
    }

    // ── Multi-clause function lowering ──────────────────────────────

    /// Lower a group of consecutive same-name FnDef nodes (multi-clause function)
    /// into a single MirFunction with a match body dispatching on parameter patterns.
    /// The body of a function written as clauses: one match of the
    /// parameters (`__param_N`) against each clause's parameter patterns and
    /// guard. Several parameters are matched as columns; no tuple is built.
    /// A function written as clauses: a match of its arguments against each
    /// clause's patterns. The resources a clause binds are its own, dropped
    /// as it ends, as a function drops the resource parameters it owns; a
    /// call no clause matches drops its resource arguments, then panics.
    /// Both leaked the resource before.
    fn lower_clause_match(
        &mut self,
        clauses: &[&FnDef],
        params: &[(String, MirType)],
        param_srcs: &[Ty],
        return_type: &MirType,
        fn_name: &str,
    ) -> MirExpr {
        let mut arms = Vec::new();
        for clause in clauses {
            self.push_scope();
            let discards_before = self.discarded_resources.len();
            let pattern = self.clause_pattern(clause.param_list(), params, param_srcs);
            let discarded = self.discarded_resources.split_off(discards_before);
            let guard = self.lower_clause_guard(clause);
            let body = self.lower_fn_body(clause);
            let mut owned = self.clause_resource_bindings(clause, param_srcs);
            owned.extend(discarded);
            let body = self.wrap_resource_scopes(body, owned);
            self.pop_scope();
            arms.push(MirMatchArm {
                pattern,
                guard,
                body,
            });
        }
        // A borrowed parameter is the caller's to drop.
        let borrowed: Vec<bool> = clauses
            .first()
            .and_then(|clause| clause.param_list())
            .map(|list| {
                list.params()
                    .map(|param| param.ownership() == ParamOwnership::Borrow)
                    .collect()
            })
            .unwrap_or_default();
        let mut no_match: Vec<MirExpr> = params
            .iter()
            .zip(param_srcs)
            .enumerate()
            .filter(|(index, _)| !borrowed.get(*index).copied().unwrap_or(false))
            .filter_map(|(_, ((name, _), ty))| {
                let destructor = self.resource_destructor(ty)?;
                Some(Self::resource_drop(
                    name,
                    &resolve_type(ty, self.registry),
                    destructor,
                ))
            })
            .collect();
        if !no_match.is_empty() {
            no_match.push(MirExpr::Panic {
                message: "non-exhaustive match".to_string(),
                file: fn_name.to_string(),
                line: 0,
            });
            arms.push(MirMatchArm {
                pattern: MirPattern::Wildcard,
                guard: None,
                body: MirExpr::Block(no_match, MirType::Never),
            });
        }

        MirExpr::Match {
            scrutinee: Box::new(clause_scrutinee(params)),
            arms,
            ty: return_type.clone(),
        }
    }

    /// What a clause's parameters (in `param_list`) match the arguments,
    /// bound as `params` and typed `param_srcs`, against: one parameter's
    /// pattern, several as a tuple (matched as columns), none anything. A
    /// parameter written as a name binds it.
    fn clause_pattern(
        &mut self,
        param_list: Option<ParamList>,
        params: &[(String, MirType)],
        param_srcs: &[Ty],
    ) -> MirPattern {
        let mut patterns: Vec<MirPattern> = param_list
            .iter()
            .flat_map(ParamList::params)
            .zip(params.iter().zip(param_srcs))
            .map(|(param, ((_, ty), source))| match param.pattern() {
                Some(pattern) => {
                    let discards_before = self.discarded_resources.len();
                    let lowered = self.lower_pattern_with_expected(&pattern, Some(source));
                    // A borrowed parameter's resources are its caller's to drop.
                    if param.ownership() == ParamOwnership::Borrow {
                        self.discarded_resources.truncate(discards_before);
                    }
                    lowered
                }
                None => {
                    let name = param
                        .name()
                        .expect("a parameter is a pattern or a name")
                        .text()
                        .to_string();
                    self.insert_var(name.clone(), ty.clone());
                    MirPattern::Var(name, ty.clone())
                }
            })
            .collect();
        match patterns.len() {
            0 => MirPattern::Wildcard,
            1 => patterns.pop().unwrap(),
            _ => MirPattern::Tuple(patterns),
        }
    }

    /// The resources a clause's parameters own: a resource-typed name, or
    /// the resources a pattern binds, unless the parameter is borrowed.
    fn clause_resource_bindings(&self, clause: &FnDef, param_srcs: &[Ty]) -> Vec<(String, Ty)> {
        let mut bindings = Vec::new();
        for (param, ty) in clause
            .param_list()
            .iter()
            .flat_map(|list| list.params())
            .zip(param_srcs)
            .filter(|(param, _)| param.ownership() != ParamOwnership::Borrow)
        {
            if let Some(pattern) = param.pattern() {
                bindings.extend(self.resource_pattern_bindings(&pattern));
            } else if self.registry.is_resource_type(ty) {
                let name = param
                    .name()
                    .expect("a clause parameter is a pattern or a name");
                bindings.push((name.text().to_string(), ty.clone()));
            }
        }
        bindings
    }

    /// Lower a clause's guard expression to an optional MirExpr.
    fn lower_clause_guard(&mut self, clause: &FnDef) -> Option<MirExpr> {
        clause
            .guard()
            .and_then(|gc| gc.expr())
            .map(|e| self.lower_expr(&e))
    }

    // ── Struct lowering ──────────────────────────────────────────────

    fn lower_struct_def(&mut self, struct_def: &StructDef) {
        if struct_def.is_opaque_resource() {
            return;
        }

        let name = struct_def
            .name()
            .and_then(|n| n.text())
            .expect("the parser names every struct");
        let registry = self.registry;
        let info = registry
            .struct_defs
            .get(&name)
            .expect("the type checker registers every struct");

        let fields: Vec<(String, MirType)> = info
            .fields
            .iter()
            // A tuple field holds the pointer to its heap block.
            .map(|(fname, fty)| {
                (
                    fname.clone(),
                    runtime_value_type(resolve_type(fty, registry)),
                )
            })
            .collect();

        // Check if this is a generic struct (trait functions generated lazily at instantiation).
        let has_generic_params = !info.generic_params.is_empty();

        if struct_def.is_declared_resource() {
            if !has_generic_params {
                self.structs.push(MirStructDef { name, fields });
            }
            return;
        }

        if !has_generic_params {
            // Conditional MIR generation based on deriving clause.
            // No deriving clause = backward compat (generate all default trait functions).
            let has_deriving = struct_def.has_deriving_clause();
            let derive_list = struct_def.deriving_traits();
            let derive_all = !has_deriving;
            // Only what the type checker granted (a type holding a function
            // gets no Eq, Ord, Hash, Debug or Display).
            let struct_ty = Ty::Con(mesh_typeck::ty::TyCon::new(&name));
            let granted = |lowerer: &Self, t: &str| lowerer.trait_registry.has_impl(t, &struct_ty);

            let typed_fields = info.fields.clone();
            if (derive_all || derive_list.iter().any(|t| t == "Debug")) && granted(self, "Debug") {
                self.generate_display_struct_typed(&name, &name, &name, &typed_fields, true);
            }
            if (derive_all || derive_list.iter().any(|t| t == "Eq")) && granted(self, "Eq") {
                self.generate_eq_struct_typed(&name, &typed_fields);
            }
            if (derive_all || derive_list.iter().any(|t| t == "Ord")) && granted(self, "Ord") {
                self.generate_ord_struct_typed(&name, &name, &typed_fields);
            }
            if (derive_all || derive_list.iter().any(|t| t == "Hash")) && granted(self, "Hash") {
                self.generate_hash_struct_typed(&name, &name, &typed_fields);
            }
            // Display: only via explicit deriving(Display), never auto-derived
            if derive_list.iter().any(|t| t == "Display") && granted(self, "Display") {
                self.generate_display_struct_typed(&name, &name, &name, &typed_fields, false);
            }
            // Json: only via explicit deriving(Json), never auto-derived.
            // Its `from_json` string wrapper was made before any item.
            if derive_list.iter().any(|t| t == "Json") {
                self.generate_to_json_struct_typed(&name, &name, &typed_fields);
                self.generate_from_json_struct_typed(&name, &name, &typed_fields);
            }
            // Row: only via explicit deriving(Row), never auto-derived
            if derive_list.iter().any(|t| t == "Row") {
                self.generate_from_row_struct(&name, &typed_fields);
            }

            // Schema: only via explicit deriving(Schema), never auto-derived
            if let Some(schema) = info.schema.clone() {
                // Inject timestamp fields if requested.
                let mut schema_fields = fields.clone();
                if schema.timestamps {
                    schema_fields.push(("inserted_at".to_string(), MirType::String));
                    schema_fields.push(("updated_at".to_string(), MirType::String));
                }

                self.generate_schema_metadata(
                    &name,
                    &schema_fields,
                    &struct_def.relationships(),
                    &schema,
                );

                // Use extended fields (with timestamps) for the struct layout.
                if schema.timestamps {
                    self.structs.push(MirStructDef {
                        name,
                        fields: schema_fields,
                    });
                } else {
                    self.structs.push(MirStructDef { name, fields });
                }
            } else {
                self.structs.push(MirStructDef { name, fields });
            }
        }
        // For generic structs: trait functions generated lazily at instantiation
        // via ensure_monomorphized_struct_trait_fns. The MirStructDef is also
        // generated lazily with the mangled name and concrete field types.
    }

    /// Lazily generate monomorphized trait functions for a generic struct instantiation.
    ///
    /// When a generic struct like `Box<T>` is instantiated as `Box<Int>`, this method:
    /// 1. Computes the mangled name (e.g., "Box_Int")
    /// 2. Substitutes generic params with concrete types in the field list
    /// 3. Generates Display, Eq, Debug, etc. MIR functions with the mangled name
    /// 4. Pushes a MirStructDef with the mangled name and concrete fields
    ///
    /// Called with the instantiation (`Box<Int>`) of the generic struct
    /// `base_name`, from `lower_struct_literal` and `ensure_instantiation_traits`.
    fn ensure_monomorphized_struct_trait_fns(&mut self, base_name: &str, typeck_ty: &Ty) {
        let (_, type_args) = ty_head(typeck_ty).expect("a struct type has a head");
        let registry = self.registry;
        let struct_info = registry
            .struct_defs
            .get(base_name)
            .expect("the type checker registers every struct");

        let mangled = mangle_type_name(base_name, type_args, self.registry);
        let helper = self.instantiation_helper_name(base_name, type_args);

        // Already generated?
        if !self.monomorphized_trait_fns.insert(helper.clone()) {
            return;
        }

        // Build a substitution map: generic param name -> concrete Ty.
        let subst: HashMap<String, &Ty> = struct_info
            .generic_params
            .iter()
            .zip(type_args.iter())
            .map(|(param, arg)| (param.clone(), arg))
            .collect();

        // Substitute generic params with concrete types in the field list.
        let typed_fields: Vec<(String, Ty)> = struct_info
            .fields
            .iter()
            .map(|(fname, fty)| (fname.clone(), substitute_type_params(fty, &subst)))
            .collect();
        let fields: Vec<(String, MirType)> = typed_fields
            .iter()
            .map(|(fname, fty)| {
                (
                    fname.clone(),
                    runtime_value_type(resolve_type(fty, self.registry)),
                )
            })
            .collect();

        // Check which traits are registered via the trait registry.
        // Use the parametric typeck type for lookup (e.g., Ty::App(Con("Box"), [Con("Int")])).
        let has_display = self.trait_registry.has_impl("Display", typeck_ty);
        let has_eq = self.trait_registry.has_impl("Eq", typeck_ty);
        let has_debug = self.trait_registry.has_impl("Debug", typeck_ty);
        let has_ord = self.trait_registry.has_impl("Ord", typeck_ty);
        let has_hash = self.trait_registry.has_impl("Hash", typeck_ty);
        let has_json = self.trait_registry.has_impl("ToJson", typeck_ty);

        // Generate trait functions for the monomorphized name.
        // Display and Debug use base_name for human-readable output (e.g., "Box(42)" not "Box_Int(42)").
        if has_debug {
            self.generate_display_struct_typed(&mangled, &helper, base_name, &typed_fields, true);
        }
        if has_eq {
            self.generate_eq_struct_typed_as(&mangled, &helper, &typed_fields);
        }
        if has_ord {
            self.generate_ord_struct_typed(&mangled, &helper, &typed_fields);
        }
        if has_hash {
            self.generate_hash_struct_typed(&mangled, &helper, &typed_fields);
        }
        if has_display {
            self.generate_display_struct_typed(&mangled, &helper, base_name, &typed_fields, false);
        }
        if has_json {
            self.generate_to_json_struct_typed(&mangled, &helper, &typed_fields);
            self.generate_from_json_struct_typed(&mangled, &helper, &typed_fields);
            self.generate_from_json_string_wrapper(&helper);
        }

        // Push the monomorphized struct definition, once per layout.
        if !self.structs.iter().any(|s| s.name == mangled) {
            self.structs.push(MirStructDef {
                name: mangled,
                fields,
            });
        }
    }

    // ── Sum type lowering ────────────────────────────────────────────

    fn lower_sum_type_def(&mut self, sum_def: &SumTypeDef) {
        let name = sum_def
            .name()
            .and_then(|n| n.text())
            .unwrap_or_else(|| "<unnamed>".to_string());

        // Conditional MIR generation based on deriving clause.
        // No deriving clause = backward compat (generate all default trait functions).
        let has_deriving = sum_def.has_deriving_clause();
        let derive_list = sum_def.deriving_traits();
        let derive_all = !has_deriving;
        // Only what the type checker granted (a type holding a function
        // gets no Eq, Ord, Hash, Debug or Display).
        let sum_ty = Ty::Con(mesh_typeck::ty::TyCon::new(&name));
        let granted = |lowerer: &Self, t: &str| lowerer.trait_registry.has_impl(t, &sum_ty);

        // The source field types: Eq and Display compare and print each
        // payload by its own type.
        // (Its layout was registered with every other sum type's.)
        let typed_variants: Vec<(String, Vec<Ty>)> = self.registry.sum_type_defs[&name]
            .variants
            .iter()
            .map(|v| {
                let fields = v
                    .fields
                    .iter()
                    .map(|f| match f {
                        mesh_typeck::VariantFieldInfo::Positional(ty)
                        | mesh_typeck::VariantFieldInfo::Named(_, ty) => ty.clone(),
                    })
                    .collect();
                (v.name.clone(), fields)
            })
            .collect();

        if (derive_all || derive_list.iter().any(|t| t == "Debug")) && granted(self, "Debug") {
            self.generate_display_sum_typed(&name, &name, &typed_variants, true);
        }
        if (derive_all || derive_list.iter().any(|t| t == "Eq")) && granted(self, "Eq") {
            self.generate_eq_sum_typed(&name, &typed_variants);
        }
        if (derive_all || derive_list.iter().any(|t| t == "Ord")) && granted(self, "Ord") {
            self.generate_ord_sum_typed(&name, &name, &typed_variants);
        }
        // Display: only via explicit deriving(Display), never auto-derived
        if derive_list.iter().any(|t| t == "Display") && granted(self, "Display") {
            self.generate_display_sum_typed(&name, &name, &typed_variants, false);
        }
        // Hash: only via explicit deriving(Hash) for sum types
        if has_deriving && derive_list.iter().any(|t| t == "Hash") && granted(self, "Hash") {
            self.generate_hash_sum_typed(&name, &name, &typed_variants);
        }
        // Json: only via explicit deriving(Json) for sum types. Its
        // `from_json` string wrapper was made before any item.
        if derive_list.iter().any(|t| t == "Json") {
            self.generate_to_json_sum_typed(&name, &name, &typed_variants);
            self.generate_from_json_sum_typed(&name, &name, &typed_variants);
        }
    }

    // ── Debug inspect generation ────────────────────────────────────

    /// Generate a synthetic `Debug__inspect__StructName` MIR function that
    /// produces a developer-readable string like `"Point { x: 1, y: 2 }"`.
    /// Generate `Display__to_string__{helper}` (`Name(a, b)`) or, with
    /// `debug`, `Debug__inspect__{helper}` (`Name { x: a, y: b }`) for the
    /// struct `name`, showing each field by its source type (a `List<String>`
    /// field as a list of strings, not as a pointer).
    fn generate_display_struct_typed(
        &mut self,
        name: &str,
        helper: &str,
        display_name: &str,
        fields: &[(String, Ty)],
        debug: bool,
    ) {
        let mangled = if debug {
            format!("Debug__inspect__{helper}")
        } else {
            format!("Display__to_string__{helper}")
        };
        let struct_ty = MirType::Struct(name.to_string());
        self.known_functions.insert(
            mangled.clone(),
            MirType::FnPtr(vec![struct_ty.clone()], Box::new(MirType::String)),
        );
        let (open, close) = match (debug, fields.is_empty()) {
            (true, true) => (format!("{display_name} {{}}"), ""),
            (true, false) => (format!("{display_name} {{ "), " }"),
            (false, _) => (format!("{display_name}("), ")"),
        };
        let text = |s: &str| MirExpr::StringLit(s.to_string(), MirType::String);
        let mut parts = vec![text(&open)];
        for (i, (field, ty)) in fields.iter().enumerate() {
            if i > 0 {
                parts.push(text(", "));
            }
            if debug {
                parts.push(text(&format!("{field}: ")));
            }
            let value = MirExpr::FieldAccess {
                object: Box::new(MirExpr::Var("self".to_string(), struct_ty.clone())),
                field: field.clone(),
                ty: self.binding_type(ty),
            };
            parts.push(if debug {
                self.debug_string(value, ty)
            } else {
                self.wrap_to_string(value, Some(ty))
            });
        }
        parts.push(text(close));
        self.push_helper_fn(
            &mangled,
            vec![("self".to_string(), struct_ty)],
            MirType::String,
            Self::concat_all(parts),
        );
    }

    // ── Ord generation ──────────────────────────────────────────────

    /// Generate `Ord__lt__{helper}` and `Ord__compare__{helper}` for values
    /// of MIR type `self_ty`. `cmp` builds the three-way comparison (an Int)
    /// of `self` against `other`; the helpers are declared first, so it may
    /// refer to them (a recursive type compares its fields with them).
    fn generate_ord_typed(
        &mut self,
        helper: &str,
        self_ty: MirType,
        cmp: impl FnOnce(&mut Self) -> MirExpr,
    ) {
        let ordering_ty = MirType::SumType("Ordering".to_string());
        let lt = format!("Ord__lt__{helper}");
        let compare = format!("Ord__compare__{helper}");
        let operands = vec![self_ty.clone(), self_ty.clone()];
        self.known_functions.insert(
            lt.clone(),
            MirType::FnPtr(operands.clone(), Box::new(MirType::Bool)),
        );
        self.known_functions.insert(
            compare.clone(),
            MirType::FnPtr(operands.clone(), Box::new(ordering_ty.clone())),
        );
        let params = vec![
            ("self".to_string(), self_ty.clone()),
            ("other".to_string(), self_ty.clone()),
        ];
        let var = |name: &str| MirExpr::Var(name.to_string(), self_ty.clone());
        let body = match cmp(self) {
            // Nothing to compare (no fields, no variants): never less.
            MirExpr::IntLit(0, _) => MirExpr::BoolLit(false, MirType::Bool),
            cmp => MirExpr::BinOp {
                op: BinOp::Lt,
                lhs: Box::new(cmp),
                rhs: Box::new(MirExpr::IntLit(0, MirType::Int)),
                ty: MirType::Bool,
            },
        };
        self.push_helper_fn(&lt, params.clone(), MirType::Bool, body);
        // compare(a, b): Less when a < b, Greater when b < a, else Equal.
        let is_lt = |a: &str, b: &str| {
            Self::call_named(&lt, operands.clone(), vec![var(a), var(b)], MirType::Bool)
        };
        let ordering = |variant: &str| MirExpr::ConstructVariant {
            type_name: "Ordering".to_string(),
            variant: variant.to_string(),
            fields: vec![],
            ty: ordering_ty.clone(),
        };
        let body = MirExpr::If {
            cond: Box::new(is_lt("self", "other")),
            then_body: Box::new(ordering("Less")),
            else_body: Box::new(MirExpr::If {
                cond: Box::new(is_lt("other", "self")),
                then_body: Box::new(ordering("Greater")),
                else_body: Box::new(ordering("Equal")),
                ty: ordering_ty.clone(),
            }),
            ty: ordering_ty.clone(),
        };
        self.push_helper_fn(&compare, params, ordering_ty, body);
    }

    /// Derived Ord for the struct `name`: lexicographic over its fields,
    /// each compared by its source type.
    fn generate_ord_struct_typed(&mut self, name: &str, helper: &str, fields: &[(String, Ty)]) {
        let struct_ty = MirType::Struct(name.to_string());
        self.generate_ord_typed(helper, struct_ty.clone(), |lowerer| {
            let terms = fields
                .iter()
                .map(|(field, ty)| {
                    let access = |object: &str| MirExpr::FieldAccess {
                        object: Box::new(MirExpr::Var(object.to_string(), struct_ty.clone())),
                        field: field.clone(),
                        ty: lowerer.binding_type(ty),
                    };
                    let (a, b) = (access("self"), access("other"));
                    lowerer.cmp_expr(a, b, ty)
                })
                .collect();
            Self::lex_cmp(terms)
        });
    }

    /// Derived Ord for the sum type `name`: by variant order, then
    /// lexicographic over the payload, each field compared by its type.
    fn generate_ord_sum_typed(&mut self, name: &str, helper: &str, variants: &[(String, Vec<Ty>)]) {
        let sum_ty = MirType::SumType(name.to_string());
        self.generate_ord_typed(helper, sum_ty.clone(), |lowerer| {
            if variants.is_empty() {
                return MirExpr::IntLit(0, MirType::Int);
            }
            let int = |n: i64| MirExpr::IntLit(n, MirType::Int);
            let var = |name: &str, ty: &MirType| MirExpr::Var(name.to_string(), ty.clone());
            let variant_pattern =
                |variant: &str, fields: Vec<(String, MirType)>| MirPattern::Constructor {
                    type_name: name.to_string(),
                    variant: variant.to_string(),
                    fields: fields
                        .iter()
                        .map(|(n, t)| MirPattern::Var(n.clone(), t.clone()))
                        .collect(),
                    bindings: fields,
                };
            let wildcards = |variant: &str, arity: usize| MirPattern::Constructor {
                type_name: name.to_string(),
                variant: variant.to_string(),
                fields: vec![MirPattern::Wildcard; arity],
                bindings: vec![],
            };
            let tag = |scrutinee: &str| MirExpr::Match {
                scrutinee: Box::new(var(scrutinee, &sum_ty)),
                arms: variants
                    .iter()
                    .enumerate()
                    .map(|(i, (variant, fields))| MirMatchArm {
                        pattern: wildcards(variant, fields.len()),
                        guard: None,
                        body: int(i as i64),
                    })
                    .collect(),
                ty: MirType::Int,
            };
            // Same variant: compare the payloads.
            let mut arms = Vec::new();
            for (variant, fields) in variants.iter().filter(|(_, f)| !f.is_empty()) {
                let tys: Vec<MirType> = fields.iter().map(|f| lowerer.binding_type(f)).collect();
                let bind = |prefix: &str| -> Vec<(String, MirType)> {
                    tys.iter()
                        .enumerate()
                        .map(|(i, t)| (format!("{prefix}{i}"), t.clone()))
                        .collect()
                };
                let (mine, theirs) = (bind("__self_"), bind("__other_"));
                let terms = fields
                    .iter()
                    .zip(mine.iter().zip(&theirs))
                    .map(|(ty, ((a, t), (b, _)))| lowerer.cmp_expr(var(a, t), var(b, t), ty))
                    .collect();
                let inner = MirExpr::Match {
                    scrutinee: Box::new(var("other", &sum_ty)),
                    arms: vec![
                        MirMatchArm {
                            pattern: variant_pattern(variant, theirs),
                            guard: None,
                            body: Self::lex_cmp(terms),
                        },
                        MirMatchArm {
                            pattern: MirPattern::Wildcard,
                            guard: None,
                            body: int(0),
                        },
                    ],
                    ty: MirType::Int,
                };
                arms.push(MirMatchArm {
                    pattern: variant_pattern(variant, mine),
                    guard: None,
                    body: inner,
                });
            }
            let payload = if arms.is_empty() {
                int(0)
            } else {
                arms.push(MirMatchArm {
                    pattern: MirPattern::Wildcard,
                    guard: None,
                    body: int(0),
                });
                MirExpr::Match {
                    scrutinee: Box::new(var("self", &sum_ty)),
                    arms,
                    ty: MirType::Int,
                }
            };
            let tags = lowerer.cmp_expr(
                var("__self_tag", &MirType::Int),
                var("__other_tag", &MirType::Int),
                &Ty::int(),
            );
            let body = Self::lex_cmp(vec![tags, payload]);
            let bind = |name: &str, value: MirExpr, body: MirExpr| MirExpr::Let {
                name: name.to_string(),
                ty: MirType::Int,
                value: Box::new(value),
                body: Box::new(body),
            };
            bind(
                "__self_tag",
                tag("self"),
                bind("__other_tag", tag("other"), body),
            )
        });
    }

    // ── Compare generation ──────────────────────────────────────────

    /// Generate a synthetic `Ord__compare__PrimitiveName` MIR function for primitives.
    /// Uses BinOp::Lt and BinOp::Eq directly instead of calling trait functions.
    fn generate_compare_primitive(&mut self, type_name: &str, mir_type: MirType) {
        let mangled = format!("Ord__compare__{}", type_name);
        let ordering_ty = MirType::SumType("Ordering".to_string());
        let self_var = MirExpr::Var("self".to_string(), mir_type.clone());
        let other_var = MirExpr::Var("other".to_string(), mir_type.clone());

        // if self < other then Less
        // else if self == other then Equal
        // else Greater
        let body = MirExpr::If {
            cond: Box::new(MirExpr::BinOp {
                op: BinOp::Lt,
                lhs: Box::new(self_var.clone()),
                rhs: Box::new(other_var.clone()),
                ty: MirType::Bool,
            }),
            then_body: Box::new(MirExpr::ConstructVariant {
                type_name: "Ordering".to_string(),
                variant: "Less".to_string(),
                fields: vec![],
                ty: ordering_ty.clone(),
            }),
            else_body: Box::new(MirExpr::If {
                cond: Box::new(MirExpr::BinOp {
                    op: BinOp::Eq,
                    lhs: Box::new(self_var),
                    rhs: Box::new(other_var),
                    ty: MirType::Bool,
                }),
                then_body: Box::new(MirExpr::ConstructVariant {
                    type_name: "Ordering".to_string(),
                    variant: "Equal".to_string(),
                    fields: vec![],
                    ty: ordering_ty.clone(),
                }),
                else_body: Box::new(MirExpr::ConstructVariant {
                    type_name: "Ordering".to_string(),
                    variant: "Greater".to_string(),
                    fields: vec![],
                    ty: ordering_ty.clone(),
                }),
                ty: ordering_ty.clone(),
            }),
            ty: ordering_ty.clone(),
        };

        let func = MirFunction {
            name: mangled.clone(),
            params: vec![
                ("self".to_string(), mir_type.clone()),
                ("other".to_string(), mir_type.clone()),
            ],
            return_type: ordering_ty.clone(),
            body,
            is_closure_fn: false,
            captures: vec![],
            has_tail_calls: false,
        };

        self.functions.push(func);
        self.known_functions.insert(
            mangled,
            MirType::FnPtr(vec![mir_type.clone(), mir_type], Box::new(ordering_ty)),
        );
    }

    // ── json { } literal lowering (Phase 132-02) ─────────────────────

    /// Lower a `json { key: val, ... }` expression to the object it builds,
    /// `mesh_json_object_put(...(mesh_json_object_new()))`: a `Json` like any
    /// other, encoded only where a `String` is expected.
    fn lower_json_expr(&mut self, json_expr: &JsonExpr) -> MirExpr {
        let new_fn_ty = MirType::FnPtr(vec![], Box::new(MirType::Ptr));
        let mut result = MirExpr::Call {
            func: Box::new(MirExpr::Var("mesh_json_object_new".to_string(), new_fn_ty)),
            args: vec![],
            ty: MirType::Ptr,
        };

        let put_fn_ty = MirType::FnPtr(
            vec![MirType::Ptr, MirType::Ptr, MirType::Ptr],
            Box::new(MirType::Ptr),
        );

        for field in json_expr.fields() {
            let key_str = field.key_text().unwrap_or_default();
            let key_mir = MirExpr::StringLit(key_str, MirType::String);

            let val_expr = field
                .value()
                .expect("the parser gives a json field its value");

            // Look up the typeck-inferred type for this field value.
            let val_ty = self
                .types
                .get(&val_expr.syntax().text_range())
                .cloned()
                .unwrap_or_else(Ty::string);

            // Dispatch: choose how to convert the field value to a JSON pointer.
            // A Json (a nested literal among them) is embedded as it is.
            let json_val = if ty_is_json(&val_ty) {
                self.lower_expr(&val_expr)
            } else {
                // All other types: lower to the raw Mesh value then convert it
                // to a JSON pointer by its type (`nil` is null).
                let val_lowered = self.lower_expr(&val_expr);
                self.json_encode_expr(val_lowered, &val_ty)
            };

            result = MirExpr::Call {
                func: Box::new(MirExpr::Var(
                    "mesh_json_object_put".to_string(),
                    put_fn_ty.clone(),
                )),
                args: vec![result, key_mir, json_val],
                ty: MirType::Ptr,
            };
        }

        result
    }

    /// Derived `from_row` for the struct `name`: each field from the row's
    /// (a `Map<String, String>`'s) column of its name, parsed as its type.
    /// An `Option` field is `None` for a missing column or an empty value
    /// (SQL NULL); a missing column of any other field is an error.
    fn generate_from_row_struct(&mut self, name: &str, fields: &[(String, Ty)]) {
        let mangled = format!("FromRow__from_row__{name}");
        self.known_functions.insert(
            mangled.clone(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        let struct_lit = MirExpr::StructLit {
            name: name.to_string(),
            fields: fields
                .iter()
                .enumerate()
                .map(|(i, (field, ty))| {
                    let value = MirExpr::Var(format!("__f_{i}"), self.binding_type(ty));
                    (field.clone(), value)
                })
                .collect(),
            ty: MirType::Struct(name.to_string()),
        };
        let row = MirExpr::Var("row".to_string(), MirType::Ptr);
        let mut body = Self::json_ok(struct_lit);
        for (i, (field, ty)) in fields.iter().enumerate().rev() {
            let (got, text, parsed) = (
                self.json_fresh("column"),
                self.json_fresh("text"),
                self.json_fresh("parsed"),
            );
            let column = Self::json_call(
                "mesh_row_from_row_get",
                vec![
                    row.clone(),
                    MirExpr::StringLit(field.clone(), MirType::String),
                ],
            );
            let decode = MirExpr::Let {
                name: text.clone(),
                ty: MirType::String,
                value: Box::new(Self::json_call(
                    "mesh_result_unwrap",
                    vec![MirExpr::Var(got.clone(), MirType::Ptr)],
                )),
                body: Box::new(self.row_decode(&text, ty)),
            };
            let value = match ty_head(ty) {
                // A missing column is NULL too.
                Some(("Option", _)) => MirExpr::Let {
                    name: got.clone(),
                    ty: MirType::Ptr,
                    value: Box::new(column),
                    body: Box::new(MirExpr::If {
                        cond: Box::new(Self::call_named(
                            "mesh_result_is_ok",
                            vec![MirType::Ptr],
                            vec![MirExpr::Var(got, MirType::Ptr)],
                            MirType::Int,
                        )),
                        then_body: Box::new(decode),
                        else_body: Box::new(Self::json_ok(self.option_variant(ty, None))),
                        ty: MirType::Ptr,
                    }),
                },
                _ => Self::json_then(&got, column, decode),
            };
            let bound = self.json_bind(&format!("__f_{i}"), &parsed, ty, body);
            body = Self::json_then(&parsed, value, bound);
        }
        self.push_helper_fn(
            &mangled,
            vec![("row".to_string(), MirType::Ptr)],
            MirType::Ptr,
            body,
        );
    }

    /// The column text in `text` parsed as a `ty` (an `Int`, `Float`,
    /// `Bool`, `String`, or an `Option` of one: the type checker allows no
    /// other field in a row), as a `*mut MeshResult`. An empty text is an
    /// `Option`'s `None`.
    fn row_decode(&mut self, text: &str, ty: &Ty) -> MirExpr {
        let text_var = MirExpr::Var(text.to_string(), MirType::String);
        let parse = match ty_head(ty) {
            Some(("Int", _)) => "mesh_row_parse_int",
            Some(("Float", _)) => "mesh_row_parse_float",
            Some(("Bool", _)) => "mesh_row_parse_bool",
            Some(("Option", [inner])) => {
                let decoded = self.json_fresh("decoded");
                let some = self.option_variant(ty, Some(self.json_payload(&decoded, inner)));
                let present =
                    Self::json_then(&decoded, self.row_decode(text, inner), Self::json_ok(some));
                let length = Self::call_named(
                    "mesh_string_length",
                    vec![MirType::String],
                    vec![text_var],
                    MirType::Int,
                );
                return MirExpr::If {
                    cond: Box::new(MirExpr::BinOp {
                        op: BinOp::Eq,
                        lhs: Box::new(length),
                        rhs: Box::new(MirExpr::IntLit(0, MirType::Int)),
                        ty: MirType::Bool,
                    }),
                    then_body: Box::new(Self::json_ok(self.option_variant(ty, None))),
                    else_body: Box::new(present),
                    ty: MirType::Ptr,
                };
            }
            _ => return Self::json_ok(text_var),
        };
        Self::json_call(parse, vec![text_var])
    }

    /// `Some(value)`, or `None` without one, of the `Option` type `ty`.
    fn option_variant(&self, ty: &Ty, value: Option<MirExpr>) -> MirExpr {
        let option = self.binding_type(ty);
        let type_name = mir_type_to_impl_name(&option);
        let (variant, fields) = match value {
            Some(value) => ("Some", vec![value]),
            None => ("None", vec![]),
        };
        MirExpr::ConstructVariant {
            type_name,
            variant: variant.to_string(),
            fields,
            ty: option,
        }
    }

    /// Generate Schema metadata functions for a struct with `deriving(Schema)`.
    ///
    /// Generates synthetic MIR functions:
    /// - `{Name}____table__()` -> String (lowercased, pluralized struct name or custom)
    /// - `{Name}____fields__()` -> List<String> (field name strings)
    /// - `{Name}____primary_key__()` -> String (default: "id" or custom)
    /// - `{Name}____relationships__()` -> List<String> (encoded as "kind:name:target")
    /// - `{Name}____field_types__()` -> List<String> (encoded as "field:SQL_TYPE")
    /// - `{Name}____relationship_meta__()` -> List<String> (encoded as
    ///   "kind:name:target:fk:target_table:key")
    /// - `{Name}____{field}_col__()` -> String (per-field column accessor)
    fn generate_schema_metadata(
        &mut self,
        name: &str,
        fields: &[(String, MirType)],
        relationships: &[RelationshipDecl],
        schema: &mesh_typeck::SchemaInfo,
    ) {
        // ── __table__() ──────────────────────────────────────────────
        let table_name = schema.table.clone();
        let table_fn_name = format!("{}____table__", name);
        self.functions.push(MirFunction {
            name: table_fn_name.clone(),
            params: vec![],
            return_type: MirType::String,
            body: MirExpr::StringLit(table_name, MirType::String),
            is_closure_fn: false,
            captures: vec![],
            has_tail_calls: false,
        });
        self.known_functions.insert(
            table_fn_name,
            MirType::FnPtr(vec![], Box::new(MirType::String)),
        );

        // ── __fields__() ─────────────────────────────────────────────
        // Returns a List<String> of field names.
        let field_elements: Vec<MirExpr> = fields
            .iter()
            .map(|(fname, _)| MirExpr::StringLit(fname.clone(), MirType::String))
            .collect();
        let fields_fn_name = format!("{}____fields__", name);
        self.functions.push(MirFunction {
            name: fields_fn_name.clone(),
            params: vec![],
            return_type: MirType::Ptr, // List<String> is Ptr at runtime
            body: MirExpr::ListLit {
                elements: field_elements,
                ty: MirType::Ptr,
            },
            is_closure_fn: false,
            captures: vec![],
            has_tail_calls: false,
        });
        self.known_functions.insert(
            fields_fn_name,
            MirType::FnPtr(vec![], Box::new(MirType::Ptr)),
        );

        // ── __primary_key__() ────────────────────────────────────────
        let pk_value = schema.primary_key.clone();
        let pk_fn_name = format!("{}____primary_key__", name);
        self.functions.push(MirFunction {
            name: pk_fn_name.clone(),
            params: vec![],
            return_type: MirType::String,
            body: MirExpr::StringLit(pk_value, MirType::String),
            is_closure_fn: false,
            captures: vec![],
            has_tail_calls: false,
        });
        self.known_functions.insert(
            pk_fn_name,
            MirType::FnPtr(vec![], Box::new(MirType::String)),
        );

        // ── __relationships__() ──────────────────────────────────────
        // Returns a List<String> where each string is "kind:name:target".
        let rel_elements: Vec<MirExpr> = relationships
            .iter()
            .filter_map(|rel| {
                let kind = rel.kind_text()?;
                let assoc = rel.assoc_name()?;
                let target = rel.target_type()?;
                Some(MirExpr::StringLit(
                    format!("{}:{}:{}", kind, assoc, target),
                    MirType::String,
                ))
            })
            .collect();
        let rels_fn_name = format!("{}____relationships__", name);
        self.functions.push(MirFunction {
            name: rels_fn_name.clone(),
            params: vec![],
            return_type: MirType::Ptr, // List<String> is Ptr at runtime
            body: MirExpr::ListLit {
                elements: rel_elements,
                ty: MirType::Ptr,
            },
            is_closure_fn: false,
            captures: vec![],
            has_tail_calls: false,
        });
        self.known_functions
            .insert(rels_fn_name, MirType::FnPtr(vec![], Box::new(MirType::Ptr)));

        // ── __field_types__() ────────────────────────────────────────
        // Returns List<String> where each entry is "field_name:SQL_TYPE".
        let field_type_elements: Vec<MirExpr> = fields
            .iter()
            .map(|(fname, fty)| {
                let sql_type = mir_type_to_sql_type(fty);
                MirExpr::StringLit(format!("{}:{}", fname, sql_type), MirType::String)
            })
            .collect();
        let ft_fn_name = format!("{}____field_types__", name);
        self.functions.push(MirFunction {
            name: ft_fn_name.clone(),
            params: vec![],
            return_type: MirType::Ptr,
            body: MirExpr::ListLit {
                elements: field_type_elements,
                ty: MirType::Ptr,
            },
            is_closure_fn: false,
            captures: vec![],
            has_tail_calls: false,
        });
        self.known_functions
            .insert(ft_fn_name, MirType::FnPtr(vec![], Box::new(MirType::Ptr)));

        // ── __relationship_meta__() ──────────────────────────────────
        // Returns List<String> where each string is
        // "kind:name:target:fk:target_table:key:owner", `key` being the primary key
        // the foreign key refers to: the owner's for has_many and has_one, the
        // target's for belongs_to. The target's table and key are what its own
        // deriving(Schema) declares, wherever it is declared. `owner` is this
        // struct: Repo.preload reads a nested path's next association from
        // the struct the previous one loads, where two may share a name.
        let meta_elements: Vec<MirExpr> = relationships
            .iter()
            .filter_map(|rel| {
                let kind = rel.kind_text()?;
                let assoc = rel.assoc_name()?;
                let target = rel.target_type()?;
                let target_schema = self
                    .registry
                    .struct_defs
                    .get(&target)
                    .and_then(|info| info.schema.as_ref());
                let target_table = target_schema.map_or_else(
                    || mesh_typeck::default_schema_table(&target),
                    |target| target.table.clone(),
                );

                // Infer foreign key by convention:
                // - belongs_to :user, User -> fk is "user_id" (assoc_name + "_id")
                // - has_many :posts, Post on User -> fk is "user_id" (owner_lowercase + "_id")
                // - has_one :profile, Profile on User -> fk is "user_id" (owner_lowercase + "_id")
                let (fk, key) = match kind.as_str() {
                    "belongs_to" => (
                        format!("{}_id", assoc),
                        target_schema.map_or("id", |target| target.primary_key.as_str()),
                    ),
                    // The parser's other relationships: has_many and has_one.
                    _ => (
                        format!("{}_id", name.to_lowercase()),
                        schema.primary_key.as_str(),
                    ),
                };

                Some(MirExpr::StringLit(
                    format!("{kind}:{assoc}:{target}:{fk}:{target_table}:{key}:{name}"),
                    MirType::String,
                ))
            })
            .collect();

        let meta_fn_name = format!("{}____relationship_meta__", name);
        self.functions.push(MirFunction {
            name: meta_fn_name.clone(),
            params: vec![],
            return_type: MirType::Ptr,
            body: MirExpr::ListLit {
                elements: meta_elements,
                ty: MirType::Ptr,
            },
            is_closure_fn: false,
            captures: vec![],
            has_tail_calls: false,
        });
        self.known_functions
            .insert(meta_fn_name, MirType::FnPtr(vec![], Box::new(MirType::Ptr)));

        // ── Per-field column accessors ───────────────────────────────
        // User.__name_col__() -> "name"
        for (fname, _fty) in fields {
            let col_fn_name = format!("{}____{}_col__", name, fname);
            self.functions.push(MirFunction {
                name: col_fn_name.clone(),
                params: vec![],
                return_type: MirType::String,
                body: MirExpr::StringLit(fname.clone(), MirType::String),
                is_closure_fn: false,
                captures: vec![],
                has_tail_calls: false,
            });
            self.known_functions.insert(
                col_fn_name,
                MirType::FnPtr(vec![], Box::new(MirType::String)),
            );
        }
    }

    // ── Json derivation by source types ──────────────────────────────
    //
    // An encoder turns a value into a `*mut MeshJson`. A decoder turns a
    // `*mut MeshJson` into a `*mut MeshResult` whose Ok payload holds the
    // value the way a collection slot does (`__mesh_uniform_encode`), which
    // is what `mesh_json_to_list` stores and `json_payload` reads back.

    fn json_call(f: &str, args: Vec<MirExpr>) -> MirExpr {
        let params = args.iter().map(|arg| arg.ty().clone()).collect();
        Self::call_named(f, params, args, MirType::Ptr)
    }

    /// `decoded`, a decoding's Result, whose error names `step` (`.field`,
    /// `[index]`) at the front of the path it failed at.
    fn json_error_at(decoded: MirExpr, step: &str) -> MirExpr {
        Self::json_call(
            "mesh_json_error_at",
            vec![
                decoded,
                MirExpr::StringLit(step.to_string(), MirType::String),
            ],
        )
    }

    /// A name for a binding in generated Json code, unique in its function.
    fn json_fresh(&mut self, base: &str) -> String {
        self.json_counter += 1;
        format!("__json_{base}_{}", self.json_counter)
    }

    /// `value` as a raw slot word typed `ty`: `__mesh_uniform_decode` of
    /// `__mesh_uniform_encode`, the way a collection slot converts.
    fn slot_as(value: MirExpr, ty: MirType) -> MirExpr {
        let slot = Self::call_named(
            "__mesh_uniform_encode",
            vec![value.ty().clone()],
            vec![value],
            MirType::Int,
        );
        Self::call_named("__mesh_uniform_decode", vec![MirType::Int], vec![slot], ty)
    }

    /// `Ok(value)` as a `*mut MeshResult`.
    fn json_ok(value: MirExpr) -> MirExpr {
        Self::json_call(
            "mesh_alloc_result",
            vec![
                MirExpr::IntLit(0, MirType::Int),
                Self::slot_as(value, MirType::Ptr),
            ],
        )
    }

    /// `Err(message)` as a `*mut MeshResult`.
    fn json_err(message: String) -> MirExpr {
        Self::json_call(
            "mesh_alloc_result",
            vec![
                MirExpr::IntLit(1, MirType::Int),
                MirExpr::StringLit(message, MirType::String),
            ],
        )
    }

    /// The value of source type `ty` in the Ok result `res`.
    fn json_payload(&self, res: &str, ty: &Ty) -> MirExpr {
        let unwrap = Self::json_call(
            "mesh_result_unwrap",
            vec![MirExpr::Var(res.to_string(), MirType::Ptr)],
        );
        Self::slot_as(unwrap, self.binding_type(ty))
    }

    /// `let res = result; if res is Ok then body else res`.
    fn json_then(res: &str, result: MirExpr, body: MirExpr) -> MirExpr {
        let var = || MirExpr::Var(res.to_string(), MirType::Ptr);
        MirExpr::Let {
            name: res.to_string(),
            ty: MirType::Ptr,
            value: Box::new(result),
            body: Box::new(MirExpr::If {
                cond: Box::new(Self::call_named(
                    "mesh_result_is_ok",
                    vec![MirType::Ptr],
                    vec![var()],
                    MirType::Int,
                )),
                then_body: Box::new(body),
                else_body: Box::new(var()),
                ty: MirType::Ptr,
            }),
        }
    }

    /// `let name = <value of type ty in the Ok result res>; body`.
    fn json_bind(&self, name: &str, res: &str, ty: &Ty, body: MirExpr) -> MirExpr {
        MirExpr::Let {
            name: name.to_string(),
            ty: self.binding_type(ty),
            value: Box::new(self.json_payload(res, ty)),
            body: Box::new(body),
        }
    }

    /// `value`, of source type `ty`, as a `*mut MeshJson`. A value JSON
    /// cannot hold (a function) encodes as `null`.
    fn json_encode_expr(&mut self, value: MirExpr, ty: &Ty) -> MirExpr {
        let null = || Self::json_call("mesh_json_null", vec![]);
        if let Ty::Tuple(elems) = ty {
            if elems.is_empty() {
                return MirExpr::Block(vec![value, null()], MirType::Ptr);
            }
            let f = self.json_tuple_encode_fn(elems);
            return Self::json_call(&f, vec![value]);
        }
        let Some((name, args)) = ty_head(ty) else {
            return MirExpr::Block(vec![value, null()], MirType::Ptr);
        };
        let arg = |i: usize| args.get(i).cloned().unwrap_or_else(Ty::int);
        match name {
            "Int" => Self::json_call("mesh_json_from_int", vec![value]),
            "Float" => Self::json_call("mesh_json_from_float", vec![value]),
            "Bool" => Self::json_call("mesh_json_from_bool", vec![value]),
            "String" => Self::json_call("mesh_json_from_string", vec![value]),
            "Unit" => MirExpr::Block(vec![value, null()], MirType::Ptr),
            "List" => {
                let cb = self.json_encode_callback(&arg(0));
                Self::json_call("mesh_json_from_list", vec![value, cb])
            }
            "Map" => {
                let cb = self.json_encode_callback(&arg(1));
                Self::json_call("mesh_json_from_map", vec![value, cb])
            }
            "Option" => {
                let inner = arg(0);
                let option = mir_type_to_impl_name(&self.binding_type(ty));
                let var = self.json_fresh("some");
                let inner_mir = self.binding_type(&inner);
                let some =
                    self.json_encode_expr(MirExpr::Var(var.clone(), inner_mir.clone()), &inner);
                MirExpr::Match {
                    scrutinee: Box::new(value),
                    arms: vec![
                        MirMatchArm {
                            pattern: MirPattern::Constructor {
                                type_name: option.clone(),
                                variant: "Some".to_string(),
                                fields: vec![MirPattern::Var(var.clone(), inner_mir.clone())],
                                bindings: vec![(var, inner_mir)],
                            },
                            guard: None,
                            body: some,
                        },
                        MirMatchArm {
                            pattern: MirPattern::Wildcard,
                            guard: None,
                            body: null(),
                        },
                    ],
                    ty: MirType::Ptr,
                }
            }
            _ => {
                self.ensure_instantiation_traits(ty);
                let f = format!(
                    "ToJson__to_json__{}",
                    self.instantiation_helper_name(name, args)
                );
                if self.known_functions.contains_key(&f)
                    || (args.is_empty() && self.trait_registry.has_impl("ToJson", ty))
                {
                    Self::json_call(&f, vec![value])
                } else {
                    MirExpr::Block(vec![value, null()], MirType::Ptr)
                }
            }
        }
    }

    /// Decode the `*mut MeshJson` `json` as a value of source type `ty`, as
    /// a `*mut MeshResult` (see the section comment).
    fn json_decode_expr(&mut self, json: MirExpr, ty: &Ty) -> MirExpr {
        let fail = |json: MirExpr| {
            MirExpr::Block(
                vec![
                    json,
                    Self::json_err(format!("cannot decode {ty} from JSON")),
                ],
                MirType::Ptr,
            )
        };
        if let Ty::Tuple(elems) = ty {
            if elems.is_empty() {
                return MirExpr::Block(vec![json, Self::json_ok(MirExpr::Unit)], MirType::Ptr);
            }
            let f = self.json_tuple_decode_fn(elems);
            return Self::json_call(&f, vec![json]);
        }
        let Some((name, args)) = ty_head(ty) else {
            return fail(json);
        };
        let arg = |i: usize| args.get(i).cloned().unwrap_or_else(Ty::int);
        match name {
            "Int" | "Float" | "Bool" | "String" => {
                let f = format!("mesh_json_as_{}", name.to_lowercase());
                Self::json_call(&f, vec![json])
            }
            "List" => {
                let cb = self.json_decode_callback(&arg(0));
                Self::json_call("mesh_json_to_list", vec![json, cb])
            }
            "Map" => {
                let cb = self.json_decode_callback(&arg(1));
                Self::json_call("mesh_json_to_map", vec![json, cb])
            }
            "Option" => {
                // null is None; anything else is the inner value, as Some.
                let inner = arg(0);
                let option_ty = self.binding_type(ty);
                let option = mir_type_to_impl_name(&option_ty);
                let (j, res, val) = (
                    self.json_fresh("opt"),
                    self.json_fresh("res"),
                    self.json_fresh("val"),
                );
                let inner_mir = self.binding_type(&inner);
                let variant = |variant: &str, fields: Vec<MirExpr>| MirExpr::ConstructVariant {
                    type_name: option.clone(),
                    variant: variant.to_string(),
                    fields,
                    ty: option_ty.clone(),
                };
                let decoded = self.json_decode_expr(MirExpr::Var(j.clone(), MirType::Ptr), &inner);
                let some = Self::json_then(
                    &res,
                    decoded,
                    self.json_bind(
                        &val,
                        &res,
                        &inner,
                        Self::json_ok(variant("Some", vec![MirExpr::Var(val.clone(), inner_mir)])),
                    ),
                );
                MirExpr::Let {
                    name: j.clone(),
                    ty: MirType::Ptr,
                    value: Box::new(json),
                    body: Box::new(MirExpr::If {
                        cond: Box::new(Self::call_named(
                            "mesh_json_is_null",
                            vec![MirType::Ptr],
                            vec![MirExpr::Var(j, MirType::Ptr)],
                            MirType::Bool,
                        )),
                        then_body: Box::new(Self::json_ok(variant("None", vec![]))),
                        else_body: Box::new(some),
                        ty: MirType::Ptr,
                    }),
                }
            }
            _ => {
                self.ensure_instantiation_traits(ty);
                let f = format!(
                    "FromJson__from_json__{}",
                    self.instantiation_helper_name(name, args)
                );
                if self.known_functions.contains_key(&f)
                    || (args.is_empty() && self.trait_registry.has_impl("FromJson", ty))
                {
                    Self::json_call(&f, vec![json])
                } else {
                    fail(json)
                }
            }
        }
    }

    /// The `fn(slot) -> *mut MeshJson` callback `mesh_json_from_list` and
    /// `mesh_json_from_map` call per element of type `elem`.
    fn json_encode_callback(&mut self, elem: &Ty) -> MirExpr {
        let fn_ty = MirType::FnPtr(vec![MirType::Int], Box::new(MirType::Ptr));
        let name = match ty_head(elem) {
            Some(("Int", _)) => "mesh_json_from_int".to_string(),
            Some(("String", _)) => "mesh_json_from_string".to_string(),
            _ => {
                let name = format!(
                    "__json_encode_slot_{}",
                    Self::ty_specialization_component(elem)
                );
                if !self.known_functions.contains_key(&name) {
                    self.known_functions.insert(name.clone(), fn_ty.clone());
                    let value = self.decode_slot("__slot", elem);
                    let body = self.json_encode_expr(value, elem);
                    self.push_helper_fn(
                        &name,
                        vec![("__slot".to_string(), MirType::Int)],
                        MirType::Ptr,
                        body,
                    );
                }
                name
            }
        };
        MirExpr::Var(name, fn_ty)
    }

    /// The `fn(*mut MeshJson) -> *mut MeshResult` callback
    /// `mesh_json_to_list` and `mesh_json_to_map` call per element of type
    /// `elem`; its Ok payload is the element's slot.
    fn json_decode_callback(&mut self, elem: &Ty) -> MirExpr {
        let fn_ty = MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr));
        let name = match ty_head(elem) {
            Some((scalar @ ("Int" | "Float" | "Bool" | "String"), _)) => {
                format!("mesh_json_as_{}", scalar.to_lowercase())
            }
            _ => {
                let name = format!(
                    "__json_decode_slot_{}",
                    Self::ty_specialization_component(elem)
                );
                if !self.known_functions.contains_key(&name) {
                    self.known_functions.insert(name.clone(), fn_ty.clone());
                    let body = self
                        .json_decode_expr(MirExpr::Var("__json".to_string(), MirType::Ptr), elem);
                    self.push_helper_fn(
                        &name,
                        vec![("__json".to_string(), MirType::Ptr)],
                        MirType::Ptr,
                        body,
                    );
                }
                name
            }
        };
        MirExpr::Var(name, fn_ty)
    }

    /// `fn(t: Ptr) -> *mut MeshJson` encoding a tuple as an array.
    fn json_tuple_encode_fn(&mut self, elems: &[Ty]) -> String {
        let name = format!(
            "__json_encode_{}",
            Self::ty_specialization_component(&Ty::Tuple(elems.to_vec()))
        );
        if self.known_functions.contains_key(&name) {
            return name;
        }
        self.known_functions.insert(
            name.clone(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        let tys: Vec<MirType> = elems.iter().map(|e| self.binding_type(e)).collect();
        let mut array = Self::json_call("mesh_json_array_new", vec![]);
        for (i, (elem, t)) in elems.iter().zip(&tys).enumerate() {
            let value = self.json_encode_expr(MirExpr::Var(format!("__e_{i}"), t.clone()), elem);
            array = Self::json_call("mesh_json_array_push", vec![array, value]);
        }
        let body = MirExpr::Match {
            scrutinee: Box::new(MirExpr::Var("__t".to_string(), MirType::Ptr)),
            arms: vec![MirMatchArm {
                pattern: MirPattern::Tuple(
                    tys.iter()
                        .enumerate()
                        .map(|(i, t)| MirPattern::Var(format!("__e_{i}"), t.clone()))
                        .collect(),
                ),
                guard: None,
                body: array,
            }],
            ty: MirType::Ptr,
        };
        self.push_helper_fn(
            &name,
            vec![("__t".to_string(), MirType::Ptr)],
            MirType::Ptr,
            body,
        );
        name
    }

    /// `fn(*mut MeshJson) -> *mut MeshResult` decoding an array as a tuple.
    fn json_tuple_decode_fn(&mut self, elems: &[Ty]) -> String {
        let name = format!(
            "__json_decode_{}",
            Self::ty_specialization_component(&Ty::Tuple(elems.to_vec()))
        );
        if self.known_functions.contains_key(&name) {
            return name;
        }
        self.known_functions.insert(
            name.clone(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        let values: Vec<MirExpr> = elems
            .iter()
            .enumerate()
            .map(|(i, e)| MirExpr::Var(format!("__v_{i}"), self.binding_type(e)))
            .collect();
        let tuple = Self::call_named(
            "__mesh_make_tuple",
            vec![MirType::Int; values.len()],
            values,
            MirType::Ptr,
        );
        let array = MirExpr::Var("__json".to_string(), MirType::Ptr);
        let body = self.json_decode_indexed(&array, elems, "__v_", Self::json_ok(tuple));
        self.push_helper_fn(
            &name,
            vec![("__json".to_string(), MirType::Ptr)],
            MirType::Ptr,
            body,
        );
        name
    }

    /// Decode element `i` of the JSON array `array` as `elems[i]` into
    /// `{prefix}{i}`, for every element, then `body`.
    fn json_decode_indexed(
        &mut self,
        array: &MirExpr,
        elems: &[Ty],
        prefix: &str,
        body: MirExpr,
    ) -> MirExpr {
        let mut body = body;
        for (i, elem) in elems.iter().enumerate().rev() {
            let (got, dec) = (self.json_fresh("item"), self.json_fresh("dec"));
            let item = Self::json_call(
                "mesh_json_array_get",
                vec![array.clone(), MirExpr::IntLit(i as i64, MirType::Int)],
            );
            let unwrapped = Self::json_call(
                "mesh_result_unwrap",
                vec![MirExpr::Var(got.clone(), MirType::Ptr)],
            );
            let decoded =
                Self::json_error_at(self.json_decode_expr(unwrapped, elem), &format!("[{i}]"));
            let bound = self.json_bind(&format!("{prefix}{i}"), &dec, elem, body);
            body = Self::json_then(&got, item, Self::json_then(&dec, decoded, bound));
        }
        body
    }

    /// Derived `to_json` for the struct `name`, named for `helper`: an
    /// object with a member per field, each encoded by its source type.
    fn generate_to_json_struct_typed(&mut self, name: &str, helper: &str, fields: &[(String, Ty)]) {
        let mangled = format!("ToJson__to_json__{helper}");
        let struct_ty = MirType::Struct(name.to_string());
        self.known_functions.insert(
            mangled.clone(),
            MirType::FnPtr(vec![struct_ty.clone()], Box::new(MirType::Ptr)),
        );
        let mut body = Self::json_call("mesh_json_object_new", vec![]);
        for (field, ty) in fields {
            let access = MirExpr::FieldAccess {
                object: Box::new(MirExpr::Var("self".to_string(), struct_ty.clone())),
                field: field.clone(),
                ty: self.binding_type(ty),
            };
            let value = self.json_encode_expr(access, ty);
            let key = MirExpr::StringLit(field.clone(), MirType::String);
            body = Self::json_call("mesh_json_object_put", vec![body, key, value]);
        }
        self.push_helper_fn(
            &mangled,
            vec![("self".to_string(), struct_ty)],
            MirType::Ptr,
            body,
        );
    }

    /// Derived `from_json` for the struct `name`, named for `helper`: each
    /// field decoded by its source type from the member of its name.
    fn generate_from_json_struct_typed(
        &mut self,
        name: &str,
        helper: &str,
        fields: &[(String, Ty)],
    ) {
        let mangled = format!("FromJson__from_json__{helper}");
        self.known_functions.insert(
            mangled.clone(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        let value = |i: usize, ty: &Ty, lowerer: &Self| {
            MirExpr::Var(format!("__f_{i}"), lowerer.binding_type(ty))
        };
        let struct_lit = MirExpr::StructLit {
            name: name.to_string(),
            fields: fields
                .iter()
                .enumerate()
                .map(|(i, (field, ty))| (field.clone(), value(i, ty, self)))
                .collect(),
            ty: MirType::Struct(name.to_string()),
        };
        let json = MirExpr::Var("json".to_string(), MirType::Ptr);
        let mut body = Self::json_ok(struct_lit);
        for (i, (field, ty)) in fields.iter().enumerate().rev() {
            let (got, dec) = (self.json_fresh("member"), self.json_fresh("dec"));
            let member = Self::json_call(
                "mesh_json_object_get",
                vec![
                    json.clone(),
                    MirExpr::StringLit(field.clone(), MirType::String),
                ],
            );
            let unwrapped = Self::json_call(
                "mesh_result_unwrap",
                vec![MirExpr::Var(got.clone(), MirType::Ptr)],
            );
            let decoded =
                Self::json_error_at(self.json_decode_expr(unwrapped, ty), &format!(".{field}"));
            let bound = self.json_bind(&format!("__f_{i}"), &dec, ty, body);
            body = Self::json_then(&got, member, Self::json_then(&dec, decoded, bound));
        }
        self.push_helper_fn(
            &mangled,
            vec![("json".to_string(), MirType::Ptr)],
            MirType::Ptr,
            body,
        );
    }

    /// Derived `to_json` for the sum type `name`, named for `helper`:
    /// `{"tag": "Variant", "fields": [...]}`, each field encoded by its type.
    fn generate_to_json_sum_typed(
        &mut self,
        name: &str,
        helper: &str,
        variants: &[(String, Vec<Ty>)],
    ) {
        let mangled = format!("ToJson__to_json__{helper}");
        let sum_ty = MirType::SumType(name.to_string());
        self.known_functions.insert(
            mangled.clone(),
            MirType::FnPtr(vec![sum_ty.clone()], Box::new(MirType::Ptr)),
        );
        let arms: Vec<MirMatchArm> = variants
            .iter()
            .map(|(variant, fields)| {
                let bindings: Vec<(String, MirType)> = fields
                    .iter()
                    .enumerate()
                    .map(|(i, f)| (format!("__tj_{variant}_{i}"), self.binding_type(f)))
                    .collect();
                let mut array = Self::json_call("mesh_json_array_new", vec![]);
                for (f, (var, t)) in fields.iter().zip(&bindings) {
                    let value = self.json_encode_expr(MirExpr::Var(var.clone(), t.clone()), f);
                    array = Self::json_call("mesh_json_array_push", vec![array, value]);
                }
                let tag = Self::json_call(
                    "mesh_json_from_string",
                    vec![MirExpr::StringLit(variant.clone(), MirType::String)],
                );
                let key = |k: &str| MirExpr::StringLit(k.to_string(), MirType::String);
                let object = Self::json_call("mesh_json_object_new", vec![]);
                let object = Self::json_call("mesh_json_object_put", vec![object, key("tag"), tag]);
                let object =
                    Self::json_call("mesh_json_object_put", vec![object, key("fields"), array]);
                MirMatchArm {
                    pattern: MirPattern::Constructor {
                        type_name: name.to_string(),
                        variant: variant.clone(),
                        fields: bindings
                            .iter()
                            .map(|(n, t)| MirPattern::Var(n.clone(), t.clone()))
                            .collect(),
                        bindings,
                    },
                    guard: None,
                    body: object,
                }
            })
            .collect();
        let body = if arms.is_empty() {
            Self::json_call("mesh_json_object_new", vec![])
        } else {
            MirExpr::Match {
                scrutinee: Box::new(MirExpr::Var("self".to_string(), sum_ty.clone())),
                arms,
                ty: MirType::Ptr,
            }
        };
        self.push_helper_fn(
            &mangled,
            vec![("self".to_string(), sum_ty)],
            MirType::Ptr,
            body,
        );
    }

    /// Derived `from_json` for the sum type `name`, named for `helper`: the
    /// variant named by `"tag"`, its fields decoded from `"fields"`.
    fn generate_from_json_sum_typed(
        &mut self,
        name: &str,
        helper: &str,
        variants: &[(String, Vec<Ty>)],
    ) {
        let mangled = format!("FromJson__from_json__{helper}");
        self.known_functions.insert(
            mangled.clone(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        let sum_ty = MirType::SumType(name.to_string());
        let json = MirExpr::Var("json".to_string(), MirType::Ptr);
        let key = |k: &str| MirExpr::StringLit(k.to_string(), MirType::String);
        let mut dispatch = Self::json_err(format!("unknown variant for {name}"));
        for (variant, fields) in variants.iter().rev() {
            let values = fields
                .iter()
                .enumerate()
                .map(|(i, f)| MirExpr::Var(format!("__fv_{variant}_{i}"), self.binding_type(f)))
                .collect();
            let constructed = Self::json_ok(MirExpr::ConstructVariant {
                type_name: name.to_string(),
                variant: variant.clone(),
                fields: values,
                ty: sum_ty.clone(),
            });
            let decode = if fields.is_empty() {
                constructed
            } else {
                let got = self.json_fresh("fields");
                let array = self.json_fresh("array");
                let array_var = MirExpr::Var(array.clone(), MirType::Ptr);
                let prefix = format!("__fv_{variant}_");
                let each = Self::json_error_at(
                    self.json_decode_indexed(&array_var, fields, &prefix, constructed),
                    ".fields",
                );
                let member =
                    Self::json_call("mesh_json_object_get", vec![json.clone(), key("fields")]);
                let unwrapped = Self::json_call(
                    "mesh_result_unwrap",
                    vec![MirExpr::Var(got.clone(), MirType::Ptr)],
                );
                Self::json_then(
                    &got,
                    member,
                    MirExpr::Let {
                        name: array,
                        ty: MirType::Ptr,
                        value: Box::new(unwrapped),
                        body: Box::new(each),
                    },
                )
            };
            dispatch = MirExpr::If {
                cond: Box::new(Self::call_named(
                    "mesh_string_eq",
                    vec![MirType::String, MirType::String],
                    vec![
                        MirExpr::Var("__tag".to_string(), MirType::String),
                        MirExpr::StringLit(variant.clone(), MirType::String),
                    ],
                    MirType::Bool,
                )),
                then_body: Box::new(decode),
                else_body: Box::new(dispatch),
                ty: MirType::Ptr,
            };
        }
        let (got, tag) = (self.json_fresh("tag"), self.json_fresh("tag_str"));
        let member = Self::json_call("mesh_json_object_get", vec![json, key("tag")]);
        let as_string = Self::json_call(
            "mesh_json_as_string",
            vec![Self::json_call(
                "mesh_result_unwrap",
                vec![MirExpr::Var(got.clone(), MirType::Ptr)],
            )],
        );
        let body = Self::json_then(
            &got,
            member,
            Self::json_then(
                &tag,
                as_string,
                self.json_bind("__tag", &tag, &Ty::string(), dispatch),
            ),
        );
        self.push_helper_fn(
            &mangled,
            vec![("json".to_string(), MirType::Ptr)],
            MirType::Ptr,
            body,
        );
    }

    /// Generate a wrapper `__json_decode__StructName` that chains
    /// mesh_json_parse + FromJson__from_json__StructName.
    /// This is what `StructName.from_json(str)` resolves to.
    /// Returns a *mut MeshResult (Ptr) -- the let-binding deref logic converts
    /// it to a SumType("Result") when bound to a typed variable.
    fn generate_from_json_string_wrapper(&mut self, name: &str) {
        let wrapper_name = format!("__json_decode__{}", name);
        let parse_ty = MirType::FnPtr(vec![MirType::String], Box::new(MirType::Ptr));
        let from_json_ty = MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr));
        let is_ok_ty = MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Int));
        let unwrap_ty = MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr));

        let str_var = MirExpr::Var("__input".to_string(), MirType::String);

        // mesh_json_parse(input) -> *mut MeshResult
        let parse_call = MirExpr::Call {
            func: Box::new(MirExpr::Var("mesh_json_parse".to_string(), parse_ty)),
            args: vec![str_var],
            ty: MirType::Ptr,
        };

        // If parse is Ok, call FromJson__from_json__(parsed_json)
        // Else, return the error result directly
        let from_json_call = MirExpr::Call {
            func: Box::new(MirExpr::Var(
                format!("FromJson__from_json__{}", name),
                from_json_ty,
            )),
            args: vec![MirExpr::Var("__parsed_json".to_string(), MirType::Ptr)],
            ty: MirType::Ptr,
        };

        let body = MirExpr::Let {
            name: "__parse_res".to_string(),
            ty: MirType::Ptr,
            value: Box::new(parse_call),
            body: Box::new(MirExpr::If {
                cond: Box::new(MirExpr::Call {
                    func: Box::new(MirExpr::Var("mesh_result_is_ok".to_string(), is_ok_ty)),
                    args: vec![MirExpr::Var("__parse_res".to_string(), MirType::Ptr)],
                    ty: MirType::Int,
                }),
                then_body: Box::new(MirExpr::Let {
                    name: "__parsed_json".to_string(),
                    ty: MirType::Ptr,
                    value: Box::new(MirExpr::Call {
                        func: Box::new(MirExpr::Var("mesh_result_unwrap".to_string(), unwrap_ty)),
                        args: vec![MirExpr::Var("__parse_res".to_string(), MirType::Ptr)],
                        ty: MirType::Ptr,
                    }),
                    body: Box::new(from_json_call),
                }),
                else_body: Box::new(MirExpr::Var("__parse_res".to_string(), MirType::Ptr)),
                ty: MirType::Ptr,
            }),
        };

        let func = MirFunction {
            name: wrapper_name.clone(),
            params: vec![("__input".to_string(), MirType::String)],
            return_type: MirType::Ptr,
            body,
            is_closure_fn: false,
            captures: vec![],
            has_tail_calls: false,
        };

        self.functions.push(func);
        self.known_functions.insert(
            wrapper_name,
            MirType::FnPtr(vec![MirType::String], Box::new(MirType::Ptr)),
        );
    }

    // ── Block lowering ───────────────────────────────────────────────

    fn lower_block(&mut self, block: &Block) -> MirExpr {
        enum Part {
            Binding {
                name: String,
                ty: MirType,
                value: MirExpr,
                resource_ty: Option<Ty>,
            },
            Destructure {
                pattern: MirPattern,
                value: MirExpr,
                resources: Vec<(String, Ty)>,
            },
            Expr(MirExpr),
        }

        let mut parts = Vec::new();
        for child in block.syntax().children() {
            // The type checker rejects any definition but a `let` in a
            // function (E0084).
            if let Some(let_) = LetBinding::cast(child.clone()) {
                let initializer = let_
                    .initializer()
                    .expect("the parser gives a `let` its value");
                let initializer_ty = self.get_ty(initializer.syntax().text_range()).cloned();
                // A polymorphic closure, or a generic function named
                // by the `let` (`let id = identity`, `let pop =
                // Queue.pop`), gets one compiled copy per concrete
                // type it is used at, bound here so its captures are
                // the values in scope at the `let`.
                let mut specialized_everywhere = false;
                let poly_value = match &initializer {
                    expr @ (Expr::ClosureExpr(_) | Expr::NameRef(_)) => Some(expr),
                    expr @ Expr::FieldAccess(fa) if matches!(fa.base(), Some(Expr::NameRef(_))) => {
                        Some(expr)
                    }
                    _ => None,
                };
                if let (Some(poly_value), Some(generic), Some(name)) = (
                    poly_value,
                    initializer_ty.as_ref(),
                    let_.name().and_then(|name| name.text()),
                ) {
                    if Self::ty_contains_var(generic) {
                        let (uses, all_concrete) = self.poly_closure_uses(block, &let_, &name);
                        specialized_everywhere = all_concrete;
                        for (use_ty, spec_name) in uses {
                            let value =
                                self.lower_closure_specialized(poly_value, generic, &use_ty);
                            let ty = value.ty().clone();
                            self.insert_var(spec_name.clone(), ty.clone());
                            self.poly_closure_specs
                                .entry(name.clone())
                                .or_default()
                                .push((use_ty, spec_name.clone()));
                            parts.push(Part::Binding {
                                name: spec_name,
                                ty,
                                value,
                                resource_ty: None,
                            });
                        }
                    }
                }
                // Every use has its own copy: the generic one (whose
                // operators may not know their operand types) is unused.
                let value = if specialized_everywhere {
                    MirExpr::Unit
                } else {
                    self.lower_expr(&initializer)
                };

                if let Some(pattern) = let_.pattern() {
                    let mut resources = self.resource_pattern_bindings(&pattern);
                    let discards_before = self.discarded_resources.len();
                    let pattern =
                        self.lower_pattern_with_expected(&pattern, initializer_ty.as_ref());
                    resources.extend(self.discarded_resources.split_off(discards_before));
                    parts.push(Part::Destructure {
                        pattern,
                        value,
                        resources,
                    });
                } else {
                    let name = let_
                        .name()
                        .and_then(|name| name.text())
                        .unwrap_or_else(|| "_".to_string());
                    let ty = value.ty().clone();
                    let resource_ty =
                        initializer_ty.filter(|ty| self.registry.is_resource_type(ty));
                    self.insert_var(name.clone(), ty.clone());
                    parts.push(Part::Binding {
                        name,
                        ty,
                        value,
                        resource_ty,
                    });
                }
                continue;
            }
            if let Some(expr) = Expr::cast(child) {
                parts.push(Part::Expr(self.lower_expr(&expr)));
            }
        }

        let mut result = None;
        for part in parts.into_iter().rev() {
            match part {
                Part::Binding {
                    name,
                    ty,
                    value,
                    resource_ty,
                } => {
                    let body = result.take().unwrap_or(MirExpr::Unit);
                    let body = match resource_ty {
                        Some(resource_ty) => self.wrap_resource_scope(body, &name, &resource_ty),
                        None => body,
                    };
                    result = Some(MirExpr::Let {
                        name,
                        ty,
                        value: Box::new(value),
                        body: Box::new(body),
                    });
                }
                Part::Destructure {
                    pattern,
                    value,
                    resources,
                } => {
                    let mut body = result.take().unwrap_or(MirExpr::Unit);
                    for (name, resource_ty) in resources.into_iter().rev() {
                        body = self.wrap_resource_scope(body, &name, &resource_ty);
                    }
                    let ty = effective_return_type(&body);
                    result = Some(MirExpr::Match {
                        scrutinee: Box::new(value),
                        arms: vec![MirMatchArm {
                            pattern,
                            guard: None,
                            body,
                        }],
                        ty,
                    });
                }
                Part::Expr(expression) => {
                    result = Some(match result.take() {
                        Some(tail) => {
                            let ty = effective_return_type(&tail);
                            MirExpr::Block(vec![expression, tail], ty)
                        }
                        None => expression,
                    });
                }
            }
        }

        result.unwrap_or(MirExpr::Unit)
    }

    // ── Expression lowering ──────────────────────────────────────────

    /// Job.async(f) / Job.map(list, f): f's result crosses back to the caller
    /// once the job actor is gone, so its shape goes along with the callback.
    /// `result` is what f returns; `range` is the whole call, whose own type
    /// (`Pid<T>`, `List<Result<T, String>>`) names T for piped calls as well.
    fn job_result_shape(&self, name: &str, result: &MirType, range: TextRange) -> Option<MsgShape> {
        fn first_arg(ty: &Ty) -> Option<&Ty> {
            ty_head(ty).and_then(|(_, args)| args.first())
        }
        let call_ty = self.get_ty(range);
        let result_ty = match name {
            "mesh_job_async" => call_ty.and_then(first_arg),
            "mesh_job_map" => call_ty.and_then(first_arg).and_then(first_arg),
            _ => return None,
        };
        self.slot_shape(result, result_ty)
    }

    /// The shape of a value of type `ty` held in a uniform slot
    /// (`__mesh_uniform_encode`) that crosses to another actor, or `None`
    /// when the slot holds plain bits. The representation `rep`, not the
    /// type, says whether the word is a reference at all: the runtime boxes
    /// a scalar and hands a reference on as it is, so a reference always
    /// gets a shape, `Shared` when nothing more is known (no type whose
    /// values are references has a scalar shape).
    fn slot_shape(&self, rep: &MirType, ty: Option<&Ty>) -> Option<MsgShape> {
        if matches!(
            rep,
            MirType::Int
                | MirType::Float
                | MirType::Bool
                | MirType::Unit
                | MirType::Never
                | MirType::Pid(_)
        ) {
            return None;
        }
        Some(ty.map_or(MsgShape::Shared, |ty| self.msg_shape(ty, &mut Vec::new())))
    }

    /// `Iter.next` hands back the element word as the `Some` payload, as
    /// `List.find` does; a scalar payload is read through a pointer, so it
    /// is boxed here.
    fn box_next_scalar(&self, expr: MirExpr, range: TextRange) -> MirExpr {
        let is_next = matches!(&expr, MirExpr::Call { func, .. }
            if matches!(func.as_ref(), MirExpr::Var(name, _) if name == "mesh_iter_generic_next"));
        let scalar = match self.get_ty(range).and_then(ty_head) {
            Some(("Option", [elem, ..])) => is_scalar_word(&resolve_type(elem, self.registry)),
            _ => false,
        };
        if is_next && scalar {
            boxed_scalar_option(expr)
        } else {
            expr
        }
    }

    fn adapt_uniform_callback_call(&mut self, expr: MirExpr, range: TextRange) -> MirExpr {
        let MirExpr::Call { func, mut args, ty } = expr else {
            return expr;
        };
        let MirExpr::Var(name, _) = func.as_ref() else {
            return MirExpr::Call { func, args, ty };
        };
        let name = name.clone();
        let Some(callback_index) = uniform_callback_index(&name) else {
            return MirExpr::Call { func, args, ty };
        };
        let callback = args
            .get(callback_index)
            .cloned()
            .expect("the type checker gives a runtime function its callback");
        let callback = self.as_fn_item(callback);
        let callback_ty = callback.ty().clone();
        let (param_types, return_type) = callback_ty
            .function_parts()
            .map(|(params, ret)| (params.to_vec(), ret.clone()))
            .expect("the type checker gives a runtime function its callback");
        let is_closure = matches!(callback_ty, MirType::Closure(..));

        let adapter_name = self.generated_fn_name("uniform_callback");
        let raw_params = param_types
            .iter()
            .enumerate()
            .map(|(index, _)| (format!("__slot_{index}"), MirType::Int))
            .collect::<Vec<_>>();
        let decoded_args = param_types
            .iter()
            .enumerate()
            .map(|(index, param_ty)| MirExpr::Call {
                func: Box::new(MirExpr::Var(
                    "__mesh_uniform_decode".to_string(),
                    MirType::FnPtr(vec![MirType::Int], Box::new(param_ty.clone())),
                )),
                args: vec![MirExpr::Var(format!("__slot_{index}"), MirType::Int)],
                ty: param_ty.clone(),
            })
            .collect::<Vec<_>>();
        let callback_var = MirExpr::Var("__callback".to_string(), callback_ty.clone());
        let callback_call = if is_closure {
            MirExpr::ClosureCall {
                closure: Box::new(callback_var),
                args: decoded_args,
                ty: return_type.clone(),
            }
        } else {
            MirExpr::Call {
                func: Box::new(callback_var),
                args: decoded_args,
                ty: return_type.clone(),
            }
        };
        let body = MirExpr::Call {
            func: Box::new(MirExpr::Var(
                "__mesh_uniform_encode".to_string(),
                MirType::FnPtr(vec![return_type.clone()], Box::new(MirType::Int)),
            )),
            args: vec![callback_call],
            ty: MirType::Int,
        };
        let mut function_params = vec![("__env".to_string(), MirType::Ptr)];
        function_params.extend(raw_params.clone());
        self.functions.push(MirFunction {
            name: adapter_name.clone(),
            params: function_params,
            return_type: MirType::Int,
            body,
            is_closure_fn: true,
            captures: vec![("__callback".to_string(), callback_ty)],
            has_tail_calls: false,
        });
        let adapter = MirExpr::MakeClosure {
            fn_name: adapter_name,
            captures: vec![callback],
            ty: MirType::Closure(
                raw_params.into_iter().map(|(_, ty)| ty).collect(),
                Box::new(MirType::Int),
            ),
        };
        args[callback_index] = match self.job_result_shape(&name, &return_type, range) {
            Some(shape) => MirExpr::Shaped {
                value: Box::new(adapter),
                shape,
            },
            None => adapter,
        };
        let call = MirExpr::Call { func, args, ty };
        // `List.find` hands back the element word as the `Some` payload; a
        // scalar payload is read through a pointer, so it is boxed here.
        if matches!(name.as_str(), "mesh_list_find" | "mesh_iter_find")
            && param_types.first().is_some_and(is_scalar_word)
        {
            return boxed_scalar_option(call);
        }
        call
    }

    /// A function passed where a callback returning `()` is expected may return
    /// anything; the type checker recorded its range. It is wrapped in an
    /// adapter that calls it and returns `()`, so the callee never sees the
    /// result. The runtime calls some callbacks through a bare pointer typed
    /// for its own return value, and a large result would be written through a
    /// return slot it never passes.
    fn discard_callback_result(&mut self, expr: MirExpr, range: TextRange) -> MirExpr {
        if !self.discarded_callback_results.contains(&range) {
            return expr;
        }
        let callback = self.as_fn_item(expr);
        let callback_ty = callback.ty().clone();
        let (param_types, return_type) = callback_ty
            .function_parts()
            .map(|(params, ret)| (params.to_vec(), ret.clone()))
            .expect("the type checker recorded a function whose result is discarded");
        let is_closure = matches!(callback_ty, MirType::Closure(..));

        let adapter_name = self.generated_fn_name("discard_callback");
        let params = param_types
            .iter()
            .enumerate()
            .map(|(index, ty)| (format!("__arg_{index}"), ty.clone()))
            .collect::<Vec<_>>();
        let args = params
            .iter()
            .map(|(name, ty)| MirExpr::Var(name.clone(), ty.clone()))
            .collect();
        let callback_var = MirExpr::Var("__callback".to_string(), callback_ty.clone());
        let call = if is_closure {
            MirExpr::ClosureCall {
                closure: Box::new(callback_var),
                args,
                ty: return_type,
            }
        } else {
            MirExpr::Call {
                func: Box::new(callback_var),
                args,
                ty: return_type,
            }
        };
        let mut function_params = vec![("__env".to_string(), MirType::Ptr)];
        function_params.extend(params);
        self.functions.push(MirFunction {
            name: adapter_name.clone(),
            params: function_params,
            return_type: MirType::Unit,
            body: MirExpr::Block(vec![call, MirExpr::Unit], MirType::Unit),
            is_closure_fn: true,
            captures: vec![("__callback".to_string(), callback_ty)],
            has_tail_calls: false,
        });
        MirExpr::MakeClosure {
            fn_name: adapter_name,
            captures: vec![callback],
            ty: MirType::Closure(param_types, Box::new(MirType::Unit)),
        }
    }

    fn lower_expr(&mut self, expr: &Expr) -> MirExpr {
        let lowered = match expr {
            Expr::Literal(lit) => self.lower_literal(lit),
            Expr::NameRef(name_ref) => self.lower_name_ref(name_ref),
            Expr::BinaryExpr(bin) => self.lower_binary_expr(bin),
            Expr::UnaryExpr(un) => self.lower_unary_expr(un),
            Expr::CallExpr(call) => self.lower_call_expr(call),
            Expr::PipeExpr(pipe) => self.lower_pipe_expr(pipe),
            Expr::FieldAccess(fa) => self.lower_field_access(fa),
            Expr::IndexExpr(_) => unreachable!("the type checker rejects indexing (E0078)"),
            Expr::IfExpr(if_) => self.lower_if_expr(if_),
            Expr::CaseExpr(case) => self.lower_case_expr(case),
            Expr::ClosureExpr(closure) => self.lower_closure_expr(closure),
            Expr::Block(block) => self.lower_block(block),
            Expr::StringExpr(str_expr) => self.lower_string_expr(str_expr),
            Expr::ReturnExpr(ret) => self.lower_return_expr(ret),
            Expr::TupleExpr(tuple) => self.lower_tuple_expr(tuple),
            Expr::StructLiteral(sl) => self.lower_struct_literal(sl),
            Expr::MapLiteral(map_lit) => self.lower_map_literal(map_lit),
            Expr::ListLiteral(list_lit) => self.lower_list_literal(list_lit),
            // Actor expressions
            Expr::SpawnExpr(spawn) => self.lower_spawn_expr(spawn),
            Expr::SendExpr(send) => self.lower_send_expr(send),
            Expr::ReceiveExpr(recv) => self.lower_receive_expr(recv),
            Expr::SelfExpr(_) => MirExpr::ActorSelf {
                ty: self.resolve_range(expr.syntax().text_range()),
            },
            Expr::LinkExpr(link) => self.lower_link_expr(link),
            // Loop expressions
            Expr::WhileExpr(w) => self.lower_while_expr(w),
            Expr::BreakExpr(_) => MirExpr::Break,
            Expr::ContinueExpr(_) => MirExpr::Continue,
            Expr::ForInExpr(for_in) => self.lower_for_in_expr(for_in),
            // Try expression -- desugar to Match + Return (Phase 45)
            Expr::TryExpr(try_expr) => self.lower_try_expr(try_expr),
            // Atom literal -- lower to string constant at runtime
            Expr::AtomLiteral(atom) => {
                let name = atom.atom_text().unwrap_or_default();
                MirExpr::StringLit(name, MirType::String)
            }
            // Regex literal -- desugar to mesh_regex_from_literal(pattern, flags_bitmask).
            // Flags bitmask: i=1, m=2, s=4 (the lexer allows no other flag).
            Expr::RegexExpr(rx) => {
                let pattern = rx.pattern().unwrap_or_default();
                let flags_bits = rx
                    .flags()
                    .chars()
                    .filter_map(|flag| "ims".find(flag))
                    .fold(0i64, |bits, bit| bits | 1 << bit);
                let fn_ty =
                    MirType::FnPtr(vec![MirType::String, MirType::Int], Box::new(MirType::Ptr));
                MirExpr::Call {
                    func: Box::new(MirExpr::Var("mesh_regex_from_literal".to_string(), fn_ty)),
                    args: vec![
                        MirExpr::StringLit(pattern, MirType::String),
                        MirExpr::IntLit(flags_bits, MirType::Int),
                    ],
                    ty: MirType::Ptr,
                }
            }
            // Struct update expression: %{base | field: value, ...}
            Expr::StructUpdate(update) => self.lower_struct_update(update),
            // Slot pipe expression -- |N> desugaring (Phase 116, Plan 02)
            Expr::SlotPipeExpr(pipe) => self.lower_slot_pipe_expr(pipe),
            // Json object literal -- Phase 132-02 codegen
            Expr::JsonExpr(json_expr) => self.lower_json_expr(json_expr),
        };
        self.finish_lowered(lowered, expr.syntax().text_range())
    }

    /// What every expression's lowering ends with: `lowered`, the
    /// expression at `range`, with its runtime callbacks adapted, a scalar
    /// `Iter.next` payload boxed, a discarded callback result dropped, and a
    /// `Json` passed where a `String` is expected encoded.
    fn finish_lowered(&mut self, lowered: MirExpr, range: TextRange) -> MirExpr {
        let lowered = self.adapt_uniform_callback_call(lowered, range);
        let lowered = self.box_next_scalar(lowered, range);
        let lowered = self.discard_callback_result(lowered, range);
        self.json_text(lowered, range)
    }

    /// A `Json` argument passed where a `String` is expected, which the type
    /// checker recorded: its encoded text.
    fn json_text(&self, expr: MirExpr, range: TextRange) -> MirExpr {
        if !self.json_text_arguments.contains(&range) {
            return expr;
        }
        Self::call_named(
            "mesh_json_encode",
            vec![MirType::Ptr],
            vec![expr],
            MirType::String,
        )
    }

    // ── Literal lowering ─────────────────────────────────────────────

    /// A number, `true`, `false` or `nil`: the parser makes a literal of one
    /// of these tokens (a string is a string expression), and the type
    /// checker rejects a number out of range (E0072).
    fn lower_literal(&self, lit: &Literal) -> MirExpr {
        let token = lit.token().expect("a literal is its token");
        let text = token.text();
        match token.kind() {
            SyntaxKind::INT_LITERAL => {
                MirExpr::IntLit(parse_int_literal(text).unwrap_or(0), MirType::Int)
            }
            SyntaxKind::FLOAT_LITERAL => {
                MirExpr::FloatLit(parse_float_literal(text).unwrap_or(0.0), MirType::Float)
            }
            SyntaxKind::TRUE_KW => MirExpr::BoolLit(true, MirType::Bool),
            SyntaxKind::FALSE_KW => MirExpr::BoolLit(false, MirType::Bool),
            _ => MirExpr::Unit,
        }
    }

    // ── Name reference lowering ──────────────────────────────────────

    /// The concrete function types the let-bound closure `name` is used at
    /// within `block` after its `let`, each with the variable name that will
    /// hold that specialization.
    /// The concrete types the closure bound to `name` by `let_` is used at
    /// in `block`, each with the name of its specialized copy, and whether
    /// every use is at a concrete type (so the generic copy is never used).
    fn poly_closure_uses(
        &self,
        block: &Block,
        let_: &LetBinding,
        name: &str,
    ) -> (Vec<(Ty, String)>, bool) {
        let after = let_.syntax().text_range().end();
        let mut uses: Vec<(Ty, String)> = Vec::new();
        let mut all_concrete = true;
        for node in block.syntax().descendants() {
            let Some(name_ref) = NameRef::cast(node) else {
                continue;
            };
            if name_ref.text().as_deref() != Some(name)
                || name_ref.syntax().text_range().start() < after
            {
                continue;
            }
            // `let g = name`: the uses of `g` are uses of this value.
            let alias = name_ref
                .syntax()
                .parent()
                .and_then(LetBinding::cast)
                .filter(|alias| {
                    alias.initializer().map(|init| init.syntax().text_range())
                        == Some(name_ref.syntax().text_range())
                });
            if let Some((alias, alias_name)) =
                alias.and_then(|alias| Some((alias.clone(), alias.name()?.text()?)))
            {
                let (alias_uses, alias_concrete) =
                    self.poly_closure_uses(block, &alias, &alias_name);
                all_concrete &= alias_concrete;
                for (use_ty, _) in alias_uses {
                    if !uses.iter().any(|(ty, _)| *ty == use_ty) {
                        let spec_name = format!(
                            "{name}__spec_{}",
                            Self::ty_specialization_component(&use_ty)
                        );
                        uses.push((use_ty, spec_name));
                    }
                }
                continue;
            }
            // A name the type checker gave no type is no use of the value:
            // a keyword key (`f(name: 1)`) spells it too.
            let Some(use_ty) = self.get_ty(name_ref.syntax().text_range()) else {
                continue;
            };
            if !matches!(use_ty, Ty::Fun(..)) || Self::ty_contains_var(use_ty) {
                all_concrete = false;
                continue;
            }
            if uses.iter().any(|(ty, _)| ty == use_ty) {
                continue;
            }
            let spec_name = format!("{name}__spec_{}", Self::ty_specialization_component(use_ty));
            uses.push((use_ty.clone(), spec_name));
        }
        (uses, all_concrete)
    }

    /// Lower `closure` (a closure, or a name of a generic function) with the
    /// type variables of its `generic` type bound as in `concrete`, the type
    /// it is used at.
    fn lower_closure_specialized(
        &mut self,
        closure: &Expr,
        generic: &Ty,
        concrete: &Ty,
    ) -> MirExpr {
        let mut bindings = Vec::new();
        bind_type_vars(generic, concrete, &mut bindings);
        let closure_range = closure.syntax().text_range();
        let overlay: Vec<(TextRange, Ty)> = self
            .types
            .keys()
            .filter(|range| closure_range.contains_range(**range))
            .filter_map(|range| {
                let effective = self.get_ty(*range)?;
                Self::ty_contains_var(effective)
                    .then(|| (*range, apply_type_vars(effective, &bindings)))
            })
            .collect();
        let saved = self.spec_types.clone();
        self.spec_types.extend(overlay);
        let value = match closure {
            Expr::ClosureExpr(closure) => self.lower_closure_expr(closure),
            other => self.lower_expr(other),
        };
        self.spec_types = saved;
        value
    }

    /// The specialization of the let-bound closure `name` for the type it
    /// is used at in `range`, when one was compiled and is in scope.
    fn poly_closure_spec_for(&self, name: &str, range: TextRange) -> Option<MirExpr> {
        let use_ty = self.get_ty(range)?;
        let (_, spec_name) = self
            .poly_closure_specs
            .get(name)?
            .iter()
            .find(|(ty, _)| ty == use_ty)?;
        let ty = self.lookup_non_global_var(spec_name)?;
        Some(MirExpr::Var(spec_name.clone(), ty))
    }

    fn lower_local_ref(&self, name: String, ty: MirType, range: TextRange) -> MirExpr {
        if self
            .get_ty(range)
            .is_some_and(|typeck_ty| self.registry.is_resource_type(typeck_ty))
        {
            MirExpr::ResourceMove {
                value: Box::new(MirExpr::Var(name.clone(), ty.clone())),
                ty,
                source: MirResourceMoveSource::Slot(name),
            }
        } else {
            MirExpr::Var(name, ty)
        }
    }

    fn lower_name_ref(&mut self, name_ref: &NameRef) -> MirExpr {
        let name = name_ref.text().unwrap_or_else(|| "<unknown>".to_string());
        let range = name_ref.syntax().text_range();
        let resolved_ty = self.resolve_range(range);

        // Check if this is a nullary variant constructor (e.g., Red, None, Point).
        // These are NameRef nodes that refer to sum type variants with no fields.
        // The type checker types a constructor as the instance of its sum
        // type it builds (`Option_Int`).
        if find_type_for_variant(&name, Some(&resolved_ty), self.registry, Some(0)).is_some() {
            let concrete_name = mir_type_to_impl_name(&resolved_ty);
            return MirExpr::ConstructVariant {
                type_name: concrete_name.clone(),
                variant: name,
                fields: vec![],
                ty: MirType::SumType(concrete_name),
            };
        }

        // Check non-global scopes first for local variables. This ensures pattern
        // bindings and params (e.g., `head` from `head :: tail`, or a local
        // `node_name`) shadow top-level function names without breaking normal
        // function references registered in the root scope.
        if let Some(scope_ty) = self.lookup_non_global_var(&name) {
            if let Some(spec) = self.poly_closure_spec_for(&name, range) {
                return spec;
            }
            return self.lower_local_ref(name, scope_ty, range);
        }

        if let Some(scope_ty) = self.lookup_var(&name) {
            if self.user_fn_defs.contains(&name) {
                let qualified_name = self.qualify_name(&name);
                let lowered_name = self.lowered_fn_symbol_name(&name, &qualified_name, range);
                return MirExpr::Var(lowered_name, resolved_ty);
            }
            return self.lower_local_ref(name, scope_ty, range);
        }

        if let Some(module) = self.stdlib_imports.get(&name) {
            let fn_ty = self.get_ty(range).cloned();
            return self.lower_stdlib_function(module, &name, fn_ty, resolved_ty);
        }

        // Map builtin function names to their runtime equivalents.
        let mapped_name = if self.imported_functions.contains(&name) {
            name.clone()
        } else {
            map_builtin_name(&name)
        };
        let ty = resolved_ty;

        // The functions this module defines are in scope, found above; an
        // imported one keeps its own name.
        let lowered_name = if self.imported_functions.contains(&mapped_name) {
            self.lowered_fn_symbol_name(&mapped_name, &mapped_name, range)
        } else {
            mapped_name
        };

        MirExpr::Var(lowered_name, ty)
    }

    // ── Binary expression lowering ───────────────────────────────────

    fn lower_binary_expr(&mut self, bin: &BinaryExpr) -> MirExpr {
        let lhs = bin
            .lhs()
            .map(|e| self.lower_expr(&e))
            .unwrap_or(MirExpr::Unit);
        let rhs = bin
            .rhs()
            .map(|e| self.lower_expr(&e))
            .unwrap_or(MirExpr::Unit);

        let op = bin
            .op()
            .map(|t| match t.kind() {
                SyntaxKind::PLUS => BinOp::Add,
                SyntaxKind::MINUS => BinOp::Sub,
                SyntaxKind::STAR => BinOp::Mul,
                SyntaxKind::SLASH => BinOp::Div,
                SyntaxKind::PERCENT => BinOp::Mod,
                SyntaxKind::EQ_EQ => BinOp::Eq,
                SyntaxKind::NOT_EQ => BinOp::NotEq,
                SyntaxKind::LT => BinOp::Lt,
                SyntaxKind::GT => BinOp::Gt,
                SyntaxKind::LT_EQ => BinOp::LtEq,
                SyntaxKind::GT_EQ => BinOp::GtEq,
                SyntaxKind::AND_KW | SyntaxKind::AMP_AMP => BinOp::And,
                SyntaxKind::OR_KW | SyntaxKind::PIPE_PIPE => BinOp::Or,
                SyntaxKind::PLUS_PLUS | SyntaxKind::DIAMOND => BinOp::Concat,
                _ => BinOp::Add, // fallback
            })
            .unwrap_or(BinOp::Add);

        let ty = self.resolve_range(bin.syntax().text_range());

        // `start..end` outside a `for` header is a Range value.
        if bin.op().map(|t| t.kind()) == Some(SyntaxKind::DOT_DOT) {
            return Self::call_named(
                "mesh_range_new",
                vec![MirType::Int, MirType::Int],
                vec![lhs, rhs],
                MirType::Ptr,
            );
        }

        // Comparison is decided by the operand's source type: primitives by
        // the hardware (and the string runtime), everything else by
        // structure or by the type's own Eq/Ord (`eq_expr`, `cmp_fn`).
        let lhs_source = bin
            .lhs()
            .and_then(|e| self.get_ty(e.syntax().text_range()).cloned());
        let primitive = |ty: &Ty| matches!(ty, Ty::Con(tc) if matches!(tc.name.as_str(), "Int" | "Float" | "Bool" | "String"));
        // A value that never comes into being needs no comparison: the
        // hardware path ends where the operand does.
        let needs_fn = |t: &Ty| !matches!(t, Ty::Var(_) | Ty::Never) && !primitive(t);
        if let Some(source) = lhs_source.filter(needs_fn) {
            match op {
                BinOp::Eq | BinOp::NotEq => {
                    let equal = self.eq_expr(lhs, rhs, &source);
                    return if op == BinOp::NotEq {
                        MirExpr::BinOp {
                            op: BinOp::Eq,
                            lhs: Box::new(equal),
                            rhs: Box::new(MirExpr::BoolLit(false, MirType::Bool)),
                            ty,
                        }
                    } else {
                        equal
                    };
                }
                BinOp::Lt | BinOp::Gt | BinOp::LtEq | BinOp::GtEq => {
                    let cmp = self.cmp_fn(&source);
                    let param = self.binding_type(&source);
                    let ordering = Self::call_named(
                        &cmp,
                        vec![param.clone(), param],
                        vec![lhs, rhs],
                        MirType::Int,
                    );
                    return MirExpr::BinOp {
                        op,
                        lhs: Box::new(ordering),
                        rhs: Box::new(MirExpr::IntLit(0, MirType::Int)),
                        ty,
                    };
                }
                _ => {}
            }
        }

        // Arithmetic on user types dispatches to the type's operator impl:
        // comparisons of them were decided above, and the type checker
        // admits no other operator on them, nor one without its impl.
        let lhs_ty = lhs.ty().clone();
        if matches!(lhs_ty, MirType::Struct(_) | MirType::SumType(_)) {
            let (trait_name, method_name) = match op {
                BinOp::Add => ("Add", "add"),
                BinOp::Sub => ("Sub", "sub"),
                BinOp::Mul => ("Mul", "mul"),
                BinOp::Div => ("Div", "div"),
                _ => ("Mod", "mod"),
            };
            let type_name = mir_type_to_impl_name(&lhs_ty);
            let mangled = format!("{}__{}__{}", trait_name, method_name, type_name);
            let rhs_ty = rhs.ty().clone();
            let fn_ty = MirType::FnPtr(vec![lhs_ty, rhs_ty], Box::new(ty.clone()));
            return MirExpr::Call {
                func: Box::new(MirExpr::Var(mangled, fn_ty)),
                args: vec![lhs, rhs],
                ty,
            };
        }

        MirExpr::BinOp {
            op,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
            ty,
        }
    }

    // ── Unary expression lowering ────────────────────────────────────

    fn lower_unary_expr(&mut self, un: &UnaryExpr) -> MirExpr {
        let operand = un
            .operand()
            .map(|e| self.lower_expr(&e))
            .expect("the parser gives a unary operator its operand");

        // The parser's unary operators are `-`, and `!` or `not`.
        let op = match un.op().map(|t| t.kind()) {
            Some(SyntaxKind::MINUS) => UnaryOp::Neg,
            _ => UnaryOp::Not,
        };

        let ty = self.resolve_range(un.syntax().text_range());

        // A negated struct or sum type calls its Neg impl, which the type
        // checker requires; Int and Float are negated by the hardware.
        let operand_ty = operand.ty().clone();
        if op == UnaryOp::Neg && matches!(operand_ty, MirType::Struct(_) | MirType::SumType(_)) {
            let mangled = format!("Neg__neg__{}", mir_type_to_impl_name(&operand_ty));
            let fn_ty = MirType::FnPtr(vec![operand_ty], Box::new(ty.clone()));
            return MirExpr::Call {
                func: Box::new(MirExpr::Var(mangled, fn_ty)),
                args: vec![operand],
                ty,
            };
        }

        MirExpr::UnaryOp {
            op,
            operand: Box::new(operand),
            ty,
        }
    }

    // ── Trait dispatch helpers ────────────────────────────────────────

    /// Check if a name refers to a sum type (e.g., Shape, Option).
    /// Used to prevent intercepting variant constructor calls like Shape.Circle(5.0).
    fn is_sum_type_name(&self, name: &str) -> bool {
        self.registry.sum_type_defs.contains_key(name)
    }

    /// Check if a name refers to a struct type (e.g., Point).
    /// Used to prevent intercepting module-style qualified calls on struct names.
    fn is_struct_type_name(&self, name: &str) -> bool {
        self.registry.struct_defs.contains_key(name)
    }

    /// Resolve a trait method callee: given a method name and the first argument's type,
    /// check if it's a trait method and rewrite to the mangled name (Trait__Method__Type).
    /// Returns the resolved callee (either mangled or original).
    /// The derived trait method `name` (`to_string`, `compare`, ...) of the
    /// generic instantiation `source` (`Box<List<Int>>`), whose helpers go
    /// by its source types rather than its layout (see
    /// `instantiation_helper_name`); `None` for any other callee.
    fn instantiation_trait_callee(&mut self, name: &str, source: &Ty) -> Option<String> {
        let (Ty::App(..), Some((type_name, args))) = (source, ty_head(source)) else {
            return None;
        };
        if self.known_functions.contains_key(name) {
            return None;
        }
        self.ensure_instantiation_traits(source);
        let helper = self.instantiation_helper_name(type_name, args);
        ["Display", "Debug", "Eq", "Ord", "Hash"]
            .iter()
            .map(|t| format!("{t}__{name}__{helper}"))
            .find(|f| self.known_functions.contains_key(f))
    }

    /// The mangled name of the impl method `method` that type `ty` provides
    /// as a static method (no `self`), if exactly one impl does. Conversions
    /// (`From`, `TryFrom`, derived Json and Row decoding) have their own
    /// lowering, which picks by argument and wraps the decoders.
    fn static_impl_method(&self, method: &str, ty: &Ty) -> Option<String> {
        let impls: Vec<_> = self
            .trait_registry
            .impls_providing(method, ty)
            .into_iter()
            .filter(|(imp, _)| imp.methods.get(method).is_some_and(|sig| !sig.has_self))
            .collect();
        if impls.len() != 1
            || impls.iter().any(|(imp, _)| {
                matches!(
                    imp.trait_name.as_str(),
                    "From" | "TryFrom" | "FromJson" | "FromRow"
                )
            })
        {
            return None;
        }
        impls.into_iter().next().map(|(imp, _)| {
            let args: Vec<String> = imp.trait_type_args.iter().map(trait_arg_name).collect();
            mangle_trait_method(&imp.trait_name, &args, method, &imp.impl_type_name)
        })
    }

    /// The function of `type_name`'s `trait_name` (`From` or `TryFrom`)
    /// impl that converts a `source`: `Meters.from(5)`, with `From<Int>` and
    /// `From<String>`, is `From_Int__from__Meters`.
    fn conversion_fn(
        &self,
        trait_name: &str,
        method: &str,
        type_name: &str,
        source: Option<&Ty>,
    ) -> String {
        let target = Ty::Con(mesh_typeck::ty::TyCon::new(type_name));
        let impls: Vec<_> = self
            .trait_registry
            .impls_providing(method, &target)
            .into_iter()
            .map(|(imp, _)| imp)
            .filter(|imp| imp.trait_name == trait_name)
            .collect();
        let chosen = source
            .and_then(|source| {
                impls.iter().find(|imp| {
                    imp.trait_type_args
                        .first()
                        .is_some_and(|arg| trait_arg_name(arg) == trait_arg_name(source))
                })
            })
            .or(impls.first())
            .expect("the type checker admits a conversion only with its impl");
        let type_args: Vec<String> = chosen.trait_type_args.iter().map(trait_arg_name).collect();
        mangle_trait_method(trait_name, &type_args, method, &chosen.impl_type_name)
    }

    /// How a call of `fa` (`Base.method(...)`) is lowered when its base
    /// names an interface or a type rather than a module or a value: an
    /// interface's method (`Iface.method(value, ...)`), a type's instance
    /// method (`Type.method(value, ...)`, as `value.method(...)`), or a
    /// static method of a type (`Int.tag()`, `Config.default()`) or, in a
    /// generic body, of a type parameter (`T.version()`, bound per
    /// specialization). `None` for any other call.
    fn qualified_route(&self, fa: &FieldAccess) -> Option<QualifiedRoute> {
        let Some(Expr::NameRef(base)) = fa.base() else {
            return None;
        };
        let base_name = base.text()?;
        if self.lookup_non_global_var(&base_name).is_some() {
            return None;
        }
        let method = fa.field()?.text().to_string();
        let is_type = self.is_struct_type_name(&base_name) || self.is_sum_type_name(&base_name);
        let is_module = STDLIB_MODULES.contains(&base_name.as_str())
            || self.user_modules.contains_key(&base_name)
            || self.service_modules.contains_key(&base_name);
        let interface_method = !is_type
            && !is_module
            && self
                .trait_registry
                .get_trait(&base_name)
                .is_some_and(|trait_def| {
                    trait_def
                        .methods
                        .iter()
                        .any(|m| m.name == method && m.has_self)
                });
        if interface_method {
            return Some(QualifiedRoute::Interface(base_name, method));
        }
        let ty = match base_name.as_str() {
            "Int" | "Float" | "String" | "Bool" => Ty::Con(mesh_typeck::ty::TyCon::new(&base_name)),
            _ if is_type => Ty::Con(mesh_typeck::ty::TyCon::new(&base_name)),
            _ => self
                .get_ty(base.syntax().text_range())
                .filter(|ty| !Self::ty_contains_var(ty))?
                .clone(),
        };
        if is_type
            && self
                .trait_registry
                .find_method_sig(&method, &ty)
                .is_some_and(|sig| sig.has_self)
        {
            return Some(QualifiedRoute::TypeMethod(method));
        }
        let conversion = match method.as_str() {
            "from" => Some("From"),
            "try_from" => Some("TryFrom"),
            _ => None,
        };
        let conversion = conversion.filter(|trait_name| {
            is_type
                && self
                    .trait_registry
                    .impls_providing(&method, &ty)
                    .iter()
                    .any(|(imp, _)| imp.trait_name == *trait_name)
        });
        if let Some(trait_name) = conversion {
            return Some(QualifiedRoute::Conversion(
                trait_name.to_string(),
                method,
                base_name,
            ));
        }
        self.static_impl_method(&method, &ty)
            .map(QualifiedRoute::Static)
    }

    /// A call routed by `qualified_route`, of the lowered `args`; `first_ty`
    /// is the first argument's type: the receiver of a method, the source of
    /// a conversion.
    fn lower_qualified_call(
        &mut self,
        call_range: TextRange,
        route: QualifiedRoute,
        args: Vec<MirExpr>,
        first_ty: Option<Ty>,
    ) -> MirExpr {
        let ty = self.resolve_range(call_range);
        let is_method = matches!(route, QualifiedRoute::Interface(..));
        let callee = match route {
            QualifiedRoute::TypeMethod(method) => {
                let mut args = args.into_iter();
                let receiver = args.next().unwrap_or(MirExpr::Unit);
                return self.lower_method_call(
                    call_range,
                    &method,
                    receiver,
                    first_ty,
                    args.collect(),
                );
            }
            QualifiedRoute::Static(callee) => callee,
            // The argument's type picks the impl.
            QualifiedRoute::Conversion(trait_name, method, type_name) => {
                self.conversion_fn(&trait_name, &method, &type_name, first_ty.as_ref())
            }
            QualifiedRoute::Interface(trait_name, method) => first_ty
                .and_then(|receiver| {
                    self.trait_registry
                        .impls_providing(&method, &receiver)
                        .into_iter()
                        .find(|(imp, _)| imp.trait_name == trait_name)
                        .map(|(imp, _)| {
                            let type_args: Vec<String> =
                                imp.trait_type_args.iter().map(trait_arg_name).collect();
                            mangle_trait_method(
                                &trait_name,
                                &type_args,
                                &method,
                                &imp.impl_type_name,
                            )
                        })
                })
                .expect("the type checker calls an interface's method on a type implementing it"),
        };
        let callee = MirExpr::Var(
            builtin_trait_redirect(callee),
            MirType::FnPtr(
                args.iter().map(|arg| arg.ty().clone()).collect(),
                Box::new(ty.clone()),
            ),
        );
        let args = match is_method {
            true => self.borrow_receiver(&callee, args),
            false => args,
        };
        MirExpr::Call {
            func: Box::new(callee),
            args,
            ty,
        }
    }

    /// A call of the trait method `method_name` on `receiver` (of type
    /// `receiver_source`, as far as it is known) with the arguments `rest`:
    /// `value.method(...)`, and `Type.method(value, ...)` routed here.
    fn lower_method_call(
        &mut self,
        call_range: TextRange,
        method_name: &str,
        receiver: MirExpr,
        receiver_source: Option<Ty>,
        rest: Vec<MirExpr>,
    ) -> MirExpr {
        let mut args = vec![receiver];
        args.extend(rest);

        let ty = self.resolve_range(call_range);

        // An instantiated generic sum type (`Option<Int>`) gets
        // its trait functions on first use.
        if let Some(source) = &receiver_source {
            self.ensure_instantiation_traits(source);
        }

        // Route through the shared trait dispatch helper
        let first_arg_ty = args[0].ty().clone();
        let callee_var_ty = MirType::FnPtr(
            args.iter().map(|a| a.ty().clone()).collect(),
            Box::new(ty.clone()),
        );
        let compared = receiver_source
            .as_ref()
            .filter(|ty| method_name == "compare" && args.len() == 2 && !matches!(ty, Ty::Var(_)));
        if let Some(source) = compared {
            return self.compare_call(source, args);
        }
        let call_result = self.get_ty(call_range).cloned();
        let callee_name = receiver_source
            .as_ref()
            .and_then(|source| {
                self.parameterized_impl_callee(method_name, source, call_result.as_ref())
            })
            .or_else(|| {
                receiver_source
                    .as_ref()
                    .and_then(|source| self.instantiation_trait_callee(method_name, source))
            })
            .unwrap_or_else(|| self.resolve_trait_callee(method_name, &first_arg_ty));

        // Apply the same post-dispatch optimizations as bare-name calls: a
        // String's Display is the String itself, its Debug quotes and
        // escapes it.
        let mut receiver_and_rest = args.into_iter();
        match callee_name.as_str() {
            "Display__to_string__String" => {
                return receiver_and_rest.next().expect("a method has a receiver")
            }
            "Debug__inspect__String" => {
                return Self::inspect_string(
                    receiver_and_rest.next().expect("a method has a receiver"),
                )
            }
            _ => {}
        }
        let args: Vec<MirExpr> = receiver_and_rest.collect();

        // `to_string` / `inspect` on a value whose type, not a
        // nominal impl, decides how it prints.
        let shown_by_type =
            matches!(callee_name.as_str(), "to_string" | "debug" | "inspect") && args.len() == 1;
        let shown = receiver_source
            .as_ref()
            .filter(|_| shown_by_type)
            .and_then(|ty| self.display_by_type(&args[0], ty, callee_name == "inspect"));
        if let Some(shown) = shown {
            return shown;
        }
        let callee = MirExpr::Var(callee_name, callee_var_ty);
        let args = self.borrow_receiver(&callee, args);
        let args = self.apply_direct_resource_modes(&callee, args);
        MirExpr::Call {
            func: Box::new(callee),
            args,
            ty,
        }
    }

    /// The impl method a call of `method` on a `receiver` goes to when its
    /// impl belongs to a generic interface (named with the interface's type
    /// arguments: `Convert_Int__convert__Meters`), picked among several by
    /// the call's `result` type. `.into()` and `.try_into()` go to the
    /// `From` and `TryFrom` impls they stand for.
    fn parameterized_impl_callee(
        &self,
        method: &str,
        receiver: &Ty,
        result: Option<&Ty>,
    ) -> Option<String> {
        let mut candidates: Vec<_> = self
            .trait_registry
            .impls_providing(method, receiver)
            .into_iter()
            .filter(|(imp, _)| !imp.trait_type_args.is_empty())
            .collect();
        let (imp, _) = if candidates.len() == 1 {
            candidates.pop()?
        } else {
            let result = result.map(|ty| resolve_type(ty, self.registry))?;
            candidates.into_iter().find(|(_, ret)| {
                ret.as_ref()
                    .is_some_and(|ret| resolve_type(ret, self.registry) == result)
            })?
        };
        let args: Vec<String> = imp.trait_type_args.iter().map(trait_arg_name).collect();
        let source = [trait_arg_name(&imp.impl_type)];
        let mangled = match (imp.trait_name.as_str(), args.first()) {
            ("Into", Some(target)) => mangle_trait_method("From", &source, "from", target),
            ("TryInto", Some(target)) => {
                mangle_trait_method("TryFrom", &source, "try_from", target)
            }
            _ => mangle_trait_method(&imp.trait_name, &args, method, &imp.impl_type_name),
        };
        Some(builtin_trait_redirect(mangled))
    }

    /// The function a call of `name` with a first argument of type
    /// `first_arg_ty` runs: the trait method of that type it names, or
    /// `name` itself.
    fn resolve_trait_callee(&self, name: &str, first_arg_ty: &MirType) -> String {
        if !self.known_functions.contains_key(name) {
            let ty_for_lookup = mir_type_to_ty(first_arg_ty);
            let mut matching_traits = self.trait_registry.find_method_traits(name, &ty_for_lookup);
            matching_traits.sort(); // Defense-in-depth: deterministic trait selection
            if !matching_traits.is_empty() {
                let trait_name = &matching_traits[0];
                let type_name = mir_type_to_impl_name(first_arg_ty);
                let mangled = format!("{}__{}__{}", trait_name, name, type_name);

                return builtin_trait_redirect(mangled);
            }

            // A generic type's instance has its trait functions found by
            // `instantiation_trait_callee`, and a method of a String or a
            // collection (`"hello".length()`) is its module's function,
            // lowered as the stdlib call it is before this.

            // Defense-in-depth warning -- skip module-scoped helpers (Module__func),
            // compiler-generated service stubs (__service_*), and runtime intrinsics (mesh_*).
            let type_name = mir_type_to_impl_name(first_arg_ty);
            // A generic function's unspecialized body has no receiver type
            // yet; only its specializations are called, so it is not a bug.
            if self.lookup_var(name).is_none()
                && !self.known_functions.contains_key(name)
                && !name.contains("__")
                && !name.starts_with("mesh_")
                && type_name != "Unknown"
            {
                eprintln!(
                    "[mesh-codegen] warning: call to '{}' could not be resolved \
                     as a trait method for type '{}'. This may indicate a type checker bug.",
                    name, type_name
                );
            }
        }
        name.to_string()
    }

    // ── Call expression lowering ─────────────────────────────────────

    /// A method's receiver is borrowed from its caller, as the ownership
    /// check has it (`s.close()` leaves `s` with the caller, who drops it),
    /// unless the callee states its own modes. It was moved out, nulling
    /// the caller's `s`: its later reads saw zeroes, and nothing dropped it.
    fn borrow_receiver(&self, callee: &MirExpr, mut args: Vec<MirExpr>) -> Vec<MirExpr> {
        if matches!(callee, MirExpr::Var(name, _) if self.ownership_signatures.contains_key(name)) {
            return args;
        }
        // A method call has its receiver first.
        if let MirExpr::ResourceMove { value, ty, .. } = &args[0] {
            args[0] = MirExpr::ResourceBorrow {
                value: value.clone(),
                ty: ty.clone(),
            };
        }
        args
    }

    fn apply_direct_resource_modes(&self, callee: &MirExpr, args: Vec<MirExpr>) -> Vec<MirExpr> {
        let MirExpr::Var(name, _) = callee else {
            return args;
        };
        let Some(modes) = self.ownership_signatures.get(name) else {
            return args;
        };

        args.into_iter()
            .enumerate()
            .map(|(index, argument)| match (modes.get(index), argument) {
                (Some(ParamOwnership::Borrow), MirExpr::ResourceMove { value, ty, .. }) => {
                    MirExpr::ResourceBorrow { value, ty }
                }
                (_, argument) => argument,
            })
            .collect()
    }

    fn lower_call_expr(&mut self, call: &CallExpr) -> MirExpr {
        let lowered = self.lower_call_expr_unshaped(call);
        // Inside an actor with parameters, a call to the actor itself runs its
        // body function; the actor's own name is the spawn entry, which takes
        // an argument buffer. Tail calls become loops later.
        if let (
            Some((actor, body_fn, param_tys)),
            Some(Expr::NameRef(callee)),
            MirExpr::Call { args, .. },
        ) = (&self.actor_body_target, call.callee(), &lowered)
        {
            if callee.text().as_deref() == Some(actor.as_str())
                && self.lookup_non_global_var(actor).is_none()
            {
                return MirExpr::Call {
                    func: Box::new(MirExpr::Var(
                        body_fn.clone(),
                        MirType::FnPtr(param_tys.clone(), Box::new(MirType::Unit)),
                    )),
                    args: args.clone(),
                    ty: MirType::Unit,
                };
            }
        }
        // A message that crosses to another actor, last of the arguments:
        // `Timer.send_after(pid, ms, message)`'s, and the one a monitor sends.
        let MirExpr::Call { func, mut args, ty } = lowered else {
            return lowered;
        };
        let arity = match func.as_ref() {
            MirExpr::Var(name, _) => match name.as_str() {
                "mesh_timer_send_after" => 3,
                "mesh_process_monitor" | "mesh_node_monitor" => 2,
                _ => 0,
            },
            _ => 0,
        };
        let message = call
            .arg_list()
            .and_then(|list| list.args().nth(arity.max(1) - 1));
        if let (true, Some(message)) = (arity > 0 && args.len() == arity, message) {
            let value = args.pop().unwrap();
            args.push(self.shaped(value, message.syntax().text_range()));
        }
        MirExpr::Call { func, args, ty }
    }

    /// The source of the two arguments an `assert_eq`/`assert_ne` compares,
    /// joined by `op`: `x + 1 == 2`.
    fn compared_source(call: &CallExpr, op: &str) -> String {
        let sides: Vec<String> = call
            .args()
            .iter()
            .map(|arg| arg.syntax().text().to_string().trim().to_string())
            .collect();
        sides.join(&format!(" {op} "))
    }

    fn lower_call_expr_unshaped(&mut self, call: &CallExpr) -> MirExpr {
        // `panic(message)`: the runtime raises it, and nothing runs after it.
        if let Some(Expr::NameRef(callee)) = call.callee() {
            if callee.text().as_deref() == Some("panic") && self.lookup_var("panic").is_none() {
                let message = call
                    .args()
                    .first()
                    .map(|arg| self.lower_expr(arg))
                    .unwrap_or(MirExpr::Unit);
                let raise = MirExpr::Call {
                    func: Box::new(MirExpr::Var(
                        "mesh_panic_str".to_string(),
                        MirType::FnPtr(vec![MirType::String], Box::new(MirType::Unit)),
                    )),
                    args: vec![message],
                    ty: MirType::Unit,
                };
                let unreachable = MirExpr::Panic {
                    message: "unreachable".to_string(),
                    file: "<panic>".to_string(),
                    line: 0,
                };
                return MirExpr::Block(vec![raise, unreachable], MirType::Never);
            }
        }
        if let Some(metadata) = self
            .clustered_route_wrappers
            .get(&call.syntax().text_range())
            .cloned()
        {
            return self.lower_clustered_route_wrapper(call, &metadata);
        }

        // `Iface.method(value, ...)`, `Type.method(value, ...)` and
        // `Type.method(...)`: see `qualified_route`.
        if let Some(Expr::FieldAccess(fa)) = call.callee() {
            if let Some(route) = self.qualified_route(&fa) {
                let exprs = call.args();
                let first_ty = exprs
                    .first()
                    .and_then(|arg| self.get_ty(arg.syntax().text_range()).cloned());
                let args = exprs.iter().map(|arg| self.lower_expr(arg)).collect();
                return self.lower_qualified_call(
                    call.syntax().text_range(),
                    route,
                    args,
                    first_ty,
                );
            }
        }

        // A stdlib function called as a method (`m.get(k)`, `xs.contains(x)`)
        // lowers like `Map.get(m, k)`, through the general call path below.
        let stdlib_method = match call.callee() {
            Some(Expr::FieldAccess(fa)) => {
                self.stdlib_method_module(&fa).map(|module| (module, fa))
            }
            _ => None,
        };

        // Method call interception: if callee is a FieldAccess (expr.method(...)),
        // extract receiver + method name, prepend receiver to args, and route
        // through trait dispatch. This MUST happen BEFORE lower_expr on the callee,
        // because lower_expr would route to lower_field_access which produces a
        // struct GEP (MirExpr::FieldAccess), not a callable.
        if let Some(Expr::FieldAccess(ref fa)) = call.callee() {
            // Check if this is a module/service/variant/struct access (NOT a method call).
            // Module-qualified calls (String.length), service methods (Counter.start),
            // variant constructors (Shape.Circle), and struct-qualified calls are
            // handled by lower_field_access.
            let is_module_or_special = match fa.base() {
                Some(Expr::NameRef(ref name_ref)) => name_ref.text().is_some_and(|base_name| {
                    STDLIB_MODULES.contains(&base_name.as_str())
                        || self.user_modules.contains_key(&base_name)
                        || self.service_modules.contains_key(&base_name)
                        || self.is_sum_type_name(&base_name)
                        || self.is_struct_type_name(&base_name)
                }),
                _ => false,
            };

            if !is_module_or_special && stdlib_method.is_none() {
                let method_name = fa.field().map(|t| t.text().to_string()).unwrap_or_default();

                // `record.field(args)` where the field holds a function
                // value calls that value; no receiver is passed.
                let field_holds_function = fa
                    .base()
                    .and_then(|base| self.get_ty(base.syntax().text_range()))
                    .and_then(ty_head)
                    .and_then(|(name, _)| self.registry.struct_defs.get(name))
                    .is_some_and(|info| {
                        info.fields
                            .iter()
                            .any(|(field, ty)| *field == method_name && matches!(ty, Ty::Fun(..)))
                    });
                if field_holds_function {
                    let func = self.lower_field_access(fa);
                    let args = call.args().iter().map(|arg| self.lower_expr(arg)).collect();
                    let ty = self.resolve_range(call.syntax().text_range());
                    return MirExpr::Call {
                        func: Box::new(func),
                        args,
                        ty,
                    };
                }

                let receiver_source = fa
                    .base()
                    .and_then(|base| self.get_ty(base.syntax().text_range()).cloned());
                let receiver = fa
                    .base()
                    .map(|e| self.lower_expr(&e))
                    .unwrap_or(MirExpr::Unit);
                let rest = call.args().iter().map(|arg| self.lower_expr(arg)).collect();
                return self.lower_method_call(
                    call.syntax().text_range(),
                    &method_name,
                    receiver,
                    receiver_source,
                    rest,
                );
            }
        }

        // ── Test DSL special lowering (Phase 138) ────────────────────────────
        // In test mode, the assertions are expanded to their runtime checks,
        // before the normal lowering path would count their arguments.
        if self.is_test_mode {
            if let Some(Expr::NameRef(callee)) = call.callee() {
                let name = callee.text().unwrap_or_default();
                if let Some(assertion) = self.lower_test_assertion(call, &name) {
                    return assertion;
                }
            }
        }

        // Non-method-call path: normal function calls.
        let callee = if let Some((module, fa)) = &stdlib_method {
            // The method's type: the receiver's, then the arguments', to
            // the call's.
            let method = fa.field().map(|t| t.text().to_string()).unwrap_or_default();
            let args = call.args();
            let params: Option<Vec<Ty>> = fa
                .base()
                .into_iter()
                .chain(args)
                .map(|e| self.get_ty(e.syntax().text_range()).cloned())
                .collect();
            let ret = self.get_ty(call.syntax().text_range()).cloned();
            let fn_ty = params
                .zip(ret)
                .map(|(params, ret)| Ty::Fun(params, Box::new(ret)));
            let fallback = fn_ty
                .as_ref()
                .map(|ty| resolve_type(ty, self.registry))
                .unwrap_or(MirType::Unit);
            self.lower_stdlib_function(module, &method, fn_ty, fallback)
        } else {
            let callee = call.callee().expect("the parser gives a call its callee");
            self.lower_call_target(call.syntax().text_range(), &callee)
        };
        let receiver = stdlib_method
            .as_ref()
            .and_then(|(_, fa)| fa.base())
            .map(|base| self.lower_expr(&base));
        let explicit = call.args();
        let args: Vec<MirExpr> = receiver
            .into_iter()
            .chain(explicit.iter().map(|a| self.lower_expr(a)))
            .collect();

        // A tuple result is the pointer to its block, as `resolve_range`
        // types every tuple value.
        let ty = self.resolve_range(call.syntax().text_range());

        // Check if this is a variant constructor call (e.g., Circle(5.0)).
        if let MirExpr::Var(ref name, _) = callee {
            if find_type_for_variant(name, Some(&ty), self.registry, Some(args.len())).is_some() {
                let concrete_name = mir_type_to_impl_name(&ty);
                return MirExpr::ConstructVariant {
                    type_name: concrete_name.clone(),
                    variant: name.clone(),
                    fields: args,
                    ty: MirType::SumType(concrete_name),
                };
            }
        }

        // For Map functions that take a key argument (put, get, has_key, delete),
        // String keys wrap the map argument in mesh_map_tag_string(). Keys that
        // are neither words nor strings use typed wrappers (`resolve_table_by`).
        let args = if let MirExpr::Var(ref name, _) = callee {
            if matches!(
                name.as_str(),
                "mesh_map_put"
                    | "mesh_map_get"
                    | "mesh_map_fetch"
                    | "mesh_map_has_key"
                    | "mesh_map_delete"
            ) && args.len() >= 2
            {
                let key_ty = args[1].ty().clone();
                if matches!(key_ty, MirType::String) {
                    // String key: tag the map for string comparison
                    let mut new_args = args;
                    let map_arg = new_args.remove(0);
                    let tagged_map = MirExpr::Call {
                        func: Box::new(MirExpr::Var(
                            "mesh_map_tag_string".to_string(),
                            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
                        )),
                        args: vec![map_arg],
                        ty: MirType::Ptr,
                    };
                    new_args.insert(0, tagged_map);
                    new_args
                } else {
                    args
                }
            } else {
                args
            }
        } else {
            args
        };

        // Static trait method dispatch: bare `default()` with zero arguments.
        // The type is the call's own, which the type checker fixes (E0064),
        // not a first argument's (Default::default has no self parameter).
        if let MirExpr::Var(ref name, _) = callee {
            if name == "default" && args.is_empty() {
                // Primitive defaults are literals; any other type's is its
                // impl's function.
                return match mir_type_to_impl_name(&ty).as_str() {
                    "Int" => MirExpr::IntLit(0, MirType::Int),
                    "Float" => MirExpr::FloatLit(0.0, MirType::Float),
                    "Bool" => MirExpr::BoolLit(false, MirType::Bool),
                    "String" => MirExpr::StringLit(String::new(), MirType::String),
                    type_name => Self::call_named(
                        &format!("Default__default__{type_name}"),
                        vec![],
                        vec![],
                        ty,
                    ),
                };
            }
        }

        // compare(a, b) dispatches on the operands' type, which the type
        // checker gives them (a generic body is lowered at concrete types).
        if matches!(&callee, MirExpr::Var(name, _) if name == "compare") && args.len() == 2 {
            let source = call
                .arg_list()
                .and_then(|list| list.args().next())
                .and_then(|arg| self.get_ty(arg.syntax().text_range()).cloned())
                .expect("the type checker types compare's operands");
            return self.compare_call(&source, args);
        }

        // `String.from(x)` shows `x` the way interpolating it would.
        if let MirExpr::Var(ref name, _) = callee {
            if name == "mesh_string_from" && args.len() == 1 {
                let source_ty = call
                    .args()
                    .first()
                    .and_then(|arg| self.get_ty(arg.syntax().text_range()))
                    .cloned();
                let arg = args.into_iter().next().unwrap_or(MirExpr::Unit);
                return self.wrap_to_string(arg, source_ty.as_ref());
            }
        }

        // `to_string(value)` / `inspect(value)` of a value whose type, not a
        // nominal impl, decides how it prints (a tuple, a collection, a PID,
        // an instantiated generic type), as `value.to_string()` does.
        if let MirExpr::Var(ref name, _) = callee {
            if (name == "to_string" || name == "debug" || name == "inspect") && args.len() == 1 {
                let source_ty = call
                    .args()
                    .first()
                    .and_then(|arg| self.get_ty(arg.syntax().text_range()))
                    .cloned();
                if let Some(shown) = source_ty
                    .and_then(|ty| self.display_by_type(&args[0], &ty, name != "to_string"))
                {
                    return shown;
                }
            }
        }

        // The name the call was written with, whose ownership modes hold for
        // whatever it dispatches to below.
        let source_callee = callee.clone();

        // Trait method call rewriting: use shared resolve_trait_callee helper.
        // If the callee is a bare method name (not in known_functions), check if
        // it's a trait method for the first arg's type. If so, rewrite to the
        // mangled name (Trait__Method__Type).
        // Skip trait dispatch for functions from user-defined modules (Phase 39).
        let is_user_module_fn = if let MirExpr::Var(ref name, _) = callee {
            self.user_modules.values().any(|fns| fns.contains(name))
                || self.imported_functions.contains(name)
                || self.is_inferred_specialization_name(name)
        } else {
            false
        };
        // A bare call of a static interface method (`version()`) goes to the
        // one impl that provides it (the type checker rejects several).
        let callee = match callee {
            MirExpr::Var(ref name, ref var_ty)
                if args.is_empty()
                    && !is_user_module_fn
                    && !self.known_functions.contains_key(name)
                    && self.lookup_var(name).is_none() =>
            {
                let impls = self.trait_registry.impls_with_static_method(name);
                match impls.as_slice() {
                    [imp] => {
                        let trait_args: Vec<String> =
                            imp.trait_type_args.iter().map(trait_arg_name).collect();
                        let mangled = mangle_trait_method(
                            &imp.trait_name,
                            &trait_args,
                            name,
                            &imp.impl_type_name,
                        );
                        MirExpr::Var(builtin_trait_redirect(mangled), var_ty.clone())
                    }
                    _ => callee,
                }
            }
            _ => callee,
        };
        let callee = if let MirExpr::Var(ref name, ref var_ty) = callee {
            if !args.is_empty() && !is_user_module_fn {
                let first_arg_ty = args[0].ty().clone();
                let first_arg_source = call
                    .arg_list()
                    .and_then(|list| list.args().next())
                    .and_then(|arg| self.get_ty(arg.syntax().text_range()).cloned());
                let call_result = self.get_ty(call.syntax().text_range()).cloned();
                let resolved = first_arg_source
                    .and_then(|source| {
                        self.parameterized_impl_callee(name, &source, call_result.as_ref())
                            .or_else(|| self.instantiation_trait_callee(name, &source))
                    })
                    .unwrap_or_else(|| self.resolve_trait_callee(name, &first_arg_ty));
                MirExpr::Var(resolved, var_ty.clone())
            } else {
                callee
            }
        } else {
            callee
        };

        // Short-circuit: Display__to_string__String is identity -- return the
        // first argument directly without emitting a function call.
        if let MirExpr::Var(ref name, _) = callee {
            if name == "Display__to_string__String" && !args.is_empty() {
                return args.into_iter().next().unwrap();
            }
            // Debug__inspect__String quotes and escapes the value.
            if name == "Debug__inspect__String" && !args.is_empty() {
                return Self::inspect_string(args.into_iter().next().unwrap());
            }
        }

        // Json.encode struct/sum type dispatch: if encoding a struct or sum type
        // with ToJson, chain ToJson__to_json__TypeName + mesh_json_encode.
        if let MirExpr::Var(ref name, _) = callee {
            if name == "mesh_json_encode" && args.len() == 1 {
                // A value is built into a JSON tree by its source type (a
                // derived ToJson tells `Box<List<Int>>` from
                // `Box<List<String>>`): its raw word was read as a tree (a map
                // printed as a number, an Int crashed). A `Json` value is a
                // tree already.
                let source = call
                    .args()
                    .first()
                    .and_then(|arg| self.get_ty(arg.syntax().text_range()).cloned())
                    .expect("the type checker types Json.encode's argument");
                let tree = match ty_head(&source) {
                    Some((head, _)) => head != "Json",
                    None => matches!(source, Ty::Tuple(_)),
                };
                if tree {
                    let json = self.json_encode_expr(args[0].clone(), &source);
                    return MirExpr::Call {
                        func: Box::new(callee),
                        args: vec![json],
                        ty: MirType::String,
                    };
                }
            }
        }

        // A method called bare (`close(s)`) takes the modes of its written
        // name, with `self` borrowed: the impl function it was dispatched to
        // has none of its own, so the receiver was moved out of the caller.
        let modes_of = match &callee {
            MirExpr::Var(name, _) if !self.ownership_signatures.contains_key(name) => {
                &source_callee
            }
            _ => &callee,
        };
        let args = self.apply_direct_resource_modes(modes_of, args);

        if matches!(&callee, MirExpr::Var(name, _) if name == "mesh_secret_destroy")
            && args.len() == 1
        {
            let value = args.into_iter().next().unwrap();
            let resource_ty = value.ty().clone();
            return MirExpr::ResourceDestroy {
                value: Box::new(value),
                resource_ty,
                destructor: MirResourceDestructor::Opaque,
            };
        }

        // Determine if this is a direct function call or a closure call.
        let is_known_fn = match &callee {
            MirExpr::Var(name, _) => self.known_functions.contains_key(name),
            _ => false,
        };

        if is_known_fn {
            MirExpr::Call {
                func: Box::new(callee),
                args,
                ty,
            }
        } else {
            // Check the callee type. If it's a Closure type, use ClosureCall.
            match callee.ty() {
                MirType::Closure(_, _) => MirExpr::ClosureCall {
                    closure: Box::new(callee),
                    args,
                    ty,
                },
                _ => MirExpr::Call {
                    func: Box::new(callee),
                    args,
                    ty,
                },
            }
        }
    }

    // ── Pipe expression lowering (DESUGARING) ────────────────────────

    fn lower_pipe_expr(&mut self, pipe: &PipeExpr) -> MirExpr {
        self.lower_piped(pipe.syntax().text_range(), pipe.lhs(), pipe.rhs(), 0)
    }

    // ── Slot pipe expression lowering (DESUGARING) ───────────────────

    fn lower_slot_pipe_expr(&mut self, pipe: &SlotPipeExpr) -> MirExpr {
        // 1-indexed, >= 2 by parse guarantee.
        let slot = pipe.slot().unwrap_or(2) as usize;
        self.lower_piped(pipe.syntax().text_range(), pipe.lhs(), pipe.rhs(), slot - 1)
    }

    /// `lhs |> rhs` (`index` 0) and `lhs |N> rhs` (`index` N - 1) desugar to
    /// a call of `rhs` with `lhs` inserted among its arguments at `index`,
    /// clamped to the end: `x |> f(a)` is `f(x, a)`, `x |2> f(a, b)` is
    /// `f(a, x, b)`. A bare `rhs` is called with `lhs` alone.
    fn lower_piped(
        &mut self,
        pipe_range: TextRange,
        lhs_expr: Option<Expr>,
        rhs: Option<Expr>,
        index: usize,
    ) -> MirExpr {
        // `x |> f(a)?` is `(x |> f(a))?`; the checker typed the call at the
        // `?` and the value at the pipe.
        if let Some(Expr::TryExpr(try_expr)) = &rhs {
            let try_range = try_expr.syntax().text_range();
            let piped = self.lower_piped(try_range, lhs_expr, try_expr.operand(), index);
            let piped = self.finish_lowered(piped, try_range);
            let operand_typeck = self.get_ty(try_range).cloned();
            let success_ty = runtime_value_type(self.resolve_range(pipe_range));
            return self.lower_try(piped, operand_typeck, success_ty);
        }
        let lhs = lhs_expr
            .as_ref()
            .map(|e| self.lower_expr(e))
            .unwrap_or(MirExpr::Unit);
        let ty = self.resolve_range(pipe_range);
        if let Some(Expr::SendExpr(send)) = &rhs {
            return self.lower_piped_send(send, lhs, lhs_expr, index);
        }
        if let Some(shown) = self.piped_string_from(&rhs, &lhs, lhs_expr.clone()) {
            return shown;
        }
        let lhs_ty = lhs_expr
            .as_ref()
            .and_then(|e| self.get_ty(e.syntax().text_range()).cloned());

        match rhs.expect("the parser gives a pipe its right-hand side") {
            Expr::CallExpr(call) => {
                let route = match call.callee() {
                    Some(Expr::FieldAccess(fa)) => self.qualified_route(&fa),
                    _ => None,
                };
                let callee = match route {
                    Some(_) => None,
                    None => {
                        let callee = call.callee().expect("the parser gives a call its callee");
                        Some(self.lower_call_target(call.syntax().text_range(), &callee))
                    }
                };
                let explicit = call.args();
                let index = index.min(explicit.len());
                let mut args: Vec<MirExpr> =
                    explicit.iter().map(|arg| self.lower_expr(arg)).collect();
                args.insert(index, lhs);
                if let Some(route) = route {
                    let first_ty = match index {
                        0 => lhs_ty,
                        _ => explicit
                            .first()
                            .and_then(|arg| self.get_ty(arg.syntax().text_range()).cloned()),
                    };
                    return self.lower_qualified_call(
                        call.syntax().text_range(),
                        route,
                        args,
                        first_ty,
                    );
                }
                let callee = callee.expect("a call without a route has its callee");
                let args = self.apply_direct_resource_modes(&callee, args);
                MirExpr::Call {
                    ty,
                    func: Box::new(callee),
                    args,
                }
            }
            rhs_expr => {
                // `x |> f`: `f(x)`.
                if let Expr::FieldAccess(fa) = &rhs_expr {
                    if let Some(route) = self.qualified_route(fa) {
                        return self.lower_qualified_call(pipe_range, route, vec![lhs], lhs_ty);
                    }
                }
                let func = self.lower_call_target(rhs_expr.syntax().text_range(), &rhs_expr);
                let args = self.apply_direct_resource_modes(&func, vec![lhs]);
                MirExpr::Call {
                    ty,
                    func: Box::new(func),
                    args,
                }
            }
        }
    }

    // ── Field access lowering ────────────────────────────────────────

    /// The index of `field` in the struct `parent_ty`, where the type checker
    /// found it.
    fn resource_field_index(&self, parent_ty: &Ty, field: &str) -> u32 {
        let (name, _) = ty_head(parent_ty).expect("a struct is a named type");
        self.registry.struct_defs[name]
            .fields
            .iter()
            .position(|(candidate, _)| candidate == field)
            .expect("the type checker found the field in its struct") as u32
    }

    /// The stdlib module whose function the method call `fa` names: the
    /// receiver is a `String`, `Range`, `List`, `Map`, `Set` or `Iter` and the
    /// method is not a trait method (user functions are never methods).
    fn stdlib_method_module(&self, fa: &FieldAccess) -> Option<&'static str> {
        let base = fa.base()?;
        if let Expr::NameRef(name) = &base {
            let name = name.text()?;
            if STDLIB_MODULES.contains(&name.as_str())
                || self.user_modules.contains_key(&name)
                || self.service_modules.contains_key(&name)
                || self.is_sum_type_name(&name)
                || self.is_struct_type_name(&name)
            {
                return None;
            }
        }
        let receiver = self.get_ty(base.syntax().text_range())?;
        let module = mesh_typeck::infer::method_module(receiver)?;
        let method = fa.field()?.text().to_string();
        // An `Iter` pipeline's methods are the `Iter` functions, whatever
        // the built-in `Iterator` impl of the handle behind it says.
        if module != "Iter"
            && !self
                .trait_registry
                .find_method_traits(&method, receiver)
                .is_empty()
        {
            return None;
        }
        Some(module)
    }

    /// The runtime function behind the stdlib function `base_name.field`
    /// (`Map.get`) instantiated at `fn_ty`; `fallback` is its MIR type when
    /// the runtime function has no declared one.
    /// `Option.map` and the other `Option` and `Result` functions (`module`
    /// and `field`), at the type the checker gave this use (`params` to
    /// `ret`): a function generated once per instantiation, whose body is
    /// the `case` a call stands for.
    fn option_result_function(
        &mut self,
        module: &str,
        field: &str,
        params: &[Ty],
        ret: &Ty,
    ) -> MirExpr {
        let fn_ty = Ty::Fun(params.to_vec(), Box::new(ret.clone()));
        let name = format!(
            "__{}_{field}__{}",
            module.to_lowercase(),
            Self::ty_specialization_component(&fn_ty)
        );
        let args: Vec<(String, MirType)> = params
            .iter()
            .enumerate()
            .map(|(i, ty)| (format!("__arg_{i}"), self.binding_type(ty)))
            .collect();
        let return_type = self.binding_type(ret);
        let mir_ty = MirType::FnPtr(
            args.iter().map(|(_, ty)| ty.clone()).collect(),
            Box::new(return_type.clone()),
        );
        if !self.known_functions.contains_key(&name) {
            self.known_functions.insert(name.clone(), mir_ty.clone());
            let arg = |i: usize| MirExpr::Var(args[i].0.clone(), args[i].1.clone());
            let body =
                self.option_result_body(field, params, ret, arg(0), args.get(1).map(|_| arg(1)));
            self.push_helper_fn(&name, args, return_type, body);
        }
        MirExpr::Var(name, mir_ty)
    }

    /// The `case` behind `option_result_function`: `value` (an `Option` or a
    /// `Result`, `params[0]`) matched, with `extra` the function or the
    /// default the helper takes.
    fn option_result_body(
        &self,
        field: &str,
        params: &[Ty],
        ret: &Ty,
        value: MirExpr,
        extra: Option<MirExpr>,
    ) -> MirExpr {
        let (module, payloads) = ty_head(&params[0]).expect("the checker types the value");
        let (present, absent) = if module == "Option" {
            ("Some", "None")
        } else {
            ("Ok", "Err")
        };
        let sum = mir_type_to_impl_name(value.ty());
        // The payloads, bound: the present variant's and an `Err`'s.
        let payload = |i: usize, name: &str| {
            let ty = self.binding_type(&payloads[i]);
            (
                MirExpr::Var(name.to_string(), ty.clone()),
                Some((name.to_string(), ty)),
            )
        };
        let (v, bind_v) = payload(0, "__present");
        let (e, bind_e) = if module == "Result" {
            payload(1, "__absent")
        } else {
            (MirExpr::Unit, None)
        };
        let arm = |variant: &str, binding: Option<(String, MirType)>, body: MirExpr| {
            let arity = usize::from(variant != "None");
            MirMatchArm {
                pattern: MirPattern::Constructor {
                    type_name: sum.clone(),
                    variant: variant.to_string(),
                    fields: match &binding {
                        Some((name, ty)) => vec![MirPattern::Var(name.clone(), ty.clone())],
                        None => vec![MirPattern::Wildcard; arity],
                    },
                    bindings: binding.into_iter().collect(),
                },
                guard: None,
                body,
            }
        };
        let construct = |ty: &Ty, variant: &str, fields: Vec<MirExpr>| {
            let mir = self.binding_type(ty);
            MirExpr::ConstructVariant {
                type_name: mir_type_to_impl_name(&mir),
                variant: variant.to_string(),
                fields,
                ty: mir,
            }
        };
        // `extra`, the function, applied to `arg`.
        let apply = |arg: MirExpr| {
            let f = extra.clone().expect("the helper takes a function");
            let (_, result) = fun_parts(&params[1]);
            MirExpr::ClosureCall {
                closure: Box::new(f),
                args: vec![arg],
                ty: self.binding_type(result),
            }
        };
        let bool_lit = |b: bool| MirExpr::BoolLit(b, MirType::Bool);
        let (present_body, absent_body) = match field {
            "map" => (
                construct(ret, present, vec![apply(v)]),
                construct(ret, absent, bind_e.iter().map(|_| e.clone()).collect()),
            ),
            "and_then" => (
                apply(v),
                construct(ret, absent, bind_e.iter().map(|_| e.clone()).collect()),
            ),
            "map_err" => (
                construct(ret, "Ok", vec![v]),
                construct(ret, "Err", vec![apply(e.clone())]),
            ),
            "unwrap_or" => (v, extra.clone().expect("unwrap_or takes a default")),
            "is_some" | "is_ok" => (bool_lit(true), bool_lit(false)),
            "is_none" | "is_err" => (bool_lit(false), bool_lit(true)),
            "ok_or" => (
                construct(ret, "Ok", vec![v]),
                construct(
                    ret,
                    "Err",
                    vec![extra.clone().expect("ok_or takes an error")],
                ),
            ),
            // `ok`, the last of the functions the type checker has.
            _ => (
                construct(ret, "Some", vec![v]),
                construct(ret, "None", vec![]),
            ),
        };
        // Only the bodies that read a payload bind it.
        let present_binding = bind_v.filter(|_| !field.starts_with("is_"));
        let absent_binding = bind_e.filter(|_| matches!(field, "map" | "and_then" | "map_err"));
        MirExpr::Match {
            scrutinee: Box::new(value),
            arms: vec![
                arm(present, present_binding, present_body),
                arm(absent, absent_binding, absent_body),
            ],
            ty: self.binding_type(ret),
        }
    }

    fn lower_stdlib_function(
        &mut self,
        base_name: &str,
        field: &str,
        fn_ty: Option<Ty>,
        fallback: MirType,
    ) -> MirExpr {
        if let ("Option" | "Result", Some(Ty::Fun(params, ret))) = (base_name, &fn_ty) {
            return self.option_result_function(base_name, field, params, ret);
        }
        // Convert to prefixed name: String.length -> string_length
        let prefix = match base_name {
            "WsClient" => "ws_client".to_string(),
            "BytesBuilder" => "bytes_builder".to_string(),
            "SecretMap" => "secret_map".to_string(),
            "StorageKey" => "storage_key".to_string(),
            "X25519PrivateKey" => "x25519_private_key".to_string(),
            "SigningPrivateKey" => "signing_private_key".to_string(),
            "MlKemPrivateKey" => "mlkem_private_key".to_string(),
            _ => base_name.to_lowercase(),
        };
        let prefixed = format!("{prefix}_{field}");
        // Map to runtime name
        let runtime_name = map_builtin_name(&prefixed);
        // Map keys that are not words or strings compare by the
        // key type's Eq; String keys from a list or an iterator
        // make a string-keyed map.
        let table_op = runtime_name
            .strip_prefix("mesh_map_")
            .map(|op| ("map", "Map", op))
            .or_else(|| {
                runtime_name
                    .strip_prefix("mesh_set_")
                    .map(|op| ("set", "Set", op))
            });
        // Each of the functions handled below is typed as the function it is
        // (a module's constant, `Math.pi`, is none of them).
        let function_type =
            || fun_parts(fn_ty.as_ref().expect("the type checker types a function"));
        if let Some((collection, type_name, op)) = table_op {
            // `Map.get` runs as `mesh_map_fetch`.
            let op = if op == "fetch" { "get" } else { op };
            let (params, ret) = function_type();
            let table_ty = match op {
                "from_list" | "collect" => Some(ret),
                _ => params.first(),
            };
            let key = table_ty
                .and_then(ty_head)
                .filter(|(name, _)| *name == type_name)
                .and_then(|(_, args)| args.first().cloned());
            if let Some(key) = key {
                let string = matches!(&key, Ty::Con(tc) if tc.name == "String");
                let (params, ret) = (params.to_vec(), Box::new(ret.clone()));
                if let Some(helper) =
                    self.resolve_table_by(collection, op, &params, &ret, &key, string)
                {
                    let ty = self.known_functions[&helper].clone();
                    return MirExpr::Var(helper, ty);
                }
            }
        }
        // `Iter.from` starts the iterator of its source's collection type.
        if runtime_name == "mesh_iter_from" {
            let source = function_type().0.first();
            let constructor = match source.and_then(ty_head) {
                Some(("Map", _)) => Some("mesh_map_iter_new"),
                Some(("Set", _)) => Some("mesh_set_iter_new"),
                Some(("Range", _)) => Some("mesh_range_iter"),
                _ => None,
            };
            if let Some(constructor) = constructor {
                return MirExpr::Var(
                    constructor.to_string(),
                    MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
                );
            }
        }
        if runtime_name == "mesh_channel_try_send" {
            let value = &function_type().0[1];
            if let Some(helper) = self.resolve_channel_send(value) {
                let ty = self.known_functions[&helper].clone();
                return MirExpr::Var(helper, ty);
            }
        }
        if runtime_name == "mesh_queue_pop" {
            // `Queue.pop` returns the value and the queue left.
            let value = function_type()
                .1
                .parts()
                .next()
                .expect("Queue.pop returns a pair")
                .clone();
            let helper = self.resolve_queue_pop(&value);
            let ty = self.known_functions[&helper].clone();
            return MirExpr::Var(helper, ty);
        }
        // Membership of a value that is not a word compares by
        // the element type's Eq, not by the raw slot.
        if runtime_name == "mesh_list_contains" {
            let elem = function_type().0[1].clone();
            let by_word = matches!(&elem, Ty::Var(_))
                || matches!(&elem, Ty::Con(tc) if matches!(tc.name.as_str(), "Int" | "Bool" | "String"));
            if !by_word {
                let helper = self.resolve_list_contains(&elem);
                let ty = self.known_functions[&helper].clone();
                return MirExpr::Var(helper, ty);
            }
        }
        // Use known_functions type if available (more accurate for
        // opaque Ptr returns like List.head on List<(A,B)>), otherwise
        // fall back to typeck-resolved type. A function that takes a
        // function keeps the parameters it has as a Mesh value, which its
        // wrapper takes where it is one (`wrap_builtin_values`).
        let ty = match self.known_functions.get(&runtime_name) {
            Some(MirType::FnPtr(_, ret)) if takes_function(&fallback) => {
                let (params, _) = fallback
                    .function_parts()
                    .expect("a function that takes a function is one");
                MirType::Closure(params.to_vec(), ret.clone())
            }
            Some(known_ty) => known_ty.clone(),
            None => fallback,
        };
        MirExpr::Var(runtime_name, ty)
    }

    /// The function `Type.field` names, for the struct or sum type
    /// `ty_name`, when lowering knows it: `from_json` is the decoding
    /// wrapper (of the instantiation a generic type's call returns),
    /// `from_row` a struct's row reader, `from` and `try_from` the
    /// conversion from the function type's parameter, a `deriving(Schema)`
    /// struct's metadata functions are `Type____table__` and the like, and
    /// any other static interface method is its impl's `Trait__field__Type`.
    fn type_function(
        &mut self,
        fa: &FieldAccess,
        ty_name: &str,
        field: &str,
        is_struct: bool,
    ) -> Option<MirExpr> {
        // Each of these is a function, as the type checker types it.
        let fn_ty = self.get_ty(fa.syntax().text_range()).cloned();
        let function_type =
            || fun_parts(fn_ty.as_ref().expect("the type checker types a function"));
        let specific = match field {
            "from_json" => {
                // It returns a `Result` of the instance it decodes.
                let decoded = ty_head(function_type().1).and_then(|(_, args)| args.first());
                let instance = match decoded {
                    Some(ty @ Ty::App(_, ty_args)) if !ty_args.is_empty() => {
                        let ty = ty.clone();
                        self.ensure_instantiation_traits(&ty);
                        Some(self.instantiation_helper_name(ty_name, ty_args))
                    }
                    _ => None,
                };
                Some(format!(
                    "__json_decode__{}",
                    instance.as_deref().unwrap_or(ty_name)
                ))
            }
            "from_row" if is_struct => Some(format!("FromRow__from_row__{ty_name}")),
            "from" | "try_from" => {
                let trait_name = if field == "from" { "From" } else { "TryFrom" };
                let source = function_type().0.first();
                Some(self.conversion_fn(trait_name, field, ty_name, source))
            }
            "__table__"
            | "__fields__"
            | "__primary_key__"
            | "__relationships__"
            | "__field_types__"
            | "__relationship_meta__"
                if is_struct =>
            {
                Some(format!("{ty_name}__{field}"))
            }
            _ if is_struct && field.starts_with("__") && field.ends_with("_col__") => {
                Some(format!("{ty_name}__{field}"))
            }
            _ => None,
        };
        let known = specific.and_then(|name| {
            let ty = self.known_functions.get(&name)?.clone();
            Some(MirExpr::Var(name, ty))
        });
        known.or_else(|| {
            let suffix = format!("__{field}__{ty_name}");
            self.known_functions
                .iter()
                .filter(|(fn_name, _)| fn_name.ends_with(&suffix) && !fn_name.starts_with("__"))
                .min_by(|a, b| a.0.cmp(b.0))
                .map(|(fn_name, fn_ty)| MirExpr::Var(fn_name.clone(), fn_ty.clone()))
        })
    }

    fn lower_field_access(&mut self, fa: &FieldAccess) -> MirExpr {
        // Check if this is a module-qualified access (e.g., String.length).
        // If the base is a NameRef whose text is a known stdlib module,
        // resolve as a function reference instead of a struct field access.
        // User-defined modules take precedence over stdlib modules to allow
        // user code with modules named "Math", "Int", "Float", etc.
        if let Some(Expr::NameRef(ref name_ref)) = fa.base() {
            if let Some(base_name) = name_ref.text() {
                let field = fa.field().map(|t| t.text().to_string()).unwrap_or_default();
                let range = fa.syntax().text_range();
                // Service modules first: a service method is its generated
                // function (`Counter.start` is `__service_counter_start`),
                // which a user module's bare name would shadow.
                let service_fn = self.service_modules.get(&base_name).and_then(|methods| {
                    methods
                        .iter()
                        .find(|(method, _)| *method == field)
                        .map(|(_, generated)| generated.clone())
                });
                if let Some(generated) = service_fn {
                    return MirExpr::Var(generated, self.resolve_range(range));
                }

                // Check user-defined modules (Phase 39) -- they shadow stdlib.
                if self
                    .user_modules
                    .get(&base_name)
                    .is_some_and(|functions| functions.contains(&field))
                {
                    let ty = self.resolve_range(range);
                    let lowered_name = self.lowered_fn_symbol_name(&field, &field, range);
                    return MirExpr::Var(lowered_name, ty);
                }

                // A qualified variant constructor (`Color.Red`, `Result.Ok`)
                // lowers like the unqualified one. Qualified by the module
                // exporting its type (`Geo.Dot`), the type is the one the
                // checker gave it.
                let owner = if self.user_modules.contains_key(&base_name) {
                    let result = match self.get_ty(range) {
                        Some(Ty::Fun(_, ret)) => Some(ret.as_ref().clone()),
                        other => other.cloned(),
                    };
                    result
                        .as_ref()
                        .and_then(ty_head)
                        .map_or(base_name.clone(), |(name, _)| name.to_string())
                } else {
                    base_name.clone()
                };
                let variant_arity = self.registry.sum_type_defs.get(&owner).and_then(|info| {
                    info.variants
                        .iter()
                        .find(|v| v.name == field)
                        .map(|v| v.fields.len())
                });
                if let Some(arity) = variant_arity {
                    let ty = self.resolve_range(range);
                    if arity > 0 {
                        // The call around it constructs the variant.
                        return MirExpr::Var(field, ty);
                    }
                    let concrete = mir_type_to_impl_name(&ty);
                    return MirExpr::ConstructVariant {
                        type_name: concrete.clone(),
                        variant: field,
                        fields: vec![],
                        ty: MirType::SumType(concrete),
                    };
                }

                // Check stdlib modules (after user modules so user code can shadow).
                if STDLIB_MODULES.contains(&base_name.as_str()) {
                    let fn_ty = self.get_ty(range).cloned();
                    let fallback = self.resolve_range(range);
                    return self.lower_stdlib_function(&base_name, &field, fn_ty, fallback);
                }

                // `Type.name` of a struct or sum type: a function of the type.
                let is_struct = self.registry.struct_defs.contains_key(&base_name);
                if is_struct || self.registry.sum_type_defs.contains_key(&base_name) {
                    if let Some(function) = self.type_function(fa, &base_name, &field, is_struct) {
                        return function;
                    }
                }
            }
        }

        let base = fa.base();
        let parent_typeck = base
            .as_ref()
            .and_then(|expression| self.get_ty(expression.syntax().text_range()))
            .cloned();
        let object = base
            .map(|expression| self.lower_expr(&expression))
            .unwrap_or(MirExpr::Unit);

        let field = fa.field().map(|t| t.text().to_string()).unwrap_or_default();

        let ty = self.resolve_range(fa.syntax().text_range());

        if let MirExpr::ResourceMove {
            value,
            ty: immediate_parent_ty,
            source,
        } = object
        {
            // A field chain is one pending move rooted at the original local. Do not
            // execute an intermediate projection move while evaluating a deeper field.
            let projection = MirExpr::FieldAccess {
                object: value,
                field: field.clone(),
                ty: ty.clone(),
            };
            let field_is_resource = self
                .get_ty(fa.syntax().text_range())
                .is_some_and(|field_ty| self.registry.is_resource_type(field_ty));
            if !field_is_resource {
                return projection;
            }
            let parent_typeck = parent_typeck.expect("the type checker types a field's parent");
            let next_field_index = self.resource_field_index(&parent_typeck, &field);
            let source = match source {
                MirResourceMoveSource::Slot(root) => MirResourceMoveSource::Projection {
                    root,
                    parent_ty: immediate_parent_ty,
                    parent_destructor: self
                        .resource_destructor(&parent_typeck)
                        .expect("a struct holding a resource is destroyed as one"),
                    field_index: next_field_index,
                    nested_field_indices: Vec::new(),
                },
                MirResourceMoveSource::Projection {
                    root,
                    parent_ty,
                    parent_destructor,
                    field_index,
                    mut nested_field_indices,
                } => {
                    nested_field_indices.push(next_field_index);
                    MirResourceMoveSource::Projection {
                        root,
                        parent_ty,
                        parent_destructor,
                        field_index,
                        nested_field_indices,
                    }
                }
            };
            return MirExpr::ResourceMove {
                value: Box::new(projection),
                ty,
                source,
            };
        }

        MirExpr::FieldAccess {
            object: Box::new(object),
            field,
            ty,
        }
    }

    // ── If expression lowering ───────────────────────────────────────

    fn lower_if_expr(&mut self, if_: &IfExpr) -> MirExpr {
        let cond = if_
            .condition()
            .map(|e| self.lower_expr(&e))
            .unwrap_or(MirExpr::BoolLit(true, MirType::Bool));

        let then_body = if_
            .then_branch()
            .map(|b| self.lower_block(&b))
            .unwrap_or(MirExpr::Unit);

        let else_body = if let Some(else_branch) = if_.else_branch() {
            if let Some(chained_if) = else_branch.if_expr() {
                // else-if chain
                self.lower_if_expr(&chained_if)
            } else {
                // The parser gives an `else` a chained `if` or a block.
                let block = else_branch
                    .block()
                    .expect("the parser gives an `else` its block");
                self.lower_block(&block)
            }
        } else {
            MirExpr::Unit
        };

        let ty = self.resolve_range(if_.syntax().text_range());
        // Without an `else`, the `if` has no value: the then branch's value
        // is dropped rather than stored in a result of another type.
        let then_body = if if_.else_branch().is_none()
            && !matches!(then_body.ty(), MirType::Unit | MirType::Never)
        {
            MirExpr::Block(vec![then_body, MirExpr::Unit], MirType::Unit)
        } else {
            then_body
        };

        MirExpr::If {
            cond: Box::new(cond),
            then_body: Box::new(then_body),
            else_body: Box::new(else_body),
            ty,
        }
    }

    // ── While expression lowering ───────────────────────────────────

    fn lower_while_expr(&mut self, w: &WhileExpr) -> MirExpr {
        let cond = w
            .condition()
            .map(|e| self.lower_expr(&e))
            .unwrap_or(MirExpr::BoolLit(true, MirType::Bool));

        let body = w
            .body()
            .map(|b| self.lower_block(&b))
            .unwrap_or(MirExpr::Unit);

        MirExpr::While {
            cond: Box::new(cond),
            body: Box::new(body),
            ty: MirType::Unit,
        }
    }

    // ── For-in expression lowering ──────────────────────────────────

    fn lower_for_in_expr(&mut self, for_in: &ForInExpr) -> MirExpr {
        // Check if iterable is a DotDot range (keep existing ForInRange behavior).
        if let Some(Expr::BinaryExpr(ref bin)) = for_in.iterable() {
            if bin.op().map(|t| t.kind()) == Some(SyntaxKind::DOT_DOT) {
                return self.lower_for_in_range(for_in, bin);
            }
        }

        // Non-range: detect collection type from typeck results.
        let iterable_ty = for_in
            .iterable()
            .and_then(|e| self.get_ty(e.syntax().text_range()))
            .cloned();

        let ty = iterable_ty.expect("the type checker types a loop's iterable");
        if let Some([key_ty, val_ty]) = collection_elems(&ty, "Map").as_deref() {
            return self.lower_for_in_map(for_in, key_ty, val_ty);
        }
        if let Some([elem_ty]) = collection_elems(&ty, "Set").as_deref() {
            return self.lower_for_in_set(for_in, elem_ty);
        }
        if let Some(elem_ty) = list_elem(&ty) {
            return self.lower_for_in_list(for_in, &elem_ty);
        }
        // The type checker admits nothing else but an Iterable, which hands
        // over its iterator, and an Iterator.
        let is_iterable = self.trait_registry.has_impl("Iterable", &ty);
        self.lower_for_in_iterator(for_in, &ty, is_iterable)
    }

    /// The loop variable of a `for`: its name, or for a pattern binding a
    /// synthetic name whose value the body destructures.
    fn loop_var_name(&self, for_in: &ForInExpr) -> String {
        if for_in.pattern().is_some() {
            format!(
                "__for_elem_{}",
                u32::from(for_in.syntax().text_range().start())
            )
        } else {
            for_in
                .binding_name()
                .and_then(|n| n.text())
                .unwrap_or_else(|| "_".to_string())
        }
    }

    /// Lower a loop's filter and body with the loop variable(s) in scope.
    /// A pattern binding wraps both in a match on `elem` (the loop element,
    /// of source type `elem_src`) that binds the pattern's names.
    fn lower_loop_parts(
        &mut self,
        for_in: &ForInExpr,
        elem: MirExpr,
        elem_src: Option<&Ty>,
    ) -> (Option<Box<MirExpr>>, MirExpr) {
        let Some(pattern) = for_in.pattern() else {
            let filter = for_in.filter().map(|f| Box::new(self.lower_expr(&f)));
            let body = for_in
                .body()
                .map(|b| self.lower_block(&b))
                .unwrap_or(MirExpr::Unit);
            return (filter, body);
        };
        let destructure = |lowerer: &mut Self, inner: MirExpr| {
            let ty = inner.ty().clone();
            MirExpr::Match {
                scrutinee: Box::new(elem.clone()),
                arms: vec![MirMatchArm {
                    pattern: lowerer.lower_pattern_with_expected(&pattern, elem_src),
                    guard: None,
                    body: inner,
                }],
                ty,
            }
        };
        self.push_scope();
        let filter = for_in.filter().map(|f| {
            let filter = self.lower_expr(&f);
            Box::new(destructure(self, filter))
        });
        let body = for_in
            .body()
            .map(|b| self.lower_block(&b))
            .unwrap_or(MirExpr::Unit);
        let body = destructure(self, body);
        self.pop_scope();
        (filter, body)
    }

    fn lower_for_in_iterator(&mut self, for_in: &ForInExpr, ty: &Ty, is_iterable: bool) -> MirExpr {
        let var_name = self.loop_var_name(for_in);

        // An Iterable hands over its iterator; an Iterator is its own.
        let trait_name = if is_iterable { "Iterable" } else { "Iterator" };
        let (iter_fn, iterator_ty) = if is_iterable {
            let iter_fn = match ty_head(ty) {
                Some(("Range", _)) => "mesh_range_iter".to_string(),
                _ => format!(
                    "Iterable__iter__{}",
                    mir_type_to_impl_name(&resolve_type(ty, self.registry))
                ),
            };
            let iterator_ty = self
                .trait_registry
                .resolve_associated_type("Iterable", "Iter", ty)
                .expect("an Iterable impl names its Iter");
            (Some(iter_fn), iterator_ty)
        } else {
            (None, ty.clone())
        };
        // A runtime iterator (`Iter<T>`, a `ListIterator`, an adapter) is a
        // pointer in MIR, and the runtime's `next` advances any of them.
        let next_fn = match resolve_type(&iterator_ty, self.registry) {
            MirType::Ptr => "mesh_iter_generic_next".to_string(),
            iterator => format!("Iterator__next__{}", mir_type_to_impl_name(&iterator)),
        };
        // An `Iter<T>` yields `T`.
        let elem_ty = match ty_head(&iterator_ty) {
            Some(("Iter", [elem])) => elem.clone(),
            _ => self
                .trait_registry
                .resolve_associated_type(trait_name, "Item", ty)
                .expect("an iterator impl names its Item"),
        };

        // Lower the iterable/iterator expression.
        let collection = for_in
            .iterable()
            .map(|e| self.lower_expr(&e))
            .unwrap_or(MirExpr::Unit);

        let elem_mir_ty = runtime_value_type(resolve_type(&elem_ty, self.registry));

        self.push_scope();
        self.insert_var(var_name.clone(), elem_mir_ty.clone());
        let elem = MirExpr::Var(var_name.clone(), elem_mir_ty.clone());
        let (filter, body) = self.lower_loop_parts(for_in, elem, Some(&elem_ty));
        self.pop_scope();

        MirExpr::ForInIterator {
            var: var_name,
            iterator: Box::new(collection),
            filter,
            body: Box::new(body),
            elem_ty: elem_mir_ty,
            next_fn,
            iter_fn,
            ty: MirType::Ptr,
        }
    }

    fn lower_for_in_range(&mut self, for_in: &ForInExpr, bin: &BinaryExpr) -> MirExpr {
        let var_name = self.loop_var_name(for_in);

        let start = bin
            .lhs()
            .map(|e| self.lower_expr(&e))
            .unwrap_or(MirExpr::IntLit(0, MirType::Int));
        let end = bin
            .rhs()
            .map(|e| self.lower_expr(&e))
            .unwrap_or(MirExpr::IntLit(0, MirType::Int));

        self.push_scope();
        self.insert_var(var_name.clone(), MirType::Int);
        let elem = MirExpr::Var(var_name.clone(), MirType::Int);
        let (filter, body) = self.lower_loop_parts(for_in, elem, Some(&Ty::int()));
        self.pop_scope();

        MirExpr::ForInRange {
            var: var_name,
            start: Box::new(start),
            end: Box::new(end),
            filter,
            body: Box::new(body),
            ty: MirType::Ptr,
        }
    }

    fn lower_for_in_list(&mut self, for_in: &ForInExpr, elem_ty_src: &Ty) -> MirExpr {
        let var_name = self.loop_var_name(for_in);

        let collection = for_in
            .iterable()
            .map(|e| self.lower_expr(&e))
            .unwrap_or(MirExpr::Unit);

        // A tuple element is a heap pointer, as every tuple value is.
        let elem_mir_ty = runtime_value_type(resolve_type(elem_ty_src, self.registry));

        self.push_scope();
        self.insert_var(var_name.clone(), elem_mir_ty.clone());
        let elem = MirExpr::Var(var_name.clone(), elem_mir_ty.clone());
        let (filter, body) = self.lower_loop_parts(for_in, elem, Some(elem_ty_src));
        self.pop_scope();

        MirExpr::ForInList {
            var: var_name,
            collection: Box::new(collection),
            filter,
            body: Box::new(body),
            elem_ty: elem_mir_ty,
            ty: MirType::Ptr,
        }
    }

    fn lower_for_in_map(
        &mut self,
        for_in: &ForInExpr,
        key_ty_src: &Ty,
        val_ty_src: &Ty,
    ) -> MirExpr {
        let (key_var, val_var) = if let Some(destr) = for_in.destructure_binding() {
            let names = destr.names();
            let k = names
                .first()
                .and_then(|n| n.text())
                .unwrap_or_else(|| "_".to_string());
            let v = names
                .get(1)
                .and_then(|n| n.text())
                .unwrap_or_else(|| "_".to_string());
            (k, v)
        } else if for_in.pattern().is_some() {
            let base = self.loop_var_name(for_in);
            (format!("{base}_key"), format!("{base}_val"))
        } else {
            let var_name = for_in
                .binding_name()
                .and_then(|n| n.text())
                .unwrap_or_else(|| "_".to_string());
            (var_name, "_".to_string())
        };

        let collection = for_in
            .iterable()
            .map(|e| self.lower_expr(&e))
            .unwrap_or(MirExpr::Unit);

        let key_mir_ty = runtime_value_type(resolve_type(key_ty_src, self.registry));
        let val_mir_ty = runtime_value_type(resolve_type(val_ty_src, self.registry));

        self.push_scope();
        self.insert_var(key_var.clone(), key_mir_ty.clone());
        self.insert_var(val_var.clone(), val_mir_ty.clone());
        // A pattern destructures the `(key, value)` pair.
        let pair = MirExpr::Call {
            func: Box::new(MirExpr::Var(
                "__mesh_make_tuple".to_string(),
                MirType::FnPtr(vec![MirType::Int; 2], Box::new(MirType::Ptr)),
            )),
            args: vec![
                MirExpr::Var(key_var.clone(), key_mir_ty.clone()),
                MirExpr::Var(val_var.clone(), val_mir_ty.clone()),
            ],
            ty: MirType::Ptr,
        };
        let pair_src = Ty::Tuple(vec![key_ty_src.clone(), val_ty_src.clone()]);
        let (filter, body) = self.lower_loop_parts(for_in, pair, Some(&pair_src));
        self.pop_scope();

        MirExpr::ForInMap {
            key_var,
            val_var,
            collection: Box::new(collection),
            filter,
            body: Box::new(body),
            key_ty: key_mir_ty,
            val_ty: val_mir_ty,
            ty: MirType::Ptr,
        }
    }

    fn lower_for_in_set(&mut self, for_in: &ForInExpr, elem_ty_src: &Ty) -> MirExpr {
        let var_name = self.loop_var_name(for_in);

        let collection = for_in
            .iterable()
            .map(|e| self.lower_expr(&e))
            .unwrap_or(MirExpr::Unit);

        let elem_mir_ty = runtime_value_type(resolve_type(elem_ty_src, self.registry));

        self.push_scope();
        self.insert_var(var_name.clone(), elem_mir_ty.clone());
        let elem = MirExpr::Var(var_name.clone(), elem_mir_ty.clone());
        let (filter, body) = self.lower_loop_parts(for_in, elem, Some(elem_ty_src));
        self.pop_scope();

        MirExpr::ForInSet {
            var: var_name,
            collection: Box::new(collection),
            filter,
            body: Box::new(body),
            elem_ty: elem_mir_ty,
            ty: MirType::Ptr,
        }
    }

    // ── Case expression lowering ─────────────────────────────────────

    fn lower_case_expr(&mut self, case: &CaseExpr) -> MirExpr {
        let scrutinee_expr = case.scrutinee();
        let scrutinee_typeck = scrutinee_expr
            .as_ref()
            .and_then(|expr| self.get_ty(expr.syntax().text_range()))
            .cloned();
        let scrutinee = scrutinee_expr
            .map(|expr| self.lower_expr(&expr))
            .unwrap_or(MirExpr::Unit);

        let case_typeck = self.get_ty(case.syntax().text_range()).cloned();
        let arms: Vec<MirMatchArm> = case
            .arms()
            .map(|arm| self.lower_match_arm(&arm, scrutinee_typeck.as_ref(), case_typeck.as_ref()))
            .collect();

        let ty = self.resolve_range(case.syntax().text_range());

        MirExpr::Match {
            scrutinee: Box::new(scrutinee),
            arms,
            ty,
        }
    }

    fn lower_match_arm(
        &mut self,
        arm: &MatchArm,
        expected: Option<&Ty>,
        result: Option<&Ty>,
    ) -> MirMatchArm {
        self.push_scope();

        let written = arm.pattern().expect("the parser gives an arm its pattern");
        let discards_before = self.discarded_resources.len();
        let pattern = self.lower_pattern_with_expected(&written, expected);
        let discarded = self.discarded_resources.split_off(discards_before);

        let guard = arm.guard().map(|e| self.lower_expr(&e));

        let body = match arm.body() {
            Some(body) => {
                let body = self.lower_expr(&body);
                let mut owned = self.resource_pattern_bindings(&written);
                owned.extend(discarded);
                self.wrap_resource_scopes(body, owned)
            }
            // An arm without `->` (the parser gives any other its body)
            // stands for its pattern's value.
            None => self.lower_rebuilt_pattern(&written, result),
        };

        self.pop_scope();

        MirMatchArm {
            pattern,
            guard,
            body,
        }
    }

    /// A pass-through arm's value: its pattern rebuilt as `expected`, the type
    /// of the whole `case` (the checker rejected patterns that are not values).
    fn lower_rebuilt_pattern(&mut self, pat: &Pattern, expected: Option<&Ty>) -> MirExpr {
        let ty = expected.map_or(MirType::Unit, |ty| resolve_type(ty, self.registry));
        let (variant, fields) = match pat {
            Pattern::Ident(ident) => {
                let name = ident
                    .name()
                    .map(|t| t.text().to_string())
                    .unwrap_or_default();
                if !name.starts_with(|c: char| c.is_uppercase()) {
                    let scope_ty = self.lookup_var(&name).unwrap_or(ty);
                    return self.lower_local_ref(name, scope_ty, ident.syntax().text_range());
                }
                (name, Vec::new())
            }
            Pattern::Literal(lit) => {
                return match literal_pattern_value(lit) {
                    Some(MirLiteral::Int(value)) => MirExpr::IntLit(value, MirType::Int),
                    Some(MirLiteral::Float(value)) => MirExpr::FloatLit(value, MirType::Float),
                    Some(MirLiteral::Bool(value)) => MirExpr::BoolLit(value, MirType::Bool),
                    Some(MirLiteral::String(value)) => MirExpr::StringLit(value, MirType::String),
                    None => MirExpr::Unit,
                };
            }
            Pattern::Constructor(ctor) => {
                let variant = ctor
                    .variant_name()
                    .map(|t| t.text().to_string())
                    .unwrap_or_default();
                let type_name = self.constructor_type_name(ctor, &variant, expected);
                let field_types = self.variant_field_types(&type_name, &variant, expected);
                let fields = ctor
                    .fields()
                    .enumerate()
                    .map(|(index, field)| {
                        self.lower_rebuilt_pattern(&field, field_types.get(index))
                    })
                    .collect();
                (variant, fields)
            }
            _ => unreachable!("the type checker rejects a pattern that is not a value (E0056)"),
        };
        // The arm's value is the `case`'s type, a sum type's instance.
        let type_name = mir_type_to_impl_name(&ty);
        MirExpr::ConstructVariant {
            type_name: type_name.clone(),
            variant,
            fields,
            ty: MirType::SumType(type_name),
        }
    }

    /// The sum type a constructor pattern names: `Shape` in `Shape.Circle(r)`,
    /// otherwise the one `expected` or the registry says has the variant.
    fn constructor_type_name(
        &self,
        ctor: &mesh_parser::ast::pat::ConstructorPat,
        variant: &str,
        expected: Option<&Ty>,
    ) -> String {
        // A qualifier that is not a type is the module exporting it (`Geo.Dot`).
        if let Some(type_name) = ctor.type_name() {
            let type_name = type_name.text().to_string();
            if self.registry.sum_type_defs.contains_key(&type_name) {
                return type_name;
            }
        }
        let expected_mir = expected.map(|ty| resolve_type(ty, self.registry));
        find_type_for_variant(variant, expected_mir.as_ref(), self.registry, None)
            .unwrap_or_default()
    }

    /// The field types of `variant_name` in the sum type `type_name`, with the
    /// type arguments of `expected` substituted.
    fn variant_field_types(
        &self,
        type_name: &str,
        variant_name: &str,
        expected: Option<&Ty>,
    ) -> Vec<Ty> {
        self.registry
            .sum_type_defs
            .get(type_name)
            .and_then(|info| {
                let variant = info
                    .variants
                    .iter()
                    .find(|variant| variant.name == variant_name)?;
                let substitutions: HashMap<String, &Ty> = match expected.and_then(ty_head) {
                    Some((name, args)) if name == type_name => {
                        info.generic_params.iter().cloned().zip(args).collect()
                    }
                    _ => HashMap::new(),
                };
                Some(
                    variant
                        .fields
                        .iter()
                        .map(|field| {
                            let ty = match field {
                                mesh_typeck::VariantFieldInfo::Positional(ty)
                                | mesh_typeck::VariantFieldInfo::Named(_, ty) => ty,
                            };
                            substitute_type_params(ty, &substitutions)
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .unwrap_or_default()
    }

    // ── Pattern lowering ─────────────────────────────────────────────

    fn resource_pattern_bindings(&self, pattern: &Pattern) -> Vec<(String, Ty)> {
        pattern
            .binders()
            .into_iter()
            .filter_map(|name| {
                let ty = self.get_ty(name.parent()?.text_range())?.clone();
                self.registry
                    .is_resource_type(&ty)
                    .then(|| (name.text().to_string(), ty))
            })
            .collect()
    }

    /// The fields of struct type `ty`, in declaration order, with the type
    /// arguments of `ty` substituted: `Box<Int>`'s `value` is an `Int`.
    fn concrete_struct_fields(&self, ty: &Ty) -> Option<Vec<(String, Ty)>> {
        let (name, args) = ty_head(ty)?;
        let def = self.registry.struct_defs.get(name)?;
        let subst: HashMap<String, &Ty> = def.generic_params.iter().cloned().zip(args).collect();
        Some(
            def.fields
                .iter()
                .map(|(field, field_ty)| (field.clone(), substitute_type_params(field_ty, &subst)))
                .collect(),
        )
    }

    fn lower_pattern(&mut self, pat: &Pattern) -> MirPattern {
        self.lower_pattern_with_expected(pat, None)
    }

    fn lower_pattern_with_expected(&mut self, pat: &Pattern, expected: Option<&Ty>) -> MirPattern {
        match pat {
            // A `_` over a resource binds no name the program sees, but the
            // value is still owned here: it gets a name of its own.
            Pattern::Wildcard(wildcard) => match self
                .get_ty(wildcard.syntax().text_range())
                .filter(|ty| self.registry.is_resource_type(ty))
                .cloned()
            {
                Some(ty) => {
                    let name = format!("__discarded_{}", self.resource_temp_counter);
                    self.resource_temp_counter += 1;
                    let mir_ty = runtime_value_type(resolve_type(&ty, self.registry));
                    self.insert_var(name.clone(), mir_ty.clone());
                    self.discarded_resources.push((name.clone(), ty));
                    MirPattern::Var(name, mir_ty)
                }
                None => MirPattern::Wildcard,
            },

            Pattern::Ident(ident) => {
                let name = ident
                    .name()
                    .map(|t| t.text().to_string())
                    .unwrap_or_else(|| "_".to_string());

                // Check if this identifier is a known nullary constructor
                // (e.g., None, Less, Equal, Greater). The parser produces
                // IDENT_PAT for these because they lack parentheses, but
                // they must be lowered as Constructor patterns for correct
                // pattern matching codegen (switch on tag).
                let variant_type = name
                    .starts_with(|c: char| c.is_uppercase())
                    .then(|| {
                        let expected_mir = expected.map(|ty| resolve_type(ty, self.registry));
                        find_type_for_variant(&name, expected_mir.as_ref(), self.registry, None)
                    })
                    .flatten();
                if let Some(type_name) = variant_type {
                    let variant_fields = self
                        .registry
                        .sum_type_defs
                        .get(&type_name)
                        .and_then(|info| info.variants.iter().find(|v| v.name == name))
                        .map(|v| v.fields.len())
                        .unwrap_or(0);
                    // Nullary constructor: no fields.
                    // Payload-bearing constructor without explicit binder: treat as
                    // Constructor(_) -- wildcards cover all fields, bind nothing.
                    return MirPattern::Constructor {
                        type_name: self.pattern_sum_name(expected, type_name),
                        variant: name,
                        fields: vec![MirPattern::Wildcard; variant_fields],
                        bindings: vec![],
                    };
                }

                let ty = expected
                    .map(|ty| resolve_type(ty, self.registry))
                    .unwrap_or_else(|| self.resolve_range(ident.syntax().text_range()));
                let ty = if matches!(ty, MirType::Tuple(_)) {
                    MirType::Ptr
                } else {
                    ty
                };
                self.insert_var(name.clone(), ty.clone());
                MirPattern::Var(name, ty)
            }

            Pattern::Literal(lit) => literal_pattern_value(lit)
                .map(MirPattern::Literal)
                .unwrap_or(MirPattern::Wildcard),

            Pattern::Constructor(ctor) => {
                let variant_name = ctor
                    .variant_name()
                    .map(|t| t.text().to_string())
                    .unwrap_or_default();

                let type_name = self.constructor_type_name(ctor, &variant_name, expected);
                let expected_fields = self.variant_field_types(&type_name, &variant_name, expected);
                let fields: Vec<MirPattern> = ctor
                    .fields()
                    .enumerate()
                    .map(|(index, pattern)| {
                        self.lower_pattern_with_expected(&pattern, expected_fields.get(index))
                    })
                    .collect();

                // Collect bindings introduced by sub-patterns.
                let bindings = collect_pattern_bindings(&fields);

                MirPattern::Constructor {
                    type_name: self.pattern_sum_name(expected, type_name),
                    variant: variant_name,
                    fields,
                    bindings,
                }
            }

            Pattern::Tuple(tuple) => {
                let expected_elements = match expected {
                    Some(Ty::Tuple(elements)) => Some(elements.as_slice()),
                    _ => None,
                };
                let patterns = tuple
                    .patterns()
                    .enumerate()
                    .map(|(index, pattern)| {
                        self.lower_pattern_with_expected(
                            &pattern,
                            expected_elements.and_then(|elements| elements.get(index)),
                        )
                    })
                    .collect();
                MirPattern::Tuple(patterns)
            }

            Pattern::Struct(struct_pat) => {
                // The type checker types a struct pattern as its struct.
                let struct_ty = expected
                    .cloned()
                    .or_else(|| self.get_ty(struct_pat.syntax().text_range()).cloned())
                    .expect("the type checker types a struct pattern");
                let fields = self
                    .concrete_struct_fields(&struct_ty)
                    .expect("a struct pattern matches a struct");
                let name = mir_type_to_impl_name(&resolve_type(&struct_ty, self.registry));
                let fields = fields
                    .into_iter()
                    .map(|(field, field_ty)| {
                        let pattern = struct_pat
                            .fields()
                            .find(|f| f.name().is_some_and(|n| n.text() == field))
                            .and_then(|f| f.pattern())
                            .map(|sub| self.lower_pattern_with_expected(&sub, Some(&field_ty)))
                            .unwrap_or(MirPattern::Wildcard);
                        let mir_field_ty =
                            runtime_value_type(resolve_type(&field_ty, self.registry));
                        (field, mir_field_ty, pattern)
                    })
                    .collect();
                MirPattern::Struct { name, fields }
            }

            Pattern::Or(or) => {
                let alts: Vec<MirPattern> =
                    or.alternatives().map(|p| self.lower_pattern(&p)).collect();
                MirPattern::Or(alts)
            }

            Pattern::As(as_pat) => {
                // Layered pattern: bind name AND match inner pattern.
                let binding_name = as_pat
                    .binding_name()
                    .map(|t| t.text().to_string())
                    .unwrap_or_else(|| "_".to_string());
                // Tuples live on the heap: a binding to one is a pointer, as
                // for a plain variable pattern (`resolve_range` gives it).
                let ty = self.resolve_range(as_pat.syntax().text_range());
                self.insert_var(binding_name.clone(), ty.clone());
                let inner = as_pat
                    .pattern()
                    .expect("the parser gives an `as` its pattern");
                MirPattern::As {
                    name: binding_name,
                    ty,
                    inner: Box::new(self.lower_pattern_with_expected(&inner, expected)),
                }
            }

            Pattern::Cons(cons_pat) => {
                // List cons pattern: head :: tail. The head is matched as the
                // element type (so `Some(a) :: _` knows its payload type), the
                // tail as the list type.
                let list_src = self
                    .get_ty(cons_pat.syntax().text_range())
                    .cloned()
                    .or_else(|| expected.cloned());
                let elem_src = list_src.as_ref().and_then(list_elem);
                // A tuple element is a heap pointer, as everywhere. If the
                // list type is not resolved, Int is the default element type.
                let elem_mir_ty = elem_src
                    .as_ref()
                    .map(|ty| runtime_value_type(resolve_type(ty, self.registry)))
                    .unwrap_or(MirType::Int);

                let (head, tail) = cons_pat
                    .head()
                    .zip(cons_pat.tail())
                    .expect("the parser gives a cons pattern its head and tail");
                let head_pat = self.lower_pattern_with_expected(&head, elem_src.as_ref());
                let tail_pat = self.lower_pattern_with_expected(&tail, list_src.as_ref());

                MirPattern::ListCons {
                    head: Box::new(head_pat),
                    tail: Box::new(tail_pat),
                    elem_ty: elem_mir_ty,
                }
            }

            Pattern::List(list_pat) => {
                // `[a, b]` is `a :: b :: []`: cons cells ending in the empty list.
                let elem_src = self
                    .get_ty(list_pat.syntax().text_range())
                    .cloned()
                    .and_then(|ty| list_elem(&ty));
                let elem_mir_ty = elem_src
                    .as_ref()
                    .map(|ty| runtime_value_type(resolve_type(ty, self.registry)))
                    .unwrap_or(MirType::Int);
                list_pat
                    .patterns()
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .fold(MirPattern::ListNil, |tail, elem| MirPattern::ListCons {
                        head: Box::new(self.lower_pattern_with_expected(&elem, elem_src.as_ref())),
                        tail: Box::new(tail),
                        elem_ty: elem_mir_ty.clone(),
                    })
            }
        }
    }

    /// The sum type a constructor pattern matches: the instantiation it is
    /// matched as (`Option_Int`), else `type_name`.
    fn pattern_sum_name(&self, expected: Option<&Ty>, type_name: String) -> String {
        match expected.map(|ty| resolve_type(ty, self.registry)) {
            Some(MirType::SumType(name)) => name,
            _ => type_name,
        }
    }

    // ── Closure expression lowering (CLOSURE CONVERSION) ─────────────

    fn lower_closure_expr(&mut self, closure: &ClosureExpr) -> MirExpr {
        // Check for closures that match their arguments and dispatch accordingly.
        if closure.matches_arguments() {
            return self.lower_multi_clause_closure(closure);
        }

        let closure_fn_name = self.generated_fn_name("closure");

        let closure_ty = self
            .get_ty(closure.syntax().text_range())
            .cloned()
            .expect("the type checker types every closure");
        let (param_srcs, ret) = fun_parts(&closure_ty);
        let param_types: Vec<MirType> = param_srcs.iter().map(|p| self.binding_type(p)).collect();
        let return_type = self.binding_type(ret);

        // Extract parameter names.
        let mut param_names = Vec::new();
        if let Some(param_list) = closure.param_list() {
            for param in param_list.params() {
                let name = param
                    .name()
                    .map(|t| t.text().to_string())
                    .unwrap_or_else(|| "_".to_string());
                param_names.push(name);
            }
        }

        // Build params: env_ptr first, then user params.
        let mut fn_params = Vec::new();
        fn_params.push(("__env".to_string(), MirType::Ptr));

        fn_params.extend(param_names.iter().cloned().zip(param_types.iter().cloned()));

        // Determine captured variables by scanning the closure body.
        // Any variable referenced in the body that is not a parameter and
        // exists in the outer scope is a capture.
        let outer_vars = self.capturable_vars();

        let param_set: std::collections::HashSet<&str> =
            param_names.iter().map(|s| s.as_str()).collect();

        // Lower the body in a new scope with params.
        // Track closure's return type for ? operator desugaring (Phase 45).
        let prev_fn_return_type = self.current_fn_return_type.take();
        let prev_fn_return_typeck = self.current_fn_return_typeck.take();
        self.current_fn_return_type = Some(return_type.clone());
        self.current_fn_return_typeck = Some(ret.clone());

        self.push_scope();
        for (name, ty) in &fn_params {
            self.insert_var(name.clone(), ty.clone());
        }

        let body = self.lower_block(&closure.body().expect("the parser gives a closure its body"));

        self.pop_scope();

        // Restore previous function return type.
        self.current_fn_return_type = prev_fn_return_type;
        self.current_fn_return_typeck = prev_fn_return_typeck;

        // Find captured variables by scanning the lowered body for Var references
        // that match outer scope names and are not parameters.
        let mut captures: Vec<(String, MirType)> = Vec::new();
        let mut capture_exprs: Vec<MirExpr> = Vec::new();
        collect_free_vars(&body, &param_set, &outer_vars, &mut captures);
        for (name, ty) in &captures {
            capture_exprs.push(self.shaped_capture(closure.syntax(), name, ty));
        }

        // Create the lifted function.
        self.functions.push(MirFunction {
            name: closure_fn_name.clone(),
            params: fn_params,
            return_type: return_type.clone(),
            body,
            is_closure_fn: true,
            captures: captures.clone(),
            has_tail_calls: false,
        });

        // Create the MakeClosure expression.
        let mir_ty = MirType::Closure(param_types, Box::new(return_type));

        MirExpr::MakeClosure {
            fn_name: closure_fn_name,
            captures: capture_exprs,
            ty: mir_ty,
        }
    }

    /// Lower a multi-clause closure expression.
    ///
    /// Multi-clause closures like `fn 0 -> "zero" | n -> to_string(n) end` are
    /// desugared into a single-param closure whose body is a MirExpr::Match.
    /// For single-param multi-clause, uses Match directly on the param.
    /// For multi-param multi-clause, uses an if-else chain (same as named fn lowering).
    fn lower_multi_clause_closure(&mut self, closure: &ClosureExpr) -> MirExpr {
        let closure_fn_name = self.generated_fn_name("closure");

        let closure_ty = self
            .get_ty(closure.syntax().text_range())
            .cloned()
            .expect("the type checker types every closure");
        let (param_srcs, ret) = fun_parts(&closure_ty);
        let param_types: Vec<MirType> = param_srcs.iter().map(|p| self.binding_type(p)).collect();
        let return_type = self.binding_type(ret);

        // Create synthetic parameter names: __cparam_0, __cparam_1, etc.
        let params: Vec<(String, MirType)> = param_types
            .iter()
            .enumerate()
            .map(|(i, ty)| (format!("__cparam_{}", i), ty.clone()))
            .collect();

        // Build fn params: env_ptr first, then user params.
        let mut fn_params = Vec::new();
        fn_params.push(("__env".to_string(), MirType::Ptr));
        fn_params.extend(params.iter().cloned());

        // Collect outer vars for capture analysis.
        let outer_vars = self.capturable_vars();

        let param_names: Vec<String> = params.iter().map(|(n, _)| n.clone()).collect();
        let param_set: std::collections::HashSet<&str> =
            param_names.iter().map(|s| s.as_str()).collect();

        // Track closure's return type for ? operator desugaring (Phase 45).
        let prev_fn_return_type = self.current_fn_return_type.take();
        let prev_fn_return_typeck = self.current_fn_return_typeck.take();
        self.current_fn_return_type = Some(return_type.clone());
        self.current_fn_return_typeck = Some(ret.clone());

        // Build the body using match or if-else chain.
        self.push_scope();
        for (name, ty) in &fn_params {
            self.insert_var(name.clone(), ty.clone());
        }

        // The clauses are the arms of one match on the parameters (see
        // `lower_clause_match`).
        let clauses: Vec<(Option<ParamList>, Option<GuardClause>, Option<Block>)> =
            std::iter::once((closure.param_list(), closure.guard(), closure.body()))
                .chain(
                    closure
                        .clauses()
                        .map(|clause| (clause.param_list(), clause.guard(), clause.body())),
                )
                .collect();
        let mut arms = Vec::new();
        for (param_list, guard, block) in &clauses {
            self.push_scope();
            let pattern = self.clause_pattern(param_list.clone(), &params, param_srcs);
            let guard = guard
                .as_ref()
                .and_then(|gc| gc.expr())
                .map(|e| self.lower_expr(&e));
            let body =
                self.lower_block(block.as_ref().expect("the parser gives a clause its body"));
            self.pop_scope();
            arms.push(MirMatchArm {
                pattern,
                guard,
                body,
            });
        }
        let body = MirExpr::Match {
            scrutinee: Box::new(clause_scrutinee(&params)),
            arms,
            ty: return_type.clone(),
        };

        self.pop_scope();

        // Restore previous function return type.
        self.current_fn_return_type = prev_fn_return_type;
        self.current_fn_return_typeck = prev_fn_return_typeck;

        // Find captured variables.
        let mut captures: Vec<(String, MirType)> = Vec::new();
        let mut capture_exprs: Vec<MirExpr> = Vec::new();
        collect_free_vars(&body, &param_set, &outer_vars, &mut captures);
        for (name, ty) in &captures {
            capture_exprs.push(self.shaped_capture(closure.syntax(), name, ty));
        }

        // Create the lifted function.
        self.functions.push(MirFunction {
            name: closure_fn_name.clone(),
            params: fn_params,
            return_type: return_type.clone(),
            body,
            is_closure_fn: true,
            captures: captures.clone(),
            has_tail_calls: false,
        });

        // Create the MakeClosure expression.
        let mir_ty = MirType::Closure(param_types, Box::new(return_type));

        MirExpr::MakeClosure {
            fn_name: closure_fn_name,
            captures: capture_exprs,
            ty: mir_ty,
        }
    }

    // ── String expression lowering (INTERPOLATION DESUGARING) ────────

    fn lower_string_expr(&mut self, str_expr: &StringExpr) -> MirExpr {
        // Walk the STRING_EXPR node's children to find STRING_CONTENT and
        // INTERPOLATION segments.

        // Detect triple-quoted string from STRING_START token text (""" vs ")
        let is_triple = str_expr
            .syntax()
            .children_with_tokens()
            .filter_map(|c| c.into_token())
            .find(|t| t.kind() == SyntaxKind::STRING_START)
            .map(|t| t.text().starts_with("\"\"\""))
            .unwrap_or(false);

        // For triple-quoted strings, determine the trim level from the last STRING_CONTENT token.
        // The last STRING_CONTENT ends with "\n<indent>" where <indent> matches the closing """.
        let trim_level: usize = if is_triple {
            str_expr
                .syntax()
                .children_with_tokens()
                .filter_map(|c| c.into_token())
                .filter(|t| t.kind() == SyntaxKind::STRING_CONTENT)
                .last()
                .map(|t| {
                    let text = t.text().to_string();
                    // The last line of the last STRING_CONTENT is the closing indent line
                    text.split('\n')
                        .next_back()
                        .unwrap_or("")
                        .chars()
                        .take_while(|c| *c == ' ' || *c == '\t')
                        .count()
                })
                .unwrap_or(0)
        } else {
            0
        };

        let mut segments: Vec<MirExpr> = Vec::new();
        // Track whether the next STRING_CONTENT is the first one (for leading newline stripping)
        let mut is_first_content = is_triple;
        let children: Vec<_> = str_expr.syntax().children_with_tokens().collect();
        let last_content = children
            .iter()
            .rposition(|child| child.kind() == SyntaxKind::STRING_CONTENT);

        for (index, child) in children.iter().enumerate() {
            match child.kind() {
                SyntaxKind::STRING_CONTENT => {
                    // A heredoc's lines end in `\n` whatever the file's
                    // line endings (an escaped `\r\n` stays).
                    let raw_text = child
                        .as_token()
                        .map(|t| {
                            if is_triple {
                                unescape_string(&t.text().replace("\r\n", "\n"))
                            } else {
                                unescape_string(t.text())
                            }
                        })
                        .unwrap_or_default();

                    let text = if is_triple {
                        apply_heredoc_content(
                            raw_text,
                            is_first_content,
                            Some(index) == last_content,
                            trim_level,
                        )
                    } else {
                        raw_text
                    };
                    is_first_content = false;

                    if !text.is_empty() {
                        segments.push(MirExpr::StringLit(text, MirType::String));
                    }
                }
                SyntaxKind::INTERPOLATION => {
                    // After any interpolation, subsequent STRING_CONTENT is not first
                    is_first_content = false;
                    // INTERPOLATION node contains an expression child.
                    let node = child.as_node().expect("an interpolation is a node");
                    for expr in node.children().filter_map(Expr::cast) {
                        let typeck_ty = self.get_ty(expr.syntax().text_range()).cloned();
                        let lowered = self.lower_expr(&expr);
                        // Wrap in a to_string call based on the expression's type.
                        let converted = self.wrap_to_string(lowered, typeck_ty.as_ref());
                        segments.push(converted);
                    }
                }
                _ => {
                    // STRING_START, STRING_END, INTERPOLATION_START, INTERPOLATION_END:
                    // skip these tokens.
                }
            }
        }

        // If no segments, return empty string.
        if segments.is_empty() {
            return MirExpr::StringLit(String::new(), MirType::String);
        }

        // If single segment, return it directly.
        if segments.len() == 1 {
            return segments.pop().unwrap();
        }

        // Chain concat calls: concat(concat(seg0, seg1), seg2) ...
        let mut result = segments.remove(0);
        for seg in segments {
            result = MirExpr::Call {
                func: Box::new(MirExpr::Var(
                    "mesh_string_concat".to_string(),
                    MirType::FnPtr(
                        vec![MirType::String, MirType::String],
                        Box::new(MirType::String),
                    ),
                )),
                args: vec![result, seg],
                ty: MirType::String,
            };
        }

        result
    }

    /// Wrap an expression in a to_string runtime call based on its type.
    ///
    /// `typeck_ty` is the optional original typeck `Ty` for the expression,
    /// used to resolve collection element types for Display dispatch.
    fn wrap_to_string(&mut self, expr: MirExpr, typeck_ty: Option<&Ty>) -> MirExpr {
        if let Some(shown) = typeck_ty.and_then(|ty| self.display_by_type(&expr, ty, false)) {
            return shown;
        }
        let runtime = match expr.ty() {
            // Already a string; and a value that never comes into being
            // (`"#{panic(..)}"`) is never shown: the panic ends the string.
            MirType::String | MirType::Never => return expr,
            MirType::Unit => {
                return MirExpr::Block(
                    vec![expr, MirExpr::StringLit("()".to_string(), MirType::String)],
                    MirType::String,
                )
            }
            MirType::Int => "mesh_int_to_string".to_string(),
            MirType::Float => "mesh_float_to_string".to_string(),
            MirType::Bool => "mesh_bool_to_string".to_string(),
            // A struct or sum type: its Display impl's `to_string`, or else
            // its Debug `inspect`. With neither (a payload of a type that
            // derives Display, which the type checker lets through), a call
            // of an undefined `to_string`, which code generation reports if
            // the call is ever compiled.
            _ => {
                let type_name = mir_type_to_impl_name(expr.ty());
                let matching = self
                    .trait_registry
                    .find_method_traits("to_string", &mir_type_to_ty(expr.ty()));
                let debug = format!("Debug__inspect__{type_name}");
                match matching.first() {
                    Some(trait_name) => format!("{trait_name}__to_string__{type_name}"),
                    None if self.known_functions.contains_key(&debug) => debug,
                    None => "to_string".to_string(),
                }
            }
        };
        let param = expr.ty().clone();
        Self::call_named(&runtime, vec![param], vec![expr], MirType::String)
    }

    /// The display of `expr`, a List, Map or Set of type `ty`: its runtime
    /// function called with the callback that shows each element (and each
    /// value). A collection its type does not parameterize shows Ints.
    fn wrap_collection_to_string(&mut self, expr: &MirExpr, ty: &Ty, debug: bool) -> MirExpr {
        let (base_name, args) = ty_head(ty).expect("a collection type has a head");
        let arg = |i: usize| args.get(i).cloned().unwrap_or_else(Ty::int);
        let mut callbacks = vec![self.resolve_to_string_callback(&arg(0), debug)];
        let runtime = match base_name {
            "List" => "mesh_list_to_string",
            "Map" => {
                callbacks.push(self.resolve_to_string_callback(&arg(1), debug));
                "mesh_map_to_string"
            }
            // A Set, the last collection `display_by_type` shows here.
            _ => "mesh_set_to_string",
        };
        let fn_ptr_ty = MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr));
        let args: Vec<MirExpr> = std::iter::once(expr.clone())
            .chain(
                callbacks
                    .into_iter()
                    .map(|callback| MirExpr::Var(callback, fn_ptr_ty.clone())),
            )
            .collect();
        Self::call_named(
            runtime,
            vec![MirType::Ptr; args.len()],
            args,
            MirType::String,
        )
    }

    /// The callback a collection runtime function calls per element:
    /// `fn(slot) -> String` for the raw slot word of an element of type
    /// `elem_ty`. Scalars and strings have runtime functions; everything
    /// else decodes the slot and displays the value by its type.
    fn resolve_to_string_callback(&mut self, elem_ty: &Ty, debug: bool) -> String {
        match elem_ty {
            Ty::Con(con) if con.name == "Int" => "mesh_int_to_string".to_string(),
            Ty::Con(con) if con.name == "Bool" => "mesh_bool_to_string".to_string(),
            Ty::Con(con) if con.name == "String" && !debug => "mesh_string_to_string".to_string(),
            _ => {
                let name = format!(
                    "__{}_slot_{}",
                    if debug { "inspect" } else { "display" },
                    Self::ty_specialization_component(elem_ty)
                );
                if self.known_functions.contains_key(&name) {
                    return name;
                }
                self.known_functions.insert(
                    name.clone(),
                    MirType::FnPtr(vec![MirType::Int], Box::new(MirType::String)),
                );
                let value = self.decode_slot("__slot", elem_ty);
                let body = if debug {
                    self.debug_string(value, elem_ty)
                } else {
                    self.wrap_to_string(value, Some(elem_ty))
                };
                self.push_helper_fn(
                    &name,
                    vec![("__slot".to_string(), MirType::Int)],
                    MirType::String,
                    body,
                );
                name
            }
        }
    }

    // ── Equality, ordering and display decided by type ───────────────
    //
    // Lists, maps, sets, tuples, unit and instantiated generic sum types
    // (`Option<Int>`) have no nominal impl to call: their functions are
    // generated here, once per type, and compare or print by contents.

    fn push_helper_fn(
        &mut self,
        name: &str,
        params: Vec<(String, MirType)>,
        return_type: MirType,
        body: MirExpr,
    ) {
        self.functions.push(MirFunction {
            name: name.to_string(),
            params,
            return_type,
            body,
            is_closure_fn: false,
            captures: vec![],
            has_tail_calls: false,
        });
    }

    fn call_named(name: &str, params: Vec<MirType>, args: Vec<MirExpr>, ret: MirType) -> MirExpr {
        MirExpr::Call {
            func: Box::new(MirExpr::Var(
                name.to_string(),
                MirType::FnPtr(params, Box::new(ret.clone())),
            )),
            args,
            ty: ret,
        }
    }

    fn and_all(terms: Vec<MirExpr>) -> MirExpr {
        terms
            .into_iter()
            .reduce(|acc, term| MirExpr::BinOp {
                op: BinOp::And,
                lhs: Box::new(acc),
                rhs: Box::new(term),
                ty: MirType::Bool,
            })
            .unwrap_or(MirExpr::BoolLit(true, MirType::Bool))
    }

    fn concat_all(parts: Vec<MirExpr>) -> MirExpr {
        parts
            .into_iter()
            .reduce(|acc, part| {
                Self::call_named(
                    "mesh_string_concat",
                    vec![MirType::String, MirType::String],
                    vec![acc, part],
                    MirType::String,
                )
            })
            .unwrap_or(MirExpr::StringLit(String::new(), MirType::String))
    }

    /// Whether a payload of sum type `inner` inside sum type `owner` is
    /// stored boxed: the two are the same type, or each reaches the other.
    fn boxed_payload(&self, owner: &str, inner: &str) -> bool {
        let (owner, inner) = (sum_type_base(owner), sum_type_base(inner));
        owner == inner
            || (self
                .sum_reach
                .get(owner)
                .is_some_and(|set| set.contains(inner))
                && self
                    .sum_reach
                    .get(inner)
                    .is_some_and(|set| set.contains(owner)))
    }

    /// The MIR type a value of `ty` has once bound by a pattern or held in
    /// a variable: tuples are heap pointers.
    fn binding_type(&self, ty: &Ty) -> MirType {
        runtime_value_type(resolve_type(ty, self.registry))
    }

    /// The raw slot word in `var` decoded to a value of type `ty`.
    fn decode_slot(&self, var: &str, ty: &Ty) -> MirExpr {
        let slot = MirExpr::Var(var.to_string(), MirType::Int);
        match self.binding_type(ty) {
            MirType::Int | MirType::Unit | MirType::Never => slot,
            mir => Self::call_named("__mesh_uniform_decode", vec![MirType::Int], vec![slot], mir),
        }
    }

    /// The registry's sum type `base` instantiated at `args`: its mangled
    /// name and each variant's field types.
    fn sum_instantiation(
        &self,
        base: &str,
        args: &[Ty],
    ) -> Option<(String, Vec<(String, Vec<Ty>)>)> {
        let info = self.registry.sum_type_defs.get(base)?;
        let subst: HashMap<String, &Ty> = info
            .generic_params
            .iter()
            .cloned()
            .zip(args.iter())
            .collect();
        let variants = info
            .variants
            .iter()
            .map(|v| {
                let fields = v
                    .fields
                    .iter()
                    .map(|f| match f {
                        mesh_typeck::VariantFieldInfo::Positional(ty)
                        | mesh_typeck::VariantFieldInfo::Named(_, ty) => {
                            substitute_type_params(ty, &subst)
                        }
                    })
                    .collect();
                (v.name.clone(), fields)
            })
            .collect();
        Some((mangle_type_name(base, args, self.registry), variants))
    }

    /// Generate the trait functions the type checker grants an instantiated
    /// generic sum type (`Option<Int>`: Eq, Ord, Display, Debug) or struct
    /// (`Box<List<Int>>`), once, on first use.
    fn ensure_instantiation_traits(&mut self, ty: &Ty) {
        let (Ty::App(..), Some((type_name, args))) = (ty, ty_head(ty)) else {
            return;
        };
        if self
            .registry
            .struct_defs
            .get(type_name)
            .is_some_and(|info| !info.generic_params.is_empty())
        {
            self.ensure_monomorphized_struct_trait_fns(type_name, ty);
            return;
        }
        let Some((mangled, variants)) = self.sum_instantiation(type_name, args) else {
            return;
        };
        let helper = self.instantiation_helper_name(type_name, args);
        let known = |lowerer: &Self, prefix: &str| {
            lowerer
                .known_functions
                .contains_key(&format!("{prefix}{helper}"))
        };
        if self.trait_registry.has_impl("Eq", ty) && !known(self, "Eq__eq__") {
            self.generate_eq_sum_typed_as(&mangled, &helper, &variants);
        }
        if self.trait_registry.has_impl("Ord", ty) && !known(self, "Ord__lt__") {
            self.generate_ord_sum_typed(&mangled, &helper, &variants);
        }
        if self.trait_registry.has_impl("ToJson", ty) && !known(self, "ToJson__to_json__") {
            self.generate_to_json_sum_typed(&mangled, &helper, &variants);
            self.generate_from_json_sum_typed(&mangled, &helper, &variants);
            self.generate_from_json_string_wrapper(&helper);
        }
        if self.trait_registry.has_impl("Hash", ty) && !known(self, "Hash__hash__") {
            self.generate_hash_sum_typed(&mangled, &helper, &variants);
        }
        if self.trait_registry.has_impl("Display", ty) && !known(self, "Display__to_string__") {
            self.generate_display_sum_typed_as(&mangled, &helper, type_name, &variants, false);
        }
        if self.trait_registry.has_impl("Debug", ty) && !known(self, "Debug__inspect__") {
            self.generate_display_sum_typed_as(&mangled, &helper, type_name, &variants, true);
        }
    }

    /// The name the derived helpers (Eq, Ord, Display, Debug) of the generic
    /// instantiation `base<args>` go by. Its layout name (`mangle_type_name`)
    /// erases the arguments to MIR types, so `Option<List<Int>>` and
    /// `Option<List<String>>` share a layout, `Option_Ptr`; they must not
    /// share helpers, which compare and print the payloads by their types.
    fn instantiation_helper_name(&self, base: &str, args: &[Ty]) -> String {
        let layout = mangle_type_name(base, args, self.registry);
        let erased = args.iter().any(|arg| {
            matches!(
                runtime_value_type(resolve_type(arg, self.registry)),
                MirType::Ptr
            ) && !matches!(arg, Ty::Var(_))
        });
        if erased {
            let components: Vec<String> = args
                .iter()
                .map(|arg| Self::ty_specialization_component(&apply_default_unit(arg)))
                .collect();
            format!("{layout}__of__{}", components.join("__"))
        } else {
            layout
        }
    }

    /// `lhs == rhs` for values of type `ty`, compared by structure:
    /// collections, tuples and sum types by contents, nominal types by
    /// their `Eq`, primitives by the hardware. Each operand is evaluated once.
    fn eq_expr(&mut self, lhs: MirExpr, rhs: MirExpr, ty: &Ty) -> MirExpr {
        let hardware = |lhs, rhs| MirExpr::BinOp {
            op: BinOp::Eq,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
            ty: MirType::Bool,
        };
        let always = |lhs, rhs| {
            MirExpr::Block(
                vec![lhs, rhs, MirExpr::BoolLit(true, MirType::Bool)],
                MirType::Bool,
            )
        };
        let ptr3 = vec![MirType::Ptr, MirType::Ptr, MirType::Ptr];
        if is_pid(ty) {
            return hardware(lhs, rhs);
        }
        match ty {
            Ty::Tuple(elems) if elems.is_empty() => always(lhs, rhs),
            // A type nothing fixed (`None == None`, `Ok(1) == Ok(1)`'s error
            // type) has no values to tell apart.
            Ty::Var(_) => always(lhs, rhs),
            Ty::Tuple(elems) => {
                let f = self.tuple_eq_fn(elems);
                Self::call_named(
                    &f,
                    vec![MirType::Ptr, MirType::Ptr],
                    vec![lhs, rhs],
                    MirType::Bool,
                )
            }
            Ty::App(..) => {
                let (name, args) = ty_head(ty).expect("an applied type has a named head");
                match name {
                    "List" => {
                        let elem_eq = self.resolve_eq_callback(args.first().unwrap_or(&Ty::int()));
                        let callback = MirExpr::Var(
                            elem_eq,
                            MirType::FnPtr(
                                vec![MirType::Int, MirType::Int],
                                Box::new(MirType::Bool),
                            ),
                        );
                        Self::call_named(
                            "mesh_list_eq",
                            ptr3,
                            vec![lhs, rhs, callback],
                            MirType::Bool,
                        )
                    }
                    "Map" => {
                        let val_eq = self.resolve_eq_callback(args.get(1).unwrap_or(&Ty::int()));
                        let callback = MirExpr::Var(
                            val_eq,
                            MirType::FnPtr(
                                vec![MirType::Int, MirType::Int],
                                Box::new(MirType::Bool),
                            ),
                        );
                        match args.first().filter(|key| Self::key_needs_eq("map", key)) {
                            Some(key) => {
                                let (key_eq, key_hash) = self.key_callbacks(key);
                                Self::call_named(
                                    "mesh_map_eq_by",
                                    vec![MirType::Ptr; 5],
                                    vec![lhs, rhs, callback, key_eq, key_hash],
                                    MirType::Bool,
                                )
                            }
                            None => Self::call_named(
                                "mesh_map_eq",
                                ptr3,
                                vec![lhs, rhs, callback],
                                MirType::Bool,
                            ),
                        }
                    }
                    "Set" => match args.first() {
                        Some(elem) if Self::key_needs_eq("set", elem) => {
                            let (eq, hash) = self.key_callbacks(elem);
                            Self::call_named(
                                "mesh_set_eq_by",
                                vec![
                                    MirType::Ptr,
                                    MirType::Ptr,
                                    eq.ty().clone(),
                                    hash.ty().clone(),
                                ],
                                vec![lhs, rhs, eq, hash],
                                MirType::Bool,
                            )
                        }
                        _ => Self::call_named(
                            "mesh_set_eq",
                            vec![MirType::Ptr, MirType::Ptr],
                            vec![lhs, rhs],
                            MirType::Bool,
                        ),
                    },
                    _ => {
                        self.ensure_instantiation_traits(ty);
                        let f = format!("Eq__eq__{}", self.instantiation_helper_name(name, args));
                        // An imported type (`App(Point, [])`) has its helpers
                        // in its own module.
                        if self.known_functions.contains_key(&f)
                            || (args.is_empty() && self.trait_registry.has_impl("Eq", ty))
                        {
                            let params = vec![lhs.ty().clone(), rhs.ty().clone()];
                            Self::call_named(&f, params, vec![lhs, rhs], MirType::Bool)
                        } else {
                            hardware(lhs, rhs)
                        }
                    }
                }
            }
            Ty::Con(tc) => match tc.name.as_str() {
                // An atom is its name at run time.
                "Int" | "Float" | "Bool" | "String" | "Atom" => hardware(lhs, rhs),
                "Unit" => always(lhs, rhs),
                "Json" => Self::call_named(
                    "mesh_json_eq",
                    vec![MirType::Ptr, MirType::Ptr],
                    vec![lhs, rhs],
                    MirType::Bool,
                ),
                "List" => {
                    let callback = MirExpr::Var(
                        self.resolve_eq_callback(&Ty::int()),
                        MirType::FnPtr(vec![MirType::Int, MirType::Int], Box::new(MirType::Bool)),
                    );
                    Self::call_named(
                        "mesh_list_eq",
                        ptr3,
                        vec![lhs, rhs, callback],
                        MirType::Bool,
                    )
                }
                "Map" => {
                    let callback = MirExpr::Var(
                        self.resolve_eq_callback(&Ty::int()),
                        MirType::FnPtr(vec![MirType::Int, MirType::Int], Box::new(MirType::Bool)),
                    );
                    Self::call_named("mesh_map_eq", ptr3, vec![lhs, rhs, callback], MirType::Bool)
                }
                name => {
                    let f = format!("Eq__eq__{name}");
                    if self.known_functions.contains_key(&f)
                        || self.trait_registry.has_impl("Eq", ty)
                    {
                        let params = vec![lhs.ty().clone(), rhs.ty().clone()];
                        Self::call_named(&f, params, vec![lhs, rhs], MirType::Bool)
                    } else {
                        hardware(lhs, rhs)
                    }
                }
            },
            _ => hardware(lhs, rhs),
        }
    }

    /// `fn(a: Ptr, b: Ptr) -> Bool` comparing two tuples of `elems` element-wise.
    fn tuple_eq_fn(&mut self, elems: &[Ty]) -> String {
        let name = format!(
            "__eq_tuple_{}",
            Self::ty_specialization_component(&Ty::Tuple(elems.to_vec()))
        );
        if self.known_functions.contains_key(&name) {
            return name;
        }
        self.known_functions.insert(
            name.clone(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Bool)),
        );
        let (a_pats, b_pats, terms) =
            self.tuple_elementwise(elems, |lowerer, a, b, ty| lowerer.eq_expr(a, b, ty));
        let body = Self::destructure_both(a_pats, b_pats, Self::and_all(terms), MirType::Bool);
        self.push_helper_fn(
            &name,
            vec![
                ("__a".to_string(), MirType::Ptr),
                ("__b".to_string(), MirType::Ptr),
            ],
            MirType::Bool,
            body,
        );
        name
    }

    /// Tuple patterns binding `__a_i` / `__b_i` for two tuples of `elems`,
    /// and `combine` applied to each pair of bound elements.
    fn tuple_elementwise(
        &mut self,
        elems: &[Ty],
        combine: impl Fn(&mut Self, MirExpr, MirExpr, &Ty) -> MirExpr,
    ) -> (Vec<MirPattern>, Vec<MirPattern>, Vec<MirExpr>) {
        let tys: Vec<MirType> = elems.iter().map(|e| self.binding_type(e)).collect();
        let a_pats = tys
            .iter()
            .enumerate()
            .map(|(i, t)| MirPattern::Var(format!("__a_{i}"), t.clone()))
            .collect();
        let b_pats = tys
            .iter()
            .enumerate()
            .map(|(i, t)| MirPattern::Var(format!("__b_{i}"), t.clone()))
            .collect();
        let terms = elems
            .iter()
            .zip(&tys)
            .enumerate()
            .map(|(i, (elem, t))| {
                combine(
                    self,
                    MirExpr::Var(format!("__a_{i}"), t.clone()),
                    MirExpr::Var(format!("__b_{i}"), t.clone()),
                    elem,
                )
            })
            .collect();
        (a_pats, b_pats, terms)
    }

    /// `case __a do (a..) -> case __b do (b..) -> body end end`.
    fn destructure_both(
        a_pats: Vec<MirPattern>,
        b_pats: Vec<MirPattern>,
        body: MirExpr,
        ty: MirType,
    ) -> MirExpr {
        let inner = MirExpr::Match {
            scrutinee: Box::new(MirExpr::Var("__b".to_string(), MirType::Ptr)),
            arms: vec![MirMatchArm {
                pattern: MirPattern::Tuple(b_pats),
                guard: None,
                body,
            }],
            ty: ty.clone(),
        };
        MirExpr::Match {
            scrutinee: Box::new(MirExpr::Var("__a".to_string(), MirType::Ptr)),
            arms: vec![MirMatchArm {
                pattern: MirPattern::Tuple(a_pats),
                guard: None,
                body: inner,
            }],
            ty,
        }
    }

    /// `Eq__eq__{name}` for a sum type whose variants carry the given field
    /// types: the same variant with every field equal, each field compared
    /// by its own type (a `List<Int>` payload by contents, not by pointer).
    fn generate_eq_sum_typed(&mut self, name: &str, variants: &[(String, Vec<Ty>)]) {
        self.generate_eq_sum_typed_as(name, name, variants);
    }

    /// `generate_eq_sum_typed` for the sum type `name`, named for `helper`
    /// (see `instantiation_helper_name`).
    fn generate_eq_sum_typed_as(
        &mut self,
        name: &str,
        helper: &str,
        variants: &[(String, Vec<Ty>)],
    ) {
        let mangled = format!("Eq__eq__{helper}");
        let sum_ty = MirType::SumType(name.to_string());
        self.known_functions.insert(
            mangled.clone(),
            MirType::FnPtr(
                vec![sum_ty.clone(), sum_ty.clone()],
                Box::new(MirType::Bool),
            ),
        );
        let arms: Vec<MirMatchArm> = variants
            .iter()
            .map(|(variant, fields)| {
                let tys: Vec<MirType> = fields.iter().map(|f| self.binding_type(f)).collect();
                let vars = |prefix: &str| -> Vec<(String, MirType)> {
                    tys.iter()
                        .enumerate()
                        .map(|(i, t)| (format!("{prefix}_{i}"), t.clone()))
                        .collect()
                };
                let pattern = |bindings: &[(String, MirType)]| MirPattern::Constructor {
                    type_name: name.to_string(),
                    variant: variant.clone(),
                    fields: bindings
                        .iter()
                        .map(|(n, t)| MirPattern::Var(n.clone(), t.clone()))
                        .collect(),
                    bindings: bindings.to_vec(),
                };
                let (self_vars, other_vars) = (vars("self"), vars("other"));
                let terms = fields
                    .iter()
                    .zip(self_vars.iter().zip(&other_vars))
                    .map(|(field, ((a, t), (b, _)))| {
                        self.eq_expr(
                            MirExpr::Var(a.clone(), t.clone()),
                            MirExpr::Var(b.clone(), t.clone()),
                            field,
                        )
                    })
                    .collect();
                let same_variant = MirExpr::Match {
                    scrutinee: Box::new(MirExpr::Var("other".to_string(), sum_ty.clone())),
                    arms: vec![
                        MirMatchArm {
                            pattern: pattern(&other_vars),
                            guard: None,
                            body: Self::and_all(terms),
                        },
                        MirMatchArm {
                            pattern: MirPattern::Wildcard,
                            guard: None,
                            body: MirExpr::BoolLit(false, MirType::Bool),
                        },
                    ],
                    ty: MirType::Bool,
                };
                MirMatchArm {
                    pattern: pattern(&self_vars),
                    guard: None,
                    body: same_variant,
                }
            })
            .collect();
        let body = if arms.is_empty() {
            MirExpr::BoolLit(true, MirType::Bool)
        } else {
            MirExpr::Match {
                scrutinee: Box::new(MirExpr::Var("self".to_string(), sum_ty.clone())),
                arms,
                ty: MirType::Bool,
            }
        };
        self.push_helper_fn(
            &mangled,
            vec![
                ("self".to_string(), sum_ty.clone()),
                ("other".to_string(), sum_ty),
            ],
            MirType::Bool,
            body,
        );
    }

    /// `Eq__eq__{name}` for a struct: every field equal, by its own type.
    fn generate_eq_struct_typed(&mut self, name: &str, fields: &[(String, Ty)]) {
        self.generate_eq_struct_typed_as(name, name, fields);
    }

    /// `generate_eq_struct_typed` for the struct `name`, named for `helper`
    /// (see `instantiation_helper_name`).
    fn generate_eq_struct_typed_as(&mut self, name: &str, helper: &str, fields: &[(String, Ty)]) {
        let mangled = format!("Eq__eq__{helper}");
        let struct_ty = MirType::Struct(name.to_string());
        self.known_functions.insert(
            mangled.clone(),
            MirType::FnPtr(
                vec![struct_ty.clone(), struct_ty.clone()],
                Box::new(MirType::Bool),
            ),
        );
        let terms = fields
            .iter()
            .map(|(field, ty)| {
                let mir = runtime_value_type(resolve_type(ty, self.registry));
                let access = |object: &str| MirExpr::FieldAccess {
                    object: Box::new(MirExpr::Var(object.to_string(), struct_ty.clone())),
                    field: field.clone(),
                    ty: mir.clone(),
                };
                self.eq_expr(access("self"), access("other"), ty)
            })
            .collect();
        let body = Self::and_all(terms);
        self.push_helper_fn(
            &mangled,
            vec![
                ("self".to_string(), struct_ty.clone()),
                ("other".to_string(), struct_ty),
            ],
            MirType::Bool,
            body,
        );
    }

    /// `compare(lhs, rhs)` as an Int (negative, zero, positive) for values
    /// of type `ty`. The operands must be variables: they are read twice.
    fn cmp_expr(&mut self, lhs: MirExpr, rhs: MirExpr, ty: &Ty) -> MirExpr {
        let int = |n: i64| MirExpr::IntLit(n, MirType::Int);
        let three_way = |lt: MirExpr, gt: MirExpr| MirExpr::If {
            cond: Box::new(lt),
            then_body: Box::new(int(-1)),
            else_body: Box::new(MirExpr::If {
                cond: Box::new(gt),
                then_body: Box::new(int(1)),
                else_body: Box::new(int(0)),
                ty: MirType::Int,
            }),
            ty: MirType::Int,
        };
        let binop = |op, lhs: &MirExpr, rhs: &MirExpr| MirExpr::BinOp {
            op,
            lhs: Box::new(lhs.clone()),
            rhs: Box::new(rhs.clone()),
            ty: MirType::Bool,
        };
        let by_lt = |lowerer: &Self, f: String, lhs: MirExpr, rhs: MirExpr| {
            let params = vec![lhs.ty().clone(), rhs.ty().clone()];
            let ty = lowerer
                .known_functions
                .get(&f)
                .cloned()
                .unwrap_or(MirType::FnPtr(params, Box::new(MirType::Bool)));
            let lt = |a: &MirExpr, b: &MirExpr| MirExpr::Call {
                func: Box::new(MirExpr::Var(f.clone(), ty.clone())),
                args: vec![a.clone(), b.clone()],
                ty: MirType::Bool,
            };
            three_way(lt(&lhs, &rhs), lt(&rhs, &lhs))
        };
        match ty {
            Ty::Tuple(elems) if elems.is_empty() => int(0),
            // A type nothing fixed (`Ok(1) < Ok(2)`'s error type) has no
            // values to tell apart, as in `eq_expr`.
            Ty::Var(_) => MirExpr::Block(vec![lhs, rhs, int(0)], MirType::Int),
            Ty::Tuple(elems) => {
                let f = self.tuple_cmp_fn(elems);
                Self::call_named(
                    &f,
                    vec![MirType::Ptr, MirType::Ptr],
                    vec![lhs, rhs],
                    MirType::Int,
                )
            }
            Ty::App(..) => {
                let (name, args) = ty_head(ty).expect("an applied type has a named head");
                if name == "List" {
                    let elem_cmp =
                        self.resolve_compare_callback(args.first().unwrap_or(&Ty::int()));
                    let callback = MirExpr::Var(
                        elem_cmp,
                        MirType::FnPtr(vec![MirType::Int, MirType::Int], Box::new(MirType::Int)),
                    );
                    return Self::call_named(
                        "mesh_list_compare",
                        vec![MirType::Ptr, MirType::Ptr, MirType::Ptr],
                        vec![lhs, rhs, callback],
                        MirType::Int,
                    );
                }
                self.ensure_instantiation_traits(ty);
                let f = format!("Ord__lt__{}", self.instantiation_helper_name(name, args));
                if self.known_functions.contains_key(&f)
                    || (args.is_empty() && self.trait_registry.has_impl("Ord", ty))
                {
                    by_lt(self, f, lhs, rhs)
                } else {
                    three_way(binop(BinOp::Lt, &lhs, &rhs), binop(BinOp::Gt, &lhs, &rhs))
                }
            }
            Ty::Con(tc) => match tc.name.as_str() {
                "Int" | "Float" => {
                    three_way(binop(BinOp::Lt, &lhs, &rhs), binop(BinOp::Gt, &lhs, &rhs))
                }
                "String" => Self::call_named(
                    "mesh_string_compare",
                    vec![MirType::String, MirType::String],
                    vec![lhs, rhs],
                    MirType::Int,
                ),
                "Bool" => {
                    let not = |e: &MirExpr| MirExpr::UnaryOp {
                        op: UnaryOp::Not,
                        operand: Box::new(e.clone()),
                        ty: MirType::Bool,
                    };
                    let and = |a: MirExpr, b: MirExpr| MirExpr::BinOp {
                        op: BinOp::And,
                        lhs: Box::new(a),
                        rhs: Box::new(b),
                        ty: MirType::Bool,
                    };
                    three_way(and(not(&lhs), rhs.clone()), and(lhs.clone(), not(&rhs)))
                }
                "Unit" => int(0),
                name => {
                    let f = format!("Ord__lt__{name}");
                    if self.known_functions.contains_key(&f)
                        || self.trait_registry.has_impl("Ord", ty)
                    {
                        by_lt(self, f, lhs, rhs)
                    } else {
                        three_way(binop(BinOp::Lt, &lhs, &rhs), binop(BinOp::Gt, &lhs, &rhs))
                    }
                }
            },
            _ => three_way(binop(BinOp::Lt, &lhs, &rhs), binop(BinOp::Gt, &lhs, &rhs)),
        }
    }

    /// `hash(value)` for a value of type `ty`, agreeing with `eq_expr`:
    /// values equal by their type's Eq hash alike. A value with no hash
    /// that could (a function) hashes to a constant.
    fn hash_expr(&mut self, value: MirExpr, ty: &Ty) -> MirExpr {
        let by = |f: &str, arg: MirType, value: MirExpr| {
            Self::call_named(f, vec![arg], vec![value], MirType::Int)
        };
        let constant = |value: MirExpr| {
            MirExpr::Block(vec![value, MirExpr::IntLit(0, MirType::Int)], MirType::Int)
        };
        let (name, args) = match ty {
            Ty::Con(_) | Ty::App(..) => ty_head(ty).expect("a named type has a head"),
            Ty::Tuple(elems) if elems.is_empty() => return constant(value),
            Ty::Tuple(elems) => {
                let f = self.tuple_hash_fn(elems);
                return by(&f, MirType::Ptr, value);
            }
            _ => return constant(value),
        };
        let arg = |i: usize| args.get(i).cloned().unwrap_or_else(Ty::int);
        let callback = |lowerer: &mut Self, elem: &Ty| {
            MirExpr::Var(
                lowerer.resolve_hash_callback(elem),
                MirType::FnPtr(vec![MirType::Int], Box::new(MirType::Int)),
            )
        };
        match name {
            "Int" => by("mesh_hash_int", MirType::Int, value),
            "Float" => by("mesh_hash_float", MirType::Float, value),
            "Bool" => by("mesh_hash_bool", MirType::Bool, value),
            "String" | "Atom" => by("mesh_hash_string", MirType::String, value),
            "List" | "Set" => {
                let cb = callback(self, &arg(0));
                let f = if name == "List" {
                    "mesh_list_hash_by"
                } else {
                    "mesh_set_hash_by"
                };
                let params = vec![MirType::Ptr, cb.ty().clone()];
                Self::call_named(f, params, vec![value, cb], MirType::Int)
            }
            "Map" => {
                let (k, v) = (callback(self, &arg(0)), callback(self, &arg(1)));
                let params = vec![MirType::Ptr, k.ty().clone(), v.ty().clone()];
                Self::call_named("mesh_map_hash_by", params, vec![value, k, v], MirType::Int)
            }
            _ => {
                self.ensure_instantiation_traits(ty);
                let f = format!("Hash__hash__{}", self.instantiation_helper_name(name, args));
                if self.known_functions.contains_key(&f)
                    || (args.is_empty() && self.trait_registry.has_impl("Hash", ty))
                {
                    let param = value.ty().clone();
                    by(&f, param, value)
                } else {
                    constant(value)
                }
            }
        }
    }

    /// The `fn(slot) -> Int` callback `mesh_list_hash_by` and friends call
    /// per element: decodes the raw slot as `elem_ty` and hashes it.
    fn resolve_hash_callback(&mut self, elem_ty: &Ty) -> String {
        let name = format!("__hash_slot_{}", Self::ty_specialization_component(elem_ty));
        if self.known_functions.contains_key(&name) {
            return name;
        }
        self.known_functions.insert(
            name.clone(),
            MirType::FnPtr(vec![MirType::Int], Box::new(MirType::Int)),
        );
        let value = self.decode_slot("__slot", elem_ty);
        let body = self.hash_expr(value, elem_ty);
        self.push_helper_fn(
            &name,
            vec![("__slot".to_string(), MirType::Int)],
            MirType::Int,
            body,
        );
        name
    }

    /// `fn(t: Ptr) -> Int` hashing a tuple of `elems` element by element.
    fn tuple_hash_fn(&mut self, elems: &[Ty]) -> String {
        let name = format!(
            "__hash_tuple_{}",
            Self::ty_specialization_component(&Ty::Tuple(elems.to_vec()))
        );
        if self.known_functions.contains_key(&name) {
            return name;
        }
        self.known_functions.insert(
            name.clone(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Int)),
        );
        let tys: Vec<MirType> = elems.iter().map(|e| self.binding_type(e)).collect();
        let pats = tys
            .iter()
            .enumerate()
            .map(|(i, t)| MirPattern::Var(format!("__e_{i}"), t.clone()))
            .collect();
        let hashes = elems
            .iter()
            .zip(&tys)
            .enumerate()
            .map(|(i, (elem, t))| self.hash_expr(MirExpr::Var(format!("__e_{i}"), t.clone()), elem))
            .collect();
        let body = MirExpr::Match {
            scrutinee: Box::new(MirExpr::Var("__t".to_string(), MirType::Ptr)),
            arms: vec![MirMatchArm {
                pattern: MirPattern::Tuple(pats),
                guard: None,
                body: Self::combine_hashes(elems.len() as i64, hashes),
            }],
            ty: MirType::Int,
        };
        self.push_helper_fn(
            &name,
            vec![("__t".to_string(), MirType::Ptr)],
            MirType::Int,
            body,
        );
        name
    }

    /// `hashes` chained with `mesh_hash_combine`, starting from `seed`'s
    /// hash (a variant's index, a tuple's length).
    fn combine_hashes(seed: i64, hashes: Vec<MirExpr>) -> MirExpr {
        let start = Self::call_named(
            "mesh_hash_int",
            vec![MirType::Int],
            vec![MirExpr::IntLit(seed, MirType::Int)],
            MirType::Int,
        );
        hashes.into_iter().fold(start, |acc, hash| {
            Self::call_named(
                "mesh_hash_combine",
                vec![MirType::Int, MirType::Int],
                vec![acc, hash],
                MirType::Int,
            )
        })
    }

    /// Derived Hash for the struct `name`, named for `helper`: its fields
    /// hashed by their types, so it agrees with the derived Eq.
    fn generate_hash_struct_typed(&mut self, name: &str, helper: &str, fields: &[(String, Ty)]) {
        let mangled = format!("Hash__hash__{helper}");
        let struct_ty = MirType::Struct(name.to_string());
        self.known_functions.insert(
            mangled.clone(),
            MirType::FnPtr(vec![struct_ty.clone()], Box::new(MirType::Int)),
        );
        let hashes = fields
            .iter()
            .map(|(field, ty)| {
                let access = MirExpr::FieldAccess {
                    object: Box::new(MirExpr::Var("self".to_string(), struct_ty.clone())),
                    field: field.clone(),
                    ty: self.binding_type(ty),
                };
                self.hash_expr(access, ty)
            })
            .collect();
        let body = Self::combine_hashes(fields.len() as i64, hashes);
        self.push_helper_fn(
            &mangled,
            vec![("self".to_string(), struct_ty)],
            MirType::Int,
            body,
        );
    }

    /// Derived Hash for the sum type `name`, named for `helper`: the
    /// variant's index, then its payload hashed by its types.
    fn generate_hash_sum_typed(
        &mut self,
        name: &str,
        helper: &str,
        variants: &[(String, Vec<Ty>)],
    ) {
        let mangled = format!("Hash__hash__{helper}");
        let sum_ty = MirType::SumType(name.to_string());
        self.known_functions.insert(
            mangled.clone(),
            MirType::FnPtr(vec![sum_ty.clone()], Box::new(MirType::Int)),
        );
        let arms: Vec<MirMatchArm> = variants
            .iter()
            .enumerate()
            .map(|(tag, (variant, fields))| {
                let bindings: Vec<(String, MirType)> = fields
                    .iter()
                    .enumerate()
                    .map(|(i, f)| (format!("field_{i}"), self.binding_type(f)))
                    .collect();
                let hashes = fields
                    .iter()
                    .zip(&bindings)
                    .map(|(f, (var, t))| self.hash_expr(MirExpr::Var(var.clone(), t.clone()), f))
                    .collect();
                MirMatchArm {
                    pattern: MirPattern::Constructor {
                        type_name: name.to_string(),
                        variant: variant.clone(),
                        fields: bindings
                            .iter()
                            .map(|(n, t)| MirPattern::Var(n.clone(), t.clone()))
                            .collect(),
                        bindings,
                    },
                    guard: None,
                    body: Self::combine_hashes(tag as i64, hashes),
                }
            })
            .collect();
        let body = if arms.is_empty() {
            Self::combine_hashes(0, vec![])
        } else {
            MirExpr::Match {
                scrutinee: Box::new(MirExpr::Var("self".to_string(), sum_ty.clone())),
                arms,
                ty: MirType::Int,
            }
        };
        self.push_helper_fn(
            &mangled,
            vec![("self".to_string(), sum_ty)],
            MirType::Int,
            body,
        );
    }

    /// `compare(a, b)` on two values of type `ty`, as an `Ordering`.
    fn compare_call(&mut self, ty: &Ty, args: Vec<MirExpr>) -> MirExpr {
        let f = self.cmp_fn(ty);
        let operand = self.binding_type(ty);
        let cmp = Self::call_named(&f, vec![operand.clone(), operand], args, MirType::Int);
        let ordering_ty = MirType::SumType("Ordering".to_string());
        let ordering = |variant: &str| MirExpr::ConstructVariant {
            type_name: "Ordering".to_string(),
            variant: variant.to_string(),
            fields: vec![],
            ty: ordering_ty.clone(),
        };
        let c = || MirExpr::Var("__compared".to_string(), MirType::Int);
        let test = |op| MirExpr::BinOp {
            op,
            lhs: Box::new(c()),
            rhs: Box::new(MirExpr::IntLit(0, MirType::Int)),
            ty: MirType::Bool,
        };
        // A Let's type is its binding's; the block gives the whole its own.
        let result = MirExpr::Let {
            name: "__compared".to_string(),
            ty: MirType::Int,
            value: Box::new(cmp),
            body: Box::new(MirExpr::If {
                cond: Box::new(test(BinOp::Lt)),
                then_body: Box::new(ordering("Less")),
                else_body: Box::new(MirExpr::If {
                    cond: Box::new(test(BinOp::Eq)),
                    then_body: Box::new(ordering("Equal")),
                    else_body: Box::new(ordering("Greater")),
                    ty: ordering_ty.clone(),
                }),
                ty: ordering_ty.clone(),
            }),
        };
        MirExpr::Block(vec![result], ordering_ty)
    }

    /// `fn(a, b) -> Int` comparing two values of type `ty`; what a
    /// comparison operator on a non-primitive type calls.
    fn cmp_fn(&mut self, ty: &Ty) -> String {
        let name = format!("__cmp_{}", Self::ty_specialization_component(ty));
        if self.known_functions.contains_key(&name) {
            return name;
        }
        let param_ty = self.binding_type(ty);
        self.known_functions.insert(
            name.clone(),
            MirType::FnPtr(
                vec![param_ty.clone(), param_ty.clone()],
                Box::new(MirType::Int),
            ),
        );
        let body = self.cmp_expr(
            MirExpr::Var("__a".to_string(), param_ty.clone()),
            MirExpr::Var("__b".to_string(), param_ty.clone()),
            ty,
        );
        self.push_helper_fn(
            &name,
            vec![
                ("__a".to_string(), param_ty.clone()),
                ("__b".to_string(), param_ty),
            ],
            MirType::Int,
            body,
        );
        name
    }

    /// The lexicographic combination of the three-way comparisons `terms`
    /// (Ints): `c0 != 0 ? c0 : (c1 != 0 ? c1 : ... : c_last)`, 0 for none.
    fn lex_cmp(terms: Vec<MirExpr>) -> MirExpr {
        terms
            .into_iter()
            .enumerate()
            .rev()
            .fold(None, |rest: Option<MirExpr>, (i, term)| {
                Some(match rest {
                    None => term,
                    Some(rest) => {
                        let var = format!("__c_{i}");
                        MirExpr::Let {
                            name: var.clone(),
                            ty: MirType::Int,
                            value: Box::new(term),
                            body: Box::new(MirExpr::If {
                                cond: Box::new(MirExpr::BinOp {
                                    op: BinOp::NotEq,
                                    lhs: Box::new(MirExpr::Var(var.clone(), MirType::Int)),
                                    rhs: Box::new(MirExpr::IntLit(0, MirType::Int)),
                                    ty: MirType::Bool,
                                }),
                                then_body: Box::new(MirExpr::Var(var, MirType::Int)),
                                else_body: Box::new(rest),
                                ty: MirType::Int,
                            }),
                        }
                    }
                })
            })
            .unwrap_or(MirExpr::IntLit(0, MirType::Int))
    }

    /// `fn(a: Ptr, b: Ptr) -> Int` comparing two tuples lexicographically.
    fn tuple_cmp_fn(&mut self, elems: &[Ty]) -> String {
        let name = format!(
            "__cmp_tuple_{}",
            Self::ty_specialization_component(&Ty::Tuple(elems.to_vec()))
        );
        if self.known_functions.contains_key(&name) {
            return name;
        }
        self.known_functions.insert(
            name.clone(),
            MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Int)),
        );
        let (a_pats, b_pats, terms) =
            self.tuple_elementwise(elems, |lowerer, a, b, ty| lowerer.cmp_expr(a, b, ty));
        let body = Self::lex_cmp(terms);
        let body = Self::destructure_both(a_pats, b_pats, body, MirType::Int);
        self.push_helper_fn(
            &name,
            vec![
                ("__a".to_string(), MirType::Ptr),
                ("__b".to_string(), MirType::Ptr),
            ],
            MirType::Int,
            body,
        );
        name
    }

    /// The `fn(slot, slot) -> Bool` callback `mesh_list_eq` and friends call
    /// per element: decodes both raw slots as `elem_ty` and compares them.
    fn resolve_eq_callback(&mut self, elem_ty: &Ty) -> String {
        let name = format!("__eq_slot_{}", Self::ty_specialization_component(elem_ty));
        if self.known_functions.contains_key(&name) {
            return name;
        }
        self.known_functions.insert(
            name.clone(),
            MirType::FnPtr(vec![MirType::Int, MirType::Int], Box::new(MirType::Bool)),
        );
        let (a, b) = (
            self.decode_slot("__a", elem_ty),
            self.decode_slot("__b", elem_ty),
        );
        let body = self.eq_expr(a, b, elem_ty);
        self.push_helper_fn(
            &name,
            vec![
                ("__a".to_string(), MirType::Int),
                ("__b".to_string(), MirType::Int),
            ],
            MirType::Bool,
            body,
        );
        name
    }

    /// Whether a map or set with keys of type `key` compares them by the key
    /// type's Eq: keys that are not words, nor strings in a map (a map's tag
    /// can say its keys are strings; a set's cannot).
    fn key_needs_eq(collection: &str, key: &Ty) -> bool {
        let by_word = |name: &str| match name {
            "Int" | "Bool" | "Float" => true,
            "String" | "Atom" => collection == "map",
            _ => false,
        };
        !matches!(key, Ty::Var(_)) && !matches!(key, Ty::Con(tc) if by_word(&tc.name))
    }

    /// A typed wrapper for operation `op` of a map (`put`, `get`, ...) or set
    /// (`add`, `contains`, ...), of type `params -> ret` over keys (a set's
    /// elements) of type `key`, when the runtime needs to be told how to
    /// compare them: by the key type's Eq (`mesh_<collection>_<op>_by`), or,
    /// for a map built from a list or an iterator, as strings. `None` when the
    /// plain runtime function already compares correctly.
    fn resolve_table_by(
        &mut self,
        collection: &str,
        op: &str,
        params: &[Ty],
        ret: &Ty,
        key: &Ty,
        string: bool,
    ) -> Option<String> {
        let is_map = collection == "map";
        let by_eq = Self::key_needs_eq(collection, key);
        let builds = matches!(op, "from_list" | "collect");
        let supported = if is_map {
            matches!(
                op,
                "put" | "get" | "has_key" | "delete" | "merge" | "from_list" | "collect"
            )
        } else {
            matches!(
                op,
                "add"
                    | "remove"
                    | "contains"
                    | "union"
                    | "intersection"
                    | "difference"
                    | "from_list"
                    | "collect"
            )
        };
        if !supported || !(by_eq || (is_map && string && builds)) {
            return None;
        }
        let name = format!(
            "__{collection}_{op}_{}",
            Self::ty_specialization_component(&Ty::Fun(params.to_vec(), Box::new(ret.clone())))
        );
        if self.known_functions.contains_key(&name) {
            return Some(name);
        }
        let param_tys: Vec<MirType> = params.iter().map(|ty| self.binding_type(ty)).collect();
        let ret_ty = self.binding_type(ret);
        self.known_functions.insert(
            name.clone(),
            MirType::FnPtr(param_tys.clone(), Box::new(ret_ty.clone())),
        );
        // The key type's Eq and Hash (the runtime indexes large tables by the
        // hash); no callbacks: the runtime compares by the table's key type.
        let (key_eq, key_hash) = if by_eq {
            self.key_callbacks(key)
        } else {
            (
                MirExpr::IntLit(0, MirType::Ptr),
                MirExpr::IntLit(0, MirType::Ptr),
            )
        };
        let arg = |index: usize| MirExpr::Var(format!("__arg_{index}"), param_tys[index].clone());
        let slot = |index: usize| {
            Self::call_named(
                "__mesh_uniform_encode",
                vec![param_tys[index].clone()],
                vec![arg(index)],
                MirType::Int,
            )
        };
        let (mut args, mut arg_tys) = match op {
            "put" => (
                vec![arg(0), slot(1), slot(2)],
                vec![MirType::Ptr, MirType::Int, MirType::Int],
            ),
            "get" | "has_key" | "delete" | "add" | "remove" | "contains" => {
                (vec![arg(0), slot(1)], vec![MirType::Ptr, MirType::Int])
            }
            "merge" | "union" | "intersection" | "difference" => {
                (vec![arg(0), arg(1)], vec![MirType::Ptr, MirType::Ptr])
            }
            _ if is_map => (
                vec![
                    arg(0),
                    MirExpr::IntLit(if string { 1 } else { 0 }, MirType::Int),
                ],
                vec![MirType::Ptr, MirType::Int],
            ),
            _ => (vec![arg(0)], vec![MirType::Ptr]),
        };
        args.push(key_eq);
        arg_tys.push(MirType::Ptr);
        args.push(key_hash);
        arg_tys.push(MirType::Ptr);
        let raw_ret = match op {
            "get" => MirType::Int,
            "has_key" | "contains" => MirType::Bool,
            _ => MirType::Ptr,
        };
        // `get` is `Map.get`, which panics on a missing key.
        let runtime_op = if op == "get" { "fetch" } else { op };
        let call = Self::call_named(
            &format!("mesh_{collection}_{runtime_op}_by"),
            arg_tys,
            args,
            raw_ret,
        );
        let body = if op == "get" {
            match &ret_ty {
                MirType::Int | MirType::Unit | MirType::Never => call,
                _ => Self::call_named(
                    "__mesh_uniform_decode",
                    vec![MirType::Int],
                    vec![call],
                    ret_ty.clone(),
                ),
            }
        } else {
            call
        };
        let fn_params = param_tys
            .iter()
            .enumerate()
            .map(|(index, ty)| (format!("__arg_{index}"), ty.clone()))
            .collect();
        self.push_helper_fn(&name, fn_params, ret_ty, body);
        Some(name)
    }

    /// The `fn(slot, slot) -> Bool` Eq and `fn(slot) -> Int` Hash callbacks
    /// a table's `_by` runtime functions take for keys of type `key`.
    fn key_callbacks(&mut self, key: &Ty) -> (MirExpr, MirExpr) {
        (
            MirExpr::Var(
                self.resolve_eq_callback(key),
                MirType::FnPtr(vec![MirType::Int, MirType::Int], Box::new(MirType::Bool)),
            ),
            MirExpr::Var(
                self.resolve_hash_callback(key),
                MirType::FnPtr(vec![MirType::Int], Box::new(MirType::Int)),
            ),
        )
    }

    /// `Queue.pop` for elements of type `elem_ty`: `(front, rest)`, built
    /// here so the element takes a tuple field's form (an aggregate of one
    /// word inline), not the queue slot's (always boxed).
    fn resolve_queue_pop(&mut self, elem_ty: &Ty) -> String {
        let name = format!("__queue_pop_{}", Self::ty_specialization_component(elem_ty));
        if self.known_functions.contains_key(&name) {
            return name;
        }
        let elem_mir = self.binding_type(elem_ty);
        self.known_functions.insert(
            name.clone(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
        );
        let queue = || MirExpr::Var("__queue".to_string(), MirType::Ptr);
        // The rest first: it is what panics on an empty queue.
        let rest = Self::call_named(
            "mesh_queue_pop",
            vec![MirType::Ptr],
            vec![queue()],
            MirType::Ptr,
        );
        let front = Self::call_named(
            "mesh_queue_peek",
            vec![MirType::Ptr],
            vec![queue()],
            elem_mir,
        );
        let tuple = Self::call_named(
            "__mesh_make_tuple",
            vec![MirType::Int; 2],
            vec![front, MirExpr::Var("__rest".to_string(), MirType::Ptr)],
            MirType::Ptr,
        );
        let body = MirExpr::Let {
            name: "__rest".to_string(),
            ty: MirType::Ptr,
            value: Box::new(rest),
            body: Box::new(tuple),
        };
        self.push_helper_fn(
            &name,
            vec![("__queue".to_string(), MirType::Ptr)],
            MirType::Ptr,
            body,
        );
        name
    }

    /// `Channel.try_send` of a value of type `value_ty` that references heap
    /// objects: its slot goes with its shape, so the channel holds a copy
    /// the sender's collector cannot free. `None` for plain bits, which the
    /// runtime takes as they are.
    fn resolve_channel_send(&mut self, value_ty: &Ty) -> Option<String> {
        if Self::ty_contains_var(value_ty) {
            return None;
        }
        let value_mir = self.binding_type(value_ty);
        let shape = self.slot_shape(&value_mir, Some(value_ty))?;
        let name = format!(
            "__channel_try_send_{}",
            Self::ty_specialization_component(value_ty)
        );
        if self.known_functions.contains_key(&name) {
            return Some(name);
        }
        self.known_functions.insert(
            name.clone(),
            MirType::FnPtr(
                vec![MirType::Int, value_mir.clone()],
                Box::new(MirType::Ptr),
            ),
        );
        let slot = Self::call_named(
            "__mesh_uniform_encode",
            vec![value_mir.clone()],
            vec![MirExpr::Var("__value".to_string(), value_mir.clone())],
            MirType::Int,
        );
        let body = Self::call_named(
            "mesh_channel_try_send",
            vec![MirType::Int, MirType::Int],
            vec![
                MirExpr::Var("__channel".to_string(), MirType::Int),
                MirExpr::Shaped {
                    value: Box::new(slot),
                    shape,
                },
            ],
            MirType::Ptr,
        );
        self.push_helper_fn(
            &name,
            vec![
                ("__channel".to_string(), MirType::Int),
                ("__value".to_string(), value_mir),
            ],
            MirType::Ptr,
            body,
        );
        Some(name)
    }

    /// `List.contains` for elements of type `elem_ty`: the runtime scan
    /// compares each slot with the element's own Eq.
    fn resolve_list_contains(&mut self, elem_ty: &Ty) -> String {
        let name = format!(
            "__list_contains_{}",
            Self::ty_specialization_component(elem_ty)
        );
        if self.known_functions.contains_key(&name) {
            return name;
        }
        let elem_mir = self.binding_type(elem_ty);
        self.known_functions.insert(
            name.clone(),
            MirType::FnPtr(
                vec![MirType::Ptr, elem_mir.clone()],
                Box::new(MirType::Bool),
            ),
        );
        let eq = self.resolve_eq_callback(elem_ty);
        let slot = Self::call_named(
            "__mesh_uniform_encode",
            vec![elem_mir.clone()],
            vec![MirExpr::Var("__elem".to_string(), elem_mir.clone())],
            MirType::Int,
        );
        let body = Self::call_named(
            "mesh_list_contains_by",
            vec![
                MirType::Ptr,
                MirType::Int,
                MirType::FnPtr(vec![MirType::Int, MirType::Int], Box::new(MirType::Bool)),
            ],
            vec![
                MirExpr::Var("__list".to_string(), MirType::Ptr),
                slot,
                MirExpr::Var(
                    eq,
                    MirType::FnPtr(vec![MirType::Int, MirType::Int], Box::new(MirType::Bool)),
                ),
            ],
            MirType::Bool,
        );
        self.push_helper_fn(
            &name,
            vec![
                ("__list".to_string(), MirType::Ptr),
                ("__elem".to_string(), elem_mir),
            ],
            MirType::Bool,
            body,
        );
        name
    }

    /// The `fn(slot, slot) -> Int` callback `mesh_list_compare` calls per
    /// element.
    fn resolve_compare_callback(&mut self, elem_ty: &Ty) -> String {
        let name = format!("__cmp_slot_{}", Self::ty_specialization_component(elem_ty));
        if self.known_functions.contains_key(&name) {
            return name;
        }
        self.known_functions.insert(
            name.clone(),
            MirType::FnPtr(vec![MirType::Int, MirType::Int], Box::new(MirType::Int)),
        );
        let (a, b) = (
            self.decode_slot("__a", elem_ty),
            self.decode_slot("__b", elem_ty),
        );
        let body = self.cmp_expr(a, b, elem_ty);
        self.push_helper_fn(
            &name,
            vec![
                ("__a".to_string(), MirType::Int),
                ("__b".to_string(), MirType::Int),
            ],
            MirType::Int,
            body,
        );
        name
    }

    /// `expr` (of type `ty`) as a String when the type, not a nominal impl,
    /// decides how it prints: collections, tuples, unit and instantiated
    /// generic sum types. `debug` prefers the type's `inspect`.
    fn display_by_type(&mut self, expr: &MirExpr, ty: &Ty, debug: bool) -> Option<MirExpr> {
        if is_pid(ty) {
            return Some(Self::call_named(
                "mesh_pid_to_string",
                vec![expr.ty().clone()],
                vec![expr.clone()],
                MirType::String,
            ));
        }
        let unit = |expr: &MirExpr| {
            MirExpr::Block(
                vec![
                    expr.clone(),
                    MirExpr::StringLit("()".to_string(), MirType::String),
                ],
                MirType::String,
            )
        };
        match ty {
            Ty::Tuple(elems) if elems.is_empty() => Some(unit(expr)),
            Ty::Tuple(elems) => {
                let f = self.tuple_display_fn(elems, debug);
                Some(Self::call_named(
                    &f,
                    vec![MirType::Ptr],
                    vec![expr.clone()],
                    MirType::String,
                ))
            }
            Ty::App(..) => {
                let (name, args) = ty_head(ty).expect("an applied type has a named head");
                if matches!(name, "List" | "Map" | "Set") {
                    return Some(self.wrap_collection_to_string(expr, ty, debug));
                }
                self.ensure_instantiation_traits(ty);
                let mangled = self.instantiation_helper_name(name, args);
                let candidates = if debug {
                    [
                        format!("Debug__inspect__{mangled}"),
                        format!("Display__to_string__{mangled}"),
                    ]
                } else {
                    [
                        format!("Display__to_string__{mangled}"),
                        format!("Debug__inspect__{mangled}"),
                    ]
                };
                let imported = |trait_name: &str| {
                    args.is_empty() && self.trait_registry.has_impl(trait_name, ty)
                };
                candidates
                    .into_iter()
                    .find(|f| {
                        self.known_functions.contains_key(f)
                            || (f.starts_with("Display__") && imported("Display"))
                            || (f.starts_with("Debug__") && imported("Debug"))
                    })
                    .map(|f| {
                        Self::call_named(
                            &f,
                            vec![expr.ty().clone()],
                            vec![expr.clone()],
                            MirType::String,
                        )
                    })
            }
            Ty::Con(tc) if matches!(tc.name.as_str(), "List" | "Map" | "Set") => {
                Some(self.wrap_collection_to_string(expr, ty, debug))
            }
            Ty::Con(tc) if tc.name == "Unit" => Some(unit(expr)),
            Ty::Con(tc) if tc.name == "Json" => Some(Self::call_named(
                "mesh_json_encode",
                vec![MirType::Ptr],
                vec![expr.clone()],
                MirType::String,
            )),
            _ => None,
        }
    }

    /// The string `expr` as `inspect` shows it: quoted and escaped.
    fn inspect_string(expr: MirExpr) -> MirExpr {
        Self::call_named(
            "mesh_string_inspect",
            vec![MirType::String],
            vec![expr],
            MirType::String,
        )
    }

    /// `expr` as `inspect` would show it: strings quoted, otherwise the
    /// type's `Debug` when it has one, else its display.
    fn debug_string(&mut self, expr: MirExpr, ty: &Ty) -> MirExpr {
        if matches!(ty, Ty::Con(tc) if tc.name == "String") {
            return Self::inspect_string(expr);
        }
        if let Some(shown) = self.display_by_type(&expr, ty, true) {
            return shown;
        }
        let inspect = format!("Debug__inspect__{}", mir_type_to_impl_name(expr.ty()));
        if self.known_functions.contains_key(&inspect) {
            return Self::call_named(
                &inspect,
                vec![expr.ty().clone()],
                vec![expr],
                MirType::String,
            );
        }
        self.wrap_to_string(expr, Some(ty))
    }

    /// `fn(t: Ptr) -> String` printing a tuple of `elems` as `(a, b)`.
    fn tuple_display_fn(&mut self, elems: &[Ty], debug: bool) -> String {
        let name = format!(
            "__{}_tuple_{}",
            if debug { "inspect" } else { "display" },
            Self::ty_specialization_component(&Ty::Tuple(elems.to_vec()))
        );
        if self.known_functions.contains_key(&name) {
            return name;
        }
        self.known_functions.insert(
            name.clone(),
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::String)),
        );
        let tys: Vec<MirType> = elems.iter().map(|e| self.binding_type(e)).collect();
        let pats = tys
            .iter()
            .enumerate()
            .map(|(i, t)| MirPattern::Var(format!("__e_{i}"), t.clone()))
            .collect();
        let mut parts = vec![MirExpr::StringLit("(".to_string(), MirType::String)];
        for (i, (elem, t)) in elems.iter().zip(&tys).enumerate() {
            if i > 0 {
                parts.push(MirExpr::StringLit(", ".to_string(), MirType::String));
            }
            let value = MirExpr::Var(format!("__e_{i}"), t.clone());
            parts.push(if debug {
                self.debug_string(value, elem)
            } else {
                self.wrap_to_string(value, Some(elem))
            });
        }
        parts.push(MirExpr::StringLit(")".to_string(), MirType::String));
        let body = MirExpr::Match {
            scrutinee: Box::new(MirExpr::Var("__t".to_string(), MirType::Ptr)),
            arms: vec![MirMatchArm {
                pattern: MirPattern::Tuple(pats),
                guard: None,
                body: Self::concat_all(parts),
            }],
            ty: MirType::String,
        };
        self.push_helper_fn(
            &name,
            vec![("__t".to_string(), MirType::Ptr)],
            MirType::String,
            body,
        );
        name
    }

    /// `Display__to_string__{name}` (or, with `debug`, `Debug__inspect__{name}`)
    /// for a sum type whose variants carry the given field types:
    /// `Variant` or `Variant(field, ...)`, each field shown by its own type.
    fn generate_display_sum_typed(
        &mut self,
        name: &str,
        display_name: &str,
        variants: &[(String, Vec<Ty>)],
        debug: bool,
    ) {
        self.generate_display_sum_typed_as(name, name, display_name, variants, debug);
    }

    /// `generate_display_sum_typed` for the sum type `name`, named for
    /// `helper` (see `instantiation_helper_name`).
    fn generate_display_sum_typed_as(
        &mut self,
        name: &str,
        helper: &str,
        display_name: &str,
        variants: &[(String, Vec<Ty>)],
        debug: bool,
    ) {
        let mangled = if debug {
            format!("Debug__inspect__{helper}")
        } else {
            format!("Display__to_string__{helper}")
        };
        let sum_ty = MirType::SumType(name.to_string());
        self.known_functions.insert(
            mangled.clone(),
            MirType::FnPtr(vec![sum_ty.clone()], Box::new(MirType::String)),
        );
        let arms: Vec<MirMatchArm> = variants
            .iter()
            .map(|(variant, fields)| {
                let tys: Vec<MirType> = fields.iter().map(|f| self.binding_type(f)).collect();
                let bindings: Vec<(String, MirType)> = tys
                    .iter()
                    .enumerate()
                    .map(|(i, t)| (format!("field_{i}"), t.clone()))
                    .collect();
                let body = if fields.is_empty() {
                    MirExpr::StringLit(variant.clone(), MirType::String)
                } else {
                    let mut parts =
                        vec![MirExpr::StringLit(format!("{variant}("), MirType::String)];
                    for (i, (field, (var, t))) in fields.iter().zip(&bindings).enumerate() {
                        if i > 0 {
                            parts.push(MirExpr::StringLit(", ".to_string(), MirType::String));
                        }
                        let value = MirExpr::Var(var.clone(), t.clone());
                        parts.push(if debug {
                            self.debug_string(value, field)
                        } else {
                            self.wrap_to_string(value, Some(field))
                        });
                    }
                    parts.push(MirExpr::StringLit(")".to_string(), MirType::String));
                    Self::concat_all(parts)
                };
                MirMatchArm {
                    pattern: MirPattern::Constructor {
                        type_name: name.to_string(),
                        variant: variant.clone(),
                        fields: bindings
                            .iter()
                            .map(|(n, t)| MirPattern::Var(n.clone(), t.clone()))
                            .collect(),
                        bindings,
                    },
                    guard: None,
                    body,
                }
            })
            .collect();
        let body = if arms.is_empty() {
            MirExpr::StringLit(format!("<{display_name}>"), MirType::String)
        } else {
            MirExpr::Match {
                scrutinee: Box::new(MirExpr::Var("self".to_string(), sum_ty.clone())),
                arms,
                ty: MirType::String,
            }
        };
        self.push_helper_fn(
            &mangled,
            vec![("self".to_string(), sum_ty)],
            MirType::String,
            body,
        );
    }

    // ── Return expression lowering ───────────────────────────────────

    fn lower_return_expr(&mut self, ret: &ReturnExpr) -> MirExpr {
        let value = ret
            .value()
            .map(|e| self.lower_expr(&e))
            .unwrap_or(MirExpr::Unit);

        MirExpr::Return(Box::new(value))
    }

    // ── Try expression lowering (Phase 45) ─────────────────────────

    /// Desugar `expr?` to a match expression with early return.
    ///
    /// For `Result<T, E>`:
    /// ```text
    /// case expr do
    ///   Ok(__try_val_N) -> __try_val_N
    ///   Err(__try_err_N) -> return Err(__try_err_N)
    /// end
    /// ```
    ///
    /// For `Option<T>`:
    /// ```text
    /// case expr do
    ///   Some(__try_val_N) -> __try_val_N
    ///   None -> return None
    /// end
    /// ```
    fn lower_try_expr(&mut self, try_expr: &TryExpr) -> MirExpr {
        let operand_expr = try_expr
            .operand()
            .expect("the parser gives every `?` an operand");
        let operand_typeck = self.get_ty(operand_expr.syntax().text_range()).cloned();
        let operand = self.lower_expr(&operand_expr);
        // `expr?` has the success type the type checker gave it.
        let success_ty = runtime_value_type(self.resolve_range(try_expr.syntax().text_range()));
        self.lower_try(operand, operand_typeck, success_ty)
    }

    /// `?` on `operand`, a `Result` or an `Option` of the checked type
    /// `operand_typeck`: its value, of `success_ty`, or the function's early
    /// return of the `Err` (converted by `From` where the function returns
    /// another error type) or `None`.
    fn lower_try(
        &mut self,
        operand: MirExpr,
        operand_typeck: Option<Ty>,
        success_ty: MirType,
    ) -> MirExpr {
        let error_types = operand_typeck
            .as_ref()
            .and_then(Self::result_error_type)
            .cloned()
            .zip(
                self.current_fn_return_typeck
                    .as_ref()
                    .and_then(Self::result_error_type)
                    .cloned(),
            );
        self.try_counter += 1;
        let val_name = format!("__try_val_{}", self.try_counter);
        // The failure variant's binding, and the value its early return
        // carries: an `Err`'s error, a `None` nothing.
        let (type_name, [success, failure], error) = match operand.ty() {
            MirType::SumType(name) if sum_type_base(name) == "Result" => {
                let err_name = format!("__try_err_{}", self.try_counter);
                (
                    "Result",
                    ["Ok", "Err"],
                    Some(self.try_error(err_name, error_types)),
                )
            }
            // The type checker admits `?` on a Result or an Option.
            _ => ("Option", ["Some", "None"], None),
        };
        let (fields, bindings, returned) = match error {
            Some((name, ty, value)) => (
                vec![MirPattern::Var(name.clone(), ty.clone())],
                vec![(name, ty)],
                vec![value],
            ),
            None => (vec![], vec![], vec![]),
        };
        // An actor's body returns nothing: a failure there ends the actor.
        let fn_ret_ty = self.current_fn_return_type.clone().unwrap_or(MirType::Unit);
        MirExpr::Match {
            scrutinee: Box::new(operand),
            arms: vec![
                MirMatchArm {
                    pattern: MirPattern::Constructor {
                        type_name: type_name.to_string(),
                        variant: success.to_string(),
                        fields: vec![MirPattern::Var(val_name.clone(), success_ty.clone())],
                        bindings: vec![(val_name.clone(), success_ty.clone())],
                    },
                    guard: None,
                    body: MirExpr::Var(val_name, success_ty.clone()),
                },
                MirMatchArm {
                    pattern: MirPattern::Constructor {
                        type_name: type_name.to_string(),
                        variant: failure.to_string(),
                        fields,
                        bindings,
                    },
                    guard: None,
                    body: MirExpr::Return(Box::new(MirExpr::ConstructVariant {
                        type_name: type_name.to_string(),
                        variant: failure.to_string(),
                        fields: returned,
                        ty: fn_ret_ty,
                    })),
                },
            ],
            ty: success_ty,
        }
    }

    fn result_error_type(ty: &Ty) -> Option<&Ty> {
        match ty_head(ty)? {
            ("Result", [_, error]) => Some(error),
            _ => None,
        }
    }

    fn same_try_error_type(left: &Ty, right: &Ty) -> bool {
        if left == right {
            return true;
        }

        match (left, right) {
            (Ty::Con(con), Ty::App(app, args)) | (Ty::App(app, args), Ty::Con(con)) => {
                args.is_empty() && matches!(app.as_ref(), Ty::Con(app) if app == con)
            }
            _ => false,
        }
    }

    /// A failed `?`'s error, bound as `name`: its type in the binding, and
    /// the error the early return carries. That is the error itself, or,
    /// where the function returns another error type, the error converted
    /// by that type's `From` impl.
    fn try_error(&self, name: String, error_types: Option<(Ty, Ty)>) -> (String, MirType, MirExpr) {
        let Some((source, target)) =
            error_types.filter(|(operand, function)| !Self::same_try_error_type(operand, function))
        else {
            // A Result's error is a generic payload: a pointer.
            return (name.clone(), MirType::Ptr, MirExpr::Var(name, MirType::Ptr));
        };
        // A tuple is the pointer to its heap block, as `From` takes it.
        let source_ty = runtime_value_type(resolve_type(&source, self.registry));
        // A struct is a pointer to the heap, as the payload slot holds it.
        let target_ty = match resolve_type(&target, self.registry) {
            MirType::Struct(_) => MirType::Ptr,
            other => other,
        };
        let from_fn = mangle_trait_method(
            "From",
            &[trait_arg_name(&source)],
            "from",
            &trait_arg_name(&target),
        );
        let converted = MirExpr::Call {
            func: Box::new(MirExpr::Var(
                from_fn,
                MirType::FnPtr(vec![source_ty.clone()], Box::new(target_ty.clone())),
            )),
            args: vec![MirExpr::Var(name.clone(), source_ty.clone())],
            ty: target_ty,
        };
        (name, source_ty, converted)
    }

    // ── Tuple expression lowering ────────────────────────────────────

    fn lower_tuple_expr(&mut self, tuple: &TupleExpr) -> MirExpr {
        let elements: Vec<MirExpr> = tuple.elements().map(|e| self.lower_expr(&e)).collect();

        // Per decision 03-02: single-element tuple is grouping parens, not a tuple.
        if elements.len() == 1 {
            return elements.into_iter().next().unwrap();
        }

        if elements.is_empty() {
            return MirExpr::Unit;
        }

        // Multi-element tuple: generate a heap-allocated runtime tuple.
        // Runtime layout: { u64 len, u64[len] elements }
        // Allocate via mesh_gc_alloc_actor, store length + elements, return pointer.
        let n = elements.len();

        // Generate a synthetic __mesh_make_tuple(elem0, elem1, ...) call.
        // Codegen expands this inline: gc_alloc + store length + store elements.
        MirExpr::Call {
            func: Box::new(MirExpr::Var(
                "__mesh_make_tuple".to_string(),
                MirType::FnPtr(vec![MirType::Int; n], Box::new(MirType::Ptr)),
            )),
            args: elements,
            ty: MirType::Ptr,
        }
    }

    // ── Map literal lowering ────────────────────────────────────────

    /// Desugar `%{k1 => v1, k2 => v2}` to:
    ///   mesh_map_new_typed(key_type_tag)
    ///   |> mesh_map_put(_, k1, v1)
    ///   |> mesh_map_put(_, k2, v2)
    fn lower_map_literal(&mut self, map_lit: &MapLiteral) -> MirExpr {
        let key_type_tag = self.infer_map_key_type(map_lit.syntax().text_range());

        let new_typed_fn = MirExpr::Var(
            "mesh_map_new_typed".to_string(),
            MirType::FnPtr(vec![MirType::Int], Box::new(MirType::Ptr)),
        );
        let mut result = MirExpr::Call {
            func: Box::new(new_typed_fn),
            args: vec![MirExpr::IntLit(key_type_tag, MirType::Int)],
            ty: MirType::Ptr,
        };

        let put_fn_ty = MirType::FnPtr(
            vec![MirType::Ptr, MirType::Int, MirType::Int],
            Box::new(MirType::Ptr),
        );
        // Keys that are not words or strings go through `Map.put` for their
        // type, which compares them by the key type's Eq.
        let map_ty = self
            .get_ty(map_lit.syntax().text_range())
            .cloned()
            .expect("the type checker types a map literal");
        let [key, value] = collection_elems(&map_ty, "Map")
            .and_then(|elems| <[Ty; 2]>::try_from(elems).ok())
            .expect("a map literal's type is a Map of keys and values");
        let put_fn = self
            .resolve_table_by(
                "map",
                "put",
                &[map_ty.clone(), key.clone(), value],
                &map_ty,
                &key,
                false,
            )
            .map(|helper| {
                let ty = self.known_functions[&helper].clone();
                MirExpr::Var(helper, ty)
            })
            .unwrap_or_else(|| MirExpr::Var("mesh_map_put".to_string(), put_fn_ty.clone()));

        for entry in map_lit.entries() {
            // For keyword argument entries (name: value), the key is a NAME_REF
            // that should be treated as a string literal (the identifier text).
            let key = if entry.is_keyword_entry() {
                entry
                    .keyword_key_text()
                    .map(|text| MirExpr::StringLit(text, MirType::String))
                    .unwrap_or(MirExpr::Unit)
            } else {
                entry
                    .key()
                    .map(|e| self.lower_expr(&e))
                    .unwrap_or(MirExpr::Unit)
            };
            let val = entry
                .value()
                .map(|e| self.lower_expr(&e))
                .unwrap_or(MirExpr::Unit);

            result = MirExpr::Call {
                func: Box::new(put_fn.clone()),
                args: vec![result, key, val],
                ty: MirType::Ptr,
            };
        }

        result
    }

    // ── List literal lowering ────────────────────────────────────────

    /// Lower a list literal `[e1, e2, ...]` to MIR.
    ///
    /// For empty lists: calls mesh_list_new().
    /// For non-empty lists: creates a MirExpr::ListLit with lowered elements.
    /// The codegen will stack-allocate an array, store elements, and call
    /// mesh_list_from_array(arr_ptr, count).
    fn lower_list_literal(&mut self, list_lit: &ListLiteral) -> MirExpr {
        let elements: Vec<MirExpr> = list_lit.elements().map(|e| self.lower_expr(&e)).collect();

        if elements.is_empty() {
            // Empty list: call mesh_list_new()
            let fn_ty = MirType::FnPtr(vec![], Box::new(MirType::Ptr));
            return MirExpr::Call {
                func: Box::new(MirExpr::Var("mesh_list_new".to_string(), fn_ty)),
                args: vec![],
                ty: MirType::Ptr,
            };
        }

        MirExpr::ListLit {
            elements,
            ty: MirType::Ptr,
        }
    }

    // ── Struct literal lowering ──────────────────────────────────────

    fn lower_struct_literal(&mut self, sl: &StructLiteral) -> MirExpr {
        // The struct the literal builds: its type's, which differs from the
        // written name for a literal through an alias (`IntBox { .. }`).
        let base_name = self
            .get_ty(sl.syntax().text_range())
            .and_then(ty_head)
            .map(|(name, _)| name)
            .filter(|name| self.registry.struct_defs.contains_key(*name))
            .map(str::to_string)
            .or_else(|| sl.type_name())
            .unwrap_or_else(|| "<unnamed>".to_string());

        let fields: Vec<(String, MirExpr)> = sl
            .fields()
            .map(|f| {
                let field_name = f.name().and_then(|n| n.text()).unwrap_or_default();
                let value = f
                    .value()
                    .map(|e| self.lower_expr(&e))
                    .unwrap_or(MirExpr::Unit);
                (field_name, value)
            })
            .collect();

        let ty = self.resolve_range(sl.syntax().text_range());

        // A generic struct's literal has its instantiation's struct type
        // (`Box_Int`), whose layout and trait functions are made on first
        // use.
        let name = mir_type_to_impl_name(&ty);
        if name != base_name {
            let typeck_ty = self
                .get_ty(sl.syntax().text_range())
                .cloned()
                .expect("the type checker types a struct literal");
            self.ensure_monomorphized_struct_trait_fns(&base_name, &typeck_ty);
        }

        MirExpr::StructLit { name, fields, ty }
    }

    // ── Struct update lowering ────────────────────────────────────────

    fn lower_struct_update(&mut self, update: &StructUpdate) -> MirExpr {
        let base_expr = update.base_expr();
        let base_typeck = base_expr
            .as_ref()
            .and_then(|expression| self.get_ty(expression.syntax().text_range()))
            .cloned();
        let base = base_expr
            .map(|expression| self.lower_expr(&expression))
            .unwrap_or(MirExpr::Unit);

        let overrides: Vec<(String, MirExpr)> = update
            .override_fields()
            .iter()
            .map(|f| {
                let field_name = f.name().and_then(|n| n.text()).unwrap_or_default();
                let value = f
                    .value()
                    .map(|e| self.lower_expr(&e))
                    .unwrap_or(MirExpr::Unit);
                (field_name, value)
            })
            .collect();

        let override_indices = base_typeck
            .as_ref()
            .map(|base_ty| {
                overrides
                    .iter()
                    .map(|(field, _)| self.resource_field_index(base_ty, field))
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_default();
        let resource_overrides = base_typeck
            .as_ref()
            .and_then(|base_ty| self.resource_destructor(base_ty))
            .map(|destructor| destructor.fields().to_vec())
            .unwrap_or_default()
            .into_iter()
            .filter(|field| override_indices.contains(&field.index))
            .collect();

        let ty = self.resolve_range(update.syntax().text_range());

        MirExpr::StructUpdate {
            base: Box::new(base),
            overrides,
            resource_overrides,
            ty,
        }
    }

    // ── Actor definition lowering ──────────────────────────────────────

    fn lower_actor_def(&mut self, actor_def: &ActorDef) {
        let name = actor_def
            .name()
            .and_then(|n| n.text())
            .unwrap_or_else(|| "<anonymous_actor>".to_string());
        self.actors.push(name.clone());

        let actor_ty = self
            .get_ty(actor_def.syntax().text_range())
            .cloned()
            .expect("the type checker types every actor");
        let (param_tys, _) = fun_parts(&actor_ty);

        // Extract parameter names and types.
        let mut params = Vec::new();
        self.push_scope();

        if let Some(param_list) = actor_def.param_list() {
            for (param, param_ty) in param_list.params().zip(param_tys) {
                let param_name = param
                    .name()
                    .map(|t| t.text().to_string())
                    .unwrap_or_else(|| "_".to_string());
                let mir_ty = resolve_type(param_ty, self.registry);
                self.insert_var(param_name.clone(), mir_ty.clone());
                params.push((param_name, mir_ty));
            }
        }

        // Actor entry functions are called by the scheduler. They don't return
        // a value to the caller. The spawn expression returns the Pid.
        let return_type = MirType::Unit;

        let body_fn_name = format!("__actor_{}_body", name);
        let saved_target = self.actor_body_target.take();
        if !params.is_empty() {
            let param_tys = params.iter().map(|(_, ty)| ty.clone()).collect();
            self.actor_body_target = Some((name.clone(), body_fn_name.clone(), param_tys));
        }

        // Lower the actor body. The body contains a receive block that loops.
        let mut body = self.lower_block(
            &actor_def
                .body()
                .expect("the parser gives an actor its body"),
        );
        self.actor_body_target = saved_target;

        // Handle terminate clause: lower to a separate callback function.
        let terminate_callback_name = if let Some(term_clause) = actor_def.terminate_clause() {
            let cb_name = format!("__terminate_{}", name);
            let cb_body = self.lower_block(
                &term_clause
                    .body()
                    .expect("the parser gives a terminate clause its body"),
            );

            // Terminate callback signature: (state_ptr: Ptr, reason_ptr: Ptr) -> Unit
            self.functions.push(MirFunction {
                name: cb_name.clone(),
                params: vec![
                    ("state_ptr".to_string(), MirType::Ptr),
                    ("reason_ptr".to_string(), MirType::Ptr),
                ],
                return_type: MirType::Unit,
                body: cb_body,
                is_closure_fn: false,
                captures: Vec::new(),
                has_tail_calls: false,
            });

            Some(cb_name)
        } else {
            None
        };

        self.pop_scope();

        // Store the terminate callback name for use by spawn codegen.
        // We attach it as a known function and store a mapping.
        if let Some(ref cb_name) = terminate_callback_name {
            self.known_functions.insert(
                cb_name.clone(),
                MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Unit)),
            );
        }

        // For actors WITH parameters, generate a wrapper + body pair (Phase 93.2).
        // The runtime calls actor entry functions with signature `extern "C" fn(*const u8)`,
        // passing a pointer to a serialized args buffer. Actors with typed parameters need
        // a wrapper that accepts the raw pointer and deserializes args before calling the
        // actual actor body with typed values.
        if !params.is_empty() {
            // TCE: Rewrite self-recursive tail calls to TailCall nodes (Phase 48).
            // Self-calls were lowered as calls to the body function.
            let has_tail_calls = rewrite_tail_calls(&mut body, &body_fn_name);

            // 1. Push the body function with original typed params.
            self.functions.push(MirFunction {
                name: body_fn_name.clone(),
                params: params.clone(),
                return_type: return_type.clone(),
                body,
                is_closure_fn: false,
                captures: Vec::new(),
                has_tail_calls,
            });

            // Register the body function in known_functions so codegen can find it.
            let body_param_types: Vec<MirType> = params.iter().map(|(_, ty)| ty.clone()).collect();
            self.known_functions.insert(
                body_fn_name,
                MirType::FnPtr(body_param_types, Box::new(MirType::Unit)),
            );

            // 2. Push the wrapper function with Ptr param and Unit body.
            // Codegen detects this pattern (single __args_ptr param + matching __actor_*_body)
            // and generates the arg deserialization + body call.
            self.functions.push(MirFunction {
                name: name.clone(),
                params: vec![("__args_ptr".to_string(), MirType::Ptr)],
                return_type,
                body: MirExpr::Unit,
                is_closure_fn: false,
                captures: Vec::new(),
                has_tail_calls: false,
            });

            // Register the wrapper in known_functions with Ptr -> Unit signature
            // so that spawn references resolve correctly.
            self.known_functions.insert(
                name,
                MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Unit)),
            );
        } else {
            // For actors WITHOUT parameters, keep existing behavior unchanged.
            // The runtime passes null as args_ptr which is harmlessly ignored.

            // TCE: Rewrite self-recursive tail calls to TailCall nodes (Phase 48).
            let has_tail_calls = rewrite_tail_calls(&mut body, &name);

            self.functions.push(MirFunction {
                name,
                params,
                return_type,
                body,
                is_closure_fn: false,
                captures: Vec::new(),
                has_tail_calls,
            });
        }
    }

    // ── Supervisor lowering ─────────────────────────────────────────────

    fn lower_supervisor_def(&mut self, sup_def: &SupervisorDef) {
        let name = sup_def
            .name()
            .and_then(|n| n.text())
            .unwrap_or_else(|| "<anonymous_supervisor>".to_string());

        // The type checker allows only these strategies (E0019), restart
        // types (E0020) and shutdowns (E0021).
        let strategy: u8 = sup_def
            .strategy()
            .and_then(|node| {
                node.children_with_tokens()
                    .filter_map(|c| c.into_token())
                    .filter(|t| t.kind() == SyntaxKind::IDENT)
                    .last()
            })
            .map_or(0, |value| match value.text() {
                "one_for_one" => 0,
                "one_for_all" => 1,
                "rest_for_one" => 2,
                // simple_one_for_one, the last strategy the type checker admits.
                _ => 3,
            });

        // Extract max_restarts (default: 3).
        let max_restarts: u32 = sup_def
            .max_restarts()
            .and_then(|node| {
                node.children_with_tokens()
                    .filter_map(|c| c.into_token())
                    .find(|t| t.kind() == SyntaxKind::INT_LITERAL)
                    .and_then(|t| parse_int_literal(t.text()))
                    .and_then(|value| value.try_into().ok())
            })
            .unwrap_or(3);

        // Extract max_seconds (default: 5).
        let max_seconds: u64 = sup_def
            .max_seconds()
            .and_then(|node| {
                node.children_with_tokens()
                    .filter_map(|c| c.into_token())
                    .find(|t| t.kind() == SyntaxKind::INT_LITERAL)
                    .and_then(|t| parse_int_literal(t.text()))
                    .and_then(|value| value.try_into().ok())
            })
            .unwrap_or(5);

        let mut children = Vec::new();
        for child_node in sup_def.child_specs() {
            let child_id = child_node
                .children()
                .find(|c| c.kind() == SyntaxKind::NAME)
                .and_then(|n| {
                    n.children_with_tokens()
                        .filter_map(|c| c.into_token())
                        .find(|t| t.kind() == SyntaxKind::IDENT)
                        .map(|t| t.text().to_string())
                })
                .unwrap_or_else(|| "child".to_string());
            let block = child_node
                .children()
                .find_map(Block::cast)
                .expect("the parser gives a child spec its body");
            // `restart: <ident>` and `shutdown: <int or ident>`; `start`'s
            // value is an expression, not a token.
            let settings: Vec<_> = block
                .syntax()
                .children_with_tokens()
                .filter_map(|c| c.into_token())
                .filter(|t| matches!(t.kind(), SyntaxKind::IDENT | SyntaxKind::INT_LITERAL))
                .collect();
            let setting = |key: &str| {
                settings
                    .windows(2)
                    .rfind(|pair| pair[0].text() == key)
                    .map(|pair| pair[1].clone())
            };
            let restart_type = setting("restart").map_or(0, |value| match value.text() {
                "permanent" => 0,
                "transient" => 1,
                // temporary, the last restart type the type checker admits.
                _ => 2,
            });
            // A timeout in milliseconds, or 0 for `brutal_kill`.
            let shutdown_ms = setting("shutdown").map_or(5000, |value| match value.kind() {
                SyntaxKind::INT_LITERAL => {
                    parse_int_literal(value.text()).unwrap_or(i64::MAX) as u64
                }
                _ => 0,
            });
            let Some(start_fn) = self.supervisor_child_entry(&name, &child_id, &block) else {
                continue;
            };

            children.push(MirChildSpec {
                id: child_id,
                start_fn,
                restart_type,
                shutdown_ms,
                child_type: 0, // worker
            });
        }

        // Create a MIR function for the supervisor.
        // The supervisor's body is a SupervisorStart expression.
        let body = MirExpr::SupervisorStart {
            name: name.clone(),
            strategy,
            max_restarts,
            max_seconds,
            children,
            ty: MirType::Pid(None),
        };

        self.functions.push(MirFunction {
            name,
            params: vec![],
            return_type: MirType::Pid(None),
            body,
            is_closure_fn: false,
            captures: Vec::new(),
            has_tail_calls: false,
        });
    }

    /// The function a supervisor runs as its child `child`'s process, at the
    /// start and at each restart: the child's `start` function's body, with
    /// the `spawn` that ends it running the spawned actor in place. The
    /// runtime starts a child without arguments, so an actor's arguments
    /// are evaluated here, in the child.
    fn supervisor_child_entry(
        &mut self,
        supervisor: &str,
        child: &str,
        block: &Block,
    ) -> Option<String> {
        let body = match block.syntax().children().find_map(Expr::cast) {
            Some(Expr::ClosureExpr(start)) => start.body(),
            _ => None,
        };
        let spawn = body.as_ref().and_then(Block::tail_expr);
        let (Some(body), Some(Expr::SpawnExpr(spawn))) = (body, spawn) else {
            self.lowering_errors.push(format!(
                "the child `{child}` of supervisor `{supervisor}` must start as \
                 `fn -> spawn(actor, ...) end`"
            ));
            return None;
        };
        let entry = format!("__supervisor_{supervisor}_{child}");
        let outer = self.supervised_spawn.replace(spawn.syntax().text_range());
        self.push_scope();
        let body = self.lower_block(&body);
        self.pop_scope();
        self.supervised_spawn = outer;
        self.push_helper_fn(
            &entry,
            vec![("__args_ptr".to_string(), MirType::Ptr)],
            MirType::Unit,
            body,
        );
        Some(entry)
    }

    /// `spawn(actor, args...)` ending a supervisor child's start: the actor's
    /// body run with the arguments, in the process the supervisor started.
    fn run_actor_in_place(&mut self, spawn: &SpawnExpr) -> MirExpr {
        let mut args = spawn
            .arg_list()
            .map(|list| list.args().collect::<Vec<_>>())
            .unwrap_or_default()
            .into_iter();
        let actor = args.next().and_then(|actor| match actor {
            Expr::NameRef(actor) => Some((actor.text()?, actor.syntax().text_range())),
            _ => None,
        });
        let Some((actor, range)) = actor else {
            self.lowering_errors
                .push("a supervisor child must spawn an actor by its name".to_string());
            return MirExpr::Unit;
        };
        let (params, _) = fun_parts(
            self.get_ty(range)
                .expect("the type checker types the spawned actor"),
        );
        let params: Vec<MirType> = params
            .iter()
            .map(|ty| runtime_value_type(resolve_type(ty, self.registry)))
            .collect();
        // An actor with parameters runs as its body function.
        let callee = if params.is_empty() {
            actor
        } else {
            format!("__actor_{actor}_body")
        };
        let args = args.map(|arg| self.lower_expr(&arg)).collect();
        MirExpr::Call {
            func: Box::new(MirExpr::Var(
                callee,
                MirType::FnPtr(params, Box::new(MirType::Unit)),
            )),
            args,
            ty: MirType::Unit,
        }
    }

    // ── Service lowering ─────────────────────────────────────────────────

    fn lower_service_def(&mut self, service_def: &ServiceDef) {
        let name = service_def
            .name()
            .and_then(|n| n.text())
            .unwrap_or_else(|| "<anonymous_service>".to_string());

        let name_lower = name.to_lowercase();

        // Collect handler info from the AST.
        let call_handlers = service_def.call_handlers();
        let cast_handlers = service_def.cast_handlers();

        // Assign sequential type tags.
        // Call handlers: tags 0, 1, 2, ...
        // Cast handlers: tags N, N+1, N+2, ... (where N = call_handlers.len())
        let num_calls = call_handlers.len();

        // ── Collect handler info ─────────────────────────────────────────

        struct CallInfo {
            snake_name: String,
            tag: u64,
            param_names: Vec<String>,
            param_types: Vec<MirType>,
            /// Shapes of the arguments and the reply, which cross between the
            /// caller and the service actor.
            param_shapes: Vec<MsgShape>,
            reply_shape: MsgShape,
            state_param: Option<String>,
            /// The MIR type of the reply value (second element of the handler's
            /// return tuple). Used to set the correct return type on the call
            /// helper function so the codegen can properly convert the reply
            /// from its tuple-encoded i64 representation.
            reply_type: MirType,
        }

        struct CastInfo {
            snake_name: String,
            tag: u64,
            param_names: Vec<String>,
            param_types: Vec<MirType>,
            param_shapes: Vec<MsgShape>,
            state_param: Option<String>,
        }

        let mut call_infos = Vec::new();
        for (i, handler) in call_handlers.iter().enumerate() {
            let variant_name = handler
                .name()
                .and_then(|n| n.text())
                .unwrap_or_else(|| format!("call_{}", i));
            let (param_names, param_types, param_shapes) = self.handler_params(handler.params());
            // The handler returns (state, reply); only the reply leaves the
            // service, typed as the type checker's (state, reply) says.
            let reply = handler
                .body()
                .and_then(|block| block.tail_expr())
                .and_then(|expr| self.get_ty(expr.syntax().text_range()))
                .and_then(|ty| ty.parts().nth(1).cloned())
                .expect("the type checker gives a call handler a (state, reply) body");
            call_infos.push(CallInfo {
                snake_name: to_snake_case(&variant_name),
                tag: i as u64,
                param_names,
                param_types,
                param_shapes,
                reply_shape: self.msg_shape(&reply, &mut Vec::new()),
                state_param: handler.state_param_name(),
                reply_type: self.binding_type(&reply),
            });
        }

        let mut cast_infos = Vec::new();
        for (i, handler) in cast_handlers.iter().enumerate() {
            let variant_name = handler
                .name()
                .and_then(|n| n.text())
                .unwrap_or_else(|| format!("cast_{}", i));
            let (param_names, param_types, param_shapes) = self.handler_params(handler.params());
            cast_infos.push(CastInfo {
                snake_name: to_snake_case(&variant_name),
                tag: (num_calls + i) as u64,
                param_names,
                param_types,
                param_shapes,
                state_param: handler.state_param_name(),
            });
        }

        // ── Generate init function ───────────────────────────────────────
        // Lower the init function body to get initial state.
        let mut init_params = Vec::new();
        // Without an `init`, the state is 0.
        let init_body = if let Some(init_fn) = service_def.init_fn() {
            self.push_scope();
            let init_ty = self
                .get_ty(init_fn.syntax().text_range())
                .cloned()
                .expect("the type checker types a service's init");
            if let Some(param_list) = init_fn.param_list() {
                for (param, param_ty) in param_list.params().zip(fun_parts(&init_ty).0) {
                    let param_name = param
                        .name()
                        .map(|t| t.text().to_string())
                        .unwrap_or_else(|| "_".to_string());
                    let mir_ty = resolve_type(param_ty, self.registry);
                    self.insert_var(param_name.clone(), mir_ty.clone());
                    init_params.push((param_name, mir_ty));
                }
            }
            let body = self.lower_fn_body(&init_fn);
            self.pop_scope();
            body
        } else {
            MirExpr::IntLit(0, MirType::Int)
        };

        let init_fn_name = format!("__service_{}_init", name_lower);
        let (init_body, init_ret_ty) = service_state(init_body);
        self.functions.push(MirFunction {
            name: init_fn_name.clone(),
            params: init_params.clone(),
            return_type: init_ret_ty.clone(),
            body: init_body,
            is_closure_fn: false,
            captures: Vec::new(),
            has_tail_calls: false,
        });
        self.known_functions.insert(
            init_fn_name.clone(),
            MirType::FnPtr(
                init_params.iter().map(|(_, t)| t.clone()).collect(),
                Box::new(init_ret_ty.clone()),
            ),
        );

        // ── Generate handler body functions ──────────────────────────────
        // Each handler becomes a function:
        //   __service_{name}_handle_call_{snake}(state: i64, args...) -> i64 (for call: returns tuple-encoded {new_state, reply})
        //   __service_{name}_handle_cast_{snake}(state: i64, args...) -> i64 (for cast: returns new_state)

        for (i, handler) in call_handlers.iter().enumerate() {
            let info = &call_infos[i];
            let handler_fn_name =
                format!("__service_{}_handle_call_{}", name_lower, info.snake_name);

            self.push_scope();

            // State param: use the actual init return type (e.g. Int for PoolHandle, Struct for WriterState).
            let state_param_name = info
                .state_param
                .clone()
                .unwrap_or_else(|| "state".to_string());
            self.insert_var(state_param_name.clone(), init_ret_ty.clone());
            let mut params = vec![(state_param_name, init_ret_ty.clone())];

            for (p_name, mir_ty) in info.param_names.iter().zip(&info.param_types) {
                self.insert_var(p_name.clone(), mir_ty.clone());
                params.push((p_name.clone(), mir_ty.clone()));
            }

            // Lower handler body. Body returns (new_state, reply).
            let body =
                self.lower_block(&handler.body().expect("the parser gives a handler its body"));

            self.pop_scope();

            // Call handler body returns a heap-allocated tuple (new_state, reply).
            // The return type is ALWAYS Ptr since __mesh_make_tuple returns a pointer.
            // Note: body.ty() may not report Ptr when the body is wrapped in Let
            // bindings (Let.ty is the binding's value type, not the body's final type).
            let ret_ty = MirType::Ptr;
            self.functions.push(MirFunction {
                name: handler_fn_name.clone(),
                params,
                return_type: ret_ty.clone(),
                body,
                is_closure_fn: false,
                captures: Vec::new(),
                has_tail_calls: false,
            });
            self.known_functions
                .insert(handler_fn_name, MirType::FnPtr(vec![], Box::new(ret_ty)));
        }

        for (i, handler) in cast_handlers.iter().enumerate() {
            let info = &cast_infos[i];
            let handler_fn_name =
                format!("__service_{}_handle_cast_{}", name_lower, info.snake_name);

            self.push_scope();

            let state_param_name = info
                .state_param
                .clone()
                .unwrap_or_else(|| "state".to_string());
            self.insert_var(state_param_name.clone(), init_ret_ty.clone());
            let mut params = vec![(state_param_name, init_ret_ty.clone())];

            for (p_name, mir_ty) in info.param_names.iter().zip(&info.param_types) {
                self.insert_var(p_name.clone(), mir_ty.clone());
                params.push((p_name.clone(), mir_ty.clone()));
            }

            // Lower handler body. Body returns new_state.
            let body =
                self.lower_block(&handler.body().expect("the parser gives a handler its body"));

            self.pop_scope();

            // Cast handler returns new state.
            let (body, cast_ret_ty) = service_state(body);
            self.functions.push(MirFunction {
                name: handler_fn_name.clone(),
                params,
                return_type: cast_ret_ty.clone(),
                body,
                is_closure_fn: false,
                captures: Vec::new(),
                has_tail_calls: false,
            });
            self.known_functions.insert(
                handler_fn_name,
                MirType::FnPtr(vec![], Box::new(cast_ret_ty)),
            );
        }

        // The service runs as the loop actor: code generation writes its body
        // (receive, dispatch on the message's tag to a handler, reply to a
        // call, loop with the new state) from the service's dispatch table.
        let loop_fn_name = format!("__service_{}_loop", name_lower);

        // Track methods for this service so field access can resolve them.
        let mut methods = Vec::new();

        // Start function.
        let start_fn_name = format!("__service_{}_start", name_lower);
        methods.push(("start".to_string(), start_fn_name.clone()));

        // Call helper functions.
        for info in &call_infos {
            let fn_name = format!("__service_{}_call_{}", name_lower, info.snake_name);
            methods.push((info.snake_name.clone(), fn_name.clone()));
            let mut fn_param_types = vec![MirType::Pid(None)];
            fn_param_types.extend(info.param_types.iter().cloned());
            self.known_functions.insert(
                fn_name.clone(),
                MirType::FnPtr(fn_param_types, Box::new(info.reply_type.clone())),
            );
        }

        // Cast helper functions.
        for info in &cast_infos {
            let fn_name = format!("__service_{}_cast_{}", name_lower, info.snake_name);
            methods.push((info.snake_name.clone(), fn_name.clone()));
            let mut fn_param_types = vec![MirType::Pid(None)];
            fn_param_types.extend(info.param_types.iter().cloned());
            self.known_functions.insert(
                fn_name.clone(),
                MirType::FnPtr(fn_param_types, Box::new(MirType::Unit)),
            );
        }

        // Register the service module for field access resolution.
        self.service_modules.insert(name.clone(), methods);

        // ── Generate call helper functions ─────────────────────────────────
        // __service_{name}_call_{snake}(pid: i64, args...) -> Int
        // Builds message: [u64 type_tag][args as i64s]
        // Calls mesh_service_call(pid, tag, payload_ptr, payload_size)
        // Returns reply as i64

        for info in &call_infos {
            let fn_name = format!("__service_{}_call_{}", name_lower, info.snake_name);

            // Use actual param types so LLVM function signature matches call sites.
            let mut params = vec![("__pid".to_string(), MirType::Int)];
            for (p_name, p_ty) in info.param_names.iter().zip(info.param_types.iter()) {
                params.push((p_name.clone(), p_ty.clone()));
            }

            // Body: call mesh_service_call(pid, tag, payload, size)
            // Codegen intercepts calls to "mesh_service_call" and packs args
            // into a payload buffer, coercing all values to i64.
            let body = MirExpr::Call {
                func: Box::new(MirExpr::Var(
                    "mesh_service_call".to_string(),
                    MirType::FnPtr(
                        vec![MirType::Int, MirType::Int, MirType::Ptr, MirType::Int],
                        Box::new(MirType::Ptr),
                    ),
                )),
                args: {
                    let mut args = vec![
                        MirExpr::Var("__pid".to_string(), MirType::Int),
                        MirExpr::IntLit(info.tag as i64, MirType::Int),
                    ];
                    // Pack the call arguments as the payload.
                    // Codegen will coerce each arg to i64 for the message buffer.
                    args.extend(shaped_params(
                        &info.param_names,
                        &info.param_types,
                        &info.param_shapes,
                    ));
                    args
                },
                ty: info.reply_type.clone(),
            };
            // The service loop reads the reply's shape from here when it
            // sends the reply back; to the caller the wrapper is transparent.
            let body = if info.reply_shape.is_scalar() {
                body
            } else {
                MirExpr::Shaped {
                    value: Box::new(body),
                    shape: info.reply_shape.clone(),
                }
            };

            self.functions.push(MirFunction {
                name: fn_name.clone(),
                params,
                return_type: info.reply_type.clone(),
                body,
                is_closure_fn: false,
                captures: Vec::new(),
                has_tail_calls: false,
            });
        }

        // ── Generate cast helper functions ─────────────────────────────────
        // __service_{name}_cast_{snake}(pid: i64, args...) -> Unit
        // Builds message: [u64 type_tag][args as i64s]
        // Calls mesh_actor_send(pid, msg_ptr, msg_size) (fire-and-forget)

        for info in &cast_infos {
            let fn_name = format!("__service_{}_cast_{}", name_lower, info.snake_name);

            // Use actual param types so LLVM function signature matches call sites.
            let mut params = vec![("__pid".to_string(), MirType::Int)];
            for (p_name, p_ty) in info.param_names.iter().zip(info.param_types.iter()) {
                params.push((p_name.clone(), p_ty.clone()));
            }

            // Body: build message buffer with [tag][args] and call mesh_actor_send.
            // Cast message format: [u64 type_tag][u64 0 (no caller)][args as i64s]
            // Codegen intercepts the mesh_actor_send with int-lit tag and packs args.
            let body = MirExpr::Call {
                func: Box::new(MirExpr::Var(
                    "mesh_actor_send".to_string(),
                    MirType::FnPtr(
                        vec![MirType::Int, MirType::Ptr, MirType::Int],
                        Box::new(MirType::Unit),
                    ),
                )),
                args: {
                    let mut args = vec![
                        MirExpr::Var("__pid".to_string(), MirType::Int),
                        MirExpr::IntLit(info.tag as i64, MirType::Int),
                    ];
                    args.extend(shaped_params(
                        &info.param_names,
                        &info.param_types,
                        &info.param_shapes,
                    ));
                    args
                },
                ty: MirType::Unit,
            };

            self.functions.push(MirFunction {
                name: fn_name.clone(),
                params,
                return_type: MirType::Unit,
                body,
                is_closure_fn: false,
                captures: Vec::new(),
                has_tail_calls: false,
            });
        }

        // ── Generate start function ──────────────────────────────────────
        // __service_{name}_start(init_args...) -> Pid(None)
        // Calls init to get initial state, spawns the loop actor, returns PID.

        {
            // Body: let state = init(args); spawn(loop, state)
            // Use the actual init return type (e.g., struct type) so the full
            // state is allocated and copied into the spawn args buffer.
            let init_call = MirExpr::Call {
                func: Box::new(MirExpr::Var(
                    init_fn_name.clone(),
                    MirType::FnPtr(
                        init_params.iter().map(|(_, t)| t.clone()).collect(),
                        Box::new(init_ret_ty.clone()),
                    ),
                )),
                args: init_params
                    .iter()
                    .map(|(n, t)| MirExpr::Var(n.clone(), t.clone()))
                    .collect(),
                ty: init_ret_ty.clone(),
            };

            let body = MirExpr::Let {
                name: "__init_state".to_string(),
                ty: init_ret_ty.clone(),
                value: Box::new(init_call),
                body: Box::new(MirExpr::ActorSpawn {
                    func: Box::new(MirExpr::Var(
                        loop_fn_name.clone(),
                        MirType::FnPtr(vec![init_ret_ty.clone()], Box::new(MirType::Unit)),
                    )),
                    args: vec![MirExpr::Var(
                        "__init_state".to_string(),
                        init_ret_ty.clone(),
                    )],
                    priority: 1,
                    terminate_callback: None,
                    ty: MirType::Pid(None),
                }),
            };

            self.functions.push(MirFunction {
                name: start_fn_name.clone(),
                params: init_params.clone(),
                return_type: MirType::Pid(None),
                body,
                is_closure_fn: false,
                captures: Vec::new(),
                has_tail_calls: false,
            });
            self.known_functions.insert(
                start_fn_name,
                MirType::FnPtr(
                    init_params.iter().map(|(_, t)| t.clone()).collect(),
                    Box::new(MirType::Pid(None)),
                ),
            );
        }

        // Each handler's tag, function and argument count, in tag order.
        let call_handlers = call_infos
            .iter()
            .map(|info| {
                let handler = format!("__service_{}_handle_call_{}", name_lower, info.snake_name);
                (info.tag, handler, info.param_names.len())
            })
            .collect();
        let cast_handlers = cast_infos
            .iter()
            .map(|info| {
                let handler = format!("__service_{}_handle_cast_{}", name_lower, info.snake_name);
                (info.tag, handler, info.param_names.len())
            })
            .collect();
        self.service_dispatch
            .insert(loop_fn_name.clone(), (call_handlers, cast_handlers));

        // The loop receives its initial state as the actor's argument buffer.
        self.functions.push(MirFunction {
            name: loop_fn_name.clone(),
            params: vec![("__args_ptr".to_string(), MirType::Ptr)],
            return_type: MirType::Unit,
            body: MirExpr::Unit,
            is_closure_fn: false,
            captures: Vec::new(),
            has_tail_calls: false,
        });
        self.known_functions.insert(
            loop_fn_name,
            MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Unit)),
        );
    }

    /// A service handler's parameters: their names, their types (a `()` is
    /// passed as an Int) and the shapes they cross to the service in.
    fn handler_params(
        &self,
        params: Option<ParamList>,
    ) -> (Vec<String>, Vec<MirType>, Vec<MsgShape>) {
        let mut names = Vec::new();
        let mut types = Vec::new();
        let mut shapes = Vec::new();
        for param in params.iter().flat_map(ParamList::params) {
            let range = param.syntax().text_range();
            names.push(
                param
                    .name()
                    .map_or_else(|| "_".to_string(), |name| name.text().to_string()),
            );
            types.push(match self.resolve_range(range) {
                MirType::Unit => MirType::Int,
                ty => ty,
            });
            shapes.push(self.msg_shape_at(range));
        }
        (names, types, shapes)
    }

    /// `assert(cond)`, `assert_eq(a, b)`, `assert_ne(a, b)` or
    /// `assert_raises(f)` in a test (the type checker gives each its
    /// arguments): the runtime check, given the asserted source and an empty
    /// location. `None` for any other call.
    fn lower_test_assertion(&mut self, call: &CallExpr, name: &str) -> Option<MirExpr> {
        let args = call.args();
        let text = |text: String| MirExpr::StringLit(text, MirType::String);
        let (runtime, mut params, mut values) = match name {
            "assert" => (
                "mesh_test_assert",
                vec![MirType::Bool, MirType::Ptr],
                vec![
                    self.lower_expr(&args[0]),
                    text(args[0].syntax().text().to_string()),
                ],
            ),
            // Both sides are compared as `"#{value}"` would show them, and the
            // failure shows the comparison as written.
            "assert_eq" | "assert_ne" => {
                let (runtime, op) = match name {
                    "assert_eq" => ("mesh_test_assert_eq", "=="),
                    _ => ("mesh_test_assert_ne", "!="),
                };
                (
                    runtime,
                    vec![MirType::Ptr; 3],
                    vec![
                        self.lower_shown(&args[0]),
                        self.lower_shown(&args[1]),
                        text(Self::compared_source(call, op)),
                    ],
                )
            }
            // The closure is passed as its function and environment.
            "assert_raises" => (
                "mesh_test_assert_raises",
                vec![MirType::Ptr; 2],
                vec![self.lower_expr(&args[0])],
            ),
            _ => return None,
        };
        // The location: an empty file name, its length and line 0.
        params.extend([MirType::Ptr, MirType::Int, MirType::Int]);
        values.extend([
            text(String::new()),
            MirExpr::IntLit(0, MirType::Int),
            MirExpr::IntLit(0, MirType::Int),
        ]);
        Some(Self::call_named(runtime, params, values, MirType::Unit))
    }

    // ── Actor expression lowering ───────────────────────────────────────

    fn lower_spawn_expr(&mut self, spawn: &SpawnExpr) -> MirExpr {
        if self.supervised_spawn == Some(spawn.syntax().text_range()) {
            return self.run_actor_in_place(spawn);
        }
        // The type checker types the spawn as the pid it gives.
        let ty = self.resolve_range(spawn.syntax().text_range());

        // The first argument is the function to spawn, named rather than
        // passed as a value; the rest, its initial state, cross to the new
        // actor.
        let mut args = spawn
            .arg_list()
            .map(|list| list.args().collect::<Vec<_>>())
            .unwrap_or_default()
            .into_iter();
        let actor = args
            .next()
            .expect("the type checker gives spawn its function");
        let func = Box::new(self.lower_callee(&actor));
        let state_args: Vec<MirExpr> = args
            .map(|arg| {
                let lowered = self.lower_expr(&arg);
                self.shaped(lowered, arg.syntax().text_range())
            })
            .collect();

        // Check if the spawned function has a terminate callback.
        // Look up by function name in known functions to find matching __terminate_<name>.
        let terminate_callback = if let MirExpr::Var(ref fn_name, _) = *func {
            let cb_name = format!("__terminate_{}", fn_name);
            if self.known_functions.contains_key(&cb_name) {
                Some(Box::new(MirExpr::Var(
                    cb_name.clone(),
                    MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Unit)),
                )))
            } else {
                None
            }
        } else {
            None
        };

        MirExpr::ActorSpawn {
            func,
            args: state_args,
            priority: 1, // Normal priority
            terminate_callback,
            ty,
        }
    }

    /// `x |> String.from()`: shown the way `String.from(x)` is.
    fn piped_string_from(
        &mut self,
        rhs: &Option<Expr>,
        lhs: &MirExpr,
        lhs_expr: Option<Expr>,
    ) -> Option<MirExpr> {
        let Some(Expr::CallExpr(call)) = rhs else {
            return None;
        };
        let Some(Expr::FieldAccess(fa)) = call.callee() else {
            return None;
        };
        let is_string_from = fa.field().is_some_and(|f| f.text() == "from")
            && matches!(fa.base(), Some(Expr::NameRef(base)) if base.text().as_deref() == Some("String"));
        if !is_string_from || !call.args().is_empty() {
            return None;
        }
        let source_ty = lhs_expr.and_then(|e| self.get_ty(e.syntax().text_range()).cloned());
        Some(self.wrap_to_string(lhs.clone(), source_ty.as_ref()))
    }

    /// `send` on the right of a pipe: the piped value (`lhs`, written as
    /// `lhs_expr`) is the argument at `insert_idx`.
    fn lower_piped_send(
        &mut self,
        send: &SendExpr,
        lhs: MirExpr,
        lhs_expr: Option<Expr>,
        insert_idx: usize,
    ) -> MirExpr {
        let explicit: Vec<Expr> = send
            .arg_list()
            .map(|list| list.args().collect())
            .unwrap_or_default();
        let mut args: Vec<(MirExpr, TextRange)> = explicit
            .iter()
            .map(|arg| (self.lower_expr(arg), arg.syntax().text_range()))
            .collect();
        let at = insert_idx.min(args.len());
        let lhs_range = lhs_expr
            .expect("the parser gives a pipe its left-hand side")
            .syntax()
            .text_range();
        args.insert(at, (lhs, lhs_range));
        // The type checker gives `send` its target and its message, which
        // crosses to another actor.
        let [(target, _), (message, range)] = <[_; 2]>::try_from(args)
            .unwrap_or_else(|_| panic!("the type checker gives send a target and a message"));
        let message = self.shaped(message, range);
        MirExpr::ActorSend {
            target: Box::new(target),
            message: Box::new(message),
            ty: MirType::Int,
        }
    }

    fn lower_send_expr(&mut self, send: &SendExpr) -> MirExpr {
        // send(target, message) -> Int status: the type checker gives it
        // both, and the message crosses to another actor.
        let args: Vec<Expr> = send
            .arg_list()
            .map(|list| list.args().collect())
            .unwrap_or_default();
        let [target, message] = <[Expr; 2]>::try_from(args)
            .unwrap_or_else(|_| panic!("the type checker gives send a target and a message"));
        let target = self.lower_expr(&target);
        let lowered = self.lower_expr(&message);
        let message = self.shaped(lowered, message.syntax().text_range());
        MirExpr::ActorSend {
            target: Box::new(target),
            message: Box::new(message),
            ty: MirType::Int,
        }
    }

    /// A receive binds the next message to a temporary and matches it against
    /// the arms exactly as a `case` would, guards included.
    fn lower_receive_expr(&mut self, recv: &ReceiveExpr) -> MirExpr {
        let ty = self.resolve_range(recv.syntax().text_range());
        let msg_typeck = recv
            .arms()
            .find_map(|arm| arm.pattern())
            .and_then(|pat| self.get_ty(pat.syntax().text_range()).cloned());
        // A receive with no arm (only an `after`) binds no message.
        let msg_ty = msg_typeck.as_ref().map_or(MirType::Int, |t| {
            runtime_value_type(resolve_type(t, self.registry))
        });
        let msg_var = format!(
            "__recv_msg_{}",
            u32::from(recv.syntax().text_range().start())
        );

        self.push_scope();
        self.insert_var(msg_var.clone(), msg_ty.clone());
        let match_arms: Vec<MirMatchArm> = recv
            .arms()
            .map(|arm| {
                self.push_scope();
                let pattern = arm
                    .pattern()
                    .map(|p| self.lower_pattern_with_expected(&p, msg_typeck.as_ref()))
                    .unwrap_or(MirPattern::Wildcard);
                let guard = arm.guard().map(|e| self.lower_expr(&e));
                let body = arm
                    .body()
                    .map(|e| self.lower_expr(&e))
                    .unwrap_or(MirExpr::Unit);
                self.pop_scope();
                MirMatchArm {
                    pattern,
                    guard,
                    body,
                }
            })
            .collect();
        self.pop_scope();

        // The message is bound to one variable and matched against the arms.
        let handler = (!match_arms.is_empty()).then(|| {
            let body = MirExpr::Match {
                scrutinee: Box::new(MirExpr::Var(msg_var.clone(), msg_ty.clone())),
                arms: match_arms,
                ty: ty.clone(),
            };
            (msg_var, msg_ty, Box::new(body))
        });

        // Handle optional after (timeout) clause.
        let (timeout_ms, timeout_body) = if let Some(after) = recv.after_clause() {
            let ms = after.timeout().map(|e| Box::new(self.lower_expr(&e)));
            let body = after.body().map(|e| Box::new(self.lower_expr(&e)));
            (ms, body)
        } else {
            (None, None)
        };

        MirExpr::ActorReceive {
            handler,
            timeout_ms,
            timeout_body,
            ty,
        }
    }

    fn lower_link_expr(&mut self, link: &LinkExpr) -> MirExpr {
        let target = link
            .arg_list()
            .and_then(|list| list.args().into_iter().next())
            .expect("the type checker gives link its target");
        MirExpr::ActorLink {
            target: Box::new(self.lower_expr(&target)),
            ty: MirType::Unit,
        }
    }

    /// Lower an expression and show it as a String, exactly as `"#{expr}"` would.
    fn lower_shown(&mut self, expr: &Expr) -> MirExpr {
        let typeck_ty = self.get_ty(expr.syntax().text_range()).cloned();
        let lowered = self.lower_expr(expr);
        self.wrap_to_string(lowered, typeck_ty.as_ref())
    }
}

// ── Helper functions ─────────────────────────────────────────────────

/// Set of known stdlib module names for qualified access lowering.
const STDLIB_MODULES: &[&str] = &[
    "String",
    "IO",
    "Env",
    "File",
    "List",
    "Map",
    "Set",
    "Tuple",
    "Range",
    "Queue",
    "HTTP",
    "JSON",
    "Json",
    "Request",
    "Job",
    "Math",
    "Int",
    "Float",
    "Timer",
    "Sqlite",
    "Pg",
    "Ws",
    "Pool",
    "Node",
    "Process", // Phase 67
    "Global",  // Phase 68
    "Iter",    // Phase 76
    "Orm",     // Phase 97
    "Expr",
    "Query",     // Phase 98
    "Repo",      // Phase 98
    "Changeset", // Phase 99
    "Migration", // Phase 101
    "Regex",     // Phase 119
    "Bytes",
    "Host",
    "BytesBuilder",
    "Secret",
    "SecretMap",
    "StorageKey",
    "X25519PrivateKey",
    "SigningPrivateKey",
    "MlKemPrivateKey",
    "U64",
    "U128",
    "I128",
    "Crypto",   // Phase 135
    "Base64",   // Phase 135
    "Hex",      // Phase 135
    "DateTime", // Phase 136
    "Checked",
    "Monotonic",
    "Duration",
    "Channel",
    "Random",
    "Http", // Phase 137
    "WsClient",
    "Test",       // Phase 138
    "Continuity", // continuity
    "Cluster",
    "Option",
    "Result",
];

/// Map Mesh builtin function names to their runtime equivalents.
///
/// Mesh source uses clean names like `println`, `print`, `string_length`.
/// These are mapped to the actual runtime function names like `mesh_println`,
/// `mesh_print`, `mesh_string_length` at the MIR level.
fn map_builtin_name(name: &str) -> String {
    match name {
        "println" => "mesh_println".to_string(),
        "print" => "mesh_print".to_string(),
        // String operations
        "string_length" => "mesh_string_length".to_string(),
        "string_slice" => "mesh_string_slice".to_string(),
        "string_contains" => "mesh_string_contains".to_string(),
        "string_starts_with" => "mesh_string_starts_with".to_string(),
        "string_ends_with" => "mesh_string_ends_with".to_string(),
        "string_trim" => "mesh_string_trim".to_string(),
        "string_repeat" => "mesh_string_repeat".to_string(),
        "string_trim_start" => "mesh_string_trim_start".to_string(),
        "string_trim_end" => "mesh_string_trim_end".to_string(),
        "string_to_upper" => "mesh_string_to_upper".to_string(),
        "string_to_lower" => "mesh_string_to_lower".to_string(),
        "string_replace" => "mesh_string_replace".to_string(),
        "string_split" => "mesh_string_split".to_string(),
        "string_join" => "mesh_string_join".to_string(),
        "string_to_int" => "mesh_string_to_int".to_string(),
        "string_to_float" => "mesh_string_to_float".to_string(),
        // File I/O functions
        "file_read" => "mesh_file_read".to_string(),
        "file_read_bytes" => "mesh_file_read_bytes".to_string(),
        "file_write_bytes" => "mesh_file_write_bytes".to_string(),
        "file_size" => "mesh_file_size".to_string(),
        "file_write" => "mesh_file_write".to_string(),
        "file_append" => "mesh_file_append".to_string(),
        "file_exists" => "mesh_file_exists".to_string(),
        "file_delete" => "mesh_file_delete".to_string(),
        // IO functions
        "io_read_line" => "mesh_io_read_line".to_string(),
        "io_eprintln" => "mesh_io_eprintln".to_string(),
        // Env functions
        // "env_get" is the prefixed form of Env.get — routes to 2-arg with-default variant
        "env_get" => "mesh_env_get_with_default".to_string(),
        "env_get_with_default" => "mesh_env_get_with_default".to_string(),
        "env_get_int" => "mesh_env_get_int".to_string(),
        "env_get_secret_hex" => "mesh_env_get_secret_hex".to_string(),
        "env_args" => "mesh_env_args".to_string(),
        // Regex functions (Phase 119)
        "regex_from_literal" => "mesh_regex_from_literal".to_string(),
        // "regex_compile" is the prefixed form of Regex.compile
        "regex_compile" => "mesh_regex_compile".to_string(),
        "regex_is_match" => "mesh_regex_match".to_string(),
        "regex_captures" => "mesh_regex_captures".to_string(),
        "regex_replace" => "mesh_regex_replace".to_string(),
        "regex_split" => "mesh_regex_split".to_string(),
        // Crypto functions (Phase 135)
        "crypto_sha256" => "mesh_crypto_sha256".to_string(),
        "crypto_sha512" => "mesh_crypto_sha512".to_string(),
        "crypto_sha256_hex" => "mesh_crypto_sha256_hex".to_string(),
        "crypto_sha512_hex" => "mesh_crypto_sha512_hex".to_string(),
        "crypto_random_bytes" => "mesh_crypto_random_bytes".to_string(),
        "crypto_hmac_sha256" => "mesh_crypto_hmac_sha256".to_string(),
        "crypto_hkdf_sha256" => "mesh_crypto_hkdf_sha256".to_string(),
        "crypto_argon2id" => "mesh_crypto_argon2id".to_string(),
        "crypto_x25519_generate" => "mesh_crypto_x25519_generate".to_string(),
        "crypto_x25519_from_seed" => "mesh_crypto_x25519_from_seed".to_string(),
        "crypto_x25519_from_secret" => "mesh_crypto_x25519_from_secret".to_string(),
        "crypto_x25519_public" => "mesh_crypto_x25519_public".to_string(),
        "crypto_x25519_shared" => "mesh_crypto_x25519_shared".to_string(),
        "crypto_hpke_seal" => "mesh_crypto_hpke_seal".to_string(),
        "crypto_hpke_open" => "mesh_crypto_hpke_open".to_string(),
        "crypto_hpke_seal_secret" => "mesh_crypto_hpke_seal_secret".to_string(),
        "crypto_hpke_open_secret" => "mesh_crypto_hpke_open_secret".to_string(),
        "crypto_mlkem_generate" => "mesh_crypto_mlkem_generate".to_string(),
        "crypto_mlkem_from_seed" => "mesh_crypto_mlkem_from_seed".to_string(),
        "crypto_mlkem_from_secret" => "mesh_crypto_mlkem_from_secret".to_string(),
        "crypto_mlkem_encapsulate" => "mesh_crypto_mlkem_encapsulate".to_string(),
        "crypto_mlkem_decapsulate" => "mesh_crypto_mlkem_decapsulate".to_string(),
        "crypto_signing_generate" => "mesh_crypto_signing_generate".to_string(),
        "crypto_signing_from_seed" => "mesh_crypto_signing_from_seed".to_string(),
        "crypto_signing_from_secret" => "mesh_crypto_signing_from_secret".to_string(),
        "crypto_sign" => "mesh_crypto_sign".to_string(),
        "crypto_verify" => "mesh_crypto_verify".to_string(),
        "crypto_aead_key" => "mesh_crypto_aead_key".to_string(),
        "crypto_aead_seal" => "mesh_crypto_aead_seal".to_string(),
        "crypto_aead_open" => "mesh_crypto_aead_open".to_string(),
        "crypto_hmac_sha512" => "mesh_crypto_hmac_sha512".to_string(),
        "crypto_uuid4" => "mesh_crypto_uuid4".to_string(),
        // Base64 functions (Phase 135)
        "base64_encode" => "mesh_base64_encode".to_string(),
        "base64_decode" => "mesh_base64_decode".to_string(),
        "base64_encode_url" => "mesh_base64_encode_url".to_string(),
        "base64_decode_url" => "mesh_base64_decode_url".to_string(),
        // Hex functions (Phase 135)
        "hex_encode" => "mesh_hex_encode".to_string(),
        "hex_decode" => "mesh_hex_decode".to_string(),
        // Binary-safe Bytes functions
        "bytes_empty" => "mesh_bytes_empty".to_string(),
        "bytes_from_list" => "mesh_bytes_from_list".to_string(),
        "bytes_to_list" => "mesh_bytes_to_list".to_string(),
        "bytes_repeat" => "mesh_bytes_repeat".to_string(),
        "bytes_length" => "mesh_bytes_length".to_string(),
        "bytes_get" => "mesh_bytes_get".to_string(),
        "bytes_slice" => "mesh_bytes_slice".to_string(),
        "bytes_concat" => "mesh_bytes_concat".to_string(),
        "bytes_secure_equals" => "mesh_bytes_secure_equals".to_string(),
        "bytes_from_utf8" => "mesh_bytes_from_utf8".to_string(),
        "bytes_to_utf8" => "mesh_bytes_to_utf8".to_string(),
        "bytes_to_base64" => "mesh_bytes_to_base64".to_string(),
        "bytes_from_base64" => "mesh_bytes_from_base64".to_string(),
        "bytes_to_base58" => "mesh_bytes_to_base58".to_string(),
        "bytes_from_base58" => "mesh_bytes_from_base58".to_string(),
        "bytes_to_hex" => "mesh_bytes_to_hex".to_string(),
        "bytes_from_hex" => "mesh_bytes_from_hex".to_string(),
        "bytes_read_uint_le" => "mesh_bytes_read_uint_le".to_string(),
        "bytes_write_uint_le" => "mesh_bytes_write_uint_le".to_string(),
        "bytes_read_u16_be" => "mesh_bytes_read_u16_be".to_string(),
        "bytes_read_u16_le" => "mesh_bytes_read_u16_le".to_string(),
        "bytes_read_u32_be" => "mesh_bytes_read_u32_be".to_string(),
        "bytes_read_u32_le" => "mesh_bytes_read_u32_le".to_string(),
        "bytes_read_u64_be" => "mesh_bytes_read_u64_be".to_string(),
        "bytes_read_u64_le" => "mesh_bytes_read_u64_le".to_string(),
        "bytes_write_u16_be" => "mesh_bytes_write_u16_be".to_string(),
        "bytes_write_u32_be" => "mesh_bytes_write_u32_be".to_string(),
        "bytes_write_u64_be" => "mesh_bytes_write_u64_be".to_string(),
        "bytes_builder_new" => "mesh_bytes_builder_new".to_string(),
        "bytes_builder_write_u8" => "mesh_bytes_builder_write_u8".to_string(),
        "bytes_builder_write_u16_be" => "mesh_bytes_builder_write_u16_be".to_string(),
        "bytes_builder_write_u32_be" => "mesh_bytes_builder_write_u32_be".to_string(),
        "bytes_builder_write_bytes" => "mesh_bytes_builder_write_bytes".to_string(),
        "bytes_builder_finish" => "mesh_bytes_builder_finish".to_string(),
        "host_secure_store_put" => "mesh_host_secure_store_put".to_string(),
        "host_secure_store_get" => "mesh_host_secure_store_get".to_string(),
        "host_secure_store_delete" => "mesh_host_secure_store_delete".to_string(),
        "host_push_get_token" => "mesh_host_push_get_token".to_string(),
        "host_background_schedule" => "mesh_host_background_schedule".to_string(),
        "host_network_state" => "mesh_host_network_state".to_string(),
        "host_monotonic_clock" => "mesh_host_monotonic_clock".to_string(),
        "host_wall_clock" => "mesh_host_wall_clock".to_string(),
        "host_log_redacted" => "mesh_host_log_redacted".to_string(),
        "secret_random" => "mesh_secret_random".to_string(),
        "secret_concat" => "mesh_secret_concat".to_string(),
        "secret_destroy" => "mesh_secret_destroy".to_string(),
        "secret_map_new"
        | "secret_map_insert"
        | "secret_map_contains"
        | "secret_map_fork"
        | "secret_map_copy"
        | "secret_map_delete"
        | "secret_map_merge" => format!("mesh_{name}"),
        "storage_key_ephemeral"
        | "storage_key_platform"
        | "storage_key_seal_bytes"
        | "storage_key_unseal_bytes" => format!("mesh_{name}"),
        "secret_seal_for_storage"
        | "secret_unseal_from_storage"
        | "secret_map_seal_for_storage"
        | "secret_map_unseal_from_storage"
        | "signing_private_key_seal_for_storage"
        | "signing_private_key_unseal_from_storage"
        | "x25519_private_key_seal_for_storage"
        | "x25519_private_key_unseal_from_storage"
        | "mlkem_private_key_seal_for_storage"
        | "mlkem_private_key_unseal_from_storage" => format!("mesh_{name}"),
        "u64_parse" | "u64_compare" | "u64_add" | "u64_subtract" | "u64_multiply"
        | "u64_divide" | "u64_to_int" | "u64_to_string" | "u128_parse" | "u128_compare"
        | "u128_add" | "u128_subtract" | "u128_multiply" | "u128_divide" | "u128_to_int"
        | "u128_to_string" | "i128_parse" | "i128_compare" | "i128_add" | "i128_subtract"
        | "i128_multiply" | "i128_divide" | "i128_to_int" | "i128_to_string" => {
            format!("mesh_{name}")
        }
        // DateTime functions (Phase 136)
        "datetime_utc_now" => "mesh_datetime_utc_now".to_string(),
        "datetime_from_iso8601" => "mesh_datetime_from_iso8601".to_string(),
        "datetime_to_iso8601" => "mesh_datetime_to_iso8601".to_string(),
        "datetime_from_unix_ms" => "mesh_datetime_from_unix_ms".to_string(),
        "datetime_to_unix_ms" => "mesh_datetime_to_unix_ms".to_string(),
        "datetime_from_unix_secs" => "mesh_datetime_from_unix_secs".to_string(),
        "datetime_to_unix_secs" => "mesh_datetime_to_unix_secs".to_string(),
        "datetime_add" => "mesh_datetime_add".to_string(),
        "datetime_diff" => "mesh_datetime_diff".to_string(),
        "datetime_is_before" => "mesh_datetime_before".to_string(),
        "datetime_is_after" => "mesh_datetime_after".to_string(),
        "checked_add" => "mesh_checked_add".to_string(),
        "checked_sub" => "mesh_checked_sub".to_string(),
        "checked_mul" => "mesh_checked_mul".to_string(),
        "checked_div" => "mesh_checked_div".to_string(),
        "checked_abs" => "mesh_checked_abs".to_string(),
        "checked_mul_div" => "mesh_checked_mul_div".to_string(),
        "checked_rescale" => "mesh_checked_rescale".to_string(),
        "monotonic_now_nanos" => "mesh_monotonic_now_nanos".to_string(),
        "monotonic_elapsed" => "mesh_monotonic_elapsed".to_string(),
        "duration_millis" => "mesh_duration_millis".to_string(),
        "duration_seconds" => "mesh_duration_seconds".to_string(),
        "channel_bounded" => "mesh_channel_bounded".to_string(),
        "channel_bounded_bytes" => "mesh_channel_bounded_bytes".to_string(),
        "channel_try_send" => "mesh_channel_try_send".to_string(),
        "channel_recv" => "mesh_channel_recv".to_string(),
        "channel_depth" => "mesh_channel_depth".to_string(),
        "channel_byte_depth" => "mesh_channel_byte_depth".to_string(),
        "channel_dropped" => "mesh_channel_dropped".to_string(),
        "random_seed" => "mesh_random_seed".to_string(),
        "random_next_int" => "mesh_random_next_int".to_string(),
        "random_next_unit_ppm" => "mesh_random_next_unit_ppm".to_string(),
        // Http client functions (Phase 137)
        "http_build" => "mesh_http_build".to_string(),
        "http_header" => "mesh_http_header".to_string(),
        "http_body" => "mesh_http_body".to_string(),
        "http_body_bytes" => "mesh_http_body_bytes".to_string(),
        "http_timeout" => "mesh_http_timeout".to_string(),
        "http_stage_timeout" => "mesh_http_stage_timeout".to_string(),
        "http_max_redirects" => "mesh_http_max_redirects".to_string(),
        "http_max_response_bytes" => "mesh_http_max_response_bytes".to_string(),
        "http_query" => "mesh_http_query".to_string(),
        "http_json" => "mesh_http_json".to_string(),
        "http_send" => "mesh_http_send".to_string(),
        // Http streaming + cancel + keep-alive (Phase 137 Plan 02)
        "http_stream" => "mesh_http_stream".to_string(),
        "http_stream_bytes" => "mesh_http_stream_bytes".to_string(),
        "http_cancel" => "mesh_http_cancel".to_string(),
        "http_client" => "mesh_http_client".to_string(),
        "http_send_with" => "mesh_http_send_with".to_string(),
        "http_client_close" => "mesh_http_client_close".to_string(),
        "http_retry_class" => "mesh_http_retry_class".to_string(),
        "http_metrics" => "mesh_http_metrics".to_string(),
        "ws_client_options"
        | "ws_client_connect_timeout"
        | "ws_client_heartbeat_timeout"
        | "ws_client_max_message_bytes"
        | "ws_client_queue_capacity"
        | "ws_client_connect"
        | "ws_client_send_text"
        | "ws_client_send_bytes"
        | "ws_client_recv"
        | "ws_client_close"
        | "ws_client_reconnect_delay" => format!("mesh_{name}"),
        // Test DSL assertion builtins (Phase 138) — lowercase with test_ prefix
        "test_assert" => "mesh_test_assert".to_string(),
        "test_assert_eq" => "mesh_test_assert_eq".to_string(),
        "test_assert_ne" => "mesh_test_assert_ne".to_string(),
        "test_assert_raises" => "mesh_test_assert_raises".to_string(),
        "test_begin" => "mesh_test_begin".to_string(),
        "test_pass" => "mesh_test_pass".to_string(),
        "test_fail_msg" => "mesh_test_fail_msg".to_string(),
        "test_summary" => "mesh_test_summary".to_string(),
        "test_cleanup_actors" => "mesh_test_cleanup_actors".to_string(),
        "test_run_body" => "mesh_test_run_body".to_string(),
        "test_end" => "mesh_test_end".to_string(),
        "test_mock_actor" => "mesh_test_mock_actor".to_string(),
        "test_install_in_memory_secure_store" => {
            "mesh_test_install_in_memory_secure_store".to_string()
        }
        "test_set_push_token" => "mesh_test_set_push_token".to_string(),
        "test_pass_count" => "mesh_test_pass_count".to_string(),
        "test_fail_count" => "mesh_test_fail_count".to_string(),
        // ── Collection functions (Phase 8 Plan 02) ───────────────────
        // List operations
        "list_new" => "mesh_list_new".to_string(),
        "list_length" => "mesh_list_length".to_string(),
        "list_append" => "mesh_list_append".to_string(),
        "list_head" => "mesh_list_head".to_string(),
        "list_tail" => "mesh_list_tail".to_string(),
        "list_get" => "mesh_list_get".to_string(),
        "list_concat" => "mesh_list_concat".to_string(),
        "list_reverse" => "mesh_list_reverse".to_string(),
        "list_map" => "mesh_list_map".to_string(),
        "list_filter" => "mesh_list_filter".to_string(),
        "list_reduce" => "mesh_list_reduce".to_string(),
        // Phase 46: sort, find, any, all, contains
        "list_sort" => "mesh_list_sort".to_string(),
        "list_find" => "mesh_list_find".to_string(),
        "list_any" => "mesh_list_any".to_string(),
        "list_all" => "mesh_list_all".to_string(),
        "list_contains" => "mesh_list_contains".to_string(),
        // Phase 47: zip, flat_map, flatten, enumerate, take, drop, last, nth
        "list_zip" => "mesh_list_zip".to_string(),
        "list_flat_map" => "mesh_list_flat_map".to_string(),
        "list_flatten" => "mesh_list_flatten".to_string(),
        "list_enumerate" => "mesh_list_enumerate".to_string(),
        "list_take" => "mesh_list_take".to_string(),
        "list_drop" => "mesh_list_drop".to_string(),
        "list_last" => "mesh_list_last".to_string(),
        "list_nth" => "mesh_list_nth".to_string(),
        // Map operations
        "map_new" => "mesh_map_new".to_string(),
        "map_put" => "mesh_map_put".to_string(),
        // `Map.get` of a missing key panics (`mesh_map_get` reads it as 0,
        // which the runtime's own callers rely on).
        "map_get" => "mesh_map_fetch".to_string(),
        "map_has_key" => "mesh_map_has_key".to_string(),
        "map_delete" => "mesh_map_delete".to_string(),
        "map_size" => "mesh_map_size".to_string(),
        "map_keys" => "mesh_map_keys".to_string(),
        "map_values" => "mesh_map_values".to_string(),
        // Phase 47: Map merge/to_list/from_list
        "map_merge" => "mesh_map_merge".to_string(),
        "map_to_list" => "mesh_map_to_list".to_string(),
        "map_from_list" => "mesh_map_from_list".to_string(),
        // Set operations
        "set_new" => "mesh_set_new".to_string(),
        "set_add" => "mesh_set_add".to_string(),
        "set_remove" => "mesh_set_remove".to_string(),
        "set_contains" => "mesh_set_contains".to_string(),
        "set_size" => "mesh_set_size".to_string(),
        "set_union" => "mesh_set_union".to_string(),
        "set_intersection" => "mesh_set_intersection".to_string(),
        // Phase 47: Set difference/to_list/from_list
        "set_difference" => "mesh_set_difference".to_string(),
        "set_to_list" => "mesh_set_to_list".to_string(),
        "set_from_list" => "mesh_set_from_list".to_string(),
        // Tuple operations
        "tuple_nth" => "mesh_tuple_nth".to_string(),
        "tuple_first" => "mesh_tuple_first".to_string(),
        "tuple_second" => "mesh_tuple_second".to_string(),
        "tuple_size" => "mesh_tuple_size".to_string(),
        // Range operations
        "range_new" => "mesh_range_new".to_string(),
        "range_to_list" => "mesh_range_to_list".to_string(),
        "range_map" => "mesh_range_map".to_string(),
        "range_filter" => "mesh_range_filter".to_string(),
        "range_length" => "mesh_range_length".to_string(),
        // Queue operations
        "queue_new" => "mesh_queue_new".to_string(),
        "queue_push" => "mesh_queue_push".to_string(),
        "queue_pop" => "mesh_queue_pop".to_string(),
        "queue_peek" => "mesh_queue_peek".to_string(),
        "queue_size" => "mesh_queue_size".to_string(),
        "queue_is_empty" => "mesh_queue_is_empty".to_string(),
        // The prelude's bare list functions. A name imported from a standard
        // module lowers as its qualified call (see `stdlib_imports`).
        "map" => "mesh_list_map".to_string(),
        "filter" => "mesh_list_filter".to_string(),
        "reduce" => "mesh_list_reduce".to_string(),
        "head" => "mesh_list_head".to_string(),
        "tail" => "mesh_list_tail".to_string(),
        // ── JSON functions (Phase 8 Plan 04) ─────────────────────────
        "json_parse" => "mesh_json_parse".to_string(),
        "json_encode" => "mesh_json_encode".to_string(),
        "json_encode_string" => "mesh_json_encode_string".to_string(),
        "json_encode_int" => "mesh_json_encode_int".to_string(),
        "json_encode_bool" => "mesh_json_encode_bool".to_string(),
        "json_encode_map" => "mesh_json_encode_map".to_string(),
        "json_encode_list" => "mesh_json_encode_list".to_string(),
        "json_object_get" => "mesh_json_object_get".to_string(),
        "json_array_get" => "mesh_json_array_get".to_string(),
        "json_array_length" => "mesh_json_array_length".to_string(),
        "json_is_null" => "mesh_json_is_null".to_string(),
        "json_as_int" => "mesh_json_value_as_int".to_string(),
        "json_as_float" => "mesh_json_value_as_float".to_string(),
        "json_as_string" => "mesh_json_as_string".to_string(),
        "json_as_bool" => "mesh_json_value_as_bool".to_string(),
        "json_from_int" => "mesh_json_from_int".to_string(),
        "json_from_float" => "mesh_json_from_float".to_string(),
        "json_from_bool" => "mesh_json_from_bool".to_string(),
        "json_from_string" => "mesh_json_from_string".to_string(),
        // Phase 103: JSON field extraction
        "json_get" => "mesh_json_get".to_string(),
        "json_get_nested" => "mesh_json_get_nested".to_string(),
        "json_is_string" => "mesh_json_is_string".to_string(),
        // ── HTTP functions (Phase 8 Plan 05) ──────────────────────────
        "http_router" => "mesh_http_router".to_string(),
        "http_route" => "mesh_http_route".to_string(),
        "http_serve" => "mesh_http_serve".to_string(),
        "http_serve_tls" => "mesh_http_serve_tls".to_string(),
        "http_response" => "mesh_http_response_new".to_string(),
        "http_response_bytes" => "mesh_http_response_bytes_new".to_string(),
        "http_response_bytes_with_headers" => "mesh_http_response_bytes_with_headers".to_string(),
        "http_response_with_headers" => "mesh_http_response_with_headers".to_string(),
        // Request accessor functions (prefixed form from module-qualified access)
        "request_method" => "mesh_http_request_method".to_string(),
        "request_path" => "mesh_http_request_path".to_string(),
        "request_body" => "mesh_http_request_body".to_string(),
        "request_body_bytes" => "mesh_http_request_body_bytes".to_string(),
        "request_header" => "mesh_http_request_header".to_string(),
        "request_query" => "mesh_http_request_query".to_string(),
        // Phase 51: Path parameter accessor
        "request_param" => "mesh_http_request_param".to_string(),
        "http_request_id" => "mesh_http_request_id".to_string(),
        "http_idempotency_key" => "mesh_http_idempotency_key".to_string(),
        "cluster_capacity" => "mesh_cluster_capacity".to_string(),
        "cluster_pressure" => "mesh_cluster_pressure".to_string(),
        "cluster_telemetry" => "mesh_cluster_telemetry".to_string(),
        "cluster_role" => "mesh_cluster_role".to_string(),
        "cluster_state" => "mesh_cluster_state".to_string(),
        // Phase 51: Method-specific routing (HTTP.on_get -> http_on_get -> mesh_http_route_get)
        "http_on_get" => "mesh_http_route_get".to_string(),
        "http_on_post" => "mesh_http_route_post".to_string(),
        "http_on_put" => "mesh_http_route_put".to_string(),
        "http_on_delete" => "mesh_http_route_delete".to_string(),
        // Phase 52: Middleware
        "http_use" => "mesh_http_use_middleware".to_string(),
        // ── SQLite functions (Phase 53) ──────────────────────────────────
        "sqlite_open" => "mesh_sqlite_open".to_string(),
        "sqlite_close" => "mesh_sqlite_close".to_string(),
        "sqlite_execute" => "mesh_sqlite_execute".to_string(),
        "sqlite_query" => "mesh_sqlite_query".to_string(),
        "sqlite_execute_values" => "mesh_sqlite_execute_values".to_string(),
        "sqlite_query_values" => "mesh_sqlite_query_values".to_string(),
        // ── PostgreSQL functions (Phase 54) ──────────────────────────────
        "pg_connect" => "mesh_pg_connect".to_string(),
        "pg_close" => "mesh_pg_close".to_string(),
        "pg_execute" => "mesh_pg_execute".to_string(),
        "pg_query" => "mesh_pg_query".to_string(),
        "pg_execute_values" => "mesh_pg_execute_values".to_string(),
        "pg_query_values" => "mesh_pg_query_values".to_string(),
        // ── Phase 57: PG Transaction functions ──────────────────────────
        "pg_begin" => "mesh_pg_begin".to_string(),
        "pg_commit" => "mesh_pg_commit".to_string(),
        "pg_rollback" => "mesh_pg_rollback".to_string(),
        "pg_transaction" => "mesh_pg_transaction".to_string(),
        // ── PostgreSQL expression helpers ───────────────────────────────
        "pg_cast" => "mesh_pg_cast".to_string(),
        "pg_jsonb" => "mesh_pg_jsonb".to_string(),
        "pg_int" => "mesh_pg_int".to_string(),
        "pg_text" => "mesh_pg_text".to_string(),
        "pg_uuid" => "mesh_pg_uuid".to_string(),
        "pg_timestamptz" => "mesh_pg_timestamptz".to_string(),
        "pg_gen_salt" => "mesh_pg_gen_salt".to_string(),
        "pg_crypt" => "mesh_pg_crypt".to_string(),
        "pg_to_tsvector" => "mesh_pg_to_tsvector".to_string(),
        "pg_plainto_tsquery" => "mesh_pg_plainto_tsquery".to_string(),
        "pg_ts_rank" => "mesh_pg_ts_rank".to_string(),
        "pg_tsvector_matches" => "mesh_pg_tsvector_matches".to_string(),
        "pg_jsonb_contains" => "mesh_pg_jsonb_contains".to_string(),
        // ── PostgreSQL schema helpers ─────────────────────────────────
        "pg_create_extension" => "mesh_pg_create_extension".to_string(),
        "pg_create_range_partitioned_table" => "mesh_pg_create_range_partitioned_table".to_string(),
        "pg_create_gin_index" => "mesh_pg_create_gin_index".to_string(),
        "pg_create_daily_partitions_ahead" => "mesh_pg_create_daily_partitions_ahead".to_string(),
        "pg_list_daily_partitions_before" => "mesh_pg_list_daily_partitions_before".to_string(),
        "pg_drop_partition" => "mesh_pg_drop_partition".to_string(),
        // ── Phase 57: SQLite Transaction functions ──────────────────────
        "sqlite_begin" => "mesh_sqlite_begin".to_string(),
        "sqlite_commit" => "mesh_sqlite_commit".to_string(),
        "sqlite_rollback" => "mesh_sqlite_rollback".to_string(),
        // ── Phase 57: Connection Pool functions ─────────────────────────
        "pool_open" => "mesh_pool_open".to_string(),
        "pool_close" => "mesh_pool_close".to_string(),
        "pool_query" => "mesh_pool_query".to_string(),
        "pool_execute" => "mesh_pool_execute".to_string(),
        "pool_query_values" => "mesh_pool_query_values".to_string(),
        "pool_execute_values" => "mesh_pool_execute_values".to_string(),
        // ── Phase 58: Struct-to-Row Mapping ───────────────────────────────
        "pg_query_as" => "mesh_pg_query_as".to_string(),
        "pool_query_as" => "mesh_pool_query_as".to_string(),
        // ── Phase 97: ORM SQL Generation ─────────────────────────────────
        "orm_build_select" => "mesh_orm_build_select".to_string(),
        "orm_build_insert" => "mesh_orm_build_insert".to_string(),
        "orm_build_update" => "mesh_orm_build_update".to_string(),
        "orm_build_delete" => "mesh_orm_build_delete".to_string(),
        // ── Neutral expression builder ─────────────────────────────────
        "expr_column" => "mesh_expr_column".to_string(),
        "expr_value" => "mesh_expr_value".to_string(),
        "expr_null" => "mesh_expr_null".to_string(),
        "expr_call" => "mesh_expr_call".to_string(),
        "expr_fn_call" => "mesh_expr_call".to_string(),
        "expr_add" => "mesh_expr_add".to_string(),
        "expr_sub" => "mesh_expr_sub".to_string(),
        "expr_mul" => "mesh_expr_mul".to_string(),
        "expr_div" => "mesh_expr_div".to_string(),
        "expr_eq" => "mesh_expr_eq".to_string(),
        "expr_neq" => "mesh_expr_neq".to_string(),
        "expr_lt" => "mesh_expr_lt".to_string(),
        "expr_lte" => "mesh_expr_lte".to_string(),
        "expr_gt" => "mesh_expr_gt".to_string(),
        "expr_gte" => "mesh_expr_gte".to_string(),
        "expr_case" => "mesh_expr_case".to_string(),
        "expr_case_when" => "mesh_expr_case".to_string(),
        "expr_coalesce" => "mesh_expr_coalesce".to_string(),
        "expr_excluded" => "mesh_expr_excluded".to_string(),
        "expr_alias" => "mesh_expr_alias".to_string(),
        "expr_label" => "mesh_expr_alias".to_string(),
        // ── Phase 98: Query Builder ─────────────────────────────────────
        "query_from" => "mesh_query_from".to_string(),
        "query_where" => "mesh_query_where".to_string(),
        "query_where_op" => "mesh_query_where_op".to_string(),
        "query_where_in" => "mesh_query_where_in".to_string(),
        "query_where_null" => "mesh_query_where_null".to_string(),
        "query_where_not_null" => "mesh_query_where_not_null".to_string(),
        "query_where_not_in" => "mesh_query_where_not_in".to_string(),
        "query_where_between" => "mesh_query_where_between".to_string(),
        "query_where_or" => "mesh_query_where_or".to_string(),
        "query_where_expr" => "mesh_query_where_expr".to_string(),
        "query_select" => "mesh_query_select".to_string(),
        "query_select_expr" => "mesh_query_select_expr".to_string(),
        "query_select_exprs" => "mesh_query_select_exprs".to_string(),
        "query_order_by" => "mesh_query_order_by".to_string(),
        "query_limit" => "mesh_query_limit".to_string(),
        "query_offset" => "mesh_query_offset".to_string(),
        "query_join" => "mesh_query_join".to_string(),
        "query_join_as" => "mesh_query_join_as".to_string(),
        "query_group_by" => "mesh_query_group_by".to_string(),
        "query_having" => "mesh_query_having".to_string(),
        "query_fragment" => "mesh_query_fragment".to_string(),
        // ── Phase 108: Aggregate SELECT functions ─────────────────────────
        "query_select_count" => "mesh_query_select_count".to_string(),
        "query_select_count_field" => "mesh_query_select_count_field".to_string(),
        "query_select_sum" => "mesh_query_select_sum".to_string(),
        "query_select_avg" => "mesh_query_select_avg".to_string(),
        "query_select_min" => "mesh_query_select_min".to_string(),
        "query_select_max" => "mesh_query_select_max".to_string(),
        // ── Phase 103: Query Builder Raw Extensions ─────────────────────
        "query_select_raw" => "mesh_query_select_raw".to_string(),
        "query_where_raw" => "mesh_query_where_raw".to_string(),
        // ── Phase 106: Raw ORDER BY / GROUP BY ──────────────────────────
        "query_order_by_raw" => "mesh_query_order_by_raw".to_string(),
        "query_group_by_raw" => "mesh_query_group_by_raw".to_string(),
        // ── Phase 109: Subquery WHERE ─────────────────────────────────────
        "query_where_sub" => "mesh_query_where_sub".to_string(),
        // ── Phase 98: Repo Read Operations ──────────────────────────────
        "repo_all" => "mesh_repo_all".to_string(),
        "repo_one" => "mesh_repo_one".to_string(),
        "repo_get" => "mesh_repo_get".to_string(),
        "repo_get_by" => "mesh_repo_get_by".to_string(),
        "repo_count" => "mesh_repo_count".to_string(),
        "repo_exists" => "mesh_repo_exists".to_string(),
        // ── Phase 98: Repo Write Operations ─────────────────────────────
        "repo_insert" => "mesh_repo_insert".to_string(),
        "repo_insert_expr" => "mesh_repo_insert_expr".to_string(),
        "repo_update" => "mesh_repo_update".to_string(),
        "repo_delete" => "mesh_repo_delete".to_string(),
        "repo_transaction" => "mesh_repo_transaction".to_string(),
        // ── Phase 103: Extended Repo Write Operations ────────────────────
        "repo_update_where" => "mesh_repo_update_where".to_string(),
        "repo_update_where_expr" => "mesh_repo_update_where_expr".to_string(),
        "repo_delete_where" => "mesh_repo_delete_where".to_string(),
        "repo_query_raw" => "mesh_repo_query_raw".to_string(),
        "repo_execute_raw" => "mesh_repo_execute_raw".to_string(),
        // ── Phase 109: Upsert, RETURNING, Subquery ────────────────────────
        "repo_insert_or_update" => "mesh_repo_insert_or_update".to_string(),
        "repo_insert_or_update_expr" => "mesh_repo_insert_or_update_expr".to_string(),
        "repo_delete_where_returning" => "mesh_repo_delete_where_returning".to_string(),
        // ── Phase 100: Repo Preloading ──────────────────────────────────
        "repo_preload" => "mesh_repo_preload".to_string(),
        // ── Phase 99: Repo Changeset Operations ─────────────────────────
        "repo_insert_changeset" => "mesh_repo_insert_changeset".to_string(),
        "repo_update_changeset" => "mesh_repo_update_changeset".to_string(),
        // ── Phase 99: Changeset Operations ──────────────────────────────
        "changeset_cast" => "mesh_changeset_cast".to_string(),
        "changeset_cast_with_types" => "mesh_changeset_cast_with_types".to_string(),
        "changeset_validate_required" => "mesh_changeset_validate_required".to_string(),
        "changeset_validate_length" => "mesh_changeset_validate_length".to_string(),
        "changeset_validate_format" => "mesh_changeset_validate_format".to_string(),
        "changeset_validate_inclusion" => "mesh_changeset_validate_inclusion".to_string(),
        "changeset_validate_number" => "mesh_changeset_validate_number".to_string(),
        "changeset_valid" => "mesh_changeset_valid".to_string(),
        "changeset_errors" => "mesh_changeset_errors".to_string(),
        "changeset_changes" => "mesh_changeset_changes".to_string(),
        "changeset_get_change" => "mesh_changeset_get_change".to_string(),
        "changeset_get_error" => "mesh_changeset_get_error".to_string(),
        // ── Phase 101: Migration DDL Operations ─────────────────────────
        "migration_create_table" => "mesh_migration_create_table".to_string(),
        "migration_drop_table" => "mesh_migration_drop_table".to_string(),
        "migration_add_column" => "mesh_migration_add_column".to_string(),
        "migration_drop_column" => "mesh_migration_drop_column".to_string(),
        "migration_rename_column" => "mesh_migration_rename_column".to_string(),
        "migration_create_index" => "mesh_migration_create_index".to_string(),
        "migration_drop_index" => "mesh_migration_drop_index".to_string(),
        "migration_execute" => "mesh_migration_execute".to_string(),
        // NOTE: No bare name mappings for HTTP/Request (router, route, get,
        // post, method, path, body, etc.) because they collide with common
        // variable names. Use module-qualified access instead:
        //   HTTP.router(), HTTP.route(), Request.method(), etc.
        // ── Job functions (Phase 9 Plan 04) ────────────────────────────
        "job_async" => "mesh_job_async".to_string(),
        "job_await" => "mesh_job_await".to_string(),
        "job_await_timeout" => "mesh_job_await_timeout".to_string(),
        "job_map" => "mesh_job_map".to_string(),
        // ── Math/Int/Float functions (Phase 43 Plan 01) ─────────────────
        "math_abs" => "mesh_math_abs".to_string(),
        "math_min" => "mesh_math_min".to_string(),
        "math_max" => "mesh_math_max".to_string(),
        "math_pi" => "mesh_math_pi".to_string(),
        "math_pow" => "mesh_math_pow".to_string(),
        "math_sqrt" => "mesh_math_sqrt".to_string(),
        "math_floor" => "mesh_math_floor".to_string(),
        "math_ceil" => "mesh_math_ceil".to_string(),
        "math_round" => "mesh_math_round".to_string(),
        "int_to_float" => "mesh_int_to_float".to_string(),
        "int_to_string" => "mesh_int_to_string".to_string(),
        "float_to_int" => "mesh_float_to_int".to_string(),
        "float_to_string" => "mesh_float_to_string".to_string(),
        // ── Phase 77: From conversion dispatch ──────────────────────────
        "float_from" => "mesh_int_to_float".to_string(),
        "string_from" => "mesh_string_from".to_string(),
        // ── Timer functions (Phase 44 Plan 02) ──────────────────────────
        "timer_sleep" => "mesh_timer_sleep".to_string(),
        "timer_send_after" => "mesh_timer_send_after".to_string(),
        "timer_apply_after" => "mesh_timer_apply_after".to_string(),
        // ── WebSocket functions (Phase 60) ────────────────────────────
        "ws_serve" => "mesh_ws_serve".to_string(),
        "ws_send" => "mesh_ws_send".to_string(),
        "ws_serve_tls" => "mesh_ws_serve_tls".to_string(),
        // ── WebSocket Room functions (Phase 62) ────────────────────────
        "ws_join" => "mesh_ws_join".to_string(),
        "ws_leave" => "mesh_ws_leave".to_string(),
        "ws_broadcast" => "mesh_ws_broadcast".to_string(),
        "ws_broadcast_except" => "mesh_ws_broadcast_except".to_string(),
        // ── Phase 67: Node distribution functions ─────────────────────────
        "node_start" => "mesh_node_start".to_string(),
        "node_start_from_env" => "mesh_node_start_from_env".to_string(),
        "node_connect" => "mesh_node_connect".to_string(),
        "node_self" => "mesh_node_self".to_string(),
        "node_list" => "mesh_node_list".to_string(),
        "node_monitor" => "mesh_node_monitor".to_string(),
        "node_spawn" => "mesh_node_spawn".to_string(),
        "node_spawn_link" => "mesh_node_spawn_link".to_string(),
        // ── Phase 67: Process monitor/demonitor ───────────────────────────
        "process_monitor" => "mesh_process_monitor".to_string(),
        "process_demonitor" => "mesh_process_demonitor".to_string(),
        "process_register" => "mesh_process_register".to_string(),
        "process_whereis" => "mesh_process_whereis".to_string(),
        "process_install_shutdown_signals" => "mesh_process_install_shutdown_signals".to_string(),
        "process_shutdown_requested" => "mesh_process_shutdown_requested".to_string(),
        "process_request_shutdown" => "mesh_process_request_shutdown".to_string(),
        "process_exit" => "mesh_process_exit".to_string(),
        // ── Phase 68: Global registry functions ─────────────────────────
        "global_register" => "mesh_global_register".to_string(),
        "global_whereis" => "mesh_global_whereis".to_string(),
        "global_unregister" => "mesh_global_unregister".to_string(),
        // ── continuity: Continuity runtime functions ───────────────────────────
        "continuity_submit" => "mesh_continuity_submit_with_durability".to_string(),
        "continuity_submit_declared_work" => "mesh_continuity_submit_declared_work".to_string(),
        "continuity_status" => "mesh_continuity_status".to_string(),
        "continuity_authority_status" => "mesh_continuity_authority_status".to_string(),
        "continuity_mark_completed" => "mesh_continuity_mark_completed".to_string(),
        "continuity_acknowledge_replica" => "mesh_continuity_acknowledge_replica".to_string(),
        // ── Phase 88: WebSocket functions (handled above in Phase 60)
        // ── Phase 76: Iterator functions ──────────────────────────────
        "iter_from" => "mesh_iter_from".to_string(),
        // ── Phase 78: Lazy Combinators & Terminals ──────────────────
        "iter_map" => "mesh_iter_map".to_string(),
        "iter_filter" => "mesh_iter_filter".to_string(),
        "iter_take" => "mesh_iter_take".to_string(),
        "iter_skip" => "mesh_iter_skip".to_string(),
        "iter_enumerate" => "mesh_iter_enumerate".to_string(),
        "iter_zip" => "mesh_iter_zip".to_string(),
        "iter_count" => "mesh_iter_count".to_string(),
        "iter_sum" => "mesh_iter_sum".to_string(),
        "iter_any" => "mesh_iter_any".to_string(),
        "iter_all" => "mesh_iter_all".to_string(),
        "iter_find" => "mesh_iter_find".to_string(),
        "iter_next" => "mesh_iter_generic_next".to_string(),
        "iter_reduce" => "mesh_iter_reduce".to_string(),
        // ── Phase 79: Collect terminal operations ────────────────────────
        "list_collect" => "mesh_list_collect".to_string(),
        "map_collect" => "mesh_map_collect".to_string(),
        "set_collect" => "mesh_set_collect".to_string(),
        "string_collect" => "mesh_string_collect".to_string(),
        _ => name.to_string(),
    }
}

fn parse_int_literal(text: &str) -> Option<i64> {
    let normalized = text.replace('_', "");
    let (digits, radix) = if let Some(digits) = normalized
        .strip_prefix("0x")
        .or_else(|| normalized.strip_prefix("0X"))
    {
        (digits, 16)
    } else if let Some(digits) = normalized
        .strip_prefix("0b")
        .or_else(|| normalized.strip_prefix("0B"))
    {
        (digits, 2)
    } else if let Some(digits) = normalized
        .strip_prefix("0o")
        .or_else(|| normalized.strip_prefix("0O"))
    {
        (digits, 8)
    } else {
        (normalized.as_str(), 10)
    };
    // `9223372036854775808` only appears negated (the checker rejects it
    // otherwise): it wraps to i64::MIN, which negation leaves as is.
    u64::from_str_radix(digits, radix)
        .ok()
        .map(|value| value as i64)
}

fn parse_float_literal(text: &str) -> Option<f64> {
    text.replace('_', "").parse().ok()
}

/// Convert a PascalCase name to snake_case.
fn to_snake_case(name: &str) -> String {
    let mut result = String::new();
    for (i, ch) in name.chars().enumerate() {
        if ch.is_uppercase() {
            if i > 0 {
                result.push('_');
            }
            result.push(ch.to_lowercase().next().unwrap());
        } else {
            result.push(ch);
        }
    }
    result
}

/// Process a STRING_CONTENT segment from a triple-quoted heredoc.
///
/// - `is_first`: strip the leading newline (the one after the opening `"""`)
/// - `trim_level`: strip this many leading spaces from each line
/// - For the last segment, the last line contains only the closing indent — it is dropped.
///   Detection: if the last line is all-whitespace (pure spaces/tabs), it is the closing
///   indent line and must be stripped.
fn apply_heredoc_content(text: String, is_first: bool, is_last: bool, trim_level: usize) -> String {
    // Strip leading newline from first segment
    let s: String = if is_first {
        text.strip_prefix("\r\n")
            .or_else(|| text.strip_prefix('\n'))
            .map_or_else(|| text.clone(), str::to_string)
    } else {
        text
    };

    // Split into lines to process each one
    let mut lines: Vec<&str> = s.split('\n').collect();

    // The closing segment ends with the indent line before `"""`: drop it.
    // An earlier segment's whitespace-only last line is the indent before an
    // interpolation that starts a line; it is dedented like any other line.
    if is_last
        && lines
            .last()
            .map(|l| l.chars().all(|c| c == ' ' || c == '\t'))
            .unwrap_or(false)
    {
        lines.pop();
    }

    let stripped_lines: Vec<String> = lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            if i == 0 && !is_first {
                // Text right after an interpolation continues that line.
                return line.to_string();
            }
            // Strip the common indentation, or as much of it as the line has.
            let leading_ws: usize = line.chars().take_while(|c| *c == ' ' || *c == '\t').count();
            line[leading_ws.min(trim_level)..].to_string()
        })
        .collect();

    stripped_lines.join("\n")
}

/// Process escape sequences in a raw string token, converting `\"` → `"`,
/// `\\` → `\`, `\n` → newline, `\t` → tab, `\r` → carriage return, and
/// `\0` → null. Any other `\X` sequence passes through `X` literally.
fn unescape_string(raw: &str) -> String {
    let mut result = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            result.push(c);
            continue;
        }
        // The lexer ends no string content with a lone backslash, and the
        // parser admits only the escapes below (`\$` and `\#` are the
        // character itself).
        match chars.next().expect("a backslash escapes a character") {
            'n' => result.push('\n'),
            't' => result.push('\t'),
            'r' => result.push('\r'),
            '0' => result.push('\0'),
            // `\u{1F389}`.
            'u' => {
                let rest: String = chars.clone().collect();
                let (digits, _) = rest
                    .strip_prefix('{')
                    .and_then(|r| r.split_once('}'))
                    .expect("the parser admits only a well-formed unicode escape");
                result.extend(
                    u32::from_str_radix(digits, 16)
                        .ok()
                        .and_then(char::from_u32),
                );
                for _ in 0..digits.len() + 2 {
                    chars.next();
                }
            }
            other => result.push(other),
        }
    }
    result
}

/// Extract simple string content from a LITERAL or STRING_EXPR syntax node.
/// Walks children looking for STRING_CONTENT tokens and concatenates them;
/// a heredoc is dedented as in an expression, so a heredoc pattern matches
/// the same heredoc as a value.
fn extract_simple_string_content(node: &mesh_parser::cst::SyntaxNode) -> String {
    let is_triple = node
        .children_with_tokens()
        .filter_map(|c| c.into_token())
        .find(|t| t.kind() == SyntaxKind::STRING_START)
        .is_some_and(|t| t.text().starts_with("\"\"\""));
    let contents: Vec<String> = node
        .children_with_tokens()
        .filter_map(|c| c.into_token())
        .filter(|t| t.kind() == SyntaxKind::STRING_CONTENT)
        .map(|t| {
            if is_triple {
                t.text().replace("\r\n", "\n")
            } else {
                t.text().to_string()
            }
        })
        .collect();
    if !is_triple {
        return contents.iter().map(|c| unescape_string(c)).collect();
    }
    let trim_level = contents
        .last()
        .and_then(|last| last.split('\n').next_back())
        .map(|line| line.chars().take_while(|c| *c == ' ' || *c == '\t').count())
        .unwrap_or(0);
    let count = contents.len();
    contents
        .iter()
        .enumerate()
        .map(|(i, text)| {
            apply_heredoc_content(unescape_string(text), i == 0, i + 1 == count, trim_level)
        })
        .collect()
}

/// The value a literal pattern matches, sign included: `-1` is a MINUS and
/// then the `1` that `LiteralPat::token` returns. `None` for `nil`.
fn literal_pattern_value(lit: &mesh_parser::ast::pat::LiteralPat) -> Option<MirLiteral> {
    let token = lit.token()?;
    let text = token.text();
    let negative = lit.is_negative();
    Some(match token.kind() {
        SyntaxKind::INT_LITERAL => {
            let value = parse_int_literal(text).unwrap_or(0);
            MirLiteral::Int(if negative {
                value.wrapping_neg()
            } else {
                value
            })
        }
        SyntaxKind::FLOAT_LITERAL => {
            let value = parse_float_literal(text).unwrap_or(0.0);
            MirLiteral::Float(if negative { -value } else { value })
        }
        SyntaxKind::TRUE_KW => MirLiteral::Bool(true),
        SyntaxKind::FALSE_KW => MirLiteral::Bool(false),
        SyntaxKind::STRING_START => MirLiteral::String(extract_simple_string_content(lit.syntax())),
        // Atoms are their names at run time, as `:ok` in an expression is.
        SyntaxKind::ATOM_LITERAL => MirLiteral::String(text.trim_start_matches(':').to_string()),
        _ => return None,
    })
}

/// Find the type name that contains a variant, preferring the type inferred at
/// the use site when separate modules export constructors with the same name.
fn find_type_for_variant(
    variant: &str,
    expected: Option<&MirType>,
    registry: &mesh_typeck::TypeRegistry,
    arity: Option<usize>,
) -> Option<String> {
    let contains_variant = |info: &&mesh_typeck::SumTypeDefInfo| {
        info.variants.iter().any(|value| {
            value.name == variant && arity.is_none_or(|arity| value.fields.len() == arity)
        })
    };

    if let Some(MirType::SumType(expected_name)) = expected {
        if let Some((type_name, _)) = registry
            .sum_type_defs
            .iter()
            .filter(|(type_name, info)| {
                (expected_name == *type_name || expected_name.starts_with(&format!("{type_name}_")))
                    && contains_variant(info)
            })
            .max_by_key(|(type_name, _)| type_name.len())
        {
            return Some(type_name.clone());
        }
    }

    registry
        .sum_type_defs
        .iter()
        .find(|(_, info)| contains_variant(info))
        .map(|(type_name, _)| type_name.clone())
}

/// Collect bindings introduced by a list of patterns (for constructor pattern bindings).
fn collect_pattern_bindings(patterns: &[MirPattern]) -> Vec<(String, MirType)> {
    let mut bindings = Vec::new();
    for pat in patterns {
        pattern_bindings(pat, &mut bindings);
    }
    bindings
        .into_iter()
        .map(|(name, ty)| (name.to_string(), ty.clone()))
        .collect()
}

/// The variables `pattern` binds, with their types, in order. The
/// alternatives of an or-pattern bind the same variables, so the first's
/// stand for them all.
fn pattern_bindings<'a>(pattern: &'a MirPattern, out: &mut Vec<(&'a str, &'a MirType)>) {
    match pattern {
        MirPattern::Var(name, ty) => out.push((name, ty)),
        MirPattern::As { name, ty, inner } => {
            out.push((name, ty));
            pattern_bindings(inner, out);
        }
        MirPattern::Constructor { fields, .. } | MirPattern::Tuple(fields) => {
            fields.iter().for_each(|field| pattern_bindings(field, out))
        }
        MirPattern::Struct { fields, .. } => fields
            .iter()
            .for_each(|(_, _, field)| pattern_bindings(field, out)),
        MirPattern::Or(alternatives) => alternatives
            .iter()
            .take(1)
            .for_each(|first| pattern_bindings(first, out)),
        MirPattern::ListCons { head, tail, .. } => {
            pattern_bindings(head, out);
            pattern_bindings(tail, out);
        }
        MirPattern::Wildcard | MirPattern::Literal(_) | MirPattern::ListNil => {}
    }
}

/// The outer variables `expr` uses, in order of first use: each `Var`
/// naming one of `outer_vars` that neither `bound` (the closure's
/// parameters) nor anything inside `expr` binds first: a `let`, a loop
/// variable or a pattern.
fn collect_free_vars<'a>(
    expr: &'a MirExpr,
    bound: &HashSet<&'a str>,
    outer_vars: &HashMap<String, MirType>,
    captures: &mut Vec<(String, MirType)>,
) {
    let binding = |names: &[&'a str]| {
        let mut inner = bound.clone();
        inner.extend(names);
        inner
    };
    let arm = |arm: &'a MirMatchArm, captures: &mut Vec<(String, MirType)>| {
        let mut names = Vec::new();
        pattern_names(&arm.pattern, &mut names);
        let inner = binding(&names);
        for part in arm.guard.iter().chain([&arm.body]) {
            collect_free_vars(part, &inner, outer_vars, captures);
        }
    };
    match expr {
        MirExpr::Var(name, _) => {
            if !bound.contains(name.as_str())
                && name != "__env"
                && !captures.iter().any(|(captured, _)| captured == name)
            {
                if let Some(ty) = outer_vars.get(name) {
                    captures.push((name.clone(), ty.clone()));
                }
            }
        }
        MirExpr::Let {
            name, value, body, ..
        } => {
            collect_free_vars(value, bound, outer_vars, captures);
            collect_free_vars(body, &binding(&[name]), outer_vars, captures);
        }
        MirExpr::Match {
            scrutinee, arms, ..
        } => {
            collect_free_vars(scrutinee, bound, outer_vars, captures);
            arms.iter().for_each(|each| arm(each, captures));
        }
        MirExpr::ActorReceive {
            handler,
            timeout_ms,
            timeout_body,
            ..
        } => {
            if let Some((name, _, body)) = handler {
                collect_free_vars(body, &binding(&[name]), outer_vars, captures);
            }
            for part in timeout_ms.iter().chain(timeout_body) {
                collect_free_vars(part, bound, outer_vars, captures);
            }
        }
        // A loop's variables are bound in its filter and body, not in the
        // range or collection it runs over.
        MirExpr::ForInRange {
            var,
            start,
            end,
            filter,
            body,
            ..
        } => {
            collect_free_vars(start, bound, outer_vars, captures);
            collect_free_vars(end, bound, outer_vars, captures);
            let inner = binding(&[var]);
            for part in filter.iter().chain([body]) {
                collect_free_vars(part, &inner, outer_vars, captures);
            }
        }
        MirExpr::ForInList {
            var,
            collection,
            filter,
            body,
            ..
        }
        | MirExpr::ForInSet {
            var,
            collection,
            filter,
            body,
            ..
        }
        | MirExpr::ForInIterator {
            var,
            iterator: collection,
            filter,
            body,
            ..
        } => {
            collect_free_vars(collection, bound, outer_vars, captures);
            let inner = binding(&[var]);
            for part in filter.iter().chain([body]) {
                collect_free_vars(part, &inner, outer_vars, captures);
            }
        }
        MirExpr::ForInMap {
            key_var,
            val_var,
            collection,
            filter,
            body,
            ..
        } => {
            collect_free_vars(collection, bound, outer_vars, captures);
            let inner = binding(&[key_var, val_var]);
            for part in filter.iter().chain([body]) {
                collect_free_vars(part, &inner, outer_vars, captures);
            }
        }
        _ => {
            for child in expr.children() {
                collect_free_vars(child, bound, outer_vars, captures);
            }
        }
    }
}

// ── TCE rewrite pass ─────────────────────────────────────────────────

/// Post-lowering rewrite pass: detect self-recursive calls in tail position
/// and rewrite them to TailCall nodes. Returns true if any rewrites were made.
fn rewrite_tail_calls(expr: &mut MirExpr, current_fn_name: &str) -> bool {
    TailCalls {
        name: current_fn_name,
        cleanups: Vec::new(),
        temps: 0,
    }
    .rewrite(expr)
}

/// The walk of `rewrite_tail_calls`. A resource scope, `let r = value;
/// drop; r` (see `Lowerer::wrap_resource_scope`), keeps its value in tail
/// position: a self-call ending it evaluates its arguments, runs the drops
/// of the scopes around it (`cleanups`, innermost last), then jumps. The
/// call used to end up in the scope's value, out of tail position, so a
/// function holding a resource recursed on the stack.
struct TailCalls<'a> {
    name: &'a str,
    cleanups: Vec<MirExpr>,
    temps: usize,
}

impl TailCalls<'_> {
    fn rewrite(&mut self, expr: &mut MirExpr) -> bool {
        match expr {
            MirExpr::Call { func, args, ty } => {
                if !matches!(func.as_ref(), MirExpr::Var(name, _) if name == self.name) {
                    return false;
                }
                // The jump drops what the scopes around it own: an argument
                // still reading one of those (lending it to the next call)
                // needs it alive, so the call stays an ordinary call.
                if args.iter().any(|arg| self.reads_dropped(arg)) {
                    return false;
                }
                let args = std::mem::take(args);
                let ty = ty.clone();
                *expr = self.tail_call(args, ty);
                true
            }
            MirExpr::Block(exprs, _) => {
                // Only the LAST expression in a block is in tail position
                exprs.last_mut().is_some_and(|last| self.rewrite(last))
            }
            MirExpr::Let {
                name, value, body, ..
            } if name.starts_with(RESOURCE_TEMP_PREFIX) => {
                // A resource scope's body is a block of its cleanup, then its
                // result (`wrap_resource_scope`, `cleanup_before_exits`).
                let cleanup = body.children()[0].clone();
                self.cleanups.push(cleanup);
                let rewritten = self.rewrite(value);
                self.cleanups.pop();
                // A value that cannot finish (every way through it ends in the
                // jump, or returns) never reaches the scope's own drop and
                // result: emitted after it, they would follow a terminator.
                if !Lowerer::can_fall_through(value) {
                    let value = std::mem::replace(value.as_mut(), MirExpr::Unit);
                    *expr = value;
                }
                rewritten
            }
            MirExpr::Let { body, .. } => {
                // The body (continuation) of a let is in tail position; the value is NOT
                self.rewrite(body)
            }
            MirExpr::If {
                then_body,
                else_body,
                ..
            } => {
                // BOTH branches are in tail position; the condition is NOT
                let a = self.rewrite(then_body);
                let b = self.rewrite(else_body);
                a || b
            }
            MirExpr::Match { arms, .. } => {
                // All arm bodies are in tail position; the scrutinee is NOT
                let mut any = false;
                for arm in arms.iter_mut() {
                    any |= self.rewrite(&mut arm.body);
                }
                any
            }
            MirExpr::ActorReceive {
                handler,
                timeout_body,
                ..
            } => {
                // The handler's body and the timeout body are in tail position
                let handled = handler
                    .as_mut()
                    .is_some_and(|(_, _, body)| self.rewrite(body));
                let timed_out = timeout_body
                    .as_deref_mut()
                    .is_some_and(|tb| self.rewrite(tb));
                handled || timed_out
            }
            MirExpr::Return(inner) => {
                // The inner expression of Return IS in tail position. A tail call
                // jumps back to the top of the function, so `return self(...)` is
                // the tail call itself: a `ret` after the jump would be a second
                // terminator in the block.
                let rewritten = self.rewrite(inner);
                if !Lowerer::can_fall_through(inner) {
                    let tail_call = std::mem::replace(inner.as_mut(), MirExpr::Unit);
                    *expr = tail_call;
                }
                rewritten
            }
            // Everything else is NOT a tail context -- do NOT recurse.
            // This includes: BinOp, UnaryOp, Call (non-self), ClosureCall, StructLit,
            // FieldAccess, ConstructVariant, MakeClosure, ListLit, While, ForIn*, etc.
            _ => false,
        }
    }

    /// Whether `expr` reads a resource the scopes around the walk drop,
    /// other than by moving it out.
    fn reads_dropped(&self, expr: &MirExpr) -> bool {
        match expr {
            MirExpr::ResourceMove {
                source: MirResourceMoveSource::Slot(_),
                ..
            } => false,
            MirExpr::Var(name, _) => self.cleanups.iter().any(|cleanup| {
                matches!(cleanup, MirExpr::ResourceDrop { value, .. }
                    if matches!(value.as_ref(), MirExpr::Var(dropped, _) if dropped == name))
            }),
            other => other
                .children()
                .into_iter()
                .any(|child| self.reads_dropped(child)),
        }
    }

    /// The jump back to the top of the function with `args`, after the
    /// drops of the scopes it leaves: the arguments are evaluated first,
    /// since they may read what the drops destroy.
    fn tail_call(&mut self, args: Vec<MirExpr>, ty: MirType) -> MirExpr {
        if self.cleanups.is_empty() {
            return MirExpr::TailCall { args, ty };
        }
        let mut evaluated = Vec::with_capacity(args.len());
        let mut tail_args = Vec::with_capacity(args.len());
        for argument in args {
            let argument_ty = effective_return_type(&argument);
            let name = format!("__tail_arg_{}", self.temps);
            self.temps += 1;
            tail_args.push(MirExpr::Var(name.clone(), argument_ty.clone()));
            evaluated.push((name, argument_ty, argument));
        }
        let mut steps: Vec<MirExpr> = self.cleanups.iter().rev().cloned().collect();
        steps.push(MirExpr::TailCall {
            args: tail_args,
            ty,
        });
        let mut result = MirExpr::Block(steps, MirType::Never);
        for (name, argument_ty, argument) in evaluated.into_iter().rev() {
            result = MirExpr::Let {
                name,
                ty: argument_ty,
                value: Box::new(argument),
                body: Box::new(result),
            };
        }
        result
    }
}

/// Builtins with no function of their own: codegen expands a call of one
/// where it is (`String.from` is chosen by its argument's type in lowering).
const INLINE_BUILTINS: &[&str] = &[
    "mesh_float_to_int",
    "mesh_int_to_float",
    "mesh_math_abs",
    "mesh_math_ceil",
    "mesh_math_floor",
    "mesh_math_max",
    "mesh_math_min",
    "mesh_math_pow",
    "mesh_math_round",
    "mesh_math_sqrt",
    "mesh_string_from",
];

/// An operand that never finishes (a `panic`, `return`, `break` or
/// `continue`, or a call that never returns) ends what uses it: the rest
/// never runs, and codegen would put it after the block's end. Such an
/// expression becomes a block of its operands up to that one.
fn stop_at_never(expr: &mut MirExpr) {
    for child in expr.children_mut() {
        stop_at_never(child);
    }
    let mut operands = eager_operands(expr);
    let Some(last) = operands.iter().position(|op| *op.ty() == MirType::Never) else {
        return;
    };
    let run = operands
        .drain(..=last)
        .map(|op| std::mem::replace(op, MirExpr::Unit))
        .collect();
    *expr = MirExpr::Block(run, MirType::Never);
}

/// The operands `expr` evaluates, in order, before anything else it does.
fn eager_operands(expr: &mut MirExpr) -> Vec<&mut MirExpr> {
    match expr {
        MirExpr::BinOp {
            op: BinOp::And | BinOp::Or,
            lhs,
            ..
        } => vec![lhs],
        MirExpr::BinOp { lhs, rhs, .. }
        | MirExpr::ActorSend {
            target: lhs,
            message: rhs,
            ..
        }
        | MirExpr::ForInRange {
            start: lhs,
            end: rhs,
            ..
        } => vec![lhs, rhs],
        MirExpr::UnaryOp { operand, .. }
        | MirExpr::FieldAccess {
            object: operand, ..
        }
        | MirExpr::Let { value: operand, .. }
        | MirExpr::If { cond: operand, .. }
        | MirExpr::Match {
            scrutinee: operand, ..
        }
        | MirExpr::Return(operand)
        | MirExpr::ResourceMove { value: operand, .. }
        | MirExpr::ResourceBorrow { value: operand, .. }
        | MirExpr::ResourceDrop { value: operand, .. }
        | MirExpr::ResourceDestroy { value: operand, .. }
        | MirExpr::Shaped { value: operand, .. }
        | MirExpr::ActorLink {
            target: operand, ..
        }
        | MirExpr::ForInList {
            collection: operand,
            ..
        }
        | MirExpr::ForInMap {
            collection: operand,
            ..
        }
        | MirExpr::ForInSet {
            collection: operand,
            ..
        }
        | MirExpr::ForInIterator {
            iterator: operand, ..
        } => vec![operand],
        MirExpr::Call { func, args, .. }
        | MirExpr::ClosureCall {
            closure: func,
            args,
            ..
        }
        | MirExpr::ActorSpawn { func, args, .. } => {
            std::iter::once(&mut **func).chain(args).collect()
        }
        MirExpr::StructUpdate {
            base, overrides, ..
        } => std::iter::once(&mut **base)
            .chain(overrides.iter_mut().map(|(_, value)| value))
            .collect(),
        MirExpr::StructLit { fields, .. } => fields.iter_mut().map(|(_, value)| value).collect(),
        MirExpr::ListLit { elements: args, .. }
        | MirExpr::ConstructVariant { fields: args, .. }
        | MirExpr::MakeClosure { captures: args, .. }
        | MirExpr::TailCall { args, .. } => args.iter_mut().collect(),
        _ => Vec::new(),
    }
}

/// A builtin used as a value (`List.map(xs, Int.to_float)`, `let f =
/// Math.sqrt`, `let m = List.map`) refers to a wrapper function that calls
/// it: many builtins are expanded inline where they are called and have no
/// function of their own to point at ("Undefined variable
/// 'mesh_math_sqrt'"), and a runtime function takes a function argument as
/// two pointers, where a function value takes it whole. The names of the
/// wrappers of runtime functions that take a function come back, for their
/// callbacks to be adapted as a direct call's are.
fn wrap_builtin_values(functions: &mut Vec<MirFunction>) -> Vec<String> {
    let defined: HashSet<String> = functions.iter().map(|f| f.name.clone()).collect();
    let mut wrappers: Vec<MirFunction> = Vec::new();
    for function in functions.iter_mut() {
        let mut bound: HashSet<String> = function.params.iter().map(|(n, _)| n.clone()).collect();
        bound.extend(function.captures.iter().map(|(n, _)| n.clone()));
        collect_bound_names(&function.body, &mut bound);
        wrap_builtin_values_in(&mut function.body, &defined, &bound, &mut wrappers);
    }
    let taking_functions = wrappers
        .iter()
        .filter(|wrapper| wrapper.params.iter().any(|(_, ty)| is_function_type(ty)))
        .map(|wrapper| wrapper.name.clone())
        .collect();
    functions.extend(wrappers);
    taking_functions
}

fn is_function_type(ty: &MirType) -> bool {
    ty.function_parts().is_some()
}

/// Whether the function type `ty` has a parameter that is a function.
fn takes_function(ty: &MirType) -> bool {
    ty.function_parts()
        .is_some_and(|(params, _)| params.iter().any(is_function_type))
}

fn wrap_builtin_values_in(
    expr: &mut MirExpr,
    defined: &HashSet<String>,
    bound: &HashSet<String>,
    wrappers: &mut Vec<MirFunction>,
) {
    match expr {
        // A builtin called directly is expanded where it is.
        MirExpr::Call { func, args, .. } => {
            if !matches!(func.as_ref(), MirExpr::Var(..)) {
                wrap_builtin_values_in(func, defined, bound, wrappers);
            }
            for arg in args {
                wrap_builtin_values_in(arg, defined, bound, wrappers);
            }
        }
        MirExpr::Var(name, ty)
            if (INLINE_BUILTINS.contains(&name.as_str())
                || name.starts_with("mesh_") && takes_function(ty))
                && !defined.contains(name)
                && !bound.contains(name) =>
        {
            // A builtin named as a value has its function type.
            let (params, ret) = ty
                .function_parts()
                .map(|(params, ret)| (params.to_vec(), ret.clone()))
                .expect("a builtin function value has a function type");
            let signature: String = format!("{params:?}{ret:?}")
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                .collect();
            let wrapper = format!("__mesh_value_{name}__{signature}");
            if !wrappers.iter().any(|w| w.name == wrapper) {
                let args: Vec<MirExpr> = params
                    .iter()
                    .enumerate()
                    .map(|(i, ty)| MirExpr::Var(format!("__arg{i}"), ty.clone()))
                    .collect();
                wrappers.push(MirFunction {
                    name: wrapper.clone(),
                    params: params
                        .iter()
                        .enumerate()
                        .map(|(i, ty)| (format!("__arg{i}"), ty.clone()))
                        .collect(),
                    return_type: ret.clone(),
                    body: builtin_call(name, &params, &ret, args),
                    is_closure_fn: false,
                    captures: Vec::new(),
                    has_tail_calls: false,
                });
            }
            *name = wrapper;
        }
        other => {
            for child in other.children_mut() {
                wrap_builtin_values_in(child, defined, bound, wrappers);
            }
        }
    }
}

/// A direct call of the builtin `name`. `String.from` is chosen by its
/// argument's type where it is called; here the type is the parameter's.
fn builtin_call(name: &str, params: &[MirType], ret: &MirType, args: Vec<MirExpr>) -> MirExpr {
    let name = match (name, params) {
        ("mesh_string_from", [MirType::String]) => {
            return args.into_iter().next().unwrap_or(MirExpr::Unit)
        }
        ("mesh_string_from", [MirType::Int]) => "mesh_int_to_string",
        ("mesh_string_from", [MirType::Float]) => "mesh_float_to_string",
        ("mesh_string_from", [MirType::Bool]) => "mesh_bool_to_string",
        (name, _) => name,
    };
    MirExpr::Call {
        func: Box::new(MirExpr::Var(
            name.to_string(),
            MirType::FnPtr(params.to_vec(), Box::new(ret.clone())),
        )),
        args,
        ty: ret.clone(),
    }
}

/// Whether `ty` is a process's PID, typed or not: an integer at run time.
fn is_pid(ty: &Ty) -> bool {
    matches!(ty_head(ty), Some(("Pid", _)))
}

/// The variables a pattern binds.
fn pattern_names<'a>(pattern: &'a MirPattern, names: &mut Vec<&'a str>) {
    let mut bindings = Vec::new();
    pattern_bindings(pattern, &mut bindings);
    names.extend(bindings.into_iter().map(|(name, _)| name));
}

/// Every name `expr` binds: `let`s, loop variables and pattern variables.
fn collect_bound_names(expr: &MirExpr, bound: &mut HashSet<String>) {
    for node in expr.descendants() {
        let mut names = Vec::new();
        match node {
            MirExpr::Let { name, .. } => names.push(name.as_str()),
            MirExpr::ForInRange { var, .. }
            | MirExpr::ForInList { var, .. }
            | MirExpr::ForInSet { var, .. }
            | MirExpr::ForInIterator { var, .. } => names.push(var),
            MirExpr::ForInMap {
                key_var, val_var, ..
            } => names.extend([key_var.as_str(), val_var.as_str()]),
            MirExpr::Match { arms, .. } => arms
                .iter()
                .for_each(|arm| pattern_names(&arm.pattern, &mut names)),
            MirExpr::ActorReceive {
                handler: Some((name, ..)),
                ..
            } => names.push(name),
            _ => {}
        }
        bound.extend(names.into_iter().map(str::to_string));
    }
}

// ── Public API ───────────────────────────────────────────────────────

/// Lower a parsed and type-checked Mesh program to MIR.
///
/// This is the main entry point for AST-to-MIR conversion. It walks the
/// typed AST, desugars pipe operators and string interpolation, lifts closures,
/// and produces a flat MIR module.
pub fn lower_to_mir(
    parse: &Parse,
    typeck: &TypeckResult,
    module_name: &str,
    pub_fns: &HashSet<String>,
    inferred_fn_usage_types: &HashMap<String, Vec<Ty>>,
) -> Result<MirModule, String> {
    lower_module_to_mir(
        parse,
        typeck,
        module_name,
        pub_fns,
        inferred_fn_usage_types,
        &[],
    )
}

/// The specializations one module of a project needs of the other modules'
/// generic functions, by name: a call in one of its generic functions takes
/// a concrete type in each specialization `inferred_fn_usage_types` gives
/// that function. Importers come before what they import in reverse
/// compilation order, so taken in that order every module is asked for what
/// its callers need before it asks its own imports.
pub fn imported_specializations(
    parse: &Parse,
    typeck: &TypeckResult,
    module_name: &str,
    pub_fns: &HashSet<String>,
    inferred_fn_usage_types: &HashMap<String, Vec<Ty>>,
) -> HashMap<String, Vec<Ty>> {
    let mut lowerer = Lowerer::new(typeck, parse, module_name, pub_fns, inferred_fn_usage_types);
    lowerer.prepare_specializations(&parse.tree());
    lowerer.imported_specializations
}

/// A default method body of an interface declared in another module.
#[derive(Clone, Copy)]
struct ForeignDefault<'a> {
    parse: &'a Parse,
    types: &'a FxHashMap<TextRange, Ty>,
    range: TextRange,
}

/// `lower_to_mir` for one module of a project: `other_modules` are the
/// project's other modules, whose interfaces' default methods this
/// module's impls may inherit.
pub fn lower_module_to_mir<'a>(
    parse: &'a Parse,
    typeck: &'a TypeckResult,
    module_name: &str,
    pub_fns: &HashSet<String>,
    inferred_fn_usage_types: &HashMap<String, Vec<Ty>>,
    other_modules: &[(&'a Parse, &'a TypeckResult)],
) -> Result<MirModule, String> {
    if let Some(reason) = typeck.errors.iter().find_map(|error| match error {
        TypeError::ResourceViolation { reason, .. }
            if reason.contains("cannot be captured by a closure") =>
        {
            Some(reason)
        }
        _ => None,
    }) {
        return Err(format!("unsafe resource closure rejected: {reason}"));
    }

    let source_file = parse.tree();

    let mut lowerer = Lowerer::new(typeck, parse, module_name, pub_fns, inferred_fn_usage_types);
    for &(other_parse, other) in other_modules {
        for (key, &range) in &other.default_method_bodies {
            if !typeck.default_method_bodies.contains_key(key) {
                lowerer.foreign_defaults.insert(
                    key.clone(),
                    ForeignDefault {
                        parse: other_parse,
                        types: &other.types,
                        range,
                    },
                );
            }
        }
    }

    // Also register builtin sum types from the registry (Option, Result).
    // Generic type params (T, E) are resolved to Ptr since all Mesh values
    // are heap-allocated pointers at the LLVM level.
    for (name, info) in &typeck.type_registry.sum_type_defs {
        let generic_params: Vec<String> = info.generic_params.clone();
        let variants = info
            .variants
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let fields = v
                    .fields
                    .iter()
                    .map(|f| {
                        let ty = match f {
                            mesh_typeck::VariantFieldInfo::Positional(ty) => ty,
                            mesh_typeck::VariantFieldInfo::Named(_, ty) => ty,
                        };
                        // Check if this is a generic type parameter.
                        // Generic params like T, E resolve to MirType::Struct("T")
                        // because they're not known types. Replace with Ptr since
                        // all variant payloads are pointer-sized at LLVM level.
                        if let Ty::Con(con) = ty {
                            if generic_params.contains(&con.name) {
                                return MirType::Ptr;
                            }
                        }
                        match resolve_type(ty, &typeck.type_registry) {
                            // A recursive payload is boxed like a generic one;
                            // a tuple is a pointer already.
                            MirType::SumType(inner) if lowerer.boxed_payload(name, &inner) => {
                                MirType::Ptr
                            }
                            MirType::Tuple(_) => MirType::Ptr,
                            other => other,
                        }
                    })
                    .collect();
                MirVariantDef {
                    name: v.name.clone(),
                    fields,
                    tag: i as u8,
                }
            })
            .collect();

        lowerer.sum_types.push(MirSumTypeDef {
            name: name.clone(),
            variants,
        });
    }

    // `Ordering` is a built-in sum type with no source definition to derive
    // from; it compares, orders and prints like a derived one.
    let typed: Vec<(String, Vec<Ty>)> = typeck.type_registry.sum_type_defs["Ordering"]
        .variants
        .iter()
        .map(|v| (v.name.clone(), vec![]))
        .collect();
    lowerer.generate_eq_sum_typed("Ordering", &typed);
    lowerer.generate_ord_sum_typed("Ordering", "Ordering", &typed);
    lowerer.generate_display_sum_typed("Ordering", "Ordering", &typed, false);
    lowerer.generate_display_sum_typed("Ordering", "Ordering", &typed, true);

    // Crypto V2 value/keypair structs are registry-backed builtins rather than
    // source declarations, so they need concrete MIR layouts here.
    for name in [
        "X25519PublicKey",
        "MlKemPublicKey",
        "MlKemCiphertext",
        "SigningPublicKey",
        "Signature",
        "X25519KeyPair",
        "MlKemKeyPair",
        "SigningKeyPair",
    ] {
        lowerer.structs.push(MirStructDef {
            name: name.to_string(),
            fields: typeck.type_registry.struct_defs[name]
                .fields
                .iter()
                .map(|(field, ty)| {
                    (
                        field.clone(),
                        runtime_value_type(resolve_type(ty, &typeck.type_registry)),
                    )
                })
                .collect(),
        });
    }

    // Pre-seed stdlib structs for builtin field access (Phase 137+).
    // Layouts MUST match the Mesh-facing runtime structs in mesh-rt exactly.
    lowerer.structs.push(MirStructDef {
        name: "HttpResponse".to_string(),
        fields: vec![
            ("status".to_string(), MirType::Int),
            ("body".to_string(), MirType::Ptr), // *mut MeshString
            ("headers".to_string(), MirType::Ptr), // *mut MeshMap
            ("body_bytes".to_string(), MirType::Ptr), // *mut MeshBytes
        ],
    });
    lowerer.structs.push(MirStructDef {
        name: "HttpClientMetrics".to_string(),
        fields: [
            "requests",
            "in_flight",
            "dns_micros",
            "connect_micros",
            "tls_micros",
            "dns_failures",
            "connect_failures",
            "tls_failures",
            "timeouts",
            "first_byte_micros",
            "total_micros",
            "response_bytes",
            "cancellations",
        ]
        .into_iter()
        .map(|name| (name.to_string(), MirType::Int))
        .collect(),
    });
    lowerer.structs.push(MirStructDef {
        name: "WsMessage".to_string(),
        fields: vec![
            ("kind".to_string(), MirType::String),
            ("data".to_string(), MirType::Ptr),
            ("close_code".to_string(), MirType::Int),
            ("close_reason".to_string(), MirType::String),
        ],
    });
    lowerer.structs.push(MirStructDef {
        name: "BootstrapStatus".to_string(),
        fields: vec![
            ("mode".to_string(), MirType::String),
            ("node_name".to_string(), MirType::String),
            ("cluster_port".to_string(), MirType::Int),
            ("discovery_seed".to_string(), MirType::String),
        ],
    });
    lowerer.structs.push(MirStructDef {
        name: "ContinuityAuthorityStatus".to_string(),
        fields: vec![
            ("cluster_role".to_string(), MirType::String),
            ("promotion_epoch".to_string(), MirType::Int),
            ("replication_health".to_string(), MirType::String),
        ],
    });
    lowerer.structs.push(MirStructDef {
        name: "ContinuityRecord".to_string(),
        fields: vec![
            ("request_key".to_string(), MirType::String),
            ("payload_hash".to_string(), MirType::String),
            ("attempt_id".to_string(), MirType::String),
            ("phase".to_string(), MirType::String),
            ("result".to_string(), MirType::String),
            ("ingress_node".to_string(), MirType::String),
            ("owner_node".to_string(), MirType::String),
            ("replica_node".to_string(), MirType::String),
            ("replication_count".to_string(), MirType::Int),
            ("replica_status".to_string(), MirType::String),
            ("cluster_role".to_string(), MirType::String),
            ("promotion_epoch".to_string(), MirType::Int),
            ("replication_health".to_string(), MirType::String),
            ("execution_node".to_string(), MirType::String),
            ("routed_remotely".to_string(), MirType::Bool),
            ("fell_back_locally".to_string(), MirType::Bool),
            ("error".to_string(), MirType::String),
        ],
    });
    lowerer.structs.push(MirStructDef {
        name: "ContinuitySubmitDecision".to_string(),
        fields: vec![
            ("outcome".to_string(), MirType::String),
            ("conflict_reason".to_string(), MirType::String),
            (
                "record".to_string(),
                MirType::Struct("ContinuityRecord".to_string()),
            ),
        ],
    });

    // Generate Ord__compare__ for built-in primitive types (Int, Float, String, Bool).
    // These use BinOp::Lt and BinOp::Eq directly since primitives don't have
    // generated Ord__lt__ / Eq__eq__ functions.
    lowerer.generate_compare_primitive("Int", MirType::Int);
    lowerer.generate_compare_primitive("Float", MirType::Float);
    lowerer.generate_compare_primitive("String", MirType::String);
    lowerer.generate_compare_primitive("Bool", MirType::Bool);

    // Generate cross-module trait method wrappers for imported structs/sum types.
    // When a struct like User is defined in module A with deriving(Json), module A
    // generates FromJson__from_json__User and __json_decode__User. After MIR merge,
    // these functions are available globally. But the importing module's lowerer
    // needs __json_decode__User in known_functions so that User.from_json(str)
    // resolves correctly in lower_field_access. Generate the thin wrappers here
    // BEFORE lower_source_file so they're available during field access resolution.
    // Nothing has been lowered yet, so none of these is known already.
    {
        let struct_names: Vec<String> = typeck.type_registry.struct_defs.keys().cloned().collect();
        for name in &struct_names {
            let struct_ty = Ty::Con(mesh_typeck::ty::TyCon::new(name));
            // FromJson: generate the __json_decode__ wrapper.
            if typeck.trait_registry.has_impl("FromJson", &struct_ty) {
                lowerer.generate_from_json_string_wrapper(name);
            }

            // ToJson: register known_functions entry for ToJson__to_json__StructName
            // The actual function body is generated in the defining module's MIR.
            if typeck.trait_registry.has_impl("ToJson", &struct_ty) {
                lowerer.known_functions.insert(
                    format!("ToJson__to_json__{}", name),
                    MirType::FnPtr(vec![MirType::Struct(name.clone())], Box::new(MirType::Ptr)),
                );
            }

            // FromRow: register known_functions entry for FromRow__from_row__StructName
            // The actual function body is generated in the defining module's MIR.
            if typeck.trait_registry.has_impl("FromRow", &struct_ty) {
                lowerer.known_functions.insert(
                    format!("FromRow__from_row__{}", name),
                    MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)),
                );
            }
        }

        // Also handle sum types with FromJson
        let sum_names: Vec<String> = typeck.type_registry.sum_type_defs.keys().cloned().collect();
        for name in &sum_names {
            let sum_ty = Ty::Con(mesh_typeck::ty::TyCon::new(name));
            if typeck.trait_registry.has_impl("FromJson", &sum_ty) {
                lowerer.generate_from_json_string_wrapper(name);
            }
        }

        // Schema metadata: register known_functions entries for imported structs
        // with deriving(Schema). The actual function bodies are generated in the
        // defining module's MIR and available after merge. The importing module's
        // lowerer needs these in known_functions so that StructName.__table__(),
        // StructName.__fields__(), etc. resolve correctly in lower_field_access.
        for name in &struct_names {
            let table_fn = format!("{}____table__", name);
            let struct_ty = Ty::Con(mesh_typeck::ty::TyCon::new(name));
            if typeck.trait_registry.has_impl("Schema", &struct_ty) {
                // __table__() -> String
                lowerer
                    .known_functions
                    .insert(table_fn, MirType::FnPtr(vec![], Box::new(MirType::String)));
                // __fields__() -> Ptr (List<String>)
                lowerer.known_functions.insert(
                    format!("{}____fields__", name),
                    MirType::FnPtr(vec![], Box::new(MirType::Ptr)),
                );
                // __primary_key__() -> String
                lowerer.known_functions.insert(
                    format!("{}____primary_key__", name),
                    MirType::FnPtr(vec![], Box::new(MirType::String)),
                );
                // __relationships__() -> Ptr (List<String>)
                lowerer.known_functions.insert(
                    format!("{}____relationships__", name),
                    MirType::FnPtr(vec![], Box::new(MirType::Ptr)),
                );
                // __field_types__() -> Ptr (List<String>)
                lowerer.known_functions.insert(
                    format!("{}____field_types__", name),
                    MirType::FnPtr(vec![], Box::new(MirType::Ptr)),
                );
                // __relationship_meta__() -> Ptr (List<String>)
                lowerer.known_functions.insert(
                    format!("{}____relationship_meta__", name),
                    MirType::FnPtr(vec![], Box::new(MirType::Ptr)),
                );
                // Per-field column accessors: __{field}_col__() -> String
                for (field_name, _) in &typeck.type_registry.struct_defs[name].fields {
                    lowerer.known_functions.insert(
                        format!("{}____{}_col__", name, field_name),
                        MirType::FnPtr(vec![], Box::new(MirType::String)),
                    );
                }
            }
        }
    }

    lowerer.lower_source_file(source_file);

    if !lowerer.lowering_errors.is_empty() {
        return Err(lowerer.lowering_errors.join("\n"));
    }

    let unlowered_clustered_routes = typeck
        .clustered_route_wrappers
        .iter()
        .filter(|&(range, _metadata)| !lowerer.consumed_clustered_route_wrappers.contains(range))
        .map(|(_range, metadata)| metadata.runtime_name.clone())
        .collect::<Vec<_>>();
    if !unlowered_clustered_routes.is_empty() {
        return Err(unlowered_clustered_routes
            .into_iter()
            .map(|runtime_name| {
                format!(
                    "clustered route wrapper `{runtime_name}` did not lower to a concrete route shim"
                )
            })
            .collect::<Vec<_>>()
            .join("\n"));
    }

    // A wrapper passes its callback on as a direct call does.
    for wrapper in wrap_builtin_values(&mut lowerer.functions) {
        let index = lowerer
            .functions
            .iter()
            .position(|function| function.name == wrapper)
            .expect("the wrapper was just added");
        let body = std::mem::replace(&mut lowerer.functions[index].body, MirExpr::Unit);
        lowerer.functions[index].body =
            lowerer.adapt_uniform_callback_call(body, TextRange::default());
    }
    for function in &mut lowerer.functions {
        stop_at_never(&mut function.body);
    }

    Ok(MirModule {
        functions: lowerer.functions,
        native_functions: lowerer.native_functions,
        structs: lowerer.structs,
        sum_types: lowerer.sum_types,
        entry_function: lowerer.entry_function,
        service_dispatch: lowerer.service_dispatch,
        actors: lowerer.actors,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use rustc_hash::{FxHashMap, FxHashSet};

    use super::*;
    use mesh_typeck::ty::{Scheme, Ty, TyCon};
    use mesh_typeck::{ImportContext, ModuleExports};

    /// Helper to parse and type-check a Mesh source, then lower to MIR.
    /// Whether any expression in `expression` (itself included) is `is`.
    fn contains(expression: &MirExpr, is: impl Fn(&MirExpr) -> bool) -> bool {
        expression.descendants().into_iter().any(is)
    }

    /// Whether `expression` calls a function whose name `named` accepts.
    fn calls_where(expression: &MirExpr, named: impl Fn(&str) -> bool) -> bool {
        contains(expression, |node| {
            matches!(node, MirExpr::Call { func, .. }
                if matches!(func.as_ref(), MirExpr::Var(name, _) if named(name)))
        })
    }

    /// Whether `expression` calls the function named `target`.
    fn calls(expression: &MirExpr, target: &str) -> bool {
        calls_where(expression, |name| name == target)
    }

    /// How many times `expression` names the variable `target`.
    fn var_refs(expression: &MirExpr, target: &str) -> usize {
        expression
            .descendants()
            .into_iter()
            .filter(|node| matches!(node, MirExpr::Var(name, _) if name == target))
            .count()
    }

    /// How many resource drops of the variable `target` `expression` has.
    fn drops_of(expression: &MirExpr, target: &str) -> usize {
        expression
            .descendants()
            .into_iter()
            .filter(|node| {
                matches!(node, MirExpr::ResourceDrop { value, .. }
                    if matches!(value.as_ref(), MirExpr::Var(name, _) if name == target))
            })
            .count()
    }

    /// The destructor of the first resource drop in `expression`.
    fn first_destructor(expression: &MirExpr) -> Option<&MirResourceDestructor> {
        expression
            .descendants()
            .into_iter()
            .find_map(|node| match node {
                MirExpr::ResourceDrop { destructor, .. } => Some(destructor),
                _ => None,
            })
    }

    fn lower(source: &str) -> MirModule {
        let parse = mesh_parser::parse(source);
        assert!(parse.errors().is_empty(), "{:?}", parse.errors());
        let typeck = mesh_typeck::check(&parse);
        let empty_pub_fns = HashSet::new();
        // Ignore type errors for MIR lowering tests -- we test lowering, not typeck.
        lower_to_mir(&parse, &typeck, "", &empty_pub_fns, &HashMap::new())
            .expect("MIR lowering failed")
    }

    fn route_handler_ty() -> Ty {
        Ty::fun(
            vec![Ty::Con(TyCon::new("Request"))],
            Ty::Con(TyCon::new("Response")),
        )
    }

    fn route_handler_scheme() -> Scheme {
        Scheme::mono(route_handler_ty())
    }

    fn route_module_exports(module_name: &str, exported_handlers: &[&str]) -> ModuleExports {
        let mut functions = FxHashMap::default();
        for handler in exported_handlers {
            functions.insert((*handler).to_string(), route_handler_scheme());
        }

        ModuleExports {
            module_name: module_name.to_string(),
            functions,
            struct_defs: FxHashMap::default(),
            sum_type_defs: FxHashMap::default(),
            service_defs: FxHashMap::default(),
            actor_defs: FxHashMap::default(),
            private_names: FxHashSet::default(),
            type_aliases: FxHashMap::default(),
            ..ModuleExports::default()
        }
    }

    fn lower_with_imports(source: &str, import_ctx: ImportContext) -> MirModule {
        let parse = mesh_parser::parse(source);
        let typeck = mesh_typeck::check_with_imports(&parse, &import_ctx);
        assert!(
            typeck.errors.is_empty(),
            "expected clustered route lowering fixture to type-check cleanly, got {:?}",
            typeck.errors
        );
        let empty_pub_fns = HashSet::new();
        lower_to_mir(&parse, &typeck, "", &empty_pub_fns, &HashMap::new())
            .expect("MIR lowering failed")
    }

    #[test]
    fn lower_int_literal() {
        let mir = lower("fn answer() -> Int do 42 end");
        assert!(
            !mir.sum_types.is_empty(),
            "expected builtin sum types to survive MIR lowering"
        );
    }

    #[test]
    fn lower_function_def() {
        let mir = lower("fn add(a :: Int, b :: Int) -> Int do a + b end");
        let func = mir.functions.iter().find(|f| f.name == "add");
        assert!(func.is_some(), "Expected 'add' function in MIR");
        let func = func.unwrap();
        assert_eq!(func.params.len(), 2);
        assert_eq!(func.params[0].0, "a");
        assert_eq!(func.params[0].1, MirType::Int);
        assert_eq!(func.params[1].0, "b");
        assert_eq!(func.params[1].1, MirType::Int);
        assert_eq!(func.return_type, MirType::Int);

        // Body should be a BinOp
        assert!(matches!(func.body, MirExpr::BinOp { op: BinOp::Add, .. }));
    }

    #[test]
    fn opaque_resources_have_no_struct_layout_but_resource_structs_do() {
        let mir = lower(
            "resource StorageKey\n\
             resource struct RatchetSecrets do\n\
               root_key :: SecretBytes\n\
             end",
        );

        assert!(
            mir.structs
                .iter()
                .all(|definition| definition.name != "StorageKey"),
            "opaque resources must not expose a forgeable struct representation: {:?}",
            mir.structs
        );
        assert!(
            mir.structs.iter().any(|definition| {
                definition.name == "RatchetSecrets"
                    && definition.fields == vec![("root_key".to_string(), MirType::Ptr)]
            }),
            "resource structs must retain their field layout: {:?}",
            mir.structs
        );
    }

    #[test]
    fn mobile_storage_calls_lower_to_runtime_intrinsics() {
        let mir = lower(
            "fn persist(signing :: SigningPrivateKey, mlkem :: MlKemPrivateKey, context :: Bytes, value :: Bytes) -> Bytes ! CryptoError do\n\
               let key = StorageKey.platform()?\n\
               let _signing_blob = SigningPrivateKey.seal_for_storage(signing, key, context)?\n\
               let _mlkem_blob = MlKemPrivateKey.seal_for_storage(mlkem, key, context)?\n\
               let sealed = StorageKey.seal_bytes(value, key, context)?\n\
               StorageKey.unseal_bytes(sealed, key, context)\n\
             end",
        );
        let lowered = format!("{mir:?}");
        for runtime in [
            "mesh_storage_key_platform",
            "mesh_signing_private_key_seal_for_storage",
            "mesh_mlkem_private_key_seal_for_storage",
            "mesh_storage_key_seal_bytes",
            "mesh_storage_key_unseal_bytes",
        ] {
            assert!(lowered.contains(runtime), "missing {runtime} in {lowered}");
        }
    }

    #[test]
    fn resource_structs_do_not_generate_exposing_trait_functions() {
        let mir = lower(
            "resource struct RatchetSecrets do\n\
               root_key :: SecretBytes\n\
             end",
        );

        assert!(
            mir.functions.iter().all(|function| {
                !function.name.ends_with("__RatchetSecrets")
                    && !function.name.contains("__RatchetSecrets__")
            }),
            "resource structs must not synthesize Debug/Eq/Ord/Hash exposure helpers: {:?}",
            mir.functions
                .iter()
                .map(|function| &function.name)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn direct_resource_calls_lower_borrow_consume_and_default_move_explicitly() {
        let mir = lower(
            "resource Token\n\
             fn inspect(token :: borrow Token) do nil end\n\
             fn consume_token(token :: consume Token) do nil end\n\
             fn take(token :: Token) do nil end\n\
             fn use_resources(a :: Token, b :: Token, c :: Token) do\n\
               inspect(a)\n\
               consume_token(b)\n\
               take(c)\n\
             end",
        );
        let function = mir
            .functions
            .iter()
            .find(|function| function.name == "use_resources")
            .expect("expected use_resources MIR function");

        fn count_ops(expression: &MirExpr) -> (usize, usize) {
            match expression {
                MirExpr::ResourceBorrow { value, .. } => {
                    let (borrows, moves) = count_ops(value);
                    (borrows + 1, moves)
                }
                MirExpr::ResourceMove { value, .. } => {
                    let (borrows, moves) = count_ops(value);
                    (borrows, moves + 1)
                }
                MirExpr::Call { func, args, .. } => {
                    args.iter().fold(count_ops(func), |sum, arg| {
                        let next = count_ops(arg);
                        (sum.0 + next.0, sum.1 + next.1)
                    })
                }
                MirExpr::Let { value, body, .. } => {
                    let left = count_ops(value);
                    let right = count_ops(body);
                    (left.0 + right.0, left.1 + right.1)
                }
                MirExpr::Block(expressions, _) => expressions.iter().fold((0, 0), |sum, item| {
                    let next = count_ops(item);
                    (sum.0 + next.0, sum.1 + next.1)
                }),
                _ => (0, 0),
            }
        }

        assert_eq!(
            count_ops(&function.body),
            (1, 2),
            "body: {:?}",
            function.body
        );
    }

    #[test]
    fn pipe_calls_apply_resource_borrow_modes() {
        let mir = lower(
            "resource Token\n\
             fn inspect(token :: borrow Token) do nil end\n\
             fn pipe_borrow(token :: Token) do token |> inspect end",
        );
        let function = mir
            .functions
            .iter()
            .find(|function| function.name == "pipe_borrow")
            .expect("pipe_borrow function");

        fn borrowed_inspect(expression: &MirExpr) -> bool {
            match expression {
                MirExpr::Call { func, args, .. } if matches!(func.as_ref(), MirExpr::Var(name, _) if name == "inspect") =>
                {
                    matches!(args.as_slice(), [MirExpr::ResourceBorrow { .. }])
                }
                MirExpr::Let { value, body, .. } => {
                    borrowed_inspect(value) || borrowed_inspect(body)
                }
                MirExpr::Block(expressions, _) => expressions.iter().any(borrowed_inspect),
                _ => false,
            }
        }

        assert!(
            borrowed_inspect(&function.body),
            "body: {:?}",
            function.body
        );
    }

    #[test]
    fn imported_borrow_call_keeps_the_resource_owned_for_destroy() {
        let module_parse =
            mesh_parser::parse("pub fn inspect(secret :: borrow SecretBytes) do nil end");
        let module_typeck = mesh_typeck::check(&module_parse);
        assert!(
            module_typeck.errors.is_empty(),
            "{:?}",
            module_typeck.errors
        );
        let exports = mesh_typeck::collect_exports(&module_parse, &module_typeck);
        let module = ModuleExports::new("Secrets".to_string(), &exports);

        let mut imports = ImportContext::empty();
        imports.module_exports.insert("Secrets".to_string(), module);
        let mir = lower_with_imports(
            "import Secrets\n\
             fn use_import(secret :: SecretBytes) do\n\
               Secrets.inspect(secret)\n\
               Secret.destroy(secret)\n\
             end",
            imports,
        );
        let function = mir
            .functions
            .iter()
            .find(|function| function.name == "use_import")
            .unwrap();

        fn ownership_ops(expression: &MirExpr) -> (usize, usize) {
            match expression {
                MirExpr::ResourceBorrow { value, .. } => {
                    let (borrows, destroys) = ownership_ops(value);
                    (borrows + 1, destroys)
                }
                MirExpr::ResourceDestroy { value, .. } => {
                    let (borrows, destroys) = ownership_ops(value);
                    (borrows, destroys + 1)
                }
                MirExpr::ResourceMove { value, .. } | MirExpr::ResourceDrop { value, .. } => {
                    ownership_ops(value)
                }
                MirExpr::Call { func, args, .. } => {
                    args.iter().fold(ownership_ops(func), |sum, argument| {
                        let next = ownership_ops(argument);
                        (sum.0 + next.0, sum.1 + next.1)
                    })
                }
                MirExpr::Let { value, body, .. } => {
                    let value = ownership_ops(value);
                    let body = ownership_ops(body);
                    (value.0 + body.0, value.1 + body.1)
                }
                MirExpr::Block(expressions, _) => {
                    expressions.iter().fold((0, 0), |sum, expression| {
                        let next = ownership_ops(expression);
                        (sum.0 + next.0, sum.1 + next.1)
                    })
                }
                _ => (0, 0),
            }
        }

        assert_eq!(ownership_ops(&function.body), (1, 1), "{:?}", function.body);
    }

    #[test]
    fn crypto_v2_calls_use_runtime_symbols_and_resource_modes() {
        let mir = lower(
            "fn hmac(key :: borrow SecretBytes, message :: Bytes) -> Result<SecretBytes, CryptoError> do\n\
               Crypto.hmac_sha256(key, message)\n\
             end\n\
             fn derive(password :: borrow SecretBytes, salt :: Bytes) -> Result<SecretBytes, CryptoError> do\n\
               Crypto.argon2id(password, salt, 32, 3, 1, 32)\n\
             end\n\
             fn make_aead(material :: SecretBytes) -> Result<AeadKey, CryptoError> do\n\
               Crypto.aead_key(material)\n\
             end\n\
             fn seal(key :: borrow AeadKey, nonce :: Bytes, aad :: Bytes, body :: Bytes) -> Result<Bytes, CryptoError> do\n\
               Crypto.aead_seal(key, nonce, aad, body)\n\
             end\n\
             fn encapsulate(key :: MlKemPublicKey) -> Result<(MlKemCiphertext, SecretBytes), CryptoError> do\n\
               Crypto.mlkem_encapsulate(key)\n\
             end\n\
             fn decapsulate(key :: borrow MlKemPrivateKey, ciphertext :: MlKemCiphertext) -> Result<SecretBytes, CryptoError> do\n\
               Crypto.mlkem_decapsulate(key, ciphertext)\n\
             end",
        );

        fn find_call<'a>(expression: &'a MirExpr, callee: &str) -> Option<&'a MirExpr> {
            match expression {
                MirExpr::Call { func, .. } if matches!(func.as_ref(), MirExpr::Var(name, _) if name == callee) => {
                    Some(expression)
                }
                MirExpr::Let { value, body, .. } => {
                    find_call(value, callee).or_else(|| find_call(body, callee))
                }
                MirExpr::Block(expressions, _) => {
                    expressions.iter().find_map(|item| find_call(item, callee))
                }
                _ => None,
            }
        }

        let hmac = find_call(
            &mir.functions
                .iter()
                .find(|function| function.name == "hmac")
                .unwrap()
                .body,
            "mesh_crypto_hmac_sha256",
        )
        .expect("hmac runtime call");
        assert!(matches!(
            hmac,
            MirExpr::Call { args, ty: MirType::SumType(name), .. }
                if name == "Result_SecretBytes_CryptoError"
                    && matches!(args.first(), Some(MirExpr::ResourceBorrow { .. }))
        ));

        let argon2id = find_call(
            &mir.functions
                .iter()
                .find(|function| function.name == "derive")
                .unwrap()
                .body,
            "mesh_crypto_argon2id",
        )
        .expect("Argon2id runtime call");
        assert!(matches!(
            argon2id,
            MirExpr::Call { args, ty: MirType::SumType(name), .. }
                if name == "Result_SecretBytes_CryptoError"
                    && args.len() == 6
                    && matches!(args.first(), Some(MirExpr::ResourceBorrow { .. }))
        ));

        let aead_key = find_call(
            &mir.functions
                .iter()
                .find(|function| function.name == "make_aead")
                .unwrap()
                .body,
            "mesh_crypto_aead_key",
        )
        .expect("aead key runtime call");
        assert!(matches!(
            aead_key,
            MirExpr::Call { args, ty: MirType::SumType(name), .. }
                if name == "Result_AeadKey_CryptoError"
                    && matches!(args.first(), Some(MirExpr::ResourceMove { .. }))
        ));

        let seal = find_call(
            &mir.functions
                .iter()
                .find(|function| function.name == "seal")
                .unwrap()
                .body,
            "mesh_crypto_aead_seal",
        )
        .expect("aead seal runtime call");
        assert!(
            matches!(
                seal,
                MirExpr::Call { args, ty: MirType::SumType(name), .. }
                    if name == "Result_Ptr_CryptoError"
                        && matches!(args.first(), Some(MirExpr::ResourceBorrow { .. }))
            ),
            "{seal:?}"
        );

        find_call(
            &mir.functions
                .iter()
                .find(|function| function.name == "encapsulate")
                .unwrap()
                .body,
            "mesh_crypto_mlkem_encapsulate",
        )
        .expect("ML-KEM encapsulation runtime call");

        let decapsulate = find_call(
            &mir.functions
                .iter()
                .find(|function| function.name == "decapsulate")
                .unwrap()
                .body,
            "mesh_crypto_mlkem_decapsulate",
        )
        .expect("ML-KEM decapsulation runtime call");
        assert!(matches!(
            decapsulate,
            MirExpr::Call { args, ty: MirType::SumType(name), .. }
                if name == "Result_SecretBytes_CryptoError"
                    && matches!(args.first(), Some(MirExpr::ResourceBorrow { .. }))
        ));
    }

    #[test]
    fn owned_resource_params_drop_on_normal_and_early_return_paths() {
        let mir = lower(
            "fn normal(secret :: SecretBytes) do nil end\n\
             fn early(secret :: SecretBytes, stop :: Bool) do\n\
               if stop do return nil else nil end\n\
             end",
        );

        let normal = mir
            .functions
            .iter()
            .find(|function| function.name == "normal")
            .unwrap();
        assert_eq!(
            drops_of(&normal.body, "secret"),
            1,
            "normal body: {:?}",
            normal.body
        );

        let early = mir
            .functions
            .iter()
            .find(|function| function.name == "early")
            .unwrap();
        assert_eq!(
            drops_of(&early.body, "secret"),
            2,
            "one cleanup is required on each reachable exit path: {:?}",
            early.body
        );
    }

    /// What a `case` arm binds and leaves unmoved is destroyed where the arm
    /// ends, and before a return out of it, like a `let` in a block.
    #[test]
    fn case_arm_resources_drop_where_the_arm_ends() {
        let mir = lower(
            "fn keep(result :: Result<SecretBytes, CryptoError>, stop :: Bool) do\n\
               case result do\n\
                 Ok(secret) -> if stop do return nil else nil end\n\
                 Err(_) as whole -> nil\n\
               end\n\
             end",
        );
        let keep = function_body(&mir, "keep");
        assert_eq!(drops_of(&keep, "secret"), 2, "{keep:?}");
        assert_eq!(drops_of(&keep, "whole"), 1, "{keep:?}");

        // So is what a parameter pattern binds.
        let mir = lower(
            "fn peek(secret :: borrow SecretBytes) -> Int = 1\n\
             fn keep((secret, n)) -> Int = peek(secret) + n",
        );
        let keep = function_body(&mir, "keep");
        assert_eq!(drops_of(&keep, "secret"), 1, "{keep:?}");

        // And what a `_` stands for, in an arm, a `let` and a clause.
        let mir = lower(
            "fn arm(result :: Result<SecretBytes, CryptoError>) -> Int do\n\
               case result do\n\
                 Ok(_) -> 1\n\
                 Err(_) -> 0\n\
               end\n\
             end\n\
             fn bind(pair :: (SecretBytes, Int)) -> Int do\n\
               let (_, n) = pair\n\
               n\n\
             end\n\
             fn clause((_, n) :: (SecretBytes, Int)) -> Int = n",
        );
        for name in ["arm", "bind", "clause"] {
            let body = function_body(&mir, name);
            let discards = body
                .descendants()
                .into_iter()
                .filter(|node| {
                    matches!(node, MirExpr::ResourceDrop { value, .. }
                        if matches!(value.as_ref(), MirExpr::Var(name, _) if name.starts_with("__discarded_")))
                })
                .count();
            assert_eq!(discards, 1, "{name}: {body:?}");
        }
    }

    fn function_body(mir: &MirModule, name: &str) -> MirExpr {
        mir.functions
            .iter()
            .find(|function| function.name == name)
            .unwrap()
            .body
            .clone()
    }

    /// A resource still owned where control leaves its scope another way
    /// than by returning is destroyed first: at a `break` or `continue` out
    /// of the loop body that holds it, and before a self tail call, which
    /// stays a tail call. The call used to end up in the scope's value, out
    /// of tail position: a deep recursion kept every level's secret alive.
    #[test]
    fn owned_resources_drop_before_break_continue_and_tail_calls() {
        let mir = lower(
            "fn scan(n :: Int) -> Int ! CryptoError do\n\
               for i in 0..n do\n\
                 let s = Secret.random(1) ?\n\
                 if i == 1 do\n\
                   break\n\
                 end\n\
                 if i == 2 do\n\
                   continue\n\
                 end\n\
                 Secret.destroy(s)\n\
               end\n\
               Ok(0)\n\
             end\n\
             fn spin(n :: Int) -> Int ! CryptoError do\n\
               let s = Secret.random(1) ?\n\
               if n == 0 do\n\
                 Secret.destroy(s)\n\
                 Ok(0)\n\
               else\n\
                 spin(n - 1)\n\
               end\n\
             end",
        );
        let scan = function_body(&mir, "scan");
        // One at each exit of the body: the break, the continue, and its
        // end, where a moved `s` is already null and dropping it does nothing.
        assert_eq!(drops_of(&scan, "s"), 3, "{scan:?}");
        let spin = function_body(&mir, "spin");
        assert!(
            spin.descendants()
                .iter()
                .any(|node| matches!(node, MirExpr::TailCall { .. })),
            "{spin:?}"
        );
        // Before the tail call, and at the end of the scope.
        assert_eq!(drops_of(&spin, "s"), 2, "{spin:?}");

        // Lent to the next call, `s` must outlive it: no tail call, and it
        // is dropped once, after the call returns.
        let mir = lower(
            "fn chain(n :: Int, prev :: borrow SecretBytes) -> Int ! CryptoError do\n\
               if n == 0 do\n\
                 Ok(0)\n\
               else\n\
                 let s = Secret.random(1) ?\n\
                 chain(n - 1, s)\n\
               end\n\
             end",
        );
        let chain = function_body(&mir, "chain");
        assert!(
            !chain
                .descendants()
                .iter()
                .any(|node| matches!(node, MirExpr::TailCall { .. })),
            "{chain:?}"
        );
        assert_eq!(drops_of(&chain, "s"), 1, "{chain:?}");
    }

    /// A function written as clauses owns the resources its clauses bind
    /// as one with a plain body owns its parameters: a clause that keeps
    /// one drops it as it ends, and a call no clause matches drops its
    /// resource arguments before it panics. Both leaked the resource.
    #[test]
    fn clause_functions_drop_the_resources_they_are_given() {
        let mir = lower(
            "fn open(secret :: SecretBytes, n :: Int) when n > 0 do\n\
               Secret.destroy(secret)\n\
             end\n\
             fn peek(secret :: SecretBytes, n :: Int) when n > 0 do\n\
               nil\n\
             end\n\
             fn look(secret :: borrow SecretBytes, n :: Int) when n > 0 do\n\
               nil\n\
             end",
        );
        let open = function_body(&mir, "open");
        // The clause's own `secret` as it ends (moved by then, so a no-op),
        // and the argument itself when no clause matches, before the panic.
        assert_eq!(drops_of(&open, "secret"), 1, "{open:?}");
        assert_eq!(drops_of(&open, "__param_0"), 1, "{open:?}");
        let peek = function_body(&mir, "peek");
        assert_eq!(drops_of(&peek, "secret"), 1, "{peek:?}");
        // A borrowed one is the caller's, matched or not.
        let look = function_body(&mir, "look");
        assert_eq!(drops_of(&look, "secret"), 0, "{look:?}");
        assert_eq!(drops_of(&look, "__param_0"), 0, "{look:?}");
    }

    /// A method, or an interface's default method, drops the resource
    /// parameters it owns as a function does; they leaked. Its `self` is
    /// borrowed from the caller, who drops it.
    #[test]
    fn methods_drop_owned_resource_parameters_but_not_self() {
        let mir = lower(
            "resource struct Session do\n  id :: Int\nend\n\
             interface Closer do\n  fn close(self) -> Int\n  fn wipe(self, key :: SecretBytes) -> Int do\n    1\n  end\nend\n\
             impl Closer for Session do\n  fn close(self) -> Int do\n    self.id\n  end\nend",
        );
        let close = function_body(&mir, "Closer__close__Session");
        assert_eq!(drops_of(&close, "self"), 0, "{close:?}");
        let wipe = function_body(&mir, "Closer__wipe__Session");
        assert_eq!(drops_of(&wipe, "key"), 1, "{wipe:?}");
        assert_eq!(drops_of(&wipe, "self"), 0, "{wipe:?}");
    }

    #[test]
    fn owned_resource_drops_when_try_returns_from_a_let_initializer() {
        let mir = lower(
            "fn try_init(secret :: SecretBytes) -> Int ! CryptoError do\n\
               let generated = Secret.random(32) ?\n\
               Secret.destroy(generated)\n\
               Ok(0)\n\
             end",
        );
        let function = mir
            .functions
            .iter()
            .find(|function| function.name == "try_init")
            .unwrap();

        assert_eq!(
            drops_of(&function.body, "secret"),
            2,
            "the still-owned parameter needs one drop on the ? error path and one on success: {:?}",
            function.body
        );
    }

    #[test]
    fn try_on_same_non_generic_error_does_not_call_from_itself() {
        let mir = lower(
            "type ProofError do\n\
               InvalidFixture\n\
             end\n\
             fn proof() -> Int ! ProofError do\n\
               let value = case true do\n\
                 true -> Ok(1)\n\
                 false -> Err(InvalidFixture)\n\
               end ?\n\
               Ok(value)\n\
             end",
        );
        let proof = mir
            .functions
            .iter()
            .find(|function| function.name == "proof")
            .expect("proof function");

        assert!(
            !format!("{:?}", proof.body).contains("From_ProofError__from__ProofError"),
            "same-error propagation must not synthesize a From call: {:?}",
            proof.body
        );
    }

    #[test]
    fn nested_resource_scopes_preserve_generic_result_return_type() {
        let mir = lower(
            "fn nested() -> Int ! CryptoError do\n\
               let first = Secret.random(1) ?\n\
               Secret.destroy(first)\n\
               let second = Secret.random(1) ?\n\
               Secret.destroy(second)\n\
               Ok(0)\n\
             end",
        );
        let nested = mir
            .functions
            .iter()
            .find(|function| function.name == "nested")
            .expect("nested function");

        assert_eq!(
            effective_return_type(&nested.body),
            nested.return_type,
            "resource scope wrappers and constructors must preserve the concrete Result type: {:?}",
            nested.body
        );
    }

    /// A closure's capture carries the shape of what its variable holds,
    /// read off a use of the variable inside the closure. A keyword key of
    /// the same name (`size(xs: 1)`) is no use of it: taking its missing
    /// type left the capture to the shape of its MIR type, which says
    /// nothing of a list's elements.
    #[test]
    fn closure_captures_are_shaped_by_a_typed_use() {
        let mir = lower(
            "fn size(m :: Map<String, Int>) -> Int do\n  Map.size(m)\nend\n\n\
             fn main() do\n  let xs = [\"a\"]\n  let f = fn () -> size(xs: 1) + List.length(xs) end\n  println(\"#{f()}\")\nend",
        );
        let main = function_body(&mir, "mesh_main");
        let captures = main
            .descendants()
            .into_iter()
            .find_map(|node| match node {
                MirExpr::MakeClosure { captures, .. } => Some(captures.clone()),
                _ => None,
            })
            .expect("main makes a closure");
        let xs = captures
            .iter()
            .find(|capture| var_refs(capture, "xs") == 1)
            .expect("the closure captures xs");
        assert!(
            matches!(xs, MirExpr::Shaped { shape: MsgShape::List(element), .. }
                if **element == MsgShape::String),
            "{captures:?}"
        );
    }

    /// A closure calls a top-level function by its name, as any function
    /// does; only the enclosing function's variables are captured. The
    /// global scope's function names were captured too, each taking an
    /// environment slot for a pointer the call never read.
    #[test]
    fn closures_capture_locals_but_not_top_level_functions() {
        let mir = lower(
            "fn double(n :: Int) -> Int do\n  n * 2\nend\n\n\
             fn main() do\n  let k = 3\n  let f = fn (x :: Int) -> double(x) + k end\n  println(\"#{f(1)}\")\nend",
        );
        let main = function_body(&mir, "mesh_main");
        let captures = main
            .descendants()
            .into_iter()
            .find_map(|node| match node {
                MirExpr::MakeClosure { captures, .. } => Some(captures.clone()),
                _ => None,
            })
            .expect("main makes a closure");
        assert_eq!(captures.len(), 1, "{captures:?}");
        assert_eq!(var_refs(&captures[0], "k"), 1, "{captures:?}");
    }

    /// Each function is generated once: a struct or sum type deriving Json
    /// had its `from_json` string wrapper made both before any item was
    /// lowered and again with its other Json functions, and code
    /// generation met the name twice.
    #[test]
    fn json_types_get_one_from_json_wrapper() {
        let mir = lower(
            "struct U do\n  a :: Int\nend deriving(Json)\n\n\
             type T do\n  A(Int)\nend deriving(Json)\n\n\
             fn main() do\n  nil\nend",
        );
        let mut seen = HashSet::new();
        let twice: Vec<&str> = mir
            .functions
            .iter()
            .map(|function| function.name.as_str())
            .filter(|name| !seen.insert(*name))
            .collect();
        assert!(twice.is_empty(), "generated twice: {twice:?}");
        assert!(seen.contains("__json_decode__U") && seen.contains("__json_decode__T"));
    }

    /// The checker counts a pid of resource messages as a resource, but the
    /// actor it names owns those messages: nothing drops the pid.
    #[test]
    fn pids_of_resource_messages_are_not_dropped() {
        let mir = lower(
            "resource struct Vault do\n  key :: SecretBytes\nend\n\n\
             fn keep(p :: Pid<SecretBytes>, v :: Pid<Vault>) -> Int do\n  1\nend",
        );
        let keep = function_body(&mir, "keep");
        assert_eq!(drops_of(&keep, "p"), 0, "{keep:?}");
        assert_eq!(drops_of(&keep, "v"), 0, "{keep:?}");
    }

    #[test]
    fn resource_struct_drop_plan_only_recurses_into_resource_fields() {
        let mir = lower(
            "resource struct KeyPair do\n\
               private_key :: SecretBytes\n\
               public_bytes :: Bytes\n\
             end\n\
             fn discard(pair :: KeyPair) do nil end",
        );
        let discard = mir
            .functions
            .iter()
            .find(|function| function.name == "discard")
            .unwrap();

        let destructor = first_destructor(&discard.body).expect("expected automatic drop");
        match destructor {
            MirResourceDestructor::Aggregate(fields) => {
                assert_eq!(fields.len(), 1, "drop plan: {destructor:?}");
                assert_eq!(fields[0].index, 0);
                assert!(matches!(
                    fields[0].destructor,
                    MirResourceDestructor::Opaque
                ));
            }
            other => panic!("expected aggregate resource destructor, got {other:?}"),
        }
    }

    #[test]
    fn resource_result_drop_plan_destroys_resource_ok_payload() {
        let mir = lower("fn discard(result :: Result<SecretBytes, CryptoError>) do nil end");
        let discard = mir
            .functions
            .iter()
            .find(|function| function.name == "discard")
            .unwrap();

        assert!(matches!(
            first_destructor(&discard.body),
            Some(MirResourceDestructor::SumVariants(variants))
                if matches!(variants.as_slice(), [variant]
                    if variant.tag == 0
                        && variant.field_types == [MirType::Ptr]
                        && matches!(variant.resource_fields.as_slice(), [field]
                            if field.index == 0
                                && matches!(field.destructor, MirResourceDestructor::Opaque)))
        ));
    }

    #[test]
    fn pg_connection_result_drop_keeps_boxed_sum_storage_and_handle_semantics() {
        let mir = lower(
            "fn discard(url :: String) do\n\
               let connection = Pg.connect(url)\n\
               nil\n\
             end",
        );
        let discard = mir
            .functions
            .iter()
            .find(|function| function.name == "discard")
            .unwrap();

        assert!(matches!(
            first_destructor(&discard.body),
            Some(MirResourceDestructor::SumVariants(variants))
                if matches!(variants.as_slice(), [variant]
                    if variant.tag == 0
                        && variant.field_types == [MirType::Ptr]
                        && matches!(variant.resource_fields.as_slice(), [field]
                            if field.index == 0
                                && field.ty == MirType::Int
                                && matches!(field.destructor, MirResourceDestructor::PgConnection)))
        ));
    }

    #[test]
    fn resource_result_drop_plan_destroys_resource_err_payload() {
        let mir = lower("fn discard(result :: Result<Bytes, SecretBytes>) do nil end");
        let discard = mir
            .functions
            .iter()
            .find(|function| function.name == "discard")
            .unwrap();

        assert!(matches!(
            first_destructor(&discard.body),
            Some(MirResourceDestructor::SumVariants(variants))
                if matches!(variants.as_slice(), [variant]
                    if variant.tag == 1
                        && variant.field_types == [MirType::Ptr]
                        && matches!(variant.resource_fields.as_slice(), [field]
                            if field.index == 0
                                && matches!(field.destructor, MirResourceDestructor::Opaque)))
        ));
    }

    #[test]
    fn resource_result_drop_plan_destroys_both_resource_variants() {
        let mir = lower("fn discard(result :: Result<SecretBytes, SecretBytes>) do nil end");
        let discard = mir
            .functions
            .iter()
            .find(|function| function.name == "discard")
            .unwrap();

        let MirResourceDestructor::SumVariants(variants) =
            first_destructor(&discard.body).expect("resource result drop")
        else {
            panic!(
                "expected variant-aware result destruction: {:?}",
                discard.body
            );
        };
        assert_eq!(
            variants
                .iter()
                .map(|variant| variant.tag)
                .collect::<Vec<_>>(),
            vec![0, 1]
        );
        assert!(variants.iter().all(|variant| {
            variant.field_types == [MirType::Ptr]
                && matches!(variant.resource_fields.as_slice(), [field]
                    if field.index == 0
                        && matches!(field.destructor, MirResourceDestructor::Opaque))
        }));
    }

    #[test]
    fn option_resource_drop_plan_destroys_some_but_not_none() {
        let mir = lower("fn discard(value :: Option<SecretBytes>) do nil end");
        let discard = mir
            .functions
            .iter()
            .find(|function| function.name == "discard")
            .unwrap();

        assert!(matches!(
            first_destructor(&discard.body),
            Some(MirResourceDestructor::SumVariants(variants))
                if matches!(variants.as_slice(), [variant]
                    if variant.tag == 0
                        && variant.field_types == [MirType::Ptr]
                        && matches!(variant.resource_fields.as_slice(), [field]
                            if field.index == 0
                                && matches!(field.destructor, MirResourceDestructor::Opaque)))
        ));
    }

    #[test]
    fn custom_sum_resource_drop_plan_uses_each_variant_field_layout() {
        let mir = lower(
            "type SecretChoice do\n\
               Empty\n\
               Public(Bytes)\n\
               Private(SecretBytes)\n\
               Pair(Int, SecretBytes)\n\
             end\n\
             fn discard(value :: SecretChoice) do nil end",
        );
        let discard = mir
            .functions
            .iter()
            .find(|function| function.name == "discard")
            .unwrap();

        let MirResourceDestructor::SumVariants(variants) =
            first_destructor(&discard.body).expect("custom sum resource drop")
        else {
            panic!(
                "expected a variant-aware sum destructor: {:?}",
                discard.body
            );
        };
        assert_eq!(
            variants
                .iter()
                .map(|variant| variant.tag)
                .collect::<Vec<_>>(),
            vec![2, 3]
        );
        assert_eq!(variants[0].field_types, vec![MirType::Ptr]);
        assert_eq!(variants[0].resource_fields[0].index, 0);
        assert_eq!(variants[1].field_types, vec![MirType::Int, MirType::Ptr]);
        assert_eq!(variants[1].resource_fields[0].index, 1);
    }

    #[test]
    fn builtin_crypto_struct_layouts_are_registered_without_source_declarations() {
        let mir = lower("");

        let fields = |name: &str| {
            mir.structs
                .iter()
                .find(|definition| definition.name == name)
                .map(|definition| definition.fields.clone())
        };
        assert_eq!(
            fields("X25519PublicKey"),
            Some(vec![("bytes".to_string(), MirType::Ptr)])
        );
        assert_eq!(
            fields("X25519KeyPair"),
            Some(vec![
                ("private_key".to_string(), MirType::Ptr),
                (
                    "public_key".to_string(),
                    MirType::Struct("X25519PublicKey".to_string()),
                ),
            ])
        );
        assert_eq!(
            fields("MlKemKeyPair"),
            Some(vec![
                ("private_key".to_string(), MirType::Ptr),
                (
                    "public_key".to_string(),
                    MirType::Struct("MlKemPublicKey".to_string()),
                ),
            ])
        );
        assert_eq!(
            fields("SigningKeyPair"),
            Some(vec![
                ("private_key".to_string(), MirType::Ptr),
                (
                    "public_key".to_string(),
                    MirType::Struct("SigningPublicKey".to_string()),
                ),
            ])
        );
    }

    #[test]
    fn moving_resource_field_records_sibling_cleanup_and_invalidates_parent() {
        let mir = lower(
            "resource struct DoubleSecret do\n\
               first :: SecretBytes\n\
               second :: SecretBytes\n\
             end\n\
             fn take(secret :: consume SecretBytes) do nil end\n\
             fn move_first(pair :: DoubleSecret) do take(pair.first) end",
        );
        let function = mir
            .functions
            .iter()
            .find(|function| function.name == "move_first")
            .unwrap();

        fn find_projection(expression: &MirExpr) -> Option<&MirResourceMoveSource> {
            match expression {
                MirExpr::ResourceMove { source, .. }
                    if matches!(source, MirResourceMoveSource::Projection { .. }) =>
                {
                    Some(source)
                }
                MirExpr::ResourceMove { value, .. }
                | MirExpr::ResourceBorrow { value, .. }
                | MirExpr::ResourceDrop { value, .. }
                | MirExpr::ResourceDestroy { value, .. } => find_projection(value),
                MirExpr::Call { func, args, .. } => {
                    find_projection(func).or_else(|| args.iter().find_map(find_projection))
                }
                MirExpr::Let { value, body, .. } => {
                    find_projection(value).or_else(|| find_projection(body))
                }
                MirExpr::Block(expressions, _) => expressions.iter().find_map(find_projection),
                _ => None,
            }
        }

        match find_projection(&function.body) {
            Some(MirResourceMoveSource::Projection {
                field_index,
                parent_destructor: MirResourceDestructor::Aggregate(fields),
                ..
            }) => {
                assert_eq!(*field_index, 0);
                assert_eq!(
                    fields.iter().map(|field| field.index).collect::<Vec<_>>(),
                    vec![0, 1]
                );
            }
            other => panic!("expected field projection resource move, got {other:?}"),
        }
    }

    #[test]
    fn resource_struct_update_records_the_replaced_field_destructor() {
        let mir = lower(
            "resource struct KeyState do\n\
               current :: SecretBytes\n\
               previous :: SecretBytes\n\
             end\n\
             fn replace(state :: KeyState, next :: SecretBytes) do\n\
               %{state | current: next}\n\
             end",
        );
        let function = mir
            .functions
            .iter()
            .find(|function| function.name == "replace")
            .unwrap();

        fn find_update(expression: &MirExpr) -> Option<&[MirResourceField]> {
            match expression {
                MirExpr::StructUpdate {
                    resource_overrides, ..
                } => Some(resource_overrides),
                MirExpr::Let { value, body, .. } => {
                    find_update(value).or_else(|| find_update(body))
                }
                MirExpr::Block(expressions, _) => expressions.iter().find_map(find_update),
                _ => None,
            }
        }

        let resource_overrides = find_update(&function.body).expect("expected struct update");
        assert_eq!(resource_overrides.len(), 1);
        assert_eq!(resource_overrides[0].index, 0);
        assert!(matches!(
            resource_overrides[0].destructor,
            MirResourceDestructor::Opaque
        ));
    }

    #[test]
    fn moving_nested_resource_field_keeps_one_rooted_projection() {
        let mir = lower(
            "resource struct InnerSecrets do\n\
               selected :: SecretBytes\n\
               inner_sibling :: SecretBytes\n\
             end\n\
             resource struct OuterSecrets do\n\
               inner :: InnerSecrets\n\
               outer_sibling :: SecretBytes\n\
             end\n\
             fn take(secret :: consume SecretBytes) do nil end\n\
             fn move_nested(outer :: OuterSecrets) do take(outer.inner.selected) end",
        );
        let function = mir
            .functions
            .iter()
            .find(|function| function.name == "move_nested")
            .unwrap();

        fn find_take_argument(expression: &MirExpr) -> Option<&MirExpr> {
            match expression {
                MirExpr::Call { func, args, .. } if matches!(func.as_ref(), MirExpr::Var(name, _) if name == "take") => {
                    args.first()
                }
                MirExpr::Let { value, body, .. } => {
                    find_take_argument(value).or_else(|| find_take_argument(body))
                }
                MirExpr::Block(expressions, _) => expressions.iter().find_map(find_take_argument),
                _ => None,
            }
        }

        let argument = find_take_argument(&function.body).expect("expected take call");
        match argument {
            MirExpr::ResourceMove {
                value,
                source:
                    MirResourceMoveSource::Projection {
                        parent_destructor: MirResourceDestructor::Aggregate(root_fields),
                        nested_field_indices,
                        ..
                    },
                ..
            } => {
                assert!(matches!(
                    value.as_ref(),
                    MirExpr::FieldAccess { object, field, .. }
                        if field == "selected"
                            && matches!(object.as_ref(), MirExpr::FieldAccess { object, field, .. }
                                if field == "inner"
                                    && matches!(object.as_ref(), MirExpr::Var(name, _) if name == "outer"))
                ));
                assert_eq!(root_fields.len(), 2, "root destructor: {root_fields:?}");
                assert_eq!(nested_field_indices, &[0]);
                assert!(matches!(
                    &root_fields[0].destructor,
                    MirResourceDestructor::Aggregate(inner_fields) if inner_fields.len() == 2
                ));
            }
            other => panic!("expected one rooted nested projection move, got {other:?}"),
        }
    }

    #[test]
    fn explicit_secret_destroy_lowers_to_resource_destroy_not_a_raw_call() {
        let mir = lower("fn destroy_now(secret :: SecretBytes) do Secret.destroy(secret) end");
        let function = mir
            .functions
            .iter()
            .find(|function| function.name == "destroy_now")
            .unwrap();

        assert!(
            contains(&function.body, |node| matches!(
                node,
                MirExpr::ResourceDestroy { .. }
            )),
            "body: {:?}",
            function.body
        );
    }

    #[test]
    fn secret_concat_lowers_to_the_runtime_and_moves_both_inputs() {
        let mir = lower(
            "fn join(first :: SecretBytes, second :: SecretBytes) do\n  Secret.concat(first, second)\nend",
        );
        let function = mir
            .functions
            .iter()
            .find(|function| function.name == "join")
            .expect("join function");
        assert!(calls(&function.body, "mesh_secret_concat"));
    }

    #[test]
    fn lower_pipe_desugars_to_call() {
        // `x |> f` should desugar to `f(x)`
        let mir = lower(
            "fn double(x :: Int) -> Int do x * 2 end\n\
             fn main() do 5 |> double end",
        );
        let main = mir.functions.iter().find(|f| f.name == "mesh_main");
        assert!(main.is_some(), "Expected 'mesh_main' function in MIR");
        let main = main.unwrap();

        // Body should be a Call with func=double, args=[5]
        match &main.body {
            MirExpr::Call { func, args, .. } => {
                assert!(matches!(func.as_ref(), MirExpr::Var(name, _) if name == "double"));
                assert_eq!(args.len(), 1);
                assert!(matches!(&args[0], MirExpr::IntLit(5, _)));
            }
            other => panic!("Expected Call, got {:?}", other),
        }
    }

    #[test]
    fn lower_clustered_route_wrapper_rewrites_direct_and_pipe_forms_to_one_bare_shim() {
        let mut import_ctx = ImportContext::empty();
        import_ctx.current_module = Some("App.Router".to_string());
        let mir = lower_with_imports(
            r#"
pub fn handle_local(req :: Request) -> Response do
  HTTP.response(200, "ok")
end

fn build() do
  let router = HTTP.router()
  let router = HTTP.on_get(router, "/one", HTTP.clustered(handle_local))
  router |> HTTP.on_get("/two", HTTP.clustered(handle_local))
end
"#,
            import_ctx,
        );

        let shim_name = "__declared_route_app_router_handle_local";
        let shim_fns = mir
            .functions
            .iter()
            .filter(|func| func.name == shim_name)
            .collect::<Vec<_>>();
        assert_eq!(
            shim_fns.len(),
            1,
            "expected one deduped clustered route shim, got {:?}",
            mir.functions
                .iter()
                .map(|func| &func.name)
                .collect::<Vec<_>>()
        );

        let shim = shim_fns[0];
        assert_eq!(shim.params, vec![("__request".to_string(), MirType::Ptr)]);
        assert_eq!(shim.return_type, MirType::Ptr);
        assert!(
            has_call_to(&shim.body, "handle_local"),
            "expected shim body to call the real handler, got {:?}",
            shim.body
        );

        let build = mir
            .functions
            .iter()
            .find(|func| func.name == "build")
            .expect("expected build function to lower");
        assert!(
            has_call_to(&build.body, "mesh_http_route_get"),
            "expected lowered route registration call, got {:?}",
            build.body
        );
        assert!(
            !has_call_to(&build.body, "http_clustered"),
            "clustered route wrappers must not survive as runtime calls: {:?}",
            build.body
        );
        assert_eq!(
            var_refs(&build.body, shim_name),
            2,
            "expected both direct and pipe routes to reference the same shim: {:?}",
            build.body
        );
    }

    #[test]
    fn lower_clustered_route_wrapper_uses_imported_runtime_identity_for_shim_name() {
        let mut import_ctx = ImportContext::empty();
        import_ctx.current_module = Some("App.Router".to_string());
        import_ctx.module_exports.insert(
            "Todos".to_string(),
            route_module_exports("Api.Todos", &["handle_list_todos"]),
        );
        let mir = lower_with_imports(
            r#"
from Api.Todos import handle_list_todos

fn build() do
  HTTP.router() |> HTTP.on_get("/todos", HTTP.clustered(handle_list_todos))
end
"#,
            import_ctx,
        );

        let shim = mir
            .functions
            .iter()
            .find(|func| func.name == "__declared_route_api_todos_handle_list_todos")
            .expect("expected imported route shim to preserve defining-module runtime identity");
        assert_eq!(shim.params, vec![("__request".to_string(), MirType::Ptr)]);
        assert_eq!(shim.return_type, MirType::Ptr);
        assert!(
            has_call_to(&shim.body, "handle_list_todos"),
            "expected imported route shim to call the lowered handler symbol, got {:?}",
            shim.body
        );

        let build = mir
            .functions
            .iter()
            .find(|func| func.name == "build")
            .expect("expected build function to lower");
        assert!(
            has_var_ref(&build.body, "__declared_route_api_todos_handle_list_todos"),
            "expected route registration to use imported shim, got {:?}",
            build.body
        );
        assert!(
            !has_call_to(&build.body, "http_clustered"),
            "clustered route wrappers must lower away from runtime calls: {:?}",
            build.body
        );
    }

    #[test]
    fn lower_string_interpolation_desugars_to_concat() {
        let source = r#"
fn main() do
  let name = "world"
  "hello ${name}"
end
"#;
        let mir = lower(source);
        let main = mir.functions.iter().find(|f| f.name == "mesh_main");
        assert!(main.is_some());
        let main = main.unwrap();

        // The body should contain a concat call somewhere.

        assert!(
            calls(&main.body, "mesh_string_concat"),
            "Expected mesh_string_concat call in interpolated string body: {:?}",
            main.body
        );
    }

    #[test]
    fn lower_closure_produces_lifted_function() {
        let source = r#"
fn main() do
  let y = 10
  let inc = fn(x :: Int) -> x + y end
  inc
end
"#;
        let mir = lower(source);

        // Should have a lifted closure function
        let closure_fn = mir
            .functions
            .iter()
            .find(|f| f.name.starts_with("__closure_"));
        assert!(
            closure_fn.is_some(),
            "Expected lifted closure function, got functions: {:?}",
            mir.functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
        let closure_fn = closure_fn.unwrap();
        assert!(closure_fn.is_closure_fn);
        // First param should be __env
        assert_eq!(closure_fn.params[0].0, "__env");
    }

    /// A closure captures the outer variables it uses, not names it binds
    /// itself: a `let`, a pattern or a loop variable that shadows an outer
    /// one. Capturing a shadowed resource made lowering refuse the closure.
    #[test]
    fn closures_capture_only_what_they_do_not_bind() {
        let captures_of = |source: &str| -> Vec<String> {
            let mir = lower(source);
            mir.functions
                .iter()
                .flat_map(|function| function.body.descendants())
                .find_map(|node| match node {
                    MirExpr::MakeClosure { captures, .. } => Some(
                        // Each capture, perhaps wrapped with its shape.
                        captures
                            .iter()
                            .flat_map(MirExpr::descendants)
                            .filter_map(|capture| match capture {
                                MirExpr::Var(name, _) => Some(name.clone()),
                                _ => None,
                            })
                            .collect(),
                    ),
                    _ => None,
                })
                .unwrap()
        };
        assert_eq!(
            captures_of(
                "fn main() do\n  let y = 10\n  let z = 5\n  let f = fn(x :: Int) do\n    let z = x + y\n    case z do\n      w -> w + z\n    end\n  end\n  f(z)\nend\n"
            ),
            ["y"]
        );
        assert_eq!(
            captures_of(
                "fn main() do\n  let v = 1\n  let f = fn() do\n    for v in [1, 2] do\n      v\n    end\n    0\n  end\n  f() + v\nend\n"
            ),
            Vec::<String>::new()
        );

        let parse = mesh_parser::parse(
            "fn make_closure(secret :: SecretBytes) do\n\
               fn () do\n\
                 let secret = 1\n\
                 secret\n\
               end\n\
             end",
        );
        let typeck = mesh_typeck::check(&parse);
        lower_to_mir(&parse, &typeck, "", &HashSet::new(), &HashMap::new())
            .expect("a closure's own `secret` is not the outer resource");
    }

    #[test]
    fn lowering_fails_closed_for_resource_closure_capture() {
        let parse = mesh_parser::parse(
            "fn make_closure(secret :: SecretBytes) do\n\
               fn () -> Secret.destroy(secret) end\n\
             end",
        );
        let typeck = mesh_typeck::check(&parse);
        let error = lower_to_mir(&parse, &typeck, "", &HashSet::new(), &HashMap::new())
            .expect_err("resource capture must never reach closure conversion");

        assert!(error.contains("cannot be captured by a closure"), "{error}");
    }

    #[test]
    fn lower_main_sets_entry_function() {
        let mir = lower("fn main() do 0 end");
        assert_eq!(mir.entry_function, Some("mesh_main".to_string()));
    }

    #[test]
    fn lower_if_expr() {
        let mir = lower("fn test(x :: Bool) -> Int do if x do 1 else 2 end end");
        let func = mir.functions.iter().find(|f| f.name == "test");
        assert!(func.is_some());
        assert!(matches!(func.unwrap().body, MirExpr::If { .. }));
    }

    #[test]
    fn lower_self_expr() {
        let source = r#"
actor counter(n :: Int) do
  receive do
    _ -> counter(n)
  end
end

fn main() do
  let pid = spawn(counter, 0)
  0
end
"#;
        let mir = lower(source);
        // The actor should produce a function named "counter"
        let actor_fn = mir.functions.iter().find(|f| f.name == "counter");
        assert!(
            actor_fn.is_some(),
            "Expected 'counter' actor function in MIR, got: {:?}",
            mir.functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn lower_spawn_produces_actor_spawn() {
        let source = r#"
actor counter(n :: Int) do
  receive do
    _ -> counter(n)
  end
end

fn main() do
  let pid = spawn(counter, 0)
  0
end
"#;
        let mir = lower(source);
        let main = mir.functions.iter().find(|f| f.name == "mesh_main");
        assert!(main.is_some());
        let main = main.unwrap();

        // Check body has ActorSpawn somewhere
        assert!(
            contains(&main.body, |node| matches!(
                node,
                MirExpr::ActorSpawn { .. }
            )),
            "Expected ActorSpawn in main body: {:?}",
            main.body
        );
    }

    #[test]
    fn lower_pid_type_resolves() {
        let source = r#"
actor echo() do
  receive do
    _ -> echo()
  end
end

fn main() do
  let pid = spawn(echo)
  0
end
"#;
        let mir = lower(source);
        let main = mir.functions.iter().find(|f| f.name == "mesh_main");
        assert!(main.is_some());
    }

    #[test]
    fn lower_case_expr() {
        let source = r#"
fn test(x :: Int) -> Int do
  case x do
    0 -> 1
    _ -> 2
  end
end
"#;
        let mir = lower(source);
        let func = mir.functions.iter().find(|f| f.name == "test");
        assert!(func.is_some());
        let func = func.unwrap();
        assert!(
            matches!(func.body, MirExpr::Match { .. }),
            "Expected Match, got {:?}",
            func.body
        );
    }

    #[test]
    fn lower_service_def_generates_functions() {
        let mir = lower(
            r#"
service Counter do
  fn init(initial :: Int) -> Int do
    initial
  end

  call GetCount() :: Int do |count|
    (count, count)
  end

  cast Reset() do |_count|
    0
  end
end
"#,
        );

        let fn_names: Vec<&str> = mir.functions.iter().map(|f| f.name.as_str()).collect();

        // Should have generated init, loop, start, call helper, cast helper, and handler functions.
        assert!(
            fn_names
                .iter()
                .any(|n| n.contains("__service_counter_init")),
            "Missing init function. Functions: {:?}",
            fn_names
        );
        assert!(
            fn_names
                .iter()
                .any(|n| n.contains("__service_counter_loop")),
            "Missing loop function. Functions: {:?}",
            fn_names
        );
        assert!(
            fn_names
                .iter()
                .any(|n| n.contains("__service_counter_start")),
            "Missing start function. Functions: {:?}",
            fn_names
        );
        assert!(
            fn_names
                .iter()
                .any(|n| n.contains("__service_counter_call_get_count")),
            "Missing call helper function. Functions: {:?}",
            fn_names
        );
        assert!(
            fn_names
                .iter()
                .any(|n| n.contains("__service_counter_cast_reset")),
            "Missing cast helper function. Functions: {:?}",
            fn_names
        );
        assert!(
            fn_names
                .iter()
                .any(|n| n.contains("__service_counter_handle_call_get_count")),
            "Missing call handler function. Functions: {:?}",
            fn_names
        );
        assert!(
            fn_names
                .iter()
                .any(|n| n.contains("__service_counter_handle_cast_reset")),
            "Missing cast handler function. Functions: {:?}",
            fn_names
        );
    }

    #[test]
    fn lower_service_dispatch_table_populated() {
        let mir = lower(
            r#"
service Counter do
  fn init(initial :: Int) -> Int do
    initial
  end

  call GetCount() :: Int do |count|
    (count, count)
  end

  cast Reset() do |_count|
    0
  end
end
"#,
        );

        // Should have a service_dispatch entry for the loop.
        assert!(
            !mir.service_dispatch.is_empty(),
            "service_dispatch should not be empty"
        );
        let loop_key = mir
            .service_dispatch
            .keys()
            .find(|k| k.contains("counter_loop"))
            .expect("Missing counter_loop dispatch entry");
        let (calls, casts) = &mir.service_dispatch[loop_key];
        assert_eq!(calls.len(), 1, "Should have 1 call handler");
        assert_eq!(casts.len(), 1, "Should have 1 cast handler");
        assert_eq!(calls[0].0, 0, "Call handler tag should be 0");
        assert_eq!(casts[0].0, 1, "Cast handler tag should be 1");
    }

    #[test]
    fn lower_service_field_access_resolves() {
        let mir = lower(
            r#"
service Counter do
  fn init(initial :: Int) -> Int do
    initial
  end

  call GetCount() :: Int do |count|
    (count, count)
  end
end

fn main() do
  let pid = Counter.start(0)
  let count = Counter.get_count(pid)
  println(int_to_string(count))
end
"#,
        );

        let main_fn = mir.functions.iter().find(|f| f.name == "mesh_main");
        assert!(
            main_fn.is_some(),
            "Missing mesh_main function. Functions: {:?}",
            mir.functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn impl_method_produces_mangled_mir_function() {
        let source = r#"
interface Greetable do
  fn greet(self) -> String
end

struct Point do
  x :: Int
end

impl Greetable for Point do
  fn greet(self) -> String do
    "hello"
  end
end
"#;
        let mir = lower(source);

        let fn_names: Vec<&str> = mir.functions.iter().map(|f| f.name.as_str()).collect();

        // Assert that a MirFunction with the mangled name exists.
        let mangled_fn = mir
            .functions
            .iter()
            .find(|f| f.name == "Greetable__greet__Point");
        assert!(
            mangled_fn.is_some(),
            "Expected MirFunction named 'Greetable__greet__Point'. Found: {:?}",
            fn_names
        );

        let mangled_fn = mangled_fn.unwrap();

        // Assert the first parameter is named "self" with type MirType::Struct("Point").
        assert!(
            !mangled_fn.params.is_empty(),
            "Expected at least one parameter (self)"
        );
        assert_eq!(
            mangled_fn.params[0].0, "self",
            "First param should be named 'self'"
        );
        assert_eq!(
            mangled_fn.params[0].1,
            MirType::Struct("Point".to_string()),
            "First param type should be MirType::Struct(\"Point\")"
        );

        // Assert the return type is String.
        assert_eq!(
            mangled_fn.return_type,
            MirType::String,
            "Return type should be String"
        );
    }

    #[test]
    fn call_site_rewrites_to_mangled_name() {
        let source = r#"
interface Greetable do
  fn greet(self) -> String
end

struct Point do
  x :: Int
end

impl Greetable for Point do
  fn greet(self) -> String do
    "hello"
  end
end

fn main() do
  let p = Point { x: 1 }
  greet(p)
end
"#;
        let mir = lower(source);

        // The main function body should contain a Call to "Greetable__greet__Point".
        let main_fn = mir.functions.iter().find(|f| f.name == "mesh_main");
        assert!(main_fn.is_some(), "Expected mesh_main function");
        let main_fn = main_fn.unwrap();

        assert!(
            calls(&main_fn.body, "Greetable__greet__Point"),
            "Expected call to Greetable__greet__Point in main body, got: {:?}",
            main_fn.body
        );
    }

    #[test]
    fn binop_on_user_type_emits_trait_call() {
        let source = r#"
interface Add do
  fn add(self, other) -> Int
end

struct Vec2 do
  x :: Int
end

impl Add for Vec2 do
  fn add(self, other) -> Int do
    0
  end
end

fn main() do
  let a = Vec2 { x: 1 }
  let b = Vec2 { x: 2 }
  a + b
end
"#;
        let mir = lower(source);

        let main_fn = mir.functions.iter().find(|f| f.name == "mesh_main");
        assert!(main_fn.is_some(), "Expected mesh_main function");
        let main_fn = main_fn.unwrap();

        // a + b with impl Add for Vec2 should become Call to Add__add__Vec2.
        assert!(
            calls(&main_fn.body, "Add__add__Vec2"),
            "Expected call to Add__add__Vec2 in main body, got: {:?}",
            main_fn.body
        );
    }

    #[test]
    fn primitive_binop_unchanged() {
        // Regression test: Int + Int should still produce BinOp, not a trait call.
        let source = r#"
fn main() do
  let a = 1
  let b = 2
  a + b
end
"#;
        let mir = lower(source);

        let main_fn = mir.functions.iter().find(|f| f.name == "mesh_main");
        assert!(main_fn.is_some());
        let main_fn = main_fn.unwrap();

        assert!(
            contains(&main_fn.body, |node| matches!(
                node,
                MirExpr::BinOp { op: BinOp::Add, .. }
            )),
            "Expected BinOp::Add for Int + Int, got: {:?}",
            main_fn.body
        );
    }

    // ── End-to-end trait codegen integration tests (19-04) ────────────

    /// Recursive helper to find a Call to a specific function name anywhere in a MirExpr tree.

    #[test]
    fn test_secure_store_builtin_lowers_to_runtime_symbol() {
        let mir = lower_with_imports(
            "fn main() -> Bool do\n  Test.install_in_memory_secure_store()\nend\n",
            ImportContext {
                test_builtins: true,
                ..ImportContext::default()
            },
        );
        let main = mir
            .functions
            .iter()
            .find(|function| function.name == "mesh_main")
            .expect("expected main function");
        assert!(calls(
            &main.body,
            "mesh_test_install_in_memory_secure_store"
        ));
    }

    #[test]
    fn test_push_token_builtin_lowers_to_runtime_symbol() {
        let mir = lower_with_imports(
            "fn main() -> Bool do\n  Test.set_push_token(Bytes.from_utf8(\"expo/raw/v1\"), Bytes.from_utf8(\"token\"))\nend\n",
            ImportContext {
                test_builtins: true,
                ..ImportContext::default()
            },
        );
        let main = mir
            .functions
            .iter()
            .find(|function| function.name == "mesh_main")
            .expect("expected main function");
        assert!(
            calls(&main.body, "mesh_test_set_push_token"),
            "{:?}",
            main.body
        );
    }

    /// Success Criterion 1: A Mesh program with interface, impl, struct, and trait
    /// method call compiles through MIR lowering and produces correct mangled call.
    #[test]
    fn e2e_trait_method_call_compiles() {
        let source = r#"
interface Greetable do
  fn greet(self) -> String
end

struct Greeter do
  name :: String
end

impl Greetable for Greeter do
  fn greet(self) -> String do
    "hello"
  end
end

fn main() do
  let g = Greeter { name: "world" }
  let result = greet(g)
  println(result)
end
"#;
        let mir = lower(source);

        // 1. MirProgram contains a function named Greetable__greet__Greeter
        let mangled = mir
            .functions
            .iter()
            .find(|f| f.name == "Greetable__greet__Greeter");
        assert!(
            mangled.is_some(),
            "Expected MirFunction 'Greetable__greet__Greeter'. Found: {:?}",
            mir.functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );

        // 2. Main body contains a Call referencing the mangled name
        let main_fn = mir
            .functions
            .iter()
            .find(|f| f.name == "mesh_main")
            .expect("Expected mesh_main function");
        assert!(
            calls(&main_fn.body, "Greetable__greet__Greeter"),
            "Expected call to Greetable__greet__Greeter in main body, got: {:?}",
            main_fn.body
        );

        // 3. No function named bare "greet" exists (only the mangled version)
        let bare_greet = mir.functions.iter().find(|f| f.name == "greet");
        assert!(
            bare_greet.is_none(),
            "Bare 'greet' function should NOT exist in MIR -- only the mangled version"
        );
    }

    /// Success Criterion 2: Trait method calls resolve to mangled names visible in MIR
    /// using the Trait__Method__Type pattern with double-underscore separators.
    #[test]
    fn e2e_mangled_names_in_mir() {
        let source = r#"
interface Describable do
  fn describe(self) -> String
end

struct Widget do
  label :: String
end

impl Describable for Widget do
  fn describe(self) -> String do
    "widget"
  end
end
"#;
        let mir = lower(source);

        let mangled = mir
            .functions
            .iter()
            .find(|f| f.name == "Describable__describe__Widget")
            .expect("Expected mangled function Describable__describe__Widget");

        // Verify name uses exactly 2 double-underscore separators: Trait__Method__Type
        let dunder_count = mangled.name.matches("__").count();
        assert_eq!(
            dunder_count, 2,
            "Mangled name should have exactly 2 '__' separators, got {} in '{}'",
            dunder_count, mangled.name
        );

        // Verify the three parts
        let parts: Vec<&str> = mangled.name.split("__").collect();
        assert_eq!(parts.len(), 3, "Expected 3 parts: [Trait, Method, Type]");
        assert_eq!(parts[0], "Describable");
        assert_eq!(parts[1], "describe");
        assert_eq!(parts[2], "Widget");
    }

    /// Success Criterion 3: self parameter in impl methods receives the concrete struct type.
    #[test]
    fn e2e_self_param_has_concrete_type() {
        let source = r#"
interface Greetable do
  fn greet(self) -> String
end

struct Greeter do
  name :: String
end

impl Greetable for Greeter do
  fn greet(self) -> String do
    "hello"
  end
end
"#;
        let mir = lower(source);

        let mangled = mir
            .functions
            .iter()
            .find(|f| f.name == "Greetable__greet__Greeter")
            .expect("Expected Greetable__greet__Greeter function");

        // First param must be named "self"
        assert!(
            !mangled.params.is_empty(),
            "Expected at least one parameter (self)"
        );
        assert_eq!(
            mangled.params[0].0, "self",
            "First param should be named 'self'"
        );

        // Type must be the concrete struct, NOT Unit, NOT Ptr, NOT Struct("self")
        assert_eq!(
            mangled.params[0].1,
            MirType::Struct("Greeter".to_string()),
            "self param type should be MirType::Struct(\"Greeter\")"
        );
        assert_ne!(
            mangled.params[0].1,
            MirType::Unit,
            "self param type must NOT be Unit"
        );
    }

    /// Success Criterion 1+: Multiple traits with different methods for the same type
    /// all produce correctly mangled and callable functions.
    #[test]
    fn e2e_multiple_traits_different_types() {
        let source = r#"
struct Dog do name :: String end
struct Cat do name :: String end

interface Speakable do
  fn speak(self) -> String
end

impl Speakable for Dog do
  fn speak(self) -> String do "woof" end
end

impl Speakable for Cat do
  fn speak(self) -> String do "meow" end
end

fn main() do
  let d = Dog { name: "Rex" }
  let c = Cat { name: "Whiskers" }
  println(speak(d))
  println(speak(c))
end
"#;
        let mir = lower(source);
        let fn_names: Vec<&str> = mir.functions.iter().map(|f| f.name.as_str()).collect();

        // Both mangled functions must exist
        assert!(
            fn_names.contains(&"Speakable__speak__Dog"),
            "Expected Speakable__speak__Dog. Found: {:?}",
            fn_names
        );
        assert!(
            fn_names.contains(&"Speakable__speak__Cat"),
            "Expected Speakable__speak__Cat. Found: {:?}",
            fn_names
        );

        // Main body has calls to both mangled names (not bare 'speak')
        let main_fn = mir
            .functions
            .iter()
            .find(|f| f.name == "mesh_main")
            .expect("Expected mesh_main function");
        assert!(
            calls(&main_fn.body, "Speakable__speak__Dog"),
            "Expected call to Speakable__speak__Dog in main body"
        );
        assert!(
            calls(&main_fn.body, "Speakable__speak__Cat"),
            "Expected call to Speakable__speak__Cat in main body"
        );
    }

    /// Success Criterion 4: Where-clause constrained functions reject calls with
    /// unsatisfied bounds at compile time (handled by typeck, not MIR lowerer).
    #[test]
    fn e2e_where_clause_enforcement() {
        // This source should FAIL typeck: Int does not implement Displayable.
        let source = r#"
interface Displayable do
  fn display(self) -> String
end

fn show<T>(x :: T) -> String where T: Displayable do
  display(x)
end

fn main() do
  show(42)
end
"#;
        let parse = mesh_parser::parse(source);
        let typeck = mesh_typeck::check(&parse);

        // Typeck should report TraitNotSatisfied error for Int not implementing Displayable.
        let has_trait_error = typeck
            .errors
            .iter()
            .any(|e| matches!(e, mesh_typeck::error::TypeError::TraitNotSatisfied { .. }));
        assert!(
            has_trait_error,
            "Expected TraitNotSatisfied error from typeck when calling show(42) without \
             Displayable impl for Int. Errors: {:?}",
            typeck.errors
        );

        // MIR lowering still succeeds (it's error-tolerant), confirming CODEGEN-04
        // is handled by typeck, not the lowerer.
        let empty_pub_fns = HashSet::new();
        let mir = lower_to_mir(&parse, &typeck, "", &empty_pub_fns, &HashMap::new());
        assert!(
            mir.is_ok(),
            "MIR lowering should succeed even with typeck errors (error recovery)"
        );
    }

    /// TSND-01: Where-clause constraints propagate through direct let aliases.
    /// `let f = show; f(42)` must produce TraitNotSatisfied.
    #[test]
    fn e2e_where_clause_alias_propagation() {
        let source = r#"
interface Displayable do
  fn display(self) -> String
end

fn show<T>(x :: T) -> String where T: Displayable do
  display(x)
end

fn main() do
  let f = show
  f(42)
end
"#;
        let parse = mesh_parser::parse(source);
        let typeck = mesh_typeck::check(&parse);

        let has_trait_error = typeck
            .errors
            .iter()
            .any(|e| matches!(e, mesh_typeck::error::TypeError::TraitNotSatisfied { .. }));
        assert!(
            has_trait_error,
            "Expected TraitNotSatisfied when calling aliased constrained function f(42). Errors: {:?}",
            typeck.errors
        );
    }

    /// TSND-01: Where-clause constraints propagate through chain aliases.
    /// `let f = show; let g = f; g(42)` must produce TraitNotSatisfied.
    #[test]
    fn e2e_where_clause_chain_alias() {
        let source = r#"
interface Displayable do
  fn display(self) -> String
end

fn show<T>(x :: T) -> String where T: Displayable do
  display(x)
end

fn main() do
  let f = show
  let g = f
  g(42)
end
"#;
        let parse = mesh_parser::parse(source);
        let typeck = mesh_typeck::check(&parse);

        let has_trait_error = typeck
            .errors
            .iter()
            .any(|e| matches!(e, mesh_typeck::error::TypeError::TraitNotSatisfied { .. }));
        assert!(
            has_trait_error,
            "Expected TraitNotSatisfied when calling chain-aliased constrained function g(42). Errors: {:?}",
            typeck.errors
        );
    }

    /// TSND-01: Where-clause constraints work with user-defined traits through aliases,
    /// and do NOT produce false positives for conforming types.
    #[test]
    fn e2e_where_clause_alias_user_trait() {
        // Part A: Should error -- Int does not implement Greetable
        let source_bad = r#"
interface Greetable do
  fn greet(self) -> String
end

fn say_hello<T>(x :: T) -> String where T: Greetable do
  greet(x)
end

fn main() do
  let f = say_hello
  f(42)
end
"#;
        let parse = mesh_parser::parse(source_bad);
        let typeck = mesh_typeck::check(&parse);

        let has_trait_error = typeck
            .errors
            .iter()
            .any(|e| matches!(e, mesh_typeck::error::TypeError::TraitNotSatisfied { .. }));
        assert!(
            has_trait_error,
            "Expected TraitNotSatisfied for user-defined trait Greetable via alias. Errors: {:?}",
            typeck.errors
        );

        // Part B: Should NOT error -- Person implements Greetable
        let source_good = r#"
interface Greetable do
  fn greet(self) -> String
end

struct Person do
  name :: String
end

impl Greetable for Person do
  fn greet(self) -> String do
    "hello"
  end
end

fn say_hello<T>(x :: T) -> String where T: Greetable do
  greet(x)
end

fn main() do
  let f = say_hello
  let p = Person { name: "Alice" }
  f(p)
end
"#;
        let parse_good = mesh_parser::parse(source_good);
        let typeck_good = mesh_typeck::check(&parse_good);

        let has_trait_error_good = typeck_good
            .errors
            .iter()
            .any(|e| matches!(e, mesh_typeck::error::TypeError::TraitNotSatisfied { .. }));
        assert!(
            !has_trait_error_good,
            "Should NOT get TraitNotSatisfied when calling aliased constrained function with conforming type. Errors: {:?}",
            typeck_good.errors
        );
    }

    /// QUAL-01: Higher-order apply with conforming type should NOT produce
    /// TraitNotSatisfied. apply(show, 42) where Int implements Displayable.
    #[test]
    fn e2e_qualified_type_higher_order_apply() {
        let source = r#"
interface Displayable do
  fn display(self) -> String
end

impl Displayable for Int do
  fn display(self) -> String do
    "int"
  end
end

fn show<T>(x :: T) -> String where T: Displayable do
  display(x)
end

fn apply(f, x) do
  f(x)
end

fn main() do
  apply(show, 42)
end
"#;
        let parse = mesh_parser::parse(source);
        let typeck = mesh_typeck::check(&parse);

        let has_trait_error = typeck
            .errors
            .iter()
            .any(|e| matches!(e, mesh_typeck::error::TypeError::TraitNotSatisfied { .. }));
        assert!(
            !has_trait_error,
            "Should NOT get TraitNotSatisfied when passing show to apply with conforming type. Errors: {:?}",
            typeck.errors
        );
    }

    /// QUAL-03: Higher-order apply with non-conforming type MUST produce
    /// TraitNotSatisfied. apply(say_hello, 42) where Int does NOT implement Greetable.
    #[test]
    fn e2e_qualified_type_higher_order_violation() {
        let source = r#"
interface Greetable do
  fn greet(self) -> String
end

fn say_hello<T>(x :: T) -> String where T: Greetable do
  greet(x)
end

fn apply(f, x) do
  f(x)
end

fn main() do
  apply(say_hello, 42)
end
"#;
        let parse = mesh_parser::parse(source);
        let typeck = mesh_typeck::check(&parse);

        let has_trait_error = typeck
            .errors
            .iter()
            .any(|e| matches!(e, mesh_typeck::error::TypeError::TraitNotSatisfied { .. }));
        assert!(
            has_trait_error,
            "Expected TraitNotSatisfied when passing constrained function to apply with non-conforming type. Errors: {:?}",
            typeck.errors
        );
    }

    /// QUAL-02: Nested higher-order constraint propagation.
    /// wrap(apply, show, 42) should NOT produce TraitNotSatisfied when Int implements Displayable.
    #[test]
    fn e2e_qualified_type_nested_higher_order() {
        let source = r#"
interface Displayable do
  fn display(self) -> String
end

impl Displayable for Int do
  fn display(self) -> String do
    "int"
  end
end

fn show<T>(x :: T) -> String where T: Displayable do
  display(x)
end

fn apply(f, x) do
  f(x)
end

fn wrap(f, g, x) do
  f(g, x)
end

fn main() do
  wrap(apply, show, 42)
end
"#;
        let parse = mesh_parser::parse(source);
        let typeck = mesh_typeck::check(&parse);

        let has_trait_error = typeck
            .errors
            .iter()
            .any(|e| matches!(e, mesh_typeck::error::TypeError::TraitNotSatisfied { .. }));
        assert!(
            !has_trait_error,
            "Should NOT get TraitNotSatisfied for nested higher-order constraint propagation. Errors: {:?}",
            typeck.errors
        );
    }

    /// QUAL-01 positive: Conforming type with full impl body.
    /// apply(show, 42) where Int implements Displayable with actual method.
    #[test]
    fn e2e_qualified_type_higher_order_conforming() {
        let source = r#"
interface Displayable do
  fn display(self) -> String
end

impl Displayable for Int do
  fn display(self) -> String do
    "${self}"
  end
end

fn show<T>(x :: T) -> String where T: Displayable do
  display(x)
end

fn apply(f, x) do
  f(x)
end

fn main() do
  let result = apply(show, 42)
  result
end
"#;
        let parse = mesh_parser::parse(source);
        let typeck = mesh_typeck::check(&parse);

        let has_trait_error = typeck
            .errors
            .iter()
            .any(|e| matches!(e, mesh_typeck::error::TypeError::TraitNotSatisfied { .. }));
        assert!(
            !has_trait_error,
            "Should NOT get TraitNotSatisfied with conforming type in higher-order apply. Errors: {:?}",
            typeck.errors
        );
    }

    /// QUAL-01 + Phase 25 interaction: let alias of constrained function passed as
    /// higher-order argument. let f = show; apply(f, 42) should NOT produce TraitNotSatisfied.
    #[test]
    fn e2e_qualified_type_higher_order_let_alias() {
        let source = r#"
interface Displayable do
  fn display(self) -> String
end

impl Displayable for Int do
  fn display(self) -> String do
    "int"
  end
end

fn show<T>(x :: T) -> String where T: Displayable do
  display(x)
end

fn apply(f, x) do
  f(x)
end

fn main() do
  let f = show
  apply(f, 42)
end
"#;
        let parse = mesh_parser::parse(source);
        let typeck = mesh_typeck::check(&parse);

        let has_trait_error = typeck
            .errors
            .iter()
            .any(|e| matches!(e, mesh_typeck::error::TypeError::TraitNotSatisfied { .. }));
        assert!(
            !has_trait_error,
            "Should NOT get TraitNotSatisfied when passing let-aliased constrained function to apply. Errors: {:?}",
            typeck.errors
        );
    }

    /// Success Criterion 5: Depth limit machinery is in place.
    /// Normal programs produce no Panic nodes; the depth counter fields exist.
    #[test]
    fn e2e_depth_limit_field_exists() {
        // Lower a normal trait-using program and verify no Panic nodes.
        let source = r#"
interface Greetable do
  fn greet(self) -> String
end

struct Greeter do
  name :: String
end

impl Greetable for Greeter do
  fn greet(self) -> String do
    "hello"
  end
end

fn main() do
  let g = Greeter { name: "world" }
  greet(g)
end
"#;
        let mir = lower(source);

        // No Panic nodes should appear in a normal program.

        for func in &mir.functions {
            assert!(
                !contains(&func.body, |node| matches!(node, MirExpr::Panic { .. })),
                "Normal trait program should not have Panic nodes, found in '{}': {:?}",
                func.name,
                func.body
            );
        }

        // Verify the Lowerer is initialized with depth tracking by confirming
        // that lowering succeeds (the fields exist and are properly initialized).
        // The Lowerer struct is private, so we verify indirectly through behavior.
        let parse = mesh_parser::parse(source);
        let typeck = mesh_typeck::check(&parse);
        let empty_pub_fns = HashSet::new();
        let _mir = lower_to_mir(&parse, &typeck, "", &empty_pub_fns, &HashMap::new())
            .expect("MIR lowering with depth tracking");
    }

    #[test]
    fn debug_inspect_struct_generates_mir_function() {
        let source = r#"
struct Point do
  x :: Int
  y :: Int
end

fn main() do
  let p = Point { x: 1, y: 2 }
  println("test")
end
"#;
        let mir = lower(source);
        let inspect_fn = mir
            .functions
            .iter()
            .find(|f| f.name == "Debug__inspect__Point");
        assert!(
            inspect_fn.is_some(),
            "Expected Debug__inspect__Point function in MIR. Functions: {:?}",
            mir.functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
        let inspect_fn = inspect_fn.unwrap();
        assert_eq!(inspect_fn.params.len(), 1);
        assert_eq!(inspect_fn.params[0].0, "self");
        assert_eq!(inspect_fn.return_type, MirType::String);
    }

    #[test]
    fn debug_inspect_sum_type_generates_mir_function() {
        let source = r#"
type Color do
  Red
  Green
  Blue
end

fn main() do
  println("test")
end
"#;
        let mir = lower(source);
        let inspect_fn = mir
            .functions
            .iter()
            .find(|f| f.name == "Debug__inspect__Color");
        assert!(
            inspect_fn.is_some(),
            "Expected Debug__inspect__Color function in MIR. Functions: {:?}",
            mir.functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
        let inspect_fn = inspect_fn.unwrap();
        assert_eq!(inspect_fn.params.len(), 1);
        assert_eq!(inspect_fn.params[0].0, "self");
        assert_eq!(inspect_fn.return_type, MirType::String);
    }

    #[test]
    fn eq_struct_generates_mir_function() {
        let source = r#"
struct Point do
  x :: Int
  y :: Int
end

fn main() do
  let p = Point { x: 1, y: 2 }
  println("test")
end
"#;
        let mir = lower(source);
        let eq_fn = mir.functions.iter().find(|f| f.name == "Eq__eq__Point");
        assert!(
            eq_fn.is_some(),
            "Expected Eq__eq__Point function in MIR. Functions: {:?}",
            mir.functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
        let eq_fn = eq_fn.unwrap();
        assert_eq!(eq_fn.params.len(), 2);
        assert_eq!(eq_fn.params[0].0, "self");
        assert_eq!(eq_fn.params[1].0, "other");
        assert_eq!(eq_fn.return_type, MirType::Bool);
    }

    #[test]
    fn ord_struct_generates_mir_function() {
        let source = r#"
struct Point do
  x :: Int
  y :: Int
end

fn main() do
  let p = Point { x: 1, y: 2 }
  println("test")
end
"#;
        let mir = lower(source);
        let ord_fn = mir.functions.iter().find(|f| f.name == "Ord__lt__Point");
        assert!(
            ord_fn.is_some(),
            "Expected Ord__lt__Point function in MIR. Functions: {:?}",
            mir.functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
        let ord_fn = ord_fn.unwrap();
        assert_eq!(ord_fn.params.len(), 2);
        assert_eq!(ord_fn.params[0].0, "self");
        assert_eq!(ord_fn.params[1].0, "other");
        assert_eq!(ord_fn.return_type, MirType::Bool);
        // `self` is less when the lexicographic comparison is negative.
        assert!(matches!(ord_fn.body, MirExpr::BinOp { op: BinOp::Lt, .. }));
    }

    #[test]
    fn eq_empty_struct_returns_true() {
        let source = r#"
struct Empty do
end

fn main() do
  println("test")
end
"#;
        let mir = lower(source);
        let eq_fn = mir.functions.iter().find(|f| f.name == "Eq__eq__Empty");
        assert!(eq_fn.is_some());
        let eq_fn = eq_fn.unwrap();
        assert!(matches!(eq_fn.body, MirExpr::BoolLit(true, _)));
    }

    #[test]
    fn ord_empty_struct_returns_false() {
        let source = r#"
struct Empty do
end

fn main() do
  println("test")
end
"#;
        let mir = lower(source);
        let ord_fn = mir.functions.iter().find(|f| f.name == "Ord__lt__Empty");
        assert!(ord_fn.is_some());
        let ord_fn = ord_fn.unwrap();
        assert!(matches!(ord_fn.body, MirExpr::BoolLit(false, _)));
    }

    #[test]
    fn struct_eq_operator_dispatches_to_trait_call() {
        let source = r#"
struct Point do
  x :: Int
  y :: Int
end

fn check(a :: Point, b :: Point) -> Bool do
  a == b
end
"#;
        let mir = lower(source);
        let check_fn = mir.functions.iter().find(|f| f.name == "check");
        assert!(
            check_fn.is_some(),
            "Expected 'check' function in MIR. Functions: {:?}",
            mir.functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
        let check_fn = check_fn.unwrap();
        let body_str = format!("{:?}", check_fn.body);
        assert!(
            body_str.contains("Eq__eq__Point"),
            "Expected Eq__eq__Point call in check body, got: {}",
            body_str
        );
    }

    #[test]
    fn struct_neq_operator_negates_eq() {
        let source = r#"
struct Point do
  x :: Int
  y :: Int
end

fn check(a :: Point, b :: Point) -> Bool do
  a != b
end
"#;
        let mir = lower(source);
        let check_fn = mir.functions.iter().find(|f| f.name == "check");
        assert!(
            check_fn.is_some(),
            "Expected 'check' function in MIR. Functions: {:?}",
            mir.functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
        let check_fn = check_fn.unwrap();
        let body_str = format!("{:?}", check_fn.body);
        // Should contain Eq__eq__Point (since != dispatches through Eq with negation)
        assert!(
            body_str.contains("Eq__eq__Point"),
            "Expected Eq__eq__Point call in check body for !=, got: {}",
            body_str
        );
    }

    #[test]
    fn struct_lt_operator_dispatches_to_ord() {
        let source = r#"
struct Point do
  x :: Int
  y :: Int
end

fn check(a :: Point, b :: Point) -> Bool do
  a < b
end
"#;
        let mir = lower(source);
        let check_fn = mir.functions.iter().find(|f| f.name == "check");
        assert!(
            check_fn.is_some(),
            "Expected 'check' function in MIR. Functions: {:?}",
            mir.functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
        let check_fn = check_fn.unwrap();
        // `<` compares through the type's generated three-way `__cmp_Point`,
        // which is what calls the Ord impl.
        assert!(calls(&check_fn.body, "__cmp_Point"), "{:?}", check_fn.body);
        let cmp_fn = mir
            .functions
            .iter()
            .find(|f| f.name == "__cmp_Point")
            .unwrap();
        let body_str = format!("{:?}", cmp_fn.body);
        assert!(
            body_str.contains("Ord__lt__Point"),
            "Expected Ord__lt__Point call in check body for <, got: {}",
            body_str
        );
    }

    // ── Sum type Eq/Ord tests ────────────────────────────────────────

    #[test]
    fn eq_sum_generates_mir_function() {
        let source = r#"
type Color do
  Red
  Green(Int)
  Blue(Int, Int)
end

fn main() do
  println("test")
end
"#;
        let mir = lower(source);
        let eq_fn = mir.functions.iter().find(|f| f.name == "Eq__eq__Color");
        assert!(
            eq_fn.is_some(),
            "Expected Eq__eq__Color function in MIR. Functions: {:?}",
            mir.functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
        let eq_fn = eq_fn.unwrap();
        assert_eq!(eq_fn.params.len(), 2);
        assert_eq!(eq_fn.params[0].0, "self");
        assert_eq!(eq_fn.params[1].0, "other");
        assert_eq!(eq_fn.params[0].1, MirType::SumType("Color".to_string()));
        assert_eq!(eq_fn.return_type, MirType::Bool);
        // Body uses Match for variant dispatch
        assert!(matches!(eq_fn.body, MirExpr::Match { .. }));
    }

    #[test]
    fn ord_sum_generates_mir_function() {
        let source = r#"
type Color do
  Red
  Green(Int)
  Blue(Int, Int)
end

fn main() do
  println("test")
end
"#;
        let mir = lower(source);
        let ord_fn = mir.functions.iter().find(|f| f.name == "Ord__lt__Color");
        assert!(
            ord_fn.is_some(),
            "Expected Ord__lt__Color function in MIR. Functions: {:?}",
            mir.functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
        let ord_fn = ord_fn.unwrap();
        assert_eq!(ord_fn.params.len(), 2);
        assert_eq!(ord_fn.params[0].0, "self");
        assert_eq!(ord_fn.params[1].0, "other");
        assert_eq!(ord_fn.params[0].1, MirType::SumType("Color".to_string()));
        assert_eq!(ord_fn.return_type, MirType::Bool);
        // `self` is less when the tag-then-payload comparison is negative.
        assert!(matches!(ord_fn.body, MirExpr::BinOp { op: BinOp::Lt, .. }));
    }

    #[test]
    fn eq_sum_no_variants_returns_true() {
        let source = r#"
type Empty do
end

fn main() do
  println("test")
end
"#;
        let mir = lower(source);
        let eq_fn = mir.functions.iter().find(|f| f.name == "Eq__eq__Empty");
        assert!(eq_fn.is_some());
        let eq_fn = eq_fn.unwrap();
        assert!(matches!(eq_fn.body, MirExpr::BoolLit(true, _)));
    }

    #[test]
    fn ord_sum_no_variants_returns_false() {
        let source = r#"
type Empty do
end

fn main() do
  println("test")
end
"#;
        let mir = lower(source);
        let ord_fn = mir.functions.iter().find(|f| f.name == "Ord__lt__Empty");
        assert!(ord_fn.is_some());
        let ord_fn = ord_fn.unwrap();
        assert!(matches!(ord_fn.body, MirExpr::BoolLit(false, _)));
    }

    #[test]
    fn sum_eq_operator_dispatches_to_trait_call() {
        let source = r#"
type Color do
  Red
  Green(Int)
end

fn check(a :: Color, b :: Color) -> Bool do
  a == b
end
"#;
        let mir = lower(source);
        let check_fn = mir.functions.iter().find(|f| f.name == "check");
        assert!(
            check_fn.is_some(),
            "Expected 'check' function in MIR. Functions: {:?}",
            mir.functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
        let check_fn = check_fn.unwrap();
        let body_str = format!("{:?}", check_fn.body);
        assert!(
            body_str.contains("Eq__eq__Color"),
            "Expected Eq__eq__Color call in check body, got: {}",
            body_str
        );
    }

    #[test]
    fn sum_neq_operator_negates_eq() {
        let source = r#"
type Color do
  Red
  Green(Int)
end

fn check(a :: Color, b :: Color) -> Bool do
  a != b
end
"#;
        let mir = lower(source);
        let check_fn = mir.functions.iter().find(|f| f.name == "check");
        assert!(
            check_fn.is_some(),
            "Expected 'check' function in MIR. Functions: {:?}",
            mir.functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
        let check_fn = check_fn.unwrap();
        let body_str = format!("{:?}", check_fn.body);
        // != dispatches through Eq with negation
        assert!(
            body_str.contains("Eq__eq__Color"),
            "Expected Eq__eq__Color call in check body for !=, got: {}",
            body_str
        );
    }

    #[test]
    fn sum_lt_operator_dispatches_to_ord() {
        let source = r#"
type Color do
  Red
  Green(Int)
end

fn check(a :: Color, b :: Color) -> Bool do
  a < b
end
"#;
        let mir = lower(source);
        let check_fn = mir.functions.iter().find(|f| f.name == "check");
        assert!(
            check_fn.is_some(),
            "Expected 'check' function in MIR. Functions: {:?}",
            mir.functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
        let check_fn = check_fn.unwrap();
        assert!(calls(&check_fn.body, "__cmp_Color"), "{:?}", check_fn.body);
        let cmp_fn = mir
            .functions
            .iter()
            .find(|f| f.name == "__cmp_Color")
            .unwrap();
        let body_str = format!("{:?}", cmp_fn.body);
        assert!(
            body_str.contains("Ord__lt__Color"),
            "Expected Ord__lt__Color call in check body for <, got: {}",
            body_str
        );
    }

    #[test]
    fn eq_sum_unit_variants_only() {
        // Sum type with only unit variants (no payload fields)
        let source = r#"
type Direction do
  North
  South
  East
  West
end

fn main() do
  println("test")
end
"#;
        let mir = lower(source);
        let eq_fn = mir.functions.iter().find(|f| f.name == "Eq__eq__Direction");
        assert!(eq_fn.is_some());
        let eq_fn = eq_fn.unwrap();
        // Body should be a Match with variant-based dispatch
        assert!(matches!(eq_fn.body, MirExpr::Match { .. }));
        // Each arm should ultimately yield true (same variant) or false (different variant)
        if let MirExpr::Match { arms, .. } = &eq_fn.body {
            assert_eq!(arms.len(), 4, "Should have one arm per variant");
        }
    }

    // ── Hash MIR generation tests ───────────────────────────────────

    #[test]
    fn hash_struct_generates_mir_function() {
        let source = r#"
struct Point do
  x :: Int
  y :: Int
end

fn main() do
  println("test")
end
"#;
        let mir = lower(source);
        let hash_fn = mir.functions.iter().find(|f| f.name == "Hash__hash__Point");
        assert!(
            hash_fn.is_some(),
            "Expected Hash__hash__Point function in MIR"
        );
        let hash_fn = hash_fn.unwrap();
        assert_eq!(hash_fn.params.len(), 1);
        assert_eq!(hash_fn.params[0].0, "self");
        assert_eq!(hash_fn.return_type, MirType::Int);
    }

    #[test]
    fn hash_struct_field_chaining() {
        let source = r#"
struct Point do
  x :: Int
  y :: Int
end

fn main() do
  println("test")
end
"#;
        let mir = lower(source);
        let hash_fn = mir
            .functions
            .iter()
            .find(|f| f.name == "Hash__hash__Point")
            .unwrap();
        // Body should contain a mesh_hash_combine call (chaining two field hashes).
        assert!(
            calls(&hash_fn.body, "mesh_hash_combine"),
            "Hash body should contain mesh_hash_combine for multi-field struct"
        );
    }

    #[test]
    fn hash_empty_struct_returns_constant() {
        let source = r#"
struct Empty do
end

fn main() do
  println("test")
end
"#;
        let mir = lower(source);
        let hash_fn = mir.functions.iter().find(|f| f.name == "Hash__hash__Empty");
        assert!(
            hash_fn.is_some(),
            "Expected Hash__hash__Empty function in MIR"
        );
        let hash_fn = hash_fn.unwrap();
        // Empty struct hash is a constant: the hash of its field count.
        assert!(matches!(
            &hash_fn.body,
            MirExpr::Call { args, .. } if matches!(args[..], [MirExpr::IntLit(0, _)])
        ));
    }

    #[test]
    fn map_put_with_struct_key_compares_by_eq() {
        let source = r#"
struct Point do
  x :: Int
  y :: Int
end

fn main() do
  let p = Point { x: 1, y: 2 }
  let m = Map.new()
  let m2 = Map.put(m, p, 42)
  m2
end
"#;
        let mir = lower(source);
        let main_fn = mir.functions.iter().find(|f| f.name == "mesh_main");
        assert!(main_fn.is_some(), "Expected mesh_main function in MIR");
        // A struct key is compared by Point's Eq: the put goes through a typed
        // wrapper passing the key Eq callback (it used to store the key's hash,
        // so two keys with one hash collided and Map.keys returned hashes).
        assert!(calls_where(&main_fn.unwrap().body, |name| name
            .starts_with("__map_put_")));
        let wrapper = mir
            .functions
            .iter()
            .find(|f| f.name.starts_with("__map_put_"))
            .expect("typed put wrapper");
        assert!(format!("{:?}", wrapper.body).contains("mesh_map_put_by"));
    }

    // ── Default MIR lowering tests ──────────────────────────────────

    #[test]
    fn default_int_short_circuits_to_literal() {
        let source = r#"
fn main() do
  let x :: Int = default()
  x
end
"#;
        let mir = lower(source);
        let main_fn = mir.functions.iter().find(|f| f.name == "mesh_main");
        assert!(main_fn.is_some(), "Expected mesh_main function in MIR");
        // The body should contain an IntLit(0) somewhere (from default() -> 0).
        fn has_int_zero(expr: &MirExpr) -> bool {
            match expr {
                MirExpr::IntLit(0, MirType::Int) => true,
                MirExpr::Let { value, body, .. } => has_int_zero(value) || has_int_zero(body),
                MirExpr::Call { args, .. } => args.iter().any(has_int_zero),
                _ => false,
            }
        }
        assert!(
            has_int_zero(&main_fn.unwrap().body),
            "default() for Int should produce IntLit(0)"
        );
    }

    #[test]
    fn default_float_short_circuits_to_literal() {
        let source = r#"
fn main() do
  let x :: Float = default()
  x
end
"#;
        let mir = lower(source);
        let main_fn = mir.functions.iter().find(|f| f.name == "mesh_main");
        assert!(main_fn.is_some(), "Expected mesh_main function in MIR");
        fn has_float_zero(expr: &MirExpr) -> bool {
            match expr {
                MirExpr::FloatLit(val, MirType::Float) if *val == 0.0 => true,
                MirExpr::Let { value, body, .. } => has_float_zero(value) || has_float_zero(body),
                MirExpr::Call { args, .. } => args.iter().any(has_float_zero),
                _ => false,
            }
        }
        assert!(
            has_float_zero(&main_fn.unwrap().body),
            "default() for Float should produce FloatLit(0.0)"
        );
    }

    #[test]
    fn default_string_short_circuits_to_literal() {
        let source = r#"
fn main() do
  let x :: String = default()
  x
end
"#;
        let mir = lower(source);
        let main_fn = mir.functions.iter().find(|f| f.name == "mesh_main");
        assert!(main_fn.is_some(), "Expected mesh_main function in MIR");
        fn has_empty_string(expr: &MirExpr) -> bool {
            match expr {
                MirExpr::StringLit(s, MirType::String) if s.is_empty() => true,
                MirExpr::Let { value, body, .. } => {
                    has_empty_string(value) || has_empty_string(body)
                }
                MirExpr::Call { args, .. } => args.iter().any(has_empty_string),
                _ => false,
            }
        }
        assert!(
            has_empty_string(&main_fn.unwrap().body),
            "default() for String should produce StringLit(\"\")"
        );
    }

    #[test]
    fn default_bool_short_circuits_to_literal() {
        let source = r#"
fn main() do
  let x :: Bool = default()
  x
end
"#;
        let mir = lower(source);
        let main_fn = mir.functions.iter().find(|f| f.name == "mesh_main");
        assert!(main_fn.is_some(), "Expected mesh_main function in MIR");
        fn has_bool_false(expr: &MirExpr) -> bool {
            match expr {
                MirExpr::BoolLit(false, MirType::Bool) => true,
                MirExpr::Let { value, body, .. } => has_bool_false(value) || has_bool_false(body),
                MirExpr::Call { args, .. } => args.iter().any(has_bool_false),
                _ => false,
            }
        }
        assert!(
            has_bool_false(&main_fn.unwrap().body),
            "default() for Bool should produce BoolLit(false)"
        );
    }

    // ── Default method body tests (21-03) ────────────────────────────

    #[test]
    fn default_method_skips_missing_error() {
        // An impl that omits a method with has_default_body=true should compile without error.
        let source = r#"
struct Point do
  x :: Int
  y :: Int
end

interface Describable do
  fn describe(self) -> String do
    "unknown"
  end
end

impl Describable for Point do
end
"#;
        let parse = mesh_parser::parse(source);
        let typeck = mesh_typeck::check(&parse);
        // Check that there are no MissingTraitMethod errors.
        let missing_errors: Vec<_> = typeck
            .errors
            .iter()
            .filter(|e| matches!(e, mesh_typeck::error::TypeError::MissingTraitMethod { .. }))
            .collect();
        assert!(
            missing_errors.is_empty(),
            "Expected no MissingTraitMethod errors, got: {:?}",
            missing_errors
        );
        // Should also lower to MIR without failure.
        let empty_pub_fns = HashSet::new();
        let mir = lower_to_mir(&parse, &typeck, "", &empty_pub_fns, &HashMap::new())
            .expect("MIR lowering failed");
        assert!(
            mir.functions
                .iter()
                .any(|f| f.name == "Describable__describe__Point"),
            "Expected default method function Describable__describe__Point in MIR, got: {:?}",
            mir.functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn default_method_body_lowered_for_concrete_type() {
        // Verify that when impl Describable for Point omits `describe`,
        // the MIR contains a Describable__describe__Point function generated from the default body.
        let source = r#"
struct Point do
  x :: Int
  y :: Int
end

interface Describable do
  fn describe(self) -> String do
    "unknown"
  end
end

impl Describable for Point do
end
"#;
        let mir = lower(source);
        let func = mir
            .functions
            .iter()
            .find(|f| f.name == "Describable__describe__Point");
        assert!(
            func.is_some(),
            "Expected Describable__describe__Point function in MIR, got: {:?}",
            mir.functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
        let func = func.unwrap();
        // The self parameter should be present and typed to the concrete type.
        assert!(!func.params.is_empty(), "Expected at least self parameter");
        assert_eq!(func.params[0].0, "self");
    }

    #[test]
    fn override_replaces_default() {
        // When impl provides the method, the default is NOT used.
        let source = r#"
struct Point do
  x :: Int
  y :: Int
end

interface Describable do
  fn describe(self) -> String do
    "unknown"
  end
end

impl Describable for Point do
  fn describe(self) -> String do
    "point"
  end
end
"#;
        let mir = lower(source);
        // There should be exactly one Describable__describe__Point function (the override).
        let funcs: Vec<_> = mir
            .functions
            .iter()
            .filter(|f| f.name == "Describable__describe__Point")
            .collect();
        assert_eq!(
            funcs.len(),
            1,
            "Expected exactly 1 Describable__describe__Point, got {}",
            funcs.len()
        );
        // The body should contain the override string "point", not "unknown".
        fn has_string(expr: &MirExpr, s: &str) -> bool {
            match expr {
                MirExpr::StringLit(val, _) => val == s,
                MirExpr::Block(exprs, _) => exprs.iter().any(|e| has_string(e, s)),
                MirExpr::Let { value, body, .. } => has_string(value, s) || has_string(body, s),
                _ => false,
            }
        }
        assert!(
            has_string(&funcs[0].body, "point"),
            "Override body should contain 'point', got: {:?}",
            funcs[0].body
        );
        assert!(
            !has_string(&funcs[0].body, "unknown"),
            "Override body should NOT contain 'unknown'"
        );
    }

    // ── Collection Display tests (Phase 21 Plan 04) ─────────────────

    /// Helper: recursively check if a MirExpr tree contains a Call to a
    /// function with the given name.
    fn has_call_to(expr: &MirExpr, fn_name: &str) -> bool {
        match expr {
            MirExpr::Call { func, args, .. } => {
                if let MirExpr::Var(name, _) = func.as_ref() {
                    if name == fn_name {
                        return true;
                    }
                }
                args.iter().any(|a| has_call_to(a, fn_name)) || has_call_to(func, fn_name)
            }
            MirExpr::Block(exprs, _) => exprs.iter().any(|e| has_call_to(e, fn_name)),
            MirExpr::Let { value, body, .. } => {
                has_call_to(value, fn_name) || has_call_to(body, fn_name)
            }
            _ => false,
        }
    }

    /// Helper: check if a MirExpr tree contains a Var reference to the given name.
    fn has_var_ref(expr: &MirExpr, var_name: &str) -> bool {
        match expr {
            MirExpr::Var(name, _) => name == var_name,
            MirExpr::Call { func, args, .. } => {
                has_var_ref(func, var_name) || args.iter().any(|a| has_var_ref(a, var_name))
            }
            MirExpr::Block(exprs, _) => exprs.iter().any(|e| has_var_ref(e, var_name)),
            MirExpr::Let { value, body, .. } => {
                has_var_ref(value, var_name) || has_var_ref(body, var_name)
            }
            _ => false,
        }
    }

    #[test]
    fn list_display_emits_runtime_call() {
        // String interpolation with a List should emit mesh_list_to_string
        // with mesh_int_to_string as the element callback.
        let source = r#"
fn main() do
  let xs = List.append(List.new(), 1)
  "items: ${xs}"
end
"#;
        let mir = lower(source);
        let main = mir.functions.iter().find(|f| f.name == "mesh_main");
        assert!(main.is_some(), "Expected 'mesh_main' function in MIR");
        let main = main.unwrap();

        assert!(
            has_call_to(&main.body, "mesh_list_to_string"),
            "Expected mesh_list_to_string call in interpolated string body.\n\
             Body: {:?}",
            main.body
        );
        assert!(
            has_var_ref(&main.body, "mesh_int_to_string"),
            "Expected mesh_int_to_string callback reference in interpolated string body.\n\
             Body: {:?}",
            main.body
        );
    }

    #[test]
    fn map_display_emits_runtime_call() {
        // String interpolation with a Map<String, Int> should emit mesh_map_to_string
        // with mesh_string_to_string and mesh_int_to_string as callbacks.
        let source = r#"
fn main() do
  let m = %{"a" => 1}
  "map: ${m}"
end
"#;
        let mir = lower(source);
        let main = mir.functions.iter().find(|f| f.name == "mesh_main");
        assert!(main.is_some(), "Expected 'mesh_main' function in MIR");
        let main = main.unwrap();

        assert!(
            has_call_to(&main.body, "mesh_map_to_string"),
            "Expected mesh_map_to_string call in interpolated string body.\n\
             Body: {:?}",
            main.body
        );
    }

    #[test]
    fn set_display_emits_runtime_call() {
        // String interpolation with a Set should emit mesh_set_to_string.
        let source = r#"
fn main() do
  let s = Set.add(Set.new(), 1)
  "set: ${s}"
end
"#;
        let mir = lower(source);
        let main = mir.functions.iter().find(|f| f.name == "mesh_main");
        assert!(main.is_some(), "Expected 'mesh_main' function in MIR");
        let main = main.unwrap();

        assert!(
            has_call_to(&main.body, "mesh_set_to_string"),
            "Expected mesh_set_to_string call in interpolated string body.\n\
             Body: {:?}",
            main.body
        );
    }

    // ── Phase 24 Plan 01: Nested collection Display ─────────────────

    #[test]
    fn nested_list_callback_generates_wrapper() {
        // When a Lowerer encounters a Ty::App(Con("List"), [Ty::Con("Int")])
        // element type, resolve_to_string_callback should generate a synthetic
        // __display_list_Int_to_str wrapper function.
        //
        // We test this indirectly: lower a program with list string interpolation,
        // then verify the mesh_list_to_string call is present and uses
        // mesh_int_to_string (flat case). The wrapper generation for nested
        // types (List<List<Int>>) will be exercised once the type system
        // supports generic collection element types (TGEN-02).
        let source = r#"
fn main() do
  let xs = List.append(List.new(), 42)
  "${xs}"
end
"#;
        let mir = lower(source);
        let main = mir.functions.iter().find(|f| f.name == "mesh_main");
        assert!(main.is_some(), "Expected 'mesh_main' function in MIR");
        let main = main.unwrap();

        // The flat list case: mesh_list_to_string with mesh_int_to_string callback
        assert!(
            has_call_to(&main.body, "mesh_list_to_string"),
            "Expected mesh_list_to_string call.\nBody: {:?}",
            main.body
        );
        assert!(
            has_var_ref(&main.body, "mesh_int_to_string"),
            "Expected mesh_int_to_string callback reference.\nBody: {:?}",
            main.body
        );

        // Verify no wrapper was generated for the flat case (Int is handled
        // directly, no __display_ wrapper needed).
        let has_display_wrapper = mir
            .functions
            .iter()
            .any(|f| f.name.starts_with("__display_"));
        assert!(
            !has_display_wrapper,
            "Flat List<Int> should NOT generate a __display_ wrapper.\n\
             Functions: {:?}",
            mir.functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
    }

    // ── Phase 23 Plan 02: Ordering & compare tests ──────────────────

    #[test]
    fn ordering_sum_type_registered_in_mir() {
        // Ordering should be registered as a built-in sum type in every MIR module.
        let mir = lower("fn main() do 1 end");
        let ordering = mir.sum_types.iter().find(|s| s.name == "Ordering");
        assert!(
            ordering.is_some(),
            "Expected Ordering sum type in MIR. Sum types: {:?}",
            mir.sum_types.iter().map(|s| &s.name).collect::<Vec<_>>()
        );
        let ordering = ordering.unwrap();
        assert_eq!(ordering.variants.len(), 3);
        assert_eq!(ordering.variants[0].name, "Less");
        assert_eq!(ordering.variants[0].tag, 0);
        assert_eq!(ordering.variants[1].name, "Equal");
        assert_eq!(ordering.variants[1].tag, 1);
        assert_eq!(ordering.variants[2].name, "Greater");
        assert_eq!(ordering.variants[2].tag, 2);
    }

    #[test]
    fn compare_primitive_functions_generated() {
        // Ord__compare__Int, Ord__compare__Float, Ord__compare__String should exist.
        let mir = lower("fn main() do 1 end");
        let fns: Vec<&str> = mir.functions.iter().map(|f| f.name.as_str()).collect();
        assert!(
            fns.contains(&"Ord__compare__Int"),
            "Missing Ord__compare__Int. Fns: {:?}",
            fns
        );
        assert!(
            fns.contains(&"Ord__compare__Float"),
            "Missing Ord__compare__Float. Fns: {:?}",
            fns
        );
        assert!(
            fns.contains(&"Ord__compare__String"),
            "Missing Ord__compare__String. Fns: {:?}",
            fns
        );

        // Check Ord__compare__Int signature
        let compare_int = mir
            .functions
            .iter()
            .find(|f| f.name == "Ord__compare__Int")
            .unwrap();
        assert_eq!(compare_int.params.len(), 2);
        assert_eq!(compare_int.params[0].1, MirType::Int);
        assert_eq!(compare_int.params[1].1, MirType::Int);
        assert_eq!(
            compare_int.return_type,
            MirType::SumType("Ordering".to_string())
        );
    }

    #[test]
    fn compare_call_dispatches_by_operand_type() {
        // compare(3, 5) compares the operands as Ints
        let source = r#"
fn main() -> Ordering do
  compare(3, 5)
end
"#;
        let mir = lower(source);
        let main = mir.functions.iter().find(|f| f.name == "mesh_main");
        assert!(main.is_some(), "Expected 'mesh_main' function in MIR");
        let main = main.unwrap();
        assert!(
            has_call_to(&main.body, "__cmp_Int"),
            "Expected __cmp_Int call in main body.\nBody: {:?}",
            main.body
        );
    }

    #[test]
    fn compare_struct_generated_for_user_types() {
        // User structs with Ord derive should get Ord__compare__StructName.
        let source = r#"
struct Point do
  x :: Int
  y :: Int
end

fn main() do
  let p = Point { x: 1, y: 2 }
  println("test")
end
"#;
        let mir = lower(source);
        let compare_fn = mir
            .functions
            .iter()
            .find(|f| f.name == "Ord__compare__Point");
        assert!(
            compare_fn.is_some(),
            "Expected Ord__compare__Point function in MIR. Functions: {:?}",
            mir.functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
        let compare_fn = compare_fn.unwrap();
        assert_eq!(compare_fn.params.len(), 2);
        assert_eq!(
            compare_fn.return_type,
            MirType::SumType("Ordering".to_string())
        );
    }

    #[test]
    fn compare_sum_generated_for_user_sum_types() {
        // User sum types with Ord derive should get Ord__compare__SumTypeName.
        let source = r#"
type Color do
  Red
  Green
  Blue
end

fn main() do
  println("test")
end
"#;
        let mir = lower(source);
        let compare_fn = mir
            .functions
            .iter()
            .find(|f| f.name == "Ord__compare__Color");
        assert!(
            compare_fn.is_some(),
            "Expected Ord__compare__Color function in MIR. Functions: {:?}",
            mir.functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
        let compare_fn = compare_fn.unwrap();
        assert_eq!(compare_fn.params.len(), 2);
        assert_eq!(
            compare_fn.return_type,
            MirType::SumType("Ordering".to_string())
        );
    }

    #[test]
    fn pattern_match_some_extracts_field() {
        // case Some(42) do Some(x) -> x | None -> 0 end
        // The match should produce MirExpr::Match with Constructor patterns.
        let source = r#"
fn main() -> Int do
  let opt = Some(42)
  case opt do
    Some(x) -> x
    None -> 0
  end
end
"#;
        let mir = lower(source);
        let main = mir.functions.iter().find(|f| f.name == "mesh_main");
        assert!(main.is_some(), "Expected 'mesh_main' function in MIR");
        let main = main.unwrap();

        // The body should contain a Match expression with Constructor patterns
        fn has_match_with_some(expr: &MirExpr) -> bool {
            match expr {
                MirExpr::Match { arms, .. } => {
                    arms.iter().any(|arm| matches!(&arm.pattern, MirPattern::Constructor { variant, .. } if variant == "Some"))
                }
                MirExpr::Let { value, body, .. } => {
                    has_match_with_some(value) || has_match_with_some(body)
                }
                MirExpr::Block(exprs, _) => exprs.iter().any(has_match_with_some),
                _ => false,
            }
        }
        assert!(
            has_match_with_some(&main.body),
            "Expected Match with Some constructor pattern in main body.\nBody: {:?}",
            main.body
        );
    }

    #[test]
    fn pattern_match_ordering_variants() {
        // Pattern matching on Ordering should produce Constructor patterns.
        let source = r#"
fn main() -> Int do
  let ord = compare(3, 5)
  case ord do
    Less -> 1
    Equal -> 2
    Greater -> 3
  end
end
"#;
        let mir = lower(source);
        let main = mir.functions.iter().find(|f| f.name == "mesh_main");
        assert!(main.is_some(), "Expected 'mesh_main' function in MIR");
        let main = main.unwrap();

        // Should dispatch compare call
        assert!(
            has_call_to(&main.body, "__cmp_Int"),
            "Expected __cmp_Int call in main body.\nBody: {:?}",
            main.body
        );
    }

    // ── Method dot-syntax MIR tests (Phase 30-02) ─────────────────────

    #[test]
    fn e2e_method_dot_syntax_basic() {
        // METH-01 + METH-02: p.to_string() should produce same mangled call as to_string(p)
        let source = r#"
struct Point do
  x :: Int
  y :: Int
end

interface Display do
  fn to_string(self) -> String
end

impl Display for Point do
  fn to_string(p) do
    "Point"
  end
end

fn main() do
  let p = Point { x: 10, y: 20 }
  let result = p.to_string()
  println(result)
end
"#;
        let mir = lower(source);
        let main_fn = mir
            .functions
            .iter()
            .find(|f| f.name == "mesh_main")
            .expect("Expected mesh_main function");
        assert!(
            calls(&main_fn.body, "Display__to_string__Point"),
            "Expected call to Display__to_string__Point in main body (method dot-syntax), got: {:?}",
            main_fn.body
        );
    }

    #[test]
    fn e2e_method_dot_syntax_equivalence() {
        // METH-02: p.to_string() and to_string(p) should resolve to same mangled name
        let source = r#"
struct Point do
  x :: Int
  y :: Int
end

interface Display do
  fn to_string(self) -> String
end

impl Display for Point do
  fn to_string(p) do
    "Point"
  end
end

fn main() do
  let p = Point { x: 1, y: 2 }
  let a = to_string(p)
  let b = p.to_string()
  println(a)
  println(b)
end
"#;
        let mir = lower(source);
        let main_fn = mir
            .functions
            .iter()
            .find(|f| f.name == "mesh_main")
            .expect("Expected mesh_main function");

        // Count calls to the mangled name -- should be 2 (one bare, one dot-syntax)
        fn count_calls(expr: &MirExpr, target: &str) -> usize {
            match expr {
                MirExpr::Call { func, args, .. } => {
                    let mut n = if let MirExpr::Var(name, _) = func.as_ref() {
                        if name == target {
                            1
                        } else {
                            0
                        }
                    } else {
                        0
                    };
                    n += count_calls(func, target);
                    for arg in args {
                        n += count_calls(arg, target);
                    }
                    n
                }
                MirExpr::Let { value, body, .. } => {
                    count_calls(value, target) + count_calls(body, target)
                }
                MirExpr::Block(exprs, _) => exprs.iter().map(|e| count_calls(e, target)).sum(),
                MirExpr::If {
                    cond,
                    then_body,
                    else_body,
                    ..
                } => {
                    count_calls(cond, target)
                        + count_calls(then_body, target)
                        + count_calls(else_body, target)
                }
                _ => 0,
            }
        }

        let call_count = count_calls(&main_fn.body, "Display__to_string__Point");
        assert_eq!(
            call_count, 2,
            "Expected exactly 2 calls to Display__to_string__Point (bare + dot), got {}.\nBody: {:?}",
            call_count, main_fn.body
        );
    }

    #[test]
    fn e2e_method_dot_syntax_with_args() {
        // METH-02: receiver + additional args
        let source = r#"
interface Greeter do
  fn greet(self, greeting :: String) -> String
end

struct Person do
  name :: String
end

impl Greeter for Person do
  fn greet(p, greeting) do
    greeting
  end
end

fn main() do
  let bob = Person { name: "Bob" }
  let result = bob.greet("Hello")
  println(result)
end
"#;
        let mir = lower(source);
        let main_fn = mir
            .functions
            .iter()
            .find(|f| f.name == "mesh_main")
            .expect("Expected mesh_main function");
        assert!(
            calls(&main_fn.body, "Greeter__greet__Person"),
            "Expected call to Greeter__greet__Person in main body (dot-syntax with args), got: {:?}",
            main_fn.body
        );
    }

    #[test]
    fn e2e_method_dot_syntax_field_access_preserved() {
        // INTG-01: p.x should still produce FieldAccess, not a method call
        let source = r#"
struct Point do
  x :: Int
  y :: Int
end

fn main() do
  let p = Point { x: 42, y: 99 }
  let val = p.x
  println(Int.to_string(val))
end
"#;
        let mir = lower(source);
        let main_fn = mir
            .functions
            .iter()
            .find(|f| f.name == "mesh_main")
            .expect("Expected mesh_main function");

        // Check that a FieldAccess for "x" exists in the body
        fn has_field_access(expr: &MirExpr, field_name: &str) -> bool {
            match expr {
                MirExpr::FieldAccess { field, object, .. } => {
                    field == field_name || has_field_access(object, field_name)
                }
                MirExpr::Let { value, body, .. } => {
                    has_field_access(value, field_name) || has_field_access(body, field_name)
                }
                MirExpr::Block(exprs, _) => exprs.iter().any(|e| has_field_access(e, field_name)),
                MirExpr::Call { func, args, .. } => {
                    has_field_access(func, field_name)
                        || args.iter().any(|a| has_field_access(a, field_name))
                }
                _ => false,
            }
        }

        assert!(
            has_field_access(&main_fn.body, "x"),
            "Expected FieldAccess for 'x' in main body (field access must be preserved), got: {:?}",
            main_fn.body
        );
    }

    #[test]
    fn e2e_method_dot_syntax_module_qualified_preserved() {
        // INTG-02: String.length(s) should still work as module-qualified call
        let source = r#"
fn main() do
  let s = "hello world"
  let len = String.length(s)
  println(Int.to_string(len))
end
"#;
        let mir = lower(source);
        let main_fn = mir
            .functions
            .iter()
            .find(|f| f.name == "mesh_main")
            .expect("Expected mesh_main function");
        assert!(
            calls(&main_fn.body, "mesh_string_length"),
            "Expected call to mesh_string_length in main body (module-qualified preserved), got: {:?}",
            main_fn.body
        );
    }

    #[test]
    fn lower_while_expr() {
        let mir = lower("fn test() do while true do 1 end end");
        let func = mir.functions.iter().find(|f| f.name == "test");
        assert!(func.is_some(), "Expected 'test' function in MIR");
        assert!(
            matches!(func.unwrap().body, MirExpr::While { .. }),
            "Expected MirExpr::While, got: {:?}",
            func.unwrap().body
        );
    }

    #[test]
    fn lower_break_expr() {
        let mir = lower("fn test() do while true do break end end");
        let func = mir.functions.iter().find(|f| f.name == "test");
        assert!(func.is_some());
        // The while body should contain a Break
        fn has_break(expr: &MirExpr) -> bool {
            match expr {
                MirExpr::Break => true,
                MirExpr::While { body, .. } => has_break(body),
                MirExpr::Block(exprs, _) => exprs.iter().any(has_break),
                _ => false,
            }
        }
        assert!(
            has_break(&func.unwrap().body),
            "Expected MirExpr::Break in while body"
        );
    }

    #[test]
    fn lower_continue_expr() {
        let mir = lower("fn test() do while true do continue end end");
        let func = mir.functions.iter().find(|f| f.name == "test");
        assert!(func.is_some());
        fn has_continue(expr: &MirExpr) -> bool {
            match expr {
                MirExpr::Continue => true,
                MirExpr::While { body, .. } => has_continue(body),
                MirExpr::Block(exprs, _) => exprs.iter().any(has_continue),
                _ => false,
            }
        }
        assert!(
            has_continue(&func.unwrap().body),
            "Expected MirExpr::Continue in while body"
        );
    }

    #[test]
    fn lower_for_in_range_expr() {
        let mir = lower("fn test() do for i in 0..10 do println(i) end end");
        let func = mir.functions.iter().find(|f| f.name == "test");
        assert!(func.is_some(), "Expected 'test' function in MIR");
        let func = func.unwrap();
        match &func.body {
            MirExpr::ForInRange {
                var,
                start,
                end,
                ty,
                ..
            } => {
                assert_eq!(var, "i");
                assert!(
                    matches!(start.as_ref(), MirExpr::IntLit(0, _)),
                    "Expected start=0, got {:?}",
                    start
                );
                assert!(
                    matches!(end.as_ref(), MirExpr::IntLit(10, _)),
                    "Expected end=10, got {:?}",
                    end
                );
                assert_eq!(*ty, MirType::Ptr);
            }
            other => panic!("Expected MirExpr::ForInRange, got {:?}", other),
        }
    }
}
