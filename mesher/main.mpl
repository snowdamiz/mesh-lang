# Mesher monitoring platform entry point.
# Connects to PostgreSQL, creates schema and partitions, starts all services.
# This is the foundation layer -- Phase 88 (Ingestion Pipeline) will add HTTP.serve.
#
# Service definitions live in main.mpl because Mesh's module export system does
# not yet support cross-module service resolution. Services register their methods
# (start, call helpers, cast helpers) in the local typechecker environment; these
# are not included in ModuleExports. All services are thin actor wrappers that
# delegate to the exported query functions in Storage.Queries.
#
# StorageWriter batch logic also lives in main.mpl because functions with
# polymorphic (inferred) type parameters cannot be imported cross-module --
# the typechecker's type variable IDs are module-scoped. Only insert_event
# (with fully concrete parameters) is imported from Storage.Writer.

from Storage.Schema import create_schema, create_partitions_ahead
from Storage.Queries import insert_org, get_org, list_orgs, insert_project, get_project, list_projects_by_org, create_api_key, get_project_by_api_key, revoke_api_key, create_user, authenticate_user, get_user, create_session, validate_session, delete_session, add_member, get_members
from Storage.Writer import insert_event
from Types.Project import Organization, Project
from Types.User import User, Session, OrgMembership

# WriterState holds all service state for a single project writer.
# Buffer stores JSON-encoded event strings (List<String>) -- PostgreSQL parses
# the JSON server-side during INSERT (see Storage.Writer.insert_event).
struct WriterState do
  pool :: PoolHandle
  project_id :: String
  buffer :: List<String>
  buffer_len :: Int
  batch_size :: Int
  max_buffer :: Int
end

# Helper function for two-step login: authenticate then create session.
# Extracted from UserService.Login handler to avoid complex case expressions
# inside service call bodies (LLVM codegen limitation with Result pattern
# matching inside service dispatch handlers).
fn login_user(pool :: PoolHandle, email :: String, password :: String) -> String!String do
  let auth_result = authenticate_user(pool, email, password)
  case auth_result do
    Ok(user) -> create_session(pool, user.id)
    Err(_) -> Err("authentication failed")
  end
end

# --- Batch flush and retry logic ---
# LOCKED: 3 attempts with exponential backoff (100ms, 500ms), drop on final failure.
# Uses recursive loop since Mesh has no mutable variable assignment.

fn flush_loop(pool :: PoolHandle, project_id :: String, events, i :: Int, total :: Int) -> Int!String do
  if i < total do
    let event_json = List.get(events, i)
    let r = insert_event(pool, project_id, event_json)
    case r do
      Ok(_) -> flush_loop(pool, project_id, events, i + 1, total)
      Err(_) -> Err("flush failed")
    end
  else
    Ok(0)
  end
end

fn flush_batch(pool :: PoolHandle, project_id :: String, events) -> Int!String do
  let total = List.length(events)
  flush_loop(pool, project_id, events, 0, total)
end

fn flush_drop(project_id :: String, count_val :: Int) -> Int!String do
  println("[StorageWriter] Dropping batch of " <> String.from(count_val) <> " events for project " <> project_id <> " after 3 retries")
  Ok(0)
end

fn flush_retry3(pool :: PoolHandle, project_id :: String, events, event_count :: Int) -> Int!String do
  Timer.sleep(500)
  let r3 = flush_batch(pool, project_id, events)
  case r3 do
    Ok(n) -> Ok(n)
    Err(_) -> flush_drop(project_id, event_count)
  end
end

fn flush_retry2(pool :: PoolHandle, project_id :: String, events, event_count :: Int) -> Int!String do
  Timer.sleep(100)
  let r2 = flush_batch(pool, project_id, events)
  case r2 do
    Ok(n) -> Ok(n)
    Err(_) -> flush_retry3(pool, project_id, events, event_count)
  end
end

fn flush_with_retry(pool :: PoolHandle, project_id :: String, events) -> Int!String do
  let event_count = List.length(events)
  let r1 = flush_batch(pool, project_id, events)
  case r1 do
    Ok(n) -> Ok(n)
    Err(_) -> flush_retry2(pool, project_id, events, event_count)
  end
end

# --- Buffer management helpers ---
# Kept as standalone functions so cast handler bodies remain minimal
# (avoids complex expressions inside service dispatch codegen).

fn writer_store(state :: WriterState, event_json :: String) -> WriterState do
  let appended = List.append(state.buffer, event_json)
  let new_len = state.buffer_len + 1
  # Drop oldest if over capacity (LOCKED: drop-oldest backpressure)
  let buf = if new_len > state.max_buffer do List.drop(appended, new_len - state.max_buffer) else appended end
  let blen = if new_len > state.max_buffer do state.max_buffer else new_len end
  # Flush if batch size reached (LOCKED: size trigger)
  if blen >= state.batch_size do
    let _ = flush_with_retry(state.pool, state.project_id, buf)
    WriterState { pool: state.pool, project_id: state.project_id, buffer: List.new(), buffer_len: 0, batch_size: state.batch_size, max_buffer: state.max_buffer }
  else
    WriterState { pool: state.pool, project_id: state.project_id, buffer: buf, buffer_len: blen, batch_size: state.batch_size, max_buffer: state.max_buffer }
  end
end

fn writer_flush(state :: WriterState) -> WriterState do
  if state.buffer_len > 0 do
    let _ = flush_with_retry(state.pool, state.project_id, state.buffer)
    WriterState { pool: state.pool, project_id: state.project_id, buffer: List.new(), buffer_len: 0, batch_size: state.batch_size, max_buffer: state.max_buffer }
  else
    state
  end
end

# --- Organization Service ---
# Thin actor wrapper for org CRUD. 3 call handlers.

