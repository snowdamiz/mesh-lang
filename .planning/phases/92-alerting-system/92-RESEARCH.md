# Phase 92: Alerting System - Research

**Researched:** 2026-02-15
**Domain:** Timer-driven alert rule evaluation, threshold and event-based alert conditions, WebSocket alert notifications, deduplication/cooldown, alert state management -- all implemented in Mesh (.mpl) with PostgreSQL
**Confidence:** HIGH

## Summary

Phase 92 adds a complete alerting system to Mesher: users create alert rules with configurable conditions (event count thresholds, new issue creation, issue regression), a single timer-driven evaluator actor periodically checks all rules, triggered alerts are delivered via WebSocket to connected dashboards, and deduplication/cooldown prevents alert fatigue. This phase builds directly on the established patterns from Phases 87-91: service actors for state, Timer.sleep + recursive actors for periodic evaluation, Ws.broadcast for WebSocket delivery, PostgreSQL for persistence, and POST routes for all mutations.

The existing codebase already has the `alert_rules` table (Phase 87 schema) with `condition_json` and `action_json` JSONB columns, the `AlertRule` and `AlertCondition` type structs in `types/alert.mpl`, and an index on enabled rules (`idx_alert_rules_project`). What is missing is: (1) new schema for fired alerts and alert state tracking (an `alerts` table), (2) an AlertEvaluator service/actor that runs on a timer to check rules, (3) SQL queries for threshold evaluation (event counts in time windows), (4) integration hooks for event-based alerts (new issue creation, regression) triggered from `routes.mpl` after event processing, (5) HTTP API routes for alert rule CRUD and alert state management, and (6) deduplication/cooldown tracking to prevent repeated firing.

The primary architectural decision is the evaluator pattern. The prior decision from STATE.md is explicit: "Timer.send_after spawns OS thread per call -- use single recurring timer actor for alerting." This aligns with the established `spike_checker` pattern in `pipeline.mpl`: a recursive actor that calls Timer.sleep, evaluates conditions via SQL, and loops. For threshold-based alerts (ALERT-02), the evaluator queries the events table with `COUNT(*) WHERE received_at > now() - interval 'N minutes'` per project. For event-based alerts (ALERT-03), the triggering happens inline in `routes.mpl` after `EventProcessor.process_event` returns -- when the upsert_issue RETURNING clause indicates a new issue (first_seen = last_seen) or a regression (status flipped from resolved to unresolved).

**Primary recommendation:** Build the AlertEvaluator as a single recursive actor (like spike_checker) running every 30 seconds. It loads all enabled alert rules via SQL, evaluates threshold conditions via aggregate queries, and fires alerts by inserting into an `alerts` table and broadcasting via Ws.broadcast. Event-based alerts (new issue, regression) are triggered synchronously from the event processing path in `routes.mpl`. Deduplication uses a `last_fired_at` timestamp per rule in the database, with a configurable cooldown period checked before firing. Alert state management (active/acknowledged/resolved) uses POST routes following the issue lifecycle pattern.

## Standard Stack

### Core
| Component | Version/Detail | Purpose | Why Standard |
|-----------|---------------|---------|--------------|
| Mesh language | v8.0+ (current) | All application code | Dogfooding -- entire alerting system in Mesh |
| service blocks | Built-in | AlertService for alert state management | GenServer pattern, established in all prior services |
| Timer.sleep + recursive actor | Built-in | Periodic alert rule evaluation (30s tick) | Established pattern from spike_checker, flush_ticker, health_checker |
| Pool.query / Pool.execute | Built-in | Alert rule CRUD, threshold evaluation, fired alert tracking | Existing PostgreSQL query layer |
| Ws.broadcast | Built-in (Phase 90) | Push alert notifications to project rooms | Existing room infrastructure, auto-cleanup on disconnect |
| Request.param / Request.query | Built-in | HTTP route parameter extraction | Established in all API handlers |
| HTTP.on_get / HTTP.on_post | Built-in | Alert API endpoints | Existing routing infrastructure |
| PipelineRegistry | Existing service | Pool handle lookup for HTTP handlers | Process.whereis("mesher_registry") pattern |
| extract_json_field pattern | Established (91-03) | Parse JSON body fields via PostgreSQL JSONB extraction | Avoids cross-module from_json limitation |

### Supporting
| Component | Detail | Purpose | When to Use |
|-----------|--------|---------|-------------|
| Map.get / Map.put | Built-in | Alert state tracking in service state | In-memory cooldown tracking |
| String.from(Int) | Built-in | Convert counts/thresholds to String for SQL params | Query parameter construction |
| String.to_int | Built-in | Parse threshold/window from query results | Configuration parsing |
| List.map | Built-in | Transform query result rows to JSON | Response serialization |
| String.join | Built-in | Build JSON arrays from lists | Response construction |
| Map.has_key | Built-in | Check cooldown state existence | Deduplication logic |
| both_match helper | Established (90-01) | AND logic for condition matching | Avoids && codegen issue |

