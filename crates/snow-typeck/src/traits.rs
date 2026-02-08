//! Trait registry, impl lookup, and constraint resolution.
//!
//! Manages interface (trait) definitions, impl registrations, and where-clause
//! constraint checking. Also handles compiler-known traits for operator dispatch
//! (Add, Sub, Mul, Div, Mod, Eq, Ord, Not).

use rustc_hash::FxHashMap;

use crate::error::{ConstraintOrigin, TypeError};
use crate::ty::Ty;
use crate::unify::InferCtx;

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
}

/// A trait (interface) definition.
#[derive(Clone, Debug)]
pub struct TraitDef {
    /// The trait name.
    pub name: String,
    /// Method signatures required by this trait.
    pub methods: Vec<TraitMethodSig>,
}

/// An impl registration: which type implements which trait.
#[derive(Clone, Debug)]
pub struct ImplDef {
    /// The trait being implemented.
    pub trait_name: String,
    /// The concrete type that implements the trait.
    pub impl_type: Ty,
    /// A human-readable name for the implementing type (for error messages).
    pub impl_type_name: String,
    /// Methods provided by this impl, keyed by method name.
    /// Value is (param_count, return_type).
    pub methods: FxHashMap<String, ImplMethodSig>,
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
}

impl TraitRegistry {
    /// Create a new, empty trait registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a trait definition.
    pub fn register_trait(&mut self, def: TraitDef) {
        self.traits.insert(def.name.clone(), def);
    }

    /// Register an impl: `impl Trait for Type`.
    ///
    /// Validates that all required methods are present and have compatible
    /// signatures. Returns errors for missing or mismatched methods.
    pub fn register_impl(&mut self, impl_def: ImplDef) -> Vec<TypeError> {
        let mut errors = Vec::new();

        // Look up the trait definition.
        if let Some(trait_def) = self.traits.get(&impl_def.trait_name).cloned() {
            // Check that all required methods are present.
            for method in &trait_def.methods {
                match impl_def.methods.get(&method.name) {
                    None => {
                        errors.push(TypeError::MissingTraitMethod {
                            trait_name: impl_def.trait_name.clone(),
                            method_name: method.name.clone(),
                            impl_ty: impl_def.impl_type_name.clone(),
                        });
                    }
                    Some(impl_method) => {
                        // Check return type compatibility if both are annotated.
                        if let (Some(expected_ret), Some(actual_ret)) =
                            (&method.return_type, &impl_method.return_type)
                        {
                            if expected_ret != actual_ret {
                                errors.push(TypeError::TraitMethodSignatureMismatch {
                                    trait_name: impl_def.trait_name.clone(),
                                    method_name: method.name.clone(),
                                    expected: expected_ret.clone(),
                                    found: actual_ret.clone(),
                                });
                            }
                        }
                    }
                }
            }
        }

        // Store the impl (even if it has errors, for method lookup).
        let trait_name = impl_def.trait_name.clone();
        self.impls.entry(trait_name).or_default().push(impl_def);

        errors
    }

    /// Check whether a concrete type satisfies a trait constraint.
    ///
    /// Uses structural type matching via temporary unification: the impl's
    /// stored type is freshened (type parameters replaced with fresh vars)
    /// and then unified against the query type in a throwaway InferCtx.
    pub fn has_impl(&self, trait_name: &str, ty: &Ty) -> bool {
        let impls = match self.impls.get(trait_name) {
            Some(v) => v,
            None => return false,
        };
        for impl_def in impls {
            let mut ctx = InferCtx::new();
            let freshened = freshen_type_params(&impl_def.impl_type, &mut ctx);
            if ctx
                .unify(freshened, ty.clone(), ConstraintOrigin::Builtin)
                .is_ok()
            {
                return true;
            }
        }
        false
    }

    /// Find the impl for a given trait and type.
    ///
    /// Uses structural matching via temporary unification to find the first
    /// impl whose type unifies with the query type.
    pub fn find_impl(&self, trait_name: &str, ty: &Ty) -> Option<&ImplDef> {
        let impls = self.impls.get(trait_name)?;
        for impl_def in impls {
            let mut ctx = InferCtx::new();
            let freshened = freshen_type_params(&impl_def.impl_type, &mut ctx);
            if ctx
                .unify(freshened, ty.clone(), ConstraintOrigin::Builtin)
                .is_ok()
            {
                return Some(impl_def);
            }
        }
        None
    }

    /// Look up a trait definition by name.
    pub fn get_trait(&self, name: &str) -> Option<&TraitDef> {
        self.traits.get(name)
    }

    /// Look up a trait method's return type, given a concrete type.
    ///
    /// Searches all registered impls across all traits for one that provides
    /// the named method and structurally matches the argument type. If the
    /// method's return type contains freshened type variables, they are
    /// resolved through the temporary InferCtx after unification.
    pub fn resolve_trait_method(
        &self,
        method_name: &str,
        arg_ty: &Ty,
    ) -> Option<Ty> {
        for impl_list in self.impls.values() {
            for impl_def in impl_list {
                if let Some(method_sig) = impl_def.methods.get(method_name) {
                    let mut ctx = InferCtx::new();
                    let freshened = freshen_type_params(&impl_def.impl_type, &mut ctx);
                    if ctx
                        .unify(freshened, arg_ty.clone(), ConstraintOrigin::Builtin)
                        .is_ok()
                    {
                        // Resolve the return type through the temp context
                        // in case it contains freshened vars that were bound
                        // during unification.
                        return match &method_sig.return_type {
                            Some(ret_ty) => Some(ctx.resolve(ret_ty.clone())),
                            None => None,
                        };
                    }
                }
            }
        }
        None
    }

