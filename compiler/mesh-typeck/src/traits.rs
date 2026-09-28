//! Trait registry, impl lookup, and constraint resolution.
//!
//! Manages interface (trait) definitions, impl registrations, and where-clause
//! constraint checking. Also handles compiler-known traits for operator dispatch
//! (Add, Sub, Mul, Div, Mod, Eq, Ord, Not).

use rustc_hash::{FxHashMap, FxHashSet};

use crate::error::ConstraintOrigin;
use crate::ty::{Ty, TyCon, TyVar};
use crate::unify::InferCtx;

/// Whether `ty` mentions `Self`, an associated type of it (`Self.Item`), or a
/// type parameter of a generic interface (`T`, which no declared type in
/// `nominal` is): what only the impl decides. Signature comparison skips
/// such a type.
fn ty_contains_self(ty: &Ty, nominal: &FxHashSet<String>) -> bool {
    match ty {
        Ty::Con(con) => {
            con.name == "Self" || con.name.starts_with("Self.") || is_type_param(&con.name, nominal)
        }
        _ => ty.parts().any(|part| ty_contains_self(part, nominal)),
    }
}

/// An interface's type `ty` as an impl for `impl_type` sees it: its `Self`
/// is the implementing type, and its `Self.Item` the impl's `type Item`.
fn in_impl(ty: &Ty, impl_type: &Ty, assoc_types: &FxHashMap<String, Ty>) -> Ty {
    ty.replace_cons(&mut |con| match con.name.strip_prefix("Self.") {
        Some(assoc) => assoc_types.get(assoc).cloned(),
        None => (con.name == "Self").then(|| impl_type.clone()),
    })
}

/// A method signature within a trait definition.
#[derive(Clone, Debug)]
pub struct TraitMethodSig {
    /// Method name.
    pub name: String,
    /// Parameter types (including self, represented as a placeholder).
    /// The first param is `self` for instance methods.
    pub has_self: bool,
    /// Number of non-self parameters.
    pub param_count: usize,
    /// The return type of the method, if annotated.
    pub return_type: Option<Ty>,
    /// Whether this method has a default body in the interface definition.
    /// When true, impl blocks may omit this method and the default body
    /// will be used instead.
    pub has_default_body: bool,
    /// The non-self parameter types, when every one is annotated.
    pub param_types: Option<Vec<Ty>>,
}

/// An associated type declaration in a trait.
#[derive(Clone, Debug)]
pub struct AssocTypeDef {
    pub name: String,
}

/// A trait (interface) definition.
#[derive(Clone, Debug)]
pub struct TraitDef {
    /// The trait name.
    pub name: String,
    /// Method signatures required by this trait.
    pub methods: Vec<TraitMethodSig>,
    /// Associated type declarations (e.g., `type Item` in interface body).
    pub associated_types: Vec<AssocTypeDef>,
}

/// An impl registration: which type implements which trait.
#[derive(Clone, Debug)]
pub struct ImplDef {
    /// The trait being implemented.
    pub trait_name: String,
    /// Type arguments on the trait (e.g., `[Ty::int()]` for `From<Int>`).
    /// Empty for non-parameterized traits.
    pub trait_type_args: Vec<Ty>,
    /// The concrete type that implements the trait.
    pub impl_type: Ty,
    /// A human-readable name for the implementing type (for error messages).
    pub impl_type_name: String,
    /// Methods provided by this impl, keyed by method name.
    /// Value is (param_count, return_type).
    pub methods: FxHashMap<String, ImplMethodSig>,
    /// Associated type bindings (e.g., `type Item = Int`).
    pub associated_types: FxHashMap<String, Ty>,
}

/// What is wrong with an impl, as its registration finds it; the type
/// checker reports each where the impl says it.
#[derive(Clone, Debug, PartialEq)]
pub enum ImplProblem {
    /// A method the interface declares, without a default, is missing.
    MissingMethod(String),
    /// A method differs from the interface's: its `self`, its parameters
    /// (`expected` and `found` are then function types) or its return type.
    MethodMismatch {
        method_name: String,
        expected: Ty,
        found: Ty,
    },
    /// An associated type the interface declares is not bound.
    MissingAssocType(String),
    /// An associated type is bound that the interface does not declare.
    ExtraAssocType(String),
    /// An earlier impl, for the named type, covers the same types.
    Duplicate(String),
}

/// A method signature in an impl block.
#[derive(Clone, Debug)]
pub struct ImplMethodSig {
    /// Whether the method takes self.
    pub has_self: bool,
    /// Number of non-self parameters.
    pub param_count: usize,
    /// The return type.
    pub return_type: Option<Ty>,
    /// The non-self parameter types, when every one is annotated and known.
    pub param_types: Option<Vec<Ty>>,
}

/// The trait registry: stores all trait definitions and impl registrations.
///
/// This is the central structure for trait resolution. It supports:
/// - Registering trait definitions (from `interface` declarations)
/// - Registering impl blocks (from `impl ... for ... do ... end`)
/// - Looking up whether a type satisfies a trait constraint
/// - Finding method signatures for trait method dispatch
#[derive(Default, Debug)]
pub struct TraitRegistry {
    /// Trait definitions keyed by trait name.
    traits: FxHashMap<String, TraitDef>,
    /// Impl registrations keyed by trait name.
    /// Each trait maps to a list of impls; lookup uses structural type
    /// matching via temporary unification instead of string keys.
    impls: FxHashMap<String, Vec<ImplDef>>,
    /// Declared type names: a single-letter name here (`struct P`) is a
    /// type, not a type parameter, when impl types are matched.
    nominal: FxHashSet<String>,
}

impl TraitRegistry {
    /// Create a new, empty trait registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Every type the registered impls name: implementing types, trait
    /// arguments, associated types and method signatures.
    pub fn impl_types(&self) -> Vec<&Ty> {
        let mut types = Vec::new();
        for imp in self.impls.values().flatten() {
            types.push(&imp.impl_type);
            types.extend(&imp.trait_type_args);
            types.extend(imp.associated_types.values());
            for method in imp.methods.values() {
                types.extend(&method.return_type);
                types.extend(method.param_types.iter().flatten());
            }
        }
        types
    }

