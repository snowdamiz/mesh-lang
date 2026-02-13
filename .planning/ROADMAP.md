# Roadmap: Mesh

## Milestones

- [x] **v1.0 MVP** - Phases 1-10 (shipped 2026-02-07)
- [x] **v1.1 Language Polish** - Phases 11-15 (shipped 2026-02-08)
- [x] **v1.2 Runtime & Type Fixes** - Phases 16-17 (shipped 2026-02-08)
- [x] **v1.3 Traits & Protocols** - Phases 18-22 (shipped 2026-02-08)
- [x] **v1.4 Compiler Polish** - Phases 23-25 (shipped 2026-02-08)
- [x] **v1.5 Compiler Correctness** - Phases 26-29 (shipped 2026-02-09)
- [x] **v1.6 Method Dot-Syntax** - Phases 30-32 (shipped 2026-02-09)
- [x] **v1.7 Loops & Iteration** - Phases 33-36 (shipped 2026-02-09)
- [x] **v1.8 Module System** - Phases 37-42 (shipped 2026-02-09)
- [x] **v1.9 Stdlib & Ergonomics** - Phases 43-48 (shipped 2026-02-10)
- [x] **v2.0 Database & Serialization** - Phases 49-54 (shipped 2026-02-12)
- [x] **v3.0 Production Backend** - Phases 55-58 (shipped 2026-02-12)
- [x] **v4.0 WebSocket Support** - Phases 59-62 (shipped 2026-02-12)
- [x] **v5.0 Distributed Actors** - Phases 63-69 (shipped 2026-02-13)
- [ ] **v6.0 Website & Documentation** - Phases 70-73 (in progress)

## Phases

<details>
<summary>v1.0 MVP (Phases 1-10) - SHIPPED 2026-02-07</summary>

See milestones/v1.0-ROADMAP.md for full phase details.
55 plans across 10 phases. 52,611 lines of Rust. 213 commits.

</details>

<details>
<summary>v1.1 Language Polish (Phases 11-15) - SHIPPED 2026-02-08</summary>

See milestones/v1.1-ROADMAP.md for full phase details.
10 plans across 5 phases. 56,539 lines of Rust (+3,928). 45 commits.

</details>

<details>
<summary>v1.2 Runtime & Type Fixes (Phases 16-17) - SHIPPED 2026-02-08</summary>

See milestones/v1.2-ROADMAP.md for full phase details.
6 plans across 2 phases. 57,657 lines of Rust (+1,118). 22 commits.

</details>

<details>
<summary>v1.3 Traits & Protocols (Phases 18-22) - SHIPPED 2026-02-08</summary>

See milestones/v1.3-ROADMAP.md for full phase details.
18 plans across 5 phases. 63,189 lines of Rust (+5,532). 65 commits.

</details>

<details>
<summary>v1.4 Compiler Polish (Phases 23-25) - SHIPPED 2026-02-08</summary>

See milestones/v1.4-ROADMAP.md for full phase details.
5 plans across 3 phases. 64,548 lines of Rust (+1,359). 13 commits.

</details>

<details>
<summary>v1.5 Compiler Correctness (Phases 26-29) - SHIPPED 2026-02-09</summary>

See milestones/v1.5-ROADMAP.md for full phase details.
6 plans across 4 phases. 66,521 lines of Rust (+1,973). 29 commits.

</details>

<details>
<summary>v1.6 Method Dot-Syntax (Phases 30-32) - SHIPPED 2026-02-09</summary>

See milestones/v1.6-ROADMAP.md for full phase details.
6 plans across 3 phases. 67,546 lines of Rust (+1,025). 24 commits.

</details>

<details>
<summary>v1.7 Loops & Iteration (Phases 33-36) - SHIPPED 2026-02-09</summary>

See milestones/v1.7-ROADMAP.md for full phase details.
8 plans across 4 phases. 70,501 lines of Rust (+2,955). 34 commits.

</details>

<details>
<summary>v1.8 Module System (Phases 37-42) - SHIPPED 2026-02-09</summary>

See milestones/v1.8-ROADMAP.md for full phase details.
12 plans across 6 phases. 73,384 lines of Rust (+2,883). 52 commits.

</details>

