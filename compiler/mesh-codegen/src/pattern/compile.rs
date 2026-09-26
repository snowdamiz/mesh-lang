//! Pattern matrix to decision tree compiler.
//!
//! Implements Maranget's algorithm for compiling pattern matrices into
//! efficient decision trees. The algorithm works by:
//!
//! 1. Representing match arms as a pattern matrix (rows = arms, columns = positions)
//! 2. Selecting the column with the most constructor diversity
//! 3. Specializing the matrix for each constructor
//! 4. Recursing on specialized sub-matrices
//! 5. Producing Leaf nodes when all patterns are wildcards/variables

use rustc_hash::{FxHashMap, FxHashSet};

use crate::mir::{
    by_sum_type_name, MirExpr, MirLiteral, MirMatchArm, MirPattern, MirSumTypeDef, MirType,
};
use crate::pattern::{AccessPath, ConstructorTag, DecisionTree};

// ── Pattern Matrix ──────────────────────────────────────────────────

/// A row in the pattern matrix: one pattern per column, with metadata
/// from the original match arm.
#[derive(Debug, Clone)]
struct PatRow {
    /// The patterns in each column position.
    patterns: Vec<MirPattern>,
    /// Original arm index (preserved through expansion).
    arm_index: usize,
    /// Optional guard expression.
    guard: Option<MirExpr>,
    /// Accumulated variable bindings collected so far.
    bindings: Vec<(String, MirType, AccessPath)>,
}

/// The pattern matrix: rows of patterns with access paths for each column.
#[derive(Debug, Clone)]
struct PatMatrix {
    /// The rows of the matrix.
    rows: Vec<PatRow>,
    /// Access path for each column (how to reach the sub-value).
    column_paths: Vec<AccessPath>,
    /// Type of each column.
    column_types: Vec<MirType>,
}

// ── Head constructor extraction ─────────────────────────────────────

/// A "head constructor" found in a pattern column -- either a literal value
/// or a sum type constructor.
#[derive(Debug, Clone)]
enum HeadCtor {
    /// A literal value (Int, Float, Bool, String).
    Literal(MirLiteral),
    /// A sum type constructor with variant info.
    Constructor {
        type_name: String,
        variant: String,
        tag: u8,
        arity: usize,
    },
    /// A list cons pattern (head :: tail).
    ListCons { elem_ty: MirType },
    /// The empty list pattern (`[]`).
    ListNil,
}

// ── Public API ──────────────────────────────────────────────────────

/// Compile a single match expression into a decision tree.
///
/// Takes the scrutinee type, a list of match arms, and source location
/// information for generating Fail nodes.
pub fn compile_match(
    scrutinee_ty: &MirType,
    arms: &[MirMatchArm],
    file: &str,
    line: u32,
    sum_type_defs: &FxHashMap<String, MirSumTypeDef>,
) -> DecisionTree {
    // Step 1: Expand or-patterns by duplicating arms for each alternative.
    let expanded = expand_or_patterns(arms);

    // Step 2: Build initial pattern matrix with one column (the scrutinee).
    let rows: Vec<PatRow> = expanded
        .iter()
        .map(|(arm_idx, pat, guard)| PatRow {
            patterns: vec![pat.clone()],
            arm_index: *arm_idx,
            guard: guard.clone(),
            bindings: Vec::new(),
        })
        .collect();

    let matrix = PatMatrix {
        rows,
        column_paths: vec![AccessPath::Root],
        column_types: vec![scrutinee_ty.clone()],
    };

    // Step 3: Compile the matrix into a decision tree.
    compile_matrix(matrix, file, line, sum_type_defs)
}

/// Whether every arm (every or-alternative) of a match on a tuple literal of
/// `arity` values is a tuple pattern of that arity or `_`, so the values can
/// be matched as separate columns without building the tuple.
pub fn arms_match_columns(arms: &[MirMatchArm], arity: usize) -> bool {
    arms.iter().all(|arm| {
        pattern_alternatives(&arm.pattern)
            .iter()
            .all(|pattern| match pattern {
                MirPattern::Tuple(elements) => elements.len() == arity,
                MirPattern::Wildcard => true,
                _ => false,
            })
    })
}

/// Compile a match on several values at once: arm patterns are tuples with
/// one sub-pattern per value (see `arms_match_columns`), and value N is
/// reached through `AccessPath::Column(N, ..)`.
pub fn compile_match_columns(
    column_types: &[MirType],
    arms: &[MirMatchArm],
    file: &str,
    line: u32,
    sum_type_defs: &FxHashMap<String, MirSumTypeDef>,
) -> DecisionTree {
    let rows = expand_or_patterns(arms)
        .into_iter()
        .map(|(arm_index, pattern, guard)| PatRow {
            patterns: match pattern {
                MirPattern::Tuple(elements) => elements,
                _ => vec![MirPattern::Wildcard; column_types.len()],
            },
            arm_index,
            guard,
            bindings: Vec::new(),
        })
        .collect();
    let matrix = PatMatrix {
        rows,
        column_paths: column_types
            .iter()
            .enumerate()
            .map(|(index, ty)| AccessPath::Column(index, ty.clone()))
            .collect(),
        column_types: column_types.to_vec(),
    };
    compile_matrix(matrix, file, line, sum_type_defs)
}

// ── Or-pattern expansion ────────────────────────────────────────────

/// Expand or-patterns by duplicating arms for each alternative.
/// Returns (original_arm_index, pattern, guard) tuples.
fn expand_or_patterns(arms: &[MirMatchArm]) -> Vec<(usize, MirPattern, Option<MirExpr>)> {
    let mut result = Vec::new();
    for (i, arm) in arms.iter().enumerate() {
        expand_pattern(i, &arm.pattern, &arm.guard, &mut result);
    }
    result
}

/// Expand the or-patterns in a single pattern, wherever they are nested: each
/// alternative becomes its own row with the same arm_index (they share the
/// body). `Some(1 | 2)` becomes `Some(1)` and `Some(2)`.
fn expand_pattern(
    arm_index: usize,
    pattern: &MirPattern,
    guard: &Option<MirExpr>,
    out: &mut Vec<(usize, MirPattern, Option<MirExpr>)>,
) {
    for alternative in pattern_alternatives(pattern) {
        out.push((arm_index, alternative, guard.clone()));
    }
}

/// The or-free patterns that together match what `pattern` matches.
fn pattern_alternatives(pattern: &MirPattern) -> Vec<MirPattern> {
    /// Every combination of one alternative per sub-pattern.
    fn product(parts: &[MirPattern]) -> Vec<Vec<MirPattern>> {
        let mut combos = vec![Vec::new()];
        for part in parts {
            let alternatives = pattern_alternatives(part);
            combos = combos
                .into_iter()
                .flat_map(|combo| {
                    alternatives.iter().map(move |alternative| {
                        let mut next = combo.clone();
                        next.push(alternative.clone());
                        next
                    })
                })
                .collect();
        }
        combos
    }

    match pattern {
        MirPattern::Or(alternatives) => {
            alternatives.iter().flat_map(pattern_alternatives).collect()
        }
        MirPattern::Constructor {
            type_name,
            variant,
            fields,
            bindings,
        } => product(fields)
            .into_iter()
            .map(|fields| MirPattern::Constructor {
                type_name: type_name.clone(),
                variant: variant.clone(),
                fields,
                bindings: bindings.clone(),
            })
            .collect(),
        MirPattern::Tuple(elements) => product(elements)
            .into_iter()
            .map(MirPattern::Tuple)
            .collect(),
        MirPattern::Struct { name, fields } => {
            let patterns: Vec<MirPattern> = fields.iter().map(|(_, _, p)| p.clone()).collect();
            product(&patterns)
                .into_iter()
                .map(|patterns| MirPattern::Struct {
                    name: name.clone(),
                    fields: fields
                        .iter()
                        .zip(patterns)
                        .map(|((field, ty, _), pattern)| (field.clone(), ty.clone(), pattern))
                        .collect(),
                })
                .collect()
        }
        MirPattern::ListCons {
            head,
            tail,
            elem_ty,
        } => product(&[(**head).clone(), (**tail).clone()])
            .into_iter()
            .map(|mut parts| {
                let tail = parts.pop().unwrap();
                let head = parts.pop().unwrap();
                MirPattern::ListCons {
                    head: Box::new(head),
                    tail: Box::new(tail),
                    elem_ty: elem_ty.clone(),
                }
            })
            .collect(),
        MirPattern::As { name, ty, inner } => pattern_alternatives(inner)
            .into_iter()
            .map(|inner| MirPattern::As {
                name: name.clone(),
                ty: ty.clone(),
                inner: Box::new(inner),
            })
            .collect(),
        MirPattern::Wildcard
        | MirPattern::Var(..)
        | MirPattern::Literal(_)
        | MirPattern::ListNil => {
            vec![pattern.clone()]
        }
    }
}