    /// Record a declared struct or sum type name.
    pub fn register_nominal(&mut self, name: &str) {
        self.nominal.insert(name.to_string());
    }

    /// Whether an impl for `impl_type` could match a type headed by
    /// `head` (from `impl_head`); the lookups below check this before
    /// unifying, which builds a whole inference context per impl.
    fn may_match(&self, impl_type: &Ty, head: Option<&str>) -> bool {
        match (head, impl_head(impl_type, &self.nominal)) {
            (Some(head), Some(impl_head)) => head == impl_head,
            _ => true,
        }
    }

    /// Whether impls `a` and `b` of one interface would answer for the same
    /// types: theirs unify, and so do their trait arguments.
    fn overlap(&self, a: &ImplDef, b: &ImplDef) -> bool {
        self.may_match(&a.impl_type, impl_head(&b.impl_type, &self.nominal)) && {
            let mut ctx = InferCtx::new();
            std::iter::once((&a.impl_type, &b.impl_type))
                .chain(a.trait_type_args.iter().zip(&b.trait_type_args))
                .all(|(a, b)| {
                    let a = self.freshen(a, &mut ctx);
                    let b = self.freshen(b, &mut ctx);
                    ctx.unify(a, b, ConstraintOrigin::Builtin).is_ok()
                })
        }
    }

    /// `ty` with its type parameters replaced by fresh variables.
    fn freshen(&self, ty: &Ty, ctx: &mut InferCtx) -> Ty {
        freshen_type_params_with_names(ty, ctx, &[], &self.nominal)
    }

    /// The traits that declare a method `method` (`inspect`: Debug), by
    /// name.
    pub fn traits_declaring(&self, method: &str) -> Vec<String> {
        let mut names: Vec<String> = self
            .traits
            .values()
            .filter(|def| def.methods.iter().any(|m| m.name == method))
            .map(|def| def.name.clone())
            .collect();
        names.sort();
        names
    }

    /// Take back the impl of `trait_name` registered for the type named
    /// `type_name`: a derive it cannot have after all, a field of it lacking
    /// the trait (see `check_derive_needs`).
    pub fn remove_impl(&mut self, trait_name: &str, type_name: &str) {
        self.impls
            .get_mut(trait_name)
            .expect("the impl taken back was registered")
            .retain(|imp| imp.impl_type_name != type_name);
    }

    /// Register a trait definition.
    pub fn register_trait(&mut self, def: TraitDef) {
        self.traits.insert(def.name.clone(), def);
    }

    /// Fill in what an impl method's body showed about its types (a return
    /// or parameter type left unannotated) once the impl, registered from
    /// its signatures, has been checked.
    pub fn update_impl_method(
        &mut self,
        trait_name: &str,
        trait_type_args: &[Ty],
        impl_type_name: &str,
        method: &str,
        return_type: Option<Ty>,
        param_types: Option<Vec<Ty>>,
    ) {
        let Some(sig) = self.impls.get_mut(trait_name).and_then(|impls| {
            impls
                .iter_mut()
                .find(|i| {
                    i.impl_type_name == impl_type_name && i.trait_type_args == trait_type_args
                })
                .and_then(|i| i.methods.get_mut(method))
        }) else {
            return;
        };
        if sig.return_type.is_none() {
            sig.return_type = return_type;
        }
        if sig.param_types.is_none() {
            sig.param_types = param_types;
        }
    }

    /// What the default method `method` of the interface `trait_name`,
    /// written without a return type, was found to return (`ret`, in terms
    /// of `Self`): the interface's signature says so, as does every impl's
    /// that says nothing yet (one inheriting the default, or overriding it
    /// unannotated, which is then checked against it).
    pub fn set_default_return(&mut self, trait_name: &str, method: &str, ret: &Ty) {
        let sig = self
            .traits
            .get_mut(trait_name)
            .and_then(|def| def.methods.iter_mut().find(|m| m.name == method))
            .expect("the interface is registered with its default method");
        sig.return_type = Some(ret.clone());
        for impl_def in self.impls.get_mut(trait_name).into_iter().flatten() {
            let (impl_type, assoc_types) = (&impl_def.impl_type, &impl_def.associated_types);
            // Every impl has the method, its own or the default.
            let unsaid = impl_def
                .methods
                .get_mut(method)
                .filter(|sig| sig.return_type.is_none());
            if let Some(impl_sig) = unsaid {
                impl_sig.return_type = Some(in_impl(ret, impl_type, assoc_types));
            }
        }
    }

    /// What the interface `trait_name` declares its method `method` returns,
    /// as an impl for `impl_type` binding `assoc_types` sees it, when that is
    /// a type of its own and not one the impl decides (a type parameter of a
    /// generic interface).
    pub fn declared_return_type(
        &self,
        trait_name: &str,
        method: &str,
        impl_type: &Ty,
        assoc_types: &FxHashMap<String, Ty>,
    ) -> Option<Ty> {
        let trait_def = self.traits.get(trait_name)?;
        let sig = trait_def.methods.iter().find(|m| m.name == method)?;
        let ty = in_impl(sig.return_type.as_ref()?, impl_type, assoc_types);
        (!ty_contains_self(&ty, &self.nominal)).then_some(ty)
    }

    /// Register an impl: `impl Trait for Type`. Returns what is wrong with
    /// it: a method or associated type missing or unlike the interface's,
    /// or an earlier impl for the same types.
    pub fn register_impl(&mut self, impl_def: ImplDef) -> Vec<ImplProblem> {
        self.register_impl_checking_overlap(impl_def, true)
    }

    /// An impl of a module checked earlier. Its own module compared it with
    /// every impl before it, so it is not compared again: each module
    /// re-registers every earlier module's impls, and comparing them all
    /// against each other made checking a project quadratic.
    pub fn register_imported_impl(&mut self, impl_def: ImplDef) {
        let _ = self.register_impl_checking_overlap(impl_def, false);
    }

