//! Trait registry, impl lookup, and constraint resolution.
//!
//! Manages interface (trait) definitions, impl registrations, and where-clause
//! constraint checking. Also handles compiler-known traits for operator dispatch
//! (Add, Sub, Mul, Div, Mod, Eq, Ord, Not).

use rustc_hash::FxHashMap;

use crate::error::{ConstraintOrigin, TypeError};
use crate::ty::Ty;

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
    /// Impl registrations: (trait_name, type_key) -> ImplDef.
    /// The type_key is a string representation of the implementing type
    /// for lookup purposes.
    impls: FxHashMap<(String, String), ImplDef>,
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
        let type_key = type_to_key(&impl_def.impl_type);
        self.impls
            .insert((impl_def.trait_name.clone(), type_key), impl_def);

        errors
    }

    /// Check whether a concrete type satisfies a trait constraint.
    pub fn has_impl(&self, trait_name: &str, ty: &Ty) -> bool {
        let type_key = type_to_key(ty);
        self.impls.contains_key(&(trait_name.to_string(), type_key))
    }

    /// Find the impl for a given trait and type.
    pub fn find_impl(&self, trait_name: &str, ty: &Ty) -> Option<&ImplDef> {
        let type_key = type_to_key(ty);
        self.impls.get(&(trait_name.to_string(), type_key))
    }

    /// Look up a trait definition by name.
    pub fn get_trait(&self, name: &str) -> Option<&TraitDef> {
        self.traits.get(name)
    }

    /// Look up a trait method's return type, given a concrete type.
    ///
    /// Used for dispatching trait method calls on concrete types.
    pub fn resolve_trait_method(
        &self,
        method_name: &str,
        arg_ty: &Ty,
    ) -> Option<Ty> {
        // Search all impls for one that provides this method and matches the type.
        let type_key = type_to_key(arg_ty);
        for ((_, impl_type_key), impl_def) in &self.impls {
            if *impl_type_key == type_key {
                if let Some(method_sig) = impl_def.methods.get(method_name) {
                    return method_sig.return_type.clone();
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

/// Convert a type to a string key for impl lookup.
///
/// This is a simple key for exact-match lookup. Type variables are not
/// supported as impl targets (only concrete types).
fn type_to_key(ty: &Ty) -> String {
    match ty {
        Ty::Con(c) => c.name.clone(),
        Ty::App(con, args) => {
            let con_key = type_to_key(con);
            let arg_keys: Vec<String> = args.iter().map(type_to_key).collect();
            format!("{}<{}>", con_key, arg_keys.join(", "))
        }
        Ty::Tuple(elems) => {
            let elem_keys: Vec<String> = elems.iter().map(type_to_key).collect();
            format!("({})", elem_keys.join(", "))
        }
        _ => format!("{}", ty),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_find_trait() {
        let mut registry = TraitRegistry::new();
        registry.register_trait(TraitDef {
            name: "Printable".to_string(),
            methods: vec![TraitMethodSig {
                name: "to_string".to_string(),
                has_self: true,
                param_count: 0,
                return_type: Some(Ty::string()),
            }],
        });

        assert!(registry.get_trait("Printable").is_some());
        assert!(registry.get_trait("NonExistent").is_none());
    }

    #[test]
    fn register_impl_and_lookup() {
        let mut registry = TraitRegistry::new();
        registry.register_trait(TraitDef {
            name: "Printable".to_string(),
            methods: vec![TraitMethodSig {
                name: "to_string".to_string(),
                has_self: true,
                param_count: 0,
                return_type: Some(Ty::string()),
            }],
        });

        let mut methods = FxHashMap::default();
        methods.insert(
            "to_string".to_string(),
            ImplMethodSig {
                has_self: true,
                param_count: 0,
                return_type: Some(Ty::string()),
            },
        );

        let errors = registry.register_impl(ImplDef {
            trait_name: "Printable".to_string(),
            impl_type: Ty::int(),
            impl_type_name: "Int".to_string(),
            methods,
        });

        assert!(errors.is_empty());
        assert!(registry.has_impl("Printable", &Ty::int()));
        assert!(!registry.has_impl("Printable", &Ty::float()));
    }

    #[test]
    fn missing_method_error() {
        let mut registry = TraitRegistry::new();
        registry.register_trait(TraitDef {
            name: "Printable".to_string(),
            methods: vec![TraitMethodSig {
                name: "to_string".to_string(),
                has_self: true,
                param_count: 0,
                return_type: Some(Ty::string()),
            }],
        });

        let errors = registry.register_impl(ImplDef {
            trait_name: "Printable".to_string(),
            impl_type: Ty::int(),
            impl_type_name: "Int".to_string(),
            methods: FxHashMap::default(), // no methods
        });

        assert_eq!(errors.len(), 1);
        assert!(matches!(&errors[0], TypeError::MissingTraitMethod { .. }));
    }
}