### No New Runtime Extensions Required

All required functionality exists in the current Mesh runtime. WebSocket broadcast (Ws.broadcast) is already working from Phase 90. Timer.sleep + recursive actors are the established periodic evaluation pattern. No Rust runtime modifications needed.

## Architecture Patterns

### Recommended Project Structure
```
mesher/
  types/
    alert.mpl                   # EXTEND: add AlertState type, Alert (fired) struct, AlertAction struct
  storage/
    queries.mpl                 # EXTEND: add alert rule CRUD, threshold evaluation, fired alert queries
    schema.mpl                  # EXTEND: add alerts table, alert_rules columns for cooldown
  services/
    alert_service.mpl           # NEW: AlertService for alert state management (acknowledge, resolve)
  api/
    alerts.mpl                  # NEW: HTTP route handlers for alert rule CRUD and alert management
  ingestion/
    pipeline.mpl                # EXTEND: spawn alert_evaluator actor, start AlertService
    routes.mpl                  # EXTEND: add event-based alert firing after event processing
```

### Pattern 1: Timer-Driven Alert Evaluator Actor
**What:** A single recursive actor (like spike_checker) that periodically loads all enabled alert rules, evaluates threshold conditions via SQL, and fires alerts when conditions are met.
**When to use:** ALERT-02 (threshold-based alerts -- event count > N in M minutes)
**Design:** Runs every 30 seconds. For each enabled rule with condition_type = 'threshold', queries `SELECT count(*) FROM events WHERE project_id = $1::uuid AND received_at > now() - interval '$2 minutes'` and compares against the threshold. If exceeded and cooldown has elapsed, inserts an alert record and broadcasts via Ws.broadcast.
**Example:**
```mesh
# Recursive evaluator actor -- same pattern as spike_checker, health_checker.
# Loads enabled alert rules, evaluates threshold conditions, fires alerts.
actor alert_evaluator(pool :: PoolHandle) do
  Timer.sleep(30000)
  let result = evaluate_all_rules(pool)
  case result do
    Ok(n) -> log_eval_result(n)
    Err(e) -> log_eval_error(e)
  end
  alert_evaluator(pool)
end
```

### Pattern 2: Event-Based Alert Triggering (New Issue / Regression)
**What:** After EventProcessor.process_event returns, check if the event created a new issue or caused a regression, and fire matching alert rules inline.
**When to use:** ALERT-03 (new issue creation, issue regression)
**Integration point:** In `routes.mpl`, in the `broadcast_event` helper (or a new helper called after it), query alert_rules for condition_type = 'new_issue' or 'regression' for the project, and fire matching alerts.
**Key insight:** The upsert_issue query already returns the issue_id. To detect new vs existing, we need to check if this was a new insert or an update. The current upsert uses `ON CONFLICT DO UPDATE` which always returns a row -- we cannot distinguish new from existing purely from the RETURNING clause. Two approaches: (A) query first_seen = last_seen on the returned issue (new issues have these equal), or (B) use a separate SQL query to detect the condition.
**Example:**
```mesh
# In routes.mpl, after successful event processing:
fn check_event_alerts(pool :: PoolHandle, project_id :: String, issue_id :: String) do
  # Check if this issue was just created (first_seen = last_seen implies first event)
  let rows_result = Pool.query(pool, "SELECT CASE WHEN first_seen = last_seen THEN 'new' WHEN status = 'unresolved' AND last_seen = now() THEN 'regression_candidate' ELSE 'existing' END AS issue_state FROM issues WHERE id = $1::uuid", [issue_id])
  case rows_result do
    Ok(rows) -> evaluate_event_alerts(pool, project_id, issue_id, rows)
    Err(_) -> 0
  end
end
```

### Pattern 3: Deduplication via last_fired_at and Cooldown Period
**What:** Each alert rule has a cooldown period. Before firing an alert, check if the last firing was within the cooldown window. Store last_fired_at on the alert_rules row or in a separate table.
**When to use:** ALERT-05 (cooldown and deduplication windows)
**Design:** Add `cooldown_minutes` column to alert_rules (default 60) and `last_fired_at` TIMESTAMPTZ column (nullable). Before firing, check `last_fired_at IS NULL OR last_fired_at < now() - interval '$cooldown minutes'`. Update last_fired_at atomically with firing.
**Example:**
```mesh
# Check cooldown before firing -- returns true if alert should fire
fn should_fire_alert(pool :: PoolHandle, rule_id :: String, cooldown_minutes :: String) -> Bool!String do
  let rows = Pool.query(pool, "SELECT 1 AS ok FROM alert_rules WHERE id = $1::uuid AND (last_fired_at IS NULL OR last_fired_at < now() - interval '1 minute' * $2::int)", [rule_id, cooldown_minutes])?
  if List.length(rows) > 0 do
    Ok(true)
  else
    Ok(false)
  end
end
```

