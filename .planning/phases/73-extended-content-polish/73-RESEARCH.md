# Phase 73: Extended Content + Polish - Research

**Researched:** 2026-02-13
**Domain:** VitePress extended documentation content (web, database, distributed, tooling) + production site features (search, SEO, copy button, 404, edit link, last-updated, version badge)
**Confidence:** HIGH

## Summary

Phase 73 has two distinct workstreams: (1) writing four new documentation sections covering Mesh's web features, database layer, distributed actor system, and developer tooling; and (2) adding production-quality site features including full-text search, copy-to-clipboard on code blocks, SEO meta tags, a custom 404 page, edit-on-GitHub links, last-updated timestamps, and a version badge.

The documentation content work follows the same pattern established in Phase 72: write markdown files using `mesh` code fences, source all code examples from the runtime source (`crates/mesh-rt/src/`) and e2e test files (`tests/e2e/`), and register new pages in the VitePress sidebar config. The key difference from Phase 72 is that some Phase 73 topics (WebSocket rooms/channels, distributed actors, connection pooling, TLS) lack e2e test files -- the code examples must be derived from the runtime implementation and codegen function signatures. The Mesh-level API surface is: `HTTP.*`, `Request.*`, `Ws.*`, `Sqlite.*`, `Pg.*`, `Pool.*`, `Node.*`, `Global.*`, and `Process.*`.

The site feature work leverages VitePress built-in capabilities. Search uses VitePress's built-in MiniSearch local search, which is trivially enabled via `themeConfig.search.provider: 'local'` and now has `VPNavBarSearch` officially exported from `vitepress/theme` (confirmed in the installed v1.6.4). The copy-to-clipboard button is already injected by VitePress's markdown renderer on all code blocks (the `<button class="copy">` element is present regardless of theme) -- we just need CSS to style it. SEO uses `transformPageData` to dynamically inject Open Graph meta tags. Last-updated uses `lastUpdated: true` in site config with git timestamps exposed via `useData().page.value.lastUpdated`. Edit links use `themeConfig.editLink.pattern`. The 404 page uses `useData().page.value.isNotFound`. The version badge reads from a static source (the meshc Cargo.toml version `0.1.0`).

**Primary recommendation:** Split work into two streams -- (A) content plans for the four doc sections, each writing markdown with sidebar config updates, (B) a features plan that adds search, copy button styling, SEO, 404, edit link, last-updated, and version badge to the VitePress config and custom theme components.

## Standard Stack

### Core (already installed from Phases 70-72)

| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| VitePress | 1.6.4 | SSG with built-in MiniSearch, copy button, lastUpdated, transformPageData | All needed features are built-in or configurable |
| Vue 3 | 3.5.28 | Components for 404 page, version badge, edit link, last-updated display | Already installed |
| Tailwind CSS v4 | 4.1.18 | Styling for copy button, 404 page, metadata components | Already installed |
| lucide-vue-next | 0.564.0 | Icons for edit link (Pencil/ExternalLink), search button, copy icon | Already installed |

### From vitepress/theme (use existing exports)

| Export | Purpose | Notes |
|--------|---------|-------|
| `VPNavBarSearch` | Search UI component (search button + modal) | Officially exported in v1.6.4 from `vitepress/theme` -- confirmed in `theme.d.ts` |

### New Dependencies

**None.** All features are achievable with existing packages and VitePress built-in capabilities.

### Alternatives Considered

| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| VitePress built-in MiniSearch | Algolia DocSearch | Algolia requires external service, API key, indexing. MiniSearch is zero-config, client-side, no external dependency. Use MiniSearch. |
| VPNavBarSearch from `vitepress/theme` | Custom search component importing `@localSearchIndex` | Custom component would need to handle MiniSearch initialization, UI, keyboard shortcuts. VPNavBarSearch does all of this. Use the official export. |
| CSS-only copy button styling | Custom Vue component wrapping code blocks | VitePress already injects `<button class="copy">` and handles the clipboard JS. We only need CSS. Don't over-engineer. |
| `transformPageData` for SEO | Per-page frontmatter `head:` entries | transformPageData is DRY -- one function handles all pages automatically. Frontmatter would require manual entries on every page. Use transformPageData. |
| Git-based lastUpdated | Manual dates in frontmatter | Git timestamps are automatic and accurate. Use VitePress built-in. |

