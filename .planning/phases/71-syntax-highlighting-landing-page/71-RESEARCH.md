# Phase 71: Syntax Highlighting + Landing Page - Research

**Researched:** 2026-02-13
**Domain:** Shiki custom language/theme in VitePress + Vue landing page components
**Confidence:** HIGH

## Summary

Phase 71 builds on the Phase 70 VitePress scaffold to add two capabilities: (1) Mesh syntax highlighting via the existing TextMate grammar loaded into Shiki with a custom monochrome theme, and (2) a landing page with hero, feature showcase, and comparison sections rendered as Vue components.

The architecture is straightforward. VitePress uses Shiki internally for all code block highlighting. Custom languages are registered via `markdown.languages` in the VitePress config by importing the existing `mesh.tmLanguage.json` grammar. Custom themes are passed via `markdown.theme` as TextMate JSON theme objects. For the monochrome aesthetic, two custom themes (light and dark) are needed, each using only zero-chroma OKLCH-equivalent hex colors that align with the site's existing palette. VitePress generates dual-theme HTML with CSS variables (`--shiki-dark`, `--shiki-dark-bg`) that toggle via the `.dark` class -- which already works from Phase 70's `appearance: true` setup.

The landing page is a Vue SFC component rendered when `frontmatter.layout === 'home'` in `docs/index.md`. The existing Layout.vue already reads frontmatter from `useData()`. The landing page component contains the hero section, feature showcase with highlighted code examples, and the "Why Mesh?" comparison. Code examples on the landing page use Shiki programmatically via the `codeToHtml` function (not markdown code fences), giving full control over rendering within Vue templates.

**Primary recommendation:** Load the Mesh TextMate grammar into VitePress via `markdown.languages`, create two custom monochrome TextMate JSON themes (light + dark) passed via `markdown.theme: { light, dark }`, add the required CSS for `.vp-code` dual-theme switching, and build the landing page as a Vue SFC with Shiki programmatic highlighting for inline code examples.

## Standard Stack

### Core (already installed from Phase 70)

| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| VitePress | ^1.6.4 | SSG with built-in Shiki integration | Shiki is bundled -- no additional install needed |
| Shiki | (bundled with VitePress) | Syntax highlighter using TextMate grammars | VitePress uses Shiki internally. Same version VitePress ships. |
| Vue 3 | ^3.5.28 | Landing page components | Already installed from Phase 70 |
| Tailwind CSS v4 | ^4.1.18 | Utility classes for landing page layout | Already installed from Phase 70 |

### Supporting (no new installs needed)

| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| lucide-vue-next | ^0.564.0 | Icons in feature cards | Arrow icons for CTA, feature icons |
| shadcn-vue Button | (installed) | CTA buttons on landing page | Hero CTA, secondary actions |

### New Dependencies

**None.** VitePress bundles Shiki. The existing TextMate grammar is already in the repo. All landing page components use Vue + Tailwind already installed.

### Alternatives Considered

| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| Custom TextMate JSON themes | `createCssVariablesTheme` from shiki/core | CSS variables theme is less granular (only ~10 token categories vs unlimited scopes). Custom JSON themes give precise control over every TextMate scope, which is essential for making keywords/types/operators visually distinct in monochrome. Use custom JSON themes. |
| Programmatic Shiki in landing page | Markdown code fences in index.md | Code fences render as markdown content via `<Content />` with no layout control. The landing page needs code inside specific card/section layouts. Use programmatic `codeToHtml` for Vue component code blocks. |
| Two separate theme files (light.json + dark.json) | Single CSS variables theme with dark mode overrides | Two themes gives full control and integrates with VitePress's native dual-theme system (`markdown.theme: { light, dark }`). CSS variables theme requires manual CSS variable definitions and loses per-scope granularity. Use two theme files. |

## Architecture Patterns

### Recommended Project Structure (additions to Phase 70)

