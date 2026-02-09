# Roadmap: Snow

## Milestones

- [x] **v1.0 MVP** - Phases 1-10 (shipped 2026-02-07)
- [x] **v1.1 Language Polish** - Phases 11-15 (shipped 2026-02-08)
- [x] **v1.2 Runtime & Type Fixes** - Phases 16-17 (shipped 2026-02-08)
- [x] **v1.3 Traits & Protocols** - Phases 18-22 (shipped 2026-02-08)
- [x] **v1.4 Compiler Polish** - Phases 23-25 (shipped 2026-02-08)
- [x] **v1.5 Compiler Correctness** - Phases 26-29 (shipped 2026-02-09)
- [x] **v1.6 Method Dot-Syntax** - Phases 30-32 (shipped 2026-02-09)
- [x] **v1.7 Loops & Iteration** - Phases 33-36 (shipped 2026-02-09)
- [ ] **v1.8 Module System** - Phases 37-42 (in progress)

## Phases

<details>
<summary>v1.0 MVP (Phases 1-10) - SHIPPED 2026-02-07</summary>

See milestones/v1.0-ROADMAP.md for full phase details.
55 plans across 10 phases. 52,611 lines of Rust. 213 commits.

</details>

<details>
<summary>v1.1 Language Polish (Phases 11-15) - SHIPPED 2026-02-08</summary>

See milestones/v1.1-ROADMAP.md for full phase details.
10 plans across 5 phases. 56,539 lines of Rust (+3,928). 45 commits.

</details>

<details>
<summary>v1.2 Runtime & Type Fixes (Phases 16-17) - SHIPPED 2026-02-08</summary>

See milestones/v1.2-ROADMAP.md for full phase details.
6 plans across 2 phases. 57,657 lines of Rust (+1,118). 22 commits.

</details>

<details>
<summary>v1.3 Traits & Protocols (Phases 18-22) - SHIPPED 2026-02-08</summary>

See milestones/v1.3-ROADMAP.md for full phase details.
18 plans across 5 phases. 63,189 lines of Rust (+5,532). 65 commits.

</details>

<details>
<summary>v1.4 Compiler Polish (Phases 23-25) - SHIPPED 2026-02-08</summary>

See milestones/v1.4-ROADMAP.md for full phase details.
5 plans across 3 phases. 64,548 lines of Rust (+1,359). 13 commits.

</details>

<details>
<summary>v1.5 Compiler Correctness (Phases 26-29) - SHIPPED 2026-02-09</summary>

See milestones/v1.5-ROADMAP.md for full phase details.
6 plans across 4 phases. 66,521 lines of Rust (+1,973). 29 commits.

</details>

<details>
<summary>v1.6 Method Dot-Syntax (Phases 30-32) - SHIPPED 2026-02-09</summary>

See milestones/v1.6-ROADMAP.md for full phase details.
6 plans across 3 phases. 67,546 lines of Rust (+1,025). 24 commits.

</details>

<details>
<summary>v1.7 Loops & Iteration (Phases 33-36) - SHIPPED 2026-02-09</summary>

See milestones/v1.7-ROADMAP.md for full phase details.
8 plans across 4 phases. 70,501 lines of Rust (+2,955). 34 commits.

</details>

### v1.8 Module System (In Progress)

**Milestone Goal:** Add a module system enabling multi-file projects with file-based modules, pub visibility, qualified and selective imports, and dependency graph resolution -- compiled into a single LLVM module via MIR merge.

- [ ] **Phase 37: Module Graph Foundation** - File discovery, module naming, dependency graph with topological sort and cycle detection
- [ ] **Phase 38: Multi-File Build Pipeline** - Per-file parsing, snowc build orchestration, backward compatibility
- [ ] **Phase 39: Cross-Module Type Checking** - Import resolution, qualified/selective access, cross-module functions/structs/sum types/traits
- [ ] **Phase 40: Visibility Enforcement** - Private-by-default semantics, pub modifier, access control errors
- [ ] **Phase 41: MIR Merge & Codegen** - Module-qualified name mangling, MIR merge, cross-module generics via monomorphization
- [ ] **Phase 42: Diagnostics & Integration** - Module-aware error messages, qualified type names in diagnostics, end-to-end validation

## Phase Details

### Phase 37: Module Graph Foundation
**Goal**: Compiler can discover, name, and order modules from a project directory
**Depends on**: Nothing (first phase of v1.8)
**Requirements**: INFRA-01, INFRA-02, INFRA-05, IMPORT-03, IMPORT-04, IMPORT-05
**Success Criteria** (what must be TRUE):
  1. Running `snowc build` on a directory with nested `.snow` files discovers all source files recursively
  2. File `math/vector.snow` maps to module name `Math.Vector` (snake_case dirs become PascalCase segments)
  3. `main.snow` in the project root is recognized as the entry point, not as a module
  4. Import declarations in parsed files produce a dependency graph with deterministic topological ordering
  5. A circular import chain (A imports B, B imports C, C imports A) produces a compile error naming the cycle path
