//! Unification engine for Hindley-Milner type inference.
//!
//! Implements the core unification algorithm using `ena`'s union-find table.
//! Supports occurs check (infinite type detection), level-based generalization,
//! and scheme instantiation.

use ena::unify::InPlaceUnificationTable;
use rowan::TextRange;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::error::{ConstraintOrigin, TypeError};
use crate::ty::{Scheme, Ty, TyCon, TyVar};
use crate::{ClusteredRouteReplicationCount, ClusteredRouteWrapperMetadata};

/// The inference context -- owns the unification table, level state, and errors.
///
/// All type inference happens through this context. It creates fresh type
/// variables, unifies types, tracks levels for generalization, and collects
/// errors.
/// A method call that several impls could answer (`Meters.convert()` with
/// `Convert<Int>` and `Convert<String>`): its open result type, and the
/// return types of the impls.
#[derive(Clone, Debug)]
pub struct ImplChoice {
    pub result: Ty,
    pub method: String,
    pub receiver: Ty,
    pub candidates: Vec<Ty>,
    pub span: TextRange,
}

pub struct InferCtx {
    /// The union-find unification table (ena).
    table: InPlaceUnificationTable<TyVar>,
    /// Current let-nesting level for generalization.
    current_level: u32,
    /// Level at which each type variable was created.
    /// Indexed by `TyVar.0`.
    var_levels: Vec<u32>,
    /// Type errors accumulated during inference.
    pub errors: Vec<TypeError>,
    /// Warnings accumulated during inference (e.g. redundant match arms).
    pub warnings: Vec<TypeError>,
    /// Current loop nesting depth (0 = not inside any loop).
    /// Incremented when entering a while body, reset to 0 when entering a closure body.
    pub loop_depth: u32,
    /// The interface whose default method body is being checked, if any:
    /// `self` then has the type `Self`, whose methods are the interface's own.
    pub current_interface: Option<String>,
    /// User-defined module namespaces for qualified access (e.g., Vector.add()).
    /// Populated during ImportDecl processing, read during field access resolution.
    /// Key is the module namespace name, value maps function/type name to its scheme.
    pub qualified_modules: FxHashMap<String, FxHashMap<String, Scheme>>,
    /// Function names imported via `from Module import name1, name2`.
    /// Tracked so the MIR lowerer can skip trait dispatch for these.
    pub imported_functions: Vec<String>,
    /// Defining module for imported bare function references.
    /// Keyed by the imported in-scope name (e.g. `handle_submit` -> `Work`).
    pub imported_function_origins: FxHashMap<String, String>,
    /// Full module name for qualified module namespaces (e.g. `Todos` -> `Api.Todos`).
    pub qualified_module_origins: FxHashMap<String, String>,
    /// Private names exported by a qualified module namespace.
    /// Used to turn `HTTP.clustered(Module.hidden)` into a focused privacy diagnostic.
    pub qualified_module_private_names: FxHashMap<String, FxHashSet<String>>,
    /// Top-level function visibility in the current module.
    /// Keyed by the unqualified function name; value is true when the fn is public.
    pub top_level_function_visibility: FxHashMap<String, bool>,
    /// Metadata for each `HTTP.clustered(...)` wrapper call.
    pub clustered_route_wrappers: FxHashMap<TextRange, ClusteredRouteWrapperMetadata>,
    /// Wrapper call ranges that were consumed in the route-handler slot.
    pub consumed_clustered_route_wrappers: FxHashSet<TextRange>,
    /// Function arguments passed where a callback returning `()` is expected
    /// whose own result is discarded; lowering wraps each one in an adapter.
    pub discarded_callback_results: FxHashSet<TextRange>,
    /// Replication counts already declared for a clustered route runtime name.
    pub clustered_route_replication_counts: FxHashMap<String, ClusteredRouteReplicationCount>,
    /// Service method mappings imported from other modules.
    /// Maps service_name -> Vec<(method_name, generated_fn_name)>.
    /// Populated during import resolution, propagated to TypeckResult for MIR lowering.
    pub imported_service_methods: FxHashMap<String, Vec<(String, String)>>,
    /// Locally-defined service export info for collect_exports.
    /// Populated by infer_service_def with resolved helper function types.
    pub local_service_exports: FxHashMap<String, crate::ServiceExportInfo>,
    /// The name of the current module being type-checked (e.g., "Geometry").
    /// None for single-file mode. Used to set display_prefix on locally-defined types.
    pub current_module: Option<String>,
    /// Whether compiler-provided test-only builtins are available.
    pub test_builtins: bool,
    /// Stack of enclosing function/closure return types.
    /// Pushed when entering a function/closure body, popped when leaving.
    /// `None` means the return type is not yet known (will be inferred).
    pub fn_return_type_stack: Vec<Option<Ty>>,
    /// Per entry of `fn_return_type_stack`: the types of `return` values in
    /// a function whose return type is not declared.
    pub fn_returned_types: Vec<Vec<Ty>>,
    /// The `where` bounds of the function whose body is being checked: each
    /// type parameter's variable with a trait it must implement.
    pub where_bounds: Vec<(Ty, String)>,
    /// Associated types used through a type parameter (`c.first()` with
    /// `first(self) -> Self.Item` under `where T: Container`): the variable
    /// standing for it, the trait, the associated type's name and the
    /// receiver. Lowering binds the variable in each specialization.
    pub assoc_projections: Vec<(Ty, String, String, Ty)>,
    /// A generic function's body can fix an associated type (`c.first() + 1`
    /// makes `T.Item` an Int). Each instance of the function then requires it
    /// of its receiver: (required type, trait, associated type name, the
    /// instance's receiver, where the function was used).
    pub projection_requirements: Vec<(Ty, String, String, Ty, Option<TextRange>)>,
    /// The module's types that derive Json, known before any is checked, so
    /// `deriving(Json)` accepts fields of types declared later or recursively.
    pub json_types: rustc_hash::FxHashSet<String>,
    /// The variants of this module's sum types, by name, with the type that
    /// declares each: one name may belong to only one of them.
    pub local_variants: FxHashMap<String, String>,
    /// Type definitions registered before the main pass, which skips them.
    pub registered_items: FxHashSet<TextRange>,
    /// Operators applied to values of a type not yet known, with the trait
    /// each needs: a generic function's own type parameter must be bounded
    /// by it (`where T: Ord`), checked when the function's body is done.
    pub operand_traits: Vec<(Ty, String, ConstraintOrigin)>,
    /// The builtin `default()` calls of the function being inferred, with
    /// the type each builds (checked when the function is done).
    pub default_calls: Vec<(Ty, TextRange)>,
    /// Method calls that several impls could answer, to be decided by the
    /// type their context gives the result (checked when the function is done).
    pub impl_choices: Vec<ImplChoice>,
    /// Pub fn names that have multiple definitions with different arities.
    /// Used to mangle exported names as name__N for arity overloading.
    pub overloaded_pub_fn_names: FxHashSet<String>,
    /// Maps call-site TextRange -> mangled callee name (e.g. "slugify__2").
    /// Populated during inference for arity-overloaded calls.
    /// Consumed by the MIR lowerer to emit the correct function reference.
    pub overloaded_call_targets: FxHashMap<TextRange, String>,
    /// The expressions being inferred, innermost last: where a constraint
    /// with no more specific origin is reported.
    pub expr_spans: Vec<TextRange>,
}

