# Domain Pitfalls: Mesh Documentation Website

**Domain:** Programming language documentation website (VitePress + custom theme + Tailwind v4 + shadcn-vue)
**Researched:** 2026-02-13

## Critical Pitfalls

Mistakes that cause rewrites or major issues.

### Pitfall 1: Extending the Default Theme Instead of Replacing It

**What goes wrong:** Starting by importing VitePress's default theme and overriding styles to achieve a monochrome design. Ends up fighting CSS specificity, using `!important` everywhere, and breaking when VitePress updates.
**Why it happens:** Default theme seems "easier" -- get something working, then customize. But the default theme has hundreds of scoped styles with high specificity.
**Consequences:** Unmaintainable CSS. Visual inconsistencies between custom and default-styled elements. VitePress updates break overrides. Eventually forced to rewrite as a custom theme anyway, wasting the initial work.
**Prevention:** Start with a fully custom theme from day one. Create `Layout.vue` + `index.ts` without importing `vitepress/theme`. The effort difference is small -- the default theme's layout is just Vue components; writing your own is straightforward with shadcn-vue primitives.
**Detection:** If you see `!important` in your CSS, or if updating VitePress changes your site's appearance unexpectedly.

### Pitfall 2: Tailwind v4 CSS Variable Conflicts with shadcn-vue

