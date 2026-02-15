# Reusable query helper functions for all Mesher entity types.
# Provides CRUD operations using Pool.query and Pool.execute.
# All functions take the pool handle (PoolHandle) as first argument.
# Pool.query returns List<Map<String, String>>; struct construction
# is done manually from the Map fields.

from Types.Project import Organization, Project, ApiKey
from Types.User import User, OrgMembership, Session
from Types.Issue import Issue
from Types.Event import Event
from Types.Alert import AlertRule

# --- Organization queries ---

# Insert a new organization. Returns the generated UUID.
pub fn insert_org(pool :: PoolHandle, name :: String, slug :: String) -> String!String do
  let rows = Pool.query(pool, "INSERT INTO organizations (name, slug) VALUES ($1, $2) RETURNING id::text", [name, slug])?
  if List.length(rows) > 0 do
    Ok(Map.get(List.head(rows), "id"))
  else
    Err("insert_org: no id returned")
  end
end

# Get an organization by ID.
pub fn get_org(pool :: PoolHandle, id :: String) -> Organization!String do
  let rows = Pool.query(pool, "SELECT id::text, name, slug, created_at::text FROM organizations WHERE id = $1::uuid", [id])?
  if List.length(rows) > 0 do
    let row = List.head(rows)
    Ok(Organization { id: Map.get(row, "id"), name: Map.get(row, "name"), slug: Map.get(row, "slug"), created_at: Map.get(row, "created_at") })
  else
    Err("not found")
  end
end

# List all organizations.
pub fn list_orgs(pool :: PoolHandle) -> List<Organization>!String do
  let rows = Pool.query(pool, "SELECT id::text, name, slug, created_at::text FROM organizations ORDER BY name", [])?
  Ok(List.map(rows, fn(row) do
    Organization { id: Map.get(row, "id"), name: Map.get(row, "name"), slug: Map.get(row, "slug"), created_at: Map.get(row, "created_at") }
  end))
end

# --- Project queries ---

# Insert a new project. Returns the generated UUID.
pub fn insert_project(pool :: PoolHandle, org_id :: String, name :: String, platform :: String) -> String!String do
  let rows = Pool.query(pool, "INSERT INTO projects (org_id, name, platform) VALUES ($1::uuid, $2, $3) RETURNING id::text", [org_id, name, platform])?
  if List.length(rows) > 0 do
    Ok(Map.get(List.head(rows), "id"))
  else
    Err("insert_project: no id returned")
  end
end

# Get a project by ID.
pub fn get_project(pool :: PoolHandle, id :: String) -> Project!String do
  let rows = Pool.query(pool, "SELECT id::text, org_id::text, name, platform, created_at::text FROM projects WHERE id = $1::uuid", [id])?
  if List.length(rows) > 0 do
    let row = List.head(rows)
    Ok(Project { id: Map.get(row, "id"), org_id: Map.get(row, "org_id"), name: Map.get(row, "name"), platform: Map.get(row, "platform"), created_at: Map.get(row, "created_at") })
  else
    Err("not found")
  end
end

# List all projects for an organization.
pub fn list_projects_by_org(pool :: PoolHandle, org_id :: String) -> List<Project>!String do
  let rows = Pool.query(pool, "SELECT id::text, org_id::text, name, platform, created_at::text FROM projects WHERE org_id = $1::uuid ORDER BY name", [org_id])?
  Ok(List.map(rows, fn(row) do
    Project { id: Map.get(row, "id"), org_id: Map.get(row, "org_id"), name: Map.get(row, "name"), platform: Map.get(row, "platform"), created_at: Map.get(row, "created_at") }
  end))
end

# --- API key queries ---

# Create a new API key for a project. Returns the generated key_value (mshr_ prefixed).
pub fn create_api_key(pool :: PoolHandle, project_id :: String, label :: String) -> String!String do
  let rows = Pool.query(pool, "INSERT INTO api_keys (project_id, key_value, label) VALUES ($1::uuid, 'mshr_' || encode(gen_random_bytes(24), 'hex'), $2) RETURNING key_value", [project_id, label])?
  if List.length(rows) > 0 do
    Ok(Map.get(List.head(rows), "key_value"))
  else
    Err("create_api_key: no key returned")
  end
end

