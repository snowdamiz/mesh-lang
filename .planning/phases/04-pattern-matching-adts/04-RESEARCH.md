# Phase 4: Pattern Matching & Algebraic Data Types - Research

**Researched:** 2026-02-06
**Domain:** Compiler engineering -- algebraic data types, pattern compilation, exhaustiveness checking
**Confidence:** HIGH

<user_constraints>
## User Constraints (from CONTEXT.md)

### Locked Decisions

#### Sum type syntax
- `type` keyword with `do/end` block (Elixir-style): `type Shape do Circle(Float) ... end`
- Variants can have named fields: `Rectangle(width: Float, height: Float)`
- Sum types support generic type parameters: `type Option<T> do Some(T) None end` -- replaces compiler builtins
- Variants constructed via qualified access: `Shape.Circle(5.0)` -- no global variant names

#### Pattern syntax & nesting
- Or-patterns supported with pipe syntax: `Circle(_) | Point -> ...`
- As-patterns supported with `as` keyword: `Circle(_) as c -> use_circle(c)`
- Arbitrary nesting depth for destructuring: `Some(Circle(r)) -> r`
- Named fields destructure by name: `Rectangle(w: w, h: _) -> w`
- Pattern matching works in both case/match blocks AND function heads (multi-clause functions with exhaustiveness checking across clauses)

#### Guard behavior
- Restricted guard expressions: comparisons (`>`, `<`, `==`), boolean ops (`and`/`or`/`not`), and specific builtin functions (no user-defined functions in guards) -- Erlang/Elixir style
- Guards CAN reference bindings from the pattern: `Circle(r) when r > 0.0 -> ...`
- Guards do NOT count toward exhaustiveness -- a guarded arm is treated as potentially non-matching, so a fallback is required
- Consistent `when` keyword in both match arms and function heads

