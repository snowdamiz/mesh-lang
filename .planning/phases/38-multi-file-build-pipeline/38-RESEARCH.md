# Phase 38: Multi-File Build Pipeline - Research

**Researched:** 2026-02-09
**Domain:** Compiler build orchestration -- parsing multiple files, storing per-file ASTs, modifying `snowc build` to process all project files, preserving single-file backward compatibility
**Confidence:** HIGH

## Summary

Phase 38 bridges the infrastructure-only Phase 37 (module graph, file discovery, topological sort) with the later cross-module phases (39-42). Its job is narrow but critical: **modify `snowc build <dir>` to parse ALL `.snow` files into independent ASTs and pass them through the existing single-file pipeline, while preserving identical behavior for single-file projects.** This phase does NOT implement cross-module type checking, name mangling, or MIR merging -- those are Phases 39-41.

The current `build()` function in `snowc/src/main.rs` reads exactly one file (`main.snow`), parses it, type-checks it, and compiles it. Phase 38 must change this to: (1) call `build_module_graph(project_root)` from Phase 37's `discovery.rs` to discover all files and get the topological compilation order, (2) parse each file into its own `Parse` result, (3) type-check and compile only `main.snow` for now (since cross-module type checking is Phase 39), while storing the per-file parse results for downstream phases to consume. The critical constraint is that `build_module_graph` already reads and parses files internally to extract imports -- Phase 38 must either reuse those parse results or restructure the pipeline to avoid double-parsing.

Currently `build_module_graph` parses files internally but does NOT return the `Parse` results -- it only returns `(ModuleGraph, Vec<ModuleId>)`. The function reads source strings, calls `snow_parser::parse()` to extract imports, then discards the `Parse` objects. Phase 38 must either (a) modify `build_module_graph` to return parse results alongside the graph, or (b) separate the graph building from parsing so files are parsed once and the results used for both import extraction and later compilation.

**Primary recommendation:** Refactor the pipeline in two stages. First, modify `build_module_graph` to return per-file source strings and parse results alongside the graph. Second, modify `build()` in `snowc` to iterate over all discovered modules in topological order, type-check each independently (no cross-module imports yet), and compile them. For Phase 38 specifically, only `main.snow` needs to proceed through codegen -- additional files are parsed and type-checked but their results are stored for Phase 39. The key invariant to preserve: single-file projects (directory with only `main.snow`) must produce identical output to before.

## Standard Stack

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| snow-common (module_graph) | local | ModuleGraph, ModuleId, ModuleInfo, topological_sort | Built in Phase 37, the foundation for all multi-file work |
| snow-parser | local | Per-file parsing into independent Parse results | Existing parser, zero changes needed to parse multiple files |
| snow-typeck | local | Per-file type checking | Existing type checker, called once per file |
| snow-codegen | local | Code generation from single Parse+TypeckResult | Existing codegen, receives entry module's data |
| snowc (discovery) | local | discover_snow_files, build_module_graph, extract_imports | Built in Phase 37, needs signature extension |

### Supporting
| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| tempfile | 3 (dev-dep) | Creating multi-file test project directories | Integration and e2e tests |
| insta | 1.46 (workspace) | Snapshot tests for multi-file compilation output | Optional, for complex output verification |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| Extending build_module_graph return type | New separate "parse all files" function | Separate function avoids modifying Phase 37 API but creates redundant parsing. Extending is cleaner. |
| Storing Parse objects in ModuleGraph | Separate Vec alongside ModuleGraph | Parse objects borrow source strings, so storing them in ModuleGraph creates lifetime complexity. Separate Vec with matching indices is simpler. |

**Installation:**
No new dependencies needed. All libraries are already in the workspace.

## Architecture Patterns

### Recommended Project Structure
```
crates/snow-common/src/
  module_graph.rs     # ModuleId, ModuleInfo, ModuleGraph, topological_sort (Phase 37, minimal changes)

crates/snowc/src/
  main.rs             # build() function modified for multi-file pipeline
  discovery.rs        # build_module_graph extended to return parse results
```

