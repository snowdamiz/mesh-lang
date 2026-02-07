# Phase 10: Developer Tooling - Research

**Researched:** 2026-02-07
**Domain:** Compiler developer tools (diagnostics, formatter, REPL, package manager, LSP)
**Confidence:** HIGH (most patterns well-established in the Rust compiler tooling ecosystem)

<user_constraints>
## User Constraints (from CONTEXT.md)

### Locked Decisions

**Error message style:**
- Fix suggestions only when confident the suggestion is correct (avoid misleading suggestions)
- Colorized terminal output by default, `--json` flag for machine-readable output (editors, CI)
- Error codes continue existing E0001+ / W0001+ scheme

**Formatter behavior:**
- Minimal config: a few knobs (line width, indent size) but most decisions fixed
- 2-space indentation as default
- 100-column line width as default
- `snowc fmt` formats in-place by default; `--check` flag for CI (exit 1 if unformatted)

**REPL experience:**
- Incremental compilation via LLVM (not interpreter) -- behavior identical to compiled code
- Full actor support -- scheduler runs in background, spawn/send/receive work in REPL
- Multi-line input handling needed for do/end blocks

**Package manager design:**
- TOML project file (`snow.toml`) for project metadata and dependencies
- Git-based dependencies first, designed so a central registry can be added later without breaking changes
- Lockfile (`snow.lock`) always generated and committed for reproducible builds
- `snowc init` creates project scaffold

### Claude's Discretion

**Errors:**
- Tone and formatting approach (Elm-style conversational vs Rust-style structured)
- Multi-span display strategy (when to show both definition + call site vs call site only)
- Which specific error types get fix suggestions

**REPL:**
- Value display format (value + type annotation vs value only with :type command)
- Whether to support `:load file.snow` for importing definitions into session
- REPL command set (`:help`, `:type`, `:quit`, etc.)

**Package manager:**
- Exact project layout from `snowc init` (minimal vs standard)
- Dependency resolution algorithm
- Version constraint syntax

**LSP server:**
- Full discretion on LSP implementation approach
- Must provide: diagnostics, go-to-definition, type-on-hover

### Deferred Ideas (OUT OF SCOPE)
None -- discussion stayed within phase scope
</user_constraints>

## Summary

This phase adds five major developer tools to the Snow compiler toolchain: polished error diagnostics, an AST-based code formatter, an LLVM JIT-powered REPL, a git-based package manager, and an LSP server for editor integration. All tools build on the existing `snowc` CLI and the project's established patterns (rowan CST, ariadne diagnostics, inkwell/LLVM codegen).

The existing infrastructure is remarkably well-suited for this work. Snow already uses rowan for lossless CST (perfect for formatting and LSP), ariadne for diagnostics (already has multi-span labels, error codes, and fix suggestions), and inkwell with LLVM 21 for code generation (provides JIT execution engine for the REPL). The primary challenge is incremental compilation for the REPL with actor support, which requires initializing the snow-rt scheduler as a background system within the REPL process.

The standard approach for each tool is: (1) diagnostics -- extend the existing ariadne pipeline with color support and JSON output; (2) formatter -- walk the rowan CST and emit a format IR that a printer renders respecting line width; (3) REPL -- create per-expression LLVM modules and execute via JIT, with rustyline for input; (4) package manager -- parse `snow.toml` with the `toml` crate, clone deps with `git2`, simple DFS version resolution; (5) LSP -- use `tower-lsp` with the existing parse/typecheck pipeline to serve diagnostics, definitions, and hover info.

**Primary recommendation:** Build each tool as a new crate in the workspace (`snow-fmt`, `snow-repl`, `snow-pkg`, `snow-lsp`) with subcommands added to the existing `snowc` CLI. Leverage the existing rowan CST and ariadne infrastructure heavily -- do not duplicate parsing or diagnostic rendering.

## Standard Stack

### Core

| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| ariadne | 0.6 | Diagnostic rendering (already in use) | Already integrated; supports multi-span labels, color, builder pattern. Extends existing E0001-E0021/W0001 scheme. |
| rowan | 0.16 | Lossless CST (already in use) | Parser already produces rowan CST with whitespace/comment trivia preserved. Perfect for formatter and LSP. |
| inkwell | 0.8.0 (llvm21-1) | JIT execution engine for REPL | Already used for codegen. `create_jit_execution_engine()` provides MCJIT for per-expression compilation. |
| tower-lsp | 0.20 | LSP server framework | Standard Rust LSP framework. Provides `LanguageServer` trait, stdio/TCP transport, protocol handling. |
| rustyline | 17 | REPL line editing | De facto Rust readline library. History, Emacs/Vi keybindings, multi-line editing, completion hooks. |
| toml | latest | Parse `snow.toml` | Standard TOML parser with serde integration. Used by Cargo itself. |
| git2 | latest | Clone git dependencies | Safe Rust bindings to libgit2. `RepoBuilder` supports branch/tag/rev checkout. |
| serde + serde_json | 1 | JSON diagnostic output, lockfile | Already a workspace dependency. Needed for `--json` flag and `snow.lock`. |
| clap | 4.5 | CLI subcommands (already in use) | Already integrated. Add `fmt`, `repl`, `init`, `build` (enhanced), `lsp` subcommands. |

### Supporting

| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| lsp-types | 0.97 | LSP protocol type definitions | Comes as transitive dep of tower-lsp. Contains all LSP request/response types. |
| semver | latest | Version constraint parsing | Package manager version parsing and comparison. |
| sha2 or blake3 | latest | Lockfile content hashing | Computing dependency hashes for `snow.lock` integrity. |
| tokio | 1 (transitive) | Async runtime for LSP | tower-lsp requires tokio. LSP server runs async. |

### Alternatives Considered

| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| tower-lsp | lsp-server (rust-analyzer's) | lsp-server is lower-level (crossbeam-channel, manual dispatch). tower-lsp is higher-level with trait-based API. For a new LSP, tower-lsp is simpler to get started. |
| rustyline | reedline | reedline is nushell's readline. More features (menus, syntax highlighting) but heavier. rustyline is simpler and sufficient. |
| git2 | Command::new("git") | Shelling out to git avoids libgit2 dependency but requires git installed. git2 is more portable. |
| toml + serde | toml_edit | toml_edit preserves formatting on edit. Only needed if the package manager modifies `snow.toml` programmatically. Start with `toml` for read-only, upgrade to `toml_edit` for `snowc add` later. |

**Installation (additions to workspace Cargo.toml):**
```toml
[workspace.dependencies]
tower-lsp = "0.20"
rustyline = "17"
toml = "0.8"
git2 = "0.19"
semver = "1"
serde_json = "1"
```

## Architecture Patterns

### Recommended New Crate Structure

```
crates/
  snow-fmt/              # Code formatter (CST walk + format IR + printer)
    src/
      lib.rs             # Public API: format_source(source: &str, config: &FmtConfig) -> String
      config.rs          # FmtConfig: line_width, indent_size
      format_cst.rs      # CST walker that produces FormatIR
      ir.rs              # FormatIR enum (Space, Indent, Newline, Group, Text, etc.)
      printer.rs         # Renders FormatIR to String respecting line_width
  snow-repl/             # REPL with JIT
    src/
      lib.rs             # REPL loop: read -> parse -> typecheck -> JIT compile -> execute
      session.rs         # Session state: accumulated definitions, type environment
      display.rs         # Value display formatting
  snow-pkg/              # Package manager
    src/
      lib.rs             # Public API
      manifest.rs        # snow.toml parsing (SnowManifest struct)
      lockfile.rs        # snow.lock generation and parsing
      resolver.rs        # Dependency resolution (DFS with version constraints)
      fetcher.rs         # Git clone + cache management
      init.rs            # Project scaffold generation
  snow-lsp/              # LSP server
    src/
      lib.rs             # LanguageServer trait impl
      analysis.rs        # Parse + typecheck on document change
      diagnostics.rs     # Convert TypeError -> lsp_types::Diagnostic
      navigation.rs      # Go-to-definition using CST + TypeckResult
      hover.rs           # Type-on-hover using TypeckResult.types map
  snowc/                 # CLI (extend existing)
    src/
      main.rs            # Add fmt, repl, init, lsp subcommands
```

### Pattern 1: CST-Based Formatter (Wadler-Lindig IR Approach)

**What:** Walk the rowan CST and produce an intermediate format representation, then print it respecting line width constraints.
**When to use:** For the `snowc fmt` command.

The formatter does NOT reparse or reconstruct source. It walks the existing lossless CST (which preserves all whitespace, comments, and trivia) and produces a new source string by walking nodes and emitting formatting decisions.

**Three-phase architecture:**
1. **CST Walk:** Visit each SyntaxNode/SyntaxToken, emitting FormatIR elements
2. **IR:** An algebraic data type representing formatting primitives (text, space, line break, indent, group)
3. **Printer:** Flatten the IR to a string, breaking groups when they exceed line width

```rust
// Format IR (inspired by Wadler-Lindig / Biome)
enum FormatIR {
    Text(String),          // Literal text (token content)
    Space,                 // Single space (can be omitted at line break)
    Newline,               // Forced line break
    Indent(Box<FormatIR>), // Increase indent for inner content
    Group(Vec<FormatIR>),  // Try to fit on one line; break if exceeds width
    IfBreak {              // Conditional: emit one thing if group breaks, another if flat
        flat: Box<FormatIR>,
        broken: Box<FormatIR>,
    },
}
```

**Key insight:** Because Snow uses `do`/`end` blocks (not braces), the formatter has natural break points. A `do...end` block always gets newlines. Single-expression bodies may stay inline if they fit.

### Pattern 2: JIT REPL with Incremental Module Compilation

**What:** Each REPL input is compiled to a fresh LLVM module, added to the execution engine, and executed via JIT.
**When to use:** For `snowc repl`.

**The Kaleidoscope pattern (verified from inkwell examples):**
1. Create one LLVM Context for the REPL session
2. For each input expression, create a fresh Module
3. Compile the expression into an anonymous function in that module
4. Add the module to the ExecutionEngine
5. Look up the function and call it via `get_function()`
6. Accumulate type environment and definitions across inputs

**Actor support challenge:** The snow-rt scheduler must be initialized once at REPL startup (call `snow_rt_init_actor()`) and run in the background. When REPL expressions spawn actors, they use the already-running scheduler. The main REPL thread is itself registered as a process (already supported by `snow_rt_init_actor()` which creates a main thread process entry).

### Pattern 3: Document-Level LSP Analysis

**What:** On each `didChange` notification, re-parse and re-typecheck the full document. Use the resulting CST and TypeckResult to answer queries.
**When to use:** For `snowc lsp`.

**Architecture:**
1. On `did_open` / `did_change`: re-run `snow_parser::parse()` + `snow_typeck::check()` on the document
2. Store the latest `Parse` + `TypeckResult` for the document
3. On `hover`: look up the TextRange at cursor position in `TypeckResult.types` map
4. On `goto_definition`: walk CST to find definition site of the name under cursor
5. On diagnostics: convert `TypeckResult.errors` to `lsp_types::Diagnostic`

This is a simple, non-incremental approach. It is sufficient for single-file editing. Multi-file support and incremental recompilation (salsa) are future work.

### Pattern 4: JSON Diagnostic Output

**What:** A `--json` flag on `snowc build` that outputs diagnostics as JSON instead of colorized terminal output.
**When to use:** For editor integration and CI.

```rust
#[derive(Serialize)]
struct JsonDiagnostic {
    code: String,           // "E0001"
    severity: String,       // "error" | "warning"
    message: String,        // "expected Int, found String"
    file: String,           // "main.snow"
    spans: Vec<JsonSpan>,   // Primary + secondary spans
    fix: Option<String>,    // Suggested fix
}

#[derive(Serialize)]
struct JsonSpan {
    start_line: u32,
    start_col: u32,
    end_line: u32,
    end_col: u32,
    label: Option<String>,
    primary: bool,
}
```

One JSON object per line (JSON Lines format) for easy parsing by editors.

### Anti-Patterns to Avoid

- **Reinventing the CST for the formatter:** The parser already produces a lossless rowan CST with all trivia. The formatter must walk THIS tree, not re-parse the source.
- **Interpreter-based REPL:** The user explicitly requires LLVM JIT compilation so behavior matches compiled code exactly. Do not build an AST interpreter.
- **Monolithic LSP analysis:** Do not try to build incremental/salsa-based analysis in this phase. Full re-parse + re-typecheck per change is fast enough for single files.
- **Central registry in v1 package manager:** Git-based dependencies first. Design the manifest format so a registry field can be added later, but do not build registry support now.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Line editing, history, key bindings | Custom terminal input loop | rustyline | Terminal handling is platform-specific nightmare (raw mode, escape sequences, Windows compatibility). rustyline handles it. |
| LSP protocol parsing | Custom JSON-RPC handler | tower-lsp | LSP protocol has 100+ message types, versioned capabilities, lifecycle management. tower-lsp handles handshake, dispatch, transport. |
| Git operations | Shelling out to git | git2 | Parsing git output is fragile. git2 provides structured API for clone, checkout, rev-parse. |
| TOML parsing | Custom config parser | toml + serde | TOML spec has many edge cases (multi-line strings, inline tables, datetime). Derive deserialization is clean. |
| Diagnostic rendering | Custom terminal colorizer | ariadne (already using) | Multi-span label layout, overlap resolution, color generation -- all solved by ariadne. |
| Version constraint parsing | Regex-based parser | semver crate | Semver has surprising edge cases (pre-release ordering, build metadata). The `semver` crate handles them correctly. |

**Key insight:** This phase's value is in the Snow-specific logic (what to format, how to display types, what constitutes a valid project). The infrastructure plumbing should all be handled by established crates.

## Common Pitfalls

### Pitfall 1: REPL LLVM Context Lifetime

**What goes wrong:** The LLVM Context is created, modules are added to it, but the Context is dropped while the ExecutionEngine still holds references to modules.
**Why it happens:** Inkwell's lifetime system ties modules to their Context via `'ctx`. If the Context is stack-allocated in the REPL loop, it gets dropped each iteration.
**How to avoid:** Create the Context ONCE at REPL startup and hold it for the entire session. All modules share the same Context lifetime. The ExecutionEngine is also created once and modules are added via `add_module()`.
**Warning signs:** Segfault or use-after-free when calling JIT-compiled functions.

### Pitfall 2: Formatter Idempotency Failures

**What goes wrong:** Formatting already-formatted code produces different output (e.g., extra/missing whitespace, comment displacement).
**Why it happens:** The formatter makes decisions based on input formatting (e.g., "keep existing newlines") that conflict with its own output.
**How to avoid:** The formatter should be purely structural -- it should ALWAYS emit the same output for the same CST structure regardless of input whitespace. Test idempotency by running `format(format(code)) == format(code)` on every test case.
**Warning signs:** CI `--check` flag fails on code that was just formatted.

### Pitfall 3: REPL Actor Scheduler Blocking

**What goes wrong:** The REPL blocks forever waiting for the scheduler to complete (because `snow_rt_run_scheduler()` blocks until all actors exit).
**Why it happens:** In compiled programs, `snow_rt_run_scheduler()` is called after `snow_main()` returns and blocks until actors complete. In the REPL, we need the scheduler running continuously while accepting new input.
**How to avoid:** Start the scheduler in a background thread at REPL startup. Do NOT call `snow_rt_run_scheduler()` in the REPL's main loop. Instead, init the scheduler with `snow_rt_init_actor()` and let spawned actors run on scheduler threads. The REPL's main thread reads input and JIT-compiles; actor spawns happen via the global scheduler.
**Warning signs:** REPL hangs after first actor spawn.

### Pitfall 4: LSP Position Encoding Mismatch

**What goes wrong:** Go-to-definition jumps to wrong position, hover shows wrong type.
**Why it happens:** LSP uses (line, UTF-16 offset) positions. Rowan uses byte offsets. If the conversion doesn't account for multi-byte characters, positions are wrong.
**How to avoid:** Implement a `position_to_offset()` helper that correctly converts LSP positions (line + UTF-16 character offset) to rowan byte offsets. Test with multi-byte characters (emoji, CJK).
**Warning signs:** Features work on ASCII code but break on files with non-ASCII characters.

### Pitfall 5: Package Manager Diamond Dependencies

**What goes wrong:** Two dependencies require different versions of a shared transitive dependency.
**Why it happens:** Git-based deps don't have version ranges -- they point at specific commits/tags. If A depends on C@v1 and B depends on C@v2, there's no resolution.
**How to avoid:** For v1, use a simple approach: each dependency gets its own copy. Deduplication happens only when the same git URL + revision is requested by multiple deps. Flag conflicts as errors for the user to resolve. This matches early Go modules behavior.
**Warning signs:** Silent linking of incompatible library versions.

### Pitfall 6: Formatter Comment Displacement

**What goes wrong:** Comments get moved to wrong positions after formatting, or comments between tokens disappear.
**Why it happens:** The CST walker skips trivia tokens or reattaches them to the wrong node.
**How to avoid:** Process ALL tokens including trivia in CST order. When emitting formatted output for a node, emit its leading trivia first, then its content, then its trailing trivia. Never discard tokens.
**Warning signs:** Comments disappear or appear in wrong locations after `snowc fmt`.

## Code Examples

### Extending snowc CLI with New Subcommands

```rust
// Source: existing crates/snowc/src/main.rs pattern
#[derive(Subcommand)]
enum Commands {
    /// Compile a Snow project to a native binary
    Build { /* existing fields */ },
    /// Format Snow source files
    Fmt {
        /// Files or directories to format (default: current directory)
        paths: Vec<PathBuf>,
        /// Check formatting without modifying files (exit 1 if unformatted)
        #[arg(long)]
        check: bool,
    },
    /// Start an interactive REPL
    Repl,
    /// Initialize a new Snow project
    Init {
        /// Project name (default: current directory name)
        name: Option<String>,
    },
    /// Start the LSP server (for editor integration)
    Lsp,
}
```

### Ariadne Color Control for --json vs Terminal

```rust
// Source: existing diagnostics.rs pattern + ariadne 0.6 Config API
use ariadne::{Config, Report, Source};

fn render_diagnostic_colored(error: &TypeError, source: &str, filename: &str) -> String {
    // Color enabled (default terminal output)
    let config = Config::default(); // colors ON by default in ariadne 0.6
    // ... build Report with config ...
    let mut buf = Vec::new();
    report.write(Source::from(source), &mut buf).unwrap();
    String::from_utf8(buf).unwrap()
}

fn render_diagnostic_json(error: &TypeError, source: &str, filename: &str) -> String {
    // JSON output -- compute spans and serialize
    let diag = JsonDiagnostic {
        code: error_code(error).to_string(),
        severity: match error { /* warning vs error */ },
        message: error_message(error),
        file: filename.to_string(),
        spans: compute_spans(error, source),
        fix: fix_suggestion_for(error),
    };
    serde_json::to_string(&diag).unwrap()
}
```

### REPL JIT Compilation Pattern

```rust
// Source: inkwell kaleidoscope example pattern, adapted for Snow
use inkwell::context::Context;
use inkwell::execution_engine::ExecutionEngine;
use inkwell::OptimizationLevel;

struct ReplSession<'ctx> {
    context: &'ctx Context,
    execution_engine: ExecutionEngine<'ctx>,
    // Accumulated definitions and type environment
    type_env: snow_typeck::env::TypeEnv,
    definitions: Vec<String>, // Previously entered definitions as source
}

