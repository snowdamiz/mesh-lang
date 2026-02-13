# Architecture Patterns: Mesh Documentation Website

**Domain:** Static documentation website (VitePress custom theme + Tailwind CSS + shadcn-vue)
**Researched:** 2026-02-13
**Overall confidence:** HIGH

## Recommended Architecture

VitePress with a **fully custom theme** (no default theme imported). Content authored as Markdown files compiled at build time. Static HTML output deployed to any static host. The site lives in `/website` within the existing Mesh monorepo.

### System Diagram

```
/website/
  .vitepress/
    config.ts                      # VitePress config: sidebar, nav, Shiki, meta
    theme/
      index.ts                     # Theme entry: registers Layout + enhanceApp
      Layout.vue                   # Root layout: routing between landing + docs
      components/
        LandingPage.vue            # Hero, feature showcase, code samples, CTA
        DocsLayout.vue             # Sidebar + content + ToC (three-column)
        NavBar.vue                 # Top nav: logo, links, theme toggle, GitHub
        Sidebar.vue                # Left sidebar: collapsible navigation tree
        SidebarItem.vue            # Recursive sidebar item (supports nesting)
        ThemeToggle.vue            # Dark/light/system mode dropdown
        TableOfContents.vue        # Right sidebar: per-page heading links
        PrevNext.vue               # Bottom navigation: previous/next page
        MobileMenu.vue             # Sheet-based mobile navigation overlay
      styles/
        main.css                   # Tailwind imports + @theme + prose overrides
        shiki-theme.json           # Custom monochrome Shiki theme (optional)
      composables/
        useSidebar.ts              # Sidebar open/close state, route watcher
  docs/
    index.md                       # Landing page (layout: home)
    getting-started/
      index.md
      hello-world.md
    language/
      basics.md
      types.md
      pattern-matching.md
      control-flow.md
      pipe-operator.md
      error-handling.md
      modules.md
    actors/
      introduction.md
      spawn-send-receive.md
      linking-monitoring.md
      supervision.md
      services.md
    web/
      http.md
      websocket.md
      tls.md
    database/
      sqlite.md
      postgresql.md
    distributed/
      nodes.md
      remote-actors.md
    tooling/
      compiler.md
      formatter.md
      lsp.md
      repl.md
      package-manager.md
  public/
    logo.svg
    og-image.png
    favicon.ico
  package.json
  tsconfig.json
```

### Component Boundaries

| Component | Responsibility | Communicates With |
|-----------|---------------|-------------------|
| `Layout.vue` | Root layout. Reads frontmatter `layout` field to choose between landing page and docs layout. Renders NavBar on all pages. | VitePress `useData()`, NavBar, LandingPage, DocsLayout |
| `LandingPage.vue` | Full-width marketing page. Hero section, feature showcase with code samples, CTA buttons. No sidebar. | NavBar, ThemeToggle |
| `DocsLayout.vue` | Three-column docs layout: sidebar (left), content (center), table of contents (right). Renders VitePress `<Content />` in the center column. | Sidebar, TableOfContents, PrevNext, VitePress `<Content />` |
| `NavBar.vue` | Top navigation bar. Logo, main nav links (Docs, GitHub), theme toggle, mobile menu trigger. Fixed position. | ThemeToggle, MobileMenu, VitePress router |
| `Sidebar.vue` | Left sidebar. Renders navigation tree from VitePress `themeConfig.sidebar`. Tracks active page via route. Collapsible sections. | SidebarItem (recursive), VitePress `useRoute()` |
| `SidebarItem.vue` | Single sidebar item. Renders link or collapsible group with children. Highlights when active. | Parent Sidebar, child SidebarItems (recursive) |
| `ThemeToggle.vue` | Dark/light/system mode dropdown. Uses shadcn-vue DropdownMenu + lucide icons. | VueUse `useDark()` / `useColorMode()` |
| `TableOfContents.vue` | Right sidebar (desktop only). Renders heading links for current page. Highlights active heading on scroll. | VitePress `useData()` (page headers), IntersectionObserver |
| `PrevNext.vue` | Bottom-of-page navigation links to previous and next docs pages. | VitePress sidebar config (derives prev/next from page order) |
| `MobileMenu.vue` | Off-canvas navigation for mobile. Uses shadcn-vue Sheet. Contains full sidebar content. Closes on navigation. | Sidebar content, VitePress `useRoute()` (close on navigate) |

