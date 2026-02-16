//! Migration DDL generation module for the Mesh runtime.
//!
//! Provides eight `extern "C"` DDL builder functions that produce correctly
//! quoted PostgreSQL DDL SQL and execute it via `Pool.execute`:
//!
//! - `mesh_migration_create_table`: CREATE TABLE IF NOT EXISTS with column defs
//! - `mesh_migration_drop_table`: DROP TABLE IF EXISTS
//! - `mesh_migration_add_column`: ALTER TABLE ADD COLUMN IF NOT EXISTS
//! - `mesh_migration_drop_column`: ALTER TABLE DROP COLUMN IF EXISTS
//! - `mesh_migration_rename_column`: ALTER TABLE RENAME COLUMN
//! - `mesh_migration_create_index`: CREATE [UNIQUE] INDEX IF NOT EXISTS
//! - `mesh_migration_drop_index`: DROP INDEX IF EXISTS
//! - `mesh_migration_execute`: Raw SQL escape hatch via Pool.execute
//!
//! All functions accept Mesh runtime types (MeshString pointers, List pointers)
//! and execute the generated DDL via `mesh_pool_execute`. SQL identifiers are
//! double-quoted per PostgreSQL convention.

use crate::collections::list::{mesh_list_get, mesh_list_length, mesh_list_new};
use crate::db::pool::mesh_pool_execute;
use crate::string::{mesh_string_new, MeshString};

// ── Helpers ──────────────────────────────────────────────────────────

/// Quote a SQL identifier with double quotes (PostgreSQL convention).
/// Escapes embedded double quotes by doubling them.
fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Extract a Vec<String> from a Mesh List<String> pointer.
unsafe fn list_to_strings(list_ptr: *mut u8) -> Vec<String> {
    let len = mesh_list_length(list_ptr);
    let mut result = Vec::with_capacity(len as usize);
    for i in 0..len {
        let elem = mesh_list_get(list_ptr, i) as *const MeshString;
        if !elem.is_null() {
            result.push((*elem).as_str().to_string());
        }
    }
    result
}

/// Create a MeshString from a Rust &str and return as *mut u8.
unsafe fn rust_string_to_mesh(s: &str) -> *mut u8 {
    mesh_string_new(s.as_ptr(), s.len() as u64) as *mut u8
}

// ── Pure Rust SQL builders (testable without GC) ─────────────────────

/// Build CREATE TABLE SQL from table name and column definitions.
///
/// Each column entry is colon-separated: `"name:TYPE:CONSTRAINTS"` (3 parts)
/// or `"name:TYPE"` (2 parts). Column names are quoted; types and constraints
/// are passed through verbatim.
///
/// Example: `["id:UUID:PRIMARY KEY", "name:TEXT:NOT NULL", "age:BIGINT"]`
/// produces: `CREATE TABLE IF NOT EXISTS "t" ("id" UUID PRIMARY KEY, "name" TEXT NOT NULL, "age" BIGINT)`
pub(crate) fn build_create_table_sql(table: &str, columns: &[String]) -> String {
    let mut sql = format!("CREATE TABLE IF NOT EXISTS {}", quote_ident(table));
    sql.push_str(" (");
    let col_defs: Vec<String> = columns
        .iter()
        .map(|c| {
            let parts: Vec<&str> = c.splitn(3, ':').collect();
            match parts.len() {
                3 => format!("{} {} {}", quote_ident(parts[0]), parts[1], parts[2]),
                2 => format!("{} {}", quote_ident(parts[0]), parts[1]),
                _ => c.to_string(),
            }
        })
        .collect();
    sql.push_str(&col_defs.join(", "));
    sql.push(')');
    sql
}

/// Build DROP TABLE SQL.
pub(crate) fn build_drop_table_sql(table: &str) -> String {
    format!("DROP TABLE IF EXISTS {}", quote_ident(table))
}

