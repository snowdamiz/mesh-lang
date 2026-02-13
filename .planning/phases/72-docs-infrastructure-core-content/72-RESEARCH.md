# Phase 72: Docs Infrastructure + Core Content - Research

**Researched:** 2026-02-13
**Domain:** VitePress custom theme docs layout (sidebar, TOC, prev/next) + Mesh documentation content
**Confidence:** HIGH

## Summary

Phase 72 adds documentation infrastructure and core content to the Mesh website. The infrastructure work involves building a three-column docs layout: collapsible sidebar navigation on the left, markdown content in the center, and a "On this page" table of contents on the right, plus previous/next page links at the bottom. The content work involves writing complete guides covering Getting Started, Language Basics, Type System, Concurrency, and a Syntax Cheatsheet.

The project uses a fully custom VitePress theme (blank Layout.vue, not extending default theme). This means we cannot use VitePress's built-in sidebar, outline, or prev/next components -- they live in `vitepress/dist/client/theme-default/` and are tightly coupled to the default theme's internal state. However, VitePress exposes all the raw data we need: `useData()` provides `theme` (which contains `themeConfig.sidebar`), `page` (which contains `page.relativePath` for current page detection and `page.headers` for outline data), and `frontmatter` (for per-page overrides). VitePress also exports `useRoute()`, `useRouter()`, `onContentUpdated()`, and `getScrollOffset()` from `'vitepress'` -- all essential for building custom navigation components.

The key architectural insight is that we can reuse the VitePress default theme's *logic* (the `getSidebar`, `getFlatSideBarLinks`, `getSidebarGroups`, `hasActiveLink` utility functions from the `support/sidebar.js` module, and the `getHeaders`, `resolveHeaders`, `useActiveAnchor` functions from `composables/outline.js`) as reference implementations for our own composables. We build our own Vue components styled with Tailwind/shadcn-vue, but the sidebar resolution algorithm and outline extraction logic follow the same proven patterns. For mobile, the existing shadcn-vue Sheet component (already installed from Phase 70) provides the sidebar overlay. For the sidebar collapse/expand behavior, shadcn-vue's Collapsible component handles accessible toggle state.

**Primary recommendation:** Define sidebar config in `themeConfig.sidebar`, build custom sidebar/TOC/prev-next Vue components using VitePress runtime APIs (`useData`, `onContentUpdated`, `getScrollOffset`, `useRoute`) with Tailwind styling and shadcn-vue primitives (Sheet, Collapsible, ScrollArea), activate `@tailwindcss/typography` for prose styling of markdown content, and write all five documentation sections as markdown files with `mesh` code fences.

## Standard Stack

### Core (already installed from Phases 70-71)

| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| VitePress | 1.6.4 | SSG framework, provides `useData`, `useRoute`, `onContentUpdated`, `getScrollOffset`, `Content` | All runtime APIs needed for custom theme navigation are exported from `'vitepress'` |
| Vue 3 | 3.5.28 | Component framework for sidebar, TOC, prev/next components | Already installed |
| Tailwind CSS v4 | 4.1.18 | Utility classes for docs layout, responsive breakpoints | Already installed |
| @tailwindcss/typography | 0.5.19 | `prose` / `prose-invert` classes for markdown content styling | Already installed as dependency but NOT yet activated in CSS |
| reka-ui | 2.8.0 | Primitive components underlying shadcn-vue (Dialog for Sheet, Collapsible) | Already installed |
| @vueuse/core | 14.2.1 | `useMediaQuery` for responsive sidebar behavior, `useScroll` for scroll tracking | Already installed |
| lucide-vue-next | 0.564.0 | Icons for sidebar toggle, collapse arrows, prev/next chevrons | Already installed |

### Supporting (need to add via shadcn-vue CLI)

| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| shadcn-vue Collapsible | (from reka-ui) | Collapsible sidebar section groups with accessible toggle | Each sidebar group with `collapsed` property |
| shadcn-vue ScrollArea | (from reka-ui) | Custom scrollbar for sidebar content area | Sidebar content overflow on long navigation trees |

### New Dependencies

**None.** All npm packages are already installed. Only need to scaffold two new shadcn-vue component sets (Collapsible, ScrollArea) via `npx shadcn-vue@latest add collapsible scroll-area`.

### Alternatives Considered

| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| Custom sidebar components | Extend VitePress default theme | Project decision: custom theme (no default theme extension). Cannot use `useSidebar` from `vitepress/theme` -- it creates separate instances not connected to default theme Layout. |
| DOM-based heading extraction (`getHeaders`) | `useData().page.value.headers` from `markdown.headers` config | The `page.headers` approach requires setting `markdown.headers: true` and has wrong data for dynamically rendered headings. DOM-based extraction (like default theme does) is more reliable. Use DOM approach. |
| Manual sidebar config | `vitepress-sidebar` auto-generation plugin | Auto-generation loses control over ordering, grouping, and naming. Manual config gives precise control for 5 documentation sections. Use manual config. |
| Custom scroll-aware active heading tracking | Simple hash-based active state | Scroll-aware tracking (IntersectionObserver or scroll position) provides better UX -- the TOC highlights the section currently visible, not just the last clicked link. Worth the implementation effort. |

