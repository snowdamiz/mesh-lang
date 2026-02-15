# Phase 95: React Frontend - Research

**Researched:** 2026-02-15
**Domain:** React 19 SPA for monitoring platform (dashboard, events, issues, alerts, streaming)
**Confidence:** HIGH

<user_constraints>
## User Constraints (from CONTEXT.md)

### Locked Decisions

#### Dashboard layout
- Split view: left panel for charts/stats, right panel for live issue list -- equal weight to both
- Three chart types: event volume over time, error level breakdown, and health/unresolved trend
- Issue list uses compact single-line rows (title, count, last seen) -- Linear-style density
- Project switcher lives in the sidebar as a persistent list of projects (always visible, no dropdown)

#### Navigation & page structure
- Persistent left sidebar with navigation items
- 6 pages: Dashboard, Issues, Events, Live Stream, Alerts, Settings
- Event/issue detail renders as a push panel from the right -- content area compresses to make room (master-detail pattern, not overlay)
- Projects listed in sidebar section above nav items

#### Real-time streaming UX
- Dashboard charts and issue counts auto-update as events arrive via WebSocket -- always current
- Dedicated Live Stream page shows events as compact cards, newest first, stacking from top
- Live Stream has full filter bar (search, tags, level, environment) matching the Events page filters
- WebSocket connection status shown as a small green/red dot in the header area -- always visible, subtle

#### Visual style & density
- Dark and light themes with a toggle
- Linear-inspired aesthetic: clean, minimal, lots of whitespace, subtle borders, monochrome
- No accent color -- monochrome palette consistent with the existing Mesh landing page
- Only color variation comes from semantic status colors (red errors, yellow warnings, green healthy)
- Shadcn (React) for component library -- Tailwind-based, copy-paste components

#### Tech stack
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

### Deferred Ideas (OUT OF SCOPE)
None -- discussion stayed within phase scope
</user_constraints>

## Summary

This phase builds a React 19 SPA that consumes the existing Mesher monitoring platform REST API (HTTP on port 8080) and WebSocket streaming API (WS on port 8081). The backend exposes 30+ HTTP endpoints for dashboard aggregation, issue management, event search, alert rules, team management, settings, and storage visibility. WebSocket streaming provides real-time event notifications, issue count updates, and alert notifications via room-based broadcasting (`project:{id}`).

The frontend stack is locked: React 19.2.x + Vite 7.x + Tailwind CSS v4 + shadcn/ui + Recharts 3.x + TanStack Table 8.x. The visual design follows a Linear-inspired monochrome aesthetic matching the existing Mesh landing page, which already defines a complete set of CSS custom properties (oklch color space) for both light and dark themes using the shadcn "neutral" base color with "new-york" style. The existing website uses Inter for body text and JetBrains Mono for code, which should carry over to the frontend.

The most significant architectural decisions are: (1) use Zustand for global state management given the moderate complexity and WebSocket-driven updates, (2) use React Router v7 in library mode for full SPA routing, (3) implement a custom WebSocket hook with exponential backoff rather than depending on `react-use-websocket` which lacks official React 19 support, and (4) use Vite's proxy to handle CORS during development since the backend has no CORS headers.

**Primary recommendation:** Scaffold with `npm create vite@latest` + `npx shadcn@latest init` using neutral base color, copy the existing website's CSS variables verbatim for theme consistency, and organize by feature (dashboard/, issues/, events/, etc.) with shared hooks and API client layers.

## Standard Stack

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| react | 19.2.x | UI framework | Locked decision, latest stable |
| react-dom | 19.2.x | DOM rendering | Paired with React |
| typescript | ~5.9.x | Type safety | Current stable, Vite template default |
| vite | 7.x | Build tool + dev server | Locked decision, fastest DX |
| @vitejs/plugin-react | latest | React Fast Refresh for Vite | Official Vite React plugin |
| tailwindcss | 4.x | Utility-first CSS | Locked decision, v4 with Vite plugin |
| @tailwindcss/vite | latest | Tailwind Vite integration | v4 recommended approach (no PostCSS) |

