# Mesher monitoring platform entry point.
# Connects to PostgreSQL, creates schema and partitions, starts all services.
# Services are defined in mesher/services/ modules.

from Storage.Schema import create_schema, create_partitions_ahead
from Services.Org import OrgService
from Services.Project import ProjectService
from Services.User import UserService
from Services.Writer import StorageWriter

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