### Pattern 4: Alert Notification via Ws.broadcast
**What:** When an alert fires, broadcast a notification to the project's WebSocket room.
**When to use:** ALERT-04 (WebSocket delivery to dashboards)
**Design:** Use the existing room infrastructure (`project:<project_id>` rooms). Broadcast a JSON notification with alert metadata (rule name, condition, severity). This reuses the exact same pattern as issue state change broadcasting in `routes.mpl`.
**Example:**
```mesh
fn broadcast_alert(project_id :: String, rule_name :: String, alert_id :: String, condition_type :: String) do
  let room = "project:" <> project_id
  let msg = "{\"type\":\"alert\",\"alert_id\":\"" <> alert_id <> "\",\"rule_name\":\"" <> rule_name <> "\",\"condition\":\"" <> condition_type <> "\"}"
  let _ = Ws.broadcast(room, msg)
  0
end
```

### Pattern 5: Alert State Machine (Active -> Acknowledged -> Resolved)
**What:** Fired alerts have a lifecycle: active (just fired) -> acknowledged (user saw it) -> resolved (user confirmed fix). Uses POST routes like issue lifecycle.
**When to use:** ALERT-06 (manage alert states)
**Design:** The `alerts` table has a `status` column (default 'active'). POST /api/v1/alerts/:id/acknowledge and POST /api/v1/alerts/:id/resolve transition states. Same pattern as resolve_issue, archive_issue etc.
**Example:**
```mesh
pub fn handle_acknowledge_alert(request) do
  let reg_pid = Process.whereis("mesher_registry")
  let pool = PipelineRegistry.get_pool(reg_pid)
  let alert_id = require_param(request, "id")
  let result = acknowledge_alert(pool, alert_id)
  case result do
    Ok(n) -> acknowledge_success(pool, alert_id, n)
    Err(e) -> HTTP.response(500, "{\"error\":\"" <> e <> "\"}")
  end
end
```

### Pattern 6: Alert Rule CRUD via POST with JSON Body Parsing
**What:** Create and manage alert rules via HTTP POST with JSON body, using PostgreSQL JSONB extraction for field parsing.
**When to use:** ALERT-01 (create alert rules)
**Design:** Same as assign_issue pattern (89-02) and extract_json_field helper (91-03). The JSON body contains name, condition (threshold/window/type), action type, and cooldown. PostgreSQL extracts fields server-side.
**Example:**
```mesh
# Create alert rule -- parse JSON body fields via PostgreSQL
fn create_alert_rule(pool :: PoolHandle, project_id :: String, body :: String) -> String!String do
  let sql = "INSERT INTO alert_rules (project_id, name, condition_json, action_json, cooldown_minutes) SELECT $1::uuid, j->>'name', (j->'condition')::jsonb, COALESCE((j->'action')::jsonb, '{\"type\":\"websocket\"}'::jsonb), COALESCE((j->>'cooldown_minutes')::int, 60) FROM (SELECT $2::jsonb AS j) AS sub RETURNING id::text"
  let rows = Pool.query(pool, sql, [project_id, body])?
  if List.length(rows) > 0 do
    Ok(Map.get(List.head(rows), "id"))
  else
    Err("create_alert_rule: no id returned")
  end
end
```

### Anti-Patterns to Avoid
- **Evaluating threshold rules inside EventProcessor:** The EventProcessor is a single-threaded service that processes every event synchronously. Adding threshold evaluation (which involves aggregate SQL queries) would block event processing. Evaluate thresholds in the separate alert_evaluator actor instead.
- **Using Timer.send_after for alert evaluation:** Decision [87-02] established that Timer.send_after is incompatible with service dispatch tags. Use Timer.sleep + recursive actor (alert_evaluator) instead.
- **Tracking cooldown in-memory only:** If the process crashes and restarts, in-memory cooldown state is lost, causing duplicate alerts. Store `last_fired_at` in PostgreSQL for persistence across restarts.
- **Broadcasting inside the evaluator actor without cooldown check:** Always check cooldown before broadcasting. An alert evaluator that fires on every tick when the condition is true causes alert fatigue -- the exact problem ALERT-05 is designed to prevent.
- **Complex case arm logic in route handlers:** Per decision [88-02], extract multi-line logic into helper functions. Keep handler bodies minimal with single-expression case arms.
- **Using && operator in nested if blocks:** Per decision [90-01], use the `both_match(a, b)` helper for AND logic to avoid LLVM PHI node codegen issues.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Periodic evaluation | Custom timer/scheduler | Timer.sleep + recursive actor (alert_evaluator) | Established pattern from spike_checker, health_checker, flush_ticker |
| WebSocket notification delivery | Custom connection tracking | Ws.broadcast to project rooms | Existing room infrastructure with auto-cleanup |
| Cooldown tracking | In-memory timestamp map | PostgreSQL `last_fired_at` column on alert_rules | Survives process restarts, atomic update |
| Threshold computation | Mesh-level event counting | PostgreSQL `COUNT(*) WHERE received_at > now() - interval` | Database aggregation is O(index scan), not O(all events in memory) |
| JSON body parsing | Cross-module from_json | PostgreSQL JSONB extraction (`$1::jsonb->>'field'`) | Avoids cross-module from_json limitation [88-02] |
| Deduplication window | Custom hash map with expiry | SQL WHERE `last_fired_at < now() - interval` check | Atomic, persistent, no state to manage |
| Alert ID generation | Application-level UUID | PostgreSQL `DEFAULT uuidv7()` | Mesh has no UUID/random generation |

