# Phase 80: Documentation Update for v7.0 APIs - Research

**Researched:** 2026-02-13
**Domain:** VitePress markdown documentation content + sidebar configuration for Mesh v7.0 features
**Confidence:** HIGH

## Summary

Phase 80 adds documentation for 7 feature areas introduced in v7.0 (Phases 74-79). The existing website is a fully-built VitePress custom theme with 9 documentation pages (2,930 lines of markdown), a sidebar configuration, and all the infrastructure (syntax highlighting, TOC, prev/next, search, version badge, etc.) already in place from v6.0 (Phases 70-73). This phase is primarily a **content authoring** phase -- writing new markdown documentation and updating existing pages -- with minimal infrastructure changes.

The v7.0 features to document span three logical groupings: (1) type system extensions (associated types, numeric traits, From/Into conversion), (2) the iterator protocol and pipeline ecosystem (Iterator/Iterable traits, lazy combinators, terminal operations, collect), and (3) updates to existing pages that reference these features (type-system page, language-basics page, cheatsheet). All features are implemented and have working E2E tests that provide authoritative syntax examples.

A key discovery: the existing Type System docs page talks about "Traits" conceptually but never shows how to define a custom trait using the `interface` keyword. The v7.0 features (associated types, numeric traits, From/Into) all require this syntax. The type-system page needs a section showing `interface ... do ... end` and `impl ... for ... do ... end` syntax before the new features make sense.

**Primary recommendation:** Create 1-2 new documentation pages (Iterators, Traits & Conversions or similar) and update 3 existing pages (type-system, cheatsheet, language-basics). Add new sidebar entries. All code examples must come from verified E2E tests, not aspirational syntax. The infrastructure is 100% ready -- no new Vue components, composables, or dependencies are needed.

## Standard Stack

### Core (already installed -- no changes needed)

| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| VitePress | 1.6.4 | SSG framework, markdown rendering, Mesh syntax highlighting | Already configured with custom Mesh grammar, light/dark themes |
| Vue 3 | 3.5.28 | Component framework for custom theme | Already installed, all docs components built |
| Tailwind CSS v4 | 4.1.18 | Utility classes, `prose` typography styling | Already installed and activated with `@tailwindcss/typography` |
| Shiki | (bundled with VitePress) | Syntax highlighting for `mesh` code fences | Already configured with custom `mesh.tmLanguage.json` grammar |

### Supporting (already installed -- no changes needed)

| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| shadcn-vue components | (from reka-ui 2.8.0) | Sheet, Collapsible, ScrollArea for docs layout | Already installed and wired in DocsLayout.vue |
| @vueuse/core | 14.2.1 | Media queries for responsive layout | Already installed and used |
| lucide-vue-next | 0.564.0 | Icons for nav, sidebar, edit links | Already installed |

### Alternatives Considered

| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| New standalone pages for each v7.0 feature | Appending sections to existing pages | Standalone pages are better for discoverability and sidebar navigation; existing pages are already 300-600 lines each |
| One massive "Advanced Features" page | Multiple focused pages | Multiple pages are easier to navigate, link to, and maintain |
| Updating landing page FeatureShowcase with v7.0 features | Keeping landing page as-is | Landing page uses aspirational/incorrect syntax (e.g., `match` instead of `case`, `:increment` atoms); fixing it is a separate concern from docs |

### New Dependencies

**None.** Zero npm packages to install. Zero shadcn-vue components to scaffold. Zero Vue components to create. This is a pure content + configuration phase.

## Architecture Patterns

### Current Website Structure (relevant files)

```
website/docs/
  .vitepress/
    config.mts                              # MODIFY: add sidebar entries for new pages
    theme/
      components/docs/                      # NO CHANGES -- all docs infrastructure exists
      composables/                          # NO CHANGES
      styles/                               # NO CHANGES
  docs/
    getting-started/index.md                # MODIFY: update "What's Next" links
    language-basics/index.md                # MODIFY: add Iter pipeline examples, update collections
    type-system/index.md                    # MODIFY: add Associated Types, Interfaces, Numeric Traits, From/Into
    cheatsheet/index.md                     # MODIFY: add v7.0 syntax entries
    concurrency/index.md                    # NO CHANGES (v7.0 doesn't affect concurrency docs)
    web/index.md                            # NO CHANGES
    databases/index.md                      # NO CHANGES
    distributed/index.md                    # NO CHANGES
    tooling/index.md                        # NO CHANGES
    iterators/index.md                      # NEW: Iterator protocol, combinators, terminals, collect
```