### UI Components
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| shadcn/ui (CLI) | latest | Copy-paste component library | Locked decision, Tailwind-based, full ownership |
| @radix-ui/* | (via shadcn) | Accessible primitives | Underlying shadcn dependency |
| lucide-react | latest | Icon library | Default shadcn icon library |
| class-variance-authority | latest | Component variant management | shadcn dependency |
| clsx + tailwind-merge | latest | Class name utilities | shadcn `cn()` helper dependencies |

### Data & Charts
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| recharts | 3.7.x | Time-series charts, bar charts | Locked decision, React 19 compatible |
| @tanstack/react-table | 8.21.x | Headless table with sort/filter/pagination | Locked decision, pairs with shadcn data-table |

### State & Routing
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| zustand | 5.x | Global state management | Recommended: simple, no provider, works with WS |
| react-router | 7.x | Client-side routing | Mature, SPA library mode, React 19 compatible |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| Zustand | Jotai | Jotai is better for many independent atoms; Zustand is better for centralized stores (dashboard state, WS state). Zustand wins here because the app has a few well-defined stores, not many atoms |
| Zustand | React Context | Context causes re-renders on any change; Zustand has selector-based subscriptions. Context is insufficient for WS-driven frequent updates |
| react-router | TanStack Router | TanStack Router is type-safe but v1 is newer. React Router v7 is more battle-tested for SPAs |
| Custom WS hook | react-use-websocket | react-use-websocket lacks official React 19 support (last publish 1yr ago, open issue #256). Custom hook is safer and simpler for this use case |

**Installation:**
```bash
# Create project
npm create vite@latest frontend -- --template react-ts
cd frontend

# Core dependencies
npm install tailwindcss @tailwindcss/vite
npm install recharts @tanstack/react-table
npm install zustand react-router

# Initialize shadcn/ui
npx shadcn@latest init
# Select: New York style, Neutral base color, CSS variables: yes

# Add shadcn components (progressive -- add as needed)
npx shadcn@latest add button card table sidebar badge input select tabs dialog dropdown-menu separator scroll-area tooltip toggle sheet skeleton
```

## Architecture Patterns

### Recommended Project Structure
```
frontend/
├── index.html
├── vite.config.ts
├── tsconfig.json
├── tsconfig.app.json
├── components.json              # shadcn config
├── src/
│   ├── main.tsx                 # Entry point, router mount
│   ├── app.tsx                  # Root layout (sidebar + content area)
│   ├── globals.css              # Tailwind import + CSS variables (from website)
│   ├── lib/
│   │   ├── utils.ts             # cn() helper (shadcn generated)
│   │   └── api.ts               # Fetch wrapper for REST API
│   ├── hooks/
│   │   ├── use-websocket.ts     # Custom WS hook with reconnection
│   │   ├── use-api.ts           # Data fetching hooks (SWR-like with Zustand)
│   │   └── use-theme.ts         # Dark/light theme toggle
│   ├── stores/
│   │   ├── project-store.ts     # Active project, project list
│   │   ├── ws-store.ts          # WebSocket connection state, messages
│   │   └── ui-store.ts          # Sidebar state, detail panel state
│   ├── components/
│   │   ├── ui/                  # shadcn generated components
│   │   ├── layout/
│   │   │   ├── app-sidebar.tsx  # Project list + nav items
│   │   │   ├── header.tsx       # WS status dot, theme toggle
│   │   │   └── push-panel.tsx   # Right-side detail push panel
│   │   ├── charts/
│   │   │   ├── volume-chart.tsx # Event volume over time (Area/Line)
│   │   │   ├── level-chart.tsx  # Error level breakdown (Bar/Pie)
│   │   │   └── health-chart.tsx # Health/unresolved trend (Line)
│   │   └── shared/
│   │       ├── filter-bar.tsx   # Reusable filter bar (level, env, search, tags)
│   │       ├── issue-row.tsx    # Compact single-line issue row
│   │       ├── event-card.tsx   # Compact event card for Live Stream
│   │       └── status-badge.tsx # Semantic color badges
│   ├── pages/
│   │   ├── dashboard.tsx        # Split view: charts left, issues right
│   │   ├── issues.tsx           # TanStack Table with filters + push panel
│   │   ├── events.tsx           # TanStack Table with search + push panel
│   │   ├── live-stream.tsx      # Real-time event cards
│   │   ├── alerts.tsx           # Alert rules CRUD + fired alerts list
│   │   └── settings.tsx         # Project settings, API keys, team, storage
│   └── types/
│       └── api.ts               # TypeScript types matching backend JSON shapes
```

### Pattern 1: Zustand Store with WebSocket Integration
**What:** Centralized store that receives WebSocket messages and updates UI state
**When to use:** Dashboard live updates, issue count updates, alert notifications
**Example:**
```typescript
// stores/ws-store.ts
import { create } from 'zustand';

interface WsState {
  status: 'connecting' | 'connected' | 'disconnected';
  lastEvent: WsMessage | null;
  setStatus: (status: WsState['status']) => void;
  onMessage: (msg: WsMessage) => void;
}

export const useWsStore = create<WsState>((set) => ({
  status: 'disconnected',
  lastEvent: null,
  setStatus: (status) => set({ status }),
  onMessage: (msg) => set({ lastEvent: msg }),
}));
```

### Pattern 2: Custom WebSocket Hook with Exponential Backoff
**What:** Hook managing WS lifecycle, reconnection, and message dispatch to Zustand
**When to use:** Single connection per project, shared across all pages
**Example:**
```typescript
// hooks/use-websocket.ts
import { useEffect, useRef, useCallback } from 'react';
import { useWsStore } from '@/stores/ws-store';

export function useProjectWebSocket(projectId: string | null) {
  const wsRef = useRef<WebSocket | null>(null);
  const retriesRef = useRef(0);
  const setStatus = useWsStore((s) => s.setStatus);
  const onMessage = useWsStore((s) => s.onMessage);

  const connect = useCallback(() => {
    if (!projectId) return;
    const ws = new WebSocket(`ws://localhost:8081/stream/projects/${projectId}`);
    wsRef.current = ws;

    ws.onopen = () => {
      retriesRef.current = 0;
      setStatus('connected');
    };

    ws.onmessage = (e) => {
      const msg = JSON.parse(e.data);
      onMessage(msg);
    };

    ws.onclose = () => {
      setStatus('disconnected');
      const delay = Math.min(1000 * Math.pow(2, retriesRef.current), 30000);
      const jitter = delay * 0.1 * Math.random();
      retriesRef.current++;
      setTimeout(connect, delay + jitter);
    };
  }, [projectId, setStatus, onMessage]);

  useEffect(() => {
    setStatus('connecting');
    connect();
    return () => { wsRef.current?.close(); };
  }, [connect, setStatus]);
}
```

### Pattern 3: Push Panel (Master-Detail Without Overlay)
**What:** Right-side panel that compresses main content when detail is selected
**When to use:** Event detail, issue detail -- content shifts left, not covered by overlay
**Example:**
```typescript
// components/layout/push-panel.tsx
interface PushPanelLayoutProps {
  children: React.ReactNode;
  panel: React.ReactNode | null;
  panelWidth?: string; // default "w-[480px]"
}

export function PushPanelLayout({ children, panel, panelWidth = "w-[480px]" }: PushPanelLayoutProps) {
  return (
    <div className="flex h-full">
      <div className={`flex-1 min-w-0 overflow-auto transition-all ${panel ? 'mr-0' : ''}`}>
        {children}
      </div>
      {panel && (
        <div className={`${panelWidth} border-l border-border overflow-auto shrink-0`}>
          {panel}
        </div>
      )}
    </div>
  );
}
```

### Pattern 4: API Client with Typed Fetch
**What:** Thin wrapper for REST calls with error handling and type inference
**When to use:** All REST API interactions
**Example:**
```typescript
// lib/api.ts
const API_BASE = '/api/v1';

async function fetchApi<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(`${API_BASE}${path}`, {
    ...init,
    headers: { 'Content-Type': 'application/json', ...init?.headers },
  });
  if (!res.ok) {
    const err = await res.json().catch(() => ({ error: res.statusText }));
    throw new Error(err.error || res.statusText);
  }
  return res.json();
}

// Dashboard endpoints
export const api = {
  dashboard: {
    volume: (projectId: string, bucket = 'hour') =>
      fetchApi<VolumePoint[]>(`/projects/${projectId}/dashboard/volume?bucket=${bucket}`),
    levels: (projectId: string) =>
      fetchApi<LevelBreakdown[]>(`/projects/${projectId}/dashboard/levels`),
    topIssues: (projectId: string, limit = 10) =>
      fetchApi<TopIssue[]>(`/projects/${projectId}/dashboard/top-issues?limit=${limit}`),
    health: (projectId: string) =>
      fetchApi<HealthSummary>(`/projects/${projectId}/dashboard/health`),
  },
  issues: {
    list: (projectId: string, params?: IssueFilterParams) =>
      fetchApi<PaginatedResponse<Issue>>(`/projects/${projectId}/issues?${toQueryString(params)}`),
    resolve: (issueId: string) =>
      fetchApi<ActionResponse>(`/issues/${issueId}/resolve`, { method: 'POST' }),
    // ... more
  },
  // ... events, alerts, settings, team
};
```

### Pattern 5: Theme System Matching Existing Website
**What:** Copy the oklch CSS variables from the Mesh landing page for consistent monochrome theme
**When to use:** globals.css setup -- exact same variables ensure visual consistency
**Example:**
```css
/* globals.css -- copy from website/docs/.vitepress/theme/styles/main.css */
@import "tailwindcss";

