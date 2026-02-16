---
phase: 98-query-builder-repo
plan: 01
subsystem: database
tags: [query-builder, pipe-composition, atoms, opaque-ptr, immutable-data]

# Dependency graph
requires:
  - phase: 97-schema-metadata-sql-generation
    provides: "Orm.build_select/insert/update/delete, Schema metadata (__table__, __fields__)"
provides:
  - "Query module with 14 pipe-composable builder functions (from, where, where_op, where_in, where_null, where_not_null, select, order_by, limit, offset, join, group_by, having, fragment)"
  - "Query struct as opaque 13-slot 104-byte heap object with copy-on-write semantics"
  - "Atom type parameters for field/operator arguments in Query module"
  - "Composable scope pattern: regular functions taking/returning Query values"
affects: [98-02, 98-03, 99-changesets, 100-relationships]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Query builder: immutable copy-on-write Ptr objects via GC allocation"
    - "Atom parameters: Atom type in typeck, String at MIR level"
    - "Keyword-as-field-name: WHERE_KW accepted after dot in field access"

key-files:
  created:
    - "crates/mesh-rt/src/db/query.rs"
  modified:
    - "crates/mesh-typeck/src/infer.rs"
    - "crates/mesh-codegen/src/mir/lower.rs"
    - "crates/mesh-codegen/src/codegen/intrinsics.rs"
    - "crates/mesh-codegen/src/mir/types.rs"
    - "crates/mesh-rt/src/db/mod.rs"
    - "crates/mesh-rt/src/lib.rs"
    - "crates/mesh-repl/src/jit.rs"
    - "crates/mesh-parser/src/parser/expressions.rs"
    - "crates/mesh-parser/src/ast/expr.rs"
    - "crates/meshc/tests/e2e.rs"

key-decisions:
  - "98-01: Query type signatures use Atom type for field/operator parameters (not String) since atoms are distinct at typeck level"
  - "98-01: Ptr and Atom type constructors added to resolve_con (Ptr->MirType::Ptr, Atom->MirType::String)"
  - "98-01: Parser accepts WHERE_KW after dot for Query.where() field access"
  - "98-01: Schema pipe transform deferred to explicit form: Query.from(User.__table__()) instead of implicit User |> Query.where()"
  - "98-01: Query.select accepts List<String> (not Ptr) to match list literal type"

patterns-established:
  - "Query builder copy-on-write: clone 104 bytes, modify slots, return new pointer"
  - "Atom-to-SQL mapping: atom strings map to SQL operators/directions/join types at runtime"

# Metrics
duration: 14min
completed: 2026-02-16
---

# Phase 98 Plan 01: Query Builder Summary

**Pipe-composable Query struct with 14 builder functions (from, where, select, order_by, limit, etc.) registered across full compiler pipeline with Atom-typed field parameters and immutable copy-on-write semantics**

## Performance

- **Duration:** 14 min
- **Started:** 2026-02-16T18:56:42Z
- **Completed:** 2026-02-16T19:11:11Z
- **Tasks:** 2
- **Files modified:** 11

## Accomplishments
- Query module registered across typeck (STDLIB_MODULE_NAMES + 14 function types), MIR (STDLIB_MODULES + known_functions + map_builtin_name), intrinsics (14 LLVM declarations), JIT (14 symbol mappings), and runtime (14 extern C functions in query.rs)
- All 14 builder functions implemented with immutable copy-on-write semantics: each allocates new 104-byte Query, copies previous state, modifies relevant slots
- 7 e2e tests verify: basic from, pipe chain composition, where_op/null variants, all clause types, immutability, composable scopes, and schema pipe with __table__()
- 187 total e2e tests pass (180 existing + 7 new), zero regressions

## Task Commits

Each task was committed atomically:

1. **Task 1: Register Query module and implement runtime query builder functions** - `1f16ff26` (feat)
2. **Task 2: Add e2e tests for Query builder pipe composition and composable scopes** - `1e803fc3` (test)