## Architecture Patterns

### New Files and Modifications

```
website/docs/.vitepress/
  config.mts                              # MODIFY: add search, lastUpdated, editLink, transformPageData, sidebar entries
  theme/
    Layout.vue                            # MODIFY: add 404 detection, search component placement
    index.ts                              # MODIFY: (unchanged from Phase 72)
    components/
      NavBar.vue                          # MODIFY: add VPNavBarSearch import and placement
      docs/
        DocsLayout.vue                    # MODIFY: add edit link, last-updated, version badge
        DocsEditLink.vue                  # NEW: "Edit on GitHub" link component
        DocsLastUpdated.vue               # NEW: Last-updated timestamp component
        DocsVersionBadge.vue              # NEW: Version badge component
      NotFoundPage.vue                    # NEW: Custom 404 page
    styles/
      code.css                            # MODIFY: add copy button styling

website/docs/
  docs/
    web/
      index.md                            # NEW: DOCS-05 - HTTP server, routing, middleware, WebSocket, rooms, TLS
    databases/
      index.md                            # NEW: DOCS-06 - SQLite, PostgreSQL, pooling, transactions, struct mapping
    distributed/
      index.md                            # NEW: DOCS-07 - Node connections, remote actors, global registry
    tooling/
      index.md                            # NEW: DOCS-08 - Formatter, REPL, package manager, LSP, editor support
```

### Pattern 1: VitePress Config for Search + SEO + Features

**What:** Enable all production features through VitePress site config.
**When:** Single config modification covers search, lastUpdated, editLink, and SEO.

```typescript
// .vitepress/config.mts
import { defineConfig } from 'vitepress'

export default defineConfig({
  // ... existing config ...

  // FEAT-06: Enable git-based last-updated timestamps
  lastUpdated: true,

  // FEAT-03: Dynamic SEO meta tags
  transformPageData(pageData) {
    const canonicalUrl = `https://meshlang.org/${pageData.relativePath}`
      .replace(/index\.md$/, '')
      .replace(/\.md$/, '.html')

    pageData.frontmatter.head ??= []
    pageData.frontmatter.head.push(
      ['link', { rel: 'canonical', href: canonicalUrl }],
      ['meta', { property: 'og:title', content: pageData.title + ' | Mesh' }],
      ['meta', { property: 'og:description', content: pageData.description }],
      ['meta', { property: 'og:url', content: canonicalUrl }],
      ['meta', { property: 'og:type', content: 'article' }],
    )
  },

  // Global head tags (site-wide SEO defaults)
  head: [
    ['meta', { property: 'og:site_name', content: 'Mesh Programming Language' }],
    ['meta', { name: 'twitter:card', content: 'summary' }],
  ],

  themeConfig: {
    // FEAT-01: Zero-config local search via MiniSearch
    search: {
      provider: 'local',
    },

    // FEAT-05: Edit-on-GitHub link
    editLink: {
      pattern: 'https://github.com/user/mesh/edit/main/website/docs/:path',
      text: 'Edit this page on GitHub',
    },

    // Extended sidebar with new sections
    sidebar: {
      '/docs/': [
        {
          text: 'Getting Started',
          items: [
            { text: 'Introduction', link: '/docs/getting-started/' },
          ],
        },
        {
          text: 'Language Guide',
          collapsed: false,
          items: [
            { text: 'Language Basics', link: '/docs/language-basics/' },
            { text: 'Type System', link: '/docs/type-system/' },
            { text: 'Concurrency', link: '/docs/concurrency/' },
          ],
        },
        {
          text: 'Web & Networking',
          collapsed: false,
          items: [
            { text: 'Web', link: '/docs/web/' },
          ],
        },
        {
          text: 'Data',
          collapsed: false,
          items: [
            { text: 'Databases', link: '/docs/databases/' },
          ],
        },
        {
          text: 'Distribution',
          collapsed: false,
          items: [
            { text: 'Distributed Actors', link: '/docs/distributed/' },
          ],
        },
        {
          text: 'Tooling',
          collapsed: false,
          items: [
            { text: 'Developer Tools', link: '/docs/tooling/' },
          ],
        },
        {
          text: 'Reference',
          collapsed: false,
          items: [
            { text: 'Syntax Cheatsheet', link: '/docs/cheatsheet/' },
          ],
        },
      ],
    },
    outline: { level: [2, 3], label: 'On this page' },
  },
})
```

Source: [VitePress Site Config](https://vitepress.dev/reference/site-config), verified against installed v1.6.4

### Pattern 2: Search Integration in Custom Theme NavBar

**What:** Import VPNavBarSearch from `vitepress/theme` and place it in the custom NavBar.
**When:** Adding search to the custom theme.

```vue
<!-- NavBar.vue -->
<script setup lang="ts">
import { VPNavBarSearch } from 'vitepress/theme'
import ThemeToggle from './ThemeToggle.vue'
// ...
</script>