/// Build ADD COLUMN SQL from table name and column definition.
///
/// Column definition uses same colon encoding as create_table.
pub(crate) fn build_add_column_sql(table: &str, column_def: &str) -> String {
    let parts: Vec<&str> = column_def.splitn(3, ':').collect();
    match parts.len() {
        3 => format!(
            "ALTER TABLE {} ADD COLUMN IF NOT EXISTS {} {} {}",
            quote_ident(table),
            quote_ident(parts[0]),
            parts[1],
            parts[2]
        ),
        2 => format!(
            "ALTER TABLE {} ADD COLUMN IF NOT EXISTS {} {}",
            quote_ident(table),
            quote_ident(parts[0]),
            parts[1]
        ),
        _ => format!(
            "ALTER TABLE {} ADD COLUMN {}",
            quote_ident(table),
            column_def
        ),
    }
}

/// Build DROP COLUMN SQL.
pub(crate) fn build_drop_column_sql(table: &str, column: &str) -> String {
    format!(
        "ALTER TABLE {} DROP COLUMN IF EXISTS {}",
        quote_ident(table),
        quote_ident(column)
    )
}

/// Build RENAME COLUMN SQL.
pub(crate) fn build_rename_column_sql(table: &str, old_name: &str, new_name: &str) -> String {
    format!(
        "ALTER TABLE {} RENAME COLUMN {} TO {}",
        quote_ident(table),
        quote_ident(old_name),
        quote_ident(new_name)
    )
}

/// Build CREATE INDEX SQL.
///
/// Options is a string with space-separated key:value pairs:
/// - `unique:true` -- creates a UNIQUE index
/// - `where:condition` -- adds a WHERE clause for partial index
///
/// The index name is auto-generated as `idx_{table}_{col1}_{col2}`.
pub(crate) fn build_create_index_sql(
    table: &str,
    columns: &[String],
    options: &str,
) -> String {
    let is_unique = options.contains("unique:true") || options.contains("unique: true");
    let index_name = format!("idx_{}_{}", table, columns.join("_"));
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
///
/// The index name is derived as `idx_{table}_{col1}_{col2}` to match
/// the convention used by `build_create_index_sql`.
pub(crate) fn build_drop_index_sql(table: &str, columns: &[String]) -> String {
    let index_name = format!("idx_{}_{}", table, columns.join("_"));
    format!("DROP INDEX IF EXISTS {}", quote_ident(&index_name))
}

// ── Extern C wrappers ───────────────────────────────────────────────

/// Create a table with the given column definitions.
///
/// # Signature
///
/// `mesh_migration_create_table(pool: u64, table: ptr, columns: ptr) -> ptr`
///
/// - `pool`: Pool handle (i64/u64)
/// - `table`: MeshString table name
/// - `columns`: List<String> of colon-separated column definitions
///
/// Returns: Result<Int, String> (from Pool.execute)
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
        let sql_ptr = rust_string_to_mesh(&sql) as *const MeshString;
        let empty_params = mesh_list_new();
        mesh_pool_execute(pool, sql_ptr, empty_params)
    }
}

/// Drop a table.
///
/// # Signature
///
/// `mesh_migration_drop_table(pool: u64, table: ptr) -> ptr`
#[no_mangle]
pub extern "C" fn mesh_migration_drop_table(
    pool: u64,
    table: *const MeshString,
) -> *mut u8 {
    unsafe {
        let table_name = (*table).as_str();
        let sql = build_drop_table_sql(table_name);
        let sql_ptr = rust_string_to_mesh(&sql) as *const MeshString;
        let empty_params = mesh_list_new();
        mesh_pool_execute(pool, sql_ptr, empty_params)
    }
}

/// Add a column to an existing table.
///
/// # Signature
///
/// `mesh_migration_add_column(pool: u64, table: ptr, column_def: ptr) -> ptr`
#[no_mangle]
pub extern "C" fn mesh_migration_add_column(
    pool: u64,
    table: *const MeshString,
    column_def: *const MeshString,
) -> *mut u8 {
    unsafe {
        let table_name = (*table).as_str();
        let col_def = (*column_def).as_str();
        let sql = build_add_column_sql(table_name, col_def);
        let sql_ptr = rust_string_to_mesh(&sql) as *const MeshString;
        let empty_params = mesh_list_new();
        mesh_pool_execute(pool, sql_ptr, empty_params)
    }
}

