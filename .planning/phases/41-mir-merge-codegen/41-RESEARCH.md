# Phase 41: MIR Merge & Codegen - Research

**Researched:** 2026-02-09
**Domain:** Compiler MIR-level name mangling, cross-module monomorphization, and MIR merge correctness for multi-module codegen
**Confidence:** HIGH

## Summary

Phase 41 addresses two concrete problems in the Snow compiler's multi-module compilation pipeline: (1) private functions with identical names in different modules collide during MIR merge (XMOD-07), and (2) generic functions defined in one module and called with concrete types in another module must be monomorphized correctly in the merged MIR (XMOD-06). Both problems are well-scoped and affect existing code in `snow-codegen/src/lib.rs` (the `merge_mir_modules` function) and `snow-codegen/src/mir/lower.rs` (the `lower_to_mir` function and its `Lowerer` struct).

The current `merge_mir_modules` function (lines 247-297 of `lib.rs`) uses a `seen_functions: HashSet<String>` to deduplicate functions by name. When two modules define a private function named `helper`, only the first one survives the merge -- the second is silently dropped. This causes incorrect codegen: the second module's calls to its own `helper` will dispatch to the first module's version. The fix is to prefix private (non-pub) function names with a module-qualified prefix during MIR lowering, before the merge. Since trait method mangling already uses `Trait__Method__Type` naming and the `main` function is already renamed to `snow_main`, adding module prefixes to private functions is the natural extension.

For cross-module generics (XMOD-06), the current pipeline already works by design for most cases. The type checker resolves all generic call sites to concrete types during inference (`check_with_imports`). When MIR lowering runs per-module via `lower_to_mir_raw`, each module's lowerer independently resolves concrete types at call sites. The `ensure_monomorphized_struct_trait_fns` method handles generic struct trait functions at struct literal sites. The key risk is when module A defines `pub fn identity<T>(x :: T) -> T = x` and module B calls `identity(42)` -- the lowerer in module B must generate the call using the correct function name that module A's lowerer produced. Since both modules share the same TypeckResult-derived function scheme information (via ImportContext), and the lowerer resolves types from the typeck map, the concrete function names should align. However, this needs verification via E2E tests. If the generic function's MIR is only emitted by module A's lowerer but module B calls a monomorphized variant (e.g., `identity_Int`), there is a potential gap where the monomorphized version does not exist in the merged MIR.

**Primary recommendation:** Add module-qualified prefixes to private function names during MIR lowering (e.g., `Utils::helper` becomes `Utils__helper` in MIR). The lowerer needs the module name, which must be threaded from the build pipeline. For generics, add E2E tests first to validate current behavior, then fix any gaps.

## Standard Stack

### Core
| Crate | Role | Key Files |
|-------|------|-----------|
| `snow-codegen` | MIR lowering, monomorphization, MIR merge, LLVM codegen | `src/lib.rs`, `src/mir/lower.rs`, `src/mir/mono.rs`, `src/mir/types.rs` |
| `snow-typeck` | TypeckResult with `qualified_modules`, `imported_functions` | `src/lib.rs` (TypeckResult, ImportContext, ExportedSymbols) |
| `snowc` | Build pipeline: per-module lowering, merge, codegen | `src/main.rs` (build function, lines 337-351) |
| `snow-common` | ModuleGraph, ModuleId, module naming | `src/module_graph.rs` |

### Supporting
| Crate | Role | When to Use |
|-------|------|-------------|
| `tempfile` (dev-dep) | Creating multi-file test project directories | All E2E tests |
| `inkwell` 0.8.0 | LLVM bindings for codegen | Already used, no changes needed |

### No New Dependencies
All changes are in existing compiler crates. No new libraries needed.

## Architecture Patterns

### Current Build Pipeline (Lines 337-371 of `snowc/src/main.rs`)

```
for each module in compilation_order:
    lower_to_mir_raw(parse, typeck)  -->  MirModule (no monomorphization)
    push to mir_modules vec

merge_mir_modules(mir_modules, entry_idx)
    --> dedup functions/structs/sum_types by name
    --> monomorphize (reachability pass from entry point)

compile_mir_to_binary(merged_mir, ...)
```