### Current build() Flow (Single-File)
```rust
fn build(dir: &Path, ...) -> Result<(), String> {
    let main_snow = dir.join("main.snow");     // Find single file
    let source = fs::read_to_string(&main_snow)?;  // Read it
    let parse = snow_parser::parse(&source);         // Parse it
    let typeck = snow_typeck::check(&parse);         // Type-check it
    report_diagnostics(...);                          // Report errors
    snow_codegen::compile_to_binary(&parse, &typeck, ...)?;  // Compile
}
```

### Target build() Flow (Phase 38: Multi-File Parse, Single-File Compile)
```rust
fn build(dir: &Path, ...) -> Result<(), String> {
    // Step 1: Build module graph (discovers files, parses for imports, toposorts)
    let (graph, compilation_order, module_sources, module_parses) =
        build_module_graph_with_parses(dir)?;

    // Step 2: Find the entry module
    let entry_idx = compilation_order.iter()
        .find(|id| graph.get(**id).is_entry)
        .ok_or("No main.snow entry point found")?;

    // Step 3: Type-check and compile the entry module
    // (Phase 39 will add cross-module type checking for non-entry modules)
    let entry_parse = &module_parses[entry_idx.0 as usize];
    let entry_source = &module_sources[entry_idx.0 as usize];
    let typeck = snow_typeck::check(entry_parse);
    report_diagnostics(entry_source, &main_snow_path, entry_parse, &typeck, diag_opts);

    // Step 4: Compile to binary (identical to current pipeline)
    snow_codegen::compile_to_binary(entry_parse, &typeck, &output_path, opt_level, target, None)?;
}
```

### Pattern 1: Extended build_module_graph Return Type
**What:** Modify `build_module_graph` (or create a new wrapper) to return per-file source strings and Parse results alongside the existing graph and compilation order.
**When to use:** When Phase 38 needs parse results that were already created internally during graph construction.
**Example:**
```rust
/// Extended module graph result that includes per-file parse data.
/// Indices match ModuleId.0 -- module_sources[id.0] is the source for ModuleId(id).
pub struct ProjectData {
    pub graph: ModuleGraph,
    pub compilation_order: Vec<ModuleId>,
    pub module_sources: Vec<String>,
    pub module_parses: Vec<snow_parser::Parse>,
}

pub fn build_project(project_root: &Path) -> Result<ProjectData, String> {
    // Discover files
    let files = discover_snow_files(project_root)?;
    let mut graph = ModuleGraph::new();
    let mut module_sources = Vec::new();
    let mut module_parses = Vec::new();

    // Phase 1: Register modules and parse files
    for relative_path in &files {
        let full_path = project_root.join(relative_path);
        let source = fs::read_to_string(&full_path)
            .map_err(|e| format!("Failed to read '{}': {}", full_path.display(), e))?;

        let is_entry = relative_path == Path::new("main.snow");
        let name = if is_entry {
            "Main".to_string()
        } else {
            path_to_module_name(relative_path)
                .ok_or_else(|| format!("Cannot determine module name for '{}'", relative_path.display()))?
        };

        let parse = snow_parser::parse(&source);
        let id = graph.add_module(name, relative_path.clone(), is_entry);

        module_sources.push(source);
        module_parses.push(parse);
    }

    // Phase 2: Extract imports and build edges (using stored parses)
    for id_val in 0..graph.module_count() {
        let id = ModuleId(id_val as u32);
        let tree = module_parses[id_val].tree();
        let imports = extract_imports(&tree);
        let module_name = graph.get(id).name.clone();

        for import_name in imports {
            match graph.resolve(&import_name) {
                None => { /* unknown import -- skip silently */ }
                Some(dep_id) if dep_id == id => {
                    return Err(format!("Module '{}' cannot import itself", module_name));
                }
                Some(dep_id) => {
                    graph.add_dependency(id, dep_id);
                }
            }
        }
    }

    // Phase 3: Topological sort
    let compilation_order = module_graph::topological_sort(&graph)
        .map_err(|e| format!("Circular dependency: {}", e))?;

    Ok(ProjectData {
        graph,
        compilation_order,
        module_sources,
        module_parses,
    })
}
```

