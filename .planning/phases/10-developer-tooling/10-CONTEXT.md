# Phase 10: Developer Tooling - Context

**Gathered:** 2026-02-07
**Status:** Ready for planning

<domain>
## Phase Boundary

Developer tools that make Snow practical for daily use: polished error messages with multi-span diagnostics and fix suggestions, an AST-based code formatter, an interactive REPL with incremental compilation, a package manager with git-based dependencies, and an LSP server for editor integration. All tools are subcommands of `snowc` or standalone servers.

</domain>

<decisions>
## Implementation Decisions

### Error message style
- Fix suggestions only when confident the suggestion is correct (avoid misleading suggestions)
- Colorized terminal output by default, `--json` flag for machine-readable output (editors, CI)
- Error codes continue existing E0001+ / W0001+ scheme

### Claude's Discretion (errors)
- Tone and formatting approach (Elm-style conversational vs Rust-style structured)
- Multi-span display strategy (when to show both definition + call site vs call site only)
- Which specific error types get fix suggestions

### Formatter behavior
- Minimal config: a few knobs (line width, indent size) but most decisions fixed
- 2-space indentation as default
- 100-column line width as default
- `snowc fmt` formats in-place by default; `--check` flag for CI (exit 1 if unformatted)

### REPL experience
- Incremental compilation via LLVM (not interpreter) -- behavior identical to compiled code
- Full actor support -- scheduler runs in background, spawn/send/receive work in REPL
- Multi-line input handling needed for do/end blocks

### Claude's Discretion (REPL)
- Value display format (value + type annotation vs value only with :type command)
- Whether to support `:load file.snow` for importing definitions into session
- REPL command set (`:help`, `:type`, `:quit`, etc.)

### Package manager design
- TOML project file (`snow.toml`) for project metadata and dependencies
- Git-based dependencies first, designed so a central registry can be added later without breaking changes
- Lockfile (`snow.lock`) always generated and committed for reproducible builds
- `snowc init` creates project scaffold

### Claude's Discretion (package manager)
- Exact project layout from `snowc init` (minimal vs standard)
- Dependency resolution algorithm
- Version constraint syntax

### LSP server
- No specific preferences discussed -- Claude has full discretion on LSP implementation approach
- Must provide: diagnostics, go-to-definition, type-on-hover (from success criteria)

</decisions>

<specifics>
## Specific Ideas

- Error JSON output format for editor/CI integration alongside colorized terminal output
- Formatter should feel like gofmt/rustfmt -- minimal config, in-place by default
- REPL must handle actors because Snow's identity is concurrent -- can't test the core feature without it
- Package manager follows the Cargo/Go convention: git-first, registry-later

</specifics>

<deferred>
## Deferred Ideas

None -- discussion stayed within phase scope

</deferred>

---

*Phase: 10-developer-tooling*
*Context gathered: 2026-02-07*
