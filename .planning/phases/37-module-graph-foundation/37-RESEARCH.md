# Phase 37: Module Graph Foundation - Research

**Researched:** 2026-02-09
**Domain:** File discovery, module naming convention, dependency graph construction, topological sort, cycle detection
**Confidence:** HIGH

## Summary

Phase 37 lays the foundation for multi-file compilation by building three capabilities: (1) recursive discovery of `.snow` files in a project directory, (2) deterministic mapping of file paths to module names using PascalCase convention, and (3) construction of a dependency graph from import declarations with topological ordering and cycle detection. This phase produces pure infrastructure with NO changes to the existing compilation pipeline -- it adds new data structures and algorithms that later phases will integrate.

The compiler already has significant infrastructure to build on. The `collect_snow_files_recursive` function in `snowc/src/main.rs` (used by `snowc fmt`) already walks directories to find `.snow` files. The parser already handles `import Foo.Bar` and `from Foo.Bar import name1, name2` syntax, producing `IMPORT_DECL` and `FROM_IMPORT_DECL` AST nodes with `Path::segments()` accessors. The `snow-common` crate already holds shared primitives (`Span`, `Token`, error types) and is the natural home for `ModuleGraph`.

The critical algorithm is Kahn's topological sort (~40 lines of Rust), which simultaneously determines compilation order and detects circular dependencies. This is preferred over petgraph (10K+ line dependency) because the module graph is a simple DAG with typically 5-50 nodes. Kahn's algorithm handles diamond dependencies correctly (A imports B and C, both import D) and produces deterministic output when ties are broken alphabetically.

**Primary recommendation:** Build `ModuleId`, `ModuleInfo`, and `ModuleGraph` in `snow-common`. Implement file discovery and path-to-name mapping in `snowc`. Implement Kahn's algorithm with cycle path extraction. Test with directory fixtures and snapshot tests. Do NOT modify the existing build pipeline in this phase.

## Standard Stack

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| std::fs | stable | Directory traversal, file discovery | Already used by `collect_snow_files_recursive` in `snowc` |
| std::path | stable | Path manipulation, module name derivation | Already used throughout the compiler |
| std::collections::VecDeque | stable | BFS queue for Kahn's algorithm | Standard library, zero-cost |
| rustc-hash | 2 (workspace) | FxHashMap/FxHashSet for name lookups | Already in workspace deps, used by typeck |
| snow-parser | local | Parse files to extract import declarations | Already exists, provides `ImportDecl`, `FromImportDecl`, `Path` |
| rowan | 0.16 (workspace) | CST traversal for extracting imports from parsed files | Already used by parser |

### Supporting
| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| insta | 1.46 (workspace) | Snapshot testing for module graph output | Testing topo sort order, cycle detection messages |
| tempfile | 3 (dev-dep in snowc) | Creating test project directories | e2e tests for file discovery |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| Hand-written Kahn's | petgraph | petgraph is 10K+ lines for a 40-line algorithm. Over-engineered for 5-50 node DAGs. |
| String module names | Interned strings (lasso) | Module names are short, few in number. Interning adds lifetime complexity for negligible benefit at this scale. |

**Installation:**
No new dependencies needed. Everything is already in the workspace.

## Architecture Patterns

### Recommended Project Structure
```
crates/snow-common/src/
  lib.rs              # Add: pub mod module_graph;
  module_graph.rs     # NEW: ModuleId, ModuleInfo, ModuleGraph, topological_sort

crates/snowc/src/
  main.rs             # Add: discover_modules() function, integrate into build()
  discovery.rs        # NEW: file discovery, path-to-name mapping (extracted from main.rs)
```

