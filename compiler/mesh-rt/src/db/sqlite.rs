//! SQLite C FFI wrapper functions for the Mesh runtime.
//!
//! Provides six extern "C" functions that Mesh programs call to interact
//! with SQLite databases:
//! - `mesh_sqlite_open`: Open a database connection
//! - `mesh_sqlite_close`: Close a connection
//! - `mesh_sqlite_execute`: Execute a write query (INSERT/UPDATE/DELETE/CREATE)
//! - `mesh_sqlite_query`: Execute a read query (SELECT), returns rows
//! - `mesh_sqlite_execute_values`: Execute with typed `DbValue` parameters
//! - `mesh_sqlite_query_values`: Query typed `DbValue` rows
//!
//! Connection handles are opaque u64 values (Box::into_raw as u64) for GC
//! safety. The GC never traces integer values, so the connection won't be
//! corrupted by garbage collection.

use libsqlite3_sys::*;
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};

use crate::bytes::mesh_bytes_new;
use crate::collections::list::mesh_list_from_array;
use crate::collections::map::mesh_map_from_string_entries;
use crate::db::pg::{
    alloc_db_value, db_values, text_values, BindValue, MeshDbValue, DB_VALUE_BINARY, DB_VALUE_NULL,
    DB_VALUE_TEXT, MAX_DB_VALUE_BYTES,
};
use crate::io::{alloc_result, box_scalar, err_result};
use crate::string::text_of;
use crate::string::{mesh_str, MeshString};

// ponytail: fixed safety caps; make these connection options only if real workloads need more.
const MAX_SQLITE_RESULT_BYTES: usize = 64 * 1024 * 1024;
const MAX_SQLITE_VALUES: usize = 32_766;
const MAX_SQLITE_ROWS: usize = 100_000;

/// Wrapper around a raw SQLite database pointer.
struct SqliteConn {
    db: *mut sqlite3,
}

/// RAII guard ensuring sqlite3_finalize is always called on a prepared
/// statement, even when an error causes an early return.
struct StmtGuard {
    stmt: *mut sqlite3_stmt,
}

impl Drop for StmtGuard {
    fn drop(&mut self) {
        if !self.stmt.is_null() {
            unsafe {
                sqlite3_finalize(self.stmt);
            }
        }
    }
}

/// SQLITE_TRANSIENT tells SQLite to copy bound parameter data immediately.
/// It is defined as ((void(*)(void*))-1) in the C API, which is -1 cast to
/// a destructor function pointer.
const SQLITE_TRANSIENT_VALUE: isize = -1;

unsafe fn sqlite_transient() -> Option<unsafe extern "C" fn(*mut std::ffi::c_void)> {
    std::mem::transmute::<isize, Option<unsafe extern "C" fn(*mut std::ffi::c_void)>>(
        SQLITE_TRANSIENT_VALUE,
    )
}

#[cfg(test)]
unsafe fn unbox_u64_payload(ptr: *mut u8) -> u64 {
    *(ptr as *const u64)
}

#[cfg(test)]
unsafe fn unbox_i64_payload(ptr: *mut u8) -> i64 {
    *(ptr as *const i64)
}

/// Create an error MeshResult from a sqlite3 error message.
unsafe fn sqlite_err_result(db: *mut sqlite3) -> *mut u8 {
    err_result(&sqlite_err_string(db))
}

/// The connection's last error. `sqlite3_errmsg` always returns text, "out
/// of memory" when it cannot say more.
unsafe fn sqlite_err_string(db: *mut sqlite3) -> String {
    CStr::from_ptr(sqlite3_errmsg(db))
        .to_string_lossy()
        .into_owned()
}

unsafe fn prepare_statement(db: *mut sqlite3, sql: &str) -> Result<StmtGuard, String> {
    let sql = CString::new(sql).map_err(|_| "SQL contains null byte".to_string())?;
    let mut stmt = std::ptr::null_mut();
    let rc = sqlite3_prepare_v2(db, sql.as_ptr(), -1, &mut stmt, std::ptr::null_mut());
    if rc != SQLITE_OK {
        Err(sqlite_err_string(db))
    } else if stmt.is_null() {
        Err("SQLite statement is empty".to_string())
    } else {
        Ok(StmtGuard { stmt })
    }
}