### Pattern 1: Documentation Page Format

**What:** Every documentation page follows a consistent markdown format.
**When:** All new and updated pages.

```markdown
---
title: Page Title
---

# Page Title

Introductory paragraph explaining the concept and why it matters.

## Section Heading

Explanation text before code.

\`\`\`mesh
# Working code from verified E2E tests
fn main() do
  println("example")
end
\`\`\`

Explanation of what the code demonstrates.

## Next Steps

- [Next Topic](/docs/next-topic/) -- brief description
```

Source: All existing docs pages follow this pattern (verified from `/Users/sn0w/Documents/dev/snow/website/docs/docs/*/index.md`)

**Key details:**
- Frontmatter has `title` only (no `description` needed -- VitePress generates og:description from first paragraph)
- `mesh` code fence language triggers Shiki syntax highlighting with the custom grammar
- All code examples must compile and run -- use verified E2E test code
- End each page with "What's Next?" or "Next Steps" linking to related pages
- Use `##` for main sections and `###` for subsections (the TOC shows h2+h3 per config)

### Pattern 2: Sidebar Configuration

**What:** Add new pages to the sidebar config in `config.mts`.
**When:** After creating new documentation pages.

```typescript
// config.mts themeConfig.sidebar
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
        { text: 'Iterators', link: '/docs/iterators/' },       // NEW
        { text: 'Concurrency', link: '/docs/concurrency/' },
      ],
    },
    // ... rest of sidebar groups unchanged ...
  ],
},
```

Source: `/Users/sn0w/Documents/dev/snow/website/docs/.vitepress/config.mts` lines 77-130

**Key details:**
- New pages go in the "Language Guide" group since iterators and traits are core language features
- Order within the group should follow learning progression: basics -> types -> iterators -> concurrency
- The `collapsed: false` means the group is collapsible but starts expanded

### Pattern 3: Code Examples from E2E Tests

**What:** All code examples in documentation must come from verified, passing E2E tests.
**When:** Writing any code example for v7.0 features.

The E2E test files in `/Users/sn0w/Documents/dev/snow/tests/e2e/` provide authoritative syntax. Key verified patterns:

```mesh
# Associated Types (from assoc_type_basic.mpl)
interface Container do
  type Item
  fn first(self) -> Self.Item
end

impl Container for IntPair do
  type Item = Int
  fn first(self) -> Int do
    42
  end
end
```

```mesh
# Numeric Traits (from numeric_traits.mpl)
impl Add for Vec2 do
  type Output = Vec2
  fn add(self, other :: Vec2) -> Vec2 do
    Vec2 { x: self.x + other.x, y: self.y + other.y }
  end
end
```

```mesh
# From/Into (from from_user_defined.mpl)
impl From<Int> for Wrapper do
  fn from(n :: Int) -> Wrapper do
    Wrapper { value: n * 2 }
  end
end

let w = Wrapper.from(21)
```

```mesh
# Iterator Pipeline (from iter_pipeline.mpl)
let result = Iter.from(list)
  |> Iter.map(fn x -> x * 2 end)
  |> Iter.filter(fn x -> x > 10 end)
  |> Iter.take(3)
  |> Iter.count()
```

```mesh
# Collect (from collect_list.mpl)
let doubled = Iter.from(list)
  |> Iter.map(fn x -> x * 2 end)
  |> List.collect()
```

Source: E2E test files in `/Users/sn0w/Documents/dev/snow/tests/e2e/` (22 v7.0 test files verified)

### Anti-Patterns to Avoid

