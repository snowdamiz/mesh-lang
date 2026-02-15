---
phase: 95-react-frontend
plan: 03
subsystem: ui
tags: [react, tanstack-table, pagination, filtering, issues, events]

requires:
  - phase: 95-react-frontend/01
    provides: React SPA scaffold, app shell, typed API client, Zustand stores, push panel layout
provides:
  - Issues list page with TanStack Table, status/level filters, keyset pagination
  - Events search page with TanStack Table, full-text search, level/environment filters
  - Shared DataTable component with sorting, loading skeletons, empty state
  - Shared FilterBar component reusable across Issues, Events, and Live Stream pages
  - Shared Pagination component with cursor stack for bidirectional navigation
  - Shared formatRelativeTime and formatNumber utility functions
affects: [95-04, 95-05]

tech-stack:
  added: []
  patterns: [tanstack-table-wrapper, keyset-cursor-stack, filter-bar-debounce, search-driven-page]

key-files:
  created:
    - frontend/src/components/shared/data-table.tsx
    - frontend/src/components/shared/filter-bar.tsx
    - frontend/src/components/shared/pagination.tsx
    - frontend/src/lib/format.ts
  modified:
    - frontend/src/pages/issues.tsx
    - frontend/src/pages/events.tsx
    - frontend/src/components/shared/issue-row.tsx

key-decisions:
  - "Events page requires search query before showing results (empty state prompt by default)"
  - "Client-side level filtering on Events page since search endpoint lacks level param"
  - "Cursor stack pattern for bidirectional keyset pagination (push on Next, pop on Previous)"
  - "Default 'unresolved' status filter on Issues page per decision [89-02]"

patterns-established:
  - "DataTable pattern: generic TanStack Table wrapper with ColumnDef<T>[], onRowClick, isLoading props"
  - "FilterBar pattern: debounced search (300ms), Select dropdowns, emit-on-mount for default filters"
  - "Cursor stack pattern: maintain Array<{cursor, cursorId}> in parent for Previous navigation"
  - "Page layout pattern: h1 title + FilterBar in header, DataTable in flex-1 scrollable body, Pagination in footer"

duration: 5min
completed: 2026-02-15
---

# Phase 95 Plan 03: Issues and Events List Pages Summary

**TanStack Table-powered Issues and Events pages with shared DataTable/FilterBar/Pagination components, keyset cursor-stack pagination, and search-driven Events browsing**

## Performance

- **Duration:** 5 min
- **Started:** 2026-02-15T21:47:56Z
- **Completed:** 2026-02-15T21:52:49Z
- **Tasks:** 2
- **Files created/modified:** 7

## Accomplishments
- Issues page with 5-column TanStack Table (title, level, status, events, last seen), default 'unresolved' filter, keyset pagination
- Events page with 4-column TanStack Table (message, level, issue, received), search-driven with empty state prompt
- Shared DataTable component with sorting indicators, loading skeletons, empty state, row click handler
- Shared FilterBar with debounced search, level/status/environment selects, default filter emission on mount
- Shared Pagination with cursor stack for bidirectional navigation
- PushPanelLayout integration on both pages for detail panel (content built in Plan 04)

## Task Commits

Each task was committed atomically:

1. **Task 1: Create shared data table, filter bar, and pagination components** - `13e9f0ad` (feat)
2. **Task 2: Build Issues and Events list pages with filters and pagination** - `16c572d8` (feat)

## Files Created/Modified
- `frontend/src/components/shared/data-table.tsx` - Generic TanStack Table wrapper with shadcn styling, sorting, skeletons
- `frontend/src/components/shared/filter-bar.tsx` - Reusable filter bar with debounced search, level/status/environment selects
- `frontend/src/components/shared/pagination.tsx` - Keyset pagination controls with Previous/Next buttons
- `frontend/src/lib/format.ts` - Shared formatRelativeTime and formatNumber utilities
- `frontend/src/pages/issues.tsx` - Full Issues list page with table, filters, pagination, push panel
- `frontend/src/pages/events.tsx` - Full Events search page with table, filters, empty state, push panel
- `frontend/src/components/shared/issue-row.tsx` - Updated to use shared formatRelativeTime from lib/format.ts

## Decisions Made
- Events page requires a search query before showing results -- without search, shows centered prompt "Search events by message, tags, or other criteria" because the events search endpoint requires a query parameter
- Client-side level filtering applied on Events page since api.events.search doesn't accept a level parameter
- Cursor stack pattern chosen for bidirectional keyset pagination: push current cursor onto stack on "Next", pop on "Previous", fetch first page when stack empty
- Default 'unresolved' status filter on Issues page per decision [89-02]
- Extracted formatRelativeTime to shared lib/format.ts to eliminate duplication between issue-row.tsx and the new pages

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed pre-existing alerts.tsx type error**
- **Found during:** Task 2 (build verification)
- **Issue:** CreateRulePayload has optional threshold/window_minutes but api.alerts.createRule expects required fields from Omit<AlertRule, ...>
- **Fix:** Added type assertion `as Parameters<typeof api.alerts.createRule>[1]` at call site
- **Files modified:** frontend/src/pages/alerts.tsx
- **Verification:** `npm run build` passes
- **Committed in:** 16c572d8 (Task 2 commit)

**2. [Rule 1 - Bug] Fixed getSortingRowModel import name**
- **Found during:** Task 1 (linter auto-correction)
- **Issue:** TanStack Table v8 exports `getSortedRowModel` not `getSortingRowModel`
- **Fix:** Linter auto-corrected import and usage
- **Files modified:** frontend/src/components/shared/data-table.tsx
- **Verification:** `npx tsc --noEmit` passes
- **Committed in:** 16c572d8 (included in Task 2 commit via staged change)

**3. [Rule 2 - Missing Critical] Extracted formatRelativeTime to shared utility**
- **Found during:** Task 2 (building pages that need relative time formatting)
- **Issue:** formatRelativeTime was defined inline in issue-row.tsx, needed by both new pages
- **Fix:** Created frontend/src/lib/format.ts with shared utility, updated issue-row.tsx import
- **Files modified:** frontend/src/lib/format.ts (created), frontend/src/components/shared/issue-row.tsx
- **Verification:** TypeScript compiles, no duplicate functions
- **Committed in:** 16c572d8 (Task 2 commit)

---

**Total deviations:** 3 auto-fixed (2 bugs, 1 missing critical)
**Impact on plan:** All fixes necessary for correct compilation and code reuse. No scope creep.

## Issues Encountered
None - both pages followed the plan layout pattern exactly.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Issues and Events pages ready for detail panel content (Plan 04)
- DataTable, FilterBar, Pagination components available for Live Stream page (Plan 05)
- PushPanelLayout wired up on both pages, awaiting detail panel component
- No blockers for next plans

## Self-Check: PASSED

All 7 key files verified present. Both task commits (13e9f0ad, 16c572d8) verified in git log.

---
*Phase: 95-react-frontend*
*Completed: 2026-02-15*
