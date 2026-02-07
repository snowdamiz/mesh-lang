---
phase: 10-developer-tooling
verified: 2026-02-07T10:25:00Z
status: passed
score: 5/5 must-haves verified
---

# Phase 10: Developer Tooling Verification Report

**Phase Goal:** Developer tools that make Snow practical for daily use -- polished error messages, code formatter, REPL, package manager, and LSP server

**Verified:** 2026-02-07T10:25:00Z
**Status:** PASSED
**Re-verification:** No — initial verification

## Goal Achievement

### Observable Truths

| # | Truth | Status | Evidence |
|---|-------|--------|----------|
| 1 | Compiler error messages consistently identify the problem, show the relevant source code with underlined spans, and suggest fixes where possible (Elm/Rust quality standard) | ✓ VERIFIED | DiagnosticOptions API exists (1229 lines), render_json_diagnostic implemented, --json and --no-color flags work. Manual test: type error shows underlined span with ariadne formatting, JSON output contains code/severity/message/spans/fix fields. Multi-span diagnostics show parameter type vs call site. |
| 2 | `snowc fmt` formats Snow source code to a canonical style, and formatting is idempotent | ✓ VERIFIED | snow-fmt crate exists (530 lines lib.rs + walker/printer/ir modules), format_source function implemented. 79 tests pass including idempotency tests. Manual test: formatting "fn add(a,b) do\na+b\nend" produces "fn add(a, b) do\n  a + b\nend\n" consistently. E2E test test_fmt_idempotent passes. |
| 3 | `snowc repl` starts an interactive session where expressions can be evaluated, types are displayed, and previous results are accessible | ✓ VERIFIED | snow-repl crate exists (642 lines lib.rs + jit/session modules), run_repl function implemented with ReplConfig. JIT engine uses LLVM with actor runtime support. 44 tests pass. CLI subcommand wired in main.rs. Session management with result history implemented. |
| 4 | A package manager can initialize a project, declare dependencies, and resolve/fetch them | ✓ VERIFIED | snow-pkg crate exists with manifest/lockfile/resolver/scaffold modules. scaffold_project creates snow.toml + main.snow. resolve_dependencies handles git and path deps with transitive resolution. 24 tests pass. Manual test: snowc init creates project, snowc deps generates snow.lock. |
| 5 | An LSP server provides diagnostics, go-to-definition, and type-on-hover in editors | ✓ VERIFIED | snow-lsp crate exists (31 lines lib.rs + analysis/definition/server modules), run_server implemented with tower-lsp. Server capabilities advertise hover_provider and definition_provider. 31 tests pass. analyze_document produces LSP diagnostics, goto_definition resolves identifiers, hover queries return type info. |

**Score:** 5/5 truths verified

### Required Artifacts

| Artifact | Expected | Status | Details |
|----------|----------|--------|---------|
| `crates/snow-typeck/src/diagnostics.rs` | Enhanced diagnostic rendering with color toggle and JSON output | ✓ VERIFIED (1229 lines) | DiagnosticOptions struct, render_json_diagnostic function, JsonDiagnostic/JsonSpan types, multi-span labels for FnArg/Return/Assignment, Levenshtein distance for "did you mean" suggestions |
| `crates/snowc/src/main.rs` | CLI --json flag and --no-color flag | ✓ VERIFIED (464 lines) | Commands enum has Build with json/no_color flags, report_diagnostics dispatches to JSON or human-readable output, all subcommands wired (build/init/deps/fmt/repl/lsp) |
| `crates/snow-fmt/src/lib.rs` | Formatter core with format_source | ✓ VERIFIED (530 lines) | format_source function, FormatConfig struct, walker/printer/ir modules, idempotency tests |
| `crates/snow-repl/src/lib.rs` | REPL core with JIT engine | ✓ VERIFIED (642 lines) | run_repl function, ReplConfig, CommandResult, process_command, is_input_complete multi-line handling |
| `crates/snow-repl/src/jit.rs` | JIT compilation via LLVM | ✓ VERIFIED (referenced) | jit_eval function, init_runtime_and_symbols registers actor runtime symbols, compile_and_execute_mir uses LLVM ExecutionEngine |
| `crates/snow-pkg/src/lib.rs` | Package manager core | ✓ VERIFIED (9 lines + 4 modules) | Re-exports Manifest, resolve_dependencies, scaffold_project |
| `crates/snow-pkg/src/resolver.rs` | Dependency resolution | ✓ VERIFIED (referenced) | resolve_dependencies function handles git/path deps, transitive resolution, cycle detection, lockfile generation |
| `crates/snow-lsp/src/lib.rs` | LSP server entry point | ✓ VERIFIED (31 lines) | run_server async function using tower_lsp |
| `crates/snow-lsp/src/server.rs` | LSP capabilities | ✓ VERIFIED (referenced) | SnowBackend implements LanguageServer trait, initialize advertises hover/definition providers, did_open/did_change/hover/goto_definition implemented |

