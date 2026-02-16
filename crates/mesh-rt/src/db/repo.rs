//! Repo (repository) module for the Mesh runtime.
//!
//! Provides stateless database operations that consume Query structs
//! (built by the Query module) and execute them via Pool.query. Each read
//! function reads the Query object's 13 slots to build parameterized SQL,
//! then delegates to `mesh_pool_query` for execution. Write functions use
//! the ORM SQL builders from orm.rs with RETURNING * clauses.
//!
//! ## Read Functions
//!
//! - `mesh_repo_all`: Execute query, return all matching rows
//! - `mesh_repo_one`: Execute query with LIMIT 1, return first row or error
//! - `mesh_repo_get`: Fetch single row by primary key
//! - `mesh_repo_get_by`: Fetch single row by field condition
//! - `mesh_repo_count`: Return integer count of matching rows
//! - `mesh_repo_exists`: Return boolean existence check
//!
//! ## Write Functions
//!
//! - `mesh_repo_insert`: INSERT with RETURNING *, accepts Map<String,String> fields
//! - `mesh_repo_update`: UPDATE with RETURNING *, accepts id + Map<String,String> fields
//! - `mesh_repo_delete`: DELETE with RETURNING *, accepts id
//! - `mesh_repo_transaction`: Wraps callback in checkout/begin/commit-or-rollback/checkin

use crate::collections::list::{mesh_list_get, mesh_list_length, mesh_list_new, mesh_list_append};
use crate::collections::map::mesh_map_get;
use crate::db::pool::{mesh_pool_query, mesh_pool_checkout, mesh_pool_checkin};
use crate::db::pg::{mesh_pg_begin, mesh_pg_commit, mesh_pg_rollback};
use crate::db::changeset::{
    SLOT_CHANGES, SLOT_VALID,
    map_constraint_error, add_constraint_error_to_changeset,
};
use crate::io::{alloc_result, MeshResult};
use crate::string::{mesh_string_new, MeshString};

// ── Helpers ──────────────────────────────────────────────────────────

/// Extract a Rust &str from a raw MeshString pointer.
unsafe fn mesh_str_ref(ptr: *mut u8) -> &'static str {
    let ms = ptr as *const MeshString;
    (*ms).as_str()
}

/// Create a MeshString from a Rust &str and return as *mut u8.
unsafe fn rust_str_to_mesh(s: &str) -> *mut u8 {
    mesh_string_new(s.as_ptr(), s.len() as u64) as *mut u8
}

/// Create an error MeshResult from a Rust string.
fn err_result(msg: &str) -> *mut u8 {
    unsafe {
        let s = rust_str_to_mesh(msg);
        alloc_result(1, s) as *mut u8
    }
}

/// Create an Ok MeshResult wrapping a value pointer.
fn ok_result(value: *mut u8) -> *mut u8 {
    alloc_result(0, value) as *mut u8
}

/// Quote a SQL identifier with double quotes (PostgreSQL convention).
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

/// Build a Mesh List<String> from a Vec of Rust strings.
unsafe fn strings_to_mesh_list(strings: &[String]) -> *mut u8 {
    let mut list = mesh_list_new();
    for s in strings {
        let ms = rust_str_to_mesh(s);
        list = mesh_list_append(list, ms as u64);
    }
    list
}

// ── Query slot access ────────────────────────────────────────────────

// Slot indices (must match query.rs exactly)
const SLOT_SOURCE: usize = 0;
const SLOT_SELECT: usize = 1;
const SLOT_WHERE_CLAUSES: usize = 2;
const SLOT_WHERE_PARAMS: usize = 3;
const SLOT_ORDER: usize = 4;
const SLOT_LIMIT: usize = 5;
const SLOT_OFFSET: usize = 6;
const SLOT_JOIN: usize = 7;
const SLOT_GROUP: usize = 8;
const SLOT_HAVING_CLAUSES: usize = 9;
const SLOT_HAVING_PARAMS: usize = 10;
const SLOT_FRAGMENT_PARTS: usize = 11;
const SLOT_FRAGMENT_PARAMS: usize = 12;

unsafe fn query_get(q: *mut u8, slot: usize) -> *mut u8 {
    *(q.add(slot * 8) as *mut *mut u8)
}

unsafe fn query_get_int(q: *mut u8, slot: usize) -> i64 {
    *(q.add(slot * 8) as *mut i64)
}

// ── Comprehensive SQL Builder ────────────────────────────────────────

/// Read all 13 slots of a Query struct and produce a complete SELECT SQL
/// statement with parameterized placeholders, plus the parameter values list.
///
/// Returns `(sql_string, params_vec)` as pure Rust types.
unsafe fn query_to_select_sql(query: *mut u8) -> (String, Vec<String>) {
    let source_ptr = query_get(query, SLOT_SOURCE);
    let source = mesh_str_ref(source_ptr);
    let select_fields = list_to_strings(query_get(query, SLOT_SELECT));
    let where_clauses = list_to_strings(query_get(query, SLOT_WHERE_CLAUSES));
    let where_params = list_to_strings(query_get(query, SLOT_WHERE_PARAMS));
    let order_fields = list_to_strings(query_get(query, SLOT_ORDER));
    let limit_val = query_get_int(query, SLOT_LIMIT);
    let offset_val = query_get_int(query, SLOT_OFFSET);
    let join_clauses = list_to_strings(query_get(query, SLOT_JOIN));
    let group_fields = list_to_strings(query_get(query, SLOT_GROUP));
    let having_clauses = list_to_strings(query_get(query, SLOT_HAVING_CLAUSES));
    let having_params = list_to_strings(query_get(query, SLOT_HAVING_PARAMS));
    let fragment_parts = list_to_strings(query_get(query, SLOT_FRAGMENT_PARTS));
    let fragment_params = list_to_strings(query_get(query, SLOT_FRAGMENT_PARAMS));

    build_select_sql_from_parts(
        source,
        &select_fields,
        &where_clauses,
        &where_params,
        &order_fields,
        limit_val,
        offset_val,
        &join_clauses,
        &group_fields,
        &having_clauses,
        &having_params,
        &fragment_parts,
        &fragment_params,
    )
}