# Get the project associated with a valid (non-revoked) API key.
pub fn get_project_by_api_key(pool :: PoolHandle, key_value :: String) -> Project!String do
  let rows = Pool.query(pool, "SELECT p.id::text, p.org_id::text, p.name, p.platform, p.created_at::text FROM projects p JOIN api_keys ak ON ak.project_id = p.id WHERE ak.key_value = $1 AND ak.revoked_at IS NULL", [key_value])?
  if List.length(rows) > 0 do
    let row = List.head(rows)
    Ok(Project { id: Map.get(row, "id"), org_id: Map.get(row, "org_id"), name: Map.get(row, "name"), platform: Map.get(row, "platform"), created_at: Map.get(row, "created_at") })
  else
    Err("not found")
  end
end

# Revoke an API key by setting revoked_at to now().
pub fn revoke_api_key(pool :: PoolHandle, key_id :: String) -> Int!String do
  let result = Pool.execute(pool, "UPDATE api_keys SET revoked_at = now() WHERE id = $1::uuid", [key_id])
  result
end

# --- User queries ---

# Create a new user with bcrypt password hashing via pgcrypto (cost factor 12).
pub fn create_user(pool :: PoolHandle, email :: String, password :: String, display_name :: String) -> String!String do
  let rows = Pool.query(pool, "INSERT INTO users (email, password_hash, display_name) VALUES ($1, crypt($2, gen_salt('bf', 12)), $3) RETURNING id::text", [email, password, display_name])?
  if List.length(rows) > 0 do
    Ok(Map.get(List.head(rows), "id"))
  else
    Err("create_user: no id returned")
  end
end

# Authenticate a user by email and password.
# Returns the User if credentials match, Err("not found") otherwise.
pub fn authenticate_user(pool :: PoolHandle, email :: String, password :: String) -> User!String do
  let rows = Pool.query(pool, "SELECT id::text, email, display_name, created_at::text FROM users WHERE email = $1 AND password_hash = crypt($2, password_hash)", [email, password])?
  if List.length(rows) > 0 do
    let row = List.head(rows)
    Ok(User { id: Map.get(row, "id"), email: Map.get(row, "email"), display_name: Map.get(row, "display_name"), created_at: Map.get(row, "created_at") })
  else
    Err("not found")
  end
end

# Get a user by ID.
pub fn get_user(pool :: PoolHandle, id :: String) -> User!String do
  let rows = Pool.query(pool, "SELECT id::text, email, display_name, created_at::text FROM users WHERE id = $1::uuid", [id])?
  if List.length(rows) > 0 do
    let row = List.head(rows)
    Ok(User { id: Map.get(row, "id"), email: Map.get(row, "email"), display_name: Map.get(row, "display_name"), created_at: Map.get(row, "created_at") })
  else
    Err("not found")
  end
end

# --- Session queries ---

# Create a new session with a cryptographically random token.
# Returns the 64-char hex token.
pub fn create_session(pool :: PoolHandle, user_id :: String) -> String!String do
  let rows = Pool.query(pool, "INSERT INTO sessions (token, user_id) VALUES (encode(gen_random_bytes(32), 'hex'), $1::uuid) RETURNING token", [user_id])?
  if List.length(rows) > 0 do
    Ok(Map.get(List.head(rows), "token"))
  else
    Err("create_session: no token returned")
  end
end

# Validate a session token. Returns the Session if valid and not expired.
pub fn validate_session(pool :: PoolHandle, token :: String) -> Session!String do
  let rows = Pool.query(pool, "SELECT token, user_id::text, created_at::text, expires_at::text FROM sessions WHERE token = $1 AND expires_at > now()", [token])?
  if List.length(rows) > 0 do
    let row = List.head(rows)
    Ok(Session { token: Map.get(row, "token"), user_id: Map.get(row, "user_id"), created_at: Map.get(row, "created_at"), expires_at: Map.get(row, "expires_at") })
  else
    Err("not found")
  end
end

# Delete a session by token (logout).
pub fn delete_session(pool :: PoolHandle, token :: String) -> Int!String do
  let result = Pool.execute(pool, "DELETE FROM sessions WHERE token = $1", [token])
  result
end

# --- Org membership queries ---

# Add a user to an organization with a role (owner/admin/member).
pub fn add_member(pool :: PoolHandle, user_id :: String, org_id :: String, role :: String) -> String!String do
  let rows = Pool.query(pool, "INSERT INTO org_memberships (user_id, org_id, role) VALUES ($1::uuid, $2::uuid, $3) RETURNING id::text", [user_id, org_id, role])?
  if List.length(rows) > 0 do
    Ok(Map.get(List.head(rows), "id"))
  else
    Err("add_member: no id returned")
  end
