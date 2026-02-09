# Phase 40: Visibility Enforcement - Research

**Researched:** 2026-02-09
**Domain:** Compiler visibility/access control for Snow's module system
**Confidence:** HIGH

## Summary

Phase 40 adds visibility enforcement to Snow's module system. Currently, `collect_exports` in `snow-typeck/src/lib.rs` exports ALL top-level items from every module -- there is no filtering by `pub`. The parser already fully supports the `pub` keyword (`PUB_KW` token, `VISIBILITY` AST node, `parse_optional_visibility()` on all item types), so no parser work is needed. The AST already provides `.visibility()` methods on `FnDef`, `StructDef`, `SumTypeDef`, `InterfaceDef`, and `ModuleDef`.

The implementation is primarily a filtering change in `collect_exports` plus a new `TypeError` variant for "private item accessed from another module" with a helpful "add `pub`" suggestion. The existing `ImportNameNotFound` error already shows available names, but a new, more specific error variant should be added to distinguish "name exists but is private" from "name does not exist at all" (VIS-03 requires a `pub` suggestion).

**Primary recommendation:** Filter exports in `collect_exports` by checking `item.visibility().is_some()`, add a new `PrivateItem` TypeError variant, and pass both public + private item names to the type checker so it can distinguish "private" from "nonexistent" during import resolution.

## Standard Stack

This phase is entirely within the Snow compiler's existing Rust codebase. No external libraries needed.

### Core
| Crate | Role | Key Files |
|-------|------|-----------|
| `snow-parser` | Already parses `pub` keyword and builds VISIBILITY AST nodes | `src/ast/item.rs`, `src/parser/items.rs` |
| `snow-typeck` | Export collection and import resolution | `src/lib.rs` (collect_exports), `src/infer.rs` (import handling), `src/error.rs`, `src/diagnostics.rs` |
| `snowc` | Build pipeline that calls collect_exports | `src/main.rs` (build_import_context) |

### No New Dependencies
No new crates or libraries are needed. All changes are in existing compiler crates.

## Architecture Patterns

### Where Visibility is Checked (Data Flow)

```
[Module A source]
       |
       v
   snow_parser::parse() -- produces AST with VISIBILITY nodes
       |
       v
   snow_typeck::check_with_imports() -- type-checks module A
       |
       v
   snow_typeck::collect_exports(parse, typeck) -- FILTER HERE by pub
       |
       v
   ExportedSymbols { functions, struct_defs, sum_type_defs, ... }
       |
       v
   build_import_context() in snowc -- builds ImportContext from ExportedSymbols
       |
       v
   snow_typeck::check_with_imports(module_B_parse, &import_ctx)
       |
       v  (import resolution in infer_item for ImportDecl/FromImportDecl)
   mod_exports.functions.get(&name) -- only sees pub items
       |
       v  (if name not found but exists as private -> PrivateItem error)
   TypeError::PrivateItem { ... } with "add `pub`" suggestion
```

### Pattern 1: Two-Tier Export Collection

**What:** `collect_exports` returns two sets: public exports (go into `ExportedSymbols`) and private names (used for error messages). Alternatively, `ExportedSymbols` is filtered to pub-only, and private names are passed separately.

**When to use:** When the type checker needs to distinguish "name exists but is private" from "name does not exist" to produce the correct error message (VIS-03).

**Recommended approach:** Add a `private_names: HashSet<String>` field to `ExportedSymbols` (or a parallel structure). During import resolution in `infer_item`, when a name is not found in the public exports, check if it exists in the private names set to produce a `PrivateItem` error instead of `ImportNameNotFound`.

### Pattern 2: Visibility Check in collect_exports

**What:** The `collect_exports` function in `snow-typeck/src/lib.rs` (lines 181-278) currently iterates over all items and exports everything. Add a visibility check: only export items where `item.visibility().is_some()`.

