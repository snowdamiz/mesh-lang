//! ORM SQL generation module for the Mesh runtime.
//!
//! Provides four `extern "C"` SQL builder functions that produce correctly
//! quoted, parameterized PostgreSQL SQL from structured inputs:
//!
//! - `mesh_orm_build_select`: SELECT with columns, WHERE, ORDER BY, LIMIT, OFFSET
//! - `mesh_orm_build_insert`: INSERT INTO with VALUES and RETURNING
//! - `mesh_orm_build_update`: UPDATE with SET, WHERE, and RETURNING
//! - `mesh_orm_build_delete`: DELETE FROM with WHERE and RETURNING
//!
//! All functions accept Mesh runtime types (MeshString pointers, List pointers)
//! and return MeshString pointers. SQL identifiers are double-quoted per
//! PostgreSQL convention, and parameters use $N placeholders.

use super::quote_name;
use crate::collections::list::list_strings;
use crate::string::{mesh_str, MeshString};

// ── Helpers ──────────────────────────────────────────────────────────

// ── Pure Rust SQL builders (testable without GC) ─────────────────────

/// `names` quoted and joined by commas.
fn quoted_list(names: &[String]) -> String {
    names
        .iter()
        .map(|name| quote_name(name))
        .collect::<Vec<_>>()
        .join(", ")
}

/// ` WHERE ...` for "column op" entries ("name =", "age >", "deleted_at
/// IS NULL", or a bare column for `=`), their placeholders numbered on
/// from `param_idx`; nothing for no entries.
fn where_sql(wheres: &[String], mut param_idx: usize) -> String {
    if wheres.is_empty() {
        return String::new();
    }
    let mut placeholder = || {
        param_idx += 1;
        format!("${}", param_idx - 1)
    };
    let conditions: Vec<String> = wheres
        .iter()
        .map(|w| match w.split_once(' ') {
            Some((col, op)) => match op.trim() {
                op @ ("IS NULL" | "IS NOT NULL") => format!("{} {op}", quote_name(col)),
                op => format!("{} {op} {}", quote_name(col), placeholder()),
            },
            None => format!("{} = {}", quote_name(w), placeholder()),
        })
        .collect();
    format!(" WHERE {}", conditions.join(" AND "))
}

/// ` RETURNING ...`, or nothing for no columns.
fn returning_sql(returning: &[String]) -> String {
    match returning.is_empty() {
        true => String::new(),
        false => format!(" RETURNING {}", quoted_list(returning)),
    }
}

/// Build a SELECT SQL string from pure Rust types.
fn build_select_sql(
    table: &str,
    columns: &[String],
    wheres: &[String],
    orders: &[String],
    limit: i64,
    offset: i64,
) -> String {
    let columns = match columns.is_empty() {
        true => "*".to_string(),
        false => quoted_list(columns),
    };
    let mut sql = format!(
        "SELECT {columns} FROM {}{}",
        quote_name(table),
        where_sql(wheres, 1)
    );

    // ORDER BY clause: "column direction", or a bare column (ASC)
    if !orders.is_empty() {
        let order_parts: Vec<String> = orders
            .iter()
            .map(|o| match o.rsplit_once(' ') {
                Some((col, dir)) => format!("{} {}", quote_name(col), dir.to_uppercase()),
                None => format!("{} ASC", quote_name(o)),
            })
            .collect();
        sql.push_str(&format!(" ORDER BY {}", order_parts.join(", ")));
    }
    if limit >= 0 {
        sql.push_str(&format!(" LIMIT {}", limit));
    }
    if offset >= 0 {
        sql.push_str(&format!(" OFFSET {}", offset));
    }
    sql
}

/// Build an INSERT SQL string from pure Rust types, one placeholder per
/// column. Public within crate for use by repo.rs write operations.
pub(crate) fn build_insert_sql(table: &str, columns: &[String], returning: &[String]) -> String {
    let placeholders: Vec<String> = (1..=columns.len()).map(|i| format!("${}", i)).collect();
    format!(
        "INSERT INTO {} ({}) VALUES ({}){}",
        quote_name(table),
        quoted_list(columns),
        placeholders.join(", "),
        returning_sql(returning)
    )
}