@custom-variant dark (&:where(.dark, .dark *));

/* Fonts: Inter + JetBrains Mono (same as landing page) */
@font-face {
  font-family: 'Inter';
  font-style: normal;
  font-weight: 100 900;
  font-display: swap;
  src: url('https://fonts.gstatic.com/s/inter/v18/UcCO3FwrK3iLTeHuS_nVMrMxCp50SjIw2boKoduKmMEVuLyfAZ9hjQ.woff2') format('woff2');
}

@font-face {
  font-family: 'JetBrains Mono';
  font-style: normal;
  font-weight: 400 700;
  font-display: swap;
  src: url('https://fonts.gstatic.com/s/jetbrainsmono/v18/tDbY2o-flEEny0FZhsfKu5WU4zr3E_BX0PnT8RD8yKxjPVmUsaaDhw.woff2') format('woff2');
}

:root {
  --background: oklch(1 0 0);
  --foreground: oklch(0.098 0 0);
  --card: oklch(1 0 0);
  --card-foreground: oklch(0.098 0 0);
  --primary: oklch(0.145 0 0);
  --primary-foreground: oklch(0.985 0 0);
  --secondary: oklch(0.965 0 0);
  --secondary-foreground: oklch(0.145 0 0);
  --muted: oklch(0.955 0 0);
  --muted-foreground: oklch(0.45 0 0);
  --accent: oklch(0.955 0 0);
  --accent-foreground: oklch(0.145 0 0);
  --destructive: oklch(0.577 0.245 27.325);
  --border: oklch(0.905 0 0);
  --input: oklch(0.905 0 0);
  --ring: oklch(0.708 0 0);
  --radius: 0.5rem;
  --sidebar: oklch(0.98 0 0);
  --sidebar-foreground: oklch(0.145 0 0);
  --sidebar-primary: oklch(0.145 0 0);
  --sidebar-primary-foreground: oklch(0.985 0 0);
  --sidebar-accent: oklch(0.955 0 0);
  --sidebar-accent-foreground: oklch(0.145 0 0);
  --sidebar-border: oklch(0.905 0 0);
  --sidebar-ring: oklch(0.708 0 0);
}

