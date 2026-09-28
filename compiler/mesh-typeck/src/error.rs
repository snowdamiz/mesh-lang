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
    /// `call_site` is the argument's span, or the call's when the constraint
    /// is on the callee or an implicit argument.
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
    /// A constraint the checker derived while inferring the expression at
    /// `span`, with no more specific origin.
    Expr { span: TextRange },
    /// A pattern must match the value it is matched against: the value's
    /// type is expected, the pattern's found.
    Pattern { pattern_span: TextRange },
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
    /// A function of `elements` parameters where one taking an
    /// `elements`-tuple is expected (or the reverse): a closure written
    /// `fn (a, b) -> ...` for `Map.to_list` pairs.
    TupleParameterSplit {
        elements: usize,
        origin: ConstraintOrigin,
    },
    /// Slot pipe position N exceeds the function's total arity.
    ///
    /// Example: `x |5> func(a, b, c)` — func only takes 3 arguments.
    SlotPipeOutOfRange {
        /// The slot position used (1-indexed).
        slot: u32,
        /// Name of the function being called (empty string if unknown).
        fn_name: String,
        /// Total arity of the function (including all parameters).
        arity: usize,
        span: TextRange,
    },
    /// A variable is used but not defined in scope. `suggestion` is a name
    /// in scope it may be a misspelling of.
    UnboundVariable {
        name: String,
        span: TextRange,
        suggestion: Option<String>,
    },
    /// A non-function value is called as a function.
    NotAFunction { ty: Ty, span: TextRange },
    /// A type does not satisfy a required trait constraint. `builtin` when
    /// the type is a built-in one, which has no definition to derive on.
    TraitNotSatisfied {
        ty: Ty,
        trait_name: String,
        builtin: bool,
        origin: ConstraintOrigin,
    },
    /// An impl block is missing a method required by the trait.
    MissingTraitMethod {
        trait_name: String,
        method_name: String,
        impl_ty: String,
        /// The impl's header.
        span: TextRange,
    },
    /// An impl method's signature does not match the trait's method signature.
    TraitMethodSignatureMismatch {
        trait_name: String,
        method_name: String,
        expected: Ty,
        found: Ty,
        /// The impl method.
        span: TextRange,
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
    /// A method call on a type with no such method.
    NoSuchMethod {
        ty: Ty,
        method_name: String,
        span: TextRange,
    },
    /// Manual continuity promotion is not part of the Mesh surface anymore.
    ManualContinuityPromotionDisabled { span: TextRange },
    /// A variant name was used in a pattern but does not exist.
    /// `suggestion` is a known one it may be a misspelling of.
    /// `cons_tail`: the name ends a `head :: tail` pattern, as a type
    /// annotation written in a pattern (`n :: Int`) would.
    UnknownVariant {
        name: String,
        span: TextRange,
        suggestion: Option<String>,
        cons_tail: bool,
    },
    /// Or-pattern alternatives bind different sets of variables.
    /// `list_tail`: the or-pattern ends a list pattern, as `[a | rest]`, a
    /// tail as another language writes it, would.
    OrPatternBindingMismatch {
        expected_bindings: Vec<String>,
        found_bindings: Vec<String>,
        list_tail: bool,
        span: TextRange,
    },
    /// A match/case expression is not exhaustive.
    NonExhaustiveMatch {
        scrutinee_type: String,
        missing_patterns: Vec<String>,
        span: TextRange,
    },
    /// Function or closure clauses that do not cover every argument (a
    /// warning: a call no clause matches panics at run time).
    NonExhaustiveClauses {
        scrutinee_type: String,
        missing_patterns: Vec<String>,
        span: TextRange,
    },
    /// A match arm is redundant (unreachable given prior arms).
    RedundantArm { arm_index: usize, span: TextRange },
    /// Sending a message of wrong type to a typed Pid<M>.
    SendTypeMismatch {
        expected: Ty,
        found: Ty,
        span: TextRange,
    },
    /// self() called outside an actor block.
    SelfOutsideActor { span: TextRange },
    /// `Process.monitor` or `Node.monitor` outside an actor block: its message
    /// has no actor to go to.
    MonitorOutsideActor { span: TextRange },
    /// spawn called with a non-function argument.
    SpawnNonFunction { found: Ty, span: TextRange },
    /// receive used outside an actor block.
    ReceiveOutsideActor { span: TextRange },
    /// Child spec start function does not return Pid.
    InvalidChildStart {
        child_name: String,
        found: Ty,
        span: TextRange,
    },
    /// Unknown supervision strategy.
    InvalidStrategy { found: String, span: TextRange },
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
    /// Visibility/generics/return type on a non-first clause of a multi-clause function.
    NonFirstClauseAnnotation {
        fn_name: String,
        what: String,
        span: TextRange,
    },
    /// Two impl blocks implement the same trait for the same type (or structurally overlapping types).
    DuplicateImpl {
        trait_name: String,
        impl_type: String,
        /// Description of the first impl location (e.g. "previously defined here").
        first_impl: String,
        /// The second impl's header.
        span: TextRange,
    },
    /// Multiple traits provide a method with the same name for a given type, causing ambiguity.
    AmbiguousMethod {
        method_name: String,
        /// The trait names that all provide this method.
        candidate_traits: Vec<String>,
        ty: Ty,
        span: TextRange,
    },
    /// An unsupported trait name appears in a deriving clause.
    UnsupportedDerive {
        trait_name: String,
        type_name: String,
        span: TextRange,
    },
    /// A derived trait requires another trait that is not in the deriving list.
    MissingDerivePrerequisite {
        trait_name: String,
        requires: String,
        type_name: String,
        span: TextRange,
    },
    /// `break` used outside of a loop.
    BreakOutsideLoop { span: TextRange },
    /// `continue` used outside of a loop.
    ContinueOutsideLoop { span: TextRange },
    /// Module not found during import resolution (IMPORT-06).
    ImportModuleNotFound {
        module_name: String,
        span: TextRange,
        /// Optional suggestion (closest module name match).
        suggestion: Option<String>,
    },
    /// Name not found in imported module (IMPORT-07).
    ImportNameNotFound {
        module_name: String,
        name: String,
        span: TextRange,
        /// Available names in the module (for "did you mean?" suggestions).
        available: Vec<String>,
    },
    /// Attempted to import a private (non-pub) item from a module (VIS-03).
    PrivateItem {
        module_name: String,
        name: String,
        span: TextRange,
    },
    /// `HTTP.clustered(...)` received malformed arguments or a non-handler reference.
    HttpClusteredInvalidArguments { reason: String, span: TextRange },
    /// `HTTP.clustered(...)` referenced a private handler.
    HttpClusteredPrivateHandler {
        handler_name: String,
        span: TextRange,
    },
    /// `HTTP.clustered(...)` was not used in the route-handler position.
    HttpClusteredOutsideRouteHandlerPosition { span: TextRange },
    /// The same clustered route handler was declared with conflicting counts.
    HttpClusteredConflictingReplicationCount {
        runtime_name: String,
        first_count: u32,
        current_count: u32,
        first_span: TextRange,
        span: TextRange,
    },
    /// Imported bare handler resolution lost the defining-module origin.
    HttpClusteredImportedOriginMissing {
        handler_name: String,
        span: TextRange,
    },
    /// `?` operator used in function that doesn't return Result or Option.
    TryIncompatibleReturn {
        /// The type of the operand (e.g., Result<Int, String>).
        operand_ty: Ty,
        /// The enclosing function's return type (e.g., Int).
        fn_return_ty: Ty,
        span: TextRange,
    },
    /// `?` operator used on a value that is not Result or Option.
    TryOnNonResultOption {
        /// The actual type of the operand.
        operand_ty: Ty,
        span: TextRange,
    },
    /// A field type in a `deriving(Json)` struct is not JSON-serializable.
    NonSerializableField {
        struct_name: String,
        field_name: String,
        field_type: String,
        span: TextRange,
    },
    /// A field type in a `deriving(Row)` struct is not row-mappable.
    NonMappableField {
        struct_name: String,
        field_name: String,
        field_type: String,
        span: TextRange,
    },
    /// An impl block is missing a required associated type declared by the trait.
    MissingAssocType {
        trait_name: String,
        assoc_name: String,
        impl_ty: String,
        /// The impl's header.
        span: TextRange,
    },
    /// An impl block provides an associated type not declared by the trait.
    ExtraAssocType {
        trait_name: String,
        assoc_name: String,
        impl_ty: String,
        /// The binding of the associated type.
        span: TextRange,
    },
    /// An associated type reference (Self.Item) could not be resolved.
    UnresolvedAssocType { assoc_name: String, span: TextRange },
    /// A type alias references a type name that does not exist (ALIAS-04).
    UndefinedType {
        /// The name of the type alias.
        alias_name: String,
        /// The target type name that could not be resolved.
        target_name: String,
        span: TextRange,
    },
    /// A native ABI declaration is unsafe, ambiguous, or not fully typed.
    NativeDeclarationInvalid { reason: String, span: TextRange },
    /// A library export does not match the stable binary request/response ABI.
    ExportDeclarationInvalid { reason: String, span: TextRange },
    /// A destructuring `let` used a refutable or otherwise unsupported pattern.
    InvalidLetPattern { reason: String, span: TextRange },
    /// A match arm with no `->` has a pattern that cannot stand for a value.
    InvalidPassThroughArm { reason: String, span: TextRange },
    /// One pattern binds the same name twice (`(a, a)`).
    DuplicateBinding { name: String, span: TextRange },
    /// A generic function applies an operator to a value of its type
    /// parameter without bounding the parameter by the operator's trait.
    UnboundedTypeParam {
        param: String,
        trait_name: String,
        origin: ConstraintOrigin,
    },
    /// A method several impls provide, with different return types, called
    /// where nothing picks one (`found` is the type the context asked for,
    /// when it asked for one none of them returns). With `by_argument`, the
    /// impls differ in the argument they take instead (`Meters.from(x)`),
    /// and `found` is the argument's type.
    AmbiguousImplMethod {
        method: String,
        receiver: Ty,
        candidates: Vec<Ty>,
        found: Option<Ty>,
        span: TextRange,
        by_argument: bool,
    },
    /// A field, variant, parameter or type defined twice.
    DuplicateDefinition {
        kind: &'static str,
        name: String,
        span: TextRange,
    },
    /// An annotation names a type that does not exist.
    UnknownType { name: String, span: TextRange },
    /// A field read from a value whose type nothing determines.
    UnknownFieldOwner { field: String, span: TextRange },
    /// A type's name used as a value (`let x = Point`): a struct or sum
    /// type's, or a built-in type's (`let x = Int`).
    TypeNotValue {
        name: String,
        builtin: bool,
        span: TextRange,
    },
    /// A definition (`fn`, `struct`, `import`, ...) inside a function body:
    /// only `let` binds there.
    NestedDefinition {
        keyword: &'static str,
        span: TextRange,
    },
    /// A generic type named with another number of type arguments than it
    /// takes (`Option<Int, String>`).
    TypeArgumentCount {
        name: String,
        expected: usize,
        found: usize,
        span: TextRange,
    },
    /// A method parameter whose type nothing fixes: a method is compiled
    /// once, for the type it is implemented for, not for each call.
    UntypedMethodParam {
        method: String,
        param: String,
        span: TextRange,
    },
    /// A method an impl provides for the receiver's type whose return type
    /// nothing fixes there: the call cannot be given one.
    MethodReturnUnknown {
        method: String,
        ty: Ty,
        span: TextRange,
    },
    /// An `impl` names an interface that does not exist.
    UnknownInterface { name: String, span: TextRange },
    /// A numeric literal that is malformed or does not fit its type.
    InvalidLiteral { reason: String, span: TextRange },
    /// `Module.name` where the module has no such function.
    NoSuchModuleFunction {
        module: String,
        name: String,
        available: Vec<String>,
        span: TextRange,
    },
    /// `assert_receive PATTERN, TIMEOUT` outside a test file, where
    /// `meshc test` does not expand it.
    AssertReceiveOutsideTest { span: TextRange },
    /// An `impl` for a type that takes type parameters (`impl Show for
    /// Box` with `struct Box<T>`), which is not supported.
    GenericImplTarget { name: String, span: TextRange },
    /// A bare reference to a fn defined at more than one arity, which
    /// names no single function.
    OverloadedFunctionValue {
        name: String,
        arities: Vec<usize>,
        span: TextRange,
    },
    /// `value[index]`: Mesh has no indexing syntax.
    IndexingUnsupported { span: TextRange },
    /// Nothing in the module tells the type of the messages an actor
    /// receives (it is sent to only through untyped `Pid`s).
    ActorMessageTypeUnknown { actor: String, span: TextRange },
    /// A `let` outside any function: it makes no global.
    TopLevelLet { name: String, span: TextRange },
    /// An expression outside every function (`println("hi")` at the top of
    /// a file), which nothing runs.
    TopLevelStatement { span: TextRange },
    /// A module of the project named without importing it: `Geo.area(1)`
    /// with no `import Geo` (the file declaring `module Geo` imports it too).
    ModuleNotImported {
        name: String,
        module: String,
        span: TextRange,
    },
    /// `<>` or `++` on values that are neither strings nor lists.
    InvalidConcat {
        op: &'static str,
        ty: Ty,
        span: TextRange,
    },
    /// A generic function's body fixes one of its type parameters (to a
    /// type, or to another parameter).
    RigidTypeParam {
        param: String,
        found: Ty,
        span: TextRange,
    },
    /// A static interface method (no `self`) called bare, which several
    /// types provide.
    AmbiguousStaticMethod {
        method: String,
        types: Vec<String>,
        span: TextRange,
    },
    /// Nothing tells which type a `default()` call builds.
    AmbiguousDefault { span: TextRange },
    /// A type alias expands into itself.
    CyclicAlias { alias_name: String, span: TextRange },
    /// Two sum types of one module declare a variant of the same name.
    DuplicateVariant {
        variant: String,
        first_type: String,
        second_type: String,
        span: TextRange,
    },
    /// A derived trait needs to compare, hash or show a field that holds a
    /// function.
    UnderivableField {
        trait_name: String,
        type_name: String,
        field_name: String,
        span: TextRange,
    },
    /// A struct literal or update gives the same field twice.
    DuplicateField { field_name: String, span: TextRange },
    /// A struct literal names, or a struct update is applied to, something
    /// that is not a struct (`ty` is the value's type; a variable when it
    /// is not known there).
    NotAStruct { ty: Ty, span: TextRange },
    /// A value with affine resource ownership crossed an invalid boundary or
    /// was used in an invalid ownership state.
    ResourceViolation { reason: String, span: TextRange },
}

