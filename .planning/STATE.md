# Project State

## Project Reference

See: .planning/PROJECT.md (updated 2026-02-13)

**Core value:** Expressive, readable concurrency -- writing concurrent programs should feel as natural and clean as writing sequential code, with the safety net of supervision and fault tolerance built into the language.
**Current focus:** v6.0 Website & Documentation -- Phase 73: Extended Content + Polish

## Current Position

Phase: 73 (4 of 4 in v6.0) — Extended Content + Polish
Plan: 03 of 03 complete
Status: Phase Complete
Last activity: 2026-02-13 — Plan 73-03 complete (site features)

Progress: [██████████] 100% (4/4 phases in v6.0)

## Performance Metrics

**All-time Totals:**
- Plans completed: 203
- Phases completed: 73
- Milestones shipped: 16 (v1.0-v6.0)
- Lines of Rust: ~93,500
- Timeline: 9 days (2026-02-05 -> 2026-02-13)

## Accumulated Context

### Decisions

(See PROJECT.md Key Decisions table for full log)

- Quick-1: File extension .mpl chosen for Mesh source files (not .mesh to avoid .mesh/ directory confusion)
- Quick-1: Delete and regenerate snapshots rather than manual edit (simpler, guarantees correctness)
- 70-01: .vitepress/ placed inside docs/ (VitePress source root) for correct theme resolution
- 70-01: config.mts used instead of config.ts for ESM compatibility with VitePress
- 70-01: shadcn-vue neutral base color chosen for zero-chroma monochrome OKLCH palette
- [Phase 70-02]: Used VitePress isDark instead of VueUse useDark to avoid dual localStorage keys fighting
- 71-01: Removed aliases: ['mesh'] from Shiki language config to fix circular alias build error
- 71-01: Used as any casts on TextMate JSON imports per VitePress maintainer recommendation
- 71-02: Used onMounted client-side highlighting with raw code fallback for SSR compatibility
- 71-02: Composed landing page from 3 section components (Hero, Features, WhyMesh) for maintainability
- 72-01: Used VitePress public API only (useData, useRoute, onContentUpdated) -- no vitepress/theme imports
- 72-01: DOM-based heading extraction over page.headers for reliable dynamic content TOC
- 72-02: Created stub docs pages for all sidebar sections to prevent dead link build errors
- 72-03: All code examples verified against e2e test files, not invented
- 72-03: Used Result type T!E syntax and ? operator for error handling docs (not try/catch)
- 72-03: Documented import and from-import module syntax based on e2e tests
- 72-04: All code examples sourced from e2e test files for syntax accuracy
- 72-04: Added Option/Result shorthand syntax (T? and T!E) to Type System docs
- 72-04: Included service auto-generated method naming convention in Concurrency docs
- 73-01: All web and database API names verified against codegen function mapping in mir/lower.rs
- 73-01: WebSocket/TLS/transaction/pooling examples derived from runtime API, marked with comment
- 73-01: Documented actual transaction API (begin/commit/rollback) instead of non-existent execute_batch
- 73-01: JSON documented via Json module (Json.encode, Json.parse, deriving(Json)) not HTTP-specific methods
- 73-02: Distributed examples derived from codegen mapping and runtime source (no e2e tests for distributed features)
- 73-02: Documented Node.spawn and Node.spawn_link based on runtime extern C API signatures
- 73-02: Included mesh.toml manifest format with git and path dependency examples from manifest.rs source
- 73-03: VPNavBarSearch from vitepress/theme for search (zero-config, handles Cmd+K, modal, results)
- 73-03: CSS-only copy button styling (VitePress injects button.copy, we only add CSS)
- 73-03: Separate VitePress CSS variable bridge block in main.css (not merged with OKLCH theme variables)
- 73-03: meshVersion hardcoded in themeConfig (not dynamic from Cargo.toml)

### Research Notes

- VitePress custom theme (blank Layout.vue) -- do NOT extend default theme
- Tailwind CSS v4 with @tailwindcss/vite plugin + @theme directive for monochrome OKLCH
- shadcn-vue with Tailwind v4 CSS variable bridge (follow official migration guide)
- Existing TextMate grammar at editors/vscode-mesh/syntaxes/mesh.tmLanguage.json loads into Shiki
- FOUC prevention via inline head script applying dark class before paint
- Full research in .planning/research/SUMMARY.md

### Pending Todos

None.

### Blockers/Concerns

None.

### Quick Tasks Completed

| # | Description | Date | Commit | Directory |
|---|-------------|------|--------|-----------|
| 1 | Rename project from Snow to Mesh, change .snow file extension to .mpl | 2026-02-13 | 3fe109e1 | [1-rename-project-from-snow-to-mesh-change-](./quick/1-rename-project-from-snow-to-mesh-change-/) |

## Session Continuity

Last session: 2026-02-13
Stopped at: Completed 73-03-PLAN.md (site features) -- Phase 73 complete
Resume file: None
Next action: Phase 73 complete, v6.0 milestone complete
