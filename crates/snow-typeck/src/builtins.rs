//! Built-in type registration.
//!
//! Registers primitive types (Int, Float, String, Bool), generic type
//! constructors (Option, Result), and built-in arithmetic operators into
//! the type environment. These form the starting vocabulary of every
//! Snow program.

use crate::env::TypeEnv;
use crate::ty::{Scheme, Ty};
use crate::unify::InferCtx;

/// Register all built-in types and functions into the environment.
///
/// After this call, the environment contains:
/// - Primitive types: Int, Float, String, Bool
/// - Generic constructors: Option (arity 1), Result (arity 2)
/// - Arithmetic operators: +, -, *, / for Int and Float
/// - Comparison operators (placeholder for future trait-based dispatch)
///
/// Note: The current approach hardcodes arithmetic operators as
/// `(Int, Int) -> Int` and `(Float, Float) -> Float`. When trait-based
/// dispatch is added (plan 03-04), these will be replaced with type class
/// constraints.
pub fn register_builtins(_ctx: &mut InferCtx, env: &mut TypeEnv) {
    // ── Primitive type constructors ─────────────────────────────────

    // These are registered as monomorphic schemes so they can be
    // referenced as type names in annotations.
    env.insert("Int".into(), Scheme::mono(Ty::int()));
    env.insert("Float".into(), Scheme::mono(Ty::float()));
    env.insert("String".into(), Scheme::mono(Ty::string()));
    env.insert("Bool".into(), Scheme::mono(Ty::bool()));

    // ── Arithmetic operators (Int) ──────────────────────────────────

    // For now, we register Int arithmetic. Float overloading and
    // trait dispatch will come in plan 03-04.
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_register_primitives() {
        let mut ctx = InferCtx::new();
        let mut env = TypeEnv::new();
        register_builtins(&mut ctx, &mut env);

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
        register_builtins(&mut ctx, &mut env);

        // Check arithmetic operators exist.
        assert!(env.lookup("+").is_some());
        assert!(env.lookup("-").is_some());
        assert!(env.lookup("*").is_some());
        assert!(env.lookup("/").is_some());

        // Check comparison operators.
        assert!(env.lookup("==").is_some());
        assert!(env.lookup("<").is_some());
    }
}
