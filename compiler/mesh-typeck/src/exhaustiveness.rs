//! Maranget's usefulness algorithm for exhaustiveness and redundancy checking.
//!
//! This module implements Algorithm U from Luc Maranget's paper
//! "Warnings for Pattern Matching" (2007). It operates on an abstract
//! pattern representation (`Pat`), not AST nodes directly. Translation
//! from AST patterns to `Pat` happens elsewhere (Plan 04-04).
//!
//! The core predicate `is_useful(matrix, row, type_info)` determines whether
//! a new pattern row adds any coverage to the existing matrix. Both
//! exhaustiveness (is wildcard useful after all arms?) and redundancy
//! (is each arm useful given prior arms?) are expressed via `is_useful`.

use rustc_hash::{FxHashMap, FxHashSet};

/// The kind of a literal pattern value.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum LitKind {
    Int,
    Float,
    Bool,
    String,
}

/// Abstract pattern representation for exhaustiveness checking.
///
/// These are NOT AST nodes -- they are a simplified representation
/// used only by the usefulness algorithm.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pat {
    /// Matches anything (wildcard `_` or variable binding).
    Wildcard,
    /// Matches a specific constructor with arguments.
    Constructor {
        name: String,
        type_name: String,
        args: Vec<Pat>,
    },
    /// Matches a specific literal value.
    Literal { value: String, ty: LitKind },
    /// Matches any of the alternatives (or-pattern).
    Or { alternatives: Vec<Pat> },
}

/// A row in the pattern matrix (one match arm's patterns).
pub type PatternRow = Vec<Pat>;

/// The pattern matrix: each row corresponds to one match arm.
#[derive(Clone, Debug)]
pub struct PatternMatrix {
    pub rows: Vec<PatternRow>,
}

/// Signature of a constructor (name + arity).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConstructorSig {
    pub name: String,
    pub arity: usize,
}

/// Type information needed for exhaustiveness checking.
///
/// Tells the algorithm what constructors a type has, so it can
/// determine if all cases are covered.
#[derive(Clone, Debug)]
pub enum TypeInfo {
    /// A sum type with known, finite variants.
    SumType { variants: Vec<ConstructorSig> },
    /// Bool type (two constructors: true, false).
    Bool,
    /// A literal type with infinite inhabitants (Int, Float, String).
    Infinite,
}

/// Registry mapping type names to their complete constructor sets.
///
/// This is essential for nested specialization: when checking
/// `Option<Shape>` patterns, after specializing by `Some`, the inner
/// column needs the complete set of `Shape` constructors. The registry
/// provides this lookup.
#[derive(Clone, Debug, Default)]
pub struct TypeRegistry {
    types: FxHashMap<String, TypeInfo>,
}

impl TypeRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a type's info in the registry.
    pub fn register(&mut self, name: impl Into<String>, info: TypeInfo) {
        self.types.insert(name.into(), info);
    }

    /// Look up type info by name.
    pub fn lookup(&self, name: &str) -> Option<&TypeInfo> {
        self.types.get(name)
    }
}

// ── Constructor abstraction ──────────────────────────────────────────

/// A unified constructor representation used internally by the algorithm.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Constructor {
    /// A named constructor from a sum type (or Bool literal).
    Named { name: String, arity: usize },
    /// A literal value (acts as a nullary constructor).
    Literal { value: String, ty: LitKind },
}

impl Constructor {
    /// The constructor a pattern applies, and what it applies it to (a
    /// literal is a constructor of no arguments, and `true` and `false` are
    /// Bool's two); `None` for a wildcard or an or-pattern.
    fn of(pat: &Pat) -> Option<(Constructor, &[Pat])> {
        match pat {
            Pat::Constructor { name, args, .. } => Some((
                Constructor::Named {
                    name: name.clone(),
                    arity: args.len(),
                },
                args,
            )),
            Pat::Literal {
                value,
                ty: LitKind::Bool,
            } => Some((
                Constructor::Named {
                    name: value.clone(),
                    arity: 0,
                },
                &[],
            )),
            Pat::Literal { value, ty } => Some((
                Constructor::Literal {
                    value: value.clone(),
                    ty: ty.clone(),
                },
                &[],
            )),
            Pat::Wildcard | Pat::Or { .. } => None,
        }
    }

    fn arity(&self) -> usize {
        match self {
            Constructor::Named { arity, .. } => *arity,
            Constructor::Literal { .. } => 0,
        }
    }

    fn name_key(&self) -> String {
        match self {
            Constructor::Named { name, .. } => name.clone(),
            Constructor::Literal { value, ty } => format!("{:?}:{}", ty, value),
        }
    }
}

// ── Specialize & Default matrices ────────────────────────────────────
//
// Every row of a matrix is as wide as the row checked against it, which is
// not empty where a matrix is specialized or defaulted.

/// Specialize the matrix by a constructor: each row headed by it, or by a
/// wildcard, with the head replaced by its arguments (a wildcard's, by
/// wildcards); an or-pattern head by each alternative.
fn specialize_matrix(matrix: &PatternMatrix, ctor: &Constructor) -> PatternMatrix {
    let mut rows = Vec::new();
    for row in &matrix.rows {
        specialize_row_into(&mut rows, &row[0], &row[1..], ctor);
    }
    PatternMatrix { rows }
}