### Pattern 1: Module-Qualified Private Name Mangling

**What:** During MIR lowering (`lower_to_mir`), prefix private (non-pub) function names with the module name to prevent collisions. Public functions keep their unqualified names (they are already unique by the type checker's import resolution semantics).

**When to use:** Always, for all non-pub functions, non-pub struct definitions, and non-pub sum type definitions.

**Naming scheme:** `{ModuleName}__{function_name}` using double-underscore separator (consistent with existing trait mangling `Trait__Method__Type`).

**Example:**
```
Module "Utils" with private function "helper":
  MIR function name: "Utils__helper"

Module "Math" with private function "helper":
  MIR function name: "Math__helper"

Module "Main" with pub function "add" (exported):
  MIR function name: "add"  (unchanged -- pub functions are unique by import rules)
```

**Why this works:** The type checker's import resolution already ensures that pub function names are unique within a given scope. Two modules cannot both export a pub function `add` to the same importer -- the importer would have a name conflict at the type-checking level. Only private functions can share names across modules, and those are never referenced cross-module.

**Implementation location:** Thread the module name into `lower_to_mir_raw` and into the `Lowerer` struct. In `lower_fn_def` and `lower_multi_clause_fn`, if the function is not `pub` and not `main`, prefix with `{module_name}__`.

### Pattern 2: Closure Name Prefixing

**What:** Lifted closure function names (`__closure_0`, `__closure_1`, etc.) must also be module-qualified to avoid collisions. The current `closure_counter` is per-Lowerer (per-module), so two modules will both generate `__closure_0`.

**Naming scheme:** `{ModuleName}__closure_{n}`

**Implementation location:** Modify the closure naming in `Lowerer` to include the module name prefix.

### Pattern 3: Propagating Module Name to Lowerer

**What:** The `Lowerer` struct needs to know the module name to apply prefixes. Thread it from `snowc/src/main.rs` through `lower_to_mir_raw`.

**Implementation:**
```rust
// In snow-codegen/src/lib.rs
pub fn lower_to_mir_raw(
    parse: &snow_parser::Parse,
    typeck: &snow_typeck::TypeckResult,
    module_name: &str,        // NEW parameter
    is_pub_fn: &HashSet<String>,  // NEW: set of pub function names
) -> Result<mir::MirModule, String> {
    let module = lower_to_mir(parse, typeck, module_name, is_pub_fn)?;
    Ok(module)
}

// In Lowerer struct
struct Lowerer<'a> {
    module_name: String,      // NEW field
    pub_functions: HashSet<String>,  // NEW: functions that should NOT be prefixed
    // ... existing fields
}
```

**In `snowc/src/main.rs`:** The module name is available from `project.graph.get(id).name` (e.g., "Utils", "Math.Vector", "Main").

### Pattern 4: MIR Merge Deduplication Strategy

**What:** The current `merge_mir_modules` deduplicates by exact name match. With module-qualified private names, private functions will naturally have unique names. Builtin functions (e.g., `Ord__compare__Int`) and pub functions will still be deduplicated correctly since they share the same unqualified name across modules.

**Current behavior (correct for builtins):** Each module's lowerer generates builtin trait functions like `Ord__compare__Int`, `Ord__compare__Float`, `Ord__compare__String`. The merge's `seen_functions` HashSet correctly deduplicates these since they have identical names and implementations.

**Current behavior (broken for private functions):** Module A has `helper`, module B has `helper`. Merge keeps only module A's version. Module B's calls to `helper` dispatch to module A's version. With module-qualified names, this becomes `A__helper` and `B__helper` -- no collision.

### Pattern 5: Cross-Module Generic Function Monomorphization

**What:** When module A defines `pub fn identity<T>(x :: T) -> T = x` and module B calls `identity(42)`, the type checker resolves this at module B's call site to concrete type `Int`. Module B's lowerer generates a call to `identity` with `MirType::Int` arguments. Module A's lowerer generates the generic function body. Since the current monomorphization pass is a reachability-only pass (not a true specialization pass), the generic function body in MIR already uses the concrete types from the typeck resolution.

**Key insight from code analysis:** The Snow compiler does NOT perform true monomorphization (creating specialized copies like `identity_Int`). Instead, the type checker resolves all generic call sites to concrete types during inference, and the MIR lowerer emits concrete types directly. The `monomorphize` pass in `mir/mono.rs` is purely a dead-code elimination pass (reachability from entry point). This means cross-module generics should work correctly as long as:
1. Module A's lowerer emits the function body with concrete parameter types matching what module B calls.
2. Both modules agree on the function's name.

**Potential issue:** When module A defines `identity<T>`, its lowerer sees `Ty::Fun([Ty::Var(T)], Ty::Var(T))` for the function type. The lowerer resolves `Ty::Var` to `MirType::Unit` as a fallback (line 26-30 of `types.rs`). This means the function may be emitted with `MirType::Unit` parameters rather than the concrete types of module B's call site. Module B's call, however, will have `MirType::Int` arguments. This type mismatch at the LLVM level would cause a codegen failure.

**Resolution for this issue:** This is a well-known gap in the current design. The prior decision "post-merge monomorphization" indicates the intent is to handle this after merge. The current approach works for non-generic cross-module functions (they have concrete types), and for generic functions called within the same module (the lowerer has the concrete types from the call site). For cross-module generics, the type checker in module B resolves the call to concrete types, but module A's lowerer only sees the generic signature. The solution is one of:

1. **Emit the generic function body once with generic/placeholder types, then specialize during monomorphization** -- this requires a true monomorphization pass, not just reachability.
2. **Have module B's lowerer also emit the function body using its concrete types** -- this means the calling module emits the specialized version. The merge deduplicates, keeping the first (concrete) version.
3. **Extend the lowerer to accept cross-module function bodies** -- the lowerer for module B would lower imported generic function bodies using B's concrete type information.

Given the prior decision of "post-merge monomorphization," option (1) is closest, but that is a significant change to `mir/mono.rs`. However, looking more carefully at how the type checker works: when module B calls `identity(42)`, the type checker instantiates T=Int and records the call site type as `Ty::Fun([Int], Int)`. The lowerer for module B generates `MirExpr::Call { func: Var("identity", FnPtr([Int], Int)), args: [IntLit(42)], ty: Int }`. Module A's lowerer generates a function `identity` with params `[(x, Unit)]` (because T is unresolved in A's context). This is the type mismatch.

