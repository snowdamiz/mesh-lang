# Phase 101: Migration System - Research

**Researched:** 2026-02-16
**Domain:** Mesh compiler CLI + runtime SQL DDL generation + migration file management
**Confidence:** HIGH

## Summary

Phase 101 adds a versioned migration system to the Mesh ORM. The system has three distinct implementation areas: (1) a Migration DSL implemented as runtime Rust functions that generate DDL SQL strings and execute them via `Pool.execute`, (2) a migration runner implemented as runtime Rust functions that discover migration files, track applied state in a `_mesh_migrations` table, and apply/rollback within transactions, and (3) CLI integration via a new `meshc migrate` subcommand with sub-subcommands (up, down, status, generate).

The Migration DSL functions (`Migration.create_table`, `Migration.alter_table`, `Migration.drop_table`, `Migration.create_index`, `Migration.drop_index`) follow the same pattern as existing ORM modules (Orm, Query, Repo, Changeset): type signatures registered in `infer.rs`, known_functions in `lower.rs`, LLVM intrinsic declarations in `intrinsics.rs`, and `extern "C"` implementations in a new `crates/mesh-rt/src/db/migration.rs` module. These functions take a pool handle and structured parameters, build DDL SQL strings internally, and execute them via `Pool.execute`.

Migration files are standard `.mpl` Mesh source files with two public functions: `pub fn up(pool :: PoolHandle) -> Int!String` and `pub fn down(pool :: PoolHandle) -> Int!String`. They live in a `migrations/` directory within the project and are named with timestamp prefixes (e.g., `migrations/20260216120000_create_users.mpl`). The migration runner discovers these files, compares against the `_mesh_migrations` tracking table, and applies pending ones in timestamp order.

The CLI `meshc migrate` subcommand is the most architecturally significant piece: it must compile and execute migration files, not just the main project. This requires either (a) compiling each migration independently and running it, or (b) generating a synthetic entry point that calls migrations in order. The recommended approach is (b): the runner generates a temporary `main.mpl` that imports all pending migration modules and calls their `up(pool)` functions in sequence within transactions.

**Primary recommendation:** Implement in three plans: (1) Migration DSL runtime functions for DDL generation, (2) migration runner + tracking table + CLI subcommand, (3) scaffold generation and expand-migrate-contract documentation.

## Standard Stack

### Core

| Crate | Location | Purpose | Relevance |
|-------|----------|---------|-----------|
| mesh-rt | crates/mesh-rt | Migration DSL runtime functions | New `db/migration.rs` module with DDL builders |
| mesh-codegen | crates/mesh-codegen | LLVM intrinsics + known_functions | Declare + register Migration module functions |
| mesh-typeck | crates/mesh-typeck | Type signatures | Register Migration module in stdlib modules |
| meshc | crates/meshc | CLI subcommand | New `Migrate` variant in Commands enum |
| mesh-pkg | crates/mesh-pkg | Scaffold generation | File creation utilities (same pattern as `scaffold_project`) |
| clap | workspace | CLI argument parsing | Already used by meshc; add `Migrate` subcommand |

### Supporting

| Library | Version | Purpose | When Used |
|---------|---------|---------|-----------|
| chrono | (from std) | Timestamp generation | `meshc migrate generate` timestamp prefix (use `std::time` + manual formatting, no extra dep needed) |
| inkwell | 0.8.0 (LLVM 21.1) | LLVM intrinsic declarations | Declare migration runtime functions |

### Alternatives Considered

| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| DDL builder functions in Rust | Raw SQL strings in Mesh migrations | Raw SQL is simpler but loses type-safety and portability; builder functions provide consistent quoting and validation |
| Synthetic main.mpl runner | Compile each migration separately | Separate compilation is simpler conceptually but requires N compile+link cycles; synthetic entry point compiles once |
| Timestamp-prefix file naming | Sequential integer version numbers | Timestamps prevent merge conflicts when multiple developers create migrations concurrently; integers are simpler but conflict-prone |

## Architecture Patterns

### Recommended Project Structure

```
my_project/
  mesh.toml
  main.mpl
  migrations/
    20260216120000_create_users.mpl
    20260216120100_create_posts.mpl
    20260216120200_add_email_index.mpl
```

### Pattern 1: Migration DSL as Runtime Functions

**What:** Migration DSL functions are `extern "C"` Rust functions in `mesh-rt/src/db/migration.rs` that build DDL SQL and execute it via `Pool.execute`. They follow the exact same pattern as `orm.rs` (Phase 97) and `repo.rs` (Phase 98).

**When to use:** All DDL operations in migration files.

**Implementation approach:**

Each DSL function takes a pool handle (i64) and structured parameters describing the table/column/index to create/modify. The function internally builds a SQL DDL string (CREATE TABLE, ALTER TABLE, etc.) and calls `mesh_pool_execute` to execute it.

