---
phase: 70-scaffold-design-system
plan: 01
subsystem: ui
tags: [vitepress, tailwindcss-v4, shadcn-vue, oklch, dark-mode, vue3]

# Dependency graph
requires: []
provides:
  - VitePress site scaffold with custom theme (no default theme)
  - Tailwind CSS v4 with monochrome OKLCH palette and @theme inline bridge
  - shadcn-vue Button, DropdownMenu, Sheet components
  - FOUC-free dark mode via VitePress appearance:true
  - cn() utility for Tailwind class merging
affects: [70-02, 80-landing-page, 90-docs-content]

# Tech tracking
tech-stack:
  added: [vitepress 1.6.4, vue 3.5.28, tailwindcss 4.1.18, @tailwindcss/vite 4.1.18, shadcn-vue (button/dropdown-menu/sheet), reka-ui 2.8.0, @vueuse/core 14.2.1, lucide-vue-next 0.564.0, class-variance-authority 0.7.1, clsx 2.1.1, tailwind-merge 3.4.0, tw-animate-css 1.4.0]
  patterns: [VitePress custom theme (blank Layout.vue), @theme inline CSS variable bridge, @custom-variant dark for class-based dark mode, source("../..") for Tailwind content scanning]

key-files:
  created:
    - website/package.json
    - website/tsconfig.json
    - website/components.json
    - website/docs/index.md
    - website/docs/.vitepress/config.mts
    - website/docs/.vitepress/theme/index.ts
    - website/docs/.vitepress/theme/Layout.vue
    - website/docs/.vitepress/theme/styles/main.css
    - website/docs/.vitepress/theme/lib/utils.ts
    - website/docs/.vitepress/theme/components/ui/button/
    - website/docs/.vitepress/theme/components/ui/dropdown-menu/
    - website/docs/.vitepress/theme/components/ui/sheet/
  modified:
    - .gitignore

key-decisions:
  - "Placed .vitepress/ inside docs/ (VitePress source root) instead of website/ root -- VitePress resolves theme relative to source directory"
  - "Used config.mts instead of config.ts for ESM compatibility with VitePress + @tailwindcss/vite"
  - "Used shadcn-vue neutral base color for zero-chroma monochrome OKLCH palette"

patterns-established:
  - "VitePress custom theme: blank Layout.vue + theme/index.ts satisfies Theme, no default theme import"
  - "@theme inline bridge: CSS variables in :root/.dark + @theme inline mapping to Tailwind --color-* namespace"
  - "@custom-variant dark (&:where(.dark, .dark *)): class-based dark mode for VitePress"
  - "source(\"../..\"): Tailwind v4 content scanning for .vitepress dotfile directory"
  - "shadcn-vue @/ alias: tsconfig paths + Vite resolve.alias + components.json aliases all map to docs/.vitepress/theme/"

# Metrics
duration: 7min
completed: 2026-02-13
---

# Phase 70 Plan 01: VitePress + Tailwind v4 + shadcn-vue Scaffold Summary

**VitePress custom theme with Tailwind CSS v4 monochrome OKLCH palette, shadcn-vue components, and FOUC-free dark mode**

## Performance

- **Duration:** 7 min
- **Started:** 2026-02-13T18:46:19Z
- **Completed:** 2026-02-13T18:53:36Z
- **Tasks:** 2
- **Files modified:** 38

## Accomplishments
- Scaffolded VitePress site in website/ with fully custom theme (no default theme leakage)
- Integrated Tailwind CSS v4 with monochrome OKLCH palette via @theme inline bridge
- Installed shadcn-vue Button, DropdownMenu, and Sheet components with correct @/ alias resolution
- Verified FOUC prevention works via VitePress appearance:true inline script
- Production build succeeds with Tailwind utilities correctly included

## Task Commits

Each task was committed atomically:

1. **Task 1: Scaffold VitePress project with npm dependencies** - `d804cd38` (feat)
2. **Task 2: Create VitePress custom theme with Tailwind v4 + shadcn-vue foundation** - `bb9315c9` (feat)