### Pattern 2: Backward-Compatible build() with Feature Detection
**What:** The modified `build()` function detects whether additional `.snow` files exist beyond `main.snow` and processes them, but the entry module's pipeline is identical to the current single-file path.
**When to use:** Always -- this is the core pattern for Phase 38.
**Example:**
```rust
fn build(dir: &Path, ...) -> Result<(), String> {
    // Validate project directory (unchanged)
    if !dir.exists() { return Err(...); }
    if !dir.is_dir() { return Err(...); }
    let main_snow = dir.join("main.snow");
    if !main_snow.exists() { return Err(...); }

    // Build the module graph (discovers ALL files, parses for imports)
    let project = discovery::build_project(dir)?;

    // Find the entry module in the graph
    let entry_id = project.compilation_order.iter()
        .copied()
        .find(|id| project.graph.get(*id).is_entry)
        .ok_or("No entry module found")?;

    let entry_idx = entry_id.0 as usize;
    let entry_parse = &project.module_parses[entry_idx];
    let entry_source = &project.module_sources[entry_idx];
    let entry_path = dir.join(project.graph.get(entry_id).path.clone());

    // Type-check the entry module (Phase 39 adds cross-module imports)
    let typeck = snow_typeck::check(entry_parse);

    // Report diagnostics for the entry module
    let has_errors = report_diagnostics(entry_source, &entry_path, entry_parse, &typeck, diag_opts);

    // Also report parse errors from other modules
    for id in &project.compilation_order {
        if *id == entry_id { continue; }
        let idx = id.0 as usize;
        let parse = &project.module_parses[idx];
        if !parse.errors().is_empty() {
            let mod_path = dir.join(project.graph.get(*id).path.clone());
            let mod_source = &project.module_sources[idx];
            has_errors |= report_parse_errors(mod_source, &mod_path, parse, diag_opts);
        }
    }

    if has_errors {
        return Err("Compilation failed due to errors above.".to_string());
    }

    // Compile entry module to binary (identical to current pipeline)
    // ...
}
```

### Pattern 3: Per-Module Parse Error Reporting
**What:** Even though Phase 38 only compiles `main.snow`, parse errors in ANY module file should be reported. A syntax error in `utils.snow` should fail the build even if `main.snow` is fine.
**When to use:** During the build pipeline, after parsing all files.
**Rationale:** Users expect `snowc build` to validate all project files. A parse error in any file means the project is broken.

### Anti-Patterns to Avoid
- **Double-parsing files:** The current `build_module_graph` parses files internally to extract imports, then discards the Parse results. If `build()` parses files again separately, that is wasteful and risks inconsistency. Parse once, use everywhere.
- **Modifying single-file codegen path:** Phase 38 must NOT change how `snow_codegen::compile_to_binary` is called. The entry module's Parse and TypeckResult flow to codegen identically to before.
- **Eagerly implementing cross-module type checking:** Phase 38 is about PARSING all files and ORCHESTRATING the build. Cross-module symbol resolution is Phase 39. Do not seed TypeEnv with imports from other modules yet.
- **Changing the CLI interface:** `snowc build <dir>` takes a directory. Do not add a `snowc build <file>` variant in this phase.
- **Storing Parse in ModuleGraph:** The `Parse` type contains a `rowan::GreenNode` which is not `Send` or easily clonable across threads. Keeping parse results in a parallel `Vec` indexed by `ModuleId.0` is simpler and avoids lifetime entanglement with `ModuleGraph`.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| File discovery | New file walker | `discovery::discover_snow_files()` | Already built in Phase 37, tested, handles hidden dirs |
| Module graph construction | New graph builder | `discovery::build_module_graph()` (extended) | Already built in Phase 37, handles cycles, self-imports, toposort |
| Import extraction | Manual CST walking | `discovery::extract_imports()` | Already built in Phase 37, handles both import forms |
| Single-file parse pipeline | New parser entry point | `snow_parser::parse(&source)` | Existing, battle-tested across 37 phases |
| Single-file type checking | New typeck entry point | `snow_typeck::check(&parse)` | Existing, unchanged for entry module |

**Key insight:** Phase 38's new code should be minimal (~50-80 lines of actual new logic). The bulk of the work is refactoring existing functions to expose their intermediate results (parse data) and wiring the multi-file discovery into `build()`. Most logic already exists from Phase 37.

## Common Pitfalls

