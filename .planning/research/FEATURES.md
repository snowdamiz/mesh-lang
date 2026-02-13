# Feature Landscape: Mesh Documentation Website

**Domain:** Programming language documentation website with landing page
**Researched:** 2026-02-13
**Confidence:** HIGH (surveyed Elixir, Rust, Zig, Go, Gleam documentation sites; verified VitePress/Shiki capabilities against official docs; reviewed shadcn-vue integration patterns)

## Table Stakes

Features users expect from a programming language documentation site. Missing = the site feels incomplete or amateurish.

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| Landing page hero section | First impression. Every successful language site (Elixir, Rust, Zig, Go) leads with a tagline + code sample + CTA. Without this, visitors bounce. | Medium | Custom VitePress layout with Vue components. |
| Sidebar navigation | The primary navigation pattern for docs sites. Users scan the sidebar to orient themselves. | Low | VitePress provides config-driven sidebar out of the box. Custom theme renders it with shadcn-vue components. |
| Code syntax highlighting | Mesh code examples are the core content. Without highlighting, code blocks are unreadable grey text. | Low | Shiki (built into VitePress) + existing `mesh.tmLanguage.json`. Zero new work for the grammar. |
| Dark/light mode toggle | Expected by developers since ~2020. All major docs sites support it. | Low | VueUse `useDark()` + Tailwind `dark:` variant + shadcn-vue DropdownMenu. |
| Mobile responsive layout | Developers read docs on phones/tablets. Non-responsive = unusable on phones. | Low | Tailwind responsive utilities + shadcn-vue Sheet for mobile sidebar. |
| Full-text search | Users need to find specific topics quickly. Without search, large docs are unnavigable. | Low | VitePress built-in MiniSearch. Zero-config `search.provider: 'local'`. |
| Copy-to-clipboard on code blocks | Developers copy code constantly. Missing = friction on every interaction. | Low | VitePress includes this via markdown config. |
| Page table of contents | Right-side "On this page" ToC for long pages. Shows section headings, highlights current section. | Low | VitePress extracts headings automatically. Custom theme renders them. |
| Getting started guide | The first thing any new user looks for. "How do I install and run hello world?" | Medium (content) | Markdown content. No special tooling. |
| Previous/next page links | Bottom-of-page navigation to continue reading sequentially. | Low | VitePress provides `useData()` with prev/next page info. |
| Anchor links on headings | Click a heading to get a shareable URL. Essential for linking in issues, chat, etc. | Low | VitePress generates anchor links on all headings by default. |
| SEO meta tags | Documentation should be findable via search engines. Title, description, Open Graph tags. | Low | VitePress supports frontmatter-based meta tags. |
| 404 page | Users will hit broken links. A helpful 404 page beats a blank page. | Low | VitePress supports custom 404 layout. |

## Differentiators

Features that set the site apart. Not expected by every visitor, but signal quality and care.

| Feature | Value Proposition | Complexity | Notes |
|---------|-------------------|------------|-------|
| Monochrome design aesthetic | Distinctive visual identity. Communicates technical seriousness (like Zig, Gleam). Stands out from default VitePress/Docusaurus look. | Medium | Custom Tailwind `@theme` with grayscale palette. Custom Shiki code theme. |
| Custom monochrome Shiki theme | Code blocks that match the site's monochrome aesthetic instead of generic VS Code themes. | Medium | Define a custom Shiki JSON theme with neutral token colors. |
| Landing page feature showcase with code | Like Elixir's landing page: show 3-4 features with real Mesh code samples. Proves the language works. | Medium | Custom Vue components with syntax-highlighted code. |
| "Why Mesh?" comparison section | Explains Mesh's niche vs. Elixir (static types + native binaries), vs. Rust (simpler syntax + actor runtime), vs. Go (pattern matching + supervision). | Medium | Content authoring only. |
| Line highlighting in code blocks | Highlight specific lines in code blocks (e.g., `{3-5}`). Useful for tutorials. | Low | VitePress + Shiki support this natively. |
| Code block diff display | Show diffs in code blocks (lines added/removed). Useful for migration guides. | Low | VitePress built-in feature. |
| Collapsible sidebar groups | Group docs into expandable sections. Reduces sidebar overwhelm. | Low | VitePress sidebar config supports nested groups with `collapsed` option. |
| Admonition/callout blocks | Tip, Warning, Danger, Info callout blocks. "WARNING: This function panics if..." | Low | VitePress supports GitHub-style alerts natively. |
| Version badge | Show current Mesh version prominently. Users need to know if docs match their install. | Low | Config variable rendered in layout. |
| Edit this page link | "Edit on GitHub" link on every page. Encourages community contributions. | Low | VitePress built-in `editLink` config. |
| Last updated timestamp | Shows when a page was last modified. Signals freshness. | Low | VitePress built-in `lastUpdated` config via git. |
| Language design philosophy page | Explains Mesh's guiding principles. Helps users understand WHY, not just HOW. | Low | Content authoring only. |
| Syntax cheatsheet | Single-page quick reference for syntax, operators, types. High-value for returning users. | Medium | Dense content authoring with careful formatting. |

## Anti-Features

Features to explicitly NOT build.