impl InferCtx {
    /// Create a new, empty inference context.
    pub fn new() -> Self {
        InferCtx {
            table: InPlaceUnificationTable::new(),
            current_level: 0,
            var_levels: Vec::new(),
            errors: Vec::new(),
            warnings: Vec::new(),
            loop_depth: 0,
            current_interface: None,
            qualified_modules: FxHashMap::default(),
            imported_functions: Vec::new(),
            imported_function_origins: FxHashMap::default(),
            qualified_module_origins: FxHashMap::default(),
            qualified_module_private_names: FxHashMap::default(),
            top_level_function_visibility: FxHashMap::default(),
            clustered_route_wrappers: FxHashMap::default(),
            consumed_clustered_route_wrappers: FxHashSet::default(),
            discarded_callback_results: FxHashSet::default(),
            clustered_route_replication_counts: FxHashMap::default(),
            imported_service_methods: FxHashMap::default(),
            local_service_exports: FxHashMap::default(),
            current_module: None,
            test_builtins: false,
            fn_return_type_stack: Vec::new(),
            fn_returned_types: Vec::new(),
            where_bounds: Vec::new(),
            assoc_projections: Vec::new(),
            projection_requirements: Vec::new(),
            json_types: Default::default(),
            local_variants: Default::default(),
            registered_items: Default::default(),
            operand_traits: Vec::new(),
            default_calls: Vec::new(),
            impl_choices: Vec::new(),
            overloaded_pub_fn_names: FxHashSet::default(),
            overloaded_call_targets: FxHashMap::default(),
            expr_spans: Vec::new(),
        }
    }

    /// Enter a loop body -- increments loop_depth.
    pub fn enter_loop(&mut self) {
        self.loop_depth += 1;
    }

    /// Exit a loop body -- decrements loop_depth.
    pub fn exit_loop(&mut self) {
        self.loop_depth = self.loop_depth.saturating_sub(1);
    }

    /// Enter a closure body -- saves and resets loop_depth to 0.
    /// Returns the saved loop_depth to restore later.
    pub fn enter_closure(&mut self) -> u32 {
        let saved = self.loop_depth;
        self.loop_depth = 0;
        saved
    }

    /// Exit a closure body -- restores loop_depth from saved value.
    pub fn exit_closure(&mut self, saved_depth: u32) {
        self.loop_depth = saved_depth;
    }

    /// Whether we are currently inside a loop.
    pub fn in_loop(&self) -> bool {
        self.loop_depth > 0
    }

    /// Push a function return type onto the stack (call when entering a function body).
    pub fn push_fn_return_type(&mut self, ty: Option<Ty>) {
        self.fn_return_type_stack.push(ty);
        self.fn_returned_types.push(Vec::new());
    }

