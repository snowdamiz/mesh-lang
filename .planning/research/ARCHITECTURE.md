# Architecture: Module System Integration

**Domain:** Extending the Snow compiler from single-file to multi-file compilation with module resolution, cross-file type checking, and unified code generation
**Researched:** 2026-02-09
**Confidence:** HIGH (based on direct analysis of all 11 compiler crates, every public API surface, and data structure)

---

## Current Pipeline (Single-File)

```
snowc build <dir>
  |
  v
[1] Find main.snow (single file)
  |
  v
[2] Read source string
  |
  v
[3] snow_parser::parse(&source) -> Parse { green: GreenNode, errors: Vec<ParseError> }
  |
  v
[4] snow_typeck::check(&parse) -> TypeckResult {
      types: FxHashMap<TextRange, Ty>,
      errors: Vec<TypeError>,
      type_registry: TypeRegistry { struct_defs, sum_type_defs, type_aliases },
      trait_registry: TraitRegistry,
      default_method_bodies: FxHashMap<(String,String), TextRange>,
    }
  |
  v
[5] snow_codegen::lower_to_mir_module(&parse, &typeck) -> MirModule {
      functions: Vec<MirFunction>,
      structs: Vec<MirStructDef>,
      sum_types: Vec<MirSumTypeDef>,
      entry_function: Option<String>,
      service_dispatch: HashMap<...>,
    }
    + monomorphize(&mut module)
  |
  v
[6] CodeGen::new(&context, "snow_module", opt_level, target)
    CodeGen::compile(&mir)
    CodeGen::emit_object(output)
  |
  v
[7] link::link(&obj_path, output, rt_lib_path)   // cc -o output obj.o -lsnow_rt
```

Each stage operates on data from exactly one file. No cross-file awareness.

### Key Data Structures and Their Scope

| Structure | Crate | Current Scope | Module System Impact |
|-----------|-------|---------------|---------------------|
| `Parse` | snow-parser | Single file | Need one per `.snow` file |
| `TypeckResult` | snow-typeck | Single file | Must be composed across files |
| `TypeEnv` | snow-typeck | Single file scope stack | Must be pre-seeded with imported symbols |
| `TypeRegistry` | snow-typeck | Single file | Must accept imported type definitions |
| `TraitRegistry` | snow-typeck | Single file | Must accept imported trait defs + impls |
| `MirModule` | snow-codegen | Single compilation unit | Must be merged across files |
| `Lowerer` | snow-codegen | Single `(Parse, TypeckResult)` | Gets name-remap table for imports |
| `CodeGen` | snow-codegen | Single LLVM module | Receives merged `MirModule` (unchanged) |

### Existing Module Syntax Support (Already Implemented)

The parser already handles all needed syntax, producing correct CST nodes:

```
SyntaxKind::IMPORT_DECL       -> ImportDecl { module_path: Path }
SyntaxKind::FROM_IMPORT_DECL  -> FromImportDecl { module_path: Path, import_list: ImportList }
SyntaxKind::MODULE_DEF        -> ModuleDef { name: Name, items: impl Iterator<Item> }
SyntaxKind::PATH              -> dot-separated segments (Foo.Bar.Baz)
SyntaxKind::VISIBILITY        -> pub keyword
SyntaxKind::PUB_KW            -> pub token
```

The type checker has hardcoded stdlib modules (infer.rs lines 210-490):

```rust
// Currently: ImportDecl is skipped, FromImportDecl checks stdlib_modules() only
Item::ImportDecl(_) => None,
Item::FromImportDecl(ref from_import) => {
    let modules = stdlib_modules();  // Hardcoded HashMap<String, HashMap<String, Scheme>>
    // Looks up module, injects imported names into TypeEnv
}
```

The MIR lowerer explicitly skips all module/import items:

```rust
Item::ModuleDef(_) | Item::ImportDecl(_) | Item::FromImportDecl(_) => {
    // Skip -- module/import handling is not needed for single-file compilation.
}
```

**Bottom line:** The parser is ready. The type checker and MIR lowerer need extension, not rewriting.

---

## Target Pipeline (Multi-File)

