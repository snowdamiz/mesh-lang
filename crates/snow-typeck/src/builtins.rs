//! Built-in type registration.
//!
//! Registers primitive types (Int, Float, String, Bool), generic type
//! constructors (Option, Result), built-in arithmetic operators, and
//! compiler-known traits (Add, Sub, Mul, Div, Mod, Eq, Ord, Not) into
//! the type environment and trait registry.

use rustc_hash::FxHashMap;

use crate::env::TypeEnv;
use crate::traits::{ImplDef, ImplMethodSig, TraitDef, TraitMethodSig, TraitRegistry};
use crate::ty::{Scheme, Ty};
use crate::unify::InferCtx;

/// Register all built-in types and functions into the environment.
///
/// After this call, the environment contains:
/// - Primitive types: Int, Float, String, Bool
/// - Generic constructors: Option (arity 1), Result (arity 2)
/// - Arithmetic operators: +, -, *, / for Int and Float
/// - Comparison operators (returns Bool)
/// - Logical operators (and, or, not)
///
/// And the trait registry contains compiler-known traits:
/// - Add, Sub, Mul, Div, Mod (numeric operations)
/// - Eq (equality comparison)
/// - Ord (ordering comparison)
/// - Not (logical negation)
pub fn register_builtins(
    _ctx: &mut InferCtx,
    env: &mut TypeEnv,
    trait_registry: &mut TraitRegistry,
) {
    // ── Primitive type constructors ─────────────────────────────────

    // These are registered as monomorphic schemes so they can be
    // referenced as type names in annotations.
    env.insert("Int".into(), Scheme::mono(Ty::int()));
    env.insert("Float".into(), Scheme::mono(Ty::float()));
    env.insert("String".into(), Scheme::mono(Ty::string()));
    env.insert("Bool".into(), Scheme::mono(Ty::bool()));

    // ── Actor type constructor ────────────────────────────────────
    //
    // Pid is a type constructor with arity 1: Pid<M> where M is the
    // message type. Untyped Pid (arity 0) is also valid as an escape
    // hatch. We register the bare name so `Pid` resolves in annotations.
    env.insert("Pid".into(), Scheme::mono(Ty::untyped_pid()));

    // ── I/O builtins ──────────────────────────────────────────────

    // println(String) -> () -- prints a string with trailing newline
    env.insert(
        "println".into(),
        Scheme::mono(Ty::fun(vec![Ty::string()], Ty::Tuple(vec![]))),
    );

    // print(String) -> () -- prints a string without trailing newline
    env.insert(
        "print".into(),
        Scheme::mono(Ty::fun(vec![Ty::string()], Ty::Tuple(vec![]))),
    );

    // ── Compiler-known traits ──────────────────────────────────────

    register_compiler_known_traits(trait_registry);

    // ── Arithmetic operators (Int) ──────────────────────────────────

    // These remain in the env for backward compatibility. The binary
    // operator inference also uses trait dispatch for type checking.
    let int_binop = Scheme::mono(Ty::fun(vec![Ty::int(), Ty::int()], Ty::int()));
    env.insert("+".into(), int_binop.clone());
    env.insert("-".into(), int_binop.clone());
    env.insert("*".into(), int_binop.clone());
    env.insert("/".into(), int_binop);

    // ── Float arithmetic ────────────────────────────────────────────

    let float_binop = Scheme::mono(Ty::fun(
        vec![Ty::float(), Ty::float()],
        Ty::float(),
    ));
    env.insert("+.".into(), float_binop.clone());
    env.insert("-.".into(), float_binop.clone());
    env.insert("*.".into(), float_binop.clone());
    env.insert("/.".into(), float_binop);

    // ── String concatenation ────────────────────────────────────────

    env.insert(
        "<>".into(),
        Scheme::mono(Ty::fun(vec![Ty::string(), Ty::string()], Ty::string())),
    );

    // ── Comparison (returns Bool) ───────────────────────────────────

    let int_cmp = Scheme::mono(Ty::fun(vec![Ty::int(), Ty::int()], Ty::bool()));
    env.insert("==".into(), int_cmp.clone());
    env.insert("!=".into(), int_cmp.clone());
    env.insert("<".into(), int_cmp.clone());
    env.insert(">".into(), int_cmp.clone());
    env.insert("<=".into(), int_cmp.clone());
    env.insert(">=".into(), int_cmp);

    // ── Logical operators ───────────────────────────────────────────

    let bool_binop = Scheme::mono(Ty::fun(vec![Ty::bool(), Ty::bool()], Ty::bool()));
    env.insert("and".into(), bool_binop.clone());
    env.insert("or".into(), bool_binop);
    env.insert(
        "not".into(),
        Scheme::mono(Ty::fun(vec![Ty::bool()], Ty::bool())),
    );

    // ── Standard library: String operations (Phase 8) ────────────────

    env.insert(
        "string_length".into(),
        Scheme::mono(Ty::fun(vec![Ty::string()], Ty::int())),
    );
    env.insert(
        "string_slice".into(),
        Scheme::mono(Ty::fun(vec![Ty::string(), Ty::int(), Ty::int()], Ty::string())),
    );
    env.insert(
        "string_contains".into(),
        Scheme::mono(Ty::fun(vec![Ty::string(), Ty::string()], Ty::bool())),
    );
    env.insert(
        "string_starts_with".into(),
        Scheme::mono(Ty::fun(vec![Ty::string(), Ty::string()], Ty::bool())),
    );
    env.insert(
        "string_ends_with".into(),
        Scheme::mono(Ty::fun(vec![Ty::string(), Ty::string()], Ty::bool())),
    );
    env.insert(
        "string_trim".into(),
        Scheme::mono(Ty::fun(vec![Ty::string()], Ty::string())),
    );
    env.insert(
        "string_to_upper".into(),
        Scheme::mono(Ty::fun(vec![Ty::string()], Ty::string())),
    );
    env.insert(
        "string_to_lower".into(),
        Scheme::mono(Ty::fun(vec![Ty::string()], Ty::string())),
    );
    env.insert(
        "string_replace".into(),
        Scheme::mono(Ty::fun(
            vec![Ty::string(), Ty::string(), Ty::string()],
            Ty::string(),
        )),
    );

    // ── Standard library: IO functions (Phase 8) ─────────────────────

    env.insert(
        "io_read_line".into(),
        Scheme::mono(Ty::fun(vec![], Ty::result(Ty::string(), Ty::string()))),
    );
    env.insert(
        "io_eprintln".into(),
        Scheme::mono(Ty::fun(vec![Ty::string()], Ty::Tuple(vec![]))),
    );

    // ── Standard library: Env functions (Phase 8) ────────────────────

    env.insert(
        "env_get".into(),
        Scheme::mono(Ty::fun(vec![Ty::string()], Ty::option(Ty::string()))),
    );
}