/// Specialize the row `head` then `rest` by a constructor, appending the
/// rows it gives to `out`.
fn specialize_row_into(out: &mut Vec<PatternRow>, head: &Pat, rest: &[Pat], ctor: &Constructor) {
    match head {
        Pat::Wildcard => {
            let mut new_row = vec![Pat::Wildcard; ctor.arity()];
            new_row.extend_from_slice(rest);
            out.push(new_row);
        }
        Pat::Or { alternatives } => {
            for alt in alternatives {
                specialize_row_into(out, alt, rest, ctor);
            }
        }
        _ => {
            if let Some((_, args)) =
                Constructor::of(head).filter(|(head, _)| head.name_key() == ctor.name_key())
            {
                let mut new_row = args.to_vec();
                new_row.extend_from_slice(rest);
                out.push(new_row);
            }
        }
    }
}

/// The default matrix: the rows headed by a wildcard (or an or-pattern
/// with one), without their head.
fn default_matrix(matrix: &PatternMatrix) -> PatternMatrix {
    let mut rows = Vec::new();
    for row in &matrix.rows {
        default_row_into(&mut rows, &row[0], &row[1..]);
    }
    PatternMatrix { rows }
}

/// The rows of the default matrix the row `head` then `rest` gives.
fn default_row_into(out: &mut Vec<PatternRow>, head: &Pat, rest: &[Pat]) {
    match head {
        Pat::Wildcard => out.push(rest.to_vec()),
        Pat::Or { alternatives } => {
            for alt in alternatives {
                default_row_into(out, alt, rest);
            }
        }
        Pat::Constructor { .. } | Pat::Literal { .. } => {}
    }
}

// ── Type info inference for nested columns ───────────────────────────

/// Infer the `TypeInfo` for a column created by specialization, from its
/// patterns: the type a constructor there names (the registry has every
/// sum type, struct and list; a tuple type is its patterns' arity), Bool
/// for `true` and `false`, and otherwise a type of endless values.
fn infer_type_info_for_column(
    matrix: &PatternMatrix,
    row: &[Pat],
    col: usize,
    registry: &TypeRegistry,
) -> TypeInfo {
    let rows = matrix.rows.iter().map(Vec::as_slice);
    let column = column(rows.chain(std::iter::once(row)), col);
    if let Some((type_name, name, arity)) = typed_constructor(&column) {
        if let Some(info) = registry.lookup(type_name) {
            return info.clone();
        }
        if name == TUPLE {
            return tuple_type_info(arity);
        }
    }
    let is_bool = |pat: &&Pat| {
        matches!(
            pat,
            Pat::Literal {
                ty: LitKind::Bool,
                ..
            }
        )
    };
    if column.iter().any(is_bool) {
        return TypeInfo::Bool;
    }
    TypeInfo::Infinite
}

/// Column `col` of `rows`, with or-patterns taken apart into their
/// alternatives.
fn column<'a>(rows: impl Iterator<Item = &'a [Pat]>, col: usize) -> Vec<&'a Pat> {
    fn alternatives_into<'a>(out: &mut Vec<&'a Pat>, pat: &'a Pat) {
        match pat {
            Pat::Or { alternatives } => {
                for alt in alternatives {
                    alternatives_into(out, alt);
                }
            }
            _ => out.push(pat),
        }
    }
    let mut pats = Vec::new();
    for row in rows {
        alternatives_into(&mut pats, &row[col]);
    }
    pats
}

/// The first constructor in `column` that names its type: that name, and
/// the constructor's name and arity.
fn typed_constructor<'a>(column: &[&'a Pat]) -> Option<(&'a str, &'a str, usize)> {
    column.iter().find_map(|pat| match pat {
        Pat::Constructor {
            name,
            type_name,
            args,
        } if !type_name.is_empty() => Some((type_name.as_str(), name.as_str(), args.len())),
        _ => None,
    })
}

/// Name shared by every tuple constructor and tuple type in abstract patterns.
pub const TUPLE: &str = "Tuple";

/// Type name of lists and the names of their two constructors in abstract patterns.
pub const LIST: &str = "List";
pub const CONS: &str = "::";
pub const NIL: &str = "[]";

/// The type info of a list: `head :: tail` or the empty list.
pub fn list_type_info() -> TypeInfo {
    TypeInfo::SumType {
        variants: vec![
            ConstructorSig {
                name: CONS.to_string(),
                arity: 2,
            },
            ConstructorSig {
                name: NIL.to_string(),
                arity: 0,
            },
        ],
    }
}

/// The type info of a tuple: a single constructor of the given arity.
pub fn tuple_type_info(arity: usize) -> TypeInfo {
    TypeInfo::SumType {
        variants: vec![ConstructorSig {
            name: TUPLE.to_string(),
            arity,
        }],
    }
}

// ── Collect head constructors ────────────────────────────────────────

/// The constructors heading the matrix's rows, once each.
fn collect_head_constructors(matrix: &PatternMatrix) -> Vec<Constructor> {
    let mut seen = FxHashSet::default();
    column(matrix.rows.iter().map(Vec::as_slice), 0)
        .into_iter()
        .filter_map(|head| Constructor::of(head).map(|(ctor, _)| ctor))
        .filter(|ctor| seen.insert(ctor.name_key()))
        .collect()
}

// ── Core algorithm ───────────────────────────────────────────────────

