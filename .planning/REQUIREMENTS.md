# Requirements: v1.8 Module System

## Module Infrastructure

- [ ] **INFRA-01**: Compiler discovers all `.snow` files in a project directory recursively
- [ ] **INFRA-02**: File paths map to module names via convention (`math/vector.snow` -> `Math.Vector`, snake_case -> PascalCase)
- [ ] **INFRA-03**: Each `.snow` file is parsed independently into a per-module AST
- [ ] **INFRA-04**: `snowc build <dir>` compiles all discovered `.snow` files as a unified project (not just `main.snow`)
- [ ] **INFRA-05**: `main.snow` in project root is the entry point containing `fn main()`, not treated as a module

## Import Resolution

- [ ] **IMPORT-01**: `import Math.Vector` brings `Vector` into scope as a qualified namespace (`Vector.add(a, b)`)
- [ ] **IMPORT-02**: `from Math.Vector import { add, scale }` injects specific names into local scope unqualified
- [ ] **IMPORT-03**: Compiler builds module dependency graph from import declarations
- [ ] **IMPORT-04**: Modules are compiled in topological sort order (dependencies before dependents)
- [ ] **IMPORT-05**: Circular imports produce a compile error with the cycle path (`A -> B -> C -> A`)
- [ ] **IMPORT-06**: Importing a non-existent module produces a clear error with file path suggestion
- [ ] **IMPORT-07**: Importing a non-existent name from a module produces a clear error

## Cross-Module Semantics

- [ ] **XMOD-01**: Functions defined in one module can be called from another module via qualified access (`Vector.add(a, b)`)
- [ ] **XMOD-02**: Functions imported selectively can be called unqualified (`add(a, b)` after `from Math.Vector import { add }`)
- [ ] **XMOD-03**: Struct types defined in one module can be constructed and used in another module
- [ ] **XMOD-04**: Sum types defined in one module can be pattern-matched in another module
- [ ] **XMOD-05**: Trait impls are globally visible across all modules (not selectively imported)
- [ ] **XMOD-06**: Generic types and functions work correctly across module boundaries (monomorphization)
- [ ] **XMOD-07**: Two modules can each define private functions with the same name without collision

## Visibility

- [ ] **VIS-01**: All items (fn, struct, sum type, interface) are private by default
- [ ] **VIS-02**: `pub` modifier makes an item visible to importing modules
- [ ] **VIS-03**: Accessing a non-pub item from another module produces a compile error with `pub` suggestion
- [ ] **VIS-04**: All fields of a `pub struct` are accessible to importers (no per-field visibility in v1.8)
- [ ] **VIS-05**: All variants of a `pub type` (sum type) are accessible to importers

## Diagnostics

- [ ] **DIAG-01**: Error messages for cross-module issues include the source module name and file path
- [ ] **DIAG-02**: Type errors involving imported types show the module origin (e.g., "expected Math.Vector.Point")
- [ ] **DIAG-03**: Single-file projects continue to compile identically (full backward compatibility)

## Traceability

| REQ-ID | Phase |
|--------|-------|
| INFRA-01 | |
| INFRA-02 | |
| INFRA-03 | |
| INFRA-04 | |
| INFRA-05 | |
| IMPORT-01 | |
| IMPORT-02 | |
| IMPORT-03 | |
| IMPORT-04 | |
| IMPORT-05 | |
| IMPORT-06 | |
| IMPORT-07 | |
| XMOD-01 | |
| XMOD-02 | |
| XMOD-03 | |
| XMOD-04 | |
| XMOD-05 | |
| XMOD-06 | |
| XMOD-07 | |
| VIS-01 | |
| VIS-02 | |
| VIS-03 | |
| VIS-04 | |
| VIS-05 | |
| DIAG-01 | |
| DIAG-02 | |
| DIAG-03 | |

## Future Requirements (Deferred)

- Import aliasing (`import Math.Vector as Vec`)
- Unused import warnings
- Re-exports (`pub import Math.Vector`)
- Module doc comments
- Formatter import ordering
- LSP cross-file go-to-definition
- Per-field `pub` visibility on struct fields
- Opaque sum types (pub type without pub constructors)
- Incremental compilation
- `pub(crate)` restricted visibility
- Implicit prelude module

## Out of Scope

- Glob imports (`from Module import *`) -- explicitly rejected in parser, obscures name origins
- Circular module dependencies -- destroys modularity, complicates compilation ordering
- First-class modules (modules as values) -- massive type system complexity, use traits instead
- Functors (parameterized modules) -- use generic types and traits instead
- Module signatures/interface files (`.snowi`) -- `pub` items define the interface
- Dynamic/runtime imports -- incompatible with static compilation and HM inference
- Module-level mutable state -- violates immutability-first design, use actors
- Relative imports (`from ../utils import helper`) -- fragile, use absolute module paths
- Orphan rules -- defer until package ecosystem exists