### Data Flow

```
BUILD TIME:
  1. Markdown files (.md) in /docs/
       |
       v
  2. markdown-it parses to HTML
     - Shiki highlights code blocks (including ```mesh blocks via custom grammar)
     - Frontmatter extracted (title, description, layout)
       |
       v
  3. HTML wrapped as Vue SFC (each page = Vue component)
       |
       v
  4. Custom theme Layout.vue receives page via <Content />
       |
       v
  5. VitePress generates static HTML for each route (SSG)
       |
       v
  6. Output: /website/.vitepress/dist/ (static HTML + JS + CSS)

RUNTIME (in browser):
  1. Static HTML loads instantly (content visible, no JS needed)
       |
       v
  2. Vue hydrates the page (SPA navigation enabled)
       |
       v
  3. User interactions:
     - Theme toggle -> useDark() -> toggles .dark class on <html>
                    -> localStorage persists preference
     - Sidebar click -> VitePress router -> SPA navigation (no full reload)
     - Mobile menu -> Sheet open/close
     - Scroll -> IntersectionObserver -> TableOfContents highlights heading
     - Search -> MiniSearch queries local index
```

## Patterns to Follow

### Pattern 1: Frontmatter-Driven Layout Switching

**What:** Use markdown frontmatter to determine which layout a page uses.
**When:** Different page types need fundamentally different layouts (landing vs docs).
**Example:**

```markdown
---
# docs/index.md (landing page)
layout: home
title: Mesh Programming Language
---
```

```vue
<!-- Layout.vue -->
<script setup>
import { useData } from 'vitepress'
import NavBar from './components/NavBar.vue'
import LandingPage from './components/LandingPage.vue'
import DocsLayout from './components/DocsLayout.vue'

const { frontmatter, page } = useData()
</script>

<template>
  <NavBar />
  <LandingPage v-if="frontmatter.layout === 'home'" />
  <DocsLayout v-else>
    <Content />
  </DocsLayout>
</template>
```

### Pattern 2: Config-Driven Sidebar

**What:** Define sidebar structure in VitePress config, not in component code.
**When:** Sidebar navigation mirrors docs directory structure.
**Example:**

```typescript
// .vitepress/config.ts
export default defineConfig({
  themeConfig: {
    sidebar: [
      {
        text: 'Getting Started',
        items: [
          { text: 'Installation', link: '/getting-started/' },
          { text: 'Hello World', link: '/getting-started/hello-world' },
        ]
      },
      {
        text: 'Language',
        collapsed: false,
        items: [
          { text: 'Basics', link: '/language/basics' },
          { text: 'Types', link: '/language/types' },
          { text: 'Pattern Matching', link: '/language/pattern-matching' },
        ]
      }
    ]
  }
})
```

The sidebar component reads this from VitePress theme data and renders it. Content organization lives in one place.

### Pattern 3: CSS Custom Properties for Monochrome Theme

**What:** Define the entire color palette as CSS custom properties in Tailwind's `@theme`.
**When:** The design requires a consistent monochrome palette across light and dark modes.
**Example:**

```css
/* styles/main.css */
@import "tailwindcss";
@plugin "@tailwindcss/typography";

@custom-variant dark (&:is(.dark *));

:root {
  --color-bg: oklch(1 0 0);            /* white */
  --color-bg-soft: oklch(0.97 0 0);    /* very light gray */
  --color-bg-muted: oklch(0.93 0 0);   /* light gray */
  --color-text: oklch(0.15 0 0);       /* near-black */
  --color-text-muted: oklch(0.45 0 0); /* medium gray */
  --color-border: oklch(0.90 0 0);     /* border gray */
  --color-accent: oklch(0.15 0 0);     /* emphasis = near-black */
}