// ── Core compilation ────────────────────────────────────────────────

/// Compile a pattern matrix into a decision tree (Maranget's algorithm).
fn compile_matrix(
    mut matrix: PatMatrix,
    file: &str,
    line: u32,
    sum_type_defs: &FxHashMap<String, MirSumTypeDef>,
) -> DecisionTree {
    // `inner as name` binds the column's value and then matches like `inner`.
    // Peeling it here, where every column's path and type are known, keeps
    // the rest of the algorithm free of the variant.
    for row in &mut matrix.rows {
        for col in 0..row.patterns.len() {
            while let MirPattern::As { name, ty, inner } = &row.patterns[col] {
                let bind_ty = if *ty == MirType::Unit {
                    matrix.column_types[col].clone()
                } else {
                    ty.clone()
                };
                row.bindings
                    .push((name.clone(), bind_ty, matrix.column_paths[col].clone()));
                row.patterns[col] = (**inner).clone();
            }
        }
    }

    // Base case 1: No rows -- match failure.
    if matrix.rows.is_empty() {
        return DecisionTree::Fail {
            message: "non-exhaustive match".to_string(),
            file: file.to_string(),
            line,
        };
    }

    // Base case 2: No columns -- all patterns consumed. The first row wins.
    // Base case 3: First row is all wildcards/variables -- it matches.
    if matrix.column_paths.is_empty() || row_is_all_wildcards(&matrix.rows[0]) {
        let mut row = matrix.rows.remove(0);
        // Collect variable bindings from this row.
        collect_bindings_from_row(&mut row, &matrix.column_paths, &matrix.column_types);
        return make_leaf_or_guard(&row, matrix, file, line, sum_type_defs);
    }

    // Step 1: Select the best column to test (most constructor diversity).
    let col = select_column(&matrix);

    // Step 1.5: If the selected column contains tuple or struct patterns,
    // expand them first: they need no switch or test, only decomposition.
    if let Some(sub_columns) = product_columns(&matrix, col) {
        let expanded = expand_product_column(&matrix, col, sub_columns);
        return compile_matrix(expanded, file, line, sum_type_defs);
    }

    // Step 2: Collect head constructors from the selected column. The first
    // row tests something, so the selected column holds at least one.
    let head_ctors = collect_head_constructors(&matrix, col, sum_type_defs);

    // Step 3: Determine if we need a Switch (constructors), ListDecons, or Tests (literals).
    let has_list_cons = head_ctors
        .iter()
        .any(|c| matches!(c, HeadCtor::ListCons { .. } | HeadCtor::ListNil));
    let has_constructors = head_ctors
        .iter()
        .any(|c| matches!(c, HeadCtor::Constructor { .. }));

    if has_list_cons {
        compile_list_cons(&matrix, col, &head_ctors, file, line, sum_type_defs)
    } else if has_constructors {
        compile_constructor_switch(&matrix, col, &head_ctors, file, line, sum_type_defs)
    } else {
        compile_literal_tests(&matrix, col, &head_ctors, file, line, sum_type_defs)
    }
}

// ── Leaf / Guard creation ───────────────────────────────────────────

/// Create a Leaf node, possibly wrapping it in a Guard if the arm has a guard.
/// If the guard fails, matching continues with the remaining rows: `rest` is
/// the matrix without `row`, whose columns the rows have not been tested
/// against yet.
fn make_leaf_or_guard(
    row: &PatRow,
    rest: PatMatrix,
    file: &str,
    line: u32,
    sum_type_defs: &FxHashMap<String, MirSumTypeDef>,
) -> DecisionTree {
    let arm_index = row.arm_index;
    let bindings = row.bindings.clone();
    match &row.guard {
        Some(guard_expr) => DecisionTree::Guard {
            guard_expr: guard_expr.clone(),
            arm_index,
            bindings,
            failure: Box::new(compile_matrix(rest, file, line, sum_type_defs)),
        },
        None => DecisionTree::Leaf {
            arm_index,
            bindings,
        },
    }
}

// ── Row analysis ────────────────────────────────────────────────────

/// Check if all patterns in a row are wildcards or variables.
fn row_is_all_wildcards(row: &PatRow) -> bool {
    row.patterns
        .iter()
        .all(|p| matches!(p, MirPattern::Wildcard | MirPattern::Var(..)))
}

/// Check if a pattern is a wildcard or variable (matches anything).
fn is_wildcard_like(p: &MirPattern) -> bool {
    matches!(p, MirPattern::Wildcard | MirPattern::Var(..))
}

/// Collect variable bindings from all columns of a row into row.bindings.
fn collect_bindings_from_row(
    row: &mut PatRow,
    column_paths: &[AccessPath],
    column_types: &[MirType],
) {
    for (i, pat) in row.patterns.iter().enumerate() {
        if let MirPattern::Var(name, ty) = pat {
            let path = column_paths[i].clone();
            let bind_ty = if *ty == MirType::Unit {
                // If the pattern type is Unit (might be unresolved), use column type
                column_types[i].clone()
            } else {
                ty.clone()
            };
            row.bindings.push((name.clone(), bind_ty, path));
        }
    }
}

// ── Column selection ────────────────────────────────────────────────

/// Select the column with the most constructor diversity.
/// This heuristic produces better (smaller) decision trees.
fn select_column(matrix: &PatMatrix) -> usize {
    let score = |col: usize| {
        matrix
            .rows
            .iter()
            .filter_map(|row| head_ctor_key(&row.patterns[col]))
            .collect::<FxHashSet<_>>()
            .len()
    };
    // The first of the columns with the highest score.
    (0..matrix.column_paths.len())
        .rev()
        .max_by_key(|&col| score(col))
        .unwrap_or(0)
}

/// Get a unique string key for the head constructor of a pattern. (`As` is
/// peeled and `Or` expanded before a column is selected.)
fn head_ctor_key(p: &MirPattern) -> Option<String> {
    match p {
        MirPattern::Literal(lit) => Some(format!("lit:{}", literal_key(lit))),
        MirPattern::Constructor { variant, .. } => Some(format!("ctor:{}", variant)),
        MirPattern::Tuple(elems) => Some(format!("tuple:{}", elems.len())),
        MirPattern::Struct { name, .. } => Some(format!("struct:{name}")),
        MirPattern::ListCons { .. } => Some("list_cons".to_string()),
        MirPattern::ListNil => Some("list_nil".to_string()),
        MirPattern::As { .. } | MirPattern::Or(_) | MirPattern::Wildcard | MirPattern::Var(..) => {
            None
        }
    }
}

/// A literal's identity: two literals match the same values exactly when
/// their keys are equal (a float prints as the shortest text that reads
/// back as it, so `0.0` and `-0.0` differ).
fn literal_key(lit: &MirLiteral) -> String {
    match lit {
        MirLiteral::Int(n) => format!("int:{}", n),
        MirLiteral::Float(f) => format!("float:{}", f),
        MirLiteral::Bool(b) => format!("bool:{}", b),
        MirLiteral::String(s) => format!("str:{}", s),
    }
}

// ── Head constructor collection ─────────────────────────────────────

/// Collect all distinct head constructors from a column.
/// Uses `sum_type_defs` to look up the correct tag value from the type
/// definition rather than assigning tags by pattern appearance order.
fn collect_head_constructors(
    matrix: &PatMatrix,
    col: usize,
    sum_type_defs: &FxHashMap<String, MirSumTypeDef>,
) -> Vec<HeadCtor> {
    let mut result: Vec<HeadCtor> = Vec::new();
    let mut seen: Vec<String> = Vec::new();

    for row in &matrix.rows {
        let pattern = &row.patterns[col];
        let ctor = match pattern {
            MirPattern::Literal(lit) => HeadCtor::Literal(lit.clone()),
            MirPattern::Constructor {
                type_name,
                variant,
                fields,
                ..
            } => {
                // Look up the actual tag from the sum type definition.
                // This ensures tags match the type definition order, not
                // the order constructors appear in the user's pattern.
                let tag = by_sum_type_name(sum_type_defs, type_name)
                    .and_then(|def| def.variants.iter().find(|v| v.name == *variant))
                    .map(|v| v.tag)
                    .unwrap_or_else(|| {
                        // Fallback: count existing constructors (old behavior)
                        // if type not found in sum_type_defs map.
                        result
                            .iter()
                            .filter(|c| matches!(c, HeadCtor::Constructor { .. }))
                            .count() as u8
                    });
                HeadCtor::Constructor {
                    type_name: type_name.clone(),
                    variant: variant.clone(),
                    tag,
                    arity: fields.len(),
                }
            }
            MirPattern::ListCons { elem_ty, .. } => HeadCtor::ListCons {
                elem_ty: elem_ty.clone(),
            },
            MirPattern::ListNil => HeadCtor::ListNil,
            // Wildcards and variables match every constructor. (A column of
            // tuples or structs is taken apart before it gets here.)
            _ => continue,
        };
        let key = head_ctor_key(pattern).unwrap_or_default();
        if !seen.contains(&key) {
            seen.push(key);
            result.push(ctor);
        }
    }

    result
}