/// Core usefulness predicate (Algorithm U).
///
/// Returns `true` if `row` is useful with respect to `matrix` -- i.e.,
/// there exists a value matched by `row` but not by any row in `matrix`.
/// `type_info` describes the columns, and `registry` gives the complete
/// constructor sets of the types nested patterns name.
fn is_useful(
    matrix: &PatternMatrix,
    row: &[Pat],
    type_info: &[TypeInfo],
    registry: &TypeRegistry,
) -> bool {
    // Nothing matches yet: anything is useful.
    if matrix.rows.is_empty() {
        return true;
    }
    // Every column matched by some row: nothing is left.
    let Some((head, rest)) = row.split_first() else {
        return false;
    };
    if let Some((ctor, args)) = Constructor::of(head) {
        return is_useful_under(matrix, &ctor, args.to_vec(), rest, type_info, registry);
    }
    if let Pat::Or { alternatives } = head {
        return alternatives.iter().any(|alt| {
            let mut new_row = vec![alt.clone()];
            new_row.extend_from_slice(rest);
            is_useful(matrix, &new_row, type_info, registry)
        });
    }
    // A wildcard. When the rows' heads name every constructor of the
    // column's type, it is useful if it is under one of them; otherwise
    // (or for a type of endless values) if it is against the rows headed by
    // wildcards.
    let complete = all_constructors_for_type(type_info.first()).filter(|ctors| {
        let heads: FxHashSet<String> = collect_head_constructors(matrix)
            .iter()
            .map(Constructor::name_key)
            .collect();
        ctors.iter().all(|ctor| heads.contains(&ctor.name_key()))
    });
    match complete {
        Some(ctors) => ctors.iter().any(|ctor| {
            let args = vec![Pat::Wildcard; ctor.arity()];
            is_useful_under(matrix, ctor, args, rest, type_info, registry)
        }),
        None => is_useful(&default_matrix(matrix), rest, &type_info[1..], registry),
    }
}

/// Whether the row `ctor` applied to `args`, then `rest`, is useful: the
/// arguments against the matrix specialized by `ctor`.
fn is_useful_under(
    matrix: &PatternMatrix,
    ctor: &Constructor,
    mut args: Vec<Pat>,
    rest: &[Pat],
    type_info: &[TypeInfo],
    registry: &TypeRegistry,
) -> bool {
    let spec_matrix = specialize_matrix(matrix, ctor);
    args.extend_from_slice(rest);
    let inner_type_info =
        build_specialized_type_info(&spec_matrix, &args, ctor.arity(), &type_info[1..], registry);
    is_useful(&spec_matrix, &args, &inner_type_info, registry)
}

/// Get all constructors for a type from TypeInfo.
fn all_constructors_for_type(col_type: Option<&TypeInfo>) -> Option<Vec<Constructor>> {
    let ti = col_type?;
    match ti {
        TypeInfo::SumType { variants } => Some(
            variants
                .iter()
                .map(|v| Constructor::Named {
                    name: v.name.clone(),
                    arity: v.arity,
                })
                .collect(),
        ),
        TypeInfo::Bool => Some(vec![
            Constructor::Named {
                name: "true".to_string(),
                arity: 0,
            },
            Constructor::Named {
                name: "false".to_string(),
                arity: 0,
            },
        ]),
        TypeInfo::Infinite => None,
    }
}

/// Build type info for columns created by specialization.
fn build_specialized_type_info(
    spec_matrix: &PatternMatrix,
    spec_row: &[Pat],
    ctor_arity: usize,
    remaining_type_info: &[TypeInfo],
    registry: &TypeRegistry,
) -> Vec<TypeInfo> {
    let mut result = Vec::with_capacity(ctor_arity + remaining_type_info.len());
    for col_idx in 0..ctor_arity {
        result.push(infer_type_info_for_column(
            spec_matrix,
            spec_row,
            col_idx,
            registry,
        ));
    }
    result.extend_from_slice(remaining_type_info);
    result
}

// ── Public API ───────────────────────────────────────────────────────

/// Check whether a match expression is exhaustive.
///
/// Returns `None` if exhaustive, or `Some(witnesses)` with example
/// patterns that are not covered.
///
/// The `registry` provides complete constructor sets for all types
/// that may appear in nested patterns. For simple (non-nested) checks,
/// an empty registry suffices.
pub fn check_exhaustiveness(
    arms: &[Pat],
    scrutinee_type: &TypeInfo,
    registry: &TypeRegistry,
) -> Option<Vec<Pat>> {
    // A type without variants has no values, so no arm is missing.
    if matches!(scrutinee_type, TypeInfo::SumType { variants } if variants.is_empty()) {
        return None;
    }
    let matrix = PatternMatrix {
        rows: arms.iter().map(|arm| vec![arm.clone()]).collect(),
    };

    let wildcard_row = vec![Pat::Wildcard];
    let type_info = vec![scrutinee_type.clone()];

    if is_useful(&matrix, &wildcard_row, &type_info, registry) {
        let witnesses = find_witnesses(arms, scrutinee_type, registry);
        Some(witnesses)
    } else {
        None
    }
}