.dark {
  --background: oklch(0.115 0 0);
  --foreground: oklch(0.955 0 0);
  --card: oklch(0.145 0 0);
  --card-foreground: oklch(0.955 0 0);
  --primary: oklch(0.955 0 0);
  --primary-foreground: oklch(0.115 0 0);
  --secondary: oklch(0.2 0 0);
  --secondary-foreground: oklch(0.955 0 0);
  --muted: oklch(0.2 0 0);
  --muted-foreground: oklch(0.58 0 0);
  --accent: oklch(0.2 0 0);
  --accent-foreground: oklch(0.955 0 0);
  --destructive: oklch(0.396 0.141 25.723);
  --border: oklch(0.22 0 0);
  --input: oklch(0.22 0 0);
  --ring: oklch(0.439 0 0);
  --sidebar: oklch(0.145 0 0);
  --sidebar-foreground: oklch(0.955 0 0);
  --sidebar-primary: oklch(0.955 0 0);
  --sidebar-primary-foreground: oklch(0.115 0 0);
  --sidebar-accent: oklch(0.2 0 0);
  --sidebar-accent-foreground: oklch(0.955 0 0);
  --sidebar-border: oklch(0.22 0 0);
  --sidebar-ring: oklch(0.439 0 0);
}
```

### Anti-Patterns to Avoid
- **Polling instead of WebSocket for live data:** The backend already broadcasts events via WS rooms. Do not poll REST endpoints for real-time updates.
- **Multiple WebSocket connections:** One WS connection per active project. Do not open per-page or per-component connections.
- **Using overlay/drawer for detail panels:** User explicitly chose push panel pattern where main content compresses. Do not use Dialog or Sheet for event/issue detail.
- **Hardcoding colors instead of CSS variables:** All colors must use the CSS variable system for theme switching. Use `text-foreground`, `bg-card`, etc., not `text-gray-900`.
- **Fetching without keyset pagination:** Backend uses cursor-based pagination (`cursor`, `cursor_id`), not offset-based. Always pass cursor parameters for paginated lists.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Data tables with sort/filter/pagination | Custom table component | TanStack Table + shadcn data-table | Column definitions, virtual scrolling, pagination state management |
| UI component primitives | Custom buttons, inputs, dialogs | shadcn/ui components | Accessibility (WAI-ARIA), keyboard navigation, focus management |
| Chart rendering | SVG chart from scratch | Recharts | Responsive containers, tooltips, animations, axis formatting |
| Client-side routing | Manual URL parsing | React Router v7 | Nested routes, URL params, navigation guards, history management |
| Theme toggling | Manual class manipulation | shadcn theme system + `useTheme` hook | Persists preference, handles system preference, SSR-safe |
| Toast/notification UI | Custom notification system | shadcn Sonner (toast) component | Animation, stacking, auto-dismiss, accessibility |
| Form validation | Manual validation logic | React Hook Form + Zod | Schema validation, error messages, field-level validation |

**Key insight:** The shadcn/ui ecosystem provides pre-built patterns for nearly every UI element in this app. The official "dashboard-01" block includes sidebar + data table + charts and should be used as the structural reference. The planner should task adding shadcn components progressively via CLI rather than writing UI primitives.

## Common Pitfalls

### Pitfall 1: Missing CORS Headers in Backend
**What goes wrong:** Frontend on `localhost:5173` (Vite dev server) cannot reach backend on `localhost:8080` due to cross-origin restrictions. All API calls fail with CORS errors.
**Why it happens:** The Mesher backend has zero CORS handling (confirmed: no `Access-Control-*` headers in any route). This is a purely frontend development concern.
**How to avoid:** Configure Vite's built-in proxy in `vite.config.ts`:
```typescript
export default defineConfig({
  server: {
    proxy: {
      '/api': 'http://localhost:8080',
    },
  },
});
```
For WebSocket proxying, add:
```typescript
server: {
  proxy: {
    '/api': 'http://localhost:8080',
    '/ws': {
      target: 'ws://localhost:8081',
      ws: true,
    },
  },
}
```
**Warning signs:** `Failed to fetch` errors in console, no network response in DevTools.

### Pitfall 2: Missing Org/Project CRUD REST Endpoints
**What goes wrong:** The UI requirement (UI-07) says "User can manage organizations and projects through the UI" but there are NO HTTP routes for creating, listing, or updating orgs and projects. Only internal OrgService and ProjectService actors exist.
**Why it happens:** The backend was built in phases 87-94 with org/project management as internal services, not exposed via REST. HTTP routes exist for team management (members, API keys) but not for org/project CRUD.
**How to avoid:** Either (a) add REST endpoints to the backend as a prerequisite task, or (b) scope UI-07 to only what the current API supports: team membership management, API key management, and project settings. The frontend cannot manage orgs/projects without backend endpoints.
**Warning signs:** No `POST /api/v1/orgs` or `POST /api/v1/projects` in the route table.

### Pitfall 3: Recharts react-is Peer Dependency Warning
**What goes wrong:** npm install warns about `react-is` peer dependency mismatch with React 19, or charts fail to render in some edge cases.
**Why it happens:** Recharts 3.x internally uses `react-is` which may not match React 19's version.
**How to avoid:** If warnings appear, add `react-is` override in `package.json`:
```json
{
  "overrides": {
    "react-is": "^19.0.0"
  }
}
```
**Warning signs:** npm peer dependency warnings mentioning `react-is` during installation.

### Pitfall 4: Tailwind v4 Configuration Confusion
**What goes wrong:** Using Tailwind v3 configuration patterns (tailwind.config.js, `@tailwind base/components/utilities` directives) with Tailwind v4 results in no styles or broken builds.
**Why it happens:** Tailwind v4 is a complete rewrite. Configuration is now CSS-first via `@theme` directive. There is no `tailwind.config.js`.
**How to avoid:** Use only the v4 approach: `@import "tailwindcss"` in CSS, `@tailwindcss/vite` plugin, and `@theme inline {}` for design token customization. shadcn CLI handles this correctly when initialized with the latest version.
**Warning signs:** Empty styles, "unknown at rule" warnings in editor, config file being ignored.

### Pitfall 5: WebSocket Message Types Not Handled
**What goes wrong:** The WS broadcasts multiple message types (`event`, `issue`, `issue_count`, `alert`, `filters_updated`) but the frontend only handles one, causing missed updates.
**Why it happens:** The streaming backend broadcasts different message shapes from different code paths (event ingestion, issue state changes, alert firing).
**How to avoid:** Implement a message dispatcher that routes by `type` field:
```typescript
function dispatchWsMessage(msg: WsMessage) {
  switch (msg.type) {
    case 'event': /* update live stream, dashboard */ break;
    case 'issue': /* update issue list (action: resolved/archived/etc) */ break;
    case 'issue_count': /* update unresolved count badge */ break;
    case 'alert': /* show toast notification, update alerts page */ break;
    case 'filters_updated': /* confirm filter change */ break;
  }
}
```
**Warning signs:** Dashboard updates on new events but issue count badge stays stale.

### Pitfall 6: Keyset Pagination Misuse
**What goes wrong:** Using offset-based pagination (`?page=2`) when backend expects cursor-based (`?cursor=<timestamp>&cursor_id=<uuid>`). Or losing cursor state on page refresh.
**Why it happens:** Most table examples show offset pagination. The Mesher backend uses keyset pagination for performance on large datasets.
**How to avoid:** Store the `next_cursor` and `next_cursor_id` from API responses. Pass them as query parameters for the next page. Use the `has_more` boolean to determine if more data exists. Reset cursors on filter changes.
**Warning signs:** Getting the same data on every page, or 400 errors from malformed cursor values.

## Code Examples

Verified patterns from backend API analysis:

### TypeScript Types Matching Backend JSON
```typescript
// types/api.ts -- derived from backend response shapes