```rust
// crates/mesh-rt/src/db/migration.rs

/// Build CREATE TABLE SQL from table name and column definitions.
///
/// columns: List<String> where each entry is "column_name:type:constraints"
/// e.g., ["id:UUID:PRIMARY KEY DEFAULT gen_random_uuid()", "name:TEXT:NOT NULL", "age:BIGINT"]
fn build_create_table_sql(table: &str, columns: &[String]) -> String {
    let mut sql = format!("CREATE TABLE IF NOT EXISTS {}", quote_ident(table));
    sql.push_str(" (");
    let col_defs: Vec<String> = columns.iter().map(|c| {
        let parts: Vec<&str> = c.splitn(3, ':').collect();
        match parts.len() {
            3 => format!("{} {} {}", quote_ident(parts[0]), parts[1], parts[2]),
            2 => format!("{} {}", quote_ident(parts[0]), parts[1]),
            _ => quote_ident(c).to_string(),
        }
    }).collect();
    sql.push_str(&col_defs.join(", "));
    sql.push(')');
    sql
}

#[no_mangle]
pub extern "C" fn mesh_migration_create_table(
    pool: u64,
    table: *const MeshString,
    columns: *mut u8,
) -> *mut u8 {
    unsafe {
        let table_name = (*table).as_str();
        let cols = list_to_strings(columns);
        let sql = build_create_table_sql(table_name, &cols);
        let sql_ptr = rust_str_to_mesh(&sql) as *const MeshString;
        let empty_params = mesh_list_new();
        mesh_pool_execute(pool, sql_ptr, empty_params)
    }
}
```

**Mesh-level API:**

```mesh
pub fn up(pool :: PoolHandle) -> Int!String do
  Migration.create_table(pool, "users", [
    "id:UUID:PRIMARY KEY DEFAULT gen_random_uuid()",
    "name:TEXT:NOT NULL",
    "email:TEXT:NOT NULL UNIQUE",
    "inserted_at:TIMESTAMPTZ:NOT NULL DEFAULT now()",
    "updated_at:TIMESTAMPTZ:NOT NULL DEFAULT now()"
  ])?

  Migration.create_index(pool, "users", ["email"], "unique: true")?
  Ok(0)
end

pub fn down(pool :: PoolHandle) -> Int!String do
  Migration.drop_table(pool, "users")?
  Ok(0)
end
```

### Pattern 2: Column Definition Encoding

**What:** Column definitions are encoded as colon-separated strings in a List<String>, following the established pattern from Phase 96-04 (relationship metadata: "kind:name:target") and Phase 97-01 (field types: "field_name:sql_type").

**Format:** `"column_name:sql_type:constraints"` where constraints is optional.

**Examples:**
- `"id:UUID:PRIMARY KEY DEFAULT gen_random_uuid()"` -- UUID primary key with auto-generation
- `"name:TEXT:NOT NULL"` -- required text column
- `"email:TEXT:NOT NULL UNIQUE"` -- required unique text column
- `"age:BIGINT"` -- nullable bigint column (no constraints)
- `"inserted_at:TIMESTAMPTZ:NOT NULL DEFAULT now()"` -- timestamp with default

**Why strings not structs:** The existing ORM infrastructure uses encoded strings for all structured metadata (field types, relationship metadata, WHERE clauses). This avoids complex MIR map/struct construction and keeps the Mesh-level API simple. The Rust runtime function parses the encoding and builds correct DDL.

### Pattern 3: Migration Runner as Synthetic Compilation

**What:** The `meshc migrate` command works by generating a temporary Mesh program that imports and calls migration functions, then compiling and executing it.

**Steps:**
1. Read `_mesh_migrations` tracking table to find applied versions
2. Discover `migrations/*.mpl` files, sort by timestamp prefix
3. For `migrate up`: generate a temporary `main.mpl` that:
   - Connects to the database (reads DATABASE_URL from env)
   - Opens a pool
   - For each pending migration, within a transaction:
     - Calls `ModuleName.up(pool)`
     - Inserts a row into `_mesh_migrations`
   - Closes pool
4. Compile the synthetic project and run the binary
5. For `migrate down`: same approach but calls last applied migration's `down(pool)` and deletes the tracking row

**Alternative approach (simpler, recommended):** Instead of generating a synthetic Mesh program, implement the migration runner entirely in Rust within `meshc`. The runner:
1. Reads DATABASE_URL from environment
2. Connects to PostgreSQL directly using `mesh_pg_connect` (the same PG wire protocol already in mesh-rt)
3. Creates `_mesh_migrations` table if not exists
4. Discovers migration files, determines pending ones
5. For each pending migration: compiles the migration file as a standalone module, links and runs it
6. The migration binary connects to the same DB and executes its `up()` function

**Recommended approach (most practical):** Implement the runner in Rust but use a hybrid approach. The migration DSL functions (`Migration.create_table` etc.) are runtime functions callable from Mesh. The runner itself is Rust code in meshc that:
1. Connects to PG directly (reuse mesh-rt PG wire protocol) for tracking table management
2. For each pending migration, compiles a synthetic Mesh program that calls the migration's up/down
3. Runs the compiled binary

Actually, the simplest and most consistent approach: **the migration runner is itself a runtime function** (`mesh_migration_run`). The `meshc migrate` CLI subcommand generates a minimal Mesh program that calls `Migration.run(pool, "up")` or `Migration.run(pool, "down")`, compiles it, and executes it. The `Migration.run` runtime function handles all the discovery, ordering, transaction wrapping, and tracking table management.

But this has a circular problem: the runtime function would need to discover and execute Mesh migration modules, which requires compilation. This cannot happen at runtime.

**Final recommended approach:** The migration runner logic lives in Rust within `meshc` (not in mesh-rt). The CLI:

1. `meshc migrate` (or `meshc migrate up`):
   - Connects to PG using mesh-rt's PG wire protocol directly from Rust
   - Creates `_mesh_migrations` tracking table if not exists
   - Discovers `migrations/*.mpl` files in the project directory
   - Queries tracking table for already-applied versions
   - For each pending migration (timestamp order):
     - Generates a synthetic `_migrate_main.mpl` that imports the migration module and calls `up(pool)`
     - Compiles this as a mini-project (migration file + synthetic main)
     - Runs the resulting binary
     - If successful, records the version in `_mesh_migrations`
   - Cleans up temporary files

2. `meshc migrate down`:
   - Same connection + tracking table pattern
   - Finds the last applied migration
   - Generates synthetic main that calls `down(pool)`
   - Compiles and runs
   - Deletes the tracking row

3. `meshc migrate status`:
   - Connects, reads tracking table
   - Lists applied vs pending migrations

4. `meshc migrate generate <name>`:
   - Creates `migrations/` directory if not exists
   - Generates timestamped file with up/down stubs

This approach reuses the existing compilation pipeline completely and requires no new runtime infrastructure for the runner itself. The DSL functions (create_table, etc.) are the runtime part.

### Pattern 4: Tracking Table Schema

**What:** The `_mesh_migrations` table stores which migrations have been applied.

**Schema:**
```sql
CREATE TABLE IF NOT EXISTS _mesh_migrations (
    version BIGINT PRIMARY KEY,
    name TEXT NOT NULL,
    applied_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
```

- `version`: The timestamp prefix from the filename (e.g., 20260216120000), stored as BIGINT for easy ordering
- `name`: The human-readable migration name (e.g., "create_users")
- `applied_at`: When the migration was applied

**Why BIGINT for version:** Timestamp prefixes like 20260216120000 fit in BIGINT (8 bytes). Using BIGINT allows simple numeric comparison for ordering and range queries. TEXT would work but numeric comparison is more natural for "which migrations are newer than X?"

### Pattern 5: Synthetic Main for Migration Execution

**What:** To execute a migration, `meshc` generates a temporary Mesh main file that connects to the database and calls the migration's up/down function.

**Example synthetic main for running `20260216120000_create_users.mpl` up:**

```mesh
import Migrations.CreateUsers

fn main() do
  let url = Env.get("DATABASE_URL")
  let pool_result = Pool.open(url, 1, 2, 5000)
  match pool_result do
    Ok(pool) ->
      let result = Migrations.CreateUsers.up(pool)
      match result do
        Ok(_) -> IO.puts("Migration applied: 20260216120000_create_users")
        Err(e) -> IO.puts("Migration failed: " <> e)
      end
      Pool.close(pool)
    Err(e) ->
      IO.puts("Connection failed: " <> e)
  end
end
```

**Key insight:** The migration file is a normal Mesh module. It follows the standard module conventions: filename maps to module name (e.g., `migrations/20260216120000_create_users.mpl` becomes module `Migrations.CreateUsers`). The synthetic main imports this module and calls its `up()` or `down()` function.

**Module name derivation:** The file `migrations/20260216120000_create_users.mpl` has path `migrations/20260216120000_create_users.mpl` relative to project root. Using the existing `path_to_module_name` function from `discovery.rs`, this becomes `Migrations.20260216120000CreateUsers` (PascalCase). However, the numeric prefix creates an invalid Mesh identifier (identifiers cannot start with digits after the dot).

**Solution:** Strip the numeric prefix for the module name. The discovery/naming can treat migration files specially: `20260216120000_create_users.mpl` -> module name `CreateUsers` within the `Migrations` namespace. Or simpler: the runner doesn't need module imports at all. Instead, it can create a temporary directory structure where the migration file is renamed/copied to a clean module name and compiled as part of a temporary project.

**Simplest approach:** Copy the migration file to a temporary directory as `migration.mpl`, generate a `main.mpl` that does `import Migration` and calls `Migration.up(pool)`. Compile in the temp directory. Run. This avoids all module naming issues.

### Anti-Patterns to Avoid

- **Running migrations without transactions:** Each migration must execute within a transaction. If a migration fails partway through, the transaction ensures it is fully rolled back. PostgreSQL supports transactional DDL (unlike MySQL), so `CREATE TABLE` etc. are transactional.
- **Storing migration state in files:** Migration state must be in the database (the `_mesh_migrations` table), not in a local file. This ensures all instances of an application agree on which migrations have been applied.
- **Applying migrations in non-deterministic order:** Migrations must always be applied in timestamp order. Discovery should sort files by their numeric prefix before processing.
- **Combining schema changes with data migrations in one migration:** Each migration should be either a schema change or a data migration, not both. This supports the expand-migrate-contract pattern.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| PG wire protocol for runner | New connection logic | Reuse `mesh-rt` PG functions from Rust | mesh_pg_connect, pg_simple_command already handle auth, TLS, etc. |
| SQL identifier quoting | Manual quoting in DDL builders | Reuse `quote_ident` from orm.rs | Already handles escaping, tested |
| File discovery in migrations/ | Custom walker | Reuse patterns from `discovery.rs` | discover_mesh_files already walks directories |
| CLI argument parsing | Manual arg parsing | Extend existing clap `Commands` enum | meshc already uses clap; just add Migrate variant |
| Project compilation | Custom compiler invocation | Reuse `build()` function from main.rs | The existing build pipeline handles everything |
| Timestamp formatting | chrono dependency | `std::time::SystemTime` + manual format | YYYYMMDDHHMMSS is simple enough; no new deps needed |