/// Check for redundant (unreachable) arms in a match expression.
///
/// Returns the indices (0-based) of arms that are unreachable.
///
/// `guarded[i]` says whether arm `i` has a `when` guard. A guarded arm may
/// fail its guard, so it never makes a later arm unreachable; it is still
/// itself checked against the unguarded arms before it.
///
/// The `registry` provides complete constructor sets for all types.
pub fn check_redundancy(
    arms: &[Pat],
    guarded: &[bool],
    scrutinee_type: &TypeInfo,
    registry: &TypeRegistry,
) -> Vec<usize> {
    let mut redundant = Vec::new();
    let type_info = vec![scrutinee_type.clone()];

    for i in 0..arms.len() {
        let prior_matrix = PatternMatrix {
            rows: arms[..i]
                .iter()
                .enumerate()
                .filter(|(j, _)| !guarded.get(*j).copied().unwrap_or(false))
                .map(|(_, arm)| vec![arm.clone()])
                .collect(),
        };
        let row = vec![arms[i].clone()];

        if !is_useful(&prior_matrix, &row, &type_info, registry) {
            redundant.push(i);
        }
    }

    redundant
}

/// Refine a useful `row` by filling its wildcards of finite type with the
/// first constructor that keeps the row useful, `depth` levels deep, so a
/// match on `[]` and `[x]` reports the missing `_ :: _ :: []` rather than
/// `_ :: _`, which would also cover the `[x]` arm.
fn refine_row(
    matrix: &PatternMatrix,
    row: Vec<Pat>,
    type_info: &[TypeInfo],
    registry: &TypeRegistry,
    depth: usize,
) -> Vec<Pat> {
    if row.is_empty() || depth == 0 {
        return row;
    }
    match row[0].clone() {
        Pat::Wildcard => {
            let candidates: Vec<Pat> = match type_info.first() {
                Some(TypeInfo::Bool) => ["true", "false"]
                    .iter()
                    .map(|value| Pat::Literal {
                        value: value.to_string(),
                        ty: LitKind::Bool,
                    })
                    .collect(),
                Some(TypeInfo::SumType { variants }) => {
                    let type_name = column_type_name(matrix, variants);
                    variants
                        .iter()
                        .map(|v| Pat::Constructor {
                            name: v.name.clone(),
                            type_name: type_name.clone(),
                            args: vec![Pat::Wildcard; v.arity],
                        })
                        .collect()
                }
                _ => Vec::new(),
            };
            for candidate in candidates {
                let mut trial = row.clone();
                trial[0] = candidate;
                if is_useful(matrix, &trial, type_info, registry) {
                    return refine_row(matrix, trial, type_info, registry, depth - 1);
                }
            }
            let rest = refine_row(
                &default_matrix(matrix),
                row[1..].to_vec(),
                &type_info[1..],
                registry,
                depth,
            );
            std::iter::once(Pat::Wildcard).chain(rest).collect()
        }
        Pat::Constructor {
            name,
            type_name,
            args,
        } => {
            let ctor = Constructor::Named {
                name: name.clone(),
                arity: args.len(),
            };
            let spec_matrix = specialize_matrix(matrix, &ctor);
            let mut spec_row = args.clone();
            spec_row.extend_from_slice(&row[1..]);
            let inner_type_info = build_specialized_type_info(
                &spec_matrix,
                &spec_row,
                args.len(),
                &type_info[1..],
                registry,
            );
            let refined = refine_row(
                &spec_matrix,
                spec_row,
                &inner_type_info,
                registry,
                depth - 1,
            );
            let (new_args, rest) = refined.split_at(args.len().min(refined.len()));
            std::iter::once(Pat::Constructor {
                name,
                type_name,
                args: new_args.to_vec(),
            })
            .chain(rest.iter().cloned())
            .collect()
        }
        _ => row,
    }
}

/// The type name to give a witness constructor for a column: the name the
/// column's own constructor patterns carry, or the list type's for a list.
fn column_type_name(matrix: &PatternMatrix, variants: &[ConstructorSig]) -> String {
    let column = column(matrix.rows.iter().map(Vec::as_slice), 0);
    match typed_constructor(&column) {
        Some((type_name, ..)) => type_name.to_string(),
        None if variants.iter().any(|v| v.name == CONS) => LIST.to_string(),
        None => String::new(),
    }
}