// Dashboard
interface VolumePoint { bucket: string; count: number; }
interface LevelBreakdown { level: string; count: number; }
interface TopIssue {
  id: string; title: string; level: string;
  status: string; event_count: number; last_seen: string;
}
interface HealthSummary {
  unresolved_count: number; events_24h: number; new_today: number;
}

// Issues
interface Issue {
  id: string; title: string; level: string; status: string;
  event_count: number; first_seen: string; last_seen: string;
  assigned_to: string;
}
interface PaginatedResponse<T> {
  data: T[];
  has_more: boolean;
  next_cursor?: string;
  next_cursor_id?: string;
}

// Events
interface EventSummary {
  id: string; issue_id: string; level: string;
  message: string; received_at: string;
}
interface EventDetail {
  event: {
    id: string; project_id: string; issue_id: string;
    level: string; message: string; fingerprint: string;
    exception: any; stacktrace: any; breadcrumbs: any;
    tags: Record<string, string>; extra: any; user_context: any;
    sdk_name: string; sdk_version: string; received_at: string;
  };
  navigation: { next_id: string | null; prev_id: string | null; };
}

// Alerts
interface AlertRule {
  id: string; project_id: string; name: string;
  condition: { condition_type: string; threshold: number; window_minutes: number; };
  action: any; enabled: boolean; cooldown_minutes: number;
  last_fired_at: string | null; created_at: string;
}
interface Alert {
  id: string; rule_id: string; project_id: string;
  status: string; message: string; condition_snapshot: any;
  triggered_at: string; acknowledged_at: string | null;
  resolved_at: string | null; rule_name: string;
}

// Settings
interface ProjectSettings { retention_days: number; sample_rate: number; }
interface ProjectStorage { event_count: number; estimated_bytes: number; }

// Team / API Keys
interface Member {
  id: string; user_id: string; email: string;
  display_name: string; role: string; joined_at: string;
}
interface ApiKey {
  id: string; project_id: string; key_value: string;
  label: string; created_at: string; revoked_at: string | null;
}

// WebSocket messages
type WsMessage =
  | { type: 'event'; issue_id: string; data: any; }
  | { type: 'issue'; action: string; issue_id: string; }
  | { type: 'issue_count'; project_id: string; count: number; }
  | { type: 'alert'; alert_id: string; rule_name: string; condition: string; message: string; }
  | { type: 'filters_updated'; }
  | { type: 'error'; message: string; };