    /// Pop a function return type from the stack (call when leaving a function
    /// body). Returns the types of the `return` values seen while the return
    /// type was not declared, for the caller to join with the body's type.
    pub fn pop_fn_return_type(&mut self) -> Vec<Ty> {
        self.fn_return_type_stack.pop();
        self.fn_returned_types.pop().unwrap_or_default()
    }

    /// Record the type of a `return` value in a function without a declared
    /// return type.
    pub fn record_return(&mut self, ty: Ty) {
        if let Some(returns) = self.fn_returned_types.last_mut() {
            returns.push(ty);
        }
    }

    /// Get the current enclosing function's return type (top of stack).
    pub fn current_fn_return_type(&self) -> Option<&Ty> {
        self.fn_return_type_stack.last().and_then(|t| t.as_ref())
    }

    // ── Type Variable Creation ──────────────────────────────────────────

    /// Create a fresh type variable at the current level.
    pub fn fresh_var(&mut self) -> Ty {
        let var = self.table.new_key(None);
        // Ensure var_levels is large enough.
        while self.var_levels.len() <= var.0 as usize {
            self.var_levels.push(0);
        }
        self.var_levels[var.0 as usize] = self.current_level;
        Ty::Var(var)
    }

    /// A fresh variable at the level of `anchor`'s variable (the current level
    /// when it has none): a `let` inside `anchor`'s scope does not generalize
    /// it away from `anchor`.
    pub fn fresh_var_beside(&mut self, anchor: &Ty) -> Ty {
        let fresh = self.fresh_var();
        if let (Ty::Var(anchor), Ty::Var(var)) = (self.resolve(anchor.clone()), &fresh) {
            if let Some(&level) = self.var_levels.get(anchor.0 as usize) {
                self.var_levels[var.0 as usize] = level;
            }
        }
        fresh
    }

    // ── Resolution ──────────────────────────────────────────────────────

    /// Resolve a type by following union-find indirection.
    ///
    /// If the type is a variable with a known value, recursively resolve
    /// that value. Otherwise return the type as-is.
    pub fn resolve(&mut self, ty: Ty) -> Ty {
        match ty {
            Ty::Var(v) => {
                let probe = self.table.probe_value(v);
                match probe {
                    Some(inner) => self.resolve(inner),
                    None => {
                        // Normalize to the root key so that variables
                        // in the same equivalence class resolve to the
                        // same representative. This is critical for
                        // generalization: two unified-but-unbound vars
                        // must appear as the same variable.
                        let root = self.table.find(v);
                        Ty::Var(root)
                    }
                }
            }
            // For compound types, resolve recursively.
            Ty::Fun(params, ret) => {
                let params = params.into_iter().map(|p| self.resolve(p)).collect();
                let ret = Box::new(self.resolve(*ret));
                Ty::Fun(params, ret)
            }
            Ty::App(con, args) => {
                let con = Box::new(self.resolve(*con));
                let args = args.into_iter().map(|a| self.resolve(a)).collect();
                // A tuple row whose tail has become known is that tuple.
                Ty::App(con, args).normalize_tuple_row()
            }
            Ty::Tuple(elems) => {
                let elems = elems.into_iter().map(|e| self.resolve(e)).collect();
                Ty::Tuple(elems)
            }
            other => other,
        }
    }

    // ── Occurs Check ────────────────────────────────────────────────────

    /// Check if a type variable occurs anywhere within a type.
    ///
    /// This prevents infinite types like `a ~ (a) -> Int` which would
    /// create the infinite type `(((((...) -> Int) -> Int) -> Int) -> Int)`.
    pub fn occurs_in(&mut self, var: TyVar, ty: &Ty) -> bool {
        match ty {
            Ty::Var(v) => {
                if *v == var {
                    return true;
                }
                // Follow the union-find to see if this var is bound.
                let probe = self.table.probe_value(*v);
                match probe {
                    Some(inner) => self.occurs_in(var, &inner),
                    None => false,
                }
            }
            Ty::Con(_) => false,
            Ty::Fun(params, ret) => {
                params.iter().any(|p| self.occurs_in(var, p)) || self.occurs_in(var, ret)
            }
            Ty::App(con, args) => {
                self.occurs_in(var, con) || args.iter().any(|a| self.occurs_in(var, a))
            }
            Ty::Tuple(elems) => elems.iter().any(|e| self.occurs_in(var, e)),
            Ty::Never => false,
        }
    }

    /// Check if two TyCon names are compatible because they both represent
    /// opaque iterator handle pointers. All iterator handle types
    /// (ListIterator, MapIterator, etc.) and adapter types resolve to
    /// MirType::Ptr at the MIR/codegen level, so they must be unifiable
    /// with each other and with the generic `Ptr` type in the type checker.
    /// Json auto-coerces to String at use sites.
    ///
    /// When a String is expected but a Json is provided (or vice versa), they are
    /// considered compatible. This enables `HTTP.response(200, json { ... })` to
    /// work without explicit conversion.
    fn json_string_compatible(c1: &TyCon, c2: &TyCon) -> bool {
        matches!(
            (c1.name.as_str(), c2.name.as_str()),
            ("Json", "String") | ("String", "Json")
        )
    }

