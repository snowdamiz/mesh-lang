---
phase: 96-compiler-additions
plan: 05
subsystem: compiler
tags: [map-collect, string-keys, from_json, from_row, cross-module, trait-dispatch, codegen, mir-lowering]

# Dependency graph
requires:
  - phase: 96-04
    provides: deriving(Schema) and relationship declarations pattern for struct trait generation
provides:
  - Map.collect correctly handles string key propagation via pipe chain AST analysis
  - Cross-module from_json/from_row/to_json trait method resolution in MIR lowerer
  - BUILTIN_PREFIXES updated for FromRow__, FromJson__, ToJson__ trait functions
  - Deriving-generated trait impls exported in collect_exports
affects: [97-schema-metadata, 98-query-builder, 99-repo-pattern, orm-cross-module-structs]

# Tech tracking
tech-stack:
  added: [mesh_map_collect_string_keys runtime function]
  patterns: [pipe-chain-AST-analysis-for-type-inference, cross-module-trait-wrapper-generation]

key-files:
  created:
    - tests/e2e/collect_map_string_keys.mpl
  modified:
    - crates/mesh-rt/src/iter.rs
    - crates/mesh-rt/src/lib.rs
    - crates/mesh-codegen/src/codegen/intrinsics.rs
    - crates/mesh-repl/src/jit.rs
    - crates/mesh-codegen/src/mir/lower.rs
    - crates/mesh-typeck/src/lib.rs
    - crates/meshc/tests/e2e.rs

key-decisions:
  - "Pipe chain AST walk for string key detection instead of type-level inference (HM generalization severs type variable connections through Ptr bottleneck)"
  - "Separate mesh_map_collect_string_keys runtime function rather than auto-detecting key type at runtime"
  - "Generate cross-module __json_decode__ wrappers before lower_source_file so they are available during field access resolution"
  - "Register ToJson/FromRow in known_functions for imported structs rather than regenerating full trait method bodies"

patterns-established:
  - "Pipe chain backward walk: ty_has_string_map_keys + pipe_chain_has_string_keys for compile-time type analysis through pipe chains"
  - "Cross-module trait wrapper pre-generation: generate thin wrappers in lower_to_mir before source file lowering for imported types"

# Metrics
duration: ~90min
completed: 2026-02-16
---

# Phase 96 Plan 05: Compiler Bug Fixes Summary

**Map.collect string key propagation via pipe chain AST analysis, and cross-module from_json/from_row/to_json trait resolution via pre-generated wrappers and known_functions registration**

## Performance

- **Duration:** ~90 min
- **Started:** 2026-02-16T08:35:00Z
- **Completed:** 2026-02-16T10:06:00Z
- **Tasks:** 2
- **Files modified:** 8

## Accomplishments
- Map.collect on string-keyed tuple iterators now produces maps with correct string key_type (1), enabling string key lookups after collect
- Cross-module from_json, to_json, and from_row trait methods resolve correctly when struct is defined in a different module
- Pipe chain AST backward walk technique enables compile-time type analysis through the iterator Ptr type erasure boundary
- Both namespace import (`import Models`) and selective import (`from Models import User`) work for cross-module trait methods

## Task Commits

Each task was committed atomically:

1. **Task 1: Fix Map.collect string key type propagation (COMP-07)** - `df9a7d45` (feat)
2. **Task 2: Fix cross-module from_row and from_json resolution (COMP-08)** - `5af70a5f` (feat)

## Files Created/Modified

- `crates/mesh-rt/src/iter.rs` - Added `mesh_map_collect_string_keys` runtime function that creates map with key_type=1
- `crates/mesh-rt/src/lib.rs` - Exported new runtime function
- `crates/mesh-codegen/src/codegen/intrinsics.rs` - Registered `mesh_map_collect_string_keys` as LLVM intrinsic
- `crates/mesh-repl/src/jit.rs` - Registered JIT symbol for REPL
- `crates/mesh-codegen/src/mir/lower.rs` - (1) Added `ty_has_string_map_keys` + `pipe_chain_has_string_keys` for pipe chain type analysis, modified `lower_pipe_expr` to swap collect functions; (2) Added `FromRow__`, `FromJson__`, `ToJson__` to BUILTIN_PREFIXES; (3) Added cross-module wrapper pre-generation in `lower_to_mir`
- `crates/mesh-typeck/src/lib.rs` - Extended `collect_exports` to export deriving-generated trait impls (Json -> ToJson + FromJson, Row -> FromRow)
- `crates/meshc/tests/e2e.rs` - Added 3 e2e tests: `e2e_collect_map_string_keys`, `e2e_cross_module_from_json`, `e2e_cross_module_from_json_selective_import`
- `tests/e2e/collect_map_string_keys.mpl` - Test fixture for string key map collect roundtrip