impl TypeError {
    /// Where in the source the error is: its own span, or its constraint's
    /// origin; `None` for a constraint of the builtins, at no one place.
    pub fn span(&self) -> Option<TextRange> {
        match self {
            TypeError::Mismatch { origin, .. }
            | TypeError::InfiniteType { origin, .. }
            | TypeError::ArityMismatch { origin, .. }
            | TypeError::TupleParameterSplit { origin, .. }
            | TypeError::TraitNotSatisfied { origin, .. }
            | TypeError::UnboundedTypeParam { origin, .. } => origin.span(),
            TypeError::NonConsecutiveClauses { second_span, .. } => Some(*second_span),
            TypeError::UnboundVariable { span, .. }
            | TypeError::NotAFunction { span, .. }
            | TypeError::MissingTraitMethod { span, .. }
            | TypeError::TraitMethodSignatureMismatch { span, .. }
            | TypeError::MissingField { span, .. }
            | TypeError::UnknownField { span, .. }
            | TypeError::NoSuchField { span, .. }
            | TypeError::UnknownVariant { span, .. }
            | TypeError::OrPatternBindingMismatch { span, .. }
            | TypeError::NonExhaustiveMatch { span, .. }
            | TypeError::NonExhaustiveClauses { span, .. }
            | TypeError::RedundantArm { span, .. }
            | TypeError::SendTypeMismatch { span, .. }
            | TypeError::SelfOutsideActor { span, .. }
            | TypeError::MonitorOutsideActor { span, .. }
            | TypeError::SpawnNonFunction { span, .. }
            | TypeError::ReceiveOutsideActor { span, .. }
            | TypeError::InvalidChildStart { span, .. }
            | TypeError::InvalidStrategy { span, .. }
            | TypeError::InvalidRestartType { span, .. }
            | TypeError::InvalidShutdownValue { span, .. }
            | TypeError::CatchAllNotLast { span, .. }
            | TypeError::NonFirstClauseAnnotation { span, .. }
            | TypeError::DuplicateImpl { span, .. }
            | TypeError::AmbiguousMethod { span, .. }
            | TypeError::UnsupportedDerive { span, .. }
            | TypeError::MissingDerivePrerequisite { span, .. }
            | TypeError::NoSuchMethod { span, .. }
            | TypeError::ManualContinuityPromotionDisabled { span }
            | TypeError::BreakOutsideLoop { span, .. }
            | TypeError::ContinueOutsideLoop { span, .. }
            | TypeError::ImportModuleNotFound { span, .. }
            | TypeError::ImportNameNotFound { span, .. }
            | TypeError::PrivateItem { span, .. }
            | TypeError::HttpClusteredInvalidArguments { span, .. }
            | TypeError::HttpClusteredPrivateHandler { span, .. }
            | TypeError::HttpClusteredOutsideRouteHandlerPosition { span }
            | TypeError::HttpClusteredConflictingReplicationCount { span, .. }
            | TypeError::HttpClusteredImportedOriginMissing { span, .. }
            | TypeError::TryIncompatibleReturn { span, .. }
            | TypeError::TryOnNonResultOption { span, .. }
            | TypeError::NonSerializableField { span, .. }
            | TypeError::NonMappableField { span, .. }
            | TypeError::MissingAssocType { span, .. }
            | TypeError::ExtraAssocType { span, .. }
            | TypeError::UnresolvedAssocType { span, .. }
            | TypeError::SlotPipeOutOfRange { span, .. }
            | TypeError::UndefinedType { span, .. }
            | TypeError::NativeDeclarationInvalid { span, .. }
            | TypeError::ExportDeclarationInvalid { span, .. }
            | TypeError::InvalidLetPattern { span, .. }
            | TypeError::InvalidPassThroughArm { span, .. }
            | TypeError::DuplicateBinding { span, .. }
            | TypeError::DuplicateField { span, .. }
            | TypeError::NotAStruct { span, .. }
            | TypeError::UnderivableField { span, .. }
            | TypeError::DuplicateVariant { span, .. }
            | TypeError::CyclicAlias { span, .. }
            | TypeError::AmbiguousDefault { span }
            | TypeError::AmbiguousImplMethod { span, .. }
            | TypeError::AmbiguousStaticMethod { span, .. }
            | TypeError::RigidTypeParam { span, .. }
            | TypeError::DuplicateDefinition { span, .. }
            | TypeError::UnknownType { span, .. }
            | TypeError::UnknownFieldOwner { span, .. }
            | TypeError::UntypedMethodParam { span, .. }
            | TypeError::MethodReturnUnknown { span, .. }
            | TypeError::TypeNotValue { span, .. }
            | TypeError::NestedDefinition { span, .. }
            | TypeError::TypeArgumentCount { span, .. }
            | TypeError::UnknownInterface { span, .. }
            | TypeError::InvalidLiteral { span, .. }
            | TypeError::InvalidConcat { span, .. }
            | TypeError::IndexingUnsupported { span }
            | TypeError::ActorMessageTypeUnknown { span, .. }
            | TypeError::TopLevelLet { span, .. }
            | TypeError::TopLevelStatement { span }
            | TypeError::ModuleNotImported { span, .. }
            | TypeError::NoSuchModuleFunction { span, .. }
            | TypeError::OverloadedFunctionValue { span, .. }
            | TypeError::GenericImplTarget { span, .. }
            | TypeError::AssertReceiveOutsideTest { span }
            | TypeError::ResourceViolation { span, .. } => Some(*span),
        }
    }
}

