---
phase: 10-developer-tooling
plan: 04
subsystem: repl
tags: [repl, jit, llvm, interactive, session, commands]
completed: 2026-02-07
duration: 10min

dependency-graph:
  requires: [05-llvm-codegen-native-binaries]
  provides: [snow-repl crate, JIT eval engine, REPL commands, multi-line input]
  affects: [10-05]

tech-stack:
  added: [rustyline 15]
  patterns: [LLVM JIT execution, token-based input balancing, command dispatch]

key-files:
  created:
    - crates/snow-repl/Cargo.toml
    - crates/snow-repl/src/lib.rs
    - crates/snow-repl/src/jit.rs
    - crates/snow-repl/src/session.rs
  modified:
    - Cargo.toml
    - crates/snow-codegen/src/codegen/mod.rs
    - crates/snow-typeck/Cargo.toml

decisions:
  - key: repl-jit-per-eval
    value: "LLVM Context created per evaluation (not persistent)"
    reason: "Simplicity; persistent context requires complex lifetime management with execution engines"
  - key: repl-definition-detection
    value: "Keyword-prefix heuristic for definition vs expression classification"
    reason: "Fast, simple, covers all Snow definition forms (fn, let, type, struct, module, actor, service, etc.)"
  - key: repl-multiline-detection
    value: "Token-based do/end and delimiter balancing via snow_lexer"
    reason: "Reuses existing lexer for accurate Snow-aware tokenization"
  - key: repl-display-format
    value: "value :: Type format for expressions, Defined: name :: Type for definitions"
    reason: "Haskell-inspired, clear type visibility"
  - key: codegen-into-module
    value: "Added into_module() public method to CodeGen"
    reason: "Clean API for JIT use -- consumes CodeGen, returns LLVM Module for execution engine creation"

metrics:
  tests: 43
  files-created: 4
  files-modified: 3
---

# Phase 10 Plan 04: REPL with LLVM JIT Summary

Snow REPL crate with JIT compilation engine, session state management, multi-line input detection, and REPL command processing.

## Task Commits

| Task | Name | Commit | Key Changes |
|------|------|--------|-------------|
| 1 | Create snow-repl crate with JIT engine and session management | cc5fbda | snow-repl crate, jit_eval(), ReplSession, into_module() on CodeGen |
| 2 | Multi-line input handling and REPL command processing | f346dbe | is_input_complete(), process_command(), CommandResult, format_result() |

## What Was Built

### JIT Evaluation Engine (jit.rs)
- `jit_eval()` -- full compiler pipeline: parse -> typecheck -> MIR -> LLVM IR -> JIT execute
- `is_definition()` -- keyword-prefix heuristic classifying input as definition or expression
- Expression wrapping in unique `__repl_eval_N` functions for JIT execution
- Definition validation through full parse+typecheck pipeline before storing
- Type-aware result formatting (Int, Bool, Float, Unit, opaque types)
- EvalResult struct carrying value and type information

### Session State Management (session.rs)
- `ReplSession` -- accumulates definitions, tracks eval counter, stores result history
- `wrap_expression()` -- prepends all prior definitions, wraps in unique function
- `add_definition()` -- stores validated definitions for future inputs
- `reset()` -- full session reset capability

### Multi-line Input Detection (lib.rs)
- `is_input_complete()` -- token-based balancing using snow_lexer
- Tracks do/end blocks, parentheses, brackets, braces, and string literals
- Returns false when delimiters are unmatched (triggers continuation mode)

### REPL Command Processing (lib.rs)
- `process_command()` -- dispatches colon-prefixed commands
- `:help` / `:h` -- displays command reference and usage tips
- `:type` / `:t` -- type-checks expression without evaluating, displays inferred type
- `:quit` / `:q` -- clean exit signal
- `:clear` -- ANSI screen clear
- `:reset` -- full session state reset
- `:load <file>` -- reads, validates, and stores file contents as definitions
- CommandResult enum for typed dispatch (Output, TypeInfo, Quit, Continue, Error)

### Result Display
- Expression results: `value :: Type` format (e.g., `42 :: Int`)
- Definition results: `Defined: name :: Type` format
- Unit results suppressed (no output)

### CodeGen API Extension
- Added `into_module()` to `CodeGen<'ctx>` -- consumes CodeGen, returns LLVM Module
- Enables JIT execution engine creation from compiled module

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Fixed snow-typeck missing serde/serde_json dependencies**
- **Found during:** Task 2
- **Issue:** snow-typeck Cargo.toml lacked serde and serde_json deps that diagnostics.rs imports
- **Fix:** Added serde and serde_json to snow-typeck dependencies
- **Files modified:** crates/snow-typeck/Cargo.toml

**2. [Rule 3 - Blocking] Included pre-existing DiagnosticOptions changes**
- **Found during:** Task 2
- **Issue:** snow-typeck source files had uncommitted changes adding DiagnosticOptions parameter to render_errors() -- needed for snow-repl compilation
- **Fix:** Committed alongside Task 2 as the changes were required for the REPL to call render_errors()
- **Files modified:** crates/snow-typeck/src/diagnostics.rs, crates/snow-typeck/src/lib.rs, crates/snow-typeck/tests/integration.rs

**3. [Rule 3 - Blocking] rustyline version 15 (not 17)**
- **Found during:** Task 1
- **Issue:** Plan specified rustyline 17 but workspace already had rustyline 15 configured
- **Fix:** Used existing workspace version (15) rather than adding a conflicting version
- **Files modified:** None (used existing workspace dep)

## Decisions Made

1. **LLVM Context per evaluation** -- simpler than persistent context; execution engine takes ownership of module
2. **Keyword-prefix definition detection** -- checking first token against fn/let/type/struct/module/actor/service keywords
3. **Token-based multi-line detection** -- uses snow_lexer for accurate do/end and delimiter counting
4. **Colorless diagnostics for REPL errors** -- using DiagnosticOptions::colorless() for clean error output
5. **into_module() API** -- clean public method on CodeGen rather than making module field public

## Test Summary

- 42 unit tests + 1 doc test = 43 total
- Session tests: 7 (state management, wrapping, reset)
- JIT tests: 11 (definition detection, formatting, empty input)
- Multi-line tests: 8 (do/end balance, parens, nesting, strings)
- Command tests: 13 (help, quit, clear, reset, type, load, unknown)
- Format tests: 4 (expression, bool, definition, unit display)

## Next Phase Readiness

Plan 05 (REPL CLI integration) can proceed. The following are ready:
- `jit_eval()` for expression evaluation
- `ReplSession` for state management
- `is_input_complete()` for multi-line continuation
- `process_command()` for command dispatch
- `format_result()` for output formatting
- `ReplConfig` for prompt configuration
- `run_repl()` stub ready for rustyline integration

## Self-Check: PASSED