/// Pure Rust SQL builder from decomposed Query parts.
/// Separated for testability without GC.
fn build_select_sql_from_parts(
    source: &str,
    select_fields: &[String],
    where_clauses: &[String],
    where_params: &[String],
    order_fields: &[String],
    limit_val: i64,
    offset_val: i64,
    join_clauses: &[String],
    group_fields: &[String],
    having_clauses: &[String],
    having_params: &[String],
    fragment_parts: &[String],
    fragment_params: &[String],
) -> (String, Vec<String>) {
    let mut sql = String::new();
    let mut params: Vec<String> = Vec::new();
    let mut param_idx = 1usize;

    // SELECT clause
    sql.push_str("SELECT ");
    if select_fields.is_empty() {
        sql.push('*');
    } else {
        let cols: Vec<String> = select_fields.iter().map(|f| quote_ident(f)).collect();
        sql.push_str(&cols.join(", "));
    }

    // FROM clause
    sql.push_str(&format!(" FROM {}", quote_ident(source)));

    // JOIN clauses (format: "TYPE:table:on_clause")
    for join in join_clauses {
        let parts: Vec<&str> = join.splitn(3, ':').collect();
        if parts.len() == 3 {
            sql.push_str(&format!(
                " {} JOIN {} ON {}",
                parts[0],
                quote_ident(parts[1]),
                parts[2]
            ));
        }
    }

    // WHERE clause
    if !where_clauses.is_empty() {
        sql.push_str(" WHERE ");
        let mut conditions = Vec::new();
        let mut wp_idx = 0;
        for clause in where_clauses {
            if let Some(space_pos) = clause.find(' ') {
                let col = &clause[..space_pos];
                let op = clause[space_pos + 1..].trim();
                if op == "IS NULL" || op == "IS NOT NULL" {
                    // No parameter consumed
                    conditions.push(format!("{} {}", quote_ident(col), op));
                } else if op.starts_with("IN:") {
                    // IN clause: "field IN:N"
                    let count: usize = op[3..].parse().unwrap_or(0);
                    let placeholders: Vec<String> = (0..count)
                        .map(|i| format!("${}", param_idx + i))
                        .collect();
                    conditions.push(format!(
                        "{} IN ({})",
                        quote_ident(col),
                        placeholders.join(", ")
                    ));
                    for _ in 0..count {
                        if wp_idx < where_params.len() {
                            params.push(where_params[wp_idx].clone());
                            wp_idx += 1;
                        }
                        param_idx += 1;
                    }
                    continue; // skip default param handling
                } else {
                    // Regular operator: "field op" -> "field" op $N
                    conditions.push(format!(
                        "{} {} ${}",
                        quote_ident(col),
                        op,
                        param_idx
                    ));
                    if wp_idx < where_params.len() {
                        params.push(where_params[wp_idx].clone());
                        wp_idx += 1;
                    }
                    param_idx += 1;
                }
            } else {
                // Just a column name, default to = operator
                conditions.push(format!("{} = ${}", quote_ident(clause), param_idx));
                if wp_idx < where_params.len() {
                    params.push(where_params[wp_idx].clone());
                    wp_idx += 1;
                }
                param_idx += 1;
            }
        }
        sql.push_str(&conditions.join(" AND "));
    }

    // GROUP BY clause
    if !group_fields.is_empty() {
        let cols: Vec<String> = group_fields.iter().map(|f| quote_ident(f)).collect();
        sql.push_str(&format!(" GROUP BY {}", cols.join(", ")));
    }

    // HAVING clause
    if !having_clauses.is_empty() {
        sql.push_str(" HAVING ");
        let mut having_parts_sql = Vec::new();
        for clause in having_clauses {
            having_parts_sql.push(format!("{} ${}", clause, param_idx));
            param_idx += 1;
        }
        sql.push_str(&having_parts_sql.join(" AND "));
        for p in having_params {
            params.push(p.clone());
        }
    }

    // Fragment injection (raw SQL appended)
    for frag in fragment_parts {
        // Replace ? with $N for each fragment parameter placeholder
        let mut frag_sql = String::new();
        for ch in frag.chars() {
            if ch == '?' {
                frag_sql.push_str(&format!("${}", param_idx));
                param_idx += 1;
            } else {
                frag_sql.push(ch);
            }
        }
        sql.push_str(&format!(" {}", frag_sql));
    }
    for p in fragment_params {
        params.push(p.clone());
    }

    // ORDER BY clause
    if !order_fields.is_empty() {
        sql.push_str(" ORDER BY ");
        let orders: Vec<String> = order_fields
            .iter()
            .map(|o| {
                if let Some(space_pos) = o.rfind(' ') {
                    let col = &o[..space_pos];
                    let dir = &o[space_pos + 1..];
                    format!("{} {}", quote_ident(col), dir)
                } else {
                    format!("{} ASC", quote_ident(o))
                }
            })
            .collect();
        sql.push_str(&orders.join(", "));
    }

    // LIMIT clause
    if limit_val >= 0 {
        sql.push_str(&format!(" LIMIT {}", limit_val));
    }

    // OFFSET clause
    if offset_val >= 0 {
        sql.push_str(&format!(" OFFSET {}", offset_val));
    }

    (sql, params)
}

/// Build SQL for count queries: SELECT COUNT(*) FROM ... WHERE ...
/// (reuses WHERE/JOIN/GROUP/HAVING/FRAGMENT logic but overrides SELECT)
unsafe fn query_to_count_sql(query: *mut u8) -> (String, Vec<String>) {
    let source_ptr = query_get(query, SLOT_SOURCE);
    let source = mesh_str_ref(source_ptr);
    let where_clauses = list_to_strings(query_get(query, SLOT_WHERE_CLAUSES));
    let where_params = list_to_strings(query_get(query, SLOT_WHERE_PARAMS));
    let join_clauses = list_to_strings(query_get(query, SLOT_JOIN));
    let group_fields = list_to_strings(query_get(query, SLOT_GROUP));
    let having_clauses = list_to_strings(query_get(query, SLOT_HAVING_CLAUSES));
    let having_params = list_to_strings(query_get(query, SLOT_HAVING_PARAMS));
    let fragment_parts = list_to_strings(query_get(query, SLOT_FRAGMENT_PARTS));
    let fragment_params = list_to_strings(query_get(query, SLOT_FRAGMENT_PARAMS));

    build_count_sql_from_parts(
        source,
        &where_clauses,
        &where_params,
        &join_clauses,
        &group_fields,
        &having_clauses,
        &having_params,
        &fragment_parts,
        &fragment_params,
    )
}