/// Bind `values` to the statement's parameters, one for each.
unsafe fn bind_values(
    db: *mut sqlite3,
    stmt: *mut sqlite3_stmt,
    values: Vec<BindValue<'_>>,
) -> Result<(), String> {
    let expected = sqlite3_bind_parameter_count(stmt) as usize;
    if values.len() != expected {
        return Err(format!(
            "SQLite statement expects {expected} parameters but received {}",
            values.len()
        ));
    }
    for (index, value) in values.into_iter().enumerate() {
        let sqlite_index = (index + 1) as c_int;
        let rc = match value {
            BindValue::Text(bytes) => sqlite3_bind_text(
                stmt,
                sqlite_index,
                bytes.as_ptr() as *const c_char,
                bytes.len() as c_int,
                sqlite_transient(),
            ),
            BindValue::Binary([]) => sqlite3_bind_zeroblob(stmt, sqlite_index, 0),
            BindValue::Binary(bytes) => sqlite3_bind_blob(
                stmt,
                sqlite_index,
                bytes.as_ptr() as *const std::ffi::c_void,
                bytes.len() as c_int,
                sqlite_transient(),
            ),
            BindValue::Null => sqlite3_bind_null(stmt, sqlite_index),
        };
        if rc != SQLITE_OK {
            return Err(sqlite_err_string(db));
        }
    }
    Ok(())
}

/// How a statement's parameters and rows are typed: `List<String>` and
/// `Map<String, String>` rows (a NULL reads as ""), or `DbValue`s.
#[derive(Clone, Copy, PartialEq)]
enum Values {
    Text,
    Typed,
}

/// `sql` prepared on the connection, with `params` bound.
unsafe fn prepared(
    conn_handle: u64,
    sql: *const MeshString,
    params: *mut u8,
    values: Values,
) -> Result<(*mut sqlite3, StmtGuard), String> {
    let db = (*(conn_handle as *const SqliteConn)).db;
    let guard = prepare_statement(db, text_of(sql))?;
    let params = match values {
        Values::Text => text_values(params, MAX_SQLITE_VALUES, "SQLite")?,
        Values::Typed => db_values(params, MAX_SQLITE_VALUES, "SQLite")?,
    };
    bind_values(db, guard.stmt, params)?;
    Ok((db, guard))
}

/// Run a statement for its effect: `Ok(rows changed)`.
unsafe fn execute(
    conn_handle: u64,
    sql: *const MeshString,
    params: *mut u8,
    values: Values,
) -> *mut u8 {
    let changes = prepared(conn_handle, sql, params, values).and_then(|(db, guard)| {
        match sqlite3_step(guard.stmt) {
            SQLITE_DONE | SQLITE_ROW => Ok(sqlite3_changes(db) as i64),
            _ => Err(sqlite_err_string(db)),
        }
    });
    match changes {
        Ok(changes) => alloc_result(0, box_scalar(changes)) as *mut u8,
        Err(error) => err_result(&error),
    }
}

/// Run a query: `Ok(rows)`, each a map from column name to value, within
/// the row and byte limits. A later column of the same name wins.
unsafe fn query(
    conn_handle: u64,
    sql: *const MeshString,
    params: *mut u8,
    values: Values,
) -> *mut u8 {
    let rows = prepared(conn_handle, sql, params, values)
        .and_then(|(db, guard)| read_rows(db, guard.stmt, values));
    match rows {
        Ok(rows) => {
            alloc_result(0, mesh_list_from_array(rows.as_ptr(), rows.len() as i64)) as *mut u8
        }
        Err(error) => err_result(&error),
    }
}

