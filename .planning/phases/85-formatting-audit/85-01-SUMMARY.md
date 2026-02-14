---
phase: 85-formatting-audit
plan: 01
subsystem: tooling
tags: [lsp, formatter, mesh-fmt, mesh-lsp, textDocument/formatting]

# Dependency graph
requires:
  - phase: 80-formatter
    provides: mesh-fmt crate with CST walker and Wadler-Lindig IR printer
  - phase: 81-lsp
    provides: mesh-lsp server with tower-lsp backend
provides:
  - LSP textDocument/formatting handler for VS Code "Format Document"
  - Dedicated formatter handlers for MAP_LITERAL, MAP_ENTRY, LIST_LITERAL, ASSOC_TYPE_BINDING
  - Idempotent formatting for map literals, list literals, and associated type bindings
affects: [85-02-audit, vscode-extension]

# Tech tracking
tech-stack:
  added: [mesh-fmt dependency in mesh-lsp]
  patterns: [full-document TextEdit replacement for LSP formatting, dedicated CST walker dispatch for collection literals]

key-files:
  created: []
  modified:
    - crates/mesh-lsp/Cargo.toml
    - crates/mesh-lsp/src/server.rs
    - crates/mesh-fmt/src/walker.rs
    - crates/mesh-fmt/src/lib.rs

key-decisions:
  - "Full-document replacement via single TextEdit covering line 0 to line_count -- simpler than computing diff edits"
  - "Tab size from LSP params.options.tab_size per LSP standard -- users control indent via editor settings"
  - "Return None when formatted == source to skip no-op edits"

patterns-established:
  - "LSP formatting: parse -> format_source -> compare -> TextEdit if changed"
  - "Collection literal handlers: explicit token dispatch with comma+space, group wrapping"

# Metrics
duration: 3min
completed: 2026-02-14
---

# Phase 85 Plan 01: LSP Formatting + Collection Literal Handlers Summary

**LSP textDocument/formatting wired to mesh-fmt with dedicated handlers for map literals, list literals, and associated type bindings**

## Performance

- **Duration:** 3 min
- **Started:** 2026-02-14T18:05:40Z
- **Completed:** 2026-02-14T18:09:12Z
- **Tasks:** 2
- **Files modified:** 4

## Accomplishments
- LSP server now advertises and handles textDocument/formatting, enabling VS Code "Format Document" for .mpl files
- Dedicated formatter walker handlers for MAP_LITERAL (%{k => v}), MAP_ENTRY, LIST_LITERAL ([1, 2, 3]), and ASSOC_TYPE_BINDING (type Item = Int)
- Added ASSOC_TYPE_DEF, FUN_TYPE, CONS_PAT to explicit inline dispatch list
- All new formatting is verified idempotent (format(format(x)) == format(x))

## Task Commits

Each task was committed atomically:

1. **Task 1: LSP Formatting Handler + Capability** - `0866a604` (feat)
2. **Task 2: Formatter Walker Handlers + Idempotency Tests** - `26389352` (feat)

## Files Created/Modified
- `crates/mesh-lsp/Cargo.toml` - Added mesh-fmt dependency
- `crates/mesh-lsp/src/server.rs` - Added document_formatting_provider capability and formatting handler
- `crates/mesh-fmt/src/walker.rs` - Added walk_map_literal, walk_map_entry, walk_list_literal, walk_assoc_type_binding handlers + tests
- `crates/mesh-fmt/src/lib.rs` - Added idempotency tests for list/map literals, nested lists, assoc type bindings

## Decisions Made
- Full-document TextEdit replacement (single edit spanning entire file) rather than computing per-line diffs -- simpler, reliable, standard LSP practice
- Tab size sourced from LSP DocumentFormattingParams per LSP standard -- users set tab_size in their editor
- Returns None (no edits) when formatted output matches source, avoiding unnecessary document mutations

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered
- Build cache corruption required `rm -rf target` and full rebuild -- resolved automatically, no impact on correctness

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness
- Formatting is live in the LSP server, ready for VS Code testing
- Plan 85-02 (formatting audit) can now test all formatter coverage against real-world Mesh code

## Self-Check: PASSED

All files verified present, all commit hashes found in git log.

---
*Phase: 85-formatting-audit*
*Completed: 2026-02-14*
