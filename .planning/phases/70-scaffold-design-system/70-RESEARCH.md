# Phase 70: Scaffold + Design System - Research

**Researched:** 2026-02-13
**Domain:** VitePress custom theme + Tailwind CSS v4 + shadcn-vue + dark mode
**Confidence:** HIGH

## Summary

Phase 70 scaffolds a VitePress site in `/website` with a fully custom theme (blank Layout.vue), integrates Tailwind CSS v4 with a monochrome OKLCH palette, initializes shadcn-vue components, implements dark/light mode toggle with persistence and FOUC prevention, and creates a NavBar component. This is the foundation every subsequent phase builds on.

The stack is well-established: VitePress 1.6.x provides the static site framework with built-in markdown pipeline and Shiki syntax highlighting. Tailwind CSS v4 uses `@tailwindcss/vite` for zero-config integration with CSS-first `@theme` directives. shadcn-vue 2.x provides accessible component primitives (Button, Sheet, DropdownMenu) that render with Tailwind classes. The monochrome design is achieved by using shadcn-vue's `neutral` base color, which already uses zero-chroma OKLCH values out of the box.

A critical finding from this research: VitePress has its own built-in dark mode system. The `appearance` config option injects an inline `<script>` that reads `localStorage` (key: `vitepress-theme-appearance`) and applies the `.dark` class before paint -- solving FOUC automatically. The dark mode toggle should use `useData().isDark` from VitePress (not VueUse's `useDark()`) combined with `useToggle()` from `@vueuse/core`. This approach integrates cleanly with VitePress's SSG pipeline.

**Primary recommendation:** Use VitePress's built-in `appearance` system for dark mode (FOUC prevention is automatic), shadcn-vue's `neutral` base color for the monochrome OKLCH palette (it already has zero chroma), and the `@custom-variant dark (&:where(.dark, .dark *))` directive for Tailwind v4 class-based dark mode. Do not extend the default VitePress theme.

## Standard Stack

### Core

| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| VitePress | ^1.6.4 | Static site generator, markdown pipeline, SSG, routing | Purpose-built for docs. Custom theme = blank canvas + free content pipeline. Used by Vue, Vite, Vitest docs. |
| Vue 3 | ^3.5.28 | UI framework (bundled with VitePress) | Composition API + `<script setup>` for all custom components. |
| Tailwind CSS | ^4.1.18 | Utility-first CSS framework | CSS-first config via `@theme`. Monochrome palette trivially expressed. `dark:` variant for dark mode. |
| @tailwindcss/vite | ^4.1.18 | Vite plugin for Tailwind v4 | First-party. Zero-config content detection. Add to VitePress `vite.plugins`. |

### Supporting

| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| @vueuse/core | ^14.2.1 | Vue composition utilities | `useToggle()` for dark mode toggle. `useMediaQuery()` for responsive behavior. |
| lucide-vue-next | ^0.563.0 | Icon library | Sun/Moon icons for theme toggle. Menu icon for mobile nav. Tree-shakeable. |
| class-variance-authority | latest | Component variant management | Required by shadcn-vue components. |
| clsx | latest | Conditional class names | Required by shadcn-vue `cn()` utility. |
| tailwind-merge | latest | Merge Tailwind classes | Required by shadcn-vue `cn()` utility. |
| tw-animate-css | latest | Animation utilities | Required by shadcn-vue components (replaces deprecated `tailwindcss-animate`). |
| @tailwindcss/typography | ^0.5.19 | Prose styling for markdown | `prose` + `dark:prose-invert` for rendered markdown content. Needed in later phases, can install now. |

### shadcn-vue Components (copy-paste, not npm deps)

| Component | Purpose | When to Use |
|-----------|---------|-------------|
| Button | Theme toggle trigger, CTA buttons | NavBar theme toggle, landing page |
| DropdownMenu | Theme mode selector (light/dark/system) | ThemeToggle component |
| Sheet | Mobile navigation overlay | MobileMenu (later phases, but install now for NavBar) |

### Alternatives Considered

| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| VitePress built-in `isDark` | VueUse `useDark()` | `useDark()` works but requires manual FOUC script. VitePress's `appearance` handles FOUC automatically with injected inline script. Use VitePress's built-in system. |
| shadcn-vue neutral base | Custom OKLCH palette from scratch | Neutral base already uses zero-chroma OKLCH. Saves designing 20+ color variables. Tweak individual values if needed. |
| `@custom-variant dark (&:where(.dark, .dark *))` | `@custom-variant dark (&:is(.dark *))` | shadcn-vue uses `:is(.dark *)` but official Tailwind docs recommend `:where(.dark, .dark *)` which also matches the `.dark` element itself and has zero specificity. Use the Tailwind official version. |

### Installation

```bash
# From repo root
mkdir -p website && cd website

# Initialize package.json
npm init -y

# Core: VitePress + Vue
npm install vitepress vue

# Tailwind CSS v4
npm install tailwindcss @tailwindcss/vite @tailwindcss/typography

# shadcn-vue dependencies (manual install -- not the CLI init)
npm install class-variance-authority clsx tailwind-merge tw-animate-css

# Vue utilities
npm install @vueuse/core

# Icons
npm install lucide-vue-next

# Dev dependencies
npm install -D typescript @types/node

# Then add shadcn-vue components via CLI:
npx shadcn-vue@latest init
npx shadcn-vue@latest add button
npx shadcn-vue@latest add dropdown-menu
npx shadcn-vue@latest add sheet
```

## Architecture Patterns

### Recommended Project Structure

```
website/
  .vitepress/
    config.ts                      # VitePress config: appearance, vite plugins, head
    theme/
      index.ts                     # Theme entry: exports Layout + enhanceApp
      Layout.vue                   # Root layout: NavBar + <Content />
      components/
        NavBar.vue                 # Top nav: logo, links, theme toggle
        ThemeToggle.vue            # Dark/light mode dropdown
      composables/
        (empty for now)
      lib/
        utils.ts                   # cn() helper for shadcn-vue
      styles/
        main.css                   # Tailwind imports + @theme inline + @custom-variant
      components/
        ui/                        # shadcn-vue generated components (Button, DropdownMenu, Sheet)
  docs/
    index.md                       # Placeholder page (confirms site works)
  public/
    (empty for now)
  package.json
  tsconfig.json
  components.json                  # shadcn-vue config
```

### Pattern 1: VitePress Custom Theme Entry

**What:** Minimal theme entry that exports Layout.vue without importing default theme.
**When:** Always. This is the foundation.

```typescript
// .vitepress/theme/index.ts
import type { Theme } from 'vitepress'
import Layout from './Layout.vue'
import './styles/main.css'

export default {
  Layout,
  enhanceApp({ app }) {
    // Register global components if needed
  }
} satisfies Theme
```

Source: [VitePress Custom Theme Guide](https://vitepress.dev/guide/custom-theme)

### Pattern 2: Minimal Layout with Content

**What:** Layout.vue renders NavBar + VitePress Content component.
**When:** Phase 70 -- minimal shell that proves the site works.

```vue
<!-- .vitepress/theme/Layout.vue -->
<script setup lang="ts">
import { useData } from 'vitepress'
import NavBar from './components/NavBar.vue'

const { frontmatter } = useData()
</script>

<template>
  <div class="min-h-screen bg-background text-foreground">
    <NavBar />
    <main class="mx-auto max-w-4xl px-4 py-8">
      <Content />
    </main>
  </div>
</template>
```

Source: [VitePress Runtime API](https://vitepress.dev/reference/runtime-api)

### Pattern 3: VitePress Built-in Dark Mode with useData().isDark

**What:** Use VitePress's own `isDark` ref for the theme toggle instead of VueUse's `useDark()`.
**When:** Always in VitePress custom themes.
**Why:** VitePress injects an inline script for FOUC prevention automatically when `appearance: true`. The `isDark` ref from `useData()` syncs with this system.

```vue
<!-- ThemeToggle.vue -->
<script setup lang="ts">
import { useData } from 'vitepress'
import { useToggle } from '@vueuse/core'
import { Sun, Moon } from 'lucide-vue-next'
import { Button } from '@/components/ui/button'

const { isDark } = useData()
const toggleDark = useToggle(isDark)
</script>

<template>
  <Button variant="ghost" size="icon" @click="toggleDark()">
    <Sun class="h-5 w-5 rotate-0 scale-100 transition-all dark:-rotate-90 dark:scale-0" />
    <Moon class="absolute h-5 w-5 rotate-90 scale-0 transition-all dark:rotate-0 dark:scale-100" />
    <span class="sr-only">Toggle theme</span>
  </Button>
</template>
```

Source: [shadcn-vue VitePress Dark Mode](https://www.shadcn-vue.com/docs/dark-mode/vitepress)

### Pattern 4: Tailwind v4 CSS with shadcn-vue Variables + Monochrome OKLCH

**What:** Single CSS file that imports Tailwind, defines monochrome OKLCH variables for shadcn-vue, and bridges them to Tailwind via `@theme inline`.
**When:** Phase 70 CSS foundation.

```css
/* .vitepress/theme/styles/main.css */
@import "tailwindcss" source("../..");
@import "tw-animate-css";

@custom-variant dark (&:where(.dark, .dark *));

:root {
  --background: oklch(1 0 0);
  --foreground: oklch(0.145 0 0);
  --card: oklch(1 0 0);
  --card-foreground: oklch(0.145 0 0);
  --popover: oklch(1 0 0);
  --popover-foreground: oklch(0.145 0 0);
  --primary: oklch(0.205 0 0);
  --primary-foreground: oklch(0.985 0 0);
  --secondary: oklch(0.97 0 0);
  --secondary-foreground: oklch(0.205 0 0);
  --muted: oklch(0.97 0 0);
  --muted-foreground: oklch(0.556 0 0);
  --accent: oklch(0.97 0 0);
  --accent-foreground: oklch(0.205 0 0);
  --destructive: oklch(0.577 0.245 27.325);
  --destructive-foreground: oklch(0.577 0.245 27.325);
  --border: oklch(0.922 0 0);
  --input: oklch(0.922 0 0);
  --ring: oklch(0.708 0 0);
  --radius: 0.625rem;
  --sidebar: oklch(0.985 0 0);
  --sidebar-foreground: oklch(0.145 0 0);
  --sidebar-primary: oklch(0.205 0 0);
  --sidebar-primary-foreground: oklch(0.985 0 0);
  --sidebar-accent: oklch(0.97 0 0);
  --sidebar-accent-foreground: oklch(0.205 0 0);
  --sidebar-border: oklch(0.922 0 0);
  --sidebar-ring: oklch(0.708 0 0);
}

.dark {
  --background: oklch(0.145 0 0);
  --foreground: oklch(0.985 0 0);
  --card: oklch(0.145 0 0);
  --card-foreground: oklch(0.985 0 0);
  --popover: oklch(0.145 0 0);
  --popover-foreground: oklch(0.985 0 0);
  --primary: oklch(0.985 0 0);
  --primary-foreground: oklch(0.205 0 0);
  --secondary: oklch(0.269 0 0);
  --secondary-foreground: oklch(0.985 0 0);
  --muted: oklch(0.269 0 0);
  --muted-foreground: oklch(0.708 0 0);
  --accent: oklch(0.269 0 0);
  --accent-foreground: oklch(0.985 0 0);
  --destructive: oklch(0.396 0.141 25.723);
  --destructive-foreground: oklch(0.637 0.237 25.331);
  --border: oklch(0.269 0 0);
  --input: oklch(0.269 0 0);
  --ring: oklch(0.439 0 0);
  --sidebar: oklch(0.205 0 0);
  --sidebar-foreground: oklch(0.985 0 0);
  --sidebar-primary: oklch(0.488 0.243 264.376);
  --sidebar-primary-foreground: oklch(0.985 0 0);
  --sidebar-accent: oklch(0.269 0 0);
  --sidebar-accent-foreground: oklch(0.985 0 0);
  --sidebar-border: oklch(0.269 0 0);
  --sidebar-ring: oklch(0.439 0 0);
}

@theme inline {
  --color-background: var(--background);
  --color-foreground: var(--foreground);
  --color-card: var(--card);
  --color-card-foreground: var(--card-foreground);
  --color-popover: var(--popover);
  --color-popover-foreground: var(--popover-foreground);
  --color-primary: var(--primary);
  --color-primary-foreground: var(--primary-foreground);
  --color-secondary: var(--secondary);
  --color-secondary-foreground: var(--secondary-foreground);
  --color-muted: var(--muted);
  --color-muted-foreground: var(--muted-foreground);
  --color-accent: var(--accent);
  --color-accent-foreground: var(--accent-foreground);
  --color-destructive: var(--destructive);
  --color-destructive-foreground: var(--destructive-foreground);
  --color-border: var(--border);
  --color-input: var(--input);
  --color-ring: var(--ring);
  --radius-sm: calc(var(--radius) - 4px);
  --radius-md: calc(var(--radius) - 2px);
  --radius-lg: var(--radius);
  --radius-xl: calc(var(--radius) + 4px);
  --color-sidebar: var(--sidebar);
  --color-sidebar-foreground: var(--sidebar-foreground);
  --color-sidebar-primary: var(--sidebar-primary);
  --color-sidebar-primary-foreground: var(--sidebar-primary-foreground);
  --color-sidebar-accent: var(--sidebar-accent);
  --color-sidebar-accent-foreground: var(--sidebar-accent-foreground);
  --color-sidebar-border: var(--sidebar-border);
  --color-sidebar-ring: var(--sidebar-ring);
}

@layer base {
  * {
    @apply border-border outline-ring/50;
  }
  body {
    @apply bg-background text-foreground;
  }
}
```

Source: [shadcn-vue Manual Installation](https://www.shadcn-vue.com/docs/installation/manual)

**CRITICAL: The `source("../..")` parameter.** Tailwind v4's Vite plugin has a known issue where it does not automatically scan the `.vitepress` directory for utility classes. The `source()` parameter on the `@import "tailwindcss"` line tells Tailwind to scan from the website root directory (two levels up from `.vitepress/theme/styles/`), ensuring it finds classes in all `.vue` and `.md` files. Without this, Tailwind utilities used in components may be purged in production.

Source: [Tailwind CSS Issue #16050](https://github.com/tailwindlabs/tailwindcss/issues/16050)

### Pattern 5: VitePress Config with Tailwind Vite Plugin

**What:** Configure VitePress to use the Tailwind v4 Vite plugin and set up appearance/dark mode.

```typescript
// .vitepress/config.ts
import { defineConfig } from 'vitepress'
import tailwindcss from '@tailwindcss/vite'
import path from 'node:path'

export default defineConfig({
  title: 'Mesh',
  description: 'The Mesh Programming Language',

  // Built-in dark mode with FOUC prevention
  appearance: true,

  vite: {
    plugins: [
      tailwindcss(),
    ],
    resolve: {
      alias: {
        '@': path.resolve(__dirname, './theme'),
      },
    },
  },
})
```

Source: [VitePress Site Config](https://vitepress.dev/reference/site-config#vite)

### Pattern 6: Path Aliases for shadcn-vue in VitePress

**What:** Align tsconfig paths, Vite resolve aliases, and components.json so `@/` resolves to `.vitepress/theme/`.

```json
// website/tsconfig.json
{
  "compilerOptions": {
    "target": "ES2020",
    "module": "ESNext",
    "moduleResolution": "bundler",
    "strict": true,
    "jsx": "preserve",
    "baseUrl": ".",
    "paths": {
      "@/*": ["./.vitepress/theme/*"]
    }
  },
  "include": [".vitepress/**/*.ts", ".vitepress/**/*.vue", "docs/**/*.md"]
}
```

```json
// website/components.json
{
  "$schema": "https://shadcn-vue.com/schema.json",
  "style": "new-york",
  "typescript": true,
  "tailwind": {
    "config": "",
    "css": ".vitepress/theme/styles/main.css",
    "baseColor": "neutral",
    "cssVariables": true,
    "prefix": ""
  },
  "aliases": {
    "components": "@/components",
    "composables": "@/composables",
    "utils": "@/lib/utils",
    "ui": "@/components/ui",
    "lib": "@/lib"
  },
  "iconLibrary": "lucide"
}
```

**Key alignment:** `@/` maps to `.vitepress/theme/` in three places:
1. `tsconfig.json` `paths` -- for TypeScript type checking
2. VitePress `vite.resolve.alias` -- for Vite module resolution
3. `components.json` `aliases` -- for shadcn-vue CLI component generation

### Anti-Patterns to Avoid

- **Extending the default VitePress theme:** Do NOT import `vitepress/theme`. The default theme has hundreds of scoped CSS rules. Overriding them for a monochrome design leads to `!important` warfare. Start with a blank `Layout.vue`.
- **Installing vue-router:** VitePress has its own router. Two routers conflict. Use `useRoute()` and `useData()` from `vitepress`.
- **Using VueUse `useDark()` directly:** VitePress has its own `isDark` ref in `useData()` that syncs with the built-in FOUC prevention script. Using `useDark()` would create two competing dark mode systems with different localStorage keys.
- **Skipping `source()` on the Tailwind import:** Without `source("../..")`, Tailwind v4 may not scan `.vitepress/` directory files, causing utility classes in `.vue` components to be missing in production builds.
- **Using `@theme` (block) instead of `@theme inline`:** shadcn-vue's CSS variables need `@theme inline` so Tailwind maps them without wrapping in `hsl()` or `oklch()` functions. The variables already contain the complete color values.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Dark mode toggle + persistence | Custom localStorage + class toggle + FOUC script | VitePress `appearance: true` + `useData().isDark` + `useToggle()` | VitePress handles FOUC prevention, localStorage persistence (key: `vitepress-theme-appearance`), system preference detection, and SSG hydration. Custom implementation would duplicate all of this. |
| Accessible dropdown menu | Custom `<div>` with click handlers | shadcn-vue DropdownMenu (built on Reka UI) | WAI-ARIA compliance, keyboard navigation, focus trapping, screen reader announcements. Hundreds of edge cases. |
| CSS utility framework | Custom CSS variables + custom classes | Tailwind CSS v4 | Thousands of utilities, responsive variants, dark mode variants, purging, production optimization. |
| Class name merging | String concatenation | `cn()` helper (clsx + tailwind-merge) | tailwind-merge resolves conflicting Tailwind classes (e.g., `p-4 p-2` becomes `p-2`). String concatenation produces both classes, causing unpredictable results. |
| Mobile slide-out menu | Custom CSS transform + transition | shadcn-vue Sheet (built on Reka UI Dialog) | Focus trapping, scroll lock, backdrop click, escape key, animation, accessibility. |

## Common Pitfalls

### Pitfall 1: Tailwind v4 Content Detection Misses .vitepress Directory

**What goes wrong:** Tailwind utility classes used in `.vue` components inside `.vitepress/theme/` are present in dev mode but disappear in production builds.
**Why it happens:** Tailwind v4's Vite plugin auto-detects content but has a known issue with dotfile directories (`.vitepress`). The plugin skips directories starting with `.` by default.
**How to avoid:** Use the `source()` parameter on the Tailwind import: `@import "tailwindcss" source("../..");` -- this explicitly tells Tailwind to scan from the website root. The path is relative to the CSS file location.
**Warning signs:** Styles work in `npx vitepress dev` but break after `npx vitepress build`.
**Confidence:** HIGH -- verified via [GitHub issue #16050](https://github.com/tailwindlabs/tailwindcss/issues/16050) (closed as completed, Feb 2025).

### Pitfall 2: CSS Variable Naming Mismatch Between shadcn-vue and Tailwind v4

**What goes wrong:** shadcn-vue components render without colors or with wrong colors. Tailwind utilities like `bg-background` don't resolve.
**Why it happens:** shadcn-vue uses CSS variables like `--background`, `--foreground`. Tailwind v4 requires them registered via `@theme inline` with `--color-` prefix (e.g., `--color-background: var(--background)`). If this bridge is missing, Tailwind can't find the colors.
**How to avoid:** Use the exact CSS from Pattern 4 above. The `:root`/`.dark` selectors define the base variables, and `@theme inline` bridges them to Tailwind's namespace.
**Warning signs:** Components appear unstyled or use browser defaults.
**Confidence:** HIGH -- verified via [shadcn-vue manual installation guide](https://www.shadcn-vue.com/docs/installation/manual).

### Pitfall 3: Using `useDark()` Instead of VitePress's Built-in `isDark`

**What goes wrong:** Two competing dark mode systems with different localStorage keys. VitePress's FOUC prevention script reads `vitepress-theme-appearance` but `useDark()` writes to `vueuse-color-scheme`. Toggle button appears to work but FOUC returns because the inline script reads the wrong key.
**Why it happens:** The domain research initially recommended `useDark()` from VueUse. This is correct for plain Vite apps but wrong for VitePress, which has its own integrated dark mode system.
**How to avoid:** Use `const { isDark } = useData()` from `vitepress` for reading dark mode state. Use `useToggle(isDark)` from `@vueuse/core` for toggling. Do not import `useDark` from `@vueuse/core`.
**Warning signs:** FOUC on page load despite having a toggle that works after hydration.
**Confidence:** HIGH -- verified via [VitePress Runtime API](https://vitepress.dev/reference/runtime-api) and [shadcn-vue VitePress dark mode guide](https://www.shadcn-vue.com/docs/dark-mode/vitepress).

### Pitfall 4: Wrong `@custom-variant` Syntax for Dark Mode

**What goes wrong:** `dark:` prefix utilities in Tailwind don't apply even though the `.dark` class is on the `<html>` element.
**Why it happens:** Tailwind v4 defaults to `prefers-color-scheme` media query for dark mode. VitePress uses class-based dark mode (`.dark` on `<html>`). Without the `@custom-variant` directive, Tailwind ignores the class.
**How to avoid:** Add `@custom-variant dark (&:where(.dark, .dark *));` to the main CSS file after the Tailwind import.
**Warning signs:** Dark mode toggle changes the class on `<html>` but no visual change occurs.
**Confidence:** HIGH -- verified via [Tailwind CSS Dark Mode docs](https://tailwindcss.com/docs/dark-mode).

### Pitfall 5: shadcn-vue CLI Init Fails in VitePress Project

**What goes wrong:** `npx shadcn-vue@latest init` may fail because it expects a standard Vite project structure (`src/`, `vite.config.ts`) rather than VitePress's structure.
**Why it happens:** shadcn-vue CLI looks for `vite.config.ts` (which VitePress projects don't have) and a `src/` directory.
**How to avoid:** Configure `components.json` manually (see Pattern 6). Set `tailwind.css` to `.vitepress/theme/styles/main.css`. Create the `lib/utils.ts` file manually. Then use `npx shadcn-vue@latest add <component>` to add individual components -- the `add` command respects `components.json` paths even if `init` had issues.
**Warning signs:** CLI errors during `init` about missing config files.
**Confidence:** MEDIUM -- based on user reports in [shadcn-vue discussion #785](https://github.com/unovue/shadcn-vue/discussions/785). Manual configuration is a reliable fallback.

## Code Examples

### VitePress Config (complete for Phase 70)

```typescript
// .vitepress/config.ts
import { defineConfig } from 'vitepress'
import tailwindcss from '@tailwindcss/vite'
import path from 'node:path'

export default defineConfig({
  title: 'Mesh',
  description: 'The Mesh Programming Language',
  appearance: true, // built-in dark mode + FOUC prevention

  head: [
    // Additional head tags can go here (favicon, OG tags in later phases)
  ],

  vite: {
    plugins: [
      tailwindcss(),
    ],
    resolve: {
      alias: {
        '@': path.resolve(__dirname, './theme'),
      },
    },
  },
})
```

Source: [VitePress Site Config](https://vitepress.dev/reference/site-config)

### Theme Entry (index.ts)

```typescript
// .vitepress/theme/index.ts
import type { Theme } from 'vitepress'
import Layout from './Layout.vue'
import './styles/main.css'

export default {
  Layout,
  enhanceApp({ app }) {
    // Global component registration if needed
  },
} satisfies Theme
```

Source: [VitePress Custom Theme](https://vitepress.dev/guide/custom-theme)

### cn() Utility

```typescript
// .vitepress/theme/lib/utils.ts
import type { ClassValue } from 'clsx'
import { clsx } from 'clsx'
import { twMerge } from 'tailwind-merge'

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs))
}
```

Source: [shadcn-vue Manual Installation](https://www.shadcn-vue.com/docs/installation/manual)

### NavBar Component

```vue
<!-- .vitepress/theme/components/NavBar.vue -->
<script setup lang="ts">
import ThemeToggle from './ThemeToggle.vue'
</script>

<template>
  <header class="sticky top-0 z-50 w-full border-b border-border bg-background/95 backdrop-blur supports-[backdrop-filter]:bg-background/60">
    <div class="mx-auto flex h-14 max-w-7xl items-center justify-between px-4">
      <!-- Logo / Wordmark -->
      <a href="/" class="flex items-center gap-2 font-semibold text-foreground">
        Mesh
      </a>

      <!-- Navigation Links -->
      <nav class="hidden items-center gap-6 text-sm md:flex">
        <a href="/docs/getting-started/" class="text-muted-foreground transition-colors hover:text-foreground">
          Docs
        </a>
        <a href="https://github.com/user/mesh" class="text-muted-foreground transition-colors hover:text-foreground">
          GitHub
        </a>
      </nav>

      <!-- Right side: theme toggle -->
      <div class="flex items-center gap-2">
        <ThemeToggle />
      </div>
    </div>
  </header>
</template>
```

### ThemeToggle Component

```vue
<!-- .vitepress/theme/components/ThemeToggle.vue -->
<script setup lang="ts">
import { useData } from 'vitepress'
import { useToggle } from '@vueuse/core'
import { Sun, Moon } from 'lucide-vue-next'
import { Button } from '@/components/ui/button'

const { isDark } = useData()
const toggleDark = useToggle(isDark)
</script>

<template>
  <Button variant="ghost" size="icon" @click="toggleDark()">
    <Sun class="h-5 w-5 rotate-0 scale-100 transition-all dark:-rotate-90 dark:scale-0" />
    <Moon class="absolute h-5 w-5 rotate-90 scale-0 transition-all dark:rotate-0 dark:scale-100" />
    <span class="sr-only">Toggle theme</span>
  </Button>
</template>
```

Source: [shadcn-vue VitePress Dark Mode](https://www.shadcn-vue.com/docs/dark-mode/vitepress)

### Placeholder docs/index.md

```markdown
---
title: Mesh Programming Language
---

# Mesh

Welcome to the Mesh programming language documentation.
```

### package.json Scripts

```json
{
  "scripts": {
    "dev": "vitepress dev docs",
    "build": "vitepress build docs",
    "preview": "vitepress preview docs"
  }
}
```

**Note on directory structure:** VitePress can be configured to use `docs/` as the source directory. The `dev`/`build`/`preview` commands take the source dir as an argument. Alternatively, with VitePress in the `website/` root, markdown files live directly in `website/` and the commands run without a directory argument. The choice depends on preference. Using `docs/` as a subdirectory is the standard VitePress convention.

### .gitignore Additions

```
# Website
website/node_modules/
website/.vitepress/dist/
website/.vitepress/cache/
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Tailwind v3 `tailwind.config.js` | Tailwind v4 CSS-first `@theme` + `@tailwindcss/vite` | Jan 2025 (v4.0) | No JavaScript config file. CSS-first. 5-100x faster builds. |
| shadcn-vue HSL colors | shadcn-vue OKLCH colors | 2025 (v2.x with TW v4) | Better perceptual uniformity. Zero-chroma = true monochrome. |
| `tailwindcss-animate` | `tw-animate-css` | Mar 2025 | Old package deprecated. New package is pure CSS. |
| PostCSS for Tailwind | `@tailwindcss/vite` plugin | Jan 2025 (v4.0) | No PostCSS config needed. Better HMR. |
| VueUse `useDark()` in VitePress | VitePress `useData().isDark` | Always (VitePress feature) | Proper integration with VitePress FOUC prevention. |
| `@tailwind base/components/utilities` | `@import "tailwindcss"` | Jan 2025 (v4.0) | Single import replaces three directives. |
| shadcn-vue `default` style | shadcn-vue `new-york` style | 2025 (default deprecated) | `new-york` is now the only actively maintained style. |

**Deprecated/outdated:**
- `tailwindcss-animate`: Replaced by `tw-animate-css`. Do not install the old package.
- `tailwind.config.js`: Not needed with Tailwind v4. Do not create this file.
- `postcss.config.js`: Not needed with `@tailwindcss/vite`. Do not create this file.
- shadcn-vue `default` style: Deprecated. Use `new-york` style.

## Open Questions

1. **shadcn-vue CLI `init` behavior in VitePress project**
   - What we know: The CLI expects a standard Vite project. VitePress projects have a different structure.
   - What's unclear: Whether `npx shadcn-vue@latest init` works out of the box in a VitePress project or requires manual configuration.
   - Recommendation: Try the CLI first. If it fails, create `components.json` and `lib/utils.ts` manually, then use `npx shadcn-vue@latest add <component>` which should work once `components.json` exists. The manual installation path is fully documented.

2. **Exact `source()` path for Tailwind content detection**
   - What we know: `source(".")` tells Tailwind to scan from the CSS file's directory. The CSS is in `.vitepress/theme/styles/`.
   - What's unclear: Whether `source("../..")` (website root) or `source(".")` is the correct relative path to ensure all `.vue` files and `.md` files are found.
   - Recommendation: Use `source("../..")` to point to the `website/` root. If classes are still missing, add explicit `@source` directives: `@source "../components/**/*.vue";` and `@source "../../../docs/**/*.md";`. Test with `npx vitepress build` early to catch purge issues.

3. **VitePress `docs/` subdirectory vs root-level markdown**
   - What we know: VitePress supports both patterns. The `docs/` subdirectory is conventional.
   - What's unclear: Whether the VitePress CLI expects a specific structure.
   - Recommendation: Use `docs/` as the source directory. Pass `docs` to VitePress commands: `vitepress dev docs`. This keeps markdown content separate from the `.vitepress/` config directory.

## Sources

### Primary (HIGH confidence)
- [VitePress Custom Theme Guide](https://vitepress.dev/guide/custom-theme) -- theme entry file, Layout.vue, enhanceApp
- [VitePress Runtime API](https://vitepress.dev/reference/runtime-api) -- useData(), isDark, Content component
- [VitePress Site Config - appearance](https://vitepress.dev/reference/site-config#appearance) -- dark mode, FOUC prevention, localStorage key
- [shadcn-vue Manual Installation](https://www.shadcn-vue.com/docs/installation/manual) -- complete CSS file with OKLCH variables, @theme inline, components.json
- [shadcn-vue VitePress Dark Mode](https://www.shadcn-vue.com/docs/dark-mode/vitepress) -- useData().isDark + useToggle pattern
- [Tailwind CSS Dark Mode docs](https://tailwindcss.com/docs/dark-mode) -- @custom-variant dark syntax
- [Tailwind CSS v4 Vite integration](https://tailwindcss.com/blog/tailwindcss-v4) -- @tailwindcss/vite plugin, CSS-first config

### Secondary (MEDIUM confidence)
- [Tailwind CSS GitHub Issue #16050](https://github.com/tailwindlabs/tailwindcss/issues/16050) -- .vitepress directory scanning fix, source() directive
- [Using Tailwind in VitePress (Paul van der Meijs)](https://paulvandermeijs.lol/articles/2025/06/using-tailwind-in-vitepress) -- source(".") pattern, VitePress config example
- [Migrating VitePress to Tailwind v4 (Soubiran)](https://soubiran.dev/series/create-a-blog-with-vitepress-and-vue-js-from-scratch/migrating-our-vitepress-blog-to-tailwind-css-version-4) -- @source directives, migration steps
- [shadcn-vue GitHub Discussion #785](https://github.com/unovue/shadcn-vue/discussions/785) -- installing shadcn-vue in VitePress

### Tertiary (LOW confidence)
- shadcn-vue CLI behavior in VitePress: inferred from standard Vite installation docs; no VitePress-specific CLI documentation found. Manual installation is the reliable fallback.

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- all versions verified against npm registry and official docs
- Architecture: HIGH -- VitePress custom theme is a well-documented pattern; shadcn-vue CSS variable structure verified from official manual installation guide
- Dark mode: HIGH -- VitePress built-in appearance system verified; FOUC prevention is automatic
- Tailwind v4 integration: HIGH -- source() directive verified via GitHub issue and community guides
- shadcn-vue in VitePress: MEDIUM -- works (confirmed by working examples) but CLI init may need manual fallback
- Pitfalls: HIGH -- all pitfalls verified with official documentation or GitHub issues

**Research date:** 2026-02-13
**Valid until:** 2026-03-15 (stable ecosystem, 30-day validity)