<template>
  <header class="...">
    <div class="...">
      <!-- ... logo, nav links ... -->
      <div class="flex items-center gap-2">
        <VPNavBarSearch />
        <ThemeToggle />
      </div>
    </div>
  </header>
</template>
```

Source: VitePress theme.d.ts exports (confirmed in installed node_modules), [GitHub Issue #4476](https://github.com/vuejs/vitepress/issues/4476)

**Key detail:** `VPNavBarSearch` handles everything: the search button, keyboard shortcut (Cmd/Ctrl+K), the modal dialog, search index loading, result display, and navigation. No additional configuration needed beyond `themeConfig.search.provider: 'local'`.

### Pattern 3: Copy Button Styling (CSS Only)

**What:** Style the `<button class="copy">` element that VitePress already injects into every code block.
**When:** Making the copy button visible and themed.

VitePress's markdown renderer injects `<button class="copy">` into every `div[class*='language-']` block. The app-level `useCopyCode()` composable handles the clipboard logic via a global click listener. We only need CSS.

```css
/* code.css -- add to existing file */

/* Copy button (VitePress injects button.copy in every code block) */
div[class*='language-'] > button.copy {
  direction: ltr;
  position: absolute;
  top: 12px;
  right: 12px;
  z-index: 3;
  display: block;
  justify-content: center;
  align-items: center;
  border-radius: 4px;
  width: 40px;
  height: 40px;
  border: 1px solid var(--border);
  background-color: var(--muted);
  background-image: var(--vp-icon-copy);
  background-position: 50%;
  background-size: 20px;
  background-repeat: no-repeat;
  cursor: pointer;
  opacity: 0;
  transition: opacity 0.15s ease, border-color 0.15s ease;
}

div[class*='language-']:hover > button.copy,
div[class*='language-'] > button.copy:focus {
  opacity: 1;
}

div[class*='language-'] > button.copy:hover,
div[class*='language-'] > button.copy.copied {
  border-color: var(--muted-foreground);
  background-color: var(--secondary);
}

div[class*='language-'] > button.copy.copied::before {
  position: relative;
  top: -1px;
  transform: translateX(-50%);
  display: flex;
  justify-content: center;
  align-items: center;
  border: 1px solid var(--border);
  border-radius: 4px;
  padding: 0 10px;
  width: fit-content;
  height: 40px;
  font-size: 12px;
  font-weight: 500;
  color: var(--foreground);
  background-color: var(--muted);
  white-space: nowrap;
  content: 'Copied';
}

/* Language label */
div[class*='language-'] > span.lang {
  position: absolute;
  top: 6px;
  right: 12px;
  z-index: 2;
  font-size: 12px;
  font-weight: 500;
  color: var(--muted-foreground);
  transition: opacity 0.15s ease;
}

/* Hide language label when copy button appears */
div[class*='language-']:hover > button.copy + span.lang,
div[class*='language-'] > button.copy:focus + span.lang {
  opacity: 0;
}