    fn iterator_ptr_compatible(c1: &TyCon, c2: &TyCon) -> bool {
        fn is_iter_ptr(name: &str) -> bool {
            name == "Ptr"
                || name == "ListIterator"
                || name == "MapIterator"
                || name == "SetIterator"
                || name == "RangeIterator"
                || name == "MapAdapterIterator"
                || name == "FilterAdapterIterator"
                || name == "TakeAdapterIterator"
                || name == "SkipAdapterIterator"
                || name == "EnumerateAdapterIterator"
                || name == "ZipAdapterIterator"
        }
        is_iter_ptr(&c1.name) && is_iter_ptr(&c2.name)
    }

    /// Unify a tuple row (`Ty::tuple_row`) with a tuple, another row, or the
    /// untyped `Tuple`. `None` when neither side is a row, or the other side is
    /// a variable, which `unify` binds to the row like to any other type.
    fn unify_tuple_row(
        &mut self,
        a: &Ty,
        b: &Ty,
        origin: &ConstraintOrigin,
    ) -> Option<Result<(), TypeError>> {
        // (the row's leading elements, its tail, what they must equal, what the tail must equal)
        let (elems, tail, known, rest) = match (a.as_tuple_row(), b.as_tuple_row()) {
            (None, None) => return None,
            // Two rows agree on the elements both name; the shorter one's tail
            // is the longer one's remainder.
            (Some(row_a), Some(row_b)) => {
                let (short, long) = if row_a.0.len() <= row_b.0.len() {
                    (row_a, row_b)
                } else {
                    (row_b, row_a)
                };
                let (named, remainder) = long.0.split_at(short.0.len());
                let rest = if remainder.is_empty() {
                    long.1.clone()
                } else {
                    Ty::tuple_row(remainder.to_vec(), long.1.clone())
                };
                (short.0.to_vec(), short.1.clone(), named.to_vec(), rest)
            }
            (Some(row), None) | (None, Some(row)) => {
                let other = if a.as_tuple_row().is_some() { b } else { a };
                let (known, rest) = match other {
                    Ty::Var(_) => return None,
                    // The row's elements are the tuple's first ones, its tail the rest.
                    Ty::Tuple(all) if all.len() >= row.0.len() => {
                        let (named, remainder) = all.split_at(row.0.len());
                        (named.to_vec(), Ty::Tuple(remainder.to_vec()))
                    }
                    // The untyped `Tuple` promises `Int` elements, as its accessors do.
                    Ty::Con(c) if c.name == "Tuple" => {
                        (vec![Ty::int(); row.0.len()], row.1.clone())
                    }
                    _ => {
                        let err = TypeError::Mismatch {
                            expected: a.clone(),
                            found: b.clone(),
                            origin: origin.clone(),
                        };
                        self.errors.push(err.clone());
                        return Some(Err(err));
                    }
                };
                (row.0.to_vec(), row.1.clone(), known, rest)
            }
        };
        for (elem, known) in elems.into_iter().zip(known) {
            if let Err(err) = self.unify(elem, known, origin.clone()) {
                return Some(Err(err));
            }
        }
        Some(self.unify(tail, rest, origin.clone()))
    }

    // ── Unification ─────────────────────────────────────────────────────