fn build_count_sql_from_parts(
    source: &str,
    where_clauses: &[String],
    where_params: &[String],
    join_clauses: &[String],
    group_fields: &[String],
    having_clauses: &[String],
    having_params: &[String],
    fragment_parts: &[String],
    fragment_params: &[String],
) -> (String, Vec<String>) {
    let mut sql = String::new();
    let mut params: Vec<String> = Vec::new();
    let mut param_idx = 1usize;

    sql.push_str(&format!("SELECT COUNT(*) FROM {}", quote_ident(source)));

    // JOIN clauses
    for join in join_clauses {
        let parts: Vec<&str> = join.splitn(3, ':').collect();
        if parts.len() == 3 {
            sql.push_str(&format!(
                " {} JOIN {} ON {}",
                parts[0],
                quote_ident(parts[1]),
                parts[2]
            ));
        }
    }

    // WHERE clause
    if !where_clauses.is_empty() {
        sql.push_str(" WHERE ");
        let mut conditions = Vec::new();
        let mut wp_idx = 0;
        for clause in where_clauses {
            if let Some(space_pos) = clause.find(' ') {
                let col = &clause[..space_pos];
                let op = clause[space_pos + 1..].trim();
                if op == "IS NULL" || op == "IS NOT NULL" {
                    conditions.push(format!("{} {}", quote_ident(col), op));
                } else if op.starts_with("IN:") {
                    let count: usize = op[3..].parse().unwrap_or(0);
                    let placeholders: Vec<String> = (0..count)
                        .map(|i| format!("${}", param_idx + i))
                        .collect();
                    conditions.push(format!(
                        "{} IN ({})",
                        quote_ident(col),
                        placeholders.join(", ")
                    ));
                    for _ in 0..count {
                        if wp_idx < where_params.len() {
                            params.push(where_params[wp_idx].clone());
                            wp_idx += 1;
                        }
                        param_idx += 1;
                    }
                    continue;
                } else {
                    conditions.push(format!("{} {} ${}", quote_ident(col), op, param_idx));
                    if wp_idx < where_params.len() {
                        params.push(where_params[wp_idx].clone());
                        wp_idx += 1;
                    }
                    param_idx += 1;
                }
            } else {
                conditions.push(format!("{} = ${}", quote_ident(clause), param_idx));
                if wp_idx < where_params.len() {
                    params.push(where_params[wp_idx].clone());
                    wp_idx += 1;
                }
                param_idx += 1;
            }
        }
        sql.push_str(&conditions.join(" AND "));
    }

    // GROUP BY clause
    if !group_fields.is_empty() {
        let cols: Vec<String> = group_fields.iter().map(|f| quote_ident(f)).collect();
        sql.push_str(&format!(" GROUP BY {}", cols.join(", ")));
    }

    // HAVING clause
    if !having_clauses.is_empty() {
        sql.push_str(" HAVING ");
        let mut having_parts_sql = Vec::new();
        for clause in having_clauses {
            having_parts_sql.push(format!("{} ${}", clause, param_idx));
            param_idx += 1;
        }
        sql.push_str(&having_parts_sql.join(" AND "));
        for p in having_params {
            params.push(p.clone());
        }
    }

    // Fragment injection
    for frag in fragment_parts {
        let mut frag_sql = String::new();
        for ch in frag.chars() {
            if ch == '?' {
                frag_sql.push_str(&format!("${}", param_idx));
                param_idx += 1;
            } else {
                frag_sql.push(ch);
            }
        }
        sql.push_str(&format!(" {}", frag_sql));
    }
    for p in fragment_params {
        params.push(p.clone());
    }

    (sql, params)
}

/// Build SQL for exists queries: SELECT EXISTS(SELECT 1 FROM ... WHERE ... LIMIT 1)
unsafe fn query_to_exists_sql(query: *mut u8) -> (String, Vec<String>) {
    let source_ptr = query_get(query, SLOT_SOURCE);
    let source = mesh_str_ref(source_ptr);
    let where_clauses = list_to_strings(query_get(query, SLOT_WHERE_CLAUSES));
    let where_params = list_to_strings(query_get(query, SLOT_WHERE_PARAMS));
    let join_clauses = list_to_strings(query_get(query, SLOT_JOIN));

    build_exists_sql_from_parts(source, &where_clauses, &where_params, &join_clauses)
}

fn build_exists_sql_from_parts(
    source: &str,
    where_clauses: &[String],
    where_params: &[String],
    join_clauses: &[String],
) -> (String, Vec<String>) {
    let mut inner_sql = String::new();
    let mut params: Vec<String> = Vec::new();
    let mut param_idx = 1usize;

    inner_sql.push_str(&format!("SELECT 1 FROM {}", quote_ident(source)));

    // JOIN clauses
    for join in join_clauses {
        let parts: Vec<&str> = join.splitn(3, ':').collect();
        if parts.len() == 3 {
            inner_sql.push_str(&format!(
                " {} JOIN {} ON {}",
                parts[0],
                quote_ident(parts[1]),
                parts[2]
            ));
        }
    }

    // WHERE clause
    if !where_clauses.is_empty() {
        inner_sql.push_str(" WHERE ");
        let mut conditions = Vec::new();
        let mut wp_idx = 0;
        for clause in where_clauses {
            if let Some(space_pos) = clause.find(' ') {
                let col = &clause[..space_pos];
                let op = clause[space_pos + 1..].trim();
                if op == "IS NULL" || op == "IS NOT NULL" {
                    conditions.push(format!("{} {}", quote_ident(col), op));
                } else if op.starts_with("IN:") {
                    let count: usize = op[3..].parse().unwrap_or(0);
                    let placeholders: Vec<String> = (0..count)
                        .map(|i| format!("${}", param_idx + i))
                        .collect();
                    conditions.push(format!(
                        "{} IN ({})",
                        quote_ident(col),
                        placeholders.join(", ")
                    ));
                    for _ in 0..count {
                        if wp_idx < where_params.len() {
                            params.push(where_params[wp_idx].clone());
                            wp_idx += 1;
                        }
                        param_idx += 1;
                    }
                    continue;
                } else {
                    conditions.push(format!("{} {} ${}", quote_ident(col), op, param_idx));
                    if wp_idx < where_params.len() {
                        params.push(where_params[wp_idx].clone());
                        wp_idx += 1;
                    }
                    param_idx += 1;
                }
            } else {
                conditions.push(format!("{} = ${}", quote_ident(clause), param_idx));
                if wp_idx < where_params.len() {
                    params.push(where_params[wp_idx].clone());
                    wp_idx += 1;
                }
                param_idx += 1;
            }
        }
        inner_sql.push_str(&conditions.join(" AND "));
    }

    inner_sql.push_str(" LIMIT 1");

    let sql = format!("SELECT EXISTS({})", inner_sql);
    (sql, params)
}