impl<'ctx> ReplSession<'ctx> {
    fn new(context: &'ctx Context) -> Self {
        let module = context.create_module("repl_init");
        let ee = module.create_jit_execution_engine(OptimizationLevel::None)
            .expect("failed to create JIT engine");
        Self {
            context,
            execution_engine: ee,
            type_env: snow_typeck::env::TypeEnv::new(),
            definitions: Vec::new(),
        }
    }

    fn eval(&mut self, input: &str) -> Result<String, String> {
        // 1. Parse input
        let parse = snow_parser::parse(input);
        if !parse.ok() {
            return Err(format_parse_errors(&parse));
        }

        // 2. Type check with accumulated environment
        let typeck = snow_typeck::check_with_env(&parse, &self.type_env);
        if !typeck.errors.is_empty() {
            return Err(format_type_errors(&typeck, input));
        }

        // 3. Create fresh module for this expression
        let module = self.context.create_module("repl_expr");

        // 4. Compile to LLVM IR in the module
        // ... codegen into module ...

        // 5. Add module to execution engine
        self.execution_engine.add_module(&module)
            .map_err(|_| "module already attached")?;

        // 6. Get and call the anonymous function
        let func = unsafe {
            self.execution_engine
                .get_function::<unsafe extern "C" fn() -> i64>("__repl_expr")
        }.map_err(|e| format!("JIT error: {}", e))?;

        let result = unsafe { func.call() };

        // 7. Update accumulated type environment
        // ... merge new bindings ...

        Ok(format_result(result, &typeck.result_type))
    }
}
```

### snow.toml Manifest Format

```toml
# Source: Designed based on Cargo.toml / gleam.toml conventions
[package]
name = "my-project"
version = "0.1.0"
description = "A Snow project"
authors = ["Your Name"]