    /// Unify two types, making them equal.
    ///
    /// This is the core of HM inference. Both types are first resolved
    /// through the union-find table, then structurally compared. If they
    /// differ, a type error is recorded.
    pub fn unify(&mut self, a: Ty, b: Ty, origin: ConstraintOrigin) -> Result<(), TypeError> {
        let origin = match (origin, self.expr_spans.last()) {
            (ConstraintOrigin::Builtin, Some(&span)) => ConstraintOrigin::Expr { span },
            (origin, _) => origin,
        };
        let a = self.resolve(a);
        let b = self.resolve(b);

        if let Some(result) = self.unify_tuple_row(&a, &b, &origin) {
            return result;
        }

        match (a, b) {
            // Two identical variables -- already unified.
            (Ty::Var(v1), Ty::Var(v2)) if v1 == v2 => Ok(()),

            // Variable meets variable -- union them.
            (Ty::Var(v1), Ty::Var(v2)) => {
                self.table
                    .unify_var_var(v1, v2)
                    .expect("unifying two unbound vars should not fail");
                Ok(())
            }

            // Variable meets concrete type -- bind the variable (with occurs check).
            (Ty::Var(v), ty) | (ty, Ty::Var(v)) => {
                if self.occurs_in(v, &ty) {
                    let err = TypeError::InfiniteType { var: v, ty, origin };
                    self.errors.push(err.clone());
                    Err(err)
                } else {
                    self.table.unify_var_value(v, Some(ty)).expect(
                        "binding a var to a concrete type after occurs check should not fail",
                    );
                    Ok(())
                }
            }

            // Concrete constructor meets concrete constructor -- names must match.
            (Ty::Con(c1), Ty::Con(c2)) => {
                if c1 == c2
                    || Self::iterator_ptr_compatible(&c1, &c2)
                    || Self::json_string_compatible(&c1, &c2)
                {
                    Ok(())
                } else {
                    let err = TypeError::Mismatch {
                        expected: Ty::Con(c1),
                        found: Ty::Con(c2),
                        origin,
                    };
                    self.errors.push(err.clone());
                    Err(err)
                }
            }

            // Function types -- unify params pairwise, then return types.
            (Ty::Fun(p1, r1), Ty::Fun(p2, r2)) => {
                if p1.len() != p2.len() {
                    let err = TypeError::ArityMismatch {
                        expected: p1.len(),
                        found: p2.len(),
                        origin,
                    };
                    self.errors.push(err.clone());
                    Err(err)
                } else {
                    for (a, b) in p1.into_iter().zip(p2.into_iter()) {
                        self.unify(a, b, origin.clone())?;
                    }
                    self.unify(*r1, *r2, origin)
                }
            }

            // Pid escape hatch: untyped Pid (Con) unifies with typed Pid<M> (App).
            // This allows: let untyped :: Pid = typed_pid  (typed -> untyped).
            (Ty::Con(ref c), Ty::App(ref con, _)) | (Ty::App(ref con, _), Ty::Con(ref c))
                if c.name == "Pid" && matches!(con.as_ref(), Ty::Con(tc) if tc.name == "Pid") =>
            {
                Ok(())
            }

            // Iterator handles (`ListIterator`, the untyped `Ptr`, ...) carry no
            // element type; they are compatible with any `Iter<T>`.
            (Ty::Con(ref c), Ty::App(ref con, _)) | (Ty::App(ref con, _), Ty::Con(ref c))
                if matches!(con.as_ref(), Ty::Con(tc) if tc.name == "Iter")
                    && Self::iterator_ptr_compatible(c, &TyCon::new("Ptr")) =>
            {
                Ok(())
            }

            // Non-generic type identity: Con("Point") == App(Con("Point"), [])
            // This arises because infer_struct_literal returns App(Con(name), []) for
            // non-generic structs, while name_to_type returns Con(name). Both represent
            // the same type.
            (Ty::Con(ref c), Ty::App(ref con, ref args))
            | (Ty::App(ref con, ref args), Ty::Con(ref c))
                if args.is_empty()
                    && matches!(con.as_ref(), Ty::Con(ref ac) if ac.name == c.name) =>
            {
                Ok(())
            }

            // Type applications -- unify constructor and args.
            (Ty::App(c1, a1), Ty::App(c2, a2)) => {
                self.unify(*c1, *c2, origin.clone())?;
                if a1.len() != a2.len() {
                    let err = TypeError::ArityMismatch {
                        expected: a1.len(),
                        found: a2.len(),
                        origin,
                    };
                    self.errors.push(err.clone());
                    Err(err)
                } else {
                    for (a, b) in a1.into_iter().zip(a2.into_iter()) {
                        self.unify(a, b, origin.clone())?;
                    }
                    Ok(())
                }
            }

            // Tuple escape hatch: untyped Tuple (Con) unifies with any typed tuple (Ty::Tuple).
            // This allows Tuple.first/Tuple.second to accept concrete tuple types like (Int, String).
            (Ty::Con(ref c), Ty::Tuple(_)) | (Ty::Tuple(_), Ty::Con(ref c))
                if c.name == "Tuple" =>
            {
                Ok(())
            }

            // Tuple types -- unify element-wise.
            (Ty::Tuple(e1), Ty::Tuple(e2)) => {
                if e1.len() != e2.len() {
                    let err = TypeError::ArityMismatch {
                        expected: e1.len(),
                        found: e2.len(),
                        origin,
                    };
                    self.errors.push(err.clone());
                    Err(err)
                } else {
                    for (a, b) in e1.into_iter().zip(e2.into_iter()) {
                        self.unify(a, b, origin.clone())?;
                    }
                    Ok(())
                }
            }

            // Never unifies with anything (bottom type).
            (Ty::Never, _) | (_, Ty::Never) => Ok(()),

            // Everything else is a mismatch.
            (a, b) => {
                let err = TypeError::Mismatch {
                    expected: a,
                    found: b,
                    origin,
                };
                self.errors.push(err.clone());
                Err(err)
            }
        }
    }

    // ── Level Management ────────────────────────────────────────────────

    /// Enter a new let-binding level (increases nesting depth).
    pub fn enter_level(&mut self) {
        self.current_level += 1;
    }

    /// Leave the current let-binding level (decreases nesting depth).
    pub fn leave_level(&mut self) {
        debug_assert!(self.current_level > 0, "cannot leave level 0");
        self.current_level -= 1;
    }

    /// Current nesting level.
    pub fn current_level(&self) -> u32 {
        self.current_level
    }

    // ── Generalization ──────────────────────────────────────────────────

