---
phase: 20
plan: 03
subsystem: typeck-codegen-mir
tags: [debug, inspect, trait-dispatch, auto-derive, mir-generation]
depends_on:
  requires: [20-01, 20-02]
  provides: [debug-trait-registration, debug-inspect-mir-generation, primitive-debug-dispatch]
  affects: [20-04, 20-05]
tech_stack:
  added: []
  patterns: [auto-register-trait-impl-for-struct-def, synthetic-mir-function-generation, debug-inspect-field-iteration]
key_files:
  created: []
  modified:
    - crates/snow-typeck/src/builtins.rs
    - crates/snow-typeck/src/infer.rs
    - crates/snow-codegen/src/mir/lower.rs
decisions:
  - id: 20-03-01
    decision: "Debug impls auto-registered only for non-generic struct/sum types"
    rationale: "Generic types need monomorphized impls which require type parameter resolution; non-generic covers the common case"
  - id: 20-03-02
    decision: "Primitive Debug inspect redirects to same runtime functions as Display (Int/Float/Bool)"
    rationale: "For primitives, inspect and to_string produce identical output; Debug__inspect__String wraps in quotes"
  - id: 20-03-03
    decision: "Sum type Debug inspect returns variant name only (payload shown as '...' for variants with fields)"
    rationale: "Full payload inspection requires runtime tag-based field access; variant name is sufficient for initial Debug"
metrics:
  duration: 23min
  completed: 2026-02-08
---

# Phase 20 Plan 03: Debug Trait + Inspect Dispatch Summary

**One-liner:** Registered Debug trait with inspect(self)->String; auto-register Debug impls for all non-generic structs/sum types; generate synthetic Debug__inspect__TypeName MIR functions.

## What Was Done

### Task 1: Register Debug trait with primitive impls and auto-register for structs/sum types

**builtins.rs changes:**
- Registered Debug trait with `inspect(self) -> String` method signature in `register_compiler_known_traits()`
- Added Debug impls for all primitive types (Int, Float, String, Bool)
- Added `debug_trait_registered_for_primitives` test verifying has_impl and find_method_traits

**infer.rs changes:**
- Modified `register_struct_def` signature to accept `&mut TraitRegistry`
- After struct registration, auto-registers a `Debug` impl via `TraitImplDef` for non-generic structs
- Modified `register_sum_type_def` signature to accept `&mut TraitRegistry`
- After sum type registration, auto-registers a `Debug` impl for non-generic sum types
- Updated both call sites in `infer_item` to pass `trait_registry`

### Task 2: Generate Debug__inspect__TypeName MIR function bodies for structs and sum types

**lower.rs changes:**

1. **Struct inspect generation** (`generate_debug_inspect_struct`):
   - Builds a `MirFunction` named `Debug__inspect__StructName`
   - Takes `(self: StructType)` parameter, returns `String`
   - Body iterates fields: for each field, accesses `self.field_name`, converts to string via `wrap_to_string`, concatenates with `snow_string_concat`
   - Output format: `"StructName { field1: val1, field2: val2 }"`
   - Registers in `known_functions` for codegen resolution

2. **Sum type inspect generation** (`generate_debug_inspect_sum_type`):
   - Builds a `MirFunction` named `Debug__inspect__SumTypeName`
   - Body is a `MirExpr::Match` on the self parameter (tag-based dispatch)
   - Each arm matches a tag literal and returns the variant name string
   - Variants with fields shown as `"VariantName(...)"`
   - Registers in `known_functions`

3. **Primitive Debug dispatch** (call rewriting in `lower_call_expr`):
   - `Debug__inspect__Int` -> `snow_int_to_string`
   - `Debug__inspect__Float` -> `snow_float_to_string`
   - `Debug__inspect__Bool` -> `snow_bool_to_string`
   - `Debug__inspect__String` -> wraps value in `"` quotes via concat

4. **Tests added**:
   - `debug_inspect_struct_generates_mir_function`: verifies `Debug__inspect__Point` appears in MIR
   - `debug_inspect_sum_type_generates_mir_function`: verifies `Debug__inspect__Color` appears in MIR

## Task Commits

| Task | Name | Commit | Key Changes |
|------|------|--------|-------------|
| 1 | Register Debug trait with primitive impls and auto-register | f96788b | builtins.rs: +36 lines (Debug trait + impls); infer.rs: +33 lines (auto-register in struct/sum def) |
| 2 | Generate Debug__inspect__TypeName MIR function bodies | 5d4e24b | lower.rs: +226 lines (inspect generation helpers, primitive redirects, tests) |

## Verification Results

- `cargo test --workspace`: 1,131 tests pass, 0 failures (3 new tests added)
- `cargo build --workspace`: clean compilation
- Debug trait registered: `has_impl("Debug", &Ty::int())` returns true for all primitives
- `find_method_traits("inspect", &Ty::int())` returns `["Debug"]`
- Struct Debug auto-registration: non-generic structs get Debug impl in typeck
- Sum type Debug auto-registration: non-generic sum types get Debug impl in typeck
- MIR generation: `Debug__inspect__Point` function generated with correct params and return type
- MIR generation: `Debug__inspect__Color` function generated with match-based body

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Sum type test used wrong Snow syntax**

- **Found during:** Task 2 verification
- **Issue:** Test source used `type Color = | Red | Green | Blue end` which is not valid Snow syntax
- **Fix:** Changed to `type Color do Red Green Blue end` matching Snow's actual sum type syntax
- **Files modified:** crates/snow-codegen/src/mir/lower.rs (test only)
- **Commit:** 5d4e24b

## Decisions Made

| ID | Decision | Rationale |
|----|----------|-----------|
| 20-03-01 | Debug impls auto-registered only for non-generic struct/sum types | Generic types need monomorphized impls; non-generic covers the common case |
| 20-03-02 | Primitive Debug inspect reuses Display runtime functions (Int/Float/Bool) | Same output for primitives; String adds quote wrapping |
| 20-03-03 | Sum type inspect returns variant name only (no payload details) | Full payload inspection requires runtime tag-based field access; simplification for v1.3 |

## Next Phase Readiness

**Unblocked:** Debug trait fully registered with auto-derive for structs/sum types. MIR functions generated for all struct and sum type definitions.

**Ready for:**
- Eq trait for structs (20-04) -- same auto-register + MIR generation pattern established
- Ord trait for structs (20-04/20-05)
- End-to-end `inspect(my_struct)` once codegen handles the generated MIR functions

**Pattern established:** The auto-register + MIR generation pattern (register impl in typeck, generate function body in MIR lowering) is now proven and can be reused for Eq/Ord auto-derive.

## Self-Check: PASSED
