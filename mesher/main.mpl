# Mesher monitoring platform entry point.
# Connects to PostgreSQL, creates schema and partitions, starts all services.
# Services are defined in mesher/services/ modules.
# Ingestion pipeline wires HTTP routes and WS handler.

from Storage.Schema import create_schema, create_partitions_ahead
from Services.Org import OrgService
from Services.Project import ProjectService
from Services.User import UserService
from Services.Writer import StorageWriter
from Ingestion.Pipeline import start_pipeline
from Ingestion.Routes import handle_event, handle_bulk
from Ingestion.WsHandler import ws_on_connect, ws_on_message, ws_on_close

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

  # Start ingestion pipeline (registers PipelineRegistry by name)
  let registry_pid = start_pipeline(pool)

  println("[Mesher] Foundation ready")

  # Set up HTTP routes with ingestion handlers
  let r = HTTP.router()
  let r = HTTP.on_post(r, "/api/v1/events", handle_event)
  let r = HTTP.on_post(r, "/api/v1/events/bulk", handle_bulk)

  println("[Mesher] HTTP server starting on :8080")
  HTTP.serve(r, 8080)
end

fn main() do
  println("[Mesher] Connecting to PostgreSQL...")
  let pool_result = Pool.open("postgres://mesh:mesh@localhost:5432/mesher", 2, 10, 5000)
  case pool_result do
    Ok(pool) -> start_services(pool)
    Err(_) -> println("[Mesher] Failed to connect to PostgreSQL")
  end
end
