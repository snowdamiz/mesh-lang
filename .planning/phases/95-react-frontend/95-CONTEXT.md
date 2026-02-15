# Phase 95: React Frontend - Context

**Gathered:** 2026-02-15
**Status:** Ready for planning

<domain>
## Phase Boundary

Build a React 19 SPA frontend for the Mesher monitoring platform. All backend APIs (REST, WebSocket, ingestion) already exist from Phases 87-94. This phase is purely frontend -- no backend changes. Users interact with dashboards, event browsing, issue management, alerting, and real-time streaming through the browser.

</domain>

<decisions>
## Implementation Decisions

### Dashboard layout
- Split view: left panel for charts/stats, right panel for live issue list -- equal weight to both
- Three chart types: event volume over time, error level breakdown, and health/unresolved trend
- Issue list uses compact single-line rows (title, count, last seen) -- Linear-style density
- Project switcher lives in the sidebar as a persistent list of projects (always visible, no dropdown)

### Navigation & page structure
- Persistent left sidebar with navigation items
- 6 pages: Dashboard, Issues, Events, Live Stream, Alerts, Settings
- Event/issue detail renders as a push panel from the right -- content area compresses to make room (master-detail pattern, not overlay)
- Projects listed in sidebar section above nav items

### Real-time streaming UX
- Dashboard charts and issue counts auto-update as events arrive via WebSocket -- always current
- Dedicated Live Stream page shows events as compact cards, newest first, stacking from top
- Live Stream has full filter bar (search, tags, level, environment) matching the Events page filters
- WebSocket connection status shown as a small green/red dot in the header area -- always visible, subtle

### Visual style & density
- Dark and light themes with a toggle
- Linear-inspired aesthetic: clean, minimal, lots of whitespace, subtle borders, monochrome
- No accent color -- monochrome palette consistent with the existing Mesh landing page
- Only color variation comes from semantic status colors (red errors, yellow warnings, green healthy)
- Shadcn (React) for component library -- Tailwind-based, copy-paste components

### Tech stack
- React 19 with TypeScript
- Vite for build tooling
- Tailwind CSS for styling
- Shadcn/ui for component library
- Recharts for charts (replacing ECharts from original roadmap)
- TanStack Table for issue/event lists
- State management: Claude's discretion based on app complexity

### Claude's Discretion
- URL routing strategy (full routing vs minimal for detail panels)
- State management solution (Zustand, Jotai, React context, etc.)
- Loading skeleton design and error state handling
- Exact spacing, typography, and responsive breakpoints
- WebSocket reconnection strategy and retry logic
- Chart time range selectors and aggregation controls

</decisions>

<specifics>
## Specific Ideas

- Linear is the primary visual reference -- clean, not cluttered, high information density with whitespace balance
- Monochrome aesthetic matching the existing Mesh landing page (website/)
- Push panel for detail views (not overlay/drawer) -- main content shifts and becomes narrower
- Live Stream page should feel like a real-time event tail with card-based rendering
- Status colors are the only non-monochrome elements in the UI

</specifics>

<deferred>
## Deferred Ideas

None -- discussion stayed within phase scope

</deferred>

---

*Phase: 95-react-frontend*
*Context gathered: 2026-02-15*
