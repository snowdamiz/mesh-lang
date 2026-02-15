# HTTP route handlers for the ingestion API.
# Handlers are bare functions (HTTP routing does not support closures).
# Service PIDs and pool handle are obtained via the PipelineRegistry service,
# which is looked up by name using Process.whereis.

from Ingestion.Auth import authenticate_request
from Ingestion.Validation import validate_payload_size
from Ingestion.Pipeline import PipelineRegistry
from Services.RateLimiter import RateLimiter
from Services.EventProcessor import EventProcessor
from Types.Project import Project
from Types.Issue import Issue
from Storage.Queries import resolve_issue, archive_issue, unresolve_issue, assign_issue, discard_issue, delete_issue, list_issues_by_status

# Helper: build 401 response
fn unauthorized_response() do
  HTTP.response(401, "{\"error\":\"unauthorized\"}")
end

# Helper: build 400 response with reason
fn bad_request_response(reason :: String) do
  HTTP.response(400, "{\"error\":\"" <> reason <> "\"}")
end

# Helper: build 429 rate-limited response with Retry-After header
fn rate_limited_response() do
  let empty_headers = Map.new()
  let headers = Map.put(empty_headers, "Retry-After", "60")
  HTTP.response_with_headers(429, "{\"error\":\"rate limited\"}", headers)
end

# Helper: build 202 accepted response
fn accepted_response() do
  HTTP.response(202, "{\"status\":\"accepted\"}")
end

# --- Event broadcasting helpers (STREAM-01, STREAM-04) ---
# Defined before route_to_processor (Mesh requires define-before-use).

# Helper: broadcast issue count from query result rows
fn broadcast_count_from_rows(project_id :: String, rows) do
  if List.length(rows) > 0 do
    let count = Map.get(List.head(rows), "cnt")
    let room = "project:" <> project_id
    let _ = Ws.broadcast(room, "{\"type\":\"issue_count\",\"project_id\":\"" <> project_id <> "\",\"count\":" <> count <> "}")
    0
  else
    0
  end
end

# Helper: broadcast updated issue count for a project
fn broadcast_issue_count(project_id :: String) do
  let reg_pid = Process.whereis("mesher_registry")
  let pool = PipelineRegistry.get_pool(reg_pid)
  let count_result = Pool.query(pool, "SELECT count(*)::text AS cnt FROM issues WHERE project_id = $1::uuid AND status = 'unresolved'", [project_id])
  case count_result do
    Ok(rows) -> broadcast_count_from_rows(project_id, rows)
    Err(_) -> 0
  end
end

# Helper: broadcast event notification and issue count update to project room
fn broadcast_event(project_id :: String, issue_id :: String, body :: String) do
  let room = "project:" <> project_id
  let notification = "{\"type\":\"event\",\"issue_id\":\"" <> issue_id <> "\",\"data\":" <> body <> "}"
  let _ = Ws.broadcast(room, notification)
  broadcast_issue_count(project_id)
  accepted_response()
end

# Helper: route event to processor, broadcast on success, and build response
fn route_to_processor(processor_pid, project_id :: String, writer_pid, body :: String) do
  let result = EventProcessor.process_event(processor_pid, project_id, writer_pid, body)
  case result do
    Ok(issue_id) -> broadcast_event(project_id, issue_id, body)
    Err(reason) -> bad_request_response(reason)
  end
end

# Helper: process a single event after auth and rate-limit checks pass.
fn process_event_body(processor_pid, project_id :: String, writer_pid, body :: String) do
  let size_check = validate_payload_size(body, 1048576)
  case size_check do
    Err(reason) -> bad_request_response(reason)
    Ok(_) -> route_to_processor(processor_pid, project_id, writer_pid, body)
  end
end

# Helper: handle event after authentication succeeds
fn handle_event_authed(project_id :: String, rate_limiter_pid, processor_pid, writer_pid, request) do
  let allowed = RateLimiter.check_limit(rate_limiter_pid, project_id)
  if allowed do
    let body = Request.body(request)
    process_event_body(processor_pid, project_id, writer_pid, body)
  else
    rate_limited_response()
  end
end

# Handle POST /api/v1/events
# Flow: get registry -> get pool+pids -> auth -> rate limit -> validate -> process -> 202
pub fn handle_event(request) do
  let reg_pid = Process.whereis("mesher_registry")
  let pool = PipelineRegistry.get_pool(reg_pid)
  let rate_limiter_pid = PipelineRegistry.get_rate_limiter(reg_pid)
  let processor_pid = PipelineRegistry.get_processor(reg_pid)
  let writer_pid = PipelineRegistry.get_writer(reg_pid)
  let auth_result = authenticate_request(pool, request)
  case auth_result do
    Err(_) -> unauthorized_response()
    Ok(project) -> handle_event_authed(project.id, rate_limiter_pid, processor_pid, writer_pid, request)
  end
