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

# Helper: build 401 response
fn unauthorized_response() do
  HTTP.response(401, "{\"error\":\"unauthorized\"}")
end

# Helper: build 400 response with reason
fn bad_request_response(reason :: String) do
  HTTP.response(400, "{\"error\":\"" <> reason <> "\"}")
end

# Helper: build 429 rate-limited response
fn rate_limited_response() do
  HTTP.response(429, "{\"error\":\"rate limited\"}")
end

# Helper: build 202 accepted response
fn accepted_response() do
  HTTP.response(202, "{\"status\":\"accepted\"}")
end

# Helper: route event to processor and build response
fn route_to_processor(processor_pid, project_id :: String, writer_pid, body :: String) do
  let result = EventProcessor.process_event(processor_pid, project_id, writer_pid, body)
  case result do
    Ok(_) -> accepted_response()
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

# Helper: handle bulk after authentication succeeds
fn handle_bulk_authed(project_id :: String, rate_limiter_pid, processor_pid, writer_pid, request) do
  let allowed = RateLimiter.check_limit(rate_limiter_pid, project_id)
  if allowed do
    let body = Request.body(request)
    let size_check = validate_payload_size(body, 5242880)
    case size_check do
      Err(reason) -> bad_request_response(reason)
      Ok(_) -> accepted_response()
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