service OrgService do
  fn init(pool :: PoolHandle) -> PoolHandle do
    pool
  end

  call CreateOrg(name :: String, slug :: String) do |pool|
    let result = insert_org(pool, name, slug)
    (pool, result)
  end

  call GetOrg(id :: String) do |pool|
    let result = get_org(pool, id)
    (pool, result)
  end

  call ListOrgs() do |pool|
    let result = list_orgs(pool)
    (pool, result)
  end
end

# --- Project Service ---
# Thin actor wrapper for project CRUD and API key management. 6 call handlers.

service ProjectService do
  fn init(pool :: PoolHandle) -> PoolHandle do
    pool
  end

  call CreateProject(org_id :: String, name :: String, platform :: String) do |pool|
    let result = insert_project(pool, org_id, name, platform)
    (pool, result)
  end

  call GetProject(id :: String) do |pool|
    let result = get_project(pool, id)
    (pool, result)
  end

  call ListProjectsByOrg(org_id :: String) do |pool|
    let result = list_projects_by_org(pool, org_id)
    (pool, result)
  end

  call CreateApiKey(project_id :: String, label :: String) do |pool|
    let result = create_api_key(pool, project_id, label)
    (pool, result)
  end

  call GetProjectByApiKey(key_value :: String) do |pool|
    let result = get_project_by_api_key(pool, key_value)
    (pool, result)
  end

  call RevokeApiKey(key_id :: String) do |pool|
    let result = revoke_api_key(pool, key_id)
    (pool, result)
  end
end

# --- User Service ---
# Thin actor wrapper for user auth, sessions, and org membership. 7 call handlers.
# Login is two-step: authenticate credentials, then create session.

service UserService do
  fn init(pool :: PoolHandle) -> PoolHandle do
    pool
  end

  call Register(email :: String, password :: String, display_name :: String) do |pool|
    let result = create_user(pool, email, password, display_name)
    (pool, result)
  end

  call Login(email :: String, password :: String) do |pool|
    let result = login_user(pool, email, password)
    (pool, result)
  end

  call ValidateSession(token :: String) do |pool|
    let result = validate_session(pool, token)
    (pool, result)
  end

  call Logout(token :: String) do |pool|
    let result = delete_session(pool, token)
    (pool, result)
  end

  call GetUser(id :: String) do |pool|
    let result = get_user(pool, id)
    (pool, result)
  end

  call AddMember(user_id :: String, org_id :: String, role :: String) do |pool|
    let result = add_member(pool, user_id, org_id, role)
    (pool, result)
  end

  call GetMembers(org_id :: String) do |pool|
    let result = get_members(pool, org_id)
    (pool, result)
  end
end

# --- Storage Writer Service ---
# Per-project batch writer with bounded buffer and dual flush triggers.
# LOCKED: per-project writers for isolation, drop-oldest backpressure,
# size + timer flush triggers, retry with exponential backoff.
# insert_event imported from Storage.Writer. All other logic is local
# to avoid cross-module polymorphic type variable scoping issues.
# Buffer stores JSON-encoded event strings; PostgreSQL parses JSON server-side.

service StorageWriter do
  fn init(pool :: PoolHandle, project_id :: String) -> WriterState do
    WriterState {
      pool: pool,
      project_id: project_id,
      buffer: List.new(),
      buffer_len: 0,
      batch_size: 50,
      max_buffer: 500
    }
  end

  # Store a JSON-encoded event string in the buffer.
  # Drops oldest events if buffer exceeds capacity.
  # Triggers immediate flush if buffer reaches batch_size threshold.
  cast Store(event_json) do |state|
    writer_store(state, event_json)
  end

  # Flush all buffered events to PostgreSQL. Called by ticker actor on timer.
  cast Flush() do |state|
    writer_flush(state)
  end
end

# Ticker actor for periodic flush (LOCKED: timer flush trigger).
# Uses Timer.sleep + recursive call because Timer.send_after delivers raw bytes
# that cannot match service cast dispatch tags (type_tag-based dispatch).
# Spawned alongside each StorageWriter to provide the timer-based flush trigger.
actor flush_ticker(writer_pid, interval :: Int) do
  Timer.sleep(interval)
  StorageWriter.flush(writer_pid)
  flush_ticker(writer_pid, interval)
end

# --- Main Entry Point ---

fn start_services(pool :: PoolHandle) do
  # Run schema creation (idempotent -- all CREATE IF NOT EXISTS)
  let schema_result = create_schema(pool)
  case schema_result do
    Ok(_) -> println("[Mesher] Schema created/verified")
    Err(_) -> println("[Mesher] Schema error")
  end

  # Create initial partitions (7 days ahead)
  let partition_result = create_partitions_ahead(pool, 7)
  case partition_result do
    Ok(_) -> println("[Mesher] Partitions created (7 days ahead)")
    Err(_) -> println("[Mesher] Partition error")
  end

  # Start services
  let org_svc = OrgService.start(pool)
  println("[Mesher] OrgService started")

  let project_svc = ProjectService.start(pool)
  println("[Mesher] ProjectService started")

  let user_svc = UserService.start(pool)
  println("[Mesher] UserService started")

  println("[Mesher] Foundation ready")

  # Keep the main process alive (services run as actors).
  # In Phase 88, this will be replaced by HTTP.serve which blocks.
  Timer.sleep(999999999)
end

fn main() do
  println("[Mesher] Connecting to PostgreSQL...")
  let pool_result = Pool.open("postgres://mesh:mesh@localhost:5432/mesher", 2, 10, 5000)
  case pool_result do
    Ok(pool) -> start_services(pool)
    Err(_) -> println("[Mesher] Failed to connect to PostgreSQL")
  end
end