/// Build an UPDATE SQL string from pure Rust types; the WHERE placeholders
/// follow the SET ones.
fn build_update_sql(
    table: &str,
    set_columns: &[String],
    wheres: &[String],
    returning: &[String],
) -> String {
    let set_parts: Vec<String> = set_columns
        .iter()
        .enumerate()
        .map(|(i, c)| format!("{} = ${}", quote_name(c), i + 1))
        .collect();
    format!(
        "UPDATE {} SET {}{}{}",
        quote_name(table),
        set_parts.join(", "),
        where_sql(wheres, set_columns.len() + 1),
        returning_sql(returning)
    )
}

/// Build a DELETE SQL string from pure Rust types.
fn build_delete_sql(table: &str, wheres: &[String], returning: &[String]) -> String {
    format!(
        "DELETE FROM {}{}{}",
        quote_name(table),
        where_sql(wheres, 1),
        returning_sql(returning)
    )
}

/// Build an INSERT ... ON CONFLICT ... DO UPDATE SET SQL string from pure Rust types.
/// Public within crate for use by repo.rs upsert operations.
///
/// Generated SQL pattern:
/// ```sql
/// INSERT INTO "table" ("col1", "col2") VALUES ($1, $2)
/// ON CONFLICT ("unique_col") DO UPDATE SET "col1" = EXCLUDED."col1", "col2" = EXCLUDED."col2"
/// RETURNING *
/// ```
pub(crate) fn build_upsert_sql(
    table: &str,
    columns: &[String],
    conflict_targets: &[String],
    update_columns: &[String],
    returning: &[String],
) -> String {
    let set_parts: Vec<String> = update_columns
        .iter()
        .map(|c| format!("{} = EXCLUDED.{}", quote_name(c), quote_name(c)))
        .collect();
    format!(
        "{} ON CONFLICT ({}) DO UPDATE SET {}{}",
        build_insert_sql(table, columns, &[]),
        quoted_list(conflict_targets),
        set_parts.join(", "),
        returning_sql(returning)
    )
}

// ── Extern C functions ───────────────────────────────────────────────

/// Build a parameterized SELECT query.
///
/// # Signature
///
/// `mesh_orm_build_select(table: ptr, columns: ptr, where_clauses: ptr,
///     order_by: ptr, limit: i64, offset: i64) -> ptr (MeshString)`
///
/// - `table`: table name string
/// - `columns`: List<String> of column names (empty = SELECT *)
/// - `where_clauses`: List<String> where each entry is "column op" (e.g. "name =", "age >")
/// - `order_by`: List<String> where each entry is "column direction" (e.g. "name ASC")
/// - `limit`: -1 means no limit, otherwise LIMIT N
/// - `offset`: -1 means no offset, otherwise OFFSET N
#[no_mangle]
pub extern "C" fn mesh_orm_build_select(
    table: *const MeshString,
    columns: *mut u8,
    where_clauses: *mut u8,
    order_by: *mut u8,
    limit: i64,
    offset: i64,
) -> *mut u8 {
    unsafe {
        let table_name = (*table).as_str();
        let cols = list_strings(columns);
        let wheres = list_strings(where_clauses);
        let orders = list_strings(order_by);
        let sql = build_select_sql(table_name, &cols, &wheres, &orders, limit, offset);
        mesh_str(&sql) as *mut u8
    }
}

/// Build a parameterized INSERT query.
///
/// # Signature
///
/// `mesh_orm_build_insert(table: ptr, columns: ptr, returning: ptr) -> ptr (MeshString)`
///
/// - `columns`: List<String> of column names for the VALUES clause
/// - `returning`: List<String> for RETURNING clause (empty = no RETURNING)
#[no_mangle]
pub extern "C" fn mesh_orm_build_insert(
    table: *const MeshString,
    columns: *mut u8,
    returning: *mut u8,
) -> *mut u8 {
    unsafe {
        let table_name = (*table).as_str();
        let cols = list_strings(columns);
        let ret = list_strings(returning);
        let sql = build_insert_sql(table_name, &cols, &ret);
        mesh_str(&sql) as *mut u8
    }
}

