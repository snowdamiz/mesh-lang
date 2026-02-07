---
phase: 10-developer-tooling
plan: 09
status: complete
dependency-graph:
  requires: ["10-08"]
  provides: ["go-to-definition for variables, functions, types, and module members"]
  affects: ["10-10"]
tech-stack:
  added: []
  patterns: ["source-to-tree offset conversion for whitespace-free rowan CST", "upward CST walk for scope-aware definition resolution"]
key-files:
  created:
    - crates/snow-lsp/src/definition.rs
  modified:
    - crates/snow-lsp/src/server.rs
    - crates/snow-lsp/src/analysis.rs
    - crates/snow-lsp/src/lib.rs
    - crates/snow-lsp/Cargo.toml
    - Cargo.lock
decisions:
  - id: "10-09-01"
    description: "Source-to-tree offset conversion via re-lexing to handle whitespace-free rowan CST"
    rationale: "Snow's lexer skips whitespace, so rowan TextRange offsets differ from source byte offsets. Re-lexing is cheap and gives exact mapping."
  - id: "10-09-02"
    description: "SOURCE_FILE searches all children (forward references) while BLOCK searches only earlier siblings"
    rationale: "Top-level functions are often called before definition; inside blocks, let bindings must precede usage."
metrics:
  duration: 10min
  completed: 2026-02-07
---

# Phase 10 Plan 09: Go-to-Definition Summary

Go-to-definition via CST traversal with source/tree coordinate conversion for the whitespace-free rowan CST.

## Task Commits

| Task | Name | Commit | Key Changes |
|------|------|--------|-------------|
| 1 | Go-to-definition via CST traversal | `41882c2` | definition.rs, server.rs goto_definition handler, offset conversion |
| 2 | LSP integration tests | `26d790d` | Comprehensive tests for diagnostics, hover, go-to-def, position conversion |

## What Was Built

### Go-to-definition engine (definition.rs)

- `find_definition(source, root, source_offset)` -- main entry point
- Resolves NAME_REF tokens (variable/function references) by walking upward through BLOCK and SOURCE_FILE nodes
- Resolves function parameters by checking PARAM_LIST in enclosing FN_DEF/CLOSURE_EXPR
- Resolves type names in TYPE_ANNOTATION context to STRUCT_DEF/SUM_TYPE_DEF/TYPE_ALIAS_DEF
- Resolves module-qualified names (e.g., MyModule.my_func) by finding MODULE_DEF then searching its FN_DEFs
- Returns None for built-in modules (IO, String, List, Map, etc.)
- Forward reference support: SOURCE_FILE searches all definitions; BLOCK searches only earlier siblings

### Source/Tree Offset Conversion

- `source_to_tree_offset(source, offset)` -- converts source byte offset to rowan tree offset
- `tree_to_source_offset(source, tree_offset)` -- inverse conversion
- Necessary because Snow's lexer skips whitespace, so rowan CST omits spaces
- Uses re-lexing to build token span mapping

### LSP Server Integration

- `goto_definition` handler in server.rs
- Converts LSP Position -> source byte offset -> tree offset for CST lookup
- Converts tree TextRange result back -> source byte offset -> LSP Position/Location
- Returns `GotoDefinitionResponse::Scalar(location)` with same-document URI

### Comprehensive Test Suite (31 tests total)

- **Diagnostics** (6): valid source, valid function, type errors with range, parse errors, multiple errors
- **Hover** (2): integer literal, empty space returns None
- **Go-to-definition** (12): let binding, function call, function params, nested scope shadowing, unknown returns None, struct name at definition site, type annotation, builtin returns None
- **Position conversion** (7): single-line, multi-line, at-end, roundtrip, past-EOF
- **Offset mapping** (4): source-to-tree, tree-to-source, roundtrip verification

## Decisions Made

1. **Source-to-tree offset conversion via re-lexing** -- The Snow lexer skips whitespace, making rowan TextRange offsets different from source byte offsets. Rather than modifying the lexer (which would affect the entire compiler), we re-lex on each definition lookup to build the mapping. This is cheap for single-file LSP operations.

2. **Forward references at top level only** -- SOURCE_FILE searches all children regardless of position (supporting forward references to functions defined later in the file). BLOCK nodes only search earlier siblings (let bindings must be defined before use).

3. **Definition site returns None** -- When the cursor is on the definition itself (e.g., the NAME in `fn add`), find_definition returns None rather than pointing to itself. This matches standard LSP behavior.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Rowan CST coordinate system mismatch**

- **Found during:** Task 1
- **Issue:** Rowan TextRange offsets do not match source byte offsets because the Snow lexer skips whitespace, producing a CST without space tokens. This caused "Bad offset" panics in tests.
- **Fix:** Added source_to_tree_offset and tree_to_source_offset functions that re-lex the source to build token span mapping. Updated find_definition to accept source text and perform the conversion.
- **Files modified:** crates/snow-lsp/src/definition.rs
- **Commit:** 41882c2

**2. [Rule 3 - Blocking] Missing snow-lexer dependency**

- **Found during:** Task 1
- **Issue:** definition.rs uses snow_lexer::Lexer::tokenize for offset conversion, but snow-lsp did not depend on snow-lexer.
- **Fix:** Added snow-lexer dependency to Cargo.toml.
- **Files modified:** crates/snow-lsp/Cargo.toml
- **Commit:** 41882c2

## Next Phase Readiness

- Go-to-definition, hover, and diagnostics all working and tested
- LSP server advertises definitionProvider capability (was already advertised in 10-08)
- The rowan coordinate mismatch is a known pre-existing limitation that also affects hover accuracy for multi-token expressions (hover works within the type map's rowan ranges, not source byte offsets). This should be addressed in a future plan if needed.
- Ready for Plan 10 (completion/references or whatever remains)

## Self-Check: PASSED