```
snowc build <dir>
  |
  v
[1] snow-resolve: discover all .snow files under project_root
  |
  v
[2] Map file paths to module names (math/vector.snow -> Math.Vector)
  |
  v
[3] Parse ALL files independently: Vec<(ModulePath, Parse, source)>
  |
  v
[4] Scan import declarations in each Parse -> dependency edges
  |
  v
[5] Build module graph + topological sort (Kahn's algorithm)
  |   |
  |   +-> Error: circular dependency detected (with cycle path)
  |
  v
[6] Type-check in topological order:
     for each module in topo_order:
       a. Build ImportContext from already-checked dependencies
       b. snow_typeck::check_with_imports(&parse, &import_ctx) -> TypeckResult
       c. Collect public exports (pub fn, pub struct, pub type, pub interface)
       d. Store exports for downstream modules
  |
  v
[7] Lower each module to MIR with name mangling: Vec<MirModule>
  |
  v
[8] Merge MIR modules into single MirModule + monomorphize
  |
  v
[9] Compile merged MirModule -> single .o file -> link -> binary
```

### Key Architectural Decisions

**Decision 1: Single LLVM module, not separate compilation.**

Use a single LLVM module for the merged MIR. Do not compile each .snow file to a separate .o file.

Rationale:
- Monomorphization is whole-program. The existing `monomorphize()` pass needs to see all generic instantiation sites across all files. Separate compilation would require a fundamentally different monomorphization strategy.
- The `CodeGen` struct maintains caches (`struct_types`, `sum_type_layouts`, `functions`) that assume a single module. Making it work across multiple modules would require significant refactoring for no immediate benefit.
- Single LLVM module enables cross-module inlining, dead code elimination, and full optimization without LTO.
- Snow projects are small enough that compilation time is not a concern at this scale.

**Decision 2: New `snow-resolve` crate for module resolution.**

Module resolution cannot live in any existing crate:
- `snow-parser` is file-local
- `snow-typeck` operates on single `Parse`
- `snow-codegen` operates on single `MirModule`
- `snowc` is the CLI driver -- LSP also needs module resolution

A dedicated `snow-resolve` crate provides clean separation and is usable by both `snowc` and `snow-lsp`.

**Decision 3: Pre-seeded TypeEnv, not demand-driven resolution.**

Type-check files in dependency order. Before checking file B (which imports A), extract A's public symbols and inject them into B's TypeEnv before inference begins. The existing HM inference logic works unchanged -- it just finds more names in scope.

**Decision 4: Parse phase is embarrassingly parallel.** Each file is parsed independently. Rowan's `GreenNode` per file is self-contained. Future optimization: parse in parallel with rayon.

**Decision 5: Type-check phase is inherently sequential** within a dependency chain, but independent modules at the same topo level could theoretically be parallelized. Not needed initially.

---

## Component Boundaries

```
                          +---------------+
                          |    snowc      |  CLI driver: orchestrates phases
                          +-------+-------+
                                  |
                    +-------------+-------------+
                    |                           |
            +-------v-------+           +-------v-------+
            | snow-resolve  |           | snow-lsp      |
            | (NEW)         |           | (uses resolve)|
            +-------+-------+           +---------------+
                    |
         +----------+----------+
         |                     |
  +------v------+       +------v------+
  | snow-parser |       | snow-typeck |
  | (per-file)  |       | (per-file   |
  |             |       |  + imports) |
  +------+------+       +------+------+
         |                     |
         +----------+----------+
                    |
            +-------v-------+
            | snow-codegen  |
            | (merged MIR   |
            |  single LLVM) |
            +-------+-------+
                    |
            +-------v-------+
            |   link.rs     |
            | (cc linker)   |
            +---------------+
```

| Component | Responsibility | Communicates With |
|-----------|---------------|-------------------|
| `snowc` (driver) | Orchestrates multi-file pipeline, CLI interface | snow-resolve, snow-parser, snow-typeck, snow-codegen |
| `snow-resolve` (NEW) | File discovery, module path mapping, dependency graph, topological sort, cycle detection | snow-parser (Parse), snow-common (shared types) |
| `snow-parser` | Per-file parsing (unchanged) | snow-lexer, snow-common |
| `snow-typeck` | Per-module type checking with imported symbols, visibility enforcement, export collection | snow-parser (Parse), snow-common |
| `snow-codegen` (MIR) | Per-module MIR lowering with name mangling + MIR merge | snow-typeck (TypeckResult), snow-parser (Parse) |
| `snow-codegen` (LLVM) | Single merged MirModule -> LLVM IR -> .o file (unchanged) | MirModule |

