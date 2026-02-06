---
phase: "04"
plan: "03"
subsystem: "type-checker"
tags: ["exhaustiveness", "pattern-matching", "algorithm-u", "maranget", "tdd"]
depends_on:
  requires: ["04-01"]
  provides: ["exhaustiveness-checking", "redundancy-checking", "pattern-matrix"]
  affects: ["04-04", "04-05"]
tech-stack:
  added: []
  patterns: ["Maranget Algorithm U", "pattern matrix specialization", "type registry for nested types"]
key-files:
  created:
    - "crates/snow-typeck/src/exhaustiveness.rs"
  modified:
    - "crates/snow-typeck/src/lib.rs"
decisions:
  - id: "04-03-registry"
    description: "TypeRegistry parameter on check_exhaustiveness/check_redundancy for complete nested type resolution"
    rationale: "Pattern-only inference cannot know about constructors not mentioned in patterns (e.g., Shape.Point when only Some(Circle(_)) is matched)"
  - id: "04-03-bool-as-named"
    description: "Bool literals treated as named constructors 'true'/'false' rather than literal constructors"
    rationale: "Unifies Bool with sum type handling -- Bool is a finite type with two constructors"
  - id: "04-03-api-change"
    description: "check_exhaustiveness and check_redundancy take &TypeRegistry parameter (deviation from plan's registry-free API)"
    rationale: "Required for correct nested exhaustiveness where inner type constructors are not visible in patterns"
metrics:
  duration: "13min"
  completed: "2026-02-06"
---

# Phase 4 Plan 03: Maranget's Usefulness Algorithm Summary

Implemented Algorithm U for exhaustiveness and redundancy checking of pattern matching, with TypeRegistry for nested type resolution.

## Task Commits

| Task | Type | Commit | Description |
|------|------|--------|-------------|
| RED | test | `0f51954` | 29 failing tests for is_useful, check_exhaustiveness, check_redundancy |
| GREEN | feat | `fa2fe16` | Full implementation passing all 29 tests |

## What Was Built

### Core Algorithm: `is_useful(matrix, row, type_info)`

Recursive predicate that determines whether a pattern row adds coverage to a pattern matrix. Implements Maranget's Algorithm U with these cases:

1. **Empty matrix**: Any pattern is useful (true)
2. **Empty row**: Not useful if matrix has rows (false)
3. **Constructor head**: Specialize matrix and row by the constructor, recurse
4. **Literal head**: Treat as nullary constructor (Bool literals as named constructors)
5. **Or-pattern head**: Useful if ANY alternative is useful
6. **Wildcard head**:
   - Finite type with all constructors covered: useful if ANY specialization reveals usefulness
   - Finite type incomplete: use default matrix
   - Infinite type: use default matrix

### Matrix Operations

- `specialize_matrix(matrix, ctor)`: Filter and expand rows by constructor
- `default_matrix(matrix)`: Keep only wildcard-fronted rows, drop first column
- `build_specialized_type_info(...)`: Infer TypeInfo for columns created by specialization

### Public API

- `check_exhaustiveness(arms, scrutinee_type, registry) -> Option<Vec<Pat>>`: Returns None if exhaustive, Some(witnesses) if not
- `check_redundancy(arms, scrutinee_type, registry) -> Vec<usize>`: Returns indices of unreachable arms
- `is_useful(matrix, row, type_info) -> bool`: Core predicate (builds internal registry from patterns)

### TypeRegistry

Maps type names to their complete constructor sets. Essential for nested patterns -- when checking `Option<Shape>`, after specializing by `Some`, the inner column needs Shape's full constructor set (Circle + Point), even if only Circle appears in the patterns.

## Test Coverage (29 tests)

- **Base cases**: Empty matrix/row, empty-empty
- **Bool**: Exhaustive (true+false), non-exhaustive (true only), wildcard
- **Sum types**: Exhaustive (Circle+Point), non-exhaustive (Circle only), wildcard
- **Redundancy**: Wildcard-then-constructor, no redundancy, duplicate arm
- **Nested**: Option<Shape> exhaustive and non-exhaustive
- **Or-patterns**: Exhaustive (Circle|Point), non-exhaustive (Circle|Circle)
- **Literals**: With wildcard, without wildcard, wildcard only
- **Multi-column**: 2-column bool matrix useful/not-useful
- **Nested specialization**: Direct is_useful test for type info propagation

## Decisions Made

1. **TypeRegistry parameter**: `check_exhaustiveness` and `check_redundancy` accept a `&TypeRegistry` for complete nested type resolution. The plan suggested a registry-free API, but this was necessary because pattern-only inference cannot determine the complete constructor set for types whose constructors don't all appear in the pattern set.

2. **Bool as named constructors**: Bool literals `true`/`false` are treated as `Constructor::Named` (not `Constructor::Literal`), unifying their handling with sum type constructors. This makes Bool a finite type with two known constructors.

3. **Pattern-based registry building**: `is_useful()` (the public standalone function) builds a TypeRegistry automatically by scanning all patterns. This works when all constructors appear somewhere in the matrix/row, but `check_exhaustiveness`/`check_redundancy` use the caller-provided registry for correctness.

## Deviations from Plan

### [Rule 2 - Missing Critical] TypeRegistry parameter added to public API

- **Found during:** GREEN phase, test_nested_non_exhaustive failure
- **Issue:** Without external type info, the algorithm couldn't detect that `Some(Circle(_)), None` is non-exhaustive for `Option<Shape>` because `Point` doesn't appear in any pattern
- **Fix:** Added `TypeRegistry` struct and `&TypeRegistry` parameter to `check_exhaustiveness` and `check_redundancy`
- **Files modified:** `crates/snow-typeck/src/exhaustiveness.rs`
- **Impact:** Plan 04-04 (AST-to-Pat translation) must build and pass a TypeRegistry

## Next Phase Readiness

Plan 04-04 can proceed: the exhaustiveness module provides a clean API for checking pattern match completeness. The translation layer needs to:
1. Convert AST patterns to `Pat` enum
2. Build a `TypeRegistry` from the type checker's known sum types
3. Call `check_exhaustiveness` and `check_redundancy`

## Self-Check: PASSED