### Pattern 1: ModuleGraph as Shared Data Structure in snow-common
**What:** The `ModuleGraph` struct lives in `snow-common` so all downstream crates (parser, typeck, codegen, snowc) can reference it.
**When to use:** This is the core pattern for this phase.
**Example:**
```rust
// Source: Derived from .planning/research/STACK.md patterns + codebase analysis

use std::path::PathBuf;
use rustc_hash::FxHashMap;

/// Unique identifier for a module within a compilation unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ModuleId(pub u32);

/// Metadata about a single module (source file).
#[derive(Debug, Clone)]
pub struct ModuleInfo {
    /// Unique ID (index into ModuleGraph.modules).
    pub id: ModuleId,
    /// Fully qualified module name (e.g., "Math.Vector").
    pub name: String,
    /// Filesystem path relative to project root (e.g., "math/vector.snow").
    pub path: PathBuf,
    /// Module IDs this module depends on (derived from import declarations).
    pub dependencies: Vec<ModuleId>,
    /// Whether this is the entry point (main.snow).
    pub is_entry: bool,
}

/// The dependency graph of all modules in a project.
#[derive(Debug)]
pub struct ModuleGraph {
    /// All modules, indexed by ModuleId.
    pub modules: Vec<ModuleInfo>,
    /// Name-to-ID lookup for resolving import paths.
    name_to_id: FxHashMap<String, ModuleId>,
}

impl ModuleGraph {
    pub fn new() -> Self {
        Self {
            modules: Vec::new(),
            name_to_id: FxHashMap::default(),
        }
    }

    /// Add a module, returning its ID.
    pub fn add_module(&mut self, name: String, path: PathBuf, is_entry: bool) -> ModuleId {
        let id = ModuleId(self.modules.len() as u32);
        self.name_to_id.insert(name.clone(), id);
        self.modules.push(ModuleInfo {
            id,
            name,
            path,
            dependencies: Vec::new(),
            is_entry,
        });
        id
    }

    /// Resolve a module name to its ID.
    pub fn resolve(&self, name: &str) -> Option<ModuleId> {
        self.name_to_id.get(name).copied()
    }

    /// Add a dependency edge: `from` depends on `to`.
    pub fn add_dependency(&mut self, from: ModuleId, to: ModuleId) {
        self.modules[from.0 as usize].dependencies.push(to);
    }
}
```

### Pattern 2: File Path to Module Name Convention
**What:** Deterministic mapping from filesystem paths to PascalCase module names.
**When to use:** During file discovery, before dependency graph construction.
**Example:**
```rust
// Source: Derived from REQUIREMENTS.md INFRA-02 + STACK.md convention

/// Convert a relative file path to a module name.
///
/// Convention:
/// - `math/vector.snow` -> `Math.Vector`
/// - `utils.snow` -> `Utils`
/// - `math/linear_algebra.snow` -> `Math.LinearAlgebra`
/// - `main.snow` -> None (entry point, not a module)
///
/// Each path segment is converted from snake_case to PascalCase.
pub fn path_to_module_name(relative_path: &Path) -> Option<String> {
    let stem = relative_path.file_stem()?.to_str()?;

    // main.snow is the entry point, not a module
    if stem == "main" && relative_path.parent().map_or(true, |p| p == Path::new("")) {
        return None;
    }

    let mut segments = Vec::new();

    // Directory components become module path segments
    if let Some(parent) = relative_path.parent() {
        for component in parent.components() {
            if let std::path::Component::Normal(name) = component {
                segments.push(to_pascal_case(name.to_str()?));
            }
        }
    }

    // File stem becomes the last segment
    segments.push(to_pascal_case(stem));

    Some(segments.join("."))
}

/// Convert a snake_case string to PascalCase.
///
/// "vector" -> "Vector"
/// "linear_algebra" -> "LinearAlgebra"
fn to_pascal_case(s: &str) -> String {
    s.split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => {
                    let mut result = first.to_uppercase().to_string();
                    result.extend(chars);
                    result
                }
            }
        })
        .collect()
}
```