---

## Data Flow

### Phase 1: Discovery and Graph Building

```
Filesystem                 snow-resolve                 Output
-----------                ------------                 ------
project/                   resolve_project(dir)
  main.snow       ------>  1. Discover .snow files       ModuleGraph {
  math/                    2. Map to module paths          modules: [
    vector.snow            3. Parse all files                ResolvedModule { path: "Math.Vector", ... },
  utils.snow               4. Extract import edges           ResolvedModule { path: "Utils", ... },
                           5. Topological sort               ResolvedModule { path: "main", ... },
                           6. Detect cycles                ],
                                                          topo_order: [0, 1, 2],  // leaves first
                                                          entry: 2,
                                                        }
```

### Phase 2: Sequential Type Checking

```
topo_order = [Math.Vector, Utils, main]

Step 1: Check Math.Vector -- no dependencies
  TypeEnv = builtins + stdlib_modules
  -> TypeckResult for Math.Vector
  -> Exports = { pub fn add: Fun(Int, Int) -> Int, pub struct Vec3: ... }

Step 2: Check Utils -- no dependencies on user modules
  TypeEnv = builtins + stdlib_modules
  -> TypeckResult for Utils
  -> Exports = { pub fn clamp: Fun(Int, Int, Int) -> Int }

Step 3: Check main -- depends on Math.Vector, Utils
  TypeEnv = builtins + stdlib_modules
           + Math.Vector exports (for qualified Math.Vector.add or selective import)
           + Utils exports (for qualified Utils.clamp or selective import)
  -> TypeckResult for main
  -> No exports (entry point)
```

### Phase 3: MIR Merge and Codegen

```
MIR lowering per module (with name mangling):
  MirModule for Math.Vector:
    functions: [Math__Vector__add(...)]
    structs: [Math__Vector__Vec3(...)]

  MirModule for Utils:
    functions: [Utils__clamp(...)]

  MirModule for main:
    functions: [main(...)]
    // References to add() are remapped to Math__Vector__add

Merge into single MirModule:
  functions: [Math__Vector__add, Utils__clamp, main]
  structs: [Math__Vector__Vec3]
  entry_function: Some("main")

monomorphize(&mut merged)
CodeGen::compile(&merged)  // single LLVM module -> single .o -> link -> binary
```

---

## Detailed Integration Points Per Crate

### snow-common: No Changes

No modifications needed. The shared types (`Token`, `Span`, `Error`) are already file-agnostic. Module-specific types go in `snow-resolve`, not `snow-common`, to keep the leaf crate minimal.

### snow-lexer: No Changes

Tokenization is inherently file-local. No module awareness needed.

### snow-parser: Minimal Changes (0-2 small items)

The parser is already ready. All needed syntax nodes exist.

**Verify:** The `from ... import` syntax currently parses `from` as an IDENT (not a keyword), matching via `p.at(SyntaxKind::IDENT) && p.current_text() == "from"` in `parse_item_or_stmt`. This works but should be verified for deeply nested paths like `from Foo.Bar.Baz import x, y, z`.

**Verify:** The `pub` keyword in `parse_optional_visibility` works for `fn`, `struct`, `type`, `interface`, `supervisor`. Confirm it is called in all definition parse functions where visibility applies.

**Estimated scope:** 0-2 small verifications/fixes. ~0-20 lines changed.

### snow-resolve: New Crate (~400-600 lines)

```toml
[package]
name = "snow-resolve"
version = "0.1.0"
edition = "2021"

[dependencies]
snow-common = { path = "../snow-common" }
snow-parser = { path = "../snow-parser" }
rustc-hash = { workspace = true }
```

**Core types:**

