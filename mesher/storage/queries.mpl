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