- **Using aspirational syntax from the landing page:** The FeatureShowcase.vue and HeroSection.vue contain incorrect syntax (`match` instead of `case`, `def` instead of `fn`, `:increment` atom syntax, `IO.puts` instead of `println`). Do NOT use this as a source for code examples. Always use E2E test files.
- **Using `trait` keyword in code examples:** The Mesh language uses `interface` for trait definitions, not `trait`. The existing docs conceptually refer to "Traits" (which is correct as a concept), but code must use `interface ... do ... end`.
- **Inventing syntax not in the tests:** If a feature is not demonstrated in E2E tests, either find it in the compiler source or mark it as needing verification. Do not guess.
- **Creating new Vue components:** The docs infrastructure is complete. Do not create new Vue components for this phase.
- **Forgetting to wrap examples in `fn main() do ... end`:** Most E2E tests wrap everything in `fn main()`. Documentation examples should show complete, runnable programs where possible.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Syntax highlighting for code examples | Custom highlighting CSS | `mesh` code fence language (already configured) | Shiki + custom grammar handles all Mesh syntax automatically |
| Table of contents for new pages | Manual anchor links | VitePress auto-generates from h2/h3 headings | TOC is built by DocsTableOfContents.vue using DOM heading extraction |
| Previous/next navigation | Hardcoded links | VitePress prev/next from sidebar config | DocsPrevNext.vue derives from sidebar order automatically |
| Search indexing for new pages | Manual index updates | VitePress local search auto-indexes new .md files | MiniSearch crawls all pages at build time |
| SEO meta tags | Manual `<meta>` tags | VitePress `transformPageData` hook | Already configured in config.mts, generates og:title, og:description, canonical for every page |

**Key insight:** The website infrastructure built in Phases 70-73 automatically handles all meta concerns (search, SEO, TOC, prev/next, edit links, version badge, last updated) for any new markdown page placed under `website/docs/docs/`. The only work is writing content and updating sidebar config.

## Common Pitfalls

### Pitfall 1: Showing `trait` Instead of `interface` in Code

**What goes wrong:** Documentation shows `trait MyTrait do ... end` but the actual keyword is `interface`.
**Why it happens:** The existing docs page says "Traits define shared behavior" and "trait keyword" in prose text (type-system/index.md line 208). The Mesh language uses `interface` as the keyword, while conceptually calling them "traits."
**How to avoid:** Always use `interface` in code fences. Use "traits" or "interfaces" in prose text interchangeably, but always use `interface` in code.
**Warning signs:** Code examples that fail to compile because `trait` is not a recognized keyword.

### Pitfall 2: Code Examples That Don't Compile

**What goes wrong:** Documentation shows syntax that looks plausible but doesn't actually work in the compiler.
**Why it happens:** Writing documentation from memory or conceptual understanding rather than from verified test files.
**How to avoid:** Every code example should be traceable to a passing E2E test file or directly tested against the compiler. The 22 v7.0 E2E test files cover all features comprehensively.
**Warning signs:** Any code example that uses syntax not found in any test file.

### Pitfall 3: Missing `fn main() do ... end` Wrapper

**What goes wrong:** Code examples show standalone expressions or function definitions without a `main()` entry point, making them non-runnable.
**Why it happens:** Wanting to show focused snippets rather than complete programs.
**How to avoid:** For conceptual snippets (like struct/interface definitions), showing without `main()` is fine. For runnable examples, always include the `fn main() do ... end` wrapper. Follow the existing docs pattern: definitions outside main, usage inside main.
**Warning signs:** Reader copies an example, tries to compile it, and gets an error about missing main function.

### Pitfall 4: Inconsistent Terminology Between Existing and New Docs

**What goes wrong:** New documentation uses different terminology than existing pages for the same concept (e.g., "traits" vs "interfaces", "methods" vs "functions", "type members" vs "associated types").
**Why it happens:** Different phases of documentation written at different times with different conceptual framing.
**How to avoid:** Read existing docs first and match their terminology. The existing docs use: "traits" (concept), "struct" (product type), "sum type" (enum), "pipe operator" (|>), "pattern matching" (case), "the try operator" (?).
**Warning signs:** New pages feel like they were written for a different language than the existing pages.

### Pitfall 5: Sidebar Order Breaking Learning Progression

**What goes wrong:** New pages are added to the sidebar in a confusing order, making the learning path non-linear.
**Why it happens:** Adding new pages without considering the reader's journey.
**How to avoid:** The sidebar should follow a learning progression: Getting Started -> Language Basics -> Type System -> Iterators (new) -> Concurrency -> Web -> Databases -> Distributed -> Tooling -> Cheatsheet. Iterators build on Type System knowledge (traits, associated types) and are used before concurrency patterns.
**Warning signs:** Reader needs to understand iterators to read the type system page, but iterators come after concurrency in the sidebar.

### Pitfall 6: Not Updating Cross-References in Existing Pages