/// Drop a column from an existing table.
///
/// # Signature
///
/// `mesh_migration_drop_column(pool: u64, table: ptr, column: ptr) -> ptr`
#[no_mangle]
pub extern "C" fn mesh_migration_drop_column(
    pool: u64,
    table: *const MeshString,
    column: *const MeshString,
) -> *mut u8 {
    unsafe {
        let table_name = (*table).as_str();
        let col_name = (*column).as_str();
        let sql = build_drop_column_sql(table_name, col_name);
        let sql_ptr = rust_string_to_mesh(&sql) as *const MeshString;
        let empty_params = mesh_list_new();
        mesh_pool_execute(pool, sql_ptr, empty_params)
    }
}

/// Rename a column in an existing table.
///
/// # Signature
///
/// `mesh_migration_rename_column(pool: u64, table: ptr, old_name: ptr, new_name: ptr) -> ptr`
#[no_mangle]
pub extern "C" fn mesh_migration_rename_column(
    pool: u64,
    table: *const MeshString,
    old_name: *const MeshString,
    new_name: *const MeshString,
) -> *mut u8 {
    unsafe {
        let table_name = (*table).as_str();
        let old = (*old_name).as_str();
        let new = (*new_name).as_str();
        let sql = build_rename_column_sql(table_name, old, new);
        let sql_ptr = rust_string_to_mesh(&sql) as *const MeshString;
        let empty_params = mesh_list_new();
        mesh_pool_execute(pool, sql_ptr, empty_params)
    }
}

/// Create an index on the given columns.
///
/// # Signature
///
/// `mesh_migration_create_index(pool: u64, table: ptr, columns: ptr, options: ptr) -> ptr`
///
/// Options: `"unique:true"` for unique index, `"where:condition"` for partial.
#[no_mangle]
pub extern "C" fn mesh_migration_create_index(
    pool: u64,
    table: *const MeshString,
    columns: *mut u8,
    options: *const MeshString,
) -> *mut u8 {
    unsafe {
        let table_name = (*table).as_str();
        let cols = list_to_strings(columns);
        let opts = (*options).as_str();
        let sql = build_create_index_sql(table_name, &cols, opts);
        let sql_ptr = rust_string_to_mesh(&sql) as *const MeshString;
        let empty_params = mesh_list_new();
        mesh_pool_execute(pool, sql_ptr, empty_params)
    }
}

/// Drop an index (derived name: idx_{table}_{col1}_{col2}).
///
/// # Signature
///
/// `mesh_migration_drop_index(pool: u64, table: ptr, columns: ptr) -> ptr`
#[no_mangle]
pub extern "C" fn mesh_migration_drop_index(
    pool: u64,
    table: *const MeshString,
    columns: *mut u8,
) -> *mut u8 {
    unsafe {
        let table_name = (*table).as_str();
        let cols = list_to_strings(columns);
        let sql = build_drop_index_sql(table_name, &cols);
        let sql_ptr = rust_string_to_mesh(&sql) as *const MeshString;
        let empty_params = mesh_list_new();
        mesh_pool_execute(pool, sql_ptr, empty_params)
    }
}

/// Execute raw SQL (escape hatch for operations not covered by the DSL).
///
/// # Signature
///
/// `mesh_migration_execute(pool: u64, sql: ptr) -> ptr`
#[no_mangle]
pub extern "C" fn mesh_migration_execute(
    pool: u64,
    sql: *const MeshString,
) -> *mut u8 {
    let empty_params = mesh_list_new();
    mesh_pool_execute(pool, sql, empty_params)
}