```rust
/// A dot-separated module path: ["Math", "Vector"] represents Math.Vector
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct ModulePath(pub Vec<String>);

/// A resolved module graph for a Snow project.
pub struct ModuleGraph {
    /// All modules, indexed by position.
    pub modules: Vec<ResolvedModule>,
    /// Map from module path to index in `modules`.
    pub path_to_index: FxHashMap<ModulePath, usize>,
    /// Topological order (dependency-first).
    pub topo_order: Vec<usize>,
    /// The entry module index (main.snow).
    pub entry: usize,
}

/// A single resolved module.
pub struct ResolvedModule {
    /// Canonical module path (e.g., Math.Vector).
    pub path: ModulePath,
    /// File system path to the .snow file.
    pub file_path: PathBuf,
    /// Parsed CST.
    pub parse: Parse,
    /// Source code (for diagnostics).
    pub source: String,
    /// Indices of modules this one imports.
    pub dependencies: Vec<usize>,
    /// Import declarations (for type checker to process).
    pub imports: Vec<ImportInfo>,
}

/// Information about a single import statement.
pub enum ImportInfo {
    /// `import Math.Vector` -- qualified access only
    Qualified(ModulePath),
    /// `from Math.Vector import add, scale` -- selective import
    Selective { module: ModulePath, names: Vec<String> },
}
```

**Core functions:**

```rust
/// Discover all .snow files, parse them, build dependency graph, topological sort.
pub fn resolve_project(project_root: &Path) -> Result<ModuleGraph, Vec<ResolveError>>;

/// Map file path to module path.
/// math/vector.snow -> ModulePath(["Math", "Vector"])
/// main.snow -> ModulePath(["main"])
fn file_to_module_path(root: &Path, file: &Path) -> ModulePath;

/// Extract import paths from a parsed file.
fn extract_imports(parse: &Parse) -> Vec<ImportInfo>;

/// Topological sort with cycle detection (Kahn's algorithm).
fn topological_sort(modules: &[ResolvedModule]) -> Result<Vec<usize>, Vec<ModulePath>>;
```

**Module path mapping rules:**
- `main.snow` -> `ModulePath(["main"])` (special: entry point, not importable)
- `math.snow` -> `ModulePath(["Math"])` (file name capitalized)
- `math/vector.snow` -> `ModulePath(["Math", "Vector"])` (directory = parent module)
- `utils/string_helpers.snow` -> `ModulePath(["Utils", "StringHelpers"])` (snake_case to PascalCase)

### snow-typeck: Moderate Changes (~200-300 lines)

**Change 1: New `check_with_imports` entry point (~60-80 lines).**

```rust
/// Type-check with pre-resolved imports from other modules.
pub fn check_with_imports(
    parse: &Parse,
    import_ctx: &ImportContext,
) -> TypeckResult;

/// Context built by the driver from already-checked modules.
pub struct ImportContext {
    /// Module name -> exported symbols (for qualified and selective access)
    pub module_symbols: FxHashMap<String, FxHashMap<String, Scheme>>,
    /// Imported struct definitions
    pub struct_defs: FxHashMap<String, StructDefInfo>,
    /// Imported sum type definitions
    pub sum_type_defs: FxHashMap<String, SumTypeDefInfo>,
    /// Imported type aliases
    pub type_aliases: FxHashMap<String, TypeAliasInfo>,
    /// Imported trait definitions
    pub trait_defs: Vec<TraitDef>,
    /// Imported trait implementations
    pub trait_impls: Vec<TraitImplDef>,
}
```

This wraps the existing `infer()` function, pre-seeding `TypeEnv`, `TypeRegistry`, and `TraitRegistry` with imported data before inference begins.

**Change 2: Modify `infer_item` for import handling (~30-50 lines).**

```rust
// ImportDecl: register module for qualified access
Item::ImportDecl(ref import) => {
    if let Some(path) = import.module_path() {
        let module_key = path.segments().join(".");
        // Check user modules first, then stdlib_modules() fallback
        if let Some(mod_exports) = self.import_ctx.module_symbols.get(&module_key) {
            self.qualified_modules.insert(module_key, mod_exports.clone());
        } else {
            let stdlib = stdlib_modules();
            if let Some(mod_exports) = stdlib.get(&module_key) {
                self.qualified_modules.insert(module_key, mod_exports.clone());
            }
        }
    }
    None
}

// FromImportDecl: inject specific names into TypeEnv
Item::FromImportDecl(ref from_import) => {
    // Same logic but check import_ctx first, then stdlib_modules()
    // For each imported name, insert into env
}
```

**Change 3: Extend qualified name resolution (~20-30 lines).**

`infer_field_access` already handles `Module.function` via `stdlib_modules()`. Extend it to also check the `qualified_modules` map populated from import declarations.