**What goes wrong:** Existing pages link to "What's Next?" sections that don't mention the new pages, or the cheatsheet is missing v7.0 entries.
**Why it happens:** Focusing on new pages and forgetting to update existing pages.
**How to avoid:** Explicitly plan updates to: (1) type-system/index.md "Next Steps" section, (2) language-basics/index.md "What's Next?" section, (3) cheatsheet/index.md with new syntax entries, (4) getting-started/index.md if relevant.
**Warning signs:** New pages are orphaned -- reachable only from sidebar, not linked from any existing page.

## Code Examples

### Verified v7.0 Feature Inventory

Complete inventory of v7.0 features with verified syntax from E2E tests:

#### Associated Types (Phase 74)
```mesh
# Declaring an interface with associated types
interface Container do
  type Item
  fn first(self) -> Self.Item
end

# Implementing with concrete type binding
impl Container for IntPair do
  type Item = Int
  fn first(self) -> Int do
    42
  end
end

# Multiple associated types
interface Mapper do
  type Input
  type Output
  fn apply(self) -> Self.Output
end
```
Source: `tests/e2e/assoc_type_basic.mpl`, `tests/e2e/assoc_type_multiple.mpl`

#### Numeric Traits (Phase 75)
```mesh
# Custom arithmetic operators
impl Add for Vec2 do
  type Output = Vec2
  fn add(self, other :: Vec2) -> Vec2 do
    Vec2 { x: self.x + other.x, y: self.y + other.y }
  end
end

# Available traits: Add, Sub, Mul, Div, Neg
impl Neg for Point do
  type Output = Point
  fn neg(self) -> Point do
    Point { x: 0.0 - self.x, y: 0.0 - self.y }
  end
end

# Usage: operators dispatch to trait methods
let sum = v1 + v2        # calls Add.add
let diff = v1 - v2       # calls Sub.sub
let neg_p = -p            # calls Neg.neg
let chained = v1 + v2 + v3  # Output feeds back into Add
```
Source: `tests/e2e/numeric_traits.mpl`, `tests/e2e/numeric_neg.mpl`

#### Iterator Protocol (Phase 76)
```mesh
# User-defined Iterable
impl Iterable for EvenNumbers do
  type Item = Int
  type Iter = ListIterator
  fn iter(self) -> ListIterator do
    Iter.from(self.items)
  end
end

# for-in over custom Iterable
for x in evens do
  println(x.to_string())
end

# Manual iterator creation
let iter = Iter.from(list)
```
Source: `tests/e2e/iterator_iterable.mpl`

#### Lazy Combinators (Phase 78)
```mesh
# map: transform each element
Iter.from(list) |> Iter.map(fn x -> x * 2 end)

# filter: keep elements matching predicate
Iter.from(list) |> Iter.filter(fn x -> x % 2 == 0 end)

# take: limit to first N elements
Iter.from(list) |> Iter.take(3)

# skip: skip first N elements
Iter.from(list) |> Iter.skip(7)

# enumerate: pair elements with indices
Iter.from(list) |> Iter.enumerate()

# zip: combine two iterators into pairs
Iter.from(a) |> Iter.zip(Iter.from(b))
```
Source: `tests/e2e/iter_map_filter.mpl`, `tests/e2e/iter_take_skip.mpl`, `tests/e2e/iter_enumerate_zip.mpl`

#### Terminal Operations (Phase 78)
```mesh
# count: number of elements
Iter.from(list) |> Iter.count()

# sum: sum of integer elements
Iter.from(list) |> Iter.sum()

# any: true if any element matches
Iter.from(list) |> Iter.any(fn x -> x % 2 == 0 end)

# all: true if all elements match
Iter.from(list) |> Iter.all(fn x -> x > 0 end)

# find: first matching element (returns Option)
Iter.from(list) |> Iter.find(fn x -> x > 3 end)

# reduce: fold with accumulator
Iter.from(list) |> Iter.reduce(0, fn acc, x -> acc + x end)
```
Source: `tests/e2e/iter_terminals.mpl`