// ── Extern C functions ───────────────────────────────────────────────

/// Execute a query and return all matching rows.
///
/// `Repo.all(pool, query)` -> `Result<List<Map<String,String>>, String>`
///
/// Reads the Query struct's slots, builds complete SELECT SQL with all
/// clause types, and executes via Pool.query.
#[no_mangle]
pub extern "C" fn mesh_repo_all(pool: u64, query: *mut u8) -> *mut u8 {
    unsafe {
        let (sql, params) = query_to_select_sql(query);
        let sql_ptr = rust_str_to_mesh(&sql) as *const MeshString;
        let params_ptr = strings_to_mesh_list(&params);
        mesh_pool_query(pool, sql_ptr, params_ptr)
    }
}

/// Execute a query and return the first matching row or error.
///
/// `Repo.one(pool, query)` -> `Result<Map<String,String>, String>`
///
/// Adds LIMIT 1 to the query, executes, and extracts the first row.
/// Returns Err("not found") if no rows match.
#[no_mangle]
pub extern "C" fn mesh_repo_one(pool: u64, query: *mut u8) -> *mut u8 {
    unsafe {
        // Read the query but force limit to 1
        let source_ptr = query_get(query, SLOT_SOURCE);
        let source = mesh_str_ref(source_ptr);
        let select_fields = list_to_strings(query_get(query, SLOT_SELECT));
        let where_clauses = list_to_strings(query_get(query, SLOT_WHERE_CLAUSES));
        let where_params = list_to_strings(query_get(query, SLOT_WHERE_PARAMS));
        let order_fields = list_to_strings(query_get(query, SLOT_ORDER));
        let offset_val = query_get_int(query, SLOT_OFFSET);
        let join_clauses = list_to_strings(query_get(query, SLOT_JOIN));
        let group_fields = list_to_strings(query_get(query, SLOT_GROUP));
        let having_clauses = list_to_strings(query_get(query, SLOT_HAVING_CLAUSES));
        let having_params = list_to_strings(query_get(query, SLOT_HAVING_PARAMS));
        let fragment_parts = list_to_strings(query_get(query, SLOT_FRAGMENT_PARTS));
        let fragment_params = list_to_strings(query_get(query, SLOT_FRAGMENT_PARAMS));

        let (sql, params) = build_select_sql_from_parts(
            source,
            &select_fields,
            &where_clauses,
            &where_params,
            &order_fields,
            1, // force LIMIT 1
            offset_val,
            &join_clauses,
            &group_fields,
            &having_clauses,
            &having_params,
            &fragment_parts,
            &fragment_params,
        );

        let sql_ptr = rust_str_to_mesh(&sql) as *const MeshString;
        let params_ptr = strings_to_mesh_list(&params);
        let result = mesh_pool_query(pool, sql_ptr, params_ptr);

        // Check if query succeeded
        let r = &*(result as *const MeshResult);
        if r.tag != 0 {
            return result; // propagate query error
        }

        // Extract first row from the result list
        let list = r.value;
        let list_len = mesh_list_length(list);
        if list_len == 0 {
            return err_result("not found");
        }

        let first_row = mesh_list_get(list, 0) as *mut u8;
        ok_result(first_row)
    }
}

/// Fetch a single row by primary key.
///
/// `Repo.get(pool, table, id)` -> `Result<Map<String,String>, String>`
///
/// Builds: `SELECT * FROM "table" WHERE "id" = $1 LIMIT 1`
#[no_mangle]
pub extern "C" fn mesh_repo_get(pool: u64, table: *mut u8, id: *mut u8) -> *mut u8 {
    unsafe {
        let table_str = mesh_str_ref(table);
        let sql = format!(
            "SELECT * FROM {} WHERE {} = $1 LIMIT 1",
            quote_ident(table_str),
            quote_ident("id")
        );
        let sql_ptr = rust_str_to_mesh(&sql) as *const MeshString;
        let mut params_list = mesh_list_new();
        params_list = mesh_list_append(params_list, id as u64);
        let result = mesh_pool_query(pool, sql_ptr, params_list);

        // Check if query succeeded
        let r = &*(result as *const MeshResult);
        if r.tag != 0 {
            return result;
        }

        let list = r.value;
        let list_len = mesh_list_length(list);
        if list_len == 0 {
            return err_result("not found");
        }

        let first_row = mesh_list_get(list, 0) as *mut u8;
        ok_result(first_row)
    }
}

/// Fetch a single row by field condition.
///
/// `Repo.get_by(pool, table, field, value)` -> `Result<Map<String,String>, String>`
///
/// Builds: `SELECT * FROM "table" WHERE "field" = $1 LIMIT 1`
#[no_mangle]
pub extern "C" fn mesh_repo_get_by(
    pool: u64,
    table: *mut u8,
    field: *mut u8,
    value: *mut u8,
) -> *mut u8 {
    unsafe {
        let table_str = mesh_str_ref(table);
        let field_str = mesh_str_ref(field);
        let sql = format!(
            "SELECT * FROM {} WHERE {} = $1 LIMIT 1",
            quote_ident(table_str),
            quote_ident(field_str)
        );
        let sql_ptr = rust_str_to_mesh(&sql) as *const MeshString;
        let mut params_list = mesh_list_new();
        params_list = mesh_list_append(params_list, value as u64);
        let result = mesh_pool_query(pool, sql_ptr, params_list);

        let r = &*(result as *const MeshResult);
        if r.tag != 0 {
            return result;
        }

        let list = r.value;
        let list_len = mesh_list_length(list);
        if list_len == 0 {
            return err_result("not found");
        }

        let first_row = mesh_list_get(list, 0) as *mut u8;
        ok_result(first_row)
    }
}