/// Find witness patterns for non-exhaustive match.
fn find_witnesses(arms: &[Pat], scrutinee_type: &TypeInfo, registry: &TypeRegistry) -> Vec<Pat> {
    match scrutinee_type {
        TypeInfo::SumType { variants } => {
            let mut missing = Vec::new();
            for v in variants {
                let ctor_pat = Pat::Constructor {
                    name: v.name.clone(),
                    type_name: String::new(),
                    args: vec![Pat::Wildcard; v.arity],
                };
                let matrix = PatternMatrix {
                    rows: arms.iter().map(|arm| vec![arm.clone()]).collect(),
                };
                let type_info = vec![scrutinee_type.clone()];

                if is_useful(
                    &matrix,
                    std::slice::from_ref(&ctor_pat),
                    &type_info,
                    registry,
                ) {
                    let refined = refine_row(&matrix, vec![ctor_pat], &type_info, registry, 4);
                    missing.push(refined.into_iter().next().unwrap_or(Pat::Wildcard));
                }
            }
            // A sum type that is not covered has a variant that is not.
            missing
        }
        TypeInfo::Bool => {
            let mut missing = Vec::new();
            for val in &["true", "false"] {
                let lit_pat = Pat::Literal {
                    value: val.to_string(),
                    ty: LitKind::Bool,
                };
                let matrix = PatternMatrix {
                    rows: arms.iter().map(|arm| vec![arm.clone()]).collect(),
                };
                let type_info = vec![scrutinee_type.clone()];

                if is_useful(
                    &matrix,
                    std::slice::from_ref(&lit_pat),
                    &type_info,
                    registry,
                ) {
                    missing.push(lit_pat);
                }
            }
            missing
        }
        TypeInfo::Infinite => {
            vec![Pat::Wildcard]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `is_useful` with a registry of the sum types the patterns name, each
    /// with the constructors they use.
    fn is_useful_by_patterns(matrix: &PatternMatrix, row: &[Pat], type_info: &[TypeInfo]) -> bool {
        fn collect(pat: &Pat, registry: &mut TypeRegistry) {
            match pat {
                Pat::Constructor {
                    name,
                    type_name,
                    args,
                } => {
                    if !type_name.is_empty() {
                        let entry = registry.types.entry(type_name.clone()).or_insert_with(|| {
                            TypeInfo::SumType {
                                variants: Vec::new(),
                            }
                        });
                        if let TypeInfo::SumType { variants } = entry {
                            if !variants.iter().any(|v| v.name == *name) {
                                variants.push(ConstructorSig {
                                    name: name.clone(),
                                    arity: args.len(),
                                });
                            }
                        }
                    }
                    args.iter().for_each(|arg| collect(arg, registry));
                }
                Pat::Or { alternatives } => {
                    alternatives.iter().for_each(|alt| collect(alt, registry))
                }
                Pat::Wildcard | Pat::Literal { .. } => {}
            }
        }
        let mut registry = TypeRegistry::new();
        for pat in matrix.rows.iter().flatten().chain(row) {
            collect(pat, &mut registry);
        }
        is_useful(matrix, row, type_info, &registry)
    }

    // ── Helper constructors ──────────────────────────────────────────

    fn wildcard() -> Pat {
        Pat::Wildcard
    }

    fn ctor(name: &str, type_name: &str, args: Vec<Pat>) -> Pat {
        Pat::Constructor {
            name: name.to_string(),
            type_name: type_name.to_string(),
            args,
        }
    }

    fn lit_int(value: i64) -> Pat {
        Pat::Literal {
            value: value.to_string(),
            ty: LitKind::Int,
        }
    }

    fn lit_bool(value: bool) -> Pat {
        Pat::Literal {
            value: value.to_string(),
            ty: LitKind::Bool,
        }
    }

    fn or_pat(alternatives: Vec<Pat>) -> Pat {
        Pat::Or { alternatives }
    }

    fn bool_type() -> TypeInfo {
        TypeInfo::Bool
    }

    fn int_type() -> TypeInfo {
        TypeInfo::Infinite
    }

    fn shape_type() -> TypeInfo {
        TypeInfo::SumType {
            variants: vec![
                ConstructorSig {
                    name: "Circle".to_string(),
                    arity: 1,
                },
                ConstructorSig {
                    name: "Point".to_string(),
                    arity: 0,
                },
            ],
        }
    }

    fn option_shape_type() -> TypeInfo {
        TypeInfo::SumType {
            variants: vec![
                ConstructorSig {
                    name: "Some".to_string(),
                    arity: 1,
                },
                ConstructorSig {
                    name: "None".to_string(),
                    arity: 0,
                },
            ],
        }
    }

    fn matrix(rows: Vec<Vec<Pat>>) -> PatternMatrix {
        PatternMatrix { rows }
    }

    /// Registry with Shape and Option type info for nested tests.
    fn test_registry() -> TypeRegistry {
        let mut reg = TypeRegistry::new();
        reg.register("Shape", shape_type());
        reg.register("Option", option_shape_type());
        reg
    }

    fn empty_registry() -> TypeRegistry {
        TypeRegistry::new()
    }

    // ── is_useful base cases ─────────────────────────────────────────

    #[test]
    fn test_is_useful_empty_matrix_returns_true() {
        // Any pattern is useful against an empty matrix
        let m = matrix(vec![]);
        assert!(is_useful_by_patterns(&m, &[wildcard()], &[int_type()]));
    }

    #[test]
    fn test_is_useful_empty_row_returns_false() {
        // No more columns to match -- row is not useful
        let m = matrix(vec![vec![]]);
        assert!(!is_useful_by_patterns(&m, &[], &[]));
    }

    #[test]
    fn test_is_useful_empty_matrix_empty_row_returns_true() {
        // 0 rows, 0 columns: pattern is useful (no existing coverage)
        let m = matrix(vec![]);
        assert!(is_useful_by_patterns(&m, &[], &[]));
    }

    // ── Bool exhaustiveness ──────────────────────────────────────────

    #[test]
    fn test_bool_exhaustive() {
        // match x { true -> ..., false -> ... } is exhaustive
        let result = check_exhaustiveness(
            &[lit_bool(true), lit_bool(false)],
            &bool_type(),
            &empty_registry(),
        );
        assert!(result.is_none(), "Bool [true, false] should be exhaustive");
    }

    #[test]
    fn test_bool_non_exhaustive() {
        // match x { true -> ... } is NOT exhaustive, missing false
        let result = check_exhaustiveness(&[lit_bool(true)], &bool_type(), &empty_registry());
        assert!(result.is_some(), "Bool [true] should NOT be exhaustive");
        let witnesses = result.unwrap();
        assert!(!witnesses.is_empty());
    }

    #[test]
    fn test_bool_wildcard_exhaustive() {
        // match x { _ -> ... } is exhaustive for Bool
        let result = check_exhaustiveness(&[wildcard()], &bool_type(), &empty_registry());
        assert!(result.is_none(), "Bool [_] should be exhaustive");
    }

    // ── Sum type exhaustiveness ──────────────────────────────────────

    #[test]
    fn test_sum_type_exhaustive() {
        // match shape { Circle(_) -> ..., Point -> ... } is exhaustive
        let result = check_exhaustiveness(
            &[
                ctor("Circle", "Shape", vec![wildcard()]),
                ctor("Point", "Shape", vec![]),
            ],
            &shape_type(),
            &test_registry(),
        );
        assert!(
            result.is_none(),
            "Shape [Circle(_), Point] should be exhaustive"
        );
    }

    #[test]
    fn test_sum_type_non_exhaustive() {
        // match shape { Circle(_) -> ... } is NOT exhaustive, missing Point
        let result = check_exhaustiveness(
            &[ctor("Circle", "Shape", vec![wildcard()])],
            &shape_type(),
            &test_registry(),
        );
        assert!(
            result.is_some(),
            "Shape [Circle(_)] should NOT be exhaustive"
        );
    }

    #[test]
    fn test_sum_type_wildcard_exhaustive() {
        // match shape { _ -> ... } is exhaustive
        let result = check_exhaustiveness(&[wildcard()], &shape_type(), &test_registry());
        assert!(result.is_none(), "Shape [_] should be exhaustive");
    }

    // ── Redundancy checking ──────────────────────────────────────────

    #[test]
    fn test_redundant_arm_after_wildcard() {
        // match shape { _ -> ..., Circle(_) -> ... }
        // arm 1 (Circle) is redundant because _ catches everything
        let result = check_redundancy(
            &[wildcard(), ctor("Circle", "Shape", vec![wildcard()])],
            &[false; 2],
            &shape_type(),
            &test_registry(),
        );
        assert_eq!(result, vec![1], "Arm 1 should be redundant after wildcard");
    }

    #[test]
    fn test_no_redundancy() {
        // match shape { Circle(_) -> ..., Point -> ... }
        // No redundant arms
        let result = check_redundancy(
            &[
                ctor("Circle", "Shape", vec![wildcard()]),
                ctor("Point", "Shape", vec![]),
            ],
            &[false; 2],
            &shape_type(),
            &test_registry(),
        );
        assert!(result.is_empty(), "No arms should be redundant");
    }

    #[test]
    fn test_duplicate_arm_redundant() {
        // match shape { Circle(_) -> ..., Circle(_) -> ..., Point -> ... }
        // arm 1 is redundant
        let result = check_redundancy(
            &[
                ctor("Circle", "Shape", vec![wildcard()]),
                ctor("Circle", "Shape", vec![wildcard()]),
                ctor("Point", "Shape", vec![]),
            ],
            &[false; 3],
            &shape_type(),
            &test_registry(),
        );
        assert_eq!(result, vec![1], "Duplicate Circle arm should be redundant");
    }

    // ── Nested patterns ──────────────────────────────────────────────

    #[test]
    fn test_nested_exhaustive() {
        // match opt_shape {
        //   Some(Circle(_)) -> ...,
        //   Some(Point) -> ...,
        //   None -> ...
        // }
        let result = check_exhaustiveness(
            &[
                ctor(
                    "Some",
                    "Option",
                    vec![ctor("Circle", "Shape", vec![wildcard()])],
                ),
                ctor("Some", "Option", vec![ctor("Point", "Shape", vec![])]),
                ctor("None", "Option", vec![]),
            ],
            &option_shape_type(),
            &test_registry(),
        );
        assert!(
            result.is_none(),
            "Option<Shape> fully covered should be exhaustive"
        );
    }

    #[test]
    fn test_nested_non_exhaustive() {
        // match opt_shape {
        //   Some(Circle(_)) -> ...,
        //   None -> ...
        // }
        // Missing Some(Point)
        let result = check_exhaustiveness(
            &[
                ctor(
                    "Some",
                    "Option",
                    vec![ctor("Circle", "Shape", vec![wildcard()])],
                ),
                ctor("None", "Option", vec![]),
            ],
            &option_shape_type(),
            &test_registry(),
        );
        assert!(
            result.is_some(),
            "Option<Shape> missing Some(Point) should NOT be exhaustive"
        );
    }

    /// Without the nested type in the registry, its column is taken for a
    /// type of endless values: every variant named there still leaves a
    /// value no arm covers.
    #[test]
    fn test_nested_type_missing_from_the_registry_is_endless() {
        let mut registry = TypeRegistry::new();
        registry.register("Option", option_shape_type());
        let result = check_exhaustiveness(
            &[
                ctor(
                    "Some",
                    "Option",
                    vec![ctor("Circle", "Shape", vec![wildcard()])],
                ),
                ctor("Some", "Option", vec![ctor("Point", "Shape", vec![])]),
                ctor("None", "Option", vec![]),
            ],
            &option_shape_type(),
            &registry,
        );
        assert!(
            result.is_some(),
            "an unregistered nested type cannot be covered"
        );
    }

    // ── Or-patterns ──────────────────────────────────────────────────

    #[test]
    fn test_or_pattern_exhaustive() {
        // match shape { Circle(_) | Point -> ... } is exhaustive
        let result = check_exhaustiveness(
            &[or_pat(vec![
                ctor("Circle", "Shape", vec![wildcard()]),
                ctor("Point", "Shape", vec![]),
            ])],
            &shape_type(),
            &test_registry(),
        );
        assert!(
            result.is_none(),
            "Shape [Circle(_) | Point] should be exhaustive"
        );
    }

    #[test]
    fn test_or_pattern_non_exhaustive() {
        // match shape { Circle(_) | Circle(_) -> ... } NOT exhaustive (missing Point)
        let result = check_exhaustiveness(
            &[or_pat(vec![
                ctor("Circle", "Shape", vec![wildcard()]),
                ctor("Circle", "Shape", vec![wildcard()]),
            ])],
            &shape_type(),
            &test_registry(),
        );
        assert!(
            result.is_some(),
            "Shape [Circle(_) | Circle(_)] should NOT be exhaustive"
        );
    }

    // ── Literal patterns ─────────────────────────────────────────────

    #[test]
    fn test_literal_with_wildcard_exhaustive() {
        // match x { 1 -> ..., 2 -> ..., _ -> ... } is exhaustive
        let result = check_exhaustiveness(
            &[lit_int(1), lit_int(2), wildcard()],
            &int_type(),
            &empty_registry(),
        );
        assert!(result.is_none(), "Int [1, 2, _] should be exhaustive");
    }

    #[test]
    fn test_literal_without_wildcard_non_exhaustive() {
        // match x { 1 -> ..., 2 -> ... } NOT exhaustive for Int (infinite)
        let result =
            check_exhaustiveness(&[lit_int(1), lit_int(2)], &int_type(), &empty_registry());
        assert!(result.is_some(), "Int [1, 2] should NOT be exhaustive");
    }

    #[test]
    fn test_literal_wildcard_only_exhaustive() {
        // match x { _ -> ... } is exhaustive for Int
        let result = check_exhaustiveness(&[wildcard()], &int_type(), &empty_registry());
        assert!(result.is_none(), "Int [_] should be exhaustive");
    }

    // ── is_useful with sum type constructors ─────────────────────────

    #[test]
    fn test_is_useful_constructor_against_different_constructor() {
        // Matrix has Circle(_), testing Point -- should be useful
        let m = matrix(vec![vec![ctor("Circle", "Shape", vec![wildcard()])]]);
        assert!(is_useful_by_patterns(
            &m,
            &[ctor("Point", "Shape", vec![])],
            &[shape_type()],
        ));
    }

    #[test]
    fn test_is_useful_constructor_against_same_constructor() {
        // Matrix has Circle(_), testing Circle(_) -- NOT useful
        let m = matrix(vec![vec![ctor("Circle", "Shape", vec![wildcard()])]]);
        assert!(!is_useful_by_patterns(
            &m,
            &[ctor("Circle", "Shape", vec![wildcard()])],
            &[shape_type()],
        ));
    }

    #[test]
    fn test_is_useful_wildcard_after_all_constructors() {
        // Matrix has [Circle(_), Point], testing _ -- NOT useful (type is complete)
        let m = matrix(vec![
            vec![ctor("Circle", "Shape", vec![wildcard()])],
            vec![ctor("Point", "Shape", vec![])],
        ]);
        assert!(!is_useful_by_patterns(&m, &[wildcard()], &[shape_type()]));
    }

    #[test]
    fn test_is_useful_wildcard_after_partial_constructors() {
        // Matrix has [Circle(_)], testing _ -- useful (Point not covered)
        let m = matrix(vec![vec![ctor("Circle", "Shape", vec![wildcard()])]]);
        assert!(is_useful_by_patterns(&m, &[wildcard()], &[shape_type()]));
    }

    // ── is_useful with literals ──────────────────────────────────────

    #[test]
    fn test_is_useful_new_literal_value() {
        // Matrix has [1], testing 2 -- useful
        let m = matrix(vec![vec![lit_int(1)]]);
        assert!(is_useful_by_patterns(&m, &[lit_int(2)], &[int_type()]));
    }

    #[test]
    fn test_is_useful_duplicate_literal_value() {
        // Matrix has [1], testing 1 -- NOT useful
        let m = matrix(vec![vec![lit_int(1)]]);
        assert!(!is_useful_by_patterns(&m, &[lit_int(1)], &[int_type()]));
    }

    // ── Multi-column patterns ────────────────────────────────────────

    #[test]
    fn test_is_useful_multi_column() {
        // Matrix: [[true, true], [false, false]]
        // Test: [true, false] -- should be useful
        let m = matrix(vec![
            vec![lit_bool(true), lit_bool(true)],
            vec![lit_bool(false), lit_bool(false)],
        ]);
        assert!(is_useful_by_patterns(
            &m,
            &[lit_bool(true), lit_bool(false)],
            &[bool_type(), bool_type()],
        ));
    }

    #[test]
    fn test_is_useful_multi_column_not_useful() {
        // Matrix: [[true, _], [false, _]]
        // Test: [true, true] -- NOT useful (first row covers it)
        let m = matrix(vec![
            vec![lit_bool(true), wildcard()],
            vec![lit_bool(false), wildcard()],
        ]);
        assert!(!is_useful_by_patterns(
            &m,
            &[lit_bool(true), lit_bool(true)],
            &[bool_type(), bool_type()],
        ));
    }

    // ── Bool redundancy edge cases ───────────────────────────────────

    #[test]
    fn test_bool_true_false_true_redundant() {
        // match b { true -> ..., false -> ..., true -> ... }
        // arm 2 is redundant
        let result = check_redundancy(
            &[lit_bool(true), lit_bool(false), lit_bool(true)],
            &[false; 3],
            &bool_type(),
            &empty_registry(),
        );
        assert_eq!(result, vec![2]);
    }

    #[test]
    fn test_guarded_arm_covers_nothing_for_later_arms() {
        // match b { true when g -> ..., true -> ..., false -> ... }
        // arm 1 is reachable: arm 0 may fail its guard.
        let arms = [lit_bool(true), lit_bool(true), lit_bool(false)];
        let result = check_redundancy(
            &arms,
            &[true, false, false],
            &bool_type(),
            &empty_registry(),
        );
        assert!(
            result.is_empty(),
            "guarded arm must not shadow arm 1: {result:?}"
        );
        // The guarded arm itself is still checked against earlier unguarded arms.
        let arms = [wildcard(), lit_bool(true)];
        let result = check_redundancy(&arms, &[false, true], &bool_type(), &empty_registry());
        assert_eq!(result, vec![1]);
    }

    fn tuple(args: Vec<Pat>) -> Pat {
        ctor(TUPLE, TUPLE, args)
    }

    #[test]
    fn test_tuple_of_bool_and_int_exhaustive() {
        // case (b, n) do (true, 0) | (true, _) | (false, 0) | (false, _) end
        let arms = [
            tuple(vec![lit_bool(true), lit_int(0)]),
            tuple(vec![lit_bool(true), wildcard()]),
            tuple(vec![lit_bool(false), lit_int(0)]),
            tuple(vec![lit_bool(false), wildcard()]),
        ];
        let registry = empty_registry();
        assert_eq!(
            check_exhaustiveness(&arms, &tuple_type_info(2), &registry),
            None
        );
        assert!(check_redundancy(&arms, &[false; 4], &tuple_type_info(2), &registry).is_empty());
    }

    #[test]
    fn test_tuple_missing_half_reports_tuple_witness() {
        // The witness names the half that is missing, not just "some tuple".
        let arms = [tuple(vec![lit_bool(true), wildcard()])];
        let missing = check_exhaustiveness(&arms, &tuple_type_info(2), &empty_registry());
        assert_eq!(
            missing,
            Some(vec![ctor(TUPLE, "", vec![lit_bool(false), wildcard()])])
        );
    }

    #[test]
    fn test_cons_pattern_needs_empty_list() {
        // case xs do h :: t -> ... end   is missing []
        let cons = ctor(CONS, LIST, vec![wildcard(), wildcard()]);
        let registry = empty_registry();
        assert_eq!(
            check_exhaustiveness(std::slice::from_ref(&cons), &list_type_info(), &registry),
            Some(vec![ctor(NIL, "", vec![])])
        );
        // case xs do h :: t -> ... | _ -> ... end   is exhaustive and arm 1 is useful
        let arms = [cons, wildcard()];
        assert_eq!(
            check_exhaustiveness(&arms, &list_type_info(), &registry),
            None
        );
        assert!(check_redundancy(&arms, &[false; 2], &list_type_info(), &registry).is_empty());
    }

    #[test]
    fn test_nested_tuple_inside_constructor_exhaustive() {
        // case opt do Some((true, _)) | Some((false, _)) | None end
        let arms = [
            ctor(
                "Some",
                "Option",
                vec![tuple(vec![lit_bool(true), wildcard()])],
            ),
            ctor(
                "Some",
                "Option",
                vec![tuple(vec![lit_bool(false), wildcard()])],
            ),
            ctor("None", "Option", vec![]),
        ];
        let mut registry = TypeRegistry::new();
        registry.register(
            "Option",
            TypeInfo::SumType {
                variants: vec![
                    ConstructorSig {
                        name: "Some".into(),
                        arity: 1,
                    },
                    ConstructorSig {
                        name: "None".into(),
                        arity: 0,
                    },
                ],
            },
        );
        let opt = registry.lookup("Option").unwrap().clone();
        assert_eq!(check_exhaustiveness(&arms, &opt, &registry), None);
        let arms = [
            ctor(
                "Some",
                "Option",
                vec![tuple(vec![lit_bool(true), wildcard()])],
            ),
            ctor("None", "Option", vec![]),
        ];
        assert!(check_exhaustiveness(&arms, &opt, &registry).is_some());
    }

    // ── TypeInfo for nested specialization ───────────────────────────

    #[test]
    fn test_nested_specialization_type_info() {
        // When we specialize Option by Some, the inner column needs
        // Shape type info. This tests that the algorithm correctly
        // handles nested type information via recursive specialization.
        //
        // Matrix: [Some(Circle(_)), None]
        // Test: Some(Point) -- should be useful
        let m = matrix(vec![
            vec![ctor(
                "Some",
                "Option",
                vec![ctor("Circle", "Shape", vec![wildcard()])],
            )],
            vec![ctor("None", "Option", vec![])],
        ]);
        // For this test, is_useful builds registry from patterns.
        // Patterns mention Circle and Point for Shape, so registry is complete.
        let result = is_useful_by_patterns(
            &m,
            &[ctor("Some", "Option", vec![ctor("Point", "Shape", vec![])])],
            &[option_shape_type()],
        );
        assert!(
            result,
            "Some(Point) should be useful when only Some(Circle(_)) and None are covered"
        );
    }
}
