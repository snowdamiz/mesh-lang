fn show(label :: String, result :: Result<Int, String>) do
  case result do
    Ok(_) -> println(label <> ":ok")
    Err(error) -> println(label <> ":error:" <> error)
  end
end

fn show_list(label :: String, result :: Result<List<String>, String>) do
  case result do
    Ok(names) -> println(label <> ":" <> String.join(names, ","))
    Err(error) -> println(label <> ":error:" <> error)
  end
end

fn failed<T>(label :: String, result :: Result<T, String>) do
  case result do
    Ok(_) -> println(label <> ":unexpected-ok")
    Err(_) -> println(label <> ":failed")
  end
end

fn tables(pool :: PoolHandle) do
  show("table",
    Pg.create_range_partitioned_table(pool,
      "events",
      ["id:bigserial", "at:date:not null", "body:text", "tags:jsonb", "primary key (id, at)"],
      "at"))
  show("table_again",
    Pg.create_range_partitioned_table(pool, "events", ["id:bigint", "at:date"], "at"))
  show("table_empty_name", Pg.create_range_partitioned_table(pool, " ", ["at:date"], "at"))
  show("table_no_partition_column", Pg.create_range_partitioned_table(pool, "t", ["at:date"], " "))
  show("table_bad_column", Pg.create_range_partitioned_table(pool, "t", [":date"], "at"))
  show("table_blank_column", Pg.create_range_partitioned_table(pool, "t", [" "], "at"))
  show("table_only_constraints",
    Pg.create_range_partitioned_table(pool, "t", ["primary key (at)"], "at"))
  show("table_partition_column_missing",
    Pg.create_range_partitioned_table(pool, "t", ["id:int"], "at"))
end

fn indexes(pool :: PoolHandle) do
  show("gin_trgm", Pg.create_gin_index(pool, "events", "events_body_trgm", "body", "gin_trgm_ops"))
  show("gin_qualified",
    Pg.create_gin_index(pool, "events", "events_tags", "tags", "pg_catalog.jsonb_path_ops"))
  show("gin_empty_table", Pg.create_gin_index(pool, " ", "i", "c", "o"))
  show("gin_empty_index", Pg.create_gin_index(pool, "t", " ", "c", "o"))
  show("gin_empty_column", Pg.create_gin_index(pool, "t", "i", " ", "o"))
  show("gin_empty_opclass", Pg.create_gin_index(pool, "t", "i", "c", " "))
  show("gin_empty_segment", Pg.create_gin_index(pool, "t", "i", "c", "pg_catalog..ops"))
end

fn partitions(pool :: PoolHandle) -> Int!String do
  show("ahead", Pg.create_daily_partitions_ahead(pool, "events", 3))
  show("ahead_none", Pg.create_daily_partitions_ahead(pool, "events", 0))
  show("ahead_negative", Pg.create_daily_partitions_ahead(pool, "events", -1))
  show("ahead_empty_parent", Pg.create_daily_partitions_ahead(pool, " ", 1))
  failed("ahead_missing_parent", Pg.create_daily_partitions_ahead(pool, "no_such_table", 1))
  let _ = Pool.execute(pool,
    "CREATE TABLE events_20000101 PARTITION OF events FOR VALUES FROM ('2000-01-01') TO ('2000-01-02')",
    [])?
  show_list("before", Pg.list_daily_partitions_before(pool, "events", 30))
  show_list("before_none", Pg.list_daily_partitions_before(pool, "events", 100000))
  show_list("before_negative", Pg.list_daily_partitions_before(pool, "events", -1))
  show("drop", Pg.drop_partition(pool, "events_20000101"))
  show("drop_empty", Pg.drop_partition(pool, " "))
  show_list("before_dropped", Pg.list_daily_partitions_before(pool, "events", 30))
  let rows = Pool.query(pool,
    "SELECT count(*)::text AS n FROM pg_inherits WHERE inhparent = 'events'::regclass",
    [])?
  println("partitions:" <> Map.get(List.head(rows), "n"))
  Ok(0)
end

fn run() -> Int!String do
  let url = Env.get("MESH_TEST_DATABASE_URL",
    "postgres://mesh_test:mesh_test@localhost:5432/mesh_test?sslmode=disable")
  let pool = Pool.open(url, 1, 1, 5000)?
  let _ = Pool.execute(pool, "DROP SCHEMA IF EXISTS mesh_pg_schema_e2e CASCADE", [])?
  let _ = Pool.execute(pool, "CREATE SCHEMA mesh_pg_schema_e2e", [])?
  let _ = Pool.execute(pool, "SET search_path TO mesh_pg_schema_e2e, public", [])?
  show("extension", Pg.create_extension(pool, "pg_trgm"))
  show("extension_again", Pg.create_extension(pool, "pg_trgm"))
  show("extension_empty", Pg.create_extension(pool, " "))
  failed("extension_unknown", Pg.create_extension(pool, "no_such_extension"))
  tables(pool)
  indexes(pool)
  let _ = partitions(pool)?
  let _ = Pool.execute(pool, "DROP SCHEMA mesh_pg_schema_e2e CASCADE", [])?
  Pool.close(pool)
  Ok(0)
end

fn main() do
  case run() do
    Ok(_) -> println("done")
    Err(error) -> println("error:" <> error)
  end
end
