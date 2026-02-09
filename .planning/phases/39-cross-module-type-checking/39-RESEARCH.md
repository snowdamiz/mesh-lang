# Phase 39: Cross-Module Type Checking - Research

**Researched:** 2026-02-09
**Domain:** Compiler cross-module type checking -- import resolution, export collection, pre-seeded type environments, qualified/selective access, cross-module struct/sum type/trait visibility
**Confidence:** HIGH

## Summary

Phase 39 is the central phase of the module system: it makes types and functions from one `.snow` file usable in another via imports. The current compiler type-checks only the entry module (`main.snow`), ignoring all other parsed files. Phase 39 must change this to: (1) type-check ALL modules in topological order, (2) collect each module's public exports after type checking, (3) pre-seed the TypeEnv/TypeRegistry/TraitRegistry of downstream modules with upstream exports before inference, and (4) resolve both `import Foo` (qualified access: `Foo.bar()`) and `from Foo import { bar }` (unqualified access: `bar()`) against user-defined modules (not just the hardcoded stdlib).

The existing `check()` function in `snow-typeck` creates a fresh `TypeEnv`, `TypeRegistry`, and `TraitRegistry`, registers builtins, and runs inference on a single `Parse`. Phase 39 needs a new `check_with_imports()` entry point that accepts an `ImportContext` containing pre-resolved symbols from dependency modules. This ImportContext populates the TypeEnv (function schemes), TypeRegistry (struct/sum type definitions), and TraitRegistry (trait defs and impls) before inference begins. The existing `infer()` logic is unchanged -- it just finds more names in scope.

The current import handling in `infer_item` only supports stdlib modules via the hardcoded `stdlib_modules()` HashMap. Phase 39 extends both `ImportDecl` and `FromImportDecl` handling to check user-defined modules first, falling back to stdlib. Qualified access (`Vector.add(a, b)`) is resolved in `infer_field_access`, which currently checks `is_stdlib_module()` -- this must be extended to check a `qualified_modules` map populated from the ImportContext. Error reporting for non-existent modules (IMPORT-06) and non-existent names (IMPORT-07) is also required. Trait impls (XMOD-05) are globally visible -- all impls from all modules are merged into every module's TraitRegistry without explicit import.

**Primary recommendation:** Add `check_with_imports(parse, import_ctx)` to `snow-typeck`, add `collect_exports(parse, typeck)` to extract public symbols, and modify the `snowc build()` pipeline to type-check modules in topological order using the accumulator pattern. Keep `check()` unchanged for backward compatibility.

## Standard Stack

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| snow-typeck | local | Type checking with import context support | Existing crate, extended with `check_with_imports` + `collect_exports` |
| snow-common (module_graph) | local | ModuleGraph, ModuleId, ModuleInfo | Built in Phase 37, provides module identity and dependency structure |
| snowc (discovery) | local | ProjectData, build_project, extract_imports | Built in Phase 38, provides parsed ASTs and compilation order |
| snow-parser | local | Per-file Parse results, AST node types | Existing parser, provides ImportDecl/FromImportDecl AST nodes |
| rustc-hash | workspace | FxHashMap for type registries and export maps | Already used throughout typeck for performance |

### Supporting
| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| tempfile | 3 (dev-dep) | Creating multi-file test project directories | All integration and e2e tests |
| rowan | workspace | TextRange for span tracking in cross-module diagnostics | Already a dependency, used for error provenance |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| ImportContext struct | Passing raw HashMaps | ImportContext is more type-safe and self-documenting |
| Pre-seeded TypeEnv | Demand-driven module loading | Pre-seeding is simpler, no file I/O in typeck, no circular dependency risk |
| Module-last-segment as namespace key | Full dot-path as key | Last segment (`Vector`) matches user syntax; full path (`Math.Vector`) avoids collisions but is more complex. Use last segment for `import`, full path internally. |

**Installation:**
No new dependencies needed. All libraries are already in the workspace.

## Architecture Patterns