[dependencies]
# Git-based dependencies (v1)
json_parser = { git = "https://github.com/user/snow-json.git", tag = "v0.2.0" }
http_client = { git = "https://github.com/user/snow-http.git", branch = "main" }

# Path dependencies (for local development)
my_lib = { path = "../my-lib" }

# Future: registry dependencies (field reserved, not implemented in v1)
# some_lib = "1.2.3"
# some_lib = { version = "~> 1.2", features = ["extra"] }
```

### Tower-LSP Minimal Setup

```rust
// Source: tower-lsp 0.20 docs pattern
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

struct SnowLsp {
    client: Client,
    // Per-document state
    documents: dashmap::DashMap<Url, DocumentState>,
}

struct DocumentState {
    source: String,
    parse: snow_parser::Parse,
    typeck: snow_typeck::TypeckResult,
}

#[tower_lsp::async_trait]
impl LanguageServer for SnowLsp {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                definition_provider: Some(OneOf::Left(true)),
                diagnostic_provider: Some(DiagnosticServerCapabilities::Options(
                    DiagnosticOptions::default(),
                )),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        self.analyze_document(params.text_document.uri, params.text_document.text).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        if let Some(change) = params.content_changes.into_iter().last() {
            self.analyze_document(params.text_document.uri, change.text).await;
        }
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        // Look up type at position from TypeckResult.types
        // ...
        Ok(None)
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        // Walk CST to find definition site
        // ...
        Ok(None)
    }
}
```

## Discretionary Recommendations

### Error Tone: Rust-Style Structured (Recommended)

**Recommendation:** Use Rust/ariadne-style structured diagnostics, not Elm-style conversational.

**Rationale:** The existing diagnostics code (crates/snow-typeck/src/diagnostics.rs) already uses ariadne's structured pattern with error codes, labeled spans, and help text. Changing to Elm-style conversational would require rewriting all 21+ existing diagnostic renderers. Rust-style is a proven, respected standard. Snow's existing diagnostics are described as "Go-minimal tone" in the doc comments -- keep this approach and polish it.

**Specific approach:**
- Error code prominently displayed: `[E0001]`
- Primary span with message, secondary spans for context
- `help:` line for fix suggestions (existing pattern)
- Show both definition + call site for type mismatches involving function signatures (dual-span labels, already implemented for IfBranches)

### Multi-Span Strategy

**Recommendation:** Show dual spans when the constraint origin has two distinct locations.

Already partially implemented in the existing `render_diagnostic()` for `IfBranches` (shows then/else spans). Extend this to:
- `FnArg`: show both the function signature (expected) and the call site (found)
- `Return`: show both the return expression and the function's declared return type
- `Assignment`: show both lhs and rhs
- `Annotation`: show both the annotation and the expression

Single span for: `UnboundVariable`, `NotAFunction`, `UnknownVariant` (no second location to show).

### Fix Suggestions: Conservative Selection

**Recommendation:** Add fix suggestions ONLY for these error types (extending existing ones):

Already implemented (keep):
- E0001 Mismatch: Option wrapping, Result wrapping, numeric conversion, to_string
- E0003 ArityMismatch: "missing N arguments" / "N extra arguments"
- E0006 TraitNotSatisfied: "add impl X for Y do ... end"
- E0007 MissingTraitMethod: "add fn X(self) ... end"
- E0009 MissingField: "add field_name: <value>"
- E0012 NonExhaustiveMatch: "add missing patterns or wildcard _"
- W0001 RedundantArm: "remove this arm or reorder"
- E0013 InvalidGuardExpression: "guards must be simple boolean expressions"
- E0017 ReceiveOutsideActor: "move receive into actor block"

New suggestions to add:
- E0004 UnboundVariable: if a similar name exists in scope, suggest "did you mean `similar_name`?" (Levenshtein distance)
- E0005 NotAFunction: if the name is a type, suggest "this is a type, not a function"
- E0010 UnknownVariant: suggest nearest known variant name

### REPL Value Display: Value + Type (Recommended)

**Recommendation:** Display both value and type by default: `42 :: Int`

This matches Snow's type annotation syntax (`::`). Users immediately see what type was inferred. Provide `:type expr` command for examining types of complex expressions without evaluating. Command set:

| Command | Description |
|---------|-------------|
| `:help` | Show available commands |
| `:type <expr>` | Show type without evaluating |
| `:load <file.snow>` | Load definitions from file |
| `:quit` or `:q` | Exit REPL |
| `:clear` | Clear accumulated definitions |
| `:reset` | Reset entire session (type env, scheduler) |

Support `:load` -- it's essential for testing modules interactively.

### Package Manager: Minimal Project Layout

**Recommendation:** `snowc init` creates a minimal layout:

```
my-project/
  snow.toml          # Project manifest
  main.snow          # Entry point (already expected by snowc build)