| Anti-Feature | Why Avoid | What to Do Instead |
|--------------|-----------|-------------------|
| Browser-based playground | Mesh compiles to native via LLVM. Running in the browser requires WASM compilation of the entire compiler + LLVM + runtime -- infeasible and massive. | Use static code examples with syntax highlighting. Show REPL transcripts as styled output blocks. |
| Auto-generated API docs from source | Mesh does not have a doc-comment extraction tool (like rustdoc or ExDoc). Building one is a compiler feature, not a website feature. | Write stdlib reference docs manually in markdown. |
| Multi-version documentation | Mesh has zero public users. Version selectors add complexity for zero current value. | Ship single-version docs matching current release. |
| Internationalization (i18n) | No community to provide translations. i18n adds routing complexity and stale-content risk. | English only. VitePress supports i18n natively if needed later. |
| Blog | Requires ongoing content commitment. Stale blog signals abandoned project. | Use landing page for announcements. |
| Comment system on docs pages | Comments become stale, off-topic, or support forums. Moderation burden is high. | Link to GitHub Discussions for questions. |
| AI chatbot / "Ask AI" | Third-party dependency, API costs, can hallucinate about a new language with no training data. | Full-text search is sufficient. |
| PDF/EPUB export | Niche use case. Adds build complexity. | Not needed. |
| Analytics tracking | Privacy concern, adds third-party scripts. | If desired later, use Plausible script tag. No npm dependency. |
| Custom CMS | Content is markdown in the repo. CMS adds hosting costs, auth, indirection. | Edit markdown files directly, commit to git. |
| Server-side rendering | Docs are entirely static. SSR adds server costs and complexity for zero benefit. | VitePress SSG. Deploy as static files. |

## Feature Dependencies

```
VitePress Project Scaffold + Custom Theme
  |
  +-- Monochrome Design System (Tailwind @theme + CSS vars)
  |     |
  |     +-- Dark/Light Mode Toggle (useDark + ThemeToggle component)
  |     |
  |     +-- Custom Monochrome Shiki Theme
  |
  +-- Mesh Syntax Highlighting (load mesh.tmLanguage.json into Shiki)
  |     |
  |     +-- Landing Page Hero Code Samples
  |     |
  |     +-- All Documentation Code Examples
  |     |
  |     +-- Syntax Cheatsheet
  |
  +-- Sidebar Navigation (VitePress config + custom Sidebar component)
  |
  +-- Documentation Content (markdown pages)
  |     |
  |     +-- Table of Contents (per page, from headings)
  |     |
  |     +-- Previous/Next Navigation
  |     |
  |     +-- Full-Text Search (needs content to index)
  |
  +-- Landing Page (custom Vue layout)
        |
        +-- Feature Showcase with Code
        |
        +-- "Why Mesh?" Section
```

### Dependency Ordering

1. **VitePress scaffold + Tailwind + shadcn-vue** must come first -- the foundation.
2. **TextMate grammar loading** should come early since it blocks all code examples.
3. **Monochrome design system** (Tailwind @theme + custom Shiki theme) should be established before building pages.
4. **Landing page** and **documentation content** can be authored in parallel once scaffold, grammar, and design system exist.
5. **Search, edit links, timestamps** are VitePress configuration -- add at any point after scaffold.

## MVP Recommendation

Prioritize (Phase 1 -- must ship for the site to be useful):
1. VitePress project scaffold with custom theme (no default theme)
2. Monochrome design system: Tailwind `@theme`, custom Shiki theme, typography
3. Landing page: hero section with tagline, code sample, CTAs, feature showcase
4. Sidebar navigation with collapsible groups + dark/light toggle
5. Mesh syntax highlighting via existing TextMate grammar
6. First docs pages: Getting Started, Language Basics, Types, Pattern Matching
7. Concurrency docs: Actors, Supervision, Message Passing (the differentiator)
8. Mobile responsive layout
9. Copy-to-clipboard on code blocks

Defer (Phase 2 -- add as content grows):
- Full-text search (add after 10+ pages)
- Web/Database/Distributed docs
- Stdlib reference pages
- Syntax cheatsheet
- "Why Mesh?" comparison page
- Edit-on-GitHub links, last-updated timestamps

Defer (Future milestones):
- Browser playground (requires WASM compiler)
- Auto-generated API docs (requires compiler feature)
- Multi-version docs (no users yet)

## Documentation Structure (Sidebar Organization)

Recommended hierarchy based on analysis of Rust Book, Elixir guides, Zig learn page, and Gleam tour:

```
Getting Started
  Installation
  Hello World
  Your First Program

Language Basics
  Variables & Types
  Functions
  Pattern Matching
  Control Flow (if/case/while/for)
  Pipe Operator
  Error Handling (Result, Option, ?)
  Modules & Imports

Type System
  Type Inference
  Generics
  Structs & Sum Types
  Traits & Protocols
  Deriving

Concurrency
  Actor Model Overview
  Spawning Processes
  Message Passing
  Process Linking & Monitoring
  Supervision Trees
  Services (GenServer)

Web
  HTTP Server
  Routing & Middleware
  WebSocket Server
  Rooms & Channels
  TLS / HTTPS

Database
  SQLite
  PostgreSQL
  Connection Pooling
  Transactions
  Struct Mapping (deriving Row)

Serialization
  JSON (deriving Json)

Distributed
  Node Connections
  Remote Actors
  Global Registry

Tooling
  Formatter
  REPL
  Package Manager
  LSP & Editor Support

Reference
  Syntax Cheatsheet
```

This follows progressive disclosure: basics first, then type system depth, then concurrency (the differentiator), then application domains, then advanced topics.

## Sources

- [VitePress features](https://vitepress.dev/guide/what-is-vitepress) -- built-in capabilities
- [VitePress markdown extensions](https://vitepress.dev/guide/markdown) -- code block features, alerts, line highlighting
- [shadcn-vue component list](https://www.shadcn-vue.com/docs/components) -- available primitives
- Programming language documentation sites surveyed: [Elixir](https://elixir-lang.org/), [Rust Book](https://doc.rust-lang.org/book/), [Zig](https://ziglang.org/documentation/), [Go](https://go.dev/doc/), [Gleam](https://gleam.run/)

---
*Feature research for: Mesh Language Website & Documentation*
*Researched: 2026-02-13*