/* CSS variables for copy icon SVG */
:root {
  --vp-icon-copy: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' fill='none' stroke='rgba(128,128,128,1)' stroke-linecap='round' stroke-linejoin='round' stroke-width='2' viewBox='0 0 24 24'%3E%3Crect width='8' height='4' x='8' y='2' rx='1' ry='1'/%3E%3Cpath d='M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2'/%3E%3C/svg%3E");
}
```

Source: VitePress default theme `vp-doc.css` styles (read from installed `node_modules/vitepress/dist/client/theme-default/styles/components/vp-doc.css`), VitePress `app/composables/copyCode.js` (click handler logic)

### Pattern 4: Custom 404 Page

**What:** Detect `page.isNotFound` and render a custom 404 component.
**When:** User navigates to a non-existent route.

```vue
<!-- Layout.vue -->
<script setup lang="ts">
import { useData } from 'vitepress'
const { page, frontmatter } = useData()
</script>

<template>
  <div class="min-h-screen bg-background text-foreground">
    <NavBar />
    <NotFoundPage v-if="page.isNotFound" />
    <LandingPage v-else-if="frontmatter.layout === 'home'" />
    <DocsLayout v-else-if="hasSidebar" />
    <main v-else class="mx-auto max-w-4xl px-4 py-8">
      <Content />
    </main>
  </div>
</template>
```

Source: [VitePress Custom Theme Guide](https://vitepress.dev/guide/custom-theme), [VitePress Runtime API - useData](https://vitepress.dev/reference/runtime-api#usedata)

### Pattern 5: Edit Link + Last Updated Components

**What:** Custom components that read from VitePress data APIs to show edit link and last-updated.
**When:** Bottom of every docs page.

```vue
<!-- DocsEditLink.vue -->
<script setup lang="ts">
import { useData } from 'vitepress'
import { computed } from 'vue'

const { theme, page } = useData()

const editLink = computed(() => {
  const { editLink } = theme.value
  if (!editLink?.pattern) return null
  const url = editLink.pattern.replace(':path', page.value.relativePath)
  return { url, text: editLink.text || 'Edit this page' }
})
</script>

<template>
  <a v-if="editLink" :href="editLink.url" target="_blank" rel="noopener noreferrer"
     class="inline-flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground transition-colors">
    {{ editLink.text }}
  </a>
</template>
```

```vue
<!-- DocsLastUpdated.vue -->
<script setup lang="ts">
import { useData } from 'vitepress'
import { computed } from 'vue'

const { page } = useData()

const lastUpdated = computed(() => {
  const timestamp = page.value.lastUpdated
  if (!timestamp) return null
  return new Date(timestamp).toLocaleDateString('en-US', {
    year: 'numeric',
    month: 'long',
    day: 'numeric',
  })
})
</script>

<template>
  <span v-if="lastUpdated" class="text-sm text-muted-foreground">
    Last updated: {{ lastUpdated }}
  </span>
</template>
```

Source: [VitePress useData API](https://vitepress.dev/reference/runtime-api#usedata) -- `page.value.lastUpdated` is a number (unix timestamp) when `lastUpdated: true` is set in config. `theme.value.editLink` contains the pattern and text from themeConfig.

### Pattern 6: Version Badge

**What:** A small component displaying the current Mesh version.
**When:** Shown somewhere visible (navbar, sidebar, or footer).

The version comes from `crates/meshc/Cargo.toml` (currently `0.1.0`). For the static site, the simplest approach is to define the version in VitePress config (or as a build-time constant) rather than reading Cargo.toml at runtime.

```typescript
// config.mts -- add to themeConfig
themeConfig: {
  meshVersion: '0.1.0',
  // ... rest of config
}
```

```vue
<!-- DocsVersionBadge.vue -->
<script setup lang="ts">
import { useData } from 'vitepress'
const { theme } = useData()
</script>

<template>
  <span class="inline-flex items-center rounded-md border border-border px-2 py-0.5 text-xs font-medium text-muted-foreground">
    v{{ theme.meshVersion }}
  </span>