**Recommended approach:** For this phase, the simplest correct approach is to ensure that the merged MIR contains correctly-typed function definitions for all cross-module generic calls. Since the type checker in each module resolves generics to concrete types at call sites, and the call site information is correct, the gap is only in the function definition. The fix: when module B imports and calls `identity<T>` as `identity(42)`, the lowerer should also emit the function body using the concrete types. This can be done by having the lowerer re-lower imported generic function bodies when it encounters a call to them with concrete types. Alternatively, the simpler approach for v1.8: document that cross-module generics with type parameters in the function signature require the caller to also have the function body available (which is already the case since the typeck result includes the full type information).

**Simplest path forward:** Test with E2E tests first. If the current pipeline already handles generic cross-module calls correctly (because the type checker resolves types before lowering), no changes needed. If it fails, add a post-merge fixup that re-types generic functions based on their call sites.

### Anti-Patterns to Avoid
- **Separate LLVM module compilation with linking:** Prior decision explicitly chose single LLVM module (merge MIR). Do not attempt separate `.o` files per module.
- **Mangling all function names:** Only private functions need module prefixes. Pub functions must keep their unqualified names for cross-module calls to resolve.
- **Adding module prefixes at codegen time:** Names must be fixed during MIR lowering, before merge, so that cross-module call references match their definitions.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Module name from file path | Custom path parsing | `discovery::path_to_module_name` + `ModuleGraph::get(id).name` | Already built in Phase 37/38 |
| Determining pub visibility | Parsing source in lowerer | Check against `ExportedSymbols.functions` keys or pass pub function set from driver | Visibility is already computed during type checking |
| Function deduplication | Custom merge logic | Existing `HashSet<String>` in `merge_mir_modules` | Already works correctly once names are unique |
| Dead code elimination | Custom DCE pass | Existing `monomorphize` reachability pass in `mono.rs` | Already proven, handles service dispatch tables |