end

# Get all members of an organization.
pub fn get_members(pool :: PoolHandle, org_id :: String) -> List<OrgMembership>!String do
  let rows = Pool.query(pool, "SELECT id::text, user_id::text, org_id::text, role, joined_at::text FROM org_memberships WHERE org_id = $1::uuid", [org_id])?
  Ok(List.map(rows, fn(row) do
    OrgMembership { id: Map.get(row, "id"), user_id: Map.get(row, "user_id"), org_id: Map.get(row, "org_id"), role: Map.get(row, "role"), joined_at: Map.get(row, "joined_at") }
  end))
end

# Get all organizations a user belongs to.
pub fn get_user_orgs(pool :: PoolHandle, user_id :: String) -> List<OrgMembership>!String do
  let rows = Pool.query(pool, "SELECT id::text, user_id::text, org_id::text, role, joined_at::text FROM org_memberships WHERE user_id = $1::uuid", [user_id])?
  Ok(List.map(rows, fn(row) do
    OrgMembership { id: Map.get(row, "id"), user_id: Map.get(row, "user_id"), org_id: Map.get(row, "org_id"), role: Map.get(row, "role"), joined_at: Map.get(row, "joined_at") }
  end))
end

# --- Issue queries (Phase 89) ---

# Upsert an issue: insert on first occurrence, update on subsequent.
# Uses PostgreSQL ON CONFLICT on (project_id, fingerprint) unique constraint.
# Handles GROUP-04 (new issue), GROUP-05 (event_count + last_seen), and
# ISSUE-02 (regression: resolved flips to unresolved on new event).
# Returns Ok(issue_id) or Err.
pub fn upsert_issue(pool :: PoolHandle, project_id :: String, fingerprint :: String, title :: String, level :: String) -> String!String do
  let sql = "INSERT INTO issues (project_id, fingerprint, title, level, event_count) VALUES ($1::uuid, $2, $3, $4, 1) ON CONFLICT (project_id, fingerprint) DO UPDATE SET event_count = issues.event_count + 1, last_seen = now(), status = CASE WHEN issues.status = 'resolved' THEN 'unresolved' ELSE issues.status END RETURNING id::text"
  let rows = Pool.query(pool, sql, [project_id, fingerprint, title, level])?
  if List.length(rows) > 0 do
    Ok(Map.get(List.head(rows), "id"))
  else
    Err("upsert_issue: no id returned")
  end
end

# Check if an issue with the given fingerprint is discarded (ISSUE-05 suppression).
# Returns true if the issue exists with status = 'discarded', false otherwise.
pub fn is_issue_discarded(pool :: PoolHandle, project_id :: String, fingerprint :: String) -> Bool!String do
  let rows = Pool.query(pool, "SELECT 1 AS found FROM issues WHERE project_id = $1::uuid AND fingerprint = $2 AND status = 'discarded'", [project_id, fingerprint])?
  if List.length(rows) > 0 do
    Ok(true)
  else
    Ok(false)
  end
end

# --- Issue management queries (Phase 89 Plan 02) ---

# Transition an issue to 'resolved' status (ISSUE-01).
pub fn resolve_issue(pool :: PoolHandle, issue_id :: String) -> Int!String do
  Pool.execute(pool, "UPDATE issues SET status = 'resolved' WHERE id = $1::uuid AND status != 'resolved'", [issue_id])
end

# Transition an issue to 'archived' status (ISSUE-01).
pub fn archive_issue(pool :: PoolHandle, issue_id :: String) -> Int!String do
  Pool.execute(pool, "UPDATE issues SET status = 'archived' WHERE id = $1::uuid", [issue_id])
end

# Reopen an issue -- set status back to 'unresolved' (ISSUE-01).
pub fn unresolve_issue(pool :: PoolHandle, issue_id :: String) -> Int!String do
  Pool.execute(pool, "UPDATE issues SET status = 'unresolved' WHERE id = $1::uuid", [issue_id])
end