### Pattern 3: Kahn's Algorithm for Topological Sort with Cycle Detection
**What:** BFS-based topological sort that simultaneously determines compilation order and detects cycles.
**When to use:** After all files are discovered, parsed for imports, and the dependency graph is built.
**Example:**
```rust
// Source: Derived from .planning/research/STACK.md + standard algorithm

use std::collections::VecDeque;

/// Error returned when the module graph contains a cycle.
#[derive(Debug, Clone)]
pub struct CycleError {
    /// The cycle path as module names, e.g., ["A", "B", "C", "A"].
    pub cycle_path: Vec<String>,
}

/// Topological sort using Kahn's algorithm.
///
/// Returns modules in dependency order (dependencies before dependents).
/// If the graph contains cycles, returns an error with the cycle path.
///
/// Ties are broken alphabetically for deterministic output.
pub fn topological_sort(graph: &ModuleGraph) -> Result<Vec<ModuleId>, CycleError> {
    let n = graph.modules.len();
    // in_degree[i] = number of modules that module i depends on
    // (i.e., how many of i's dependencies haven't been processed yet)
    //
    // NOTE: For Kahn's, we want in-degree in the REVERSE direction:
    // in_degree[i] = number of modules that IMPORT module i
    // Actually no -- we want compilation order where dependencies come first.
    // A module with no dependencies (in_degree 0 in the dep graph) compiles first.
    // in_degree here = number of modules this module depends on that haven't compiled yet.
    //
    // Wait -- standard Kahn's: in_degree = number of incoming edges.
    // In our graph, an edge from A to B means "A depends on B" (A imports B).
    // For compilation order, B must come before A.
    // So the "topological order" of the REVERSED graph is what we want.
    // Equivalently: in_degree[i] = count of i's dependencies (outgoing edges).
    // Start with modules that have NO dependencies (in_degree = 0).

    let mut in_degree = vec![0u32; n];
    for module in &graph.modules {
        in_degree[module.id.0 as usize] = module.dependencies.len() as u32;
    }

    // Seed queue with modules that have no dependencies
    let mut queue: VecDeque<ModuleId> = (0..n)
        .filter(|&i| in_degree[i] == 0)
        .map(|i| ModuleId(i as u32))
        .collect();

    // Sort initial queue alphabetically for determinism
    let mut initial: Vec<ModuleId> = queue.drain(..).collect();
    initial.sort_by(|a, b| graph.modules[a.0 as usize].name.cmp(&graph.modules[b.0 as usize].name));
    queue.extend(initial);

    let mut order = Vec::with_capacity(n);

    while let Some(id) = queue.pop_front() {
        order.push(id);

        // Find modules that depend on `id` and decrement their in-degree
        let mut newly_ready = Vec::new();
        for module in &graph.modules {
            if module.dependencies.contains(&id) {
                in_degree[module.id.0 as usize] -= 1;
                if in_degree[module.id.0 as usize] == 0 {
                    newly_ready.push(module.id);
                }
            }
        }

        // Sort for deterministic order
        newly_ready.sort_by(|a, b| {
            graph.modules[a.0 as usize].name.cmp(&graph.modules[b.0 as usize].name)
        });
        queue.extend(newly_ready);
    }

    if order.len() == n {
        Ok(order)
    } else {
        // Modules with remaining in_degree > 0 form cycles
        let cycle_path = extract_cycle_path(graph, &in_degree);
        Err(CycleError { cycle_path })
    }
}

/// Extract a human-readable cycle path from remaining unprocessed modules.
fn extract_cycle_path(graph: &ModuleGraph, in_degree: &[u32]) -> Vec<String> {
    // Find a module in the cycle and follow dependency edges
    let start = in_degree.iter()
        .position(|&d| d > 0)
        .expect("cycle must have at least one node");

    let mut path = vec![graph.modules[start].name.clone()];
    let mut current = start;
    let mut visited = FxHashSet::default();
    visited.insert(current);

    loop {
        // Follow a dependency edge to another unprocessed module
        let next = graph.modules[current].dependencies.iter()
            .find(|&&dep| in_degree[dep.0 as usize] > 0)
            .map(|dep| dep.0 as usize);

        match next {
            Some(n) if visited.contains(&n) => {
                // Found the cycle -- add the closing module name
                path.push(graph.modules[n].name.clone());
                break;
            }
            Some(n) => {
                visited.insert(n);
                path.push(graph.modules[n].name.clone());
                current = n;
            }
            None => break, // shouldn't happen if in_degree > 0
        }
    }

    path
}
```

