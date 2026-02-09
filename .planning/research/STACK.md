# Stack Research: Module System

**Domain:** Multi-file module system for Snow compiler -- dependency graph resolution, module name resolution, cross-file symbol tables, visibility enforcement
**Researched:** 2026-02-09
**Confidence:** HIGH (based on direct codebase analysis of all 11 crates, ecosystem research, and established compiler patterns)

## Executive Summary

The module system requires **ZERO new external dependencies**. Everything needed is available through Rust's standard library and the existing crate dependencies (rustc-hash, rowan, ariadne, inkwell). The work is entirely internal to the Snow compiler's architecture -- extending the existing pipeline from single-file to multi-file compilation.

The key decisions are: (1) implement topological sort by hand (~40 lines) rather than pulling in petgraph, because the module dependency graph is small and the algorithm is trivial; (2) extend `TypeEnv` to support module-scoped symbol tables rather than introducing a separate symbol table crate; (3) build `ModuleGraph` as a new data structure in `snow-common` that all downstream crates can reference; and (4) extend the existing `TypeckResult` and `MirModule` to carry per-module data rather than replacing them with entirely new abstractions.

## What Exists Today (DO NOT CHANGE -- Build Atop These)

### Parser Infrastructure Already Present

The parser already handles all module-related syntax. This is the foundation to build on, not replace.

| CST Node | Status | Current Handling |
|----------|--------|-----------------|
| `MODULE_DEF` | Parsed correctly | Typeck skips it (`Item::ModuleDef(_) => None`) |
| `IMPORT_DECL` | Parsed correctly | Typeck validates stdlib module names only |
| `FROM_IMPORT_DECL` | Parsed correctly | Typeck injects stdlib function schemes into env |
| `IMPORT_LIST` | Parsed correctly | Used by `FromImportDecl` |
| `PATH` (qualified) | Parsed correctly | `Foo.Bar.Baz` dot-separated paths |
| `VISIBILITY` (`pub`) | Parsed correctly | Not enforced -- everything is currently public |
| `MODULE_KW` | Lexed and mapped | `TokenKind::Module -> SyntaxKind::MODULE_KW` |
| `IMPORT_KW` | Lexed and mapped | `TokenKind::Import -> SyntaxKind::IMPORT_KW` |
| `PUB_KW` | Lexed and mapped | `TokenKind::Pub -> SyntaxKind::PUB_KW` |

### AST Accessors Already Present

From `snow-parser/src/ast/item.rs`:

```
SourceFile::modules()   -> Iterator<Item = ModuleDef>
ModuleDef::name()       -> Option<Name>
ModuleDef::items()      -> Iterator<Item = Item>
ImportDecl::module_path() -> Option<Path>
FromImportDecl::module_path() -> Option<Path>
FromImportDecl::import_list() -> Option<ImportList>
ImportList::names()     -> Iterator<Item = NameRef>
Path::segments()        -> Vec<String>  (dot-separated)
```

### Current Module Resolution (Stdlib Only)

In `snow-typeck/src/infer.rs` (6,104 lines):
- `stdlib_modules()` builds `HashMap<String, HashMap<String, Scheme>>` with 14 hardcoded modules (String, IO, Env, File, List, Map, Set, Tuple, Range, Queue, HTTP, JSON, Request, Job)
- `from X import y` injects the bare name AND a prefixed form (`string_length`) into the type env
- `import X` is effectively a no-op (qualified access via `X.y` is handled in field_access inference)
- `Item::ModuleDef(_) => None` -- user-defined modules are completely ignored

### Current Compiler Pipeline (Single File)

```
snowc build <dir>
  -> find main.snow
  -> read source string
  -> snow_parser::parse(&source) -> Parse
  -> snow_typeck::check(&parse) -> TypeckResult
  -> snow_codegen::compile_to_binary(&parse, &typeck, ...) -> binary
```

Everything operates on a single `Parse` + single `TypeckResult`. The module system must extend this to handle N files.

### MIR Module Structure

`MirModule` already has the concept of a module -- it just holds everything from one file:

```rust
pub struct MirModule {
    pub functions: Vec<MirFunction>,
    pub structs: Vec<MirStructDef>,
    pub sum_types: Vec<MirSumTypeDef>,
    pub entry_function: Option<String>,
    pub service_dispatch: HashMap<String, (Vec<...>, Vec<...>)>,
}
```

### Existing Dependency Infrastructure

`snow-pkg` already handles **package-level** dependency resolution (external packages via git2, semver, TOML manifests). The module system is **within** a single package -- it resolves `.snow` files relative to the project root, not external packages.

## Recommended Stack Changes

### Change 1: ModuleGraph Data Structure (snow-common)

**What:** Add a `ModuleGraph` struct to `snow-common` that represents the dependency DAG of `.snow` files in a project.

**Why:** Every crate in the pipeline (parser, typeck, codegen) needs to know which modules exist and their dependency order. Putting this in `snow-common` makes it accessible everywhere.

**Key types:**

```rust
/// Unique identifier for a module within a compilation unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ModuleId(pub u32);

/// Metadata about a single module (source file).
pub struct ModuleInfo {
    /// Unique ID.
    pub id: ModuleId,
    /// Fully qualified module name (e.g., "Math.Vector").
    pub name: String,
    /// Filesystem path relative to project root.
    pub path: PathBuf,
    /// Module IDs this module depends on (via import/from-import).
    pub dependencies: Vec<ModuleId>,
    /// Whether this module contains the entry point (main function).
    pub is_entry: bool,
}

/// The dependency graph of all modules in a project.
pub struct ModuleGraph {
    /// All modules, indexed by ModuleId.
    pub modules: Vec<ModuleInfo>,
    /// Name-to-ID lookup.
    name_to_id: FxHashMap<String, ModuleId>,
    /// Topologically sorted module IDs (dependencies before dependents).
    pub topo_order: Vec<ModuleId>,
}
```

**Why in `snow-common`:** The `snow-common` crate already holds `TokenKind`, `Span`, error types -- shared primitives. `ModuleGraph` is a shared primitive that the driver, typeck, and codegen all need.

**Why `FxHashMap` (already available):** `rustc-hash = "2"` is already in workspace dependencies. No new crate needed for the name lookup table.

### Change 2: Topological Sort (Hand-Written, NOT petgraph)

**What:** Implement Kahn's algorithm (~40 lines of Rust) directly in `snow-common` or the compiler driver.