# Assign an issue to a user. Pass empty string to unassign (ISSUE-04).
pub fn assign_issue(pool :: PoolHandle, issue_id :: String, user_id :: String) -> Int!String do
  if String.length(user_id) > 0 do
    Pool.execute(pool, "UPDATE issues SET assigned_to = $2::uuid WHERE id = $1::uuid", [issue_id, user_id])
  else
    Pool.execute(pool, "UPDATE issues SET assigned_to = NULL WHERE id = $1::uuid", [issue_id])
  end
end

# Mark an issue as discarded -- future events with this fingerprint are suppressed (ISSUE-05).
pub fn discard_issue(pool :: PoolHandle, issue_id :: String) -> Int!String do
  Pool.execute(pool, "UPDATE issues SET status = 'discarded' WHERE id = $1::uuid", [issue_id])
end

# Delete an issue and all associated events (ISSUE-05).
# Events deleted first due to FK constraint on issue_id.
pub fn delete_issue(pool :: PoolHandle, issue_id :: String) -> Int!String do
  let _ = Pool.execute(pool, "DELETE FROM events WHERE issue_id = $1::uuid", [issue_id])?
  Pool.execute(pool, "DELETE FROM issues WHERE id = $1::uuid", [issue_id])
end

# Helper: parse event_count string to Int, defaulting to 0 on failure.
fn parse_event_count(s :: String) -> Int do
  let result = String.to_int(s)
  case result do
    Some(n) -> n
    None -> 0
  end
end

# List issues for a project filtered by status (for API listing).
# Constructs Issue structs manually with parse_event_count for the Int field.
pub fn list_issues_by_status(pool :: PoolHandle, project_id :: String, status :: String) -> List<Issue>!String do
  let rows = Pool.query(pool, "SELECT id::text, project_id::text, fingerprint, title, level, status, event_count::text, first_seen::text, last_seen::text, COALESCE(assigned_to::text, '') as assigned_to FROM issues WHERE project_id = $1::uuid AND status = $2 ORDER BY last_seen DESC", [project_id, status])?
  Ok(List.map(rows, fn(row) do
    Issue { id: Map.get(row, "id"), project_id: Map.get(row, "project_id"), fingerprint: Map.get(row, "fingerprint"), title: Map.get(row, "title"), level: Map.get(row, "level"), status: Map.get(row, "status"), event_count: parse_event_count(Map.get(row, "event_count")), first_seen: Map.get(row, "first_seen"), last_seen: Map.get(row, "last_seen"), assigned_to: Map.get(row, "assigned_to") }
  end))
end

# Spike detection: escalate archived issues with sudden volume bursts (ISSUE-03).
# If an archived issue has >10x its average hourly rate (or >10 absolute) in the
# last hour, it's auto-escalated to 'unresolved'. The WHERE status='archived'
# naturally prevents re-escalation after the first flip (research Pitfall 5).
# Returns number of escalated issues.
pub fn check_volume_spikes(pool :: PoolHandle) -> Int!String do
  Pool.execute(pool, "UPDATE issues SET status = 'unresolved' WHERE status = 'archived' AND id IN (SELECT i.id FROM issues i JOIN events e ON e.issue_id = i.id AND e.received_at > now() - interval '1 hour' WHERE i.status = 'archived' GROUP BY i.id HAVING count(*) > GREATEST(10, (SELECT count(*) FROM events e2 WHERE e2.issue_id = i.id AND e2.received_at > now() - interval '7 days') / 168 * 10))", [])
end

# Extract event fields from JSON and compute fingerprint using PostgreSQL.
# This avoids the cross-module from_json limitation (decision [88-02]) by
# computing the fingerprint server-side with the same fallback chain as
# Ingestion.Fingerprint: custom > stacktrace frames > exception type > message.
# Returns a Map with keys: fingerprint, title, level.
pub fn extract_event_fields(pool :: PoolHandle, event_json :: String) -> Map<String, String>!String do
  let sql = "SELECT CASE WHEN length(COALESCE(j->>'fingerprint', '')) > 0 THEN j->>'fingerprint' WHEN j->'stacktrace' IS NOT NULL AND jsonb_typeof(j->'stacktrace') = 'array' AND jsonb_array_length(j->'stacktrace') > 0 THEN (SELECT string_agg(frame->>'filename' || '|' || frame->>'function_name', ';' ORDER BY ordinality) FROM jsonb_array_elements(j->'stacktrace') WITH ORDINALITY AS t(frame, ordinality)) || ':' || lower(COALESCE(replace(j->>'message', '0x', ''), '')) WHEN j->'exception' IS NOT NULL AND j->'exception'->>'type_name' IS NOT NULL THEN (j->'exception'->>'type_name') || ':' || lower(COALESCE(replace(j->'exception'->>'value', '0x', ''), '')) ELSE 'msg:' || lower(COALESCE(replace(j->>'message', '0x', ''), '')) END AS fingerprint, COALESCE(NULLIF(j->>'message', ''), 'Untitled') AS title, COALESCE(j->>'level', 'error') AS level FROM (SELECT $1::jsonb AS j) AS sub"
  let rows = Pool.query(pool, sql, [event_json])?
  if List.length(rows) > 0 do
    Ok(List.head(rows))
  else
    Err("extract_event_fields: no result")
  end