**Key insight:** The migration system is primarily a CLI orchestration layer on top of existing infrastructure. The DDL builder functions are the only truly new runtime code. Everything else (compilation, PG connection, file discovery) reuses existing crate APIs.

## Common Pitfalls

### Pitfall 1: Module Naming for Migration Files with Numeric Prefixes

**What goes wrong:** Migration files named `20260216120000_create_users.mpl` produce module names starting with digits after PascalCase conversion, which are invalid identifiers in most languages (including Mesh).
**Why it happens:** The existing `path_to_module_name` function in `discovery.rs` converts path components to PascalCase: `20260216120000_create_users` -> `20260216120000CreateUsers`. Identifiers starting with digits are likely not valid in Mesh's lexer.
**How to avoid:** The migration runner should NOT use the standard module discovery. Instead, it should copy the migration file to a temporary location with a clean name (e.g., `migration.mpl`) and compile a temporary project. This completely sidesteps the naming issue.
**Warning signs:** Parser error "expected identifier" or "invalid module name" when trying to import a migration module.

### Pitfall 2: Transaction Wrapping for DDL

**What goes wrong:** A migration fails partway through (e.g., second CREATE TABLE fails) but the first table was already created, leaving the database in an inconsistent state.
**Why it happens:** DDL statements are auto-committed in some databases (MySQL). In PostgreSQL, DDL IS transactional, but only if the migration is wrapped in BEGIN/COMMIT.
**How to avoid:** The migration runner must execute each migration within a transaction: BEGIN before calling up(), COMMIT on success, ROLLBACK on failure. Since `Pool.execute` auto-commits, the runner should use the pool checkout/begin/commit/rollback/checkin pattern (same as `Repo.transaction`). The synthetic main should wrap the up/down call in `Repo.transaction`.
**Warning signs:** Partial schema changes after a failed migration.

### Pitfall 3: DATABASE_URL Not Set

**What goes wrong:** `meshc migrate` fails with a cryptic error because DATABASE_URL environment variable is not set.
**Why it happens:** The migration runner needs a database connection but there is no configuration file for database settings.
**How to avoid:** Check for DATABASE_URL early and provide a clear error message: "meshc migrate: DATABASE_URL environment variable is required". This matches the convention used by most migration tools (Ecto, Diesel, ActiveRecord).
**Warning signs:** "connection refused" or "invalid URL" errors instead of a helpful "set DATABASE_URL" message.

### Pitfall 4: Migration File Discovery Order

**What goes wrong:** Migrations are applied in filesystem order (alphabetical) which happens to work for some timestamp formats but breaks for others.
**Why it happens:** Different filesystems may return entries in different orders. `std::fs::read_dir` provides no ordering guarantee.
**How to avoid:** Always sort migration files by their numeric prefix explicitly. Parse the timestamp prefix as an integer and sort numerically. The file naming convention ensures unique ordering.
**Warning signs:** Migrations applied in wrong order causing foreign key violations (child table before parent table).

### Pitfall 5: Concurrent Migration Runs

**What goes wrong:** Two instances of `meshc migrate` run simultaneously, both see the same pending migration, and both try to apply it.
**Why it happens:** There is no locking mechanism to prevent concurrent migration runs.
**How to avoid:** Use PostgreSQL advisory locks. Before running migrations, acquire an advisory lock: `SELECT pg_advisory_lock(101)` (using a fixed lock ID). After completion, release: `SELECT pg_advisory_unlock(101)`. This prevents concurrent migration runs without requiring external coordination.
**Warning signs:** "duplicate key value violates unique constraint" on `_mesh_migrations` table.

### Pitfall 6: ALTER TABLE Column Encoding Ambiguity

**What goes wrong:** `Migration.alter_table` needs to support multiple operations (ADD COLUMN, DROP COLUMN, RENAME COLUMN, ALTER COLUMN TYPE) but the encoding format is unclear.
**Why it happens:** ALTER TABLE has many sub-operations unlike CREATE TABLE which has a single structure.
**How to avoid:** Use separate functions for each ALTER TABLE operation: `Migration.add_column(pool, table, column_def)`, `Migration.drop_column(pool, table, column_name)`, `Migration.rename_column(pool, table, old_name, new_name)`. This is clearer than trying to encode multiple operation types into one function.
**Warning signs:** Complex string encoding that is error-prone and hard to remember.

## Code Examples

### Migration File Format (MIGR-01)

```mesh
# migrations/20260216120000_create_users.mpl
#
# Creates the users table with authentication fields.

pub fn up(pool :: PoolHandle) -> Int!String do
  Migration.create_table(pool, "users", [
    "id:UUID:PRIMARY KEY DEFAULT gen_random_uuid()",
    "email:TEXT:NOT NULL UNIQUE",
    "name:TEXT:NOT NULL",
    "password_hash:TEXT:NOT NULL",
    "inserted_at:TIMESTAMPTZ:NOT NULL DEFAULT now()",
    "updated_at:TIMESTAMPTZ:NOT NULL DEFAULT now()"
  ])?
  Ok(0)
end

pub fn down(pool :: PoolHandle) -> Int!String do
  Migration.drop_table(pool, "users")?
  Ok(0)
end
```