// ── Constructor switch compilation ──────────────────────────────────

/// Compile a Switch node for constructor patterns.
fn compile_constructor_switch(
    matrix: &PatMatrix,
    col: usize,
    head_ctors: &[HeadCtor],
    file: &str,
    line: u32,
    sum_type_defs: &FxHashMap<String, MirSumTypeDef>,
) -> DecisionTree {
    let scrutinee_path = matrix.column_paths[col].clone();

    let mut cases = Vec::new();

    for hc in head_ctors {
        if let HeadCtor::Constructor {
            type_name,
            variant,
            tag,
            arity,
        } = hc
        {
            let ctor_tag = ConstructorTag {
                type_name: type_name.clone(),
                variant_name: variant.clone(),
                tag: *tag,
                arity: *arity,
            };

            // Specialize matrix for this constructor.
            let specialized =
                specialize_for_constructor(matrix, col, type_name, variant, *arity, sum_type_defs);
            let subtree = compile_matrix(specialized, file, line, sum_type_defs);
            cases.push((ctor_tag, subtree));
        }
    }

    // Build default branch from rows with wildcard/variable in this column.
    let default_matrix = default_matrix(matrix, col);
    let default = if default_matrix.rows.is_empty() {
        None
    } else {
        Some(Box::new(compile_matrix(
            default_matrix,
            file,
            line,
            sum_type_defs,
        )))
    };

    DecisionTree::Switch {
        scrutinee_path,
        cases,
        default,
    }
}

/// Specialize the matrix for a specific constructor.
/// Rows matching the constructor have their sub-patterns expanded as new columns.
/// Rows with wildcards/variables in this column are kept with wildcard sub-patterns.
fn specialize_for_constructor(
    matrix: &PatMatrix,
    col: usize,
    type_name: &str,
    target_variant: &str,
    arity: usize,
    sum_type_defs: &FxHashMap<String, MirSumTypeDef>,
) -> PatMatrix {
    // Look up actual field types from the sum type definition.
    let mut field_types = by_sum_type_name(sum_type_defs, type_name)
        .and_then(|def| def.variants.iter().find(|v| v.name == target_variant))
        .map(|v| v.fields.clone())
        .unwrap_or_else(|| vec![MirType::Unit; arity]);

    // Generic sum definitions use pointer storage for their type parameters.
    // Recover the concrete semantic type from the already type-checked pattern
    // so nested matches (for example `Err(InvalidLength(...))`) retain enough
    // information to dereference the boxed payload and switch on its own tag.
    for row in &matrix.rows {
        let MirPattern::Constructor {
            variant, fields, ..
        } = &row.patterns[col]
        else {
            continue;
        };
        if variant != target_variant {
            continue;
        }
        for (field_ty, field_pattern) in field_types.iter_mut().zip(fields) {
            if let Some(concrete_ty) = pattern_type_hint(field_pattern) {
                if (matches!(&*field_ty, MirType::Ptr) && !matches!(&concrete_ty, MirType::Ptr))
                    || (matches!(&*field_ty, MirType::Unit)
                        && !matches!(&concrete_ty, MirType::Unit))
                {
                    *field_ty = concrete_ty;
                }
            }
        }
    }

    // Sub-pattern paths for the constructor fields.
    let parent_path = &matrix.column_paths[col];
    let columns = field_types
        .into_iter()
        .enumerate()
        .map(|(index, ty)| {
            let path = AccessPath::VariantField {
                parent: Box::new(parent_path.clone()),
                type_name: type_name.to_string(),
                variant: target_variant.to_string(),
                index,
                ty: ty.clone(),
            };
            (path, ty)
        })
        .collect();

    replace_column(matrix, col, columns, |pattern| match pattern {
        MirPattern::Constructor {
            variant, fields, ..
        } if variant == target_variant => {
            // Variable bindings are carried by the sub-patterns themselves and
            // are collected when those are processed, not from the
            // constructor's `bindings`, which would count them twice.
            Some(fields.clone())
        }
        MirPattern::Wildcard | MirPattern::Var(..) => Some(vec![MirPattern::Wildcard; arity]),
        // Different constructor -- skip this row.
        _ => None,
    })
}

// ── List cons compilation ────────────────────────────────────────────

/// Compile a ListDecons node for list cons patterns.
///
/// A `head :: tail` pattern tests if the list is non-empty, then extracts
/// the head element and tail list as two new columns for further matching.
fn compile_list_cons(
    matrix: &PatMatrix,
    col: usize,
    head_ctors: &[HeadCtor],
    file: &str,
    line: u32,
    sum_type_defs: &FxHashMap<String, MirSumTypeDef>,
) -> DecisionTree {
    let scrutinee_path = matrix.column_paths[col].clone();

    // Extract element type from the ListCons head constructor.
    let elem_ty = head_ctors
        .iter()
        .find_map(|hc| {
            if let HeadCtor::ListCons { elem_ty } = hc {
                Some(elem_ty.clone())
            } else {
                None
            }
        })
        .unwrap_or(MirType::Int);

    // Specialize: rows with ListCons get head/tail expanded as new columns;
    // `[]` rows cannot match a non-empty list and drop out.
    let specialized = specialize_for_list_cons(matrix, col, &elem_ty);
    let non_empty = compile_matrix(specialized, file, line, sum_type_defs);

    // Empty: rows with `[]` or a wildcard/variable, the column consumed.
    let default_mat = default_matrix_with(matrix, col, |p| matches!(p, MirPattern::ListNil));
    let empty = compile_matrix(default_mat, file, line, sum_type_defs);

    DecisionTree::ListDecons {
        scrutinee_path,
        elem_ty,
        non_empty: Box::new(non_empty),
        empty: Box::new(empty),
    }
}

/// Specialize the matrix for list cons patterns.
///
/// Rows with ListCons patterns have head/tail expanded as two new columns.
/// Rows with wildcards/variables are kept with wildcard sub-patterns for head/tail.
fn specialize_for_list_cons(matrix: &PatMatrix, col: usize, elem_ty: &MirType) -> PatMatrix {
    let parent_path = &matrix.column_paths[col];
    let columns = vec![
        (
            AccessPath::ListHead(Box::new(parent_path.clone()), elem_ty.clone()),
            elem_ty.clone(),
        ),
        // The tail is always a list (Ptr).
        (
            AccessPath::ListTail(Box::new(parent_path.clone())),
            MirType::Ptr,
        ),
    ];
    replace_column(matrix, col, columns, |pattern| match pattern {
        MirPattern::ListCons { head, tail, .. } => Some(vec![(**head).clone(), (**tail).clone()]),
        MirPattern::Wildcard | MirPattern::Var(..) => {
            Some(vec![MirPattern::Wildcard, MirPattern::Wildcard])
        }
        // `[]` cannot match a non-empty list.
        _ => None,
    })
}

// ── Literal test compilation ────────────────────────────────────────

/// Compile a chain of Test nodes for literal patterns.
fn compile_literal_tests(
    matrix: &PatMatrix,
    col: usize,
    head_ctors: &[HeadCtor],
    file: &str,
    line: u32,
    sum_type_defs: &FxHashMap<String, MirSumTypeDef>,
) -> DecisionTree {
    let scrutinee_path = matrix.column_paths[col].clone();

    // Build a chain of Test nodes, one per literal value.
    // For each literal, the success branch handles rows matching that literal,
    // and the failure branch continues to test the next literal.
    let mut literals: Vec<MirLiteral> = Vec::new();
    for hc in head_ctors {
        if let HeadCtor::Literal(lit) = hc {
            literals.push(lit.clone());
        }
    }

    // Build the chain from the last literal to the first.
    // The final failure is the default matrix (rows with wildcards).
    let default_mat = default_matrix(matrix, col);
    let mut failure_tree = compile_matrix(default_mat, file, line, sum_type_defs);

    // Build from last to first to create the chain.
    for lit in literals.iter().rev() {
        let specialized = specialize_for_literal(matrix, col, lit);
        let success_tree = compile_matrix(specialized, file, line, sum_type_defs);

        failure_tree = DecisionTree::Test {
            scrutinee_path: scrutinee_path.clone(),
            value: lit.clone(),
            success: Box::new(success_tree),
            failure: Box::new(failure_tree),
        };
    }

    failure_tree
}

