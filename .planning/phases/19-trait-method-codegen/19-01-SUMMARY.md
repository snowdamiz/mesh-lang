---
phase: 19-trait-method-codegen
plan: 01
subsystem: mir-lowering
tags: [mir, impl-def, name-mangling, self-parameter, trait-codegen]

# Dependency graph
requires:
  - phase: 18-trait-infrastructure
    provides: TraitRegistry threaded to MIR Lowerer
provides:
  - ImplDef pre-registration with mangled names in known_functions
  - ImplDef method lowering to MirFunctions with Trait__Method__Type names
  - Self parameter handled as concrete-typed first argument
  - extract_impl_names() shared helper for PATH extraction
affects: [19-02, 19-03, 19-04]

# Tech tracking
tech-stack:
  added: []
  patterns: [double-underscore-name-mangling, self-kw-detection, impl-method-lowering]

key-files:
  created: []
  modified:
    - crates/snow-codegen/src/mir/lower.rs
    - crates/snow-typeck/src/infer.rs

key-decisions:
  - "extract_impl_names() as free function reused by both pre-registration and lowering"
  - "Self parameter detected via SyntaxKind::SELF_KW, zipped with Ty::Fun param types"
  - "typeck stores impl method Ty::Fun in types map for lowerer consumption"

patterns-established:
  - "ImplDef lowering: extract_impl_names -> format Trait__Method__Type -> lower_impl_method"
  - "Self parameter: detect SELF_KW token, use 'self' as name, resolve type from Ty::Fun"

# Metrics
duration: 6min
completed: 2026-02-08
---

# Phase 19 Plan 01: Trait Method Codegen Foundation Summary

**ImplDef methods lowered to MirFunctions with Trait__Method__Type mangled names, self parameter as concrete struct type, and pre-registration in known_functions for direct call dispatch.**

## Performance

- **Duration:** 6 min
- **Started:** 2026-02-08T06:37:22Z
- **Completed:** 2026-02-08T06:43:55Z
- **Tasks:** 2
- **Files modified:** 2

## Accomplishments

### Task 1: Pre-register ImplDef methods with mangled names in known_functions

Added `extract_impl_names()` free function that extracts trait name and type name from ImplDef's PATH children (same pattern as `infer_impl_def` in infer.rs). Added `Item::ImplDef` arm to the pre-registration loop in `lower_source_file` that computes `Trait__Method__Type` mangled names for each method and inserts them into `known_functions`. This ensures that call sites emit `MirExpr::Call` (direct dispatch) rather than `MirExpr::ClosureCall` (indirect/closure dispatch).

### Task 2: Lower ImplDef methods to MirFunctions with mangled names and self parameter

Replaced the `Item::ImplDef` arm in `lower_item()` to compute mangled names and delegate to a new `lower_impl_method()` helper. The helper detects the `self` parameter via `SyntaxKind::SELF_KW` token (not name-based), names it "self", and zips it with the `Ty::Fun` param types from typeck for correct concrete type resolution. The method body is lowered identically to `lower_fn_def` (block or expr_body). Added `ImplDef` to the item import list.

Added `impl_method_produces_mangled_mir_function` test that creates an interface, struct, and impl, then asserts that a MirFunction named `Greetable__greet__Point` exists with self typed as `MirType::Struct("Point")` and return type `MirType::String`.

## Task Commits

Each task was committed atomically:

1. **Task 1: Pre-register ImplDef methods** - `a4ab8bf` (feat)
2. **Task 2: Lower ImplDef methods with mangled names** - `674bf2c` (feat)

## Files Created/Modified

- `crates/snow-codegen/src/mir/lower.rs` - Added extract_impl_names() helper, ImplDef pre-registration arm, lower_impl_method() helper, ImplDef arm in lower_item(), and unit test
- `crates/snow-typeck/src/infer.rs` - Added types.insert() for impl method function type in infer_impl_def

## Decisions Made

1. **extract_impl_names() as free function:** Rather than a method on Lowerer, this is a pure function taking `&ImplDef` since it needs no Lowerer state. Reused by both pre-registration and lowering.
2. **Self parameter via SELF_KW + Ty::Fun zip:** The self parameter is detected by token kind, named "self", but its TYPE comes from zipping with the Ty::Fun param types (where typeck stores the impl type as the first param). This avoids Pitfall 6 misalignment.
3. **Separate lower_impl_method from lower_fn_def:** Keeps concerns clean -- impl methods handle self differently and will diverge further in future phases.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] typeck did not store impl method Ty::Fun in types map**

- **Found during:** Task 2 (lower_impl_method verification)
- **Issue:** `infer_impl_def` in infer.rs constructed the method's `Ty::Fun` and inserted it into the type environment for call resolution, but did NOT insert it into the `types: FxHashMap<TextRange, Ty>` map. The MIR lowerer uses `self.get_ty(method.syntax().text_range())` to look up function types, which returned `None`, causing all parameter and return types to resolve as `MirType::Unit`.
- **Fix:** Added `types.insert(method.syntax().text_range(), fn_ty.clone())` in `infer_impl_def` right after the `fn_ty` is constructed and before the environment insert.
- **Files modified:** `crates/snow-typeck/src/infer.rs`
- **Verification:** All 227 typeck tests pass, all 86 codegen tests pass, test confirms `MirType::Struct("Point")` for self and `MirType::String` for return type.
- **Committed in:** 674bf2c (Task 2 commit)

**2. [Rule 1 - Bug] Test source used `type` keyword instead of `struct`**

- **Found during:** Task 2 (test writing)
- **Issue:** The plan specified `type Point do x :: Int end` but Snow uses the `type` keyword for sum type definitions, not struct definitions. The parser produced a `SumTypeDef` instead of a `StructDef`, and the impl block was not parsed as a separate item.
- **Fix:** Changed test source to use `struct Point do x :: Int end` which correctly produces a `StructDef` and allows the `impl` block to parse correctly as a separate `ImplDef` item.
- **Files modified:** `crates/snow-codegen/src/mir/lower.rs` (test only)
- **Verification:** Test passes with correct parsing: InterfaceDef + StructDef + ImplDef
- **Committed in:** 674bf2c (Task 2 commit)

---

**Total deviations:** 2 auto-fixed (1 blocking, 1 bug)
**Impact on plan:** Both auto-fixes necessary for correct operation. The typeck fix is essential for any impl method lowering. No scope creep.

## Issues Encountered

None beyond the deviations documented above.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness

Phase 19-02 (Call-Site Resolution) prerequisites are met:
- Mangled function names (`Trait__Method__Type`) are pre-registered in `known_functions`
- ImplDef methods produce `MirFunction`s with correct mangled names, self parameter types, and return types
- `extract_impl_names()` helper available for reuse
- The typeck types map now contains impl method function types for lowerer consumption
- All 86 codegen tests + 227 typeck tests pass

---
*Phase: 19-trait-method-codegen*
*Completed: 2026-02-08*

## Self-Check: PASSED