end

# --- Search, filter, and pagination queries (Phase 91 Plan 01) ---

# SEARCH-01 + SEARCH-05: List issues with optional filters and keyset pagination.
# Optional filters use SQL-side conditionals ($N = '' OR column = $N) to avoid injection.
# Keyset pagination uses (last_seen, id) < ($cursor, $cursor_id) for stable browsing.
# Returns raw Map rows (not Issue struct) for flexible JSON serialization.
pub fn list_issues_filtered(pool :: PoolHandle, project_id :: String, status :: String, level :: String, assigned_to :: String, cursor :: String, cursor_id :: String, limit_str :: String) -> List<Map<String, String>>!String do
  if String.length(cursor) > 0 do
    let sql = "SELECT id::text, project_id::text, fingerprint, title, level, status, event_count::text, first_seen::text, last_seen::text, COALESCE(assigned_to::text, '') as assigned_to FROM issues WHERE project_id = $1::uuid AND ($2 = '' OR status = $2) AND ($3 = '' OR level = $3) AND ($4 = '' OR assigned_to = $4::uuid) AND (last_seen, id) < ($5::timestamptz, $6::uuid) ORDER BY last_seen DESC, id DESC LIMIT $7::int"
    let rows = Pool.query(pool, sql, [project_id, status, level, assigned_to, cursor, cursor_id, limit_str])?
    Ok(rows)
  else
    let sql = "SELECT id::text, project_id::text, fingerprint, title, level, status, event_count::text, first_seen::text, last_seen::text, COALESCE(assigned_to::text, '') as assigned_to FROM issues WHERE project_id = $1::uuid AND ($2 = '' OR status = $2) AND ($3 = '' OR level = $3) AND ($4 = '' OR assigned_to = $4::uuid) ORDER BY last_seen DESC, id DESC LIMIT $5::int"
    let rows = Pool.query(pool, sql, [project_id, status, level, assigned_to, limit_str])?
    Ok(rows)
  end
end

# SEARCH-02: Full-text search on event messages using inline tsvector.
# Uses inline to_tsvector (avoids partition complications with stored tsvector column).
# Includes 24-hour default time range (SEARCH-04) for partition pruning.
# Returns relevance rank for ordering.
pub fn search_events_fulltext(pool :: PoolHandle, project_id :: String, search_query :: String, limit_str :: String) -> List<Map<String, String>>!String do
  let sql = "SELECT id::text, issue_id::text, level, message, received_at::text, ts_rank(to_tsvector('english', message), plainto_tsquery('english', $2))::text AS rank FROM events WHERE project_id = $1::uuid AND to_tsvector('english', message) @@ plainto_tsquery('english', $2) AND received_at > now() - interval '24 hours' ORDER BY rank DESC, received_at DESC LIMIT $3::int"
  let rows = Pool.query(pool, sql, [project_id, search_query, limit_str])?
  Ok(rows)
end

# SEARCH-03: Filter events by tag key-value pair using JSONB containment.
# Uses tags @> $2::jsonb operator which leverages existing GIN index (idx_events_tags).
# Includes 24-hour default time range (SEARCH-04).
pub fn filter_events_by_tag(pool :: PoolHandle, project_id :: String, tag_json :: String, limit_str :: String) -> List<Map<String, String>>!String do
  let sql = "SELECT id::text, issue_id::text, level, message, tags::text, received_at::text FROM events WHERE project_id = $1::uuid AND tags @> $2::jsonb AND received_at > now() - interval '24 hours' ORDER BY received_at DESC LIMIT $3::int"
  let rows = Pool.query(pool, sql, [project_id, tag_json, limit_str])?
  Ok(rows)