/// Build a parameterized UPDATE query.
///
/// # Signature
///
/// `mesh_orm_build_update(table: ptr, set_columns: ptr, where_clauses: ptr,
///     returning: ptr) -> ptr (MeshString)`
///
/// - `set_columns`: List<String> of column names for SET clause ($N from 1)
/// - `where_clauses`: List<String> of "column op" entries (params continue after SET)
/// - `returning`: List<String> for RETURNING clause
#[no_mangle]
pub extern "C" fn mesh_orm_build_update(
    table: *const MeshString,
    set_columns: *mut u8,
    where_clauses: *mut u8,
    returning: *mut u8,
) -> *mut u8 {
    unsafe {
        let table_name = (*table).as_str();
        let set_cols = list_strings(set_columns);
        let wheres = list_strings(where_clauses);
        let ret = list_strings(returning);
        let sql = build_update_sql(table_name, &set_cols, &wheres, &ret);
        mesh_str(&sql) as *mut u8
    }
}

/// Build a parameterized DELETE query.
///
/// # Signature
///
/// `mesh_orm_build_delete(table: ptr, where_clauses: ptr, returning: ptr) -> ptr (MeshString)`
///
/// - `where_clauses`: List<String> of "column op" entries
/// - `returning`: List<String> for RETURNING clause
#[no_mangle]
pub extern "C" fn mesh_orm_build_delete(
    table: *const MeshString,
    where_clauses: *mut u8,
    returning: *mut u8,
) -> *mut u8 {
    unsafe {
        let table_name = (*table).as_str();
        let wheres = list_strings(where_clauses);
        let ret = list_strings(returning);
        let sql = build_delete_sql(table_name, &wheres, &ret);
        mesh_str(&sql) as *mut u8
    }
}