.dark {
  --color-bg: oklch(0.10 0 0);         /* near-black */
  --color-bg-soft: oklch(0.14 0 0);    /* dark gray */
  --color-bg-muted: oklch(0.20 0 0);   /* dark gray */
  --color-text: oklch(0.93 0 0);       /* near-white */
  --color-text-muted: oklch(0.60 0 0); /* medium gray */
  --color-border: oklch(0.25 0 0);     /* dark border */
  --color-accent: oklch(0.93 0 0);     /* emphasis = near-white */
}
```

All chroma values are `0` (zero saturation) for monochrome aesthetic. Every component uses semantic color names (`bg-bg`, `text-text`, `border-border`), and dark mode flips automatically.

### Pattern 4: Build-Time Syntax Highlighting with Custom Grammar

**What:** Shiki runs at build time through VitePress's markdown pipeline. The custom Mesh grammar is loaded once at config time.
**When:** Always. Zero runtime JS cost for syntax highlighting.
**Example:**

```typescript
// .vitepress/config.ts
import meshGrammar from '../../editors/vscode-mesh/syntaxes/mesh.tmLanguage.json'

export default defineConfig({
  markdown: {
    languages: [
      {
        ...meshGrammar,
        name: 'mesh',
        scopeName: 'source.mesh',
      }
    ],
    theme: {
      light: 'github-light',
      dark: 'github-dark',
    }
  }
})
```

Markdown code blocks tagged with ` ```mesh ` are highlighted using the Mesh grammar. HTML arrives pre-highlighted -- zero runtime JS.

### Pattern 5: Composable for Sidebar State

**What:** Extract sidebar open/close and auto-close on navigation into a composable.
**When:** Multiple components need sidebar state (NavBar hamburger, Sidebar, MobileMenu).
**Example:**

```typescript
// composables/useSidebar.ts
import { ref, watch } from 'vue'
import { useRoute } from 'vitepress'

const isOpen = ref(false)

export function useSidebar() {
  const route = useRoute()

  const toggle = () => { isOpen.value = !isOpen.value }
  const close = () => { isOpen.value = false }

  // Close sidebar on route change (mobile)
  watch(() => route.path, () => close())

  return { isOpen, toggle, close }
}
```

State is module-level (singleton) so NavBar and MobileMenu share the same `isOpen` ref.

## Anti-Patterns to Avoid

### Anti-Pattern 1: Extending the Default VitePress Theme

**What:** Importing `vitepress/theme` and overriding CSS to achieve monochrome design.
**Why bad:** The default theme has hundreds of scoped styles with high specificity. Achieving a monochrome aesthetic requires overriding most of them with `!important`. Updates to VitePress break overrides. CSS grows unmaintainable.
**Instead:** Use a fully custom theme. Create `Layout.vue` from scratch. Own all CSS.

### Anti-Pattern 2: Installing vue-router

**What:** Adding `vue-router` as a dependency alongside VitePress.
**Why bad:** VitePress has its own router. Two routers conflict, navigation breaks, and SSG pre-rendering fails.
**Instead:** Use VitePress's built-in router. Use `useRoute()` and `useData()` from `vitepress`.

### Anti-Pattern 3: Runtime Markdown Rendering

**What:** Loading markdown files at runtime and rendering them in the browser.
**Why bad:** Adds JS bundle size (markdown-it is ~100KB), causes layout shift, cannot be SSG'd, and code highlighting in the browser is slow.
**Instead:** VitePress compiles markdown at build time. HTML arrives pre-rendered.

### Anti-Pattern 4: Pinia for Documentation State

**What:** Using Pinia for navigation, theme, or page state.
**Why bad:** A docs site has no complex state. Dark mode is a boolean with localStorage. Sidebar state is a boolean. Navigation data is static config. Pinia adds bundle size and boilerplate for zero benefit.
**Instead:** VueUse composables for theme. Module-level refs for sidebar state. VitePress config for navigation.