## Decisions Made

1. **Pipe chain AST walk over type-level inference:** Hindley-Milner generalization at `let` bindings severs type variable connections -- `Map.to_list(m) |> Iter.from() |> Map.collect()` erases string key type through the Ptr bottleneck. Instead of fixing HM inference (major architectural change), we walk the pipe chain AST backwards to find the source collection's type and check if it's a Map<String,V> or List<(String,V)>.

2. **Separate string-key collect function:** Rather than having `mesh_map_collect` auto-detect key types at runtime (fragile pointer heuristics), we added a distinct `mesh_map_collect_string_keys` function and let the compiler choose which to call based on compile-time analysis.

3. **Pre-generation timing for cross-module wrappers:** The `__json_decode__` wrappers must be generated BEFORE `lower_source_file` runs, not after, because `lower_field_access` checks `known_functions` during source file lowering.

4. **Register vs regenerate for cross-module traits:** For ToJson and FromRow, we only register the function signatures in `known_functions` (the actual function bodies are generated by the defining module). For FromJson, we generate the thin `__json_decode__` wrapper since it's a convenience layer that calls the trait function.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 2 - Missing Critical] Export deriving-generated trait impls in collect_exports**
- **Found during:** Task 2 (cross-module from_json resolution)
- **Issue:** `collect_exports` in `mesh-typeck/src/lib.rs` only exported trait impls from explicit `ImplDef` AST nodes, NOT from `deriving(Json)` or `deriving(Row)` clauses. Imported modules couldn't see the FromJson/ToJson impls.
- **Fix:** Added scanning of StructDef/SumTypeDef items for `deriving_traits()`, mapping "Json" to ["ToJson", "FromJson"] and "Row" to ["FromRow"], with deduplication.
- **Files modified:** `crates/mesh-typeck/src/lib.rs`
- **Verification:** Cross-module from_json test passes
- **Committed in:** 5af70a5f (Task 2 commit)

**2. [Rule 2 - Missing Critical] Register ToJson in known_functions for imported structs**
- **Found during:** Task 2 (cross-module from_json resolution)
- **Issue:** `Json.encode(u)` where `u` is an imported struct produced "null" because `ToJson__to_json__User` was not in `known_functions`, so the encode dispatch at line 6008 failed to chain the trait method.
- **Fix:** Added `ToJson__to_json__` registration in the cross-module wrapper pre-generation block.
- **Files modified:** `crates/mesh-codegen/src/mir/lower.rs`
- **Verification:** Cross-module test produces correct JSON output
- **Committed in:** 5af70a5f (Task 2 commit)

**3. [Rule 3 - Blocking] Generate wrappers before lower_source_file, not after**
- **Found during:** Task 2 (cross-module from_json resolution)
- **Issue:** Initial placement of cross-module wrapper generation was after `lower_source_file`, but `lower_field_access` (which runs during source file lowering) needs the wrappers in `known_functions`.
- **Fix:** Moved the wrapper generation block before `lower_source_file`.
- **Files modified:** `crates/mesh-codegen/src/mir/lower.rs`
- **Verification:** `User.from_json(json)` resolves to `__json_decode__User` correctly
- **Committed in:** 5af70a5f (Task 2 commit)

---

**Total deviations:** 3 auto-fixed (2 missing critical, 1 blocking)
**Impact on plan:** All auto-fixes were necessary for correct cross-module trait resolution. No scope creep -- these were gaps in the existing cross-module infrastructure that became visible when testing the specific from_json/to_json use case.

## Issues Encountered

1. **HM type inference Ptr bottleneck:** The most significant challenge was that Hindley-Milner inference with `generalize()` at let-bindings creates universal quantifiers that sever type variable connections. Multiple approaches were tried (checking typeck type at field access range, parent node types, pipe expression return types) before settling on pipe chain AST backward walk.

2. **JSON key ordering non-determinism:** The cross-module from_json test initially asserted exact JSON string output, but field ordering in JSON objects depends on hash map iteration order. Fixed by accepting both orderings in the assertion.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness
- Phase 96 (Compiler Additions) is now complete with all 5 plans executed
- Map operations work correctly with string keys (required for ORM key-value handling)
- Cross-module trait method dispatch works for from_json, to_json, from_row (required for ORM model hydration across module boundaries)
- Ready for Phase 97 (Schema Metadata) which builds on deriving(Schema) from 96-04 and the cross-module fixes from 96-05

## Self-Check: PASSED

All 8 files verified present. Both task commits (df9a7d45, 5af70a5f) verified in git history. 169 e2e tests pass, no regressions (2 pre-existing HTTP test failures unrelated to changes).

---
*Phase: 96-compiler-additions*
*Completed: 2026-02-16*
