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

use super::{quote_ident, quote_name};
use crate::collections::list::{
    mesh_list_append, mesh_list_from_array, mesh_list_get, mesh_list_length, mesh_list_new,
};
use crate::collections::map::{
    mesh_map_entry_key, mesh_map_entry_value, mesh_map_get, mesh_map_has_key, mesh_map_put,
    mesh_map_size,
};
use crate::db::changeset::{
    add_error, map_constraint_error, mesh_changeset_changes, mesh_changeset_valid,
};
use crate::db::expr::{clone_expr, parse_expr, render_expr, SqlExpr};
use crate::db::pool::{
    mesh_pool_checkin, mesh_pool_checkout, mesh_pool_execute, mesh_pool_query, unbox_u64_payload,
};
use crate::db::query::{query_parts, QueryParts};
use crate::io::{alloc_result, err_result, MeshResult};
use crate::string::{mesh_str, MeshString};

// ── Helpers ──────────────────────────────────────────────────────────

/// Create an Ok MeshResult wrapping a value pointer.
fn ok_result(value: *mut u8) -> *mut u8 {
    alloc_result(0, value) as *mut u8
}

/// `Ok(first row)` of a query's rows, `Err(none)` when it returned none;
/// a failed query as it is.
unsafe fn first_row(result: *mut u8, none: &str) -> *mut u8 {
    let r = &*(result as *const MeshResult);
    if r.tag != 0 {
        return result;
    }
    if mesh_list_length(r.value) == 0 {
        return err_result(none);
    }
    ok_result(mesh_list_get(r.value, 0) as *mut u8)
}

// ── Placeholder renumbering helper ───────────────────────────────────

/// Renumber $N placeholders in a SQL fragment.
/// E.g., with start_idx=4: "$1" -> "$4", "$2" -> "$5"
/// Also replaces `?` with next sequential $N. Quoted text and identifiers
/// (`'...'`, `"..."`) pass through whole: a `?` or `$1` in them is text.
/// Returns (renumbered_sql, number_of_params_consumed).
pub(crate) fn renumber_placeholders(sql: &str, start_idx: usize) -> (String, usize) {
    let mut result = String::with_capacity(sql.len());
    let mut max_placeholder = 0usize;
    let mut question_count = 0usize;
    let mut chars = sql.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' | '"' => {
                result.push(c);
                for quoted in chars.by_ref() {
                    result.push(quoted);
                    if quoted == c {
                        break;
                    }
                }
            }
            '?' => {
                result.push_str(&format!("${}", start_idx + question_count));
                question_count += 1;
            }
            '$' if chars.peek().is_some_and(char::is_ascii_digit) => {
                let mut digits = String::new();
                while let Some(digit) = chars.next_if(char::is_ascii_digit) {
                    digits.push(digit);
                }
                match digits.parse::<usize>() {
                    // $1 -> $start_idx, $2 -> $start_idx+1
                    Ok(n) if n > 0 => {
                        max_placeholder = max_placeholder.max(n);
                        result.push_str(&format!("${}", start_idx + n - 1));
                    }
                    // `$0` is no parameter, nor a number too long for one.
                    _ => {
                        result.push('$');
                        result.push_str(&digits);
                    }
                }
            }
            _ => result.push(c),
        }
    }
    let params_consumed = if question_count > 0 {
        question_count
    } else {
        max_placeholder
    };
    (result, params_consumed)
}

// ── Comprehensive SQL Builder ────────────────────────────────────────

/// The query's SELECT, as `Repo.all` runs it, with its parameter values,
/// numbered from `$start_idx`; `bare_select` stands in for an empty select
/// list (`*`).
fn select_sql(
    query: &QueryParts,
    bare_select: Option<&str>,
    start_idx: usize,
) -> (String, Vec<String>) {
    let mut sql = String::new();
    let mut params: Vec<String> = Vec::new();
    let mut param_idx = start_idx;

    // SELECT clause
    sql.push_str("SELECT ");
    let bare = bare_select.map(|select| format!("RAW:{select}"));
    let select_fields = match (&query.select[..], &bare) {
        ([], Some(bare)) => std::slice::from_ref(bare),
        (fields, _) => fields,
    };
    if select_fields.is_empty() {
        sql.push('*');
    } else {
        let mut cols = Vec::with_capacity(select_fields.len());
        for field in select_fields {
            if let Some(raw) = field.strip_prefix("RAW:") {
                cols.push(raw.to_string());
            } else if let Some(expr) = field.strip_prefix("EXPR:") {
                cols.push(render_expr(&parse_expr(expr), &mut params, &mut param_idx));
            } else {
                cols.push(quote_name(field));
            }
        }
        sql.push_str(&cols.join(", "));
    }

    // FROM clause
    sql.push_str(&format!(" FROM {}", quote_name(&query.source)));

    // JOIN clauses (format: "TYPE:table:on_clause" or "ALIAS:TYPE:table:alias:on_clause")
    for join in &query.joins {
        if let Some(rest) = join.strip_prefix("ALIAS:") {
            let parts: Vec<&str> = rest.splitn(4, ':').collect();
            if parts.len() == 4 {
                sql.push_str(&format!(
                    " {} JOIN {} {} ON {}",
                    parts[0],             // join type (INNER, LEFT)
                    quote_name(parts[1]), // table name
                    parts[2],             // alias (unquoted)
                    parts[3]              // on clause
                ));
            }
        } else {
            let parts: Vec<&str> = join.splitn(3, ':').collect();
            if parts.len() == 3 {
                sql.push_str(&format!(
                    " {} JOIN {} ON {}",
                    parts[0],
                    quote_name(parts[1]),
                    parts[2]
                ));
            }
        }
    }

    // WHERE clause
    if !query.where_clauses.is_empty() {
        let (where_sql, where_param_values, next_param_idx) = where_sql(query, param_idx);
        sql.push_str(&format!(" WHERE {}", where_sql));
        params.extend(where_param_values);
        param_idx = next_param_idx;
    }

    // GROUP BY clause
    if !query.group.is_empty() {
        let cols: Vec<String> = query
            .group
            .iter()
            .map(|f| {
                if let Some(raw) = f.strip_prefix("RAW:") {
                    raw.to_string() // emit verbatim
                } else {
                    quote_name(f)
                }
            })
            .collect();
        sql.push_str(&format!(" GROUP BY {}", cols.join(", ")));
    }

    // HAVING clause
    if !query.having.is_empty() {
        sql.push_str(" HAVING ");
        let mut having_parts_sql = Vec::new();
        for clause in &query.having {
            having_parts_sql.push(format!("{} ${}", clause, param_idx));
            param_idx += 1;
        }
        sql.push_str(&having_parts_sql.join(" AND "));
        params.extend(query.having_params.iter().cloned());
    }

    // Fragment injection (raw SQL appended, with $N renumbering)
    for frag in &query.fragments {
        let (renumbered, consumed) = renumber_placeholders(frag, param_idx);
        sql.push_str(&format!(" {}", renumbered));
        param_idx += consumed;
    }
    params.extend(query.fragment_params.iter().cloned());

    // ORDER BY clause
    if !query.order.is_empty() {
        sql.push_str(" ORDER BY ");
        let orders: Vec<String> = query
            .order
            .iter()
            .map(|o| {
                if let Some(raw) = o.strip_prefix("RAW:") {
                    raw.to_string() // emit verbatim
                } else if let Some(space_pos) = o.rfind(' ') {
                    let col = &o[..space_pos];
                    let dir = &o[space_pos + 1..];
                    format!("{} {}", quote_name(col), dir)
                } else {
                    format!("{} ASC", quote_name(o))
                }
            })
            .collect();
        sql.push_str(&orders.join(", "));
    }

    // LIMIT clause
    if query.limit >= 0 {
        sql.push_str(&format!(" LIMIT {}", query.limit));
    }

    // OFFSET clause
    if query.offset >= 0 {
        sql.push_str(&format!(" OFFSET {}", query.offset));
    }

    (sql, params)
}

/// How many rows the query returns: its SELECT (selecting `1` when it
/// names no columns, so a GROUP BY stands) counted as a subquery. A bare
/// `COUNT(*)` with a GROUP BY counted one group.
fn counted_sql(select: &str) -> String {
    format!("SELECT COUNT(*) AS count FROM ({select}) AS counted")
}

/// Whether the query returns a row, grouping and fragments included.
fn exists_sql(select: &str) -> String {
    format!("SELECT EXISTS({select}) AS exists")
}

/// `sql` run on the pool with `params`: `Pool.query`'s result.
fn run_query(pool: u64, sql: &str, params: &[String]) -> *mut u8 {
    mesh_pool_query(pool, mesh_str(sql), string_list(params))
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
    let (sql, params) = select_sql(unsafe { &query_parts(query) }, None, 1);
    run_query(pool, &sql, &params)
}

/// Execute a query and return the first matching row or error.
///
/// `Repo.one(pool, query)` -> `Result<Map<String,String>, String>`
///
/// Runs the query with LIMIT 1 and returns its row, or Err("not found").
#[no_mangle]
pub extern "C" fn mesh_repo_one(pool: u64, query: *mut u8) -> *mut u8 {
    unsafe {
        let mut parts = query_parts(query);
        parts.limit = 1;
        let (sql, params) = select_sql(&parts, None, 1);
        first_row(run_query(pool, &sql, &params), "not found")
    }
}

/// The column `Repo.get`, `update`, `delete` and `update_changeset` find a
/// row by: the table's primary key, read from the catalog, or `id` for a
/// table without a one-column one (a view, a composite key).
// ponytail: cached per pool handle and table for the process's life; a key
// changed by a migration while the pool is open is not seen.
unsafe fn primary_key(pool: u64, table: &str) -> Result<String, *mut u8> {
    use std::sync::{Mutex, OnceLock};
    static KEYS: OnceLock<Mutex<HashMap<(u64, String), String>>> = OnceLock::new();
    let keys = KEYS.get_or_init(Default::default);
    let cache_key = (pool, table.to_string());
    if let Some(key) = keys.lock().unwrap().get(&cache_key) {
        return Ok(key.clone());
    }
    let sql = "SELECT a.attname FROM pg_index i \
        JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = ANY(i.indkey) \
        WHERE i.indrelid = to_regclass($1) AND i.indisprimary";
    let result = mesh_pool_query(
        pool,
        mesh_str(sql) as *const MeshString,
        string_list(&[quote_name(table)]),
    );
    let r = &*(result as *const MeshResult);
    if r.tag != 0 {
        return Err(result);
    }
    let key = if mesh_list_length(r.value) == 1 {
        let row = mesh_list_get(r.value, 0) as *mut u8;
        let name = mesh_map_get(row, mesh_str("attname") as u64);
        text_of(name as *mut u8).to_string()
    } else {
        "id".to_string()
    };
    keys.lock().unwrap().insert(cache_key, key.clone());
    Ok(key)
}

/// Fetch a single row by primary key.
///
/// `Repo.get(pool, table, id)` -> `Result<Map<String,String>, String>`
///
/// Builds: `SELECT * FROM "table" WHERE "<primary key>" = $1 LIMIT 1`
#[no_mangle]
pub extern "C" fn mesh_repo_get(pool: u64, table: *mut u8, id: *mut u8) -> *mut u8 {
    unsafe {
        let table_str = text_of(table);
        let key = match primary_key(pool, table_str) {
            Ok(key) => key,
            Err(error) => return error,
        };
        let sql = format!(
            "SELECT * FROM {}{} LIMIT 1",
            quote_name(table_str),
            by_key(&key, 1)
        );
        let params = mesh_list_append(mesh_list_new(), id as u64);
        let result = mesh_pool_query(pool, mesh_str(&sql), params);
        first_row(result, "not found")
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
        let table_str = text_of(table);
        let field_str = text_of(field);
        let sql = format!(
            "SELECT * FROM {} WHERE {} = $1 LIMIT 1",
            quote_name(table_str),
            quote_name(field_str)
        );
        let params = mesh_list_append(mesh_list_new(), value as u64);
        let result = mesh_pool_query(pool, mesh_str(&sql), params);

        first_row(result, "not found")
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
        let (select, params) = select_sql(&query_parts(query), Some("1"), 1);
        let sql = counted_sql(&select);
        match single_value(pool, &sql, &params, "count") {
            Ok(count) => crate::io::ok_int(count.parse().unwrap_or(0)).cast(),
            Err(error) => error,
        }
    }
}