### Migration DSL Functions (MIGR-02, MIGR-03)

```rust
// crates/mesh-rt/src/db/migration.rs

/// Build CREATE TABLE SQL.
pub(crate) fn build_create_table_sql(table: &str, columns: &[String]) -> String {
    let mut sql = format!("CREATE TABLE IF NOT EXISTS {}", quote_ident(table));
    sql.push_str(" (\n");
    let col_defs: Vec<String> = columns.iter().map(|c| {
        let parts: Vec<&str> = c.splitn(3, ':').collect();
        match parts.len() {
            3 => format!("  {} {} {}", quote_ident(parts[0]), parts[1], parts[2]),
            2 => format!("  {} {}", quote_ident(parts[0]), parts[1]),
            _ => format!("  {}", c),
        }
    }).collect();
    sql.push_str(&col_defs.join(",\n"));
    sql.push_str("\n)");
    sql
}

/// Build DROP TABLE SQL.
pub(crate) fn build_drop_table_sql(table: &str) -> String {
    format!("DROP TABLE IF EXISTS {}", quote_ident(table))
}

/// Build ADD COLUMN SQL.
pub(crate) fn build_add_column_sql(table: &str, column_def: &str) -> String {
    let parts: Vec<&str> = column_def.splitn(3, ':').collect();
    match parts.len() {
        3 => format!(
            "ALTER TABLE {} ADD COLUMN IF NOT EXISTS {} {} {}",
            quote_ident(table), quote_ident(parts[0]), parts[1], parts[2]
        ),
        2 => format!(
            "ALTER TABLE {} ADD COLUMN IF NOT EXISTS {} {}",
            quote_ident(table), quote_ident(parts[0]), parts[1]
        ),
        _ => format!("ALTER TABLE {} ADD COLUMN {}", quote_ident(table), column_def),
    }
}

/// Build DROP COLUMN SQL.
pub(crate) fn build_drop_column_sql(table: &str, column: &str) -> String {
    format!(
        "ALTER TABLE {} DROP COLUMN IF EXISTS {}",
        quote_ident(table), quote_ident(column)
    )
}

/// Build RENAME COLUMN SQL.
pub(crate) fn build_rename_column_sql(table: &str, old_name: &str, new_name: &str) -> String {
    format!(
        "ALTER TABLE {} RENAME COLUMN {} TO {}",
        quote_ident(table), quote_ident(old_name), quote_ident(new_name)
    )
}

/// Build CREATE INDEX SQL.
/// options: space-separated key:value pairs (e.g., "unique:true", "where:active = true")
pub(crate) fn build_create_index_sql(
    table: &str,
    columns: &[String],
    options: &str,
) -> String {
    let is_unique = options.contains("unique:true") || options.contains("unique: true");
    let index_name = format!(
        "idx_{}_{}",
        table,
        columns.join("_")
    );
    let mut sql = String::new();
    sql.push_str("CREATE ");
    if is_unique {
        sql.push_str("UNIQUE ");
    }
    sql.push_str("INDEX IF NOT EXISTS ");
    sql.push_str(&quote_ident(&index_name));
    sql.push_str(" ON ");
    sql.push_str(&quote_ident(table));
    sql.push_str(" (");
    let quoted_cols: Vec<String> = columns.iter().map(|c| quote_ident(c)).collect();
    sql.push_str(&quoted_cols.join(", "));
    sql.push(')');

    // WHERE clause for partial index
    if let Some(where_start) = options.find("where:") {
        let where_clause = &options[where_start + 6..];
        sql.push_str(&format!(" WHERE {}", where_clause.trim()));
    }

    sql
}

/// Build DROP INDEX SQL.
pub(crate) fn build_drop_index_sql(table: &str, columns: &[String]) -> String {
    let index_name = format!("idx_{}_{}", table, columns.join("_"));
    format!("DROP INDEX IF EXISTS {}", quote_ident(&index_name))
}
```

### Type Checker Registration (infer.rs)