/// Register compiler-known traits and their built-in implementations.
///
/// These traits back the arithmetic and comparison operators. When the
/// inference engine encounters `a + b`, it checks that the resolved type
/// of `a` has an impl for `Add`.
fn register_compiler_known_traits(registry: &mut TraitRegistry) {
    // ── Arithmetic traits ──────────────────────────────────────────

    let arithmetic_traits = ["Add", "Sub", "Mul", "Div", "Mod"];
    for trait_name in &arithmetic_traits {
        registry.register_trait(TraitDef {
            name: trait_name.to_string(),
            methods: vec![TraitMethodSig {
                name: trait_name.to_lowercase(),
                has_self: true,
                param_count: 1,
                return_type: None, // return type is Self (the implementing type)
            }],
        });

        // Register impls for Int and Float.
        for (ty, ty_name) in &[(Ty::int(), "Int"), (Ty::float(), "Float")] {
            let mut methods = FxHashMap::default();
            methods.insert(
                trait_name.to_lowercase(),
                ImplMethodSig {
                    has_self: true,
                    param_count: 1,
                    return_type: Some(ty.clone()),
                },
            );
            let _ = registry.register_impl(ImplDef {
                trait_name: trait_name.to_string(),
                impl_type: ty.clone(),
                impl_type_name: ty_name.to_string(),
                methods,
            });
        }
    }

    // ── Eq trait ────────────────────────────────────────────────────

    registry.register_trait(TraitDef {
        name: "Eq".to_string(),
        methods: vec![TraitMethodSig {
            name: "eq".to_string(),
            has_self: true,
            param_count: 1,
            return_type: Some(Ty::bool()),
        }],
    });

    // Eq impls for Int, Float, String, Bool.
    for (ty, ty_name) in &[
        (Ty::int(), "Int"),
        (Ty::float(), "Float"),
        (Ty::string(), "String"),
        (Ty::bool(), "Bool"),
    ] {
        let mut methods = FxHashMap::default();
        methods.insert(
            "eq".to_string(),
            ImplMethodSig {
                has_self: true,
                param_count: 1,
                return_type: Some(Ty::bool()),
            },
        );
        let _ = registry.register_impl(ImplDef {
            trait_name: "Eq".to_string(),
            impl_type: ty.clone(),
            impl_type_name: ty_name.to_string(),
            methods,
        });
    }

    // ── Ord trait ───────────────────────────────────────────────────

    registry.register_trait(TraitDef {
        name: "Ord".to_string(),
        methods: vec![TraitMethodSig {
            name: "cmp".to_string(),
            has_self: true,
            param_count: 1,
            return_type: Some(Ty::bool()),
        }],
    });

    // Ord impls for Int, Float.
    for (ty, ty_name) in &[(Ty::int(), "Int"), (Ty::float(), "Float")] {
        let mut methods = FxHashMap::default();
        methods.insert(
            "cmp".to_string(),
            ImplMethodSig {
                has_self: true,
                param_count: 1,
                return_type: Some(Ty::bool()),
            },
        );
        let _ = registry.register_impl(ImplDef {
            trait_name: "Ord".to_string(),
            impl_type: ty.clone(),
            impl_type_name: ty_name.to_string(),
            methods,
        });
    }

    // ── Not trait ───────────────────────────────────────────────────

    registry.register_trait(TraitDef {
        name: "Not".to_string(),
        methods: vec![TraitMethodSig {
            name: "not".to_string(),
            has_self: true,
            param_count: 0,
            return_type: Some(Ty::bool()),
        }],
    });

    let mut not_methods = FxHashMap::default();
    not_methods.insert(
        "not".to_string(),
        ImplMethodSig {
            has_self: true,
            param_count: 0,
            return_type: Some(Ty::bool()),
        },
    );
    let _ = registry.register_impl(ImplDef {
        trait_name: "Not".to_string(),
        impl_type: Ty::bool(),
        impl_type_name: "Bool".to_string(),
        methods: not_methods,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_register_primitives() {
        let mut ctx = InferCtx::new();
        let mut env = TypeEnv::new();
        let mut trait_registry = TraitRegistry::new();
        register_builtins(&mut ctx, &mut env, &mut trait_registry);

        // Check primitive types exist.
        assert!(env.lookup("Int").is_some());
        assert!(env.lookup("Float").is_some());
        assert!(env.lookup("String").is_some());
        assert!(env.lookup("Bool").is_some());
    }

    #[test]
    fn builtins_register_operators() {
        let mut ctx = InferCtx::new();
        let mut env = TypeEnv::new();
        let mut trait_registry = TraitRegistry::new();
        register_builtins(&mut ctx, &mut env, &mut trait_registry);

        // Check arithmetic operators exist.
        assert!(env.lookup("+").is_some());
        assert!(env.lookup("-").is_some());
        assert!(env.lookup("*").is_some());
        assert!(env.lookup("/").is_some());

        // Check comparison operators.
        assert!(env.lookup("==").is_some());
        assert!(env.lookup("<").is_some());
    }

    #[test]
    fn builtins_register_compiler_known_traits() {
        let mut ctx = InferCtx::new();
        let mut env = TypeEnv::new();
        let mut trait_registry = TraitRegistry::new();
        register_builtins(&mut ctx, &mut env, &mut trait_registry);

        // Check arithmetic traits.
        assert!(trait_registry.get_trait("Add").is_some());
        assert!(trait_registry.get_trait("Sub").is_some());
        assert!(trait_registry.get_trait("Eq").is_some());
        assert!(trait_registry.get_trait("Ord").is_some());

        // Check impls.
        assert!(trait_registry.has_impl("Add", &Ty::int()));
        assert!(trait_registry.has_impl("Add", &Ty::float()));
        assert!(!trait_registry.has_impl("Add", &Ty::string()));
        assert!(trait_registry.has_impl("Eq", &Ty::string()));
    }

    #[test]
    fn builtins_register_stdlib_functions() {
        let mut ctx = InferCtx::new();
        let mut env = TypeEnv::new();
        let mut trait_registry = TraitRegistry::new();
        register_builtins(&mut ctx, &mut env, &mut trait_registry);

        // String operations
        assert!(env.lookup("string_length").is_some());
        assert!(env.lookup("string_slice").is_some());
        assert!(env.lookup("string_contains").is_some());
        assert!(env.lookup("string_starts_with").is_some());
        assert!(env.lookup("string_ends_with").is_some());
        assert!(env.lookup("string_trim").is_some());
        assert!(env.lookup("string_to_upper").is_some());
        assert!(env.lookup("string_to_lower").is_some());
        assert!(env.lookup("string_replace").is_some());

        // IO functions
        assert!(env.lookup("io_read_line").is_some());
        assert!(env.lookup("io_eprintln").is_some());

        // Env functions
        assert!(env.lookup("env_get").is_some());
    }
}