// Action responses
interface ActionResponse { status: string; affected: number; }
```

### Recharts Area Chart with CSS Variable Theming
```typescript
// components/charts/volume-chart.tsx
import { Area, AreaChart, CartesianGrid, ResponsiveContainer, Tooltip, XAxis, YAxis } from 'recharts';

interface VolumeChartProps {
  data: VolumePoint[];
}

export function VolumeChart({ data }: VolumeChartProps) {
  return (
    <ResponsiveContainer width="100%" height={240}>
      <AreaChart data={data}>
        <CartesianGrid
          strokeDasharray="3 3"
          stroke="hsl(var(--border))"
          vertical={false}
        />
        <XAxis
          dataKey="bucket"
          tickFormatter={(v) => new Date(v).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}
          stroke="hsl(var(--muted-foreground))"
          fontSize={12}
          tickLine={false}
          axisLine={false}
        />
        <YAxis
          stroke="hsl(var(--muted-foreground))"
          fontSize={12}
          tickLine={false}
          axisLine={false}
        />
        <Tooltip
          contentStyle={{
            backgroundColor: 'hsl(var(--card))',
            border: '1px solid hsl(var(--border))',
            borderRadius: '6px',
            color: 'hsl(var(--card-foreground))',
          }}
        />
        <Area
          type="monotone"
          dataKey="count"
          stroke="hsl(var(--foreground))"
          fill="hsl(var(--muted))"
          strokeWidth={1.5}
        />
      </AreaChart>
    </ResponsiveContainer>
  );
}
```

### TanStack Table with shadcn for Issues
```typescript
// Usage in pages/issues.tsx with shadcn data-table pattern
import { ColumnDef } from '@tanstack/react-table';
import { Badge } from '@/components/ui/badge';