### Recommended Project Structure
```
crates/snow-typeck/src/
  lib.rs              # Add check_with_imports(), ImportContext, ExportedSymbols, collect_exports()
  infer.rs            # Modify infer() to accept ImportContext, extend ImportDecl/FromImportDecl handling
  env.rs              # No changes (TypeEnv scope stack works as-is)
  traits.rs           # No changes (TraitRegistry works as-is, just pre-seeded)

crates/snowc/src/
  main.rs             # Modify build() to type-check all modules in topological order
  discovery.rs        # No changes (ProjectData from Phase 38 is sufficient)
```

### Pattern 1: Accumulator Pattern for Cross-Module Exports
**What:** Process modules in topological order. After type-checking each module, collect its exported symbols into a Vec indexed by ModuleId. When type-checking a downstream module, build its ImportContext from the exports of its dependency modules.
**When to use:** Always -- this is the core orchestration pattern for Phase 39.
**Example:**
```rust
// In snowc/src/main.rs build() function

let mut all_exports: Vec<ExportedSymbols> = Vec::new();

for &id in &project.compilation_order {
    let idx = id.0 as usize;
    let parse = &project.module_parses[idx];
    let source = &project.module_sources[idx];

    // Build ImportContext from already-checked dependencies
    let import_ctx = build_import_context(
        &project.graph,
        &all_exports,
        &project.module_parses[idx],  // for reading import declarations
        id,
    );

    // Type-check this module with imports
    let typeck = snow_typeck::check_with_imports(parse, &import_ctx);

    // Report diagnostics for this module
    has_errors |= report_diagnostics(source, &mod_path, parse, &typeck, diag_opts);

    // Collect exports for downstream modules
    let exports = snow_typeck::collect_exports(parse, &typeck);
    all_exports.push(exports);
}
```

### Pattern 2: ImportContext Pre-seeding
**What:** Before running inference on a module, populate the TypeEnv, TypeRegistry, and TraitRegistry with symbols from imported modules. The inference engine sees these as if they were defined locally.
**When to use:** At the start of `check_with_imports`, before the existing `infer()` logic runs.
**Example:**
```rust
// In snow-typeck/src/lib.rs

pub struct ImportContext {
    /// Module name -> exported function/value schemes
    /// Key is the last segment of the module path (e.g., "Vector" for Math.Vector)
    pub module_exports: FxHashMap<String, ModuleExports>,
    /// Struct definitions from all imported modules
    pub imported_structs: FxHashMap<String, StructDefInfo>,
    /// Sum type definitions from all imported modules
    pub imported_sum_types: FxHashMap<String, SumTypeDefInfo>,
    /// Trait definitions from ALL modules (globally visible)
    pub imported_trait_defs: Vec<traits::TraitDef>,
    /// Trait impls from ALL modules (globally visible, XMOD-05)
    pub imported_trait_impls: Vec<traits::ImplDef>,
}

pub struct ModuleExports {
    /// Function/value type schemes, keyed by name
    pub symbols: FxHashMap<String, ty::Scheme>,
    /// Struct definitions exported by this module
    pub struct_defs: FxHashMap<String, StructDefInfo>,
    /// Sum type definitions exported by this module
    pub sum_type_defs: FxHashMap<String, SumTypeDefInfo>,
}
```

### Pattern 3: Qualified Access Resolution
**What:** When `import Math.Vector` is processed, the last segment `Vector` is registered as a namespace key. `Vector.add(a, b)` is resolved by looking up `Vector` in the qualified modules map, then `add` in that module's exports.
**When to use:** In `infer_field_access` when the base expression is a NameRef that matches a qualified module name.
**Example:**
```rust
// In infer_field_access, extend the existing stdlib_module check:
if let Expr::NameRef(ref name_ref) = base_expr {
    if let Some(base_name) = name_ref.text() {
        // Check user-defined modules first (from import context)
        if let Some(mod_exports) = qualified_modules.get(&base_name) {
            if let Some(scheme) = mod_exports.symbols.get(&field_name) {
                let ty = ctx.instantiate(scheme);
                return Ok(ty);
            }
        }
        // Then check stdlib modules (existing behavior)
        if is_stdlib_module(&base_name) {
            // ... existing stdlib_modules() logic
        }
    }
}
```

