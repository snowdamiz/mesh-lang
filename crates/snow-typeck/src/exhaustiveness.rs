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

// Placeholder module -- tests will be added first per TDD, implementation follows.

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

/// Check whether a match expression is exhaustive.
///
/// Returns `None` if exhaustive, or `Some(witnesses)` with example
/// patterns that are not covered.
pub fn check_exhaustiveness(
    _arms: &[Pat],
    _scrutinee_type: &TypeInfo,
) -> Option<Vec<Pat>> {
    todo!()
}

/// Check for redundant (unreachable) arms in a match expression.
///
/// Returns the indices (0-based) of arms that are unreachable.
pub fn check_redundancy(
    _arms: &[Pat],
    _scrutinee_type: &TypeInfo,
) -> Vec<usize> {
    todo!()
}

/// Core usefulness predicate (Algorithm U).
///
/// Returns `true` if `row` is useful with respect to `matrix` --
/// i.e., there exists a value matched by `row` but not by any row
/// in `matrix`.
pub fn is_useful(
    _matrix: &PatternMatrix,
    _row: &[Pat],
    _type_info: &[TypeInfo],
) -> bool {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

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

    // ── is_useful base cases ─────────────────────────────────────────

    #[test]
    fn test_is_useful_empty_matrix_returns_true() {
        // Any pattern is useful against an empty matrix
        let m = matrix(vec![]);
        assert!(is_useful(&m, &[wildcard()], &[int_type()]));
    }

    #[test]
    fn test_is_useful_empty_row_returns_false() {
        // No more columns to match -- row is not useful
        let m = matrix(vec![vec![]]);
        assert!(!is_useful(&m, &[], &[]));
    }

    #[test]
    fn test_is_useful_empty_matrix_empty_row_returns_true() {
        // 0 rows, 0 columns: pattern is useful (no existing coverage)
        let m = matrix(vec![]);
        assert!(is_useful(&m, &[], &[]));
    }

    // ── Bool exhaustiveness ──────────────────────────────────────────

    #[test]
    fn test_bool_exhaustive() {
        // match x { true -> ..., false -> ... } is exhaustive
        let result = check_exhaustiveness(
            &[lit_bool(true), lit_bool(false)],
            &bool_type(),
        );
        assert!(result.is_none(), "Bool [true, false] should be exhaustive");
    }

    #[test]
    fn test_bool_non_exhaustive() {
        // match x { true -> ... } is NOT exhaustive, missing false
        let result = check_exhaustiveness(&[lit_bool(true)], &bool_type());
        assert!(result.is_some(), "Bool [true] should NOT be exhaustive");
        let witnesses = result.unwrap();
        assert!(!witnesses.is_empty());
    }

    #[test]
    fn test_bool_wildcard_exhaustive() {
        // match x { _ -> ... } is exhaustive for Bool
        let result = check_exhaustiveness(&[wildcard()], &bool_type());
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
        );
        assert!(result.is_none(), "Shape [Circle(_), Point] should be exhaustive");
    }

    #[test]
    fn test_sum_type_non_exhaustive() {
        // match shape { Circle(_) -> ... } is NOT exhaustive, missing Point
        let result = check_exhaustiveness(
            &[ctor("Circle", "Shape", vec![wildcard()])],
            &shape_type(),
        );
        assert!(result.is_some(), "Shape [Circle(_)] should NOT be exhaustive");
    }

    #[test]
    fn test_sum_type_wildcard_exhaustive() {
        // match shape { _ -> ... } is exhaustive
        let result = check_exhaustiveness(&[wildcard()], &shape_type());
        assert!(result.is_none(), "Shape [_] should be exhaustive");
    }

    // ── Redundancy checking ──────────────────────────────────────────

    #[test]
    fn test_redundant_arm_after_wildcard() {
        // match shape { _ -> ..., Circle(_) -> ... }
        // arm 1 (Circle) is redundant because _ catches everything
        let result = check_redundancy(
            &[wildcard(), ctor("Circle", "Shape", vec![wildcard()])],
            &shape_type(),
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
            &shape_type(),
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
            &shape_type(),
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
                ctor("Some", "Option", vec![ctor("Circle", "Shape", vec![wildcard()])]),
                ctor("Some", "Option", vec![ctor("Point", "Shape", vec![])]),
                ctor("None", "Option", vec![]),
            ],
            &option_shape_type(),
        );
        assert!(result.is_none(), "Option<Shape> fully covered should be exhaustive");
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
                ctor("Some", "Option", vec![ctor("Circle", "Shape", vec![wildcard()])]),
                ctor("None", "Option", vec![]),
            ],
            &option_shape_type(),
        );
        assert!(result.is_some(), "Option<Shape> missing Some(Point) should NOT be exhaustive");
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
        );
        assert!(result.is_none(), "Shape [Circle(_) | Point] should be exhaustive");
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
        );
        assert!(result.is_some(), "Shape [Circle(_) | Circle(_)] should NOT be exhaustive");
    }

    // ── Literal patterns ─────────────────────────────────────────────

    #[test]
    fn test_literal_with_wildcard_exhaustive() {
        // match x { 1 -> ..., 2 -> ..., _ -> ... } is exhaustive
        let result = check_exhaustiveness(
            &[lit_int(1), lit_int(2), wildcard()],
            &int_type(),
        );
        assert!(result.is_none(), "Int [1, 2, _] should be exhaustive");
    }

    #[test]
    fn test_literal_without_wildcard_non_exhaustive() {
        // match x { 1 -> ..., 2 -> ... } NOT exhaustive for Int (infinite)
        let result = check_exhaustiveness(
            &[lit_int(1), lit_int(2)],
            &int_type(),
        );
        assert!(result.is_some(), "Int [1, 2] should NOT be exhaustive");
    }

    #[test]
    fn test_literal_wildcard_only_exhaustive() {
        // match x { _ -> ... } is exhaustive for Int
        let result = check_exhaustiveness(&[wildcard()], &int_type());
        assert!(result.is_none(), "Int [_] should be exhaustive");
    }

    // ── is_useful with sum type constructors ─────────────────────────

    #[test]
    fn test_is_useful_constructor_against_different_constructor() {
        // Matrix has Circle(_), testing Point -- should be useful
        let m = matrix(vec![vec![ctor("Circle", "Shape", vec![wildcard()])]]);
        assert!(is_useful(
            &m,
            &[ctor("Point", "Shape", vec![])],
            &[shape_type()],
        ));
    }

    #[test]
    fn test_is_useful_constructor_against_same_constructor() {
        // Matrix has Circle(_), testing Circle(_) -- NOT useful
        let m = matrix(vec![vec![ctor("Circle", "Shape", vec![wildcard()])]]);
        assert!(!is_useful(
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
        assert!(!is_useful(&m, &[wildcard()], &[shape_type()]));
    }

    #[test]
    fn test_is_useful_wildcard_after_partial_constructors() {
        // Matrix has [Circle(_)], testing _ -- useful (Point not covered)
        let m = matrix(vec![vec![ctor("Circle", "Shape", vec![wildcard()])]]);
        assert!(is_useful(&m, &[wildcard()], &[shape_type()]));
    }

    // ── is_useful with literals ──────────────────────────────────────

    #[test]
    fn test_is_useful_new_literal_value() {
        // Matrix has [1], testing 2 -- useful
        let m = matrix(vec![vec![lit_int(1)]]);
        assert!(is_useful(&m, &[lit_int(2)], &[int_type()]));
    }

    #[test]
    fn test_is_useful_duplicate_literal_value() {
        // Matrix has [1], testing 1 -- NOT useful
        let m = matrix(vec![vec![lit_int(1)]]);
        assert!(!is_useful(&m, &[lit_int(1)], &[int_type()]));
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
        assert!(is_useful(
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
        assert!(!is_useful(
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
            &bool_type(),
        );
        assert_eq!(result, vec![2]);
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
            vec![ctor("Some", "Option", vec![ctor("Circle", "Shape", vec![wildcard()])])],
            vec![ctor("None", "Option", vec![])],
        ]);
        // After specializing by Some, inner column has Shape type
        // We need to provide type info for the nested level
        let result = is_useful(
            &m,
            &[ctor("Some", "Option", vec![ctor("Point", "Shape", vec![])])],
            &[option_shape_type()],
        );
        assert!(result, "Some(Point) should be useful when only Some(Circle(_)) and None are covered");
    }
}
