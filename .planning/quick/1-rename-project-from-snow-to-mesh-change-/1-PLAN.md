---
phase: quick
plan: 1
type: execute
wave: 1
depends_on: []
files_modified: ["**"]
autonomous: true
must_haves:
  truths:
    - "cargo build succeeds with all crates named mesh-*"
    - "cargo test passes with all references updated"
    - "meshc binary is produced instead of snowc"
    - "All .snow test files renamed to .mpl"
    - "LICENSE says Mesh Language Project"
  artifacts:
    - path: "Cargo.toml"
      contains: "mesh-common"
    - path: "crates/meshc/Cargo.toml"
      contains: 'name = "meshc"'
    - path: "crates/mesh-rt/Cargo.toml"
      contains: 'name = "mesh-rt"'
  key_links:
    - from: "crates/meshc/Cargo.toml"
      to: "crates/mesh-*/Cargo.toml"
      via: "path dependencies"
      pattern: 'mesh-\w+ = \{ path'
---

<objective>
Rename the entire project from Snow to Mesh: crate names, binary name, file extension (.snow -> .mpl), runtime symbols (snow_* -> mesh_*), Rust types (Snow* -> Mesh*), manifest filenames (snow.toml -> mesh.toml), documentation, and LICENSE.

Purpose: Complete brand rename from Snow to Mesh across the entire codebase.
Output: Fully renamed project that builds and passes tests under the Mesh name.
</objective>

<execution_context>
@/Users/sn0w/.claude/get-shit-done/workflows/execute-plan.md
@/Users/sn0w/.claude/get-shit-done/templates/summary.md
</execution_context>

<context>
This is a Rust workspace with 11 crates under /crates/:
- snow-common, snow-lexer, snow-parser, snow-typeck, snow-codegen, snow-rt, snow-pkg, snow-lsp, snow-fmt, snow-repl, snowc

The rename touches every layer:
1. Cargo.toml package names and dependency paths (11 crates + workspace root)
2. Directory names (crates/snow-* -> crates/mesh-*, crates/snowc -> crates/meshc)
3. Rust `use snow_*` imports across all source files (~30+ files)
4. Runtime C ABI symbols: `snow_*` extern functions (~234 in snow-rt, ~1214 refs in codegen, ~107 in repl)
5. Rust struct types: SnowString, SnowResult, SnowOption, SnowJson, SnowBackend, etc. (~583 refs)
6. File extension references: `.snow` -> `.mpl` in source code (~200+ refs in discovery, main, tests, etc.)
7. Manifest filenames: `snow.toml` -> `mesh.toml`, `snow.lock` -> `mesh.lock`, `.snow/` -> `.mesh/`
8. Test fixture files: 145 .snow files -> .mpl
9. Snapshot files: ~212 .snap files may contain `snow` references
10. REPL history file: `.snow_repl_history` -> `.mesh_repl_history`
11. VSCode extension: editors/vscode-snow/ (package.json, extension.ts, tmLanguage, etc.)
12. LICENSE copyright
13. .gitignore references
14. Doc comments and string literals throughout

CRITICAL ORDERING: File contents must be updated BEFORE directories are renamed, because
`git mv` + content changes in the same commit work cleanly, but sed-in-place on moved files
requires the new paths. Strategy: update ALL file contents first (using old paths), then
rename directories.

CRITICAL: The `snow_*` C ABI function names (extern "C" fn snow_*) are internal linker
symbols. They must be renamed to `mesh_*` consistently across snow-rt (definitions),
snow-codegen (string literals referencing them), and snow-repl (JIT symbol registration).
The codegen crate has ~1214 string literal references like `"snow_gc_alloc"` that must become
`"mesh_gc_alloc"`. The runtime has ~234 function definitions. The repl has ~107 symbol
registrations. All must match exactly.
</context>

<tasks>