**Example (current code, line 194-205):**
```rust
// CURRENT: exports ALL functions
Item::FnDef(fn_def) => {
    if let Some(name) = fn_def.name().and_then(|n| n.text()) {
        let range = fn_def.syntax().text_range();
        if let Some(ty) = typeck.types.get(&range) {
            exports.functions.insert(name, Scheme::mono(ty.clone()));
        }
    }
}
```

**Example (Phase 40: filter by pub):**
```rust
// PHASE 40: only export pub functions
Item::FnDef(fn_def) => {
    if let Some(name) = fn_def.name().and_then(|n| n.text()) {
        let range = fn_def.syntax().text_range();
        if let Some(ty) = typeck.types.get(&range) {
            if fn_def.visibility().is_some() {
                exports.functions.insert(name, Scheme::mono(ty.clone()));
            } else {
                exports.private_names.insert(name);
            }
        }
    }
}
```

### Pattern 3: New TypeError Variant for Private Access

**What:** A new error variant that produces the message: "`name` is private in module `Module`; add `pub` to make it accessible".

```rust
// In error.rs
TypeError::PrivateItem {
    module_name: String,
    name: String,
    span: TextRange,
}
```

**Why distinct from ImportNameNotFound:** VIS-03 requires a specific "add `pub`" suggestion. `ImportNameNotFound` shows available names. `PrivateItem` should say the item exists but is not public.

### Pattern 4: Entry Module Exception

**What:** The entry module (`main.snow`) does NOT need `pub` on anything. It is never imported. Its `main()` function is located by convention (entry point), not by module import.

**Why important:** Single-file programs and the entry module must continue to work without `pub`. Only items in non-entry modules that are consumed by other modules need `pub`.

### Anti-Patterns to Avoid

- **Checking visibility at codegen/MIR level:** Visibility is a source-level concept. It must be checked during type-checking / export collection, NOT during MIR lowering or codegen. The MIR should never see private items from other modules.

- **Breaking single-file programs:** Single-file programs have no module imports. `collect_exports` is only called when building multi-file projects. Ensure the single-file `check()` path is not affected.

- **Per-field visibility:** VIS-04 explicitly says no per-field visibility in v1.8. Do NOT add `pub` checks on struct fields. If a struct is `pub`, all its fields are accessible.

- **Per-variant visibility:** VIS-05 explicitly says all variants of a `pub type` (sum type) are accessible. Do NOT add visibility checks on individual variants.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Visibility AST support | New parser nodes | Existing `VISIBILITY` node, `FnDef::visibility()`, etc. | Already fully implemented in parser |
| Fuzzy name matching | Custom edit distance | Existing `find_closest_name` / `levenshtein_distance` in `diagnostics.rs` | Already proven, used for `ImportNameNotFound` |
| Error rendering | Custom diagnostic format | Existing `render_diagnostic` with ariadne | Follow established pattern from `ImportNameNotFound` / `ImportModuleNotFound` |
| Export collection | New function | Modify existing `collect_exports` in `src/lib.rs` | It has the comment "Phase 40 adds pub filtering" |

**Key insight:** The parser and AST already fully support `pub`. The only work is (1) filtering in `collect_exports`, (2) tracking private names, (3) a new TypeError, and (4) diagnostic rendering.

## Common Pitfalls

### Pitfall 1: Breaking Existing Cross-Module Tests
**What goes wrong:** All existing Phase 39 e2e tests (e2e_cross_module_qualified_function_call, e2e_cross_module_selective_import, etc.) export items WITHOUT `pub`. Adding pub filtering breaks them all.
**Why it happens:** Tests were written before visibility enforcement existed.
**How to avoid:** Update all existing cross-module test fixtures to add `pub` to exported items BEFORE or alongside enabling the filtering.
**Warning signs:** Any Phase 39 e2e test failure after the change.