```

This matches the existing `snowc build` expectation (looks for `main.snow` in the directory). Do not create `src/`, `lib/`, `test/` directories -- keep it flat and simple. The manifest can later support a `[package] main = "src/main.snow"` override.

### Dependency Resolution: Simple DFS with Conflict Detection

**Recommendation:** Simple depth-first resolution. For v1 with git-based deps only:

1. Parse `snow.toml`, collect dependencies
2. For each dependency: clone (or fetch from cache) at specified tag/branch/rev
3. Read the dependency's `snow.toml` for transitive deps
4. DFS recurse, detecting cycles
5. If same package requested at two different revisions, ERROR (user must resolve)
6. Write `snow.lock` with resolved commit SHAs

No version ranges in v1 (git deps point at exact tags/branches). Version constraint syntax reserved for future registry support: `"~> 1.2"` (compatible, like `^1.2` in npm/Cargo).

### Version Constraint Syntax (Reserved for Future)

**Recommendation:** Use Elixir/Hex-style version constraints:

| Syntax | Meaning |
|--------|---------|
| `"1.2.3"` | Exact version |
| `"~> 1.2"` | >= 1.2.0 and < 2.0.0 |
| `"~> 1.2.3"` | >= 1.2.3 and < 1.3.0 |
| `">= 1.0.0"` | Greater than or equal |

This is intuitive, well-established (Elixir/Hex), and parseable by the `semver` crate with minor adaptation. Not implemented in v1 (git deps don't use version ranges) but the syntax should be documented and the field reserved in `snow.toml`.

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Colorless diagnostics only | Color terminal + JSON machine output | Current practice | Editors and CI can consume structured diagnostics |
| Interpreter-based REPLs | JIT compilation REPLs (clang-repl, Julia) | 2020s | Behavior matches compiled code exactly |
| Custom LSP plumbing | tower-lsp trait-based framework | tower-lsp 0.20 | Eliminates protocol boilerplate |
| Formatter as pretty-printer | Formatter as CST-to-IR-to-text pipeline | Biome/2022+ | Better comment handling, idempotency, configurability |
| Registry-first package managers | Git-first with optional registry (Go, early Cargo) | Go modules/2019 | Simpler bootstrapping, no central infrastructure needed |

**Deprecated/outdated:**
- MCJIT is LLVM's legacy JIT. ORC/LLJIT is the modern replacement. However, inkwell only exposes MCJIT. For the REPL, MCJIT via inkwell is sufficient. If performance or incremental compilation becomes an issue, dropping to llvm-sys for ORC access would be the upgrade path.
- lsp-types 0.97 supports LSP 3.16 with proposed 3.17 features. The newer `lsprotocol` crate is auto-generated from the spec but is less battle-tested. Stick with lsp-types via tower-lsp.

## Open Questions

1. **REPL + Codegen Integration**
   - What we know: The existing `snow_codegen::compile()` takes a full `Parse` + `TypeckResult`. The REPL needs to compile individual expressions/definitions.
   - What's unclear: Whether the existing codegen can be called incrementally (compile a single function/expression) or needs a wrapper that synthesizes a full "program" around each REPL input.
   - Recommendation: Create a `compile_repl_expr()` function that wraps the input in a minimal module structure, compiles it, and returns the JIT function name. Reuse as much of the existing codegen as possible.

2. **REPL Previous Definition Accumulation**
   - What we know: The Kaleidoscope pattern re-compiles all previous definitions into each new module.
   - What's unclear: Whether this approach scales as REPL sessions grow (re-compiling 100+ previous definitions per input).
   - Recommendation: Start with re-compilation approach (simple, correct). If performance becomes an issue, switch to symbol resolution across modules using the ExecutionEngine's module list.

3. **LSP Multi-File Support**
   - What we know: The current compiler pipeline (`snow_parser::parse`, `snow_typeck::check`) operates on single files. The LSP needs at least basic multi-file support for imports.
   - What's unclear: How `import` / `from ... import` resolution works across files in the current compiler.
   - Recommendation: Start with single-file LSP (diagnostics, hover, go-to-definition within one file). Multi-file import resolution is a separate concern that may need compiler changes.

4. **Package Manager Dependency Caching**
   - What we know: Git clones are slow. A cache is needed.
   - What's unclear: Where to store the cache (global `~/.snow/cache/` vs project-local `.snow/`).
   - Recommendation: Use `~/.snow/cache/git/` for cloned repos (shared across projects), symlinked or copied into the project's build directory. This follows the Cargo convention.

## Sources

### Primary (HIGH confidence)
- Existing codebase: `crates/snow-typeck/src/diagnostics.rs` -- current ariadne usage pattern with 21+ error codes
- Existing codebase: `crates/snowc/src/main.rs` -- current CLI structure with clap
- Existing codebase: `crates/snow-parser/src/lib.rs` -- rowan CST with lossless trivia
- Existing codebase: `crates/snow-codegen/src/lib.rs` -- inkwell 0.8.0 + LLVM 21 codegen
- Existing codebase: `crates/snow-rt/src/actor/mod.rs` -- scheduler init + global scheduler pattern
- [ariadne docs](https://docs.rs/ariadne/latest/ariadne/) -- v0.6, Report builder, Label, Config, color control
- [tower-lsp docs](https://docs.rs/tower-lsp/latest/tower_lsp/) -- v0.20, LanguageServer trait, LspService setup
- [inkwell ExecutionEngine docs](https://thedan64.github.io/inkwell/inkwell/execution_engine/struct.ExecutionEngine.html) -- add_module, get_function, JIT creation
- [inkwell Kaleidoscope example](https://github.com/TheDan64/inkwell/blob/master/examples/kaleidoscope/main.rs) -- REPL JIT pattern
- [toml crate docs](https://docs.rs/toml/latest/toml/) -- serde-based TOML parsing
- [git2 crate docs](https://docs.rs/git2) -- RepoBuilder for branch/tag checkout

### Secondary (MEDIUM confidence)
- [Biome formatter architecture](https://docs.rs/biome_formatter/latest/biome_formatter/) -- FormatElement IR pattern, CST-based formatting
- [Cargo dependency resolution](https://doc.rust-lang.org/cargo/reference/resolver.html) -- DFS resolver, semver compatibility
- [Wadler-Lindig pretty printer](https://lindig.github.io/papers/strictly-pretty-2000.pdf) -- Algebraic formatting IR theory
- [rust-analyzer architecture](https://rust-analyzer.github.io/book/contributing/architecture.html) -- rowan CST + LSP patterns

### Tertiary (LOW confidence)
- [rustyline v17](https://docs.rs/rustyline) -- docs.rs failed to build v17; API details based on prior versions + GitHub README
- MCJIT vs ORC JIT for REPL performance -- no direct benchmarks found for Snow-like workloads

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- all libraries are already in use (ariadne, rowan, inkwell) or well-established ecosystem standards (tower-lsp, rustyline, toml, git2)
- Architecture: HIGH for diagnostics/formatter/LSP (patterns well-established), MEDIUM for REPL (JIT incremental compilation has known patterns but actor integration is novel)
- Pitfalls: HIGH -- documented from real-world experience with these specific libraries and patterns

**Research date:** 2026-02-07
**Valid until:** 2026-03-07 (30 days -- stable ecosystem, no major version changes expected)