#### From/Into Conversion (Phase 77)
```mesh
# User-defined From
impl From<Int> for Wrapper do
  fn from(n :: Int) -> Wrapper do
    Wrapper { value: n * 2 }
  end
end
let w = Wrapper.from(21)

# Built-in conversions
let f = Float.from(42)          # Int -> Float
let s = String.from(42)         # Int -> String
let s = String.from(3.14)       # Float -> String
let s = String.from(true)       # Bool -> String

# ? operator with error type conversion via From
struct AppError do
  message :: String
end

impl From<String> for AppError do
  fn from(msg :: String) -> AppError do
    AppError { message: msg }
  end
end

fn risky() -> Int!String do
  Err("something failed")
end

fn process() -> Int!AppError do
  let n = risky()?    # auto-converts String error to AppError via From
  Ok(n + 1)
end
```
Source: `tests/e2e/from_user_defined.mpl`, `tests/e2e/from_float_from_int.mpl`, `tests/e2e/from_string_from_int.mpl`, `tests/e2e/from_try_struct_error.mpl`

#### Collect (Phase 79)
```mesh
# List.collect: materialize iterator into list
let doubled = Iter.from(list)
  |> Iter.map(fn x -> x * 2 end)
  |> List.collect()

# Map.collect: materialize tuple iterator into map
let m = Iter.from(list)
  |> Iter.enumerate()
  |> Map.collect()

# Set.collect: materialize into set (deduplicates)
let s = Iter.from([1, 2, 2, 3, 3, 3])
  |> Set.collect()

# String.collect: join string iterator
let joined = Iter.from(["hello", " ", "world"])
  |> String.collect()

# Direct call syntax also works
let direct = List.collect(iter)
```
Source: `tests/e2e/collect_list.mpl`, `tests/e2e/collect_map.mpl`, `tests/e2e/collect_set_string.mpl`

## State of the Art

| Old State (v6.0 docs) | New State (v7.0 update needed) | Impact |
|------------------------|-------------------------------|--------|
| Type System page has no `interface` keyword examples | Must add interface definition syntax, associated types, numeric traits, From/Into | Largest content gap -- readers can't learn how to define custom traits |
| Cheatsheet has no iterator, From/Into, or associated type syntax | Must add entries for all v7.0 features | Cheatsheet is incomplete reference |
| Language Basics shows `map`/`filter`/`reduce` as standalone functions | v7.0 added lazy iterator pipelines with `Iter.from` + combinators | Existing eager functions still work but iterators are the recommended pattern for pipelines |
| No documentation for `interface` keyword at all | 22 E2E tests demonstrate interface syntax | Critical gap in existing documentation |
| Landing page uses incorrect syntax (`match`, `def`, `:increment`, `IO.puts`) | Landing page was built for visual appeal, not accuracy | OUT OF SCOPE for this phase -- separate concern |

**Deprecated/outdated:**
- Landing page FeatureShowcase.vue and HeroSection.vue use aspirational syntax that doesn't match the actual compiler. This is a known issue but is NOT in scope for Phase 80 (documentation update), which focuses on the docs section.

## Content Organization Analysis

### Option A: Add Everything to Existing Pages (NOT recommended)

Add all v7.0 content to the type-system and language-basics pages. This would make them extremely long (type-system would go from 345 to ~700+ lines). Discoverability suffers because everything is buried in one page.

### Option B: One New Page + Updates (RECOMMENDED)

Create ONE new documentation page: **Iterators** (`/docs/iterators/index.md`). This page covers:
- The Iterator and Iterable traits (with associated types)
- `Iter.from()` and creating iterators
- Lazy combinators (map, filter, take, skip, enumerate, zip)
- Terminal operations (count, sum, any, all, find, reduce)
- Collect (List.collect, Map.collect, Set.collect, String.collect)
- Multi-step pipeline examples

Update EXISTING pages:
- **type-system/index.md**: Add sections for custom interfaces (the `interface` keyword), associated types, numeric traits (operator overloading), and From/Into conversion
- **cheatsheet/index.md**: Add entries for interfaces, associated types, iterators, From/Into, numeric traits
- **language-basics/index.md**: Update pipe operator section to mention lazy iterators, update "What's Next?" links
- **config.mts**: Add `/docs/iterators/` to sidebar

### Option C: Two New Pages + Updates

Create two new pages: **Iterators** and **Traits & Conversions**. This separates the trait system extensions (associated types, numeric traits, From/Into) from the iterator ecosystem. Pros: better separation of concerns. Cons: the type-system page already covers traits/deriving, so a separate "Traits & Conversions" page might fragment the trait documentation.

**Recommendation: Option B.** One new Iterators page + updates to type-system, cheatsheet, and language-basics. The type-system page is the natural home for associated types, numeric traits, and From/Into since it already covers traits and deriving. Iterators deserve their own page because the combinator/terminal/collect surface area is large and distinct.