**Key insight:** The alerting system is fundamentally a combination of existing patterns: periodic actor (spike_checker) + SQL aggregation (dashboard queries) + WebSocket broadcast (routes.mpl) + state machine transitions (issue lifecycle) + JSON body parsing (team management). No new Mesh runtime capabilities are needed.

## Common Pitfalls

### Pitfall 1: Threshold Query Performance on Partitioned Events Table
**What goes wrong:** Threshold queries like `SELECT count(*) FROM events WHERE project_id = $1::uuid AND received_at > now() - interval '5 minutes'` scan only the current partition when the time window is small, but could scan multiple partitions for larger windows (e.g., 24 hours).
**Why it happens:** Events table is partitioned by `received_at` (daily partitions). PostgreSQL prunes partitions based on the `received_at` filter.
**How to avoid:** Keep default threshold windows small (5-60 minutes). The `idx_events_project_received` index on `(project_id, received_at DESC)` enables efficient range scans within a partition. For windows up to 24 hours, at most 2 partitions are scanned (today + yesterday). This is acceptable for a 30-second evaluation tick.
**Warning signs:** Slow evaluation ticks when many rules have large time windows.

### Pitfall 2: Alert Evaluator Blocking on Slow Queries
**What goes wrong:** If the evaluator actor issues a slow SQL query (many rules, large event tables), it blocks its own loop, delaying subsequent evaluations.
**Why it happens:** The evaluator is a single actor processing rules sequentially.
**How to avoid:** Evaluate rules one at a time in a loop, not in a single massive query. Each rule evaluation is a simple aggregate query (fast with index). The 30-second tick interval provides ample time. If rule count grows large, batch rules by project to reduce query round-trips.
**Warning signs:** Evaluation ticks taking longer than the tick interval (30s).

### Pitfall 3: Duplicate Alerts on Process Restart
**What goes wrong:** If the alert_evaluator crashes and restarts, it re-evaluates all rules immediately. If cooldown state was only in memory, alerts that already fired will fire again.
**How to avoid:** Store `last_fired_at` in the `alert_rules` table (PostgreSQL). The cooldown check queries this column. Even after restart, the database remembers when each rule last fired.
**Warning signs:** Burst of duplicate alerts after any service restart.

### Pitfall 4: New Issue Detection Without Extra Query
**What goes wrong:** Trying to detect "new issue" from the upsert_issue return value alone. The upsert always returns an issue_id whether it was a new insert or a conflict update.
**Why it happens:** PostgreSQL `INSERT ... ON CONFLICT ... DO UPDATE ... RETURNING id` returns a row in both the insert and update cases.
**How to avoid:** After upsert, query the issue to check `first_seen = last_seen` (a new issue has these equal because both default to `now()`). Alternatively, modify the upsert to also return `first_seen` and `last_seen` for comparison in the caller.
**Warning signs:** New-issue alerts never firing, or firing on every event for an existing issue.

### Pitfall 5: Race Between Timer Evaluator and Event-Based Triggers
**What goes wrong:** A threshold alert fires from the timer evaluator at the same time an event-based alert fires from event processing for the same rule. This causes duplicate alerts.
**Why it happens:** Timer-based and event-based evaluation run in different actors.
**How to avoid:** Use the `last_fired_at` cooldown check as the deduplication gate for ALL alert firings. The atomicity of the PostgreSQL UPDATE ensures only one fires: `UPDATE alert_rules SET last_fired_at = now() WHERE id = $1 AND (last_fired_at IS NULL OR last_fired_at < now() - interval '...' * cooldown_minutes) RETURNING id`. If the UPDATE affects 0 rows, the cooldown blocked it.
**Warning signs:** Duplicate alerts appearing within the cooldown window.

### Pitfall 6: Define-Before-Use Ordering in New Files
**What goes wrong:** Compilation errors when a route handler calls a helper that is defined below it.
**Why it happens:** Mesh requires define-before-use (decision [90-03]).
**How to avoid:** Order functions bottom-up: leaf SQL query helpers first, then evaluation logic, then broadcast helpers, then route handlers (pub functions) last.
**Warning signs:** Compilation errors about unknown functions in new alert files.