end

# Helper: handle bulk after authentication succeeds.
# Validates size (5MB limit for bulk), then routes the entire bulk payload
# to EventProcessor for persistence. Individual JSON array element parsing
# is not supported at the Mesh language level; the StorageWriter stores
# the complete bulk JSON for downstream processing.
fn handle_bulk_authed(project_id :: String, rate_limiter_pid, processor_pid, writer_pid, request) do
  let allowed = RateLimiter.check_limit(rate_limiter_pid, project_id)
  if allowed do
    let body = Request.body(request)
    let size_check = validate_payload_size(body, 5242880)
    case size_check do
      Err(reason) -> bad_request_response(reason)
      Ok(_) -> route_to_processor(processor_pid, project_id, writer_pid, body)
    end
  else
    rate_limited_response()
  end
end

# Handle POST /api/v1/events/bulk
pub fn handle_bulk(request) do
  let reg_pid = Process.whereis("mesher_registry")
  let pool = PipelineRegistry.get_pool(reg_pid)
  let rate_limiter_pid = PipelineRegistry.get_rate_limiter(reg_pid)
  let processor_pid = PipelineRegistry.get_processor(reg_pid)
  let writer_pid = PipelineRegistry.get_writer(reg_pid)
  let auth_result = authenticate_request(pool, request)
  case auth_result do
    Err(_) -> unauthorized_response()
    Ok(project) -> handle_bulk_authed(project.id, rate_limiter_pid, processor_pid, writer_pid, request)
  end
end

# --- Issue state change broadcasting helpers (STREAM-03) ---
# Defined before issue management handlers (Mesh requires define-before-use).

# Helper: broadcast issue update from project lookup rows
fn broadcast_update_from_rows(rows, issue_id :: String, action :: String) do
  if List.length(rows) > 0 do
    let project_id = Map.get(List.head(rows), "project_id")
    let room = "project:" <> project_id
    let msg = "{\"type\":\"issue\",\"action\":\"" <> action <> "\",\"issue_id\":\"" <> issue_id <> "\"}"
    let _ = Ws.broadcast(room, msg)
    0
  else
    0
  end
end

# Helper: look up project_id for an issue and broadcast state change notification
fn broadcast_issue_update(pool, issue_id :: String, action :: String) do
  let rows_result = Pool.query(pool, "SELECT project_id::text FROM issues WHERE id = $1::uuid", [issue_id])
  case rows_result do
    Ok(rows) -> broadcast_update_from_rows(rows, issue_id, action)
    Err(_) -> 0
  end
end

# Helper: broadcast resolve notification then return success response
fn resolve_success(pool, issue_id :: String, n :: Int) do
  let _ = broadcast_issue_update(pool, issue_id, "resolved")
  HTTP.response(200, "{\"status\":\"ok\",\"affected\":" <> String.from(n) <> "}")
end

# Helper: broadcast archive notification then return success response
fn archive_success(pool, issue_id :: String, n :: Int) do
  let _ = broadcast_issue_update(pool, issue_id, "archived")
  HTTP.response(200, "{\"status\":\"ok\",\"affected\":" <> String.from(n) <> "}")
end

# Helper: broadcast unresolve notification then return success response
fn unresolve_success(pool, issue_id :: String, n :: Int) do
  let _ = broadcast_issue_update(pool, issue_id, "unresolved")
  HTTP.response(200, "{\"status\":\"ok\",\"affected\":" <> String.from(n) <> "}")
end

# Helper: broadcast discard notification then return success response
fn discard_success(pool, issue_id :: String, n :: Int) do
  let _ = broadcast_issue_update(pool, issue_id, "discarded")
  HTTP.response(200, "{\"status\":\"ok\",\"affected\":" <> String.from(n) <> "}")
end

# --- Issue management route handlers (Phase 89 Plan 02) ---

# Helper: build a JSON string for a single Issue.
# Uses deriving(Json) on the Issue struct for automatic serialization.
fn issue_to_json_str(issue :: Issue) -> String do
  Json.encode(issue)
end

# Build JSON array from list of issues.
fn issues_to_json(issues :: List<Issue>) -> String do
  let json_items = List.map(issues, fn(issue) do issue_to_json_str(issue) end)
  "[" <> String.join(json_items, ",") <> "]"
