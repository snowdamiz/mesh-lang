# Technology Stack: Mesh Website & Documentation

**Project:** Mesh programming language website -- landing page + full documentation with custom syntax highlighting
**Researched:** 2026-02-13
**Confidence:** HIGH (all versions verified against npm registry, official docs checked, integration paths confirmed)

## Design Decision: VitePress, Not Custom Vite + Vue

Use **VitePress** with a fully custom theme rather than building a docs site from scratch with Vite + Vue + vue-router + shadcn-vue. Rationale:

1. **Markdown-first architecture is free.** VitePress gives you file-based routing for `.md` files, frontmatter parsing, sidebar generation from file structure, and SPA navigation -- all out of the box. Building this from scratch with `unplugin-vue-markdown` + `vue-router` + custom sidebar is 2-3 weeks of plumbing work that VitePress eliminates.

2. **Custom themes replace everything.** VitePress supports fully custom themes that completely replace the default UI. The theme entry file is just a Vue component -- same DX as a custom Vite + Vue app. You get VitePress's markdown pipeline, build system, and static generation while owning 100% of the visual design.

3. **Shiki is built in.** VitePress uses Shiki for syntax highlighting with native support for loading custom TextMate grammars. The existing `mesh.tmLanguage.json` in `editors/vscode-mesh/syntaxes/` works directly. No integration plumbing needed.

4. **shadcn-vue still works.** VitePress custom themes are standard Vue 3 SFCs. shadcn-vue components can be registered and used in the layout, sidebar, and any interactive elements.

5. **The Vue and Vite docs themselves use VitePress.** This is the battle-tested path for programming language/framework documentation.

What you lose: nothing meaningful. VitePress custom themes have the same power as a standalone Vue app. The only tradeoff is learning VitePress's config conventions, which takes an hour.

## Design Decision: Tailwind CSS v4 with CSS-First Config

Use **Tailwind CSS v4** with the `@tailwindcss/vite` plugin and CSS-first configuration (no `tailwind.config.js`). Rationale:

1. **Monochrome design maps directly to CSS custom properties.** Tailwind v4's `@theme` directive lets you define a minimal grayscale palette in CSS. No JavaScript config file needed.
2. **Dark mode via `dark:` variant.** Tailwind v4's dark mode support works with the `dark` class on `<html>`, which VueUse's `useDark` toggles.
3. **First-party Vite plugin** (`@tailwindcss/vite`) provides zero-config integration. No PostCSS config, no content globs.

## Design Decision: Reuse Existing TextMate Grammar for Mesh

The project already has a complete TextMate grammar at `editors/vscode-mesh/syntaxes/mesh.tmLanguage.json` covering keywords, types, operators, strings with interpolation, comments, function definitions, and number literals. Shiki (built into VitePress) loads custom TextMate grammars natively via the `langs` config. No new grammar needs to be written.

## Recommended Stack

### Core Framework

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| VitePress | ^1.6.4 | Static site generator, markdown pipeline, build system | Markdown-first docs framework built on Vite + Vue 3. Custom theme support means full design control. Used by Vue, Vite, and most major Vue ecosystem projects for their docs. |
| Vue 3 | ^3.5.28 | UI framework (via VitePress) | Bundled with VitePress. Composition API + `<script setup>` for all custom components. |
| Vite | ^7.3 | Build tool (via VitePress) | Bundled with VitePress 1.6.x. Instant HMR, fast builds. |

### Styling

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| Tailwind CSS | ^4.1.18 | Utility-first CSS framework | Monochrome design is trivially expressed with a minimal `@theme` palette. Dark mode via `dark:` variant. No config file needed in v4. |
| @tailwindcss/vite | ^4.1.18 | Vite plugin for Tailwind v4 | First-party integration. Zero-config content detection, Lightning CSS in production. |
| @tailwindcss/typography | ^0.5.19 | Prose styling for rendered markdown | `prose` classes handle markdown content typography (headings, lists, code blocks, paragraphs). `dark:prose-invert` for dark mode. |