### Anti-Pattern 5: Client-Side Syntax Highlighting

**What:** Running Shiki or Prism in the browser to highlight code blocks.
**Why bad:** Adds JS payload, causes flash of unstyled code, slower page load.
**Instead:** VitePress highlights at build time. Pre-highlighted HTML. Zero runtime JS.

## Scalability Considerations

| Concern | 10 pages | 100 pages | 500+ pages |
|---------|----------|-----------|------------|
| Build time | <5s | ~15s | ~60s (VitePress is fast; Shiki runs at build time) |
| Bundle size | ~50KB JS | ~60KB JS (code-split per page) | ~80KB JS (shared chunk grows slightly) |
| Sidebar | Simple flat list | Collapsible sections needed | Group into categories; collapse by default |
| Search | Not needed | MiniSearch works well | MiniSearch still works; Algolia if index >5MB |
| Content organization | Flat structure | Group into directories | Multi-level directories, category index pages |
| Navigation | Simple list | Hierarchical with sections | Add breadcrumbs, category pages, prominent search |

VitePress generates one HTML file per page and code-splits JavaScript per page. The architecture scales linearly with content volume. No structural changes needed until 500+ pages.

## Deployment

The `.vitepress/dist/` directory is fully static. Deploy to any static host:

- **GitHub Pages:** Push via GitHub Actions. Simplest for open-source. Free.
- **Vercel:** Auto-detected from Vite config. Zero configuration. Free tier.
- **Netlify:** `netlify deploy --prod --dir=website/.vitepress/dist`. Free tier.

All support clean URLs (`/docs/actors` serves `docs/actors.html`) and SPA fallback routing.

## Integration with Existing Repo

### TextMate Grammar Sharing

The website imports the Mesh TextMate grammar directly from the existing VS Code extension:

```typescript
import meshGrammar from '../../editors/vscode-mesh/syntaxes/mesh.tmLanguage.json'
```

- Single source of truth for Mesh syntax highlighting rules
- Grammar improvements benefit both VS Code extension and website
- Vite resolves the JSON import at build time (no runtime cost)

### Monorepo Structure

```
/                              # Cargo workspace root
+-- Cargo.toml                 # Rust workspace (unchanged)
+-- crates/                    # Rust compiler crates (unchanged)
+-- editors/                   # Editor extensions (unchanged)
|    +-- vscode-mesh/
|         +-- syntaxes/
|              +-- mesh.tmLanguage.json  # SHARED with website
+-- tests/                     # Compiler tests (unchanged)
+-- website/                   # NEW: documentation site
     +-- .vitepress/
     +-- docs/
     +-- package.json
```

The website is a completely independent build target:
- `cargo build` does not affect the website
- `cd website && npx vitepress build` does not affect the Rust crates
- Only coupling: shared TextMate grammar file (read-only from website's perspective)

### .gitignore Additions

```
# Website
website/node_modules/
website/.vitepress/dist/
website/.vitepress/cache/
```

## Sources

- [VitePress custom theme architecture](https://vitepress.dev/guide/custom-theme) -- Layout component contract
- [VitePress runtime API](https://vitepress.dev/reference/runtime-api) -- useData, useRoute, Content component
- [VitePress sidebar config](https://vitepress.dev/reference/default-theme-sidebar) -- sidebar data structure
- [VueUse useDark](https://vueuse.org/core/usedark/) -- dark mode composable pattern
- [Tailwind CSS v4 @theme](https://tailwindcss.com/blog/tailwindcss-v4) -- CSS custom property theming
- [Shiki custom languages](https://shiki.style/guide/load-lang) -- TextMate grammar loading in Shiki
- Programming language docs site analysis: Zig, Gleam, Rust Book, Go, Elixir (common architectural patterns)

---
*Architecture research for: Mesh Language Website & Documentation*
*Researched: 2026-02-13*