    /// Generalize a type into a polymorphic scheme.
    ///
    /// Collects all type variables in `ty` whose level is strictly greater
    /// than `current_level` -- these are the variables that were introduced
    /// at a deeper level and can be universally quantified.
    pub fn generalize(&mut self, ty: Ty) -> Scheme {
        let resolved = self.resolve(ty);
        let mut free_vars = Vec::new();
        self.collect_generalizable_vars(&resolved, &mut free_vars);
        // Deduplicate while preserving order.
        let mut seen = std::collections::HashSet::new();
        free_vars.retain(|v| seen.insert(*v));
        Scheme {
            vars: free_vars,
            ty: resolved,
        }
    }

    /// Collect type variables that can be generalized (level > current_level).
    fn collect_generalizable_vars(&mut self, ty: &Ty, out: &mut Vec<TyVar>) {
        match ty {
            Ty::Var(v) => {
                let probe = self.table.probe_value(*v);
                match probe {
                    Some(inner) => self.collect_generalizable_vars(&inner, out),
                    None => {
                        let level = self.var_levels.get(v.0 as usize).copied().unwrap_or(0);
                        if level > self.current_level {
                            out.push(*v);
                        }
                    }
                }
            }
            Ty::Con(_) | Ty::Never => {}
            Ty::Fun(params, ret) => {
                for p in params {
                    self.collect_generalizable_vars(p, out);
                }
                self.collect_generalizable_vars(ret, out);
            }
            Ty::App(con, args) => {
                self.collect_generalizable_vars(con, out);
                for a in args {
                    self.collect_generalizable_vars(a, out);
                }
            }
            Ty::Tuple(elems) => {
                for e in elems {
                    self.collect_generalizable_vars(e, out);
                }
            }
        }
    }

    // ── Instantiation ───────────────────────────────────────────────────

    /// Instantiate a polymorphic scheme with fresh type variables.
    ///
    /// Creates a fresh type variable for each quantified variable in the
    /// scheme, then substitutes them throughout the type.
    pub fn instantiate(&mut self, scheme: &Scheme) -> Ty {
        if scheme.vars.is_empty() {
            return scheme.ty.clone();
        }

        let substitution: FxHashMap<TyVar, Ty> =
            scheme.vars.iter().map(|v| (*v, self.fresh_var())).collect();

        // An associated type the scheme holds as a variable is the same
        // associated type of the instance's receiver.
        let mut instances = Vec::new();
        for (var, trait_name, assoc, receiver) in self.assoc_projections.clone() {
            let receiver = self.resolve(receiver);
            let Ty::Var(receiver_root) = receiver else {
                continue;
            };
            if !substitution.contains_key(&receiver_root) {
                continue;
            }
            let instance_receiver = substitution[&receiver_root].clone();
            match self.resolve(var) {
                Ty::Var(root) if substitution.contains_key(&root) => instances.push((
                    substitution[&root].clone(),
                    trait_name,
                    assoc,
                    instance_receiver,
                )),
                Ty::Var(_) => {}
                fixed => self.projection_requirements.push((
                    fixed,
                    trait_name,
                    assoc,
                    instance_receiver,
                    self.expr_spans.last().copied(),
                )),
            }
        }
        self.assoc_projections.extend(instances);

        self.apply_substitution(&scheme.ty, &substitution)
    }

    /// Apply a substitution map to a type.
    fn apply_substitution(&mut self, ty: &Ty, subst: &FxHashMap<TyVar, Ty>) -> Ty {
        match ty {
            Ty::Var(v) => {
                if let Some(replacement) = subst.get(v) {
                    replacement.clone()
                } else {
                    // Check if this var is bound in the table.
                    let probe = self.table.probe_value(*v);
                    match probe {
                        Some(inner) => self.apply_substitution(&inner, subst),
                        None => ty.clone(),
                    }
                }
            }
            Ty::Con(_) | Ty::Never => ty.clone(),
            Ty::Fun(params, ret) => {
                let params = params
                    .iter()
                    .map(|p| self.apply_substitution(p, subst))
                    .collect();
                let ret = Box::new(self.apply_substitution(ret, subst));
                Ty::Fun(params, ret)
            }
            Ty::App(con, args) => {
                let con = Box::new(self.apply_substitution(con, subst));
                let args = args
                    .iter()
                    .map(|a| self.apply_substitution(a, subst))
                    .collect();
                Ty::App(con, args)
            }
            Ty::Tuple(elems) => {
                let elems = elems
                    .iter()
                    .map(|e| self.apply_substitution(e, subst))
                    .collect();
                Ty::Tuple(elems)
            }
        }
    }
}

impl Default for InferCtx {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ConstraintOrigin;
    use crate::ty::TyCon;

    fn builtin_origin() -> ConstraintOrigin {
        ConstraintOrigin::Builtin
    }

