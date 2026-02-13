# Research Summary: Mesh Documentation Website

**Domain:** Programming language documentation website with landing page
**Researched:** 2026-02-13
**Overall confidence:** HIGH

## Executive Summary

Building the Mesh documentation website is a well-understood problem with a mature ecosystem. The recommended stack is **VitePress** with a fully custom theme, **Tailwind CSS v4**, **shadcn-vue**, and the existing TextMate grammar for Mesh syntax highlighting via Shiki. This combination provides maximum design control (monochrome aesthetic, dark/light mode) while eliminating weeks of infrastructure work that VitePress handles automatically (markdown compilation, file-based routing, SSG, SPA navigation, search).

The critical architectural decision is using VitePress with a **custom theme** rather than either (a) building from scratch with Vite + Vue + vue-router + vite-ssg, or (b) extending VitePress's default theme. A custom theme gives the same design freedom as a standalone Vue app while leveraging VitePress's content pipeline. The default theme should not be extended because achieving a monochrome aesthetic would require fighting hundreds of scoped CSS rules.

The project benefits from an existing TextMate grammar (`editors/vscode-mesh/syntaxes/mesh.tmLanguage.json`) that Shiki loads natively. This means custom Mesh syntax highlighting requires zero new code -- just a config line in VitePress. The monochrome design is implemented entirely through Tailwind v4's CSS-first `@theme` directive with OKLCH colors at zero chroma.

The primary risks are low: CSS variable naming conflicts between shadcn-vue and Tailwind v4 (mitigated by following the official migration guide), dark mode FOUC (mitigated by an inline head script), and silent grammar fallback in Shiki (mitigated by visual testing). No external services or APIs are required. The entire site deploys as static HTML to any CDN.

## Key Findings

**Stack:** VitePress ^1.6.4 + Vue ^3.5.28 + Tailwind CSS ^4.1.18 + shadcn-vue ^2.4.3 + @vueuse/core ^14.2.1 + lucide-vue-next ^0.563.0. All versions verified against npm registry on 2026-02-13. Shiki syntax highlighting is built into VitePress.

**Architecture:** VitePress custom theme with Layout.vue routing between landing page and docs layout. Three-column docs layout (sidebar / content / ToC). Markdown files in `/docs/` compiled at build time. Static HTML output.

**Critical pitfall:** Do not extend the default VitePress theme. Start with a fully custom theme from day one. Extending the default theme leads to CSS specificity battles that are worse than building from scratch.

## Implications for Roadmap

Based on research, suggested phase structure:

1. **Project Scaffold + Design System** - Foundation phase
   - VitePress project in `/website` with custom theme (blank Layout.vue)
   - Tailwind CSS v4 with `@tailwindcss/vite` plugin + monochrome `@theme`
   - shadcn-vue init with Tailwind v4 compatibility
   - Dark/light mode toggle with VueUse `useDark()` + FOUC prevention script
   - NavBar component (logo, links, theme toggle)
   - Addresses: Table stakes (dark mode, responsive)
   - Avoids: Pitfall 1 (extending default theme), Pitfall 2 (CSS variable conflicts), Pitfall 4 (FOUC)

2. **Mesh Syntax Highlighting + Landing Page** - First impression phase
   - Load existing `mesh.tmLanguage.json` into Shiki via VitePress config
   - Custom monochrome Shiki theme (optional, can use github-light/dark initially)
   - Landing page: hero with tagline, Mesh code sample, feature showcase, CTAs
   - Addresses: Table stakes (landing page, syntax highlighting)
   - Avoids: Pitfall 6 (silent grammar fallback -- test early)

3. **Documentation Infrastructure + Core Content** - Docs phase
   - Sidebar navigation with collapsible groups
   - DocsLayout (sidebar + content + ToC three-column)
   - MobileMenu with auto-close on navigation
   - PrevNext navigation
   - Core documentation: Getting Started, Language Basics, Types, Pattern Matching
   - Concurrency docs: Actors, Supervision, Message Passing
   - Addresses: Table stakes (sidebar, ToC, mobile responsive, prev/next)
   - Avoids: Pitfall 7 (mobile sidebar stays open), Pitfall 9 (landing page in sidebar)

4. **Extended Content + Polish** - Completeness phase
   - Web docs (HTTP, WebSocket, TLS)
   - Database docs (SQLite, PostgreSQL)
   - Distributed docs (Nodes, Remote Actors)
   - Tooling docs (Formatter, REPL, LSP, Package Manager)
   - Full-text search (VitePress MiniSearch -- zero config)
   - Edit-on-GitHub links, last-updated timestamps
   - SEO meta tags, Open Graph image
   - Addresses: Differentiators (search, edit links, timestamps)
   - Avoids: Pitfall 11 (missing social preview), Pitfall 12 (broken links)

**Phase ordering rationale:**
- Phase 1 (scaffold) must come first because all content and components depend on the theme and design system.
- Phase 2 (highlighting + landing) comes next because the landing page is the first impression and syntax highlighting is needed for all code examples.
- Phase 3 (docs infrastructure + core content) requires Phase 1+2 foundation. Writing docs content in parallel with the sidebar/layout is efficient.
- Phase 4 (extended content) is pure content authoring with infrastructure already in place.

**Research flags for phases:**
- Phase 1: shadcn-vue + Tailwind v4 CSS variable integration needs careful setup. Follow official guide precisely.
- Phase 2: Verify Mesh TextMate grammar loads correctly in Shiki. Test all grammar scopes visually.
- Phases 3-4: Standard patterns, unlikely to need additional research.

## Confidence Assessment

| Area | Confidence | Notes |
|------|------------|-------|
| Stack | HIGH | All package versions verified against npm. VitePress 1.6.x confirmed to support Vite 7, Vue 3.5+, and custom Shiki grammars. Tailwind v4 + shadcn-vue compatibility documented. |
| Features | HIGH | Table stakes and differentiators mapped from survey of 5+ programming language docs sites. VitePress capabilities verified against official documentation. |
| Architecture | HIGH | VitePress custom theme is a well-documented pattern used by Vue, Vite, and major ecosystem projects. shadcn-vue in VitePress custom themes has working examples. |
| Pitfalls | HIGH | All pitfalls are well-documented in ecosystem guides. CSS variable conflict is the most complex; official shadcn-vue migration guide covers it. |

## Gaps to Address

- **Custom Shiki theme for monochrome aesthetic:** GitHub Light/Dark themes work out of the box but aren't perfectly monochrome. A custom Shiki JSON theme with neutral token colors would better match the design. Can be created during Phase 2 or deferred (existing themes are acceptable).
- **Interactive code playground:** Infeasible without WASM compiler or server-hosted execution. Explicitly deferred. If desired in a future milestone, would require significant compiler work.
- **Auto-generated API reference:** Requires a doc-comment extraction tool in the Mesh compiler. Not a website concern. Deferred to a future compiler milestone.
- **Content volume:** 25-40 markdown pages covering all language features. This is the bulk of the work (content authoring) and is not a technical risk, but a time investment.

---
*Research completed: 2026-02-13*
*Ready for roadmap: yes*