```rust
// In the stdlib module registration section, add Migration module:

// ── Migration module (Phase 101) ───────────────────────────────────
{
    let mut migration_mod = FxHashMap::default();

    // Migration.create_table: fn(PoolHandle, String, List<String>) -> Result<Int, String>
    migration_mod.insert("create_table".to_string(), Scheme::mono(Ty::fun(
        vec![pool_handle_t.clone(), Ty::string(), Ty::list(Ty::string())],
        Ty::result(Ty::int(), Ty::string()),
    )));

    // Migration.drop_table: fn(PoolHandle, String) -> Result<Int, String>
    migration_mod.insert("drop_table".to_string(), Scheme::mono(Ty::fun(
        vec![pool_handle_t.clone(), Ty::string()],
        Ty::result(Ty::int(), Ty::string()),
    )));

    // Migration.add_column: fn(PoolHandle, String, String) -> Result<Int, String>
    migration_mod.insert("add_column".to_string(), Scheme::mono(Ty::fun(
        vec![pool_handle_t.clone(), Ty::string(), Ty::string()],
        Ty::result(Ty::int(), Ty::string()),
    )));

    // Migration.drop_column: fn(PoolHandle, String, String) -> Result<Int, String>
    migration_mod.insert("drop_column".to_string(), Scheme::mono(Ty::fun(
        vec![pool_handle_t.clone(), Ty::string(), Ty::string()],
        Ty::result(Ty::int(), Ty::string()),
    )));

    // Migration.rename_column: fn(PoolHandle, String, String, String) -> Result<Int, String>
    migration_mod.insert("rename_column".to_string(), Scheme::mono(Ty::fun(
        vec![pool_handle_t.clone(), Ty::string(), Ty::string(), Ty::string()],
        Ty::result(Ty::int(), Ty::string()),
    )));

    // Migration.create_index: fn(PoolHandle, String, List<String>, String) -> Result<Int, String>
    migration_mod.insert("create_index".to_string(), Scheme::mono(Ty::fun(
        vec![pool_handle_t.clone(), Ty::string(), Ty::list(Ty::string()), Ty::string()],
        Ty::result(Ty::int(), Ty::string()),
    )));

    // Migration.drop_index: fn(PoolHandle, String, List<String>) -> Result<Int, String>
    migration_mod.insert("drop_index".to_string(), Scheme::mono(Ty::fun(
        vec![pool_handle_t.clone(), Ty::string(), Ty::list(Ty::string())],
        Ty::result(Ty::int(), Ty::string()),
    )));

    // Migration.execute: fn(PoolHandle, String) -> Result<Int, String>
    // Raw SQL escape hatch for operations not covered by DSL
    migration_mod.insert("execute".to_string(), Scheme::mono(Ty::fun(
        vec![pool_handle_t.clone(), Ty::string()],
        Ty::result(Ty::int(), Ty::string()),
    )));

    modules.insert("Migration".to_string(), migration_mod);
}
```

### CLI Subcommand (meshc main.rs)

```rust
// Add to Commands enum:
/// Run database migrations
Migrate {
    #[command(subcommand)]
    action: Option<MigrateAction>,

    /// Project directory (default: current directory)
    #[arg(default_value = ".")]
    dir: PathBuf,
},

#[derive(Subcommand)]
enum MigrateAction {
    /// Apply all pending migrations (default)
    Up,
    /// Rollback the last applied migration
    Down,
    /// Show migration status (applied vs pending)
    Status,
    /// Generate a new migration scaffold
    Generate {
        /// Migration name (e.g., "create_users")
        name: String,
    },
}
```

### Migration Runner (Rust in meshc)

```rust
// In meshc, new module: migration_runner.rs

use mesh_rt::db::pg::{mesh_pg_connect, mesh_pg_close, pg_simple_command, PgConn};

/// Execute pending migrations.
fn run_migrations_up(project_dir: &Path) -> Result<(), String> {
    let url = std::env::var("DATABASE_URL")
        .map_err(|_| "DATABASE_URL environment variable is required for migrations")?;

    // 1. Connect directly to PG for tracking table management
    // (reuse mesh-rt PG wire protocol)
    let conn = connect_pg(&url)?;

    // 2. Ensure tracking table exists
    execute_sql(&conn, "CREATE TABLE IF NOT EXISTS _mesh_migrations (
        version BIGINT PRIMARY KEY,
        name TEXT NOT NULL,
        applied_at TIMESTAMPTZ NOT NULL DEFAULT now()
    )")?;

    // 3. Read applied versions
    let applied = query_applied_versions(&conn)?;

    // 4. Discover migration files
    let migrations = discover_migrations(&project_dir.join("migrations"))?;

    // 5. Filter pending
    let pending: Vec<_> = migrations.iter()
        .filter(|m| !applied.contains(&m.version))
        .collect();

    if pending.is_empty() {
        eprintln!("No pending migrations");
        return Ok(());
    }

    // 6. Apply each pending migration
    for migration in &pending {
        eprintln!("  Applying: {}_{}", migration.version, migration.name);

        // Create a temporary project directory
        let tmp = tempfile::tempdir().map_err(|e| e.to_string())?;
        let tmp_path = tmp.path();

        // Copy migration file as migration.mpl
        let src = project_dir.join("migrations").join(&migration.filename);
        std::fs::copy(&src, tmp_path.join("migration.mpl"))
            .map_err(|e| format!("copy: {}", e))?;

        // Generate synthetic main.mpl
        let main_code = generate_migration_main("up");
        std::fs::write(tmp_path.join("main.mpl"), &main_code)
            .map_err(|e| format!("write main: {}", e))?;

        // Compile and run
        let output = tmp_path.join("_migrate");
        build(tmp_path, 0, false, Some(&output), None, &DiagnosticOptions::default())?;
        let status = std::process::Command::new(&output)
            .env("DATABASE_URL", &url)
            .status()
            .map_err(|e| format!("run: {}", e))?;

        if !status.success() {
            return Err(format!(
                "Migration {}_{} failed",
                migration.version, migration.name
            ));
        }

        // Record in tracking table
        execute_sql(&conn, &format!(
            "INSERT INTO _mesh_migrations (version, name) VALUES ({}, '{}')",
            migration.version,
            migration.name.replace('\'', "''")
        ))?;

        eprintln!("  Applied: {}_{}", migration.version, migration.name);
    }

    close_pg(conn);
    Ok(())
}

fn generate_migration_main(direction: &str) -> String {
    format!(r#"import Migration

fn main() do
  let url = Env.get("DATABASE_URL")
  let pool_result = Pool.open(url, 1, 2, 5000)
  match pool_result do
    Ok(pool) ->
      let result = Migration.{}(pool)
      match result do
        Ok(_) -> 0
        Err(e) ->
          IO.puts("error: " <> e)
          0
      end
    Err(e) ->
      IO.puts("connection error: " <> e)
      0
  end
end
"#, direction)
}
```