    /// Check where-clause constraints: verify that a concrete type satisfies
    /// all required trait bounds.
    pub fn check_where_constraints(
        &self,
        constraints: &[(String, String)], // (type_param_name, trait_name)
        type_args: &FxHashMap<String, Ty>,
        origin: ConstraintOrigin,
    ) -> Vec<TypeError> {
        let mut errors = Vec::new();
        for (param_name, trait_name) in constraints {
            if let Some(concrete_ty) = type_args.get(param_name) {
                if !self.has_impl(trait_name, concrete_ty) {
                    errors.push(TypeError::TraitNotSatisfied {
                        ty: concrete_ty.clone(),
                        trait_name: trait_name.clone(),
                        origin: origin.clone(),
                    });
                }
            }
        }
        errors
    }
}

/// Replace type parameters in a type with fresh inference variables.
///
/// A `Ty::Con` whose name is a single uppercase ASCII letter (A-Z) is
/// treated as a type parameter and replaced with a fresh `Ty::Var`.
/// A local map ensures the same parameter name maps to the same fresh
/// variable within one freshening pass.
///
/// Concrete constructors (Int, Float, String, List, Option, etc.) are
/// never freshened -- only single-uppercase-letter names are.
fn freshen_type_params(ty: &Ty, ctx: &mut InferCtx) -> Ty {
    let mut param_map: FxHashMap<String, Ty> = FxHashMap::default();
    freshen_recursive(ty, ctx, &mut param_map)
}

fn freshen_recursive(
    ty: &Ty,
    ctx: &mut InferCtx,
    param_map: &mut FxHashMap<String, Ty>,
) -> Ty {
    match ty {
        Ty::Con(c) => {
            // A single uppercase ASCII letter is a type parameter.
            if c.name.len() == 1 && c.name.as_bytes()[0].is_ascii_uppercase() {
                param_map
                    .entry(c.name.clone())
                    .or_insert_with(|| ctx.fresh_var())
                    .clone()
            } else {
                ty.clone()
            }
        }
        Ty::App(con, args) => {
            let con_fresh = freshen_recursive(con, ctx, param_map);
            let args_fresh: Vec<Ty> = args
                .iter()
                .map(|a| freshen_recursive(a, ctx, param_map))
                .collect();
            Ty::App(Box::new(con_fresh), args_fresh)
        }
        Ty::Fun(params, ret) => {
            let params_fresh: Vec<Ty> = params
                .iter()
                .map(|p| freshen_recursive(p, ctx, param_map))
                .collect();
            let ret_fresh = freshen_recursive(ret, ctx, param_map);
            Ty::Fun(params_fresh, Box::new(ret_fresh))
        }
        Ty::Tuple(elems) => {
            let elems_fresh: Vec<Ty> = elems
                .iter()
                .map(|e| freshen_recursive(e, ctx, param_map))
                .collect();
            Ty::Tuple(elems_fresh)
        }
        // Ty::Var and Ty::Never are returned as-is.
        _ => ty.clone(),
    }
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
            }],
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
            }],
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
            impl_type: Ty::int(),
            impl_type_name: "Int".to_string(),
            methods: display_method_sig(),
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
            impl_type: Ty::int(),
            impl_type_name: "Int".to_string(),
            methods: FxHashMap::default(), // no methods
        });

        assert_eq!(errors.len(), 1);
        assert!(matches!(&errors[0], TypeError::MissingTraitMethod { .. }));
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
            impl_type: list_of_t,
            impl_type_name: "List<T>".to_string(),
            methods: display_method_sig(),
        });
        assert!(errors.is_empty());

        // Query with List<Int> -- should match via structural unification.
        assert!(registry.has_impl("Display", &Ty::list(Ty::int())));

        // Query with List<String> -- should also match.
        assert!(registry.has_impl("Display", &Ty::list(Ty::string())));

        // Query with List<List<Int>> -- should also match (T unifies with List<Int>).
        assert!(registry.has_impl(
            "Display",
            &Ty::list(Ty::list(Ty::int()))
        ));
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
            impl_type: list_of_t,
            impl_type_name: "List<T>".to_string(),
            methods: display_method_sig(),
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
            }],
        });

        let mut add_methods = FxHashMap::default();
        add_methods.insert(
            "add".to_string(),
            ImplMethodSig {
                has_self: true,
                param_count: 1,
                return_type: Some(Ty::int()),
            },
        );
        let _ = registry.register_impl(ImplDef {
            trait_name: "Add".to_string(),
            impl_type: Ty::int(),
            impl_type_name: "Int".to_string(),
            methods: add_methods,
        });

        let mut add_float_methods = FxHashMap::default();
        add_float_methods.insert(
            "add".to_string(),
            ImplMethodSig {
                has_self: true,
                param_count: 1,
                return_type: Some(Ty::float()),
            },
        );
        let _ = registry.register_impl(ImplDef {
            trait_name: "Add".to_string(),
            impl_type: Ty::float(),
            impl_type_name: "Float".to_string(),
            methods: add_float_methods,
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
            impl_type: list_of_t,
            impl_type_name: "List<T>".to_string(),
            methods: display_method_sig(),
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
            impl_type: list_of_t,
            impl_type_name: "List<T>".to_string(),
            methods: display_method_sig(),
        });

        // find_impl should return the generic impl when queried with List<Int>.
        let found = registry.find_impl("Display", &Ty::list(Ty::int()));
        assert!(found.is_some());
        assert_eq!(found.unwrap().impl_type_name, "List<T>");

        // find_impl should return None for non-matching types.
        assert!(registry.find_impl("Display", &Ty::int()).is_none());
    }
}
