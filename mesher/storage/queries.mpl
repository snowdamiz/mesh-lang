# Reusable query helper functions for all Mesher entity types.
# Provides CRUD operations using Pool.query and Pool.query_as.
# All functions take the pool handle as first argument.

module Storage.Queries

from Types.Project import { Organization, Project, ApiKey }
from Types.User import { User, OrgMembership, Session }
from Types.Issue import { Issue }
from Types.Event import { Event }
from Types.Alert import { AlertRule }

# --- Organization queries ---

# Insert a new organization. Returns the generated UUID.
pub fn insert_org(pool :: Int, name :: String, slug :: String) -> String!String do
  let rows = Pool.query(pool,
    "INSERT INTO organizations (name, slug) VALUES ($1, $2) RETURNING id::text",
    [name, slug])?
  case rows do
    [row] -> Ok(Map.get(row, "id"))
    _ -> Err("insert_org: no id returned")
  end
end

# Get an organization by ID.
pub fn get_org(pool :: Int, id :: String) -> Organization!String do
  let results = Pool.query_as(pool,
    "SELECT id::text, name, slug, created_at::text FROM organizations WHERE id = $1::uuid",
    [id],
    Organization.from_row)?
  case results do
    [item] -> Ok(item)
    _ -> Err("not found")
  end
end

# List all organizations.
pub fn list_orgs(pool :: Int) -> List<Organization>!String do
  Pool.query_as(pool,
    "SELECT id::text, name, slug, created_at::text FROM organizations ORDER BY name",
    [],
    Organization.from_row)
end

# --- Project queries ---

# Insert a new project. Returns the generated UUID.
pub fn insert_project(pool :: Int, org_id :: String, name :: String, platform :: String) -> String!String do
  let rows = Pool.query(pool,
    "INSERT INTO projects (org_id, name, platform) VALUES ($1::uuid, $2, $3) RETURNING id::text",
    [org_id, name, platform])?
  case rows do
    [row] -> Ok(Map.get(row, "id"))
    _ -> Err("insert_project: no id returned")
  end
end

# Get a project by ID.
pub fn get_project(pool :: Int, id :: String) -> Project!String do
  let results = Pool.query_as(pool,
    "SELECT id::text, org_id::text, name, platform, created_at::text FROM projects WHERE id = $1::uuid",
    [id],
    Project.from_row)?
  case results do
    [item] -> Ok(item)
    _ -> Err("not found")
  end
end

# List all projects for an organization.
pub fn list_projects_by_org(pool :: Int, org_id :: String) -> List<Project>!String do
  Pool.query_as(pool,
    "SELECT id::text, org_id::text, name, platform, created_at::text FROM projects WHERE org_id = $1::uuid ORDER BY name",
    [org_id],
    Project.from_row)
end

# --- API key queries ---

# Create a new API key for a project. Returns the generated key_value (mshr_ prefixed).
pub fn create_api_key(pool :: Int, project_id :: String, label :: String) -> String!String do
  let rows = Pool.query(pool,
    "INSERT INTO api_keys (project_id, key_value, label) VALUES ($1::uuid, 'mshr_' || encode(gen_random_bytes(24), 'hex'), $2) RETURNING key_value",
    [project_id, label])?
  case rows do
    [row] -> Ok(Map.get(row, "key_value"))
    _ -> Err("create_api_key: no key returned")
  end
end

# Get the project associated with a valid (non-revoked) API key.
pub fn get_project_by_api_key(pool :: Int, key_value :: String) -> Project!String do
  let results = Pool.query_as(pool,
    "SELECT p.id::text, p.org_id::text, p.name, p.platform, p.created_at::text FROM projects p JOIN api_keys ak ON ak.project_id = p.id WHERE ak.key_value = $1 AND ak.revoked_at IS NULL",
    [key_value],
    Project.from_row)?
  case results do
    [item] -> Ok(item)
    _ -> Err("not found")
  end
end