</template>
```

### Anti-Patterns to Avoid

- **Extending VitePress default theme for search only:** Our theme is fully custom. Do NOT use `extends: DefaultTheme` -- this would import the entire default theme layout and conflict with our custom Layout.vue. Instead, import ONLY the `VPNavBarSearch` component.
- **Hand-writing search UI:** VPNavBarSearch handles keyboard shortcuts, fuzzy search, result highlighting, modal UI, and navigation. Building a custom search would be enormous effort for no benefit.
- **Per-page frontmatter for SEO:** Use `transformPageData` for automatic SEO. Don't require manual `head:` entries on every page.
- **Inventing code examples for docs:** Prior decision from Phase 72: all code examples must be sourced from e2e test files or verified against runtime source code. Never invent syntax.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Full-text search | Custom search index + UI | VitePress MiniSearch (`themeConfig.search.provider: 'local'`) + `VPNavBarSearch` from `vitepress/theme` | MiniSearch handles indexing, fuzzy matching, result ranking. VPNavBarSearch handles Cmd+K, modal, highlighting. |
| Copy-to-clipboard | Custom clipboard Vue component | VitePress built-in `<button class="copy">` + `useCopyCode()` composable (app-level) | VitePress injects the button HTML in markdown rendering and handles clipboard JS globally. Just add CSS. |
| SEO meta generation | Manual frontmatter on each page | `transformPageData` hook in config.mts | One function covers all pages automatically, generating title, description, OG tags, canonical URL. |
| Last-updated timestamps | Manual dates or custom git scripts | `lastUpdated: true` in VitePress config | VitePress runs `git log` internally, exposes timestamp via `useData().page.value.lastUpdated`. |
| Search keyboard shortcuts | Custom keydown handlers | VPNavBarSearch built-in Cmd/Ctrl+K handler | The search component already handles focus, keyboard shortcuts, and accessibility. |

**Key insight:** VitePress 1.6.4 has matured significantly. Most "production polish" features are built-in or trivially configurable. The main work is writing CSS for the copy button and building small display components (edit link, last-updated, version badge, 404 page).

## Common Pitfalls

### Pitfall 1: Search Not Working with Custom Theme
**What goes wrong:** Enabling `themeConfig.search.provider: 'local'` but search doesn't appear because the custom theme doesn't render the search component.
**Why it happens:** The search config tells VitePress to build the search index, but the UI component must be explicitly placed in the custom theme layout.
**How to avoid:** Import `VPNavBarSearch` from `vitepress/theme` and place it in `NavBar.vue`. This is now officially supported (confirmed in v1.6.4 type declarations).
**Warning signs:** Search config present but no search button visible on the site.

### Pitfall 2: Copy Button Invisible (No CSS)
**What goes wrong:** The `<button class="copy">` is present in the DOM but invisible because the custom theme doesn't include VitePress default theme CSS.
**Why it happens:** VitePress injects the button HTML regardless of theme, but the styling lives in the default theme's CSS (`vp-doc.css`). Custom themes don't get this CSS.
**How to avoid:** Add copy button CSS to `code.css` with proper positioning, opacity transitions, and the SVG icon CSS variable (`--vp-icon-copy`).
**Warning signs:** Inspecting a code block in DevTools shows `<button class="copy">` with no visible styling.

### Pitfall 3: lastUpdated Shows Wrong Dates in CI
**What goes wrong:** All pages show the same last-updated date (the build date).
**Why it happens:** CI environments typically do shallow clones (`depth: 1`), so `git log` returns the same date for every file.
**How to avoid:** In CI/CD, use `git fetch --unshallow` before building, or set `fetch-depth: 0` in GitHub Actions checkout.
**Warning signs:** All pages showing identical last-updated timestamps.

### Pitfall 4: Inventing Code Examples Without Source Verification
**What goes wrong:** Documentation shows Mesh syntax that doesn't actually compile.
**Why it happens:** Some Phase 73 topics (WebSocket, distributed, pooling, TLS) lack e2e test files. It's tempting to guess the syntax.
**How to avoid:** Derive code examples from: (1) existing e2e test files, (2) the codegen function name mapping in `mir/lower.rs` (which shows the exact Mesh function names), (3) runtime source code doc comments. Cross-reference the Mesh API mapping: `HTTP.*` -> `mesh_http_*`, `Ws.*` -> `mesh_ws_*`, `Sqlite.*` -> `mesh_sqlite_*`, `Pg.*` -> `mesh_pg_*`, `Pool.*` -> `mesh_pool_*`, `Node.*` -> `mesh_node_*`, `Global.*` -> `mesh_global_*`.
**Warning signs:** Code examples that use function names not found in the codegen mapping.

### Pitfall 5: Dead Links from New Sidebar Entries
**What goes wrong:** VitePress build fails with dead link errors when sidebar links point to pages that don't exist yet.
**Why it happens:** VitePress validates all internal links at build time.
**How to avoid:** Create all doc pages (even as stubs) BEFORE updating the sidebar config, or update sidebar and content in the same step. This was encountered in Phase 72 (prior decision 72-02).
**Warning signs:** `vitepress build` fails with "dead link" errors.

### Pitfall 6: VPNavBarSearch CSS Conflicts
**What goes wrong:** The search component renders but looks broken because our custom theme CSS interferes with VitePress default theme component styles.
**Why it happens:** VPNavBarSearch relies on VitePress default theme CSS variables (e.g., `--vp-c-brand`, `--vp-c-bg-soft`).
**How to avoid:** Inspect the rendered search component and add CSS variable overrides or wrapper scoping to harmonize with our OKLCH palette. May need to define a small set of `--vp-*` CSS variables mapped to our theme tokens.
**Warning signs:** Search modal has white text on white background, or buttons are invisible.

## Code Examples

### Mesh Web API Surface (for DOCS-05)

Based on e2e tests and codegen mapping (`mir/lower.rs` lines 9445-9466):

```mesh
# HTTP Server with routing, middleware, and path params
fn logger(request :: Request, next) -> Response do
  next(request)
