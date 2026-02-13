# Project State

## Project Reference

See: .planning/PROJECT.md (updated 2026-02-13)

**Core value:** Expressive, readable concurrency -- writing concurrent programs should feel as natural and clean as writing sequential code, with the safety net of supervision and fault tolerance built into the language.
**Current focus:** v6.0 Website & Documentation -- Phase 72: Docs Infrastructure + Core Content

## Current Position

Phase: 72 (3 of 4 in v6.0) — Docs Infrastructure + Core Content
Plan: 01 of 04 complete
Status: In Progress
Last activity: 2026-02-13 — Plan 72-01 complete (docs infrastructure foundations)

Progress: [█████░░░░░] 50% (2/4 phases in v6.0)

## Performance Metrics

**All-time Totals:**
- Plans completed: 195
- Phases completed: 71
- Milestones shipped: 15 (v1.0-v5.0)
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
Stopped at: Completed 72-01-PLAN.md
Resume file: None
Next action: Execute 72-02-PLAN.md (docs layout components)