/// Return the count of matching rows.
///
/// `Repo.count(pool, query)` -> `Result<Int, String>`
///
/// Builds: `SELECT COUNT(*) FROM "table" WHERE ...`
/// Parses the integer from the first row's first column.
#[no_mangle]
pub extern "C" fn mesh_repo_count(pool: u64, query: *mut u8) -> *mut u8 {
    unsafe {
        let (sql, params) = query_to_count_sql(query);
        let sql_ptr = rust_str_to_mesh(&sql) as *const MeshString;
        let params_ptr = strings_to_mesh_list(&params);
        let result = mesh_pool_query(pool, sql_ptr, params_ptr);

        let r = &*(result as *const MeshResult);
        if r.tag != 0 {
            return result;
        }

        let list = r.value;
        let list_len = mesh_list_length(list);
        if list_len == 0 {
            return err_result("count returned no rows");
        }

        // Get the first row (a Map<String,String>)
        let first_row = mesh_list_get(list, 0) as *mut u8;
        // Get the "count" column value
        let count_key = rust_str_to_mesh("count");
        let count_val = mesh_map_get(first_row, count_key as u64);
        if count_val == 0 {
            // No "count" key found -- try first value by any key
            return ok_result(0i64 as *mut u8);
        }

        // Parse the string value as an integer
        let count_str = mesh_str_ref(count_val as *mut u8);
        let count: i64 = count_str.parse().unwrap_or(0);
        ok_result(count as *mut u8)
    }
}

/// Check if any rows match the query.
///
/// `Repo.exists(pool, query)` -> `Result<Bool, String>`
///
/// Builds: `SELECT EXISTS(SELECT 1 FROM "table" WHERE ... LIMIT 1)`
/// Returns true (1) or false (0) as the result value.
#[no_mangle]
pub extern "C" fn mesh_repo_exists(pool: u64, query: *mut u8) -> *mut u8 {
    unsafe {
        let (sql, params) = query_to_exists_sql(query);
        let sql_ptr = rust_str_to_mesh(&sql) as *const MeshString;
        let params_ptr = strings_to_mesh_list(&params);
        let result = mesh_pool_query(pool, sql_ptr, params_ptr);

        let r = &*(result as *const MeshResult);
        if r.tag != 0 {
            return result;
        }

        let list = r.value;
        let list_len = mesh_list_length(list);
        if list_len == 0 {
            return ok_result(0i64 as *mut u8); // false
        }

        // Get the first row, extract the "exists" column
        let first_row = mesh_list_get(list, 0) as *mut u8;
        let exists_key = rust_str_to_mesh("exists");
        let exists_val = mesh_map_get(first_row, exists_key as u64);
        if exists_val == 0 {
            return ok_result(0i64 as *mut u8); // false
        }

        let exists_str = mesh_str_ref(exists_val as *mut u8);
        let exists_bool: i64 = if exists_str == "t" || exists_str == "true" || exists_str == "1" {
            1
        } else {
            0
        };
        ok_result(exists_bool as *mut u8)
    }
}

// ── Map extraction helpers ─────────────────────────────────────────

/// Map header size in bytes (must match collections/map.rs)
const MAP_HEADER_SIZE: usize = 16;

/// Extract (column_names, values) from a Mesh Map<String, String> pointer.
/// Reads the map's internal structure directly for efficiency.
unsafe fn map_to_columns_and_values(map: *mut u8) -> (Vec<String>, Vec<String>) {
    let len = *(map as *const u64) as usize;
    let entries = map.add(MAP_HEADER_SIZE) as *const [u64; 2];
    let mut columns = Vec::with_capacity(len);
    let mut values = Vec::with_capacity(len);
    for i in 0..len {
        let key_ptr = (*entries.add(i))[0] as *const MeshString;
        let val_ptr = (*entries.add(i))[1] as *const MeshString;
        if !key_ptr.is_null() {
            columns.push((*key_ptr).as_str().to_string());
        }
        if !val_ptr.is_null() {
            values.push((*val_ptr).as_str().to_string());
        } else {
            values.push(String::new());
        }
    }
    (columns, values)
}

// ── Write Operations ──────────────────────────────────────────────────

/// Insert a new row and return the inserted record.
///
/// `Repo.insert(pool, table, fields_map)` -> `Result<Map<String,String>, String>`
///
/// 1. Extracts column names and values from the Map<String, String>
/// 2. Builds INSERT SQL with RETURNING * using ORM SQL builder
/// 3. Executes via Pool.query (RETURNING produces rows)
/// 4. Returns the first (inserted) row
#[no_mangle]
pub extern "C" fn mesh_repo_insert(pool: u64, table: *mut u8, fields: *mut u8) -> *mut u8 {
    unsafe {
        let table_str = mesh_str_ref(table);
        let (columns, values) = map_to_columns_and_values(fields);

        if columns.is_empty() {
            return err_result("insert: no fields provided");
        }

        // Build INSERT SQL with RETURNING *
        let returning = vec!["*".to_string()];
        let sql = crate::db::orm::build_insert_sql_pure(table_str, &columns, &returning);

        let sql_ptr = rust_str_to_mesh(&sql) as *const MeshString;
        let params_ptr = strings_to_mesh_list(&values);
        let result = mesh_pool_query(pool, sql_ptr, params_ptr);

        // Check if query succeeded
        let r = &*(result as *const MeshResult);
        if r.tag != 0 {
            return result; // propagate query error
        }

        // Extract first row from the result list (the inserted row)
        let list = r.value;
        let list_len = mesh_list_length(list);
        if list_len == 0 {
            return err_result("insert: no row returned");
        }

        let first_row = mesh_list_get(list, 0) as *mut u8;
        ok_result(first_row)
    }
}

