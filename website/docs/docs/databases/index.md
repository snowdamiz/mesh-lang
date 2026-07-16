---
title: Databases
description: "Mesh database APIs: neutral Expr/Query/Repo builders plus explicit PostgreSQL-only Pg helpers"
---

# Databases

Mesh separates portable query construction from database-specific capabilities. The supported boundary is:

- a neutral expression/query/write surface built from `Expr`, `Query`, `Repo`, and `Migration`
- explicit PostgreSQL-only helpers under `Pg.*`
- raw escape hatches for SQL shapes that cannot be represented faithfully by the builders

This is not a universal ORM that erases backend differences. PostgreSQL types, operators, extensions, partitions, and catalogs stay visible in application code.

> **Autonomous cluster proof:** This page explains the database API boundary. For the shared PostgreSQL topology and release proof, continue to [Autonomous Clusters](/docs/autonomous-clusters/) and [Distributed Proof](/docs/distributed-proof/).

## What is repository-proven

The database guide is anchored in repository-owned compiler, runtime, and example surfaces:

- `compiler/meshc/tests/e2e.rs` — compiler and generated-code coverage for `Query`, `Repo`, migrations, and PostgreSQL helpers
- `compiler/mesh-rt/src/db/` — runtime query, expression, repository, migration, and PostgreSQL implementations with unit tests
- `compiler/meshc/src/migrate.rs` — migration generator guidance that keeps neutral DDL under `Migration.*` and PostgreSQL extras under `Pg.*`
- `examples/todo-postgres/` — a self-contained PostgreSQL application example

Validate those surfaces with:

```bash
cargo test -p mesh-rt --locked db::
cargo test -p meshc --locked --test e2e
npm --prefix website run build
```

The autonomous-cluster release proof additionally runs a real PostgreSQL dependency in Docker and checks every acknowledged application mutation against final database state. PostgreSQL is the shared application dependency and integrity oracle in that proof; Mesh still owns routing, scaling, consensus, drain, and continuity.

## Neutral surface: `Expr`, `Query`, `Repo`, `Migration`

### Build expressions

Use `Expr.label`, not `Expr.alias`:

```mesh
let q = Query.from("accounts")
  |> Query.select_exprs([
    Expr.label(Expr.coalesce([Expr.column("nickname"), Expr.value("fallback")]), "nick"),
    Expr.label(Expr.add(Expr.column("amount"), Expr.value("2")), "next_amount"),
    Expr.label(
      Expr.case_when(
        [Expr.eq(Expr.column("status"), Expr.value("resolved"))],
        [Expr.value("closed")],
        Expr.column("status")
      ),
      "display_status"
    )
  ])
  |> Query.where(:id, "row-1")
```

The neutral expression pieces are:

- `Expr.value(...)` — bind a literal or parameter
- `Expr.column(...)` — refer to a column
- `Expr.null()` — write an actual `NULL`
- `Expr.case_when(...)` — express SQL branching
- `Expr.coalesce([...])` — express fallback/default logic
- `Expr.label(expr, "name")` — name a derived output column

### Build predicates and row shapes

Applications can mix neutral builders with explicit PostgreSQL casts:

```mesh
let q = Query.from(Issue.__table__())
  |> Query.where_expr(Expr.eq(Expr.column("project_id"), Pg.uuid(Expr.value(project_id))))
  |> Query.where_expr(Expr.eq(Expr.column("status"), Expr.value("unresolved")))
  |> Query.select_expr(Expr.label(Pg.text(Expr.fn_call("count", [Expr.column("*")])), "cnt"))
```

`Query.where_expr(...)`, `Query.select_expr(...)`, and `Query.select_exprs([...])` are neutral. `Pg.uuid(...)` and `Pg.text(...)` deliberately are not.

### Write expression-valued fields

Expression-aware writes avoid handwritten SQL for common inserts, updates, and upserts:

```mesh
let row = Repo.insert_expr(pool,
  User.__table__(),
  %{
    "email" => Expr.value(email),
    "password_hash" => Pg.crypt(Expr.value(password), Pg.gen_salt("bf", 12)),
    "display_name" => Expr.value(display_name)
  })?
```

```mesh
let update_result = Repo.update_where_expr(pool,
  "counters",
  %{
    "amount" => Expr.add(Expr.column("amount"), Expr.value("2")),
    "touched_at" => Expr.fn_call("now", []),
    "status" => Expr.case_when(
      [Expr.eq(Expr.column("status"), Expr.value("resolved"))],
      [Expr.value("unresolved")],
      Expr.column("status")
    )
  },
  q)
```

Use `Expr.null()` explicitly for a real null assignment:

```mesh
Repo.update_where_expr(pool, Issue.__table__(), %{"assigned_to" => Expr.null()}, q)?
```

### Keep common DDL neutral

Use `Migration.*` for common DDL:

```mesh
Migration.create_index(pool,
  "events",
  ["issue_id", "received_at:DESC"],
  "name:idx_events_issue_received")?
```

When DDL depends on PostgreSQL-only behavior, use `Pg.*` rather than hiding it behind a false portable abstraction.

## PostgreSQL-only `Pg.*` extras

Typed wrappers include `Pg.uuid(...)`, `Pg.timestamptz(...)`, `Pg.text(...)`, and `Pg.cast(...)`.

JSONB and text-search helpers stay explicit:

```mesh
let search_vector = Pg.to_tsvector("english", Expr.column("message"))
let search_terms = Pg.plainto_tsquery("english", Expr.value(search_query))

let q = Query.from(Event.__table__())
  |> Query.where_expr(Pg.tsvector_matches(search_vector, search_terms))
  |> Query.select_expr(Expr.label(Pg.ts_rank(search_vector, search_terms), "rank"))
```

PostgreSQL-specific schema and extension helpers include:

```mesh
Pg.create_extension(pool, "pgcrypto")?
Pg.create_gin_index(pool, "events", "idx_events_tags", "tags", "jsonb_path_ops")?
Pg.create_range_partitioned_table(pool, "events", [...], "received_at")?
Pg.create_daily_partitions_ahead(pool, "events", days)?
Pg.list_daily_partitions_before(pool, "events", max_days)?
Pg.drop_partition(pool, partition_name)?
```

## Escape hatches

- `Repo.query_raw(pool, sql, params)` — raw reads
- `Repo.execute_raw(pool, sql, params)` — raw writes and updates
- `Migration.execute(pool, sql)` — raw DDL

Prefer builders for common expression/query/write/schema shapes. Use an escape hatch when the SQL shape cannot be represented faithfully, and keep that database-specific choice visible.

## Proof and failure map

| Surface | Proof command | Primary files |
| --- | --- | --- |
| neutral expression/query/repository runtime | `cargo test -p mesh-rt --locked db::` | `compiler/mesh-rt/src/db/` |
| compiler integration and PostgreSQL helpers | `cargo test -p meshc --locked --test e2e` | `compiler/meshc/tests/e2e.rs` |
| migration scaffolding | `cargo test -p meshc --locked --test e2e` | `compiler/meshc/src/migrate.rs`, `compiler/meshc/tests/e2e.rs` |
| docs rendering | `npm --prefix website run build` | `website/docs/docs/databases/index.md` |

## What's next?

- [Autonomous Clusters](/docs/autonomous-clusters/) — scaling, routing, continuity, and PostgreSQL-backed release proof
- [Web](/docs/web/) — HTTP and WebSocket primitives that consume storage helpers
