---
title: Databases
description: SQLite, PostgreSQL, pools, row and schema deriving, query builders, repositories, changesets, and migrations in Mesh
---

# Databases

Mesh supports two deliberately different storage paths:

- `Sqlite` is a direct embedded connection for local and single-node applications.
- `Pg` and `Pool` connect to PostgreSQL. A pool is the normal choice for a shared, deployable service.

Both drivers accept parameterized SQL and return errors through `Result`. The higher-level `Query`, `Expr`, `Repo`, `Changeset`, and `Migration` modules target PostgreSQL pools. PostgreSQL-specific types and operators remain explicit under `Pg`; Mesh does not pretend every backend has the same capabilities.

> **Clustered applications:** Do not place one SQLite file behind several application nodes. Use PostgreSQL when nodes must share application state. Continue with [Autonomous Clusters](/docs/autonomous-clusters/) for clustered routing and continuity.

## SQLite

`Sqlite.open(":memory:")` creates an in-memory database; a filesystem path creates or opens a local database file.

```mesh
fn run() -> Int!String do
  let db = Sqlite.open(":memory:")?
  let _ = Sqlite.execute(
    db,
    "CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT NOT NULL)",
    []
  )?
  let _ = Sqlite.execute(
    db,
    "INSERT INTO notes (id, body) VALUES (?, ?)",
    ["1", "hello"]
  )?

  let rows = Sqlite.query(db, "SELECT id, body FROM notes WHERE id = ?", ["1"])?
  println(Map.get(List.head(rows), "body"))
  Sqlite.close(db)
  Ok(0)
end
```

| Function | Returns | Description |
|----------|---------|-------------|
| `Sqlite.open(path)` | `Result<SqliteConn, String>` | Open a file or `:memory:` database |
| `Sqlite.close(connection)` | `Unit` | Close the connection |
| `Sqlite.execute(connection, sql, params)` | `Result<Int, String>` | Execute parameterized DDL or DML and return the affected-row count |
| `Sqlite.query(connection, sql, params)` | `Result<List<Map<String, String>>, String>` | Return rows keyed by column name |
| `Sqlite.begin(connection)` | `Result<Unit, String>` | Begin a transaction |
| `Sqlite.commit(connection)` | `Result<Unit, String>` | Commit the transaction |
| `Sqlite.rollback(connection)` | `Result<Unit, String>` | Roll back the transaction |

SQLite placeholders are `?`. Values in direct-query row maps are strings; decode them manually or apply a struct's generated `from_row` function with `List.map`.

## PostgreSQL Connections and Pools

For a one-off connection, use `Pg.connect`. For an HTTP service or other concurrent application, open a bounded pool:

```mesh
fn main() do
  let url = Env.get("DATABASE_URL", "")
  case Pool.open(url, 1, 8, 5_000) do
    Ok(pool) -> do
      println("database ready")
      Pool.close(pool)
    end
    Err(error) -> IO.eprintln("database unavailable: #{error}")
  end
end
```

`Pool.open(url, min_size, max_size, checkout_timeout_ms)` eagerly creates the minimum connections and bounds concurrent checkouts at the maximum.

### Direct PostgreSQL API

| Function | Returns | Description |
|----------|---------|-------------|
| `Pg.connect(url)` | `Result<PgConn, String>` | Open one PostgreSQL connection |
| `Pg.close(connection)` | `Unit` | Consume and close it |
| `Pg.execute(connection, sql, params)` | `Result<Int, String>` | Borrow it to execute parameterized SQL |
| `Pg.query(connection, sql, params)` | `Result<List<Map<String, String>>, String>` | Borrow it to query rows as string maps |
| `Pg.query_as(connection, sql, params, decoder)` | `Result<List<Result<T, String>>, String>` | Decode every returned row |
| `Pg.begin(connection)` | `Result<Unit, String>` | Begin a transaction |
| `Pg.commit(connection)` | `Result<Unit, String>` | Commit |
| `Pg.rollback(connection)` | `Result<Unit, String>` | Roll back |
| `Pg.transaction(connection, fn)` | `Result<Unit, String>` | Borrow it for a callback whose `PgConn` parameter is also declared `borrow` |