/// Update a row by primary key and return the updated record.
///
/// `Repo.update(pool, table, id, fields_map)` -> `Result<Map<String,String>, String>`
///
/// 1. Extracts column names and values from the Map<String, String>
/// 2. Builds UPDATE SQL with SET columns and WHERE id = $N, RETURNING *
/// 3. Params: SET values first ($1..$N), then id ($N+1)
/// 4. Returns the first (updated) row
#[no_mangle]
pub extern "C" fn mesh_repo_update(
    pool: u64,
    table: *mut u8,
    id: *mut u8,
    fields: *mut u8,
) -> *mut u8 {
    unsafe {
        let table_str = mesh_str_ref(table);
        let (columns, mut values) = map_to_columns_and_values(fields);

        if columns.is_empty() {
            return err_result("update: no fields provided");
        }

        // Build UPDATE SQL: SET columns, WHERE id =, RETURNING *
        let wheres = vec!["id =".to_string()];
        let returning = vec!["*".to_string()];
        let sql = crate::db::orm::build_update_sql_pure(table_str, &columns, &wheres, &returning);

        // Params: SET values first, then id value for WHERE
        let id_str = mesh_str_ref(id);
        values.push(id_str.to_string());

        let sql_ptr = rust_str_to_mesh(&sql) as *const MeshString;
        let params_ptr = strings_to_mesh_list(&values);
        let result = mesh_pool_query(pool, sql_ptr, params_ptr);

        let r = &*(result as *const MeshResult);
        if r.tag != 0 {
            return result;
        }

        let list = r.value;
        let list_len = mesh_list_length(list);
        if list_len == 0 {
            return err_result("update: no row returned (id not found)");
        }

        let first_row = mesh_list_get(list, 0) as *mut u8;
        ok_result(first_row)
    }
}

/// Delete a row by primary key and return the deleted record.
///
/// `Repo.delete(pool, table, id)` -> `Result<Map<String,String>, String>`
///
/// 1. Builds DELETE SQL with WHERE id = $1, RETURNING *
/// 2. Returns the first (deleted) row
#[no_mangle]
pub extern "C" fn mesh_repo_delete(pool: u64, table: *mut u8, id: *mut u8) -> *mut u8 {
    unsafe {
        let table_str = mesh_str_ref(table);

        // Build DELETE SQL: WHERE id =, RETURNING *
        let wheres = vec!["id =".to_string()];
        let returning = vec!["*".to_string()];
        let sql = crate::db::orm::build_delete_sql_pure(table_str, &wheres, &returning);

        let sql_ptr = rust_str_to_mesh(&sql) as *const MeshString;
        let mut params_list = mesh_list_new();
        params_list = mesh_list_append(params_list, id as u64);
        let result = mesh_pool_query(pool, sql_ptr, params_list);

        let r = &*(result as *const MeshResult);
        if r.tag != 0 {
            return result;
        }

        let list = r.value;
        let list_len = mesh_list_length(list);
        if list_len == 0 {
            return err_result("delete: no row returned (id not found)");
        }

        let first_row = mesh_list_get(list, 0) as *mut u8;
        ok_result(first_row)
    }
}

/// Execute a callback inside a database transaction with automatic
/// checkout/begin/commit-or-rollback/checkin lifecycle.
///
/// `Repo.transaction(pool, callback)` -> `Result<Ptr, String>`
///
/// Protocol:
/// 1. Pool.checkout(pool) to get a connection handle
/// 2. Pg.begin(conn) to start transaction
/// 3. Call user callback with conn handle (via closure calling convention)
/// 4. On Ok: Pg.commit(conn), Pool.checkin(pool, conn), return Ok(value)
/// 5. On Err: Pg.rollback(conn), Pool.checkin(pool, conn), return Err(error)
/// 6. On panic: Pg.rollback(conn), Pool.checkin(pool, conn), return Err("transaction panicked")
#[no_mangle]
pub extern "C" fn mesh_repo_transaction(
    pool: u64,
    fn_ptr: *const u8,
    env_ptr: *const u8,
) -> *mut u8 {
    unsafe {
        // 1. Checkout a connection from the pool
        let checkout_result = mesh_pool_checkout(pool);
        let r = &*(checkout_result as *const MeshResult);
        if r.tag != 0 {
            return checkout_result; // propagate checkout error
        }
        let conn_handle = r.value as u64;

        // 2. BEGIN transaction
        let begin_result = mesh_pg_begin(conn_handle);
        let br = &*(begin_result as *const MeshResult);
        if br.tag != 0 {
            // BEGIN failed -- checkin and return error
            mesh_pool_checkin(pool, conn_handle);
            return begin_result;
        }

        // 3. Call the user callback with catch_unwind for panic safety
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if env_ptr.is_null() {
                let f: extern "C" fn(u64) -> *mut u8 = std::mem::transmute(fn_ptr);
                f(conn_handle)
            } else {
                let f: extern "C" fn(*const u8, u64) -> *mut u8 = std::mem::transmute(fn_ptr);
                f(env_ptr, conn_handle)
            }
        }));

        match result {
            Ok(result_ptr) => {
                // Check if callback returned Ok or Err
                let cr = &*(result_ptr as *const MeshResult);
                if cr.tag == 0 {
                    // Success -> COMMIT
                    let commit_result = mesh_pg_commit(conn_handle);
                    let cmr = &*(commit_result as *const MeshResult);
                    if cmr.tag != 0 {
                        // COMMIT failed -> ROLLBACK
                        let _ = mesh_pg_rollback(conn_handle);
                        mesh_pool_checkin(pool, conn_handle);
                        return err_result("transaction: COMMIT failed");
                    }
                    mesh_pool_checkin(pool, conn_handle);
                    result_ptr // return Ok(value) from callback
                } else {
                    // Error -> ROLLBACK
                    let _ = mesh_pg_rollback(conn_handle);
                    mesh_pool_checkin(pool, conn_handle);
                    result_ptr // propagate the Err result from callback
                }
            }
            Err(_) => {
                // Panic -> ROLLBACK
                let _ = mesh_pg_rollback(conn_handle);
                mesh_pool_checkin(pool, conn_handle);
                err_result("transaction aborted: panic in callback")
            }
        }
    }
}

// ── Changeset slot access ────────────────────────────────────────────

/// Read a pointer slot from a changeset.
unsafe fn cs_get(cs: *mut u8, slot: usize) -> *mut u8 {
    *(cs.add(slot * 8) as *const *mut u8)
}

/// Read an integer slot from a changeset.
unsafe fn cs_get_int(cs: *mut u8, slot: usize) -> i64 {
    *(cs.add(slot * 8) as *const i64)
}

// ── PG error string parsing ─────────────────────────────────────────

/// Parse the structured error string from pg.rs (tab-separated format).
///
/// Format: `{sqlstate}\t{constraint}\t{table}\t{column}\t{message}`
/// Returns: (sqlstate, constraint, table, column, message)
fn parse_pg_error_string(err: &str) -> (&str, &str, &str, &str, &str) {
    let parts: Vec<&str> = err.splitn(5, '\t').collect();
    if parts.len() == 5 {
        (parts[0], parts[1], parts[2], parts[3], parts[4])
    } else {
        // Fallback for non-PG errors or unstructured error strings
        ("", "", "", "", err)
    }
}