end

fn handler(request) do
  HTTP.response(200, "Hello from Mesh!")
end

fn user_handler(request) do
  let param = Request.param(request, "id")
  case param do
    Some(id) -> HTTP.response(200, id)
    None -> HTTP.response(400, "missing id")
  end
end

fn main() do
  let r = HTTP.router()
  let r = HTTP.use(r, logger)              # middleware
  let r = HTTP.on_get(r, "/users/:id", user_handler)  # method-specific route
  let r = HTTP.route(r, "/", handler)      # catch-all route
  HTTP.serve(r, 8080)                      # plain HTTP
  # HTTP.serve_tls(r, 8443, "cert.pem", "key.pem")  # TLS
end
```

Source: `tests/e2e/stdlib_http_middleware.mpl`, `tests/e2e/stdlib_http_path_params.mpl`, codegen `mir/lower.rs:9448` for `http_serve_tls`

### Mesh WebSocket API Surface (for DOCS-05)

Based on codegen mapping (`mir/lower.rs` lines 9521-9529):

```mesh
# WebSocket server with rooms
fn on_connect(conn) do
  Ws.join(conn, "lobby")
  Ws.send(conn, "Welcome!")
end

fn on_message(conn, msg) do
  Ws.broadcast("lobby", msg)           # broadcast to all in room
  # Ws.broadcast_except("lobby", msg, conn)  # broadcast to all except sender
end

fn on_close(conn) do
  # cleanup happens automatically (room membership cleaned on disconnect)
end

fn main() do
  Ws.serve(on_connect, on_message, on_close, 9001)
  # Ws.serve_tls(on_connect, on_message, on_close, 9001, "cert.pem", "key.pem")
end
```

Source: codegen `mir/lower.rs:9521-9529`, runtime `crates/mesh-rt/src/ws/rooms.rs` (room API), `crates/mesh-rt/src/ws/server.rs` (serve API)

### Mesh Database API Surface (for DOCS-06)

Based on e2e tests:

```mesh
# SQLite
fn main() do
  let db = Sqlite.open(":memory:")?
  let _ = Sqlite.execute(db, "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)", [])?
  let _ = Sqlite.execute(db, "INSERT INTO users (name) VALUES (?)", ["Alice"])?
  let rows = Sqlite.query(db, "SELECT name FROM users", [])?
  Sqlite.close(db)
end

# PostgreSQL
fn main() do
  let conn = Pg.connect("postgres://user:pass@localhost:5432/mydb")?
  let _ = Pg.execute(conn, "INSERT INTO users (name, age) VALUES ($1, $2)", ["Alice", "30"])?
  let rows = Pg.query(conn, "SELECT name, age FROM users", [])?
  Pg.close(conn)
end

