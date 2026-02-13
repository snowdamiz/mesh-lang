//! Type representation for the Mesh type system.
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
///
/// The `display_prefix` field is used ONLY for display in error messages
/// (e.g., "Geometry.Point"). It is intentionally excluded from `PartialEq`
/// and `Hash` to preserve type identity semantics.
#[derive(Clone, Debug)]
pub struct TyCon {
    pub name: String,
    /// Module origin for display in error messages (e.g., "Geometry").
    /// NOT used for type identity or codegen. Only affects Display output.
    pub display_prefix: Option<String>,
}

impl PartialEq for TyCon {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name // display_prefix intentionally excluded
    }
}

impl Eq for TyCon {}

impl std::hash::Hash for TyCon {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.name.hash(state); // display_prefix intentionally excluded
    }
}

impl TyCon {
    pub fn new(name: impl Into<String>) -> Self {
        TyCon { name: name.into(), display_prefix: None }
    }

    pub fn with_module(name: impl Into<String>, module: impl Into<String>) -> Self {
        TyCon { name: name.into(), display_prefix: Some(module.into()) }
    }
}

impl fmt::Display for TyCon {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(prefix) = &self.display_prefix {
            write!(f, "{}.{}", prefix, self.name)
        } else {
            write!(f, "{}", self.name)
        }
    }
}

/// A Mesh type.
///
/// Represents all possible types in the Mesh type system:
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

    /// Create a typed `Pid<M>` type (actor identity with message type M).
    pub fn pid(msg_type: Ty) -> Ty {
        Ty::App(Box::new(Ty::Con(TyCon::new("Pid"))), vec![msg_type])
    }

    /// Create an untyped `Pid` (escape hatch -- accepts any message at runtime).
    pub fn untyped_pid() -> Ty {
        Ty::Con(TyCon::new("Pid"))
    }

    /// Create a `List<T>` type.
    pub fn list(inner: Ty) -> Ty {
        Ty::App(Box::new(Ty::Con(TyCon::new("List"))), vec![inner])
    }

    /// Create an unparameterized `List` type (opaque pointer).
    pub fn list_untyped() -> Ty {
        Ty::Con(TyCon::new("List"))
    }

    /// Create a `Map<K, V>` type.
    pub fn map(key: Ty, value: Ty) -> Ty {
        Ty::App(Box::new(Ty::Con(TyCon::new("Map"))), vec![key, value])
    }

    /// Create an unparameterized `Map` type (opaque pointer).
    pub fn map_untyped() -> Ty {
        Ty::Con(TyCon::new("Map"))
    }

    /// Create a `Set<T>` type.
    pub fn set(inner: Ty) -> Ty {
        Ty::App(Box::new(Ty::Con(TyCon::new("Set"))), vec![inner])
    }

    /// Create an unparameterized `Set` type (opaque pointer).
    pub fn set_untyped() -> Ty {
        Ty::Con(TyCon::new("Set"))
    }

    /// Create a `Range` type.
    pub fn range() -> Ty {
        Ty::Con(TyCon::new("Range"))
    }

    /// Create a `Queue` type.
    pub fn queue() -> Ty {
        Ty::Con(TyCon::new("Queue"))
    }

    /// Create a named struct type with optional type arguments.
    /// Non-generic structs: `Ty::struct_ty("Point", vec![])` -> `Point`
    /// Generic structs: `Ty::struct_ty("Pair", vec![Ty::int(), Ty::string()])` -> `Pair<Int, String>`
    pub fn struct_ty(name: &str, args: Vec<Ty>) -> Ty {
        if args.is_empty() {
            Ty::App(Box::new(Ty::Con(TyCon::new(name))), vec![])
        } else {
            Ty::App(Box::new(Ty::Con(TyCon::new(name))), args)
        }
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