/// The text of `column` in the one row `sql` returns (a COUNT or EXISTS).
unsafe fn single_value(
    pool: u64,
    sql: &str,
    params: &[String],
    column: &str,
) -> Result<&'static str, *mut u8> {
    let result = run_query(pool, sql, params);
    let r = &*(result as *const MeshResult);
    if r.tag != 0 {
        return Err(result);
    }
    let row = mesh_list_get(r.value, 0) as *mut u8;
    Ok(text_of(
        mesh_map_get(row, mesh_str(column) as u64) as *const u8
    ))
}

/// Check if any rows match the query.
///
/// `Repo.exists(pool, query)` -> `Result<Bool, String>`
///
/// Builds: `SELECT EXISTS(SELECT 1 FROM "table" WHERE ... LIMIT 1)`
/// Returns the Bool, boxed as a Result payload is.
#[no_mangle]
pub extern "C" fn mesh_repo_exists(pool: u64, query: *mut u8) -> *mut u8 {
    unsafe {
        let (select, params) = select_sql(&query_parts(query), Some("1"), 1);
        let sql = exists_sql(&select);
        match single_value(pool, &sql, &params, "exists") {
            // A Bool payload is boxed.
            Ok(exists) => ok_result(crate::io::box_scalar(exists == "t")),
            Err(error) => error,
        }
    }
}

// ── Map extraction helpers ─────────────────────────────────────────

/// Extract (column_names, values) from a Mesh Map<String, String> pointer,
/// through the map's live entries (a map may be a view of a table).
unsafe fn map_to_columns_and_values(map: *mut u8) -> (Vec<String>, Vec<String>) {
    let (_, entries) = crate::collections::map::live_entries(map);
    let mut columns = Vec::with_capacity(entries.len());
    let mut values = Vec::with_capacity(entries.len());
    for [key, value] in entries {
        let key_ptr = key as *const MeshString;
        let val_ptr = value as *const MeshString;
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

/// Extract (column_names, expressions) from a Mesh Map<String, Ptr> pointer.
unsafe fn map_to_columns_and_exprs(map: *mut u8) -> (Vec<String>, Vec<SqlExpr>) {
    let (_, entries) = crate::collections::map::live_entries(map);
    let mut columns = Vec::with_capacity(entries.len());
    let mut exprs = Vec::with_capacity(entries.len());
    for [key, expr] in entries {
        let key_ptr = key as *const MeshString;
        let expr_ptr = expr as *mut u8;
        if !key_ptr.is_null() {
            columns.push((*key_ptr).as_str().to_string());
            exprs.push(clone_expr(expr_ptr));
        }
    }
    (columns, exprs)
}

fn build_set_expr_parts(
    columns: &[String],
    exprs: &[SqlExpr],
    start_idx: usize,
) -> (Vec<String>, Vec<String>, usize) {
    let mut params = Vec::new();
    let mut next_idx = start_idx;
    let set_parts = columns
        .iter()
        .zip(exprs)
        .map(|(column, expr)| {
            let expr_sql = render_expr(expr, &mut params, &mut next_idx);
            format!("{} = {}", quote_name(column), expr_sql)
        })
        .collect();
    (set_parts, params, next_idx)
}

fn build_update_where_expr_sql_pure(
    table: &str,
    columns: &[String],
    exprs: &[SqlExpr],
    query: &QueryParts,
) -> Result<(String, Vec<String>), &'static str> {
    if columns.is_empty() {
        return Err("update_where_expr: no fields provided");
    }
    if query.where_clauses.is_empty() {
        return Err("update_where_expr: no WHERE conditions");
    }

    let mut sql = format!("UPDATE {} SET ", quote_name(table));
    let (set_parts, mut params, next_idx) = build_set_expr_parts(columns, exprs, 1);
    sql.push_str(&set_parts.join(", "));

    let (conditions, where_param_values, _next_idx) = where_sql(query, next_idx);
    sql.push_str(&format!(" WHERE {} RETURNING *", conditions));
    params.extend(where_param_values);

    Ok((sql, params))
}

fn build_insert_or_update_expr_sql_pure(
    table: &str,
    insert_columns: &[String],
    insert_values: &[String],
    conflict_targets: &[String],
    update_columns: &[String],
    update_exprs: &[SqlExpr],
) -> Result<(String, Vec<String>), &'static str> {
    if insert_columns.is_empty() {
        return Err("insert_or_update_expr: no fields provided");
    }
    if conflict_targets.is_empty() {
        return Err("insert_or_update_expr: no conflict targets provided");
    }
    if update_columns.is_empty() {
        return Err("insert_or_update_expr: no update fields provided");
    }

    let quoted_columns = insert_columns
        .iter()
        .map(|column| quote_name(column))
        .collect::<Vec<_>>()
        .join(", ");
    let placeholders = (1..=insert_columns.len())
        .map(|idx| format!("${idx}"))
        .collect::<Vec<_>>()
        .join(", ");
    let quoted_targets = conflict_targets
        .iter()
        .map(|target| quote_name(target))
        .collect::<Vec<_>>()
        .join(", ");

    let update_exprs: Vec<SqlExpr> = update_exprs
        .iter()
        .map(|expr| expr.qualified(table))
        .collect();
    let (set_parts, update_params, _next_idx) =
        build_set_expr_parts(update_columns, &update_exprs, insert_columns.len() + 1);

    let sql = format!(
        "INSERT INTO {} ({}) VALUES ({}) ON CONFLICT ({}) DO UPDATE SET {} RETURNING *",
        quote_name(table),
        quoted_columns,
        placeholders,
        quoted_targets,
        set_parts.join(", ")
    );

    let mut params = insert_values.to_vec();
    params.extend(update_params);
    Ok((sql, params))
}

fn build_insert_expr_sql_pure(
    table: &str,
    columns: &[String],
    exprs: &[SqlExpr],
) -> Result<(String, Vec<String>), &'static str> {
    if columns.is_empty() {
        return Err("insert_expr: no fields provided");
    }

    let quoted_columns = columns
        .iter()
        .map(|column| quote_name(column))
        .collect::<Vec<_>>()
        .join(", ");

    let mut params = Vec::new();
    let mut next_idx = 1usize;
    let value_sql_parts: Vec<String> = exprs
        .iter()
        .map(|expr| render_expr(expr, &mut params, &mut next_idx))
        .collect();

    let sql = format!(
        "INSERT INTO {} ({}) VALUES ({}) RETURNING *",
        quote_name(table),
        quoted_columns,
        value_sql_parts.join(", ")
    );

    Ok((sql, params))
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
        let table_str = text_of(table);
        let (columns, values) = map_to_columns_and_values(fields);

        if columns.is_empty() {
            return err_result("insert: no fields provided");
        }

        // Build INSERT SQL with RETURNING *
        let returning = vec!["*".to_string()];
        let sql = crate::db::orm::build_insert_sql_pure(table_str, &columns, &returning);

        let result = run_query(pool, &sql, &values);

        // Check if query succeeded
        first_row(result, "insert: no row returned")
    }
}

/// Insert a new row using expression-valued fields and return the inserted record.
///
/// `Repo.insert_expr(pool, table, expr_fields_map)` -> `Result<Map<String,String>, String>`
#[no_mangle]
pub extern "C" fn mesh_repo_insert_expr(
    pool: u64,
    table: *mut u8,
    expr_fields: *mut u8,
) -> *mut u8 {
    unsafe {
        let table_str = text_of(table);
        let (columns, exprs) = map_to_columns_and_exprs(expr_fields);

        let (sql, params) = match build_insert_expr_sql_pure(table_str, &columns, &exprs) {
            Ok(built) => built,
            Err(msg) => return err_result(msg),
        };

        let result = run_query(pool, &sql, &params);

        first_row(result, "insert_expr: no row returned")
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
        let (columns, values) = map_to_columns_and_values(fields);
        if columns.is_empty() {
            return err_result("update: no fields provided");
        }
        let result = update_by_key(pool, text_of(table), id, &columns, values);
        first_row(result, "update: no row returned (id not found)")
    }
}

/// `UPDATE "table" SET "c1" = $1, ...`: `columns` set to the first
/// parameters, the WHERE to follow.
fn update_set_sql(table: &str, columns: &[String]) -> String {
    let set: Vec<String> = columns
        .iter()
        .enumerate()
        .map(|(i, column)| format!("{} = ${}", quote_name(column), i + 1))
        .collect();
    format!("UPDATE {} SET {}", quote_name(table), set.join(", "))
}

/// ` WHERE "<primary key>" = $n`: the primary key column is one name,
/// whatever it holds (a dot is no schema, a space no operator).
fn by_key(key: &str, n: usize) -> String {
    format!(" WHERE {} = ${n}", quote_ident(key))
}

/// `columns` set to `values` in the row of `table` whose primary key is
/// `id`, RETURNING it: the query's result, or the primary key lookup's.
unsafe fn update_by_key(
    pool: u64,
    table: &str,
    id: *mut u8,
    columns: &[String],
    mut values: Vec<String>,
) -> *mut u8 {
    let key = match primary_key(pool, table) {
        Ok(key) => key,
        Err(error) => return error,
    };
    let sql = format!(
        "{}{} RETURNING *",
        update_set_sql(table, columns),
        by_key(&key, columns.len() + 1)
    );
    values.push(text_of(id).to_string());
    run_query(pool, &sql, &values)
}

/// Delete a row by primary key and return the deleted record.
///
/// `Repo.delete(pool, table, id)` -> `Result<Map<String,String>, String>`
///
/// 1. Builds DELETE SQL with WHERE <primary key> = $1, RETURNING *
/// 2. Returns the first (deleted) row
#[no_mangle]
pub extern "C" fn mesh_repo_delete(pool: u64, table: *mut u8, id: *mut u8) -> *mut u8 {
    unsafe {
        let table_str = text_of(table);
        let key = match primary_key(pool, table_str) {
            Ok(key) => key,
            Err(error) => return error,
        };
        let sql = format!(
            "DELETE FROM {}{} RETURNING *",
            quote_name(table_str),
            by_key(&key, 1)
        );
        let params = mesh_list_append(mesh_list_new(), id as u64);
        let result = mesh_pool_query(pool, mesh_str(&sql), params);

        first_row(result, "delete: no row returned (id not found)")
    }
}

/// `Repo.transaction(pool, callback)`: `Pg.transaction` on a connection
/// checked out of `pool` for it, and checked back in after.
#[no_mangle]
pub extern "C" fn mesh_repo_transaction(
    pool: u64,
    fn_ptr: *const u8,
    env_ptr: *const u8,
) -> *mut u8 {
    unsafe {
        let checkout_result = mesh_pool_checkout(pool);
        let r = &*(checkout_result as *const MeshResult);
        if r.tag != 0 {
            return checkout_result;
        }
        let conn_handle = unbox_u64_payload(r.value);
        let result = crate::db::pg::mesh_pg_transaction(conn_handle, fn_ptr, env_ptr);
        mesh_pool_checkin(pool, conn_handle);
        result
    }
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

/// A changeset's changes as columns and values, or the `Err(changeset)`
/// to return without running SQL: it is invalid, or has nothing to write
/// (which its `_base` error then says).
unsafe fn changeset_changes(changeset: *mut u8) -> Result<(Vec<String>, Vec<String>), *mut u8> {
    if mesh_changeset_valid(changeset).is_null() {
        return Err(alloc_result(1, changeset) as *mut u8);
    }
    let (columns, values) = map_to_columns_and_values(mesh_changeset_changes(changeset));
    if columns.is_empty() {
        let unchanged = add_error(changeset, "_base", "has no changes");
        return Err(alloc_result(1, unchanged) as *mut u8);
    }
    Ok((columns, values))
}

/// Insert a row using a changeset, validating before SQL execution.
///
/// `Repo.insert_changeset(pool, table, changeset)` -> `Result<Map<String,String>, Changeset>`
///
/// 1. If changeset is invalid, or has no changes: return Err(changeset) without executing SQL
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
        let (columns, values) = match changeset_changes(changeset) {
            Ok(changes) => changes,
            Err(refused) => return refused,
        };
        let returning = vec!["*".to_string()];
        let sql = crate::db::orm::build_insert_sql_pure(text_of(table), &columns, &returning);
        let result = run_query(pool, &sql, &values);
        changeset_write_result(result, changeset, "no row returned")
    }
}