`PgConn` is affine: assignments and ordinary function parameters move it,
database operations borrow it, and `Pg.close` consumes it. It cannot cross an
actor boundary. PostgreSQL placeholders are `$1`, `$2`, and so on.

### Pool API

| Function | Returns | Description |
|----------|---------|-------------|
| `Pool.open(url, min, max, timeout_ms)` | `Result<PoolHandle, String>` | Create a bounded PostgreSQL pool |
| `Pool.close(pool)` | `Unit` | Close all pooled connections |
| `Pool.execute(pool, sql, params)` | `Result<Int, String>` | Execute using a checked-out connection |
| `Pool.query(pool, sql, params)` | `Result<List<Map<String, String>>, String>` | Query using a checked-out connection |
| `Pool.query_as(pool, sql, params, decoder)` | `Result<List<Result<T, String>>, String>` | Query and decode each row |

Pool leases are runtime-internal so their provenance cannot be forged. Use the
scoped `Pool.query`, `Pool.execute`, typed value variants, or `Repo` APIs.

## Struct Row Decoding

`deriving(Row)` generates `Type.from_row(Map<String, String>) -> Result<Type, String>`. It validates required columns and converts `String`, `Int`, `Float`, `Bool`, and optional forms.

```mesh
struct User do
  id :: String
  name :: String
  age :: Int
  bio :: Option<String>
end deriving(Row)

let decoded = Pool.query_as(
  pool,
  "SELECT id, name, age, bio FROM users ORDER BY name",
  [],
  User.from_row
)
```

The outer `Result` reports query failure. Each element's inner `Result` reports a row-decoding failure, so callers decide whether one malformed row should reject the whole result.

`Pg.query_as` and `Pool.query_as` call the decoder automatically. SQLite has no `query_as` shortcut: after `Sqlite.query`, use `List.map(rows, User.from_row)`.

## Schema Metadata

`deriving(Schema)` creates database metadata from a struct. It can be combined with `Row`:

```mesh
struct User do
  table "people"
  primary_key :uuid
  timestamps true

  uuid :: String
  name :: String
  has_many :posts, Post
end deriving(Schema, Row)

struct Post do
  id :: String
  user_id :: String
  title :: String
  belongs_to :user, User
end deriving(Schema, Row)
```

| Generated member | Description |
|------------------|-------------|
| `User.__table__()` | Configured table, or the lowercased plural struct name |
| `User.__primary_key__()` | Configured key, or `"id"` |
| `User.__fields__()` | Field-name list; timestamps add `inserted_at` and `updated_at` |
| `User.__field_types__()` | `field:SQL_TYPE` metadata |
| `User.__relationships__()` | Compact `belongs_to`, `has_one`, and `has_many` relationship metadata |
| `User.__relationship_meta__()` | Relationship metadata including foreign key and target table |
| `User.__name_col__()` | Per-field column-name accessor |

Schema metadata drives query construction and `Repo.preload`; it does not run migrations automatically.

## Query Builder

Queries are immutable and pipe-friendly:

```mesh
let query = Query.from(User.__table__())
  |> Query.select(User.__fields__())
  |> Query.where(:active, "true")
  |> Query.where_op(:age, :gte, "18")
  |> Query.order_by(:name, :asc)
  |> Query.limit(50)
```

### Filtering

| Function | Description |
|----------|-------------|
| `Query.from(table)` | Start a query |
| `Query.where(query, field, value)` | Equality predicate |
| `Query.where_op(query, field, operator, value)` | Predicate using `:eq`, `:neq`, `:lt`, `:lte`, `:gt`, `:gte`, `:like`, or `:ilike` |
| `Query.where_in(query, field, values)` | `IN` predicate |
| `Query.where_not_in(query, field, values)` | `NOT IN` predicate |
| `Query.where_between(query, field, low, high)` | Inclusive range predicate |
| `Query.where_null(query, field)` | `IS NULL` |
| `Query.where_not_null(query, field)` | `IS NOT NULL` |
| `Query.where_or(query, fields, values)` | Group parallel equality predicates with `OR` |
| `Query.where_expr(query, expression)` | Add a structured `Expr` predicate |
| `Query.where_sub(query, field, subquery)` | Add `field IN (subquery)` |