### Pitfall 7: Single-Expression Case Arm Constraint
**What goes wrong:** Multi-line logic inside case arm bodies causes parser errors.
**Why it happens:** Mesh parser requires single-expression case arms (decision [88-02]).
**How to avoid:** Extract multi-line logic into named helper functions. Each case arm should call a single helper function. This is established practice throughout the codebase (resolve_success, archive_success, broadcast_event, etc.).
**Warning signs:** Parser errors on case expressions with complex bodies.

## Code Examples

### Schema Extension: Alerts Table and Alert Rules Columns
```sql
-- New alerts table: records of fired alerts with state management
CREATE TABLE IF NOT EXISTS alerts (
    id              UUID PRIMARY KEY DEFAULT uuidv7(),
    rule_id         UUID NOT NULL REFERENCES alert_rules(id) ON DELETE CASCADE,
    project_id      UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    status          TEXT NOT NULL DEFAULT 'active',  -- active, acknowledged, resolved
    message         TEXT NOT NULL,                   -- human-readable alert description
    condition_snapshot JSONB NOT NULL,               -- snapshot of condition that triggered
    triggered_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    acknowledged_at TIMESTAMPTZ,
    resolved_at     TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_alerts_project_status ON alerts(project_id, status);
CREATE INDEX IF NOT EXISTS idx_alerts_rule ON alerts(rule_id);
CREATE INDEX IF NOT EXISTS idx_alerts_triggered ON alerts(triggered_at DESC);

-- Add cooldown and tracking columns to alert_rules
ALTER TABLE alert_rules ADD COLUMN IF NOT EXISTS cooldown_minutes INTEGER NOT NULL DEFAULT 60;
ALTER TABLE alert_rules ADD COLUMN IF NOT EXISTS last_fired_at TIMESTAMPTZ;
```

**Note:** The `ALTER TABLE ... ADD COLUMN IF NOT EXISTS` syntax requires PostgreSQL 9.6+. Since we use PostgreSQL 18, this is safe. For the schema.mpl `create_schema` function, these can be added as additional Pool.execute calls after the existing alert_rules CREATE TABLE.

### Alert Rule Creation Query
```mesh
# Create an alert rule from JSON body (PostgreSQL extracts fields server-side).
# condition_json expected shape: {"condition_type":"threshold","threshold":100,"window_minutes":5}
# or: {"condition_type":"new_issue"} or {"condition_type":"regression"}
pub fn create_alert_rule(pool :: PoolHandle, project_id :: String, body :: String) -> String!String do
  let sql = "INSERT INTO alert_rules (project_id, name, condition_json, action_json, cooldown_minutes) SELECT $1::uuid, COALESCE(j->>'name', 'Unnamed Rule'), COALESCE((j->'condition')::jsonb, '{}'::jsonb), COALESCE((j->'action')::jsonb, '{\"type\":\"websocket\"}'::jsonb), COALESCE((j->>'cooldown_minutes')::int, 60) FROM (SELECT $2::jsonb AS j) AS sub RETURNING id::text"
  let rows = Pool.query(pool, sql, [project_id, body])?
  if List.length(rows) > 0 do
    Ok(Map.get(List.head(rows), "id"))
  else
    Err("create_alert_rule: no id returned")
  end
end
```

### Threshold Evaluation Query
```mesh
# Evaluate a single threshold rule: count events in the window and compare to threshold.
# Returns true if the threshold is exceeded AND cooldown has elapsed.
pub fn evaluate_threshold_rule(pool :: PoolHandle, rule_id :: String, project_id :: String, threshold_str :: String, window_str :: String, cooldown_str :: String) -> Bool!String do
  let sql = "SELECT CASE WHEN event_count > $3::int AND (last_fired IS NULL OR last_fired < now() - interval '1 minute' * $6::int) THEN 1 ELSE 0 END AS should_fire FROM (SELECT count(*) AS event_count FROM events WHERE project_id = $2::uuid AND received_at > now() - interval '1 minute' * $4::int) counts, (SELECT last_fired_at AS last_fired FROM alert_rules WHERE id = $1::uuid) cooldown"
  let rows = Pool.query(pool, sql, [rule_id, project_id, threshold_str, window_str, "", cooldown_str])?
  if List.length(rows) > 0 do
    let should_fire = Map.get(List.head(rows), "should_fire")
    Ok(should_fire == "1")
  else
    Ok(false)
  end
end
```

