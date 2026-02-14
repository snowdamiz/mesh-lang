# PostgreSQL schema DDL for Mesher monitoring platform.
# Creates all 10 tables, 15 indexes, and manages daily event partitions.
# All operations are idempotent (IF NOT EXISTS) and use Pool.execute.

module Storage.Schema

# Create the complete Mesher database schema.
# Tables are created in dependency order (foreign keys require parent tables first).
# Returns Ok(0) on success, Err(message) on failure.
pub fn create_schema(pool :: Int) -> Int!String do
  # Enable pgcrypto extension for password hashing and random bytes
  let _ = Pool.execute(pool,
    "CREATE EXTENSION IF NOT EXISTS pgcrypto", [])?

  # 1. Organizations -- top-level tenancy unit
  let _ = Pool.execute(pool,
    "CREATE TABLE IF NOT EXISTS organizations (
      id UUID PRIMARY KEY DEFAULT uuidv7(),
      name TEXT NOT NULL,
      slug TEXT NOT NULL UNIQUE,
      created_at TIMESTAMPTZ NOT NULL DEFAULT now()
    )", [])?

  # 2. Users -- authentication accounts
  let _ = Pool.execute(pool,
    "CREATE TABLE IF NOT EXISTS users (
      id UUID PRIMARY KEY DEFAULT uuidv7(),
      email TEXT NOT NULL UNIQUE,
      password_hash TEXT NOT NULL,
      display_name TEXT NOT NULL,
      created_at TIMESTAMPTZ NOT NULL DEFAULT now()
    )", [])?

  # 3. Org memberships -- many-to-many with role
  let _ = Pool.execute(pool,
    "CREATE TABLE IF NOT EXISTS org_memberships (
      id UUID PRIMARY KEY DEFAULT uuidv7(),
      user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
      org_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
      role TEXT NOT NULL DEFAULT 'member',
      joined_at TIMESTAMPTZ NOT NULL DEFAULT now(),
      UNIQUE(user_id, org_id)
    )", [])?

  # 4. Sessions -- opaque token-based auth
  let _ = Pool.execute(pool,
    "CREATE TABLE IF NOT EXISTS sessions (
      token TEXT PRIMARY KEY,
      user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
      created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
      expires_at TIMESTAMPTZ NOT NULL DEFAULT now() + interval '7 days'
    )", [])?

  # 5. Projects -- belong to organizations
  let _ = Pool.execute(pool,
    "CREATE TABLE IF NOT EXISTS projects (
      id UUID PRIMARY KEY DEFAULT uuidv7(),
      org_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
      name TEXT NOT NULL,
      platform TEXT,
      created_at TIMESTAMPTZ NOT NULL DEFAULT now()
    )", [])?

  # 6. API keys -- multiple per project, mshr_ prefix format
  let _ = Pool.execute(pool,
    "CREATE TABLE IF NOT EXISTS api_keys (
      id UUID PRIMARY KEY DEFAULT uuidv7(),
      project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
      key_value TEXT NOT NULL UNIQUE,
      label TEXT NOT NULL DEFAULT 'default',
      created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
      revoked_at TIMESTAMPTZ
    )", [])?

  # 7. Issues -- grouped errors identified by fingerprint
  let _ = Pool.execute(pool,
    "CREATE TABLE IF NOT EXISTS issues (
      id UUID PRIMARY KEY DEFAULT uuidv7(),
      project_id UUID NOT NULL,
      fingerprint TEXT NOT NULL,
      title TEXT NOT NULL,
      level TEXT NOT NULL,
      status TEXT NOT NULL DEFAULT 'unresolved',
      event_count INTEGER NOT NULL DEFAULT 0,
      first_seen TIMESTAMPTZ NOT NULL DEFAULT now(),
      last_seen TIMESTAMPTZ NOT NULL DEFAULT now(),
      assigned_to UUID REFERENCES users(id),
      UNIQUE(project_id, fingerprint)
    )", [])?

  # 8. Events -- time-partitioned, append-only
  # Composite PK (id, received_at) required because PostgreSQL requires
  # partition key in primary key for partitioned tables.
  let _ = Pool.execute(pool,
    "CREATE TABLE IF NOT EXISTS events (
      id UUID NOT NULL DEFAULT uuidv7(),
      project_id UUID NOT NULL,
      issue_id UUID NOT NULL,
      level TEXT NOT NULL,
      message TEXT NOT NULL,
      fingerprint TEXT NOT NULL,
      exception JSONB,
      stacktrace JSONB,
      breadcrumbs JSONB,
      tags JSONB NOT NULL DEFAULT '{}',
      extra JSONB NOT NULL DEFAULT '{}',
      user_context JSONB,
      sdk_name TEXT,
      sdk_version TEXT,
      received_at TIMESTAMPTZ NOT NULL DEFAULT now(),
      PRIMARY KEY (id, received_at)
    ) PARTITION BY RANGE (received_at)", [])?

  # 9. Alert rules -- conditions and actions for automated notifications
  let _ = Pool.execute(pool,
    "CREATE TABLE IF NOT EXISTS alert_rules (
      id UUID PRIMARY KEY DEFAULT uuidv7(),
      project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
      name TEXT NOT NULL,
      condition_json JSONB NOT NULL,
      action_json JSONB NOT NULL,
      enabled BOOLEAN NOT NULL DEFAULT true,
      created_at TIMESTAMPTZ NOT NULL DEFAULT now()
    )", [])?

  # --- Indexes ---

  # Org membership indexes
  let _ = Pool.execute(pool,
    "CREATE INDEX IF NOT EXISTS idx_org_memberships_user ON org_memberships(user_id)", [])?

  let _ = Pool.execute(pool,
    "CREATE INDEX IF NOT EXISTS idx_org_memberships_org ON org_memberships(org_id)", [])?

  # Session indexes
  let _ = Pool.execute(pool,
    "CREATE INDEX IF NOT EXISTS idx_sessions_user ON sessions(user_id)", [])?

  let _ = Pool.execute(pool,
    "CREATE INDEX IF NOT EXISTS idx_sessions_expires ON sessions(expires_at)", [])?

  # Project indexes
  let _ = Pool.execute(pool,
    "CREATE INDEX IF NOT EXISTS idx_projects_org ON projects(org_id)", [])?

  # API key indexes
  let _ = Pool.execute(pool,
    "CREATE INDEX IF NOT EXISTS idx_api_keys_project ON api_keys(project_id)", [])?

  let _ = Pool.execute(pool,
    "CREATE INDEX IF NOT EXISTS idx_api_keys_value ON api_keys(key_value) WHERE revoked_at IS NULL", [])?

  # Issue indexes
  let _ = Pool.execute(pool,
    "CREATE INDEX IF NOT EXISTS idx_issues_project_status ON issues(project_id, status)", [])?

  let _ = Pool.execute(pool,
    "CREATE INDEX IF NOT EXISTS idx_issues_project_last_seen ON issues(project_id, last_seen DESC)", [])?

  # Event indexes (created per-partition automatically by PostgreSQL)
  let _ = Pool.execute(pool,
    "CREATE INDEX IF NOT EXISTS idx_events_project_received ON events(project_id, received_at DESC)", [])?

  let _ = Pool.execute(pool,
    "CREATE INDEX IF NOT EXISTS idx_events_issue_received ON events(issue_id, received_at DESC)", [])?

  let _ = Pool.execute(pool,
    "CREATE INDEX IF NOT EXISTS idx_events_level ON events(level, received_at DESC)", [])?

  let _ = Pool.execute(pool,
    "CREATE INDEX IF NOT EXISTS idx_events_fingerprint ON events(fingerprint)", [])?

  let _ = Pool.execute(pool,
    "CREATE INDEX IF NOT EXISTS idx_events_tags ON events USING GIN(tags jsonb_path_ops)", [])?

  # Alert rule indexes
  let _ = Pool.execute(pool,
    "CREATE INDEX IF NOT EXISTS idx_alert_rules_project ON alert_rules(project_id) WHERE enabled = true", [])?

  Ok(0)