### Selection and Shape

| Function | Description |
|----------|-------------|
| `Query.select(query, fields)` | Select named columns |
| `Query.select_expr(query, expression)` | Select one structured expression |
| `Query.select_exprs(query, expressions)` | Select several structured expressions |
| `Query.select_count(query)` | Select `count(*)` |
| `Query.select_count_field(query, field)` | Count non-null field values |
| `Query.select_sum(query, field)` | Select a sum |
| `Query.select_avg(query, field)` | Select an average |
| `Query.select_min(query, field)` | Select a minimum |
| `Query.select_max(query, field)` | Select a maximum |
| `Query.order_by(query, field, direction)` | Order with `:asc` or `:desc` |
| `Query.limit(query, count)` | Bound returned rows |
| `Query.offset(query, count)` | Skip rows |
| `Query.join(query, kind, table, on_clause)` | Add a join such as `:inner` or `:left` |
| `Query.join_as(query, kind, table, alias, on_clause)` | Add an aliased join |
| `Query.group_by(query, field)` | Group on a field |
| `Query.having(query, clause, value)` | Add a parameterized aggregate predicate |

### Explicit SQL Fragments

| Function | Description |
|----------|-------------|
| `Query.fragment(query, sql, params)` | Append a parameterized fragment |
| `Query.select_raw(query, expressions)` | Select raw SQL expressions |
| `Query.where_raw(query, sql, params)` | Add a raw parameterized predicate |
| `Query.order_by_raw(query, sql)` | Add a raw order expression |
| `Query.group_by_raw(query, sql)` | Add a raw grouping expression |

Prefer structured builders first. Keep database-specific SQL visible when a shape genuinely needs a raw fragment.

## Structured SQL Expressions

`Expr.value` creates a bound value; it does not interpolate text into SQL. `Expr.column` creates an identifier reference.

| Functions | Purpose |
|-----------|---------|
| `Expr.column`, `Expr.value`, `Expr.null` | Columns, parameters, and SQL `NULL` |
| `Expr.call`, `Expr.fn_call` | Function calls with expression arguments |
| `Expr.add`, `Expr.sub`, `Expr.mul`, `Expr.div` | Arithmetic |
| `Expr.eq`, `Expr.neq`, `Expr.lt`, `Expr.lte`, `Expr.gt`, `Expr.gte` | Comparisons |
| `Expr.case`, `Expr.case_when` | Paired conditions/results plus an else expression |
| `Expr.coalesce` | First non-null expression |
| `Expr.excluded` | Refer to an upsert's `EXCLUDED` value |
| `Expr.label` | Assign a selected expression's output name |

`Expr.alias` remains available as a compatibility synonym; use `Expr.label` in new code.

```mesh
let query = Query.from("accounts")
  |> Query.select_exprs([
    Expr.label(
      Expr.coalesce([Expr.column("nickname"), Expr.value("anonymous")]),
      "display_name"
    ),
    Expr.label(Expr.add(Expr.column("balance"), Expr.value("10")), "next_balance")
  ])
```

## Repository Operations

`Repo` executes queries against a `PoolHandle`.

### Reads

| Function | Description |
|----------|-------------|
| `Repo.all(pool, query)` | Return all rows |
| `Repo.one(pool, query)` | Return the first row, or `Err("not found")` |
| `Repo.get(pool, table, id)` | Read by primary-key value |
| `Repo.get_by(pool, table, field, value)` | Read by one field |
| `Repo.count(pool, query)` | Count matching rows |
| `Repo.exists(pool, query)` | Test whether a match exists |
| `Repo.preload(pool, rows, associations, relationship_meta)` | Load declared associations |