### Pattern 4: Extracting Imports from Parsed Files
**What:** Walk the AST of a parsed file to extract import declarations and resolve them to module names.
**When to use:** After parsing all files, before building the dependency graph.
**Example:**
```rust
// Source: Derived from snow-parser AST API analysis

use snow_parser::ast::item::{SourceFile, Item, ImportDecl, FromImportDecl};
use snow_parser::ast::AstNode;

/// Extract all module names that a source file imports.
///
/// Scans for `import Foo.Bar` and `from Foo.Bar import name1, name2`
/// declarations and returns the set of imported module names.
pub fn extract_imports(source_file: &SourceFile) -> Vec<String> {
    let mut imports = Vec::new();

    for item in source_file.items() {
        match item {
            Item::ImportDecl(decl) => {
                if let Some(path) = decl.module_path() {
                    let segments = path.segments();
                    if !segments.is_empty() {
                        imports.push(segments.join("."));
                    }
                }
            }
            Item::FromImportDecl(decl) => {
                if let Some(path) = decl.module_path() {
                    let segments = path.segments();
                    if !segments.is_empty() {
                        imports.push(segments.join("."));
                    }
                }
            }
            _ => {}
        }
    }

    imports
}
```

### Anti-Patterns to Avoid
- **Modifying the existing build pipeline:** Phase 37 adds infrastructure only. Do NOT change how `snowc build` invokes parse/typeck/codegen yet. That is Phase 38.
- **Using HashMap iteration order for topo sort:** Produces non-deterministic compilation order. Always sort alphabetically for tie-breaking.
- **Putting module graph construction in snowc only:** The `ModuleGraph` type must be in `snow-common` so typeck and codegen can use it in later phases. The construction logic (discovery, parsing imports) lives in `snowc`.
- **Treating `main.snow` as a module:** `main.snow` is the entry point. It is NOT importable. It should have `is_entry: true` and no module name. Other modules cannot `import Main`.
- **DFS with simple visited set for cycle detection:** A naive DFS with one visited set incorrectly flags diamond dependencies as cycles. Kahn's algorithm handles diamonds correctly by design.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| File discovery | Custom recursive walker | Adapt `collect_snow_files_recursive` from `snowc/src/main.rs` | Already exists, already sorts results, handles edge cases |
| Hash maps | Custom hash map | `rustc_hash::FxHashMap` | Already in workspace, faster than std HashMap for string keys |
| CST import extraction | Manual tree walking | `SourceFile::items()` + `ImportDecl::module_path()` + `Path::segments()` | Typed AST accessors already exist and are tested |

**Key insight:** Nearly all infrastructure for Phase 37 already exists in some form. File discovery is in `snowc` (fmt command). Import parsing is in `snow-parser`. The only truly new code is `ModuleGraph`, `path_to_module_name`, and `topological_sort`.

## Common Pitfalls

### Pitfall 1: Kahn's In-Degree Direction Confusion
**What goes wrong:** Standard Kahn's algorithm starts with nodes that have no incoming edges (in-degree 0). In a dependency graph where "A imports B" means A depends on B, B must be compiled first. The "in-degree" for compilation order is the number of unresolved dependencies, NOT the number of dependents.
**Why it happens:** Confusion between "A -> B means A depends on B" vs "A -> B means A provides to B". The edge direction determines whether you count dependencies or dependents as in-degree.
**How to avoid:** Define clearly: `module.dependencies` is the list of modules this module IMPORTS. A module with empty `dependencies` has in-degree 0 and compiles first. When module X is processed, decrement in-degree of every module that has X in its `dependencies` list.
**Warning signs:** Entry module (main.snow) compiles first instead of last. Leaf modules (no imports) compile last instead of first.