impl ConstraintOrigin {
    /// Where in the source the constraint came from; `None` for a builtin.
    pub fn span(&self) -> Option<TextRange> {
        match self {
            ConstraintOrigin::FnArg { call_site, .. } => Some(*call_site),
            ConstraintOrigin::BinOp { op_span } => Some(*op_span),
            ConstraintOrigin::IfBranches { if_span, .. } => Some(*if_span),
            ConstraintOrigin::Annotation { annotation_span } => Some(*annotation_span),
            ConstraintOrigin::Return { return_span, .. } => Some(*return_span),
            ConstraintOrigin::LetBinding { binding_span } => Some(*binding_span),
            ConstraintOrigin::Assignment { lhs_span, .. } => Some(*lhs_span),
            ConstraintOrigin::Expr { span } => Some(*span),
            ConstraintOrigin::Pattern { pattern_span } => Some(*pattern_span),
            ConstraintOrigin::Builtin => None,
        }
    }
}

impl fmt::Display for TypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TypeError::Mismatch {
                expected, found, ..
            } => {
                write!(
                    f,
                    "type mismatch: expected `{}`, found `{}`",
                    expected.with_holes(),
                    found.with_holes()
                )
            }
            TypeError::InfiniteType { var, ty, .. } => {
                write!(f, "infinite type: `?{}` occurs in `{}`", var.0, ty)
            }
            TypeError::TupleParameterSplit { elements, .. } => write!(
                f,
                "a function of {elements} parameters where one taking a {elements}-tuple is expected"
            ),
            TypeError::ArityMismatch {
                expected, found, ..
            } => {
                let plural = if *expected == 1 { "" } else { "s" };
                write!(
                    f,
                    "arity mismatch: expected {expected} argument{plural}, found {found}"
                )
            }
            TypeError::SlotPipeOutOfRange {
                slot,
                fn_name,
                arity,
                ..
            } => {
                if *arity <= 1 {
                    write!(
                        f,
                        "slot position {slot} is out of range: `{fn_name}` takes fewer than 2 arguments; use |> instead"
                    )
                } else {
                    write!(
                        f,
                        "slot position {} is out of range: `{}` takes {} arguments, so valid slot positions are 2\u{2013}{}",
                        slot, fn_name, arity, arity
                    )
                }
            }
            TypeError::UnboundVariable { name, .. } => {
                write!(f, "undefined variable `{}`", name)
            }
            TypeError::NotAFunction { ty, .. } => {
                write!(f, "`{}` is not a function", ty.with_holes())
            }
            TypeError::TraitNotSatisfied { ty, trait_name, .. } => {
                write!(
                    f,
                    "`{}` does not implement `{}`",
                    ty.with_holes(),
                    trait_name
                )
            }
            TypeError::MissingTraitMethod {
                trait_name,
                method_name,
                impl_ty,
                ..
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
                ..
            } => {
                write!(
                    f,
                    "method `{}` in impl `{}` has wrong signature: expected `{}`, found `{}`",
                    method_name,
                    trait_name,
                    expected.with_holes(),
                    found.with_holes()
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
            TypeError::NoSuchField { ty, field_name, .. } => {
                write!(
                    f,
                    "type `{}` has no field `{}`",
                    ty.with_holes(),
                    field_name
                )
            }
            TypeError::NoSuchMethod {
                ty, method_name, ..
            } => {
                write!(
                    f,
                    "no method `{}` on type `{}`",
                    method_name,
                    ty.with_holes()
                )
            }
            TypeError::ManualContinuityPromotionDisabled { .. } => {
                write!(
                    f,
                    "`Continuity.promote()` is disabled; failover is automatic-only"
                )
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
            TypeError::NonExhaustiveClauses {
                scrutinee_type,
                missing_patterns,
                ..
            } => {
                write!(
                    f,
                    "clauses do not cover every `{}`: missing patterns [{}]",
                    scrutinee_type,
                    missing_patterns.join(", ")
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
                write!(f, "redundant match arm (arm {})", arm_index + 1)
            }
            TypeError::SendTypeMismatch {
                expected, found, ..
            } => {
                write!(
                    f,
                    "message type mismatch: expected `{}`, found `{}`",
                    expected.with_holes(),
                    found.with_holes()
                )
            }
            TypeError::SelfOutsideActor { .. } => {
                write!(f, "self() used outside actor block")
            }
            TypeError::MonitorOutsideActor { .. } => {
                write!(f, "monitor used outside actor block")
            }
            TypeError::SpawnNonFunction { found, .. } => {
                write!(
                    f,
                    "cannot spawn non-function: found `{}`",
                    found.with_holes()
                )
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
                    child_name,
                    found.with_holes()
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
            TypeError::CatchAllNotLast { fn_name, arity, .. } => {
                write!(
                    f,
                    "catch-all clause must be the last clause of function `{}/{}`; clauses after a catch-all are unreachable",
                    fn_name, arity
                )
            }
            TypeError::NonConsecutiveClauses { fn_name, arity, .. } => {
                write!(
                    f,
                    "function `{}/{}` already defined; multi-clause functions must have consecutive clauses",
                    fn_name, arity
                )
            }
            TypeError::NonFirstClauseAnnotation { fn_name, what, .. } => {
                write!(
                    f,
                    "{} on non-first clause of `{}` will be ignored",
                    what, fn_name
                )
            }
            TypeError::DuplicateImpl {
                trait_name,
                impl_type,
                first_impl,
                ..
            } => {
                write!(
                    f,
                    "duplicate impl: `{}` is already implemented for `{}` ({})",
                    trait_name, impl_type, first_impl
                )
            }
            TypeError::AmbiguousMethod {
                method_name,
                candidate_traits,
                ty,
                span: _,
            } => {
                write!(
                    f,
                    "ambiguous method `{}` for type `{}`: candidates from traits [{}]",
                    method_name,
                    ty.with_holes(),
                    candidate_traits.join(", ")
                )
            }
            TypeError::UnsupportedDerive {
                trait_name,
                type_name,
                ..
            } => {
                write!(
                    f,
                    "cannot derive `{}` for `{}` -- structs derive Eq, Ord, Display, Debug, Hash, Json, Row, and Schema; sum types all but Row and Schema",
                    trait_name, type_name
                )
            }
            TypeError::MissingDerivePrerequisite {
                trait_name,
                requires,
                type_name,
                ..
            } => {
                write!(
                    f,
                    "deriving `{}` for `{}` requires `{}` to also be derived",
                    trait_name, type_name, requires
                )
            }
            TypeError::BreakOutsideLoop { .. } => {
                write!(f, "`break` outside of loop")
            }
            TypeError::ContinueOutsideLoop { .. } => {
                write!(f, "`continue` outside of loop")
            }
            TypeError::ImportModuleNotFound {
                module_name,
                suggestion,
                ..
            } => {
                if let Some(sug) = suggestion {
                    write!(
                        f,
                        "module `{}` not found; did you mean `{}`?",
                        module_name, sug
                    )
                } else {
                    write!(f, "module `{}` not found", module_name)
                }
            }
            TypeError::ImportNameNotFound {
                module_name,
                name,
                available,
                ..
            } => {
                if available.is_empty() {
                    write!(f, "`{}` is not exported by module `{}`", name, module_name)
                } else {
                    write!(
                        f,
                        "`{}` is not exported by module `{}`; available: {}",
                        name,
                        module_name,
                        available.join(", ")
                    )
                }
            }
            TypeError::PrivateItem {
                module_name, name, ..
            } => {
                write!(
                    f,
                    "`{}` is private in module `{}`; add `pub` to make it accessible",
                    name, module_name
                )
            }
            TypeError::HttpClusteredInvalidArguments { reason, .. } => {
                write!(f, "invalid HTTP.clustered(...) usage: {}", reason)
            }
            TypeError::HttpClusteredPrivateHandler { handler_name, .. } => {
                write!(
                    f,
                    "route handler `{}` must be public to use HTTP.clustered(...)",
                    handler_name
                )
            }
            TypeError::HttpClusteredOutsideRouteHandlerPosition { .. } => {
                write!(
                    f,
                    "HTTP.clustered(...) can only appear in the route handler position of HTTP.route(...) or HTTP.on_*(...)"
                )
            }
            TypeError::HttpClusteredConflictingReplicationCount {
                runtime_name,
                first_count,
                current_count,
                ..
            } => {
                write!(
                    f,
                    "clustered route handler `{}` already uses replication count {}; found conflicting count {}",
                    runtime_name, first_count, current_count
                )
            }
            TypeError::HttpClusteredImportedOriginMissing { handler_name, .. } => {
                write!(
                    f,
                    "imported route handler `{}` is missing defining-module metadata for HTTP.clustered(...)",
                    handler_name
                )
            }
            TypeError::TryIncompatibleReturn {
                operand_ty,
                fn_return_ty,
                ..
            } => {
                write!(
                    f,
                    "`?` cannot propagate `{}` from a function returning `{}`",
                    operand_ty.with_holes(),
                    fn_return_ty.with_holes()
                )
            }
            TypeError::TryOnNonResultOption { operand_ty, .. } => {
                write!(
                    f,
                    "`?` operator requires `Result` or `Option`, found `{}`",
                    operand_ty.with_holes()
                )
            }
            TypeError::NonSerializableField {
                field_name,
                field_type,
                ..
            } => {
                write!(
                    f,
                    "field `{}` of type `{}` is not JSON-serializable",
                    field_name, field_type
                )
            }
            TypeError::NonMappableField {
                field_name,
                field_type,
                ..
            } => {
                write!(
                    f,
                    "field `{}` has type `{}` which cannot be mapped from a database row (only Int, Float, Bool, String, and Option<T> are supported)",
                    field_name, field_type
                )
            }
            TypeError::MissingAssocType {
                trait_name,
                assoc_name,
                impl_ty,
                ..
            } => {
                write!(
                    f,
                    "impl `{}` for `{}` is missing associated type `{}`",
                    trait_name, impl_ty, assoc_name
                )
            }
            TypeError::ExtraAssocType {
                trait_name,
                assoc_name,
                impl_ty,
                ..
            } => {
                write!(
                    f,
                    "impl `{}` for `{}` provides associated type `{}` which is not declared by the trait",
                    trait_name, impl_ty, assoc_name
                )
            }
            TypeError::UnresolvedAssocType { assoc_name, .. } => {
                write!(f, "no associated type `{}` is declared", assoc_name)
            }
            TypeError::UndefinedType {
                alias_name,
                target_name,
                ..
            } => {
                write!(
                    f,
                    "type alias `{}` references undefined type `{}`",
                    alias_name, target_name
                )
            }
            TypeError::NativeDeclarationInvalid { reason, .. } => {
                write!(f, "invalid native function declaration: {reason}")
            }
            TypeError::ExportDeclarationInvalid { reason, .. } => {
                write!(f, "invalid exported function declaration: {reason}")
            }
            TypeError::InvalidLetPattern { reason, .. } => {
                write!(f, "invalid destructuring pattern: {reason}")
            }
            TypeError::UnboundedTypeParam {
                param, trait_name, ..
            } => {
                write!(
                    f,
                    "`{param}` is not known to implement {trait_name}; add `where {param}: {trait_name}`"
                )
            }
            TypeError::AmbiguousImplMethod {
                method,
                receiver,
                found,
                by_argument,
                ..
            } => match (found, by_argument, receiver.with_holes()) {
                (Some(found), false, receiver) => {
                    write!(f, "no impl's `{method}` on `{receiver}` returns `{found}`")
                }
                (Some(found), true, receiver) => {
                    write!(f, "no impl's `{method}` for `{receiver}` takes `{found}`")
                }
                (None, _, receiver) => write!(
                    f,
                    "cannot tell which impl's `{method}` to call on `{receiver}`"
                ),
            },
            TypeError::DuplicateDefinition { kind, name, .. } => {
                write!(f, "{kind} `{name}` is defined twice")
            }
            TypeError::UnknownType { name, .. } => write!(f, "unknown type `{name}`"),
            TypeError::UnknownInterface { name, .. } => write!(f, "unknown interface `{name}`"),
            TypeError::InvalidLiteral { reason, .. } => write!(f, "{reason}"),
            TypeError::NoSuchModuleFunction { module, name, .. } => {
                write!(f, "module `{module}` has no function `{name}`")
            }
            TypeError::AssertReceiveOutsideTest { .. } => {
                write!(
                    f,
                    "`assert_receive` works only in test files run by `meshc test`"
                )
            }
            TypeError::GenericImplTarget { name, .. } => {
                write!(
                    f,
                    "an `impl` for the generic type `{name}` is not supported"
                )
            }
            TypeError::OverloadedFunctionValue { name, arities, .. } => {
                write!(
                    f,
                    "`{name}` is defined at arities {}",
                    arity_list(arities, " and ")
                )
            }
            TypeError::InvalidConcat { op, ty, .. } => {
                write!(
                    f,
                    "`{op}` joins strings or lists, not `{}`",
                    ty.with_holes()
                )
            }
            TypeError::IndexingUnsupported { .. } => {
                write!(f, "`value[index]` indexing is not supported")
            }
            TypeError::ActorMessageTypeUnknown { actor, .. } => {
                write!(f, "cannot tell what type of message `{actor}` receives")
            }
            TypeError::TopLevelLet { name, .. } => {
                write!(f, "`let {name}` outside a function is not supported")
            }
            TypeError::TopLevelStatement { .. } => {
                write!(f, "a statement outside a function never runs")
            }
            TypeError::ModuleNotImported { name, .. } => {
                write!(f, "module `{name}` is not imported")
            }
            TypeError::UnknownFieldOwner { field, .. } => {
                write!(f, "cannot tell which type has the field `{field}`")
            }
            TypeError::TypeNotValue { name, .. } => {
                write!(f, "`{name}` is a type, not a value")
            }
            TypeError::NestedDefinition { keyword, .. } => {
                write!(
                    f,
                    "`{keyword}` belongs at the top level of a module, not inside a function"
                )
            }
            TypeError::TypeArgumentCount {
                name,
                expected,
                found,
                ..
            } => {
                let plural = if *expected == 1 { "" } else { "s" };
                write!(
                    f,
                    "`{name}` takes {expected} type argument{plural}, not {found}"
                )
            }
            TypeError::UntypedMethodParam { method, param, .. } => {
                write!(f, "the type of `{param}` in method `{method}` is not known")
            }
            TypeError::MethodReturnUnknown { method, ty, .. } => {
                let ty = ty.with_holes();
                write!(f, "the return type of method `{method}` on `{ty}` is not known")
            }
            TypeError::RigidTypeParam { param, found, .. } => {
                write!(
                    f,
                    "type parameter `{param}` stands for any type, but this function makes it `{}`",
                    found.with_holes()
                )
            }
            TypeError::AmbiguousStaticMethod { method, types, .. } => {
                write!(
                    f,
                    "`{method}` is a static method of several types ({}); call it on one",
                    types.join(", ")
                )
            }
            TypeError::AmbiguousDefault { .. } => {
                write!(f, "cannot tell which type `default()` builds here")
            }
            TypeError::CyclicAlias { alias_name, .. } => {
                write!(f, "type alias `{alias_name}` refers to itself")
            }
            TypeError::DuplicateVariant {
                variant,
                first_type,
                second_type,
                ..
            } => {
                write!(
                    f,
                    "variant `{variant}` of `{second_type}` is already a variant of `{first_type}`"
                )
            }
            TypeError::UnderivableField {
                trait_name,
                type_name,
                field_name,
                ..
            } => {
                write!(
                    f,
                    "cannot derive `{trait_name}` for `{type_name}`: field `{field_name}` holds a function"
                )
            }
            TypeError::DuplicateField { field_name, .. } => {
                write!(f, "field `{field_name}` is given more than once")
            }
            TypeError::NotAStruct { ty: Ty::Var(_), .. } => {
                write!(f, "a struct update needs a struct value")
            }
            TypeError::NotAStruct { ty, .. } => {
                write!(f, "`{}` is not a struct", ty.with_holes())
            }
            TypeError::DuplicateBinding { name, .. } => {
                write!(f, "`{name}` is bound twice in one pattern")
            }
            TypeError::InvalidPassThroughArm { reason, .. } => {
                write!(
                    f,
                    "this arm has no `->` and its pattern is not a value: {reason}"
                )
            }
            TypeError::ResourceViolation { reason, .. } => {
                write!(f, "resource ownership violation: {reason}")
            }
        }
    }
}

/// The arities of an overloaded name (two or more), the last after
/// `conjunction`: "1 and 2", "0, 1 or 3".
pub(crate) fn arity_list(arities: &[usize], conjunction: &str) -> String {
    let mut listed = String::new();
    for (index, arity) in arities.iter().enumerate() {
        if index > 0 {
            listed.push_str(if index + 1 == arities.len() {
                conjunction
            } else {
                ", "
            });
        }
        listed.push_str(&arity.to_string());
    }
    listed
}