**Key insight:** The module-qualified naming is a MIR-level concern. The type checker, parser, and LLVM codegen do not need changes. Only `lower.rs` (name generation) and `lib.rs` (API signature for `lower_to_mir_raw`) need modification.

## Common Pitfalls

### Pitfall 1: Forgetting to Prefix Closure Names
**What goes wrong:** Two modules each generate `__closure_0`, `__closure_1`. After merge, only the first module's closures survive. The second module's `MakeClosure { fn_name: "__closure_0" }` references the wrong function.
**Why it happens:** The `closure_counter` in `Lowerer` starts at 0 for every module.
**How to avoid:** Prefix closure names with the module name: `{ModuleName}__closure_{n}`.
**Warning signs:** Tests with closures in imported modules produce wrong results or segfault.

### Pitfall 2: Prefixing Pub Functions
**What goes wrong:** Module A defines `pub fn add(...)`. Module B calls `add(2, 3)`. If both are prefixed, module B's call to `add` won't find module A's `A__add`.
**Why it happens:** Applying module prefix to ALL functions instead of only private ones.
**How to avoid:** Only prefix non-pub functions. Pub functions keep their unqualified names. The caller already references the function by its unqualified name.
**Warning signs:** Cross-module function calls fail with "function not found" during codegen.

### Pitfall 3: Prefixing Main Entry Function
**What goes wrong:** `main` is renamed to `snow_main` by the lowerer. If the module prefix is applied first, it becomes `Main__main`, then the `snow_main` rename doesn't trigger.
**Why it happens:** Module prefix logic runs before the `main` -> `snow_main` rename check.
**How to avoid:** Apply the `main` -> `snow_main` rename BEFORE the module prefix logic. Or exclude `main` from prefixing entirely (it is the entry point, always unique).
**Warning signs:** Binary has no entry point, linker error about missing `main`.

### Pitfall 4: Prefixing Builtin/Runtime Function References
**What goes wrong:** Each module generates calls to builtins like `println`, `snow_string_concat`, `Ord__compare__Int`. If these get module prefixes, calls won't match the single definition.
**Why it happens:** Applying prefix to ALL function definitions, not just user-defined private ones.
**How to avoid:** Only prefix user-defined private functions. Builtins, runtime functions, and trait method implementations use their existing global names.
**Warning signs:** Runtime crashes, "undefined function" errors from LLVM verifier.

### Pitfall 5: Trait Impl Function Name Collision
**What goes wrong:** Module A and Module B both define `impl Display for MyType`. If both produce `Display__to_string__MyType`, the merge deduplicates incorrectly.
**Why it happens:** Trait impls use `Trait__Method__Type` naming without module qualification.
**How to avoid:** Per the prior decision, trait impls are globally visible (XMOD-05). The type checker prevents duplicate impls for the same (Trait, Type) pair across modules. If two modules define the same impl, it is a type error caught before MIR lowering.
**Warning signs:** This should not happen if Phase 39/40 work correctly. If it does, it indicates a type checker bug.

### Pitfall 6: Module-Qualified Struct/Sum Type Names in Merge
**What goes wrong:** Module A defines `struct Point { x, y }` (private). Module B also defines `struct Point { a, b }` (private). After merge, only one `Point` struct definition survives, causing field access errors.
**Why it happens:** Struct deduplication uses the struct name, same as functions.
**How to avoid:** Private struct/sum type definitions also need module-qualified names. Module-qualified type names are a prior decision ("Module-qualified type names from day one").
**Warning signs:** Field access on a struct from one module produces wrong field indices or crashes.

