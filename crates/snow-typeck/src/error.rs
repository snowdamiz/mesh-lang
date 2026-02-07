//! Type error types with provenance tracking.
//!
//! Every type error carries a `ConstraintOrigin` that records where the
//! constraint was generated. This enables precise, contextual error messages
//! that point to the exact source location of the mismatch.

use std::fmt;

use rowan::TextRange;

use crate::ty::{Ty, TyVar};

/// The origin of a type constraint -- where in the source code did we
/// decide these two types should be equal?
///
/// Provenance tracking is essential for good error messages. Instead of
/// "expected Int, found String", we can say "argument 2 of `add` expected
/// Int, found String (at line 42)".
#[derive(Clone, Debug)]
pub enum ConstraintOrigin {
    /// From a function argument: `foo(x)` where x's type must match param type.
    FnArg {
        call_site: TextRange,
        param_idx: usize,
    },
    /// From a binary operator: `a + b` where a and b must have compatible types.
    BinOp { op_span: TextRange },
    /// From if/else branches: both branches must have the same type.
    IfBranches {
        if_span: TextRange,
        then_span: TextRange,
        else_span: TextRange,
    },
    /// From a type annotation: `x :: Int` where x must be Int.
    Annotation { annotation_span: TextRange },
    /// From a return expression: return type must match function signature.
    Return {
        return_span: TextRange,
        fn_span: TextRange,
    },
    /// From a let binding: `let x = expr` where inferred type of expr is bound.
    LetBinding { binding_span: TextRange },
    /// From an assignment: `x = expr` where lhs and rhs must match.
    Assignment {
        lhs_span: TextRange,
        rhs_span: TextRange,
    },
    /// Synthetic origin for built-in constraints (e.g. arithmetic operators).
    Builtin,
}