/// Specialize the matrix for a specific literal value.
fn specialize_for_literal(matrix: &PatMatrix, col: usize, target_lit: &MirLiteral) -> PatMatrix {
    let target = literal_key(target_lit);
    default_matrix_with(
        matrix,
        col,
        |pattern| matches!(pattern, MirPattern::Literal(lit) if literal_key(lit) == target),
    )
}

// ── Default matrix ──────────────────────────────────────────────────

/// Build the default matrix: rows with wildcard/variable in the given column,
/// with that column removed.
fn default_matrix(matrix: &PatMatrix, col: usize) -> PatMatrix {
    default_matrix_with(matrix, col, |_| false)
}

/// The default matrix, also keeping rows whose pattern in `col` satisfies
/// `also` (a pattern the taken branch is known to match, like `[]` on the
/// empty branch).
fn default_matrix_with(
    matrix: &PatMatrix,
    col: usize,
    also: impl Fn(&MirPattern) -> bool,
) -> PatMatrix {
    replace_column(matrix, col, Vec::new(), |pattern| {
        (is_wildcard_like(pattern) || also(pattern)).then(Vec::new)
    })
}

/// `matrix` with column `col` replaced by `columns` (their paths and types),
/// placed first. `row_patterns` gives, for a row's pattern in `col`, the
/// row's patterns for the new columns, or `None` to leave the row out. A
/// variable in `col` binds the column's whole value.
fn replace_column(
    matrix: &PatMatrix,
    col: usize,
    columns: Vec<(AccessPath, MirType)>,
    row_patterns: impl Fn(&MirPattern) -> Option<Vec<MirPattern>>,
) -> PatMatrix {
    let rows = matrix
        .rows
        .iter()
        .filter_map(|row| {
            let pattern = &row.patterns[col];
            let mut patterns = row_patterns(pattern)?;
            let mut bindings = row.bindings.clone();
            if let MirPattern::Var(name, ty) = pattern {
                bindings.push((name.clone(), ty.clone(), matrix.column_paths[col].clone()));
            }
            patterns.extend(without(&row.patterns, col));
            Some(PatRow {
                patterns,
                arm_index: row.arm_index,
                guard: row.guard.clone(),
                bindings,
            })
        })
        .collect();
    let (mut column_paths, mut column_types): (Vec<_>, Vec<_>) = columns.into_iter().unzip();
    column_paths.extend(without(&matrix.column_paths, col));
    column_types.extend(without(&matrix.column_types, col));
    PatMatrix {
        rows,
        column_paths,
        column_types,
    }
}

/// The items of `items` other than the one at `index`.
fn without<T: Clone>(items: &[T], index: usize) -> impl Iterator<Item = T> + '_ {
    items
        .iter()
        .enumerate()
        .filter(move |(i, _)| *i != index)
        .map(|(_, item)| item.clone())
}

// ── Tuple and struct expansion ──────────────────────────────────────

/// Recover the concrete runtime type represented by a sub-pattern.
///
/// Generic constructor layouts intentionally erase payloads to `Ptr`. Tuple
/// patterns still retain enough type information in their elements for codegen
/// to unpack the heap-backed runtime tuple correctly. (Or-patterns are
/// expanded before any column is taken apart.)
fn pattern_type_hint(pattern: &MirPattern) -> Option<MirType> {
    match pattern {
        MirPattern::Var(_, ty) => Some(ty.clone()),
        MirPattern::Literal(MirLiteral::Int(_)) => Some(MirType::Int),
        MirPattern::Literal(MirLiteral::Float(_)) => Some(MirType::Float),
        MirPattern::Literal(MirLiteral::Bool(_)) => Some(MirType::Bool),
        MirPattern::Literal(MirLiteral::String(_)) => Some(MirType::String),
        MirPattern::Constructor { type_name, .. } => Some(MirType::SumType(type_name.clone())),
        MirPattern::Struct { name, .. } => Some(MirType::Struct(name.clone())),
        // Tuple values use the heap-backed runtime representation.
        MirPattern::Tuple(_) | MirPattern::ListCons { .. } | MirPattern::ListNil => {
            Some(MirType::Ptr)
        }
        MirPattern::As { inner, .. } => pattern_type_hint(inner),
        MirPattern::Or(_) | MirPattern::Wildcard => None,
    }
}

/// The columns (paths and types) a column of tuple or struct patterns is
/// taken apart into, one per element or field, or `None` when the column
/// holds no tuple or struct pattern.
fn product_columns(matrix: &PatMatrix, col: usize) -> Option<Vec<(AccessPath, MirType)>> {
    let parent_path = &matrix.column_paths[col];
    matrix.rows.iter().find_map(|row| match &row.patterns[col] {
        MirPattern::Struct { name, fields } => Some(
            fields
                .iter()
                .enumerate()
                .map(|(index, (_, ty, _))| {
                    let path = AccessPath::StructField {
                        parent: Box::new(parent_path.clone()),
                        name: name.clone(),
                        index,
                        ty: ty.clone(),
                    };
                    (path, ty.clone())
                })
                .collect(),
        ),
        MirPattern::Tuple(elements) => Some(
            (0..elements.len())
                .map(|index| {
                    let ty = match &matrix.column_types[col] {
                        MirType::Tuple(elements) => elements.get(index).cloned(),
                        _ => matrix.rows.iter().find_map(|row| match &row.patterns[col] {
                            MirPattern::Tuple(elements) => {
                                elements.get(index).and_then(pattern_type_hint)
                            }
                            _ => None,
                        }),
                    }
                    .unwrap_or(MirType::Unit);
                    (
                        AccessPath::TupleField(Box::new(parent_path.clone()), index, ty.clone()),
                        ty,
                    )
                })
                .collect(),
        ),
        _ => None,
    })
}

