---
phase: 10-developer-tooling
plan: 08
subsystem: lsp
tags: [lsp, tower-lsp, diagnostics, hover, editor-integration]
depends_on:
  requires: [01, 02, 03, 04]
  provides: ["LSP server with diagnostics and hover", "snowc lsp subcommand"]
  affects: [10-09, 10-10]
tech-stack:
  added: [tower-lsp 0.20, tokio 1]
  patterns: ["tower-lsp Backend trait", "document store with Mutex<HashMap>", "UTF-16 position conversion"]
key-files:
  created:
    - crates/snow-lsp/Cargo.toml
    - crates/snow-lsp/src/lib.rs
    - crates/snow-lsp/src/server.rs
    - crates/snow-lsp/src/analysis.rs
  modified:
    - Cargo.toml
    - crates/snowc/Cargo.toml
    - crates/snowc/src/main.rs
decisions:
  - "Mutex<HashMap<String, DocumentState>> for document store (simple, correct for LSP single-client model)"
  - "UTF-16 position conversion per LSP spec (counts char.len_utf16() for non-ASCII)"
  - "Smallest-range lookup for hover type (finds innermost expression at cursor position)"
  - "tokio::runtime::Runtime::new() in snowc main for LSP (avoids #[tokio::main] on synchronous CLI)"
metrics:
  duration: 8min
  completed: 2026-02-07
---

# Phase 10 Plan 08: LSP Server Summary

**One-liner:** Tower-lsp server with parse/type error diagnostics and type-on-hover via smallest-range lookup

## What Was Built

### snow-lsp crate (crates/snow-lsp/)

A complete LSP server implementation using tower-lsp 0.20 that provides:

1. **Document Analysis** (`analysis.rs`):
   - `analyze_document(uri, source)` -- parses and type-checks, produces LSP diagnostics
   - `offset_to_position(source, offset)` -- byte offset to LSP Position (UTF-16 aware)
   - `position_to_offset(source, position)` -- inverse conversion for hover lookups
   - `type_at_position(source, typeck, position)` -- smallest-range type lookup for hover
   - Converts both parse errors and type errors to `lsp_types::Diagnostic`
   - Warnings (e.g., redundant match arms) emitted with WARNING severity

2. **Server Backend** (`server.rs`):
   - `SnowBackend` struct with `Client` and `Mutex<HashMap<String, DocumentState>>`
   - `impl LanguageServer for SnowBackend`:
     - `initialize()` -- advertises Full text sync, hover, definition capabilities
     - `did_open()` / `did_change()` -- triggers analysis, publishes diagnostics
     - `did_close()` -- clears document store and diagnostics
     - `hover()` -- returns type as markdown code block

3. **Public API** (`lib.rs`):
   - `run_server()` async function -- sets up tower-lsp on stdin/stdout

### snowc lsp subcommand

- Added `Lsp` variant to `Commands` enum in snowc CLI
- `snowc lsp` creates a tokio runtime and runs `snow_lsp::run_server()`
- Editors can invoke `snowc lsp` as the language server command

## Task Commits

| Task | Name | Commit | Key Files |
|------|------|--------|-----------|
| 1 | Create snow-lsp crate with tower-lsp server, diagnostics, and hover | 3990305 | snow-lsp/{Cargo.toml, src/lib.rs, src/server.rs, src/analysis.rs} |
| 2 | Add snowc lsp subcommand | ece0cc3 | snowc/{Cargo.toml, src/main.rs} |

## Decisions Made

1. **Document store uses Mutex<HashMap>**: Simple and correct for LSP's single-client model. No need for dashmap or RwLock since operations are sequential per document.

2. **UTF-16 position conversion**: LSP spec mandates UTF-16 code units for character positions. The `offset_to_position` function counts `char.len_utf16()` to handle non-ASCII correctly.

3. **Smallest-range type lookup for hover**: Iterates all typeck ranges containing the cursor offset, returns the one with minimum length. This finds the innermost expression type (e.g., a variable inside a function call).

4. **Synchronous tokio runtime in snowc**: Uses `tokio::runtime::Runtime::new()` + `block_on()` rather than `#[tokio::main]` to keep the CLI's `main()` synchronous and avoid requiring async on the entire binary.

## Deviations from Plan

None -- plan executed exactly as written.

## Test Results

7 tests in snow-lsp:
- `analyze_valid_source_no_diagnostics` -- valid Snow produces empty diagnostics
- `analyze_type_error_produces_diagnostic` -- undefined variable produces ERROR diagnostic
- `offset_to_position_first_line` -- offset 0 and 5 on single line
- `offset_to_position_multiline` -- correct line/character across newlines
- `offset_to_position_at_end` -- end-of-file position
- `position_to_offset_roundtrip` -- every offset in multiline source round-trips
- `server_capabilities` -- initialize returns hover and text sync capabilities

All 56 snowc tests continue to pass.

## Next Phase Readiness

The LSP server is ready for editor integration. Potential enhancements for future plans:
- Go-to-definition (definition_provider is advertised but not implemented)
- Completion support (not yet implemented)
- Document symbols
- Formatting integration with snow-fmt

## Self-Check: PASSED