/// A type error encountered during type checking.
///
/// Each variant carries enough information to produce a clear, actionable
/// error message including the source location and expected vs. actual types.
#[derive(Clone, Debug)]
pub enum TypeError {
    /// Two types that should be equal are not.
    Mismatch {
        expected: Ty,
        found: Ty,
        origin: ConstraintOrigin,
    },
    /// A type variable appears in its own definition (infinite type).
    ///
    /// Example: trying to unify `a` with `(a) -> Int` creates an infinite
    /// type `(((((...) -> Int) -> Int) -> Int) -> Int)`.
    InfiniteType {
        var: TyVar,
        ty: Ty,
        origin: ConstraintOrigin,
    },
    /// Function called with wrong number of arguments.
    ArityMismatch {
        expected: usize,
        found: usize,
        origin: ConstraintOrigin,
    },
    /// A variable is used but not defined in scope.
    UnboundVariable { name: String, span: TextRange },
    /// A non-function value is called as a function.
    NotAFunction { ty: Ty, span: TextRange },
    /// A type does not satisfy a required trait constraint.
    TraitNotSatisfied {
        ty: Ty,
        trait_name: String,
        origin: ConstraintOrigin,
    },
    /// An impl block is missing a method required by the trait.
    MissingTraitMethod {
        trait_name: String,
        method_name: String,
        impl_ty: String,
    },
    /// An impl method's signature does not match the trait's method signature.
    TraitMethodSignatureMismatch {
        trait_name: String,
        method_name: String,
        expected: Ty,
        found: Ty,
    },
    /// A struct literal is missing a required field.
    MissingField {
        struct_name: String,
        field_name: String,
        span: TextRange,
    },
    /// A struct literal references an unknown field.
    UnknownField {
        struct_name: String,
        field_name: String,
        span: TextRange,
    },
    /// A field access on a type with no such field.
    NoSuchField {
        ty: Ty,
        field_name: String,
        span: TextRange,
    },
    /// A variant name was used in a pattern but does not exist.
    UnknownVariant {
        name: String,
        span: TextRange,
    },
    /// Or-pattern alternatives bind different sets of variables.
    OrPatternBindingMismatch {
        expected_bindings: Vec<String>,
        found_bindings: Vec<String>,
        span: TextRange,
    },
    /// A match/case expression is not exhaustive.
    NonExhaustiveMatch {
        scrutinee_type: String,
        missing_patterns: Vec<String>,
        span: TextRange,
    },
    /// A match arm is redundant (unreachable given prior arms).
    RedundantArm {
        arm_index: usize,
        span: TextRange,
    },
    /// A guard expression uses disallowed constructs.
    InvalidGuardExpression {
        reason: String,
        span: TextRange,
    },
    /// Sending a message of wrong type to a typed Pid<M>.
    SendTypeMismatch {
        expected: Ty,
        found: Ty,
        span: TextRange,
    },
    /// self() called outside an actor block.
    SelfOutsideActor {
        span: TextRange,
    },
    /// spawn called with a non-function argument.
    SpawnNonFunction {
        found: Ty,
        span: TextRange,
    },
    /// receive used outside an actor block.
    ReceiveOutsideActor {
        span: TextRange,
    },
    /// Child spec start function does not return Pid.
    InvalidChildStart {
        child_name: String,
        found: Ty,
        span: TextRange,
    },
    /// Unknown supervision strategy.
    InvalidStrategy {
        found: String,
        span: TextRange,
    },
    /// Invalid restart type for child spec.
    InvalidRestartType {
        found: String,
        child_name: String,
        span: TextRange,
    },
    /// Invalid shutdown value for child spec.
    InvalidShutdownValue {
        found: String,
        child_name: String,
        span: TextRange,
    },
    /// A catch-all clause appears before the last position in a multi-clause function.
    CatchAllNotLast {
        fn_name: String,
        arity: usize,
        span: TextRange,
    },
    /// A multi-clause function has non-consecutive clauses (same name appears in separate groups).
    NonConsecutiveClauses {
        fn_name: String,
        arity: usize,
        first_span: TextRange,
        second_span: TextRange,
    },
    /// Clauses in a multi-clause function have inconsistent arities.
    ClauseArityMismatch {
        fn_name: String,
        expected_arity: usize,
        found_arity: usize,
        span: TextRange,
    },
    /// Visibility/generics/return type on a non-first clause of a multi-clause function.
    NonFirstClauseAnnotation {
        fn_name: String,
        what: String,
        span: TextRange,
    },
    /// Guard expression type is not Bool.
    GuardTypeMismatch {
        expected: Ty,
        found: Ty,
        span: TextRange,
    },
}