**Change 4: Export collection function (~60-80 lines).**

New function to extract public symbols from a `TypeckResult`:

```rust
pub fn collect_exports(
    parse: &Parse,
    typeck: &TypeckResult,
) -> ExportedSymbols;

pub struct ExportedSymbols {
    pub functions: FxHashMap<String, Scheme>,
    pub struct_defs: FxHashMap<String, StructDefInfo>,
    pub sum_type_defs: FxHashMap<String, SumTypeDefInfo>,
    pub type_aliases: FxHashMap<String, TypeAliasInfo>,
    pub trait_defs: Vec<TraitDef>,
    pub trait_impls: Vec<TraitImplDef>,
}
```

Scans top-level items, checks for `pub` visibility (via `fn_def.visibility().is_some()`), and extracts type information from the `TypeckResult`.

**Change 5: Visibility enforcement (~10 lines).**

Only items with `pub` keyword are exported. The AST already provides `visibility() -> Option<Visibility>` on `FnDef`, `StructDef`, `SumTypeDef`, `InterfaceDef`, `SupervisorDef`, `ModuleDef`.

**Backward compatibility:** The existing `check(&parse) -> TypeckResult` continues to work unchanged. It is equivalent to `check_with_imports(&parse, &ImportContext::empty())`. Single-file compilation is unaffected.

**No changes to:** Core unification engine, occurs check, level-based generalization, Algorithm J inference, `InferCtx`, `Ty`, `Scheme`, `TyCon`, `TyVar`.

### snow-codegen: Moderate Changes (~200-300 lines)

**Change 1: New `lower_project_to_mir` function (~100-150 lines).**

```rust
/// Lower all modules to MIR and merge into a single MirModule.
pub fn lower_project_to_mir(
    modules: &[(ModulePath, &Parse, &TypeckResult)],
    entry_module: usize,
) -> Result<MirModule, String> {
    let mut all_functions = Vec::new();
    let mut all_structs = Vec::new();
    let mut all_sum_types = Vec::new();
    let mut entry_function = None;

    for (i, (path, parse, typeck)) in modules.iter().enumerate() {
        let name_prefix = module_prefix(path);
        let name_remap = build_name_remap(parse, &name_prefix, ...);

        let per_file_mir = lower_module(parse, typeck, &name_prefix, &name_remap)?;

        all_functions.extend(per_file_mir.functions);
        all_structs.extend(per_file_mir.structs);
        all_sum_types.extend(per_file_mir.sum_types);

        if i == entry_module {
            entry_function = per_file_mir.entry_function;
        }
    }

    // Deduplicate structs/sum types (imported types may be registered in multiple files)
    dedup_structs(&mut all_structs);
    dedup_sum_types(&mut all_sum_types);

    let mut merged = MirModule {
        functions: all_functions,
        structs: all_structs,
        sum_types: all_sum_types,
        entry_function,
        service_dispatch: HashMap::new(),
    };
    monomorphize(&mut merged);
    Ok(merged)
}
```

**Change 2: Name mangling with module prefix (~40-60 lines).**

```
Module Math.Vector, function add     -> MIR name: "Math__Vector__add"
Module Utils, function clamp         -> MIR name: "Utils__clamp"
Module main, function main           -> MIR name: "main" (no prefix for entry module)
Module main, function helper         -> MIR name: "helper" (no prefix for entry module)

Cross-module call from main:
  from Math.Vector import add
  add(1, 2)
  -> MirExpr::Call { func: Var("Math__Vector__add"), ... }
```

The double underscore convention is consistent with existing trait mangling: `Trait__Method__Type`.

**Change 3: Name remap table in Lowerer (~20-30 lines).**

```rust
struct Lowerer<'a> {
    // ... existing fields ...
    /// Maps local names to mangled MIR names (from imports).
    name_remap: HashMap<String, String>,
    /// This module's name prefix for its own definitions.
    module_prefix: String,
}
```

When lowering a `NameRef`, check `name_remap` first. If found, use the mangled name. Otherwise, use the local name (possibly with module prefix).

**Change 4: Entry function detection (~5-10 lines).**

Only `main()` from the entry module (main.snow) sets `entry_function`. If another module defines `main`, it gets module-prefixed and is not the entry point.