## Code Examples

### Current MIR Lowering Entry Point (No Module Name)
```rust
// Source: snow-codegen/src/lib.rs lines 58-64
pub fn lower_to_mir_raw(
    parse: &snow_parser::Parse,
    typeck: &snow_typeck::TypeckResult,
) -> Result<mir::MirModule, String> {
    let module = lower_to_mir(parse, typeck)?;
    Ok(module)
}
```

### Current Merge Function (Dedup by Name)
```rust
// Source: snow-codegen/src/lib.rs lines 247-297
pub fn merge_mir_modules(
    modules: Vec<mir::MirModule>,
    entry_module_idx: usize,
) -> mir::MirModule {
    let mut seen_functions: HashSet<String> = HashSet::new();
    // ...
    for module in &modules {
        for func in &module.functions {
            if seen_functions.insert(func.name.clone()) {
                merged.functions.push(func.clone());
            }
        }
        // same for structs, sum_types
    }
    monomorphize(&mut merged);
    merged
}
```

### Current Function Name Generation (No Module Prefix)
```rust
// Source: snow-codegen/src/mir/lower.rs lines 740-746
let fn_name = if name == "main" {
    self.entry_function = Some("snow_main".to_string());
    "snow_main".to_string()
} else {
    name  // <-- no module prefix applied
};
```

### Current Build Pipeline Calling Lower
```rust
// Source: snowc/src/main.rs lines 340-351
for (i, &id) in project.compilation_order.iter().enumerate() {
    let idx = id.0 as usize;
    let parse = &project.module_parses[idx];
    let typeck = all_typeck[idx].as_ref()
        .ok_or("Module was not type-checked")?;
    let mir = snow_codegen::lower_to_mir_raw(parse, typeck)?;
    if id == entry_id {
        entry_mir_idx = i;
    }
    mir_modules.push(mir);
}
let merged_mir = snow_codegen::merge_mir_modules(mir_modules, entry_mir_idx);
```