### Pitfall 2: Impl Blocks and Trait Visibility
**What goes wrong:** An `impl Printable for MyStruct` might not have a `pub` keyword but needs to be exported for the trait system to work cross-module.
**Why it happens:** Trait impls and trait defs are exported via `exports.trait_defs` and `exports.trait_impls`, which follow different paths in `collect_exports`.
**How to avoid:** Trait defs and impls should remain globally visible (XMOD-05 from Phase 39). Only check visibility for functions, structs, sum types, and interfaces in `collect_exports`. Impl blocks themselves should always be exported when the trait or type they implement is visible.
**Warning signs:** Cross-module trait dispatch failures.

### Pitfall 3: ImportNameNotFound vs PrivateItem Conflation
**What goes wrong:** When a user tries to import a private item, they get "name not found" instead of "name is private, add `pub`".
**Why it happens:** If private names are not tracked, the import resolver can't distinguish between "doesn't exist" and "exists but private".
**How to avoid:** `collect_exports` must track private names separately. The import resolution code in `infer_item` (around lines 1498-1543 in infer.rs) must check private names before emitting `ImportNameNotFound`.
**Warning signs:** Error message says "not found" for a name that clearly exists in the module source.

### Pitfall 4: Qualified Access (Module.func) Not Filtered
**What goes wrong:** `import Math` followed by `Math.private_fn()` still works because the qualified module entries in `ctx.qualified_modules` include private items.
**Why it happens:** `build_import_context` populates `ModuleExports` from `ExportedSymbols`, and `ImportDecl` handling copies these into `ctx.qualified_modules`. If `ExportedSymbols.functions` is already filtered to pub-only, this works automatically.
**How to avoid:** Ensure filtering happens in `collect_exports` (the source of truth), not downstream. All downstream consumers (`build_import_context`, `infer_item` ImportDecl/FromImportDecl handling) will automatically see only pub items.
**Warning signs:** Private functions callable via qualified syntax.

### Pitfall 5: Struct Constructor Visibility
**What goes wrong:** A struct is not `pub` but its constructor name leaks into the environment.
**Why it happens:** In `collect_exports`, struct defs are copied from `type_registry.struct_defs`, which doesn't track visibility.
**How to avoid:** Filter struct exports by checking the AST `StructDef::visibility()` during `collect_exports`, same as for functions.
**Warning signs:** Constructing a private struct from another module.

### Pitfall 6: Sum Type Variant Constructor Leakage
**What goes wrong:** A sum type is not `pub` but its variant constructors (e.g., `Circle`, `Rectangle`) are accessible from another module.
**Why it happens:** Sum type variants are registered in the environment as constructors during type checking. If the sum type is exported (via `ExportedSymbols.sum_type_defs`), its constructors become available.
**How to avoid:** Filter `sum_type_defs` in `collect_exports` by checking `SumTypeDef::visibility()`. If the sum type is not `pub`, its variants should not be exported.
**Warning signs:** Pattern matching on private sum type variants from another module.

## Code Examples

### Checking Visibility on FnDef (already in AST)
```rust
// Source: crates/snow-parser/src/ast/item.rs, line 91
impl FnDef {
    pub fn visibility(&self) -> Option<Visibility> {
        child_node(&self.syntax)
    }
}
```

### Current collect_exports (to be modified)
```rust
// Source: crates/snow-typeck/src/lib.rs, lines 181-278
// Comment on line 177: "Currently exports ALL top-level definitions (Phase 40 adds pub filtering)."
pub fn collect_exports(
    parse: &snow_parser::Parse,
    typeck: &TypeckResult,
) -> ExportedSymbols {
    // ... iterates all items, exports everything
}
```

### Existing Import Resolution Points (where PrivateItem checks go)
```rust
// Source: crates/snow-typeck/src/infer.rs, lines 1498-1543
// FromImportDecl handling: checks mod_exports.functions, struct_defs, sum_type_defs
// Falls through to ImportNameNotFound error if none match
// Phase 40: add check against private_names BEFORE ImportNameNotFound
```