impl fmt::Display for TypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TypeError::Mismatch {
                expected, found, ..
            } => {
                write!(f, "type mismatch: expected `{}`, found `{}`", expected, found)
            }
            TypeError::InfiniteType { var, ty, .. } => {
                write!(
                    f,
                    "infinite type: `?{}` occurs in `{}`",
                    var.0, ty
                )
            }
            TypeError::ArityMismatch {
                expected, found, ..
            } => {
                write!(
                    f,
                    "arity mismatch: expected {} arguments, found {}",
                    expected, found
                )
            }
            TypeError::UnboundVariable { name, .. } => {
                write!(f, "unbound variable `{}`", name)
            }
            TypeError::NotAFunction { ty, .. } => {
                write!(f, "`{}` is not a function", ty)
            }
            TypeError::TraitNotSatisfied {
                ty, trait_name, ..
            } => {
                write!(
                    f,
                    "type `{}` does not satisfy trait `{}`",
                    ty, trait_name
                )
            }
            TypeError::MissingTraitMethod {
                trait_name,
                method_name,
                impl_ty,
            } => {
                write!(
                    f,
                    "impl `{}` for `{}` is missing method `{}`",
                    trait_name, impl_ty, method_name
                )
            }
            TypeError::TraitMethodSignatureMismatch {
                trait_name,
                method_name,
                expected,
                found,
            } => {
                write!(
                    f,
                    "method `{}` in impl `{}` has wrong signature: expected `{}`, found `{}`",
                    method_name, trait_name, expected, found
                )
            }
            TypeError::MissingField {
                struct_name,
                field_name,
                ..
            } => {
                write!(
                    f,
                    "missing field `{}` in struct `{}`",
                    field_name, struct_name
                )
            }
            TypeError::UnknownField {
                struct_name,
                field_name,
                ..
            } => {
                write!(
                    f,
                    "unknown field `{}` in struct `{}`",
                    field_name, struct_name
                )
            }
            TypeError::NoSuchField {
                ty, field_name, ..
            } => {
                write!(f, "type `{}` has no field `{}`", ty, field_name)
            }
            TypeError::UnknownVariant { name, .. } => {
                write!(f, "unknown variant `{}`", name)
            }
            TypeError::OrPatternBindingMismatch {
                expected_bindings,
                found_bindings,
                ..
            } => {
                write!(
                    f,
                    "or-pattern binding mismatch: expected [{}], found [{}]",
                    expected_bindings.join(", "),
                    found_bindings.join(", ")
                )
            }
            TypeError::NonExhaustiveMatch {
                scrutinee_type,
                missing_patterns,
                ..
            } => {
                write!(
                    f,
                    "non-exhaustive match on `{}`: missing patterns [{}]",
                    scrutinee_type,
                    missing_patterns.join(", ")
                )
            }
            TypeError::RedundantArm { arm_index, .. } => {
                write!(f, "redundant match arm at index {}", arm_index)
            }
            TypeError::InvalidGuardExpression { reason, .. } => {
                write!(f, "invalid guard expression: {}", reason)
            }
            TypeError::SendTypeMismatch {
                expected, found, ..
            } => {
                write!(
                    f,
                    "message type mismatch: expected `{}`, found `{}`",
                    expected, found
                )
            }
            TypeError::SelfOutsideActor { .. } => {
                write!(f, "self() used outside actor block")
            }
            TypeError::SpawnNonFunction { found, .. } => {
                write!(f, "cannot spawn non-function: found `{}`", found)
            }
            TypeError::ReceiveOutsideActor { .. } => {
                write!(f, "receive used outside actor block")
            }
            TypeError::InvalidChildStart {
                child_name, found, ..
            } => {
                write!(
                    f,
                    "child `{}` start function must return Pid, found `{}`",
                    child_name, found
                )
            }
            TypeError::InvalidStrategy { found, .. } => {
                write!(
                    f,
                    "unknown supervision strategy `{}`, expected one_for_one, one_for_all, rest_for_one, or simple_one_for_one",
                    found
                )
            }
            TypeError::InvalidRestartType {
                found, child_name, ..
            } => {
                write!(
                    f,
                    "invalid restart type `{}` for child `{}`, expected permanent, transient, or temporary",
                    found, child_name
                )
            }
            TypeError::InvalidShutdownValue {
                found, child_name, ..
            } => {
                write!(
                    f,
                    "invalid shutdown value `{}` for child `{}`, expected a positive integer or brutal_kill",
                    found, child_name
                )
            }
            TypeError::CatchAllNotLast {
                fn_name, arity, ..
            } => {
                write!(
                    f,
                    "catch-all clause must be the last clause of function `{}/{}`; clauses after a catch-all are unreachable",
                    fn_name, arity
                )
            }
            TypeError::NonConsecutiveClauses {
                fn_name, arity, ..
            } => {
                write!(
                    f,
                    "function `{}/{}` already defined; multi-clause functions must have consecutive clauses",
                    fn_name, arity
                )
            }
            TypeError::ClauseArityMismatch {
                fn_name,
                expected_arity,
                found_arity,
                ..
            } => {
                write!(
                    f,
                    "all clauses of `{}` must have the same number of parameters; expected {}, found {}",
                    fn_name, expected_arity, found_arity
                )
            }
            TypeError::NonFirstClauseAnnotation {
                fn_name, what, ..
            } => {
                write!(
                    f,
                    "{} on non-first clause of `{}` will be ignored",
                    what, fn_name
                )
            }
            TypeError::GuardTypeMismatch {
                expected, found, ..
            } => {
                write!(
                    f,
                    "guard expression must return `{}`, found `{}`",
                    expected, found
                )
            }
        }
    }
}