**Change 5: Struct/sum type deduplication (~20-30 lines).**

When merging MirModules, struct/sum types imported across multiple files may appear multiple times. Deduplicate by name.

**No changes to:** `CodeGen`, LLVM IR generation, optimization passes, `link.rs`, `pattern/` decision tree compilation, `codegen/expr.rs`, `codegen/intrinsics.rs`, `codegen/types.rs`.

### snowc: Moderate Changes (~100-130 lines)

Replace the `build()` function body with the multi-file pipeline:

```rust
fn build(dir: &Path, ...) -> Result<(), String> {
    // Phase 1: Resolve module graph
    let graph = snow_resolve::resolve_project(dir)
        .map_err(|errors| format_resolve_errors(&errors))?;

    // Phase 2: Type-check in dependency order
    let mut checked: Vec<(&Parse, TypeckResult, ExportedSymbols)> = Vec::new();
    let mut has_errors = false;

    for &idx in &graph.topo_order {
        let module = &graph.modules[idx];
        let import_ctx = build_import_context(&graph, &checked, &module.dependencies);
        let typeck = snow_typeck::check_with_imports(&module.parse, &import_ctx);

        // Report diagnostics for this file
        has_errors |= report_diagnostics(
            &module.source, &module.file_path, &module.parse, &typeck, diag_opts
        );

        let exports = snow_typeck::collect_exports(&module.parse, &typeck);
        checked.push((&module.parse, typeck, exports));
    }

    if has_errors {
        return Err("Compilation failed due to errors above.".to_string());
    }

    // Phase 3: Lower all modules to merged MIR
    let module_data: Vec<_> = graph.topo_order.iter().map(|&idx| {
        let module = &graph.modules[idx];
        let (parse, typeck, _) = &checked[idx];
        (module.path.clone(), *parse, typeck)
    }).collect();

    let mir = snow_codegen::lower_project_to_mir(&module_data, graph.entry)?;

    // Phase 4: Codegen + link (unchanged)
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "snow_module", opt_level, target)?;
    codegen.compile(&mir)?;
    // ... emit object, link
}
```

**Backward compatibility:** A project with only `main.snow` produces a ModuleGraph with one node, zero imports. The pipeline reduces to the existing single-file path.

### Unchanged Components

| Component | Why Unchanged |
|-----------|--------------|
| `snow-lexer` | Tokenization is file-local |
| `snow-common` | Shared types are module-agnostic |
| `snow-rt` | Runtime has no module boundaries |
| `snow-fmt` | Formatting is file-local |
| `snow-pkg` | Package management is orthogonal |
| `snow-repl` | Operates in single-file mode |

### Deferred: snow-lsp

The LSP currently analyzes single files. Multi-file support needs:
- Use `snow-resolve` to build module graph from workspace
- Cache per-file parse and typeck results
- On file change, re-check changed file and dependents
- Go-to-definition across files

**Defer to a follow-up milestone.** The module system should work in the CLI first.

---

## Patterns to Follow

### Pattern 1: Accumulator Pattern for Cross-Module Exports

**What:** Process modules in topo order, accumulating exports into a shared map. Each module reads from already-processed modules and writes its own exports after type checking.

**When:** During the sequential type-check phase.

**Why:** Simple, deterministic, and guaranteed to process dependencies before dependents (by construction from topo sort). This is how every batch-mode compiler handles cross-module dependencies.

```rust
let mut all_exports: Vec<ExportedSymbols> = Vec::new();

for &idx in &graph.topo_order {
    let module = &graph.modules[idx];

    // Build import context from already-checked dependencies
    let mut import_ctx = ImportContext::new();
    for &dep_idx in &module.dependencies {
        import_ctx.add_module(&graph.modules[dep_idx].path, &all_exports[dep_idx]);
    }
    import_ctx.add_stdlib();  // Always include stdlib

    let typeck = snow_typeck::check_with_imports(&module.parse, &import_ctx);
    let exports = snow_typeck::collect_exports(&module.parse, &typeck);
    all_exports.push(exports);
}
```

### Pattern 2: Name Mangling with Module Prefix

**What:** Prefix all MIR function/type names with their module path using double underscores.

**When:** During MIR lowering.

