---
phase: 099-changesets
plan: 01
subsystem: database
tags: [changeset, validation, orm, pipeline, runtime]

# Dependency graph
requires:
  - phase: 098-query-builder-repo
    provides: Repo module, Query module, ORM SQL generation, Map/List runtime collections
provides:
  - Changeset.cast and cast_with_types for param filtering and type coercion
  - 5 validators (required, length, format, inclusion, number) with pipe-chain composition
  - Field accessors (valid, errors, changes, get_change, get_error)
  - Changeset module registered across full compiler pipeline (typeck, MIR, LLVM, JIT)
affects: [99-02-PLAN, repo-changeset-integration]

# Tech tracking
tech-stack:
  added: []
  patterns: [opaque-slot-struct, clone-check-error-return validators, Ptr-for-opaque-returns]

key-files:
  created:
    - crates/mesh-rt/src/db/changeset.rs
  modified:
    - crates/mesh-rt/src/db/mod.rs
    - crates/mesh-rt/src/lib.rs
    - crates/mesh-typeck/src/infer.rs
    - crates/mesh-codegen/src/mir/lower.rs
    - crates/mesh-codegen/src/codegen/intrinsics.rs
    - crates/mesh-repl/src/jit.rs
    - crates/mesh-parser/src/parser/expressions.rs
    - crates/mesh-parser/src/ast/expr.rs
    - crates/meshc/tests/e2e.rs

key-decisions:
  - "8-slot/64-byte GC-allocated changeset struct matching existing Query slot pattern"
  - "Clone-check-error-return validator pattern: each validator clones changeset so all run without short-circuiting"
  - "First-error-per-field wins: subsequent validators skip fields that already have errors"
  - "Type signatures use concrete types (Map<String,String>, List<Atom>) for user-facing params, Ptr for opaque changeset returns"
  - "Added CAST_KW to parser dot-field list since cast is a service handler keyword that must also work as a method name"

patterns-established:
  - "Opaque slot struct: 8-slot pointer layout for GC-managed compound objects (mirrors Query's 13-slot pattern)"
  - "Validator clone pattern: clone_changeset + check + set errors + return new"
  - "Concrete-typed stdlib signatures: use Map/List/Atom types in typeck for user-facing params to avoid Ptr unification failures"

# Metrics
duration: 17min
completed: 2026-02-16
---

# Phase 99 Plan 01: Changeset Module Summary

**Opaque changeset struct with cast/filter, 5 pipe-chainable validators, and field accessors registered across the full Mesh compiler pipeline**

## Performance

- **Duration:** 17 min
- **Started:** 2026-02-16T20:17:36Z
- **Completed:** 2026-02-16T20:34:40Z
- **Tasks:** 2
- **Files modified:** 10

## Accomplishments
- Implemented changeset.rs with 8-slot opaque struct: data, changes, errors, valid, field_types, table, primary_key, action
- Built 12 runtime functions: cast, cast_with_types, 5 validators (required, length, format, inclusion, number), 5 accessors (valid, errors, changes, get_change, get_error)
- Registered Changeset module across all 5 compiler layers: typeck, MIR lowerer, LLVM intrinsics, JIT symbols, and stdlib module lists
- Added 10 e2e tests covering all validators, pipe-chain composition, passing validation, and field accessors -- all 207 e2e tests pass

## Task Commits

Each task was committed atomically:

1. **Task 1: Implement Changeset runtime and register in compiler pipeline** - `f1f68380` (feat)
2. **Task 2: Add e2e tests for Changeset pipeline** - `0684cfee` (test)

**Plan metadata:** TBD (docs: complete plan)