    #[test]
    fn a_tuple_row_takes_its_elements_from_the_tuple_it_meets() {
        let mut ctx = InferCtx::new();
        // What `Tuple.first(p)` says about a `p` of unknown type.
        let (p, first, tail) = (ctx.fresh_var(), ctx.fresh_var(), ctx.fresh_var());
        let row = Ty::tuple_row(vec![first.clone()], tail);
        assert!(ctx.unify(p.clone(), row, builtin_origin()).is_ok());
        assert!(ctx.resolve(p.clone()).as_tuple_row().is_some());

        let pair = Ty::Tuple(vec![Ty::string(), Ty::int()]);
        assert!(ctx.unify(p.clone(), pair.clone(), builtin_origin()).is_ok());
        assert_eq!(ctx.resolve(first), Ty::string());
        assert_eq!(ctx.resolve(p), pair, "a row with a known tail is the tuple");
    }

    #[test]
    fn two_tuple_rows_agree_on_the_elements_both_name() {
        let mut ctx = InferCtx::new();
        // `Tuple.first(p)` and then `Tuple.second(p)` on the same unknown `p`.
        let p = ctx.fresh_var();
        let (a, t1) = (ctx.fresh_var(), ctx.fresh_var());
        let (b, c, t2) = (ctx.fresh_var(), ctx.fresh_var(), ctx.fresh_var());
        let first = Ty::tuple_row(vec![a.clone()], t1);
        let second = Ty::tuple_row(vec![b, c.clone()], t2);
        assert!(ctx.unify(p.clone(), first, builtin_origin()).is_ok());
        assert!(ctx.unify(p.clone(), second, builtin_origin()).is_ok());

        let triple = Ty::Tuple(vec![Ty::int(), Ty::bool(), Ty::string()]);
        assert!(ctx
            .unify(p.clone(), triple.clone(), builtin_origin())
            .is_ok());
        assert_eq!(ctx.resolve(a), Ty::int());
        assert_eq!(ctx.resolve(c), Ty::bool());
        assert_eq!(ctx.resolve(p), triple);
    }

    #[test]
    fn a_tuple_row_rejects_what_cannot_be_that_tuple() {
        let mut ctx = InferCtx::new();
        let row = |ctx: &mut InferCtx| {
            let (a, b, tail) = (ctx.fresh_var(), ctx.fresh_var(), ctx.fresh_var());
            Ty::tuple_row(vec![a, b], tail)
        };
        let too_short = Ty::Tuple(vec![Ty::int()]);
        let r = row(&mut ctx);
        assert!(ctx.unify(r, too_short, builtin_origin()).is_err());
        let r = row(&mut ctx);
        assert!(ctx.unify(r, Ty::int(), builtin_origin()).is_err());

        // The untyped `Tuple` promises Int elements, as its accessors always have.
        let (a, tail) = (ctx.fresh_var(), ctx.fresh_var());
        let r = Ty::tuple_row(vec![a.clone()], tail);
        let untyped = Ty::Con(TyCon::new("Tuple"));
        assert!(ctx.unify(r, untyped, builtin_origin()).is_ok());
        assert_eq!(ctx.resolve(a), Ty::int());
    }

    #[test]
    fn unify_two_fresh_vars() {
        let mut ctx = InferCtx::new();
        let a = ctx.fresh_var();
        let b = ctx.fresh_var();

        // Unify a ~ b
        assert!(ctx.unify(a.clone(), b.clone(), builtin_origin()).is_ok());

        // After unification, binding one to Int should make both resolve to Int.
        assert!(ctx.unify(a.clone(), Ty::int(), builtin_origin()).is_ok());
        let ra = ctx.resolve(a);
        let rb = ctx.resolve(b);
        assert_eq!(ra, Ty::int());
        assert_eq!(rb, Ty::int());
    }

    #[test]
    fn unify_var_with_concrete() {
        let mut ctx = InferCtx::new();
        let a = ctx.fresh_var();
        let int = Ty::int();

        // Unify a ~ Int
        assert!(ctx.unify(a.clone(), int.clone(), builtin_origin()).is_ok());

        // a should resolve to Int.
        let resolved = ctx.resolve(a);
        assert_eq!(resolved, int);
    }

    #[test]
    fn unify_mismatch() {
        let mut ctx = InferCtx::new();
        let int = Ty::int();
        let string = Ty::string();

        // Unify Int ~ String => should fail.
        let result = ctx.unify(int, string, builtin_origin());
        assert!(result.is_err());
        match result.unwrap_err() {
            TypeError::Mismatch {
                expected, found, ..
            } => {
                assert_eq!(expected, Ty::int());
                assert_eq!(found, Ty::string());
            }
            other => panic!("expected Mismatch, got {:?}", other),
        }
    }

    #[test]
    fn unify_function_return_mismatch() {
        let mut ctx = InferCtx::new();
        let f1 = Ty::fun(vec![Ty::int()], Ty::string());
        let f2 = Ty::fun(vec![Ty::int()], Ty::bool());

        // (Int) -> String ~ (Int) -> Bool => mismatch on return type.
        let result = ctx.unify(f1, f2, builtin_origin());
        assert!(result.is_err());
        match result.unwrap_err() {
            TypeError::Mismatch {
                expected, found, ..
            } => {
                assert_eq!(expected, Ty::string());
                assert_eq!(found, Ty::bool());
            }
            other => panic!("expected Mismatch, got {:?}", other),
        }
    }