    fn register_impl_checking_overlap(
        &mut self,
        impl_def: ImplDef,
        check_overlap: bool,
    ) -> Vec<ImplProblem> {
        let mut impl_def = impl_def;
        let mut problems = Vec::new();

        // Look up the trait definition.
        if let Some(trait_def) = self.traits.get(&impl_def.trait_name).cloned() {
            let impl_type = impl_def.impl_type.clone();
            let assoc_types = impl_def.associated_types.clone();
            let in_impl = |ty: &Ty| in_impl(ty, &impl_type, &assoc_types);
            // Check that all required methods are present.
            for method in &trait_def.methods {
                match impl_def.methods.get(&method.name) {
                    None => {
                        if method.has_default_body {
                            // The impl inherits the default method, so method
                            // lookup on the implementing type finds it; codegen
                            // lowers the interface body for this type.
                            impl_def.methods.insert(
                                method.name.clone(),
                                ImplMethodSig {
                                    has_self: method.has_self,
                                    param_count: method.param_count,
                                    return_type: method.return_type.as_ref().map(in_impl),
                                    param_types: None,
                                },
                            );
                        } else {
                            problems.push(ImplProblem::MissingMethod(method.name.clone()));
                        }
                    }
                    Some(impl_method) => {
                        // The impl takes what the interface declares: self or
                        // not, as many parameters, of the declared types.
                        let method_ty = |has_self: bool, count: usize, params: &Option<Vec<Ty>>| {
                            let mut all = Vec::new();
                            if has_self {
                                all.push(Ty::Con(TyCon::new("Self")));
                            }
                            match params {
                                Some(params) => all.extend(params.iter().cloned()),
                                None => all.extend((0..count).map(|_| Ty::Con(TyCon::new("_")))),
                            }
                            Ty::Fun(all, Box::new(Ty::Con(TyCon::new("_"))))
                        };
                        let shape_differs = method.has_self != impl_method.has_self
                            || method.param_count != impl_method.param_count;
                        let types_differ = match (&method.param_types, &impl_method.param_types) {
                            (Some(expected), Some(found)) => {
                                expected.iter().zip(found).any(|(expected, found)| {
                                    let expected = in_impl(expected);
                                    !ty_contains_self(&expected, &self.nominal)
                                        && expected != *found
                                })
                            }
                            _ => false,
                        };
                        if shape_differs || types_differ {
                            problems.push(ImplProblem::MethodMismatch {
                                method_name: method.name.clone(),
                                expected: method_ty(
                                    method.has_self,
                                    method.param_count,
                                    &method.param_types,
                                ),
                                found: method_ty(
                                    impl_method.has_self,
                                    impl_method.param_count,
                                    &impl_method.param_types,
                                ),
                            });
                            continue;
                        }
                        // Check return type compatibility if both are annotated.
                        if let (Some(expected_ret), Some(actual_ret)) =
                            (&method.return_type, &impl_method.return_type)
                        {
                            // Generic, or naming an associated type the impl
                            // does not bind (reported below): not comparable.
                            let expected_ret = in_impl(expected_ret);
                            let expected_involves_self =
                                ty_contains_self(&expected_ret, &self.nominal);
                            if !expected_involves_self && expected_ret != *actual_ret {
                                problems.push(ImplProblem::MethodMismatch {
                                    method_name: method.name.clone(),
                                    expected: expected_ret,
                                    found: actual_ret.clone(),
                                });
                            }
                        }
                    }
                }
            }

            // Check for missing associated types.
            for assoc in &trait_def.associated_types {
                if !impl_def.associated_types.contains_key(&assoc.name) {
                    problems.push(ImplProblem::MissingAssocType(assoc.name.clone()));
                }
            }

            // Check for extra associated types.
            for name in impl_def.associated_types.keys() {
                if !trait_def.associated_types.iter().any(|a| &a.name == name) {
                    problems.push(ImplProblem::ExtraAssocType(name.clone()));
                }
            }
        }

        // An impl whose type and trait arguments unify with an earlier one's
        // is a duplicate (`From<Int>` and `From<Float>` for one type are not).
        if let Some(existing) = self
            .impls
            .get(&impl_def.trait_name)
            .into_iter()
            .flatten()
            .filter(|_| check_overlap)
            .find(|existing| self.overlap(existing, &impl_def))
        {
            problems.push(ImplProblem::Duplicate(existing.impl_type_name.clone()));
        }

        // Store the impl (even if it has errors, for method lookup).
        // Capture info needed for synthetic Into / TryInto generation before moving.
        let maybe_synthesize_into =
            impl_def.trait_name == "From" && !impl_def.trait_type_args.is_empty();
        let synth_source_ty = if maybe_synthesize_into {
            Some(impl_def.trait_type_args[0].clone())
        } else {
            None
        };
        let synth_target_ty = if maybe_synthesize_into {
            Some(impl_def.impl_type.clone())
        } else {
            None
        };

        let maybe_synthesize_try_into =
            impl_def.trait_name == "TryFrom" && !impl_def.trait_type_args.is_empty();
        let synth_try_source_ty = if maybe_synthesize_try_into {
            Some(impl_def.trait_type_args[0].clone())
        } else {
            None
        };
        let synth_try_target_ty = if maybe_synthesize_try_into {
            Some(impl_def.impl_type.clone())
        } else {
            None
        };
        // Capture the TryFrom.try_from return type before moving impl_def.
        // The synthetic TryInto.try_into has the same return type (Result<T, E>).
        let synth_try_return_ty = if maybe_synthesize_try_into {
            impl_def
                .methods
                .get("try_from")
                .and_then(|sig| sig.return_type.clone())
        } else {
            None
        };

        self.impls
            .entry(impl_def.trait_name.clone())
            .or_default()
            .push(impl_def);

        // Synthetic Into generation: when `impl From<A> for B` is registered,
        // automatically synthesize `impl Into<B> for A`.
        if let (Some(source_ty), Some(target_ty)) = (synth_source_ty, synth_target_ty) {
            let mut into_methods = FxHashMap::default();
            into_methods.insert(
                "into".to_string(),
                ImplMethodSig {
                    has_self: true,
                    param_count: 0,
                    return_type: Some(target_ty.clone()),
                    param_types: None,
                },
            );
            let source_name = format!("{}", source_ty);
            let into_impl = ImplDef {
                trait_name: "Into".to_string(),
                trait_type_args: vec![target_ty],
                impl_type: source_ty,
                impl_type_name: source_name,
                methods: into_methods,
                associated_types: FxHashMap::default(),
            };
            // Insert directly to avoid infinite recursion (don't call register_impl).
            self.impls
                .entry("Into".to_string())
                .or_default()
                .push(into_impl);
        }

        // Synthetic TryInto generation (mirrors From -> Into pattern):
        // when `impl TryFrom<A> for B` is registered,
        // automatically synthesize `impl TryInto<B> for A`.
        if let (Some(try_source_ty), Some(try_target_ty)) =
            (synth_try_source_ty, synth_try_target_ty)
        {
            let mut try_into_methods = FxHashMap::default();
            try_into_methods.insert(
                "try_into".to_string(),
                ImplMethodSig {
                    has_self: true,
                    param_count: 0,
                    // Mirror the TryFrom.try_from return type (Result<T, E>) so that
                    // resolve_trait_method returns Some(ret) instead of None, allowing
                    // type-checking of 42.try_into() calls to proceed.
                    return_type: synth_try_return_ty,
                    param_types: None,
                },
            );
            let try_source_name = format!("{}", try_source_ty);
            let try_into_impl = ImplDef {
                trait_name: "TryInto".to_string(),
                trait_type_args: vec![try_target_ty],
                impl_type: try_source_ty,
                impl_type_name: try_source_name,
                methods: try_into_methods,
                associated_types: FxHashMap::default(),
            };
            // Insert directly to avoid infinite recursion (same pattern as Into synthesis).
            self.impls
                .entry("TryInto".to_string())
                .or_default()
                .push(try_into_impl);
        }

        problems
    }