### ExportedSymbols Structure (to be extended)
```rust
// Source: crates/snow-typeck/src/lib.rs, lines 90-102
pub struct ExportedSymbols {
    pub functions: FxHashMap<String, Scheme>,
    pub struct_defs: FxHashMap<String, StructDefInfo>,
    pub sum_type_defs: FxHashMap<String, SumTypeDefInfo>,
    pub trait_defs: Vec<TraitDef>,
    pub trait_impls: Vec<TraitImplDef>,
    // Phase 40: add private_names: FxHashSet<String>
}
```

### Existing Error Pattern to Follow
```rust
// Source: crates/snow-typeck/src/error.rs, lines 268-275
TypeError::ImportNameNotFound {
    module_name: String,
    name: String,
    span: TextRange,
    available: Vec<String>,
}
```

### Existing Diagnostic Pattern to Follow
```rust
// Source: crates/snow-typeck/src/diagnostics.rs, lines 1460-1479
TypeError::ImportNameNotFound { module_name, name, span, available } => {
    let msg = "name not found in module";
    // ... builds ariadne report with "not exported" label
    // Phase 40 PrivateItem follows same pattern but with "add `pub`" help
}
```

## State of the Art

| Old Approach (Phase 39) | New Approach (Phase 40) | Impact |
|--------------------------|-------------------------|--------|
| `collect_exports` exports ALL items | `collect_exports` exports only `pub` items | Private-by-default |
| `ImportNameNotFound` for all missing names | `PrivateItem` for private names, `ImportNameNotFound` for truly missing | Better error messages |
| No visibility enforcement | Full enforcement on fn, struct, sum type, interface | VIS-01 through VIS-05 |

## Requirements Mapping

| Requirement | Implementation Point | How |
|-------------|---------------------|-----|
| VIS-01: All items private by default | `collect_exports` | Only export items with `visibility().is_some()` |
| VIS-02: `pub` makes item visible | `collect_exports` | Check `item.visibility().is_some()` to include in exports |
| VIS-03: Error with `pub` suggestion | `infer_item` import resolution | New `PrivateItem` TypeError with "add `pub`" fix suggestion |
| VIS-04: All fields of pub struct accessible | No new code needed | Already the case -- struct fields have no per-field visibility |
| VIS-05: All variants of pub type accessible | No new code needed | Already the case -- variant constructors are bundled with sum type |

## Implementation Strategy

### Recommended Task Breakdown

**Task 1: Extend ExportedSymbols + Filter collect_exports**
- Add `private_names: rustc_hash::FxHashSet<String>` to `ExportedSymbols`
- In `collect_exports`, check `visibility().is_some()` for FnDef, StructDef, SumTypeDef, InterfaceDef
- Items with visibility go into exports; items without go into `private_names`
- Trait defs and impls remain unconditionally exported (XMOD-05)
- Update existing Phase 39 e2e test fixtures to add `pub` to exported items

**Task 2: New TypeError + Diagnostic + Import Resolution Update**
- Add `PrivateItem` variant to `TypeError` enum
- Add error code (e.g., `E0035`)
- Add `Display` impl
- Add diagnostic rendering with "add `pub` to make it accessible" help text
- Modify import resolution in `infer_item` (both `ImportDecl` and `FromImportDecl` handling):
  - After failing to find name in public exports, check `private_names` (passed via `ModuleExports`)
  - If found in private names, emit `PrivateItem` instead of `ImportNameNotFound`
- Add `private_names` field to `ModuleExports` structure
- Update `build_import_context` in `snowc/src/main.rs` to pass private names through

**Task 3: E2E Tests for Visibility Enforcement**
- Test: function without `pub` cannot be called from another module (compile error)
- Test: adding `pub` makes function importable
- Test: private struct not importable, pub struct fully accessible (fields)
- Test: private sum type not importable, pub sum type has all variants accessible
- Test: error message suggests adding `pub`
- Test: single-file programs unaffected (no `pub` needed)
- Test: entry module (`main.snow`) unaffected
- Test: qualified access (`Module.private_fn()`) blocked
- Test: selective import (`from Module import private_fn`) blocked

