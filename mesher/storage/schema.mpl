# PostgreSQL schema DDL for Mesher monitoring platform.
# Creates all 11 tables, 18 indexes, and manages daily event partitions.
# All operations are idempotent (IF NOT EXISTS) and use Pool.execute.

# Create the complete Mesher database schema.
# Tables are created in dependency order (foreign keys require parent tables first).
# Returns Ok(0) on success, Err(message) on failure.
pub fn create_schema(pool :: PoolHandle) -> Int!String do
  Pool.execute(pool, "CREATE EXTENSION IF NOT EXISTS pgcrypto", [])?
  Pool.execute(pool, "CREATE TABLE IF NOT EXISTS organizations (id UUID PRIMARY KEY DEFAULT uuidv7(), name TEXT NOT NULL, slug TEXT NOT NULL UNIQUE, created_at TIMESTAMPTZ NOT NULL DEFAULT now())", [])?
  Pool.execute(pool, "CREATE TABLE IF NOT EXISTS users (id UUID PRIMARY KEY DEFAULT uuidv7(), email TEXT NOT NULL UNIQUE, password_hash TEXT NOT NULL, display_name TEXT NOT NULL, created_at TIMESTAMPTZ NOT NULL DEFAULT now())", [])?
  Pool.execute(pool, "CREATE TABLE IF NOT EXISTS org_memberships (id UUID PRIMARY KEY DEFAULT uuidv7(), user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE, org_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE, role TEXT NOT NULL DEFAULT 'member', joined_at TIMESTAMPTZ NOT NULL DEFAULT now(), UNIQUE(user_id, org_id))", [])?
  Pool.execute(pool, "CREATE TABLE IF NOT EXISTS sessions (token TEXT PRIMARY KEY, user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE, created_at TIMESTAMPTZ NOT NULL DEFAULT now(), expires_at TIMESTAMPTZ NOT NULL DEFAULT now() + interval '7 days')", [])?
  Pool.execute(pool, "CREATE TABLE IF NOT EXISTS projects (id UUID PRIMARY KEY DEFAULT uuidv7(), org_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE, name TEXT NOT NULL, platform TEXT, created_at TIMESTAMPTZ NOT NULL DEFAULT now())", [])?
  Pool.execute(pool, "CREATE TABLE IF NOT EXISTS api_keys (id UUID PRIMARY KEY DEFAULT uuidv7(), project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE, key_value TEXT NOT NULL UNIQUE, label TEXT NOT NULL DEFAULT 'default', created_at TIMESTAMPTZ NOT NULL DEFAULT now(), revoked_at TIMESTAMPTZ)", [])?
  Pool.execute(pool, "CREATE TABLE IF NOT EXISTS issues (id UUID PRIMARY KEY DEFAULT uuidv7(), project_id UUID NOT NULL, fingerprint TEXT NOT NULL, title TEXT NOT NULL, level TEXT NOT NULL, status TEXT NOT NULL DEFAULT 'unresolved', event_count INTEGER NOT NULL DEFAULT 0, first_seen TIMESTAMPTZ NOT NULL DEFAULT now(), last_seen TIMESTAMPTZ NOT NULL DEFAULT now(), assigned_to UUID REFERENCES users(id), UNIQUE(project_id, fingerprint))", [])?
  Pool.execute(pool, "CREATE TABLE IF NOT EXISTS events (id UUID NOT NULL DEFAULT uuidv7(), project_id UUID NOT NULL, issue_id UUID NOT NULL, level TEXT NOT NULL, message TEXT NOT NULL, fingerprint TEXT NOT NULL, exception JSONB, stacktrace JSONB, breadcrumbs JSONB, tags JSONB NOT NULL DEFAULT '{}', extra JSONB NOT NULL DEFAULT '{}', user_context JSONB, sdk_name TEXT, sdk_version TEXT, received_at TIMESTAMPTZ NOT NULL DEFAULT now(), PRIMARY KEY (id, received_at)) PARTITION BY RANGE (received_at)", [])?
  Pool.execute(pool, "CREATE TABLE IF NOT EXISTS alert_rules (id UUID PRIMARY KEY DEFAULT uuidv7(), project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE, name TEXT NOT NULL, condition_json JSONB NOT NULL, action_json JSONB NOT NULL, enabled BOOLEAN NOT NULL DEFAULT true, created_at TIMESTAMPTZ NOT NULL DEFAULT now())", [])?
  Pool.execute(pool, "CREATE TABLE IF NOT EXISTS alerts (id UUID PRIMARY KEY DEFAULT uuidv7(), rule_id UUID NOT NULL REFERENCES alert_rules(id) ON DELETE CASCADE, project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE, status TEXT NOT NULL DEFAULT 'active', message TEXT NOT NULL, condition_snapshot JSONB NOT NULL, triggered_at TIMESTAMPTZ NOT NULL DEFAULT now(), acknowledged_at TIMESTAMPTZ, resolved_at TIMESTAMPTZ)", [])?
  Pool.execute(pool, "ALTER TABLE alert_rules ADD COLUMN IF NOT EXISTS cooldown_minutes INTEGER NOT NULL DEFAULT 60", [])?
  Pool.execute(pool, "ALTER TABLE alert_rules ADD COLUMN IF NOT EXISTS last_fired_at TIMESTAMPTZ", [])?
  Pool.execute(pool, "ALTER TABLE projects ADD COLUMN IF NOT EXISTS retention_days INTEGER NOT NULL DEFAULT 90", [])?
  Pool.execute(pool, "ALTER TABLE projects ADD COLUMN IF NOT EXISTS sample_rate REAL NOT NULL DEFAULT 1.0", [])?
  Pool.execute(pool, "CREATE INDEX IF NOT EXISTS idx_org_memberships_user ON org_memberships(user_id)", [])?
  Pool.execute(pool, "CREATE INDEX IF NOT EXISTS idx_org_memberships_org ON org_memberships(org_id)", [])?
  Pool.execute(pool, "CREATE INDEX IF NOT EXISTS idx_sessions_user ON sessions(user_id)", [])?
  Pool.execute(pool, "CREATE INDEX IF NOT EXISTS idx_sessions_expires ON sessions(expires_at)", [])?
  Pool.execute(pool, "CREATE INDEX IF NOT EXISTS idx_projects_org ON projects(org_id)", [])?
  Pool.execute(pool, "CREATE INDEX IF NOT EXISTS idx_api_keys_project ON api_keys(project_id)", [])?
  Pool.execute(pool, "CREATE INDEX IF NOT EXISTS idx_api_keys_value ON api_keys(key_value) WHERE revoked_at IS NULL", [])?
  Pool.execute(pool, "CREATE INDEX IF NOT EXISTS idx_issues_project_status ON issues(project_id, status)", [])?
  Pool.execute(pool, "CREATE INDEX IF NOT EXISTS idx_issues_project_last_seen ON issues(project_id, last_seen DESC)", [])?
  Pool.execute(pool, "CREATE INDEX IF NOT EXISTS idx_events_project_received ON events(project_id, received_at DESC)", [])?
  Pool.execute(pool, "CREATE INDEX IF NOT EXISTS idx_events_issue_received ON events(issue_id, received_at DESC)", [])?
  Pool.execute(pool, "CREATE INDEX IF NOT EXISTS idx_events_level ON events(level, received_at DESC)", [])?
  Pool.execute(pool, "CREATE INDEX IF NOT EXISTS idx_events_fingerprint ON events(fingerprint)", [])?
  Pool.execute(pool, "CREATE INDEX IF NOT EXISTS idx_events_tags ON events USING GIN(tags jsonb_path_ops)", [])?
  Pool.execute(pool, "CREATE INDEX IF NOT EXISTS idx_alert_rules_project ON alert_rules(project_id) WHERE enabled = true", [])?
  Pool.execute(pool, "CREATE INDEX IF NOT EXISTS idx_alerts_project_status ON alerts(project_id, status)", [])?
  Pool.execute(pool, "CREATE INDEX IF NOT EXISTS idx_alerts_rule ON alerts(rule_id)", [])?
  Pool.execute(pool, "CREATE INDEX IF NOT EXISTS idx_alerts_triggered ON alerts(triggered_at DESC)", [])?
  Ok(0)