## Files Created/Modified
- `crates/mesh-rt/src/db/query.rs` - Query struct runtime: 13-slot 104-byte opaque Ptr with all builder functions
- `crates/mesh-typeck/src/infer.rs` - Query module type signatures with Atom-typed field parameters
- `crates/mesh-codegen/src/mir/lower.rs` - Query known_functions entries and map_builtin_name mappings
- `crates/mesh-codegen/src/codegen/intrinsics.rs` - 14 LLVM intrinsic declarations for Query functions
- `crates/mesh-codegen/src/mir/types.rs` - Ptr and Atom type constructors in resolve_con
- `crates/mesh-rt/src/db/mod.rs` - Added pub mod query
- `crates/mesh-rt/src/lib.rs` - Re-export all mesh_query_* functions
- `crates/mesh-repl/src/jit.rs` - JIT symbol registrations for 14 Query functions
- `crates/mesh-parser/src/parser/expressions.rs` - Accept WHERE_KW after dot in field access
- `crates/mesh-parser/src/ast/expr.rs` - Include WHERE_KW in FieldAccess field() extraction
- `crates/meshc/tests/e2e.rs` - 7 new e2e tests for Query builder

## Decisions Made
- Used Atom type (not String) for field/operator parameters since atoms are typeck-distinct from strings (lowered to StringLit at MIR level)
- Added Ptr type constructor to resolve_con -> MirType::Ptr (was falling through to MirType::Struct("Ptr") causing LLVM errors)
- Added Atom type constructor to resolve_con -> MirType::String (atoms are strings at runtime)
- Parser extended to accept `where` keyword as field name after dot (enables Query.where syntax)
- Query.select typed as List<String> (not Ptr) to match list literal type resolution
- Schema-to-table pipe transform deferred: using explicit Query.from(User.__table__()) for now

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] `where` keyword not accepted as field name after dot**
- **Found during:** Task 2 (e2e tests)
- **Issue:** Parser treated `Query.where(...)` as keyword `where`, not a method name
- **Fix:** Added WHERE_KW to accepted tokens after DOT in expressions.rs and field() extraction in expr.rs
- **Files modified:** crates/mesh-parser/src/parser/expressions.rs, crates/mesh-parser/src/ast/expr.rs
- **Verification:** All Query.where tests pass
- **Committed in:** 1e803fc3 (Task 2 commit)

**2. [Rule 1 - Bug] Ptr type constructor resolved to MirType::Struct("Ptr") instead of MirType::Ptr**
- **Found during:** Task 2 (e2e tests)
- **Issue:** LLVM error "Cannot allocate unsized type %Ptr" because Ptr wasn't in resolve_con's known types
- **Fix:** Added "Ptr" to the opaque pointer match arm in resolve_con, added "Atom" -> MirType::String
- **Files modified:** crates/mesh-codegen/src/mir/types.rs
- **Verification:** All e2e tests pass, LLVM compilation succeeds
- **Committed in:** 1e803fc3 (Task 2 commit)

**3. [Rule 1 - Bug] Query function type signatures used String for atom parameters**
- **Found during:** Task 2 (e2e tests)
- **Issue:** Atoms (`:name`, `:asc`) have Atom type at typeck level, but Query functions expected String -- type mismatch
- **Fix:** Changed field/operator parameters from String to Atom type in Query module registration
- **Files modified:** crates/mesh-typeck/src/infer.rs
- **Verification:** All pipe chain tests pass with atom arguments
- **Committed in:** 1e803fc3 (Task 2 commit)

**4. [Rule 1 - Bug] Query.select expected Ptr but list literal is List<String>**
- **Found during:** Task 2 (e2e tests)
- **Issue:** `["id", "name"]` has type List<String>, not Ptr -- type mismatch in Query.select
- **Fix:** Changed Query.select parameter type from Ptr to List<String>
- **Files modified:** crates/mesh-typeck/src/infer.rs
- **Verification:** all_clauses test passes with list literal argument
- **Committed in:** 1e803fc3 (Task 2 commit)

---

**Total deviations:** 4 auto-fixed (3 bug fixes, 1 blocking)
**Impact on plan:** All auto-fixes necessary for correct type resolution and parser support. No scope creep.

## Issues Encountered
- Schema-to-table implicit pipe transform (`User |> Query.where(...)`) deferred as plan allows: "If the pipe schema-to-table transformation proves too complex for this plan, test 7 can use Query.from(User.__table__()) as fallback." Test 7 uses the explicit form.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Query builder foundation complete, ready for Repo reads (98-02) and writes (98-03)
- Query struct provides all slots needed by Repo to extract SQL components (source, where_clauses, where_params, etc.)
- Composable scopes pattern verified -- functions taking/returning Query work in pipe chains

---
*Phase: 98-query-builder-repo*
*Completed: 2026-02-16*