end

# Create a single daily partition for the events table.
# The date_str parameter is in YYYYMMDD format (e.g., "20260214").
# Partition name format: events_YYYYMMDD.
pub fn create_partition(pool :: Int, date_str :: String) -> Int!String do
  # Format YYYYMMDD to YYYY-MM-DD for partition bounds.
  # date_str is like "20260214" -> "2026-02-14"
  let year = String.slice(date_str, 0, 4)
  let month = String.slice(date_str, 4, 6)
  let day = String.slice(date_str, 6, 8)
  let formatted = year <> "-" <> month <> "-" <> day

  # Build the SQL for partition creation.
  # Upper bound is next day, computed via PostgreSQL date arithmetic.
  let sql = "CREATE TABLE IF NOT EXISTS events_" <> date_str
    <> " PARTITION OF events FOR VALUES FROM ('" <> formatted
    <> "') TO (('" <> formatted <> "'::date + 1))"

  let _ = Pool.execute(pool, sql, [])?
  Ok(0)
end

# Create daily partitions for the next N days from today.
# All date computation happens in PostgreSQL since Mesh has no date/time functions.
# Uses recursion since Mesh has no mutable variable assignment.
pub fn create_partitions_ahead(pool :: Int, days :: Int) -> Int!String do
  create_partitions_loop(pool, days, 0)
end

# Recursive helper for partition creation.
fn create_partitions_loop(pool :: Int, days :: Int, i :: Int) -> Int!String do
  if i < days do
    # Query PostgreSQL for the date string at offset i days from now
    let offset_str = String.from(i)
    let rows = Pool.query(pool,
      "SELECT to_char(now() + ($1 || ' days')::interval, 'YYYYMMDD') AS d",
      [offset_str])?

    case rows do
      [row] -> do
        let date_str = Map.get(row, "d")
        let _ = create_partition(pool, date_str)?
        0
      end
      _ -> 0
    end

    create_partitions_loop(pool, days, i + 1)
  else
    Ok(0)
  end
end
