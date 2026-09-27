fn show(label :: String, result :: Result<Int, String>) do
  case result do
    Ok(_) -> println(label <> ":ok")
    Err(_) -> println(label <> ":failed")
  end
end

fn one(pool :: PoolHandle, sql :: String) -> String!String do
  let rows = Pool.query(pool, sql, [])?
  Ok(Map.get(List.head(rows), "v"))
end

fn columns(pool :: PoolHandle) -> String!String do
  one(pool,
    "SELECT string_agg(column_name, ',' ORDER BY ordinal_position) AS v FROM information_schema.columns WHERE table_schema = 'mesh_migration_e2e' AND table_name = 'people'")
end

fn indexes(pool :: PoolHandle) -> String!String do
  one(pool,
    "SELECT string_agg(indexdef, ' | ' ORDER BY indexname) AS v FROM pg_indexes WHERE schemaname = 'mesh_migration_e2e' AND indexname <> 'people_pkey' AND indexname <> 'people_email_key'")
end

fn run() -> Int!String do
  let url = Env.get("MESH_TEST_DATABASE_URL",
    "postgres://mesh_test:mesh_test@localhost:5432/mesh_test?sslmode=disable")
  let pool = Pool.open(url, 1, 1, 5000)?
  let _ = Pool.execute(pool, "DROP SCHEMA IF EXISTS mesh_migration_e2e CASCADE", [])?
  let _ = Pool.execute(pool, "CREATE SCHEMA mesh_migration_e2e", [])?
  let _ = Pool.execute(pool, "SET search_path TO mesh_migration_e2e", [])?
  show("create",
    Migration.create_table(pool,
      "people",
      ["id:BIGSERIAL:PRIMARY KEY", "name:TEXT:NOT NULL", "age:INT", "CHECK (age >= 0)"]))
  show("create_again", Migration.create_table(pool, "people", ["id:BIGINT"]))
  show("add", Migration.add_column(pool, "people", "email:TEXT:UNIQUE"))
  show("add_again", Migration.add_column(pool, "people", "email:TEXT:UNIQUE"))
  show("add_plain", Migration.add_column(pool, "people", "nickname:TEXT"))
  show("add_untyped", Migration.add_column(pool, "people", "untyped"))
  show("add_sql", Migration.add_column(pool, "people", "score INT DEFAULT 0"))
  show("add_sql_again", Migration.add_column(pool, "people", "score INT DEFAULT 0"))
  show("rename", Migration.rename_column(pool, "people", "nickname", "alias"))
  show("rename_missing", Migration.rename_column(pool, "people", "nickname", "alias"))
  show("drop_column", Migration.drop_column(pool, "people", "alias"))
  show("drop_column_again", Migration.drop_column(pool, "people", "alias"))
  println("columns:" <> columns(pool)?)
  show("index", Migration.create_index(pool, "people", ["name", "age:DESC"], ""))
  show("index_unique",
    Migration.create_index(pool, "people", ["email"], "unique:true where:email IS NOT NULL"))
  show("index_named", Migration.create_index(pool, "people", ["age:ASC"], "name:people_by_age"))
  show("index_no_columns", Migration.create_index(pool, "people", [], ""))
  show("index_bad_option", Migration.create_index(pool, "people", ["name"], "fast:yes"))
  println("indexes:" <> indexes(pool)?)
  # The columns create_index was given name the index it made, order and all.
  show("drop_index", Migration.drop_index(pool, "people", ["name", "age:DESC"]))
  show("drop_index_again", Migration.drop_index(pool, "people", ["name", "age"]))
  show("drop_index_bad", Migration.drop_index(pool, "people", ["age:UP"]))
  println("indexes_left:" <> indexes(pool)?)
  show("execute", Migration.execute(pool, "COMMENT ON TABLE people IS 'migrated'"))
  show("execute_bad", Migration.execute(pool, "NOT SQL"))
  println("comment:" <> one(pool, "SELECT obj_description('people'::regclass) AS v")?)
  show("drop", Migration.drop_table(pool, "people"))
  show("drop_again", Migration.drop_table(pool, "people"))
  # A schema-qualified table is that schema's table, its index beside it.
  let pets = "mesh_migration_e2e.pets"
  let pets_sql = "SELECT (SELECT count(*) FROM pg_tables WHERE schemaname = 'mesh_migration_e2e' AND tablename = 'pets')::text || ',' || (SELECT count(*) FROM pg_indexes WHERE schemaname = 'mesh_migration_e2e' AND indexname = 'idx_pets_name')::text AS v"
  show("create_qualified", Migration.create_table(pool, pets, ["id:INT", "name:TEXT"]))
  show("index_qualified", Migration.create_index(pool, pets, ["name"], ""))
  println("qualified:" <> one(pool, pets_sql)?)
  show("drop_index_qualified", Migration.drop_index(pool, pets, ["name"]))
  show("drop_qualified", Migration.drop_table(pool, pets))
  println("qualified_dropped:" <> one(pool, pets_sql)?)
  let _ = Pool.execute(pool, "DROP SCHEMA mesh_migration_e2e CASCADE", [])?
  Pool.close(pool)
  Ok(0)
end

fn main() do
  case run() do
    Ok(_) -> println("done")
    Err(error) -> println("error:" <> error)
  end
end