    /// Check whether a concrete type satisfies a trait constraint.
    ///
    /// Uses structural type matching via temporary unification: the impl's
    /// stored type is freshened (type parameters replaced with fresh vars)
    /// and then unified against the query type in a throwaway InferCtx.
    pub fn has_impl(&self, trait_name: &str, ty: &Ty) -> bool {
        self.find_impl(trait_name, ty).is_some()
    }

    /// Find the impl for a given trait and type.
    ///
    /// Uses structural matching via temporary unification to find the first
    /// impl whose type unifies with the query type.
    pub fn find_impl(&self, trait_name: &str, ty: &Ty) -> Option<&ImplDef> {
        let head = impl_head(ty, &self.nominal);
        self.impls
            .get(trait_name)?
            .iter()
            .find(|impl_def| self.applies(impl_def, head, ty).is_some())
    }

    /// Whether `impl_def` is an impl for values of type `ty` (headed by
    /// `head`, see `impl_head`): its type unifies with `ty`, and what `ty`
    /// holds has the trait when the impl needs it (`elements_have_it`). The
    /// context the two unified in reads the impl's types as they are for
    /// `ty`.
    fn applies(&self, impl_def: &ImplDef, head: Option<&str>, ty: &Ty) -> Option<InferCtx> {
        if !self.may_match(&impl_def.impl_type, head) {
            return None;
        }
        let mut ctx = InferCtx::new();
        let query = import_vars(ty, &mut ctx, &mut FxHashMap::default());
        let freshened = self.freshen(&impl_def.impl_type, &mut ctx);
        let unifies = ctx
            .unify(freshened, query, ConstraintOrigin::Builtin)
            .is_ok();
        (unifies && self.elements_have_it(impl_def, ty)).then_some(ctx)
    }

    /// Whether what `ty` holds has the trait a built-in structural impl
    /// needs of it: an `Option`, `Result`, collection or tuple is equal,
    /// ordered or shown by its elements (a map by its values), so
    /// `Some(f) == Some(g)` asks for functions' `Eq`. An element not known
    /// yet may still get it.
    fn elements_have_it(&self, impl_def: &ImplDef, ty: &Ty) -> bool {
        let structural = matches!(
            impl_def.impl_type_name.as_str(),
            "Option" | "Result" | "List" | "Map" | "Set" | "Tuple"
        );
        let trait_name = impl_def.trait_name.as_str();
        if !structural || !matches!(trait_name, "Eq" | "Ord" | "Display" | "Debug") {
            return true;
        }
        let elements = match ty {
            Ty::Tuple(elements) => elements.as_slice(),
            Ty::App(_, args) if impl_def.impl_type_name == "Map" => args.get(1..).unwrap_or(&[]),
            Ty::App(_, args) => args.as_slice(),
            _ => &[],
        };
        elements.iter().all(|element| {
            matches!(element, Ty::Var(_))
                || self.has_impl(trait_name, element)
                // An element is shown by its Display, or else its Debug.
                || (matches!(trait_name, "Display" | "Debug") && self.can_show(element))
        })
    }

    /// Whether a value of type `ty` can be shown: by its Display, or else
    /// its Debug, as an element or a derived function's field is.
    pub fn can_show(&self, ty: &Ty) -> bool {
        self.has_impl("Display", ty) || self.has_impl("Debug", ty)
    }

    /// The impl of `trait_name` for `impl_ty` whose trait arguments are
    /// `trait_type_args` (`From<Int>`, of `From<Int>` and `From<Float>`).
    pub fn find_impl_with_type_args(
        &self,
        trait_name: &str,
        trait_type_args: &[Ty],
        impl_ty: &Ty,
    ) -> Option<&ImplDef> {
        let head = impl_head(impl_ty, &self.nominal);
        self.impls.get(trait_name)?.iter().find(|impl_def| {
            impl_def.trait_type_args.len() == trait_type_args.len()
                && self.may_match(&impl_def.impl_type, head)
                && {
                    let mut ctx = InferCtx::new();
                    let mut imported = FxHashMap::default();
                    std::iter::once((&impl_def.impl_type, impl_ty))
                        .chain(impl_def.trait_type_args.iter().zip(trait_type_args))
                        .all(|(stored, query)| {
                            let stored = self.freshen(stored, &mut ctx);
                            let query = import_vars(query, &mut ctx, &mut imported);
                            ctx.unify(stored, query, ConstraintOrigin::Builtin).is_ok()
                        })
                }
        })
    }

    /// Check whether a concrete type has an impl with specific trait type args.
    pub fn has_impl_with_type_args(
        &self,
        trait_name: &str,
        trait_type_args: &[Ty],
        impl_ty: &Ty,
    ) -> bool {
        self.find_impl_with_type_args(trait_name, trait_type_args, impl_ty)
            .is_some()
    }

