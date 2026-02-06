//! Unification engine for Hindley-Milner type inference.
//!
//! Implements the core unification algorithm using `ena`'s union-find table.
//! Supports occurs check (infinite type detection), level-based generalization,
//! and scheme instantiation.

use ena::unify::InPlaceUnificationTable;
use rustc_hash::FxHashMap;

use crate::error::{ConstraintOrigin, TypeError};
use crate::ty::{Scheme, Ty, TyVar};

/// The inference context -- owns the unification table, level state, and errors.
///
/// All type inference happens through this context. It creates fresh type
/// variables, unifies types, tracks levels for generalization, and collects
/// errors.
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
}

impl InferCtx {
    /// Create a new, empty inference context.
    pub fn new() -> Self {
        InferCtx {
            table: InPlaceUnificationTable::new(),
            current_level: 0,
            var_levels: Vec::new(),
            errors: Vec::new(),
        }
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
                Ty::App(con, args)
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
                params.iter().any(|p| self.occurs_in(var, p))
                    || self.occurs_in(var, ret)
            }
            Ty::App(con, args) => {
                self.occurs_in(var, con)
                    || args.iter().any(|a| self.occurs_in(var, a))
            }
            Ty::Tuple(elems) => elems.iter().any(|e| self.occurs_in(var, e)),
            Ty::Never => false,
        }
    }

    // ── Unification ─────────────────────────────────────────────────────

    /// Unify two types, making them equal.
    ///
    /// This is the core of HM inference. Both types are first resolved
    /// through the union-find table, then structurally compared. If they
    /// differ, a type error is recorded.
    pub fn unify(
        &mut self,
        a: Ty,
        b: Ty,
        origin: ConstraintOrigin,
    ) -> Result<(), TypeError> {
        let a = self.resolve(a);
        let b = self.resolve(b);

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
                    let err = TypeError::InfiniteType {
                        var: v,
                        ty,
                        origin,
                    };
                    self.errors.push(err.clone());
                    Err(err)
                } else {
                    self.table
                        .unify_var_value(v, Some(ty))
                        .expect("binding a var to a concrete type after occurs check should not fail");
                    Ok(())
                }
            }

            // Concrete constructor meets concrete constructor -- names must match.
            (Ty::Con(c1), Ty::Con(c2)) => {
                if c1 == c2 {
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
                        let level = self
                            .var_levels
                            .get(v.0 as usize)
                            .copied()
                            .unwrap_or(0);
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

        let substitution: FxHashMap<TyVar, Ty> = scheme
            .vars
            .iter()
            .map(|v| (*v, self.fresh_var()))
            .collect();

        self.apply_substitution(&scheme.ty, &substitution)
    }

    /// Apply a substitution map to a type.
    fn apply_substitution(
        &mut self,
        ty: &Ty,
        subst: &FxHashMap<TyVar, Ty>,
    ) -> Ty {
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

    fn builtin_origin() -> ConstraintOrigin {
        ConstraintOrigin::Builtin
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
        assert!(ctx
            .unify(Ty::string(), Ty::Never, builtin_origin())
            .is_ok());
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
