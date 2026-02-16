---
phase: 96-compiler-additions
plan: 03
subsystem: compiler
tags: [parser, typeck, mir, codegen, llvm, struct-update, immutable-data]

# Dependency graph
requires:
  - phase: 96-02
    provides: "Keyword args using COLON-separated MAP_ENTRY, multi-line pipe continuation"
provides:
  - "STRUCT_UPDATE_EXPR parser node with disambiguation from MAP_LITERAL"
  - "StructUpdate AST node with base_expr() and override_fields() accessors"
  - "Type checking: validates struct base, field existence, type compatibility"
  - "MirExpr::StructUpdate variant for MIR representation"
  - "LLVM codegen: alloc new struct, copy base fields, overwrite specified fields"
affects: [96-04, 96-05, 97-schema-metadata, 99-changesets]

# Tech tracking
tech-stack:
  added: []
  patterns: ["struct update expression %{base | field: value}", "parse-then-disambiguate with open_before for retroactive node wrapping"]

key-files:
  modified:
    - "crates/mesh-parser/src/syntax_kind.rs"
    - "crates/mesh-parser/src/parser/expressions.rs"
    - "crates/mesh-parser/src/ast/expr.rs"
    - "crates/mesh-typeck/src/infer.rs"
    - "crates/mesh-codegen/src/mir/mod.rs"
    - "crates/mesh-codegen/src/mir/lower.rs"
    - "crates/mesh-codegen/src/mir/mono.rs"
    - "crates/mesh-codegen/src/pattern/compile.rs"
    - "crates/mesh-codegen/src/codegen/expr.rs"
    - "crates/meshc/tests/e2e.rs"

key-decisions:
  - "Parse-then-disambiguate: parse first expr after %{, then check BAR vs FAT_ARROW to decide struct update vs map literal"
  - "Use open_before to retroactively wrap first map key in MAP_ENTRY when disambiguation resolves to map literal"
  - "Reuse STRUCT_LITERAL_FIELD nodes for struct update override fields (same name: value syntax)"
  - "Struct update codegen: alloc new struct on stack, copy all base fields, overwrite specified ones (value semantics, not GC heap)"

patterns-established:
  - "Parse-then-disambiguate pattern: parse an expression, then use the next token to decide node kind, using open_before for retroactive wrapping"
  - "Struct update expression %{base | field: value, ...} for immutable data transformation"

# Metrics
duration: 15min
completed: 2026-02-16
---

# Phase 96 Plan 03: Struct Update Expression Summary

**Struct update syntax `%{base | field: value}` fully implemented from parser through LLVM codegen with immutability-preserving value semantics**

## Performance

- **Duration:** 15 min
- **Started:** 2026-02-16T08:55:12Z
- **Completed:** 2026-02-16T09:10:20Z
- **Tasks:** 2
- **Files modified:** 10

## Accomplishments
- Struct update expression `%{user | name: "Bob"}` parses, type-checks, lowers to MIR, and compiles to LLVM IR
- Disambiguation between `%{base | field: value}` (struct update) and `%{key => value}` (map literal) works correctly
- Type checker validates base is a struct, override fields exist, and value types match field types
- Codegen creates a new struct with base fields copied and overrides applied (original unchanged)
- Three e2e tests verify: multi-field update, single-field update, and immutability preservation

## Task Commits

Each task was committed atomically:

1. **Task 1: Parse struct update syntax with disambiguation from map literals** - `17cd2954` (feat)
2. **Task 2: Type check, MIR lower, and codegen struct update with e2e tests** - `21da0ef9` (feat)

## Files Created/Modified
- `crates/mesh-parser/src/syntax_kind.rs` - Added STRUCT_UPDATE_EXPR composite node kind
- `crates/mesh-parser/src/parser/expressions.rs` - Disambiguation logic in parse_map_literal: parse-then-check BAR vs FAT_ARROW
- `crates/mesh-parser/src/ast/expr.rs` - StructUpdate AST node with base_expr() and override_fields() accessors
- `crates/mesh-typeck/src/infer.rs` - infer_struct_update validates base type, field existence, type compatibility
- `crates/mesh-codegen/src/mir/mod.rs` - MirExpr::StructUpdate variant with base, overrides, ty
- `crates/mesh-codegen/src/mir/lower.rs` - lower_struct_update lowers AST to MIR, collect_free_vars handles captures
- `crates/mesh-codegen/src/mir/mono.rs` - collect_function_refs handles StructUpdate
- `crates/mesh-codegen/src/pattern/compile.rs` - compile_expr_patterns handles StructUpdate
- `crates/mesh-codegen/src/codegen/expr.rs` - codegen_struct_update: alloc, copy base, overwrite, return new struct
- `crates/meshc/tests/e2e.rs` - Three e2e tests: basic, single_field, original_unchanged

## Decisions Made
- Parse-then-disambiguate: parse first expression after `%{`, check if next token is `BAR` (struct update) or `FAT_ARROW` (map literal). Use `open_before` to retroactively wrap the first map key expression in a MAP_ENTRY node when it resolves to a map literal.
- Reuse existing STRUCT_LITERAL_FIELD nodes for struct update override fields since they share the same `name: value` syntax.
- Codegen allocates the new struct on the stack (value semantics), copies all fields from the base, then overwrites specified fields. No GC heap allocation needed since structs are passed by value.
- Used NoSuchField error type for "struct update on non-struct" error since there's no generic TypeMismatch variant.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Added StructUpdate to mono.rs and pattern/compile.rs**
- **Found during:** Task 2
- **Issue:** collect_function_refs and compile_expr_patterns exhaustively match on MirExpr variants but would miss StructUpdate
- **Fix:** Added MirExpr::StructUpdate arms to both functions
- **Files modified:** crates/mesh-codegen/src/mir/mono.rs, crates/mesh-codegen/src/pattern/compile.rs
- **Verification:** cargo build succeeds with no warnings, all tests pass
- **Committed in:** 21da0ef9 (Task 2 commit)

**2. [Rule 3 - Blocking] Added StructUpdate to collect_free_vars in lower.rs**
- **Found during:** Task 2
- **Issue:** collect_free_vars in closure capture analysis must handle all MirExpr variants
- **Fix:** Added MirExpr::StructUpdate arm recursing into base and override expressions
- **Files modified:** crates/mesh-codegen/src/mir/lower.rs
- **Verification:** cargo build succeeds, closure tests pass
- **Committed in:** 21da0ef9 (Task 2 commit)

---

**Total deviations:** 2 auto-fixed (2 blocking)
**Impact on plan:** Both auto-fixes necessary for compilation. Added missing match arms in exhaustive MirExpr pattern matches. No scope creep.

## Issues Encountered
None - all tasks executed cleanly.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Struct update expressions are fully operational for use in ORM changesets (`%{changeset | changes: new_changes}`)
- Map literals continue to work with no disambiguation regressions
- Ready for 96-04 (Map.collect generic fix) and 96-05 (cross-module resolution)

## Self-Check: PASSED

All 10 modified files verified present. Both task commits (17cd2954, 21da0ef9) verified in git log.

---
*Phase: 96-compiler-additions*
*Completed: 2026-02-16*