### UI Components

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| shadcn-vue | ^2.4.3 | Component primitives (dropdown menu for theme toggle, sheet for mobile sidebar, scroll-area) | Copy-paste components that own their code. Uses Reka UI under the hood. Tailwind-native. Only install the 3-5 components actually needed -- not a monolithic library. |
| reka-ui | ^2.8.0 | Accessible headless primitives (transitive via shadcn-vue) | Provides WAI-ARIA compliant primitives. Installed as shadcn-vue dependency. |
| lucide-vue-next | ^0.563.0 | Icon library | 1,600+ SVG icons. Tree-shakeable. Used by shadcn-vue for default icons. Sun/Moon icons for theme toggle, Menu icon for mobile nav, Search icon, ChevronRight for sidebar. |

### Syntax Highlighting

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| shiki | (bundled with VitePress) | Code syntax highlighting | Built into VitePress. TextMate grammar engine (same as VS Code). Load `mesh.tmLanguage.json` for Mesh language support. Generates highlighted HTML at build time -- zero runtime JS. |

### Utilities

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| @vueuse/core | ^14.2.1 | Vue composition utilities | `useDark()` + `useToggle()` for dark/light mode with localStorage persistence and system preference detection. `useMediaQuery()` for responsive behavior. `useScrollLock()` for mobile sidebar. |

### Search (Deferred -- Phase 2)

| Technology | Version | Purpose | When |
|------------|---------|---------|------|
| minisearch | ^7.x | Client-side full-text search | Add when docs content is substantial. VitePress supports local search via MiniSearch out of the box with `themeConfig.search.provider: 'local'`. Zero-config. |

## Alternatives Considered

| Category | Recommended | Alternative | Why Not |
|----------|-------------|-------------|---------|
| Framework | VitePress (custom theme) | Custom Vite + Vue + vue-router | 2-3 weeks of plumbing work for markdown pipeline, file-based routing, sidebar generation, static HTML generation. VitePress provides all of this and still allows a fully custom theme. |
| Framework | VitePress (custom theme) | Astro | Astro is excellent for content sites but requires learning a new template language. VitePress gives native Vue SFC support, which is what the user specified. |
| Framework | VitePress (custom theme) | Nuxt Content | Heavier framework. Nuxt adds SSR complexity unnecessary for a static docs site. VitePress is purpose-built for this use case. |
| Styling | Tailwind CSS v4 | Tailwind CSS v3 | v3 requires `tailwind.config.js`, PostCSS config, content glob patterns. v4 is CSS-first, zero-config with Vite plugin, and 5-100x faster builds. |
| Components | shadcn-vue | Headless UI | shadcn-vue has more components, better Vue 3 support, and Tailwind v4 compatibility. Headless UI has fewer components and slower releases. |
| Components | shadcn-vue | Naive UI / Element Plus | These are styled component libraries that fight Tailwind. shadcn-vue owns its styles via Tailwind classes. Monochrome design requires full style control. |
| Dark mode | @vueuse/core useDark | Custom implementation | VueUse handles localStorage persistence, system preference detection, SSR hydration mismatch avoidance, and the HTML class toggle. 4 lines vs 40+ lines of custom code. |
| Syntax highlighting | Shiki (via VitePress) | Prism.js | Prism requires custom language definitions in a different format. Shiki uses TextMate grammars (same as VS Code), and the project already has `mesh.tmLanguage.json`. Zero extra work. |
| Syntax highlighting | Shiki (via VitePress) | Highlight.js | Same argument as Prism. TextMate grammar already exists. Shiki is the modern standard (used by VS Code, GitHub, VitePress). |
| Icons | lucide-vue-next | @iconify/vue | Lucide is what shadcn-vue uses by default. Using the same icon set avoids inconsistency. Both are tree-shakeable. |
| Search | MiniSearch (VitePress built-in) | Algolia DocSearch | DocSearch requires application/approval and external dependency. MiniSearch runs entirely client-side with zero setup. Appropriate for a language docs site. |

## What NOT to Install

These are commonly over-engineered into docs sites:

| Technology | Why Skip |
|------------|----------|
| Pinia (state management) | A docs site has no global state. Dark mode is handled by VueUse. Sidebar state is local component state. |
| vue-router | VitePress handles all routing. Adding vue-router creates conflicts. |
| unplugin-vue-markdown | VitePress already compiles markdown to Vue components. This plugin is for non-VitePress setups. |
| @shikijs/markdown-it | VitePress has Shiki integration built in. This plugin is for standalone markdown-it usage. |
| markdown-it | VitePress bundles and configures markdown-it internally. |
| Nuxt | Server-side rendering is unnecessary for a static docs site. VitePress generates static HTML. |
| CMS (Strapi, Contentful, etc.) | Documentation lives in the repo as markdown files. A CMS adds deployment complexity for zero benefit. |
| i18n | English-only for now. Add later if needed -- VitePress supports i18n natively. |
| Analytics SDK | Add a `<script>` tag for Plausible/Fathom later. No npm dependency needed. |

## Integration Points with Existing Repo

### Directory Structure

```
/website/                          # VitePress project root
  .vitepress/
    config.ts                      # VitePress configuration
    theme/
      index.ts                     # Custom theme entry
      Layout.vue                   # Root layout (replaces default theme)
      components/
        Sidebar.vue                # Docs sidebar navigation
        ThemeToggle.vue            # Dark/light mode toggle
        NavBar.vue                 # Top navigation bar
        CodeBlock.vue              # Custom code block wrapper (if needed)
      styles/
        main.css                   # Tailwind imports + @theme + prose overrides
  docs/                            # Markdown documentation pages
    index.md                       # Landing page
    getting-started/
    language/
    actors/
    stdlib/
    tooling/
  public/                          # Static assets (logo, og-image)
  package.json                     # Website-specific dependencies
  tsconfig.json
```

### TextMate Grammar Reuse

The VitePress config loads the existing grammar from the shared repo:

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
    // Shiki theme configuration for monochrome
    theme: {
      light: 'github-light',   // Or a custom monochrome theme
      dark: 'github-dark',     // Or a custom monochrome theme
    }
  }
})
```

### Monorepo Considerations

- The `/website` directory has its own `package.json` -- completely separate from the Rust workspace.
- No shared `node_modules` with the rest of the repo.
- The only cross-reference is the TextMate grammar import, which uses a relative path.
- CI can build the website independently: `cd website && npm run build`.
- The Rust `Cargo.toml` workspace is unaffected.

## Installation

```bash
# From the repo root
mkdir website && cd website

# Initialize VitePress
npm init -y
npm install vitepress vue

# Tailwind CSS v4
npm install tailwindcss @tailwindcss/vite @tailwindcss/typography

# UI utilities
npm install @vueuse/core

# Icons
npm install lucide-vue-next

# shadcn-vue CLI (for adding individual components)
npx shadcn-vue@latest init