<details>
<summary>v1.9 Stdlib & Ergonomics (Phases 43-48) - SHIPPED 2026-02-10</summary>

See milestones/v1.9-ROADMAP.md for full phase details.
13 plans across 6 phases. 76,100 lines of Rust (+2,716). 56 commits.

</details>

<details>
<summary>v2.0 Database & Serialization (Phases 49-54) - SHIPPED 2026-02-12</summary>

See milestones/v2.0-ROADMAP.md for full phase details.
13 plans across 6 phases. 81,006 lines of Rust (+4,906). 52 commits.

</details>

<details>
<summary>v3.0 Production Backend (Phases 55-58) - SHIPPED 2026-02-12</summary>

See milestones/v3.0-ROADMAP.md for full phase details.
8 plans across 4 phases. 83,451 lines of Rust (+2,445). 33 commits.

</details>

<details>
<summary>v4.0 WebSocket Support (Phases 59-62) - SHIPPED 2026-02-12</summary>

See milestones/v4.0-ROADMAP.md for full phase details.
8 plans across 4 phases. ~84,400 lines of Rust (+~950). 38 commits.

</details>

<details>
<summary>v5.0 Distributed Actors (Phases 63-69) - SHIPPED 2026-02-13</summary>

See milestones/v5.0-ROADMAP.md for full phase details.
20 plans across 7 phases. 93,515 lines of Rust (+9,115). 75 commits.

</details>

### v6.0 Website & Documentation (In Progress)

**Milestone Goal:** Create a polished documentation website and landing page that showcases Mesh's capabilities and documents all language features for developers.

- [x] **Phase 70: Scaffold + Design System** - VitePress custom theme with Tailwind v4, shadcn-vue, dark/light mode, and NavBar (completed 2026-02-13)
- [ ] **Phase 71: Syntax Highlighting + Landing Page** - Mesh code rendering via TextMate grammar and the site's first impression
- [ ] **Phase 72: Docs Infrastructure + Core Content** - Sidebar navigation, docs layout, and core language documentation
- [ ] **Phase 73: Extended Content + Polish** - Remaining docs, search, SEO, and site-wide features

#### Phase 70: Scaffold + Design System
**Goal**: Developers can visit the site and see a styled shell with dark/light mode toggle, responsive layout, and consistent monochrome design -- the foundation every subsequent page builds on
**Depends on**: Nothing (first phase of v6.0)
**Requirements**: INFRA-01, INFRA-02, INFRA-03, INFRA-04, INFRA-05, NAV-05
**Success Criteria** (what must be TRUE):
  1. Running `npm run dev` in /website serves a VitePress site with a blank custom Layout.vue (no default theme styles leak through)
  2. Tailwind utility classes render correctly with monochrome OKLCH colors (gray-50 through gray-950 at zero chroma)
  3. shadcn-vue components (Button, Sheet, etc.) render with the monochrome palette and respect dark/light mode
  4. Clicking the theme toggle switches between dark and light mode, the choice persists across page reloads, and there is no flash of wrong theme on initial load
  5. A NavBar is visible at the top of the page with the Mesh logo/wordmark, navigation links, and the theme toggle
**Plans:** 2 plans
Plans:
- [x] 70-01-PLAN.md -- Scaffold VitePress + Tailwind v4 + shadcn-vue foundation
- [x] 70-02-PLAN.md -- NavBar + ThemeToggle + visual verification

#### Phase 71: Syntax Highlighting + Landing Page
**Goal**: A visitor arriving at the site sees a compelling landing page with properly highlighted Mesh code examples that communicate what the language is and why it matters
**Depends on**: Phase 70
**Requirements**: INFRA-06, SYNTAX-01, SYNTAX-02, LAND-01, LAND-02, LAND-03
**Success Criteria** (what must be TRUE):
  1. Mesh code blocks on any page render with syntax highlighting (keywords, types, operators, strings, comments, do/end blocks, pattern matching all visually distinct)
  2. The landing page hero section displays a tagline, a highlighted Mesh code sample, and a call-to-action link to the docs
  3. A feature showcase section presents 3-4 key Mesh capabilities (actors, pattern matching, type inference, pipe operator) with real highlighted code examples
  4. A "Why Mesh?" section explains Mesh's positioning relative to Elixir, Rust, and Go
  5. Code blocks use a monochrome Shiki theme that matches the site's grayscale aesthetic in both dark and light modes