**Example:**
```
Source: Math.Vector.add     -> MIR: Math__Vector__add
Source: Math.solve          -> MIR: Math__solve
Source: main                -> MIR: main (no prefix for entry module)
Trait:  Display__show__Int  -> MIR: Display__show__Int (existing convention preserved)
```

**Why:** Avoids name collisions between modules. Double underscore chosen because:
- Single underscore is used in snake_case (`my_function`)
- Dot is used in qualified access syntax (`Math.add`)
- Double underscore is unambiguous and already the convention for trait mangling

### Pattern 3: Extend Existing Structures, Don't Replace

**What:** Add fields/parameters to existing types rather than creating parallel data structures.

**When:** Throughout the architecture.

**Why:** The existing structures are deeply integrated across 70K+ lines. Replacing them would require touching every call site. Adding optional parameters (e.g., `ImportContext` for `check_with_imports`) is backward-compatible.

```rust
// GOOD: New entry point, existing one unchanged
pub fn check(parse: &Parse) -> TypeckResult { ... }  // existing
pub fn check_with_imports(parse: &Parse, ctx: &ImportContext) -> TypeckResult { ... }  // new

// BAD: Changing existing signature
pub fn check(parse: &Parse, ctx: Option<&ImportContext>) -> TypeckResult { ... }  // breaks all callers
```

---

## Anti-Patterns to Avoid

### Anti-Pattern 1: Global Mutable Symbol Table

**What:** A single global `HashMap<String, Symbol>` shared across all modules during type checking.

**Why bad:** Race conditions if ever parallelized; unclear ownership; mutations from one module's type checking can corrupt another's; impossible to test modules in isolation.

**Instead:** Per-module TypeEnv populated from read-only exports of already-checked modules.

### Anti-Pattern 2: Separate LLVM Modules Per File

**What:** Compile each .snow file to a separate .o object file, then link them.

**Why bad for Snow:** Requires solving cross-module symbol resolution, extern declarations, duplicate type definition handling, and loses cross-module optimization. Monomorphization needs whole-program visibility. All this complexity for zero practical benefit at Snow's project scale.

**Instead:** Merge MIR into single module. One LLVM compilation. Maximum optimization.

### Anti-Pattern 3: Implicit Transitive Imports

**What:** If A imports B and B imports C, A can use C's exports without importing C directly.

**Why bad:** Creates hidden dependencies. Removing B's import of C breaks A in a non-obvious way. Makes the dependency graph opaque. Difficult to reason about.

**Instead:** Explicit imports only. If A needs C, A must `import C`. The module graph reflects actual usage, not transitive closure.

### Anti-Pattern 4: Demand-Driven Module Loading in Type Checker

**What:** Type checker reads and parses imported files on demand when it encounters an import statement.

**Why bad:** Mixes file I/O with type inference. Makes error reporting confusing. Prevents caching. Creates circular dependency issues. Couples type checker to file system.

**Instead:** Resolve entire module graph upfront. Parse all files. Type-check in topological order. The type checker never touches the file system.

### Anti-Pattern 5: Flattening Modules Into One Parse Tree

**What:** Concatenate all .snow files into a single string, parse once.

**Why bad:** Destroys file-level error reporting. Makes TextRange offsets wrong for all files except the first. Breaks the LSP (which operates per-file). Cannot report "error in math/vector.snow line 5".

**Instead:** Each file has its own `Parse` with its own `GreenNode` and `TextRange` space.

---

## Scalability Considerations

| Concern | At 5 modules | At 50 modules | At 500 modules |
|---------|-------------|---------------|----------------|
| File discovery | Instant | < 10ms | < 100ms |
| Parse (serial) | < 25ms | < 250ms | ~2.5 sec (parallelize) |
| Topo sort | Trivial | Trivial | Trivial O(V+E) |
| Type check | < 50ms | < 500ms | ~5 sec (parallelize levels) |
| MIR lower + merge | < 25ms | < 250ms | ~1 sec |
| LLVM compile | < 200ms | < 2 sec | ~20 sec (single module) |
| Linking | < 100ms | < 100ms | < 100ms (single .o) |
| Memory | ~10 MB | ~50 MB | ~500 MB |

**The bottleneck at scale is LLVM single-module compilation.** If Snow ever reaches 500+ modules, consider separate LLVM compilation + LTO. But that is a performance optimization for a future milestone.