# Connection Pooling
fn main() do
  let pool = Pool.open("postgres://...", 2, 10, 5000)?  # min, max, timeout_ms
  let rows = Pool.query(pool, "SELECT * FROM users", [])?
  let _ = Pool.execute(pool, "INSERT INTO users (name) VALUES ($1)", ["Bob"])?
  Pool.close(pool)
end

# Struct mapping with deriving(Row)
struct User do
  name :: String
  age :: Int
  score :: Float
  active :: Bool
end deriving(Row)

fn main() do
  let row = Map.new()
  let row = Map.put(row, "name", "Alice")
  let row = Map.put(row, "age", "30")
  let result = User.from_row(row)
end
```

Source: `tests/e2e/stdlib_sqlite.mpl`, `tests/e2e/stdlib_pg.mpl`, `tests/e2e/deriving_row_basic.mpl`, codegen `mir/lower.rs:9487-9495`

### Mesh Distributed API Surface (for DOCS-07)

Based on codegen mapping (`mir/lower.rs:9531-9544`):

```mesh
# Start a named node with cookie authentication
fn main() do
  Node.start("app@localhost:4000", "secret_cookie")
  Node.connect("worker@localhost:4001")

  # Global process registry
  Global.register("db_service", self())
  let pid = Global.whereis("db_service")

  # Node information
  let name = Node.self()
  let nodes = Node.list()

  # Monitor remote nodes
  Node.monitor("worker@localhost:4001")
end
```

Source: codegen `mir/lower.rs:9531-9544`, runtime `crates/mesh-rt/src/dist/node.rs`, `crates/mesh-rt/src/dist/global.rs`

### Mesh Tooling (for DOCS-08)

Based on crate source code:

- **Formatter** (`mesh-fmt`): Wadler-Lindig document IR, CST-based, preserves comments. CLI: `meshc fmt [file]`
- **REPL** (`mesh-repl`): LLVM JIT, full compiler pipeline. CLI: `meshc repl`
- **Package Manager** (`mesh-pkg`): Manifest, lockfile, dependency resolution, project scaffolding. CLI: `meshc new [name]`
- **LSP** (`mesh-lsp`): Diagnostics, hover, go-to-definition via tower-lsp. Protocol: stdio JSON-RPC
- **VS Code Extension** (`editors/vscode-mesh`): TextMate grammar, language configuration

Source: Crate source headers (`crates/mesh-fmt/src/lib.rs`, `crates/mesh-repl/src/lib.rs`, `crates/mesh-pkg/src/lib.rs`, `crates/mesh-lsp/src/lib.rs`)

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Import VPNavBarSearch from `vitepress/dist/...` (internal path) | `import { VPNavBarSearch } from 'vitepress/theme'` | VitePress 1.6.x (late 2025) | Clean, type-safe import for search in custom themes |
| Algolia DocSearch for search | Built-in MiniSearch local search | VitePress 1.x | No external service needed, zero-config |
| Manual frontmatter for SEO | `transformPageData` hook | VitePress 1.x | Automatic, DRY SEO across all pages |
| Custom copy button implementations | VitePress built-in `<button class="copy">` + `useCopyCode()` | VitePress 1.x | Clipboard handled automatically, just need CSS |

**Deprecated/outdated:**
- `vitepress-plugin-search` npm package: Unnecessary now that MiniSearch is built-in
- Deep imports from `vitepress/dist/client/theme-default/...`: Use official `vitepress/theme` exports instead

## Open Questions

1. **VPNavBarSearch CSS compatibility with custom OKLCH theme**
   - What we know: VPNavBarSearch relies on VitePress default theme CSS variables (`--vp-c-*`). Our theme uses OKLCH custom properties (`--background`, `--foreground`, etc.).
   - What's unclear: Exactly which `--vp-*` variables need mapping and whether the search modal will look correct without them.
   - Recommendation: Add a minimal set of `--vp-*` CSS variable mappings in `main.css` that point to our theme tokens. Test by building and inspecting. This is low-risk and fixable in implementation.

2. **Exact Mesh user-facing syntax for some features**
   - What we know: The codegen mapping in `mir/lower.rs` shows function name translations (e.g., `ws_serve` -> `mesh_ws_serve`). Module dispatch resolves `Ws.serve()` to `ws_serve`.
   - What's unclear: Some features (like transactions, TLS-specific options) may not have a dedicated Mesh API surface -- they may be implicit in the connection URL or handled by the runtime.
   - Recommendation: For features without e2e tests, document the API surface as derived from the codegen mapping and runtime source, and note which examples are derived (not tested). The planner should mark these for build verification.

3. **GitHub repository URL for edit links**
   - What we know: The NavBar currently links to `https://github.com/user/mesh` (placeholder).
   - What's unclear: The actual repository URL.
   - Recommendation: Use the same placeholder URL (`https://github.com/user/mesh`) in the editLink pattern. It can be updated when the real URL is known.