/// Expand a tuple or struct column into `sub_columns`, one per element or
/// field. Tuple and struct patterns become their sub-patterns; wildcards and
/// variables become wildcards (a variable binds the whole value). `()` and a
/// struct without fields have nothing to take apart: the column goes.
fn expand_product_column(
    matrix: &PatMatrix,
    col: usize,
    sub_columns: Vec<(AccessPath, MirType)>,
) -> PatMatrix {
    let arity = sub_columns.len();
    replace_column(matrix, col, sub_columns, |pattern| {
        Some(match pattern {
            MirPattern::Tuple(elements) => elements.clone(),
            MirPattern::Struct { fields, .. } => fields
                .iter()
                .map(|(_, _, pattern)| pattern.clone())
                .collect(),
            _ => vec![MirPattern::Wildcard; arity],
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir::{MirLiteral, MirMatchArm, MirPattern, MirSumTypeDef, MirType, MirVariantDef};
    use crate::pattern::{AccessPath, DecisionTree};
    use rustc_hash::FxHashMap;

    // ── Helper functions ─────────────────────────────────────────────

    fn make_arm(pattern: MirPattern, guard: Option<MirExpr>, body: MirExpr) -> MirMatchArm {
        MirMatchArm {
            pattern,
            guard,
            body,
        }
    }

    fn int_body(n: i64) -> MirExpr {
        MirExpr::IntLit(n, MirType::Int)
    }

    fn string_body(s: &str) -> MirExpr {
        MirExpr::StringLit(s.to_string(), MirType::String)
    }

    fn var_expr(name: &str, ty: MirType) -> MirExpr {
        MirExpr::Var(name.to_string(), ty)
    }

    // ── Test 1: Single wildcard arm ──────────────────────────────────

    #[test]
    fn test_wildcard_arm() {
        // match x { _ -> 1 }
        // Expected: Leaf { arm_index: 0, bindings: [] }
        let arms = vec![make_arm(MirPattern::Wildcard, None, int_body(1))];

        let tree = compile_match(&MirType::Int, &arms, "test.mpl", 1, &FxHashMap::default());

        match tree {
            DecisionTree::Leaf {
                arm_index,
                bindings,
            } => {
                assert_eq!(arm_index, 0);
                assert!(bindings.is_empty());
            }
            other => panic!("Expected Leaf, got {:?}", other),
        }
    }

    // ── Test 2: Variable binding ─────────────────────────────────────

    #[test]
    fn test_variable_binding() {
        // match x { y -> y }
        // Expected: Leaf { arm_index: 0, bindings: [(y, Int, Root)] }
        let arms = vec![make_arm(
            MirPattern::Var("y".to_string(), MirType::Int),
            None,
            var_expr("y", MirType::Int),
        )];

        let tree = compile_match(&MirType::Int, &arms, "test.mpl", 1, &FxHashMap::default());

        match tree {
            DecisionTree::Leaf {
                arm_index,
                bindings,
            } => {
                assert_eq!(arm_index, 0);
                assert_eq!(bindings.len(), 1);
                assert_eq!(bindings[0].0, "y");
                assert_eq!(bindings[0].1, MirType::Int);
                assert_eq!(bindings[0].2, AccessPath::Root);
            }
            other => panic!("Expected Leaf, got {:?}", other),
        }
    }

    // ── Test 3: Integer literal tests ────────────────────────────────

    #[test]
    fn test_integer_literals() {
        // match x { 1 -> "one", 2 -> "two", _ -> "other" }
        // Expected: Test(Root, 1, Leaf(0), Test(Root, 2, Leaf(1), Leaf(2)))
        let arms = vec![
            make_arm(
                MirPattern::Literal(MirLiteral::Int(1)),
                None,
                string_body("one"),
            ),
            make_arm(
                MirPattern::Literal(MirLiteral::Int(2)),
                None,
                string_body("two"),
            ),
            make_arm(MirPattern::Wildcard, None, string_body("other")),
        ];

        let tree = compile_match(&MirType::Int, &arms, "test.mpl", 1, &FxHashMap::default());

        // Should be Test(Root, 1, Leaf(0), Test(Root, 2, Leaf(1), Leaf(2)))
        match &tree {
            DecisionTree::Test {
                scrutinee_path,
                value,
                success,
                failure,
            } => {
                assert_eq!(*scrutinee_path, AccessPath::Root);
                assert!(matches!(value, MirLiteral::Int(1)));
                assert!(matches!(
                    success.as_ref(),
                    DecisionTree::Leaf { arm_index: 0, .. }
                ));
                // failure should be another Test for literal 2
                match failure.as_ref() {
                    DecisionTree::Test {
                        scrutinee_path: path2,
                        value: val2,
                        success: s2,
                        failure: f2,
                    } => {
                        assert_eq!(*path2, AccessPath::Root);
                        assert!(matches!(val2, MirLiteral::Int(2)));
                        assert!(matches!(
                            s2.as_ref(),
                            DecisionTree::Leaf { arm_index: 1, .. }
                        ));
                        assert!(matches!(
                            f2.as_ref(),
                            DecisionTree::Leaf { arm_index: 2, .. }
                        ));
                    }
                    other => panic!("Expected nested Test, got {:?}", other),
                }
            }
            other => panic!("Expected Test, got {:?}", other),
        }
    }

    #[test]
    fn test_as_pattern_binds_the_whole_value() {
        // match x { 1 as whole -> whole, _ -> 0 }
        // Expected: Test(Root, 1, Leaf(0, [whole @ Root]), Leaf(1))
        let arms = vec![
            make_arm(
                MirPattern::As {
                    name: "whole".to_string(),
                    ty: MirType::Unit,
                    inner: Box::new(MirPattern::Literal(MirLiteral::Int(1))),
                },
                None,
                var_expr("whole", MirType::Int),
            ),
            make_arm(MirPattern::Wildcard, None, int_body(0)),
        ];

        let tree = compile_match(&MirType::Int, &arms, "test.mpl", 1, &FxHashMap::default());

        match &tree {
            DecisionTree::Test { value, success, .. } => {
                assert!(matches!(value, MirLiteral::Int(1)));
                match success.as_ref() {
                    DecisionTree::Leaf {
                        arm_index: 0,
                        bindings,
                    } => {
                        // An unresolved (Unit) binding type falls back to the column type.
                        assert_eq!(
                            bindings,
                            &vec![("whole".to_string(), MirType::Int, AccessPath::Root)]
                        );
                    }
                    other => panic!("Expected Leaf for arm 0, got {:?}", other),
                }
            }
            other => panic!("Expected Test, got {:?}", other),
        }
    }

    // ── Test 4: Boolean literal tests ────────────────────────────────

    #[test]
    fn test_boolean_literals() {
        // match flag { true -> 1, false -> 0 }
        // Expected: Test(Root, Bool(true), Leaf(0), Test(Root, Bool(false), Leaf(1), Fail))
        // The compiler creates a test chain for each literal; the last failure
        // is a Fail node (since there are no wildcard default rows).
        let arms = vec![
            make_arm(
                MirPattern::Literal(MirLiteral::Bool(true)),
                None,
                int_body(1),
            ),
            make_arm(
                MirPattern::Literal(MirLiteral::Bool(false)),
                None,
                int_body(0),
            ),
        ];

        let tree = compile_match(&MirType::Bool, &arms, "test.mpl", 1, &FxHashMap::default());

        match &tree {
            DecisionTree::Test {
                scrutinee_path,
                value,
                success,
                failure,
            } => {
                assert_eq!(*scrutinee_path, AccessPath::Root);
                assert!(matches!(value, MirLiteral::Bool(true)));
                assert!(matches!(
                    success.as_ref(),
                    DecisionTree::Leaf { arm_index: 0, .. }
                ));
                // Second Test for false literal
                match failure.as_ref() {
                    DecisionTree::Test {
                        value: v2,
                        success: s2,
                        ..
                    } => {
                        assert!(matches!(v2, MirLiteral::Bool(false)));
                        assert!(matches!(
                            s2.as_ref(),
                            DecisionTree::Leaf { arm_index: 1, .. }
                        ));
                    }
                    other => panic!("Expected nested Test for false, got {:?}", other),
                }
            }
            other => panic!("Expected Test, got {:?}", other),
        }
    }

    // ── Test 5: Constructor switch ───────────────────────────────────

    #[test]
    fn test_constructor_switch() {
        // match shape { Circle(r) -> r, Rectangle(w, h) -> w * h }
        // Expected: Switch(Root, [(Circle/0, Leaf(0, [(r, Float, VariantField(Root, "Circle", 0))])),
        //                         (Rectangle/1, Leaf(1, [...]))])
        let arms = vec![
            make_arm(
                MirPattern::Constructor {
                    type_name: "Shape".to_string(),
                    variant: "Circle".to_string(),
                    fields: vec![MirPattern::Var("r".to_string(), MirType::Float)],
                    bindings: vec![("r".to_string(), MirType::Float)],
                },
                None,
                var_expr("r", MirType::Float),
            ),
            make_arm(
                MirPattern::Constructor {
                    type_name: "Shape".to_string(),
                    variant: "Rectangle".to_string(),
                    fields: vec![
                        MirPattern::Var("w".to_string(), MirType::Float),
                        MirPattern::Var("h".to_string(), MirType::Float),
                    ],
                    bindings: vec![
                        ("w".to_string(), MirType::Float),
                        ("h".to_string(), MirType::Float),
                    ],
                },
                None,
                var_expr("w", MirType::Float),
            ),
        ];

        let tree = compile_match(
            &MirType::SumType("Shape".to_string()),
            &arms,
            "test.mpl",
            1,
            &FxHashMap::default(),
        );

        match &tree {
            DecisionTree::Switch {
                scrutinee_path,
                cases,
                default,
            } => {
                assert_eq!(*scrutinee_path, AccessPath::Root);
                assert_eq!(cases.len(), 2);

                // First case: Circle
                assert_eq!(cases[0].0.variant_name, "Circle");
                assert_eq!(cases[0].0.tag, 0);
                match &cases[0].1 {
                    DecisionTree::Leaf {
                        arm_index,
                        bindings,
                    } => {
                        assert_eq!(*arm_index, 0);
                        assert_eq!(bindings.len(), 1);
                        assert_eq!(bindings[0].0, "r");
                        assert_eq!(
                            bindings[0].2,
                            AccessPath::VariantField {
                                parent: Box::new(AccessPath::Root),
                                type_name: "Shape".to_string(),
                                variant: "Circle".to_string(),
                                index: 0,
                                ty: MirType::Float,
                            }
                        );
                    }
                    other => panic!("Expected Leaf for Circle, got {:?}", other),
                }

                // Second case: Rectangle
                assert_eq!(cases[1].0.variant_name, "Rectangle");
                assert_eq!(cases[1].0.tag, 1);
                match &cases[1].1 {
                    DecisionTree::Leaf {
                        arm_index,
                        bindings,
                    } => {
                        assert_eq!(*arm_index, 1);
                        assert_eq!(bindings.len(), 2);
                        assert_eq!(bindings[0].0, "w");
                        assert_eq!(bindings[1].0, "h");
                    }
                    other => panic!("Expected Leaf for Rectangle, got {:?}", other),
                }

                assert!(default.is_none());
            }
            other => panic!("Expected Switch, got {:?}", other),
        }
    }

    // ── Test 6: Nested patterns (tuple + constructor) ────────────────

    #[test]
    fn test_nested_tuple_constructor() {
        // match pair { (Some(x), _) -> x, (None, y) -> y }
        // Expected: Switch on TupleField(Root, 0) for Some/None tags
        let arms = vec![
            make_arm(
                MirPattern::Tuple(vec![
                    MirPattern::Constructor {
                        type_name: "Option".to_string(),
                        variant: "Some".to_string(),
                        fields: vec![MirPattern::Var("x".to_string(), MirType::Int)],
                        bindings: vec![("x".to_string(), MirType::Int)],
                    },
                    MirPattern::Wildcard,
                ]),
                None,
                var_expr("x", MirType::Int),
            ),
            make_arm(
                MirPattern::Tuple(vec![
                    MirPattern::Constructor {
                        type_name: "Option".to_string(),
                        variant: "None".to_string(),
                        fields: vec![],
                        bindings: vec![],
                    },
                    MirPattern::Var("y".to_string(), MirType::Int),
                ]),
                None,
                var_expr("y", MirType::Int),
            ),
        ];

        let scrutinee_ty =
            MirType::Tuple(vec![MirType::SumType("Option".to_string()), MirType::Int]);
        let tree = compile_match(&scrutinee_ty, &arms, "test.mpl", 1, &FxHashMap::default());

        // The tree should switch on TupleField(Root, 0) for constructor tag
        match &tree {
            DecisionTree::Switch {
                scrutinee_path,
                cases,
                ..
            } => {
                assert_eq!(
                    *scrutinee_path,
                    AccessPath::TupleField(
                        Box::new(AccessPath::Root),
                        0,
                        MirType::SumType("Option".to_string())
                    )
                );
                assert!(cases.len() >= 2);

                // Some case should bind x from inside the variant
                let some_case = cases
                    .iter()
                    .find(|(tag, _)| tag.variant_name == "Some")
                    .expect("Should have Some case");
                match &some_case.1 {
                    DecisionTree::Leaf {
                        arm_index,
                        bindings,
                    } => {
                        assert_eq!(*arm_index, 0);
                        // Should bind x from VariantField(TupleField(Root, 0), "Some", 0)
                        let x_binding = bindings.iter().find(|(name, _, _)| name == "x");
                        assert!(x_binding.is_some(), "Should bind x");
                    }
                    other => panic!("Expected Leaf for Some case, got {:?}", other),
                }

                // None case should bind y from TupleField(Root, 1)
                let none_case = cases
                    .iter()
                    .find(|(tag, _)| tag.variant_name == "None")
                    .expect("Should have None case");
                match &none_case.1 {
                    DecisionTree::Leaf {
                        arm_index,
                        bindings,
                    } => {
                        assert_eq!(*arm_index, 1);
                        let y_binding = bindings.iter().find(|(name, _, _)| name == "y");
                        assert!(y_binding.is_some(), "Should bind y");
                    }
                    other => panic!("Expected Leaf for None case, got {:?}", other),
                }
            }
            other => panic!("Expected Switch on tuple field, got {:?}", other),
        }
    }

    // ── Test 7: Or-patterns (duplicate arms) ─────────────────────────

    #[test]
    fn test_or_pattern_duplicates_arms() {
        // match x { 1 | 2 -> "small", _ -> "big" }
        // Expected: Test(Root, 1, Leaf(0), Test(Root, 2, Leaf(0_dup), Leaf(1)))
        // Both literal matches should point to arm_index 0
        let arms = vec![
            make_arm(
                MirPattern::Or(vec![
                    MirPattern::Literal(MirLiteral::Int(1)),
                    MirPattern::Literal(MirLiteral::Int(2)),
                ]),
                None,
                string_body("small"),
            ),
            make_arm(MirPattern::Wildcard, None, string_body("big")),
        ];

        let tree = compile_match(&MirType::Int, &arms, "test.mpl", 1, &FxHashMap::default());

        // Should be Test(Root, 1, Leaf(0), Test(Root, 2, Leaf(0), Leaf(1)))
        match &tree {
            DecisionTree::Test {
                value,
                success,
                failure,
                ..
            } => {
                assert!(matches!(value, MirLiteral::Int(1)));
                match success.as_ref() {
                    DecisionTree::Leaf { arm_index, .. } => assert_eq!(*arm_index, 0),
                    other => panic!("Expected Leaf(0) for first or-alt, got {:?}", other),
                }
                match failure.as_ref() {
                    DecisionTree::Test {
                        value: v2,
                        success: s2,
                        failure: f2,
                        ..
                    } => {
                        assert!(matches!(v2, MirLiteral::Int(2)));
                        match s2.as_ref() {
                            DecisionTree::Leaf { arm_index, .. } => {
                                assert_eq!(*arm_index, 0, "Or-pattern should share arm_index 0")
                            }
                            other => {
                                panic!("Expected Leaf(0) for second or-alt, got {:?}", other)
                            }
                        }
                        match f2.as_ref() {
                            DecisionTree::Leaf { arm_index, .. } => assert_eq!(*arm_index, 1),
                            other => panic!("Expected Leaf(1) for default, got {:?}", other),
                        }
                    }
                    other => panic!("Expected nested Test for second literal, got {:?}", other),
                }
            }
            other => panic!("Expected Test at root, got {:?}", other),
        }
    }

    // ── Test 8: Guard expression ─────────────────────────────────────

    #[test]
    fn test_guard_expression() {
        // match x { n when n > 0 -> "positive", _ -> "non-positive" }
        // Expected: Guard(guard_expr, Leaf(0, [(n, Int, Root)]), Leaf(1))
        let guard = MirExpr::BinOp {
            op: crate::mir::BinOp::Gt,
            lhs: Box::new(var_expr("n", MirType::Int)),
            rhs: Box::new(int_body(0)),
            ty: MirType::Bool,
        };

        let arms = vec![
            make_arm(
                MirPattern::Var("n".to_string(), MirType::Int),
                Some(guard),
                string_body("positive"),
            ),
            make_arm(MirPattern::Wildcard, None, string_body("non-positive")),
        ];

        let tree = compile_match(&MirType::Int, &arms, "test.mpl", 1, &FxHashMap::default());

        match &tree {
            DecisionTree::Guard {
                arm_index,
                bindings,
                failure,
                ..
            } => {
                assert_eq!(*arm_index, 0);
                assert_eq!(bindings.len(), 1);
                assert_eq!(bindings[0].0, "n");
                assert_eq!(bindings[0].2, AccessPath::Root);
                match failure.as_ref() {
                    DecisionTree::Leaf { arm_index, .. } => {
                        assert_eq!(*arm_index, 1);
                    }
                    other => panic!("Expected Leaf for guard failure, got {:?}", other),
                }
            }
            other => panic!("Expected Guard, got {:?}", other),
        }
    }

    // ── Test 9: Guard with Fail fallback ─────────────────────────────

    #[test]
    fn test_guard_with_fail_fallback() {
        // match x { n when n > 0 -> "positive" }
        // Only guarded arm, no default => Fail node on guard failure
        let guard = MirExpr::BinOp {
            op: crate::mir::BinOp::Gt,
            lhs: Box::new(var_expr("n", MirType::Int)),
            rhs: Box::new(int_body(0)),
            ty: MirType::Bool,
        };

        let arms = vec![make_arm(
            MirPattern::Var("n".to_string(), MirType::Int),
            Some(guard),
            string_body("positive"),
        )];

        let tree = compile_match(&MirType::Int, &arms, "test.mpl", 1, &FxHashMap::default());

        match &tree {
            DecisionTree::Guard {
                arm_index, failure, ..
            } => {
                assert_eq!(*arm_index, 0);
                match failure.as_ref() {
                    DecisionTree::Fail { message, .. } => {
                        assert!(message.contains("non-exhaustive"));
                    }
                    other => panic!("Expected Fail for guard failure, got {:?}", other),
                }
            }
            other => panic!("Expected Guard, got {:?}", other),
        }
    }

    // ── Test 10: Constructor with wildcard default ────────────────────

    #[test]
    fn test_constructor_with_wildcard_default() {
        // match opt { Some(x) -> x, _ -> 0 }
        // Expected: Switch(Root, [(Some, Leaf(0, [x]))], default=Leaf(1))
        let arms = vec![
            make_arm(
                MirPattern::Constructor {
                    type_name: "Option".to_string(),
                    variant: "Some".to_string(),
                    fields: vec![MirPattern::Var("x".to_string(), MirType::Int)],
                    bindings: vec![("x".to_string(), MirType::Int)],
                },
                None,
                var_expr("x", MirType::Int),
            ),
            make_arm(MirPattern::Wildcard, None, int_body(0)),
        ];

        let tree = compile_match(
            &MirType::SumType("Option".to_string()),
            &arms,
            "test.mpl",
            1,
            &FxHashMap::default(),
        );

        match &tree {
            DecisionTree::Switch {
                scrutinee_path,
                cases,
                default,
            } => {
                assert_eq!(*scrutinee_path, AccessPath::Root);
                assert_eq!(cases.len(), 1);
                assert_eq!(cases[0].0.variant_name, "Some");
                match &cases[0].1 {
                    DecisionTree::Leaf {
                        arm_index,
                        bindings,
                    } => {
                        assert_eq!(*arm_index, 0);
                        assert_eq!(bindings.len(), 1);
                        assert_eq!(bindings[0].0, "x");
                    }
                    other => panic!("Expected Leaf for Some, got {:?}", other),
                }
                assert!(default.is_some());
                match default.as_ref().unwrap().as_ref() {
                    DecisionTree::Leaf { arm_index, .. } => {
                        assert_eq!(*arm_index, 1);
                    }
                    other => panic!("Expected Leaf for default, got {:?}", other),
                }
            }
            other => panic!("Expected Switch, got {:?}", other),
        }
    }

    // ── Test 11: Tuple pattern ───────────────────────────────────────

    #[test]
    fn test_tuple_pattern() {
        // match pair { (1, y) -> y, (x, 2) -> x, _ -> 0 }
        // Expected: Tests on TupleField(Root, 0) and TupleField(Root, 1)
        let arms = vec![
            make_arm(
                MirPattern::Tuple(vec![
                    MirPattern::Literal(MirLiteral::Int(1)),
                    MirPattern::Var("y".to_string(), MirType::Int),
                ]),
                None,
                var_expr("y", MirType::Int),
            ),
            make_arm(
                MirPattern::Tuple(vec![
                    MirPattern::Var("x".to_string(), MirType::Int),
                    MirPattern::Literal(MirLiteral::Int(2)),
                ]),
                None,
                var_expr("x", MirType::Int),
            ),
            make_arm(MirPattern::Wildcard, None, int_body(0)),
        ];

        let scrutinee_ty = MirType::Tuple(vec![MirType::Int, MirType::Int]);
        let tree = compile_match(&scrutinee_ty, &arms, "test.mpl", 1, &FxHashMap::default());

        // Should test tuple fields, not Root
        fn contains_tuple_field_test(tree: &DecisionTree) -> bool {
            match tree {
                DecisionTree::Test {
                    scrutinee_path,
                    failure,
                    success,
                    ..
                } => {
                    matches!(scrutinee_path, AccessPath::TupleField(_, _, _))
                        || contains_tuple_field_test(success)
                        || contains_tuple_field_test(failure)
                }
                DecisionTree::Switch {
                    scrutinee_path,
                    cases,
                    default,
                    ..
                } => {
                    matches!(scrutinee_path, AccessPath::TupleField(_, _, _))
                        || cases.iter().any(|(_, t)| contains_tuple_field_test(t))
                        || default
                            .as_ref()
                            .is_some_and(|d| contains_tuple_field_test(d))
                }
                DecisionTree::Guard { failure, .. } => contains_tuple_field_test(failure),
                _ => false,
            }
        }

        assert!(
            contains_tuple_field_test(&tree),
            "Decision tree should test tuple fields, got {:?}",
            tree
        );
    }

    // ── Test 12: String literal test ─────────────────────────────────

    #[test]
    fn test_string_literals() {
        // match s { "hello" -> 1, "world" -> 2, _ -> 0 }
        let arms = vec![
            make_arm(
                MirPattern::Literal(MirLiteral::String("hello".to_string())),
                None,
                int_body(1),
            ),
            make_arm(
                MirPattern::Literal(MirLiteral::String("world".to_string())),
                None,
                int_body(2),
            ),
            make_arm(MirPattern::Wildcard, None, int_body(0)),
        ];

        let tree = compile_match(
            &MirType::String,
            &arms,
            "test.mpl",
            1,
            &FxHashMap::default(),
        );

        match &tree {
            DecisionTree::Test {
                scrutinee_path,
                value,
                ..
            } => {
                assert_eq!(*scrutinee_path, AccessPath::Root);
                match value {
                    MirLiteral::String(s) => assert_eq!(s, "hello"),
                    other => panic!("Expected String literal, got {:?}", other),
                }
            }
            other => panic!("Expected Test, got {:?}", other),
        }
    }

    // ── Test 13: Multiple guards ─────────────────────────────────────

    #[test]
    fn test_multiple_guards() {
        // match x { n when n > 0 -> "pos", n when n < 0 -> "neg", _ -> "zero" }
        // Each guard should chain: Guard -> Guard -> Leaf
        let guard_pos = MirExpr::BinOp {
            op: crate::mir::BinOp::Gt,
            lhs: Box::new(var_expr("n", MirType::Int)),
            rhs: Box::new(int_body(0)),
            ty: MirType::Bool,
        };
        let guard_neg = MirExpr::BinOp {
            op: crate::mir::BinOp::Lt,
            lhs: Box::new(var_expr("n", MirType::Int)),
            rhs: Box::new(int_body(0)),
            ty: MirType::Bool,
        };

        let arms = vec![
            make_arm(
                MirPattern::Var("n".to_string(), MirType::Int),
                Some(guard_pos),
                string_body("pos"),
            ),
            make_arm(
                MirPattern::Var("n".to_string(), MirType::Int),
                Some(guard_neg),
                string_body("neg"),
            ),
            make_arm(MirPattern::Wildcard, None, string_body("zero")),
        ];

        let tree = compile_match(&MirType::Int, &arms, "test.mpl", 1, &FxHashMap::default());

        // Should be Guard(pos_guard, Leaf(0), Guard(neg_guard, Leaf(1), Leaf(2)))
        match &tree {
            DecisionTree::Guard {
                arm_index: 0,
                failure: first_failure,
                ..
            } => match first_failure.as_ref() {
                DecisionTree::Guard {
                    arm_index: 1,
                    failure: f2,
                    ..
                } => {
                    assert!(matches!(
                        f2.as_ref(),
                        DecisionTree::Leaf { arm_index: 2, .. }
                    ));
                }
                other => panic!("Expected nested Guard, got {:?}", other),
            },
            other => panic!("Expected Guard, got {:?}", other),
        }
    }

    // ── Test 14: Or-pattern with constructors ────────────────────────

    #[test]
    fn test_or_pattern_constructors() {
        // match color { Red | Blue -> "cool", Green -> "warm" }
        // Or-patterns on constructors should expand and both lead to arm_index 0
        let arms = vec![
            make_arm(
                MirPattern::Or(vec![
                    MirPattern::Constructor {
                        type_name: "Color".to_string(),
                        variant: "Red".to_string(),
                        fields: vec![],
                        bindings: vec![],
                    },
                    MirPattern::Constructor {
                        type_name: "Color".to_string(),
                        variant: "Blue".to_string(),
                        fields: vec![],
                        bindings: vec![],
                    },
                ]),
                None,
                string_body("cool"),
            ),
            make_arm(
                MirPattern::Constructor {
                    type_name: "Color".to_string(),
                    variant: "Green".to_string(),
                    fields: vec![],
                    bindings: vec![],
                },
                None,
                string_body("warm"),
            ),
        ];

        let tree = compile_match(
            &MirType::SumType("Color".to_string()),
            &arms,
            "test.mpl",
            1,
            &FxHashMap::default(),
        );

        match &tree {
            DecisionTree::Switch { cases, .. } => {
                // Red and Blue should both have arm_index 0, Green has arm_index 1
                let red_case = cases
                    .iter()
                    .find(|(tag, _)| tag.variant_name == "Red")
                    .expect("Should have Red case");
                match &red_case.1 {
                    DecisionTree::Leaf { arm_index, .. } => assert_eq!(*arm_index, 0),
                    other => panic!("Expected Leaf for Red, got {:?}", other),
                }

                let blue_case = cases
                    .iter()
                    .find(|(tag, _)| tag.variant_name == "Blue")
                    .expect("Should have Blue case");
                match &blue_case.1 {
                    DecisionTree::Leaf { arm_index, .. } => assert_eq!(*arm_index, 0),
                    other => panic!("Expected Leaf for Blue, got {:?}", other),
                }

                let green_case = cases
                    .iter()
                    .find(|(tag, _)| tag.variant_name == "Green")
                    .expect("Should have Green case");
                match &green_case.1 {
                    DecisionTree::Leaf { arm_index, .. } => assert_eq!(*arm_index, 1),
                    other => panic!("Expected Leaf for Green, got {:?}", other),
                }
            }
            other => panic!("Expected Switch, got {:?}", other),
        }
    }

    // ── Test 15: Tag assignment from sum type definition ──────────────

    #[test]
    fn test_tag_from_sum_type_def() {
        // Option is defined as: Some(T) = tag 0, None = tag 1.
        // Pattern order: None first, Some second (reversed from definition).
        // Tags in the Switch must come from the definition, NOT pattern order.
        let mut sum_type_defs = FxHashMap::default();
        sum_type_defs.insert(
            "Option".to_string(),
            MirSumTypeDef {
                name: "Option".to_string(),
                variants: vec![
                    MirVariantDef {
                        name: "Some".to_string(),
                        fields: vec![MirType::Int],
                        tag: 0,
                    },
                    MirVariantDef {
                        name: "None".to_string(),
                        fields: vec![],
                        tag: 1,
                    },
                ],
            },
        );

        // Arms: None -> 0, Some(x) -> x  (reversed from definition order)
        let arms = vec![
            make_arm(
                MirPattern::Constructor {
                    type_name: "Option".to_string(),
                    variant: "None".to_string(),
                    fields: vec![],
                    bindings: vec![],
                },
                None,
                int_body(0),
            ),
            make_arm(
                MirPattern::Constructor {
                    type_name: "Option".to_string(),
                    variant: "Some".to_string(),
                    fields: vec![MirPattern::Var("x".to_string(), MirType::Int)],
                    bindings: vec![("x".to_string(), MirType::Int)],
                },
                None,
                var_expr("x", MirType::Int),
            ),
        ];

        let tree = compile_match(
            &MirType::SumType("Option".to_string()),
            &arms,
            "test.mpl",
            1,
            &sum_type_defs,
        );

        match &tree {
            DecisionTree::Switch { cases, .. } => {
                assert_eq!(cases.len(), 2);

                // None should have tag 1 (from definition), NOT tag 0 (appearance order)
                let none_case = cases
                    .iter()
                    .find(|(tag, _)| tag.variant_name == "None")
                    .expect("Should have None case");
                assert_eq!(
                    none_case.0.tag, 1,
                    "None tag should be 1 (from type definition), got {}",
                    none_case.0.tag
                );

                // Some should have tag 0 (from definition), NOT tag 1 (appearance order)
                let some_case = cases
                    .iter()
                    .find(|(tag, _)| tag.variant_name == "Some")
                    .expect("Should have Some case");
                assert_eq!(
                    some_case.0.tag, 0,
                    "Some tag should be 0 (from type definition), got {}",
                    some_case.0.tag
                );
            }
            other => panic!("Expected Switch, got {:?}", other),
        }
    }

    // ── Test 16: Field type resolution from sum type definition ───────

    #[test]
    fn test_field_type_from_sum_type_def() {
        // Option is defined as: Some(Int) = tag 0, None = tag 1.
        // The binding for x in Some(x) should have type Int (from the
        // sum type definition), not Unit (the old placeholder).
        let mut sum_type_defs = FxHashMap::default();
        sum_type_defs.insert(
            "Option".to_string(),
            MirSumTypeDef {
                name: "Option".to_string(),
                variants: vec![
                    MirVariantDef {
                        name: "Some".to_string(),
                        fields: vec![MirType::Int],
                        tag: 0,
                    },
                    MirVariantDef {
                        name: "None".to_string(),
                        fields: vec![],
                        tag: 1,
                    },
                ],
            },
        );

        // Arm: Some(x) -> x  with Var type set to Unit (simulating unresolved)
        let arms = vec![
            make_arm(
                MirPattern::Constructor {
                    type_name: "Option".to_string(),
                    variant: "Some".to_string(),
                    fields: vec![MirPattern::Var("x".to_string(), MirType::Unit)],
                    bindings: vec![("x".to_string(), MirType::Unit)],
                },
                None,
                var_expr("x", MirType::Int),
            ),
            make_arm(MirPattern::Wildcard, None, int_body(0)),
        ];

        let tree = compile_match(
            &MirType::SumType("Option".to_string()),
            &arms,
            "test.mpl",
            1,
            &sum_type_defs,
        );

        match &tree {
            DecisionTree::Switch { cases, .. } => {
                assert_eq!(cases.len(), 1);
                assert_eq!(cases[0].0.variant_name, "Some");

                match &cases[0].1 {
                    DecisionTree::Leaf { bindings, .. } => {
                        assert_eq!(bindings.len(), 1);
                        assert_eq!(bindings[0].0, "x");
                        // The key assertion: x should have type Int (resolved
                        // from sum type def), NOT Unit (the placeholder).
                        assert_eq!(
                            bindings[0].1,
                            MirType::Int,
                            "Binding 'x' should have type Int from sum type def, got {:?}",
                            bindings[0].1
                        );
                    }
                    other => panic!("Expected Leaf for Some, got {:?}", other),
                }
            }
            other => panic!("Expected Switch, got {:?}", other),
        }
    }

    #[test]
    fn nested_sum_inside_generic_variant_keeps_its_concrete_access_type() {
        let mut sum_type_defs = FxHashMap::default();
        sum_type_defs.insert(
            "Result".to_string(),
            MirSumTypeDef {
                name: "Result".to_string(),
                variants: vec![
                    MirVariantDef {
                        name: "Ok".to_string(),
                        fields: vec![MirType::Ptr],
                        tag: 0,
                    },
                    MirVariantDef {
                        name: "Err".to_string(),
                        fields: vec![MirType::Ptr],
                        tag: 1,
                    },
                ],
            },
        );
        sum_type_defs.insert(
            "CryptoError".to_string(),
            MirSumTypeDef {
                name: "CryptoError".to_string(),
                variants: vec![MirVariantDef {
                    name: "InvalidLength".to_string(),
                    fields: vec![MirType::Int, MirType::Int],
                    tag: 0,
                }],
            },
        );

        let arms = vec![
            make_arm(
                MirPattern::Constructor {
                    type_name: "Result_SecretBytes_CryptoError".to_string(),
                    variant: "Err".to_string(),
                    fields: vec![MirPattern::Constructor {
                        type_name: "CryptoError".to_string(),
                        variant: "InvalidLength".to_string(),
                        fields: vec![
                            MirPattern::Var("expected".to_string(), MirType::Int),
                            MirPattern::Var("actual".to_string(), MirType::Int),
                        ],
                        bindings: vec![
                            ("expected".to_string(), MirType::Int),
                            ("actual".to_string(), MirType::Int),
                        ],
                    }],
                    bindings: vec![],
                },
                None,
                int_body(1),
            ),
            make_arm(MirPattern::Wildcard, None, int_body(0)),
        ];

        let tree = compile_match(
            &MirType::SumType("Result_SecretBytes_CryptoError".to_string()),
            &arms,
            "test.mpl",
            1,
            &sum_type_defs,
        );
        let DecisionTree::Switch { cases, .. } = tree else {
            panic!("expected outer Result switch");
        };
        let DecisionTree::Switch { scrutinee_path, .. } = &cases[0].1 else {
            panic!("expected nested CryptoError switch: {:?}", cases[0].1);
        };
        assert_eq!(
            scrutinee_path,
            &AccessPath::VariantField {
                parent: Box::new(AccessPath::Root),
                type_name: "Result_SecretBytes_CryptoError".to_string(),
                variant: "Err".to_string(),
                index: 0,
                ty: MirType::SumType("CryptoError".to_string()),
            }
        );
    }
}