unsafe fn read_rows(
    db: *mut sqlite3,
    stmt: *mut sqlite3_stmt,
    values: Values,
) -> Result<Vec<u64>, String> {
    let column_count = sqlite3_column_count(stmt) as usize;
    let column_names: Vec<String> = (0..column_count)
        .map(|column| {
            let name = sqlite3_column_name(stmt, column as c_int);
            // Null only when SQLite runs out of memory.
            if name.is_null() {
                format!("column{column}")
            } else {
                CStr::from_ptr(name).to_string_lossy().into_owned()
            }
        })
        .collect();
    let row_base_bytes = column_names
        .iter()
        .try_fold(40_usize, |total, name| total.checked_add(name.len()))
        .and_then(|total| column_count.checked_mul(96)?.checked_add(total))
        .ok_or_else(|| "SQLite result size overflow".to_string())?;

    let mut rows = Vec::new();
    let mut result_bytes = 64_usize;
    loop {
        match sqlite3_step(stmt) {
            SQLITE_DONE => return Ok(rows),
            SQLITE_ROW => {}
            _ => return Err(sqlite_err_string(db)),
        }
        if rows.len() == MAX_SQLITE_ROWS {
            return Err(format!("SQLite result exceeds {MAX_SQLITE_ROWS} row limit"));
        }
        result_bytes = add_result_bytes(result_bytes, row_base_bytes)?;

        let mut entries = Vec::<[u64; 2]>::with_capacity(column_count);
        let mut indexes = HashMap::<&str, usize>::with_capacity(column_count);
        for (column, name) in column_names.iter().enumerate() {
            let column = column as c_int;
            let value = match values {
                Values::Typed => typed_column_value(stmt, column, &mut result_bytes)? as u64,
                Values::Text if sqlite3_column_type(stmt, column) == SQLITE_NULL => {
                    mesh_str("") as u64
                }
                Values::Text => column_text(stmt, column, &mut result_bytes)? as u64,
            };
            if let Some(index) = indexes.get(name.as_str()).copied() {
                entries[index][1] = value;
            } else {
                indexes.insert(name.as_str(), entries.len());
                entries.push([mesh_str(name) as u64, value]);
            }
        }
        rows.push(mesh_map_from_string_entries(&entries) as u64);
    }
}

fn add_result_bytes(total: usize, bytes: usize) -> Result<usize, String> {
    total
        .checked_add(bytes)
        .filter(|total| *total <= MAX_SQLITE_RESULT_BYTES)
        .ok_or_else(|| format!("SQLite result exceeds {MAX_SQLITE_RESULT_BYTES} byte limit"))
}

unsafe fn typed_column_value(
    stmt: *mut sqlite3_stmt,
    column: c_int,
    result_bytes: &mut usize,
) -> Result<*mut MeshDbValue, String> {
    match sqlite3_column_type(stmt, column) {
        SQLITE_NULL => Ok(alloc_db_value(DB_VALUE_NULL, std::ptr::null_mut())),
        SQLITE_BLOB => {
            let len = column_len(stmt, column, result_bytes)?;
            let bytes = sqlite3_column_blob(stmt, column) as *const u8;
            if bytes.is_null() && len != 0 {
                return Err(format!("failed to read SQLite BLOB column {column}"));
            }
            let payload = mesh_bytes_new(bytes, len as u64) as *mut u8;
            Ok(alloc_db_value(DB_VALUE_BINARY, payload))
        }
        _ => Ok(alloc_db_value(
            DB_VALUE_TEXT,
            column_text(stmt, column, result_bytes)? as *mut u8,
        )),
    }
}

/// A column's length in bytes, counted against the result's limit.
unsafe fn column_len(
    stmt: *mut sqlite3_stmt,
    column: c_int,
    result_bytes: &mut usize,
) -> Result<usize, String> {
    let len = sqlite3_column_bytes(stmt, column) as usize;
    if len > MAX_DB_VALUE_BYTES {
        return Err(format!(
            "SQLite column {column} exceeds {MAX_DB_VALUE_BYTES} byte limit"
        ));
    }
    *result_bytes = add_result_bytes(*result_bytes, len)?;
    Ok(len)
}

/// A column as text, all its bytes (a NUL inside it too); invalid UTF-8
/// becomes U+FFFD.
unsafe fn column_text(
    stmt: *mut sqlite3_stmt,
    column: c_int,
    result_bytes: &mut usize,
) -> Result<*mut MeshString, String> {
    let bytes = sqlite3_column_text(stmt, column);
    let len = column_len(stmt, column, result_bytes)?;
    if bytes.is_null() {
        return Err(format!("failed to read SQLite text column {column}"));
    }
    let text = String::from_utf8_lossy(std::slice::from_raw_parts(bytes, len));
    if text.len() > MAX_DB_VALUE_BYTES {
        return Err(format!(
            "SQLite column {column} exceeds {MAX_DB_VALUE_BYTES} byte limit"
        ));
    }
    if text.len() > len {
        *result_bytes = add_result_bytes(*result_bytes, text.len() - len)?;
    }
    Ok(mesh_str(&text))
}

