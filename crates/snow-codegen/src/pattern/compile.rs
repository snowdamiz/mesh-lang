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

use crate::mir::{MirExpr, MirLiteral, MirMatchArm, MirModule, MirPattern, MirType};
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
    compile_matrix(matrix, file, line)
}

/// Walk all `MirExpr::Match` nodes in a module and compile them to
/// `MirExpr::CompiledMatch` with decision trees.
pub fn compile_patterns(module: &mut MirModule) {
    for func in &mut module.functions {
        compile_expr_patterns(&mut func.body);
    }
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

/// Recursively expand or-patterns in a single pattern.
fn expand_pattern(
    arm_index: usize,
    pattern: &MirPattern,
    guard: &Option<MirExpr>,
    out: &mut Vec<(usize, MirPattern, Option<MirExpr>)>,
) {
    match pattern {
        MirPattern::Or(alternatives) => {
            for alt in alternatives {
                // Each alternative in an or-pattern becomes its own row
                // with the same arm_index (they share the body).
                expand_pattern(arm_index, alt, guard, out);
            }
        }
        _ => {
            out.push((arm_index, pattern.clone(), guard.clone()));
        }
    }
}

// ── Core compilation ────────────────────────────────────────────────

/// Compile a pattern matrix into a decision tree (Maranget's algorithm).
fn compile_matrix(matrix: PatMatrix, file: &str, line: u32) -> DecisionTree {
    // Base case 1: No rows -- match failure.
    if matrix.rows.is_empty() {
        return DecisionTree::Fail {
            message: "non-exhaustive match".to_string(),
            file: file.to_string(),
            line,
        };
    }

    // Base case 2: No columns -- all patterns consumed. The first row wins.
    if matrix.column_paths.is_empty() {
        let row = &matrix.rows[0];
        return make_leaf_or_guard(row, &matrix.rows[1..], file, line);
    }

    // Base case 3: First row is all wildcards/variables -- it matches.
    if row_is_all_wildcards(&matrix.rows[0]) {
        let mut row = matrix.rows[0].clone();
        // Collect variable bindings from this row.
        collect_bindings_from_row(&mut row, &matrix.column_paths, &matrix.column_types);
        return make_leaf_or_guard(&row, &matrix.rows[1..], file, line);
    }

    // Step 1: Select the best column to test (most constructor diversity).
    let col = select_column(&matrix);

    // Step 1.5: If the selected column contains tuple patterns, expand them first.
    // Tuples are structural -- they don't need a switch/test, just decomposition.
    if column_has_tuples(&matrix, col) {
        let expanded = expand_tuple_column(&matrix, col);
        return compile_matrix(expanded, file, line);
    }

    // Step 2: Collect head constructors from the selected column.
    let head_ctors = collect_head_constructors(&matrix, col);

    if head_ctors.is_empty() {
        // All patterns in this column are wildcards/variables.
        // Collect bindings and remove the column.
        let reduced = remove_wildcard_column(&matrix, col);
        return compile_matrix(reduced, file, line);
    }

    // Step 3: Determine if we need a Switch (constructors) or Tests (literals).
    let has_constructors = head_ctors.iter().any(|c| matches!(c, HeadCtor::Constructor { .. }));

    if has_constructors {
        compile_constructor_switch(&matrix, col, &head_ctors, file, line)
    } else {
        compile_literal_tests(&matrix, col, &head_ctors, file, line)
    }
}

// ── Leaf / Guard creation ───────────────────────────────────────────

/// Create a Leaf node, possibly wrapping it in a Guard if the arm has a guard.
/// If the guard fails, fall through to the remaining rows.
fn make_leaf_or_guard(row: &PatRow, rest: &[PatRow], file: &str, line: u32) -> DecisionTree {
    let leaf = DecisionTree::Leaf {
        arm_index: row.arm_index,
        bindings: row.bindings.clone(),
    };

    match &row.guard {
        Some(guard_expr) => {
            // Build failure branch from remaining rows.
            let failure = if rest.is_empty() {
                DecisionTree::Fail {
                    message: "non-exhaustive match".to_string(),
                    file: file.to_string(),
                    line,
                }
            } else {
                // Remaining rows might also have guards, so we chain them.
                let first_rest = &rest[0];
                make_leaf_or_guard(first_rest, &rest[1..], file, line)
            };

            DecisionTree::Guard {
                guard_expr: guard_expr.clone(),
                success: Box::new(leaf),
                failure: Box::new(failure),
            }
        }
        None => leaf,
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
    let num_cols = matrix.column_paths.len();
    if num_cols == 0 {
        return 0;
    }

    let mut best_col = 0;
    let mut best_score = 0usize;

    for col in 0..num_cols {
        let mut score = 0;
        let mut seen_ctors: Vec<String> = Vec::new();

        for row in &matrix.rows {
            if col < row.patterns.len() {
                let ctor_key = head_ctor_key(&row.patterns[col]);
                if let Some(key) = ctor_key {
                    if !seen_ctors.contains(&key) {
                        seen_ctors.push(key);
                        score += 1;
                    }
                }
            }
        }

        if score > best_score {
            best_score = score;
            best_col = col;
        }
    }

    best_col
}

/// Get a unique string key for the head constructor of a pattern.
fn head_ctor_key(p: &MirPattern) -> Option<String> {
    match p {
        MirPattern::Literal(lit) => Some(format!("lit:{}", literal_key(lit))),
        MirPattern::Constructor { variant, .. } => Some(format!("ctor:{}", variant)),
        MirPattern::Tuple(elems) => Some(format!("tuple:{}", elems.len())),
        MirPattern::Or(_) => None, // Should be expanded already
        MirPattern::Wildcard | MirPattern::Var(..) => None,
    }
}

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
fn collect_head_constructors(matrix: &PatMatrix, col: usize) -> Vec<HeadCtor> {
    let mut result: Vec<HeadCtor> = Vec::new();
    let mut seen: Vec<String> = Vec::new();

    for row in &matrix.rows {
        if col >= row.patterns.len() {
            continue;
        }
        match &row.patterns[col] {
            MirPattern::Literal(lit) => {
                let key = literal_key(lit);
                if !seen.contains(&key) {
                    seen.push(key);
                    result.push(HeadCtor::Literal(lit.clone()));
                }
            }
            MirPattern::Constructor {
                type_name,
                variant,
                fields,
                ..
            } => {
                let key = format!("ctor:{}", variant);
                if !seen.contains(&key) {
                    // Assign tags based on order of first appearance.
                    let tag = result
                        .iter()
                        .filter(|c| matches!(c, HeadCtor::Constructor { .. }))
                        .count() as u8;
                    seen.push(key);
                    result.push(HeadCtor::Constructor {
                        type_name: type_name.clone(),
                        variant: variant.clone(),
                        tag,
                        arity: fields.len(),
                    });
                }
            }
            MirPattern::Tuple(_) => {
                // Tuples are deconstructed (expanded) rather than switched on.
                // We don't add them as head constructors; instead we expand the column.
            }
            _ => {} // Wildcards/variables don't contribute head constructors.
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
            let specialized = specialize_for_constructor(matrix, col, variant, *arity);
            let subtree = compile_matrix(specialized, file, line);
            cases.push((ctor_tag, subtree));
        }
    }

    // Build default branch from rows with wildcard/variable in this column.
    let default_matrix = default_matrix(matrix, col);
    let default = if default_matrix.rows.is_empty() {
        None
    } else {
        Some(Box::new(compile_matrix(default_matrix, file, line)))
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
    target_variant: &str,
    arity: usize,
) -> PatMatrix {
    let mut new_rows = Vec::new();
    let parent_path = &matrix.column_paths[col];

    for row in &matrix.rows {
        let pat = &row.patterns[col];
        match pat {
            MirPattern::Constructor {
                variant, fields, ..
            } if variant == target_variant => {
                // This row matches the constructor -- expand sub-patterns.
                let mut new_pats: Vec<MirPattern> = Vec::new();
                let new_bindings = row.bindings.clone();

                // Add sub-patterns from the constructor fields.
                // Variable bindings are carried by the sub-patterns themselves
                // (e.g., Var("r")) and will be collected when those sub-patterns
                // are processed in recursive compilation. We do NOT use the
                // Constructor's `bindings` field here to avoid double-counting.
                for field_pat in fields.iter() {
                    new_pats.push(field_pat.clone());
                }

                // Add the remaining columns (before and after the selected one).
                for (i, p) in row.patterns.iter().enumerate() {
                    if i != col {
                        new_pats.push(p.clone());
                    }
                }

                new_rows.push(PatRow {
                    patterns: new_pats,
                    arm_index: row.arm_index,
                    guard: row.guard.clone(),
                    bindings: new_bindings,
                });
            }
            MirPattern::Wildcard | MirPattern::Var(..) => {
                // Wildcard/variable rows match any constructor -- pad with wildcards.
                let mut new_pats: Vec<MirPattern> = Vec::new();
                let mut new_bindings = row.bindings.clone();

                // Collect binding if it's a variable.
                if let MirPattern::Var(name, ty) = pat {
                    new_bindings.push((
                        name.clone(),
                        ty.clone(),
                        matrix.column_paths[col].clone(),
                    ));
                }

                // Add wildcard sub-patterns for each constructor field.
                for _ in 0..arity {
                    new_pats.push(MirPattern::Wildcard);
                }

                // Add remaining columns.
                for (i, p) in row.patterns.iter().enumerate() {
                    if i != col {
                        new_pats.push(p.clone());
                    }
                }

                new_rows.push(PatRow {
                    patterns: new_pats,
                    arm_index: row.arm_index,
                    guard: row.guard.clone(),
                    bindings: new_bindings,
                });
            }
            _ => {
                // Different constructor -- skip this row.
            }
        }
    }

    // Build new column paths: sub-pattern paths + remaining column paths.
    let mut new_paths = Vec::new();
    let mut new_types = Vec::new();

    // Sub-pattern paths for the constructor fields.
    // We need to determine variant name for the access path.
    for i in 0..arity {
        new_paths.push(AccessPath::VariantField(
            Box::new(parent_path.clone()),
            target_variant.to_string(),
            i,
        ));
        // We don't know the exact field type here, use Unit as placeholder.
        // The actual type is carried by the variable patterns themselves.
        new_types.push(MirType::Unit);
    }

    // Remaining columns.
    for (i, path) in matrix.column_paths.iter().enumerate() {
        if i != col {
            new_paths.push(path.clone());
            new_types.push(matrix.column_types[i].clone());
        }
    }

    PatMatrix {
        rows: new_rows,
        column_paths: new_paths,
        column_types: new_types,
    }
}

// ── Literal test compilation ────────────────────────────────────────

/// Compile a chain of Test nodes for literal patterns.
fn compile_literal_tests(
    matrix: &PatMatrix,
    col: usize,
    head_ctors: &[HeadCtor],
    file: &str,
    line: u32,
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
    let mut failure_tree = compile_matrix(default_mat, file, line);

    // Build from last to first to create the chain.
    for lit in literals.iter().rev() {
        let specialized = specialize_for_literal(matrix, col, lit);
        let success_tree = compile_matrix(specialized, file, line);

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
fn specialize_for_literal(
    matrix: &PatMatrix,
    col: usize,
    target_lit: &MirLiteral,
) -> PatMatrix {
    let mut new_rows = Vec::new();

    for row in &matrix.rows {
        let pat = &row.patterns[col];
        match pat {
            MirPattern::Literal(lit) if literals_equal(lit, target_lit) => {
                // This row matches the literal -- remove the column.
                let mut new_pats = Vec::new();
                for (i, p) in row.patterns.iter().enumerate() {
                    if i != col {
                        new_pats.push(p.clone());
                    }
                }
                new_rows.push(PatRow {
                    patterns: new_pats,
                    arm_index: row.arm_index,
                    guard: row.guard.clone(),
                    bindings: row.bindings.clone(),
                });
            }
            MirPattern::Wildcard | MirPattern::Var(..) => {
                // Wildcard/variable rows match any literal -- keep them.
                let mut new_pats = Vec::new();
                let mut new_bindings = row.bindings.clone();

                if let MirPattern::Var(name, ty) = pat {
                    new_bindings.push((
                        name.clone(),
                        ty.clone(),
                        matrix.column_paths[col].clone(),
                    ));
                }

                for (i, p) in row.patterns.iter().enumerate() {
                    if i != col {
                        new_pats.push(p.clone());
                    }
                }
                new_rows.push(PatRow {
                    patterns: new_pats,
                    arm_index: row.arm_index,
                    guard: row.guard.clone(),
                    bindings: new_bindings,
                });
            }
            _ => {
                // Different literal -- skip.
            }
        }
    }

    // Build new column paths (remove the tested column).
    let mut new_paths = Vec::new();
    let mut new_types = Vec::new();
    for (i, path) in matrix.column_paths.iter().enumerate() {
        if i != col {
            new_paths.push(path.clone());
            new_types.push(matrix.column_types[i].clone());
        }
    }

    PatMatrix {
        rows: new_rows,
        column_paths: new_paths,
        column_types: new_types,
    }
}

/// Compare two MirLiteral values for structural equality.
fn literals_equal(a: &MirLiteral, b: &MirLiteral) -> bool {
    match (a, b) {
        (MirLiteral::Int(x), MirLiteral::Int(y)) => x == y,
        (MirLiteral::Float(x), MirLiteral::Float(y)) => x.to_bits() == y.to_bits(),
        (MirLiteral::Bool(x), MirLiteral::Bool(y)) => x == y,
        (MirLiteral::String(x), MirLiteral::String(y)) => x == y,
        _ => false,
    }
}

// ── Default matrix ──────────────────────────────────────────────────

/// Build the default matrix: rows with wildcard/variable in the given column,
/// with that column removed.
fn default_matrix(matrix: &PatMatrix, col: usize) -> PatMatrix {
    let mut new_rows = Vec::new();

    for row in &matrix.rows {
        let pat = &row.patterns[col];
        if is_wildcard_like(pat) {
            let mut new_pats = Vec::new();
            let mut new_bindings = row.bindings.clone();

            if let MirPattern::Var(name, ty) = pat {
                new_bindings.push((
                    name.clone(),
                    ty.clone(),
                    matrix.column_paths[col].clone(),
                ));
            }

            for (i, p) in row.patterns.iter().enumerate() {
                if i != col {
                    new_pats.push(p.clone());
                }
            }
            new_rows.push(PatRow {
                patterns: new_pats,
                arm_index: row.arm_index,
                guard: row.guard.clone(),
                bindings: new_bindings,
            });
        }
    }

    let mut new_paths = Vec::new();
    let mut new_types = Vec::new();
    for (i, path) in matrix.column_paths.iter().enumerate() {
        if i != col {
            new_paths.push(path.clone());
            new_types.push(matrix.column_types[i].clone());
        }
    }

    PatMatrix {
        rows: new_rows,
        column_paths: new_paths,
        column_types: new_types,
    }
}

/// Remove a column that contains only wildcards/variables.
/// Collects any variable bindings before removing.
fn remove_wildcard_column(matrix: &PatMatrix, col: usize) -> PatMatrix {
    let mut new_rows = Vec::new();

    for row in &matrix.rows {
        let mut new_pats = Vec::new();
        let mut new_bindings = row.bindings.clone();

        // Collect binding if it's a variable.
        if let MirPattern::Var(name, ty) = &row.patterns[col] {
            new_bindings.push((
                name.clone(),
                ty.clone(),
                matrix.column_paths[col].clone(),
            ));
        }

        for (i, p) in row.patterns.iter().enumerate() {
            if i != col {
                new_pats.push(p.clone());
            }
        }

        new_rows.push(PatRow {
            patterns: new_pats,
            arm_index: row.arm_index,
            guard: row.guard.clone(),
            bindings: new_bindings,
        });
    }

    let mut new_paths = Vec::new();
    let mut new_types = Vec::new();
    for (i, path) in matrix.column_paths.iter().enumerate() {
        if i != col {
            new_paths.push(path.clone());
            new_types.push(matrix.column_types[i].clone());
        }
    }

    PatMatrix {
        rows: new_rows,
        column_paths: new_paths,
        column_types: new_types,
    }
}

// ── Tuple expansion ─────────────────────────────────────────────────

// Tuples are handled by expanding the column. When a tuple pattern is
// detected in the selected column, we expand it into its sub-columns
// before proceeding with the standard algorithm. This is done inside
// `compile_matrix` before the head constructor phase, via the
// `expand_tuple_column` function.

// We need to detect and expand tuple columns in compile_matrix.
// Let me refactor to handle this properly.

/// Check if a column contains any tuple patterns.
fn column_has_tuples(matrix: &PatMatrix, col: usize) -> bool {
    matrix.rows.iter().any(|row| {
        col < row.patterns.len() && matches!(&row.patterns[col], MirPattern::Tuple(_))
    })
}

/// Expand a tuple column into its sub-columns.
/// Tuple patterns become their element patterns; wildcards/variables
/// become wildcard elements.
fn expand_tuple_column(matrix: &PatMatrix, col: usize) -> PatMatrix {
    // Determine arity of the tuple from the first tuple pattern.
    let arity = matrix
        .rows
        .iter()
        .filter_map(|row| {
            if col < row.patterns.len() {
                if let MirPattern::Tuple(elems) = &row.patterns[col] {
                    Some(elems.len())
                } else {
                    None
                }
            } else {
                None
            }
        })
        .next()
        .unwrap_or(0);

    if arity == 0 {
        // Degenerate case -- just remove the column.
        return remove_wildcard_column(matrix, col);
    }

    let parent_path = &matrix.column_paths[col];
    let parent_type = &matrix.column_types[col];

    // Build sub-column types from the parent tuple type.
    let sub_types: Vec<MirType> = match parent_type {
        MirType::Tuple(elems) => elems.clone(),
        _ => vec![MirType::Unit; arity],
    };

    let mut new_rows = Vec::new();

    for row in &matrix.rows {
        let pat = &row.patterns[col];
        let mut new_pats = Vec::new();
        let mut new_bindings = row.bindings.clone();

        match pat {
            MirPattern::Tuple(elems) => {
                // Expand tuple elements as new columns.
                for elem in elems {
                    new_pats.push(elem.clone());
                }
            }
            MirPattern::Wildcard => {
                // Pad with wildcards for each tuple element.
                for _ in 0..arity {
                    new_pats.push(MirPattern::Wildcard);
                }
            }
            MirPattern::Var(name, ty) => {
                // Variable binding for the whole tuple -- bind and pad with wildcards.
                new_bindings.push((
                    name.clone(),
                    ty.clone(),
                    parent_path.clone(),
                ));
                for _ in 0..arity {
                    new_pats.push(MirPattern::Wildcard);
                }
            }
            _ => {
                // Shouldn't happen in a well-typed program, but handle gracefully.
                for _ in 0..arity {
                    new_pats.push(MirPattern::Wildcard);
                }
            }
        }

        // Add remaining columns.
        for (i, p) in row.patterns.iter().enumerate() {
            if i != col {
                new_pats.push(p.clone());
            }
        }

        new_rows.push(PatRow {
            patterns: new_pats,
            arm_index: row.arm_index,
            guard: row.guard.clone(),
            bindings: new_bindings,
        });
    }

    // Build new column paths.
    let mut new_paths = Vec::new();
    let mut new_types = Vec::new();

    for i in 0..arity {
        new_paths.push(AccessPath::TupleField(Box::new(parent_path.clone()), i));
        new_types.push(sub_types.get(i).cloned().unwrap_or(MirType::Unit));
    }

    for (i, path) in matrix.column_paths.iter().enumerate() {
        if i != col {
            new_paths.push(path.clone());
            new_types.push(matrix.column_types[i].clone());
        }
    }

    PatMatrix {
        rows: new_rows,
        column_paths: new_paths,
        column_types: new_types,
    }
}

// ── Expression tree walking ─────────────────────────────────────────

/// Recursively compile match expressions within an expression tree.
fn compile_expr_patterns(expr: &mut MirExpr) {
    match expr {
        MirExpr::Match {
            scrutinee,
            arms,
            ty,
        } => {
            // First, recursively process sub-expressions.
            compile_expr_patterns(scrutinee);
            for arm in arms.iter_mut() {
                compile_expr_patterns(&mut arm.body);
                if let Some(guard) = &mut arm.guard {
                    compile_expr_patterns(guard);
                }
            }

            // Compile the match into a decision tree.
            let scrutinee_ty = scrutinee.ty().clone();
            let _tree = compile_match(&scrutinee_ty, arms, "<unknown>", 0);

            // Note: In a full implementation, we would replace this MirExpr::Match
            // with a MirExpr::CompiledMatch variant. For now, the decision tree
            // is computed and will be used by the LLVM codegen directly via
            // compile_match() calls.
        }
        MirExpr::BinOp { lhs, rhs, .. } => {
            compile_expr_patterns(lhs);
            compile_expr_patterns(rhs);
        }
        MirExpr::UnaryOp { operand, .. } => {
            compile_expr_patterns(operand);
        }
        MirExpr::Call { func, args, .. } => {
            compile_expr_patterns(func);
            for arg in args {
                compile_expr_patterns(arg);
            }
        }
        MirExpr::ClosureCall { closure, args, .. } => {
            compile_expr_patterns(closure);
            for arg in args {
                compile_expr_patterns(arg);
            }
        }
        MirExpr::If {
            cond,
            then_body,
            else_body,
            ..
        } => {
            compile_expr_patterns(cond);
            compile_expr_patterns(then_body);
            compile_expr_patterns(else_body);
        }
        MirExpr::Let { value, body, .. } => {
            compile_expr_patterns(value);
            compile_expr_patterns(body);
        }
        MirExpr::Block(exprs, _) => {
            for e in exprs {
                compile_expr_patterns(e);
            }
        }
        MirExpr::StructLit { fields, .. } => {
            for (_, field_expr) in fields {
                compile_expr_patterns(field_expr);
            }
        }
        MirExpr::FieldAccess { object, .. } => {
            compile_expr_patterns(object);
        }
        MirExpr::ConstructVariant { fields, .. } => {
            for f in fields {
                compile_expr_patterns(f);
            }
        }
        MirExpr::MakeClosure { captures, .. } => {
            for c in captures {
                compile_expr_patterns(c);
            }
        }
        MirExpr::Return(inner) => {
            compile_expr_patterns(inner);
        }
        // Leaf expressions -- nothing to recurse into.
        MirExpr::IntLit(..)
        | MirExpr::FloatLit(..)
        | MirExpr::BoolLit(..)
        | MirExpr::StringLit(..)
        | MirExpr::Var(..)
        | MirExpr::Panic { .. }
        | MirExpr::Unit => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir::{MirLiteral, MirMatchArm, MirPattern, MirType};
    use crate::pattern::{AccessPath, DecisionTree};

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

        let tree = compile_match(&MirType::Int, &arms, "test.snow", 1);

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

        let tree = compile_match(&MirType::Int, &arms, "test.snow", 1);

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

        let tree = compile_match(&MirType::Int, &arms, "test.snow", 1);

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
                    DecisionTree::Leaf {
                        arm_index: 0,
                        ..
                    }
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
                            DecisionTree::Leaf {
                                arm_index: 1,
                                ..
                            }
                        ));
                        assert!(matches!(
                            f2.as_ref(),
                            DecisionTree::Leaf {
                                arm_index: 2,
                                ..
                            }
                        ));
                    }
                    other => panic!("Expected nested Test, got {:?}", other),
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

        let tree = compile_match(&MirType::Bool, &arms, "test.snow", 1);

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
                    DecisionTree::Leaf {
                        arm_index: 0,
                        ..
                    }
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
                            DecisionTree::Leaf {
                                arm_index: 1,
                                ..
                            }
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

        let tree = compile_match(&MirType::SumType("Shape".to_string()), &arms, "test.snow", 1);

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
                            AccessPath::VariantField(
                                Box::new(AccessPath::Root),
                                "Circle".to_string(),
                                0
                            )
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

        let scrutinee_ty = MirType::Tuple(vec![
            MirType::SumType("Option".to_string()),
            MirType::Int,
        ]);
        let tree = compile_match(&scrutinee_ty, &arms, "test.snow", 1);

        // The tree should switch on TupleField(Root, 0) for constructor tag
        match &tree {
            DecisionTree::Switch {
                scrutinee_path,
                cases,
                ..
            } => {
                assert_eq!(
                    *scrutinee_path,
                    AccessPath::TupleField(Box::new(AccessPath::Root), 0)
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

        let tree = compile_match(&MirType::Int, &arms, "test.snow", 1);

        // Should be Test(Root, 1, Leaf(0), Test(Root, 2, Leaf(0), Leaf(1)))
        match &tree {
            DecisionTree::Test {
                value, success, failure, ..
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

        let tree = compile_match(&MirType::Int, &arms, "test.snow", 1);

        match &tree {
            DecisionTree::Guard {
                success, failure, ..
            } => {
                match success.as_ref() {
                    DecisionTree::Leaf {
                        arm_index,
                        bindings,
                    } => {
                        assert_eq!(*arm_index, 0);
                        assert_eq!(bindings.len(), 1);
                        assert_eq!(bindings[0].0, "n");
                        assert_eq!(bindings[0].2, AccessPath::Root);
                    }
                    other => panic!("Expected Leaf for guard success, got {:?}", other),
                }
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

        let tree = compile_match(&MirType::Int, &arms, "test.snow", 1);

        match &tree {
            DecisionTree::Guard {
                success, failure, ..
            } => {
                assert!(matches!(
                    success.as_ref(),
                    DecisionTree::Leaf {
                        arm_index: 0,
                        ..
                    }
                ));
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
            "test.snow",
            1,
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
        let tree = compile_match(&scrutinee_ty, &arms, "test.snow", 1);

        // Should test tuple fields, not Root
        fn contains_tuple_field_test(tree: &DecisionTree) -> bool {
            match tree {
                DecisionTree::Test {
                    scrutinee_path,
                    failure,
                    success,
                    ..
                } => {
                    matches!(scrutinee_path, AccessPath::TupleField(_, _))
                        || contains_tuple_field_test(success)
                        || contains_tuple_field_test(failure)
                }
                DecisionTree::Switch {
                    scrutinee_path,
                    cases,
                    default,
                    ..
                } => {
                    matches!(scrutinee_path, AccessPath::TupleField(_, _))
                        || cases.iter().any(|(_, t)| contains_tuple_field_test(t))
                        || default
                            .as_ref()
                            .map_or(false, |d| contains_tuple_field_test(d))
                }
                DecisionTree::Guard {
                    success, failure, ..
                } => contains_tuple_field_test(success) || contains_tuple_field_test(failure),
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

        let tree = compile_match(&MirType::String, &arms, "test.snow", 1);

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

        let tree = compile_match(&MirType::Int, &arms, "test.snow", 1);

        // Should be Guard(pos_guard, Leaf(0), Guard(neg_guard, Leaf(1), Leaf(2)))
        match &tree {
            DecisionTree::Guard {
                success,
                failure: first_failure,
                ..
            } => {
                assert!(matches!(
                    success.as_ref(),
                    DecisionTree::Leaf {
                        arm_index: 0,
                        ..
                    }
                ));
                match first_failure.as_ref() {
                    DecisionTree::Guard {
                        success: s2,
                        failure: f2,
                        ..
                    } => {
                        assert!(matches!(
                            s2.as_ref(),
                            DecisionTree::Leaf {
                                arm_index: 1,
                                ..
                            }
                        ));
                        assert!(matches!(
                            f2.as_ref(),
                            DecisionTree::Leaf {
                                arm_index: 2,
                                ..
                            }
                        ));
                    }
                    other => panic!("Expected nested Guard, got {:?}", other),
                }
            }
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
            "test.snow",
            1,
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
}