### Pattern 4: Selective Import Resolution
**What:** When `from Math.Vector import { add, scale }` is processed, each imported name is looked up in the module's exports and inserted into the TypeEnv as an unqualified name. This makes `add(a, b)` directly callable.
**When to use:** In `infer_item` for `FromImportDecl` items.
**Example:**
```rust
Item::FromImportDecl(ref from_import) => {
    // First try user-defined modules
    let module_name = segments.join(".");
    let last_segment = segments.last().unwrap();

    if let Some(mod_exports) = import_ctx.module_exports.get(last_segment) {
        if let Some(import_list) = from_import.import_list() {
            for name_node in import_list.names() {
                if let Some(name) = name_node.text() {
                    if let Some(scheme) = mod_exports.symbols.get(&name) {
                        env.insert(name.clone(), scheme.clone());
                    } else {
                        // IMPORT-07: Name not found in module
                        ctx.errors.push(TypeError::ImportNameNotFound {
                            module_name: module_name.clone(),
                            name: name.clone(),
                            span: name_node.syntax().text_range(),
                        });
                    }
                }
            }
        }
    } else {
        // Fall back to stdlib_modules() for backward compat
        let modules = stdlib_modules();
        // ... existing stdlib logic
    }
    None
}
```

### Anti-Patterns to Avoid
- **Modifying the existing `check()` entry point:** Keep it unchanged. Add `check_with_imports()` as a new entry point. The existing `check()` is equivalent to `check_with_imports()` with an empty ImportContext.
- **Demand-driven module loading in the type checker:** The type checker must never read files. All module data comes through ImportContext, pre-built by the `snowc` driver.
- **Implicit transitive imports:** If A imports B and B imports C, A cannot use C's exports. A must explicitly import C. This keeps the dependency graph honest.
- **Merging all modules into one TypeEnv:** Each module gets its own fresh TypeEnv pre-seeded with its specific imports. Do not build a global TypeEnv with everything.
- **Skipping trait impl registration for non-imported modules:** Trait impls are globally visible (XMOD-05). ALL trait impls from ALL modules must be registered in every module's TraitRegistry. The user does not import trait impls explicitly.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Module discovery + parsing | New file walker + parser | `discovery::build_project()` (Phase 38) | Already builds ProjectData with all parses in topo order |
| Topological ordering | New topo sort | `module_graph::topological_sort()` (Phase 37) | Already tested and handles cycles |
| Type environment scoping | New scope system | `TypeEnv` with `push_scope/pop_scope` | Already handles lexical scoping correctly |
| Struct/sum type registration | New registration logic | Existing `register_struct_def`/`register_sum_type_def` | Just call these with imported definitions |
| Trait impl matching | New matching logic | `TraitRegistry.register_impl()` + `has_impl()` | Already handles structural matching with freshening |
| Qualified access resolution | New field access handler | Extend existing `infer_field_access` | Already handles `Module.function` for stdlib |

**Key insight:** Phase 39's new code is primarily orchestration and wiring. The type checking engine (HM inference, unification, generalization, trait resolution) is unchanged. The new work is: (a) collecting exports, (b) building ImportContext, (c) extending import handling to check user modules, and (d) extending qualified access to check user modules.

## Common Pitfalls

### Pitfall 1: Type Identity Across Module Boundaries
**What goes wrong:** Module A defines `struct Point { x :: Int, y :: Int }`. Module B imports Point and creates `Point { x: 1, y: 2 }`. The type checker in B creates a new `Ty::App(Con("Point"), [])` which doesn't unify with A's `Point` because they are structurally identical but from different inference contexts.
**Why it happens:** Each module gets a fresh `InferCtx` with its own `InPlaceUnificationTable`. Type variables from A's inference do not exist in B's context.
**How to avoid:** Use module-qualified type names from day one (prior decision). Register imported types in B's TypeRegistry using the same names as A used. Since types are compared by name (TyCon name string), using the same name ensures identity. E.g., if A defines `Point`, B's TypeRegistry also contains `Point` with the same fields. The struct name `"Point"` matches in both contexts.
**Warning signs:** "type mismatch: expected Point, found Point" errors when using imported structs.