/// Open a SQLite database.
///
/// # Signature
///
/// `mesh_sqlite_open(path: *const MeshString) -> *mut u8 (MeshResult<u64, String>)`
///
/// Returns MeshResult with tag 0 (Ok) containing the connection handle as
/// a u64, or tag 1 (Err) containing an error message string.
#[no_mangle]
pub extern "C" fn mesh_sqlite_open(path: *const MeshString) -> *mut u8 {
    unsafe {
        let path_str = text_of(path);
        let c_path = match CString::new(path_str) {
            Ok(c) => c,
            Err(_) => return err_result("path contains null byte"),
        };

        let mut db: *mut sqlite3 = std::ptr::null_mut();
        let rc = sqlite3_open_v2(
            c_path.as_ptr(),
            &mut db,
            SQLITE_OPEN_READWRITE | SQLITE_OPEN_CREATE,
            std::ptr::null(),
        );

        if rc != SQLITE_OK {
            let result = sqlite_err_result(db);
            if !db.is_null() {
                sqlite3_close(db);
            }
            return result;
        }

        let conn = Box::new(SqliteConn { db });
        let handle = Box::into_raw(conn) as u64;
        alloc_result(0, box_scalar(handle)) as *mut u8
    }
}

/// Close a SQLite database connection.
///
/// # Signature
///
/// `mesh_sqlite_close(conn_handle: u64)`
///
/// Recovers the Box<SqliteConn> from the handle, calls sqlite3_close,
/// and lets Box::drop free the Rust memory.
#[no_mangle]
pub extern "C" fn mesh_sqlite_close(conn_handle: u64) {
    unsafe {
        let conn = Box::from_raw(conn_handle as *mut SqliteConn);
        sqlite3_close(conn.db);
        // Box drops, freeing Rust memory
    }
}

/// Execute a write SQL statement (INSERT, UPDATE, DELETE, CREATE TABLE, etc.).
///
/// # Signature
///
/// `mesh_sqlite_execute(conn_handle: u64, sql: *const MeshString, params: *mut u8)
///     -> *mut u8 (MeshResult<Int, String>)`
///
/// Parameters are bound as text via sqlite3_bind_text. Returns the number
/// of rows affected (via sqlite3_changes) on success.
#[no_mangle]
pub extern "C" fn mesh_sqlite_execute(
    conn_handle: u64,
    sql: *const MeshString,
    params: *mut u8,
) -> *mut u8 {
    unsafe { execute(conn_handle, sql, params, Values::Text) }
}

/// Execute a read SQL statement (SELECT) and return rows.
///
/// # Signature
///
/// `mesh_sqlite_query(conn_handle: u64, sql: *const MeshString, params: *mut u8)
///     -> *mut u8 (MeshResult<List<Map<String, String>>, String>)`
///
/// Each row is a Map<String, String> where keys are column names and values
/// are the text representation of column values. NULL columns become empty
/// strings.
#[no_mangle]
pub extern "C" fn mesh_sqlite_query(
    conn_handle: u64,
    sql: *const MeshString,
    params: *mut u8,
) -> *mut u8 {
    unsafe { query(conn_handle, sql, params, Values::Text) }
}

/// Execute a statement with `DbValue` parameters.
#[no_mangle]
pub extern "C" fn mesh_sqlite_execute_values(
    conn_handle: u64,
    sql: *const MeshString,
    params: *mut u8,
) -> *mut u8 {
    unsafe { execute(conn_handle, sql, params, Values::Typed) }
}

/// Query rows as `Map<String, DbValue>`, preserving BLOB and NULL values.
#[no_mangle]
pub extern "C" fn mesh_sqlite_query_values(
    conn_handle: u64,
    sql: *const MeshString,
    params: *mut u8,
) -> *mut u8 {
    unsafe { query(conn_handle, sql, params, Values::Typed) }
}

// ── Transaction Management ──────────────────────────────────────────────