const issueColumns: ColumnDef<Issue>[] = [
  {
    accessorKey: 'title',
    header: 'Issue',
    cell: ({ row }) => (
      <div className="font-medium truncate max-w-[400px]">
        {row.original.title}
      </div>
    ),
  },
  {
    accessorKey: 'level',
    header: 'Level',
    cell: ({ row }) => (
      <Badge variant={levelVariant(row.original.level)}>
        {row.original.level}
      </Badge>
    ),
  },
  {
    accessorKey: 'event_count',
    header: 'Events',
    cell: ({ row }) => (
      <span className="text-muted-foreground tabular-nums">
        {row.original.event_count.toLocaleString()}
      </span>
    ),
  },
  {
    accessorKey: 'last_seen',
    header: 'Last Seen',
    cell: ({ row }) => (
      <span className="text-muted-foreground">
        {formatRelativeTime(row.original.last_seen)}
      </span>
    ),
  },
];
```

### WebSocket Filter Subscription
```typescript
// Sending filter update to WS (matches backend's handle_subscribe_update)
function updateStreamFilters(ws: WebSocket, level?: string, environment?: string) {
  ws.send(JSON.stringify({
    type: 'subscribe',
    filters: {
      level: level || '',
      environment: environment || '',
    },
  }));
}
```

### Vite Config with Proxy and Tailwind
```typescript
// vite.config.ts
import path from "path";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  server: {
    proxy: {
      '/api': {
        target: 'http://localhost:8080',
        changeOrigin: true,
      },
    },
  },
});
```

## Backend API Reference

Complete route inventory for the frontend API client:

### Dashboard Endpoints
| Method | Path | Response | Notes |
|--------|------|----------|-------|
| GET | `/api/v1/projects/:project_id/dashboard/volume?bucket=hour\|day` | `[{bucket, count}]` | Time-series event volume |
| GET | `/api/v1/projects/:project_id/dashboard/levels` | `[{level, count}]` | Error breakdown by severity |
| GET | `/api/v1/projects/:project_id/dashboard/top-issues?limit=10` | `[{id, title, level, status, event_count, last_seen}]` | Top issues by frequency |
| GET | `/api/v1/projects/:project_id/dashboard/tags?key=<key>` | `[{value, count}]` | Tag breakdown |
| GET | `/api/v1/projects/:project_id/dashboard/health` | `{unresolved_count, events_24h, new_today}` | Project health summary |

### Issue Endpoints
| Method | Path | Response | Notes |
|--------|------|----------|-------|
| GET | `/api/v1/projects/:project_id/issues?status=&level=&assigned_to=&cursor=&cursor_id=&limit=25` | `{data: [...], has_more, next_cursor?, next_cursor_id?}` | Keyset paginated, filtered |
| GET | `/api/v1/issues/:issue_id/events?cursor=&cursor_id=&limit=25` | `{data: [...], has_more, next_cursor?, next_cursor_id?}` | Per-issue events paginated |
| GET | `/api/v1/issues/:issue_id/timeline?limit=50` | `[{id, level, message, received_at}]` | Issue event timeline |
| POST | `/api/v1/issues/:id/resolve` | `{status, affected}` | Resolve issue |
| POST | `/api/v1/issues/:id/archive` | `{status, affected}` | Archive issue |
| POST | `/api/v1/issues/:id/unresolve` | `{status, affected}` | Reopen issue |
| POST | `/api/v1/issues/:id/assign` | `{status: "ok"}` | Body: `{user_id: "..."}` |
| POST | `/api/v1/issues/:id/discard` | `{status, affected}` | Discard issue |
| POST | `/api/v1/issues/:id/delete` | `{status, affected}` | Delete issue |

### Event Endpoints
| Method | Path | Response | Notes |
|--------|------|----------|-------|
| GET | `/api/v1/projects/:project_id/events/search?q=<query>&limit=25` | `[{id, issue_id, level, message, received_at}]` | Full-text search |
| GET | `/api/v1/projects/:project_id/events/tags?key=<key>&value=<value>&limit=25` | `[{id, issue_id, level, message, tags, received_at}]` | Tag-based filter |
| GET | `/api/v1/events/:event_id` | `{event: {...all fields...}, navigation: {next_id, prev_id}}` | Full detail + nav |

### Alert Endpoints
| Method | Path | Response | Notes |
|--------|------|----------|-------|
| GET | `/api/v1/projects/:project_id/alert-rules` | `[{id, project_id, name, condition, action, enabled, cooldown_minutes, last_fired_at, created_at}]` | List rules |
| POST | `/api/v1/projects/:project_id/alert-rules` | `{id: "..."}` | Create rule (JSON body) |
| POST | `/api/v1/alert-rules/:rule_id/toggle` | `{status, affected}` | Body: `{enabled: true/false}` |
| POST | `/api/v1/alert-rules/:rule_id/delete` | `{status, affected}` | Delete rule |
| GET | `/api/v1/projects/:project_id/alerts?status=` | `[{id, rule_id, project_id, status, message, condition_snapshot, triggered_at, acknowledged_at, resolved_at, rule_name}]` | List fired alerts |
| POST | `/api/v1/alerts/:id/acknowledge` | `{status, affected}` | Acknowledge alert |
| POST | `/api/v1/alerts/:id/resolve` | `{status, affected}` | Resolve alert |

### Team & API Key Endpoints
| Method | Path | Response | Notes |
|--------|------|----------|-------|
| GET | `/api/v1/orgs/:org_id/members` | `[{id, user_id, email, display_name, role, joined_at}]` | List members |
| POST | `/api/v1/orgs/:org_id/members` | `{id: "..."}` | Body: `{user_id, role?}` |
| POST | `/api/v1/orgs/:org_id/members/:membership_id/role` | `{status, affected}` | Body: `{role: "admin"}` |
| POST | `/api/v1/orgs/:org_id/members/:membership_id/remove` | `{status, affected}` | Remove member |
| GET | `/api/v1/projects/:project_id/api-keys` | `[{id, project_id, key_value, label, created_at, revoked_at}]` | List API keys |
| POST | `/api/v1/projects/:project_id/api-keys` | `{key_value: "..."}` | Body: `{label?}` |
| POST | `/api/v1/api-keys/:key_id/revoke` | `{status, affected}` | Revoke key |

### Settings Endpoints
| Method | Path | Response | Notes |
|--------|------|----------|-------|
| GET | `/api/v1/projects/:project_id/settings` | `{retention_days, sample_rate}` | Get settings |
| POST | `/api/v1/projects/:project_id/settings` | `{status, affected}` | Update settings (JSON body) |
| GET | `/api/v1/projects/:project_id/storage` | `{event_count, estimated_bytes}` | Storage usage |

### WebSocket Streaming
| Protocol | Path | Behavior |
|----------|------|----------|
| WS | `ws://localhost:8081/stream/projects/:project_id` | Connect to project room, receive real-time updates |
| WS Send | `{type: "subscribe", filters: {level: "", environment: ""}}` | Update stream filters |
| WS Recv | `{type: "event", issue_id, data}` | New event ingested |
| WS Recv | `{type: "issue", action, issue_id}` | Issue state change (resolved/archived/etc.) |
| WS Recv | `{type: "issue_count", project_id, count}` | Unresolved issue count update |
| WS Recv | `{type: "alert", alert_id, rule_name, condition, message}` | Alert fired |
| WS Recv | `{type: "filters_updated"}` | Filter update confirmed |
| WS Recv | `{type: "error", message}` | Error message |

### NOT Available via REST (gaps)
| Missing | Exists As | Impact |
|---------|-----------|--------|
| `GET /api/v1/orgs` | OrgService.ListOrgs (internal) | Cannot list orgs from frontend |
| `POST /api/v1/orgs` | OrgService.CreateOrg (internal) | Cannot create orgs from frontend |
| `GET /api/v1/orgs/:id` | OrgService.GetOrg (internal) | Cannot fetch org detail |
| `GET /api/v1/orgs/:id/projects` | ProjectService.ListProjectsByOrg (internal) | Cannot list projects for sidebar |
| `POST /api/v1/orgs/:id/projects` | ProjectService.CreateProject (internal) | Cannot create projects |

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Tailwind v3 + tailwind.config.js | Tailwind v4 + CSS-first `@theme` | Jan 2025 | No JS config file, use @import "tailwindcss", Vite plugin |
| shadcn + Tailwind v3 | shadcn + Tailwind v4 | 2025 | Use `npx shadcn@latest`, not `shadcn@2.3.0` |
| Create React App | Vite 7.x | 2023+ | CRA deprecated, Vite is standard |
| Redux for state | Zustand / Jotai | 2022+ | Redux overkill for medium apps |
| react-router v6 | react-router v7 | Late 2024 | Library mode unchanged, framework mode new |
| Class components | Hooks + Function components | React 16.8+ | Entire app uses hooks |
| Recharts 2.x | Recharts 3.x | 2025 | Performance improvements, React 19 support |