# Then add only the components needed:
npx shadcn-vue@latest add button
npx shadcn-vue@latest add dropdown-menu
npx shadcn-vue@latest add sheet
npx shadcn-vue@latest add scroll-area
npx shadcn-vue@latest add separator
```

### Dev Dependencies

```bash
npm install -D typescript @types/node
```

### Total Dependencies

- **Runtime:** 5 direct dependencies (vitepress, vue, tailwindcss, @vueuse/core, lucide-vue-next)
- **Build:** 3 direct dev dependencies (@tailwindcss/vite, @tailwindcss/typography, typescript)
- **Copy-paste (not npm deps):** shadcn-vue components live in the project source tree
- **Transitive:** reka-ui, shiki, markdown-it (all via vitepress or shadcn-vue)

This is a minimal dependency footprint for a full-featured docs site.

## Version Pinning Summary

| Package | Version | Status | Verified |
|---------|---------|--------|----------|
| vitepress | ^1.6.4 | Latest stable | npm, 2026-02-13 |
| vue | ^3.5.28 | Latest stable | npm, 2026-02-13 |
| tailwindcss | ^4.1.18 | Latest stable | npm, 2026-02-13 |
| @tailwindcss/vite | ^4.1.18 | Latest stable | npm, 2026-02-13 |
| @tailwindcss/typography | ^0.5.19 | Latest stable | npm, 2026-02-13 |
| @vueuse/core | ^14.2.1 | Latest stable (requires Vue 3.5+) | npm, 2026-02-13 |
| lucide-vue-next | ^0.563.0 | Latest stable | npm, 2026-02-13 |
| shadcn-vue | ^2.4.3 | Latest stable (uses Reka UI v2) | npm, 2026-02-13 |
| reka-ui | ^2.8.0 | Latest stable (transitive) | npm, 2026-02-13 |
| shiki | ~3.22.0 | Bundled with VitePress | npm, 2026-02-13 |
| typescript | ^5.x | Dev dependency | stable |

## Key Risks and Mitigations

| Risk | Severity | Mitigation |
|------|----------|------------|
| VitePress custom theme fights Tailwind reset | Low | VitePress custom themes bypass the default theme CSS entirely. Add `@import "tailwindcss"` in the theme's CSS entry point. No conflicts. |
| shadcn-vue Tailwind v4 migration complexity | Low | shadcn-vue 2.x has official Tailwind v4 support and migration guide. CSS variables remap via `@theme` directive. |
| Shiki custom grammar not loading | Low | Verified: Shiki v3 supports `langs` array with inline TextMate grammar objects. The existing `mesh.tmLanguage.json` is a valid TextMate grammar. |
| VitePress version mismatch with Vite 7 | Low | VitePress 1.6.x ships with Vite 7 support. Verified in changelog. |
| @vueuse/core requires Vue 3.5+ | None | VitePress 1.6.x bundles Vue 3.5+. Verified compatibility. |
| Dark mode hydration mismatch | Low | `useDark()` from VueUse handles SSR/SSG hydration correctly by reading the value during `onMounted`. VitePress generates static HTML; hydration adds the dark class on mount. |

## Sources

- [VitePress documentation](https://vitepress.dev/) -- custom themes, markdown config, Shiki integration
- [VitePress custom theme guide](https://vitepress.dev/guide/custom-theme) -- fully replacing default theme
- [VitePress npm](https://www.npmjs.com/package/vitepress) -- v1.6.4, last published ~6 months ago
- [Vite releases](https://vite.dev/releases) -- v7.3.1 current stable
- [Vue.js releases](https://vuejs.org/about/releases) -- v3.5.28 current stable
- [Tailwind CSS v4 announcement](https://tailwindcss.com/blog/tailwindcss-v4) -- CSS-first config, Vite plugin, Lightning CSS
- [Tailwind CSS npm](https://www.npmjs.com/package/tailwindcss) -- v4.1.18
- [@tailwindcss/vite npm](https://www.npmjs.com/package/@tailwindcss/vite) -- v4.1.18
- [@tailwindcss/typography npm](https://www.npmjs.com/package/@tailwindcss/typography) -- v0.5.19
- [shadcn-vue](https://www.shadcn-vue.com/) -- v2.4.3, Reka UI migration, Tailwind v4 support
- [shadcn-vue Tailwind v4 guide](https://www.shadcn-vue.com/docs/tailwind-v4) -- migration steps, CSS variable remapping
- [shadcn-vue dark mode (Vite)](https://www.shadcn-vue.com/docs/dark-mode/vite) -- VueUse integration
- [Reka UI](https://reka-ui.com) -- v2.8.0, accessible headless primitives
- [VueUse useDark](https://vueuse.org/core/usedark/) -- dark mode composable
- [@vueuse/core npm](https://www.npmjs.com/package/@vueuse/core) -- v14.2.1, requires Vue 3.5+
- [Shiki custom languages](https://shiki.style/guide/load-lang) -- loading TextMate grammars
- [shiki npm](https://www.npmjs.com/package/shiki) -- v3.22.0
- [lucide-vue-next npm](https://www.npmjs.com/package/lucide-vue-next) -- v0.563.0
- [VS Code Syntax Highlight Guide](https://code.visualstudio.com/api/language-extensions/syntax-highlight-guide) -- TextMate grammar format reference
- Mesh codebase: `editors/vscode-mesh/syntaxes/mesh.tmLanguage.json` -- existing TextMate grammar for Mesh language

---
*Stack research for: Mesh Language Website & Documentation*
*Researched: 2026-02-13*