### Pitfall 2: Trait Impls Not Globally Visible
**What goes wrong:** Module A defines `impl Display for Point`. Module B imports Point but cannot call `point.to_string()` because B's TraitRegistry does not have A's impl.
**Why it happens:** Each module gets a fresh TraitRegistry. Without explicitly merging impls from all modules, trait resolution fails.
**How to avoid:** XMOD-05 requires trait impls to be globally visible. Collect ALL trait impls from ALL already-checked modules (not just direct dependencies) and register them in every module's TraitRegistry before inference. This includes impls from transitive dependencies.
**Warning signs:** "no method `to_string` on type Point" errors when the impl exists in another module.

### Pitfall 3: Circular Reference Between Export Collection and Type Checking
**What goes wrong:** To collect exports, you need the TypeckResult. But to type-check, you need the ImportContext (which contains exports from dependencies). If the orchestration is wrong, you try to collect exports before type checking.
**Why it happens:** Confusing the accumulator order.
**How to avoid:** The accumulator pattern enforces the correct order: (1) type-check module, (2) collect exports, (3) store exports for downstream. The topological sort guarantees all dependencies are processed first.
**Warning signs:** Panics or empty export maps when building ImportContext.

### Pitfall 4: Stdlib Modules Shadowed by User Modules
**What goes wrong:** A user creates a file `string.snow` which becomes module `String`. Now `from String import length` resolves against the user's module (which may not export `length`) instead of the stdlib String module.
**Why it happens:** User-defined modules are checked before stdlib in the import resolution order.
**How to avoid:** Check user modules first (they take priority), but if the user module does not export the requested name, fall back to stdlib. Alternatively, document that user modules shadow stdlib modules and the user should choose different names. The simpler approach: check user modules first, if not found check stdlib. If found in neither, report IMPORT-07 error.
**Warning signs:** Existing programs using `from String import length` break when a `string.snow` file is added.

### Pitfall 5: Sum Type Variant Constructors Not Imported
**What goes wrong:** Module A defines `type Shape = Circle(Float) | Rectangle(Float, Float)`. Module B imports Shape but cannot write `Circle(5.0)` because the variant constructor `Circle` is not in B's TypeEnv.
**Why it happens:** When registering a sum type, the inference engine calls `register_variant_constructors` to add variant constructor functions to the TypeEnv. If imported sum types are only registered in TypeRegistry but not in the TypeEnv, constructors are missing.
**How to avoid:** When pre-seeding imported sum types, call `register_variant_constructors` to add constructor functions to the TypeEnv, exactly as if the sum type were defined locally. Also register in the exhaustiveness TypeRegistry for pattern matching.
**Warning signs:** "unbound variable `Circle`" errors when constructing imported sum type variants.

### Pitfall 6: Breaking Existing Stdlib Import Tests
**What goes wrong:** The existing `from String import length` tests work because `infer_item` checks `stdlib_modules()`. If Phase 39 changes the import resolution order without proper fallback, these tests break.
**Why it happens:** The new import resolution checks user modules first. If the stdlib check is removed or the fallback is buggy, existing behavior breaks.
**How to avoid:** The new import handling must check user modules first, then fall back to `stdlib_modules()` for names not found in user modules. ALL existing e2e tests must pass after Phase 39 changes.
**Warning signs:** Tests using `from String import length` or `String.length(s)` fail after Phase 39.

### Pitfall 7: Non-Existent Module Error Not Reported
**What goes wrong:** `import NonExistent` silently does nothing (Phase 38 silently skips unknown imports during graph construction). Phase 39 must report this as an error (IMPORT-06).
**Why it happens:** Phase 38 explicitly skips unknown imports to defer error reporting to Phase 39. But if Phase 39 does not add this error, the user gets no feedback.
**How to avoid:** In `infer_item` for `ImportDecl`, check both user modules (via ImportContext) and stdlib modules. If neither contains the module, emit an `ImportModuleNotFound` error with the file path suggestion.
**Warning signs:** `import Typo` silently compiles without error.

## Code Examples

