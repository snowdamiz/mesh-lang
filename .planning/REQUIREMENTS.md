# Requirements: Mesh Website & Documentation

**Defined:** 2026-02-13
**Core Value:** Expressive, readable concurrency -- writing concurrent programs should feel as natural and clean as writing sequential code, with the safety net of supervision and fault tolerance built into the language.

## v6.0 Requirements

Requirements for the Mesh website and documentation site. Each maps to roadmap phases.

### Infrastructure & Design System

- [ ] **INFRA-01**: VitePress project scaffolded in /website with custom theme (blank Layout.vue, no default theme extension)
- [ ] **INFRA-02**: Tailwind CSS v4 integrated with @tailwindcss/vite plugin and monochrome OKLCH palette via @theme directive
- [ ] **INFRA-03**: shadcn-vue initialized with Tailwind v4 compatibility and CSS variable bridge
- [ ] **INFRA-04**: Dark/light mode toggle using VueUse useDark() with localStorage persistence
- [ ] **INFRA-05**: FOUC prevention via inline head script that applies dark class before paint
- [ ] **INFRA-06**: Custom monochrome Shiki code theme matching the site's grayscale aesthetic

### Landing Page

- [ ] **LAND-01**: Hero section with tagline, Mesh code sample with syntax highlighting, and CTA to docs
- [ ] **LAND-02**: Feature showcase section displaying 3-4 key Mesh features with real code examples
- [ ] **LAND-03**: "Why Mesh?" comparison section explaining Mesh's niche vs Elixir, Rust, Go

### Navigation & Layout

- [ ] **NAV-01**: Sidebar navigation with collapsible section groups for docs pages
- [ ] **NAV-02**: Mobile responsive layout with Sheet-based sidebar that auto-closes on navigation
- [ ] **NAV-03**: Per-page table of contents (right-side "On this page" showing section headings)
- [ ] **NAV-04**: Previous/next page links at bottom of docs pages
- [ ] **NAV-05**: NavBar component with logo, navigation links, and theme toggle

### Syntax Highlighting

- [ ] **SYNTAX-01**: Mesh language syntax highlighting via existing TextMate grammar (mesh.tmLanguage.json) loaded into Shiki
- [ ] **SYNTAX-02**: Visual verification that all grammar scopes highlight correctly (keywords, types, operators, strings, comments, pattern matching, do/end blocks)

### Documentation Content

- [ ] **DOCS-01**: Getting Started guide (installation, hello world, first program, compile and run)
- [ ] **DOCS-02**: Language Basics docs (variables, types, functions, pattern matching, control flow, pipe operator, error handling, modules)
- [ ] **DOCS-03**: Type System docs (type inference, generics, structs, sum types, traits, deriving)
- [ ] **DOCS-04**: Concurrency docs (actor model, spawning, message passing, linking/monitoring, supervision, GenServer)
- [ ] **DOCS-05**: Web docs (HTTP server, routing, middleware, WebSocket, rooms/channels, TLS)
- [ ] **DOCS-06**: Database docs (SQLite, PostgreSQL, connection pooling, transactions, struct mapping)
- [ ] **DOCS-07**: Distributed docs (node connections, remote actors, global registry)
- [ ] **DOCS-08**: Tooling docs (formatter, REPL, package manager, LSP, editor support)
- [ ] **DOCS-09**: Syntax cheatsheet (single-page quick reference for syntax, operators, types)

### Site Features

- [ ] **FEAT-01**: Full-text search via VitePress MiniSearch (zero-config local search)
- [ ] **FEAT-02**: Copy-to-clipboard button on all code blocks
- [ ] **FEAT-03**: SEO meta tags (title, description, Open Graph image) on all pages
- [ ] **FEAT-04**: Custom 404 page
- [ ] **FEAT-05**: Edit-on-GitHub link on every docs page
- [ ] **FEAT-06**: Last-updated timestamp on docs pages (via git)
- [ ] **FEAT-07**: Version badge showing current Mesh version

## Future Requirements

Deferred to future milestones. Tracked but not in current roadmap.

### Interactive

- **INTR-01**: Browser-based Mesh playground (requires WASM compiler)
- **INTR-02**: Auto-generated API reference from source (requires doc-comment extraction in compiler)

### Content Expansion

- **CONT-01**: Multi-version documentation with version selector
- **CONT-02**: Internationalization (i18n) with community translations

## Out of Scope

Explicitly excluded. Documented to prevent scope creep.

| Feature | Reason |
|---------|--------|
| Browser playground | Mesh compiles to native via LLVM. WASM compilation of compiler + LLVM + runtime is infeasible |
| Auto-generated API docs | Requires doc-comment extraction tool in the Mesh compiler — a compiler feature, not a website feature |
| Multi-version docs | Zero public users. Version selector adds complexity for zero current value |
| Internationalization (i18n) | No community to provide translations. Adds routing complexity and stale-content risk |
| Blog section | Requires ongoing content commitment. Stale blog signals abandoned project |
| Comment system | Comments become stale/off-topic. Moderation burden. Link to GitHub Discussions instead |
| AI chatbot / "Ask AI" | Third-party dependency, API costs, hallucination risk for a new language with no training data |
| PDF/EPUB export | Niche use case, adds build complexity |
| Analytics tracking | Privacy concern, third-party scripts. If needed later, use Plausible script tag |
| Custom CMS | Content is markdown in the repo. CMS adds hosting costs and indirection |
| Server-side rendering | Docs are entirely static. SSR adds server costs for zero benefit. VitePress SSG is sufficient |

## Traceability

Which phases cover which requirements. Updated during roadmap creation.

| Requirement | Phase | Status |
|-------------|-------|--------|
| INFRA-01 | Phase 70 | Pending |
| INFRA-02 | Phase 70 | Pending |
| INFRA-03 | Phase 70 | Pending |
| INFRA-04 | Phase 70 | Pending |
| INFRA-05 | Phase 70 | Pending |
| INFRA-06 | Phase 71 | Pending |
| LAND-01 | Phase 71 | Pending |
| LAND-02 | Phase 71 | Pending |
| LAND-03 | Phase 71 | Pending |
| NAV-01 | Phase 72 | Pending |
| NAV-02 | Phase 72 | Pending |
| NAV-03 | Phase 72 | Pending |
| NAV-04 | Phase 72 | Pending |
| NAV-05 | Phase 70 | Pending |
| SYNTAX-01 | Phase 71 | Pending |
| SYNTAX-02 | Phase 71 | Pending |
| DOCS-01 | Phase 72 | Pending |
| DOCS-02 | Phase 72 | Pending |
| DOCS-03 | Phase 72 | Pending |
| DOCS-04 | Phase 72 | Pending |
| DOCS-05 | Phase 73 | Pending |
| DOCS-06 | Phase 73 | Pending |
| DOCS-07 | Phase 73 | Pending |
| DOCS-08 | Phase 73 | Pending |
| DOCS-09 | Phase 72 | Pending |
| FEAT-01 | Phase 73 | Pending |
| FEAT-02 | Phase 73 | Pending |
| FEAT-03 | Phase 73 | Pending |
| FEAT-04 | Phase 73 | Pending |
| FEAT-05 | Phase 73 | Pending |
| FEAT-06 | Phase 73 | Pending |
| FEAT-07 | Phase 73 | Pending |

**Coverage:**
- v6.0 requirements: 32 total
- Mapped to phases: 32
- Unmapped: 0

---
*Requirements defined: 2026-02-13*
*Last updated: 2026-02-13 after roadmap creation*