end

# Event listing within an issue with keyset pagination (for DETAIL-05 context).
# Keyset pagination on (received_at, id) for stable browsing.
pub fn list_events_for_issue(pool :: PoolHandle, issue_id :: String, cursor :: String, cursor_id :: String, limit_str :: String) -> List<Map<String, String>>!String do
  if String.length(cursor) > 0 do
    let sql = "SELECT id::text, level, message, received_at::text FROM events WHERE issue_id = $1::uuid AND (received_at, id) < ($2::timestamptz, $3::uuid) ORDER BY received_at DESC, id DESC LIMIT $4::int"
    let rows = Pool.query(pool, sql, [issue_id, cursor, cursor_id, limit_str])?
    Ok(rows)
  else
    let sql = "SELECT id::text, level, message, received_at::text FROM events WHERE issue_id = $1::uuid ORDER BY received_at DESC, id DESC LIMIT $2::int"
    let rows = Pool.query(pool, sql, [issue_id, limit_str])?
    Ok(rows)
  end
end

# --- Dashboard aggregation queries (Phase 91 Plan 02) ---

# DASH-01: Event volume bucketed by hour or day for a project.
# bucket param is either "hour" or "day" (passed from handler).
# Default 24-hour time window.
pub fn event_volume_hourly(pool :: PoolHandle, project_id :: String, bucket :: String) -> List<Map<String, String>>!String do
  let sql = "SELECT date_trunc($2, received_at)::text AS bucket, count(*)::text AS count FROM events WHERE project_id = $1::uuid AND received_at > now() - interval '24 hours' GROUP BY bucket ORDER BY bucket"
  let rows = Pool.query(pool, sql, [project_id, bucket])?
  Ok(rows)
end

# DASH-02: Error breakdown by severity level for a project.
# Groups events by level (error, warning, info, etc.) with counts.
pub fn error_breakdown_by_level(pool :: PoolHandle, project_id :: String) -> List<Map<String, String>>!String do
  let sql = "SELECT level, count(*)::text AS count FROM events WHERE project_id = $1::uuid AND received_at > now() - interval '24 hours' GROUP BY level ORDER BY count DESC"
  let rows = Pool.query(pool, sql, [project_id])?
  Ok(rows)
end

# DASH-03: Top issues ranked by frequency (event count).
# Returns unresolved issues ordered by event_count DESC.
pub fn top_issues_by_frequency(pool :: PoolHandle, project_id :: String, limit_str :: String) -> List<Map<String, String>>!String do
  let sql = "SELECT id::text, title, level, status, event_count::text, last_seen::text FROM issues WHERE project_id = $1::uuid AND status = 'unresolved' ORDER BY event_count DESC LIMIT $2::int"
  let rows = Pool.query(pool, sql, [project_id, limit_str])?
  Ok(rows)
end

# DASH-04: Event breakdown by tag key (environment, release, etc.).
# Uses JSONB key-exists operator to filter events that have the specified tag.
pub fn event_breakdown_by_tag(pool :: PoolHandle, project_id :: String, tag_key :: String) -> List<Map<String, String>>!String do
  let sql = "SELECT tags->>$2 AS tag_value, count(*)::text AS count FROM events WHERE project_id = $1::uuid AND received_at > now() - interval '24 hours' AND tags ? $2 GROUP BY tag_value ORDER BY count DESC LIMIT 20"
  let rows = Pool.query(pool, sql, [project_id, tag_key])?
  Ok(rows)
end

# DASH-05: Per-issue event timeline (recent events for a specific issue).
# Ordered by received_at DESC for chronological browsing.
pub fn issue_event_timeline(pool :: PoolHandle, issue_id :: String, limit_str :: String) -> List<Map<String, String>>!String do
  let sql = "SELECT id::text, level, message, received_at::text FROM events WHERE issue_id = $1::uuid ORDER BY received_at DESC LIMIT $2::int"
  let rows = Pool.query(pool, sql, [issue_id, limit_str])?
  Ok(rows)
end