### Example 1: ImportContext and ExportedSymbols Types
```rust
// In snow-typeck/src/lib.rs

use crate::traits::{TraitDef, ImplDef as TraitImplDef};
use crate::ty::Scheme;

/// Context built by the driver from already-checked dependency modules.
/// Pre-seeds the type checker's TypeEnv, TypeRegistry, and TraitRegistry.
pub struct ImportContext {
    /// Module namespace -> exported symbols.
    /// Key is the namespace name used for qualified access (last path segment
    /// for `import Math.Vector` -> key is "Vector").
    pub module_exports: FxHashMap<String, ModuleExports>,

    /// Trait definitions from ALL processed modules (globally visible).
    pub all_trait_defs: Vec<TraitDef>,

    /// Trait impls from ALL processed modules (globally visible, XMOD-05).
    pub all_trait_impls: Vec<TraitImplDef>,
}

/// Exports from a single module.
pub struct ModuleExports {
    /// The full module name (e.g., "Math.Vector").
    pub module_name: String,

    /// Function/value type schemes, keyed by unqualified name.
    pub functions: FxHashMap<String, Scheme>,

    /// Struct definitions exported by this module.
    pub struct_defs: FxHashMap<String, StructDefInfo>,

    /// Sum type definitions exported by this module.
    pub sum_type_defs: FxHashMap<String, SumTypeDefInfo>,
}

impl ImportContext {
    pub fn empty() -> Self {
        ImportContext {
            module_exports: FxHashMap::default(),
            all_trait_defs: Vec::new(),
            all_trait_impls: Vec::new(),
        }
    }
}

/// Symbols exported by a module after type checking.
pub struct ExportedSymbols {
    /// Function type schemes (name -> scheme).
    pub functions: FxHashMap<String, Scheme>,
    /// Struct definitions.
    pub struct_defs: FxHashMap<String, StructDefInfo>,
    /// Sum type definitions.
    pub sum_type_defs: FxHashMap<String, SumTypeDefInfo>,
    /// Trait definitions declared in this module.
    pub trait_defs: Vec<TraitDef>,
    /// Trait impls declared in this module.
    pub trait_impls: Vec<TraitImplDef>,
}
```

### Example 2: check_with_imports Entry Point
```rust
// In snow-typeck/src/lib.rs

/// Type-check a parsed Snow program with pre-resolved imports.
///
/// This is the multi-module entry point. The ImportContext contains
/// symbols from already-type-checked dependency modules. These are
/// registered in the TypeEnv, TypeRegistry, and TraitRegistry before
/// inference begins.
pub fn check_with_imports(parse: &Parse, import_ctx: &ImportContext) -> TypeckResult {
    infer::infer_with_imports(parse, import_ctx)
}
```

### Example 3: infer_with_imports (Extended infer())
```rust
// In snow-typeck/src/infer.rs

pub fn infer_with_imports(parse: &Parse, import_ctx: &ImportContext) -> TypeckResult {
    let mut ctx = InferCtx::new();
    let mut env = TypeEnv::new();
    let mut trait_registry = TraitRegistry::new();
    let mut type_registry = TypeRegistry::new();
    builtins::register_builtins(&mut ctx, &mut env, &mut trait_registry);
    register_builtin_sum_types(&mut ctx, &mut env, &mut type_registry);

    // Pre-seed with imported trait defs and impls (XMOD-05: globally visible)
    for trait_def in &import_ctx.all_trait_defs {
        trait_registry.register_trait(trait_def.clone());
    }
    for impl_def in &import_ctx.all_trait_impls {
        let _ = trait_registry.register_impl(impl_def.clone());
    }

    // Pre-seed with imported struct definitions (XMOD-03)
    for mod_exports in import_ctx.module_exports.values() {
        for (name, struct_def) in &mod_exports.struct_defs {
            type_registry.register_struct(struct_def.clone());
            // Register struct constructor in TypeEnv
            let struct_ty = if struct_def.generic_params.is_empty() {
                Ty::struct_ty(name, vec![])
            } else {
                let type_args: Vec<Ty> = struct_def.generic_params.iter()
                    .map(|_| ctx.fresh_var()).collect();
                Ty::struct_ty(name, type_args)
            };
            env.insert(name.clone(), Scheme::mono(struct_ty));
        }
    }

    // Pre-seed with imported sum type definitions (XMOD-04)
    for mod_exports in import_ctx.module_exports.values() {
        for (_name, sum_type_def) in &mod_exports.sum_type_defs {
            type_registry.register_sum_type(sum_type_def.clone());
            // Register variant constructors in TypeEnv
            register_variant_constructors(
                &mut ctx, &mut env,
                &sum_type_def.name,
                &sum_type_def.generic_params,
                &sum_type_def.variants,
            );
        }
    }

    // Build qualified_modules map for qualified access resolution
    let mut qualified_modules: FxHashMap<String, FxHashMap<String, Scheme>> =
        FxHashMap::default();
    // (populated when ImportDecl items are processed during inference)

    // ... rest of existing infer() logic, but with import_ctx available
    // for ImportDecl and FromImportDecl resolution
}
```