**Deprecated/outdated:**
- Create React App: Deprecated, do not use. Vite is the standard.
- Tailwind v3 `tailwind.config.js`: Replaced by CSS-first configuration in v4.
- `@tailwind base; @tailwind components; @tailwind utilities;`: Replaced by `@import "tailwindcss"` in v4.
- `react-use-websocket`: Last published 1 year ago, no official React 19 support. Use custom hook.

## Open Questions

1. **Missing Org/Project CRUD REST Endpoints**
   - What we know: OrgService and ProjectService exist as internal Mesh services with CRUD operations, but no HTTP routes expose them. The frontend needs to list projects (for sidebar), list orgs, and potentially create both.
   - What's unclear: Whether the planner should include backend tasks to add these endpoints, or scope UI-07 to only what's currently available (team + API key management + settings).
   - Recommendation: Add a prerequisite task to expose org/project CRUD via REST routes (minimal: `GET /api/v1/orgs`, `GET /api/v1/orgs/:id/projects`). Without at least list endpoints, the sidebar project switcher cannot be populated. Alternatively, hardcode a default org/project during development and add the endpoints later.

2. **WebSocket Proxy During Development**
   - What we know: Vite proxy handles HTTP easily. WebSocket proxy requires separate configuration in `vite.config.ts`.
   - What's unclear: Whether Vite's WS proxy correctly handles the `/stream/projects/:id` path pattern with the Mesher WS server.
   - Recommendation: Configure the proxy and test early. If Vite WS proxy has issues, the frontend can connect directly to `ws://localhost:8081` during development (same-origin policy is less strict for WebSockets than HTTP).

3. **Production Deployment (Frontend Serving)**
   - What we know: This phase builds the SPA. Development uses Vite dev server with proxy.
   - What's unclear: How the built SPA will be served in production -- from the Mesher HTTP server, from a separate web server, or from a CDN.
   - Recommendation: For this phase, focus on `vite build` producing static assets. Production serving is a deployment concern that can be addressed later. The Vite proxy pattern ensures the frontend doesn't hardcode backend URLs.

## Sources

### Primary (HIGH confidence)
- Backend API routes: `/Users/sn0w/Documents/dev/snow/mesher/main.mpl` -- all 30+ HTTP routes verified
- Backend types: `/Users/sn0w/Documents/dev/snow/mesher/types/*.mpl` -- Event, Issue, Alert, Project, User structs
- Backend WebSocket handler: `/Users/sn0w/Documents/dev/snow/mesher/ingestion/ws_handler.mpl` -- dual-mode WS, filter subscribe
- Backend stream manager: `/Users/sn0w/Documents/dev/snow/mesher/services/stream_manager.mpl` -- connection state, filter matching
- Existing website theme: `/Users/sn0w/Documents/dev/snow/website/docs/.vitepress/theme/styles/main.css` -- oklch CSS variables
- Existing website config: `/Users/sn0w/Documents/dev/snow/website/components.json` -- shadcn new-york style, neutral base

### Secondary (MEDIUM-HIGH confidence)
- [React 19.2.x release](https://react.dev/blog/2025/10/01/react-19-2) -- confirmed 19.2.4 is latest stable
- [shadcn/ui Vite installation](https://ui.shadcn.com/docs/installation/vite) -- official setup guide for Tailwind v4
- [Recharts 3.7.x releases](https://github.com/recharts/recharts/releases) -- React 19 compatibility confirmed
- [TanStack Table 8.21.x](https://tanstack.com/table/latest) -- headless table, React adapter
- [Vite 7.x releases](https://vite.dev/releases) -- current stable with Tailwind v4 plugin
- [Tailwind CSS v4](https://tailwindcss.com/blog/tailwindcss-v4) -- CSS-first configuration, Vite plugin
- [React Router v7 SPA mode](https://reactrouter.com/how-to/spa) -- library mode for SPAs
- [Zustand comparison](https://zustand.docs.pmnd.rs/getting-started/comparison) -- selector-based subscriptions, no provider

### Tertiary (MEDIUM confidence)
- [WebSocket reconnection patterns](https://oneuptime.com/blog/post/2026-01-24-websocket-reconnection-logic/view) -- exponential backoff best practices
- [react-use-websocket React 19 issue](https://github.com/robtaussig/react-use-websocket/issues/256) -- no official React 19 support confirmed

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- all libraries verified with current versions and React 19 compatibility
- Architecture: HIGH -- patterns derived from actual backend API analysis and shadcn official docs
- Pitfalls: HIGH -- CORS gap and missing endpoints verified by direct code inspection
- API reference: HIGH -- every endpoint extracted from actual source code in mesher/main.mpl

**Research date:** 2026-02-15
**Valid until:** 2026-03-15 (30 days -- stack is stable, no major releases imminent)