## Files Created/Modified
- `website/package.json` - Project manifest with VitePress, Tailwind v4, shadcn-vue deps
- `website/tsconfig.json` - TypeScript config with @/ path alias to docs/.vitepress/theme/
- `website/components.json` - shadcn-vue config with neutral base color and correct aliases
- `website/docs/index.md` - Placeholder page for the Mesh documentation
- `website/docs/.vitepress/config.mts` - VitePress config with Tailwind plugin, appearance:true
- `website/docs/.vitepress/theme/index.ts` - Custom theme entry (no default theme import)
- `website/docs/.vitepress/theme/Layout.vue` - Minimal root layout with Content component
- `website/docs/.vitepress/theme/styles/main.css` - Tailwind imports + OKLCH variables + @theme inline
- `website/docs/.vitepress/theme/lib/utils.ts` - cn() helper for shadcn-vue class merging
- `website/docs/.vitepress/theme/components/ui/button/` - shadcn-vue Button component
- `website/docs/.vitepress/theme/components/ui/dropdown-menu/` - shadcn-vue DropdownMenu component
- `website/docs/.vitepress/theme/components/ui/sheet/` - shadcn-vue Sheet component
- `.gitignore` - Added website build artifact exclusions

## Decisions Made
- **Placed .vitepress/ inside docs/:** VitePress resolves the theme directory relative to the source root (the directory passed to `vitepress dev docs`). Placing .vitepress at website/ root caused VitePress to use the default theme instead of the custom one.
- **Used config.mts extension:** VitePress's config bundler uses esbuild which respects the package.json `type` field. With `"type": "commonjs"`, `.ts` files are treated as CJS and can't import ESM-only packages (vitepress, @tailwindcss/vite). Using `.mts` forces ESM treatment regardless of package.json.
- **shadcn-vue neutral base color:** Already uses zero-chroma OKLCH values, providing a true monochrome palette without custom color engineering.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Moved .vitepress/ from website/ root to website/docs/**
- **Found during:** Task 2 (VitePress custom theme creation)
- **Issue:** Plan specified files at `website/.vitepress/` but VitePress resolves the theme directory relative to the source root. Since scripts use `vitepress dev docs`, VitePress looked for `.vitepress/` inside `docs/`, not at the website root. The custom theme was not loaded and the default VitePress theme rendered instead.
- **Fix:** Moved all .vitepress/ contents into docs/.vitepress/. Updated tsconfig.json paths, components.json css path, and .gitignore accordingly.
- **Files modified:** All .vitepress/ files, tsconfig.json, components.json, .gitignore
- **Verification:** Production build produces HTML with custom Layout.vue markup, no VPNav/VPDoc classes
- **Committed in:** bb9315c9

**2. [Rule 3 - Blocking] Renamed config.ts to config.mts for ESM compatibility**
- **Found during:** Task 2 (production build test)
- **Issue:** `npm run build` failed with "ESM file cannot be loaded by require" because package.json has `"type": "commonjs"` and both vitepress and @tailwindcss/vite are ESM-only packages. The `.ts` extension was treated as CJS by esbuild.
- **Fix:** Renamed config.ts to config.mts which forces ESM module treatment regardless of package.json type field.
- **Files modified:** website/docs/.vitepress/config.mts (renamed from config.ts)
- **Verification:** `npm run build` succeeds without errors
- **Committed in:** bb9315c9

---

**Total deviations:** 2 auto-fixed (1 bug, 1 blocking)
**Impact on plan:** Both fixes necessary for the project to function. File paths differ from plan but all functionality is identical. No scope creep.

## Issues Encountered
- shadcn-vue CLI added `reka-ui` as a dependency automatically (expected -- it's the headless UI library underlying DropdownMenu and Sheet)

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- VitePress scaffold is fully operational with custom theme
- Tailwind CSS v4 with OKLCH palette renders correctly in both dev and production
- shadcn-vue components are installed and importable via @/ alias
- Ready for Plan 02 (NavBar + ThemeToggle + dark mode toggle)

---
## Self-Check: PASSED

All 13 key files verified present. Both task commits (d804cd38, bb9315c9) verified in git log.

---
*Phase: 70-scaffold-design-system*
*Completed: 2026-02-13*