end

# Create a single daily partition for the events table.
# The date_str parameter is in YYYYMMDD format (e.g., "20260214").
pub fn create_partition(pool :: PoolHandle, date_str :: String) -> Int!String do
  let year = String.slice(date_str, 0, 4)
  let month = String.slice(date_str, 4, 6)
  let day = String.slice(date_str, 6, 8)
  let formatted = year <> "-" <> month <> "-" <> day
  let part1 = "CREATE TABLE IF NOT EXISTS events_" <> date_str <> " PARTITION OF events FOR VALUES FROM ('"
  let sql = part1 <> formatted <> "') TO (('" <> formatted <> "'::date + 1))"
  Pool.execute(pool, sql, [])?
  Ok(0)
end

fn create_partitions_loop(pool :: PoolHandle, days :: Int, i :: Int) -> Int!String do
  if i < days do
    let offset_str = String.from(i)
    let rows = Pool.query(pool, "SELECT to_char(now() + ($1 || ' days')::interval, 'YYYYMMDD') AS d", [offset_str])?
    if List.length(rows) > 0 do
      let date_str = Map.get(List.head(rows), "d")
      create_partition(pool, date_str)?
      0
    else
      0
    end
    create_partitions_loop(pool, days, i + 1)
  else
    Ok(0)
  end
end

# Create daily partitions for the next N days from today.
pub fn create_partitions_ahead(pool :: PoolHandle, days :: Int) -> Int!String do
  create_partitions_loop(pool, days, 0)
end