4. **Version badge update strategy**
   - What we know: Current version is `0.1.0` in `crates/meshc/Cargo.toml`.
   - What's unclear: Whether the version should be read dynamically from Cargo.toml at build time or hardcoded in config.
   - Recommendation: Hardcode in `themeConfig.meshVersion` for simplicity. A build script could read Cargo.toml later if automated updates are needed.

## Sources

### Primary (HIGH confidence)
- VitePress installed `node_modules/vitepress/theme.d.ts` -- confirmed `VPNavBarSearch` export
- VitePress installed `node_modules/vitepress/dist/client/app/composables/copyCode.js` -- copy button click handler
- VitePress installed `node_modules/vitepress/dist/node/chunk-D3CUZ4fa.js:35100` -- copy button HTML injection in markdown rendering
- VitePress installed `node_modules/vitepress/dist/client/theme-default/styles/components/vp-doc.css` -- default copy button CSS
- `crates/mesh-codegen/src/mir/lower.rs` lines 9445-9544 -- Mesh function name to runtime function mapping
- `tests/e2e/stdlib_http_*.mpl` -- HTTP server e2e tests
- `tests/e2e/stdlib_sqlite.mpl`, `tests/e2e/stdlib_pg.mpl` -- Database e2e tests
- `tests/e2e/deriving_row_*.mpl` -- Struct mapping e2e tests
- `crates/mesh-rt/src/ws/rooms.rs` -- WebSocket room API
- `crates/mesh-rt/src/dist/node.rs` -- Distributed node API
- `crates/mesh-rt/src/dist/global.rs` -- Global registry API
- `crates/mesh-rt/src/db/pool.rs` -- Connection pool API

### Secondary (MEDIUM confidence)
- [VitePress Search Docs](https://vitepress.dev/reference/default-theme-search) -- MiniSearch configuration
- [VitePress Last Updated](https://vitepress.dev/reference/default-theme-last-updated) -- git timestamp feature
- [VitePress Edit Link](https://vitepress.dev/reference/default-theme-edit-link) -- edit link configuration
- [VitePress transformPageData](https://vitepress.dev/reference/site-config#transformpagedata) -- dynamic SEO hook
- [VitePress transformHead](https://vitepress.dev/reference/site-config#transformhead) -- build-time head injection
- [VitePress Runtime API useData](https://vitepress.dev/reference/runtime-api#usedata) -- page.isNotFound, page.lastUpdated
- [GitHub Issue #4476](https://github.com/vuejs/vitepress/issues/4476) -- VPNavBarSearch official export resolution
- [GitHub Issue #2490](https://github.com/vuejs/vitepress/issues/2490) -- Third-party theme local search support

### Tertiary (LOW confidence)
- None -- all findings verified against installed source code or official docs.

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- all libraries already installed, VPNavBarSearch export confirmed in installed type declarations
- Architecture patterns: HIGH -- config patterns verified against VitePress docs and source code, copy button mechanism confirmed in installed code
- Documentation content: HIGH for topics with e2e tests (HTTP, SQLite, Pg, Row deriving), MEDIUM for topics derived from codegen mapping only (WebSocket, distributed, pooling, TLS)
- Pitfalls: HIGH -- based on concrete analysis of custom theme + VitePress interaction and Phase 72 experience
- Site features: HIGH -- all features verified as built-in or trivially configurable in VitePress 1.6.4

**Research date:** 2026-02-13
**Valid until:** 2026-03-15 (VitePress is stable, Mesh codebase is controlled)