#### Compiler diagnostics
- Non-exhaustive match is a **hard error** (won't compile) -- Rust approach
- Redundant/unreachable pattern arm is a **warning** (compiles, dead code flagged)
- Missing pattern errors list missing variants explicitly: "Non-exhaustive match: missing Circle(_, _), Point"

### Claude's Discretion
- Fix suggestion style for non-exhaustive errors (suggest explicit arms, wildcard, or both)
- Exact exhaustiveness algorithm implementation details (Maranget's or variation)
- Internal representation of sum types and pattern compilation

### Deferred Ideas (OUT OF SCOPE)
None -- discussion stayed within phase scope
</user_constraints>

## Summary

This phase adds algebraic data types (sum types) and exhaustive pattern matching to the Snow compiler. The work spans all three compiler crates (lexer already has needed tokens, parser needs sum type definitions and new pattern node types, type checker needs ADT registration and exhaustiveness/redundancy analysis). The existing codebase has strong foundations: the parser already has `WILDCARD_PAT`, `IDENT_PAT`, `LITERAL_PAT`, `TUPLE_PAT`, and `STRUCT_PAT` syntax kinds; the `TYPE_KW` and `WHEN_KW` keywords exist; the type checker has `Ty::App`-based generic types and level-based polymorphism.

The standard approach for exhaustiveness checking is Maranget's usefulness algorithm from "Warnings for pattern matching" (JFP 2007). This algorithm is used by Rust, OCaml, Dart, and (in-progress) Elixir. The core idea: a pattern is "useful" relative to a pattern matrix if there exists a value matching the new pattern but no row in the matrix. Exhaustiveness reduces to checking whether wildcard `_` is useful relative to the full matrix. Redundancy reduces to checking whether each arm is useful relative to preceding arms.

**Primary recommendation:** Implement Maranget's usefulness algorithm (Algorithm U) with constructor specialization, operating on a pattern matrix representation. Build sum types as a new `SumTypeDef` registry (analogous to the existing `StructDefInfo`), with variant constructors registered as polymorphic function schemes. Reuse the existing `Ty::App` representation for sum type instances.

## Standard Stack

This is a compiler-internal implementation phase -- no new external libraries needed.

### Core (existing workspace dependencies)
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| rowan | 0.16 | Lossless CST nodes | Already used for all parser nodes |
| ena | 0.14 | Union-find for type inference | Already used for HM unification |
| rustc-hash | 2 | Fast hash maps | Already used throughout typeck |
| ariadne | 0.6 | Diagnostic rendering | Already used for error messages |
| insta | 1.46 | Snapshot testing | Already used for test assertions |

### No new dependencies needed
The exhaustiveness algorithm is pure logic operating on the existing type system data structures. No external pattern matching library is needed -- the algorithm is straightforward to implement from Maranget's paper and the practical guidance in the Rust compiler dev guide.

## Architecture Patterns

### New CST/AST Node Types Needed

```
SyntaxKind additions:
  SUM_TYPE_DEF        # type Shape do ... end
  VARIANT_DEF         # Circle(Float) or Rectangle(width: Float, height: Float)
  VARIANT_FIELD       # width: Float (inside a variant)
  CONSTRUCTOR_PAT     # Shape.Circle(r) or Some(x) in patterns
  OR_PAT              # Circle(_) | Point
  AS_PAT              # Circle(_) as c
  GUARD_CLAUSE        # when r > 0.0 (optional: could reuse WHEN_KW inline)
```

### Recommended File Structure for New Code

```
crates/snow-parser/src/
  parser/
    patterns.rs         # EXTEND: add constructor_pat, or_pat, as_pat parsing
    items.rs            # EXTEND: add parse_sum_type_def
  ast/
    pat.rs              # EXTEND: add ConstructorPat, OrPat, AsPat to Pattern enum
    item.rs             # EXTEND: add SumTypeDef, VariantDef to Item enum
  syntax_kind.rs        # EXTEND: add new SyntaxKind variants

crates/snow-typeck/src/
  infer.rs              # EXTEND: register_sum_type, infer constructor patterns
  exhaustiveness.rs     # NEW: Maranget's usefulness algorithm
  ty.rs                 # No changes needed -- Ty::App already handles ADTs
  error.rs              # EXTEND: NonExhaustiveMatch, RedundantArm error variants
  diagnostics.rs        # EXTEND: render new error types
```

### Pattern 1: Sum Type Internal Representation

**What:** Sum types are registered in a `SumTypeRegistry` (extension of existing `TypeRegistry`) that maps type names to their variant definitions. Each variant is a constructor with a name, optional fields, and the parent sum type.

**When to use:** During the two-pass type checking approach (register definitions first, then check bodies).

**Example:**
```rust
// Internal representation -- extend TypeRegistry
struct SumTypeDef {
    name: String,                    // "Shape"
    generic_params: Vec<String>,     // ["T"] for generic sum types
    variants: Vec<VariantDef>,       // [Circle(..), Rectangle(..), Point]
}

struct VariantDef {
    name: String,                    // "Circle"
    fields: Vec<VariantField>,       // positional or named fields
}

struct VariantField {
    name: Option<String>,            // None for positional, Some("width") for named
    ty: Ty,                          // field type (may reference generic params)
}
```

### Pattern 2: Constructor Registration as Functions

**What:** Each variant constructor is registered in the type environment as a polymorphic function. `Shape.Circle` becomes `forall <nothing>. (Float) -> Shape` and `Option.Some` becomes `forall T. (T) -> Option<T>`. Nullary constructors (no fields) are registered as constants of their type, not functions.

**When to use:** During sum type registration (analogous to existing `register_struct_def`).

**Example:**
```rust
// For: type Option<T> do Some(T) None end

// Some :: forall T. (T) -> Option<T>
// Registered as a polymorphic function scheme
ctx.enter_level();
let t_var = ctx.fresh_var();
let some_ty = Ty::Fun(vec![t_var.clone()], Box::new(Ty::option(t_var)));
ctx.leave_level();
let scheme = ctx.generalize(some_ty);
env.insert("Option.Some".into(), scheme);

// None :: forall T. Option<T>
// Registered as a polymorphic constant (not a function)
ctx.enter_level();
let t_var = ctx.fresh_var();
let none_ty = Ty::option(t_var);
ctx.leave_level();
let scheme = ctx.generalize(none_ty);
env.insert("Option.None".into(), scheme);
```

### Pattern 3: Maranget's Usefulness Algorithm

**What:** The exhaustiveness checker operates on a "pattern matrix" -- a list of rows where each row is a list of patterns (one per column). The algorithm recursively decomposes the matrix using "specialization" by constructor.

**When to use:** After type-checking a `case` expression or collecting all clauses of a multi-clause function.

**Core algorithm pseudocode:**
```
useful(matrix, pattern_row) -> bool:
    if matrix has 0 columns:
        return matrix has 0 rows  // empty matrix = pattern is useful

    let first_pat = pattern_row[0]

    if first_pat is a constructor C(p1..pn):
        // Specialize: keep only rows starting with C, expand C's fields
        let specialized = specialize(matrix, C)
        let specialized_row = C's sub-patterns ++ rest of pattern_row
        return useful(specialized, specialized_row)

    if first_pat is wildcard _:
        // Try each constructor of the type
        let constructors = all_constructors(column_type)
        let heads = constructors_in_first_column(matrix)

        if heads covers all constructors (complete signature):
            // Must be useful against at least one specialization
            return any(C in heads, useful(specialize(matrix, C),
                        wildcards(arity(C)) ++ rest of pattern_row))
        else:
            // Default matrix: keep rows with wildcard in first column
            return useful(default_matrix(matrix), rest of pattern_row)
```

### Pattern 4: Qualified Variant Access (Shape.Circle)

**What:** Variant construction uses qualified paths: `Shape.Circle(5.0)`. In the parser, this is parsed as a `FIELD_ACCESS` expression on a `NAME_REF`. The type checker must recognize when a field access is actually a variant constructor reference.

**When to use:** During expression inference, when encountering `Name.Variant(args)`.

**Design choice:** The parser already handles `expr.field` as `FIELD_ACCESS`. For variant construction, the sequence `Shape.Circle(5.0)` parses naturally as `CALL_EXPR(FIELD_ACCESS(NAME_REF(Shape), Circle), ARG_LIST(5.0))`. The type checker resolves `Shape.Circle` by looking up the sum type registry. This avoids new parser syntax.

### Pattern 5: Multi-Clause Functions

**What:** Functions with multiple clauses where each clause has different patterns in parameters. Exhaustiveness is checked across all clauses together, treating the parameter tuple as the scrutinee.

**Syntax approach:**
```snow
fn area(shape :: Shape) do
  match shape do
    Shape.Circle(r) -> 3.14 * r * r
    Shape.Rectangle(w: w, h: h) -> w * h
    Shape.Point -> 0.0
  end
end
```

Or with multi-clause function heads:
```snow
fn area(Shape.Circle(r)) do
  3.14 * r * r
end

fn area(Shape.Rectangle(w: w, h: h)) do
  w * h
end

fn area(Shape.Point) do
  0.0
end
```

**Implementation approach:** Multi-clause functions are syntactically separate `FN_DEF` nodes with the same name. During type checking, collect all clauses of the same function, extract their parameter patterns, build a pattern matrix, and run exhaustiveness checking. This is the Elixir model.

### Anti-Patterns to Avoid

- **Inlining exhaustiveness logic into the type checker:** Keep exhaustiveness as a separate module (`exhaustiveness.rs`) that receives resolved types and patterns, not raw AST. This makes it testable independently.
- **Modifying Ty enum for sum types:** Sum types are already representable as `Ty::App(Box::new(Ty::Con(TyCon::new("Shape"))), vec![])`. Do not add a new `Ty::Sum` variant. The type system already works.
- **Expanding or-patterns eagerly in the parser:** Or-patterns should be a single AST node containing sub-patterns. The exhaustiveness checker handles them during matrix construction, not the parser.
- **Mixing guard checking with exhaustiveness:** Guards are explicitly excluded from exhaustiveness. A guarded arm contributes nothing to the exhaustiveness matrix (treat it as if it might not match). This is a clear, simple rule.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Pattern matrix operations | Custom ad-hoc checks per case | Maranget's usefulness algorithm | Handles nested patterns, or-patterns, and wildcards correctly; proven correct |
| Missing pattern witnesses | Simple "which variants are missing" | Algorithm I (witness generation) from Maranget | Generates concrete example patterns for error messages |
| Variant constructor type registration | Manual type construction per variant | Reuse existing `enter_level/leave_level/generalize` pattern | Already proven for Option/Result constructors in Phase 3 |
| Type annotation parsing for variant fields | New parsing logic | Reuse existing `parse_type()` and `collect_annotation_tokens` | Already handles generics, sugar, nested types |

**Key insight:** The existing Phase 3 infrastructure for registering Option/Result constructors (`register_option_result_constructors`) is exactly the pattern needed for all sum type variant constructors. The existing `resolve_type_annotation` and `substitute_type_params` functions handle generic field types. The main new code is the exhaustiveness algorithm itself.

## Common Pitfalls

### Pitfall 1: Option/Result Migration Breakage
**What goes wrong:** Replacing builtin Option/Result with user-defined ADTs breaks existing tests and type inference for `?` and `!` sugar.
**Why it happens:** The builtins use global names (`Some`, `None`, `Ok`, `Err`) while user-defined ADTs use qualified names (`Option.Some`).
**How to avoid:** Keep backward compatibility during migration. Either (a) keep the global names as aliases that resolve to the user-defined ADT constructors, or (b) defer the migration to the end of the phase after all new infrastructure is tested. Recommend approach (b): first build sum types as a parallel system, test it, then migrate Option/Result.
**Warning signs:** Tests in Phase 3 that use `Some(x)` or `Ok(x)` start failing.

### Pitfall 2: Qualified vs Unqualified Constructor Names
**What goes wrong:** The lookup for `Shape.Circle` in the type checker fails because the name resolution doesn't connect dot-access to sum type variant lookup.
**Why it happens:** `Shape.Circle(5.0)` parses as `CALL_EXPR(FIELD_ACCESS(Shape, Circle), args)`. The type checker's `infer_field_access` currently only knows about struct fields, not variant constructors.
**How to avoid:** Extend `infer_field_access` (or add a separate check before it) to recognize when the base is a sum type name and the field is a variant name. Return the variant constructor's function type.
**Warning signs:** "type Shape has no field Circle" errors.

### Pitfall 3: Exhaustiveness Over-Reporting with Guards
**What goes wrong:** A match with guards on every arm is reported as non-exhaustive even though one arm has a wildcard pattern.
**Why it happens:** If the rule is "guarded arms don't count toward exhaustiveness," then `_ when x > 0 -> ...` is excluded from the matrix.
**How to avoid:** The rule is: a guarded arm is included in the matrix with its pattern shape, but is treated as "potentially non-matching" -- meaning the exhaustiveness checker should require a non-guarded fallback arm if all constructors are only covered by guarded arms. Implementation: when building the pattern matrix, include guarded arms' patterns but mark them. After running usefulness, if the remaining uncovered patterns could only be caught by guarded arms, report non-exhaustive.
**Warning signs:** False positive non-exhaustive errors on guarded patterns.

### Pitfall 4: Nested Pattern Exhaustiveness
**What goes wrong:** `case opt do Some(Circle(r)) -> ... None -> ... end` is reported as exhaustive, but it actually misses `Some(Rectangle(...))` and `Some(Point)`.
**Why it happens:** Incorrectly treating `Some(Circle(r))` as covering all `Some(_)` values.
**How to avoid:** Maranget's algorithm handles this correctly through recursive specialization. When specializing by `Some`, the sub-patterns must be checked against the inner type's constructors. This is the core strength of the matrix-based approach.
**Warning signs:** Missing variant errors not appearing for nested patterns.

### Pitfall 5: Or-Pattern Variable Binding Inconsistency
**What goes wrong:** `Circle(r) | Rectangle(r) -> use(r)` compiles but `r` has different types in each alternative.
**Why it happens:** Or-patterns bind the same name in multiple alternatives, and the types must be compatible.
**How to avoid:** Validate that all alternatives in an or-pattern bind the same set of variable names with compatible types. This is a semantic check after parsing. Wildcard `_` is compatible with any binding.
**Warning signs:** Runtime type confusion, or unclear type inference errors.

### Pitfall 6: Infinite Recursion in Type Resolution During Exhaustiveness
**What goes wrong:** A sum type like `type List<T> do Cons(T, List<T>) Nil end` causes the exhaustiveness checker to recurse infinitely trying to enumerate nested patterns.
**Why it happens:** Recursive types have unbounded constructor nesting.
**How to avoid:** Limit the exhaustiveness checking depth. When recursing into a type that has already been seen in the current path, treat it as opaque (wildcard). Rust and OCaml both use a recursion depth limit. For a first implementation, limiting to 2-3 levels of nesting is sufficient.
**Warning signs:** Stack overflow during exhaustiveness checking.

## Code Examples

### Sum Type Parsing (CST Structure)

```
// Input: type Shape do Circle(Float) Rectangle(width: Float, height: Float) Point end
// CST:
SUM_TYPE_DEF
  TYPE_KW "type"
  NAME "Shape"
  DO_KW "do"
  VARIANT_DEF
    NAME "Circle"
    L_PAREN "("
    TYPE_ANNOTATION (Float)
    R_PAREN ")"
  VARIANT_DEF
    NAME "Rectangle"
    L_PAREN "("
    VARIANT_FIELD
      NAME "width"
      COLON ":"
      TYPE_ANNOTATION (Float)
    COMMA ","
    VARIANT_FIELD
      NAME "height"
      COLON ":"
      TYPE_ANNOTATION (Float)
    R_PAREN ")"
  VARIANT_DEF
    NAME "Point"
  END_KW "end"
```

### Constructor Pattern Parsing

```
// Input: case shape do Shape.Circle(r) -> r * r end
// The pattern Shape.Circle(r) needs a CONSTRUCTOR_PAT node:
CONSTRUCTOR_PAT
  PATH
    IDENT "Shape"
    DOT "."
    IDENT "Circle"
  L_PAREN "("
  IDENT_PAT "r"
  R_PAREN ")"
```

### Exhaustiveness Algorithm Core Structure

```rust
// Source: Based on Maranget 2007 and Rust compiler dev guide

/// A pattern in the exhaustiveness matrix.
#[derive(Clone, Debug)]
enum CheckPat {
    /// Constructor with sub-patterns: Circle(p1), Some(p2), (p1, p2)
    Constructor {
        type_name: String,       // "Shape", "Option", etc.
        variant_name: String,    // "Circle", "Some", etc.
        sub_pats: Vec<CheckPat>, // sub-patterns for fields
    },
    /// Wildcard: matches anything
    Wildcard,
    /// Literal: 42, "hello", true
    Literal(LiteralValue),
    /// Or-pattern: p1 | p2 | p3
    Or(Vec<CheckPat>),
}

/// The pattern matrix for exhaustiveness checking.
struct PatternMatrix {
    rows: Vec<Vec<CheckPat>>,  // each row = one match arm's patterns
    col_types: Vec<Ty>,        // type of each column
}

/// Check if a new pattern row is useful relative to the matrix.
/// Returns true if there exists a value matching the row but no row in the matrix.
fn is_useful(
    matrix: &PatternMatrix,
    row: &[CheckPat],
    sum_type_registry: &SumTypeRegistry,
) -> bool {
    // Base case: zero columns
    if row.is_empty() {
        return matrix.rows.is_empty();
    }

    let first_col_type = &matrix.col_types[0];
    let first_pat = &row[0];

    match first_pat {
        CheckPat::Constructor { variant_name, sub_pats, .. } => {
            let specialized = specialize_matrix(matrix, variant_name, sum_type_registry);
            let new_row: Vec<CheckPat> = sub_pats.iter().cloned()
                .chain(row[1..].iter().cloned())
                .collect();
            is_useful(&specialized, &new_row, sum_type_registry)
        }
        CheckPat::Wildcard => {
            let head_constructors = collect_head_constructors(matrix);
            let all_constructors = get_all_constructors(first_col_type, sum_type_registry);

            if is_complete_signature(&head_constructors, &all_constructors) {
                // Try each constructor
                all_constructors.iter().any(|ctor| {
                    let specialized = specialize_matrix(matrix, &ctor.name, sum_type_registry);
                    let wildcard_fields = vec![CheckPat::Wildcard; ctor.arity];
                    let new_row: Vec<CheckPat> = wildcard_fields.into_iter()
                        .chain(row[1..].iter().cloned())
                        .collect();
                    is_useful(&specialized, &new_row, sum_type_registry)
                })
            } else {
                // Default matrix
                let default = default_matrix(matrix);
                is_useful(&default, &row[1..], sum_type_registry)
            }
        }
        CheckPat::Or(alternatives) => {
            // An or-pattern is useful if any alternative is useful
            alternatives.iter().any(|alt| {
                let new_row: Vec<CheckPat> = std::iter::once(alt.clone())
                    .chain(row[1..].iter().cloned())
                    .collect();
                is_useful(matrix, &new_row, sum_type_registry)
            })
        }
        CheckPat::Literal(_) => {
            // Treat literals like constructors with zero arity
            let specialized = specialize_matrix_literal(matrix, first_pat);
            is_useful(&specialized, &row[1..], sum_type_registry)
        }
    }
}

/// Specialize the matrix by a constructor: keep rows whose first pattern
/// is the given constructor (expanding its fields) or wildcard (expanding
/// to wildcards matching the constructor's arity).
fn specialize_matrix(
    matrix: &PatternMatrix,
    ctor_name: &str,
    registry: &SumTypeRegistry,
) -> PatternMatrix {
    let arity = registry.variant_arity(ctor_name);
    let mut new_rows = Vec::new();

    for row in &matrix.rows {
        match &row[0] {
            CheckPat::Constructor { variant_name, sub_pats, .. } if variant_name == ctor_name => {
                let new_row: Vec<CheckPat> = sub_pats.iter().cloned()
                    .chain(row[1..].iter().cloned())
                    .collect();
                new_rows.push(new_row);
            }
            CheckPat::Wildcard => {
                let new_row: Vec<CheckPat> = vec![CheckPat::Wildcard; arity]
                    .into_iter()
                    .chain(row[1..].iter().cloned())
                    .collect();
                new_rows.push(new_row);
            }
            CheckPat::Or(alternatives) => {
                // Expand or-patterns that contain the constructor
                for alt in alternatives {
                    if let CheckPat::Constructor { variant_name, sub_pats, .. } = alt {
                        if variant_name == ctor_name {
                            let new_row: Vec<CheckPat> = sub_pats.iter().cloned()
                                .chain(row[1..].iter().cloned())
                                .collect();
                            new_rows.push(new_row);
                        }
                    } else if matches!(alt, CheckPat::Wildcard) {
                        let new_row: Vec<CheckPat> = vec![CheckPat::Wildcard; arity]
                            .into_iter()
                            .chain(row[1..].iter().cloned())
                            .collect();
                        new_rows.push(new_row);
                    }
                }
            }
            _ => {
                // Different constructor -- skip this row
            }
        }
    }

    // Update column types: replace first column with ctor's field types,
    // keep remaining columns
    let field_types = registry.variant_field_types(ctor_name);
    let new_col_types: Vec<Ty> = field_types.into_iter()
        .chain(matrix.col_types[1..].iter().cloned())
        .collect();

    PatternMatrix { rows: new_rows, col_types: new_col_types }
}
```

### Witness Generation for Error Messages

```rust
/// Generate witness patterns (missing pattern examples) for non-exhaustive matches.
/// Based on Maranget's Algorithm I.
fn compute_witnesses(
    matrix: &PatternMatrix,
    sum_type_registry: &SumTypeRegistry,
) -> Vec<Vec<CheckPat>> {
    if matrix.col_types.is_empty() {
        return if matrix.rows.is_empty() {
            vec![vec![]]  // One empty witness
        } else {
            vec![]  // No witnesses
        };
    }

    let first_col_type = &matrix.col_types[0];
    let head_ctors = collect_head_constructors(matrix);
    let all_ctors = get_all_constructors(first_col_type, sum_type_registry);

    if is_complete_signature(&head_ctors, &all_ctors) {
        // All constructors present -- recurse into each
        let mut witnesses = Vec::new();
        for ctor in &all_ctors {
            let specialized = specialize_matrix(matrix, &ctor.name, sum_type_registry);
            for witness_tail in compute_witnesses(&specialized, sum_type_registry) {
                let (sub_pats, rest) = witness_tail.split_at(ctor.arity);
                let ctor_pat = CheckPat::Constructor {
                    type_name: first_col_type.to_string(),
                    variant_name: ctor.name.clone(),
                    sub_pats: sub_pats.to_vec(),
                };
                let mut full = vec![ctor_pat];
                full.extend_from_slice(rest);
                witnesses.push(full);
            }
        }
        witnesses
    } else {
        // Missing constructors -- find which ones
        let missing: Vec<_> = all_ctors.iter()
            .filter(|c| !head_ctors.contains(&c.name))
            .collect();

        let default = default_matrix(matrix);
        let tail_witnesses = compute_witnesses(&default, sum_type_registry);

        let mut witnesses = Vec::new();
        for missing_ctor in &missing {
            for tail in &tail_witnesses {
                let sub_pats = vec![CheckPat::Wildcard; missing_ctor.arity];
                let ctor_pat = CheckPat::Constructor {
                    type_name: first_col_type.to_string(),
                    variant_name: missing_ctor.name.clone(),
                    sub_pats,
                };
                let mut full = vec![ctor_pat];
                full.extend_from_slice(tail);
                witnesses.push(full);
            }
        }
        witnesses
    }
}
```

### Guard Handling in Pattern Matrix

```rust
/// Build the pattern matrix from match arms, handling guards.
fn build_matrix_from_arms(
    arms: &[MatchArmInfo],
    scrutinee_type: &Ty,
) -> (PatternMatrix, Vec<usize>) {
    let mut matrix_rows = Vec::new();
    let mut guarded_indices = Vec::new();

    for (i, arm) in arms.iter().enumerate() {
        let check_pat = lower_pattern_to_check_pat(&arm.pattern);
        matrix_rows.push(vec![check_pat]);

        if arm.has_guard {
            guarded_indices.push(i);
        }
    }

    let matrix = PatternMatrix {
        rows: matrix_rows,
        col_types: vec![scrutinee_type.clone()],
    };

    (matrix, guarded_indices)
}

/// Check exhaustiveness with guard awareness.
/// Guards don't count toward exhaustiveness -- build a "sans-guard" matrix
/// that excludes guarded arms, then check if wildcard is useful against it.
fn check_exhaustiveness_with_guards(
    arms: &[MatchArmInfo],
    scrutinee_type: &Ty,
    registry: &SumTypeRegistry,
) -> Result<(), Vec<CheckPat>> {
    // Build matrix excluding guarded arms for exhaustiveness
    let unguarded_rows: Vec<Vec<CheckPat>> = arms.iter()
        .filter(|arm| !arm.has_guard)
        .map(|arm| vec![lower_pattern_to_check_pat(&arm.pattern)])
        .collect();

    let matrix = PatternMatrix {
        rows: unguarded_rows,
        col_types: vec![scrutinee_type.clone()],
    };

    // Check if wildcard is useful (= matrix is not exhaustive)
    let wildcard_row = vec![CheckPat::Wildcard];
    if is_useful(&matrix, &wildcard_row, registry) {
        let witnesses = compute_witnesses(&matrix, registry);
        Err(witnesses.into_iter().map(|w| w[0].clone()).collect())
    } else {
        Ok(())
    }
}

/// Check redundancy: each arm should be useful relative to preceding arms.
/// Include ALL arms (guarded and unguarded) for redundancy checking.
fn check_redundancy(
    arms: &[MatchArmInfo],
    scrutinee_type: &Ty,
    registry: &SumTypeRegistry,
) -> Vec<usize> {
    let mut redundant = Vec::new();
    let mut matrix = PatternMatrix {
        rows: Vec::new(),
        col_types: vec![scrutinee_type.clone()],
    };

    for (i, arm) in arms.iter().enumerate() {
        let row = vec![lower_pattern_to_check_pat(&arm.pattern)];
        if !is_useful(&matrix, &row, registry) {
            redundant.push(i);
        }
        matrix.rows.push(row);
    }

    redundant
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Ad-hoc completeness checks per type | Maranget's usefulness matrix algorithm | 2007 (paper), adopted widely by 2015 | Handles nested patterns, or-patterns, and arbitrary ADT depth correctly |
| Separate exhaustiveness and redundancy passes | Single usefulness predicate handles both | Maranget 2007 | Simpler implementation, proven correct |
| Global variant names | Qualified/namespaced variants | Modern practice (Rust, Swift, Kotlin) | Avoids name collisions, clearer code |

**Deprecated/outdated:**
- Simple "switch completeness" checking (only works for flat, non-nested patterns) -- replaced by matrix-based approaches
- Separate algorithms for exhaustiveness vs redundancy -- both are instances of usefulness

## Recommendations for Claude's Discretion Items

### Fix Suggestion Style for Non-Exhaustive Errors
**Recommendation:** Suggest both explicit missing arms AND a wildcard fallback. Show missing arms first (more informative), then suggest `_ -> ...` as an alternative. This is the Rust approach.

Example diagnostic:
```
error[E0010]: non-exhaustive match: missing Shape.Circle(_, _), Shape.Point
  --> test.snow:5:1
  |
5 | case shape do
  | ^^^^ patterns Shape.Circle(_, _) and Shape.Point not covered
  |
help: ensure all variants are matched:
  |   Shape.Circle(_, _) -> ...
  |   Shape.Point -> ...
  |
  = or add a wildcard arm: _ -> ...
```

### Exhaustiveness Algorithm Choice
**Recommendation:** Implement Maranget's Algorithm U (usefulness) with Algorithm I (witness generation) for error messages. This is the standard choice used by Rust, OCaml, Dart, and being adopted by Elixir. The algorithm is well-documented, proven correct, and handles all the pattern forms Snow needs (constructors, wildcards, or-patterns, literals, nested patterns).

Do NOT implement decision tree compilation (Maranget's other paper "Compiling Pattern Matching to Good Decision Trees") -- that is about runtime execution efficiency and is not needed for a compiler that targets a different backend.

### Internal Representation of Sum Types
**Recommendation:** Extend the existing `TypeRegistry` with sum type awareness. Store `SumTypeDef` alongside `StructDefInfo`. Sum type instances use `Ty::App(Box::new(Ty::Con(TyCon::new("Shape"))), type_args)` -- identical to struct type representation. Variant constructors are registered as function schemes in the type environment using the same `enter_level/leave_level/generalize` pattern already used for Option/Result.

The key distinction from structs: structs have one "constructor" (the struct literal), while sum types have multiple constructors (one per variant). The exhaustiveness checker needs to know all constructors for a given type, which structs don't require.

## Open Questions

1. **Multi-clause function parsing ambiguity**
   - What we know: Multiple `fn area(...)` definitions with different patterns need to be grouped into a single function for exhaustiveness checking.
   - What's unclear: How does the parser distinguish "this is another clause of the same function" from "this is a redefinition"? Elixir uses adjacency (clauses must be consecutive). Snow could use the same rule.
   - Recommendation: Require multi-clause functions to be adjacent. The type checker groups consecutive `FN_DEF` nodes with the same name. If separated by other items, treat as a redefinition error.

2. **Named field destructuring in patterns**
   - What we know: `Rectangle(w: w, h: _)` destructures by field name. This needs a pattern that can reference named fields.
   - What's unclear: Should partial field patterns be allowed? (`Rectangle(w: w)` without `h`?) In Rust, you need `..` to skip fields.
   - Recommendation: Require all fields to be present in named field patterns (no partial patterns without explicit wildcard). This simplifies exhaustiveness and matches the "explicit is better" philosophy.

3. **Pipe `|` token conflict with or-patterns**
   - What we know: `|>` is the pipe operator (already in the lexer as `Pipe`). Or-patterns use `|` between pattern alternatives.
   - What's unclear: How to distinguish `|` in pattern context from `|>` in expression context.
   - Recommendation: The lexer already emits `|>` as a single `Pipe` token, not `|` + `>`. The bare `|` would need a new token kind (or reuse the existing `PIPE_PIPE` = `||`). Actually, checking the lexer: there is no bare `|` token -- `|>` is `Pipe`. For or-patterns, we need to introduce a bare `|` pattern separator. In the lexer, the `|` character currently produces an `Error` token (per decision [01-03]). For or-patterns, the pattern parser can check for `Error` tokens with text `"|"` (similar to how trailing closures already handle bare `|`), OR add a new `BAR` token kind to the lexer. Recommend adding `BAR` to avoid relying on error tokens.

## Sources

### Primary (HIGH confidence)
- Existing Snow codebase (all files read directly) -- current parser, AST, type checker architecture
- Maranget, L. "Warnings for pattern matching" JFP 2007 -- [paper](http://moscova.inria.fr/~maranget/papers/warn/warn.pdf)
- [Rust Compiler Dev Guide - Pattern and Exhaustiveness Checking](https://rustc-dev-guide.rust-lang.org/pat-exhaustive-checking.html) -- practical implementation guidance

### Secondary (MEDIUM confidence)
- [Dart exhaustiveness specification](https://github.com/dart-lang/language/blob/main/accepted/3.0/patterns/exhaustiveness.md) -- adapted Maranget for subtyping
- [Elixir patterns and guards documentation](https://hexdocs.pm/elixir/patterns-and-guards.html) -- guard restriction model
- [yorickpeterse/pattern-matching-in-rust](https://github.com/yorickpeterse/pattern-matching-in-rust) -- reference implementations

### Tertiary (LOW confidence)
- [Elixir type inference blog post (Jan 2026)](http://elixir-lang.org/blog/2026/01/09/type-inference-of-all-and-next-15/) -- in-progress exhaustiveness for Elixir

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- no new dependencies, extending existing architecture
- Architecture: HIGH -- based on direct codebase reading and established algorithms
- Exhaustiveness algorithm: HIGH -- Maranget's algorithm is the standard, well-documented approach
- Pattern syntax integration: HIGH -- parser patterns already partially exist, clear extension points
- Multi-clause functions: MEDIUM -- some open design questions about grouping and adjacency
- Option/Result migration: MEDIUM -- backward compatibility strategy needs care

**Research date:** 2026-02-06
**Valid until:** 2026-03-06 (stable domain, algorithms don't change)