<task type="auto">
  <name>Task 1: Update all Cargo.toml files (package names and dependency references)</name>
  <files>
    Cargo.toml
    crates/snow-common/Cargo.toml
    crates/snow-lexer/Cargo.toml
    crates/snow-parser/Cargo.toml
    crates/snow-typeck/Cargo.toml
    crates/snow-codegen/Cargo.toml
    crates/snow-rt/Cargo.toml
    crates/snow-pkg/Cargo.toml
    crates/snow-lsp/Cargo.toml
    crates/snow-fmt/Cargo.toml
    crates/snow-repl/Cargo.toml
    crates/snowc/Cargo.toml
  </files>
  <action>
    Update ALL Cargo.toml files. This must happen while directories still have old names.

    1. Root Cargo.toml: Replace all workspace member paths:
       - `"crates/snow-common"` -> `"crates/mesh-common"`
       - `"crates/snow-lexer"` -> `"crates/mesh-lexer"`
       - `"crates/snow-parser"` -> `"crates/mesh-parser"`
       - `"crates/snow-typeck"` -> `"crates/mesh-typeck"`
       - `"crates/snow-rt"` -> `"crates/mesh-rt"`
       - `"crates/snow-codegen"` -> `"crates/mesh-codegen"`
       - `"crates/snowc"` -> `"crates/meshc"`
       - `"crates/snow-pkg"` -> `"crates/mesh-pkg"`
       - `"crates/snow-lsp"` -> `"crates/mesh-lsp"`
       - `"crates/snow-fmt"` -> `"crates/mesh-fmt"`
       - `"crates/snow-repl"` -> `"crates/mesh-repl"`

    2. Each crate's Cargo.toml:
       - `name = "snow-*"` -> `name = "mesh-*"` (and `"snowc"` -> `"meshc"`)
       - All `snow-* = { path = "../snow-*" }` -> `mesh-* = { path = "../mesh-*" }`
       - snowc specifically: `snow-common` -> `mesh-common`, etc. for all 9 dependencies

    Do NOT rename directories yet. The Cargo.toml files will temporarily point to
    not-yet-existing paths -- that is fine, they will be correct after Task 5 renames dirs.
  </action>
  <verify>
    Grep all Cargo.toml files for any remaining "snow" references:
    `grep -rn 'snow' Cargo.toml crates/*/Cargo.toml` should return nothing.
  </verify>
  <done>All 12 Cargo.toml files updated with mesh-* names and paths. No "snow" references remain in any Cargo.toml.</done>
</task>