### Writes

| Function | Description |
|----------|-------------|
| `Repo.insert(pool, table, fields)` | Insert string-valued fields and return the row |
| `Repo.insert_expr(pool, table, fields)` | Insert expression-valued fields |
| `Repo.update(pool, table, id, fields)` | Update by primary key |
| `Repo.update_where(pool, table, fields, query)` | Update matching rows with string values |
| `Repo.update_where_expr(pool, table, fields, query)` | Update matching rows with expressions |
| `Repo.delete(pool, table, id)` | Delete by primary key and return the row |
| `Repo.delete_where(pool, table, query)` | Delete matching rows and return a count |
| `Repo.delete_where_returning(pool, table, query)` | Delete and return rows |
| `Repo.insert_or_update(pool, table, fields, conflict_fields, update_fields)` | Upsert string-valued fields |
| `Repo.insert_or_update_expr(pool, table, fields, conflict_fields, updates)` | Upsert with expression updates |
| `Repo.insert_changeset(pool, table, changeset)` | Insert a valid changeset |
| `Repo.update_changeset(pool, table, id, changeset)` | Update from a valid changeset |
| `Repo.transaction(pool, fn)` | Run `fn(connection :: borrow PgConn)` and commit `Ok` or roll back `Err` |

Expression writes make updates such as counters and server-side timestamps atomic:

```mesh
let query = Query.from("counters") |> Query.where(:id, "primary")
Repo.update_where_expr(
  pool,
  "counters",
  %{
    "value" => Expr.add(Expr.column("value"), Expr.value("1")),
    "touched_at" => Expr.fn_call("now", [])
  },
  query
)
```

`Repo.transaction` always returns its checked-out connection to the pool. The
callback must declare its PostgreSQL parameter as `borrow PgConn` and return a
`Result`; a callback failure or runtime exception rolls the transaction back.

### Raw Repository Escape Hatches

| Function | Description |
|----------|-------------|
| `Repo.query_raw(pool, sql, params)` | Return rows from parameterized SQL |
| `Repo.execute_raw(pool, sql, params)` | Execute parameterized SQL and return affected rows |

## Changesets

Changesets whitelist input fields, convert values, accumulate validation errors, and feed repository writes.

```mesh
let changeset = Changeset.cast(%{}, params, [:name, :email])
  |> Changeset.validate_required([:name, :email])
  |> Changeset.validate_length(:name, 2, 80)
  |> Changeset.validate_format(:email, "@")

if Changeset.valid(changeset) do
  Repo.insert_changeset(pool, "users", changeset)
else
  println(Json.encode(Changeset.errors(changeset)))
end
```

| Function | Description |
|----------|-------------|
| `Changeset.cast(data, params, allowed)` | Keep only allowed fields |
| `Changeset.cast_with_types(data, params, allowed, field_types)` | Cast values using schema field metadata |
| `Changeset.validate_required(changeset, fields)` | Require non-empty values |
| `Changeset.validate_length(changeset, field, min, max)` | Validate string length; `-1` disables a bound |
| `Changeset.validate_format(changeset, field, substring)` | Require a string to contain the supplied substring |
| `Changeset.validate_inclusion(changeset, field, allowed)` | Require one of the supplied strings |
| `Changeset.validate_number(changeset, field, gt, lt, gte, lte)` | Validate integer bounds; `-1` disables a bound |
| `Changeset.valid(changeset)` | Return whether validation succeeded |
| `Changeset.errors(changeset)` | Return `Map<String, String>` errors |
| `Changeset.changes(changeset)` | Return accepted values |
| `Changeset.get_change(changeset, field)` | Read one accepted value, or `""` when absent |
| `Changeset.get_error(changeset, field)` | Read one error, or `""` when absent |

`Repo.insert_changeset` and `Repo.update_changeset` return `Result<Map<String, String>, Changeset>`. Invalid input is returned as the `Err` changeset without executing SQL. PostgreSQL unique, foreign-key, and not-null violations are mapped back to field errors; other database failures become a `_base` error.

