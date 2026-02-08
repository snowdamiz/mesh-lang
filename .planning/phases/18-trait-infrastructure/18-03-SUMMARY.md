---
phase: 18
plan: 03
subsystem: typeck-codegen-plumbing
tags: [trait-registry, typeck-result, mir-lowerer, dispatch-unification]
dependency-graph:
  requires: [18-01]
  provides: [trait-registry-in-lowerer, unified-dispatch-path]
  affects: [19-01, 19-02]
tech-stack:
  added: []
  patterns: [registry-threading-through-result-type]
key-files:
  created: []
  modified:
    - crates/snow-typeck/src/lib.rs
    - crates/snow-typeck/src/infer.rs
    - crates/snow-codegen/src/mir/lower.rs
    - crates/snow-lsp/src/analysis.rs
    - crates/snow-typeck/src/traits.rs
key-decisions:
  - TraitRegistry re-exported from snow-typeck crate root alongside TypeRegistry
  - TraitRegistry stored by value in TypeckResult (owned, not borrowed)
  - Lowerer borrows TraitRegistry as &'a reference from TypeckResult
metrics:
  duration: 10min
  completed: 2026-02-08
---

# Phase 18 Plan 03: TraitRegistry Exposure Summary

TraitRegistry threaded from infer() through TypeckResult to MIR Lowerer, with unified dispatch test proving built-in and user-defined types share identical resolution path.

## Performance

- `cargo check --workspace`: compiles cleanly
- `cargo test -p snow-typeck`: 70 unit + 157 integration tests pass (227 total)
- `cargo test -p snow-codegen --lib`: 85 tests pass
- New test `unified_dispatch_builtin_and_user_types` validates same-path resolution

## Accomplishments

### Task 1: Thread TraitRegistry through TypeckResult to Lowerer

Added `pub use crate::traits::TraitRegistry` re-export to `snow-typeck/src/lib.rs`. Added `pub trait_registry: TraitRegistry` field to the `TypeckResult` struct with documentation. Updated `infer()` in `infer.rs` to include the existing `trait_registry` local variable in the `TypeckResult` construction (the variable was already created at the start of `infer()` and populated by `builtins::register_builtins()` but was discarded before this change). Added `trait_registry: &'a TraitRegistry` field to the `Lowerer` struct in `lower.rs` and populated it from `typeck.trait_registry` in the constructor. Updated the `snow_typeck` import in `lower.rs` to include `TraitRegistry`.

### Task 2: Unified Dispatch Path Test

Added `unified_dispatch_builtin_and_user_types` test in `traits.rs` that registers the `Add` trait, then creates impls for both `Int` (built-in type) and `MyStruct` (user-defined `Ty::Con`). The test exercises all three resolution methods -- `has_impl`, `find_impl`, and `resolve_trait_method` -- proving both types go through the identical TraitRegistry lookup code with zero special-case branching.

## Task Commits

| Task | Name | Commit | Key Files |
|------|------|--------|-----------|
| 1 | Thread TraitRegistry through TypeckResult to Lowerer | e362b52 | lib.rs, infer.rs, lower.rs, analysis.rs |
| 2 | Unified dispatch path test | (absorbed by cad88a3) | traits.rs |

## Files Modified

- `crates/snow-typeck/src/lib.rs` -- Added TraitRegistry re-export and field to TypeckResult
- `crates/snow-typeck/src/infer.rs` -- Added trait_registry to TypeckResult construction
- `crates/snow-codegen/src/mir/lower.rs` -- Added TraitRegistry import, field, and constructor init
- `crates/snow-lsp/src/analysis.rs` -- Fixed match exhaustiveness for new TypeError variants
- `crates/snow-typeck/src/traits.rs` -- Added unified_dispatch_builtin_and_user_types test

## Decisions Made

1. **TraitRegistry re-exported at crate root**: Follows the same pattern as TypeRegistry -- downstream crates (snow-codegen) import from `snow_typeck::TraitRegistry` directly.
2. **Owned TraitRegistry in TypeckResult**: The registry is moved (not cloned) from the infer() function into TypeckResult. The Lowerer then borrows it, avoiding any copying.
3. **Lowerer stores &'a TraitRegistry**: Matches the existing pattern where Lowerer borrows `&'a FxHashMap<TextRange, Ty>` and `&'a TypeRegistry` from TypeckResult.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Fixed snow-lsp match exhaustiveness for new TypeError variants**

- **Found during:** Task 1 verification (`cargo check --workspace`)
- **Issue:** Plan 18-02 (running in parallel) added `DuplicateImpl` and `AmbiguousMethod` variants to `TypeError` but did not update the `type_error_span` match in `snow-lsp/src/analysis.rs`, causing a compilation error.
- **Fix:** Added `TypeError::DuplicateImpl { .. } => None` and `TypeError::AmbiguousMethod { .. } => None` arms. Neither variant carries a `TextRange` span, so `None` is correct.
- **Files modified:** `crates/snow-lsp/src/analysis.rs`
- **Commit:** e362b52

**2. [Parallel execution artifact] Task 2 test absorbed by 18-02 commit**

- **Found during:** Task 2 commit
- **Issue:** Both 18-02 and 18-03 modified `traits.rs` concurrently. The 18-02 agent committed after 18-03 had already written the test to the working tree, so 18-02's commit (cad88a3) included the unified dispatch test.
- **Impact:** None -- the test exists, passes, and is committed. No separate Task 2 commit was needed.

## Issues Encountered

None beyond the parallel execution artifacts documented above.

## Next Phase Readiness

Phase 19 (Trait Method Codegen) prerequisites are met:
- TraitRegistry is accessible in the Lowerer via `self.trait_registry`
- All three resolution methods (`has_impl`, `find_impl`, `resolve_trait_method`) are available
- The unified dispatch model is proven: built-in and user-defined types share the same resolution path
- No changes needed to `lower_to_mir` public API or call sites

## Self-Check: PASSED