/// Update a row using a changeset, validating before SQL execution.
///
/// `Repo.update_changeset(pool, table, id, changeset)` -> `Result<Map<String,String>, Changeset>`
///
/// Same pattern as insert_changeset but builds UPDATE SQL with WHERE <primary key> = $N+1.
#[no_mangle]
pub extern "C" fn mesh_repo_update_changeset(
    pool: u64,
    table: *mut u8,
    id: *mut u8,
    changeset: *mut u8,
) -> *mut u8 {
    unsafe {
        let (columns, values) = match changeset_changes(changeset) {
            Ok(changes) => changes,
            Err(refused) => return refused,
        };
        let result = update_by_key(pool, text_of(table), id, &columns, values);
        changeset_write_result(result, changeset, "not found")
    }
}

/// A changeset write's outcome as `Result<Map<String, String>, Changeset>`:
/// the row, or the changeset carrying why there is none (a constraint
/// violation on its field, anything else on `_base`).
unsafe fn changeset_write_result(result: *mut u8, changeset: *mut u8, missing: &str) -> *mut u8 {
    let r = &*(result as *const MeshResult);
    let (field, message) = if r.tag != 0 {
        let (sqlstate, constraint, pg_table, column, _message) =
            parse_pg_error_string(text_of(r.value));
        map_constraint_error(sqlstate, constraint, pg_table, column)
            .unwrap_or_else(|| ("_base".to_string(), "database error".to_string()))
    } else if mesh_list_length(r.value) == 0 {
        ("_base".to_string(), missing.to_string())
    } else {
        return ok_result(mesh_list_get(r.value, 0) as *mut u8);
    };
    alloc_result(1, add_error(changeset, &field, &message)) as *mut u8
}

// ── Preload Operations (Phase 100) ─────────────────────────────────

use crate::collections::list::list_strings;
use crate::collections::list::string_list;
use crate::string::text_of;
use std::collections::{HashMap, HashSet};

/// Parsed relationship metadata from "kind:name:target:fk:target_table:key"
/// strings (`key` may be left out, meaning "id").
struct RelMeta {
    kind: String,         // "belongs_to", "has_many", "has_one"
    fk: String,           // foreign key column (e.g., "user_id")
    target_table: String, // target table (e.g., "posts")
    key: String,          // the primary key `fk` refers to (e.g., "id")
}

/// Parse relationship metadata strings into a lookup map keyed by association name.
fn parse_relationship_meta(meta_strings: &[String]) -> HashMap<String, RelMeta> {
    let mut map = HashMap::new();
    for entry in meta_strings {
        let parts: Vec<&str> = entry.splitn(6, ':').collect();
        if parts.len() >= 5 {
            map.insert(
                parts[1].to_string(),
                RelMeta {
                    kind: parts[0].to_string(),
                    fk: parts[3].to_string(),
                    target_table: parts[4].to_string(),
                    key: parts.get(5).unwrap_or(&"id").to_string(),
                },
            );
        }
    }
    map
}

/// Build a simple SELECT query with an IN clause for preloading.
/// Returns (sql, params) where params are the IN values.
fn build_preload_sql(table: &str, where_col: &str, ids: &[String]) -> (String, Vec<String>) {
    let mut sql = format!("SELECT * FROM {}", quote_name(table));
    let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("${}", i)).collect();
    sql.push_str(&format!(
        " WHERE {} IN ({})",
        quote_name(where_col),
        placeholders.join(", ")
    ));
    (sql, ids.to_vec())
}

/// Preload a single direct association onto a list of rows.
///
/// For has_many/has_one: collects parent "id" values, queries WHERE fk IN (...), groups by fk.
/// For belongs_to: collects parent FK values, queries WHERE id IN (...), groups by id.
///
/// Returns a new list with each row enriched with the association data:
/// - has_many: a List pointer under the association key
/// - has_one/belongs_to: a Map pointer (single row) under the association key, or null
unsafe fn preload_direct(
    pool: u64,
    rows: *mut u8,
    assoc_name: &str,
    rel_map: &HashMap<String, RelMeta>,
) -> Result<*mut u8, *mut u8> {
    let meta = rel_map.get(assoc_name)
        .ok_or_else(|| err_result(&format!("Repo.preload: unknown association '{}' -- check that the relationship metadata includes this association", assoc_name)))?;

    let row_count = mesh_list_length(rows);

    // Determine which column to extract from parent rows and which column to match in target
    let (parent_key, target_match_key) = match meta.kind.as_str() {
        "has_many" | "has_one" => {
            // Parent PK -> target FK: collect parent key values,
            // query target WHERE fk IN (...), group by fk
            (meta.key.clone(), meta.fk.clone())
        }
        "belongs_to" => {
            // Parent FK -> target PK: collect parent FK values,
            // query target WHERE key IN (...), group by key
            (meta.fk.clone(), meta.key.clone())
        }
        _ => {
            return Err(err_result(&format!(
                "Repo.preload: unknown relationship kind '{}'",
                meta.kind
            )))
        }
    };

    // 1. The distinct parent keys. An empty one is NULL (or the row lacks
    // the column), which matches no row: sent along, "" is not even a valid
    // integer or uuid to PostgreSQL.
    let parent_key_mesh = mesh_str(&parent_key) as u64;
    let key_of = |row: *mut u8| match mesh_map_get(row, parent_key_mesh) {
        0 => "",
        key => text_of(key as *mut u8),
    };
    let mut seen = HashSet::new();
    let ids: Vec<String> = (0..row_count)
        .map(|i| key_of(mesh_list_get(rows, i) as *mut u8))
        .filter(|key| !key.is_empty() && seen.insert(*key))
        .map(str::to_string)
        .collect();

    // 2. The target rows those keys match, grouped by their key
    let mut grouped: HashMap<&str, Vec<u64>> = HashMap::new();
    if !ids.is_empty() {
        let (sql, params) = build_preload_sql(&meta.target_table, &target_match_key, &ids);
        let result = run_query(pool, &sql, &params);
        let r = &*(result as *const MeshResult);
        if r.tag != 0 {
            return Err(result);
        }
        let match_key_mesh = mesh_str(&target_match_key) as u64;
        for i in 0..mesh_list_length(r.value) {
            let row = mesh_list_get(r.value, i);
            let key = text_of(mesh_map_get(row as *mut u8, match_key_mesh) as *mut u8);
            grouped.entry(key).or_default().push(row);
        }
    }

    // 3. Each parent row with its association under the association's name:
    // has_many a list of rows, has_one and belongs_to a row or null (0).
    let assoc_key_mesh = mesh_str(assoc_name) as u64;
    let many = meta.kind == "has_many";
    let mut enriched = mesh_list_new();
    for i in 0..row_count {
        let row = mesh_list_get(rows, i) as *mut u8;
        let matches = grouped.get(key_of(row)).map_or(&[][..], Vec::as_slice);
        let assoc_data = match many {
            true => mesh_list_from_array(matches.as_ptr(), matches.len() as i64) as u64,
            false => matches.first().copied().unwrap_or(0),
        };
        let new_row = mesh_map_put(row, assoc_key_mesh, assoc_data);
        enriched = mesh_list_append(enriched, new_row as u64);
    }

    Ok(enriched)
}

/// Preload `path` onto `rows`: one association (`posts`), or a dotted path
/// through them (`posts.comments`).
unsafe fn preload_path(
    pool: u64,
    rows: *mut u8,
    path: &str,
    rel_map: &HashMap<String, RelMeta>,
) -> Result<*mut u8, *mut u8> {
    match path.split_once('.') {
        Some((parent, child)) => preload_nested(pool, rows, parent, child, rel_map),
        None => preload_direct(pool, rows, path, rel_map),
    }
}

/// Preload `child_assoc` below `parent_assoc` ("posts.comments"):
/// 1. Collect all intermediate rows from the parent association (flatten all has_many lists)
/// 2. Preload child_assoc on the intermediate rows using the SAME merged metadata
/// 3. Re-stitch: rebuild parent association lists using positional tracking
unsafe fn preload_nested(
    pool: u64,
    rows: *mut u8,
    parent_assoc: &str,
    child_assoc: &str,
    rel_map: &HashMap<String, RelMeta>,
) -> Result<*mut u8, *mut u8> {
    let row_count = mesh_list_length(rows);
    let parent_key_mesh = mesh_str(parent_assoc) as *mut u8;

    // Check parent association's kind to decide how to extract intermediate rows
    let parent_meta = rel_map.get(parent_assoc).ok_or_else(|| {
        err_result(&format!(
            "Repo.preload: unknown parent association '{}' in nested path",
            parent_assoc
        ))
    })?;

    // Collect intermediate rows and track which parent row each came from
    // and its position within the parent's association list.
    // Structure: Vec<(parent_row_index, position_in_list, intermediate_row_ptr)>
    let mut intermediate_rows = mesh_list_new();
    let mut position_map: Vec<(i64, i64)> = Vec::new(); // (parent_idx, pos_in_list)

    for i in 0..row_count {
        let row = mesh_list_get(rows, i) as *mut u8;
        let assoc_val = mesh_map_get(row, parent_key_mesh as u64);
        if assoc_val != 0 {
            if parent_meta.kind == "has_many" {
                let sub_list = assoc_val as *mut u8;
                let sub_count = mesh_list_length(sub_list);
                for j in 0..sub_count {
                    let sub_row = mesh_list_get(sub_list, j);
                    intermediate_rows = mesh_list_append(intermediate_rows, sub_row);
                    position_map.push((i, j));
                }
            } else {
                // has_one or belongs_to: single row
                intermediate_rows = mesh_list_append(intermediate_rows, assoc_val);
                position_map.push((i, 0));
            }
        }
    }

    let intermediate_count = mesh_list_length(intermediate_rows);
    if intermediate_count == 0 {
        return Ok(rows); // nothing to preload at nested level
    }

    let enriched_intermediate = preload_path(pool, intermediate_rows, child_assoc, rel_map)?;

    // Re-stitch: rebuild parent rows with enriched intermediate rows
    // Group enriched intermediate rows back by parent index
    let mut parent_groups: HashMap<i64, Vec<*mut u8>> = HashMap::new();
    for (idx, &(parent_idx, _pos)) in position_map.iter().enumerate() {
        let enriched_row = mesh_list_get(enriched_intermediate, idx as i64) as *mut u8;
        parent_groups
            .entry(parent_idx)
            .or_default()
            .push(enriched_row);
    }

    // Rebuild parent rows
    let assoc_key_mesh_parent = mesh_str(parent_assoc) as *mut u8;
    let mut result = mesh_list_new();
    for i in 0..row_count {
        let row = mesh_list_get(rows, i) as *mut u8;
        if let Some(enriched_children) = parent_groups.get(&i) {
            if parent_meta.kind == "has_many" {
                // Rebuild the has_many list with enriched rows
                let mut new_list = mesh_list_new();
                for &child in enriched_children {
                    new_list = mesh_list_append(new_list, child as u64);
                }
                let new_row = mesh_map_put(row, assoc_key_mesh_parent as u64, new_list as u64);
                result = mesh_list_append(result, new_row as u64);
            } else {
                // has_one/belongs_to: single enriched row
                let new_row = mesh_map_put(
                    row,
                    assoc_key_mesh_parent as u64,
                    enriched_children[0] as u64,
                );
                result = mesh_list_append(result, new_row as u64);
            }
        } else {
            // No intermediate rows for this parent -- keep as-is
            result = mesh_list_append(result, row as u64);
        }
    }

    Ok(result)
}

/// The preloaded association paths as a tree: `["posts", "posts.comments"]`
/// is posts -> comments.
#[derive(Default)]
struct AssocTree(HashMap<String, AssocTree>);

fn assoc_tree(paths: &[String]) -> AssocTree {
    let mut tree = AssocTree::default();
    for path in paths {
        let mut node = &mut tree;
        for name in path.split('.') {
            node = node.0.entry(name.to_string()).or_default();
        }
    }
    tree
}