### Key Link Verification

| From | To | Via | Status | Details |
|------|----|----|--------|---------|
| snowc CLI | diagnostics.rs | render_diagnostic call | ✓ WIRED | main.rs report_diagnostics calls render_diagnostic with DiagnosticOptions, dispatch based on json flag |
| snowc CLI | snow-fmt | format_source call | ✓ WIRED | main.rs fmt_command calls snow_fmt::format_source, collects .snow files, writes formatted output |
| snowc CLI | snow-repl | run_repl call | ✓ WIRED | main.rs Commands::Repl calls snow_repl::run_repl with ReplConfig::default() |
| snowc CLI | snow-pkg | scaffold_project + resolve_dependencies calls | ✓ WIRED | main.rs Commands::Init calls snow_pkg::scaffold_project, Commands::Deps calls resolve_dependencies |
| snowc CLI | snow-lsp | run_server call | ✓ WIRED | main.rs Commands::Lsp creates tokio runtime and calls snow_lsp::run_server().await |
| REPL JIT | actor runtime | symbol registration | ✓ WIRED | jit.rs init_runtime_and_symbols calls snow_rt::snow_rt_init_actor, registers 20+ runtime symbols with LLVM via add_sym |
| LSP server | analysis | analyze_document call | ✓ WIRED | server.rs analyze_and_publish calls analysis::analyze_document, publishes diagnostics to client |
| LSP server | definition | find_definition call | ✓ WIRED | server.rs goto_definition calls definition::find_definition, converts tree offsets to source positions |

### Requirements Coverage

| Requirement | Status | Blocking Issue |
|-------------|--------|----------------|
| TOOL-02: Clear, human-readable error messages | ✓ SATISFIED | Ariadne-based diagnostics with color, multi-span labels, fix suggestions verified working |
| TOOL-03: Code formatter | ✓ SATISFIED | snow-fmt with CST walker, Wadler-Lindig IR, idempotent formatting verified |
| TOOL-04: REPL | ✓ SATISFIED | snow-repl with LLVM JIT, multi-line input, actor support, session management verified |
| TOOL-05: Package manager | ✓ SATISFIED | snow-pkg with TOML manifest, git/path deps, transitive resolution, lockfile verified |
| TOOL-06: LSP server | ✓ SATISFIED | snow-lsp with diagnostics, hover, goto-definition via tower-lsp verified |

### Anti-Patterns Found

None. All implementations are substantive with comprehensive test coverage.

### Human Verification Required

The following aspects require human testing as they involve interactive experiences and subjective quality assessment:

#### 1. Error Message Quality Assessment

**Test:** Create various type errors, parse errors, and pattern match errors. Read the error messages.
**Expected:** Messages should clearly explain what went wrong, show relevant code with underlined spans, and suggest fixes when applicable. Messages should be concise (1-2 sentences) and use concrete types (not type variables).
**Why human:** Subjective assessment of clarity, helpfulness, and Elm/Rust quality standard.