### Pitfall 2: Path-to-Name Edge Cases
**What goes wrong:** The path-to-module-name mapping fails on edge cases: files with leading underscores (`_internal.snow`), files in deeply nested directories, files with hyphens (`my-module.snow`), or platform-specific path separators.
**Why it happens:** Only testing with simple cases like `math/vector.snow`.
**How to avoid:** Define and test edge cases explicitly: (1) `main.snow` in root -> None (entry point), (2) `utils.snow` -> `Utils`, (3) `math/vector.snow` -> `Math.Vector`, (4) `math/linear_algebra.snow` -> `Math.LinearAlgebra`, (5) deeply nested `a/b/c/d.snow` -> `A.B.C.D`. Reject invalid names (hyphens, leading digits) with clear errors.
**Warning signs:** Module names contain path separators or file extensions.

### Pitfall 3: Diamond Dependencies Flagged as Cycles
**What goes wrong:** A valid diamond dependency (A imports B and C, B imports D, C imports D) is incorrectly reported as a circular dependency.
**Why it happens:** Using DFS with a single boolean "visited" set. When D is reached via both B and C paths, the second visit looks like a back edge. Kahn's algorithm avoids this entirely because it counts in-degrees, not visited flags.
**How to avoid:** Use Kahn's algorithm (BFS-based), not DFS-based toposort. Kahn's naturally handles diamonds: D's in-degree is 2 (B and C depend on it), so D is processed after both B and C are ready.
**Warning signs:** Simple multi-module projects fail with "circular dependency" when there are no actual cycles.

### Pitfall 4: Import Names vs Module Names Mismatch
**What goes wrong:** A Snow file writes `import Math.Vector` but the file is at `math/vector.snow`. The import uses PascalCase (`Math.Vector`) but the file path uses snake_case (`math/vector`). If the resolution logic does case-sensitive string comparison without converting both to the same convention, imports fail to resolve.
**Why it happens:** The user writes PascalCase import names (language convention) but the filesystem uses lowercase/snake_case. These must be bridged.
**How to avoid:** The `path_to_module_name` function converts file paths to PascalCase module names at discovery time. Import resolution then compares PascalCase-to-PascalCase. The canonical form is always PascalCase (e.g., `Math.Vector`), stored in `ModuleInfo.name`.
**Warning signs:** "Unknown module 'Math.Vector'" when `math/vector.snow` exists.

### Pitfall 5: Non-Deterministic Topological Sort Order
**What goes wrong:** Two developers get different compilation orders for the same project because `HashMap` iteration or `read_dir` order varies by platform.
**Why it happens:** `std::fs::read_dir` does not guarantee ordering. If the topological sort tie-breaks using insertion order, different file discovery orders produce different compilation orders.
**How to avoid:** Sort discovered files by path before processing. Sort the initial zero-in-degree queue alphabetically. Sort newly ready modules alphabetically when adding to the BFS queue. This ensures deterministic output across all platforms.
**Warning signs:** Tests pass on macOS but fail on Linux. CI produces different error messages than local builds.

### Pitfall 6: Self-Import Not Detected
**What goes wrong:** A file `math.snow` contains `import Math`. This creates a self-dependency that Kahn's algorithm detects as a cycle, but the error message says "Circular dependency: Math -> Math" which is confusing.
**Why it happens:** Self-imports are a degenerate case of circular dependency.
**How to avoid:** Check for self-imports during dependency edge construction, before running toposort. Produce a specific error: "Module 'Math' cannot import itself" rather than a generic cycle error.
**Warning signs:** Generic cycle error for self-import instead of specific diagnostic.

## Code Examples

