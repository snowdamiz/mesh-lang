---
phase: 37-module-graph-foundation
verified: 2026-02-09T19:15:00Z
status: gaps_found
score: 4/7
gaps:
  - truth: "Running `snowc build` on a directory with nested `.snow` files discovers all source files recursively"
    status: failed
    reason: "build_module_graph exists and passes tests but is NOT called from snowc build pipeline"
    artifacts:
      - path: "crates/snowc/src/main.rs"
        issue: "build() function reads only main.snow, never calls discovery::build_module_graph"
    missing:
      - "Wire discovery::build_module_graph into the build() function"
      - "Replace single-file read with multi-file discovery and ordering"
  - truth: "Import declarations in parsed files produce a dependency graph with deterministic topological ordering"
    status: failed
    reason: "Topological sort exists but is never invoked on actual project files"
    artifacts:
      - path: "crates/snowc/src/main.rs"
        issue: "Build pipeline never constructs a ModuleGraph from real project files"
    missing:
      - "Call build_module_graph in build() to get compilation order"
      - "Parse and compile modules in the returned order"
  - truth: "A circular import chain produces a compile error naming the cycle path"
    status: failed
    reason: "Cycle detection works in tests but build pipeline never reaches it"
    artifacts:
      - path: "crates/snowc/src/main.rs"
        issue: "Build pipeline has no code path that can trigger CycleError"
    missing:
      - "Handle Result from build_module_graph and report CycleError to user"
---

# Phase 37: Module Graph Foundation Verification Report

**Phase Goal:** Compiler can discover, name, and order modules from a project directory
**Verified:** 2026-02-09T19:15:00Z
**Status:** gaps_found
**Re-verification:** No — initial verification

## Goal Achievement

### Observable Truths

| #   | Truth                                                                         | Status       | Evidence                                                   |
| --- | ----------------------------------------------------------------------------- | ------------ | ---------------------------------------------------------- |
| 1   | Running `snowc build` discovers all `.snow` files recursively                 | ✗ FAILED     | build() never calls discover_snow_files                    |
| 2   | File `math/vector.snow` maps to module name `Math.Vector`                     | ✓ VERIFIED   | path_to_module_name tests pass, implementation correct     |
| 3   | `main.snow` in project root is entry point, not a module                      | ✓ VERIFIED   | path_to_module_name("main.snow") returns None              |
| 4   | Import declarations produce dependency graph with deterministic topo ordering | ✗ FAILED     | Functions exist but build() never constructs ModuleGraph   |
| 5   | Circular import chain produces compile error naming the cycle path            | ✗ FAILED     | Cycle detection works in tests but never reached in build  |
| 6   | ModuleId, ModuleInfo, ModuleGraph, CycleError types exist in snow-common      | ✓ VERIFIED   | All types public and exported, tests pass                  |
| 7   | Hidden directories (starting with '.') are skipped during discovery           | ✓ VERIFIED   | test_discover_snow_files verifies .hidden/secret.snow skip |

**Score:** 4/7 truths verified

### Required Artifacts

| Artifact                                      | Expected                                                 | Status      | Details                                                             |
| --------------------------------------------- | -------------------------------------------------------- | ----------- | ------------------------------------------------------------------- |
| `crates/snow-common/src/module_graph.rs`      | ModuleId, ModuleInfo, ModuleGraph, CycleError types      | ✓ VERIFIED  | 358 lines, all types public, 10 unit tests pass                     |
| `crates/snowc/src/discovery.rs`               | discover_snow_files, path_to_module_name, extract_imports | ✓ VERIFIED  | 378 lines, all functions public, 12 tests pass (unit + integration) |
| `crates/snow-common/src/module_graph.rs`      | topological_sort function                                | ✓ VERIFIED  | Kahn's algorithm with alphabetical tie-breaking, 6 tests pass       |
| `crates/snowc/src/discovery.rs`               | build_module_graph pipeline                              | ⚠️ ORPHANED | Exists (167-220) but NOT called from main.rs build()                |
| `crates/snow-common/Cargo.toml`               | rustc-hash dependency                                    | ✓ VERIFIED  | rustc-hash = { workspace = true } present                           |
| `crates/snow-common/src/lib.rs`               | pub mod module_graph                                     | ✓ VERIFIED  | Export at line 20                                                   |
| `crates/snowc/src/main.rs`                    | mod discovery                                            | ✓ VERIFIED  | Declared at line 20                                                 |

