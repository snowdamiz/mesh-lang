# Project State

## Project Reference

See: .planning/PROJECT.md (updated 2026-02-13)

**Core value:** Expressive, readable concurrency -- writing concurrent programs should feel as natural and clean as writing sequential code, with the safety net of supervision and fault tolerance built into the language.
**Current focus:** v8.0 Developer Tooling -- Phase 82 (Install Infrastructure)

## Current Position

Phase: 82 (2 of 6 in v8.0) -- Install Infrastructure
Plan: 2 of 2 in current phase
Status: Phase complete
Last activity: 2026-02-14 -- Plan 82-02 complete (Install Scripts)

Progress: [██░░░░░░░░] 33%

## Performance Metrics

**All-time Totals:**
- Plans completed: 222
- Phases completed: 82
- Milestones shipped: 17 (v1.0-v7.0)
- Lines of Rust: ~97,200
- Lines of website: ~5,500
- Timeline: 10 days (2026-02-05 -> 2026-02-14)

## Accumulated Context

### Decisions

(See PROJECT.md Key Decisions table for full log)

- 81-01: Removed nil from constant.language -- Mesh uses None (support.function) not nil
- 81-01: Doc comments use non-italic greener shade to visually distinguish from regular italic comments
- 81-01: Module-qualified call pattern uses lookahead to only match call sites, not field access
- 81-02: Used DocumentSymbolResponse::Nested for hierarchical Outline tree (not flat SymbolInformation)
- 81-02: IMPL_DEF named as "impl TraitName" from PATH child since no NAME child exists
- 81-02: CALL_HANDLER/CAST_HANDLER included as FUNCTION symbols for service body visibility
- 82-01: LLVM 21 via Homebrew on macOS, official tarballs on Linux x86_64/Windows, ycm-core on Linux ARM64
- 82-01: LLVM tarball caching via actions/cache keyed on target to avoid re-download overhead
- 82-02: Install location ~/.mesh/bin with ~/.mesh/env sourced from shell profiles (rustup convention)
- 82-02: Marker-based idempotent PATH configuration using '# Mesh compiler' comment
- 82-02: Checksum verification gracefully degrades (warns and continues if tools unavailable)

### Pending Todos

None.

### Blockers/Concerns

- Phase 83 (Completion): Scope-aware CST walk complexity -- may need prototype before full implementation

### Quick Tasks Completed

| # | Description | Date | Commit | Directory |
|---|-------------|------|--------|-----------|
| 1 | Rename project from Snow to Mesh, change .snow file extension to .mpl | 2026-02-13 | 3fe109e1 | [1-rename-project-from-snow-to-mesh-change-](./quick/1-rename-project-from-snow-to-mesh-change-/) |
| 2 | Write article: How Opus 4.6 and I Built a Production-Ready Programming Language in 9 Days | 2026-02-13 | (current) | [2-mesh-story-article](./quick/2-mesh-story-article/) |
| Phase 82 P01 | 3min | 2 tasks | 4 files |
| Phase 82 P02 | 3min | 2 tasks | 4 files |

## Session Continuity

Last session: 2026-02-14
Stopped at: Completed 82-02-PLAN.md (Install Scripts)
Resume file: None
Next action: Mark Phase 82 complete, begin Phase 83 planning