## Architecture Patterns

### Recommended Project Structure (additions to Phase 71)

```
website/docs/.vitepress/
  config.mts                              # MODIFY: add themeConfig.sidebar, outline config
  theme/
    Layout.vue                            # MODIFY: add docs layout with sidebar + TOC
    index.ts                              # MODIFY: add @tailwindcss/typography plugin import
    components/
      NavBar.vue                          # Existing (may add mobile sidebar toggle button)
      ThemeToggle.vue                     # Existing
      landing/                            # Existing (unchanged)
      docs/
        DocsSidebar.vue                   # NEW: Left sidebar with section groups
        DocsSidebarItem.vue               # NEW: Recursive sidebar item (link + children)
        DocsSidebarGroup.vue              # NEW: Collapsible section group
        DocsTableOfContents.vue           # NEW: Right-side "On this page" outline
        DocsOutlineItem.vue               # NEW: Recursive outline heading item
        DocsPrevNext.vue                  # NEW: Previous/next page links footer
        DocsLayout.vue                    # NEW: Three-column docs page layout
        MobileSidebar.vue                 # NEW: Sheet-based mobile sidebar
      ui/
        button/                           # Existing
        dropdown-menu/                    # Existing
        sheet/                            # Existing
        collapsible/                      # NEW (via shadcn-vue CLI)
        scroll-area/                      # NEW (via shadcn-vue CLI)
    composables/
      useShiki.ts                         # Existing
      useSidebar.ts                       # NEW: Sidebar resolution, active link detection
      useOutline.ts                       # NEW: Heading extraction, active anchor tracking
      usePrevNext.ts                      # NEW: Previous/next page computation
    styles/
      main.css                            # MODIFY: add @plugin "@tailwindcss/typography"
      code.css                            # Existing
      prose.css                           # NEW: prose overrides for Mesh docs styling

website/docs/
  index.md                                # Existing (landing page)
  docs/
    getting-started/
      index.md                            # DOCS-01: Installation, hello world, compile & run
    language-basics/
      index.md                            # DOCS-02: Variables, types, functions, pattern matching, etc.
    type-system/
      index.md                            # DOCS-03: Type inference, generics, structs, etc.
    concurrency/
      index.md                            # DOCS-04: Actors, spawning, message passing, etc.
    cheatsheet/
      index.md                            # DOCS-09: Syntax quick reference
```

### Pattern 1: Sidebar Configuration in themeConfig

**What:** Define the documentation sidebar structure in VitePress config, accessible via `useData().theme.value.sidebar`.
**When:** Config setup, defines the sidebar for all docs pages.

```typescript
// .vitepress/config.mts
export default defineConfig({
  // ... existing config ...
  themeConfig: {
    sidebar: {
      '/docs/': [
        {
          text: 'Getting Started',
          items: [
            { text: 'Installation', link: '/docs/getting-started/' },
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

Source: [VitePress Sidebar Config](https://vitepress.dev/reference/default-theme-sidebar), [VitePress Default Theme Types](https://github.com/vuejs/vitepress/blob/main/types/default-theme.d.ts)

**Key details:**
- The `themeConfig` object is passed to custom themes and accessible via `useData().theme.value`
- The `sidebar` key uses the same format as default theme: `Record<string, SidebarItem[]>` for multi-sidebar
- The `/docs/` key means this sidebar only shows on pages under `/docs/`
- `collapsed: false` means the group is collapsible but starts expanded
- `collapsed: undefined` (omitted) means the group is NOT collapsible
- The `outline` config sets heading levels [2,3] (h2 + h3) for the "On this page" panel

### Pattern 2: Custom Sidebar Composable

**What:** Composable that resolves the current sidebar from config and tracks active links.
**When:** Used by DocsSidebar.vue and MobileSidebar.vue.

```typescript
// composables/useSidebar.ts
import { computed, ref, watch } from 'vue'
import { useData, useRoute, useRouter } from 'vitepress'
import { useMediaQuery } from '@vueuse/core'

interface SidebarItem {
  text?: string
  link?: string
  items?: SidebarItem[]
  collapsed?: boolean
  base?: string
  docFooterText?: string
}