```
website/docs/.vitepress/
  config.mts                           # Add: markdown.languages, markdown.theme
  theme/
    Layout.vue                         # Modify: add frontmatter-based routing
    components/
      NavBar.vue                       # Existing (unchanged)
      ThemeToggle.vue                  # Existing (unchanged)
      landing/
        HeroSection.vue                # NEW: Hero with tagline + code sample + CTA
        FeatureShowcase.vue            # NEW: 3-4 feature cards with code examples
        WhyMesh.vue                    # NEW: Comparison section vs Elixir/Rust/Go
        LandingPage.vue                # NEW: Composes Hero + Features + WhyMesh
    shiki/
      mesh-light.json                  # NEW: Custom light monochrome Shiki theme
      mesh-dark.json                   # NEW: Custom dark monochrome Shiki theme
    styles/
      main.css                         # Modify: add code block CSS for dual themes
      code.css                         # NEW: Code block styling (vp-code, shiki)
  docs/
    index.md                           # Modify: set layout: home in frontmatter
```

### Pattern 1: Loading Custom Language in VitePress Config

**What:** Register the Mesh TextMate grammar with VitePress's Shiki integration.
**When:** Config setup, one-time.

```typescript
// .vitepress/config.mts
import { defineConfig } from 'vitepress'
import tailwindcss from '@tailwindcss/vite'
import path from 'node:path'
import meshGrammar from '../../editors/vscode-mesh/syntaxes/mesh.tmLanguage.json'
import meshLight from './theme/shiki/mesh-light.json'
import meshDark from './theme/shiki/mesh-dark.json'

export default defineConfig({
  title: 'Mesh',
  description: 'The Mesh Programming Language',
  appearance: true,

  markdown: {
    // Register Mesh language for code fences: ```mesh
    languages: [
      {
        ...meshGrammar,
        name: 'mesh',
        aliases: ['mesh'],
      } as any
    ],
    // Dual monochrome themes for light/dark mode
    theme: {
      light: meshLight as any,
      dark: meshDark as any,
    },
  },

  vite: {
    plugins: [tailwindcss()],
    resolve: {
      alias: {
        '@': path.resolve(__dirname, './theme'),
      },
    },
  },
})
```

Source: [VitePress Discussion #4702](https://github.com/vuejs/vitepress/discussions/4702), [Shiki Load Custom Languages](https://shiki.style/guide/load-lang)

**Key details:**
- The `...meshGrammar` spread provides `scopeName`, `patterns`, `repository` from the JSON
- `name: 'mesh'` sets the language ID used in markdown code fences (` ```mesh `)
- `as any` casts are needed because TypeScript cannot infer deep TextMate JSON types (confirmed by VitePress maintainer brc-dd)
- The grammar file is at `editors/vscode-mesh/syntaxes/mesh.tmLanguage.json` relative to the repo root -- the import path resolves relative to `config.mts` in `website/docs/.vitepress/`
- No need for `shikiSetup` or `loadLanguage` -- the `languages` array handles registration

### Pattern 2: Custom Monochrome TextMate Theme Structure

**What:** TextMate JSON theme files that use only grayscale colors for syntax highlighting.
**When:** Phase 71 theme creation.

```json
{
  "name": "mesh-light",
  "type": "light",
  "colors": {
    "editor.background": "#ffffff",
    "editor.foreground": "#1a1a1a"
  },
  "tokenColors": [
    {
      "settings": {
        "foreground": "#1a1a1a",
        "background": "#ffffff"
      }
    },
    {
      "name": "Comments",
      "scope": ["comment", "comment.line"],
      "settings": {
        "foreground": "#a0a0a0",
        "fontStyle": "italic"
      }
    },
    {
      "name": "Strings",
      "scope": ["string", "string.quoted"],
      "settings": {
        "foreground": "#555555"
      }
    },
    {
      "name": "Keywords",
      "scope": ["keyword", "keyword.control", "keyword.declaration", "keyword.operator.mesh"],
      "settings": {
        "foreground": "#000000",
        "fontStyle": "bold"
      }
    },
    {
      "name": "Types",
      "scope": ["entity.name.type", "support.type"],
      "settings": {
        "foreground": "#333333",
        "fontStyle": "italic"
      }
    },
    {
      "name": "Functions",
      "scope": ["entity.name.function", "support.function"],
      "settings": {
        "foreground": "#222222"
      }
    },
    {
      "name": "Operators",
      "scope": ["keyword.operator"],
      "settings": {
        "foreground": "#444444"
      }
    },
    {
      "name": "Constants and Booleans",
      "scope": ["constant", "constant.language", "constant.numeric"],
      "settings": {
        "foreground": "#666666"
      }
    },
    {
      "name": "Variables",
      "scope": ["variable", "variable.other"],
      "settings": {
        "foreground": "#2a2a2a"
      }
    },
    {
      "name": "Punctuation",
      "scope": ["punctuation"],
      "settings": {
        "foreground": "#888888"
      }
    }
  ]
}
```

