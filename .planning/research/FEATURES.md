# Feature Landscape: Module System

**Domain:** Multi-file module system for a statically-typed, functional-first compiled language with HM type inference, traits/protocols, actors, and Elixir/Ruby-inspired syntax
**Researched:** 2026-02-09
**Confidence:** HIGH (module systems are extremely well-studied; Snow's design decisions are already well-constrained by existing choices)

---

## Current State in Snow

Before defining module features, here is what already exists and directly affects module design:

**Working (infrastructure the module system interacts with):**
- `import` keyword (IMPORT_KW) with module path parsing: `import String` resolves to stdlib module
- `from ... import` syntax (FROM_IMPORT_DECL): `from String import length` brings specific names into scope
- `module` keyword (MODULE_KW) with `module Name do ... end` inline module definitions
- `pub` visibility modifier (VISIBILITY node with PUB_KW) on fn, struct, sum type, interface, supervisor
- PATH node parsing with dot-separated segments: `Foo.Bar.Baz`
- ImportList node parsing for selective imports with comma separation
- Glob imports (`from Module import *`) explicitly rejected at parse time with error message
- TypeEnv with scope stack (Vec of HashMaps) for lexical scoping
- TypeRegistry mapping struct names, type alias names, sum type names to their definitions
- TraitRegistry for interface definitions and impl blocks
- MIR lowering pipeline: Parse -> TypeckResult -> MirModule -> LLVM codegen
- MirModule contains: functions (Vec<MirFunction>), structs, sum_types, entry_function name, service_dispatch
- MirFunction has mangled name strings (e.g., `identity_Int` for monomorphized generics)
- Name resolution for qualified access: `String.length("test")` resolves `String` as stdlib module, `length` as function
- Existing build command: `snowc build <dir>` reads `main.snow`, parses, typechecks, codegens as single file
- snow-fmt: `snowc fmt` formats `.snow` files (already recursively collects `.snow` files in directories)
- snow-lsp: LSP server (single-file mode currently)
- snow-pkg: package manager with `snow.toml` manifest and `snow.lock` lockfile
- 70,501 lines of Rust across 11 crates

**Not yet working (what this milestone adds):**
- File-based modules (file path determines module name, e.g., `math/vector.snow` -> `Math.Vector`)
- Multi-file project compilation (reading multiple `.snow` files, building dependency graph)
- Cross-module name resolution (resolving `Vector.add(a, b)` to a function in another file)
- Cross-module type checking (types defined in one module used in another)
- Cross-module trait/impl resolution (impl blocks and interface definitions across files)
- Module dependency graph construction with cycle detection and topological sort
- `pub` enforcement across file boundaries (private items invisible to importers)
- Cross-module MIR lowering and LLVM codegen (merging multiple modules into one compilation unit)

**Key design constraints (already decided):**
- File = module (file path determines module name)
- Private by default, `pub` to export
- Two import forms: `import Math.Vector` (qualified) and `from Math.Vector import add, scale` (selective)
- No circular imports (compile error)
- Primary goal: split growing `.snow` files into multiple files

---

## Table Stakes

Features users expect from a module system. Missing any of these and multi-file projects feel broken.

| Feature | Why Expected | Complexity | Dependencies | Notes |
|---------|--------------|------------|--------------|-------|
| **File-based module identity** | Every comparable language maps files to modules (Rust, Go, Elixir, Haskell). Users expect `math/vector.snow` to be module `Math.Vector`. | Medium | Directory traversal in `snowc build`, path-to-module-name conversion | Convention: lowercase directories/filenames map to PascalCase module segments. `math/vector.snow` -> `Math.Vector`. |
| **Qualified import (`import Math.Vector`)** | Universal in every module system. Brings module name into scope for qualified access: `Vector.add(a, b)`. Parser already handles this syntax. | Medium | Module resolution (finding the file), binding the last segment as local name | Already parsed. Needs: file lookup, cross-module symbol table, qualified name resolution in typechecker. |
| **Selective import (`from Math.Vector import add, scale`)** | Elixir's `import`, Haskell's `import Foo (bar, baz)`, Python's `from x import y`. Already parsed. | Medium | Same file lookup, plus injecting specific names into local scope | Already parsed. `from Module import *` already rejected. Need to verify imported names exist and are `pub`. |
| **`pub` visibility enforcement** | Rust, Go, Elixir, Haskell all have visibility. Snow already has `pub` keyword on items. Currently unenforced since everything is one file. | Medium | Typechecker must check visibility when resolving cross-module references | The `pub` modifier already exists on FnDef, StructDef, SumTypeDef, InterfaceDef, SupervisorDef. Must enforce: only `pub` items visible to importers. |
| **Cross-module function calls** | The fundamental reason for having modules. `import Math.Vector` then `Vector.add(a, b)` must resolve to the `add` function in `math/vector.snow`. | High | Module symbol tables, qualified name resolution in type checker, cross-module MIR merging, LLVM symbol linking | This is the core challenge. Requires building a per-module symbol table, then resolving cross-module references during type checking. |
| **Cross-module type usage** | Types (structs, sum types) defined in one module must be usable in another. `import Geometry` then `let p = Geometry.Point { x: 1.0, y: 2.0 }`. | High | Cross-module TypeRegistry, struct/sum type name qualification, pattern matching against imported types | Must handle: struct literal construction, field access, pattern matching on imported sum type variants. |
| **Cross-module trait/impl resolution** | `impl Display for Point` in the module defining Point must be visible when Point is used elsewhere. | High | Global trait registry merging, monomorphization across module boundaries | Trait impls are not imported explicitly -- they are globally visible (same as Rust/Haskell instance declarations). |
| **Module dependency graph with topological sort** | Files must be compiled in dependency order. If A imports B, B must be processed first. Standard in every compiled language. | Medium | Graph construction from import declarations, topological sort, error on cycles | Kahn's algorithm or DFS-based topological sort. Detect cycles and report with clear diagnostics. |
| **Circular dependency detection with clear errors** | Every compiled language prohibits or detects cycles. Snow's design explicitly forbids circular imports. | Low | Cycle detection during topological sort | Report the cycle path: "Circular dependency: A -> B -> C -> A". Elixir has compile-time warnings for this. Go enforces this strictly. |
| **Project compilation mode** | `snowc build <dir>` must find and compile all `.snow` files in the project, not just `main.snow`. | Medium | Directory traversal (already exists in `collect_snow_files`), orchestrating multi-file pipeline | The `collect_snow_files_recursive` function already exists in `snowc`. Needs: process all files, not just `main.snow`. |
| **Entry point (`main.snow` with `fn main()`)** | Users need a clear program entry point. The convention is already `main.snow`. | Low | Validate that `main.snow` exists and contains `fn main()` | Already enforced by `snowc build`. Extend to multi-file context. |
| **Module-scoped name collision prevention** | Two modules can both define a private `helper` function without conflict. | Medium | Name mangling at MIR level with module prefix | `Math__add` vs `Utils__add`. Extends existing `Trait__Method__Type` mangling. |
| **Error diagnostics with module context** | When a type error involves an imported name, the error must show which module it came from. | Medium | Ariadne diagnostics integration, source file tracking per diagnostic | Currently all diagnostics reference a single file. Must extend to track source file per span. |

---

## Differentiators

Features that would make Snow's module system stand out. Not strictly expected, but provide meaningful value.

| Feature | Value Proposition | Complexity | Dependencies | Notes |
|---------|-------------------|------------|--------------|-------|
| **Import aliasing (`import Math.Vector as Vec`)** | Shortens long module names. Elixir has `alias Math.Vector, as: Vec`. Haskell has `import Data.Map as M`. Python has `import numpy as np`. Avoids name collisions between modules with same last segment. | Low | Parser extension for optional `as Name` after import path | Small parser addition. High value for usability. Nearly universal across languages. |
| **Re-exports (`pub import Math.Vector`)** | A facade module can re-export items from submodules. Rust's `pub use`, Haskell's `module Foo (module Bar)`. Allows restructuring internals without breaking consumers. | Medium | Parse `pub import`, propagate exported symbols into re-exporting module's public interface | Essential for library authors building clean public APIs from internal modules. |
| **Unused import warnings** | Catches dead imports, improves code quality. Standard in Go (error), Rust (warning), Elixir (warning). | Medium | Track which imported names are referenced during typeck | Quality-of-life. Go makes unused imports a hard error; a warning is friendlier for development. |
| **Module-level constants (`pub let PI = 3.14159`)** | Modules often export constants. Elixir has `@module_attribute`, Rust has `const`, Go has `const`. Snow's `let` at module level could serve this role. | Low | Allow `let` bindings at module top level, evaluate at compile time for literal values | Already parsed (LetBinding is an Item variant). Need constant folding for cross-module use. |
| **Module-level doc comments** | `## Math module provides...` visible in LSP hover and generated docs. | Low | `MODULE_DOC_COMMENT` token already exists in the parser | Pairs naturally with LSP multi-file support. |
| **Import ordering in formatter** | Consistent import blocks across project. Groups stdlib imports, then user module imports, alphabetically. | Low | Extend `snow-fmt` to sort import declarations | Small polish with outsized readability impact. |
| **LSP go-to-definition across files** | Click on `Math.add` -> jump to `math.snow` definition. | Medium | Multi-file index in snow-lsp, symbol location tracking | Important for developer experience but can trail core compilation. |
| **Implicit prelude module** | Common types/functions available without import. A `prelude.snow` whose exports are auto-imported. | Low | Designate a prelude file, auto-inject its exports into every module's scope | Like Haskell's Prelude, Rust's std::prelude. Could hold common type aliases or utility functions. |

---

## Anti-Features

Features to explicitly NOT build. Common in other languages but wrong for Snow or out of scope.

| Anti-Feature | Why Requested | Why Problematic | Alternative |
|--------------|---------------|-----------------|-------------|
| **Glob imports (`from Module import *`)** | Convenience -- bring everything into scope without listing names. | Already rejected in parser with explicit error. Glob imports obscure name origins, cause silent shadowing, break refactoring. Elixir docs explicitly advise against heavy `import` use, preferring `alias`. | Selective imports: `from Math.Vector import add, scale`. Or qualified: `Vector.add(a, b)`. |
| **Circular module dependencies** | Some languages (Python, JS) allow them with caveats. | Snow explicitly forbids them. Circular deps destroy modularity, cause half-loaded module bugs, complicate compilation ordering. Go, Rust, Haskell all prohibit them. | Refactor shared code into a third module. Use dependency inversion. Clear error: "Circular dependency: A -> B -> A." |
| **First-class modules (modules as values)** | OCaml has first-class modules. Could pass modules as function arguments. | Massive type system complexity. Requires existential types, module types as a separate type layer. OCaml's module system is famously complex. Snow targets simplicity. | Use traits/interfaces for abstraction. A function taking a trait-constrained argument achieves the same polymorphism. |
| **Functors (parameterized modules)** | OCaml's killer feature. `module IntSet = Set.Make(Int)`. | Enormous complexity: module-level type parameters, module signatures, applicative vs generative semantics. Would require a second type system layer. | Use generic types and traits. `Set<Int>` already works. Generic functions with `where` clauses provide parameterized behavior. |
| **Module signatures/interfaces (separate `.snowi` files)** | OCaml's `.mli` files, Haskell's explicit export lists. | Doubles file count, doubles maintenance burden. Snow already has `pub` for controlling the interface. | `pub` on items IS the module interface. The set of `pub` items defines the public API. |
| **Macro-based `use` (Elixir-style)** | Elixir's `use` injects code into the current module via macros. | Snow has no macro system. Adding one for `use` alone would be massive scope expansion. | Use traits/interfaces for shared behavior contracts. `impl GenServer for MyModule` is explicit. |
| **Conditional imports** | Import different modules based on platform/config. Rust has `#[cfg(...)]`. | Requires a conditional compilation system. Separate concern from basic modules. | Defer to a future milestone. |
| **Dynamic/runtime imports** | Load modules at runtime based on strings. | Snow compiles to native code. All modules must be known at compile time for type checking and monomorphization. Incompatible with HM inference. | Not applicable. Snow is statically compiled. |
| **Module-level mutable state** | Global mutable variables in a module. | Violates Snow's immutability-first design and actor model (state lives in actors). | Use actors/services for shared mutable state. Module-level `let` bindings are compile-time constants. |
| **Relative imports (`from ../utils import helper`)** | Filesystem-relative module paths. Python has this. | Fragile, confusing, path-dependent. Breaks when files move. | Use absolute module paths from project root: `from Utils import helper`. |
| **Inline module definitions mapping to files** | `module Foo.Bar do ... end` inside a file treating it as a separate module. | Conflates two concepts. Inline modules are namespaces within a file; file-based modules are compilation units. Mixing them creates confusing scoping. | Inline modules remain as in-file namespaces (already working). File-based modules are a separate concept. |
| **Module aliases without import (`import Math as M`)** | Rename a module on import for brevity. | Low-priority for v1.8. Adds parser complexity. `as` aliasing is a differentiator that can be added after core works. | Use selective imports for frequently used names. Qualified access for occasional use. Add `as` in polish phase if time permits. |
| **Orphan rule (Rust-style trait coherence)** | Prevents implementing foreign traits for foreign types. | Snow doesn't have a package ecosystem yet. Orphan rules solve a multi-crate problem. Within a single project, the developer controls all code. | Defer until Snow has packages. Simple rule for now: impl must be in same module as either the trait or the type. |
| **Private modules / internal visibility** | Go's `internal/` convention. Rust's `pub(crate)`. | Adds a third visibility level. `pub` vs private is sufficient for v1.8. | All modules are importable. Control what's visible with `pub` on items. |

---

## Feature Dependencies

```
[0] Multi-file Discovery & Module Identity
    |   snowc build discovers all .snow files in project directory
    |   File path -> module name conversion (math/vector.snow -> Math.Vector)
    |   Module identity struct: name, file path, source text
    |
    v
[1] Per-Module Parsing
    |   Parse each .snow file independently
    |   Extract: import declarations, pub items, type definitions
    |   Build per-module symbol table (exported names + types)
    |
    v
[2] Dependency Graph Construction
    |   From import declarations, build directed graph: module -> dependencies
    |   Topological sort for compilation order
    |   Cycle detection with diagnostic: "A -> B -> C -> A"
    |
    v
[3] Cross-Module Name Resolution
    |   Process modules in topological order
    |   For each import, resolve to target module's symbol table
    |   Qualified access: Vector.add -> look up "add" in Math.Vector's exports
    |   Selective import: from Math.Vector import add -> inject "add" into local scope
    |   Verify: imported names exist AND are pub (else error)
    |
    v
[4] Cross-Module Type Checking
    |   Types (structs, sum types) from imported modules available in TypeRegistry
    |   Trait impls from all modules merged into global TraitRegistry
    |   HM inference works across module boundaries
    |   Struct literal construction with imported types
    |   Pattern matching on imported sum type variants
    |
    v
[5] Cross-Module MIR & Codegen
    |   Merge all modules' MIR functions into single MirModule
    |   Name mangling: prefix functions with module path to avoid collisions
    |   e.g., Math.Vector.add -> Math__Vector__add
    |   Monomorphization across module boundaries
    |   Single LLVM module with all functions, linked to one binary
    |
    v
[6] pub Visibility Enforcement
    |   Type checker rejects access to non-pub items across module boundaries
    |   Clear error: "function `helper` in module Math.Vector is private"
    |   Suggestion: "did you mean to mark it `pub`?"
    |
    v
[7] Tooling Integration
        Formatter: format all .snow files in project (already recursive)
        LSP: multi-file awareness, go-to-definition across modules
        Package manager: module system enables proper dependency resolution
```

**Critical path for MVP:** [0] -> [1] -> [2] -> [3] -> [4] -> [5] -> [6]
**Tooling path (can trail):** [7] after core compilation works
**Differentiators (can be added incrementally):** Import aliasing at [3], re-exports at [3]+[6], unused import warnings at [3]

---

## Detailed Design Decisions

### Decision 1: File-to-Module Name Mapping

**Question:** How do file paths map to module names?

**Recommendation:** Lowercase file paths with `/` separators map to PascalCase module names with `.` separators.

```
project/
  main.snow           -> (entry point, no module name -- contains fn main())
  math.snow           -> Math
  math/
    vector.snow       -> Math.Vector
    matrix.snow       -> Math.Matrix
  http/
    server.snow       -> Http.Server
    router.snow       -> Http.Router
  utils.snow          -> Utils
```

**Rules:**
1. File stem (without `.snow`) becomes the module name segment
2. Directory nesting creates module path hierarchy
3. Filename `foo_bar.snow` -> module segment `FooBar` (snake_case to PascalCase)
4. `main.snow` in the project root is the entry point, not a module
5. A file `math.snow` AND a directory `math/` can coexist -- `math.snow` defines `Math`, files inside `math/` define `Math.Vector`, etc.
6. Module names must start with uppercase (enforced by PascalCase conversion)

**Rationale:**
- Elixir: modules are atoms like `MyApp.Math.Vector`, files are `lib/my_app/math/vector.ex`
- Rust: `mod math;` looks for `math.rs` or `math/mod.rs`
- Go: package name is the directory name
- Haskell: `Data.Map` lives in `Data/Map.hs`

**Confidence:** HIGH -- this is the unanimously dominant pattern.

### Decision 2: Import Semantics

**Question:** What exactly does `import Math.Vector` do?

**Recommendation:** `import Math.Vector` brings `Vector` into scope as a qualified namespace. Access items as `Vector.add(a, b)`. The last segment of the path becomes the local qualifier automatically (Elixir convention).

```snow
# Qualified import -- brings "Vector" into scope
import Math.Vector
let v = Vector.new(1.0, 2.0)
let scaled = Vector.scale(v, 3.0)

# Selective import -- brings specific names into scope unqualified
from Math.Vector import new, scale
let v = new(1.0, 2.0)
let scaled = scale(v, 3.0)

# Both can coexist
import Math.Vector
from Math.Vector import new
let v = new(1.0, 2.0)        # unqualified (from selective import)
let s = Vector.scale(v, 3.0)  # qualified (from qualified import)
```

**Key semantics:**
1. `import Math.Vector` -- the last segment `Vector` becomes the local qualifier. `Vector.foo` resolves to `Math.Vector.foo`.
2. `from Math.Vector import add, scale` -- `add` and `scale` are injected into the local scope. They shadow any existing local names (with a warning).
3. Importing a non-existent module is a compile error: "Module `Math.Vector` not found. No file at `math/vector.snow`."
4. Importing a non-pub name is a compile error: "Function `helper` in module `Math.Vector` is private."
5. Importing a name that does not exist is a compile error: "Module `Math.Vector` does not export `foo`."
6. Import statements are file-scoped (visible throughout the file, not just the current block).

**Confidence:** HIGH -- matches the existing parser design exactly.

### Decision 3: What Can Be Exported with `pub`

**Question:** Which item types support `pub`?

**Recommendation:** All top-level item types that represent API surface support `pub`.

| Item | `pub` Supported | Notes |
|------|-----------------|-------|
| `fn` / `def` | Yes (already parsed) | `pub fn add(a, b) = a + b` |
| `struct` | Yes (already parsed) | `pub struct Point do ... end` -- makes the type name public |
| `type` (sum type) | Yes (already parsed) | `pub type Color do Red; Green; Blue end` -- type AND all variants public |
| `interface` | Yes (already parsed) | `pub interface Printable do ... end` |
| `supervisor` | Yes (already parsed) | `pub supervisor MySup do ... end` |
| `let` (module constant) | Needs parser support | `pub let PI = 3.14159` exports a compile-time constant |
| `type` (alias) | Needs parser support | `pub type Coord = {Float, Float}` exports a type alias |
| `actor` | No | Actors are runtime entities, spawned not imported. Export a spawn function instead. |
| `service` | Consider adding | Services are referenced by name. `pub service Counter` exports the service type. |
| `impl` blocks | No -- globally visible | Following Rust/Haskell: trait impls are not selectively exported. All impls visible everywhere. |

**Struct field visibility (v1.8 scope):** All fields of a `pub struct` are public. Per-field `pub` can be added later. Rationale: Snow structs are data records (like Elixir structs), not encapsulated objects. All fields accessible by default matches Elixir, Go, and Haskell records.

**Sum type variant visibility:** When a `pub type` sum type is exported, ALL variants are public. Matches Rust (pub enum -> all variants pub), Go, Haskell. Opaque sum types (hiding constructors) is a future feature.

**Confidence:** HIGH -- follows directly from existing parser support and language design.

### Decision 4: Name Mangling for Cross-Module Codegen

**Question:** How are function names mangled to avoid collisions across modules?

**Recommendation:** Use module path as prefix with double-underscore separator, extending the existing mangling scheme.

```
Math.Vector.add            -> Math__Vector__add
Math.Vector.scale          -> Math__Vector__scale
main.snow's main()         -> main  (entry point, no prefix)
main.snow's helper()       -> helper  (or __main__helper if collision risk)
Display__to_string__Point  -> Display__to_string__Math__Vector__Point  (trait on imported type)
```

**Why double underscore:** Snow already uses `Trait__Method__Type` for trait method mangling (documented in key decisions). Extending with `Module__` prefix is consistent.

**Confidence:** HIGH -- extends existing mangling convention naturally.

### Decision 5: Trait Impl Visibility

**Question:** When module A defines `interface Printable` and module B defines `impl Printable for Point`, who can see the impl?

**Recommendation:** All trait impls are globally visible. They are not imported explicitly.

This matches:
- **Rust:** Trait impls are visible wherever both the trait and type are in scope.
- **Haskell:** "Instance declarations are not explicitly named in import or export lists. Every module exports all of its instance declarations."
- **Elixir:** Protocol implementations are global.

**Practical consequence:** When building the global TraitRegistry, merge impls from ALL compiled modules. The type checker sees all impls regardless of import statements.

**Simplified orphan rule for v1.8:** An impl block must be in the same module as either the trait definition or the type definition. This prevents confusing "impl exists in random third module" scenarios and can be relaxed later.

**Confidence:** HIGH -- unanimous across comparable languages.

### Decision 6: Handling the Prelude / Builtins

**Question:** Currently, builtins (println, Int, String, List, etc.) are hardcoded into the type checker. How do they interact with the module system?

**Recommendation:** Builtins remain implicitly available in all modules. No import needed for `println`, `Int`, `String`, etc.

This matches:
- **Haskell:** Prelude is implicitly imported
- **Rust:** std prelude is implicitly available
- **Elixir:** Kernel functions are available everywhere
- **Go:** builtin functions need no import

**Stdlib modules** (String, List, Map, Set, IO, File, etc.) still require explicit import. The module system treats them as regular modules implemented in the runtime.

**Confidence:** HIGH -- builtins must not regress.

### Decision 7: Distinguishing Stdlib from User Modules

**Question:** When the type checker sees `import String`, how does it know this is the stdlib String module vs a user-defined `string.snow` file?

**Recommendation:** User modules take precedence. If `string.snow` exists in the project, `import String` resolves to it. Otherwise, fall back to stdlib.

**Resolution order:**
1. Look for user file matching the module path (e.g., `string.snow` for `String`)
2. If not found, check stdlib modules (hardcoded 14 stdlib modules)
3. If neither found, error: "Module `Foo` not found"

**Why user-first:** Users should be able to shadow stdlib modules if they want. This is consistent with Rust (local modules shadow std) and Elixir (project modules shadow Hex deps).

**Confidence:** MEDIUM -- the resolution order is a design choice. User-first is the more flexible option. May want to warn on stdlib shadowing.

---

## MVP Recommendation

### Build (module system milestone)

**Phase 1: Multi-File Discovery and Parsing**
1. Extend `snowc build` to discover all `.snow` files in project directory
2. Implement file-path-to-module-name conversion (snake_case -> PascalCase, `/` -> `.`)
3. Parse each file independently, producing per-module AST
4. Extract import declarations from each module
5. Build dependency graph from imports
6. Topological sort with cycle detection (clear error on cycles)

**Phase 2: Cross-Module Name Resolution**
7. Build per-module symbol table: exported names (pub items) with their types/signatures
8. Process modules in topological order
9. For `import Math.Vector`: resolve file, bind `Vector` as qualified namespace
10. For `from Math.Vector import add`: inject `add` into local scope, verify it is pub
11. Resolve qualified names (`Vector.add`) to target module's symbol table entry
12. Error on: missing module, missing name, non-pub access

**Phase 3: Cross-Module Type Checking**
13. Merge TypeRegistry across modules (struct defs, sum type defs, type aliases)
14. Merge TraitRegistry across modules (interface defs, impl blocks)
15. Type check each module in topological order, with access to imported types
16. Cross-module struct literal construction and field access
17. Cross-module sum type variant usage and pattern matching
18. Cross-module trait method resolution (monomorphization-aware)

**Phase 4: Cross-Module Codegen**
19. Merge all modules' MIR into single MirModule
20. Apply module-path name mangling to all function names
21. Ensure monomorphization works across module boundaries
22. Link into single binary (existing pipeline)
23. Validate entry point: `main.snow` must have `fn main()`

**Phase 5: Diagnostics and Polish**
24. Error messages include module context: "in module Math.Vector (math/vector.snow)"
25. "Module not found" with file path suggestion
26. "Name not exported" with `pub` suggestion
27. "Circular dependency" with cycle path
28. Import aliasing: `import Math.Vector as Vec` (if time permits)

### Defer to Post-MVP

- Per-field `pub` visibility on struct fields
- Re-exports (`pub import Math.Vector`)
- Nested module re-exports for clean APIs
- Incremental compilation (per-module caching)
- `pub(crate)` restricted visibility
- Opaque types (pub type without pub constructors)
- LSP multi-file go-to-definition (can trail core compilation)
- Unused import warnings
- Implicit prelude module

---

## Complexity Assessment

| Feature | Estimated Effort | Risk | Notes |
|---------|-----------------|------|-------|
| Multi-file discovery and path mapping | 1 day | LOW | `collect_snow_files` already exists; add module name derivation |
| Per-module parsing | 0.5 days | LOW | Call existing `snow_parser::parse()` per file |
| Dependency graph + topological sort | 1-2 days | LOW | Standard graph algorithm; import declarations already parsed |
| Cycle detection with diagnostics | 0.5 days | LOW | Falls out of topological sort (detect back edges) |
| Per-module symbol table construction | 2-3 days | MEDIUM | Walk AST, collect pub items with type signatures; new infrastructure |
| Cross-module name resolution (qualified) | 2-3 days | MEDIUM | Extend type checker's name lookup to check imported module symbol tables |
| Cross-module name resolution (selective) | 1-2 days | MEDIUM | Inject imported names into local TypeEnv before type checking |
| Cross-module TypeRegistry merging | 1-2 days | MEDIUM | Merge struct_defs, sum_type_defs, type_aliases; handle name collisions with module prefix |
| Cross-module TraitRegistry merging | 1-2 days | MEDIUM | Merge interface_defs, impl blocks; all impls globally visible |
| Cross-module type checking | 3-5 days | HIGH | Imported types in struct literals, pattern matching, function signatures. Largest risk area. |
| Name mangling with module prefix | 1 day | LOW | Extend existing mangling scheme |
| MIR merging across modules | 1-2 days | MEDIUM | Concatenate function lists; ensure no name collisions after mangling |
| Cross-module monomorphization | 2-3 days | HIGH | Generic function in module A called with concrete type from module B |
| pub visibility enforcement | 1 day | LOW | Check visibility flag during cross-module name resolution |
| Multi-file diagnostics | 1-2 days | LOW | Track source file per span; extend ariadne integration |
| Import aliasing (`as`) | 0.5 days | LOW | Parser extension + name binding |
| Integration testing | 3-5 days | MEDIUM | Many scenarios: functions, types, traits, patterns, generics across files |

**Total estimated effort:** 25-40 days

**Key risks:**
1. **Cross-module type checking is the hardest part.** When module A imports a generic struct `Pair<T>` from module B, type inference in A must correctly instantiate the generic. The existing HM inference works within a single file; extending it requires careful handling of type schemes across module boundaries.
2. **Cross-module monomorphization.** A generic function `fn identity<T>(x :: T) -> T = x` in module A, called as `identity(42)` in module B, must generate `identity_Int`. The monomorphizer currently works on a single MirModule.
3. **Name collision after merging.** If module A and module B both define private `helper`, they must not collide in MIR. Module-path mangling prevents this but must be applied consistently.
4. **Build pipeline restructuring.** The current pipeline is `parse(source) -> check(parse) -> compile(parse, typeck)` for a single file. It must become multi-phase: parse all files, build graph, type-check in order, merge MIR, codegen once. This is a structural change to `snowc`'s `build()` function.

---

## Sources

### Rust Module System
- [Control Scope and Privacy with Modules - The Rust Book](https://doc.rust-lang.org/book/ch07-02-defining-modules-to-control-scope-and-privacy.html) -- mod, pub, use, file layout, default private
- [Separating Modules into Different Files - The Rust Book](https://doc.rust-lang.org/book/ch07-05-separating-modules-into-different-files.html) -- file hierarchy, mod.rs convention
- [Visibility and privacy - The Rust Reference](https://doc.rust-lang.org/reference/visibility-and-privacy.html) -- pub(crate), pub(super), struct field visibility
- [Struct visibility - Rust By Example](https://doc.rust-lang.org/rust-by-example/mod/struct_visibility.html) -- struct fields default private
- [Re-Exporting and Privacy in Rust](https://blog.rheinwerk-computing.com/re-exporting-and-privacy-in-rust) -- pub use patterns
- [Coherence and Orphan Rules](https://td-bn.github.io/rust/coherance-and-orphan-rules.html) -- trait impl restrictions across crates
- [Two ways of interpreting visibility in Rust](https://kobzol.github.io/rust/2025/04/23/two-ways-of-interpreting-visibility-in-rust.html) -- global vs local visibility approaches

### Elixir Module System
- [alias, require, import, and use - Elixir v1.19.5](https://hexdocs.pm/elixir/alias-require-and-import.html) -- alias, import, require, use semantics
- [Modules and functions - Elixir v1.19.5](https://hexdocs.pm/elixir/modules-and-functions.html) -- defmodule, nested modules
- [Module attributes - Elixir v1.19.5](https://hexdocs.pm/elixir/module-attributes.html) -- @moduledoc, @doc, compile-time attributes

### Haskell Module System
- [Modules - Haskell 2010 Report](https://www.haskell.org/onlinereport/haskell2010/haskellch5.html) -- export lists, import, qualified, hiding
- [Import - HaskellWiki](https://wiki.haskell.org/Import) -- qualified, as, hiding, selective imports
- [Import and export - GHC User's Guide](https://downloads.haskell.org/ghc/9.14.0.20251028/docs/users_guide/exts/import_export.html) -- GHC extensions

### OCaml Module System
- [The module system - OCaml Manual](https://ocaml.org/manual/5.4/moduleexamples.html) -- signatures, functors, open, include
- [Modules - OCaml Documentation](https://ocaml.org/docs/modules) -- basic module structure
- [Functors - Real World OCaml](https://dev.realworldocaml.org/functors.html) -- parameterized modules (explicitly out of scope for Snow)

### Go Package System
- [Understanding Package Visibility in Go - DigitalOcean](https://www.digitalocean.com/community/tutorials/understanding-package-visibility-in-go) -- exported/unexported, capital letter convention
- [An introduction to Packages, Imports and Modules in Go](https://www.alexedwards.net/blog/an-introduction-to-packages-imports-and-modules) -- package structure, import paths

### Circular Dependencies
- [Cyclic dependencies are evil - F# for fun and profit](https://fsharpforfunandprofit.com/posts/cyclic-dependencies/) -- why circular dependencies destroy modularity
- [Circular dependency - Wikipedia](https://en.wikipedia.org/wiki/Circular_dependency) -- general problem overview

### Module System Design
- [Module systems in programming languages](https://denisdefreyne.com/notes/zlc9l-nrkfw-wztwz/) -- comparison across languages
- [When Modules Are Not Just Namespaces](https://pling.jondgoodwin.com/post/cone-modules/) -- design considerations for module systems
- [CS 242: Modules - Stanford](https://stanford-cs242.github.io/f19/lectures/04-2-modules.html) -- academic overview of module system theory

---
*Feature research for: Snow Language Module System*
*Researched: 2026-02-09*