export function useSidebar() {
  const { theme, page, frontmatter } = useData()
  const route = useRoute()
  const router = useRouter()
  const is960 = useMediaQuery('(min-width: 960px)')
  const isOpen = ref(false)

  const sidebar = computed(() => {
    const sidebarConfig = theme.value.sidebar
    if (!sidebarConfig) return []
    if (Array.isArray(sidebarConfig)) return sidebarConfig

    // Multi-sidebar: find matching path
    const relativePath = page.value.relativePath
    const path = ensureStartingSlash(relativePath)
    const dir = Object.keys(sidebarConfig)
      .sort((a, b) => b.split('/').length - a.split('/').length)
      .find((dir) => path.startsWith(ensureStartingSlash(dir)))
    return dir ? sidebarConfig[dir] : []
  })

  const hasSidebar = computed(() => {
    return frontmatter.value.sidebar !== false
      && sidebar.value.length > 0
      && frontmatter.value.layout !== 'home'
  })

  // Auto-close mobile sidebar on route change
  watch(() => route.path, () => {
    isOpen.value = false
  })

  function open() { isOpen.value = true }
  function close() { isOpen.value = false }
  function toggle() { isOpen.value ? close() : open() }

  return { sidebar, hasSidebar, isOpen, is960, open, close, toggle }
}

function ensureStartingSlash(path: string) {
  return path.startsWith('/') ? path : `/${path}`
}

// Check if a link matches the current page
export function isActive(currentPath: string, matchPath?: string): boolean {
  if (!matchPath) return false
  const normalizedCurrent = ensureStartingSlash(
    currentPath.replace(/(index)?\.(md|html)$/, '')
  )
  const normalizedMatch = ensureStartingSlash(
    matchPath.replace(/(index)?\.(md|html)$/, '').replace(/\/$/, '')
  )
  return normalizedCurrent === normalizedMatch
    || normalizedCurrent.startsWith(normalizedMatch + '/')
}
```

Source: VitePress default theme `composables/sidebar.js` and `support/sidebar.js` (reference implementation read from installed node_modules)

### Pattern 3: Custom Outline/TOC Composable

**What:** Composable that extracts headings from the rendered page DOM and tracks the active heading on scroll.
**When:** Used by DocsTableOfContents.vue.

```typescript
// composables/useOutline.ts
import { onContentUpdated, getScrollOffset } from 'vitepress'
import { useData } from 'vitepress'
import { onMounted, onUnmounted, ref, shallowRef } from 'vue'

export interface OutlineItem {
  element: HTMLElement
  title: string
  link: string
  level: number
  children: OutlineItem[]
}

export function useOutline() {
  const { theme, frontmatter } = useData()
  const headers = shallowRef<OutlineItem[]>([])

  onContentUpdated(() => {
    const outlineConfig = frontmatter.value.outline ?? theme.value.outline
    headers.value = getHeaders(outlineConfig)
  })

  return { headers }
}

export function getHeaders(range: any): OutlineItem[] {
  // Query headings from the rendered content area
  const allHeaders = [...document.querySelectorAll('.docs-content :where(h1,h2,h3,h4,h5,h6)')]
    .filter((el) => el.id && el.hasChildNodes())
    .map((el) => {
      const level = Number(el.tagName[1])
      return {
        element: el as HTMLElement,
        title: serializeHeader(el as HTMLElement),
        link: '#' + el.id,
        level,
        children: [] as OutlineItem[],
      }
    })

  // Parse range config
  const levelsRange = (typeof range === 'object' && !Array.isArray(range)
    ? range.level : range) || 2
  const [high, low] = typeof levelsRange === 'number'
    ? [levelsRange, levelsRange]
    : levelsRange === 'deep' ? [2, 6] : levelsRange

  return buildTree(allHeaders, high, low)
}

function serializeHeader(h: HTMLElement): string {
  let ret = ''
  for (const node of h.childNodes) {
    if (node.nodeType === 1) {
      if (/header-anchor|ignore-header/.test((node as Element).className)) continue
      ret += node.textContent
    } else if (node.nodeType === 3) {
      ret += node.textContent
    }
  }
  return ret.trim()
}

function buildTree(data: OutlineItem[], min: number, max: number): OutlineItem[] {
  const result: OutlineItem[] = []
  const stack: OutlineItem[] = []
  for (const item of data) {
    const node = { ...item, children: [] }
    if (node.level > max || node.level < min) continue
    let parent = stack[stack.length - 1]
    while (parent && parent.level >= node.level) {
      stack.pop()
      parent = stack[stack.length - 1]
    }
    if (parent) parent.children.push(node)
    else result.push(node)
    stack.push(node)
  }
  return result
}
```

Source: VitePress default theme `composables/outline.js` (reference implementation read from installed node_modules)

**Key detail:** The `onContentUpdated` hook from VitePress fires after every page navigation and content re-render -- this is how the TOC stays in sync with the current page. Heading extraction uses DOM queries (not `page.headers`) because DOM-based extraction is more reliable for dynamically rendered content.

### Pattern 4: Custom Prev/Next Composable

**What:** Composable that computes previous and next page links from the sidebar config.
**When:** Used by DocsPrevNext.vue.

```typescript
// composables/usePrevNext.ts
import { computed } from 'vue'
import { useData } from 'vitepress'
import { isActive } from './useSidebar'

interface FlatLink {
  text: string
  link: string
  docFooterText?: string
}