### Example 4: collect_exports Function
```rust
// In snow-typeck/src/lib.rs (or infer.rs)

/// Collect exported symbols from a type-checked module.
///
/// Currently exports ALL top-level definitions (Phase 40 adds pub filtering).
/// Scans the AST for top-level FnDef, StructDef, SumTypeDef, InterfaceDef,
/// and ImplDef items, extracting their type information from the TypeckResult.
pub fn collect_exports(
    parse: &Parse,
    typeck: &TypeckResult,
) -> ExportedSymbols {
    let tree = parse.tree();
    let mut exports = ExportedSymbols {
        functions: FxHashMap::default(),
        struct_defs: FxHashMap::default(),
        sum_type_defs: FxHashMap::default(),
        trait_defs: Vec::new(),
        trait_impls: Vec::new(),
    };

    for item in tree.items() {
        match item {
            Item::FnDef(fn_def) => {
                // Phase 40 will add: if fn_def.visibility().is_none() { continue; }
                if let Some(name) = fn_def.name().and_then(|n| n.text()) {
                    // Look up the function's inferred type from the typeck result
                    let range = fn_def.syntax().text_range();
                    if let Some(ty) = typeck.types.get(&range) {
                        exports.functions.insert(
                            name,
                            Scheme::mono(ty.clone()),
                        );
                    }
                }
            }
            Item::StructDef(_) => {
                // Struct defs are in typeck.type_registry.struct_defs
                // Copy them to exports
            }
            Item::SumTypeDef(_) => {
                // Sum type defs are in typeck.type_registry.sum_type_defs
                // Copy them to exports
            }
            // Trait defs and impls come from typeck.trait_registry
            _ => {}
        }
    }

    // Copy struct/sum type defs from type_registry
    exports.struct_defs = typeck.type_registry.struct_defs.clone();
    exports.sum_type_defs = typeck.type_registry.sum_type_defs.clone();

    // Trait registry exports need a public API to extract defs/impls
    // (add accessor methods to TraitRegistry)

    exports
}
```

### Example 5: New TypeError Variants for Import Errors
```rust
// In snow-typeck/src/error.rs

/// Module not found during import resolution.
ImportModuleNotFound {
    module_name: String,
    span: TextRange,
    /// Optional suggestion (closest module name match).
    suggestion: Option<String>,
},

/// Name not found in imported module.
ImportNameNotFound {
    module_name: String,
    name: String,
    span: TextRange,
    /// Available names in the module (for "did you mean?" suggestions).
    available: Vec<String>,
},
```