end

# Handle GET /api/v1/projects/:project_id/issues?status=unresolved
# Defaults to listing 'unresolved' issues (query string parsing not available in Mesh).
pub fn handle_list_issues(request) do
  let reg_pid = Process.whereis("mesher_registry")
  let pool = PipelineRegistry.get_pool(reg_pid)
  let project_id = Request.param(request, "project_id")
  let result = list_issues_by_status(pool, project_id, "unresolved")
  case result do
    Ok(issues) -> HTTP.response(200, issues_to_json(issues))
    Err(e) -> HTTP.response(500, "{\"error\":\"" <> e <> "\"}")
  end
end

# Handle POST /api/v1/issues/:id/resolve
pub fn handle_resolve_issue(request) do
  let reg_pid = Process.whereis("mesher_registry")
  let pool = PipelineRegistry.get_pool(reg_pid)
  let issue_id = Request.param(request, "id")
  let result = resolve_issue(pool, issue_id)
  case result do
    Ok(n) -> resolve_success(pool, issue_id, n)
    Err(e) -> HTTP.response(500, "{\"error\":\"" <> e <> "\"}")
  end
end

# Handle POST /api/v1/issues/:id/archive
pub fn handle_archive_issue(request) do
  let reg_pid = Process.whereis("mesher_registry")
  let pool = PipelineRegistry.get_pool(reg_pid)
  let issue_id = Request.param(request, "id")
  let result = archive_issue(pool, issue_id)
  case result do
    Ok(n) -> archive_success(pool, issue_id, n)
    Err(e) -> HTTP.response(500, "{\"error\":\"" <> e <> "\"}")
  end
end

# Handle POST /api/v1/issues/:id/unresolve
pub fn handle_unresolve_issue(request) do
  let reg_pid = Process.whereis("mesher_registry")
  let pool = PipelineRegistry.get_pool(reg_pid)
  let issue_id = Request.param(request, "id")
  let result = unresolve_issue(pool, issue_id)
  case result do
    Ok(n) -> unresolve_success(pool, issue_id, n)
    Err(e) -> HTTP.response(500, "{\"error\":\"" <> e <> "\"}")
  end
end

# Helper: perform assignment after extracting user_id from parsed JSON rows.
fn assign_from_rows(pool :: PoolHandle, issue_id :: String, rows) do
  if List.length(rows) > 0 do
    let user_id = Map.get(List.head(rows), "user_id")
    let result = assign_issue(pool, issue_id, user_id)
    case result do
      Ok(n) -> HTTP.response(200, "{\"status\":\"ok\"}")
      Err(e) -> HTTP.response(500, "{\"error\":\"" <> e <> "\"}")
    end
  else
    HTTP.response(400, "{\"error\":\"invalid body\"}")
  end
end

# Handle POST /api/v1/issues/:id/assign
# Extracts user_id from JSON body using PostgreSQL jsonb parsing.
pub fn handle_assign_issue(request) do
  let reg_pid = Process.whereis("mesher_registry")
  let pool = PipelineRegistry.get_pool(reg_pid)
  let issue_id = Request.param(request, "id")
  let body = Request.body(request)
  let rows_result = Pool.query(pool, "SELECT COALESCE($1::jsonb->>'user_id', '') AS user_id", [body])
  case rows_result do
    Err(e) -> HTTP.response(400, "{\"error\":\"invalid json\"}")
    Ok(rows) -> assign_from_rows(pool, issue_id, rows)
  end
end

# Handle POST /api/v1/issues/:id/discard
pub fn handle_discard_issue(request) do
  let reg_pid = Process.whereis("mesher_registry")
  let pool = PipelineRegistry.get_pool(reg_pid)
  let issue_id = Request.param(request, "id")
  let result = discard_issue(pool, issue_id)
  case result do
    Ok(n) -> discard_success(pool, issue_id, n)
    Err(e) -> HTTP.response(500, "{\"error\":\"" <> e <> "\"}")
  end
end

# Handle POST /api/v1/issues/:id/delete
pub fn handle_delete_issue(request) do
  let reg_pid = Process.whereis("mesher_registry")
  let pool = PipelineRegistry.get_pool(reg_pid)
  let issue_id = Request.param(request, "id")
  let result = delete_issue(pool, issue_id)
  case result do
    Ok(n) -> HTTP.response(200, "{\"status\":\"ok\",\"affected\":" <> String.from(n) <> "}")
    Err(e) -> HTTP.response(500, "{\"error\":\"" <> e <> "\"}")
  end
end