Source: [Shiki Load Custom Themes](https://shiki.style/guide/load-theme), [Shiki Theme Colors Manipulation](https://shiki.style/guide/theme-colors)

**Key design principles for monochrome differentiation:**
- **Keywords:** Darkest (#000000) + bold -- most prominent
- **Types:** Dark (#333333) + italic -- structurally important, visually distinct via style
- **Functions:** Very dark (#222222) -- prominent but unbold
- **Variables:** Near-foreground (#2a2a2a) -- blend with body text
- **Strings:** Medium gray (#555555) -- clearly data, not code
- **Comments:** Light gray (#a0a0a0) + italic -- deliberately faded
- **Constants:** Medium-dark (#666666) -- between strings and variables
- **Operators:** Medium (#444444) -- visible but not dominant
- **Punctuation:** Light (#888888) -- structural, not semantic

The dark theme inverts this hierarchy using the existing OKLCH palette values (0.985 for lightest, 0.205 for darkest).

### Pattern 3: Required CSS for Custom Theme Code Blocks

**What:** CSS that enables dual-theme code block rendering in a VitePress custom theme.
**When:** Immediately after adding Shiki themes.

```css
/* code.css -- imported in main.css */

/* Dual theme switching for code blocks */
.dark .vp-code span {
  color: var(--shiki-dark, inherit);
  font-style: var(--shiki-dark-font-style, inherit);
  font-weight: var(--shiki-dark-font-weight, inherit);
}

html:not(.dark) .vp-code span {
  color: var(--shiki-light, inherit);
  font-style: var(--shiki-light-font-style, inherit);
  font-weight: var(--shiki-light-font-weight, inherit);
}

/* Code block container styling */
div[class*='language-'] {
  position: relative;
  border-radius: var(--radius);
  overflow: hidden;
}

/* Light mode code block background */
div[class*='language-'] > pre {
  background-color: var(--muted);
  padding: 1rem 1.25rem;
  overflow-x: auto;
}

/* Dark mode code block background */
.dark div[class*='language-'] > pre {
  background-color: var(--secondary);
}

/* Code font */
.vp-code, .vp-code * {
  font-family: ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, 'Liberation Mono', monospace;
  font-size: 0.875rem;
  line-height: 1.7;
}

/* Language label (shown in top-right) */
div[class*='language-']::before {
  content: attr(class);
  position: absolute;
  top: 0.5rem;
  right: 0.75rem;
  font-size: 0.75rem;
  color: var(--muted-foreground);
}

/* Copy button */
div[class*='language-'] > button.copy {
  position: absolute;
  top: 0.5rem;
  right: 0.5rem;
  opacity: 0;
  cursor: pointer;
  background: var(--muted);
  border: 1px solid var(--border);
  border-radius: calc(var(--radius) - 4px);
  padding: 0.25rem;
  transition: opacity 0.2s;
}

div[class*='language-']:hover > button.copy {
  opacity: 1;
}
```

Source: [VitePress Discussion #3737](https://github.com/vuejs/vitepress/discussions/3737)

**Critical:** VitePress custom themes do NOT automatically style code blocks. Without this CSS, code renders as unstyled `<pre>` blocks with no colors. The `.vp-code` class and `--shiki-dark`/`--shiki-light` CSS variables are generated by VitePress/Shiki and must be targeted explicitly.

### Pattern 4: Frontmatter-Based Layout Routing

**What:** Layout.vue routes to different page components based on frontmatter.
**When:** Landing page needs different layout than docs pages.

```vue
<!-- Layout.vue -->
<script setup lang="ts">
import { useData } from 'vitepress'
import NavBar from './components/NavBar.vue'
import LandingPage from './components/landing/LandingPage.vue'

const { frontmatter } = useData()
</script>

<template>
  <div class="min-h-screen bg-background text-foreground">
    <NavBar />
    <LandingPage v-if="frontmatter.layout === 'home'" />
    <main v-else class="mx-auto max-w-4xl px-4 py-8">
      <Content />
    </main>
  </div>
</template>
```

```markdown
<!-- docs/index.md -->
---
layout: home
title: Mesh Programming Language
---
```

Source: [VitePress Custom Theme](https://vitepress.dev/guide/custom-theme), [VitePress Frontmatter Config](https://vitepress.dev/reference/frontmatter-config)

**Key detail:** The `Content` component is NOT rendered for the landing page -- the entire page is a Vue SFC. This gives full control over layout, sections, and code examples without markdown constraints.

### Pattern 5: Programmatic Shiki Highlighting in Vue Components

**What:** Use Shiki's `codeToHtml` API directly in Vue components to render highlighted code.
**When:** Landing page code examples that need custom layout integration.

```vue
<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { createHighlighter } from 'shiki'
import meshGrammar from '../../../../editors/vscode-mesh/syntaxes/mesh.tmLanguage.json'
import meshLight from '../shiki/mesh-light.json'
import meshDark from '../shiki/mesh-dark.json'

const props = defineProps<{
  code: string
}>()

const highlightedHtml = ref('')

onMounted(async () => {
  const highlighter = await createHighlighter({
    themes: [meshLight as any, meshDark as any],
    langs: [{ ...meshGrammar, name: 'mesh' } as any],
  })

  highlightedHtml.value = highlighter.codeToHtml(props.code, {
    lang: 'mesh',
    themes: {
      light: 'mesh-light',
      dark: 'mesh-dark',
    },
  })
})
</script>

<template>
  <div class="code-example" v-html="highlightedHtml" />
</template>
```

Source: [Shiki Load Custom Languages](https://shiki.style/guide/load-lang), [Shiki Dual Themes](https://shiki.style/guide/dual-themes)

**Important consideration:** Creating a highlighter instance per component is wasteful. A better pattern is a shared composable that creates the highlighter once and caches it:

```typescript
// composables/useShiki.ts
import { ref } from 'vue'
import type { Highlighter } from 'shiki'

let highlighter: Highlighter | null = null
let highlighterPromise: Promise<Highlighter> | null = null

export async function getHighlighter(): Promise<Highlighter> {
  if (highlighter) return highlighter
  if (highlighterPromise) return highlighterPromise

  highlighterPromise = (async () => {
    const { createHighlighter } = await import('shiki')
    const meshGrammar = await import('../../../../editors/vscode-mesh/syntaxes/mesh.tmLanguage.json')
    const meshLight = await import('../shiki/mesh-light.json')
    const meshDark = await import('../shiki/mesh-dark.json')

    highlighter = await createHighlighter({
      themes: [meshLight.default as any, meshDark.default as any],
      langs: [{ ...meshGrammar.default, name: 'mesh' } as any],
    })
    return highlighter
  })()

  return highlighterPromise
}
```

**Alternative approach (simpler, recommended):** Since VitePress already has Shiki configured with the Mesh language and themes, an even better approach is to put the code examples in markdown files and use `<Content />` -- OR -- create a reusable `CodeBlock.vue` component that uses `v-html` with pre-highlighted HTML strings computed at build time. However, the simplest viable approach for a landing page is to highlight at component mount time as shown above, because the landing page code examples are static and few (5-6 blocks total).

### Anti-Patterns to Avoid

- **Using `shikiSetup` with `loadLanguage`/`loadTheme`:** Unnecessary complexity. VitePress's `markdown.languages` and `markdown.theme` config handles everything. The `shikiSetup` callback is for advanced use cases only.
- **Putting landing page content in markdown:** The landing page needs precise layout control (hero grid, feature cards, comparison table). Markdown's linear flow doesn't support this. Use Vue components.
- **Creating a Shiki highlighter in every component:** The `createHighlighter` call is expensive (parses grammars, compiles regexes). Create one instance and share it via a composable.
- **Using `createCssVariablesTheme` for the monochrome theme:** Only supports ~10 token categories. A custom TextMate JSON theme supports unlimited scopes and can differentiate keywords, types, operators, strings, comments, functions, constants, etc. individually.
- **Hardcoding hex colors in themes without relating to the palette:** The Shiki theme colors should derive from the same OKLCH palette used by the site. Convert the site's OKLCH values to hex for theme files (Shiki requires hex).
- **Forgetting `.vp-code` dual-theme CSS:** Without it, code blocks render plain white-on-white in light mode or invisible in dark mode. This is the most common mistake with VitePress custom themes.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Syntax highlighting engine | Custom regex-based highlighter | Shiki (bundled with VitePress) | TextMate grammars handle nested scopes, multiline patterns, embedded languages. Shiki handles all of this. |
| TextMate grammar for Mesh | New grammar from scratch | Existing `editors/vscode-mesh/syntaxes/mesh.tmLanguage.json` | Already tested in VS Code. Covers keywords, types, operators, strings, comments, interpolation, pattern matching, do/end blocks. |
| Dark/light theme switching for code blocks | Custom JavaScript toggle | VitePress dual-theme system (`markdown.theme: { light, dark }`) | VitePress generates CSS variables (`--shiki-dark`, `--shiki-light`) automatically. CSS selectors on `.dark` class handle switching. |
| Markdown-to-HTML pipeline | Custom markdown parser | VitePress + Markdown-it + Shiki pipeline | Battle-tested. Handles code fences, language detection, theme application, HTML generation. |
| Landing page routing | Custom router or separate HTML | VitePress frontmatter `layout` field + Layout.vue conditional rendering | Built into VitePress. Clean separation between landing page and doc pages. |

**Key insight:** Shiki is already bundled with VitePress. The Mesh TextMate grammar already exists. The dual-theme system already works with VitePress's `appearance: true` dark mode. This phase is about wiring existing pieces together and creating the theme/content, not building new infrastructure.

## Common Pitfalls

### Pitfall 1: Code Blocks Render Without Colors in Custom Theme

**What goes wrong:** After adding `markdown.theme` and `markdown.languages`, code fences in markdown render as plain `<pre>` blocks with no syntax coloring.
**Why it happens:** VitePress custom themes (blank Layout.vue, no default theme extension) do NOT include the CSS that applies Shiki's CSS variables to `<span>` elements. The default theme includes this CSS automatically, but custom themes start from zero.
**How to avoid:** Add the dual-theme CSS targeting `.vp-code span` with `color: var(--shiki-dark)` / `var(--shiki-light)`. See Pattern 3 above.
**Warning signs:** HTML inspection shows `<span style="--shiki-dark:#xxx;--shiki-light:#yyy">` but no visible color difference.
**Confidence:** HIGH -- confirmed by [VitePress Discussion #3737](https://github.com/vuejs/vitepress/discussions/3737).

### Pitfall 2: TypeScript Errors on Theme/Language JSON Imports

**What goes wrong:** TypeScript complains about type mismatch when passing imported JSON objects to `markdown.theme` or `markdown.languages`.
**Why it happens:** Shiki's types expect `BundledTheme` or `BuiltinLanguage`, but imported JSON objects are typed as generic objects. TypeScript cannot infer the deep nested TextMate types from JSON.
**How to avoid:** Cast with `as any`. This is the officially recommended approach (confirmed by VitePress maintainer brc-dd).
**Warning signs:** Red squiggly lines on theme/language assignments in config.mts.
**Confidence:** HIGH -- confirmed by [VitePress Discussion #4702](https://github.com/vuejs/vitepress/discussions/4702).

### Pitfall 3: Grammar Import Path Resolution

**What goes wrong:** The TextMate grammar JSON fails to import because the relative path from `config.mts` to the repo-root `editors/` directory is wrong.
**Why it happens:** `config.mts` is in `website/docs/.vitepress/`, so importing `editors/vscode-mesh/syntaxes/mesh.tmLanguage.json` requires going up 3 directories: `../../../editors/vscode-mesh/syntaxes/mesh.tmLanguage.json`. Getting the depth wrong causes a module-not-found error.
**How to avoid:** Use the correct relative path from `config.mts` location. Alternatively, copy the grammar file into the `.vitepress/` directory to simplify the import. However, this creates a maintenance burden (two copies). Using a symlink or the relative path is cleaner.
**Warning signs:** Build error: `Cannot find module '../editors/vscode-mesh/syntaxes/mesh.tmLanguage.json'`.
**Confidence:** HIGH -- filesystem path calculation.

### Pitfall 4: Monochrome Theme Lacks Visual Distinction

**What goes wrong:** All tokens appear similar gray, making code unreadable. Users cannot distinguish keywords from variables from strings.
**Why it happens:** Monochrome design constrains to a single hue axis (lightness only). Without careful lightness distribution AND font style variation (bold, italic), tokens merge visually.
**How to avoid:** Use multiple differentiation axes: (1) lightness spread across the full range, (2) font-weight: bold for keywords, (3) font-style: italic for comments and types, (4) relative sizing through spacing. Test with real Mesh code examples.
**Warning signs:** Screenshots of code look like a single gray blob with no structure.
**Confidence:** MEDIUM -- design judgment, needs visual verification.

### Pitfall 5: Shiki Highlighter Instance Not Available During SSR/SSG

**What goes wrong:** The landing page code examples show unhighlighted code during SSG build, or cause SSR errors.
**Why it happens:** `createHighlighter` is async and returns a Promise. During VitePress SSG, components render synchronously for the initial HTML. If the highlighter hasn't loaded yet, `v-html` receives an empty string.
**How to avoid:** Two strategies: (1) Use `onMounted` with a loading state (code appears after hydration -- acceptable for a landing page since the SSG HTML shows a placeholder), or (2) pre-compute the highlighted HTML at build time using a VitePress `transformPageData` hook or a build script. For simplicity, option (1) is sufficient since the landing page is interactive anyway.
**Warning signs:** Flash of unstyled code on initial page load, or blank code blocks in the SSG-generated HTML.
**Confidence:** MEDIUM -- depends on SSR behavior of async operations in Vue setup.

## Code Examples

### TextMate Grammar Scopes in mesh.tmLanguage.json

The existing grammar defines these scopes that the Shiki theme must target:

| TextMate Scope | Mesh Construct | Visual Treatment (monochrome) |
|----------------|----------------|-------------------------------|
| `comment.line.hash.mesh` | `# comment` | Light gray + italic |
| `string.quoted.double.mesh` | `"string"` | Medium gray |
| `constant.numeric.float.mesh` | `3.14` | Medium-dark gray |
| `constant.numeric.integer.mesh` | `42` | Medium-dark gray |
| `keyword.control.mesh` | `if`, `else`, `case`, `match`, `do`, `end`, `return` | Darkest + bold |
| `keyword.declaration.mesh` | `fn`, `let`, `def`, `type`, `struct`, `module`, `actor`, `pub` | Darkest + bold |
| `keyword.operator.mesh` | `and`, `or`, `not`, `in`, `spawn`, `send`, `receive` | Dark + bold |
| `constant.language.mesh` | `true`, `false`, `nil` | Medium-dark gray |
| `support.type.mesh` | `Int`, `Float`, `String`, `Bool`, `Option`, `Result`, `List`, `Map` | Dark + italic |
| `support.function.mesh` | `Some`, `None`, `Ok`, `Err` | Dark |
| `entity.name.type.mesh` | User-defined types (PascalCase) | Dark + italic |
| `keyword.operator.pipe.mesh` | `\|>` | Medium-dark |
| `keyword.operator.arrow.mesh` | `->` | Medium-dark |
| `keyword.operator.annotation.mesh` | `::` | Medium-dark |
| `keyword.operator.comparison.mesh` | `==`, `!=`, `<`, `>` | Medium |
| `keyword.operator.arithmetic.mesh` | `+`, `-`, `*`, `/` | Medium |
| `keyword.operator.assignment.mesh` | `=` | Medium |
| `entity.name.function.mesh` | Function names after `fn`/`def` | Very dark |
| `variable.other.mesh` | Regular identifiers | Near-foreground |
| `constant.character.escape.mesh` | `\\n`, `\\t` etc. | Medium gray |
| `meta.interpolation.mesh` | `${expr}` in strings | Medium-dark |
| `punctuation.section.interpolation.begin.mesh` | `${` | Medium |
| `punctuation.section.interpolation.end.mesh` | `}` | Medium |

### Example Mesh Code for Landing Page

**Hero section -- primary code sample:**

```mesh
# A simple HTTP server with actors
module HttpExample

pub fn main() do
  let router = HTTP.new()
    |> HTTP.on_get("/", fn(req) do
      HTTP.json(200, %{"message": "Hello, Mesh!"})
    end)
    |> HTTP.on_get("/users/:id", fn(req) do
      let id = Request.param(req, "id")
      HTTP.json(200, %{"user": id})
    end)

  HTTP.serve(router, 8080)
  IO.puts("Server running on :8080")
end
```

**Feature 1 -- Actors & Concurrency:**

```mesh
actor Counter do
  def init() do
    0
  end

  def handle_cast(:increment, state) do
    state + 1
  end

  def handle_call(:get, _from, state) do
    {:reply, state, state}
  end
end

let pid = spawn(Counter)
cast(pid, :increment)
let count = call(pid, :get)
```

**Feature 2 -- Pattern Matching:**

```mesh
fn describe(value) do
  match value do
    0 -> "zero"
    n when n > 0 -> "positive"
    n when n < 0 -> "negative"
  end
end

fn process(result) do
  match result do
    Ok(value) -> IO.puts("Got: ${value}")
    Err(msg) -> IO.puts("Error: ${msg}")
  end
end
```

**Feature 3 -- Type Inference:**

```mesh
# Types are inferred -- no annotations needed
let name = "Mesh"
let numbers = [1, 2, 3, 4, 5]
let doubled = numbers
  |> List.map(fn(n) do n * 2 end)
  |> List.filter(fn(n) do n > 4 end)

# Structs with inferred field types
struct User do
  name :: String
  age :: Int
end

let user = User { name: "Alice", age: 30 }
```

**Feature 4 -- Pipe Operator:**

```mesh
let result = "hello world"
  |> String.split(" ")
  |> List.map(fn(word) do
    String.upcase(word)
  end)
  |> String.join(", ")

# result == "HELLO, WORLD"
```

### Landing Page Section Structure

```
LandingPage.vue
  +-- HeroSection.vue
  |     +-- Tagline + subtitle
  |     +-- Highlighted code sample (hero example)
  |     +-- CTA button ("Get Started" -> /docs/getting-started/)
  |
  +-- FeatureShowcase.vue
  |     +-- Feature card 1: Actors & Concurrency
  |     +-- Feature card 2: Pattern Matching
  |     +-- Feature card 3: Type Inference
  |     +-- Feature card 4: Pipe Operator
  |     Each card: title + description + highlighted code example
  |
  +-- WhyMesh.vue
        +-- Comparison prose: vs Elixir, vs Rust, vs Go
        +-- Key differentiators table or bullet list
```

### VitePress Config Complete (Phase 71)

```typescript
// .vitepress/config.mts
import { defineConfig } from 'vitepress'
import tailwindcss from '@tailwindcss/vite'
import path from 'node:path'
// @ts-ignore -- TextMate JSON types cannot be inferred
import meshGrammar from '../../../editors/vscode-mesh/syntaxes/mesh.tmLanguage.json'
// @ts-ignore
import meshLight from './theme/shiki/mesh-light.json'
// @ts-ignore
import meshDark from './theme/shiki/mesh-dark.json'

export default defineConfig({
  title: 'Mesh',
  description: 'The Mesh Programming Language',
  appearance: true,

  markdown: {
    languages: [
      {
        ...(meshGrammar as any),
        name: 'mesh',
        aliases: ['mesh'],
      },
    ],
    theme: {
      light: meshLight as any,
      dark: meshDark as any,
    },
  },

  vite: {
    plugins: [tailwindcss()],
    resolve: {
      alias: {
        '@': path.resolve(__dirname, './theme'),
      },
    },
  },
})
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Shiki `path` property for grammar files | Import JSON, pass object to `langs`/`languages` | Shiki v1.0 (2024) | Shiki is now environment-agnostic. No filesystem access. Must import/read files yourself. |
| `shikiSetup` + `loadLanguage`/`loadTheme` | `markdown.languages` + `markdown.theme` config | VitePress 1.x | Simpler config-level approach. No async setup needed. |
| Single theme with media query toggle | `markdown.theme: { light, dark }` dual themes | Shiki v1.0 / VitePress 1.x | VitePress generates CSS variables for both themes. Toggle via `.dark` class. |
| Prism.js for highlighting | Shiki (TextMate grammar engine) | VitePress 1.0 (2024) | More accurate highlighting, VS Code parity, TextMate grammar ecosystem. |

**Deprecated/outdated:**
- `shiki.loadLanguage(path)` -- path-based loading removed in Shiki v1.0. Pass grammar objects directly.
- Prism.js -- not used by VitePress. Do not install.
- `highlight.js` -- not used by VitePress. Do not install.

## Open Questions

1. **JSON import assertion syntax in .mts files**
   - What we know: VitePress config uses `.mts` extension for ESM compatibility. JSON imports may need `with { type: 'json' }` (import assertions) or work without it depending on Node/esbuild version.
   - What's unclear: Whether VitePress's esbuild-based config bundler supports `with { type: 'json' }` syntax or requires bare imports.
   - Recommendation: Try bare import first (`import meshGrammar from './path.json'`). If it fails, add `with { type: 'json' }`. If both fail, use `createRequire` from `node:module` or `fs.readFileSync` + `JSON.parse`.

2. **SSG behavior of programmatic Shiki in Vue components**
   - What we know: VitePress SSG pre-renders pages to static HTML. Async operations in `onMounted` run only on client side.
   - What's unclear: Whether the landing page code examples will show highlighted code in the SSG output, or only after client-side hydration.
   - Recommendation: Accept client-side-only highlighting for the landing page code examples. The SSG HTML can include the raw code in a `<pre>` tag as fallback. This is acceptable because: (a) the page is interactive anyway, (b) hydration is fast, (c) the visible content above the fold (tagline, CTA) is static HTML.

3. **Grammar completeness for all Mesh constructs**
   - What we know: The existing grammar covers keywords, types, operators, strings, comments, interpolation, functions, and variables.
   - What's unclear: Whether pattern matching constructs (`match value do ... end`), actor blocks (`actor Counter do ... end`), and supervisor trees highlight correctly with the current grammar.
   - Recommendation: Test with all feature showcase code examples during development. If gaps are found, extend the grammar as part of this phase (minor additions to `mesh.tmLanguage.json`).

## Sources

### Primary (HIGH confidence)
- [VitePress Discussion #4702](https://github.com/vuejs/vitepress/discussions/4702) -- Loading custom themes and languages, `as any` cast, `markdown.languages` pattern
- [VitePress Discussion #3737](https://github.com/vuejs/vitepress/discussions/3737) -- Custom theme code block CSS, `.vp-code` styling
- [Shiki Load Custom Languages](https://shiki.style/guide/load-lang) -- Grammar object structure, `name`/`scopeName` properties
- [Shiki Load Custom Themes](https://shiki.style/guide/load-theme) -- TextMate JSON theme format, `tokenColors` structure
- [Shiki Dual Themes](https://shiki.style/guide/dual-themes) -- CSS variables approach, `--shiki-dark`/`--shiki-light`, required CSS
- [Shiki Theme Colors](https://shiki.style/guide/theme-colors) -- `createCssVariablesTheme`, `colorReplacements` options
- [VitePress Custom Theme](https://vitepress.dev/guide/custom-theme) -- Layout.vue, frontmatter routing, `useData()`
- [VitePress Frontmatter Config](https://vitepress.dev/reference/frontmatter-config) -- `layout` option, custom frontmatter fields
- [VitePress Using Vue in Markdown](https://vitepress.dev/guide/using-vue) -- Vue components in markdown, `<script setup>` blocks
- [VitePress Site Config](https://vitepress.dev/reference/site-config#markdown) -- `markdown.theme`, `markdown.languages` type definitions
- Existing `editors/vscode-mesh/syntaxes/mesh.tmLanguage.json` -- 150 lines, verified in codebase

### Secondary (MEDIUM confidence)
- [Shiki Themes](https://shiki.style/themes) -- Built-in theme catalog, `none` theme option
- [Shiki VitePress Integration](https://shiki.style/packages/vitepress) -- VitePress-specific features, code transformers

### Tertiary (LOW confidence)
- SSG behavior of async Shiki in Vue components -- inferred from Vue SSR documentation, not tested specifically in VitePress context

## Metadata

**Confidence breakdown:**
- Syntax highlighting (loading grammar + themes into VitePress): HIGH -- verified via VitePress maintainer guidance, multiple sources agree
- Custom TextMate theme format: HIGH -- verified via Shiki official docs, TextMate standard
- Code block CSS for custom themes: HIGH -- verified via VitePress discussion with working solutions
- Landing page architecture (frontmatter routing + Vue components): HIGH -- standard VitePress custom theme pattern documented officially
- Monochrome theme design (visual distinction): MEDIUM -- design judgment, needs visual testing and iteration
- Programmatic Shiki in landing page components: MEDIUM -- API verified, SSG behavior needs testing

**Research date:** 2026-02-13
**Valid until:** 2026-03-15 (stable ecosystem, 30-day validity)