/// A preloaded association as JSON: has_many a list of rows (an array),
/// has_one and belongs_to a row or null (0).
unsafe fn association_json(
    value: u64,
    many: bool,
    nested: &AssocTree,
    rel_map: &HashMap<String, RelMeta>,
) -> serde_json::Value {
    if many {
        let list = value as *mut u8;
        let rows = (0..mesh_list_length(list))
            .map(|i| row_json(mesh_list_get(list, i) as *mut u8, nested, rel_map));
        serde_json::Value::Array(rows.collect())
    } else if value == 0 {
        serde_json::Value::Null
    } else {
        row_json(value as *mut u8, nested, rel_map)
    }
}

/// A row as a JSON object: its columns as strings, the associations preloaded
/// onto it as `association_json`.
unsafe fn row_json(
    row: *mut u8,
    associations: &AssocTree,
    rel_map: &HashMap<String, RelMeta>,
) -> serde_json::Value {
    let mut object = serde_json::Map::new();
    for i in 0..mesh_map_size(row) {
        let key = text_of(mesh_map_entry_key(row, i) as *mut u8);
        let value = mesh_map_entry_value(row, i);
        let json = match associations.0.get(key) {
            Some(nested) => {
                let many = rel_map.get(key).is_some_and(|meta| meta.kind == "has_many");
                association_json(value, many, nested, rel_map)
            }
            None => serde_json::Value::String(text_of(value as *mut u8).to_string()),
        };
        object.insert(key.to_string(), json);
    }
    serde_json::Value::Object(object)
}

/// Batch preload associated records for a list of parent rows.
///
/// `Repo.preload(pool, rows, associations, relationship_meta)`
///   -> `Result<List<Map<String,String>>, String>`
///
/// For each association in the list:
/// 1. Parse relationship metadata to find FK, target table, cardinality
/// 2. Collect unique parent IDs
/// 3. Execute: SELECT * FROM "target_table" WHERE "fk" IN ($1, $2, ...)
/// 4. Group results by FK value
/// 5. Attach grouped results to each parent row under the association key
///
/// Associations are sorted by nesting depth (atoms/direct first, then "a.b", then "a.b.c")
/// to ensure parent-level data is loaded before nested preloading accesses it.
/// While they load, an association is a list or row pointer; each row then
/// gets it as JSON text (`association_json`), which is what a
/// `Map<String, String>` can hold: a pointer there would be read, printed and
/// copied between actors as a string.
#[no_mangle]
pub extern "C" fn mesh_repo_preload(
    pool: u64,
    rows: *mut u8,
    associations: *mut u8,
    rel_meta: *mut u8,
) -> *mut u8 {
    unsafe {
        let row_count = mesh_list_length(rows);
        if row_count == 0 {
            return ok_result(rows); // nothing to preload
        }

        // Parse relationship metadata into lookup map
        let meta_strings = list_strings(rel_meta);
        let rel_map = parse_relationship_meta(&meta_strings);

        // Parse association names
        let assoc_names = list_strings(associations);

        // Sort by depth: direct associations (depth 0) first, then nested
        let mut sorted_assocs: Vec<(usize, String)> = assoc_names
            .iter()
            .map(|a| (a.matches('.').count(), a.clone()))
            .collect();
        sorted_assocs.sort_by_key(|(depth, _)| *depth);

        // Working copy: enrich rows progressively
        let mut current_rows = rows;

        for (_depth, assoc_path) in &sorted_assocs {
            match preload_path(pool, current_rows, assoc_path, &rel_map) {
                Ok(enriched) => current_rows = enriched,
                Err(e) => return e,
            }
        }

        let tree = assoc_tree(&assoc_names);
        let mut encoded = mesh_list_new();
        for i in 0..mesh_list_length(current_rows) {
            let mut row = mesh_list_get(current_rows, i) as *mut u8;
            for (name, nested) in &tree.0 {
                let key = mesh_str(name) as u64;
                if mesh_map_has_key(row, key) == 0 {
                    continue;
                }
                let many = rel_map
                    .get(name)
                    .is_some_and(|meta| meta.kind == "has_many");
                let json = association_json(mesh_map_get(row, key), many, nested, &rel_map);
                row = mesh_map_put(row, key, mesh_str(&json.to_string()) as u64);
            }
            encoded = mesh_list_append(encoded, row as u64);
        }
        ok_result(encoded)
    }
}

// ── Shared WHERE clause builder ──────────────────────────────────────

/// The query's WHERE conditions (without the keyword), joined by AND.
///
/// Returns `(where_sql, params, next_param_idx)`.
/// `start_idx` is the first $N placeholder to use. Each clause the Query
/// builders write carries exactly the values it binds (`where_or` and
/// `where_raw` check theirs), in order.
fn where_sql(query: &QueryParts, start_idx: usize) -> (String, Vec<String>, usize) {
    let mut values = query.where_params.iter();
    let mut subqueries = query.subqueries.iter();
    let mut conditions = Vec::new();
    let mut params = Vec::new();
    let mut param_idx = start_idx;
    // The next `count` values, bound: the placeholders numbering them.
    let mut bind = |count: usize, params: &mut Vec<String>, param_idx: &mut usize| {
        (0..count)
            .map(|_| {
                params.push(values.next().expect("a clause has its values").clone());
                *param_idx += 1;
                format!("${}", *param_idx - 1)
            })
            .collect::<Vec<_>>()
    };

    for clause in &query.where_clauses {
        let condition = if let Some(field) = clause.strip_prefix("SUB:") {
            let sub = subqueries.next().expect("a where_sub clause has its query");
            let (sub_sql, sub_params) = select_sql(sub, None, param_idx);
            param_idx += sub_params.len();
            params.extend(sub_params);
            format!("{} IN ({sub_sql})", quote_name(field))
        } else if let Some(fields) = clause.strip_prefix("OR:") {
            // "OR:field1,field2,...": each field equal to its value.
            let fields: Vec<&str> = fields.split(',').filter(|f| !f.is_empty()).collect();
            let placeholders = bind(fields.len(), &mut params, &mut param_idx);
            let or_parts: Vec<String> = fields
                .iter()
                .zip(placeholders)
                .map(|(field, placeholder)| format!("{} = {placeholder}", quote_name(field)))
                .collect();
            match or_parts.is_empty() {
                true => "FALSE".to_string(),
                false => format!("({})", or_parts.join(" OR ")),
            }
        } else if let Some(expr) = clause.strip_prefix("EXPR:") {
            render_expr(&parse_expr(expr), &mut params, &mut param_idx)
        } else if let Some(raw_sql) = clause.strip_prefix("RAW:") {
            let (renumbered, consumed) = renumber_placeholders(raw_sql, param_idx);
            bind(consumed, &mut params, &mut param_idx);
            renumbered
        } else {
            // "field op": the field is an atom, so the first space ends it.
            let (col, op) = clause.split_once(' ').expect("a clause names its field");
            let col = quote_name(col);
            if op == "IS NULL" || op == "IS NOT NULL" {
                format!("{col} {op}")
            } else if let Some((keyword, count, of_none)) = (op.strip_prefix("IN:"))
                .map(|count| ("IN", count, "FALSE"))
                .or_else(|| {
                    op.strip_prefix("NOT_IN:")
                        .map(|count| ("NOT IN", count, "TRUE"))
                })
            {
                // No value is in an empty list (`IN ()` is not SQL).
                let count: usize = count.parse().expect("IN:<count>");
                let placeholders = bind(count, &mut params, &mut param_idx);
                match count {
                    0 => of_none.to_string(),
                    _ => format!("{col} {keyword} ({})", placeholders.join(", ")),
                }
            } else if op == "BETWEEN" {
                let bounds = bind(2, &mut params, &mut param_idx);
                format!("{col} BETWEEN {} AND {}", bounds[0], bounds[1])
            } else {
                format!("{col} {op} {}", bind(1, &mut params, &mut param_idx)[0])
            }
        };
        conditions.push(condition);
    }

    (conditions.join(" AND "), params, param_idx)
}

// ── Extended Write Operations (Phase 103) ────────────────────────────

/// Update rows matching a Query's WHERE conditions.
/// `Repo.update_where(pool, table, fields_map, query)` -> `Result<Map<String,String>, String>`
#[no_mangle]
pub extern "C" fn mesh_repo_update_where(
    pool: u64,
    table: *mut u8,
    fields: *mut u8,
    query: *mut u8,
) -> *mut u8 {
    unsafe {
        let table_str = text_of(table);
        let (columns, mut values) = map_to_columns_and_values(fields);

        if columns.is_empty() {
            return err_result("update_where: no fields provided");
        }

        let query = query_parts(query);

        if query.where_clauses.is_empty() {
            return err_result("update_where: no WHERE conditions");
        }

        let mut sql = update_set_sql(table_str, &columns);
        let start_idx = columns.len() + 1;
        let (conditions, where_param_values, _next_idx) = where_sql(&query, start_idx);
        sql.push_str(&format!(" WHERE {} RETURNING *", conditions));

        values.extend(where_param_values);

        let result = run_query(pool, &sql, &values);

        first_row(result, "update_where: no rows matched")
    }
}

/// Update rows matching a Query's WHERE conditions with expression-valued SET clauses.
/// `Repo.update_where_expr(pool, table, expr_fields_map, query)`
///   -> `Result<Map<String,String>, String>`
#[no_mangle]
pub extern "C" fn mesh_repo_update_where_expr(
    pool: u64,
    table: *mut u8,
    expr_fields: *mut u8,
    query: *mut u8,
) -> *mut u8 {
    unsafe {
        let table_str = text_of(table);
        let (columns, exprs) = map_to_columns_and_exprs(expr_fields);
        let query = query_parts(query);

        let (sql, params) =
            match build_update_where_expr_sql_pure(table_str, &columns, &exprs, &query) {
                Ok(built) => built,
                Err(msg) => return err_result(msg),
            };

        let result = run_query(pool, &sql, &params);

        first_row(result, "update_where_expr: no rows matched")
    }
}

/// Delete rows matching a Query's WHERE conditions.
/// `Repo.delete_where(pool, table, query)` -> `Result<Int, String>`
#[no_mangle]
pub extern "C" fn mesh_repo_delete_where(pool: u64, table: *mut u8, query: *mut u8) -> *mut u8 {
    unsafe {
        let table_str = text_of(table);

        let query = query_parts(query);

        if query.where_clauses.is_empty() {
            return err_result("delete_where: no WHERE conditions");
        }

        let mut sql = format!("DELETE FROM {}", quote_name(table_str));
        let (conditions, where_param_values, _next_idx) = where_sql(&query, 1);
        sql.push_str(&format!(" WHERE {}", conditions));

        mesh_pool_execute(pool, mesh_str(&sql), string_list(&where_param_values))
    }
}

/// Upsert: INSERT with ON CONFLICT DO UPDATE.
///
/// `Repo.insert_or_update(pool, table, fields_map, conflict_targets, update_fields)`
///   -> `Result<Map<String,String>, String>`
///
/// - `fields_map`: Map<String,String> of column->value for the INSERT
/// - `conflict_targets`: List<String> of conflict target column names
/// - `update_fields`: List<String> of column names to update on conflict
///
/// Generated SQL:
///   INSERT INTO "table" ("cols") VALUES ($1, ...)
///   ON CONFLICT ("target1", "target2") DO UPDATE SET "col" = EXCLUDED."col", ...
///   RETURNING *
#[no_mangle]
pub extern "C" fn mesh_repo_insert_or_update(
    pool: u64,
    table: *mut u8,
    fields: *mut u8,
    conflict_targets: *mut u8,
    update_fields: *mut u8,
) -> *mut u8 {
    unsafe {
        let table_str = text_of(table);
        let (columns, values) = map_to_columns_and_values(fields);
        if columns.is_empty() {
            return err_result("insert_or_update: no fields provided");
        }
        let targets = list_strings(conflict_targets);
        let updates = list_strings(update_fields);
        if targets.is_empty() {
            return err_result("insert_or_update: no conflict targets provided");
        }
        if updates.is_empty() {
            return err_result("insert_or_update: no update fields provided");
        }

        let returning = vec!["*".to_string()];
        let sql = crate::db::orm::build_upsert_sql_pure(
            table_str, &columns, &targets, &updates, &returning,
        );

        let result = run_query(pool, &sql, &values);

        first_row(result, "insert_or_update: no row returned")
    }
}

