---
phase: 05-llvm-codegen-native-binaries
plan: 03
subsystem: pattern-compilation
tags: [pattern-matching, decision-tree, maranget, switch, guard, or-pattern, tuple-decomposition]

# Dependency graph
requires:
  - phase: 05-llvm-codegen-native-binaries
    plan: 02
    provides: MIR type system (MirPattern, MirMatchArm, MirExpr, MirType, MirLiteral)
provides:
  - DecisionTree type definitions (Leaf, Switch, Test, Guard, Fail)
  - AccessPath type (Root, TupleField, VariantField, StructField)
  - ConstructorTag type for sum type variant identification
  - compile_match() function for individual match expression compilation
  - compile_patterns() module-level pass for MirModule
  - Maranget-style pattern matrix algorithm with column selection heuristic
affects: [05-04, 05-05]

# Tech tracking
tech-stack:
  added: []
  patterns: [Maranget pattern matrix compilation, or-pattern expansion by arm duplication, tuple column decomposition, literal test chain with default fallback]

key-files:
  created:
    - crates/snow-codegen/src/pattern/mod.rs
    - crates/snow-codegen/src/pattern/compile.rs
  modified:
    - crates/snow-codegen/src/lib.rs

key-decisions:
  - "Constructor sub-pattern bindings collected via recursive column processing, not via MirPattern::Constructor bindings field (avoids double-counting)"
  - "Or-patterns expanded before matrix construction by duplicating (arm_index, pattern, guard) tuples"
  - "Boolean literal exhaustive match produces Test chain with terminal Fail (LLVM will optimize away unreachable Fail)"
  - "Tuple patterns handled by column expansion (not Switch/Test) -- structural decomposition before head constructor analysis"
  - "Column selection heuristic: column with most distinct head constructors wins"
  - "Guard failure chains: guarded arm wraps Leaf in Guard node, failure continues to next row or Fail"

patterns-established:
  - "PatMatrix: rows of PatRow with column_paths and column_types for position tracking"
  - "HeadCtor enum: Literal vs Constructor for driving Switch vs Test compilation"
  - "Specialize/default matrix operations for Maranget's algorithm"
  - "compile_expr_patterns() recursive MirExpr walker for module-level pattern compilation"

# Metrics
duration: 8min
completed: 2026-02-06
---

# Phase 5 Plan 3: Pattern Match Compilation to Decision Trees Summary

**Maranget-style pattern match compilation producing Switch/Test/Guard/Leaf/Fail decision trees from MIR match expressions**

## Performance

- **Duration:** 8 min
- **Started:** 2026-02-06T23:16:34Z
- **Completed:** 2026-02-06T23:24:54Z
- **TDD Phases:** 3 (RED, GREEN, REFACTOR)
- **Files created:** 2
- **Files modified:** 1
- **Lines of code:** ~960 (implementation) + tests

## Accomplishments

- DecisionTree type with 5 node types: Leaf (arm execution with bindings), Switch (constructor tag dispatch), Test (literal equality), Guard (guard expression evaluation), Fail (runtime panic)
- AccessPath type with 4 variants: Root, TupleField, VariantField, StructField -- describes how to reach any sub-value of the scrutinee
- ConstructorTag type carrying type_name, variant_name, numeric tag, and arity
- Maranget-style pattern matrix algorithm: column selection heuristic, matrix specialization, default matrix, recursive compilation
- Or-pattern expansion: duplicates arms with same arm_index for each alternative before matrix construction
- Constructor specialization: expands sub-patterns as new columns, wildcard/variable rows padded with wildcards
- Literal test chain: builds Test nodes from last-to-first, default matrix produces failure branch
- Tuple decomposition: expands tuple column into sub-columns for each element position
- Guard handling: wraps Leaf in Guard node, failure branch chains to next row or Fail node
- compile_patterns() walks entire MirModule recursively processing all MirExpr::Match nodes
- 14 comprehensive tests covering wildcard, variable, integer literal, boolean literal, constructor switch, nested tuple+constructor, or-pattern literals, or-pattern constructors, guard, guard-with-fail, constructor-with-default, tuple, string literal, multiple guards
- Zero regressions across all 437 workspace tests (37 in snow-codegen, 400 in rest of workspace)

## Task Commits

Each TDD phase was committed atomically:

1. **RED: Failing tests** - `fb5a58d` (test)
2. **GREEN: Implementation** - `789610f` (feat)
3. **REFACTOR: Cleanup** - `9c61425` (refactor)

## Files Created/Modified

- `crates/snow-codegen/src/pattern/mod.rs` - DecisionTree, AccessPath, ConstructorTag type definitions
- `crates/snow-codegen/src/pattern/compile.rs` - Pattern matrix compiler: compile_match(), compile_patterns(), PatMatrix, PatRow, HeadCtor, specialization, default matrix, tuple expansion, guard handling, 14 tests
- `crates/snow-codegen/src/lib.rs` - Added `pub mod pattern;`

## Decisions Made

- **Sub-pattern bindings via recursive compilation**: Constructor sub-pattern variable bindings are collected when the expanded sub-pattern columns are processed recursively, not from the `bindings` field on `MirPattern::Constructor`. This avoids double-counting since the `fields` sub-patterns already carry the binding information.
- **Or-pattern expansion before matrix construction**: Or-patterns are expanded into multiple rows (one per alternative) with the same `arm_index` before the pattern matrix is built. This follows the locked decision from CONTEXT.md ("Or-patterns duplicate the arm body for each alternative").
- **Boolean literals produce Test chain with Fail**: When matching on booleans with no wildcard default (e.g., `true -> 1, false -> 0`), the compiler produces `Test(true, Leaf(0), Test(false, Leaf(1), Fail))`. The terminal Fail is unreachable for well-typed programs and will be optimized away by LLVM.
- **Tuple decomposition by column expansion**: Tuple patterns are not tested via Switch/Test nodes. Instead, a tuple column is expanded into its sub-columns (one per element), allowing the standard algorithm to process the element patterns individually. This naturally handles nested patterns.
- **Column selection heuristic**: Picks the column with the most distinct head constructors (literals or sum type variants). This produces smaller decision trees by testing the most discriminating position first.
- **Guard chaining**: When a guarded arm's pattern matches but the guard fails, execution falls through to the next row in the matrix. If no more rows exist, a Fail node with "non-exhaustive match" message is produced.

## Deviations from Plan

None -- plan executed exactly as written.

## Issues Encountered

None.

## User Setup Required

None.

## Next Phase Readiness

- Decision tree types are fully defined and ready for LLVM codegen (Plan 04)
- compile_match() takes scrutinee type + match arms and returns DecisionTree -- direct API for codegen
- Switch nodes map directly to LLVM switch instructions on constructor tags
- Test nodes map to LLVM icmp/fcmp + conditional branch
- Guard nodes map to LLVM conditional branch on guard expression evaluation
- Leaf nodes provide arm_index (for body lookup) and bindings (for variable materialization)
- Fail nodes map to runtime panic call with source location
- compile_patterns() is available for module-level preprocessing but Plan 04 can also call compile_match() directly during codegen
- No blockers for subsequent plans

## Self-Check: PASSED