    /// Look up a trait definition by name.
    pub fn get_trait(&self, name: &str) -> Option<&TraitDef> {
        self.traits.get(name)
    }

    /// Return all registered trait impls (flattened across all traits).
    pub fn all_impls(&self) -> impl Iterator<Item = &ImplDef> {
        self.impls.values().flat_map(|v| v.iter())
    }

    /// Look up a trait method's return type, given a concrete type.
    ///
    /// Searches all registered impls across all traits for one that provides
    /// the named method and structurally matches the argument type. If the
    /// method's return type contains freshened type variables, they are
    /// resolved through the temporary InferCtx after unification.
    /// The impls that provide `method_name` for values of type `ty`, each
    /// with its method's return type (as it reads for `ty`). Several impls
    /// of one generic interface (`Convert<Int>`, `Convert<String>`) differ
    /// in what they return, and a call picks one by that.
    pub fn impls_providing(&self, method_name: &str, ty: &Ty) -> Vec<(&ImplDef, Option<Ty>)> {
        let mut found = Vec::new();
        let head = impl_head(ty, &self.nominal);
        for impl_def in self.impls.values().flatten() {
            let Some(method_sig) = impl_def.methods.get(method_name) else {
                continue;
            };
            if let Some(mut ctx) = self.applies(impl_def, head, ty) {
                let ret = method_sig.return_type.clone().map(|ret| ctx.resolve(ret));
                found.push((impl_def, ret));
            }
        }
        found
    }

    /// The impls that provide `method_name` as a static method (without
    /// `self`), for whatever types.
    pub fn impls_with_static_method(&self, method_name: &str) -> Vec<&ImplDef> {
        self.impls
            .values()
            .flatten()
            .filter(|imp| {
                imp.methods
                    .get(method_name)
                    .is_some_and(|sig| !sig.has_self)
            })
            .collect()
    }

    pub fn resolve_trait_method(&self, method_name: &str, arg_ty: &Ty) -> Option<Ty> {
        let head = impl_head(arg_ty, &self.nominal);
        for impl_def in self.impls.values().flatten() {
            let Some(method_sig) = impl_def.methods.get(method_name) else {
                continue;
            };
            if let Some(mut ctx) = self.applies(impl_def, head, arg_ty) {
                // Resolve the return type through the temp context in case it
                // contains freshened vars that were bound during unification.
                return method_sig
                    .return_type
                    .as_ref()
                    .map(|ret_ty| ctx.resolve(ret_ty.clone()));
            }
        }
        None
    }

    /// Find the impl method signature for a given method name and self type.
    ///
    /// Searches all registered impls across all traits for one that provides
    /// the named method and structurally matches the argument type. Returns
    /// a clone of the `ImplMethodSig` if found.
    pub fn find_method_sig(&self, method_name: &str, ty: &Ty) -> Option<ImplMethodSig> {
        let head = impl_head(ty, &self.nominal);
        self.impls.values().flatten().find_map(|impl_def| {
            let method_sig = impl_def.methods.get(method_name)?;
            self.applies(impl_def, head, ty).map(|_| method_sig.clone())
        })
    }

    /// Find all trait names that provide a given method for a given type.
    ///
    /// Iterates all registered impls across all traits, collecting the trait
    /// name for each impl that (a) provides the named method and (b)
    /// structurally matches the given type. Useful for ambiguity diagnostics:
    /// if the returned list has more than one element, the call is ambiguous.
    pub fn find_method_traits(&self, method_name: &str, ty: &Ty) -> Vec<String> {
        let head = impl_head(ty, &self.nominal);
        let mut trait_names: Vec<String> = self
            .impls
            .iter()
            .filter(|(_, impl_list)| {
                impl_list.iter().any(|impl_def| {
                    impl_def.methods.contains_key(method_name)
                        && self.applies(impl_def, head, ty).is_some()
                })
            })
            .map(|(trait_name, _)| trait_name.clone())
            .collect();
        trait_names.sort();
        trait_names
    }

    /// Resolve an associated type for a concrete implementing type.
    ///
    /// Given trait "Iterator", associated type "Item", and concrete type List<Int>,
    /// finds the impl and returns the bound type (e.g., Int).
    pub fn resolve_associated_type(
        &self,
        trait_name: &str,
        assoc_name: &str,
        impl_ty: &Ty,
    ) -> Option<Ty> {
        let impl_def = self.find_impl(trait_name, impl_ty)?;
        impl_def.associated_types.get(assoc_name).cloned()
    }
}

/// Copy a query type into a private unification table.
///
/// Inference variables in `ty` belong to the caller's table; resolving them
/// in another `InferCtx` indexes a table that never allocated them. Each one
/// becomes a fresh variable of `ctx` (the same variable maps to the same fresh
/// one), so an unresolved type unifies with any impl, which is the deferred
/// answer the caller wants before the type is known.
fn import_vars(ty: &Ty, ctx: &mut InferCtx, map: &mut FxHashMap<TyVar, Ty>) -> Ty {
    match ty {
        Ty::Var(var) => map.entry(*var).or_insert_with(|| ctx.fresh_var()).clone(),
        _ => ty.map_parts(|part| import_vars(part, ctx, map)),
    }
}

/// Whether an impl's type constructor `name` stands for a type parameter:
/// a single uppercase letter that is no declared type, or a name starting
/// with `'` (the built-in impls name theirs so, and no declared type can
/// collide with them).
fn is_type_param(name: &str, nominal: &FxHashSet<String>) -> bool {
    (name.len() == 1 && name.as_bytes()[0].is_ascii_uppercase() && !nominal.contains(name))
        || name.starts_with('\'')
}

/// What a type is headed by, as far as unification can tell heads apart,
/// or `None` when it could be anything (a type parameter, a variable):
/// types with different heads never unify. Constructors that unify despite
/// their names (see `InferCtx::unify`) share one: every iterator handle and
/// `Iter` is `Ptr`, and the untyped `Tuple` is a tuple, of any length since a
/// tuple row matches several.
fn impl_head<'a>(ty: &'a Ty, nominal: &FxHashSet<String>) -> Option<&'a str> {
    match ty {
        Ty::Con(c) => {
            let name = c.name.as_str();
            if is_type_param(name, nominal) {
                return None;
            }
            Some(match name {
                "Tuple" => "(,)",
                "Iter" | "Ptr" => "Ptr",
                name if name.ends_with("Iterator") => "Ptr",
                name => name,
            })
        }
        Ty::App(con, _) => impl_head(con, nominal),
        Ty::Tuple(_) => Some("(,)"),
        Ty::Fun(..) => Some("->"),
        Ty::Var(_) | Ty::Never => None,
    }
}