/// Upsert with expression-valued ON CONFLICT DO UPDATE assignments.
/// `Repo.insert_or_update_expr(pool, table, fields_map, conflict_targets, expr_fields_map)`
///   -> `Result<Map<String,String>, String>`
#[no_mangle]
pub extern "C" fn mesh_repo_insert_or_update_expr(
    pool: u64,
    table: *mut u8,
    fields: *mut u8,
    conflict_targets: *mut u8,
    expr_fields: *mut u8,
) -> *mut u8 {
    unsafe {
        let table_str = text_of(table);
        let (insert_columns, insert_values) = map_to_columns_and_values(fields);
        let targets = list_strings(conflict_targets);
        let (update_columns, update_exprs) = map_to_columns_and_exprs(expr_fields);

        let (sql, params) = match build_insert_or_update_expr_sql_pure(
            table_str,
            &insert_columns,
            &insert_values,
            &targets,
            &update_columns,
            &update_exprs,
        ) {
            Ok(built) => built,
            Err(msg) => return err_result(msg),
        };

        let result = run_query(pool, &sql, &params);

        first_row(result, "insert_or_update_expr: no row returned")
    }
}

/// Delete rows matching a Query's WHERE conditions and return deleted rows.
/// `Repo.delete_where_returning(pool, table, query)` -> `Result<List<Map<String,String>>, String>`
#[no_mangle]
pub extern "C" fn mesh_repo_delete_where_returning(
    pool: u64,
    table: *mut u8,
    query: *mut u8,
) -> *mut u8 {
    unsafe {
        let table_str = text_of(table);
        let query = query_parts(query);

        if query.where_clauses.is_empty() {
            return err_result("delete_where_returning: no WHERE conditions");
        }

        let mut sql = format!("DELETE FROM {}", quote_name(table_str));
        let (conditions, where_param_values, _next_idx) = where_sql(&query, 1);
        sql.push_str(&format!(" WHERE {} RETURNING *", conditions));

        run_query(pool, &sql, &where_param_values)
    }
}

/// Execute raw SQL and return rows.
/// `Repo.query_raw(pool, sql, params)` -> `Result<List<Map<String,String>>, String>`
#[no_mangle]
pub extern "C" fn mesh_repo_query_raw(pool: u64, sql: *mut u8, params: *mut u8) -> *mut u8 {
    let sql_ptr = sql as *const MeshString;
    mesh_pool_query(pool, sql_ptr, params)
}

/// Execute raw SQL and return affected row count.
/// `Repo.execute_raw(pool, sql, params)` -> `Result<Int, String>`
#[no_mangle]
pub extern "C" fn mesh_repo_execute_raw(pool: u64, sql: *mut u8, params: *mut u8) -> *mut u8 {
    let sql_ptr = sql as *const MeshString;
    mesh_pool_execute(pool, sql_ptr, params)
}