### Fire Alert (Insert + Update last_fired_at + Broadcast)
```mesh
# Atomically fire an alert: insert alert record, update rule last_fired_at, broadcast.
# Uses two SQL statements (insert + update) since Mesh cannot do multi-statement transactions
# without Pool.transaction (which is not in the current runtime).
pub fn fire_alert(pool :: PoolHandle, rule_id :: String, project_id :: String, rule_name :: String, condition_type :: String, message :: String) -> String!String do
  let sql = "INSERT INTO alerts (rule_id, project_id, status, message, condition_snapshot) VALUES ($1::uuid, $2::uuid, 'active', $3, jsonb_build_object('condition_type', $4, 'rule_name', $5)) RETURNING id::text"
  let rows = Pool.query(pool, sql, [rule_id, project_id, message, condition_type, rule_name])?
  if List.length(rows) > 0 do
    let alert_id = Map.get(List.head(rows), "id")
    # Update last_fired_at on the rule (cooldown gate for next evaluation)
    let _ = Pool.execute(pool, "UPDATE alert_rules SET last_fired_at = now() WHERE id = $1::uuid", [rule_id])
    # Broadcast alert notification to project room
    let room = "project:" <> project_id
    let msg = "{\"type\":\"alert\",\"alert_id\":\"" <> alert_id <> "\",\"rule_name\":\"" <> rule_name <> "\",\"condition\":\"" <> condition_type <> "\",\"message\":\"" <> message <> "\"}"
    let _ = Ws.broadcast(room, msg)
    Ok(alert_id)
  else
    Err("fire_alert: no id returned")
  end
end
```

### Alert Evaluator Actor (Threshold Rules)
```mesh
# Load all enabled threshold rules and evaluate each one.
fn evaluate_all_threshold_rules(pool :: PoolHandle) -> Int!String do
  let rows = Pool.query(pool, "SELECT id::text, project_id::text, name, condition_json::text, cooldown_minutes::text FROM alert_rules WHERE enabled = true AND condition_json->>'condition_type' = 'threshold'", [])?
  evaluate_rules_loop(pool, rows, 0, List.length(rows), 0)
end

fn evaluate_rules_loop(pool :: PoolHandle, rules, i :: Int, total :: Int, fired :: Int) -> Int!String do
  if i < total do
    let rule = List.get(rules, i)
    let rule_id = Map.get(rule, "id")
    let project_id = Map.get(rule, "project_id")
    let rule_name = Map.get(rule, "name")
    let cooldown_str = Map.get(rule, "cooldown_minutes")
    let new_fired = evaluate_single_rule(pool, rule_id, project_id, rule_name, cooldown_str, rule, fired)
    evaluate_rules_loop(pool, rules, i + 1, total, new_fired)
  else
    Ok(fired)
  end
end

# Evaluator actor -- runs every 30 seconds
actor alert_evaluator(pool :: PoolHandle) do
  Timer.sleep(30000)
  let result = evaluate_all_threshold_rules(pool)
  case result do
    Ok(n) -> log_eval_result(n)
    Err(e) -> log_eval_error(e)
  end
  alert_evaluator(pool)
end
```

### Alert State Management Queries
```mesh
# Acknowledge an alert (ALERT-06)
pub fn acknowledge_alert(pool :: PoolHandle, alert_id :: String) -> Int!String do
  Pool.execute(pool, "UPDATE alerts SET status = 'acknowledged', acknowledged_at = now() WHERE id = $1::uuid AND status = 'active'", [alert_id])
end

# Resolve an alert (ALERT-06)
pub fn resolve_alert(pool :: PoolHandle, alert_id :: String) -> Int!String do
  Pool.execute(pool, "UPDATE alerts SET status = 'resolved', resolved_at = now() WHERE id = $1::uuid AND status IN ('active', 'acknowledged')", [alert_id])
end

# List alerts for a project filtered by status
pub fn list_alerts(pool :: PoolHandle, project_id :: String, status :: String) -> List<Map<String, String>>!String do
  let sql = "SELECT a.id::text, a.rule_id::text, a.project_id::text, a.status, a.message, a.condition_snapshot::text, a.triggered_at::text, COALESCE(a.acknowledged_at::text, '') AS acknowledged_at, COALESCE(a.resolved_at::text, '') AS resolved_at, r.name AS rule_name FROM alerts a JOIN alert_rules r ON r.id = a.rule_id WHERE a.project_id = $1::uuid AND ($2 = '' OR a.status = $2) ORDER BY a.triggered_at DESC LIMIT 50"
  let rows = Pool.query(pool, sql, [project_id, status])?
  Ok(rows)
end
```

