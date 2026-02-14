---
phase: 86-documentation-corrections
plan: 01
subsystem: website
tags: [vitepress, documentation, landing-page, getting-started]

# Dependency graph
requires:
  - phase: 82-install-script
    provides: "curl installer at mesh-lang.org/install.sh"
provides:
  - "Corrected getting-started guide with working install, binary name, description, and project-based examples"
  - "Dynamic version badge on landing page reading from config.mts"
  - "Fixed install command URL in landing page CTA"
affects: []

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Single source of truth for version via themeConfig.meshVersion in config.mts"

key-files:
  created: []
  modified:
    - website/docs/.vitepress/config.mts
    - website/docs/.vitepress/theme/components/landing/HeroSection.vue
    - website/docs/.vitepress/theme/components/landing/GetStartedCTA.vue
    - website/docs/docs/getting-started/index.md

key-decisions:
  - "Version badge uses dynamic useData().theme.meshVersion pattern from DocsVersionBadge.vue rather than hardcoded string"

patterns-established:
  - "Landing page version badge reads from themeConfig -- future version bumps only require changing config.mts"

# Metrics
duration: 1min
completed: 2026-02-14
---

# Phase 86 Plan 01: Documentation Corrections Summary

**Fixed 5 documentation inaccuracies: meshc binary name, curl installer URL, v7.0 version badge, LLVM compilation description, and project-based code examples**

## Performance

- **Duration:** 1 min
- **Started:** 2026-02-14T18:29:17Z
- **Completed:** 2026-02-14T18:30:46Z
- **Tasks:** 2
- **Files modified:** 4

## Accomplishments
- Fixed landing page version badge from hardcoded v0.1.0 to dynamic v7.0 via themeConfig
- Fixed install command URL to include .sh extension in GetStartedCTA
- Rewrote getting-started guide: replaced build-from-source with curl installer, corrected binary name to meshc, changed compilation description from "via Rust" to "via LLVM", converted all examples to project-based workflow (meshc init / meshc build)

## Task Commits

Each task was committed atomically:

1. **Task 1: Fix landing page version badge, install command, and config** - `c48c9e25` (fix)
2. **Task 2: Rewrite getting-started guide with correct install, binary name, description, and working examples** - `43476964` (fix)

## Files Created/Modified
- `website/docs/.vitepress/config.mts` - Updated meshVersion from '0.1.0' to '7.0'
- `website/docs/.vitepress/theme/components/landing/HeroSection.vue` - Dynamic version badge via useData()
- `website/docs/.vitepress/theme/components/landing/GetStartedCTA.vue` - Fixed install.sh URL
- `website/docs/docs/getting-started/index.md` - Rewrote install section, binary names, compilation description, code examples

## Decisions Made
- Used dynamic useData().theme.meshVersion pattern (matching existing DocsVersionBadge.vue) instead of simply replacing the hardcoded string with another hardcoded string -- ensures future version bumps only need one config change

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered
None

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- All 5 DOCS requirements satisfied (DOCS-01 through DOCS-05)
- Website documentation now matches actual CLI behavior
- No blockers for future documentation work

## Self-Check: PASSED

- All 4 modified files exist on disk
- Commit c48c9e25 (Task 1) found in git log
- Commit 43476964 (Task 2) found in git log

---
*Phase: 86-documentation-corrections*
*Completed: 2026-02-14*