### File Discovery (Adapted from Existing Code)
```rust
// Source: Adapted from snowc/src/main.rs collect_snow_files_recursive

use std::path::{Path, PathBuf};

/// Discover all .snow files in a project directory.
///
/// Returns paths relative to `project_root`, sorted for determinism.
/// Excludes hidden directories (starting with '.') and build artifacts.
pub fn discover_snow_files(project_root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    discover_recursive(project_root, project_root, &mut files)?;
    files.sort(); // Deterministic order
    Ok(files)
}

fn discover_recursive(
    root: &Path,
    dir: &Path,
    files: &mut Vec<PathBuf>,
) -> Result<(), String> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| format!("Failed to read directory '{}': {}", dir.display(), e))?;

    for entry in entries {
        let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
        let path = entry.path();

        // Skip hidden directories
        if path.file_name()
            .and_then(|n| n.to_str())
            .map_or(false, |n| n.starts_with('.'))
        {
            continue;
        }

        if path.is_dir() {
            discover_recursive(root, &path, files)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("snow") {
            // Store relative path from project root
            let relative = path.strip_prefix(root)
                .map_err(|_| format!("Path '{}' not under root '{}'", path.display(), root.display()))?;
            files.push(relative.to_path_buf());
        }
    }

    Ok(())
}
```

### Building the Module Graph
```rust
// Source: Synthesis of STACK.md patterns + parser API

/// Build a ModuleGraph from a project directory.
///
/// 1. Discover all .snow files
/// 2. Map paths to module names
/// 3. Parse each file to extract imports
/// 4. Build dependency edges
/// 5. Topologically sort
pub fn build_module_graph(
    project_root: &Path,
) -> Result<(ModuleGraph, Vec<(ModuleId, String)>), String> {
    let files = discover_snow_files(project_root)?;

    let mut graph = ModuleGraph::new();
    let mut sources: Vec<(ModuleId, String)> = Vec::new();

    // Phase 1: Register all modules
    for relative_path in &files {
        let full_path = project_root.join(relative_path);
        let source = std::fs::read_to_string(&full_path)
            .map_err(|e| format!("Failed to read '{}': {}", full_path.display(), e))?;

        let is_entry = relative_path == Path::new("main.snow");
        let module_name = if is_entry {
            "Main".to_string() // Entry point gets special name
        } else {
            path_to_module_name(relative_path)
                .ok_or_else(|| format!("Cannot derive module name from '{}'", relative_path.display()))?
        };

        let id = graph.add_module(module_name, relative_path.clone(), is_entry);
        sources.push((id, source));
    }

    // Phase 2: Parse files and extract dependency edges
    for (id, source) in &sources {
        let parse = snow_parser::parse(source);
        let tree = parse.tree();
        let imports = extract_imports(&tree);

        for import_name in imports {
            match graph.resolve(&import_name) {
                Some(dep_id) => {
                    // Check for self-import
                    if dep_id == *id {
                        let module_name = &graph.modules[id.0 as usize].name;
                        return Err(format!(
                            "Module '{}' cannot import itself",
                            module_name
                        ));
                    }
                    graph.add_dependency(*id, dep_id);
                }
                None => {
                    // Unknown module -- skip for now, Phase 39 handles error reporting
                    // For Phase 37, we only care about building the graph from known modules
                }
            }
        }
    }

    Ok((graph, sources))
}
```