<task type="auto">
  <name>Task 2: Update all Rust source code references (imports, types, symbols, strings, docs)</name>
  <files>
    All .rs files under crates/
  </files>
  <action>
    This is the largest task. Apply find-and-replace across ALL .rs files in crates/.
    Files still live at their OLD paths (crates/snow-*/). Apply replacements in this order
    to avoid partial matches:

    **A. Rust crate imports (use statements and extern crate):**
    - `snow_common` -> `mesh_common` (Rust uses underscores for crate names with hyphens)
    - `snow_lexer` -> `mesh_lexer`
    - `snow_parser` -> `mesh_parser`
    - `snow_typeck` -> `mesh_typeck`
    - `snow_codegen` -> `mesh_codegen`
    - `snow_rt` -> `mesh_rt`
    - `snow_pkg` -> `mesh_pkg`
    - `snow_lsp` -> `mesh_lsp`
    - `snow_fmt` -> `mesh_fmt`
    - `snow_repl` -> `mesh_repl`
    These appear in `use snow_parser::...`, `use snow_common::...`, etc.

    **B. Rust type names (PascalCase):**
    - `SnowString` -> `MeshString`
    - `SnowResult` -> `MeshResult`
    - `SnowOption` -> `MeshOption`
    - `SnowJson` -> `MeshJson`
    - `SnowHttpRequest` -> `MeshHttpRequest`
    - `SnowHttpResponse` -> `MeshHttpResponse`
    - `SnowRouter` -> `MeshRouter`
    - `SnowBackend` -> `MeshBackend`
    - `SnowGcHeader` -> `MeshGcHeader` (if exists)
    - Any other `Snow[A-Z]` type names found

    **C. Runtime C ABI function names (string literals in codegen and repl, function defs in rt):**
    Replace ALL `snow_` prefixed identifiers that are C ABI symbols:
    - In crates/snow-rt/src/**/*.rs: rename `pub extern "C" fn snow_` -> `pub extern "C" fn mesh_`
      and all internal references to these function names
    - In crates/snow-codegen/src/**/*.rs: rename all string literals `"snow_*"` -> `"mesh_*"`
      (these are ~1214 occurrences -- function name strings used for LLVM IR generation)
    - In crates/snow-repl/src/jit.rs: rename all `add_sym("snow_*"` -> `add_sym("mesh_*"`
      and all `snow_rt::snow_*` -> `mesh_rt::mesh_*`
    - The entry point `snow_main` -> `mesh_main` (in codegen/mir/lower.rs)

    **D. File extension references (.snow -> .mpl):**
    - All string literals `".snow"` -> `".mpl"` across all .rs files
    - All string literals `"main.snow"` -> `"main.mpl"`
    - All string literals `"snow.toml"` -> `"mesh.toml"`
    - All string literals `"snow.lock"` -> `"mesh.lock"`
    - All string literals `".snow/"` or `".snow/deps"` -> `".mesh/"` or `".mesh/deps"`
    - `".snow_repl_history"` -> `".mesh_repl_history"`
    - `"test.snow"` -> `"test.mpl"` (in test files)
    - Various `*.snow` test file names in e2e test string literals like `read_fixture("hello.snow")` -> `read_fixture("hello.mpl")`

    **E. Doc comments and display strings:**
    - `"Snow "` -> `"Mesh "` (in user-facing strings, help text)
    - `"Snow language"` / `"Snow Language"` -> `"Mesh language"` / `"Mesh Language"`
    - `"Snow LSP"` -> `"Mesh LSP"`
    - `"snowc "` -> `"meshc "` (in CLI help text, binary name references)
    - `"Hello from Snow!"` -> `"Hello from Mesh!"` (in scaffold.rs)
    - Doc comments referencing Snow -> Mesh
    - The diagnostic source `source: Some("snow".to_string())` -> `source: Some("mesh".to_string())`
    - Hover display `"```snow\n"` -> `"```mesh\n"` (in LSP server.rs)

    **F. The `snow_` prefix check in codegen lower.rs:**
    Line ~302 has a check for runtime prefixes: `"snow_"` -> `"mesh_"`

    Use `find crates/ -name '*.rs' -exec sed -i '' ...` for bulk operations. Verify no
    remaining `snow` references after (excluding the word "snow" if it appears in an
    unrelated context like weather -- but this codebase has none of those).

    IMPORTANT: Be careful with `include_str!("../../../tests/fixtures/keywords.snow")` and similar
    -- these will be addressed when the .snow files are renamed. The path strings in include_str!
    macros MUST be updated to `.mpl` extension here.
  </action>
  <verify>
    `grep -rn 'snow' crates/ --include='*.rs' | grep -v '.planning/' | grep -v 'target/'` should return zero results.
    Pay special attention that no partial replacements occurred (e.g., "meshflake" from "snowflake" -- but this codebase has no such words).
  </verify>
  <done>All Rust source files updated. Zero remaining "snow"/"Snow"/"SNOW" references in any .rs file under crates/. All `use mesh_*` imports, all `mesh_*` C ABI symbols, all `Mesh*` types, all `.mpl` extensions, all `mesh.toml` references.</done>
</task>

