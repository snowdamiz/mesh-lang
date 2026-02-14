# Project State

## Project Reference

See: .planning/PROJECT.md (updated 2026-02-13)

**Core value:** Expressive, readable concurrency -- writing concurrent programs should feel as natural and clean as writing sequential code, with the safety net of supervision and fault tolerance built into the language.
**Current focus:** v8.0 Developer Tooling -- Phase 85 (Formatting + Audit)

## Current Position

Phase: 85 (5 of 6 in v8.0) -- Formatting + Audit
Plan: 2 of 2 in current phase
Status: Phase 85 complete
Last activity: 2026-02-14 -- Plan 85-02 complete (JIT Runtime Symbol Registration)

Progress: [████████░░] 100%

## Performance Metrics

**All-time Totals:**
- Plans completed: 228
- Phases completed: 85
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
- 83-01: Used actual 48-keyword list from keyword_from_str instead of plan's incorrect list
- 83-01: Whitespace fallback collects top-level names when cursor is past tokens
- 83-01: Sort ordering: 0_scope > 1_types > 2_keywords > 3_snippets for intuitive ranking
- 83-02: Multi-strategy callee type resolution: direct range, NAME_REF children, Ty::Fun containment
- 83-02: Parameter names from CST FN_DEF nodes; type-only labels for built-in functions
- 84-01: Generated icon from SVG using Pillow with 4x supersampling for antialiasing
- 84-01: VSIX at 18KB with zero dev artifacts -- only grammar, compiled JS, and metadata
- 84-02: Publisher ID changed from mesh-lang to OpenWorthTechnologies to match actual Marketplace publisher
- 84-02: Open VSX publish step uses continue-on-error to avoid blocking Marketplace publish
- 84-02: VSIX packaged in separate step with path via step output for reliability
- 85-01: Full-document TextEdit replacement for LSP formatting -- simpler than computing diff edits
- 85-01: Tab size from LSP params.options.tab_size per standard -- users control indent via editor settings
- 85-01: Return None when formatted == source to skip no-op edits
- 85-02: Collection iterator constructors referenced via full module path since not re-exported from lib.rs
- 85-02: mesh_int_to_float/mesh_float_to_int not registered -- codegen intrinsics, not runtime functions

### Pending Todos

None.

### Blockers/Concerns

None. (Phase 83 scope-walk concern resolved -- pattern works correctly.)

### Quick Tasks Completed

| # | Description | Date | Commit | Directory |
|---|-------------|------|--------|-----------|
| 1 | Rename project from Snow to Mesh, change .snow file extension to .mpl | 2026-02-13 | 3fe109e1 | [1-rename-project-from-snow-to-mesh-change-](./quick/1-rename-project-from-snow-to-mesh-change-/) |
| 2 | Write article: How Opus 4.6 and I Built a Production-Ready Programming Language in 9 Days | 2026-02-13 | (current) | [2-mesh-story-article](./quick/2-mesh-story-article/) |
| Phase 82 P01 | 3min | 2 tasks | 4 files |
| Phase 82 P02 | 3min | 2 tasks | 4 files |
| Phase 83 P01 | 4min | 2 tasks | 3 files |
| Phase 83 P02 | 4min | 2 tasks | 3 files |
| Phase 84 P01 | 4min | 2 tasks | 6 files |
| Phase 84 P02 | ~15min | 2 tasks | 3 files |
| Phase 85 P01 | 3min | 2 tasks | 4 files |
| Phase 85 P02 | 3min | 1 task | 1 file |

## Session Continuity

Last session: 2026-02-14
Stopped at: Completed 85-02-PLAN.md (JIT Runtime Symbol Registration) -- Phase 85 complete
Resume file: None
Next action: Plan/execute Phase 86