### Deterministic Topological Sort with Cycle Path
```rust
// Source: Standard Kahn's algorithm, adapted for Snow's ModuleGraph

/// Return type: compilation order (dependencies before dependents)
pub fn compilation_order(graph: &ModuleGraph) -> Result<Vec<ModuleId>, CycleError> {
    topological_sort(graph)
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Single file compilation | Multi-file with module graph | Phase 37 (this phase) | Foundation for entire v1.8 module system |
| `collect_snow_files_recursive` (fmt only) | Reusable `discover_snow_files` | Phase 37 | File discovery shared between fmt and build commands |
| No module identity | `ModuleId` + `ModuleInfo` + `ModuleGraph` | Phase 37 | Every downstream phase can reference modules by ID |

**Deprecated/outdated:**
- Nothing deprecated. Phase 37 adds new infrastructure without changing existing code.

## Open Questions

1. **Should `main.snow` have module name "Main" or no module name?**
   - What we know: `main.snow` is the entry point, not importable by other modules. The roadmap says "not treated as a module."
   - What's unclear: Does `main.snow` get a `ModuleId` in the graph? It needs to be in the graph for dependency edges (main imports other modules), but it should not be importable.
   - Recommendation: Give `main.snow` a `ModuleId` with `is_entry: true` and name `"Main"`. It participates in the dependency graph (it imports other modules) but other modules cannot import it. The topological sort places it LAST (after all its dependencies). Attempting to import `Main` produces "cannot import the entry module" error (validated in Phase 39).

2. **Should stdlib modules (String, IO, List, etc.) appear in the ModuleGraph?**
   - What we know: Currently, stdlib modules are hardcoded in `infer.rs::stdlib_modules()`. They are not `.snow` files.
   - What's unclear: Should the module graph include them as virtual modules?
   - Recommendation: NO. Stdlib modules are a type-checker concern, not a module graph concern. The module graph only tracks `.snow` files in the project directory. Stdlib imports in the dependency graph should be silently ignored (they don't resolve to any `.snow` file). Phase 39 handles the integration between file-based modules and stdlib modules.

3. **What about `math.snow` and `math/` directory coexisting?**
   - What we know: If both `math.snow` and `math/vector.snow` exist, `math.snow` maps to module `Math` and `math/vector.snow` maps to `Math.Vector`. This is valid (parent module + child modules).
   - What's unclear: Does `Math.Vector` automatically see `Math`'s exports? Is there an implicit relationship?
   - Recommendation: NO implicit relationship. `math.snow` and `math/vector.snow` are independent modules. `Math.Vector` must explicitly `import Math` to use its exports. This keeps the dependency graph explicit and avoids hidden coupling.

4. **How to handle import of unknown module names (typos, non-existent files)?**
   - What we know: In Phase 37, the graph is built from discovered files. An import referencing a module that has no corresponding file should be handled.
   - Recommendation: In Phase 37, silently skip unresolvable imports when building graph edges. The module still compiles (it just doesn't have that dependency edge). Phase 39 adds proper "unknown module" error diagnostics.

## Sources

### Primary (HIGH confidence)
- **Direct codebase analysis** of all 11 Snow compiler crates (70,501 lines):
  - `snowc/src/main.rs` - Existing `collect_snow_files_recursive`, `build()` pipeline
  - `snow-parser/src/ast/item.rs` - `ImportDecl`, `FromImportDecl`, `Path` AST accessors
  - `snow-parser/src/parser/items.rs` - Import parsing implementation
  - `snow-common/src/lib.rs` - Shared primitives location
  - `snow-codegen/src/mir/mod.rs` - MirModule structure
  - `snow-typeck/src/lib.rs` - TypeckResult, pipeline entry point

### Secondary (HIGH confidence)
- `.planning/research/STACK.md` - Module system stack research (2026-02-09)
- `.planning/research/ARCHITECTURE.md` - Multi-file pipeline architecture (2026-02-09)
- `.planning/research/PITFALLS.md` - Module system pitfalls (2026-02-09)
- `.planning/research/SUMMARY.md` - Executive summary of module research (2026-02-09)
- `.planning/REQUIREMENTS.md` - v1.8 requirements (INFRA-01, INFRA-02, INFRA-05, IMPORT-03, IMPORT-04, IMPORT-05)

### Tertiary (N/A)
- No web search was needed. All information comes from direct codebase analysis and the existing research documents.

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH - Direct codebase analysis confirms all integration points. Zero new dependencies needed.
- Architecture: HIGH - ModuleGraph in snow-common, Kahn's toposort, path-to-name convention are all well-understood patterns confirmed by existing research.
- Pitfalls: HIGH - Critical pitfalls (in-degree direction, diamond deps, non-determinism, self-import) identified from algorithm analysis and codebase study.

**Research date:** 2026-02-09
**Valid until:** 2026-03-11 (30 days -- stable domain, no external dependencies to change)