export function usePrevNext() {
  const { page, theme, frontmatter } = useData()

  return computed(() => {
    const sidebarConfig = theme.value.sidebar
    if (!sidebarConfig) return { prev: undefined, next: undefined }

    // Resolve current sidebar
    const relativePath = page.value.relativePath
    const sidebar = resolveSidebar(sidebarConfig, relativePath)

    // Flatten all links from sidebar
    const links = flattenSidebarLinks(sidebar)
    const candidates = uniqBy(links, (l) => l.link.replace(/[?#].*$/, ''))

    // Find current page index
    const index = candidates.findIndex((link) =>
      isActive(page.value.relativePath, link.link)
    )

    return {
      prev: frontmatter.value.prev === false ? undefined : {
        text: candidates[index - 1]?.docFooterText ?? candidates[index - 1]?.text,
        link: candidates[index - 1]?.link,
      },
      next: frontmatter.value.next === false ? undefined : {
        text: candidates[index + 1]?.docFooterText ?? candidates[index + 1]?.text,
        link: candidates[index + 1]?.link,
      },
    }
  })
}

function flattenSidebarLinks(items: any[]): FlatLink[] {
  const links: FlatLink[] = []
  function recurse(items: any[]) {
    for (const item of items) {
      if (item.text && item.link) {
        links.push({ text: item.text, link: item.link, docFooterText: item.docFooterText })
      }
      if (item.items) recurse(item.items)
    }
  }
  recurse(items)
  return links
}

function resolveSidebar(sidebar: any, relativePath: string): any[] {
  if (Array.isArray(sidebar)) return sidebar
  const path = relativePath.startsWith('/') ? relativePath : `/${relativePath}`
  const dir = Object.keys(sidebar)
    .sort((a, b) => b.split('/').length - a.split('/').length)
    .find((d) => path.startsWith(d.startsWith('/') ? d : `/${d}`))
  return dir ? sidebar[dir] : []
}

function uniqBy<T>(arr: T[], fn: (item: T) => string): T[] {
  const seen = new Set<string>()
  return arr.filter((item) => {
    const k = fn(item)
    return seen.has(k) ? false : (seen.add(k), true)
  })
}
```

Source: VitePress default theme `composables/prev-next.js` (reference implementation read from installed node_modules)

### Pattern 5: Three-Column Docs Layout

**What:** The main docs page layout with responsive sidebar, content area with prose styling, and aside panel with TOC.
**When:** All documentation pages (any page not using `layout: home`).

```vue
<!-- DocsLayout.vue -->
<script setup lang="ts">
import { useSidebar } from '@/composables/useSidebar'
import { useMediaQuery } from '@vueuse/core'
import DocsSidebar from './DocsSidebar.vue'
import DocsTableOfContents from './DocsTableOfContents.vue'
import DocsPrevNext from './DocsPrevNext.vue'
import MobileSidebar from './MobileSidebar.vue'

const { sidebar, hasSidebar } = useSidebar()
const isDesktop = useMediaQuery('(min-width: 960px)')
const isWide = useMediaQuery('(min-width: 1280px)')
</script>

<template>
  <div class="relative mx-auto flex max-w-[90rem]">
    <!-- Desktop sidebar -->
    <aside
      v-if="hasSidebar && isDesktop"
      class="sticky top-14 h-[calc(100vh-3.5rem)] w-64 shrink-0 border-r border-border"
    >
      <DocsSidebar :items="sidebar" />
    </aside>

    <!-- Mobile sidebar -->
    <MobileSidebar v-if="hasSidebar && !isDesktop" :items="sidebar" />

    <!-- Main content -->
    <main class="min-w-0 flex-1 px-6 py-8 lg:px-8">
      <div class="docs-content prose dark:prose-invert max-w-none">
        <Content />
      </div>
      <DocsPrevNext class="mt-12" />
    </main>

    <!-- Right aside: Table of Contents -->
    <aside
      v-if="isWide"
      class="sticky top-14 hidden h-[calc(100vh-3.5rem)] w-56 shrink-0 xl:block"
    >
      <DocsTableOfContents />
    </aside>
  </div>
</template>
```

**Key layout details:**
- Desktop sidebar: visible at >= 960px (`lg`), fixed 256px width, sticky below navbar
- Mobile sidebar: Sheet overlay below 960px, auto-closes on route change
- Content area: flex-1 with `prose dark:prose-invert` for typography
- TOC aside: visible at >= 1280px (`xl`), fixed 224px width, sticky
- `max-w-[90rem]` constrains total width; `docs-content` class is the selector for heading extraction

### Pattern 6: Typography Plugin Activation

**What:** Enable @tailwindcss/typography for prose styling of markdown-rendered content.
**When:** Required before any docs pages render properly.

```css
/* main.css -- add this line */
@import "tailwindcss" source("../..");
@import "tw-animate-css";
@import "./code.css";
@import "./prose.css";
@plugin "@tailwindcss/typography";

@custom-variant dark (&:where(.dark, .dark *));
/* ... rest of existing CSS ... */
```

```css
/* prose.css -- override prose defaults to match site palette */
.prose {
  --tw-prose-body: var(--foreground);
  --tw-prose-headings: var(--foreground);
  --tw-prose-links: var(--foreground);
  --tw-prose-bold: var(--foreground);
  --tw-prose-code: var(--foreground);
  --tw-prose-pre-bg: var(--muted);
  --tw-prose-hr: var(--border);
  --tw-prose-th-borders: var(--border);
  --tw-prose-td-borders: var(--border);
  --tw-prose-counters: var(--muted-foreground);
  --tw-prose-bullets: var(--muted-foreground);
  --tw-prose-quotes: var(--muted-foreground);
  --tw-prose-quote-borders: var(--border);
  --tw-prose-captions: var(--muted-foreground);
}

.prose :where(a):not(:where(.not-prose, .not-prose *)) {
  text-decoration: underline;
  text-underline-offset: 2px;
  font-weight: 500;
}

/* Prevent prose from interfering with code blocks */
.prose :where(pre):not(:where(.not-prose, .not-prose *)) {
  background-color: transparent;
  padding: 0;
  margin: 0;
  border-radius: 0;
}
```

Source: [Tailwind Typography Plugin](https://github.com/tailwindlabs/tailwindcss-typography), [Tailwind v4 @plugin directive](https://tailwindcss.com/blog/tailwindcss-typography-v0-5)

**Critical:** The `@plugin "@tailwindcss/typography"` directive must be in the main CSS file (Tailwind v4 style). The `prose-invert` class handles dark mode automatically. Override prose CSS variables to match the site's existing OKLCH palette. The `pre` override prevents prose from double-styling code blocks (which already have styling from `code.css`).

### Pattern 7: Mobile Sidebar with Sheet

**What:** Uses the existing shadcn-vue Sheet component for mobile sidebar overlay.
**When:** Below 960px breakpoint.

```vue
<!-- MobileSidebar.vue -->
<script setup lang="ts">
import { Sheet, SheetContent, SheetTrigger, SheetTitle } from '@/components/ui/sheet'
import { useSidebar } from '@/composables/useSidebar'
import DocsSidebar from './DocsSidebar.vue'

const props = defineProps<{ items: any[] }>()
const { isOpen, open, close, toggle } = useSidebar()
</script>

<template>
  <Sheet v-model:open="isOpen">
    <SheetContent side="left" class="w-72 p-0">
      <SheetTitle class="sr-only">Navigation</SheetTitle>
      <DocsSidebar :items="items" />
    </SheetContent>
  </Sheet>
</template>
```

Source: [shadcn-vue Sheet](https://www.shadcn-vue.com/docs/components/sheet)

**Key details:**
- Sheet `side="left"` slides in from the left
- The sidebar toggle button goes in NavBar.vue (visible only on mobile)
- `v-model:open="isOpen"` syncs with the sidebar composable
- Route change watcher in `useSidebar` sets `isOpen = false`, auto-closing the sheet
- `SheetTitle` with `sr-only` satisfies accessibility requirements (reka-ui Dialog requires a title)

### Anti-Patterns to Avoid

- **Importing from `vitepress/theme`:** Do NOT import `useSidebar`, `useOutline`, etc. from `vitepress/theme`. These are default theme internals that create separate instances not connected to a custom theme. Build custom composables using the public API (`useData`, `useRoute` from `'vitepress'`).
- **Using `page.headers` instead of DOM extraction:** The `markdown.headers` config approach provides stale data for dynamically rendered headings. The default theme uses DOM queries (`document.querySelectorAll`) for a reason. Follow the same approach.
- **Hardcoding navigation order:** Prev/next computation must derive from the sidebar config, not hardcoded page lists. When pages are reordered in sidebar config, prev/next should update automatically.
- **Wrapping `<Content />` in a `<div>` for prose without `max-w-none`:** The `prose` class sets a default `max-width: 65ch` which constrains content width. Use `max-w-none` to let the parent container control width instead.
- **Forgetting to handle the `sidebar: false` frontmatter override:** Individual pages may set `sidebar: false` in frontmatter to hide the sidebar. The composable must check this.
- **Building TOC that does not update on route change:** The `onContentUpdated` hook from VitePress is essential -- it fires after each page navigation. Without it, the TOC shows headings from the previous page.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Sidebar path resolution (multi-sidebar) | Custom path matcher | Port algorithm from VitePress `support/sidebar.js` `getSidebar()` | Handles edge cases: trailing slashes, nested paths, longest-prefix matching, `base` property |
| Active link detection | Simple `===` comparison | Port algorithm from VitePress `shared.js` `isActive()` | Handles `.md` vs `.html` extensions, hash fragments, clean URLs, index pages |
| Heading extraction from rendered content | Regex on markdown source | DOM query on rendered HTML (as VitePress default theme does) | Handles dynamic content, Vue components in markdown, proper text serialization |
| Active anchor tracking on scroll | Naive `scrollIntoView` or `IntersectionObserver` | Port pattern from VitePress `composables/outline.js` `useActiveAnchor()` | Handles edge cases: page bottom (activate last), page top (deactivate all), `getScrollOffset()` for sticky header compensation |
| Collapsible sidebar sections | Custom show/hide with v-if | shadcn-vue Collapsible component (wraps reka-ui Collapsible) | Accessible: manages aria-expanded, keyboard navigation, focus management |
| Mobile sidebar overlay | Custom modal/drawer | shadcn-vue Sheet component (already installed) | Accessible: focus trap, escape to close, backdrop click, portal rendering |
| Scrollable sidebar area | `overflow-y: auto` | shadcn-vue ScrollArea component | Cross-browser consistent scrollbar styling, custom scrollbar that matches theme |
| Markdown content typography | Custom CSS for every element | `@tailwindcss/typography` with `prose` class | Handles headings, lists, tables, blockquotes, code, links, images -- hundreds of CSS rules |
| Route change detection | Polling or MutationObserver | VitePress `useRoute()` + `watch(route.path, ...)` or `useRouter().onAfterRouteChange` | Official API, reactive, handles all navigation types |

**Key insight:** The VitePress default theme's composable implementations in `node_modules/vitepress/dist/client/theme-default/` are reference implementations for exactly the logic we need. We port the algorithms into our own composables while building our own Vue components with Tailwind/shadcn-vue styling.

## Common Pitfalls

### Pitfall 1: Mobile Sidebar Does Not Close on Link Tap

**What goes wrong:** User taps a link in the mobile sidebar Sheet, the page navigates, but the Sheet stays open, blocking the content.
**Why it happens:** The Sheet component does not auto-close on VitePress page navigation because VitePress client-side navigation does not trigger a full page reload.
**How to avoid:** Watch `route.path` in the sidebar composable and set `isOpen = false` on change. VitePress's `useRoute()` returns a reactive route object.
**Warning signs:** Sheet overlay stays visible after tapping a sidebar link on mobile.

### Pitfall 2: Table of Contents Shows Headings from Previous Page

**What goes wrong:** After navigating to a new docs page, the "On this page" panel still shows headings from the previous page.
**Why it happens:** Heading extraction runs once on mount but not on route change. VitePress uses client-side navigation, so the component does not remount.
**How to avoid:** Use `onContentUpdated()` from `'vitepress'` to re-extract headings after every page change. This hook fires after the `<Content />` component re-renders.
**Warning signs:** TOC links point to nonexistent headings, or the heading text does not match the current page.

### Pitfall 3: Prose Styling Conflicts with Code Block Styling

**What goes wrong:** Code blocks inside markdown get double-styled -- `prose` applies its own `pre`/`code` styles on top of the existing `code.css` styles.
**Why it happens:** `@tailwindcss/typography` styles `pre` and `code` elements within `.prose` containers.
**How to avoid:** Override prose `pre` styles to be transparent/zero-padding (let `code.css` handle it), or wrap code block containers with the `not-prose` class. The simplest approach is the CSS override shown in Pattern 6.
**Warning signs:** Code blocks have wrong background color, double padding, or misaligned text.

### Pitfall 4: Sidebar Active State Does Not Highlight Current Page

**What goes wrong:** The sidebar renders all links in the same style, with no visual indication of the current page.
**Why it happens:** The `isActive()` check fails because of path normalization differences (e.g., `docs/getting-started/index.md` vs `/docs/getting-started/`).
**How to avoid:** Normalize both the current `page.relativePath` and the sidebar link before comparison. Strip trailing slashes, strip `index.md`/`.md`/`.html` extensions. The VitePress `isActive` function handles this.
**Warning signs:** No sidebar item appears highlighted, or the wrong item is highlighted.

### Pitfall 5: Sticky Sidebar/TOC Overlaps Navbar

**What goes wrong:** When scrolling, the sidebar or TOC panel scrolls under the sticky navbar instead of stopping below it.
**Why it happens:** `position: sticky` with `top: 0` ignores the navbar height. The navbar is 56px (`h-14`).
**How to avoid:** Use `top: 3.5rem` (56px) and `h-[calc(100vh-3.5rem)]` for both sidebar and TOC aside panels.
**Warning signs:** Content appears to clip under the navbar when scrolling.

### Pitfall 6: Typography Plugin Not Loading

**What goes wrong:** Adding `prose` class to content produces no styling effect.
**Why it happens:** The `@tailwindcss/typography` package is installed but the `@plugin` directive is missing from the CSS. In Tailwind v4, plugins are activated via CSS `@plugin` directives, not JavaScript config.
**How to avoid:** Add `@plugin "@tailwindcss/typography";` to `main.css`.
**Warning signs:** Content renders as unstyled HTML (no heading sizes, no list bullets, no link colors).

## Code Examples

### VitePress Public API Exports (verified from source)

```typescript
// Available from 'vitepress' in custom themes:
export { useData } from './app/data'         // site, theme, page, frontmatter, isDark, etc.
export { useRoute, useRouter } from './app/router'  // route.path, onAfterRouteChange
export { getScrollOffset, inBrowser, onContentUpdated, withBase, Content } from './app/utils'
```

Source: VitePress `dist/client/index.js` (verified in installed `node_modules/vitepress/dist/client/index.js`)

### useData Return Type (verified from types)

```typescript
interface VitePressData<T = any> {
  site: Ref<SiteData<T>>
  theme: Ref<T>                  // themeConfig -- contains sidebar, outline, etc.
  page: Ref<PageData>            // relativePath, headers, frontmatter, etc.
  frontmatter: Ref<Record<string, any>>
  title: Ref<string>
  description: Ref<string>
  lang: Ref<string>
  isDark: Ref<boolean>
  dir: Ref<string>
  localeIndex: Ref<string>
  hash: Ref<string>
}

interface PageData {
  title: string
  description: string
  relativePath: string           // e.g., "docs/getting-started/index.md"
  filePath: string
  headers: Header[]              // Available but DOM extraction is preferred
  frontmatter: Record<string, any>
  isNotFound?: boolean
  lastUpdated?: number
}
```

Source: [VitePress Runtime API](https://vitepress.dev/reference/runtime-api), VitePress `types/shared.d.ts`

### SidebarItem Type (from VitePress default theme types)

```typescript
type Sidebar = SidebarItem[] | SidebarMulti

interface SidebarMulti {
  [path: string]: SidebarItem[] | { items: SidebarItem[]; base: string }
}

type SidebarItem = {
  text?: string
  link?: string
  items?: SidebarItem[]
  collapsed?: boolean     // undefined = not collapsible, true = collapsed, false = expanded
  base?: string           // base path prepended to children links
  docFooterText?: string  // custom text for prev/next footer
}
```

Source: VitePress `types/default-theme.d.ts` (verified in installed node_modules)

### Mesh Code Examples for Documentation Content

**Hello World (Getting Started):**
```mesh
fn main() do
  println("Hello, World!")
end
```

**Variables and Types (Language Basics):**
```mesh
fn main() do
  let name = "Mesh"
  let age = 30
  let pi = 3.14
  let active = true
  println("Hello, ${name}!")
end
```

**Pattern Matching (Language Basics):**
```mesh
fn describe(x :: Int) -> String do
  case x do
    0 -> "zero"
    1 -> "one"
    _ -> "other"
  end
end
```

**Pipe Operator (Language Basics):**
```mesh
fn double(x :: Int) -> Int do
  x * 2
end

fn add_one(x :: Int) -> Int do
  x + 1
end

fn main() do
  let result = 5 |> double |> add_one
  println("${result}")
end
```

**Structs and Deriving (Type System):**
```mesh
struct Point do
  x :: Int
  y :: Int
end deriving(Eq, Ord, Display, Debug, Hash)

fn main() do
  let p = Point { x: 1, y: 2 }
  println("${p}")
end
```

**Sum Types (Type System):**
```mesh
type Color do
  Red
  Green
  Blue
end
```

**Actors and Messaging (Concurrency):**
```mesh
actor worker() do
  receive do
    msg -> println("worker got: ${msg}")
  end
end

fn main() do
  let w = spawn(worker)
  send(w, "hello")
end
```

**Supervisor (Concurrency):**
```mesh
supervisor WorkerSup do
  strategy: one_for_one
  max_restarts: 3
  max_seconds: 5

  child w1 do
    start: fn -> spawn(worker) end
    restart: permanent
    shutdown: 5000
  end
end
```

**GenServer/Service (Concurrency):**
```mesh
service Store do
  fn init(start_val :: Int) -> Int do
    start_val
  end

  call Get() :: Int do |state|
    (state, state)
  end

  cast Clear() do |_state|
    0
  end
end
```

Source: Mesh e2e test files in `/Users/sn0w/Documents/dev/snow/tests/e2e/` (verified from project codebase)

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Tailwind v4 `plugins: [typography]` in config | `@plugin "@tailwindcss/typography"` in CSS | Tailwind v4 (2025) | Plugin activation moved from JS config to CSS directives |
| VitePress default theme sidebar API (`useSidebar` from `vitepress/theme`) | Custom composable using `useData().theme.value.sidebar` | N/A (custom theme requirement) | Custom themes must build their own sidebar components; importing from default theme creates disconnected instances |
| `markdown.headers: true` for page headers | DOM-based heading extraction via `document.querySelectorAll` | N/A (reliability) | DOM extraction handles dynamic content; `page.headers` can be stale |
| VitePress `useRoute` with vue-router API | VitePress-specific `useRoute` and `useRouter` | VitePress 1.x | VitePress route object differs from vue-router; use `watch(() => route.path, ...)` for navigation tracking |

**Deprecated/outdated:**
- Importing `useSidebar`/`useOutline` from `vitepress/theme` in custom themes: Creates separate instances, not connected to layout
- `markdown.headers: true` config: Can produce wrong data for dynamic headings; DOM extraction preferred

## Open Questions

1. **Documentation content accuracy for Mesh language features**
   - What we know: The e2e test files demonstrate actual working Mesh syntax. The landing page FeatureShowcase.vue contains some "aspirational" syntax (e.g., `match` instead of `case`, `:increment` atom syntax) that may not reflect the current compiler.
   - What's unclear: Whether all features described in requirements (generics, trait deriving, error handling with `try`) are fully implemented and what the exact syntax is.
   - Recommendation: Base all documentation code examples on the actual e2e test files in `tests/e2e/`. When unsure about syntax, reference the test files. Mark any aspirational features clearly.

2. **Sidebar toggle button placement in NavBar**
   - What we know: NavBar.vue has space for navigation links on desktop and theme toggle on the right. Mobile needs a hamburger/menu button to open the sidebar.
   - What's unclear: Whether the NavBar should be modified to include the sidebar toggle, or if a separate "local nav" bar should appear below the NavBar on mobile (as VitePress default theme does).
   - Recommendation: Add a hamburger icon button to the NavBar, visible only below 960px. Simpler and avoids introducing a second nav bar.

3. **ScrollArea vs native overflow for sidebar**
   - What we know: shadcn-vue ScrollArea provides custom-styled scrollbars. The sidebar needs to scroll when content exceeds viewport height.
   - What's unclear: Whether native `overflow-y: auto` with Tailwind's scrollbar utilities provides sufficient customization, or whether the full ScrollArea component is worth the added complexity.
   - Recommendation: Use shadcn-vue ScrollArea for consistent cross-browser scrollbar styling that matches the theme's monochrome aesthetic.

## Sources

### Primary (HIGH confidence)
- VitePress `dist/client/index.js` -- verified public API exports (`useData`, `useRoute`, `useRouter`, `onContentUpdated`, `getScrollOffset`, `Content`)
- VitePress `dist/client/theme-default/composables/sidebar.js` -- reference implementation for sidebar resolution
- VitePress `dist/client/theme-default/composables/outline.js` -- reference implementation for heading extraction and active anchor
- VitePress `dist/client/theme-default/composables/prev-next.js` -- reference implementation for prev/next computation
- VitePress `dist/client/theme-default/support/sidebar.js` -- `getSidebar`, `getFlatSideBarLinks`, `hasActiveLink` algorithms
- VitePress `types/default-theme.d.ts` -- `SidebarItem`, `Sidebar`, `SidebarMulti`, `Outline`, `DocFooter` type definitions
- [VitePress Runtime API](https://vitepress.dev/reference/runtime-api) -- `useData`, `useRoute`, `useRouter` documentation
- [VitePress Sidebar Config](https://vitepress.dev/reference/default-theme-sidebar) -- sidebar configuration format
- [VitePress Frontmatter Config](https://vitepress.dev/reference/frontmatter-config) -- `sidebar`, `outline`, `prev`, `next` overrides
- [VitePress Custom Theme](https://vitepress.dev/guide/custom-theme) -- custom theme entry, Layout.vue, `Content` component
- [shadcn-vue Collapsible](https://www.shadcn-vue.com/docs/components/collapsible) -- accessible collapse/expand
- [shadcn-vue Sheet](https://www.shadcn-vue.com/docs/components/sidebar) -- sidebar overlay for mobile
- [shadcn-vue ScrollArea](https://www.shadcn-vue.com/docs/components/scroll-area) -- custom scrollbar
- [Tailwind Typography Plugin](https://github.com/tailwindlabs/tailwindcss-typography) -- `@plugin` directive, prose classes, dark mode

### Secondary (MEDIUM confidence)
- [VitePress GitHub Discussion #4038](https://github.com/vuejs/vitepress/discussions/4038) -- custom theme TOC approaches
- [VitePress GitHub Discussion #2854](https://github.com/vuejs/vitepress/discussions/2854) -- custom theme data availability gaps
- [Blog: Building a VitePress Blog Theme](https://soubiran.dev/series/create-a-blog-with-vitepress-and-vue-js-from-scratch/from-default-to-custom-building-a-vitepress-blog-theme) -- custom theme architecture patterns
- Mesh e2e test files (`tests/e2e/*.mpl`) -- actual language syntax verification

### Tertiary (LOW confidence)
- Active anchor scroll tracking edge cases -- ported from default theme, but behavior with our custom layout geometry (different sidebar/content widths) needs testing

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- all packages already installed, versions verified from package.json/node_modules
- Architecture (sidebar composable + config): HIGH -- based on reading VitePress default theme source code in node_modules, porting proven algorithms
- Architecture (three-column layout): HIGH -- standard responsive layout pattern, verified breakpoint approach from default theme
- Typography plugin activation: HIGH -- verified Tailwind v4 `@plugin` directive from official docs
- Pitfalls (mobile auto-close, TOC update, prose conflicts): HIGH -- directly observed in VitePress default theme source code
- Documentation content accuracy: MEDIUM -- based on e2e test files, but some features may have syntax differences from what tests show

**Research date:** 2026-02-13
**Valid until:** 2026-03-15 (stable ecosystem, 30-day validity)