// ── Changeset Write Operations ──────────────────────────────────────

/// Insert a row using a changeset, validating before SQL execution.
///
/// `Repo.insert_changeset(pool, table, changeset)` -> `Result<Map<String,String>, Changeset>`
///
/// 1. If changeset is invalid (valid == 0): return Err(changeset) without executing SQL
/// 2. Extract changes map, build INSERT SQL with RETURNING *
/// 3. Execute via Pool.query
/// 4. On success: return Ok(first_row)
/// 5. On PG error: parse structured error, map constraint to changeset error, return Err(changeset)
#[no_mangle]
pub extern "C" fn mesh_repo_insert_changeset(
    pool: u64,
    table: *mut u8,
    changeset: *mut u8,
) -> *mut u8 {
    unsafe {
        // 1. Check if changeset is valid
        if cs_get_int(changeset, SLOT_VALID) == 0 {
            return alloc_result(1, changeset) as *mut u8;
        }

        // 2. Extract changes map
        let changes = cs_get(changeset, SLOT_CHANGES);
        let (columns, values) = map_to_columns_and_values(changes);

        if columns.is_empty() {
            return alloc_result(1, changeset) as *mut u8;
        }

        // 3. Build INSERT SQL with RETURNING *
        let table_str = mesh_str_ref(table);
        let returning = vec!["*".to_string()];
        let sql = crate::db::orm::build_insert_sql_pure(table_str, &columns, &returning);

        let sql_ptr = rust_str_to_mesh(&sql) as *const MeshString;
        let params_ptr = strings_to_mesh_list(&values);
        let result = mesh_pool_query(pool, sql_ptr, params_ptr);

        // 4. Check result
        let r = &*(result as *const MeshResult);
        if r.tag == 0 {
            // Success: extract first row
            let list = r.value;
            let list_len = mesh_list_length(list);
            if list_len == 0 {
                return err_result("insert_changeset: no row returned");
            }
            let first_row = mesh_list_get(list, 0) as *mut u8;
            ok_result(first_row)
        } else {
            // Error: try to map constraint violation to changeset error
            let err_str = mesh_str_ref(r.value);
            let (sqlstate, constraint, pg_table, column, _message) = parse_pg_error_string(err_str);

            if let Some((field, msg)) = map_constraint_error(sqlstate, constraint, pg_table, column) {
                let cs_with_err = add_constraint_error_to_changeset(changeset, &field, &msg);
                alloc_result(1, cs_with_err) as *mut u8
            } else {
                // Unknown error: add as generic _base error
                let cs_with_err = add_constraint_error_to_changeset(changeset, "_base", "database error");
                alloc_result(1, cs_with_err) as *mut u8
            }
        }
    }
}

/// Update a row using a changeset, validating before SQL execution.
///
/// `Repo.update_changeset(pool, table, id, changeset)` -> `Result<Map<String,String>, Changeset>`
///
/// Same pattern as insert_changeset but builds UPDATE SQL with WHERE id = $N+1.
#[no_mangle]
pub extern "C" fn mesh_repo_update_changeset(
    pool: u64,
    table: *mut u8,
    id: *mut u8,
    changeset: *mut u8,
) -> *mut u8 {
    unsafe {
        // 1. Check if changeset is valid
        if cs_get_int(changeset, SLOT_VALID) == 0 {
            return alloc_result(1, changeset) as *mut u8;
        }

        // 2. Extract changes map
        let changes = cs_get(changeset, SLOT_CHANGES);
        let (columns, mut values) = map_to_columns_and_values(changes);

        if columns.is_empty() {
            return alloc_result(1, changeset) as *mut u8;
        }

        // 3. Build UPDATE SQL with RETURNING *
        let table_str = mesh_str_ref(table);
        let wheres = vec!["id =".to_string()];
        let returning = vec!["*".to_string()];
        let sql = crate::db::orm::build_update_sql_pure(table_str, &columns, &wheres, &returning);

        // Params: SET values first, then id value for WHERE
        let id_str = mesh_str_ref(id);
        values.push(id_str.to_string());

        let sql_ptr = rust_str_to_mesh(&sql) as *const MeshString;
        let params_ptr = strings_to_mesh_list(&values);
        let result = mesh_pool_query(pool, sql_ptr, params_ptr);

        // 4. Check result
        let r = &*(result as *const MeshResult);
        if r.tag == 0 {
            // Success: extract first row
            let list = r.value;
            let list_len = mesh_list_length(list);
            if list_len == 0 {
                return err_result("update_changeset: no row returned (id not found)");
            }
            let first_row = mesh_list_get(list, 0) as *mut u8;
            ok_result(first_row)
        } else {
            // Error: try to map constraint violation to changeset error
            let err_str = mesh_str_ref(r.value);
            let (sqlstate, constraint, pg_table, column, _message) = parse_pg_error_string(err_str);

            if let Some((field, msg)) = map_constraint_error(sqlstate, constraint, pg_table, column) {
                let cs_with_err = add_constraint_error_to_changeset(changeset, &field, &msg);
                alloc_result(1, cs_with_err) as *mut u8
            } else {
                // Unknown error: add as generic _base error
                let cs_with_err = add_constraint_error_to_changeset(changeset, "_base", "database error");
                alloc_result(1, cs_with_err) as *mut u8
            }
        }
    }
}