#### 2. REPL Interactive Experience

**Test:** 
1. Run `snowc repl`
2. Enter `1 + 2` and press Enter
3. Enter `let x = 42` and press Enter
4. Enter `x * 2` and press Enter
5. Enter `:type x` to see type information
6. Enter a multi-line function starting with `fn add(a, b) do` (without end), verify continuation prompt appears
7. Complete with `a + b` and `end`
8. Try spawning an actor with `spawn fn -> IO.puts("hello") end`

**Expected:** 
- Expressions evaluate and display results with types
- Definitions are remembered across commands
- Multi-line input shows continuation prompt "  ... "
- Previous results accessible
- Actor spawn works (spawns and returns Pid)

**Why human:** Interactive REPL requires human to test multi-line flow, session state, actor spawning.

#### 3. Formatter Style Consistency

**Test:**
1. Create several Snow files with inconsistent formatting (mixed spacing, indentation)
2. Run `snowc fmt` on them
3. Verify all files now have consistent 2-space indentation, spacing around operators, do/end alignment
4. Check that comments are preserved in correct positions

**Expected:** All files formatted to identical style. Comments preserved.
**Why human:** Visual inspection of formatting quality and style consistency.

#### 4. Package Manager Workflow

**Test:**
1. Run `snowc init my-app`
2. Edit snow.toml to add a git dependency
3. Run `snowc deps`
4. Verify .snow/deps/ directory contains checked-out dependency
5. Verify snow.lock contains locked versions
6. Run `snowc deps` again, verify it says "Dependencies up to date"

**Expected:** Smooth workflow, clear messages, dependencies properly fetched and locked.
**Why human:** Multi-step workflow with file system inspection.

#### 5. LSP in VS Code/Cursor

**Test:**
1. Install the Snow extension from editors/vscode-snow/
2. Open a .snow file
3. Introduce a type error, verify red squiggle appears with inline diagnostic
4. Hover over a variable, verify type information appears
5. Ctrl+click (or Cmd+click) on a variable, verify jump to definition
6. Check that syntax highlighting works for keywords, strings, comments, operators

**Expected:** Real-time diagnostics, hover works, go-to-definition navigates correctly, syntax highlighting renders properly.
**Why human:** Editor integration requires interactive testing in VS Code/Cursor.

### Gaps Summary

No gaps found. All 5 success criteria met with substantive implementations and passing tests.

## Detailed Verification

### Truth 1: Compiler Error Messages

**Artifacts verified:**
- `crates/snow-typeck/src/diagnostics.rs` (1229 lines): DiagnosticOptions API, render_json_diagnostic, JsonDiagnostic struct, multi-span labels for FnArg/Return/Assignment origins
- `crates/snowc/src/main.rs` (464 lines): --json and --no-color flags in Build command, report_diagnostics dispatch

**Wiring verified:**
- main.rs imports snow_typeck::diagnostics::DiagnosticOptions
- report_diagnostics constructs DiagnosticOptions based on flags
- Dispatch: if json -> print JSON one-per-line, else -> call render_diagnostic with color config
- Parse errors also converted to JSON format with code "P0001"

**Tests verified:**
- 111 tests pass in snow-typeck
- Manual test: `snowc build --json` with type error produces valid JSON with code/severity/message/file/spans/fix
- Manual test: `snowc build --no-color` produces colorless ariadne output
- Manual test: `snowc build` produces colorized output with ANSI codes
- E2E test test_build_json_output verifies JSON parsing

**Stubness check:**
- render_json_diagnostic: 203 lines, full implementation converting TypeError to JSON
- Multi-span labels: Implementation exists for FnArg (argument number), Return (expression + type annotation), Assignment (lhs + rhs)
- Levenshtein distance for suggestions: 20+ line implementation
- All error codes mapped (E0001-E0022)