<task type="auto">
  <name>Task 3: Rename test fixture files (.snow -> .mpl) and update snapshot files</name>
  <files>
    tests/**/*.snow -> tests/**/*.mpl (145 files)
    tests/trait_codegen.snow -> tests/trait_codegen.mpl
    crates/**/tests/snapshots/*.snap (~212 files)
  </files>
  <action>
    **A. Rename all .snow test files to .mpl:**
    ```
    find tests/ -name '*.snow' -exec bash -c 'mv "$1" "${1%.snow}.mpl"' _ {} \;
    ```
    This covers:
    - tests/fixtures/*.snow (10 files)
    - tests/e2e/*.snow (132 files)
    - tests/compile_fail/*.snow (2 files)
    - tests/trait_codegen.snow (1 file)

    Use `git mv` for each to preserve git history.

    **B. Update snapshot files:**
    The .snap files under crates/*/tests/snapshots/ contain:
    - `source: crates/snow-typeck/tests/diagnostics.rs` -> `source: crates/mesh-typeck/tests/diagnostics.rs`
    - `test.snow` references in rendered diagnostic output -> `test.mpl`
    - Any other `snow` references

    Run sed across all .snap files:
    - Replace `snow-typeck` -> `mesh-typeck` in source paths
    - Replace `snow-parser` -> `mesh-parser` in source paths
    - Replace `snow-lexer` -> `mesh-lexer` in source paths
    - Replace `test.snow` -> `test.mpl`
    - Replace any `snow` in snapshot content that refers to the language

    IMPORTANT: After Task 5 renames directories, snapshot paths will need to match. The
    `source:` lines in .snap files reference the test file path -- these must use the NEW
    crate directory names. Since we are updating content before renaming dirs, we update
    the snap content to reference the new names now.

    NOTE: It is very likely simpler to just delete all .snap files and re-run
    `cargo test` with `INSTA_UPDATE=1` after the full rename to regenerate them.
    This is the PREFERRED approach if the number of affected snaps is large. Include
    this as the strategy: delete all snaps, let Task 6 verification regenerate them.
  </action>
  <verify>
    `find tests/ -name '*.snow'` returns nothing.
    `find tests/ -name '*.mpl' | wc -l` returns 145.
    No .snow files remain anywhere outside .planning/.
  </verify>
  <done>All 145 test files renamed from .snow to .mpl. Snapshot files deleted (to be regenerated on first test run).</done>
</task>

<task type="auto">
  <name>Task 4: Update LICENSE, .gitignore, and VSCode extension</name>
  <files>
    LICENSE
    .gitignore
    editors/vscode-snow/ -> editors/vscode-mesh/ (entire directory)
  </files>
  <action>
    **A. LICENSE:**
    - `"Snow Language Project"` -> `"Mesh Language Project"`

    **B. .gitignore:**
    - `editors/vscode-snow/node_modules/` -> `editors/vscode-mesh/node_modules/`

    **C. VSCode extension (editors/vscode-snow/):**
    Update all files in the extension before renaming the directory:

    1. `package.json`: Update all `snow`/`Snow` references:
       - name, displayName, description, language IDs, file extensions (.snow -> .mpl),
         grammar scope names, etc.
       - Binary name `snowc` -> `meshc` if referenced

    2. `src/extension.ts`: Update any `snow`/`Snow`/`snowc` references

    3. `syntaxes/snow.tmLanguage.json`:
       - Rename to `mesh.tmLanguage.json` (via git mv)
       - Update scope names inside: `source.snow` -> `source.mesh`, etc.

    4. `language-configuration.json`: Update if contains snow references

    5. `snow-lang-0.1.0.vsix`: Delete this built artifact (it will be rebuilt)

    6. `out/extension.js`: Delete (compiled output, will be rebuilt)

    7. Rename directory: `git mv editors/vscode-snow editors/vscode-mesh`
  </action>
  <verify>
    `grep -rn 'snow\|Snow' LICENSE .gitignore editors/` returns nothing (or only .vsix binary if kept).
  </verify>
  <done>LICENSE updated. .gitignore updated. VSCode extension fully renamed to vscode-mesh with all internal references updated.</done>
</task>