/// Execute a bare SQL command (BEGIN/COMMIT/ROLLBACK) on a SQLite connection.
/// Returns a MeshResult: Ok(null) on success, Err(message) on failure.
fn sqlite_simple_exec(conn: &SqliteConn, sql: &str) -> *mut u8 {
    let sql_cstr = match CString::new(sql) {
        Ok(c) => c,
        Err(_) => return err_result("SQL contains null byte"),
    };
    unsafe {
        let rc = sqlite3_exec(
            conn.db,
            sql_cstr.as_ptr(),
            None,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
        if rc != SQLITE_OK {
            sqlite_err_result(conn.db)
        } else {
            alloc_result(0, std::ptr::null_mut()) as *mut u8
        }
    }
}

/// Begin a SQLite transaction.
///
/// # Signature
///
/// `mesh_sqlite_begin(conn_handle: u64) -> *mut u8 (MeshResult<Unit, String>)`
///
/// Sends `BEGIN` and returns Ok(()) or Err(error_message).
#[no_mangle]
pub extern "C" fn mesh_sqlite_begin(conn_handle: u64) -> *mut u8 {
    let conn = unsafe { &*(conn_handle as *const SqliteConn) };
    sqlite_simple_exec(conn, "BEGIN")
}

/// Commit a SQLite transaction.
///
/// # Signature
///
/// `mesh_sqlite_commit(conn_handle: u64) -> *mut u8 (MeshResult<Unit, String>)`
///
/// Sends `COMMIT` and returns Ok(()) or Err(error_message).
#[no_mangle]
pub extern "C" fn mesh_sqlite_commit(conn_handle: u64) -> *mut u8 {
    let conn = unsafe { &*(conn_handle as *const SqliteConn) };
    sqlite_simple_exec(conn, "COMMIT")
}

/// Rollback a SQLite transaction.
///
/// # Signature
///
/// `mesh_sqlite_rollback(conn_handle: u64) -> *mut u8 (MeshResult<Unit, String>)`
///
/// Sends `ROLLBACK` and returns Ok(()) or Err(error_message).
#[no_mangle]
pub extern "C" fn mesh_sqlite_rollback(conn_handle: u64) -> *mut u8 {
    let conn = unsafe { &*(conn_handle as *const SqliteConn) };
    sqlite_simple_exec(conn, "ROLLBACK")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytes::MeshBytes;
    use crate::collections::list::{
        mesh_list_append, mesh_list_get, mesh_list_length, mesh_list_new,
    };
    use crate::gc::mesh_rt_init;
    use crate::io::MeshResult;
    use crate::string::mesh_string_new;

    /// Helper to create a MeshString from a byte literal.
    fn mk_str(s: &[u8]) -> *mut MeshString {
        mesh_string_new(s.as_ptr(), s.len() as u64)
    }

    #[test]
    fn test_open_close() {
        mesh_rt_init();

        // Open an in-memory database
        let path = mk_str(b":memory:");
        let result = mesh_sqlite_open(path);
        assert!(!result.is_null());

        let r = unsafe { &*(result as *const MeshResult) };
        assert_eq!(r.tag, 0, "open should succeed");

        let handle = unsafe { unbox_u64_payload(r.value) };
        assert_ne!(handle, 0, "handle should be non-zero");

        // Close it
        mesh_sqlite_close(handle);
    }

    #[test]
    fn test_execute_create_table() {
        mesh_rt_init();

        // Open
        let path = mk_str(b":memory:");
        let result = mesh_sqlite_open(path);
        let r = unsafe { &*(result as *const MeshResult) };
        assert_eq!(r.tag, 0);
        let handle = unsafe { unbox_u64_payload(r.value) };

        // Create table
        let sql = mk_str(b"CREATE TABLE test (id INTEGER PRIMARY KEY, name TEXT)");
        let empty_params = mesh_list_new();
        let exec_result = mesh_sqlite_execute(handle, sql, empty_params);
        let er = unsafe { &*(exec_result as *const MeshResult) };
        assert_eq!(er.tag, 0, "CREATE TABLE should succeed");

        mesh_sqlite_close(handle);
    }

    #[test]
    fn test_insert_and_query() {
        mesh_rt_init();

        // Open
        let path = mk_str(b":memory:");
        let result = mesh_sqlite_open(path);
        let r = unsafe { &*(result as *const MeshResult) };
        assert_eq!(r.tag, 0);
        let handle = unsafe { unbox_u64_payload(r.value) };

        // Create table
        let sql = mk_str(b"CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, age TEXT)");
        let empty_params = mesh_list_new();
        let exec_result = mesh_sqlite_execute(handle, sql, empty_params);
        let er = unsafe { &*(exec_result as *const MeshResult) };
        assert_eq!(er.tag, 0);

        // Insert a row with params
        let insert_sql = mk_str(b"INSERT INTO users (name, age) VALUES (?, ?)");
        let mut params = mesh_list_new();
        let name_val = mk_str(b"Alice");
        let age_val = mk_str(b"30");
        params = mesh_list_append(params, name_val as u64);
        params = mesh_list_append(params, age_val as u64);

        let insert_result = mesh_sqlite_execute(handle, insert_sql, params);
        let ir = unsafe { &*(insert_result as *const MeshResult) };
        assert_eq!(ir.tag, 0, "INSERT should succeed");
        assert_eq!(
            unsafe { unbox_i64_payload(ir.value) },
            1,
            "should affect 1 row"
        );

        // Query
        let query_sql = mk_str(b"SELECT name, age FROM users");
        let empty_params2 = mesh_list_new();
        let query_result = mesh_sqlite_query(handle, query_sql, empty_params2);
        let qr = unsafe { &*(query_result as *const MeshResult) };
        assert_eq!(qr.tag, 0, "SELECT should succeed");

        // The result is a MeshList with 1 row
        let list_ptr = qr.value;
        let list_len = unsafe { *(list_ptr as *const u64) };
        assert_eq!(list_len, 1, "should have 1 row");

        mesh_sqlite_close(handle);
    }

    fn result(pointer: *mut u8) -> &'static MeshResult {
        unsafe { &*(pointer as *const MeshResult) }
    }

    fn error_text(pointer: *mut u8) -> String {
        let result = result(pointer);
        assert_eq!(result.tag, 1, "expected an error");
        unsafe { (*(result.value as *const MeshString)).as_str().to_string() }
    }

    fn texts(values: &[&str]) -> *mut u8 {
        values.iter().fold(mesh_list_new(), |list, value| {
            mesh_list_append(list, mesh_str(value) as u64)
        })
    }

    /// Text parameters and columns keep every byte, a NUL too; a statement
    /// takes exactly its parameters, and SQL that holds none is refused.
    #[test]
    fn text_values_are_bound_and_read_whole() {
        mesh_rt_init();
        let open = result(mesh_sqlite_open(mesh_str(":memory:")));
        let handle = unsafe { unbox_u64_payload(open.value) };
        let run =
            |sql: &str, params: &[&str]| mesh_sqlite_execute(handle, mesh_str(sql), texts(params));
        assert_eq!(
            result(run("CREATE TABLE notes (body TEXT, extra TEXT)", &[])).tag,
            0
        );
        assert_eq!(
            result(run("INSERT INTO notes VALUES (?, NULL)", &["a\0b"])).tag,
            0
        );

        let rows = mesh_sqlite_query(
            handle,
            mesh_str("SELECT body, extra FROM notes"),
            texts(&[]),
        );
        let rows = result(rows).value;
        assert_eq!(mesh_list_length(rows), 1);
        let row = mesh_list_get(rows, 0) as *mut u8;
        let column = |name: &str| unsafe {
            let value = crate::collections::map::mesh_map_get(row, mesh_str(name) as u64);
            (*(value as *const MeshString)).as_str().to_string()
        };
        assert_eq!(column("body"), "a\0b");
        assert_eq!(column("extra"), "", "a NULL reads as the empty string");

        assert_eq!(
            error_text(run("INSERT INTO notes VALUES (?, ?)", &["one"])),
            "SQLite statement expects 2 parameters but received 1"
        );
        assert_eq!(
            error_text(run("  -- nothing", &[])),
            "SQLite statement is empty"
        );
        assert_eq!(error_text(run("SELECT \0", &[])), "SQL contains null byte");
        assert!(error_text(run("INSERT INTO missing VALUES (1)", &[])).contains("no such table"));
        assert!(
            error_text(mesh_sqlite_query(handle, mesh_str("SELEC 1"), texts(&[])))
                .contains("syntax error")
        );

        // A failure part way through the rows is the query's error.
        let failing = mesh_sqlite_query(
            handle,
            mesh_str("SELECT abs(-9223372036854775807 - 1) FROM notes"),
            texts(&[]),
        );
        assert!(error_text(failing).contains("integer overflow"));

        assert_eq!(result(mesh_sqlite_begin(handle)).tag, 0);
        assert!(error_text(mesh_sqlite_begin(handle)).contains("within a transaction"));
        assert_eq!(result(mesh_sqlite_rollback(handle)).tag, 0);
        assert!(error_text(mesh_sqlite_commit(handle)).contains("no transaction is active"));
        mesh_sqlite_close(handle);
    }

    #[test]
    fn opening_fails_for_what_is_wrong_with_the_path() {
        mesh_rt_init();
        assert_eq!(
            error_text(mesh_sqlite_open(mesh_str("a\0b"))),
            "path contains null byte"
        );
        assert!(error_text(mesh_sqlite_open(mesh_str(
            "/nonexistent-dir/for/sure/db.sqlite"
        )))
        .contains("unable to open"));
    }

    #[test]
    fn typed_values_preserve_types_and_reject_unbounded_inputs() {
        mesh_rt_init();
        let opened = mesh_sqlite_open(mk_str(b":memory:"));
        let opened = unsafe { &*(opened as *const MeshResult) };
        assert_eq!(opened.tag, 0);
        let handle = unsafe { unbox_u64_payload(opened.value) };

        let created = mesh_sqlite_execute(
            handle,
            mk_str(b"CREATE TABLE values_test (label TEXT, payload BLOB, optional BLOB, empty_payload BLOB) STRICT"),
            mesh_list_new(),
        );
        assert_eq!(unsafe { (*(created as *const MeshResult)).tag }, 0);

        let payload = mesh_bytes_new([0, 0xff, 0x80].as_ptr(), 3) as *mut u8;
        let empty = mesh_bytes_new(std::ptr::null(), 0) as *mut u8;
        assert!(!payload.is_null());
        assert!(!empty.is_null());
        let mut params = mesh_list_new();
        params = mesh_list_append(params, unsafe {
            alloc_db_value(DB_VALUE_TEXT, mk_str(b"typed") as *mut u8)
        } as u64);
        params = mesh_list_append(params, unsafe { alloc_db_value(DB_VALUE_BINARY, payload) }
            as u64);
        params =
            mesh_list_append(
                params,
                unsafe { alloc_db_value(DB_VALUE_NULL, std::ptr::null_mut()) } as u64,
            );
        params = mesh_list_append(params, unsafe { alloc_db_value(DB_VALUE_BINARY, empty) }
            as u64);
        let inserted = mesh_sqlite_execute_values(
            handle,
            mk_str(b"INSERT INTO values_test VALUES (?, ?, ?, ?)"),
            params,
        );
        assert_eq!(unsafe { (*(inserted as *const MeshResult)).tag }, 0);

        let mut filters = mesh_list_new();
        filters = mesh_list_append(filters, unsafe {
            alloc_db_value(DB_VALUE_TEXT, mk_str(b"typed") as *mut u8)
        } as u64);
        filters = mesh_list_append(filters, unsafe { alloc_db_value(DB_VALUE_BINARY, payload) }
            as u64);
        filters =
            mesh_list_append(
                filters,
                unsafe { alloc_db_value(DB_VALUE_NULL, std::ptr::null_mut()) } as u64,
            );
        let queried = mesh_sqlite_query_values(
            handle,
            mk_str(b"SELECT label, payload, optional, empty_payload FROM values_test WHERE label = ? AND payload = ? AND optional IS ?"),
            filters,
        );
        let queried = unsafe { &*(queried as *const MeshResult) };
        assert_eq!(queried.tag, 0);
        assert_eq!(mesh_list_length(queried.value), 1);
        let row = mesh_list_get(queried.value, 0) as *mut u8;
        let label = crate::collections::map::mesh_map_entry_value(row, 0) as *const MeshDbValue;
        let binary = crate::collections::map::mesh_map_entry_value(row, 1) as *const MeshDbValue;
        let null = crate::collections::map::mesh_map_entry_value(row, 2) as *const MeshDbValue;
        let empty = crate::collections::map::mesh_map_entry_value(row, 3) as *const MeshDbValue;
        unsafe {
            assert_eq!((*label).tag, DB_VALUE_TEXT);
            assert_eq!((*((*label).payload as *const MeshString)).as_str(), "typed");
            assert_eq!((*binary).tag, DB_VALUE_BINARY);
            assert_eq!(
                (*((*binary).payload as *const MeshBytes)).as_slice(),
                [0, 0xff, 0x80]
            );
            assert_eq!((*null).tag, DB_VALUE_NULL);
            assert_eq!((*empty).tag, DB_VALUE_BINARY);
            assert_eq!((*((*empty).payload as *const MeshBytes)).len, 0);
        }

        let mismatch = mesh_sqlite_execute_values(handle, mk_str(b"SELECT ?"), mesh_list_new());
        assert_eq!(unsafe { (*(mismatch as *const MeshResult)).tag }, 1);
        assert!(add_result_bytes(MAX_SQLITE_RESULT_BYTES, 1).is_err());

        mesh_sqlite_close(handle);
    }
}
