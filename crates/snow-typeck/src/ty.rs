//! Type representation for the Snow type system.
//!
//! Defines the core `Ty` enum, type constructors (`TyCon`), type variables
//! (`TyVar`), and polymorphic type schemes (`Scheme`). These form the
//! foundation of Hindley-Milner type inference.

use std::fmt;

/// A type variable, identified by a `u32` index into the unification table.
///
/// Type variables are created during inference and unified with concrete types
/// or other variables. The `ena` crate handles the union-find mechanics.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct TyVar(pub u32);

/// A type constructor -- a named type like `Int`, `String`, `Option`, etc.
///
/// Type constructors are identified by name. They can be nullary (e.g. `Int`)
/// or parameterized (e.g. `Option` with arity 1, `Result` with arity 2).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TyCon {
    pub name: String,
}

impl TyCon {
    pub fn new(name: impl Into<String>) -> Self {
        TyCon { name: name.into() }
    }
}

impl fmt::Display for TyCon {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name)
    }
}

/// A Snow type.
///
/// Represents all possible types in the Snow type system:
/// - `Var`: an inference variable (to be resolved by unification)
/// - `Con`: a concrete type constructor (Int, String, Bool, ...)
/// - `Fun`: a function type (params -> return)
/// - `App`: a type constructor application (Option<Int>, Result<T, E>)
/// - `Tuple`: a tuple type (Int, String)
/// - `Never`: the bottom type (never returns)
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Ty {
    /// A type variable (unresolved during inference).
    Var(TyVar),
    /// A concrete type constructor.
    Con(TyCon),
    /// A function type: `(param_types) -> return_type`.
    Fun(Vec<Ty>, Box<Ty>),
    /// A type constructor applied to arguments: `Option<Int>`, `Result<T, E>`.
    App(Box<Ty>, Vec<Ty>),
    /// A tuple type: `(Int, String, Bool)`.
    Tuple(Vec<Ty>),
    /// The bottom/never type -- the type of expressions that never return.
    Never,
}

impl Ty {
    /// Create an `Int` type.
    pub fn int() -> Ty {
        Ty::Con(TyCon::new("Int"))
    }

    /// Create a `Float` type.
    pub fn float() -> Ty {
        Ty::Con(TyCon::new("Float"))
    }

    /// Create a `String` type.
    pub fn string() -> Ty {
        Ty::Con(TyCon::new("String"))
    }

    /// Create a `Bool` type.
    pub fn bool() -> Ty {
        Ty::Con(TyCon::new("Bool"))
    }

    /// Create an `Option<T>` type.
    pub fn option(inner: Ty) -> Ty {
        Ty::App(Box::new(Ty::Con(TyCon::new("Option"))), vec![inner])
    }

    /// Create a `Result<T, E>` type.
    pub fn result(ok: Ty, err: Ty) -> Ty {
        Ty::App(Box::new(Ty::Con(TyCon::new("Result"))), vec![ok, err])
    }

    /// Create a function type.
    pub fn fun(params: Vec<Ty>, ret: Ty) -> Ty {
        Ty::Fun(params, Box::new(ret))
    }
}

impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Ty::Var(v) => write!(f, "?{}", v.0),
            Ty::Con(c) => write!(f, "{}", c),
            Ty::Fun(params, ret) => {
                write!(f, "(")?;
                for (i, p) in params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", p)?;
                }
                write!(f, ") -> {}", ret)
            }
            Ty::App(con, args) => {
                write!(f, "{}", con)?;
                if !args.is_empty() {
                    write!(f, "<")?;
                    for (i, a) in args.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", a)?;
                    }
                    write!(f, ">")?;
                }
                Ok(())
            }
            Ty::Tuple(elems) => {
                write!(f, "(")?;
                for (i, e) in elems.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", e)?;
                }
                write!(f, ")")
            }
            Ty::Never => write!(f, "Never"),
        }
    }
}

/// A polymorphic type scheme: a type with universally quantified variables.
///
/// For example, the type of `identity` is `forall a. a -> a`, represented as
/// `Scheme { vars: [a], ty: Fun([Var(a)], Var(a)) }`.
#[derive(Clone, Debug)]
pub struct Scheme {
    /// The quantified (generic) type variables.
    pub vars: Vec<TyVar>,
    /// The underlying type (may reference vars).
    pub ty: Ty,
}

impl Scheme {
    /// Create a monomorphic scheme (no quantified variables).
    pub fn mono(ty: Ty) -> Self {
        Scheme {
            vars: Vec::new(),
            ty,
        }
    }
}

// ── ena trait implementations ──────────────────────────────────────────

impl ena::unify::UnifyKey for TyVar {
    type Value = Option<Ty>;

    fn index(&self) -> u32 {
        self.0
    }

    fn from_index(u: u32) -> Self {
        TyVar(u)
    }

    fn tag() -> &'static str {
        "TyVar"
    }
}

impl ena::unify::EqUnifyValue for Ty {}