# Revoke an API key by setting revoked_at to now().
pub fn revoke_api_key(pool :: Int, key_id :: String) -> Int!String do
  Pool.execute(pool,
    "UPDATE api_keys SET revoked_at = now() WHERE id = $1::uuid",
    [key_id])
end

# --- User queries ---

# Create a new user with bcrypt password hashing via pgcrypto (cost factor 12).
pub fn create_user(pool :: Int, email :: String, password :: String, display_name :: String) -> String!String do
  let rows = Pool.query(pool,
    "INSERT INTO users (email, password_hash, display_name) VALUES ($1, crypt($2, gen_salt('bf', 12)), $3) RETURNING id::text",
    [email, password, display_name])?
  case rows do
    [row] -> Ok(Map.get(row, "id"))
    _ -> Err("create_user: no id returned")
  end
end

# Authenticate a user by email and password.
# Returns the User if credentials match, Err("not found") otherwise.
pub fn authenticate_user(pool :: Int, email :: String, password :: String) -> User!String do
  let results = Pool.query_as(pool,
    "SELECT id::text, email, display_name, created_at::text FROM users WHERE email = $1 AND password_hash = crypt($2, password_hash)",
    [email, password],
    User.from_row)?
  case results do
    [item] -> Ok(item)
    _ -> Err("not found")
  end
end

# Get a user by ID.
pub fn get_user(pool :: Int, id :: String) -> User!String do
  let results = Pool.query_as(pool,
    "SELECT id::text, email, display_name, created_at::text FROM users WHERE id = $1::uuid",
    [id],
    User.from_row)?
  case results do
    [item] -> Ok(item)
    _ -> Err("not found")
  end
end

# --- Session queries ---

# Create a new session with a cryptographically random token.
# Returns the 64-char hex token.
pub fn create_session(pool :: Int, user_id :: String) -> String!String do
  let rows = Pool.query(pool,
    "INSERT INTO sessions (token, user_id) VALUES (encode(gen_random_bytes(32), 'hex'), $1::uuid) RETURNING token",
    [user_id])?
  case rows do
    [row] -> Ok(Map.get(row, "token"))
    _ -> Err("create_session: no token returned")
  end
end

# Validate a session token. Returns the Session if valid and not expired.
pub fn validate_session(pool :: Int, token :: String) -> Session!String do
  let results = Pool.query_as(pool,
    "SELECT token, user_id::text, created_at::text, expires_at::text FROM sessions WHERE token = $1 AND expires_at > now()",
    [token],
    Session.from_row)?
  case results do
    [item] -> Ok(item)
    _ -> Err("not found")
  end
end

# Delete a session by token (logout).
pub fn delete_session(pool :: Int, token :: String) -> Int!String do
  Pool.execute(pool,
    "DELETE FROM sessions WHERE token = $1",
    [token])
end

# --- Org membership queries ---

# Add a user to an organization with a role (owner/admin/member).
pub fn add_member(pool :: Int, user_id :: String, org_id :: String, role :: String) -> String!String do
  let rows = Pool.query(pool,
    "INSERT INTO org_memberships (user_id, org_id, role) VALUES ($1::uuid, $2::uuid, $3) RETURNING id::text",
    [user_id, org_id, role])?
  case rows do
    [row] -> Ok(Map.get(row, "id"))
    _ -> Err("add_member: no id returned")
  end
end

# Get all members of an organization.
pub fn get_members(pool :: Int, org_id :: String) -> List<OrgMembership>!String do
  Pool.query_as(pool,
    "SELECT id::text, user_id::text, org_id::text, role, joined_at::text FROM org_memberships WHERE org_id = $1::uuid",
    [org_id],
    OrgMembership.from_row)
end

# Get all organizations a user belongs to.
pub fn get_user_orgs(pool :: Int, user_id :: String) -> List<OrgMembership>!String do
  Pool.query_as(pool,
    "SELECT id::text, user_id::text, org_id::text, role, joined_at::text FROM org_memberships WHERE user_id = $1::uuid",
    [user_id],
    OrgMembership.from_row)
end
