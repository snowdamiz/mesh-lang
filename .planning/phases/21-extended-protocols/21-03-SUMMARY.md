---
phase: 21
plan: 03
subsystem: type-system
tags: [default-methods, interfaces, traits, mir-lowering, monomorphization]
depends_on:
  requires: ["21-02"]
  provides: ["Default method bodies in interfaces, has_default_body flag, default body MIR lowering"]
  affects: ["21-04"]
tech-stack:
  added: []
  patterns: ["Default method body in interface definition", "TextRange-based syntax node lookup for MIR lowering", "Per-concrete-type default body monomorphization"]
key-files:
  created: []
  modified:
    - crates/snow-parser/src/parser/items.rs
    - crates/snow-parser/src/ast/item.rs
    - crates/snow-parser/tests/parser_tests.rs
    - crates/snow-typeck/src/traits.rs
    - crates/snow-typeck/src/infer.rs
    - crates/snow-typeck/src/lib.rs
    - crates/snow-typeck/src/builtins.rs
    - crates/snow-codegen/src/mir/lower.rs
key-decisions:
  - decision: "Store default method body as TextRange (not SyntaxNode) in TypeckResult"
    reason: "Rowan SyntaxNode uses Rc internally and is not Send+Sync; the LSP server holds TypeckResult in async context requiring Send. TextRange is Copy/Send and the lowerer can look up the node from the parse tree."
  - decision: "Default body re-lowered per concrete type via TextRange lookup in parse tree"
    reason: "Monomorphization model requires the default body to be lowered with self bound to the concrete impl type. Looking up by TextRange from parse.syntax().descendants() is reliable since the parse tree is immutable."
  - decision: "Pre-register default method functions in known_functions during pre-registration phase"
    reason: "The lowerer needs to recognize default method mangled names as known functions for call dispatch resolution, same as user-provided impl methods."
duration: "15min"
completed: "2026-02-08"
---

# Phase 21 Plan 03: Default Method Implementations Summary

Optional do...end bodies in interface methods, enabling default implementations that compile without overrides and lower per concrete type.

## Accomplishments

1. **Parser**: Extended `parse_interface_method` to accept optional `do...end` bodies after the return type annotation. Methods without bodies remain signature-only.

2. **AST**: Added `body()` accessor to `InterfaceMethod`, returning `Option<Block>` using the same `child_node` pattern as `FnDef::body()`.

3. **Typeck**: Added `has_default_body: bool` field to `TraitMethodSig`. Updated all 18+ construction sites across builtins.rs, infer.rs, and test modules. Modified `register_impl()` to skip `MissingTraitMethod` errors when the trait method has a default body.

4. **TypeckResult**: Added `default_method_bodies: FxHashMap<(String, String), TextRange>` field to store default body locations keyed by (trait_name, method_name).

5. **MIR Lowering**: Implemented `lower_default_method()` that finds the InterfaceMethod AST node by TextRange, extracts the body Block, and lowers it as a monomorphized function with `self` bound to the concrete impl type. Also pre-registers default method functions in `known_functions`.

## Task Commits

| Task | Name | Commit | Key Changes |
|------|------|--------|-------------|
| 1 | Parser + AST support for optional method bodies | `63d66c2` | parse_interface_method DO_KW handling, InterfaceMethod::body(), 2 parser tests |
| 2 | Typeck has_default_body flag + MIR lowering | `8489ab6` | TraitMethodSig field, register_impl skip, TypeckResult field, lower_default_method, 3 codegen tests |

## Files Modified

- `crates/snow-parser/src/parser/items.rs` -- Optional do...end body parsing in interface methods
- `crates/snow-parser/src/ast/item.rs` -- body() accessor on InterfaceMethod
- `crates/snow-parser/tests/parser_tests.rs` -- Two new parser tests
- `crates/snow-typeck/src/traits.rs` -- has_default_body field, register_impl skip logic
- `crates/snow-typeck/src/infer.rs` -- Set has_default_body flag, store TextRange
- `crates/snow-typeck/src/lib.rs` -- default_method_bodies field on TypeckResult
- `crates/snow-typeck/src/builtins.rs` -- has_default_body: false on all 9 builtin traits
- `crates/snow-codegen/src/mir/lower.rs` -- lower_default_method, pre-registration, 3 tests

## Decisions Made

1. **TextRange over SyntaxNode**: Stored TextRange instead of SyntaxNode in TypeckResult because rowan's SyntaxNode is not Send (uses Rc), and the LSP server requires Send+Sync on TypeckResult. The lowerer reconstructs the AST node from the parse tree using the range.

2. **Per-concrete-type lowering**: Default bodies are re-lowered for each concrete type that omits the method, consistent with Snow's monomorphization model. The self parameter is bound to the concrete type, not a generic placeholder.

3. **Pre-registration of default methods**: Default method mangled names are pre-registered in known_functions during the first pass, ensuring call dispatch resolution works correctly.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] SyntaxNode is not Send+Sync for TypeckResult**
- **Found during:** Task 2
- **Issue:** Storing `SyntaxNode` in `TypeckResult` caused compile errors in snow-lsp because the LSP server holds TypeckResult in an async Send context, and rowan's SyntaxNode uses Rc (not Arc).
- **Fix:** Changed storage to `TextRange` (which is Copy/Send) and implemented TextRange-based lookup in the lowerer via `parse.syntax().descendants()`.
- **Files modified:** crates/snow-typeck/src/lib.rs, crates/snow-typeck/src/infer.rs, crates/snow-codegen/src/mir/lower.rs
- **Commit:** 8489ab6

## Issues Encountered

None beyond the Send+Sync deviation above.

## Next Phase Readiness

Plan 21-04 (where-clause constraints on interface type params) can proceed. The has_default_body flag infrastructure is complete and working. The TraitMethodSig struct now carries all metadata needed for constraint checking.

## Self-Check: PASSED