**Plans**: TBD

#### Phase 72: Docs Infrastructure + Core Content
**Goal**: Developers can navigate a structured documentation site with sidebar, table of contents, and prev/next links, and read complete guides covering the core language (getting started, basics, types, concurrency)
**Depends on**: Phase 71
**Requirements**: NAV-01, NAV-02, NAV-03, NAV-04, DOCS-01, DOCS-02, DOCS-03, DOCS-04, DOCS-09
**Success Criteria** (what must be TRUE):
  1. A collapsible sidebar on docs pages shows all documentation sections organized into groups, with the current page highlighted
  2. On mobile, the sidebar opens as a sheet overlay and auto-closes when a link is tapped
  3. Each docs page shows a right-side "On this page" table of contents listing section headings, and previous/next page links at the bottom
  4. A developer can follow the Getting Started guide from installation through compiling and running their first Mesh program
  5. Documentation covers language basics (variables, types, functions, pattern matching, control flow, pipes, error handling, modules), the type system (inference, generics, structs, sum types, traits, deriving), concurrency (actors, spawning, message passing, linking/monitoring, supervision), and a syntax cheatsheet
**Plans**: TBD

#### Phase 73: Extended Content + Polish
**Goal**: The documentation is complete across all Mesh feature areas (web, database, distributed) and the site has production-quality features (search, SEO, edit links, version badge)
**Depends on**: Phase 72
**Requirements**: DOCS-05, DOCS-06, DOCS-07, DOCS-08, FEAT-01, FEAT-02, FEAT-03, FEAT-04, FEAT-05, FEAT-06, FEAT-07
**Success Criteria** (what must be TRUE):
  1. Documentation covers web features (HTTP server, routing, middleware, WebSocket, rooms/channels, TLS), databases (SQLite, PostgreSQL, pooling, transactions, struct mapping), distributed actors (node connections, remote actors, global registry), and tooling (formatter, REPL, package manager, LSP, editor support)
  2. Typing a search query returns relevant documentation pages via full-text search, and all code blocks have a copy-to-clipboard button
  3. Every page has SEO meta tags (title, description, Open Graph), a custom 404 page exists, and each docs page shows an "Edit on GitHub" link and a last-updated timestamp
  4. A version badge somewhere on the site displays the current Mesh version
**Plans**: TBD

## Progress

**Execution Order:**
Phases execute in numeric order: 70 -> 71 -> 72 -> 73

| Phase | Milestone | Plans Complete | Status | Completed |
|-------|-----------|----------------|--------|-----------|
| 1-10 | v1.0 | 55/55 | Complete | 2026-02-07 |
| 11-15 | v1.1 | 10/10 | Complete | 2026-02-08 |
| 16-17 | v1.2 | 6/6 | Complete | 2026-02-08 |
| 18-22 | v1.3 | 18/18 | Complete | 2026-02-08 |
| 23-25 | v1.4 | 5/5 | Complete | 2026-02-08 |
| 26-29 | v1.5 | 6/6 | Complete | 2026-02-09 |
| 30-32 | v1.6 | 6/6 | Complete | 2026-02-09 |
| 33-36 | v1.7 | 8/8 | Complete | 2026-02-09 |
| 37-42 | v1.8 | 12/12 | Complete | 2026-02-09 |
| 43-48 | v1.9 | 13/13 | Complete | 2026-02-10 |
| 49-54 | v2.0 | 13/13 | Complete | 2026-02-12 |
| 55-58 | v3.0 | 8/8 | Complete | 2026-02-12 |
| 59-62 | v4.0 | 8/8 | Complete | 2026-02-12 |
| 63-69 | v5.0 | 20/20 | Complete | 2026-02-13 |
| 70 | v6.0 | 2/2 | Complete | 2026-02-13 |
| 71 | v6.0 | 0/TBD | Not started | - |
| 72 | v6.0 | 0/TBD | Not started | - |
| 73 | v6.0 | 0/TBD | Not started | - |

**Total: 70 phases shipped across 15 milestones. 192 plans completed. v6.0 in progress (1/4 phases complete).**