/// `ty` with each type parameter (see `is_type_param`, and the explicit
/// `type_param_names` such as "Item") replaced by a fresh inference
/// variable, the same one for each use of a name.
fn freshen_type_params_with_names(
    ty: &Ty,
    ctx: &mut InferCtx,
    type_param_names: &[String],
    nominal: &FxHashSet<String>,
) -> Ty {
    let mut param_map: FxHashMap<String, Ty> = FxHashMap::default();
    ty.replace_cons(&mut |c| {
        (is_type_param(&c.name, nominal) || type_param_names.contains(&c.name)).then(|| {
            param_map
                .entry(c.name.clone())
                .or_insert_with(|| ctx.fresh_var())
                .clone()
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ty::TyCon;

    fn make_display_trait() -> TraitDef {
        TraitDef {
            name: "Display".to_string(),
            methods: vec![TraitMethodSig {
                name: "to_string".to_string(),
                has_self: true,
                param_count: 0,
                return_type: Some(Ty::string()),
                has_default_body: false,
                param_types: None,
            }],
            associated_types: vec![],
        }
    }

    fn make_printable_trait() -> TraitDef {
        TraitDef {
            name: "Printable".to_string(),
            methods: vec![TraitMethodSig {
                name: "to_string".to_string(),
                has_self: true,
                param_count: 0,
                return_type: Some(Ty::string()),
                has_default_body: false,
                param_types: None,
            }],
            associated_types: vec![],
        }
    }

    fn display_method_sig() -> FxHashMap<String, ImplMethodSig> {
        let mut methods = FxHashMap::default();
        methods.insert(
            "to_string".to_string(),
            ImplMethodSig {
                has_self: true,
                param_count: 0,
                return_type: Some(Ty::string()),
                param_types: None,
            },
        );
        methods
    }

    #[test]
    fn register_and_find_trait() {
        let mut registry = TraitRegistry::new();
        registry.register_trait(make_printable_trait());

        assert!(registry.get_trait("Printable").is_some());
        assert!(registry.get_trait("NonExistent").is_none());
    }

    #[test]
    fn register_impl_and_lookup() {
        let mut registry = TraitRegistry::new();
        registry.register_trait(make_printable_trait());

        let errors = registry.register_impl(ImplDef {
            trait_name: "Printable".to_string(),
            trait_type_args: vec![],
            impl_type: Ty::int(),
            impl_type_name: "Int".to_string(),
            methods: display_method_sig(),
            associated_types: FxHashMap::default(),
        });

        assert!(errors.is_empty());
        assert!(registry.has_impl("Printable", &Ty::int()));
        assert!(!registry.has_impl("Printable", &Ty::float()));
    }

    #[test]
    fn missing_method_error() {
        let mut registry = TraitRegistry::new();
        registry.register_trait(make_printable_trait());

        let errors = registry.register_impl(ImplDef {
            trait_name: "Printable".to_string(),
            trait_type_args: vec![],
            impl_type: Ty::int(),
            impl_type_name: "Int".to_string(),
            methods: FxHashMap::default(), // no methods
            associated_types: FxHashMap::default(),
        });

        assert!(matches!(&errors[..], [ImplProblem::MissingMethod(_)]));
    }

    // ── New tests for structural matching ────────────────────────────

    #[test]
    fn structural_match_generic_impl() {
        // Register `impl Display for List<T>` -- T is a type parameter.
        let mut registry = TraitRegistry::new();
        registry.register_trait(make_display_trait());

        let list_of_t = Ty::App(
            Box::new(Ty::Con(TyCon::new("List"))),
            vec![Ty::Con(TyCon::new("T"))],
        );
        let errors = registry.register_impl(ImplDef {
            trait_name: "Display".to_string(),
            trait_type_args: vec![],
            impl_type: list_of_t,
            impl_type_name: "List<T>".to_string(),
            methods: display_method_sig(),
            associated_types: FxHashMap::default(),
        });
        assert!(errors.is_empty());

        // Query with List<Int> -- should match via structural unification.
        assert!(registry.has_impl("Display", &Ty::list(Ty::int())));

        // Query with List<String> -- should also match.
        assert!(registry.has_impl("Display", &Ty::list(Ty::string())));

        // Query with List<List<Int>> -- should also match (T unifies with List<Int>).
        assert!(registry.has_impl("Display", &Ty::list(Ty::list(Ty::int()))));
    }

    #[test]
    fn structural_match_no_false_positive() {
        // Register only `impl Display for List<T>`.
        let mut registry = TraitRegistry::new();
        registry.register_trait(make_display_trait());

        let list_of_t = Ty::App(
            Box::new(Ty::Con(TyCon::new("List"))),
            vec![Ty::Con(TyCon::new("T"))],
        );
        let _ = registry.register_impl(ImplDef {
            trait_name: "Display".to_string(),
            trait_type_args: vec![],
            impl_type: list_of_t,
            impl_type_name: "List<T>".to_string(),
            methods: display_method_sig(),
            associated_types: FxHashMap::default(),
        });

        // Bare Int should NOT match List<T>.
        assert!(!registry.has_impl("Display", &Ty::int()));

        // Bare String should NOT match List<T>.
        assert!(!registry.has_impl("Display", &Ty::string()));

        // Option<Int> should NOT match List<T> (different constructor).
        assert!(!registry.has_impl("Display", &Ty::option(Ty::int())));
    }

    #[test]
    fn simple_type_still_works() {
        // Regression test: simple type impls (Int, Float) still resolve.
        let mut registry = TraitRegistry::new();

        registry.register_trait(TraitDef {
            name: "Add".to_string(),
            methods: vec![TraitMethodSig {
                name: "add".to_string(),
                has_self: true,
                param_count: 1,
                return_type: None,
                has_default_body: false,
                param_types: None,
            }],
            associated_types: vec![],
        });

        let mut add_methods = FxHashMap::default();
        add_methods.insert(
            "add".to_string(),
            ImplMethodSig {
                has_self: true,
                param_count: 1,
                return_type: Some(Ty::int()),
                param_types: None,
            },
        );
        let _ = registry.register_impl(ImplDef {
            trait_name: "Add".to_string(),
            trait_type_args: vec![],
            impl_type: Ty::int(),
            impl_type_name: "Int".to_string(),
            methods: add_methods,
            associated_types: FxHashMap::default(),
        });

        let mut add_float_methods = FxHashMap::default();
        add_float_methods.insert(
            "add".to_string(),
            ImplMethodSig {
                has_self: true,
                param_count: 1,
                return_type: Some(Ty::float()),
                param_types: None,
            },
        );
        let _ = registry.register_impl(ImplDef {
            trait_name: "Add".to_string(),
            trait_type_args: vec![],
            impl_type: Ty::float(),
            impl_type_name: "Float".to_string(),
            methods: add_float_methods,
            associated_types: FxHashMap::default(),
        });

        // Int has Add, Float has Add, String does not.
        assert!(registry.has_impl("Add", &Ty::int()));
        assert!(registry.has_impl("Add", &Ty::float()));
        assert!(!registry.has_impl("Add", &Ty::string()));

        // find_impl returns the correct impl.
        let int_impl = registry.find_impl("Add", &Ty::int()).unwrap();
        assert_eq!(int_impl.impl_type_name, "Int");

        let float_impl = registry.find_impl("Add", &Ty::float()).unwrap();
        assert_eq!(float_impl.impl_type_name, "Float");

        assert!(registry.find_impl("Add", &Ty::string()).is_none());
    }

    #[test]
    fn resolve_trait_method_structural() {
        let mut registry = TraitRegistry::new();
        registry.register_trait(make_display_trait());

        let list_of_t = Ty::App(
            Box::new(Ty::Con(TyCon::new("List"))),
            vec![Ty::Con(TyCon::new("T"))],
        );
        let _ = registry.register_impl(ImplDef {
            trait_name: "Display".to_string(),
            trait_type_args: vec![],
            impl_type: list_of_t,
            impl_type_name: "List<T>".to_string(),
            methods: display_method_sig(),
            associated_types: FxHashMap::default(),
        });

        // Should find to_string for List<Int>.
        let ret = registry.resolve_trait_method("to_string", &Ty::list(Ty::int()));
        assert_eq!(ret, Some(Ty::string()));

        // Should NOT find to_string for bare Int (no impl registered).
        let ret = registry.resolve_trait_method("to_string", &Ty::int());
        assert_eq!(ret, None);
    }

    #[test]
    fn find_impl_structural_generic() {
        let mut registry = TraitRegistry::new();
        registry.register_trait(make_display_trait());

        let list_of_t = Ty::App(
            Box::new(Ty::Con(TyCon::new("List"))),
            vec![Ty::Con(TyCon::new("T"))],
        );
        let _ = registry.register_impl(ImplDef {
            trait_name: "Display".to_string(),
            trait_type_args: vec![],
            impl_type: list_of_t,
            impl_type_name: "List<T>".to_string(),
            methods: display_method_sig(),
            associated_types: FxHashMap::default(),
        });

        // find_impl should return the generic impl when queried with List<Int>.
        let found = registry.find_impl("Display", &Ty::list(Ty::int()));
        assert!(found.is_some());
        assert_eq!(found.unwrap().impl_type_name, "List<T>");

        // find_impl should return None for non-matching types.
        assert!(registry.find_impl("Display", &Ty::int()).is_none());
    }

    // ── Tests for duplicate impl detection (18-02) ───────────────────

    #[test]
    fn duplicate_impl_detected() {
        let mut registry = TraitRegistry::new();
        registry.register_trait(make_printable_trait());

        // First impl: Printable for Int -- should succeed.
        let errors = registry.register_impl(ImplDef {
            trait_name: "Printable".to_string(),
            trait_type_args: vec![],
            impl_type: Ty::int(),
            impl_type_name: "Int".to_string(),
            methods: display_method_sig(),
            associated_types: FxHashMap::default(),
        });
        assert!(errors.is_empty());

        // Second impl: Printable for Int -- should produce DuplicateImpl error.
        let errors = registry.register_impl(ImplDef {
            trait_name: "Printable".to_string(),
            trait_type_args: vec![],
            impl_type: Ty::int(),
            impl_type_name: "Int".to_string(),
            methods: display_method_sig(),
            associated_types: FxHashMap::default(),
        });
        assert_eq!(errors, [ImplProblem::Duplicate("Int".to_string())]);
    }

    #[test]
    fn no_false_duplicate_for_different_types() {
        let mut registry = TraitRegistry::new();
        registry.register_trait(make_printable_trait());

        // impl Printable for Int.
        let errors = registry.register_impl(ImplDef {
            trait_name: "Printable".to_string(),
            trait_type_args: vec![],
            impl_type: Ty::int(),
            impl_type_name: "Int".to_string(),
            methods: display_method_sig(),
            associated_types: FxHashMap::default(),
        });
        assert!(errors.is_empty());

        // impl Printable for String -- different type, no duplicate.
        let errors = registry.register_impl(ImplDef {
            trait_name: "Printable".to_string(),
            trait_type_args: vec![],
            impl_type: Ty::string(),
            impl_type_name: "String".to_string(),
            methods: display_method_sig(),
            associated_types: FxHashMap::default(),
        });
        assert!(errors.is_empty());
    }

    #[test]
    fn find_method_traits_single() {
        let mut registry = TraitRegistry::new();
        registry.register_trait(make_printable_trait());

        let _ = registry.register_impl(ImplDef {
            trait_name: "Printable".to_string(),
            trait_type_args: vec![],
            impl_type: Ty::int(),
            impl_type_name: "Int".to_string(),
            methods: display_method_sig(),
            associated_types: FxHashMap::default(),
        });

        let traits = registry.find_method_traits("to_string", &Ty::int());
        assert_eq!(traits, vec!["Printable".to_string()]);
    }

    #[test]
    fn find_method_traits_multiple() {
        let mut registry = TraitRegistry::new();
        registry.register_trait(make_printable_trait());
        registry.register_trait(TraitDef {
            name: "Displayable".to_string(),
            methods: vec![TraitMethodSig {
                name: "to_string".to_string(),
                has_self: true,
                param_count: 0,
                return_type: Some(Ty::string()),
                has_default_body: false,
                param_types: None,
            }],
            associated_types: vec![],
        });

        let _ = registry.register_impl(ImplDef {
            trait_name: "Printable".to_string(),
            trait_type_args: vec![],
            impl_type: Ty::int(),
            impl_type_name: "Int".to_string(),
            methods: display_method_sig(),
            associated_types: FxHashMap::default(),
        });
        let _ = registry.register_impl(ImplDef {
            trait_name: "Displayable".to_string(),
            trait_type_args: vec![],
            impl_type: Ty::int(),
            impl_type_name: "Int".to_string(),
            methods: display_method_sig(),
            associated_types: FxHashMap::default(),
        });

        let traits = registry.find_method_traits("to_string", &Ty::int());
        // find_method_traits now returns sorted results (deterministic)
        assert_eq!(
            traits,
            vec!["Displayable".to_string(), "Printable".to_string()]
        );
    }

    // ── Unified dispatch path test (18-03) ──────────────────────────

    #[test]
    fn unified_dispatch_builtin_and_user_types() {
        // Proves that built-in types (Int) and user-defined types (MyStruct)
        // both resolve through the exact same TraitRegistry API path.
        // No special-case dispatch for built-in vs. user types.
        let mut registry = TraitRegistry::new();
        registry.register_trait(TraitDef {
            name: "Add".to_string(),
            methods: vec![TraitMethodSig {
                name: "add".to_string(),
                has_self: true,
                param_count: 1,
                return_type: None,
                has_default_body: false,
                param_types: None,
            }],
            associated_types: vec![],
        });

        // Built-in impl: Add for Int (same path as builtins.rs registration).
        let mut int_methods = FxHashMap::default();
        int_methods.insert(
            "add".to_string(),
            ImplMethodSig {
                has_self: true,
                param_count: 1,
                return_type: Some(Ty::int()),
                param_types: None,
            },
        );
        let errors = registry.register_impl(ImplDef {
            trait_name: "Add".to_string(),
            trait_type_args: vec![],
            impl_type: Ty::int(),
            impl_type_name: "Int".to_string(),
            methods: int_methods,
            associated_types: FxHashMap::default(),
        });
        assert!(errors.is_empty());

        // User-defined impl: Add for MyStruct (simulated as Ty::Con("MyStruct")).
        let my_struct = Ty::Con(TyCon::new("MyStruct"));
        let mut struct_methods = FxHashMap::default();
        struct_methods.insert(
            "add".to_string(),
            ImplMethodSig {
                has_self: true,
                param_count: 1,
                return_type: Some(my_struct.clone()),
                param_types: None,
            },
        );
        let errors = registry.register_impl(ImplDef {
            trait_name: "Add".to_string(),
            trait_type_args: vec![],
            impl_type: my_struct.clone(),
            impl_type_name: "MyStruct".to_string(),
            methods: struct_methods,
            associated_types: FxHashMap::default(),
        });
        assert!(errors.is_empty());

        // Both resolve through the same has_impl path.
        assert!(registry.has_impl("Add", &Ty::int()));
        assert!(registry.has_impl("Add", &my_struct));

        // Both resolve through the same find_impl path.
        let int_impl = registry.find_impl("Add", &Ty::int()).unwrap();
        assert_eq!(int_impl.impl_type_name, "Int");
        let struct_impl = registry.find_impl("Add", &my_struct).unwrap();
        assert_eq!(struct_impl.impl_type_name, "MyStruct");

        // Method resolution works for both through the same resolve_trait_method path.
        let int_ret = registry.resolve_trait_method("add", &Ty::int());
        assert_eq!(int_ret, Some(Ty::int()));

        let struct_ret = registry.resolve_trait_method("add", &my_struct);
        assert_eq!(struct_ret, Some(my_struct));
    }

    #[test]
    fn lookups_skip_impls_only_when_heads_cannot_unify() {
        // Lookups and the duplicate check skip an impl headed by another
        // constructor; constructors that unify despite their names must
        // still meet.
        let mut registry = TraitRegistry::new();
        registry.register_trait(make_display_trait());
        let display = |impl_type: Ty, name: &str| ImplDef {
            trait_name: "Display".to_string(),
            trait_type_args: vec![],
            impl_type,
            impl_type_name: name.to_string(),
            methods: display_method_sig(),
            associated_types: FxHashMap::default(),
        };
        let con = |name: &str| Ty::Con(TyCon::new(name));
        for (ty, name) in [
            (Ty::string(), "String"),
            (Ty::list(con("T")), "List"),
            (Ty::Tuple(vec![con("A"), con("B")]), "Tuple"),
            (con("Ptr"), "Ptr"),
        ] {
            assert!(
                registry.register_impl(display(ty, name)).is_empty(),
                "{name}"
            );
        }
        let found = |ty: Ty| {
            registry
                .find_impl("Display", &ty)
                .map(|imp| imp.impl_type_name.clone())
        };
        assert_eq!(found(con("Json")), None);
        assert_eq!(found(Ty::list(Ty::string())).as_deref(), Some("List"));
        assert_eq!(
            found(Ty::Tuple(vec![Ty::string(), Ty::string()])).as_deref(),
            Some("Tuple")
        );
        // A structural impl needs its elements to have the trait: no Int
        // is shown here, so no list or tuple holding one is.
        assert_eq!(found(Ty::list(Ty::int())), None);
        assert_eq!(found(Ty::Tuple(vec![Ty::int(), Ty::string()])), None);
        assert_eq!(found(con("Tuple")).as_deref(), Some("Tuple"));
        assert_eq!(found(con("ListIterator")).as_deref(), Some("Ptr"));
        assert_eq!(found(Ty::int()), None);

        // A second impl for the same type is still a duplicate; one imported
        // from a module checked earlier is not compared again.
        let errors = registry.register_impl(display(Ty::string(), "String"));
        assert_eq!(errors, [ImplProblem::Duplicate("String".to_string())]);
        registry.register_imported_impl(display(Ty::string(), "String"));
    }
}