**Verdict:** ✓ VERIFIED. Error messages show underlined spans, suggest fixes, support JSON output, and have multi-span support.

### Truth 2: Formatter Idempotency

**Artifacts verified:**
- `crates/snow-fmt/src/lib.rs` (530 lines): format_source function, FormatConfig struct
- `crates/snow-fmt/src/walker.rs`: CST walker producing FormatIR
- `crates/snow-fmt/src/printer.rs`: FormatConfig with indent_size/max_width, print function
- `crates/snow-fmt/src/ir.rs`: FormatIR enum (Text/Space/Hardline/Indent/Group/Concat/IfBreak)

**Wiring verified:**
- snowc main.rs Commands::Fmt calls snow_fmt::format_source
- fmt_command collects .snow files, reads source, calls format_source, writes back or checks
- --check mode compares formatted vs source, exits 1 if different

**Tests verified:**
- 79 tests pass in snow-fmt
- Idempotency tests: idempotent_empty_file, idempotent_single_let_binding, idempotent_let_with_type, idempotent_fn_with_do_end, idempotent_nested_if_else, idempotent_case_multiple_arms, idempotent_module_with_imports
- Manual test: formatting "fn add(a,b) do\na+b\nend" produces "fn add(a, b) do\n  a + b\nend\n", second pass produces identical
- E2E tests: test_fmt_formats_file, test_fmt_check_formatted, test_fmt_check_unformatted, test_fmt_idempotent all pass

**Stubness check:**
- format_source: 5-line implementation using parse -> walk_node -> print
- Walker: 1600+ lines covering all AST node types
- Printer: 320+ lines with layout algorithm
- FormatIR: 69 lines with 7 variants and constructor functions

**Verdict:** ✓ VERIFIED. Formatter formats to canonical style and is idempotent (proven by tests and manual verification).

### Truth 3: REPL Functionality

**Artifacts verified:**
- `crates/snow-repl/src/lib.rs` (642 lines): run_repl function, ReplConfig, CommandResult, process_command, is_input_complete
- `crates/snow-repl/src/jit.rs`: jit_eval, init_runtime_and_symbols, compile_and_execute_mir
- `crates/snow-repl/src/session.rs`: ReplSession with definition tracking

**Wiring verified:**
- snowc main.rs Commands::Repl calls snow_repl::run_repl(&ReplConfig::default())
- run_repl: calls init_runtime_and_symbols once, then enters readline loop
- jit_eval: wraps expression, parses, type-checks, lowers to MIR, compiles to LLVM, executes via JIT
- Actor runtime symbols registered: snow_actor_spawn, snow_actor_send, snow_actor_receive, snow_actor_self, etc. (20+ symbols)

**Tests verified:**
- 44 tests pass in snow-repl
- Tests cover: is_command, process_command (help/quit/clear/reset/type/load), is_input_complete (parens/do-end/strings), wrap_expression, record_result, session reset
- Tests for extract_definition_name, format_jit_result (Int/Bool/Unit), init_runtime_is_idempotent
- E2E test test_repl_help verifies snowc repl --help mentions REPL

**Stubness check:**
- run_repl: 40+ lines, full implementation with rustyline editor, multi-line accumulation, command dispatch, eval loop
- jit_eval: 120+ lines, full pipeline parse -> typecheck -> MIR -> LLVM -> execute
- compile_and_execute_mir: 60+ lines, LLVM module creation, JIT ExecutionEngine, function call via call_function
- init_runtime_and_symbols: 80+ lines registering all runtime symbols with LLVM

**Verdict:** ✓ VERIFIED. REPL starts, evaluates expressions via JIT, displays types, tracks definitions, supports multi-line input, integrates actor runtime.

### Truth 4: Package Manager Capabilities