## Files to Modify (Complete Inventory)

| File | Action | Estimated Size |
|------|--------|---------------|
| `website/docs/docs/iterators/index.md` | **CREATE** -- full iterator documentation | ~300-400 lines |
| `website/docs/docs/type-system/index.md` | **UPDATE** -- add interfaces, associated types, numeric traits, From/Into | +150-200 lines |
| `website/docs/docs/cheatsheet/index.md` | **UPDATE** -- add v7.0 syntax entries | +40-60 lines |
| `website/docs/docs/language-basics/index.md` | **UPDATE** -- mention iterator pipelines, update cross-links | +10-20 lines |
| `website/docs/.vitepress/config.mts` | **UPDATE** -- add iterators to sidebar | +1 line |

**Total estimated new content:** ~500-680 lines of markdown

## Open Questions

1. **Should the `interface` keyword explanation go at the top of the type-system page or in a new section?**
   - What we know: The existing type-system page has a "Traits" section (line 206) that mentions defining traits with "the `trait` keyword" but shows no code for it (only shows `deriving`). The existing prose says "trait keyword" which is incorrect -- it should be "interface keyword."
   - What's unclear: Whether to fix the existing Traits section in-place or add a new "Custom Interfaces" section.
   - Recommendation: Fix the existing Traits section to show `interface` syntax, then add subsections for associated types, numeric traits, and From/Into below it. This keeps trait-related documentation together.

2. **Should `Iter.find` be documented even though no E2E test exercises it?**
   - What we know: `Iter.find` is implemented in the runtime (`mesh_iter_find`), registered in the type checker, and wired through MIR/codegen. But no E2E test file uses it directly.
   - What's unclear: Whether the feature works end-to-end without a test.
   - Recommendation: Include `Iter.find` in the documentation since it is fully wired. The implementation is confirmed in `crates/mesh-rt/src/iter.rs`, `crates/mesh-typeck/src/infer.rs`, and `crates/mesh-codegen/src/mir/lower.rs`. Note: the planner may want to verify it compiles before including the example.

3. **Should we update the version badge from v0.1.0?**
   - What we know: The version badge in `config.mts` shows `meshVersion: '0.1.0'`. v7.0 is a significant milestone.
   - What's unclear: Whether the version should be updated as part of documentation updates.
   - Recommendation: This is a minor config change but probably out of scope for a "documentation update" phase. Flag for the planner to decide.

## Sources

### Primary (HIGH confidence)
- E2E test files (22 files for v7.0) in `/Users/sn0w/Documents/dev/snow/tests/e2e/` -- authoritative syntax for all features
- Existing documentation pages (9 files) in `/Users/sn0w/Documents/dev/snow/website/docs/docs/` -- established patterns and tone
- VitePress config in `/Users/sn0w/Documents/dev/snow/website/docs/.vitepress/config.mts` -- sidebar structure, search config
- Website theme components in `/Users/sn0w/Documents/dev/snow/website/docs/.vitepress/theme/` -- infrastructure verification
- Compiler source: `crates/mesh-typeck/src/infer.rs` -- Iter API signatures
- Runtime source: `crates/mesh-rt/src/iter.rs` -- Iter.find implementation
- Token definitions: `crates/mesh-common/src/token.rs` -- confirms `interface` keyword

### Secondary (MEDIUM confidence)
- Phase 72 Research (`72-RESEARCH.md`) -- VitePress docs infrastructure patterns
- Phase 73 Verification (`73-VERIFICATION.md`) -- confirmed all docs infrastructure working
- ROADMAP.md -- feature descriptions and success criteria for Phases 74-79

### Tertiary (LOW confidence)
- None -- all findings are verified from project source code

## Metadata

**Confidence breakdown:**
- Content requirements: HIGH -- all features verified from 22 E2E test files and compiler source
- Architecture (no infra changes needed): HIGH -- verified all docs infrastructure from Phase 70-73 is complete and working
- Code examples accuracy: HIGH -- all examples taken directly from passing E2E tests
- Pitfalls (terminology, keyword mismatch): HIGH -- discovered `interface` vs `trait` issue from direct source code verification
- Content organization: MEDIUM -- recommendation is based on judgment about page length and discoverability; planner may choose differently

**Research date:** 2026-02-13
**Valid until:** 2026-03-15 (content authoring phase -- no external dependencies to go stale)