## Files Created/Modified
- `crates/mesh-rt/src/db/changeset.rs` - 12 runtime functions: cast, cast_with_types, 5 validators, 5 accessors
- `crates/mesh-rt/src/db/mod.rs` - Added `pub mod changeset`
- `crates/mesh-rt/src/lib.rs` - Re-exported all 12 mesh_changeset_* functions
- `crates/mesh-typeck/src/infer.rs` - Changeset module in stdlib_modules, STDLIB_MODULE_NAMES
- `crates/mesh-codegen/src/mir/lower.rs` - known_functions, map_builtin_name, STDLIB_MODULES
- `crates/mesh-codegen/src/codegen/intrinsics.rs` - 12 LLVM intrinsic declarations
- `crates/mesh-repl/src/jit.rs` - 12 JIT symbol registrations
- `crates/mesh-parser/src/parser/expressions.rs` - Added CAST_KW to dot-field parser
- `crates/mesh-parser/src/ast/expr.rs` - Added CAST_KW to field() token filter
- `crates/meshc/tests/e2e.rs` - 10 new e2e tests for Changeset module

## Decisions Made
- Used 8-slot/64-byte layout matching the existing Query slot pattern for GC-managed opaque structs
- Each validator clones the changeset so all validators run without short-circuiting, enabling pipe-chain composition
- First error per field wins (subsequent validators skip fields with existing errors) to avoid confusing duplicate messages
- Used concrete types (Map<String,String>, List<Atom>, List<String>) for user-facing parameters in typeck signatures; Ptr only for opaque changeset return types
- SQL type coercion in cast_with_types handles TEXT, BIGINT, DOUBLE PRECISION, BOOLEAN with unknown types passing through

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] CAST_KW not allowed as dot-field identifier in parser**
- **Found during:** Task 2 (e2e test execution)
- **Issue:** `cast` is lexed as CAST_KW (service handler keyword). The parser's dot-field handler only accepted IDENT plus select other keywords (self, monitor, spawn, link, send, where). `Changeset.cast(...)` failed with "expected IDENT".
- **Fix:** Added `!p.eat(SyntaxKind::CAST_KW)` to the dot-field chain in expressions.rs and CAST_KW to the field() token filter in ast/expr.rs
- **Files modified:** crates/mesh-parser/src/parser/expressions.rs, crates/mesh-parser/src/ast/expr.rs
- **Verification:** `Changeset.cast(...)` parses correctly, all 207 e2e tests pass
- **Committed in:** 0684cfee (Task 2 commit)

**2. [Rule 1 - Bug] Ptr type signatures caused unification failures with Map/List literals**
- **Found during:** Task 2 (e2e test execution)
- **Issue:** Original Changeset typeck signatures used `Ptr` for all parameters. When users pass `%{"name" => "Alice"}` (Map<String,String>) or `[:name, :email]` (List<Atom>), the type checker cannot unify these concrete types with `Ptr`. Unlike List<String> which happens to work for reasons found during investigation (e.g. Query.select uses List<String> explicitly), Map<String,String> has no Ptr escape hatch.
- **Fix:** Changed cast/cast_with_types params from Ptr to Map<String,String>/List<Atom>/List<String>. Changed errors/changes return types to Map<String,String>. Changed get_change/get_error return types to String. Kept Ptr only for opaque changeset input/output.
- **Files modified:** crates/mesh-typeck/src/infer.rs
- **Verification:** All 10 changeset e2e tests pass with concrete type literals
- **Committed in:** 0684cfee (Task 2 commit)

---

**Total deviations:** 2 auto-fixed (1 blocking, 1 bug)
**Impact on plan:** Both fixes were essential for the feature to function. The parser fix ensures `Changeset.cast` can be parsed. The type signature fix ensures user code with map/list literals type-checks correctly. No scope creep.

## Issues Encountered
- The `if fn_call(args) do` pattern in Mesh parses `fn_call(args) do ... end` as a trailing closure call, not as `if condition do body end`. Tests bind the result to a variable first (`let is_valid = Changeset.valid(cs)`) and use `if is_valid do` instead. This is a pre-existing parser design choice, not a bug.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Changeset module is fully functional and registered across the compiler pipeline
- Ready for Plan 02 (Repo.insert_changeset / Repo.update_changeset integration)
- The changeset `changes` map can be passed directly to existing Repo.insert/update functions

## Self-Check: PASSED

All 11 key files verified present. Both task commits (f1f68380, 0684cfee) verified in git log.

---
*Phase: 099-changesets*
*Completed: 2026-02-16*