**Plans**: TBD

### Phase 38: Multi-File Build Pipeline
**Goal**: Compiler parses all project files and orchestrates multi-file compilation while preserving single-file behavior
**Depends on**: Phase 37
**Requirements**: INFRA-03, INFRA-04, DIAG-03
**Success Criteria** (what must be TRUE):
  1. Each `.snow` file in the project is parsed into its own independent AST
  2. `snowc build <dir>` compiles all discovered files as a unified project, producing a single binary
  3. Existing single-file programs (`snowc build foo.snow`) compile and run identically to before -- zero regressions
**Plans**: TBD

### Phase 39: Cross-Module Type Checking
**Goal**: Functions, structs, sum types, and traits defined in one module are usable from another module via imports
**Depends on**: Phase 38
**Requirements**: IMPORT-01, IMPORT-02, IMPORT-06, IMPORT-07, XMOD-01, XMOD-02, XMOD-03, XMOD-04, XMOD-05
**Success Criteria** (what must be TRUE):
  1. `import Math.Vector` brings `Vector` into scope; `Vector.add(a, b)` calls the function from that module
  2. `from Math.Vector import { add, scale }` makes `add(a, b)` callable without qualification
  3. A struct defined in module A can be constructed and field-accessed in module B after import
  4. A sum type defined in module A can be pattern-matched with exhaustiveness checking in module B after import
  5. Trait impls defined in any module are visible across all modules without explicit import
**Plans**: TBD

### Phase 40: Visibility Enforcement
**Goal**: Items are private by default and only accessible to other modules when marked `pub`
**Depends on**: Phase 39
**Requirements**: VIS-01, VIS-02, VIS-03, VIS-04, VIS-05
**Success Criteria** (what must be TRUE):
  1. A function without `pub` cannot be called from another module (compile error)
  2. Adding `pub` to a function, struct, sum type, or interface makes it importable by other modules
  3. Attempting to import a private item produces a compile error with a suggestion to add `pub`
  4. All fields of a `pub struct` are accessible to importers (no per-field visibility)
  5. All variants of a `pub type` (sum type) are accessible for construction and pattern matching by importers
**Plans**: TBD

### Phase 41: MIR Merge & Codegen
**Goal**: Multi-module projects compile to a single native binary with correct name mangling and cross-module monomorphization
**Depends on**: Phase 40
**Requirements**: XMOD-06, XMOD-07
**Success Criteria** (what must be TRUE):
  1. Generic functions and types used across module boundaries are monomorphized correctly (e.g., module A defines `identity<T>`, module B calls `identity(42)`)
  2. Two modules each defining a private function named `helper` compile without name collision
  3. A multi-module project with imports, pub items, generics, and traits produces a working native binary
**Plans**: TBD

### Phase 42: Diagnostics & Integration
**Goal**: Error messages for multi-module projects include module context, and the full module system is validated end-to-end
**Depends on**: Phase 41
**Requirements**: DIAG-01, DIAG-02
**Success Criteria** (what must be TRUE):
  1. Compile errors involving cross-module issues include the source module name and file path in the diagnostic
  2. Type errors involving imported types display the module origin (e.g., "expected Math.Vector.Point, got Main.Point")
  3. A realistic multi-module project (3+ modules with structs, traits, generics, and imports) compiles and runs correctly end-to-end
**Plans**: TBD

## Progress

**Execution Order:**
Phases execute in numeric order: 37 -> 38 -> 39 -> 40 -> 41 -> 42

| Phase | Milestone | Plans Complete | Status | Completed |
|-------|-----------|----------------|--------|-----------|
| 1-10 | v1.0 | 55/55 | Complete | 2026-02-07 |
| 11-15 | v1.1 | 10/10 | Complete | 2026-02-08 |
| 16-17 | v1.2 | 6/6 | Complete | 2026-02-08 |
| 18-22 | v1.3 | 18/18 | Complete | 2026-02-08 |
| 23-25 | v1.4 | 5/5 | Complete | 2026-02-08 |
| 26-29 | v1.5 | 6/6 | Complete | 2026-02-09 |
| 30-32 | v1.6 | 6/6 | Complete | 2026-02-09 |
| 33-36 | v1.7 | 8/8 | Complete | 2026-02-09 |
| 37. Module Graph Foundation | v1.8 | 0/TBD | Not started | - |
| 38. Multi-File Build Pipeline | v1.8 | 0/TBD | Not started | - |
| 39. Cross-Module Type Checking | v1.8 | 0/TBD | Not started | - |
| 40. Visibility Enforcement | v1.8 | 0/TBD | Not started | - |
| 41. MIR Merge & Codegen | v1.8 | 0/TBD | Not started | - |
| 42. Diagnostics & Integration | v1.8 | 0/TBD | Not started | - |

**Total: 36 phases shipped across 8 milestones. 114 plans completed. 6 phases planned for v1.8.**