**Why NOT petgraph:** petgraph is a 10K+ line library optimized for complex graph algorithms (shortest paths, max flow, isomorphism). Snow's module graph is a simple DAG with typically 5-50 nodes. A hand-written toposort is:
- Simpler to understand (40 lines vs. learning petgraph's type system)
- Zero dependency cost
- Exactly the algorithm needed (Kahn's with cycle detection), nothing more
- Matches how rustc, Go, and most compilers implement this internally

**Implementation sketch:**

```rust
/// Topological sort using Kahn's algorithm.
/// Returns Err with cycle participants if the graph has cycles.
pub fn topological_sort(graph: &ModuleGraph) -> Result<Vec<ModuleId>, Vec<ModuleId>> {
    let n = graph.modules.len();
    let mut in_degree = vec![0u32; n];
    for module in &graph.modules {
        for &dep in &module.dependencies {
            in_degree[dep.0 as usize] += 1;
        }
    }

    let mut queue: VecDeque<ModuleId> = in_degree.iter()
        .enumerate()
        .filter(|(_, &d)| d == 0)
        .map(|(i, _)| ModuleId(i as u32))
        .collect();

    let mut order = Vec::with_capacity(n);
    while let Some(id) = queue.pop_front() {
        order.push(id);
        for &dep in &graph.modules[id.0 as usize].dependencies {
            in_degree[dep.0 as usize] -= 1;
            if in_degree[dep.0 as usize] == 0 {
                queue.push_back(dep);
            }
        }
    }

    if order.len() == n {
        Ok(order)
    } else {
        // Remaining nodes form cycles
        let in_cycle: Vec<ModuleId> = (0..n)
            .filter(|&i| in_degree[i] > 0)
            .map(|i| ModuleId(i as u32))
            .collect();
        Err(in_cycle)
    }
}
```

**Cycle detection produces actionable errors:** When `Err(cycle_nodes)` is returned, the driver can report which modules form cycles, which files are involved, and which import statements create the cycle -- using the existing ariadne diagnostic infrastructure.

### Change 3: File Discovery and Module Name Convention (Driver Layer)

**What:** Extend `snowc build` to discover all `.snow` files in the project directory and map them to module names.

**Why:** Currently `build()` reads only `main.snow`. Multi-file compilation requires discovering and ordering all source files.

**Module name convention (file-system based):**

```
project/
  main.snow          -> (entry point, no module name)
  math.snow          -> module "Math"
  math/
    vector.snow      -> module "Math.Vector"
    matrix.snow      -> module "Math.Matrix"
  utils.snow         -> module "Utils"
```

- File name -> module name: capitalize first letter, strip `.snow`
- Subdirectory -> dotted prefix: `math/vector.snow` -> `Math.Vector`
- `main.snow` is special: entry point, not importable as a module

**What already exists:** `collect_snow_files_recursive()` in `snowc/src/main.rs` already walks directories to find `.snow` files (used by the `fmt` command). This can be reused/adapted for module discovery.

**No new dependencies needed.** `std::fs::read_dir` and `PathBuf` manipulation are sufficient.

### Change 4: Per-Module Parse + Typecheck (Pipeline Extension)

**What:** Parse each `.snow` file independently into its own `Parse`, then typecheck in topological order with cross-module symbol visibility.

**Why:** Rowan parse trees are naturally per-file (each `GreenNode` is self-contained). The type checker operates on one `Parse` at a time. Multi-file compilation means running the pipeline N times in dependency order, accumulating exported symbols.

**New data structures:**

```rust
/// All parse results for a compilation unit.
pub struct CompilationUnit {
    pub graph: ModuleGraph,
    pub parses: Vec<(ModuleId, Parse)>,  // indexed by module
}

/// Cross-module symbol exports accumulated during type checking.
pub struct ModuleExports {
    /// Exported type schemes per module.
    pub symbols: FxHashMap<ModuleId, FxHashMap<String, Scheme>>,
    /// Exported type definitions per module.
    pub types: FxHashMap<ModuleId, TypeRegistry>,
    /// Exported trait registries per module.
    pub traits: FxHashMap<ModuleId, TraitRegistry>,
}
```

**Why `FxHashMap` (not a new interning library):**
- The existing TypeEnv uses `String` keys in `FxHashMap<String, Scheme>`
- Module names and symbol names are short strings (typically 3-30 chars)
- At 5-50 modules with 10-100 exports each, interning provides negligible benefit
- String interning (lasso, string-interner) adds complexity for managing interner lifetimes across parse/typeck/codegen phases
- If profiling later shows string comparison is a bottleneck (unlikely at this scale), interning can be added as an optimization without changing the architecture

### Change 5: Extended TypeEnv for Cross-Module Resolution (Typeck)

**What:** Extend `TypeEnv` to support loading exported symbols from dependency modules before type-checking a module.

**Why:** When type-checking module B that imports from module A, B's type environment needs A's exported symbols. Currently, only stdlib modules are loaded.

**Approach:** Before type-checking module B:
1. Look up B's dependencies from the `ModuleGraph`
2. For each dependency module A, load A's exported public symbols into a "module scope" in B's environment
3. Process `import` and `from...import` declarations to bring specific symbols into local scope

**Key extension to `TypeEnv`:**

```rust
pub struct TypeEnv {
    scopes: Vec<FxHashMap<String, Scheme>>,
    /// Module-scoped exports: qualified name -> scheme.
    /// E.g., "Math.Vector.add" -> Scheme { ... }
    module_exports: FxHashMap<String, FxHashMap<String, Scheme>>,
}
```

This keeps the existing scope-stack mechanism intact. Module imports add to `module_exports`, and qualified name resolution (`Math.Vector.add`) checks there. Unqualified access via `from Math.Vector import add` inserts directly into the scope stack (same as current stdlib behavior).

### Change 6: Visibility Enforcement (Typeck)

**What:** Enforce `pub` visibility modifiers during cross-module type checking.

**Why:** The `VISIBILITY` node is already parsed and present in the CST. Currently, everything is public by default. With modules, non-`pub` items should only be visible within their defining module.

**Rule:** When collecting exports from a module, only include items with a `VISIBILITY` child node (i.e., `pub fn`, `pub struct`, `pub type`). Items without `pub` are module-private.

**Decision on default visibility:** Module-private by default (items without `pub` are not exported). This matches Rust, Elixir, and most modern languages. It is the safer default -- accidental exposure is worse than accidental hiding.

**No new syntax needed.** `pub` is already a keyword, already parsed, and `VISIBILITY` nodes are already in the CST.

### Change 7: Multi-Module MIR Merging (Codegen)

**What:** After type-checking all modules, merge their MIR into a single `MirModule` for LLVM codegen.

**Why:** LLVM compiles one module at a time. Rather than implementing separate compilation + linking of multiple LLVM modules (which would require a linker-level symbol resolution pass), merge all MIR functions/structs/sum_types into one `MirModule` and compile to a single LLVM module.

**Why single-module compilation (not separate compilation + linking):**
- Snow projects are small-to-medium (the compiler itself is 70K lines)
- Single-module compilation avoids: cross-module LLVM symbol resolution, separate object file linking, duplicate definition conflicts
- LLVM's optimization passes work better on a single module (inlining, LTO-like optimization by default)
- If compilation time becomes an issue at scale, separate compilation can be added as an optimization later

**Approach:** After type-checking all modules in topo order:

```rust
fn merge_mir_modules(modules: Vec<(ModuleId, MirModule)>) -> MirModule {
    let mut merged = MirModule::new();
    for (id, module) in modules {
        // Prefix function names with module path to avoid collisions
        for mut func in module.functions {
            func.name = mangle_module_name(&module_name, &func.name);
            merged.functions.push(func);
        }
        // Same for structs, sum types
        merged.structs.extend(module.structs);
        merged.sum_types.extend(module.sum_types);
    }
    merged.entry_function = find_main(&merged);
    merged
}
```

**Name mangling:** Module-qualified names use double-underscore separator: `Math__Vector__add`. This avoids conflicts with dot (used in qualified access syntax) and single underscore (used in snake_case names).

### Change 8: Import Statement Resolution (Driver + Typeck)

**What:** Resolve `import Math.Vector` and `from Math.Vector import add` to actual module files and their exported symbols.

**Why:** Currently, import resolution is hardcoded to stdlib modules. User-defined modules need filesystem-based resolution.

**Resolution algorithm:**

```
import Math.Vector
  1. Convert path to filesystem: "math/vector.snow" or "math_vector.snow"
  2. Look up in ModuleGraph by module name "Math.Vector"
  3. If found, record dependency edge: current_module -> Math.Vector
  4. Make Math.Vector's exports available for qualified access (Math.Vector.add)

from Math.Vector import add, cross
  1. Same resolution as above
  2. Additionally: inject "add" and "cross" into current module's local scope
  3. Verify each imported name exists in Math.Vector's public exports
  4. Error if name not found or not public
```

**Error cases (all reportable via existing ariadne diagnostics):**
- Module not found: "cannot find module 'Math.Vector' -- no file at math/vector.snow"
- Name not exported: "'add' is not a public export of module 'Math.Vector'"
- Circular import: "circular dependency detected: A imports B imports A"

### Change 9: Diagnostic Enhancement (Typeck + Driver)

**What:** Add new error codes for module-related diagnostics.

**Why:** Module errors need clear, actionable messages. The existing ariadne-based diagnostic system handles this -- just add new `TypeError` variants.

| Error Code | Message Pattern | When |
|-----------|----------------|------|
| `M0001` | Module not found | `import NonExistent` |
| `M0002` | Circular dependency | Import cycle detected |
| `M0003` | Name not exported | `from X import private_fn` |
| `M0004` | Visibility violation | Accessing non-pub item from another module |
| `M0005` | Duplicate module | Two files map to same module name |
| `M0006` | Self-import | Module imports itself |

**No new diagnostic library needed.** Ariadne 0.6 already supports multi-span labels, error codes, and fix suggestions.

## Alternatives Considered

| Decision | Recommended | Alternative | Why Not |
|----------|-------------|-------------|---------|
| Dependency graph | Hand-written Kahn's (~40 LOC) | petgraph 0.8.2 | Overkill for 5-50 node DAG; adds 10K+ lines of dependency for one function call |
| Symbol storage | `FxHashMap<String, Scheme>` | String interning (lasso 0.7) | Negligible perf gain at this scale; adds lifetime complexity across compiler phases |
| Module identity | `ModuleId(u32)` newtype | String-based module names everywhere | IDs are O(1) comparison, smaller in memory, prevent typo bugs |
| Multi-file codegen | Merge MIR into single LLVM module | Separate compilation + LLVM linking | Single module gets better optimization; separate compilation adds linker complexity |
| Visibility default | Private (require `pub` to export) | Public by default | Private-by-default is safer; matches Rust/Elixir conventions |
| Module naming | File-system based (math/vector.snow -> Math.Vector) | Explicit `module Math.Vector` declarations in each file | FS-based is simpler, less boilerplate, matches Go and many modern languages |
| Topological sort | Kahn's algorithm (BFS-based) | DFS-based toposort | Kahn's naturally detects cycles (remaining nodes after sort); DFS needs separate cycle detection |
| Cross-module types | Extend existing TypeRegistry | New shared type database | TypeRegistry already holds struct/sum/alias defs; extending it is simpler than replacing it |

## What NOT to Add

| Dependency / Feature | Why Not |
|---------------------|---------|
| `petgraph` crate | Module graph is a simple DAG with <100 nodes. Kahn's algorithm is 40 lines. petgraph's type system complexity (generics over graph type, edge type, direction) is not worth it for one toposort call. |
| `lasso` or `string-interner` | String interning optimizes repeated string comparisons. Snow modules have ~50-500 unique symbol names. FxHashMap already provides O(1) lookup. Interning adds lifetime management complexity that propagates through the entire compiler. Profile first, optimize later. |
| `salsa` (incremental computation) | Incremental compilation is a separate concern. The initial module system should work correctly in batch mode. Salsa can be evaluated for the LSP/IDE experience in a future milestone. |
| `dashmap` (concurrent HashMap) | Module type-checking is sequential (topological order). No parallelism needed at this stage. |
| Separate LLVM modules per source file | Would require: LLVM module linking, cross-module symbol resolution, handling of duplicate type definitions. All for negligible compilation time benefit at Snow's scale. |
| Generic module/package system | The module system handles files within one project. Cross-project dependencies are already handled by `snow-pkg`. Do not conflate the two. |
| Module-level type parameters | `module Math<T>` is not a common pattern and adds enormous complexity. Modules are namespaces, not generic types. |

## Technology Versions (No Changes)

| Technology | Current Version | Required Changes | Version Impact |
|------------|----------------|-----------------|----------------|
| Rust std | stable 2024 edition | `std::fs`, `std::path`, `std::collections::VecDeque` for toposort | None |
| rustc-hash | 2 | `FxHashMap` for module name lookup, symbol tables | Already in workspace deps, no change |
| Rowan | 0.16 | Per-file `Parse` instances (already supported) | No change |
| Ariadne | 0.6 | New diagnostic messages with existing API | No change |
| Inkwell | 0.8.0 (llvm21-1) | Name-mangled functions in single LLVM module | No change |
| serde | 1 (workspace) | Potentially serialize ModuleGraph for caching | Already in workspace deps, no change |
| snow-common | internal | Add `ModuleId`, `ModuleInfo`, `ModuleGraph`, `topological_sort` | Internal additions |
| snow-parser | internal | No parser changes -- all syntax already supported | No change |
| snow-typeck | internal | Extend `TypeEnv`, add visibility enforcement, cross-module symbol loading | Internal extensions |
| snow-codegen | internal | MIR merging, module-qualified name mangling | Internal extensions |
| snowc | internal | Multi-file build pipeline, file discovery, module graph construction | Internal extensions |

**No dependency additions. No version bumps. No new crates.**

## Crate-by-Crate Change Summary

| Crate | Changes | Estimated Lines |
|-------|---------|----------------|
| `snow-common` | `ModuleId`, `ModuleInfo`, `ModuleGraph`, `topological_sort()` | ~150 |
| `snow-parser` | No changes needed (all syntax already parsed) | 0 |
| `snow-typeck` | Extend `TypeEnv` with module exports, visibility enforcement, cross-module symbol loading, new `TypeError` variants | ~400 |
| `snow-codegen` (MIR) | Module-qualified name mangling in lowering | ~100 |
| `snow-codegen` (codegen) | Handle mangled names, merge MIR modules | ~100 |
| `snowc` (driver) | File discovery, module graph construction, multi-file build pipeline, import resolution | ~300 |
| `snow-fmt` | No changes (module/import formatting already works) | 0 |
| `snow-lsp` | Multi-file project awareness (module-aware diagnostics) | ~100 |
| Tests | Module resolution, visibility, circular deps, cross-module typeck, e2e multi-file | ~500 |
| **Total** | | **~1,650** |

## Integration Points with Existing Architecture

### Parser -> ModuleGraph (no coupling needed)

Each `.snow` file is parsed independently. The `ModuleGraph` is built by the driver by scanning import declarations in each `Parse` result. The parser does not need to know about the module graph.

### ModuleGraph -> Typeck (dependency ordering)

The typeck phase processes modules in topological order. Before type-checking module B, all of B's dependencies have been type-checked, and their exports are available. This is a simple loop:

```rust
for &module_id in &graph.topo_order {
    let parse = &parses[module_id];
    let imports = resolve_imports(module_id, &graph, &exports);
    let typeck = check_with_imports(parse, &imports);
    exports.insert(module_id, collect_public_exports(&typeck));
}
```

### Typeck -> MIR -> Codegen (merge and mangle)

After all modules are type-checked, MIR lowering happens per-module, then MIR modules are merged. Name mangling prevents collisions. The single merged `MirModule` feeds into the existing codegen pipeline unchanged.

### Stdlib Modules (backward compatible)

The existing `stdlib_modules()` hardcoded map continues to work. It provides built-in modules (String, IO, List, etc.) that are always available. User-defined modules are resolved through the `ModuleGraph`. Resolution order: user modules first, then stdlib fallback. This means a user module named "String" would shadow the stdlib String module -- which is acceptable and expected behavior.

## Sources

### Primary (HIGH confidence)
- Direct codebase analysis: `snowc/src/main.rs` build pipeline (lines 194-262)
- `snow-typeck/src/infer.rs` stdlib module resolution (lines 205-491)
- `snow-typeck/src/env.rs` TypeEnv scope stack (full file, 141 lines)
- `snow-parser/src/syntax_kind.rs` existing MODULE_DEF, IMPORT_DECL, FROM_IMPORT_DECL, VISIBILITY nodes
- `snow-parser/src/ast/item.rs` ModuleDef, ImportDecl, FromImportDecl AST accessors
- `snow-codegen/src/mir/lower.rs` line 648: modules/imports explicitly skipped
- `snow-codegen/src/mir/mod.rs` MirModule struct definition
- `snow-common/Cargo.toml` existing dependencies (serde only)

### Secondary (MEDIUM-HIGH confidence)
- [petgraph toposort API](https://docs.rs/petgraph/latest/petgraph/algo/fn.toposort.html) -- evaluated and rejected for this use case
- [petgraph crate](https://crates.io/crates/petgraph) -- version 0.8.2, evaluated
- [rustc Name Resolution](https://rustc-dev-guide.rust-lang.org/name-resolution.html) -- rib-based scope resolution pattern
- [Rust Compiler Overview](https://rustc-dev-guide.rust-lang.org/overview.html) -- compilation pipeline architecture
- [Rowan GitHub](https://github.com/rust-analyzer/rowan) -- per-file GreenNode independence confirmed

### Tertiary (MEDIUM confidence)
- [String interners in Rust](https://dev.to/cad97/string-interners-in-rust-797) -- evaluated lasso, string-interner, internment; rejected for this scale
- [topo_sort crate](https://docs.rs/topo_sort) -- evaluated, hand-written preferred for simplicity
- [Build a Compiler Symbol Table](https://marcauberer.medium.com/build-a-compiler-symbol-table-2d4582234112) -- general symbol table patterns

---
*Stack research for: Snow compiler module system features*
*Researched: 2026-02-09*
