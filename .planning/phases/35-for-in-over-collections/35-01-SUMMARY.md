---
phase: 35-for-in-over-collections
plan: 01
subsystem: compiler
tags: [for-in, iteration, list, map, set, mir, typeck, parser, codegen, runtime]

# Dependency graph
requires:
  - phase: 34-for-in-over-range
    provides: for-in range parsing, typeck, MIR ForInRange, codegen loop structure
  - phase: 08-collections
    provides: List/Map/Set runtime (snow-rt), collection intrinsics
provides:
  - runtime list builder API (snow_list_builder_new, snow_list_builder_push)
  - runtime indexed access for Map (snow_map_entry_key, snow_map_entry_value) and Set (snow_set_element_at)
  - parser support for {k, v} destructuring binding in for-in
  - typeck collection type detection (List<T>, Map<K,V>, Set<T>) with correct variable binding
  - MIR variants ForInList, ForInMap, ForInSet
  - MIR lowering dispatch based on iterable type
  - codegen intrinsic declarations for all new runtime functions
  - codegen placeholder arms for collection for-in variants
affects: [35-02-codegen, 35-03-tests]

# Tech tracking
tech-stack:
  added: []
  patterns: [collection-type-dispatch-in-lowering, indexed-iteration-model, list-builder-pattern, comprehension-return-semantics]

key-files:
  created: []
  modified:
    - crates/snow-rt/src/collections/list.rs
    - crates/snow-rt/src/collections/map.rs
    - crates/snow-rt/src/collections/set.rs
    - crates/snow-parser/src/syntax_kind.rs
    - crates/snow-parser/src/ast/expr.rs
    - crates/snow-parser/src/parser/expressions.rs
    - crates/snow-typeck/src/infer.rs
    - crates/snow-codegen/src/mir/mod.rs
    - crates/snow-codegen/src/mir/lower.rs
    - crates/snow-codegen/src/mir/mono.rs
    - crates/snow-codegen/src/pattern/compile.rs
    - crates/snow-codegen/src/codegen/expr.rs
    - crates/snow-codegen/src/codegen/intrinsics.rs

key-decisions:
  - "Indexed iteration model: all collections iterated via index counter (0..len), not Rust iterators, matching Snow's value-semantics and GC model"
  - "List builder pattern: pre-allocated capacity with in-place push for O(N) comprehension result building"
  - "Comprehension semantics: for-in returns List<body_ty> instead of Unit, enabling functional collection transforms"
  - "ForInRange ty changed from Unit to Ptr to match comprehension return semantics"

patterns-established:
  - "Collection type dispatch: lower_for_in_expr detects DotDot range vs List/Map/Set via typeck results and dispatches to variant-specific lowering"
  - "Destructure binding: parser emits DESTRUCTURE_BINDING node with NAME children for {k, v} map iteration"

# Metrics
duration: 12min
completed: 2026-02-09
---

# Phase 35 Plan 01: For-In Over Collections Foundation Summary

**Runtime list builder and indexed access APIs, parser destructuring, typeck collection detection, MIR variants and lowering dispatch for for-in over List/Map/Set**

## Performance

- **Duration:** 12 min
- **Started:** 2026-02-09T09:10:00Z
- **Completed:** 2026-02-09T09:22:06Z
- **Tasks:** 2
- **Files modified:** 13

## Accomplishments
- Runtime list builder with O(1) push and pre-allocated capacity, plus indexed access for Map entries and Set elements
- Parser support for `{k, v}` destructuring in for-in loops with DESTRUCTURE_BINDING syntax node
- Typeck rewritten to detect collection types (List, Map, Set) and bind loop variables to correct element/key/value types, with comprehension return type List<body_ty>
- MIR variants ForInList, ForInMap, ForInSet with full lowering dispatch based on iterable type
- Codegen infrastructure: intrinsic declarations for all 5 new runtime functions, placeholder codegen arms for plan 02