### Execution Order
Task 1 must come first (it establishes the filtering). Task 2 depends on Task 1 (needs private_names). Task 3 can partially overlap with Task 2 but needs both for full verification.

## Open Questions

1. **Should `main()` in the entry module be implicitly public?**
   - What we know: `main()` is found by convention (entry point), not by import. The `entry_function` field in MIR is set directly from the entry module's compilation.
   - What's unclear: Whether the entry module's `collect_exports` is even called in a meaningful way.
   - Recommendation: The entry module's exports are used by no one (nothing imports `Main`), so this is a non-issue. The entry module can have all private items and still work fine.

2. **Should interface definitions require `pub` to be usable cross-module?**
   - What we know: Trait defs are currently exported globally (XMOD-05). The requirements say interfaces should be governed by `pub`.
   - What's unclear: Whether breaking the "globally visible" trait behavior causes issues.
   - Recommendation: YES, filter interface defs by `pub`. But keep trait impls unconditionally exported (if the trait and type are visible, their impls should work). This matches Rust's behavior where traits must be `use`d to access their methods.

3. **How to handle `ModuleExports.private_names` for qualified access?**
   - What we know: `ImportDecl` handling in `infer_item` (line 1459) registers functions in `ctx.qualified_modules`. This only sees what's in `mod_exports.functions`.
   - What's unclear: Whether qualified access (`Module.private_fn()`) should produce a `PrivateItem` error or a generic "no such field" error.
   - Recommendation: Add private name tracking to `ModuleExports` and check it during qualified access resolution (in `infer_field_access`). This is a stretch goal; the minimal implementation is to simply not include private items in `ModuleExports.functions`, which will produce an "unbound variable" or "no such field" error. A `PrivateItem` error for qualified access is nice-to-have.

## Sources

### Primary (HIGH confidence)
- **snow-parser/src/ast/item.rs** - Verified `visibility()` methods on FnDef (line 91), StructDef (line 263), SumTypeDef (line 505), InterfaceDef (line 427), ModuleDef (line 204)
- **snow-parser/src/parser/items.rs** - Verified `parse_optional_visibility()` called for fn (line 37), struct (line 122), sum type (line 250), interface (line 488), module (line 689), type alias (line 1073)
- **snow-typeck/src/lib.rs** - Verified `collect_exports` with comment "Phase 40 adds pub filtering" (line 177), `ExportedSymbols` struct (line 90)
- **snow-typeck/src/infer.rs** - Verified import resolution for ImportDecl (line 1451) and FromImportDecl (line 1491)
- **snow-typeck/src/error.rs** - Verified existing `ImportNameNotFound` variant (line 268) and `ImportModuleNotFound` (line 262)
- **snow-typeck/src/diagnostics.rs** - Verified diagnostic rendering for import errors (lines 1439-1479), error codes up to E0034
- **snowc/src/main.rs** - Verified `build_import_context` (line 383) and build pipeline
- **crates/snowc/tests/e2e.rs** - Verified existing cross-module tests (lines 1642-1866) that need `pub` added

### Secondary (MEDIUM confidence)
- Phase 39 research and plans in `.planning/phases/39-cross-module-type-checking/` - Confirmed prior decisions about module architecture

## Metadata

**Confidence breakdown:**
- Architecture: HIGH - Directly read all relevant source files, traced data flow from parser through typeck to codegen
- Implementation points: HIGH - Identified exact lines of code that need changes
- Pitfalls: HIGH - Based on actual code analysis, not hypotheticals
- Test strategy: HIGH - Based on existing e2e test patterns (compile_multifile_and_run, compile_multifile_expect_error)

**Research date:** 2026-02-09
**Valid until:** 2026-03-09 (stable compiler internals, unlikely to change)