### Scaffold Generation (MIGR-07)

```rust
fn generate_migration(project_dir: &Path, name: &str) -> Result<(), String> {
    let migrations_dir = project_dir.join("migrations");
    std::fs::create_dir_all(&migrations_dir)
        .map_err(|e| format!("Failed to create migrations directory: {}", e))?;

    // Generate timestamp prefix: YYYYMMDDHHMMSS
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| format!("timestamp: {}", e))?;
    let secs = now.as_secs();
    // Convert to UTC datetime components
    let timestamp = format_timestamp(secs);

    let filename = format!("{}_{}.mpl", timestamp, name);
    let filepath = migrations_dir.join(&filename);

    let content = format!(r#"# Migration: {}
# Generated: {}

pub fn up(pool :: PoolHandle) -> Int!String do
  # Add your migration code here
  Ok(0)
end

pub fn down(pool :: PoolHandle) -> Int!String do
  # Add your rollback code here
  Ok(0)
end
"#, name, timestamp);

    std::fs::write(&filepath, content)
        .map_err(|e| format!("Failed to write migration file: {}", e))?;

    eprintln!("Created migration: migrations/{}", filename);
    Ok(())
}

/// Format a Unix timestamp as YYYYMMDDHHMMSS.
fn format_timestamp(secs: u64) -> String {
    // Simple UTC conversion without external dependencies
    let days_since_epoch = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Gregorian calendar calculation from days since 1970-01-01
    let (year, month, day) = days_to_ymd(days_since_epoch);

    format!("{:04}{:02}{:02}{:02}{:02}{:02}",
        year, month, day, hours, minutes, seconds)
}
```

### Tracking Table Queries

```rust
struct MigrationInfo {
    version: i64,
    name: String,
    filename: String,
}

fn discover_migrations(migrations_dir: &Path) -> Result<Vec<MigrationInfo>, String> {
    if !migrations_dir.exists() {
        return Ok(vec![]);
    }

    let mut migrations = Vec::new();
    for entry in std::fs::read_dir(migrations_dir)
        .map_err(|e| format!("read migrations dir: {}", e))? {
        let entry = entry.map_err(|e| format!("read entry: {}", e))?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("mpl") {
            continue;
        }
        let filename = path.file_name().unwrap().to_string_lossy().to_string();
        // Parse: YYYYMMDDHHMMSS_name.mpl
        if let Some(underscore_pos) = filename.find('_') {
            let version_str = &filename[..underscore_pos];
            if let Ok(version) = version_str.parse::<i64>() {
                let name = filename[underscore_pos + 1..].trim_end_matches(".mpl").to_string();
                migrations.push(MigrationInfo { version, name, filename });
            }
        }
    }

    migrations.sort_by_key(|m| m.version);
    Ok(migrations)
}
```

## State of the Art

| Old Approach (current) | New Approach (Phase 101) | Impact |
|------------------------|--------------------------|--------|
| Raw SQL DDL strings in schema.mpl (82 lines) | Migration.create_table DSL with column definitions | Consistent identifier quoting, IF NOT EXISTS by default |
| No migration tracking | `_mesh_migrations` table with version + timestamp | Reproducible deployments, audit trail |
| Manual schema management | `meshc migrate` discovers and applies pending | Automated, ordered, transactional schema changes |
| No rollback capability | `pub fn down(pool)` in each migration file | Reversible schema changes during development |
| Schema changes in main project source | Separate `migrations/` directory with timestamped files | Clean separation of schema evolution from application code |

## Open Questions

1. **Direct PG connection in meshc vs compiling a Mesh program for tracking table operations**
   - What we know: The migration runner needs to read/write the `_mesh_migrations` tracking table. This can be done either by directly using mesh-rt's PG wire protocol from Rust (meshc already links mesh-rt indirectly via mesh-codegen), or by compiling a Mesh program.
   - What's unclear: Whether meshc currently links mesh-rt at all (it links mesh-codegen, mesh-typeck, mesh-parser, but not mesh-rt directly). If it does not, the runner cannot call mesh-rt PG functions.
   - Recommendation: If meshc doesn't link mesh-rt, add it as a dependency. The tracking table operations (CREATE TABLE IF NOT EXISTS, SELECT, INSERT, DELETE) are simple enough that a direct Rust connection is cleaner than compiling Mesh programs just for bookkeeping. Alternatively, the tracking table operations can be done within the synthetic migration main using `Pool.execute` for each tracking operation.

2. **Transaction wrapping: in the runner or in the migration file?**
   - What we know: Each migration should run in a transaction for atomicity.
   - What's unclear: Should the runner wrap each migration in a transaction (BEGIN before up(), COMMIT/ROLLBACK after), or should the migration author use `Repo.transaction` explicitly?
   - Recommendation: The runner should wrap each migration in a transaction automatically. The synthetic main should use `Repo.transaction(pool, fn(conn) -> Migration.up(conn) end)`. This ensures atomicity without requiring migration authors to manage transactions manually. However, since migration DSL functions use `Pool.execute` (which auto-checkouts), the transaction should use the pool-level connection checkout pattern. The simplest approach: the synthetic main wraps the up() call in `Repo.transaction`.