// ── Unit tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── quote_ident tests ────────────────────────────────────────────

    #[test]
    fn test_quote_ident_simple() {
        assert_eq!(quote_name("users"), "\"users\"");
    }

    #[test]
    fn test_quote_ident_reserved_word() {
        assert_eq!(quote_name("table"), "\"table\"");
    }

    #[test]
    fn test_quote_ident_escaped_double_quote() {
        assert_eq!(quote_name("my\"col"), "\"my\"\"col\"");
    }

    // ── build_select_sql tests ───────────────────────────────────────

    #[test]
    fn test_select_all() {
        let sql = build_select_sql("users", &[], &[], &[], -1, -1);
        assert_eq!(sql, "SELECT * FROM \"users\"");
    }

    #[test]
    fn test_select_with_columns() {
        let sql = build_select_sql("users", &["id".into(), "name".into()], &[], &[], -1, -1);
        assert_eq!(sql, "SELECT \"id\", \"name\" FROM \"users\"");
    }

    #[test]
    fn test_select_with_where() {
        let sql = build_select_sql(
            "users",
            &[],
            &["name =".into(), "age >".into()],
            &[],
            -1,
            -1,
        );
        assert_eq!(
            sql,
            "SELECT * FROM \"users\" WHERE \"name\" = $1 AND \"age\" > $2"
        );
    }

    #[test]
    fn test_select_with_is_null() {
        let sql = build_select_sql(
            "users",
            &[],
            &["deleted_at IS NULL".into(), "name =".into()],
            &[],
            -1,
            -1,
        );
        assert_eq!(
            sql,
            "SELECT * FROM \"users\" WHERE \"deleted_at\" IS NULL AND \"name\" = $1"
        );
    }

    #[test]
    fn test_select_full() {
        let sql = build_select_sql(
            "users",
            &["id".into(), "name".into()],
            &["name =".into()],
            &["name ASC".into()],
            10,
            20,
        );
        assert_eq!(
            sql,
            "SELECT \"id\", \"name\" FROM \"users\" WHERE \"name\" = $1 ORDER BY \"name\" ASC LIMIT 10 OFFSET 20"
        );
    }

    #[test]
    fn test_select_default_operator() {
        let sql = build_select_sql("users", &[], &["id".into()], &[], -1, -1);
        assert_eq!(sql, "SELECT * FROM \"users\" WHERE \"id\" = $1");
    }

    #[test]
    fn test_select_order_default_direction() {
        let sql = build_select_sql("users", &[], &[], &["name".into()], -1, -1);
        assert_eq!(sql, "SELECT * FROM \"users\" ORDER BY \"name\" ASC");
    }

    #[test]
    fn test_select_multiple_orders() {
        let sql = build_select_sql(
            "users",
            &[],
            &[],
            &["name ASC".into(), "age DESC".into()],
            -1,
            -1,
        );
        assert_eq!(
            sql,
            "SELECT * FROM \"users\" ORDER BY \"name\" ASC, \"age\" DESC"
        );
    }

    // ── build_insert_sql tests ───────────────────────────────────────

    #[test]
    fn test_insert_basic() {
        let sql = build_insert_sql(
            "users",
            &["name".into(), "email".into()],
            &["id".into(), "name".into()],
        );
        assert_eq!(
            sql,
            "INSERT INTO \"users\" (\"name\", \"email\") VALUES ($1, $2) RETURNING \"id\", \"name\""
        );
    }

    #[test]
    fn test_insert_returning_star() {
        let sql = build_insert_sql("users", &["name".into(), "email".into()], &["*".into()]);
        assert_eq!(
            sql,
            "INSERT INTO \"users\" (\"name\", \"email\") VALUES ($1, $2) RETURNING *"
        );
    }

    #[test]
    fn test_insert_no_returning() {
        let sql = build_insert_sql("users", &["name".into()], &[]);
        assert_eq!(sql, "INSERT INTO \"users\" (\"name\") VALUES ($1)");
    }

    // ── build_update_sql tests ───────────────────────────────────────

    #[test]
    fn test_update_basic() {
        let sql = build_update_sql(
            "users",
            &["name".into(), "email".into()],
            &["id =".into()],
            &["id".into()],
        );
        assert_eq!(
            sql,
            "UPDATE \"users\" SET \"name\" = $1, \"email\" = $2 WHERE \"id\" = $3 RETURNING \"id\""
        );
    }

    #[test]
    fn test_update_no_where_no_returning() {
        let sql = build_update_sql("users", &["name".into()], &[], &[]);
        assert_eq!(sql, "UPDATE \"users\" SET \"name\" = $1");
    }

    // ── build_delete_sql tests ───────────────────────────────────────

    #[test]
    fn test_delete_basic() {
        let sql = build_delete_sql("users", &["id =".into()], &[]);
        assert_eq!(sql, "DELETE FROM \"users\" WHERE \"id\" = $1");
    }

    #[test]
    fn test_delete_with_returning() {
        let sql = build_delete_sql("users", &["id =".into()], &["id".into()]);
        assert_eq!(
            sql,
            "DELETE FROM \"users\" WHERE \"id\" = $1 RETURNING \"id\""
        );
    }

    #[test]
    fn test_delete_no_where() {
        let sql = build_delete_sql("users", &[], &[]);
        assert_eq!(sql, "DELETE FROM \"users\"");
    }

    // ── build_upsert_sql tests ──────────────────────────────────────────

    #[test]
    fn test_build_upsert_sql() {
        let sql = build_upsert_sql(
            "issues",
            &[
                "project_id".into(),
                "fingerprint".into(),
                "title".into(),
                "level".into(),
                "event_count".into(),
            ],
            &["project_id".into(), "fingerprint".into()],
            &["title".into(), "level".into(), "event_count".into()],
            &["*".into()],
        );
        assert_eq!(
            sql,
            "INSERT INTO \"issues\" (\"project_id\", \"fingerprint\", \"title\", \"level\", \"event_count\") VALUES ($1, $2, $3, $4, $5) ON CONFLICT (\"project_id\", \"fingerprint\") DO UPDATE SET \"title\" = EXCLUDED.\"title\", \"level\" = EXCLUDED.\"level\", \"event_count\" = EXCLUDED.\"event_count\" RETURNING *"
        );
    }
}