    #[test]
    fn occurs_check_infinite_type() {
        let mut ctx = InferCtx::new();
        let a = ctx.fresh_var();

        // Unify a ~ (a) -> Int => should detect infinite type.
        let fun = Ty::fun(vec![a.clone()], Ty::int());
        let result = ctx.unify(a, fun, builtin_origin());
        assert!(result.is_err());
        match result.unwrap_err() {
            TypeError::InfiniteType { .. } => {} // expected
            other => panic!("expected InfiniteType, got {:?}", other),
        }
    }

    #[test]
    fn generalize_and_instantiate() {
        let mut ctx = InferCtx::new();

        // Create a type a -> a at level 1.
        ctx.enter_level();
        let a = ctx.fresh_var();
        let identity_ty = Ty::fun(vec![a.clone()], a);
        ctx.leave_level();

        // Generalize: should quantify over the type variable.
        let scheme = ctx.generalize(identity_ty);
        assert_eq!(scheme.vars.len(), 1, "should have one quantified var");

        // Instantiate twice: should produce different fresh variables.
        let inst1 = ctx.instantiate(&scheme);
        let inst2 = ctx.instantiate(&scheme);

        // The two instantiations should have different type variables.
        match (&inst1, &inst2) {
            (Ty::Fun(p1, _), Ty::Fun(p2, _)) => {
                // The fresh vars in inst1 and inst2 should be different.
                assert_ne!(p1[0], p2[0], "instantiations should produce different vars");
            }
            _ => panic!("expected function types"),
        }
    }

    #[test]
    fn unify_function_arity_mismatch() {
        let mut ctx = InferCtx::new();
        let f1 = Ty::fun(vec![Ty::int()], Ty::string());
        let f2 = Ty::fun(vec![Ty::int(), Ty::int()], Ty::string());

        let result = ctx.unify(f1, f2, builtin_origin());
        assert!(result.is_err());
        match result.unwrap_err() {
            TypeError::ArityMismatch {
                expected: 1,
                found: 2,
                ..
            } => {}
            other => panic!("expected ArityMismatch(1, 2), got {:?}", other),
        }
    }

    #[test]
    fn unify_never_with_anything() {
        let mut ctx = InferCtx::new();

        // Never unifies with any type.
        assert!(ctx.unify(Ty::Never, Ty::int(), builtin_origin()).is_ok());
        assert!(ctx.unify(Ty::string(), Ty::Never, builtin_origin()).is_ok());
    }

    #[test]
    fn unify_tuple_types() {
        let mut ctx = InferCtx::new();
        let t1 = Ty::Tuple(vec![Ty::int(), Ty::string()]);
        let t2 = Ty::Tuple(vec![Ty::int(), Ty::string()]);

        assert!(ctx.unify(t1, t2, builtin_origin()).is_ok());
    }

    #[test]
    fn unify_app_types() {
        let mut ctx = InferCtx::new();
        let opt_int = Ty::option(Ty::int());
        let opt_int2 = Ty::option(Ty::int());

        assert!(ctx.unify(opt_int, opt_int2, builtin_origin()).is_ok());
    }

    #[test]
    fn unify_app_type_mismatch() {
        let mut ctx = InferCtx::new();
        let opt_int = Ty::option(Ty::int());
        let opt_str = Ty::option(Ty::string());

        let result = ctx.unify(opt_int, opt_str, builtin_origin());
        assert!(result.is_err());
    }

    #[test]
    fn con_unifies_with_app_con_empty_args() {
        let mut ctx = InferCtx::new();
        let con = Ty::Con(TyCon::new("Point"));
        let app = Ty::App(Box::new(Ty::Con(TyCon::new("Point"))), vec![]);
        // Con("Point") should unify with App(Con("Point"), [])
        assert!(ctx
            .unify(con.clone(), app.clone(), builtin_origin())
            .is_ok());
        // Symmetric: App should also unify with Con
        assert!(ctx.unify(app, con, builtin_origin()).is_ok());
    }

    #[test]
    fn con_does_not_unify_with_app_con_nonempty_args() {
        let mut ctx = InferCtx::new();
        let con = Ty::Con(TyCon::new("List"));
        let app = Ty::App(Box::new(Ty::Con(TyCon::new("List"))), vec![Ty::int()]);
        // Con("List") should NOT unify with App(Con("List"), [Int]) -- different arities
        assert!(ctx.unify(con, app, builtin_origin()).is_err());
    }

    #[test]
    fn ty_display() {
        assert_eq!(format!("{}", Ty::int()), "Int");
        assert_eq!(
            format!("{}", Ty::fun(vec![Ty::int(), Ty::string()], Ty::bool())),
            "(Int, String) -> Bool"
        );
        assert_eq!(format!("{}", Ty::option(Ty::int())), "Option<Int>");
        assert_eq!(
            format!("{}", Ty::result(Ty::string(), Ty::int())),
            "Result<String, Int>"
        );
        assert_eq!(
            format!("{}", Ty::Tuple(vec![Ty::int(), Ty::string()])),
            "(Int, String)"
        );
        assert_eq!(format!("{}", Ty::Never), "Never");
    }
}