// ── Unit tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_select_all_from_table() {
        let (sql, params) = build_select_sql_from_parts(
            "users", &[], &[], &[], &[], -1, -1, &[], &[], &[], &[], &[], &[],
        );
        assert_eq!(sql, "SELECT * FROM \"users\"");
        assert!(params.is_empty());
    }

    #[test]
    fn test_select_with_columns() {
        let (sql, _) = build_select_sql_from_parts(
            "users",
            &["id".into(), "name".into()],
            &[], &[], &[], -1, -1, &[], &[], &[], &[], &[], &[],
        );
        assert_eq!(sql, "SELECT \"id\", \"name\" FROM \"users\"");
    }

    #[test]
    fn test_select_with_where() {
        let (sql, params) = build_select_sql_from_parts(
            "users", &[],
            &["name =".into(), "age >".into()],
            &["Alice".into(), "21".into()],
            &[], -1, -1, &[], &[], &[], &[], &[], &[],
        );
        assert_eq!(
            sql,
            "SELECT * FROM \"users\" WHERE \"name\" = $1 AND \"age\" > $2"
        );
        assert_eq!(params, vec!["Alice", "21"]);
    }

    #[test]
    fn test_select_with_is_null() {
        let (sql, params) = build_select_sql_from_parts(
            "users", &[],
            &["deleted_at IS NULL".into(), "name =".into()],
            &["Alice".into()],
            &[], -1, -1, &[], &[], &[], &[], &[], &[],
        );
        assert_eq!(
            sql,
            "SELECT * FROM \"users\" WHERE \"deleted_at\" IS NULL AND \"name\" = $1"
        );
        assert_eq!(params, vec!["Alice"]);
    }

    #[test]
    fn test_select_with_in_clause() {
        let (sql, params) = build_select_sql_from_parts(
            "users", &[],
            &["status IN:3".into()],
            &["active".into(), "pending".into(), "trial".into()],
            &[], -1, -1, &[], &[], &[], &[], &[], &[],
        );
        assert_eq!(
            sql,
            "SELECT * FROM \"users\" WHERE \"status\" IN ($1, $2, $3)"
        );
        assert_eq!(params, vec!["active", "pending", "trial"]);
    }

    #[test]
    fn test_select_with_join() {
        let (sql, _) = build_select_sql_from_parts(
            "users", &[], &[], &[], &[], -1, -1,
            &["INNER:posts:posts.user_id = users.id".into()],
            &[], &[], &[], &[], &[],
        );
        assert_eq!(
            sql,
            "SELECT * FROM \"users\" INNER JOIN \"posts\" ON posts.user_id = users.id"
        );
    }

    #[test]
    fn test_select_with_group_by_having() {
        let (sql, params) = build_select_sql_from_parts(
            "orders", &[], &[], &[], &[], -1, -1, &[],
            &["category".into()],
            &["count(*) >".into()],
            &["5".into()],
            &[], &[],
        );
        assert_eq!(
            sql,
            "SELECT * FROM \"orders\" GROUP BY \"category\" HAVING count(*) > $1"
        );
        assert_eq!(params, vec!["5"]);
    }

    #[test]
    fn test_select_with_order_limit_offset() {
        let (sql, _) = build_select_sql_from_parts(
            "users", &[], &[], &[],
            &["name ASC".into(), "age DESC".into()],
            10, 20, &[], &[], &[], &[], &[], &[],
        );
        assert_eq!(
            sql,
            "SELECT * FROM \"users\" ORDER BY \"name\" ASC, \"age\" DESC LIMIT 10 OFFSET 20"
        );
    }

    #[test]
    fn test_select_full_query() {
        let (sql, params) = build_select_sql_from_parts(
            "users",
            &["id".into(), "name".into()],
            &["active =".into()],
            &["true".into()],
            &["name ASC".into()],
            10, 0,
            &["INNER:posts:posts.user_id = users.id".into()],
            &[], &[], &[], &[], &[],
        );
        assert_eq!(
            sql,
            "SELECT \"id\", \"name\" FROM \"users\" INNER JOIN \"posts\" ON posts.user_id = users.id WHERE \"active\" = $1 ORDER BY \"name\" ASC LIMIT 10 OFFSET 0"
        );
        assert_eq!(params, vec!["true"]);
    }

    #[test]
    fn test_select_with_fragment() {
        let (sql, params) = build_select_sql_from_parts(
            "users", &[], &[], &[], &[], -1, -1, &[], &[], &[], &[],
            &["AND custom_fn(?)".into()],
            &["test_val".into()],
        );
        assert_eq!(
            sql,
            "SELECT * FROM \"users\" AND custom_fn($1)"
        );
        assert_eq!(params, vec!["test_val"]);
    }

    #[test]
    fn test_count_sql() {
        let (sql, params) = build_count_sql_from_parts(
            "users",
            &["active =".into()],
            &["true".into()],
            &[], &[], &[], &[], &[], &[],
        );
        assert_eq!(
            sql,
            "SELECT COUNT(*) FROM \"users\" WHERE \"active\" = $1"
        );
        assert_eq!(params, vec!["true"]);
    }

    #[test]
    fn test_exists_sql() {
        let (sql, params) = build_exists_sql_from_parts(
            "users",
            &["name =".into()],
            &["Alice".into()],
            &[],
        );
        assert_eq!(
            sql,
            "SELECT EXISTS(SELECT 1 FROM \"users\" WHERE \"name\" = $1 LIMIT 1)"
        );
        assert_eq!(params, vec!["Alice"]);
    }

    // ── PG error string parsing tests ─────────────────────────────────

    #[test]
    fn test_parse_pg_error_string_structured() {
        let err = "23505\tusers_email_key\tusers\t\tduplicate key value violates unique constraint";
        let (sqlstate, constraint, table, column, message) = parse_pg_error_string(err);
        assert_eq!(sqlstate, "23505");
        assert_eq!(constraint, "users_email_key");
        assert_eq!(table, "users");
        assert_eq!(column, "");
        assert_eq!(message, "duplicate key value violates unique constraint");
    }

    #[test]
    fn test_parse_pg_error_string_unstructured() {
        let err = "some random error";
        let (sqlstate, constraint, table, column, message) = parse_pg_error_string(err);
        assert_eq!(sqlstate, "");
        assert_eq!(constraint, "");
        assert_eq!(table, "");
        assert_eq!(column, "");
        assert_eq!(message, "some random error");
    }

    // ── Constraint mapping tests ──────────────────────────────────────

    #[test]
    fn test_map_constraint_unique_violation() {
        let result = map_constraint_error("23505", "users_email_key", "users", "");
        assert_eq!(result, Some(("email".to_string(), "has already been taken".to_string())));
    }

    #[test]
    fn test_map_constraint_foreign_key_violation() {
        let result = map_constraint_error("23503", "posts_user_id_fkey", "posts", "");
        assert_eq!(result, Some(("user_id".to_string(), "does not exist".to_string())));
    }

    #[test]
    fn test_map_constraint_not_null_violation() {
        let result = map_constraint_error("23502", "", "", "name");
        assert_eq!(result, Some(("name".to_string(), "can't be blank".to_string())));
    }

    #[test]
    fn test_map_constraint_unknown_sqlstate() {
        let result = map_constraint_error("42601", "", "", "");
        assert_eq!(result, None);
    }
}