3. **Migration.execute as raw SQL escape hatch**
   - What we know: Some DDL operations are not covered by the DSL (e.g., CREATE EXTENSION, custom function creation, complex ALTER TABLE with constraints).
   - What's unclear: Should there be a `Migration.execute(pool, sql)` function as an escape hatch?
   - Recommendation: Yes. `Migration.execute(pool, sql)` should be a thin wrapper around `Pool.execute(pool, sql, [])`. This allows migration authors to use raw SQL for operations the DSL does not cover. The existing `pool_execute` function handles this perfectly.

4. **meshc migrate with no migrations directory**
   - What we know: `meshc migrate` needs a `migrations/` directory.
   - What's unclear: Should it error or silently succeed when no migrations directory exists?
   - Recommendation: If `migrations/` does not exist, print "No migrations directory found. Run `meshc migrate generate <name>` to create your first migration." and exit 0. This provides guidance without failing.

5. **How to handle meshc not linking mesh-rt**
   - What we know: meshc's Cargo.toml shows dependencies on mesh-common, mesh-lexer, mesh-parser, mesh-typeck, mesh-codegen, mesh-lsp, mesh-fmt, mesh-pkg, mesh-repl. mesh-repl links mesh-rt for JIT.
   - What's unclear: Can meshc use mesh-rt PG functions through mesh-repl, or does it need a direct dependency?
   - Recommendation: Add `mesh-rt` as a direct dependency of meshc for the migration runner. This is the cleanest approach. The migration runner needs PG wire protocol access for tracking table operations.

## Sources

### Primary (HIGH confidence)
- Codebase analysis: `crates/meshc/src/main.rs` -- CLI structure with clap Commands enum, build pipeline, existing subcommands (Build, Init, Deps, Fmt, Repl, Lsp)
- Codebase analysis: `crates/meshc/src/discovery.rs` -- File discovery, module naming, project building (reusable for migration discovery)
- Codebase analysis: `crates/mesh-rt/src/db/orm.rs` -- SQL builder pattern (quote_ident, pure Rust helpers, extern C wrappers)
- Codebase analysis: `crates/mesh-rt/src/db/pool.rs` -- Pool.execute, Pool.query, checkout/checkin pattern
- Codebase analysis: `crates/mesh-rt/src/db/repo.rs` -- Repo.transaction pattern (checkout/begin/callback/commit-or-rollback/checkin)
- Codebase analysis: `crates/mesh-rt/src/db/pg.rs` -- PG wire protocol, mesh_pg_connect, mesh_pg_execute, mesh_pg_begin/commit/rollback
- Codebase analysis: `crates/mesh-typeck/src/infer.rs` -- Stdlib module registration (Pool, Orm, Repo, Changeset, Query modules)
- Codebase analysis: `crates/mesh-codegen/src/mir/lower.rs` -- known_functions registration, module method resolution
- Codebase analysis: `crates/mesh-codegen/src/codegen/intrinsics.rs` -- LLVM intrinsic declaration pattern
- Codebase analysis: `crates/mesh-pkg/src/scaffold.rs` -- Project scaffold pattern (file generation)
- Codebase analysis: `mesher/storage/schema.mpl` -- Real-world DDL patterns in Mesh (82 lines of raw SQL)
- Codebase analysis: `mesher/storage/queries.mpl` -- Pool.execute usage for DDL operations
- Codebase analysis: `mesher/types/user.mpl` -- Struct definitions with deriving, function signatures
- Planning: `.planning/ROADMAP.md` -- Phase 101 requirements, success criteria, proposed plan structure
- Planning: `.planning/REQUIREMENTS.md` -- MIGR-01 through MIGR-08 requirement definitions
- Planning: `.planning/STATE.md` -- Prior decisions from Phases 96-100

### Secondary (MEDIUM confidence)
- Phase 97 research: `.planning/phases/97-schema-metadata-sql-generation/97-RESEARCH.md` -- SQL type mapping, ORM architecture patterns
- Phase 96 research: `.planning/phases/96-compiler-additions/96-RESEARCH.md` -- Compiler pipeline patterns, module system

## Metadata

**Confidence breakdown:**
- Migration DSL (runtime functions): HIGH -- Follows exact same pattern as orm.rs (Phase 97). quote_ident, pure Rust helpers, extern C wrappers, LLVM intrinsics, type checker registration. 6 phases of prior art for this pattern.
- Migration runner (CLI + compilation): HIGH -- Uses existing meshc build pipeline, discovery patterns, and PG wire protocol. All pieces exist; the runner orchestrates them.
- Migration file format: HIGH -- Standard Mesh module with two public functions. No new language features needed. Proven pattern from schema.mpl.
- Scaffold generation: HIGH -- Trivial file generation following mesh-pkg scaffold.rs pattern.
- Transaction wrapping: MEDIUM -- The synthetic main approach with Repo.transaction should work, but the interaction between transaction wrapping and DDL execution within the migration needs testing. PostgreSQL DDL is transactional, but some operations (e.g., CREATE INDEX CONCURRENTLY) cannot run inside transactions.
- Module naming for migration files: MEDIUM -- Numeric prefix issue is real; the copy-to-temp-dir workaround is practical but adds complexity. Needs validation that the temporary project compilation works correctly.

**Research date:** 2026-02-16
**Valid until:** 2026-03-16 (compiler internals are stable, controlled by this project)
