---
phase: "74"
plan: "02"
subsystem: "type-checker"
tags: ["associated-types", "trait-system", "type-inference", "validation"]
dependency-graph:
  requires: ["74-01 (parser support)"]
  provides: ["associated type storage in TraitDef/ImplDef", "Self.Item resolution in inference", "validation errors for missing/extra assoc types", "resolve_associated_type API on TraitRegistry"]
  affects: ["builtins.rs (field additions)", "infer.rs (inference wiring)", "diagnostics.rs (error rendering)", "mesh-lsp (error handling)"]
tech-stack:
  added: []
  patterns: ["AST token iteration for Self.X resolution", "type binding extraction from ASSOC_TYPE_BINDING nodes"]
key-files:
  created:
    - "crates/mesh-typeck/tests/assoc_types.rs"
  modified:
    - "crates/mesh-typeck/src/traits.rs"
    - "crates/mesh-typeck/src/error.rs"
    - "crates/mesh-typeck/src/diagnostics.rs"
    - "crates/mesh-typeck/src/infer.rs"
    - "crates/mesh-typeck/src/builtins.rs"
    - "crates/mesh-lsp/src/analysis.rs"
decisions:
  - "Self is uppercase IDENT (not SELF_KW) -- resolution uses IDENT text matching"
  - "Associated type bindings extracted by iterating tokens after EQ in ASSOC_TYPE_BINDING node"
  - "resolve_self_assoc_type filters whitespace trivia before pattern matching"
metrics:
  duration: "~20 minutes"
  completed: "2026-02-13"
---

# Phase 74 Plan 02: Type-Checker Core for Associated Types Summary

Associated type infrastructure in TraitDef/ImplDef with validation, Self.Item inference resolution, and 9 integration tests.

## What Changed

### Data Structures (traits.rs)
- Added `AssocTypeDef` struct with `name: String` field
- Extended `TraitDef` with `associated_types: Vec<AssocTypeDef>` field
- Extended `ImplDef` with `associated_types: FxHashMap<String, Ty>` field
- Added `resolve_associated_type(trait_name, assoc_name, impl_ty)` method to `TraitRegistry`
- Extended `freshen_type_params` with `freshen_type_params_with_names` variant for multi-character type parameter names
- Added associated type validation in `register_impl`: checks for missing and extra associated types

### Error Handling (error.rs, diagnostics.rs)
- `MissingAssocType { trait_name, assoc_name, impl_ty }` -- impl missing required assoc type (E0040)
- `ExtraAssocType { trait_name, assoc_name, impl_ty }` -- impl provides undeclared assoc type (E0041)
- `UnresolvedAssocType { assoc_name, span }` -- Self.Item used outside impl context (E0042)
- All three variants have Display impls and ariadne diagnostic renderings

### Inference Wiring (infer.rs)
- `infer_interface_def` now collects `type Item` declarations from InterfaceDef AST nodes
- `infer_impl_def` now collects `type Item = Int` bindings from ImplDef AST nodes
- `resolve_assoc_type_binding` helper extracts concrete type from ASSOC_TYPE_BINDING node tokens
- `resolve_self_assoc_type` resolves Self.X patterns in TYPE_ANNOTATION nodes (handles uppercase Self as IDENT, filters whitespace trivia)
- Method return types and parameter types in impl blocks try Self.Item resolution before standard type resolution

### Backward Compatibility
- All existing TraitDef/ImplDef constructions updated with empty `associated_types` fields
- 46+ construction sites updated across traits.rs, builtins.rs, infer.rs (both struct and sum type deriving)
- mesh-lsp analysis.rs updated to handle new error variants

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Self is IDENT not SELF_KW**
- **Found during:** Task 3 (test_self_item_return_type)
- **Issue:** `resolve_self_assoc_type` was looking for `SELF_KW` token, but uppercase `Self` is parsed as a regular `IDENT` token
- **Fix:** Changed pattern matching to check `IDENT` with text `"Self"` and added whitespace trivia filtering
- **Files modified:** crates/mesh-typeck/src/infer.rs
- **Commit:** 48c96514

**2. [Rule 3 - Blocking] mesh-lsp compilation failure**
- **Found during:** Task 3 (workspace test)
- **Issue:** mesh-lsp/src/analysis.rs had non-exhaustive match on TypeError variants after adding new variants
- **Fix:** Added MissingAssocType, ExtraAssocType, UnresolvedAssocType arms to the error-to-span extraction function
- **Files modified:** crates/mesh-lsp/src/analysis.rs
- **Commit:** 48c96514

## Decisions Made

1. **Self.Item token pattern:** Uppercase `Self` in Mesh is an IDENT token (the lexer only maps lowercase `self` to SELF_KW). Resolution uses IDENT text comparison, not keyword matching.
2. **Type binding extraction:** Extract concrete types from ASSOC_TYPE_BINDING nodes by iterating tokens after the EQ token and feeding them through the existing `parse_type_tokens`/`resolve_alias` pipeline.
3. **Trivia filtering:** `resolve_self_assoc_type` filters whitespace trivia tokens before pattern matching to handle `-> Self.Item` where whitespace separates `->` and `Self`.

## Verification

- 9 new integration tests (crates/mesh-typeck/tests/assoc_types.rs)
- 1630 workspace tests pass, 0 failures
- Full backward compatibility verified (all existing trait/impl tests unchanged)

## Self-Check: PASSED

- All 7 key files verified present on disk
- All 3 task commits verified in git history (983ba79c, 913281da, 48c96514)
- 1630 workspace tests pass, 0 failures