<task type="auto">
  <name>Task 5: Rename all crate directories</name>
  <files>
    crates/snow-common -> crates/mesh-common
    crates/snow-lexer -> crates/mesh-lexer
    crates/snow-parser -> crates/mesh-parser
    crates/snow-typeck -> crates/mesh-typeck
    crates/snow-codegen -> crates/mesh-codegen
    crates/snow-rt -> crates/mesh-rt
    crates/snow-pkg -> crates/mesh-pkg
    crates/snow-lsp -> crates/mesh-lsp
    crates/snow-fmt -> crates/mesh-fmt
    crates/snow-repl -> crates/mesh-repl
    crates/snowc -> crates/meshc
  </files>
  <action>
    Rename ALL crate directories using `git mv` to preserve history:

    ```bash
    cd /Users/sn0w/Documents/dev/snow
    git mv crates/snow-common crates/mesh-common
    git mv crates/snow-lexer crates/mesh-lexer
    git mv crates/snow-parser crates/mesh-parser
    git mv crates/snow-typeck crates/mesh-typeck
    git mv crates/snow-codegen crates/mesh-codegen
    git mv crates/snow-rt crates/mesh-rt
    git mv crates/snow-pkg crates/mesh-pkg
    git mv crates/snow-lsp crates/mesh-lsp
    git mv crates/snow-fmt crates/mesh-fmt
    git mv crates/snow-repl crates/mesh-repl
    git mv crates/snowc crates/meshc
    ```

    This MUST happen AFTER all file content updates (Tasks 1-4) because those tasks
    reference files at their old paths.

    After renaming, verify workspace Cargo.toml members match the new directory names.
  </action>
  <verify>
    `ls crates/` shows only mesh-* directories and meshc. No snow-* directories remain.
    `cat Cargo.toml` shows members pointing to `crates/mesh-*` and `crates/meshc`.
  </verify>
  <done>All 11 crate directories renamed. Git history preserved via git mv.</done>
</task>

<task type="auto">
  <name>Task 6: Build, regenerate snapshots, and run full test suite</name>
  <files>
    crates/**/tests/snapshots/*.snap (regenerated)
  </files>
  <action>
    1. Run `cargo build` from the workspace root. Fix any compilation errors from missed renames.
       Common issues to watch for:
       - Missed `use snow_*` import somewhere
       - Missed `snow_` C ABI symbol string literal
       - Path reference to old directory name

    2. Run `cargo test` with `INSTA_UPDATE=1` (or `cargo insta test --accept`) to regenerate
       all snapshot files with the new names/paths. The snapshots will automatically pick up
       the new crate directory paths in their `source:` lines.

    3. Run `cargo test` again WITHOUT insta update to confirm all tests pass cleanly.

    4. Do a final grep sweep:
       `grep -rn 'snow' crates/ tests/ Cargo.toml LICENSE .gitignore editors/ --include='*.rs' --include='*.toml' --include='*.json' --include='*.ts' --include='*.snap' --include='*.md'`
       Excluding .planning/ directory. Fix any remaining references.

    5. Verify the binary name: `cargo build -p meshc && ls target/debug/meshc`
  </action>
  <verify>
    `cargo build` succeeds with zero errors.
    `cargo test` passes all tests.
    `grep -rn 'snow\|Snow\|SNOW' crates/ tests/ Cargo.toml LICENSE .gitignore editors/ --include='*.rs' --include='*.toml' --include='*.json' --include='*.ts' --include='*.snap'` returns zero results.
    `target/debug/meshc` binary exists.
  </verify>
  <done>Project builds and all tests pass under the Mesh name. No remaining Snow references outside .planning/. meshc binary is produced.</done>
</task>

</tasks>

<verification>
- `cargo build` succeeds
- `cargo test` passes all tests (including regenerated snapshots)
- `ls crates/` shows only mesh-* and meshc directories
- `find tests/ -name '*.snow'` returns nothing
- `find tests/ -name '*.mpl' | wc -l` returns 145
- `grep -rn 'snow' crates/ Cargo.toml LICENSE .gitignore` returns nothing
- `target/debug/meshc --help` runs and shows Mesh references
- `.planning/` directory is untouched (historical records preserved)
</verification>

<success_criteria>
The entire Snow project is renamed to Mesh:
- All crates: mesh-common, mesh-lexer, mesh-parser, mesh-typeck, mesh-codegen, mesh-rt, mesh-pkg, mesh-lsp, mesh-fmt, mesh-repl, meshc
- Binary: meshc (not snowc)
- File extension: .mpl (not .snow)
- Manifest: mesh.toml (not snow.toml)
- Runtime symbols: mesh_* (not snow_*)
- Types: Mesh* (not Snow*)
- All tests pass
- Git history preserved for renamed files
</success_criteria>

<output>
After completion, create `.planning/quick/1-rename-project-from-snow-to-mesh-change-/1-SUMMARY.md`
</output>
</task>