**Artifacts verified:**
- `crates/snow-pkg/src/lib.rs` (9 lines): Re-exports Manifest, resolve_dependencies, scaffold_project
- `crates/snow-pkg/src/manifest.rs`: Manifest struct, TOML parsing
- `crates/snow-pkg/src/lockfile.rs`: Lockfile struct with version/packages, read/write
- `crates/snow-pkg/src/resolver.rs`: resolve_dependencies, ResolvedDep, git/path dep handling
- `crates/snow-pkg/src/scaffold.rs`: scaffold_project creating directory + snow.toml + main.snow

**Wiring verified:**
- snowc main.rs Commands::Init calls snow_pkg::scaffold_project(&name, &current_dir)
- snowc main.rs Commands::Deps calls deps_command -> resolve_dependencies -> lockfile.write
- Dependency resolution: reads snow.toml -> resolve_deps (recursive, git clone, path copy) -> generate lockfile

**Tests verified:**
- 24 tests pass in snow-pkg
- Tests cover: manifest parsing (minimal/full/git deps/path deps/validation), lockfile round-trip/determinism/empty, resolver (no deps/path dep/transitive/git/branch/cycle detection/diamond conflict), scaffold (directory structure/toml valid/main.snow content/error when exists)
- E2E test test_init_creates_project verifies snowc init creates snow.toml + main.snow
- Manual test: snowc init creates project, snowc deps generates snow.lock

**Stubness check:**
- scaffold_project: 40+ lines, creates directory, writes TOML template, writes main.snow template
- resolve_dependencies: 25+ lines orchestrating manifest read, resolution, lockfile generation
- resolve_deps: 230+ lines with git clone (libgit2), path copy, transitive traversal, cycle detection
- Lockfile::write: 15+ lines TOML serialization

**Verdict:** ✓ VERIFIED. Package manager initializes projects, declares dependencies in TOML, resolves git/path deps transitively, generates deterministic lockfile.

### Truth 5: LSP Server Capabilities

**Artifacts verified:**
- `crates/snow-lsp/src/lib.rs` (31 lines): run_server async function
- `crates/snow-lsp/src/server.rs`: SnowBackend, LanguageServer trait impl, initialize/hover/goto_definition
- `crates/snow-lsp/src/analysis.rs`: analyze_document producing AnalysisResult with diagnostics
- `crates/snow-lsp/src/definition.rs`: find_definition resolving identifiers to definition sites

**Wiring verified:**
- snowc main.rs Commands::Lsp creates tokio runtime, calls snow_lsp::run_server().await
- run_server: creates tower_lsp LspService with SnowBackend, runs Server on stdin/stdout
- SnowBackend::initialize returns ServerCapabilities with hover_provider and definition_provider
- did_open/did_change trigger analyze_and_publish -> analyze_document -> publish_diagnostics
- hover reads document state, calls find_type_at_position, returns Hover with type info
- goto_definition reads document state, calls find_definition, returns GotoDefinitionResponse with Location

**Tests verified:**
- 31 tests pass in snow-lsp
- Tests cover: analyze_document (valid source, parse error, type error, multiple errors), hover (integer literal, empty space, past EOF), goto_definition (let binding, function param, function call, shadowing, unknown, builtins), position/offset conversion, server capabilities
- E2E test test_lsp_subcommand_exists verifies snowc lsp --help exits 0

**Stubness check:**
- run_server: 6 lines, tower_lsp boilerplate creating service + server
- SnowBackend: 80+ lines implementing LanguageServer trait
- analyze_document: 60+ lines parse -> typecheck -> convert errors to LSP Diagnostic
- find_definition: 180+ lines CST traversal, scope tracking, identifier resolution
- Capabilities: hover_provider: Some(HoverProviderCapability::Simple(true)), definition_provider: Some(OneOf::Left(true))

**Verdict:** ✓ VERIFIED. LSP server provides diagnostics (parse + type errors), type-on-hover, and go-to-definition. Tower-lsp integration works, tests pass, CLI subcommand wired.

---

**Verification Complete**
**Status:** PASSED
**Timestamp:** 2026-02-07T10:25:00Z
**Verifier:** Claude (gsd-verifier)