### Alert Rule CRUD Route Handlers
```mesh
# POST /api/v1/projects/:project_id/alert-rules
pub fn handle_create_alert_rule(request) do
  let reg_pid = Process.whereis("mesher_registry")
  let pool = PipelineRegistry.get_pool(reg_pid)
  let project_id = require_param(request, "project_id")
  let body = Request.body(request)
  let result = create_alert_rule(pool, project_id, body)
  case result do
    Ok(id) -> HTTP.response(201, "{\"id\":\"" <> id <> "\"}")
    Err(e) -> HTTP.response(400, "{\"error\":\"" <> e <> "\"}")
  end
end

# GET /api/v1/projects/:project_id/alert-rules
pub fn handle_list_alert_rules(request) do
  let reg_pid = Process.whereis("mesher_registry")
  let pool = PipelineRegistry.get_pool(reg_pid)
  let project_id = require_param(request, "project_id")
  let result = list_alert_rules(pool, project_id)
  case result do
    Ok(rows) -> HTTP.response(200, rows_to_json(rows))
    Err(e) -> HTTP.response(500, "{\"error\":\"" <> e <> "\"}")
  end
end
```

### Event-Based Alert Trigger (New Issue / Regression)
```mesh
# Called from routes.mpl after successful event processing.
# Checks if the issue is new (first_seen = last_seen) or regressed,
# then fires matching alert rules for the project.
fn check_event_based_alerts(pool :: PoolHandle, project_id :: String, issue_id :: String) do
  let rows_result = Pool.query(pool, "SELECT CASE WHEN i.first_seen = i.last_seen THEN 'new_issue' WHEN i.status = 'unresolved' AND i.event_count = 1 THEN 'new_issue' ELSE 'existing' END AS state FROM issues i WHERE i.id = $1::uuid", [issue_id])
  case rows_result do
    Ok(rows) -> check_event_alert_rules(pool, project_id, issue_id, rows)
    Err(_) -> 0
  end
end

# For regression detection: check if upsert flipped status from resolved to unresolved.
# This is detected by the upsert SQL which sets status = CASE WHEN 'resolved' THEN 'unresolved'.
# We can detect regression by querying for issues where last_seen = now() and status was
# just changed. A simpler approach: modify upsert to RETURNING a was_resolved flag.
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| No alerting | Timer-driven evaluator + event-based triggers | Phase 92 | Automated alert notifications for error spikes and new issues |
| Manual dashboard monitoring | Push alerts to WebSocket rooms | Phase 92 | Proactive notification instead of reactive monitoring |
| spike_checker for archived issues only | Generalized threshold evaluation for all rules | Phase 92 | User-configurable alert conditions instead of hardcoded spike logic |
| No alert state tracking | Active/acknowledged/resolved alert lifecycle | Phase 92 | Alert management workflow for teams |

**Current state (pre-Phase 92):**
- `alert_rules` table exists with condition_json and action_json JSONB columns
- `AlertRule` and `AlertCondition` structs exist in `types/alert.mpl`
- Index on enabled rules exists (`idx_alert_rules_project WHERE enabled = true`)
- No alerts table for fired alert records
- No alert evaluation logic
- No cooldown/deduplication columns on alert_rules
- spike_checker actor exists for archived issue volume escalation (related concept, but not user-configurable)
- WebSocket broadcast infrastructure fully operational (Phase 90)

## Open Questions

1. **Regression detection mechanism**
   - What we know: The upsert_issue query sets `status = CASE WHEN issues.status = 'resolved' THEN 'unresolved' ELSE issues.status END`. This means a regression happens atomically inside the upsert.
   - What's unclear: How to detect in the calling code (routes.mpl) that a regression just occurred, since the upsert RETURNING clause only returns `id`.
   - Recommendation: Modify the upsert_issue query in queries.mpl to also return `status` and a flag like `CASE WHEN issues.status = 'resolved' AND excluded.event_count = 1 THEN true ELSE false END AS was_regression`. The caller can then check this flag. Alternatively, query the issue after upsert to check `status = 'unresolved' AND event_count = 1 AND first_seen != last_seen` (meaning it was resolved before this event reopened it). The simplest approach: extend the existing upsert RETURNING to include `first_seen::text, last_seen::text, status` so the caller can determine new vs existing vs regression.

2. **Should alert_evaluator be an actor or a service?**
   - What we know: The evaluator runs periodically and does not need to respond to synchronous calls. It only needs a pool handle and Timer.sleep.
   - What's unclear: Whether making it a service (with a timer cast) provides any advantage over a plain recursive actor.
   - Recommendation: Use a plain recursive actor (like spike_checker). It has no state to query and no external callers. A service adds unnecessary complexity. The `spawn(alert_evaluator, pool)` pattern is well-established.

3. **Threshold evaluation: per-rule SQL queries vs batch query**
   - What we know: Each threshold rule needs `COUNT(*) FROM events WHERE project_id = $1 AND received_at > now() - interval '$window'`. With N rules, that is N aggregate queries per tick.
   - What's unclear: For many rules on the same project, could we batch the evaluation?
   - Recommendation: Start with per-rule queries (simplicity). Each query uses the `idx_events_project_received` index and scans at most 1-2 partitions. For the expected scale of Mesher (tens of rules, not thousands), per-rule evaluation at 30-second intervals is negligible load. Optimize to batch queries per-project only if performance testing shows a need.

4. **Alert notification format for different condition types**
   - What we know: ALERT-04 requires WebSocket delivery. Different condition types (threshold, new_issue, regression) may need different notification payloads.
   - What's unclear: Exact JSON format for alert notifications.
   - Recommendation: Use a unified format: `{"type":"alert", "alert_id":"...", "rule_name":"...", "condition":"threshold|new_issue|regression", "message":"Event count exceeded 100 in 5 minutes", "triggered_at":"..."}`. The `message` field is human-readable and condition-type-specific. The `condition` field enables client-side routing/display logic.

5. **Interaction between spike_checker and alert_evaluator**
   - What we know: spike_checker (Phase 89) escalates archived issues with volume bursts. The alert_evaluator evaluates user-defined threshold rules. Both involve "count events in time window" logic.
   - What's unclear: Should spike_checker be replaced by alert rules, or remain as a hardcoded system behavior?
   - Recommendation: Keep spike_checker as-is. It is a system-level behavior (escalating archived issues) independent of user-defined alert rules. The alert_evaluator is for user-configured notifications. They serve different purposes and can coexist without conflict.

## Sources

### Primary (HIGH confidence)
- `/Users/sn0w/Documents/dev/snow/mesher/types/alert.mpl` -- Existing AlertRule and AlertCondition structs
- `/Users/sn0w/Documents/dev/snow/mesher/storage/schema.mpl` -- Existing alert_rules table schema, index
- `/Users/sn0w/Documents/dev/snow/mesher/ingestion/pipeline.mpl` -- spike_checker, health_checker, alert_evaluator patterns; PipelineRegistry service
- `/Users/sn0w/Documents/dev/snow/mesher/ingestion/routes.mpl` -- Event broadcasting pattern, issue state transition handlers
- `/Users/sn0w/Documents/dev/snow/mesher/services/stream_manager.mpl` -- Per-connection state management pattern
- `/Users/sn0w/Documents/dev/snow/mesher/services/writer.mpl` -- Timer.sleep + recursive actor pattern (flush_ticker)
- `/Users/sn0w/Documents/dev/snow/mesher/services/rate_limiter.mpl` -- Timer.sleep + recursive ticker pattern
- `/Users/sn0w/Documents/dev/snow/mesher/services/event_processor.mpl` -- EventProcessor pipeline, route_event pattern
- `/Users/sn0w/Documents/dev/snow/mesher/storage/queries.mpl` -- All query patterns, upsert_issue, check_volume_spikes
- `/Users/sn0w/Documents/dev/snow/mesher/api/helpers.mpl` -- query_or_default, require_param, to_json_array helpers
- `/Users/sn0w/Documents/dev/snow/mesher/ingestion/ws_handler.mpl` -- WebSocket subscription protocol, room management
- `/Users/sn0w/Documents/dev/snow/.planning/STATE.md` -- All accumulated decisions from phases 87-91.1
- `/Users/sn0w/Documents/dev/snow/.planning/REQUIREMENTS.md` -- ALERT-01 through ALERT-06 definitions
- `/Users/sn0w/Documents/dev/snow/.planning/phases/90-real-time-streaming/90-RESEARCH.md` -- Ws.broadcast patterns, room infrastructure
- `/Users/sn0w/Documents/dev/snow/.planning/phases/91-rest-api/91-RESEARCH.md` -- Query patterns, JSON serialization, REST route patterns
- `/Users/sn0w/Documents/dev/snow/.planning/phases/87-foundation/87-RESEARCH.md` -- Schema design, service patterns, Timer.send_after thread concern
- `/Users/sn0w/Documents/dev/snow/.planning/phases/88-ingestion-pipeline/88-RESEARCH.md` -- Middleware, rate limiting, PipelineRegistry pattern

### Secondary (MEDIUM confidence)
- `/Users/sn0w/Documents/dev/snow/.planning/phases/89-error-grouping-issue-lifecycle/89-RESEARCH.md` -- Fingerprinting, issue lifecycle, upsert pattern, regression detection

### Tertiary (LOW confidence)
- None -- all findings verified against actual codebase

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- all components verified against existing codebase and established patterns
- Architecture: HIGH -- evaluator actor pattern directly mirrors spike_checker; broadcast pattern mirrors routes.mpl; state management mirrors issue lifecycle
- Schema design: HIGH -- alerts table follows established pattern (uuidv7 PK, TIMESTAMPTZ, status TEXT); alert_rules extension is minimal (two new columns)
- Query patterns: HIGH -- threshold evaluation uses same index and partition-aware patterns as dashboard queries
- Deduplication: HIGH -- last_fired_at in PostgreSQL with atomic cooldown check is a standard pattern
- Pitfalls: HIGH -- all identified from direct codebase analysis and established decisions

**Research date:** 2026-02-15
**Valid until:** 2026-03-15 (stable -- Mesh runtime changes are controlled by this project)
