---
phase: 24-trait-system-generics
plan: 02
subsystem: typeck, codegen
tags: [generics, deriving, monomorphization, trait-dispatch, parametric-impl]

# Dependency graph
requires:
  - phase: 24-01
    provides: "Recursive nested collection Display callbacks, synthetic MIR wrapper generation"
  - phase: 22-auto-derive-stretch
    provides: "Deriving infrastructure for structs and sum types"
provides:
  - "Generic struct deriving support -- Box<T> deriving(Display, Eq) works for any T"
  - "Parametric trait impl registration via Ty::App in trait registry"
  - "Monomorphized trait function generation at struct literal lowering sites"
  - "GenericDerive error (E0029) removed from compiler"
affects: [25-tooling-stretch-goals]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Lazy monomorphization: generate trait functions at instantiation sites, not definition sites"
    - "Parametric impl registration: Ty::App(Con(Name), [Con(T)]) in TraitRegistry"
    - "known_functions fallback for monomorphized type trait dispatch"
    - "substitute_type_params for generic param -> concrete type substitution"
    - "_with_display_name variants for human-readable Display/Debug output"

key-files:
  created:
    - "tests/e2e/generic_deriving.snow"
  modified:
    - "crates/snow-typeck/src/infer.rs"
    - "crates/snow-typeck/src/error.rs"
    - "crates/snow-typeck/src/diagnostics.rs"
    - "crates/snow-lsp/src/analysis.rs"
    - "crates/snow-codegen/src/mir/lower.rs"
    - "crates/snowc/tests/e2e.rs"

key-decisions:
  - "Parametric Ty::App impl registration instead of per-instantiation Ty::Con impls"
  - "Lazy monomorphization at struct literal sites via ensure_monomorphized_struct_trait_fns"
  - "known_functions fallback for Display/Eq/Ord dispatch on mangled monomorphized types"
  - "display_name separation: Box(42) not Box_Int(42) for human-readable output"

patterns-established:
  - "Lazy monomorphization: detect generic struct at StructLit lowering, generate MIR functions on demand"
  - "substitute_type_params: reusable generic param substitution for type parameterization"
  - "known_functions fallback: when trait registry has parametric impl but dispatch needs mangled name"

# Metrics
duration: 18min
completed: 2026-02-08
---

# Phase 24 Plan 02: Generic Type Deriving Summary

**Parametric trait impl registration and lazy monomorphized trait function generation enabling deriving(Display, Eq) on generic structs like Box<T>**

## Performance

- **Duration:** 18 min
- **Started:** 2026-02-08T19:25:34Z
- **Completed:** 2026-02-08T19:43:50Z
- **Tasks:** 3
- **Files modified:** 7

## Accomplishments
- Removed GenericDerive error (E0029) from entire codebase (typeck, diagnostics, LSP)
- Enabled parametric trait impl registration: generic structs register Ty::App impls in trait registry
- Implemented lazy monomorphized trait function generation at struct literal lowering sites
- Added known_functions fallback for Display/Eq/Ord dispatch on monomorphized generic types
- E2e test proves Box<Int> Display("Box(42)"), Box<String> Display("Box(hello)"), and Box<Int> Eq work

## Task Commits

Each task was committed atomically:

1. **Task 1: Remove GenericDerive error and register parametric trait impls** - `c06efb2` (feat)
2. **Task 2: Generate monomorphized trait functions for generic structs** - `1898240` (feat)
3. **Task 3: Add e2e tests for generic type deriving** - `8fff984` (test)

## Files Created/Modified
- `crates/snow-typeck/src/error.rs` - Removed GenericDerive variant from TypeError enum
- `crates/snow-typeck/src/diagnostics.rs` - Removed GenericDerive diagnostic arm and E0029 code
- `crates/snow-typeck/src/infer.rs` - Parametric Ty::App impl registration for generic structs/sum types
- `crates/snow-lsp/src/analysis.rs` - Removed GenericDerive match arm
- `crates/snow-codegen/src/mir/lower.rs` - Monomorphized trait fn generation, known_functions fallback, display_name
- `crates/snowc/tests/e2e.rs` - Added e2e_generic_deriving test
- `tests/e2e/generic_deriving.snow` - E2e fixture for Box<T> deriving(Display, Eq)

## Decisions Made
- **Parametric Ty::App registration:** Register generic struct impls as Ty::App(Con("Box"), [Con("T")]) rather than deferring registration. TraitRegistry structural matching via freshen_type_params handles concrete instantiation lookup automatically.
- **Lazy monomorphization at struct literal sites:** Generate trait functions (Display, Eq, Debug, etc.) when a generic struct is instantiated as a concrete type, not at definition time. This avoids generating functions for unknown generic types and ensures correct field type substitution.
- **known_functions fallback:** When trait registry lookup fails for mangled monomorphized types (e.g., Ty::Con("Box_Int") vs parametric Ty::App), fall back to checking known_functions for the generated MIR function. Applied to wrap_to_string, binary op dispatch, and trait method call rewriting.
- **display_name separation:** Monomorphized Display/Debug output uses the base name ("Box(42)") not the mangled name ("Box_Int(42)") for human-readable output.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Removed GenericDerive from LSP analysis.rs**
- **Found during:** Task 1 (GenericDerive removal)
- **Issue:** snow-lsp/src/analysis.rs also had a GenericDerive match arm that would cause compilation failure
- **Fix:** Removed the GenericDerive match arm from the span extraction function
- **Files modified:** crates/snow-lsp/src/analysis.rs
- **Verification:** cargo build -p snow-lsp succeeds
- **Committed in:** c06efb2 (Task 1 commit)

**2. [Rule 1 - Bug] Fixed display_name for monomorphized types**
- **Found during:** Task 3 (e2e test implementation)
- **Issue:** generate_display_struct used the mangled name (Box_Int) for the Display string, producing "Box_Int(42)" instead of "Box(42)"
- **Fix:** Added _with_display_name variants for generate_display_struct and generate_debug_inspect_struct, passing base name for display and mangled name for function naming
- **Files modified:** crates/snow-codegen/src/mir/lower.rs
- **Verification:** e2e_generic_deriving test passes with expected "Box(42)" output
- **Committed in:** 8fff984 (Task 3 commit)

**3. [Rule 1 - Bug] Fixed test fixture keyword: struct not type**
- **Found during:** Task 3 (e2e test implementation)
- **Issue:** Plan specified `type Box<T> do ... end` but Snow uses `struct` keyword for struct definitions, not `type` (which is for sum types)
- **Fix:** Used correct `struct Box<T> do ... end` syntax in fixture
- **Files modified:** tests/e2e/generic_deriving.snow
- **Verification:** Parser accepts the fixture, test passes
- **Committed in:** 8fff984 (Task 3 commit)

---

**Total deviations:** 3 auto-fixed (2 bugs, 1 blocking)
**Impact on plan:** All auto-fixes necessary for correctness. No scope creep.

## Issues Encountered
None - implementation followed the planned approach with minor corrections noted in deviations.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Phase 24 (Trait System Generics) complete: both TGEN-01 and TGEN-02 delivered
- Generic struct deriving works for Display, Eq, Debug, Ord, Hash
- 1,203 tests passing (1 new, 0 regressions)
- Remaining v1.4 item: TSND-01 (Phase 25 - Tooling Stretch Goals)

## Self-Check: PASSED

---
*Phase: 24-trait-system-generics*
*Completed: 2026-02-08*