### Test Infrastructure for Multi-Module E2E Tests
```rust
// Source: snowc/tests/e2e.rs lines 1570-1609
fn compile_multifile_and_run(files: &[(&str, &str)]) -> String {
    // Creates temp dir, writes files, runs `snowc build`, runs binary, returns stdout
}

fn compile_multifile_expect_error(files: &[(&str, &str)]) -> String {
    // Creates temp dir, writes files, runs `snowc build`, expects failure, returns stderr
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Single-file compilation only | Multi-module: per-module `lower_to_mir_raw` + `merge_mir_modules` | Phase 39 (2026-02-09) | Pipeline exists, name collision is the remaining gap |
| All functions exported | `pub`-filtered exports via `collect_exports` | Phase 40 (2026-02-09) | Visibility info available for pub/private distinction |
| No module awareness in lowerer | `qualified_modules` and `imported_functions` in TypeckResult | Phase 39 | Lowerer knows about imported modules for trait dispatch skip |

**Key current state:**
- `lower_to_mir_raw` exists but takes no module name parameter
- `merge_mir_modules` exists and works for the happy path (no name collisions)
- `monomorphize` is a reachability-only pass (not true specialization)
- Trait method mangling (`Trait__Method__Type`) already uses double-underscore convention
- The entry module's `main` is already renamed to `snow_main`
- Closure names are module-local (`__closure_0`, `__closure_1`) and will collide

## Open Questions

1. **Cross-Module Generic Functions: Does the Current Pipeline Work?**
   - What we know: The type checker resolves all generic call sites to concrete types. The lowerer emits concrete MirTypes at call sites. But the function definition in the defining module may have unresolved type variables (Ty::Var -> MirType::Unit fallback).
   - What's unclear: Whether a call to `identity(42)` (emitted as `Call("identity", [IntLit(42, Int)], Int)`) correctly dispatches to a function defined with `Unit` parameter type, or whether LLVM rejects the type mismatch.
   - Recommendation: Write an E2E test for this exact scenario first. If it works (because LLVM's ptr-based calling convention is permissive), document it. If it fails, add a post-merge type fixup or re-lowering step.

2. **Should Pub Struct/Sum Type Names Be Module-Qualified?**
   - What we know: Prior decision says "Module-qualified type names from day one." Currently, struct names are NOT module-qualified (e.g., `Point` not `Geometry__Point`). The merge deduplicates by name.
   - What's unclear: Whether this phase should implement module-qualified type names, or if that's Phase 42's scope.
   - Recommendation: For this phase, focus on function name collision (XMOD-07). Type name collision is not in the requirements (XMOD-06/07). Add module-qualified struct/sum type names only if needed to fix a test failure. The prior decision about "module-qualified type names from day one" may need to be deferred to Phase 42.

3. **How Should Multi-Clause Functions Handle Module Prefixes?**
   - What we know: Multi-clause functions (e.g., `fn fib(0) = 0; fn fib(1) = 1; fn fib(n) = ...`) are lowered via `lower_multi_clause_fn` which produces a single `MirFunction` with the original name.
   - What's unclear: Whether the multi-clause function name in `lower_multi_clause_fn` also needs the module prefix.
   - Recommendation: Yes, same rule applies -- if the multi-clause function is private, prefix it. The prefixing should happen at the final name assignment, not at each clause.

4. **What About `from Module import fn_name` Call References?**
   - What we know: When module B does `from Utils import helper`, the type checker adds `helper` to `imported_functions`. Module B's lowerer generates calls to `helper`. But if `helper` is pub in Utils, it keeps its unqualified name. If it's somehow private (the type checker should block this), there's a mismatch.
   - What's unclear: Whether the selective import path correctly resolves to unqualified names.
   - Recommendation: This should work correctly since Phase 40's visibility enforcement ensures only pub functions are importable, and pub functions keep their unqualified names. Verify with E2E test.

## Sources

### Primary (HIGH confidence)
- `snow-codegen/src/lib.rs` -- `merge_mir_modules` implementation, `lower_to_mir_raw` API, `compile_mir_to_binary`
- `snow-codegen/src/mir/lower.rs` -- `Lowerer` struct, `lower_fn_def`, `lower_source_file`, `known_functions`, closure naming
- `snow-codegen/src/mir/mono.rs` -- `monomorphize` reachability pass, `collect_function_refs`
- `snow-codegen/src/mir/types.rs` -- `resolve_type`, `mangle_type_name`, Ty::Var fallback to MirType::Unit
- `snow-codegen/src/codegen/mod.rs` -- `declare_functions`, `compile_function`, LLVM function naming
- `snow-typeck/src/lib.rs` -- `TypeckResult` with `qualified_modules`, `imported_functions`, `ImportContext`, `ExportedSymbols`
- `snowc/src/main.rs` -- build pipeline, per-module lowering loop, merge invocation
- `snowc/src/discovery.rs` -- `path_to_module_name`, `build_project`, `ProjectData`
- `snowc/tests/e2e.rs` -- `compile_multifile_and_run`, `compile_multifile_expect_error` test helpers
- `snow-common/src/module_graph.rs` -- `ModuleGraph`, `ModuleId`, `ModuleInfo`

### Secondary (MEDIUM confidence)
- Phase 39 RESEARCH.md -- ImportContext design, accumulator pattern, qualified module access
- Phase 40 RESEARCH.md -- Visibility enforcement design, pub/private export filtering

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- all changes are in existing crates, no new dependencies
- Architecture (name mangling): HIGH -- well-understood problem, clear solution, follows existing `Trait__Method__Type` convention
- Architecture (cross-module generics): MEDIUM -- the current pipeline may already work for the common case (type checker resolves generics), but the Ty::Var -> MirType::Unit fallback is a risk for generic function definitions. E2E testing is required.
- Pitfalls: HIGH -- identified 6 specific pitfalls from code analysis with clear prevention strategies

**Research date:** 2026-02-09
**Valid until:** 30 days (stable compiler internals, no external dependencies changing)