### Pitfall 1: Double-Parsing Files
**What goes wrong:** `build_module_graph` parses every `.snow` file internally (to extract imports), then `build()` parses `main.snow` again to get the Parse for type checking. This parses files twice, wasting time and creating a subtle inconsistency risk if the file changes between calls (in a hypothetical watch mode).
**Why it happens:** Phase 37 designed `build_module_graph` for graph construction only. It discards parse results after import extraction. Phase 38 needs those parse results.
**How to avoid:** Refactor `build_module_graph` into a new `build_project` function that returns `(ModuleGraph, Vec<ModuleId>, Vec<String>, Vec<Parse>)` -- the graph, compilation order, source strings, and parse results. The existing `build_module_graph` can be preserved as a thin wrapper for backward compatibility (Phase 37 tests).
**Warning signs:** `snow_parser::parse()` appears in both `discovery.rs` AND `main.rs` `build()` for the same file.

### Pitfall 2: Missing Entry Module Check
**What goes wrong:** The module graph is built and contains all modules, but the entry module (`main.snow`) is not found or is not marked `is_entry: true`. The build silently produces a binary without a `main` function, or panics.
**Why it happens:** `path_to_module_name("main.snow")` returns `None` (correct -- it's the entry point), so the entry module is registered as `"Main"` with `is_entry: true`. But if the check for `is_entry` is missing, the wrong module might be compiled.
**How to avoid:** After building the project data, explicitly find the entry module by checking `is_entry: true` in the compilation order. If no entry module exists, produce a clear error: "No 'main.snow' found in project."
**Warning signs:** Tests pass for multi-file projects but `main` function not found in binary.

### Pitfall 3: Parse Errors in Non-Entry Modules Silently Ignored
**What goes wrong:** A `.snow` file with syntax errors is discovered and parsed, but since Phase 38 only type-checks `main.snow`, the parse error in the other file is never reported. The build succeeds despite broken files.
**Why it happens:** The current pipeline only calls `report_diagnostics` for the entry module. Other modules' parse results are stored but not checked for errors.
**How to avoid:** After parsing all files, iterate over ALL parse results and report errors from any file. If any file has parse errors, the build fails. This is consistent with user expectations -- `snowc build` validates the entire project.
**Warning signs:** `utils.snow` has a syntax error, `snowc build` succeeds, user is confused when Phase 39 integration breaks.

### Pitfall 4: Lifetimes and Ownership of Parse + Source Data
**What goes wrong:** `Parse` contains a `rowan::GreenNode`. The source string is borrowed by error reporting. If the source string is dropped before error reporting, the diagnostics produce garbage.
**Why it happens:** Rust ownership rules. The source strings and parse results must outlive all references to them. If they are stored in a Vec that is reorganized or moved, references become invalid.
**How to avoid:** Store `module_sources: Vec<String>` and `module_parses: Vec<Parse>` as owned collections that persist for the entire `build()` function lifetime. Use indices (via `ModuleId.0`) to access them, not references. The `ProjectData` struct owns everything.
**Warning signs:** Compilation errors about lifetimes when trying to return references from functions.

### Pitfall 5: build_module_graph Tests Breaking After Refactor
**What goes wrong:** Phase 37 has 12 tests for `build_module_graph`. Changing its signature breaks all of them.
**Why it happens:** The tests import and call `build_module_graph` directly with its current return type `Result<(ModuleGraph, Vec<ModuleId>), String>`.
**How to avoid:** Keep the existing `build_module_graph` function with its current signature as a convenience wrapper around the new `build_project` function. The tests continue to use `build_module_graph`. New tests use `build_project`.
**Warning signs:** `cargo test -p snowc` fails on existing Phase 37 tests after refactoring.

### Pitfall 6: Regression in Existing E2E Tests
**What goes wrong:** The 90+ existing e2e test fixtures in `tests/e2e/` all create a single `main.snow` in a temp directory and call `snowc build`. After Phase 38's changes to `build()`, some tests start failing.
**Why it happens:** The new code path calls `build_project` (which involves `discover_snow_files`, graph building, toposort) instead of the direct single-file path. If any step in the new pipeline has a bug, ALL existing tests break.
**How to avoid:** The single-file case (directory with only `main.snow`) must be a clean code path through the new pipeline. `discover_snow_files` returns `["main.snow"]`, graph has one module, toposort trivially returns `[Main]`, and the entry module is compiled identically. Run ALL existing tests after the change.
**Warning signs:** Previously passing e2e tests fail with errors about module graph or discovery.

## Code Examples

### Example 1: ProjectData Struct (New Type for build_project Result)
```rust
// In crates/snowc/src/discovery.rs

/// Complete project data after discovery, parsing, and graph construction.
///
/// All Vecs are indexed by ModuleId.0 -- the i-th entry corresponds to
/// the module with ModuleId(i).
pub struct ProjectData {
    /// The module dependency graph with topological ordering.
    pub graph: ModuleGraph,
    /// Modules in compilation order (dependencies before dependents).
    pub compilation_order: Vec<ModuleId>,
    /// Source code for each module (indexed by ModuleId.0).
    pub module_sources: Vec<String>,
    /// Parsed AST for each module (indexed by ModuleId.0).
    pub module_parses: Vec<snow_parser::Parse>,
}
```

### Example 2: build_project Function (Extends build_module_graph)
```rust
// In crates/snowc/src/discovery.rs

/// Build a complete project: discover files, parse all, build dependency graph.
///
/// This is the main entry point for the multi-file build pipeline.
/// Unlike `build_module_graph`, this function retains the per-file
/// Parse results and source strings for downstream compilation phases.
pub fn build_project(project_root: &Path) -> Result<ProjectData, String> {
    let files = discover_snow_files(project_root)?;
    let mut graph = ModuleGraph::new();
    let mut module_sources = Vec::new();
    let mut module_parses = Vec::new();

    // Phase 1: Register all modules, read and parse source files.
    for relative_path in &files {
        let full_path = project_root.join(relative_path);
        let source = std::fs::read_to_string(&full_path)
            .map_err(|e| format!("Failed to read '{}': {}", full_path.display(), e))?;

        let is_entry = relative_path == Path::new("main.snow");
        let name = if is_entry {
            "Main".to_string()
        } else {
            path_to_module_name(relative_path)
                .ok_or_else(|| format!("Cannot determine module name for '{}'", relative_path.display()))?
        };

        let parse = snow_parser::parse(&source);
        let _id = graph.add_module(name, relative_path.clone(), is_entry);

        module_sources.push(source);
        module_parses.push(parse);
    }

    // Phase 2: Build dependency edges from import declarations.
    for id_val in 0..graph.module_count() {
        let id = ModuleId(id_val as u32);
        let tree = module_parses[id_val].tree();
        let imports = extract_imports(&tree);
        let module_name = graph.get(id).name.clone();

        for import_name in imports {
            match graph.resolve(&import_name) {
                None => { /* Unknown import -- skip silently (Phase 39 handles errors) */ }
                Some(dep_id) if dep_id == id => {
                    return Err(format!("Module '{}' cannot import itself", module_name));
                }
                Some(dep_id) => {
                    graph.add_dependency(id, dep_id);
                }
            }
        }
    }

    // Phase 3: Topological sort.
    let compilation_order = module_graph::topological_sort(&graph)
        .map_err(|e: CycleError| format!("Circular dependency: {}", e))?;

    Ok(ProjectData {
        graph,
        compilation_order,
        module_sources,
        module_parses,
    })
}
```

### Example 3: Refactored build_module_graph as Wrapper
```rust
// In crates/snowc/src/discovery.rs

/// Build a module graph from a project directory (convenience wrapper).
///
/// Returns only the graph and compilation order (no parse data).
/// Preserves the Phase 37 API for existing tests.
pub fn build_module_graph(project_root: &Path) -> Result<(ModuleGraph, Vec<ModuleId>), String> {
    let project = build_project(project_root)?;
    Ok((project.graph, project.compilation_order))
}
```

### Example 4: Modified build() Function
```rust
// In crates/snowc/src/main.rs

fn build(
    dir: &Path,
    opt_level: u8,
    emit_llvm: bool,
    output: Option<&Path>,
    target: Option<&str>,
    diag_opts: &DiagnosticOptions,
) -> Result<(), String> {
    // Validate the project directory
    if !dir.exists() {
        return Err(format!("Project directory '{}' does not exist", dir.display()));
    }
    if !dir.is_dir() {
        return Err(format!("'{}' is not a directory", dir.display()));
    }
    let main_snow = dir.join("main.snow");
    if !main_snow.exists() {
        return Err(format!(
            "No 'main.snow' found in '{}'. Snow projects must have a main.snow entry point.",
            dir.display()
        ));
    }

    // Build the project: discover all files, parse, build module graph
    let project = discovery::build_project(dir)?;

    // Find the entry module
    let entry_id = project.compilation_order.iter()
        .copied()
        .find(|id| project.graph.get(*id).is_entry)
        .ok_or("No entry module found in module graph")?;
    let entry_idx = entry_id.0 as usize;

    // Check for parse errors in ALL modules
    let mut has_errors = false;
    for id in &project.compilation_order {
        let idx = id.0 as usize;
        let parse = &project.module_parses[idx];
        let source = &project.module_sources[idx];
        let mod_path = dir.join(&project.graph.get(*id).path);

        // Report parse errors for this module
        for error in parse.errors() {
            has_errors = true;
            // ... report error using existing report_diagnostics pattern
        }
    }

    // Type-check the entry module
    let entry_parse = &project.module_parses[entry_idx];
    let entry_source = &project.module_sources[entry_idx];
    let typeck = snow_typeck::check(entry_parse);

    // Report type-check diagnostics for the entry module
    has_errors |= report_diagnostics(entry_source, &main_snow, entry_parse, &typeck, diag_opts);

    if has_errors {
        return Err("Compilation failed due to errors above.".to_string());
    }

    // Compile to binary (identical pipeline to before)
    let project_name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("output");
    let output_path = match output {
        Some(p) => p.to_path_buf(),
        None => dir.join(project_name),
    };

    if emit_llvm {
        let ll_path = output_path.with_extension("ll");
        snow_codegen::compile_to_llvm_ir(entry_parse, &typeck, &ll_path, target)?;
        eprintln!("  LLVM IR: {}", ll_path.display());
    }

    snow_codegen::compile_to_binary(entry_parse, &typeck, &output_path, opt_level, target, None)?;
    eprintln!("  Compiled: {}", output_path.display());

    Ok(())
}
```

### Example 5: Multi-File E2E Test
```rust
// In crates/snowc/tests/e2e.rs (new test)

#[test]
fn e2e_multi_file_basic() {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let project_dir = temp_dir.path().join("project");
    std::fs::create_dir_all(&project_dir).expect("failed to create project dir");

    // Create a multi-file project (but main.snow doesn't import anything yet)
    std::fs::write(
        project_dir.join("main.snow"),
        "fn main() do\n  IO.puts(\"hello\")\nend\n",
    ).unwrap();
    std::fs::write(
        project_dir.join("utils.snow"),
        "fn helper() do\n  42\nend\n",
    ).unwrap();

    let snowc = find_snowc();
    let output = Command::new(&snowc)
        .args(["build", project_dir.to_str().unwrap()])
        .output()
        .expect("failed to invoke snowc");

    assert!(
        output.status.success(),
        "snowc build failed on multi-file project:\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Run and verify
    let binary = project_dir.join("project");
    let run_output = Command::new(&binary).output().expect("failed to run binary");
    assert!(run_output.status.success());
    assert_eq!(String::from_utf8_lossy(&run_output.stdout), "hello\n");
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Single-file-only `build()` | Multi-file aware `build()` | Phase 38 (this phase) | Pipeline discovers and parses all project files |
| Parse discarded after import extraction | Parse results retained for compilation | Phase 38 | Eliminates double-parsing, enables downstream phases |
| No graph integration in build | `build_project()` call in build pipeline | Phase 38 | Module graph drives compilation order |

**Deprecated/outdated:**
- `build_module_graph` becomes a thin wrapper around `build_project`. Its direct use is for backward compatibility with Phase 37 tests only.

## Open Questions

1. **Should `build_project` live in `discovery.rs` or a new file?**
   - What we know: `discovery.rs` already contains `build_module_graph`, `discover_snow_files`, `extract_imports`, `path_to_module_name`. Adding `build_project` and `ProjectData` here keeps all project-building logic together.
   - What's unclear: The file could grow large as later phases add more project-level logic.
   - Recommendation: Keep it in `discovery.rs` for Phase 38. It is a natural extension of the existing module. If it grows beyond ~400 lines in later phases, extract to a separate file.

2. **Should non-entry modules be type-checked in Phase 38?**
   - What we know: Phase 38's success criteria say "each .snow file is parsed into its own independent AST." It does NOT say "each file is type-checked."
   - What's unclear: Should we type-check non-entry modules independently (without cross-module imports) to catch per-file type errors early?
   - Recommendation: YES, type-check all modules independently. Each module is type-checked with `snow_typeck::check()` (no import context). This catches per-file type errors (bad function signatures, unknown types within the file, etc.) without cross-module concerns. Type errors in any module fail the build. This gives earlier feedback and validates per-module ASTs. However, non-entry modules will have unresolved import errors since stdlib imports are the only ones that work currently -- those errors should be silently filtered or the type-check of non-entry modules should be deferred to Phase 39.
   - **Revised recommendation:** Only type-check the entry module in Phase 38. Non-entry modules are parsed (catching parse errors) but NOT type-checked. Type checking of non-entry modules requires import resolution (Phase 39). Attempting to type-check a module that has `import Utils` without providing Utils's exports would produce spurious "unknown module" errors. Parse-only is the safe approach.

3. **How to handle the `report_diagnostics` function for multiple files?**
   - What we know: The current `report_diagnostics` function takes a single `source`, `path`, `parse`, and `typeck`. For multiple files, it needs to be called per-file.
   - What's unclear: Should parse error reporting for non-entry modules use the same function or a simplified version?
   - Recommendation: Extract a `report_parse_errors` helper that only checks parse errors (no type-check errors). Call it for every module. Call the full `report_diagnostics` only for the entry module (which is the only one type-checked in Phase 38).

4. **Should `build_module_graph` backward compatibility wrapper stay?**
   - What we know: Phase 37 has 6 integration tests using `build_module_graph` directly.
   - Recommendation: YES, keep `build_module_graph` as a wrapper that calls `build_project` and returns `(graph, compilation_order)`. This preserves all Phase 37 tests without modification. Zero-cost abstraction -- it just destructures the ProjectData.

## Sources

### Primary (HIGH confidence)
- **Direct codebase analysis** of:
  - `crates/snowc/src/main.rs` -- Current `build()` function (lines 196-263), `report_diagnostics` (lines 271-329), CLI structure
  - `crates/snowc/src/discovery.rs` -- Phase 37 `build_module_graph`, `extract_imports`, `discover_snow_files` (full file, 377 lines)
  - `crates/snow-common/src/module_graph.rs` -- ModuleGraph, ModuleId, topological_sort (full file, 357 lines)
  - `crates/snow-parser/src/lib.rs` -- Parse struct, `parse()` entry point
  - `crates/snow-typeck/src/lib.rs` -- TypeckResult struct, `check()` entry point
  - `crates/snow-codegen/src/lib.rs` -- `compile_to_binary`, `lower_to_mir_module` entry points
  - `crates/snowc/tests/e2e.rs` -- E2E test pattern with temp directories
  - `tests/e2e/*.snow` -- 90+ existing test fixtures

### Secondary (HIGH confidence)
- `.planning/research/ARCHITECTURE.md` -- Multi-file pipeline architecture plan (2026-02-09)
- `.planning/research/STACK.md` -- Module system stack research (2026-02-09)
- `.planning/research/PITFALLS.md` -- Module system pitfalls (2026-02-09)
- `.planning/REQUIREMENTS.md` -- INFRA-03, INFRA-04, DIAG-03 requirement definitions
- `.planning/ROADMAP.md` -- Phase 38 scope and success criteria
- `.planning/phases/37-module-graph-foundation/37-01-SUMMARY.md` -- Phase 37 Plan 01 results
- `.planning/phases/37-module-graph-foundation/37-02-SUMMARY.md` -- Phase 37 Plan 02 results

### Tertiary (N/A)
- No web search needed. All information from direct codebase analysis and existing planning documents.

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH - Zero new dependencies. All integration points verified by reading actual source code.
- Architecture: HIGH - The refactoring pattern (extend existing function, keep backward compat wrapper) is straightforward and verified against Phase 37 code.
- Pitfalls: HIGH - Double-parsing and backward compatibility risks identified from actual code analysis. E2E test regression risk is concrete (90+ tests exist).

**Research date:** 2026-02-09
**Valid until:** 2026-03-11 (30 days -- stable domain, internal compiler refactoring with no external dependencies)