## Migrations

The `Migration` module executes common PostgreSQL DDL through a pool:

| Function | Description |
|----------|-------------|
| `Migration.create_table(pool, table, columns)` | Create a table if absent |
| `Migration.drop_table(pool, table)` | Drop a table if present |
| `Migration.add_column(pool, table, definition)` | Add a column if absent |
| `Migration.drop_column(pool, table, column)` | Drop a column if present |
| `Migration.rename_column(pool, table, old, new)` | Rename a column |
| `Migration.create_index(pool, table, columns, options)` | Create a normal, unique, ordered, or partial index |
| `Migration.drop_index(pool, table, columns)` | Drop the derived index |
| `Migration.execute(pool, sql)` | Execute raw DDL |

Column definitions use `name:TYPE` or `name:TYPE:CONSTRAINTS`, for example `id:UUID:PRIMARY KEY`. Index columns may end in `:ASC` or `:DESC`. Index options accept `unique:true`, `name:index_name`, and a final `where:predicate`.

Use `meshc migrate generate <name>` to create a timestamped file exporting `up(pool)` and `down(pool)`. `meshc migrate up`, `meshc migrate down`, and `meshc migrate status` read `DATABASE_URL` and track applied versions in PostgreSQL. Migration names may contain lowercase ASCII letters, digits, and underscores.

## PostgreSQL-Specific Helpers

Typed expression helpers keep vendor-specific choices visible:

| Functions | Purpose |
|-----------|---------|
| `Pg.cast(expression, type)` | Explicit PostgreSQL cast |
| `Pg.jsonb`, `Pg.int`, `Pg.text`, `Pg.uuid`, `Pg.timestamptz` | Common typed casts |
| `Pg.gen_salt`, `Pg.crypt` | `pgcrypto` password expressions |
| `Pg.to_tsvector`, `Pg.plainto_tsquery`, `Pg.ts_rank`, `Pg.tsvector_matches` | Full-text search |
| `Pg.jsonb_contains` | JSONB containment |

PostgreSQL schema helpers operate on a pool:

| Function | Description |
|----------|-------------|
| `Pg.create_extension(pool, name)` | Install an extension if absent |
| `Pg.create_range_partitioned_table(pool, table, columns, partition_key)` | Create a range-partitioned table |
| `Pg.create_gin_index(pool, table, name, column, opclass)` | Create a GIN index |
| `Pg.create_daily_partitions_ahead(pool, table, days)` | Ensure upcoming daily partitions |
| `Pg.list_daily_partitions_before(pool, table, days)` | List old daily partitions |
| `Pg.drop_partition(pool, partition)` | Drop a named partition |

## Low-Level SQL Builders

The `Orm` functions return quoted PostgreSQL SQL strings with numbered placeholders:

| Function | Description |
|----------|-------------|
| `Orm.build_select(table, columns, where_clauses, order_by, limit, offset)` | Build `SELECT`; empty columns means `*`, and `-1` disables limit or offset |
| `Orm.build_insert(table, columns, returning)` | Build `INSERT` with one placeholder per column |
| `Orm.build_update(table, set_columns, where_clauses, returning)` | Build `UPDATE`; WHERE placeholders follow SET placeholders |
| `Orm.build_delete(table, where_clauses, returning)` | Build `DELETE` |

WHERE entries use forms such as `"name ="`, `"age >"`, or `"deleted_at IS NULL"`; order entries use forms such as `"name ASC"`. These fragments must come from trusted application schema, not request text. Most applications should compose `Query` and execute it through `Repo`; use `Orm` when another layer needs generated SQL text itself.

## What's Next?

- [Autonomous Clusters](/docs/autonomous-clusters/) — shared PostgreSQL topology, routing, scaling, and continuity
- [Web](/docs/web/) — use pools and repositories from HTTP handlers
- [Type System](/docs/type-system/) — structs, `Result`, and deriving