## Task Commits

Each task was committed atomically:

1. **Task 1: Add runtime functions** - `e37d8be` (feat)
2. **Task 2: Parser, typeck, MIR, and codegen infrastructure** - `f258773` (feat)

## Files Created/Modified
- `crates/snow-rt/src/collections/list.rs` - Added snow_list_builder_new and snow_list_builder_push
- `crates/snow-rt/src/collections/map.rs` - Added snow_map_entry_key and snow_map_entry_value
- `crates/snow-rt/src/collections/set.rs` - Added snow_set_element_at
- `crates/snow-parser/src/syntax_kind.rs` - Added DESTRUCTURE_BINDING variant
- `crates/snow-parser/src/ast/expr.rs` - Added DestructureBinding AST node, extended ForInExpr
- `crates/snow-parser/src/parser/expressions.rs` - Extended parse_for_in_expr for {k, v} syntax
- `crates/snow-typeck/src/infer.rs` - Rewrote infer_for_in for collection detection and comprehension semantics
- `crates/snow-codegen/src/mir/mod.rs` - Added ForInList, ForInMap, ForInSet variants
- `crates/snow-codegen/src/mir/lower.rs` - Added lower_for_in_list/map/set, extract_map_types, extract_set_elem_type
- `crates/snow-codegen/src/mir/mono.rs` - Extended collect_function_refs for new variants
- `crates/snow-codegen/src/pattern/compile.rs` - Extended compile_expr_patterns for new variants
- `crates/snow-codegen/src/codegen/expr.rs` - Placeholder codegen for collection for-in, updated ForInRange to return list
- `crates/snow-codegen/src/codegen/intrinsics.rs` - Registered 5 new runtime function intrinsics

## Decisions Made
- **Indexed iteration model:** All collection for-in loops use index-based iteration (counter from 0 to len), not Rust iterators. This matches Snow's value semantics and GC model where collections are heap-allocated opaque pointers.
- **List builder pattern:** Pre-allocate capacity then push in-place for O(N) result construction. The builder API (snow_list_builder_new/push) avoids creating intermediate copies on each append.
- **Comprehension semantics:** Changed for-in return type from Unit to List<body_ty>. This means `for x in list do f(x) end` now produces a new list, matching functional language conventions.
- **ForInRange type change:** Updated from MirType::Unit to MirType::Ptr to be consistent with comprehension semantics. Codegen updated to return empty list as placeholder.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed ForInRange codegen return type mismatch**
- **Found during:** Task 2 (codegen infrastructure)
- **Issue:** Changing ForInRange ty from Unit to Ptr caused LLVM verification failure: function returned struct {} (Unit) but signature expected ptr
- **Fix:** Updated codegen_for_in_range to call snow_list_new and return ptr instead of struct zero
- **Files modified:** crates/snow-codegen/src/codegen/expr.rs
- **Verification:** All 67 e2e tests pass including for_in_range_break and for_in_range_continue
- **Committed in:** f258773 (Task 2 commit)

---

**Total deviations:** 1 auto-fixed (1 bug)
**Impact on plan:** Necessary fix for type system consistency. The plan anticipated Ptr type for ForInRange but didn't account for the codegen needing to return a list instead of Unit.

## Issues Encountered
- `InferCtx::apply()` does not exist -- the correct method is `InferCtx::resolve()` for applying type substitutions. Fixed during implementation.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- All infrastructure in place for plan 02 (codegen) to implement real LLVM IR for ForInList, ForInMap, ForInSet
- Placeholder codegen returns empty list; plan 02 replaces with list builder loop pattern
- Runtime functions are tested and ready for codegen to call
- Intrinsic declarations already registered

## Self-Check: PASSED

All 14 files verified present. Both task commits (e37d8be, f258773) verified in git log. All workspace tests pass (0 failures).

---
*Phase: 35-for-in-over-collections*
*Completed: 2026-02-09*