// ── Unit tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// `Repo.all`'s SELECT for a query of these clauses (a Query's slots, in
    /// order).
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
        let parts = QueryParts {
            source: source.to_string(),
            select: select_fields.to_vec(),
            order: order_fields.to_vec(),
            limit: limit_val,
            offset: offset_val,
            joins: join_clauses.to_vec(),
            group: group_fields.to_vec(),
            having: having_clauses.to_vec(),
            having_params: having_params.to_vec(),
            fragments: fragment_parts.to_vec(),
            fragment_params: fragment_params.to_vec(),
            ..filtered(where_clauses, where_params)
        };
        select_sql(&parts, None, 1)
    }

    /// A query with only these WHERE clauses and parameters.
    fn filtered(where_clauses: &[String], where_params: &[String]) -> QueryParts {
        QueryParts {
            source: String::new(),
            select: vec![],
            where_clauses: where_clauses.to_vec(),
            where_params: where_params.to_vec(),
            order: vec![],
            limit: -1,
            offset: -1,
            joins: vec![],
            group: vec![],
            having: vec![],
            having_params: vec![],
            fragments: vec![],
            fragment_params: vec![],
            subqueries: vec![],
        }
    }

    fn build_where_from_query_parts(
        where_clauses: &[String],
        where_params: &[String],
        start_idx: usize,
    ) -> (String, Vec<String>, usize) {
        where_sql(&filtered(where_clauses, where_params), start_idx)
    }

    /// A query's entry for `expr` (in its SELECT or WHERE list).
    fn expr_entry(expr: &SqlExpr) -> String {
        format!("EXPR:{}", serde_json::to_string(expr).unwrap())
    }

    fn value(text: &str) -> Box<SqlExpr> {
        Box::new(SqlExpr::Value(text.into()))
    }

    fn column(name: &str) -> Box<SqlExpr> {
        Box::new(SqlExpr::Column(name.into()))
    }

    #[test]
    fn test_select_all_from_table() {
        let (sql, params) = build_select_sql_from_parts(
            "users",
            &[],
            &[],
            &[],
            &[],
            -1,
            -1,
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
        );
        assert_eq!(sql, "SELECT * FROM \"users\"");
        assert!(params.is_empty());
    }

    #[test]
    fn test_select_with_columns() {
        let (sql, _) = build_select_sql_from_parts(
            "users",
            &["id".into(), "name".into()],
            &[],
            &[],
            &[],
            -1,
            -1,
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
        );
        assert_eq!(sql, "SELECT \"id\", \"name\" FROM \"users\"");
    }

    #[test]
    fn test_select_with_where() {
        let (sql, params) = build_select_sql_from_parts(
            "users",
            &[],
            &["name =".into(), "age >".into()],
            &["Alice".into(), "21".into()],
            &[],
            -1,
            -1,
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
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
            "users",
            &[],
            &["deleted_at IS NULL".into(), "name =".into()],
            &["Alice".into()],
            &[],
            -1,
            -1,
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
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
            "users",
            &[],
            &["status IN:3".into()],
            &["active".into(), "pending".into(), "trial".into()],
            &[],
            -1,
            -1,
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
        );
        assert_eq!(
            sql,
            "SELECT * FROM \"users\" WHERE \"status\" IN ($1, $2, $3)"
        );
        assert_eq!(params, vec!["active", "pending", "trial"]);
    }

    // ── Phase 106: Advanced WHERE operators ──────────────────────────────

    #[test]
    fn test_select_with_not_in() {
        let (sql, params) = build_select_sql_from_parts(
            "issues",
            &[],
            &["status NOT_IN:2".into()],
            &["archived".into(), "deleted".into()],
            &[],
            -1,
            -1,
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
        );
        assert_eq!(
            sql,
            "SELECT * FROM \"issues\" WHERE \"status\" NOT IN ($1, $2)"
        );
        assert_eq!(params, vec!["archived", "deleted"]);
    }

    #[test]
    fn test_select_with_between() {
        let (sql, params) = build_select_sql_from_parts(
            "events",
            &[],
            &["age BETWEEN".into()],
            &["18".into(), "65".into()],
            &[],
            -1,
            -1,
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
        );
        assert_eq!(
            sql,
            "SELECT * FROM \"events\" WHERE \"age\" BETWEEN $1 AND $2"
        );
        assert_eq!(params, vec!["18", "65"]);
    }

    #[test]
    fn test_select_with_or() {
        let (sql, params) = build_select_sql_from_parts(
            "issues",
            &[],
            &["OR:status,level".into()],
            &["active".into(), "error".into()],
            &[],
            -1,
            -1,
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
        );
        assert_eq!(
            sql,
            "SELECT * FROM \"issues\" WHERE (\"status\" = $1 OR \"level\" = $2)"
        );
        assert_eq!(params, vec!["active", "error"]);
    }

    #[test]
    fn test_select_with_ilike() {
        let (sql, params) = build_select_sql_from_parts(
            "users",
            &[],
            &["name ILIKE".into()],
            &["%alice%".into()],
            &[],
            -1,
            -1,
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
        );
        assert_eq!(sql, "SELECT * FROM \"users\" WHERE \"name\" ILIKE $1");
        assert_eq!(params, vec!["%alice%"]);
    }

    #[test]
    fn test_mixed_where_clauses() {
        // Combines: WHERE + NOT IN + BETWEEN + OR to verify $N sequencing
        let (sql, params) = build_select_sql_from_parts(
            "events",
            &[],
            &[
                "project_id =".into(),
                "status NOT_IN:2".into(),
                "age BETWEEN".into(),
                "OR:status,priority".into(),
            ],
            &[
                "abc".into(),
                "archived".into(),
                "deleted".into(),
                "18".into(),
                "65".into(),
                "active".into(),
                "high".into(),
            ],
            &[],
            -1,
            -1,
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
        );
        assert_eq!(
            sql,
            "SELECT * FROM \"events\" WHERE \"project_id\" = $1 AND \"status\" NOT IN ($2, $3) AND \"age\" BETWEEN $4 AND $5 AND (\"status\" = $6 OR \"priority\" = $7)"
        );
        assert_eq!(
            params,
            vec!["abc", "archived", "deleted", "18", "65", "active", "high"]
        );
    }

    #[test]
    fn test_select_with_join() {
        let (sql, _) = build_select_sql_from_parts(
            "users",
            &[],
            &[],
            &[],
            &[],
            -1,
            -1,
            &["INNER:posts:posts.user_id = users.id".into()],
            &[],
            &[],
            &[],
            &[],
            &[],
        );
        assert_eq!(
            sql,
            "SELECT * FROM \"users\" INNER JOIN \"posts\" ON posts.user_id = users.id"
        );
    }

    #[test]
    fn test_select_with_group_by_having() {
        let (sql, params) = build_select_sql_from_parts(
            "orders",
            &[],
            &[],
            &[],
            &[],
            -1,
            -1,
            &[],
            &["category".into()],
            &["count(*) >".into()],
            &["5".into()],
            &[],
            &[],
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
            "users",
            &[],
            &[],
            &[],
            &["name ASC".into(), "age DESC".into()],
            10,
            20,
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
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
            10,
            0,
            &["INNER:posts:posts.user_id = users.id".into()],
            &[],
            &[],
            &[],
            &[],
            &[],
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
            "users",
            &[],
            &[],
            &[],
            &[],
            -1,
            -1,
            &[],
            &[],
            &[],
            &[],
            &["AND custom_fn(?)".into()],
            &["test_val".into()],
        );
        assert_eq!(sql, "SELECT * FROM \"users\" AND custom_fn($1)");
        assert_eq!(params, vec!["test_val"]);
    }

    /// `?` and `$N` are parameters outside quotes, and text inside them;
    /// `$0` and a number too long for an index are no parameters at all.
    #[test]
    fn placeholders_are_renumbered_outside_quotes() {
        assert_eq!(
            renumber_placeholders("a = ? AND b = '?' AND \"c?\" = ? AND d = 'it''s ?'", 3),
            (
                "a = $3 AND b = '?' AND \"c?\" = $4 AND d = 'it''s ?'".to_string(),
                2
            )
        );
        assert_eq!(
            renumber_placeholders("x = $2 AND y = '$1' AND z = $1", 5),
            ("x = $6 AND y = '$1' AND z = $5".to_string(), 2)
        );
        assert_eq!(
            renumber_placeholders("$0 + $99999999999999999999999 + $", 2),
            ("$0 + $99999999999999999999999 + $".to_string(), 0)
        );
    }

    // ── Phase 106 Plan 02: Fragment $N renumbering and raw ORDER BY/GROUP BY ──

    #[test]
    fn test_fragment_dollar_renumbering() {
        // Fragment with $1 after 2 WHERE params should renumber $1 -> $3
        let (sql, params) = build_select_sql_from_parts(
            "users",
            &[],
            &["email =".into(), "active =".into()],
            &["alice@example.com".into(), "true".into()],
            &[],
            -1,
            -1,
            &[],
            &[],
            &[],
            &[],
            &["AND password_hash = crypt($1, password_hash)".into()],
            &["secret".into()],
        );
        assert_eq!(
            sql,
            "SELECT * FROM \"users\" WHERE \"email\" = $1 AND \"active\" = $2 AND password_hash = crypt($3, password_hash)"
        );
        assert_eq!(params, vec!["alice@example.com", "true", "secret"]);
    }

    #[test]
    fn test_where_raw_dollar_renumbering() {
        // where_raw with $1 after 1 WHERE param should renumber $1 -> $2
        let (sql, params) = build_select_sql_from_parts(
            "users",
            &[],
            &["active =".into(), "RAW:email ILIKE $1".into()],
            &["true".into(), "%@example.com".into()],
            &[],
            -1,
            -1,
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
        );
        assert_eq!(
            sql,
            "SELECT * FROM \"users\" WHERE \"active\" = $1 AND email ILIKE $2"
        );
        assert_eq!(params, vec!["true", "%@example.com"]);
    }

    #[test]
    fn test_order_by_raw() {
        let (sql, params) = build_select_sql_from_parts(
            "events",
            &[],
            &[],
            &[],
            &["RAW:random()".into()],
            -1,
            -1,
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
        );
        assert_eq!(sql, "SELECT * FROM \"events\" ORDER BY random()");
        assert!(params.is_empty());
    }

    #[test]
    fn test_group_by_raw() {
        let (sql, params) = build_select_sql_from_parts(
            "events",
            &[],
            &[],
            &[],
            &[],
            -1,
            -1,
            &[],
            &["RAW:date_trunc('hour', received_at)".into()],
            &[],
            &[],
            &[],
            &[],
        );
        assert_eq!(
            sql,
            "SELECT * FROM \"events\" GROUP BY date_trunc('hour', received_at)"
        );
        assert!(params.is_empty());
    }

    #[test]
    fn test_fragment_with_pg_crypt() {
        // crypt($1, gen_salt('bf')) with offset=1 -> crypt($1, gen_salt('bf'))
        let (sql, params) = build_select_sql_from_parts(
            "users",
            &[],
            &[],
            &[],
            &[],
            -1,
            -1,
            &[],
            &[],
            &[],
            &[],
            &["AND password_hash = crypt($1, gen_salt('bf'))".into()],
            &["secret123".into()],
        );
        assert_eq!(
            sql,
            "SELECT * FROM \"users\" AND password_hash = crypt($1, gen_salt('bf'))"
        );
        assert_eq!(params, vec!["secret123"]);
    }

    #[test]
    fn test_fragment_with_jsonb() {
        // metadata @> $1::jsonb after 1 WHERE param
        let (sql, params) = build_select_sql_from_parts(
            "events",
            &[],
            &["project_id =".into()],
            &["abc".into()],
            &[],
            -1,
            -1,
            &[],
            &[],
            &[],
            &[],
            &["AND metadata @> $1::jsonb".into()],
            &[r#"{"env":"production"}"#.into()],
        );
        assert_eq!(
            sql,
            "SELECT * FROM \"events\" WHERE \"project_id\" = $1 AND metadata @> $2::jsonb"
        );
        assert_eq!(params, vec!["abc", r#"{"env":"production"}"#]);
    }

    #[test]
    fn test_mixed_fragments_and_where() {
        // Full query: WHERE + where_raw + fragment with correct numbering
        let (sql, params) = build_select_sql_from_parts(
            "events",
            &[],
            &[
                "project_id =".into(),
                "RAW:received_at > now() - interval '24 hours'".into(),
            ],
            &["abc".into()],
            &["RAW:date_trunc('hour', received_at)".into()],
            -1,
            -1,
            &[],
            &["RAW:date_trunc('hour', received_at)".into()],
            &[],
            &[],
            &["AND tags @> $1::jsonb".into()],
            &[r#"{"env":"prod"}"#.into()],
        );
        assert_eq!(
            sql,
            "SELECT * FROM \"events\" WHERE \"project_id\" = $1 AND received_at > now() - interval '24 hours' GROUP BY date_trunc('hour', received_at) AND tags @> $2::jsonb ORDER BY date_trunc('hour', received_at)"
        );
        assert_eq!(params, vec!["abc", r#"{"env":"prod"}"#]);
    }

    /// A count and an existence check run the query as `Repo.all` would,
    /// selecting `1` when it names no columns so a GROUP BY still stands.
    #[test]
    fn counts_and_existence_wrap_the_whole_query() {
        let (select, params) = build_select_sql_from_parts(
            "articles",
            &["RAW:1".into()],
            &["views >".into()],
            &["10".into()],
            &[],
            -1,
            -1,
            &[],
            &["author_id".into()],
            &["count(*) >".into()],
            &["1".into()],
            &[],
            &[],
        );
        assert_eq!(
            counted_sql(&select),
            "SELECT COUNT(*) AS count FROM (SELECT 1 FROM \"articles\" WHERE \"views\" > $1 GROUP BY \"author_id\" HAVING count(*) > $2) AS counted"
        );
        assert_eq!(
            exists_sql(&select),
            "SELECT EXISTS(SELECT 1 FROM \"articles\" WHERE \"views\" > $1 GROUP BY \"author_id\" HAVING count(*) > $2) AS exists"
        );
        assert_eq!(params, vec!["10", "1"]);
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
        assert_eq!(
            result,
            Some(("email".to_string(), "has already been taken".to_string()))
        );
    }

    #[test]
    fn test_map_constraint_foreign_key_violation() {
        let result = map_constraint_error("23503", "posts_user_id_fkey", "posts", "");
        assert_eq!(
            result,
            Some(("user_id".to_string(), "does not exist".to_string()))
        );
    }

    #[test]
    fn test_map_constraint_not_null_violation() {
        let result = map_constraint_error("23502", "", "", "name");
        assert_eq!(
            result,
            Some(("name".to_string(), "can't be blank".to_string()))
        );
    }

    #[test]
    fn test_map_constraint_unknown_sqlstate() {
        let result = map_constraint_error("42601", "", "", "");
        assert_eq!(result, None);
    }

    // ── Preload unit tests (Phase 100) ──────────────────────────────────

    #[test]
    fn test_parse_relationship_meta() {
        let meta = vec![
            "has_many:posts:Post:user_id:posts".to_string(),
            "has_one:profile:Profile:user_id:profiles".to_string(),
            "belongs_to:user:User:user_id:users".to_string(),
        ];
        let map = parse_relationship_meta(&meta);
        assert_eq!(map.len(), 3);
        let posts = map.get("posts").unwrap();
        assert_eq!(posts.kind, "has_many");
        assert_eq!(posts.fk, "user_id");
        assert_eq!(posts.target_table, "posts");
        let profile = map.get("profile").unwrap();
        assert_eq!(profile.kind, "has_one");
        let user = map.get("user").unwrap();
        assert_eq!(user.kind, "belongs_to");
        assert_eq!(user.fk, "user_id");
        assert_eq!(user.target_table, "users");
        assert_eq!(user.key, "id", "a key left out is id");
        let keyed = parse_relationship_meta(&[
            "belongs_to:owner:Account:owner_id:accounts:uuid".to_string()
        ]);
        assert_eq!(keyed.get("owner").unwrap().key, "uuid");
    }

    #[test]
    fn test_build_preload_sql_basic() {
        let ids = vec!["1".to_string(), "2".to_string(), "3".to_string()];
        let (sql, params) = build_preload_sql("posts", "user_id", &ids);
        assert_eq!(
            sql,
            "SELECT * FROM \"posts\" WHERE \"user_id\" IN ($1, $2, $3)"
        );
        assert_eq!(params, vec!["1", "2", "3"]);
    }

    #[test]
    fn test_build_preload_sql_single_id() {
        let ids = vec!["42".to_string()];
        let (sql, params) = build_preload_sql("users", "id", &ids);
        assert_eq!(sql, "SELECT * FROM \"users\" WHERE \"id\" IN ($1)");
        assert_eq!(params, vec!["42"]);
    }

    // ── RAW: prefix tests (Phase 103) ──────────────────────────────────

    #[test]
    fn test_select_with_raw_expressions() {
        let (sql, params) = build_select_sql_from_parts(
            "users",
            &["RAW:count(*)::text AS count".into(), "level".into()],
            &[],
            &[],
            &[],
            -1,
            -1,
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
        );
        assert_eq!(
            sql,
            "SELECT count(*)::text AS count, \"level\" FROM \"users\""
        );
        assert!(params.is_empty());
    }

    #[test]
    fn test_select_with_all_raw() {
        let (sql, _) = build_select_sql_from_parts(
            "sessions",
            &["RAW:count(*)".into(), "RAW:max(created_at)".into()],
            &[],
            &[],
            &[],
            -1,
            -1,
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
        );
        assert_eq!(sql, "SELECT count(*), max(created_at) FROM \"sessions\"");
    }

    #[test]
    fn test_where_raw_no_params() {
        let (sql, params) = build_select_sql_from_parts(
            "sessions",
            &[],
            &["RAW:expires_at > now()".into()],
            &[],
            &[],
            -1,
            -1,
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
        );
        assert_eq!(sql, "SELECT * FROM \"sessions\" WHERE expires_at > now()");
        assert!(params.is_empty());
    }

    #[test]
    fn test_where_raw_with_params() {
        let (sql, params) = build_select_sql_from_parts(
            "users",
            &[],
            &["RAW:status IN (?, ?)".into()],
            &["active".into(), "pending".into()],
            &[],
            -1,
            -1,
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
        );
        assert_eq!(sql, "SELECT * FROM \"users\" WHERE status IN ($1, $2)");
        assert_eq!(params, vec!["active", "pending"]);
    }

    #[test]
    fn test_where_raw_mixed_with_normal() {
        let (sql, params) = build_select_sql_from_parts(
            "users",
            &[],
            &[
                "name =".into(),
                "RAW:expires_at > now()".into(),
                "age >".into(),
            ],
            &["Alice".into(), "21".into()],
            &[],
            -1,
            -1,
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
        );
        assert_eq!(
            sql,
            "SELECT * FROM \"users\" WHERE \"name\" = $1 AND expires_at > now() AND \"age\" > $2"
        );
        assert_eq!(params, vec!["Alice", "21"]);
    }

    #[test]
    fn test_where_raw_with_params_mixed() {
        // Normal where (1 param) + RAW with 2 params + normal where (1 param)
        let (sql, params) = build_select_sql_from_parts(
            "users",
            &[],
            &[
                "org_id =".into(),
                "RAW:role IN (?, ?)".into(),
                "active =".into(),
            ],
            &[
                "org1".into(),
                "admin".into(),
                "editor".into(),
                "true".into(),
            ],
            &[],
            -1,
            -1,
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
        );
        assert_eq!(
            sql,
            "SELECT * FROM \"users\" WHERE \"org_id\" = $1 AND role IN ($2, $3) AND \"active\" = $4"
        );
        assert_eq!(params, vec!["org1", "admin", "editor", "true"]);
    }

    // ── build_where_from_query_parts tests (Phase 103) ──────────────────

    #[test]
    fn test_where_builder_basic_equality() {
        let (sql, params, next) = build_where_from_query_parts(&["id =".into()], &["42".into()], 1);
        assert_eq!(sql, "\"id\" = $1");
        assert_eq!(params, vec!["42"]);
        assert_eq!(next, 2);
    }

    #[test]
    fn test_where_builder_with_offset() {
        let (sql, params, next) = build_where_from_query_parts(
            &["id =".into(), "status !=".into()],
            &["42".into(), "resolved".into()],
            3,
        );
        assert_eq!(sql, "\"id\" = $3 AND \"status\" != $4");
        assert_eq!(params, vec!["42", "resolved"]);
        assert_eq!(next, 5);
    }

    #[test]
    fn test_where_builder_is_null() {
        let (sql, params, next) = build_where_from_query_parts(
            &["deleted_at IS NULL".into(), "id =".into()],
            &["42".into()],
            1,
        );
        assert_eq!(sql, "\"deleted_at\" IS NULL AND \"id\" = $1");
        assert_eq!(params, vec!["42"]);
        assert_eq!(next, 2);
    }

    #[test]
    fn test_where_builder_raw_clause() {
        let (sql, params, next) = build_where_from_query_parts(
            &["RAW:status IN (?, ?)".into()],
            &["active".into(), "pending".into()],
            1,
        );
        assert_eq!(sql, "status IN ($1, $2)");
        assert_eq!(params, vec!["active", "pending"]);
        assert_eq!(next, 3);
    }

    #[test]
    fn test_where_builder_in_clause() {
        let (sql, params, next) = build_where_from_query_parts(
            &["id IN:3".into()],
            &["1".into(), "2".into(), "3".into()],
            1,
        );
        assert_eq!(sql, "\"id\" IN ($1, $2, $3)");
        assert_eq!(params, vec!["1", "2", "3"]);
        assert_eq!(next, 4);
    }

    #[test]
    fn test_where_builder_between() {
        let (sql, params, next) =
            build_where_from_query_parts(&["age BETWEEN".into()], &["1".into(), "9".into()], 3);
        assert_eq!(sql, "\"age\" BETWEEN $3 AND $4");
        assert_eq!(params, vec!["1", "9"]);
        assert_eq!(next, 5);
    }

    // ── Phase 107 Plan 01: JOIN alias support and comprehensive join tests ──

    #[test]
    fn test_select_with_left_join() {
        let (sql, _) = build_select_sql_from_parts(
            "users",
            &[],
            &[],
            &[],
            &[],
            -1,
            -1,
            &["LEFT:profiles:profiles.user_id = users.id".into()],
            &[],
            &[],
            &[],
            &[],
            &[],
        );
        assert_eq!(
            sql,
            "SELECT * FROM \"users\" LEFT JOIN \"profiles\" ON profiles.user_id = users.id"
        );
    }

    #[test]
    fn test_select_with_multi_join() {
        let (sql, _) = build_select_sql_from_parts(
            "issues",
            &[],
            &[],
            &[],
            &[],
            -1,
            -1,
            &[
                "INNER:projects:projects.id = issues.project_id".into(),
                "INNER:organizations:organizations.id = projects.org_id".into(),
            ],
            &[],
            &[],
            &[],
            &[],
            &[],
        );
        assert_eq!(
            sql,
            "SELECT * FROM \"issues\" INNER JOIN \"projects\" ON projects.id = issues.project_id INNER JOIN \"organizations\" ON organizations.id = projects.org_id"
        );
    }

    #[test]
    fn test_select_with_alias_join() {
        let (sql, _) = build_select_sql_from_parts(
            "issues",
            &[],
            &[],
            &[],
            &[],
            -1,
            -1,
            &["ALIAS:INNER:projects:p:p.id = issues.project_id".into()],
            &[],
            &[],
            &[],
            &[],
            &[],
        );
        assert_eq!(
            sql,
            "SELECT * FROM \"issues\" INNER JOIN \"projects\" p ON p.id = issues.project_id"
        );
    }

    #[test]
    fn test_select_with_multi_alias_join() {
        let (sql, _) = build_select_sql_from_parts(
            "alerts",
            &[],
            &[],
            &[],
            &[],
            -1,
            -1,
            &[
                "ALIAS:INNER:alert_rules:r:r.id = alerts.rule_id".into(),
                "ALIAS:INNER:projects:p:p.id = alerts.project_id".into(),
            ],
            &[],
            &[],
            &[],
            &[],
            &[],
        );
        assert_eq!(
            sql,
            "SELECT * FROM \"alerts\" INNER JOIN \"alert_rules\" r ON r.id = alerts.rule_id INNER JOIN \"projects\" p ON p.id = alerts.project_id"
        );
    }

    #[test]
    fn test_select_with_left_alias_join() {
        let (sql, _) = build_select_sql_from_parts(
            "users",
            &[],
            &[],
            &[],
            &[],
            -1,
            -1,
            &["ALIAS:LEFT:profiles:p:p.user_id = users.id".into()],
            &[],
            &[],
            &[],
            &[],
            &[],
        );
        assert_eq!(
            sql,
            "SELECT * FROM \"users\" LEFT JOIN \"profiles\" p ON p.user_id = users.id"
        );
    }

    // ── Phase 108: Aggregate SELECT functions ─────────────────────────────

    #[test]
    fn test_aggregate_select_count() {
        let (sql, params) = build_select_sql_from_parts(
            "issues",
            &["RAW:count(*)".into()],
            &[],
            &[],
            &[],
            -1,
            -1,
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
        );
        assert_eq!(sql, "SELECT count(*) FROM \"issues\"");
        assert!(params.is_empty());
    }

    #[test]
    fn test_aggregate_select_sum() {
        let (sql, params) = build_select_sql_from_parts(
            "orders",
            &["RAW:sum(\"amount\")".into()],
            &[],
            &[],
            &[],
            -1,
            -1,
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
        );
        assert_eq!(sql, "SELECT sum(\"amount\") FROM \"orders\"");
        assert!(params.is_empty());
    }

    #[test]
    fn test_aggregate_select_avg_with_group_by() {
        let (sql, params) = build_select_sql_from_parts(
            "products",
            &["RAW:avg(\"price\")".into()],
            &[],
            &[],
            &[],
            -1,
            -1,
            &[],
            &["category".into()],
            &[],
            &[],
            &[],
            &[],
        );
        assert_eq!(
            sql,
            "SELECT avg(\"price\") FROM \"products\" GROUP BY \"category\""
        );
        assert!(params.is_empty());
    }

    #[test]
    fn test_aggregate_select_min_max() {
        let (sql, params) = build_select_sql_from_parts(
            "events",
            &[
                "RAW:min(\"created_at\")".into(),
                "RAW:max(\"created_at\")".into(),
            ],
            &[],
            &[],
            &[],
            -1,
            -1,
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
        );
        assert_eq!(
            sql,
            "SELECT min(\"created_at\"), max(\"created_at\") FROM \"events\""
        );
        assert!(params.is_empty());
    }

    #[test]
    fn test_aggregate_with_having() {
        let (sql, params) = build_select_sql_from_parts(
            "issues",
            &["RAW:count(*)".into()],
            &[],
            &[],
            &[],
            -1,
            -1,
            &[],
            &["project_id".into()],
            &["count(*) >".into()],
            &["5".into()],
            &[],
            &[],
        );
        assert_eq!(
            sql,
            "SELECT count(*) FROM \"issues\" GROUP BY \"project_id\" HAVING count(*) > $1"
        );
        assert_eq!(params, vec!["5"]);
    }

    // ── Phase 109 Plan 01: Upsert and subquery WHERE tests ──────────────

    #[test]
    fn test_upsert_sql() {
        let sql = crate::db::orm::build_upsert_sql_pure(
            "issues",
            &["project_id".into(), "fingerprint".into(), "title".into()],
            &["project_id".into(), "fingerprint".into()],
            &["title".into()],
            &["*".into()],
        );
        assert_eq!(
            sql,
            "INSERT INTO \"issues\" (\"project_id\", \"fingerprint\", \"title\") VALUES ($1, $2, $3) ON CONFLICT (\"project_id\", \"fingerprint\") DO UPDATE SET \"title\" = EXCLUDED.\"title\" RETURNING *"
        );
    }

    #[test]
    fn test_upsert_sql_multi_update() {
        let sql = crate::db::orm::build_upsert_sql_pure(
            "users",
            &["email".into(), "name".into(), "role".into()],
            &["email".into()],
            &["name".into(), "role".into()],
            &["*".into()],
        );
        assert_eq!(
            sql,
            "INSERT INTO \"users\" (\"email\", \"name\", \"role\") VALUES ($1, $2, $3) ON CONFLICT (\"email\") DO UPDATE SET \"name\" = EXCLUDED.\"name\", \"role\" = EXCLUDED.\"role\" RETURNING *"
        );
    }

    /// `Repo.all`'s SQL and parameters for the Query `q`.
    fn all_sql(q: *mut u8) -> (String, Vec<String>) {
        select_sql(unsafe { &query_parts(q) }, None, 1)
    }

    fn atom(name: &str) -> *mut u8 {
        mesh_str(name) as *mut u8
    }

    /// An empty IN list matches no row, an empty NOT IN list every row, and
    /// an OR of no fields no row (each was `IN ()` or `()`, invalid SQL).
    #[test]
    fn empty_lists_match_nothing_or_everything() {
        use crate::db::query::*;
        crate::gc::mesh_rt_init();
        let none = || string_list::<&str>(&[]);
        let q = mesh_query_where_in(mesh_query_from(atom("t")), atom("a"), none());
        let q = mesh_query_where_not_in(q, atom("b"), none());
        let q = mesh_query_where_or(q, none(), none());
        let q = mesh_query_where(q, atom("c"), atom("x"));
        let (sql, params) = all_sql(q);
        assert_eq!(
            sql,
            "SELECT * FROM \"t\" WHERE FALSE AND TRUE AND FALSE AND \"c\" = $1"
        );
        assert_eq!(params, ["x"]);
    }

    /// A subquery is the whole query `Repo.all` would run, whatever its
    /// clauses, numbered where it falls in the outer one.
    #[test]
    fn a_subquery_keeps_every_clause_it_has() {
        use crate::db::query::*;
        crate::gc::mesh_rt_init();
        let expr = crate::db::expr::mesh_expr_gt(
            crate::db::expr::mesh_expr_column(atom("views")),
            crate::db::expr::mesh_expr_value(atom("10")),
        );
        let sub = mesh_query_from(atom("articles"));
        let sub = mesh_query_select(sub, string_list(&["author_id"]));
        let sub = mesh_query_where_in(sub, atom("status"), string_list(&["live", "pinned"]));
        let sub = mesh_query_where_not_in(sub, atom("kind"), string_list(&["draft"]));
        let sub = mesh_query_where_between(sub, atom("year"), atom("2020"), atom("2026"));
        let sub = mesh_query_where_or(sub, string_list(&["a", "b"]), string_list(&["1", "2"]));
        let sub = mesh_query_where_expr(sub, expr);
        let sub = mesh_query_join(
            sub,
            atom("inner"),
            atom("writers"),
            atom("writers.handle = articles.author_id"),
        );
        let sub = mesh_query_limit(sub, 5);
        let inner = mesh_query_where(mesh_query_from(atom("bans")), atom("active"), atom("t"));
        let inner = mesh_query_select(inner, string_list(&["handle"]));
        let sub = mesh_query_where_sub(sub, atom("author_id"), inner);

        let outer = mesh_query_where(mesh_query_from(atom("writers")), atom("name"), atom("Ada"));
        let outer = mesh_query_where_sub(outer, atom("handle"), sub);
        let outer = mesh_query_where_op(outer, atom("score"), atom("gt"), atom("3"));
        let (sql, params) = all_sql(outer);
        assert_eq!(
            sql,
            "SELECT * FROM \"writers\" WHERE \"name\" = $1 AND \"handle\" IN (SELECT \"author_id\" \
             FROM \"articles\" INNER JOIN \"writers\" ON writers.handle = articles.author_id \
             WHERE \"status\" IN ($2, $3) AND \"kind\" NOT IN ($4) AND \"year\" BETWEEN $5 AND $6 \
             AND (\"a\" = $7 OR \"b\" = $8) AND (\"views\" > $9) AND \"author_id\" IN (SELECT \
             \"handle\" FROM \"bans\" WHERE \"active\" = $10) LIMIT 5) AND \"score\" > $11"
        );
        assert_eq!(
            params,
            ["Ada", "live", "pinned", "draft", "2020", "2026", "1", "2", "10", "t", "3"]
        );
    }

    #[test]
    fn test_subquery_where_clause() {
        let (sql, params) = build_select_sql_from_parts(
            "issues",
            &[],
            &[
                "status =".into(),
                "RAW:\"project_id\" IN (SELECT \"id\" FROM \"projects\" WHERE \"org_id\" = ?)"
                    .into(),
            ],
            &["open".into(), "org-123".into()],
            &[],
            -1,
            -1,
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
        );
        assert_eq!(
            sql,
            "SELECT * FROM \"issues\" WHERE \"status\" = $1 AND \"project_id\" IN (SELECT \"id\" FROM \"projects\" WHERE \"org_id\" = $2)"
        );
        assert_eq!(params, vec!["open", "org-123"]);
    }

    #[test]
    fn test_select_expr_sql_renumbers_select_params_before_where_params() {
        let (sql, params) = build_select_sql_from_parts(
            "issues",
            &[
                expr_entry(&SqlExpr::Alias {
                    expr: Box::new(SqlExpr::Coalesce(vec![
                        SqlExpr::Column("nickname".into()),
                        SqlExpr::Value("fallback".into()),
                    ])),
                    alias: "nick".into(),
                }),
                expr_entry(&SqlExpr::Alias {
                    expr: Box::new(SqlExpr::Binary {
                        op: "+".into(),
                        lhs: column("event_count"),
                        rhs: value("2"),
                    }),
                    alias: "next_count".into(),
                }),
            ],
            &["id =".into()],
            &["issue-123".into()],
            &[],
            -1,
            -1,
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
        );

        assert_eq!(
            sql,
            "SELECT COALESCE(\"nickname\", $1) AS \"nick\", (\"event_count\" + $2) AS \"next_count\" FROM \"issues\" WHERE \"id\" = $3"
        );
        assert_eq!(params, vec!["fallback", "2", "issue-123"]);
    }

    #[test]
    fn test_update_where_expr_sql_renumbers_set_and_where_params() {
        let (sql, params) = build_update_where_expr_sql_pure(
            "issues",
            &["event_count".into(), "status".into(), "last_seen".into()],
            &[
                SqlExpr::Binary {
                    op: "+".into(),
                    lhs: Box::new(SqlExpr::Column("event_count".into())),
                    rhs: Box::new(SqlExpr::Value("1".into())),
                },
                SqlExpr::Case {
                    branches: vec![(
                        SqlExpr::Binary {
                            op: "=".into(),
                            lhs: Box::new(SqlExpr::Column("status".into())),
                            rhs: Box::new(SqlExpr::Value("resolved".into())),
                        },
                        SqlExpr::Value("unresolved".into()),
                    )],
                    else_expr: Box::new(SqlExpr::Column("status".into())),
                },
                SqlExpr::Call {
                    name: "now".into(),
                    args: vec![],
                },
            ],
            &filtered(&["RAW:id = ?::uuid".into()], &["issue-123".into()]),
        )
        .expect("update_where_expr SQL should build");

        assert_eq!(
            sql,
            "UPDATE \"issues\" SET \"event_count\" = (\"event_count\" + $1), \"status\" = CASE WHEN (\"status\" = $2) THEN $3 ELSE \"status\" END, \"last_seen\" = now() WHERE id = $4::uuid RETURNING *"
        );
        assert_eq!(params, vec!["1", "resolved", "unresolved", "issue-123"]);
    }

    #[test]
    fn test_insert_or_update_expr_sql_preserves_insert_then_update_param_order() {
        let (sql, params) = build_insert_or_update_expr_sql_pure(
            "issues",
            &[
                "project_id".into(),
                "fingerprint".into(),
                "title".into(),
                "level".into(),
                "event_count".into(),
            ],
            &[
                "project-1".into(),
                "fp-1".into(),
                "Boom".into(),
                "error".into(),
                "1".into(),
            ],
            &["project_id".into(), "fingerprint".into()],
            &["event_count".into(), "status".into(), "last_seen".into()],
            &[
                SqlExpr::Binary {
                    op: "+".into(),
                    lhs: Box::new(SqlExpr::Column("event_count".into())),
                    rhs: Box::new(SqlExpr::Value("1".into())),
                },
                SqlExpr::Case {
                    branches: vec![(
                        SqlExpr::Binary {
                            op: "=".into(),
                            lhs: Box::new(SqlExpr::Column("status".into())),
                            rhs: Box::new(SqlExpr::Value("resolved".into())),
                        },
                        SqlExpr::Value("unresolved".into()),
                    )],
                    else_expr: Box::new(SqlExpr::Column("status".into())),
                },
                SqlExpr::Call {
                    name: "now".into(),
                    args: vec![],
                },
            ],
        )
        .expect("insert_or_update_expr SQL should build");

        assert_eq!(
            sql,
            "INSERT INTO \"issues\" (\"project_id\", \"fingerprint\", \"title\", \"level\", \"event_count\") VALUES ($1, $2, $3, $4, $5) ON CONFLICT (\"project_id\", \"fingerprint\") DO UPDATE SET \"event_count\" = (\"issues\".\"event_count\" + $6), \"status\" = CASE WHEN (\"issues\".\"status\" = $7) THEN $8 ELSE \"issues\".\"status\" END, \"last_seen\" = now() RETURNING *",
            "bare columns name the existing row: unqualified, PostgreSQL finds them ambiguous with EXCLUDED's"
        );
        assert_eq!(
            params,
            vec![
                "project-1",
                "fp-1",
                "Boom",
                "error",
                "1",
                "1",
                "resolved",
                "unresolved",
            ]
        );
    }

    #[test]
    fn test_where_expr_sql_renumbers_after_select_params() {
        let crypt = |salt: SqlExpr| SqlExpr::Call {
            name: "crypt".into(),
            args: vec![SqlExpr::Value("secret".into()), salt],
        };
        let (sql, params) = build_select_sql_from_parts(
            "users",
            &[expr_entry(&SqlExpr::Alias {
                expr: Box::new(crypt(SqlExpr::Column("password_hash".into()))),
                alias: "candidate".into(),
            })],
            &[expr_entry(&SqlExpr::Binary {
                op: "=".into(),
                lhs: column("password_hash"),
                rhs: Box::new(crypt(SqlExpr::Column("password_hash".into()))),
            })],
            &[],
            &[],
            -1,
            -1,
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
        );

        assert_eq!(
            sql,
            "SELECT crypt($1, \"password_hash\") AS \"candidate\" FROM \"users\" WHERE (\"password_hash\" = crypt($2, \"password_hash\"))"
        );
        assert_eq!(params, vec!["secret", "secret"]);
    }

    #[test]
    fn test_insert_expr_sql_preserves_expr_param_order() {
        let (sql, params) = build_insert_expr_sql_pure(
            "users",
            &[
                "email".into(),
                "password_hash".into(),
                "display_name".into(),
            ],
            &[
                SqlExpr::Value("alice@example.com".into()),
                SqlExpr::Call {
                    name: "crypt".into(),
                    args: vec![
                        SqlExpr::Value("secret".into()),
                        SqlExpr::Call {
                            name: "gen_salt".into(),
                            args: vec![SqlExpr::Value("bf".into()), SqlExpr::Value("12".into())],
                        },
                    ],
                },
                SqlExpr::Value("Alice".into()),
            ],
        )
        .expect("insert_expr SQL should build");

        assert_eq!(
            sql,
            "INSERT INTO \"users\" (\"email\", \"password_hash\", \"display_name\") VALUES ($1, crypt($2, gen_salt($3, $4)), $5) RETURNING *"
        );
        assert_eq!(
            params,
            vec!["alice@example.com", "secret", "bf", "12", "Alice"]
        );
    }

    /// A valid changeset with nothing to write comes back as the Err it is
    /// with a `_base` error saying why: it came back with no error at all.
    #[test]
    fn a_changeset_without_changes_says_so() {
        use crate::db::changeset::*;
        crate::gc::mesh_rt_init();
        let empty = || crate::collections::map::mesh_map_new_typed(1);
        let cs = mesh_changeset_cast(empty(), empty(), string_list::<&str>(&[]));
        let table = mesh_str("t") as *mut u8;
        for result in [
            mesh_repo_insert_changeset(0, table, cs),
            mesh_repo_update_changeset(0, table, mesh_str("1") as *mut u8, cs),
        ] {
            let r = unsafe { &*(result as *const MeshResult) };
            assert_eq!(r.tag, 1);
            let error = mesh_changeset_get_error(r.value, mesh_str("_base") as *mut u8);
            assert_eq!(unsafe { text_of(error) }, "has no changes");
            assert_eq!(mesh_changeset_valid(r.value) as i64, 0);
        }
    }

    // ── Against PostgreSQL ────────────────────────────────────────────

    /// A one-connection pool on MESH_TEST_DATABASE_URL whose session works
    /// in a fresh schema of its own, set up by `setup`'s statements.
    fn test_pool(schema: &str, setup: &[&str]) -> u64 {
        crate::gc::mesh_rt_init();
        let url = std::env::var("MESH_TEST_DATABASE_URL").expect("MESH_TEST_DATABASE_URL is set");
        let open = crate::db::pool::mesh_pool_open(mesh_str(&url), 1, 1, 5000);
        let pool = unsafe { unbox_u64_payload(ok(open)) };
        let schema_sql = [
            format!("DROP SCHEMA IF EXISTS {schema} CASCADE"),
            format!("CREATE SCHEMA {schema}"),
            format!("SET search_path TO {schema}"),
        ];
        for sql in schema_sql
            .iter()
            .map(String::as_str)
            .chain(setup.iter().copied())
        {
            ok(mesh_pool_execute(pool, mesh_str(sql), mesh_list_new()));
        }
        pool
    }

    /// The value of an `Ok` result.
    fn ok(result: *mut u8) -> *mut u8 {
        let r = unsafe { &*(result as *const MeshResult) };
        assert_eq!(r.tag, 0, "{}", unsafe { text_of(r.value) });
        r.value
    }

    /// `column` of each row in a list of rows.
    fn column_of(rows: *mut u8, column: &str) -> Vec<String> {
        (0..mesh_list_length(rows))
            .map(|i| {
                let row = mesh_list_get(rows, i) as *mut u8;
                unsafe { text_of(mesh_map_get(row, mesh_str(column) as u64) as *mut u8) }
                    .to_string()
            })
            .collect()
    }

    /// `column` of a row.
    fn field(row: *mut u8, column: &str) -> String {
        unsafe { text_of(mesh_map_get(row, mesh_str(column) as u64) as *mut u8) }.to_string()
    }

    /// A primary key column is one name, whatever it holds: `Repo.get` read
    /// a dot in it as a schema qualifier, and `update` and `delete` took
    /// what followed a space for an operator.
    #[test]
    #[ignore = "requires MESH_TEST_DATABASE_URL (the coverage run starts a database)"]
    fn a_primary_key_with_a_space_or_dot_in_its_name_finds_its_row() {
        let pool = test_pool(
            "mesh_repo_unit_keys",
            &[
                "CREATE TABLE spaced (\"row id\" TEXT PRIMARY KEY, note TEXT)",
                "CREATE TABLE dotted (\"row.id\" TEXT PRIMARY KEY, note TEXT)",
                "INSERT INTO spaced VALUES ('a', 'x')",
                "INSERT INTO dotted VALUES ('a', 'x')",
            ],
        );
        for table in ["spaced", "dotted"] {
            let table = mesh_str(table) as *mut u8;
            let id = mesh_str("a") as *mut u8;
            assert_eq!(field(ok(mesh_repo_get(pool, table, id)), "note"), "x");
            let fields = mesh_map_put(
                crate::collections::map::mesh_map_new_typed(1),
                mesh_str("note") as u64,
                mesh_str("y") as u64,
            );
            assert_eq!(
                field(ok(mesh_repo_update(pool, table, id, fields)), "note"),
                "y"
            );
            assert_eq!(field(ok(mesh_repo_delete(pool, table, id)), "note"), "y");
        }
        crate::db::pool::mesh_pool_close(pool);
    }

    /// A NULL foreign key (read as "") matches no row: it was sent among
    /// the keys to look up, and PostgreSQL refused "" as an integer.
    #[test]
    #[ignore = "requires MESH_TEST_DATABASE_URL (the coverage run starts a database)"]
    fn a_null_foreign_key_preloads_nothing() {
        let pool = test_pool(
            "mesh_repo_unit_preload",
            &[
                "CREATE TABLE authors (id INT PRIMARY KEY, name TEXT)",
                "CREATE TABLE posts (id INT PRIMARY KEY, author_id INT REFERENCES authors)",
                "INSERT INTO authors VALUES (1, 'Ada')",
                "INSERT INTO posts VALUES (1, 1), (2, NULL)",
            ],
        );
        use crate::db::query::{mesh_query_from, mesh_query_order_by};
        let posts = mesh_query_order_by(mesh_query_from(atom("posts")), atom("id"), atom("asc"));
        let rows = ok(mesh_repo_all(pool, posts));
        let meta = string_list(&["belongs_to:author:Author:author_id:authors:id"]);
        let preloaded = ok(mesh_repo_preload(
            pool,
            rows,
            string_list(&["author"]),
            meta,
        ));
        assert_eq!(
            column_of(preloaded, "author"),
            [r#"{"id":"1","name":"Ada"}"#, "null"]
        );

        // A has_many keyed on a NULL column finds nothing for that row.
        let meta = string_list(&["has_many:peers:Post:author_id:posts:author_id"]);
        let peers = ok(mesh_repo_preload(pool, rows, string_list(&["peers"]), meta));
        assert_eq!(
            column_of(peers, "peers"),
            [r#"[{"author_id":"1","id":"1"}]"#, "[]"]
        );
        crate::db::pool::mesh_pool_close(pool);
    }
}