**What goes wrong:** shadcn-vue uses CSS variables like `--background`, `--foreground`, `--primary`, `--muted`, etc. Tailwind v4's `@theme` directive expects variables following a different naming convention. If these aren't properly bridged, Tailwind utilities like `bg-background` don't resolve, or shadcn-vue components render with wrong or missing colors.
**Why it happens:** shadcn-vue was originally designed for Tailwind v3 with `tailwind.config.js`. The v4 CSS-first config requires remapping CSS variables.
**Consequences:** Components render with wrong colors or appear unstyled. Dark mode partially works (Tailwind dark utilities work but shadcn-vue component internals don't flip). Hours of debugging CSS variable chains.
**Prevention:** Follow the [shadcn-vue Tailwind v4 migration guide](https://www.shadcn-vue.com/docs/tailwind-v4) precisely. Since this is a new project, define the monochrome palette once in `:root` / `.dark` with variable names that work for both shadcn-vue and Tailwind. The shadcn-vue CLI (`npx shadcn-vue@latest init`) generates compatible CSS when targeting Tailwind v4.
**Detection:** Components look unstyled or use wrong colors after installation. Dark mode doesn't fully invert.

### Pitfall 3: Building Custom Markdown Pipeline Instead of Using VitePress

**What goes wrong:** Installing `unplugin-vue-markdown`, `markdown-it`, `@shikijs/markdown-it`, `vue-router`, and `vite-ssg` to build a custom docs framework because "the requirements are too custom for VitePress."
**Why it happens:** Developers familiar with Vue but not VitePress assume they need more control. They don't realize VitePress custom themes provide full design control while VitePress handles all the content infrastructure.
**Consequences:** 2-3 weeks of plumbing work. Must implement: file-based routing, sidebar generation, frontmatter parsing, static HTML generation, page metadata, SPA navigation, prefetching, and search. VitePress provides all of this for free.
**Prevention:** Use VitePress. The custom theme API gives you a blank canvas (just a `Layout.vue` component) while VitePress handles routing, markdown compilation, Shiki integration, SSG, search, and more.
**Detection:** If `package.json` has `vue-router`, `unplugin-vue-markdown`, `vite-ssg`, and `markdown-it` as direct dependencies alongside documentation markdown files.

## Moderate Pitfalls

### Pitfall 4: Dark Mode FOUC (Flash of Unstyled Content)

**What goes wrong:** VitePress generates static HTML at build time in light mode. When JavaScript loads, `useDark()` reads localStorage and applies the `dark` class. Users who prefer dark mode see a flash of light mode before dark kicks in.
**Why it happens:** Static HTML cannot include JavaScript-dependent class toggling. The page renders with whatever classes were in the HTML at build time, then JavaScript corrects it.
**Prevention:** Add an inline `<script>` in the `<head>` that runs before the page renders. VitePress supports this via the `transformPageData` or `transformHead` hooks. The script reads localStorage synchronously and applies the `dark` class to `<html>` before any content paints. This is a standard 5-line script:

```typescript
// .vitepress/config.ts
export default defineConfig({
  head: [
    ['script', {}, `
      (function() {
        const saved = localStorage.getItem('vueuse-color-scheme')
        const prefersDark = window.matchMedia('(prefers-color-scheme: dark)').matches
        if (saved === 'dark' || (!saved && prefersDark)) {
          document.documentElement.classList.add('dark')
        }
      })()
    `]
  ]
})
```

### Pitfall 5: Tailwind Typography `prose` Unreadable in Dark Mode

**What goes wrong:** Markdown content styled with `prose` class looks fine in light mode but becomes unreadable in dark mode (dark text on dark background).
**Why it happens:** `@tailwindcss/typography` defaults to dark text colors. Without the invert modifier, content vanishes on dark backgrounds.
**Prevention:** Always pair `prose` with `dark:prose-invert` on the content wrapper:
```html
<div class="prose dark:prose-invert max-w-none">
  <Content />
</div>
```

### Pitfall 6: Shiki Custom Grammar Errors Fail Silently

**What goes wrong:** A malformed regex in `mesh.tmLanguage.json` causes Shiki to fall back to plain text rendering for Mesh code blocks. No error is thrown during build.
**Why it happens:** TextMate grammars use Oniguruma regex syntax. Some patterns valid in VS Code may behave differently in Shiki's JavaScript regex engine (which Shiki v3 supports as an alternative to the default Oniguruma WASM engine).
**Prevention:** Test the grammar explicitly during development. Create a test documentation page with comprehensive Mesh code examples exercising all grammar rules: keywords, types, operators, strings with interpolation, comments, function definitions, numeric literals, pipe operator, pattern matching, do/end blocks. Visually verify each scope highlights correctly.
**Detection:** Code blocks tagged with ` ```mesh ` appear as plain monochrome text without any syntax coloring.

### Pitfall 7: Mobile Sidebar Stays Open After Navigation

**What goes wrong:** User opens the mobile sidebar (Sheet component), taps a docs link, the page navigates but the sidebar stays open covering the new content.
**Why it happens:** shadcn-vue's Sheet component manages its own open/close state. Route changes from VitePress don't automatically close it.
**Prevention:** Watch for route changes in the sidebar composable and close the Sheet:
```typescript
import { useRoute } from 'vitepress'
const route = useRoute()
watch(() => route.path, () => { isOpen.value = false })
```

### Pitfall 8: Tailwind Purges Classes Used in Markdown

**What goes wrong:** Custom CSS classes or Tailwind utilities used inside markdown files (via custom containers, components, or raw HTML in markdown) are purged in production build.
**Why it happens:** Tailwind v4's content detection finds classes in `.vue` and `.ts` files automatically, but may miss classes used in markdown content that gets processed through VitePress's pipeline.
**Prevention:** Tailwind v4's Vite plugin should detect classes in `.md` files within the VitePress content directory. If specific utilities are still missing in production, add them to a safelist or use them in a `.vue` component that references them. Test the production build early: `npx vitepress build` and verify the output.
**Detection:** Styles work in `npx vitepress dev` but break in `npx vitepress build` output.

## Minor Pitfalls

### Pitfall 9: Landing Page Appears in Docs Sidebar

**What goes wrong:** The landing page (`/`) shows the docs sidebar, or the landing page appears as an item in the sidebar navigation.
**Prevention:** Use `layout: home` frontmatter on the landing page. In `Layout.vue`, check `frontmatter.layout === 'home'` and render `LandingPage` without the sidebar. Do not include the root `index.md` in the sidebar config.

### Pitfall 10: Oversized Code Blocks on Mobile

**What goes wrong:** Code blocks use a fixed font size that's too large on mobile, causing excessive horizontal scrolling and poor readability.
**Prevention:** Use responsive font sizing: `text-sm md:text-base` on code block wrappers. Ensure all code blocks have `overflow-x-auto` for horizontal scrolling of long lines.

### Pitfall 11: Missing Social Preview Meta Tags

**What goes wrong:** Sharing a docs page on Twitter/LinkedIn/Slack shows a generic preview without title, description, or image.
**Prevention:** Configure VitePress `head` option for default Open Graph tags. Use frontmatter `title` and `description` per page. Add `og-image.png` to `/public/`. VitePress automatically generates `<title>` and `<meta name="description">` from frontmatter. Add OG tags in the config:
```typescript
head: [
  ['meta', { property: 'og:image', content: '/og-image.png' }],
  ['meta', { property: 'og:type', content: 'website' }],
]
```

### Pitfall 12: Broken Internal Links After Restructuring

**What goes wrong:** Moving or renaming markdown files breaks internal links. Dead links in the sidebar or in page content.
**Prevention:** VitePress detects broken links during `vitepress build` by default. Keep `ignoreDeadLinks: false` (the default) so broken links fail the build. Run the build in CI.

## Phase-Specific Warnings

| Phase Topic | Likely Pitfall | Mitigation |
|-------------|---------------|------------|
| VitePress project scaffold | Pitfall 1: Extending default theme | Start with blank `Layout.vue`. Do not import `vitepress/theme`. |
| Tailwind + shadcn-vue setup | Pitfall 2: CSS variable conflicts | Follow shadcn-vue Tailwind v4 guide. Use `npx shadcn-vue@latest init` targeting v4. |
| Dark mode implementation | Pitfall 4: FOUC on page load | Add inline `<script>` in `head` config to apply dark class before paint. |
| Mesh syntax highlighting | Pitfall 6: Grammar falls back silently | Test with comprehensive code samples page. Verify each scope. |
| Landing page | Pitfall 9: Sidebar leaks onto landing | Use frontmatter `layout: home` + conditional rendering in Layout.vue. |
| Mobile responsive | Pitfall 7: Sidebar stays open | Watch `route.path`, auto-close Sheet on navigation. |
| Production build | Pitfall 8: Tailwind purge issues | Test production build early and often. Run `npx vitepress build`. |
| Content restructuring | Pitfall 12: Broken internal links | Keep `ignoreDeadLinks: false`. Build in CI catches broken links. |
| Content writing | Pitfall 5: Dark mode prose unreadable | Always use `prose dark:prose-invert` on content wrapper. |

## Sources

- [VitePress custom theme guide](https://vitepress.dev/guide/custom-theme) -- custom vs extending default theme
- [shadcn-vue Tailwind v4 migration](https://www.shadcn-vue.com/docs/tailwind-v4) -- CSS variable remapping
- [VueUse useDark](https://vueuse.org/core/usedark/) -- hydration behavior, localStorage key
- [Tailwind CSS v4 content detection](https://tailwindcss.com/blog/tailwindcss-v4) -- automatic content discovery, Vite plugin
- [Shiki custom language loading](https://shiki.style/guide/load-lang) -- grammar loading behavior and fallback
- [@tailwindcss/typography](https://github.com/tailwindlabs/tailwindcss-typography) -- prose + dark:prose-invert

---
*Pitfalls research for: Mesh Language Website & Documentation*
*Researched: 2026-02-13*