### Example 6: Modified build() Pipeline
```rust
// In snowc/src/main.rs

fn build(dir: &Path, ...) -> Result<(), String> {
    // ... existing validation ...

    let project = discovery::build_project(dir)?;

    // Check parse errors in ALL modules first (existing Phase 38 logic)
    // ...

    // Type-check ALL modules in topological order
    let mut all_exports: Vec<snow_typeck::ExportedSymbols> = Vec::new();
    let mut all_typeck: Vec<snow_typeck::TypeckResult> = Vec::new();
    let mut has_errors = false;

    for &id in &project.compilation_order {
        let idx = id.0 as usize;
        let parse = &project.module_parses[idx];
        let source = &project.module_sources[idx];
        let module_path = dir.join(&project.graph.get(id).path);

        // Build ImportContext from already-checked dependencies
        let import_ctx = build_import_context(
            &project.graph,
            &all_exports,
            parse,
            id,
        );

        // Type-check this module with imports
        let typeck = snow_typeck::check_with_imports(parse, &import_ctx);

        // Report diagnostics
        let file_name = module_path.display().to_string();
        for error in &typeck.errors {
            has_errors = true;
            let rendered = snow_typeck::diagnostics::render_diagnostic(
                error, source, &file_name, diag_opts, None,
            );
            eprint!("{}", rendered);
        }

        // Collect exports for downstream modules
        let exports = snow_typeck::collect_exports(parse, &typeck);
        all_exports.push(exports);
        all_typeck.push(typeck);
    }

    if has_errors {
        return Err("Compilation failed due to errors above.".to_string());
    }

    // Compile entry module (Phase 41 will compile all via MIR merge)
    let entry_id = project.compilation_order.iter()
        .copied()
        .find(|id| project.graph.get(*id).is_entry)
        .ok_or("No entry module found")?;
    let entry_idx = entry_id.0 as usize;
    let entry_parse = &project.module_parses[entry_idx];
    let entry_typeck = &all_typeck[entry_idx];

    snow_codegen::compile_to_binary(entry_parse, entry_typeck, &output_path, opt_level, target, None)?;
    Ok(())
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Only entry module type-checked | All modules type-checked in topo order | Phase 39 (this phase) | Cross-module type errors caught at compile time |
| Only stdlib modules for import resolution | User-defined + stdlib modules for imports | Phase 39 | User modules usable via import |
| No export collection | Exports collected after each module check | Phase 39 | Downstream modules can use upstream symbols |
| Single `check()` entry point | `check()` + `check_with_imports()` | Phase 39 | Multi-module type checking with backward compat |

**Deprecated/outdated:**
- Direct `snow_typeck::check()` for multi-module builds: use `check_with_imports()` instead. `check()` remains for single-file/REPL use.
- `build()` type-checking only the entry module: now type-checks all modules.

## Open Questions

1. **Should ALL definitions be exported in Phase 39, or only `pub` ones?**
   - What we know: Phase 40 adds visibility enforcement (`pub` modifier). Phase 39's success criteria do not mention visibility filtering.
   - What's unclear: Whether to export everything in Phase 39 and restrict in Phase 40, or to implement pub-filtering immediately.
   - Recommendation: Export ALL definitions in Phase 39. Phase 40 will add `pub` filtering. This keeps Phase 39 focused on cross-module type checking without mixing in visibility concerns. It matches the roadmap structure (Phase 40 = "Visibility Enforcement").

2. **How should the `import` statement resolve multi-segment paths?**
   - What we know: `import Math.Vector` should bring `Vector` into scope for qualified access. The module name in ModuleGraph is `"Math.Vector"`. The user uses `Vector.add(a, b)` in code.
   - What's unclear: Should `Math.Vector.add(a, b)` also work? Should `Math` be a namespace too?
   - Recommendation: For Phase 39, `import Math.Vector` makes the LAST segment (`Vector`) available for qualified access. `Vector.add(a, b)` works. Full-path `Math.Vector.add(a, b)` is deferred -- it requires nested namespace support which adds complexity. Keep it simple: import makes the module accessible by its last path segment.

3. **How to handle the `all_exports` Vec indexing?**
   - What we know: `compilation_order` is a `Vec<ModuleId>` in topological order. When iterating, we process modules by this order. But `all_exports[i]` corresponds to the i-th module in PROCESSING ORDER, not by ModuleId.
   - What's unclear: Should `all_exports` be indexed by processing position or by `ModuleId.0`?
   - Recommendation: Index `all_exports` by `ModuleId.0` (same as `module_sources` and `module_parses`). Use `Option<ExportedSymbols>` initialized to None, fill in as modules are checked. This avoids index confusion. Alternatively, pre-allocate `Vec<Option<ExportedSymbols>>` with `graph.module_count()` entries.

4. **Should trait defs/impls be extracted from TraitRegistry or re-scanned from AST?**
   - What we know: After type checking, the TraitRegistry contains all registered trait defs and impls. But it does not distinguish between imported traits (from builtins/imports) and locally-defined traits.
   - What's unclear: How to extract only the locally-defined traits for export.
   - Recommendation: Add public accessor methods to TraitRegistry that return refs to all registered traits/impls. For Phase 39 (no visibility), export everything. Alternatively, track which traits/impls were registered during this module's inference (not pre-seeded). A simple approach: collect trait names from AST `InterfaceDef` items, then look them up in the TraitRegistry. For impls, collect them from `ImplDef` AST items.

5. **How does the codegen pipeline handle imported types?**
   - What we know: Currently `compile_to_binary` takes a single `(Parse, TypeckResult)`. Phase 39 type-checks all modules but only compiles the entry module. The entry module's TypeckResult will now contain imported types in its TypeRegistry.
   - What's unclear: Will the MIR lowerer correctly handle struct/sum types that are in the TypeRegistry but defined in a different Parse tree?
   - Recommendation: For Phase 39, the entry module's codegen works because all imported types are registered in its TypeRegistry and TypeEnv. The MIR lowerer reads type definitions from TypeckResult.type_registry, which contains both local and imported definitions. Struct layout and sum type tag information is available. Full MIR merge (lowering all modules) is Phase 41.

## Sources

### Primary (HIGH confidence)
- **Direct codebase analysis** of:
  - `crates/snow-typeck/src/lib.rs` -- TypeckResult struct, check() entry point, re-exports (lines 1-98)
  - `crates/snow-typeck/src/infer.rs` -- infer() main function (lines 497-646), stdlib_modules() (lines 210-490), infer_item import handling (lines 1389-1427), infer_field_access qualified access (lines 4219-4410), register_struct_def (lines 1447-1530), register_variant_constructors (line 768+)
  - `crates/snow-typeck/src/env.rs` -- TypeEnv scope stack (full file, 141 lines)
  - `crates/snow-typeck/src/traits.rs` -- TraitRegistry, TraitDef, ImplDef (full file, 817 lines)
  - `crates/snow-typeck/src/ty.rs` -- Ty, TyCon, TyVar, Scheme (full file, 245 lines)
  - `crates/snow-typeck/src/error.rs` -- TypeError variants (full file, 545 lines)
  - `crates/snow-typeck/src/exhaustiveness.rs` -- TypeInfo, ConstructorSig, TypeRegistry (first 80 lines)
  - `crates/snow-typeck/src/unify.rs` -- InferCtx (first 80 lines)
  - `crates/snowc/src/main.rs` -- build() function (full file, 522 lines)
  - `crates/snowc/src/discovery.rs` -- ProjectData, build_project, extract_imports (full file, 509 lines)
  - `crates/snow-common/src/module_graph.rs` -- ModuleGraph, ModuleId, topological_sort (full file, 358 lines)
  - `crates/snow-parser/src/ast/item.rs` -- ImportDecl, FromImportDecl, ImportList, Path (full file, 844 lines)
  - `crates/snow-codegen/src/lib.rs` -- compile_to_binary, lower_to_mir_module (full file, 178 lines)
  - `crates/snow-codegen/src/mir/mod.rs` -- MirModule, MirFunction, MirType (first 100 lines)
  - `crates/snow-codegen/src/mir/lower.rs` -- Item::ImportDecl/ModuleDef skip (line 648)
  - `crates/snowc/tests/e2e.rs` -- E2E test patterns including multi-file tests (Phase 38)

- **Planning documents:**
  - `.planning/research/ARCHITECTURE.md` -- Full multi-file pipeline architecture (858 lines)
  - `.planning/phases/38-multi-file-build-pipeline/38-RESEARCH.md` -- Phase 38 research (577 lines)
  - `.planning/ROADMAP.md` -- Phase 39 scope, success criteria, requirements
  - `.planning/REQUIREMENTS.md` -- IMPORT-01 through IMPORT-07, XMOD-01 through XMOD-05

### Secondary (MEDIUM confidence)
- General compiler design knowledge: accumulator pattern for cross-module compilation is standard in batch-mode compilers (OCaml, Elm, Haskell GHC)

### Tertiary (N/A)
- No web search needed. All information derived from direct codebase analysis and existing planning documents.

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- Zero new dependencies. All integration points verified by reading actual source code across 11 crates.
- Architecture: HIGH -- The pre-seeded TypeEnv pattern is explicitly described in ARCHITECTURE.md and aligns with how stdlib modules already work. The extension from stdlib-only to user-module support is a natural generalization.
- Pitfalls: HIGH -- Each pitfall identified from actual code analysis. Type identity, variant constructors, trait impl visibility, and stdlib shadowing are all verifiable from the source.

**Research date:** 2026-02-09
**Valid until:** 2026-03-11 (30 days -- stable domain, internal compiler extension with no external dependencies)