### Key Link Verification

| From                                  | To                                            | Via                                     | Status      | Details                                                   |
| ------------------------------------- | --------------------------------------------- | --------------------------------------- | ----------- | --------------------------------------------------------- |
| crates/snowc/src/discovery.rs         | crates/snow-common/src/module_graph.rs        | use snow_common::module_graph           | ✓ WIRED     | Line 9: imports ModuleGraph, ModuleId, CycleError        |
| crates/snowc/src/discovery.rs         | snow_parser::parse                            | Parses each .snow file                  | ✓ WIRED     | Line 193: snow_parser::parse(&source)                     |
| crates/snowc/src/discovery.rs         | snow_common::module_graph::topological_sort   | build_module_graph calls topological_sort | ✓ WIRED     | Line 215: module_graph::topological_sort(&graph)          |
| **crates/snowc/src/main.rs**          | **crates/snowc/src/discovery.rs**             | **build() calls build_module_graph**    | ✗ NOT_WIRED | **discovery module imported but build_module_graph never called** |
| **crates/snowc/src/main.rs**          | **crates/snowc/src/discovery.rs**             | **build() calls discover_snow_files**   | ✗ NOT_WIRED | **discover_snow_files never called from build()**         |

### Requirements Coverage

| Requirement | Status       | Blocking Issue                                |
| ----------- | ------------ | --------------------------------------------- |
| INFRA-01    | ⚠️ PARTIAL   | Types exist but not used in build pipeline    |
| INFRA-02    | ⚠️ PARTIAL   | Path-to-name works but not called by snowc    |
| INFRA-05    | ⚠️ PARTIAL   | Cycle detection works but not reachable       |
| IMPORT-03   | ⚠️ PARTIAL   | Import extraction works but not used          |
| IMPORT-04   | ⚠️ PARTIAL   | Toposort works in isolation, not integrated   |
| IMPORT-05   | ✗ BLOCKED    | Can't test cycle errors because not wired     |

### Anti-Patterns Found

| File                               | Line  | Pattern                        | Severity | Impact                                                                |
| ---------------------------------- | ----- | ------------------------------ | -------- | --------------------------------------------------------------------- |
| crates/snowc/src/main.rs           | 225   | Only reads main.snow           | 🛑 Blocker | Phase goal requires discovering ALL .snow files                       |
| crates/snowc/src/main.rs           | 229   | Single-file parse              | 🛑 Blocker | Should parse all discovered modules                                   |
| crates/snowc/src/main.rs           | 20    | `mod discovery;` declared      | ⚠️ Warning | Module imported but never used (dead code)                            |

### Gaps Summary

The module graph foundation **exists and works correctly in isolation** (all 22 tests pass), but it is **completely disconnected from the build pipeline**.

**What works:**
- File discovery finds .snow files recursively, skips hidden dirs ✓
- Path-to-module-name converts filesystem paths to PascalCase module names ✓
- Import extraction pulls module paths from ASTs ✓
- ModuleGraph constructs dependency graphs ✓
- Topological sort orders modules with alphabetical tie-breaking ✓
- Cycle detection identifies circular imports with path ✓

**What's missing:**
- `snowc build` never calls `build_module_graph` to discover files
- Build pipeline still only reads `main.snow` directly
- Multi-file projects cannot be compiled (only single-file main.snow)
- Cycle detection is unreachable from user-facing commands
- Success criteria 1, 4, and 5 cannot be tested end-to-end

**Root cause:** Phase 37 built the **library layer** (data structures and algorithms) but never **integrated it into the application layer** (the build command). This is classic bottom-up development without top-down wiring.

**Impact:** Phase goal "Compiler can discover, name, and order modules from a project directory" is NOT achieved. The compiler has the *capability* but doesn't *use* it.

---

_Verified: 2026-02-09T19:15:00Z_
_Verifier: Claude (gsd-verifier)_