// ── Unit tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_create_table_sql() {
        let sql = build_create_table_sql(
            "users",
            &[
                "id:UUID:PRIMARY KEY".to_string(),
                "name:TEXT:NOT NULL".to_string(),
                "age:BIGINT".to_string(),
            ],
        );
        assert_eq!(
            sql,
            "CREATE TABLE IF NOT EXISTS \"users\" (\"id\" UUID PRIMARY KEY, \"name\" TEXT NOT NULL, \"age\" BIGINT)"
        );
    }

    #[test]
    fn test_build_create_table_sql_two_part_columns() {
        let sql = build_create_table_sql(
            "posts",
            &[
                "id:SERIAL".to_string(),
                "title:TEXT".to_string(),
            ],
        );
        assert_eq!(
            sql,
            "CREATE TABLE IF NOT EXISTS \"posts\" (\"id\" SERIAL, \"title\" TEXT)"
        );
    }

    #[test]
    fn test_build_drop_table_sql() {
        let sql = build_drop_table_sql("users");
        assert_eq!(sql, "DROP TABLE IF EXISTS \"users\"");
    }

    #[test]
    fn test_build_add_column_sql() {
        let sql = build_add_column_sql("users", "age:BIGINT:NOT NULL DEFAULT 0");
        assert_eq!(
            sql,
            "ALTER TABLE \"users\" ADD COLUMN IF NOT EXISTS \"age\" BIGINT NOT NULL DEFAULT 0"
        );
    }

    #[test]
    fn test_build_add_column_sql_two_part() {
        let sql = build_add_column_sql("users", "bio:TEXT");
        assert_eq!(
            sql,
            "ALTER TABLE \"users\" ADD COLUMN IF NOT EXISTS \"bio\" TEXT"
        );
    }

    #[test]
    fn test_build_drop_column_sql() {
        let sql = build_drop_column_sql("users", "age");
        assert_eq!(
            sql,
            "ALTER TABLE \"users\" DROP COLUMN IF EXISTS \"age\""
        );
    }

    #[test]
    fn test_build_rename_column_sql() {
        let sql = build_rename_column_sql("users", "name", "full_name");
        assert_eq!(
            sql,
            "ALTER TABLE \"users\" RENAME COLUMN \"name\" TO \"full_name\""
        );
    }

    #[test]
    fn test_build_create_index_sql() {
        let sql = build_create_index_sql(
            "users",
            &["email".to_string()],
            "",
        );
        assert_eq!(
            sql,
            "CREATE INDEX IF NOT EXISTS \"idx_users_email\" ON \"users\" (\"email\")"
        );
    }

    #[test]
    fn test_build_create_index_sql_unique() {
        let sql = build_create_index_sql(
            "users",
            &["email".to_string()],
            "unique:true",
        );
        assert_eq!(
            sql,
            "CREATE UNIQUE INDEX IF NOT EXISTS \"idx_users_email\" ON \"users\" (\"email\")"
        );
    }

    #[test]
    fn test_build_create_index_sql_multi_column() {
        let sql = build_create_index_sql(
            "orders",
            &["user_id".to_string(), "status".to_string()],
            "",
        );
        assert_eq!(
            sql,
            "CREATE INDEX IF NOT EXISTS \"idx_orders_user_id_status\" ON \"orders\" (\"user_id\", \"status\")"
        );
    }

    #[test]
    fn test_build_create_index_sql_partial() {
        let sql = build_create_index_sql(
            "users",
            &["email".to_string()],
            "unique:true where:active = true",
        );
        assert_eq!(
            sql,
            "CREATE UNIQUE INDEX IF NOT EXISTS \"idx_users_email\" ON \"users\" (\"email\") WHERE active = true"
        );
    }

    #[test]
    fn test_build_drop_index_sql() {
        let sql = build_drop_index_sql(
            "users",
            &["email".to_string()],
        );
        assert_eq!(
            sql,
            "DROP INDEX IF EXISTS \"idx_users_email\""
        );
    }

    #[test]
    fn test_build_drop_index_sql_multi_column() {
        let sql = build_drop_index_sql(
            "orders",
            &["user_id".to_string(), "status".to_string()],
        );
        assert_eq!(
            sql,
            "DROP INDEX IF EXISTS \"idx_orders_user_id_status\""
        );
    }

    #[test]
    fn test_quote_ident_with_double_quotes() {
        // Table name with embedded double quote should be escaped
        let sql = build_drop_table_sql("my\"table");
        assert_eq!(sql, "DROP TABLE IF EXISTS \"my\"\"table\"");
    }
}
