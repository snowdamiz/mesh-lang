# Mesh database access

## SQLite and PostgreSQL drivers

Use `Sqlite` for local storage and `Pg` or `Pool` for shared PostgreSQL storage. Bind parameters separately from SQL. SQLite uses `?` placeholders and PostgreSQL uses `$1`, `$2`, and so on. `execute(connection, sql, params)` returns an affected-row count in a `Result`. `query` returns `Result<List<Map<String, String>>, String>`. Use `query` for row-producing statements, including writes with `RETURNING`. String row maps represent SQL `NULL` as `""`. [SQLite](https://meshlang.dev/docs/databases/#sqlite) · [Direct PostgreSQL API](https://meshlang.dev/docs/databases/#direct-postgresql-api)

- `Sqlite.open(path)` returns `Result<SqliteConn, String>`. It accepts a file path or `":memory:"`. Release the connection with `Sqlite.close(conn)`. `Sqlite.begin`, `Sqlite.commit`, and `Sqlite.rollback` return `Result<Unit, String>`.
- `Pg.connect(url)` returns `Result<PgConn, String>`. `PgConn` is affine. Assignments and ordinary parameters move it, database operations borrow it, and `Pg.close(conn)` consumes it. It cannot cross an actor boundary. `Pg.begin`, `Pg.commit`, and `Pg.rollback` support manual transactions. `Pg.transaction(conn, fn)` takes a callback with a `borrow PgConn` parameter and a `Result<Unit, String>` return. It commits on `Ok` and rolls back on `Err` or panic. An aborted PostgreSQL transaction still fails to commit if the callback ignores a statement error.
- `Pool.open(url, min, max, checkout_timeout_ms)` returns `Result<PoolHandle, String>` and opens the minimum connections eagerly. `Pool.query` and `Pool.execute` manage their own checkouts. `Pool.close` releases the pool. The PostgreSQL connection guide covers verified TLS and private CAs. [Connection URLs and pool functions](https://meshlang.dev/docs/databases/#postgresql-connections-and-pools)

## Binary values and typed rows

`Sqlite`, `Pg`, and `Pool` offer `execute_values` and `query_values` for `List<DbValue>` parameters. `DbValue` has `Text(String)`, `Binary(Bytes)`, and `Null` variants. `query_values` returns row maps of `DbValue`. An empty binary value is distinct from SQL `NULL`. Result rows and values have size limits. [Binary and NULL values](https://meshlang.dev/docs/databases/#binary-and-null-values)

```mesh
fn round_trip(db :: SqliteConn) -> String!String do
	Sqlite.execute(db, "CREATE TABLE notes (id INTEGER PRIMARY KEY, body BLOB, caption TEXT)", [])?
	let body = Bytes.from_hex("00ff")?
	Sqlite.execute_values(db, "INSERT INTO notes VALUES (?, ?, ?)", [Text("1"), Binary(body), Null])?
	let rows = Sqlite.query_values(db, "SELECT body, caption FROM notes WHERE id = ?", [Text("1")])?
	case rows do
		[] -> Err("no note")
		row :: _ ->
			case (Map.get(row, "body"), Map.get(row, "caption")) do
				(Binary(value), Null) -> Ok(Bytes.to_hex(value))
				_ -> Err("unexpected note values")
			end
	end
end

fn main() do
	case Sqlite.open(":memory:") do
		Ok(db) ->
			let result = round_trip(db)
			Sqlite.close(db)
			case result do
				Ok(value) -> println(value)
				Err(error) -> println(error)
			end
		Err(error) -> println(error)
	end
end
```

`deriving(Row)` generates `Type.from_row(row)` for a `Map<String, String>`. Fields use the result column names, including SQL aliases. The decoder returns `Result<Type, String>` and handles scalar `String`, `Int`, `Float`, `Bool`, and their optional forms. Pass the decoder to `Pg.query_as` or `Pool.query_as` for `Result<List<Result<Type, String>>, String>`. With SQLite, use `List.map(rows, Type.from_row)` after a successful query. [Struct row decoding](https://meshlang.dev/docs/databases/#struct-row-decoding)

## PostgreSQL queries and schema

| Module or derive | Use |
| --- | --- |
| `deriving(Schema)` | Read table, key, field, and relationship metadata. It can be combined with `Row`. [Schema metadata](https://meshlang.dev/docs/databases/#schema-metadata) |
| `Query` and `Expr` | Compose immutable queries with filters, joins, subqueries, aggregates, and ordering. `Expr.value` binds a parameter. `Expr.column` references an identifier. [Query builder](https://meshlang.dev/docs/databases/#query-builder) · [Structured expressions](https://meshlang.dev/docs/databases/#structured-sql-expressions) |
| `Repo` | Execute queries against a `PoolHandle` with `all`, `one`, `get`, `count`, and other reads. Write through insert, update, delete, or upsert helpers. Use `Repo.preload` for associations and `Repo.query_raw` or `Repo.execute_raw` for parameterized SQL. [Repository operations](https://meshlang.dev/docs/databases/#repository-operations) |
| `Changeset` | Whitelist and validate input fields before `Repo.insert_changeset` or `Repo.update_changeset`. Failures return a changeset with errors. [Changesets](https://meshlang.dev/docs/databases/#changesets) |
| `Migration` | Run PostgreSQL DDL through a pool. `meshc migrate generate <name>` creates a timestamped file with `up(pool)` and `down(pool)`. The `up`, `down`, and `status` commands read `DATABASE_URL`. [Migrations](https://meshlang.dev/docs/databases/#migrations) |
| `Pg` and `Orm` helpers | Use `Pg` for casts, JSONB, full-text search, extensions, and partition management. Use `Orm` when you need generated SQL text. [PostgreSQL-specific helpers](https://meshlang.dev/docs/databases/#postgresql-specific-helpers) · [Low-level SQL builders](https://meshlang.dev/docs/databases/#low-level-sql-builders) |

`Repo.transaction(pool, fn)` passes a `borrow PgConn` to its callback and returns the connection to the pool. The callback returns a `Result`. `Ok` commits, and `Err` or a panic rolls back. [Repository writes and transactions](https://meshlang.dev/docs/databases/#writes)

## Durable jobs

`Jobs.migrate(pool)` prepares the queue schema. `Jobs.enqueue` commits separately. Use `Jobs.enqueue_in(conn, namespace, spec)` on a borrowed connection inside a transaction to commit the enqueue together with an application write. Reuse the same enqueue `request_key` and spec when the commit outcome is uncertain. [Enqueue and transaction semantics](https://meshlang.dev/docs/concurrency/#durable-jobs)

`Jobs.start` runs named handlers in separate attempt processes. Retries keep the same `effect_key` for deduplication. `Jobs.control` inspects and manages queues, including recurring schedules. Stop and join runners before closing their pool. [Workers and retries](https://meshlang.dev/docs/concurrency/#run-workers-and-handle-retries) · [Queue operations](https://meshlang.dev/docs/concurrency/#inspect-and-operate-queues) · [Recurring jobs](https://meshlang.dev/docs/concurrency/#recurring-jobs)