# DASH-06: Project health summary with key metrics.
# Returns single row: unresolved issue count, events in last 24h, new issues today.
pub fn project_health_summary(pool :: PoolHandle, project_id :: String) -> List<Map<String, String>>!String do
  let sql = "SELECT (SELECT count(*) FROM issues WHERE project_id = $1::uuid AND status = 'unresolved')::text AS unresolved_count, (SELECT count(*) FROM events WHERE project_id = $1::uuid AND received_at > now() - interval '24 hours')::text AS events_24h, (SELECT count(*) FROM issues WHERE project_id = $1::uuid AND first_seen > now() - interval '24 hours')::text AS new_today"
  let rows = Pool.query(pool, sql, [project_id])?
  Ok(rows)
end

# --- Event detail queries (Phase 91 Plan 02) ---

# DETAIL-01..04, DETAIL-06: Get complete event with all JSONB fields.
# Returns full event payload including exception, stacktrace, breadcrumbs,
# tags, extra, user_context. JSONB fields use COALESCE for null safety.
pub fn get_event_detail(pool :: PoolHandle, event_id :: String) -> List<Map<String, String>>!String do
  let sql = "SELECT id::text, project_id::text, issue_id::text, level, message, fingerprint, COALESCE(exception::text, 'null') AS exception, COALESCE(stacktrace::text, '[]') AS stacktrace, COALESCE(breadcrumbs::text, '[]') AS breadcrumbs, COALESCE(tags::text, '{}') AS tags, COALESCE(extra::text, '{}') AS extra, COALESCE(user_context::text, 'null') AS user_context, COALESCE(sdk_name, '') AS sdk_name, COALESCE(sdk_version, '') AS sdk_version, received_at::text FROM events WHERE id = $1::uuid"
  let rows = Pool.query(pool, sql, [event_id])?
  Ok(rows)
end

# DETAIL-05: Get next and previous event IDs within an issue for navigation.
# Uses tuple comparison (received_at, id) for stable ordering.
pub fn get_event_neighbors(pool :: PoolHandle, issue_id :: String, received_at :: String, event_id :: String) -> List<Map<String, String>>!String do
  let sql = "SELECT (SELECT id::text FROM events WHERE issue_id = $1::uuid AND (received_at, id) > ($2::timestamptz, $3::uuid) ORDER BY received_at, id LIMIT 1) AS next_id, (SELECT id::text FROM events WHERE issue_id = $1::uuid AND (received_at, id) < ($2::timestamptz, $3::uuid) ORDER BY received_at DESC, id DESC LIMIT 1) AS prev_id"
  let rows = Pool.query(pool, sql, [issue_id, received_at, event_id])?
  Ok(rows)
end

# --- Team management queries (Phase 91 Plan 03 -- ORG-04) ---

# Update a member's role. SQL-side validation ensures only valid roles accepted.
# Returns affected row count (0 if invalid role or membership not found).
pub fn update_member_role(pool :: PoolHandle, membership_id :: String, new_role :: String) -> Int!String do
  Pool.execute(pool, "UPDATE org_memberships SET role = $2 WHERE id = $1::uuid AND $2 IN ('owner', 'admin', 'member')", [membership_id, new_role])
end

# Remove a member from an organization.
# Returns affected row count (0 if membership not found).
pub fn remove_member(pool :: PoolHandle, membership_id :: String) -> Int!String do
  Pool.execute(pool, "DELETE FROM org_memberships WHERE id = $1::uuid", [membership_id])
end

# List all members of an organization with user info (email, display_name).
# JOIN with users table for enriched member listing.
# Returns raw Map rows for flexible JSON serialization.
pub fn get_members_with_users(pool :: PoolHandle, org_id :: String) -> List<Map<String, String>>!String do
  let sql = "SELECT m.id::text, m.user_id::text, m.org_id::text, m.role, m.joined_at::text, u.email, u.display_name FROM org_memberships m JOIN users u ON u.id = m.user_id WHERE m.org_id = $1::uuid ORDER BY m.joined_at"
  let rows = Pool.query(pool, sql, [org_id])?
  Ok(rows)
end

# --- API token management queries (Phase 91 Plan 03 -- ORG-05) ---

# List all API keys for a project with full details.
# Returns raw Map rows. revoked_at is empty string if not revoked.
pub fn list_api_keys(pool :: PoolHandle, project_id :: String) -> List<Map<String, String>>!String do
  let sql = "SELECT id::text, project_id::text, key_value, label, created_at::text, COALESCE(revoked_at::text, '') AS revoked_at FROM api_keys WHERE project_id = $1::uuid ORDER BY created_at DESC"
  let rows = Pool.query(pool, sql, [project_id])?
  Ok(rows)
end