---

## Suggested Build Order for Implementation

### Phase 1: Module Resolution Infrastructure

**Goal:** Given a project directory, produce a `ModuleGraph` in topological order.

**Work:**
1. Create `snow-resolve` crate with `ModulePath`, `ModuleGraph`, `ResolvedModule` types
2. Implement `file_to_module_path` (directory structure to module path mapping)
3. Implement import extraction from `Parse` (walk CST for `IMPORT_DECL` and `FROM_IMPORT_DECL`)
4. Implement `resolve_project` (discover files, parse, build dependency edges, topological sort)
5. Implement cycle detection with error messages showing the cycle path
6. Test with fixture projects

**Blocked by:** Nothing
**Estimated new code:** ~400-600 lines

### Phase 2: Cross-Module Type Checking

**Goal:** Type-check files in dependency order with imported symbols.

**Work:**
1. Add `check_with_imports` and `ImportContext` to `snow-typeck`
2. Add `collect_exports` function (extract pub symbols from TypeckResult)
3. Modify `infer_item` for `ImportDecl`/`FromImportDecl` to use ImportContext, falling back to stdlib_modules
4. Extend qualified name resolution in `infer_field_access`
5. Wire up in `snowc build` to type-check in topological order
6. Test cross-module function calls, struct usage, trait impls

**Blocked by:** Phase 1
**Estimated new/modified code:** ~200-300 lines in snow-typeck, ~50 lines in snowc

### Phase 3: Cross-Module MIR Lowering and Codegen

**Goal:** Merge multi-file MIR into single module, produce working binary.

**Work:**
1. Add `lower_project_to_mir` to `snow-codegen`
2. Implement name mangling (module prefix for function/type names)
3. Implement name remapping in `Lowerer` (import names -> mangled names)
4. Handle struct/sum type deduplication during merge
5. Entry function detection: only main.snow's main()
6. Update `snowc build` to use merged MIR pipeline
7. End-to-end tests with multi-file projects

**Blocked by:** Phase 2
**Estimated new/modified code:** ~200-300 lines in snow-codegen, ~80 lines in snowc

### Phase 4: Polish and Edge Cases

**Goal:** Production-quality diagnostics and edge case handling.

**Work:**
1. Circular import detection with cycle path in error message
2. "Module not found" errors with suggestions (did you mean?)
3. "Symbol not exported" errors (private symbol imported)
4. Unused import warnings
5. Cross-module type annotations in error messages
6. Integration test suite for multi-file projects
7. Update existing e2e tests to continue working

**Blocked by:** Phase 3
**Estimated code:** ~100-200 lines

---

## Sources

### Primary (HIGH confidence)
- Direct codebase analysis: `snowc/src/main.rs` build pipeline (lines 194-262)
- `snow-typeck/src/env.rs` TypeEnv scope-stack implementation
- `snow-typeck/src/infer.rs` stdlib_modules() resolution pattern (lines 210-490)
- `snow-typeck/src/infer.rs` infer_item import handling (lines 1389-1427)
- `snow-typeck/src/lib.rs` TypeckResult structure and check() entry point
- `snow-codegen/src/mir/mod.rs` MirModule and MirFunction structure
- `snow-codegen/src/mir/lower.rs` Lowerer struct and lower_item (lines 158-655)
- `snow-codegen/src/lib.rs` compile_to_binary and lower_to_mir_module entry points
- `snow-codegen/src/codegen/mod.rs` CodeGen struct and compile() method
- `snow-codegen/src/link.rs` system linker invocation
- `snow-parser/src/syntax_kind.rs` all CST node kinds including module/import
- `snow-parser/src/parser/items.rs` import and module parse functions
- `snow-parser/src/ast/item.rs` ImportDecl, FromImportDecl, ModuleDef AST nodes

### Secondary (MEDIUM confidence)
- [JS++ Compiler Module Resolution Architecture](https://www.onux.com/jspp/blog/under-the-hood-the-jspp-import-system/)
- [LLVM Link Time Optimization](https://llvm.org/docs/LinkTimeOptimization.html)
- [Clang Standard C++ Modules](https://clang.llvm.org/docs/StandardCPlusPlusModules.html)
- [Stanford Compiler Modules](https://crypto.stanford.edu/~blynn/compiler/module.html)
