//! SQLite C FFI wrapper functions for the Mesh runtime.
//!
//! Mesh programs call these functions to interact with SQLite databases:
//! - `mesh_sqlite_open`: Open a database connection
//! - `mesh_sqlite_close`: Close a connection
//! - `mesh_sqlite_execute`: Execute a write query (INSERT/UPDATE/DELETE/CREATE)
//! - `mesh_sqlite_query`: Execute a read query (SELECT), returns rows
//! - `mesh_sqlite_execute_values`: Execute with typed `DbValue` parameters
//! - `mesh_sqlite_query_values`: Query typed `DbValue` rows
//!
//! Opaque handles identify entries in a generation-checked registry.

use libsqlite3_sys::*;
use parking_lot::Mutex;
use std::borrow::Cow;
use std::cell::Cell;
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{Arc, LazyLock};

use crate::actor::{
    cooperative_recv, cooperative_reply, CooperativeReply, CooperativeReplyReceiver,
};
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

// ponytail: admission is bounded per connection rather than across all databases.
const WRITE_QUEUE_CAPACITY: usize = 128;
const CLOSED: &str = "SQLite connection is closed";

struct Database {
    db: *mut sqlite3,
    effects: Box<Cell<StatementEffects>>,
}

#[derive(Clone, Copy, Default)]
struct StatementEffects {
    worker_only: bool,
}

// The database and its authorizer state are accessed under the connection mutex.
unsafe impl Send for Database {}

impl Drop for Database {
    fn drop(&mut self) {
        unsafe { sqlite3_close(self.db) };
    }
}

struct SqliteConn {
    state: Arc<ConnectionState>,
    requests: Mutex<Option<SyncSender<Request>>>,
}

struct ConnectionState {
    database: Mutex<Option<Database>>,
    pending: AtomicUsize,
    exit: Mutex<WorkerExit>,
}

enum WorkerExit {
    Running,
    Closing(CooperativeReply<()>),
    Stopped,
}

struct Request {
    command: Command,
    operation: Operation,
    reply: CooperativeReply<Result<Reply, String>>,
}

enum Command {
    Prepared(StmtGuard),
    Deferred {
        sql: String,
        params: Vec<OwnedValue>,
    },
}

#[derive(Clone, Copy)]
enum Operation {
    Execute,
    Query(Values),
    Transaction,
}

enum Reply {
    Changes(i64),
    Rows {
        columns: Vec<String>,
        rows: Vec<Vec<ResultValue<'static>>>,
    },
    Unit,
}

enum Dispatch {
    Inline(*mut u8),
    Queued(CooperativeReplyReceiver<Result<Reply, String>>),
}

enum OwnedValue {
    Text(Vec<u8>),
    Binary(Vec<u8>),
    Null,
}

impl OwnedValue {
    fn copy(value: BindValue<'_>) -> Self {
        match value {
            BindValue::Text(bytes) => Self::Text(bytes.to_vec()),
            BindValue::Binary(bytes) => Self::Binary(bytes.to_vec()),
            BindValue::Null => Self::Null,
        }
    }

    fn bind(&self) -> BindValue<'_> {
        match self {
            Self::Text(bytes) => BindValue::Text(bytes),
            Self::Binary(bytes) => BindValue::Binary(bytes),
            Self::Null => BindValue::Null,
        }
    }
}

#[derive(Default)]
struct Registry {
    slots: Vec<Slot>,
    free: Vec<u32>,
}

struct Slot {
    generation: u32,
    conn: Option<Arc<SqliteConn>>,
}

impl Registry {
    fn insert(&mut self, conn: Arc<SqliteConn>) -> Result<u64, String> {
        let index = match self.free.pop() {
            Some(index) => index,
            None => {
                let index = u32::try_from(self.slots.len())
                    .map_err(|_| "SQLite connection registry is full".to_string())?;
                self.slots.push(Slot {
                    generation: 1,
                    conn: None,
                });
                index
            }
        };
        let slot = &mut self.slots[index as usize];
        slot.conn = Some(conn);
        Ok(u64::from(slot.generation) << 32 | u64::from(index))
    }

    fn get(&self, handle: u64) -> Option<Arc<SqliteConn>> {
        let slot = self.slots.get(handle as u32 as usize)?;
        (slot.generation == (handle >> 32) as u32)
            .then(|| slot.conn.clone())
            .flatten()
    }

    fn remove(&mut self, handle: u64) -> Option<Arc<SqliteConn>> {
        let index = handle as u32;
        let slot = self.slots.get_mut(index as usize)?;
        if slot.generation != (handle >> 32) as u32 {
            return None;
        }
        let conn = slot.conn.take()?;
        if let Some(next) = slot.generation.checked_add(1) {
            slot.generation = next;
            self.free.push(index);
        }
        Some(conn)
    }
}

static CONNECTIONS: LazyLock<Mutex<Registry>> = LazyLock::new(|| Mutex::new(Registry::default()));

unsafe extern "C" fn authorize_statement(
    context: *mut c_void,
    action: c_int,
    _: *const c_char,
    _: *const c_char,
    _: *const c_char,
    _: *const c_char,
) -> c_int {
    let effects = &*context.cast::<Cell<StatementEffects>>();
    let mut current = effects.get();
    if matches!(
        action,
        SQLITE_TRANSACTION | SQLITE_SAVEPOINT | SQLITE_ATTACH | SQLITE_DETACH
    ) {
        // SQLite classifies transaction control and ATTACH as read-only.
        current.worker_only = true;
    }
    effects.set(current);
    SQLITE_OK
}

impl Database {
    fn open(path: &CStr) -> Result<Self, String> {
        unsafe {
            let mut db = std::ptr::null_mut();
            let rc = sqlite3_open_v2(
                path.as_ptr(),
                &mut db,
                SQLITE_OPEN_READWRITE | SQLITE_OPEN_CREATE | SQLITE_OPEN_FULLMUTEX,
                std::ptr::null(),
            );
            if rc != SQLITE_OK {
                let error = sqlite_err_string(db);
                if !db.is_null() {
                    sqlite3_close(db);
                }
                return Err(error);
            }
            let database = Self {
                db,
                effects: Box::new(Cell::new(StatementEffects::default())),
            };
            let context = std::ptr::from_ref(database.effects.as_ref())
                .cast_mut()
                .cast();
            let rc = sqlite3_set_authorizer(db, Some(authorize_statement), context);
            if rc != SQLITE_OK {
                return Err(sqlite_err_string(db));
            }
            Ok(database)
        }
    }

    fn prepare(&self, sql: &str, params: &[BindValue<'_>]) -> Result<StmtGuard, String> {
        self.effects.set(StatementEffects::default());
        unsafe {
            let mut statement = prepare_statement(self.db, sql)?;
            statement.effects = self.effects.get();
            bind_values(self.db, statement.stmt, params)?;
            Ok(statement)
        }
    }

    fn inline_read(&self, statement: &StmtGuard) -> bool {
        unsafe {
            !statement.effects.worker_only
                && sqlite3_get_autocommit(self.db) != 0
                && sqlite3_stmt_readonly(statement.stmt) != 0
        }
    }

    fn step(&self, statement: StmtGuard, operation: Operation) -> Result<Reply, String> {
        unsafe {
            match operation {
                Operation::Query(values) => {
                    let rows = read_rows(self.db, statement.stmt, |columns, bytes| {
                        (0..columns.len() as c_int)
                            .map(|i| {
                                read_cell(statement.stmt, i, values, bytes)
                                    .map(ResultValue::into_owned)
                            })
                            .collect::<Result<Vec<_>, _>>()
                    });
                    rows.map(|(columns, rows)| Reply::Rows { columns, rows })
                }
                Operation::Execute | Operation::Transaction => match sqlite3_step(statement.stmt) {
                    SQLITE_DONE | SQLITE_ROW => match operation {
                        Operation::Execute => Ok(Reply::Changes(sqlite3_changes(self.db) as i64)),
                        _ => Ok(Reply::Unit),
                    },
                    _ => Err(sqlite_err_string(self.db)),
                },
            }
        }
    }
}

impl SqliteConn {
    fn open(path: &CStr) -> Result<Self, String> {
        let state = Arc::new(ConnectionState {
            database: Mutex::new(Some(Database::open(path)?)),
            pending: AtomicUsize::new(0),
            exit: Mutex::new(WorkerExit::Running),
        });
        let (sender, receiver) = mpsc::sync_channel::<Request>(WRITE_QUEUE_CAPACITY);
        let worker_state = state.clone();
        std::thread::Builder::new()
            .name("mesh-sqlite".into())
            .spawn(move || {
                let exit = OnWorkerExit(&worker_state);
                for request in receiver {
                    let Request {
                        command,
                        operation,
                        reply,
                    } = request;
                    let result = {
                        let locked = worker_state.database.lock();
                        let db = locked.as_ref().expect("open SQLite worker has a database");
                        match command {
                            Command::Prepared(stmt) => db.step(stmt, operation),
                            Command::Deferred { sql, params } => db
                                .prepare(
                                    &sql,
                                    &params.iter().map(OwnedValue::bind).collect::<Vec<_>>(),
                                )
                                .and_then(|stmt| db.step(stmt, operation)),
                        }
                    };
                    worker_state.pending.fetch_sub(1, Ordering::Release);
                    reply.send(result);
                }
                drop(exit);
            })
            .map_err(|error| format!("could not start SQLite worker: {error}"))?;
        Ok(Self {
            state,
            requests: Mutex::new(Some(sender)),
        })
    }

    fn close(&self) {
        let (reply, receiver) = cooperative_reply();
        {
            let mut exit = self.state.exit.lock();
            if matches!(*exit, WorkerExit::Stopped) {
                return;
            }
            *exit = WorkerExit::Closing(reply);
        }
        self.requests.lock().take();
        let _ = cooperative_recv(&receiver);
    }

    fn enqueue(
        &self,
        sender: &Option<SyncSender<Request>>,
        command: Command,
        operation: Operation,
    ) -> Result<CooperativeReplyReceiver<Result<Reply, String>>, String> {
        let sender = sender.as_ref().ok_or_else(|| CLOSED.to_string())?;
        let (reply, receiver) = cooperative_reply();
        self.state.pending.fetch_add(1, Ordering::AcqRel);
        match sender.try_send(Request {
            command,
            operation,
            reply,
        }) {
            Ok(()) => Ok(receiver),
            Err(error) => {
                self.state.pending.fetch_sub(1, Ordering::Release);
                Err(match error {
                    TrySendError::Full(_) => "SQLite write queue is full",
                    TrySendError::Disconnected(_) => "SQLite worker stopped",
                }
                .to_string())
            }
        }
    }

    fn dispatch(
        &self,
        sql: &str,
        params: Vec<BindValue<'_>>,
        operation: Operation,
    ) -> Result<Dispatch, String> {
        if let Some(locked) = self.state.database.try_lock() {
            let db = locked.as_ref().ok_or_else(|| CLOSED.to_string())?;
            if self.requests.lock().is_none() {
                return Err(CLOSED.to_string());
            }
            if self.state.pending.load(Ordering::Acquire) == 0 {
                let stmt = db.prepare(sql, &params)?;
                if !matches!(operation, Operation::Transaction) && db.inline_read(&stmt) {
                    return match operation {
                        Operation::Query(values) => unsafe {
                            read_mesh_rows(db.db, stmt.stmt, values)
                        },
                        _ => db
                            .step(stmt, operation)
                            .map(|reply| encode_reply(reply, operation)),
                    }
                    .map(Dispatch::Inline);
                }
                let requests = self.requests.lock();
                if self.state.pending.load(Ordering::Acquire) == 0 {
                    return self
                        .enqueue(&requests, Command::Prepared(stmt), operation)
                        .map(Dispatch::Queued);
                }
                // A concurrent caller queued work during preparation. Reprepare
                // behind that work since it can change the schema.
            }
        }
        let command = Command::Deferred {
            sql: sql.to_owned(),
            params: params.into_iter().map(OwnedValue::copy).collect(),
        };
        self.enqueue(&self.requests.lock(), command, operation)
            .map(Dispatch::Queued)
    }
}

struct OnWorkerExit<'a>(&'a ConnectionState);

impl Drop for OnWorkerExit<'_> {
    fn drop(&mut self) {
        self.0.database.lock().take();
        let exit = std::mem::replace(&mut *self.0.exit.lock(), WorkerExit::Stopped);
        if let WorkerExit::Closing(reply) = exit {
            reply.send(());
        }
    }
}

/// RAII guard ensuring sqlite3_finalize is always called on a prepared
/// statement, even when an error causes an early return.
struct StmtGuard {
    stmt: *mut sqlite3_stmt,
    effects: StatementEffects,
}

// SQLite is opened in serialized mode. A moved statement keeps its
// connection alive through the request queue and is stepped once by its owner.
unsafe impl Send for StmtGuard {}

impl Drop for StmtGuard {
    fn drop(&mut self) {
        // `prepare_statement` makes a guard only for a statement it prepared.
        unsafe {
            sqlite3_finalize(self.stmt);
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
        Ok(StmtGuard {
            stmt,
            effects: StatementEffects::default(),
        })
    }
}

/// Bind `values` to the statement's parameters, one for each.
unsafe fn bind_values(
    db: *mut sqlite3,
    stmt: *mut sqlite3_stmt,
    values: &[BindValue<'_>],
) -> Result<(), String> {
    let expected = sqlite3_bind_parameter_count(stmt) as usize;
    if values.len() != expected {
        return Err(format!(
            "SQLite statement expects {expected} parameters but received {}",
            values.len()
        ));
    }
    for (index, value) in values.iter().copied().enumerate() {
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

fn run_sql(
    conn_handle: u64,
    sql: &str,
    params: Vec<BindValue<'_>>,
    operation: Operation,
) -> *mut u8 {
    let conn = CONNECTIONS.lock().get(conn_handle);
    match conn
        .ok_or_else(|| CLOSED.to_string())
        .and_then(|conn| conn.dispatch(sql, params, operation))
        .and_then(|dispatch| match dispatch {
            Dispatch::Inline(value) => Ok(value),
            Dispatch::Queued(receiver) => receive_reply(&receiver, operation),
        }) {
        Ok(result) => result,
        Err(error) => err_result(&error),
    }
}

fn receive_reply(
    receiver: &CooperativeReplyReceiver<Result<Reply, String>>,
    operation: Operation,
) -> Result<*mut u8, String> {
    cooperative_recv(receiver)
        .map_err(|_| "SQLite worker stopped".to_string())?
        .map(|reply| encode_reply(reply, operation))
}

fn encode_reply(reply: Reply, operation: Operation) -> *mut u8 {
    match reply {
        Reply::Changes(changes) => alloc_result(0, box_scalar(changes)) as *mut u8,
        Reply::Unit => alloc_result(0, std::ptr::null_mut()) as *mut u8,
        Reply::Rows { columns, rows } => {
            let typed = matches!(operation, Operation::Query(Values::Typed));
            let rows: Vec<_> = rows
                .into_iter()
                .map(|row| {
                    encode_row(&columns, row.into_iter().map(|cell| Ok(cell.encode(typed))))
                        .unwrap()
                })
                .collect();
            encode_rows(&rows)
        }
    }
}

unsafe fn statement(
    conn_handle: u64,
    sql: *const MeshString,
    params: *mut u8,
    values: Values,
    operation: Operation,
) -> *mut u8 {
    let params = match values {
        Values::Text => text_values(params, MAX_SQLITE_VALUES, "SQLite"),
        Values::Typed => db_values(params, MAX_SQLITE_VALUES, "SQLite"),
    };
    match params {
        Ok(params) => run_sql(conn_handle, text_of(sql), params, operation),
        Err(error) => err_result(&error),
    }
}

enum ResultValue<'a> {
    Text(Cow<'a, str>),
    Binary(Cow<'a, [u8]>),
    Null,
}

impl ResultValue<'_> {
    fn into_owned(self) -> ResultValue<'static> {
        match self {
            Self::Text(text) => ResultValue::Text(Cow::Owned(text.into_owned())),
            Self::Binary(bytes) => ResultValue::Binary(Cow::Owned(bytes.into_owned())),
            Self::Null => ResultValue::Null,
        }
    }

    fn encode(&self, typed: bool) -> u64 {
        // The tag matches the allocated Mesh payload, and NULL has no payload.
        unsafe {
            match self {
                Self::Text(text) => {
                    let text = mesh_str(text);
                    if typed {
                        alloc_db_value(DB_VALUE_TEXT, text.cast()) as u64
                    } else {
                        text as u64
                    }
                }
                Self::Binary(bytes) => {
                    let blob = mesh_bytes_new(bytes.as_ptr(), bytes.len() as u64);
                    alloc_db_value(DB_VALUE_BINARY, blob.cast()) as u64
                }
                Self::Null if typed => alloc_db_value(DB_VALUE_NULL, std::ptr::null_mut()) as u64,
                Self::Null => mesh_str("") as u64,
            }
        }
    }
}

fn encode_row(
    columns: &[String],
    values: impl IntoIterator<Item = Result<u64, String>>,
) -> Result<u64, String> {
    let mut entries = Vec::<[u64; 2]>::with_capacity(columns.len());
    let mut indexes = HashMap::<&str, usize>::with_capacity(columns.len());
    for (name, value) in columns.iter().zip(values) {
        let value = value?;
        if let Some(index) = indexes.get(name.as_str()).copied() {
            entries[index][1] = value;
        } else {
            indexes.insert(name.as_str(), entries.len());
            entries.push([mesh_str(name) as u64, value]);
        }
    }
    Ok(mesh_map_from_string_entries(&entries) as u64)
}

fn encode_rows(rows: &[u64]) -> *mut u8 {
    alloc_result(0, mesh_list_from_array(rows.as_ptr(), rows.len() as i64)) as *mut u8
}

unsafe fn read_mesh_rows(
    db: *mut sqlite3,
    stmt: *mut sqlite3_stmt,
    values: Values,
) -> Result<*mut u8, String> {
    let (_, rows) = read_rows(db, stmt, |columns, bytes| {
        encode_row(
            columns,
            (0..columns.len() as c_int).map(|i| {
                read_cell(stmt, i, values, bytes).map(|cell| cell.encode(values == Values::Typed))
            }),
        )
    })?;
    Ok(encode_rows(&rows))
}

unsafe fn read_rows<T>(
    db: *mut sqlite3,
    stmt: *mut sqlite3_stmt,
    mut row: impl FnMut(&[String], &mut usize) -> Result<T, String>,
) -> Result<(Vec<String>, Vec<T>), String> {
    let column_count = sqlite3_column_count(stmt) as usize;
    let column_names = (0..column_count)
        .map(|column| {
            // Null only when SQLite runs out of memory.
            let name = sqlite3_column_name(stmt, column as c_int);
            (!name.is_null())
                .then(|| CStr::from_ptr(name).to_string_lossy().into_owned())
                .ok_or_else(|| sqlite_err_string(db))
        })
        .collect::<Result<Vec<String>, String>>()?;
    let row_base_bytes = column_names
        .iter()
        .try_fold(40_usize, |total, name| total.checked_add(name.len()))
        .and_then(|total| column_count.checked_mul(96)?.checked_add(total))
        .ok_or_else(|| "SQLite result size overflow".to_string())?;

    let mut rows = Vec::new();
    let mut result_bytes = 64_usize;
    loop {
        match sqlite3_step(stmt) {
            SQLITE_DONE => return Ok((column_names, rows)),
            SQLITE_ROW => {}
            _ => return Err(sqlite_err_string(db)),
        }
        if rows.len() == MAX_SQLITE_ROWS {
            return Err(format!("SQLite result exceeds {MAX_SQLITE_ROWS} row limit"));
        }
        result_bytes = add_result_bytes(result_bytes, row_base_bytes)?;

        rows.push(row(&column_names, &mut result_bytes)?);
    }
}

/// The returned bytes borrow the current SQLite row, until the next step.
unsafe fn read_cell<'a>(
    stmt: *mut sqlite3_stmt,
    column: c_int,
    values: Values,
    result_bytes: &mut usize,
) -> Result<ResultValue<'a>, String> {
    match (values, sqlite3_column_type(stmt, column)) {
        (_, SQLITE_NULL) => Ok(ResultValue::Null),
        (Values::Typed, SQLITE_BLOB) => {
            let bytes = sqlite3_column_blob(stmt, column).cast::<u8>();
            let len = column_len(stmt, column, result_bytes)?;
            if len == 0 {
                Ok(ResultValue::Binary(Cow::Borrowed(&[])))
            } else if bytes.is_null() {
                Err(format!("failed to read SQLite BLOB column {column}"))
            } else {
                Ok(ResultValue::Binary(Cow::Borrowed(
                    std::slice::from_raw_parts(bytes, len),
                )))
            }
        }
        _ => column_text(stmt, column, result_bytes).map(ResultValue::Text),
    }
}

fn add_result_bytes(total: usize, bytes: usize) -> Result<usize, String> {
    total
        .checked_add(bytes)
        .filter(|total| *total <= MAX_SQLITE_RESULT_BYTES)
        .ok_or_else(|| format!("SQLite result exceeds {MAX_SQLITE_RESULT_BYTES} byte limit"))
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
unsafe fn column_text<'a>(
    stmt: *mut sqlite3_stmt,
    column: c_int,
    result_bytes: &mut usize,
) -> Result<Cow<'a, str>, String> {
    // Null only when SQLite runs out of memory: a NULL column is read before.
    let bytes = NonNull::new(sqlite3_column_text(stmt, column) as *mut u8)
        .ok_or_else(|| format!("failed to read SQLite text column {column}"))?;
    let len = column_len(stmt, column, result_bytes)?;
    let text = String::from_utf8_lossy(std::slice::from_raw_parts(bytes.as_ptr(), len));
    if text.len() > MAX_DB_VALUE_BYTES {
        return Err(format!(
            "SQLite column {column} exceeds {MAX_DB_VALUE_BYTES} byte limit"
        ));
    }
    if text.len() > len {
        *result_bytes = add_result_bytes(*result_bytes, text.len() - len)?;
    }
    Ok(text)
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
pub extern "C-unwind" fn mesh_sqlite_open(path: *const MeshString) -> *mut u8 {
    let path = match CString::new(unsafe { text_of(path) }) {
        Ok(path) => path,
        Err(_) => return err_result("path contains null byte"),
    };
    let result = SqliteConn::open(&path).and_then(|conn| CONNECTIONS.lock().insert(Arc::new(conn)));
    match result {
        Ok(handle) => alloc_result(0, box_scalar(handle)) as *mut u8,
        Err(error) => err_result(&error),
    }
}

/// Close a SQLite database connection and drain requests that were accepted.
#[no_mangle]
pub extern "C-unwind" fn mesh_sqlite_close(conn_handle: u64) {
    let conn = CONNECTIONS.lock().remove(conn_handle);
    if let Some(conn) = conn {
        conn.close();
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
pub extern "C-unwind" fn mesh_sqlite_execute(
    conn_handle: u64,
    sql: *const MeshString,
    params: *mut u8,
) -> *mut u8 {
    unsafe { statement(conn_handle, sql, params, Values::Text, Operation::Execute) }
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
pub extern "C-unwind" fn mesh_sqlite_query(
    conn_handle: u64,
    sql: *const MeshString,
    params: *mut u8,
) -> *mut u8 {
    unsafe {
        statement(
            conn_handle,
            sql,
            params,
            Values::Text,
            Operation::Query(Values::Text),
        )
    }
}

/// Execute a statement with `DbValue` parameters.
#[no_mangle]
pub extern "C-unwind" fn mesh_sqlite_execute_values(
    conn_handle: u64,
    sql: *const MeshString,
    params: *mut u8,
) -> *mut u8 {
    unsafe { statement(conn_handle, sql, params, Values::Typed, Operation::Execute) }
}

/// Query rows as `Map<String, DbValue>`, preserving BLOB and NULL values.
#[no_mangle]
pub extern "C-unwind" fn mesh_sqlite_query_values(
    conn_handle: u64,
    sql: *const MeshString,
    params: *mut u8,
) -> *mut u8 {
    unsafe {
        statement(
            conn_handle,
            sql,
            params,
            Values::Typed,
            Operation::Query(Values::Typed),
        )
    }
}

// ── Transaction Management ──────────────────────────────────────────────

/// Begin a SQLite transaction: `Result<Unit, String>`.
#[no_mangle]
pub extern "C-unwind" fn mesh_sqlite_begin(conn_handle: u64) -> *mut u8 {
    run_sql(conn_handle, "BEGIN", Vec::new(), Operation::Transaction)
}

/// Commit a SQLite transaction: `Result<Unit, String>`.
#[no_mangle]
pub extern "C-unwind" fn mesh_sqlite_commit(conn_handle: u64) -> *mut u8 {
    run_sql(conn_handle, "COMMIT", Vec::new(), Operation::Transaction)
}

/// Roll back a SQLite transaction: `Result<Unit, String>`.
#[no_mangle]
pub extern "C-unwind" fn mesh_sqlite_rollback(conn_handle: u64) -> *mut u8 {
    run_sql(conn_handle, "ROLLBACK", Vec::new(), Operation::Transaction)
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

    fn open_handle(path: &str) -> u64 {
        let reply = result(mesh_sqlite_open(mesh_str(path)));
        assert_eq!(reply.tag, 0, "could not open {path}");
        unsafe { unbox_u64_payload(reply.value) }
    }

    fn query_cell(handle: u64, sql: &str, column: &str) -> String {
        let rows = result(mesh_sqlite_query(handle, mesh_str(sql), mesh_list_new()));
        assert_eq!(rows.tag, 0, "query failed: {sql}");
        let row = crate::collections::list::mesh_list_get(rows.value, 0) as *mut u8;
        let value = crate::collections::map::mesh_map_get(row, mesh_str(column) as u64);
        unsafe { text_of(value as *const MeshString).to_string() }
    }

    #[test]
    fn closed_handle_cannot_access_a_reused_slot() {
        mesh_rt_init();
        let closed = open_handle(":memory:");
        mesh_sqlite_close(closed);
        let replacement = open_handle(":memory:");
        assert_ne!(closed, replacement);
        mesh_sqlite_close(closed);
        assert_eq!(
            error_text(mesh_sqlite_query(
                closed,
                mesh_str("SELECT 1"),
                mesh_list_new()
            )),
            CLOSED
        );
        assert_eq!(query_cell(replacement, "SELECT 2 AS n", "n"), "2");
        mesh_sqlite_close(replacement);
    }

    #[test]
    fn sqlite_worker_keeps_requests_in_order_and_rejects_overflow() {
        crate::actor::in_actor(|| {
            let handle = open_handle(":memory:");
            let conn = CONNECTIONS.lock().get(handle).unwrap();
            assert_eq!(
                result(mesh_sqlite_execute(
                    handle,
                    mesh_str("CREATE TABLE t(n INTEGER)"),
                    mesh_list_new()
                ))
                .tag,
                0
            );
            let database = conn.state.database.lock();
            let mut replies = Vec::new();
            for n in 0..=WRITE_QUEUE_CAPACITY + 1 {
                let command = Command::Deferred {
                    sql: format!("INSERT INTO t VALUES ({n})"),
                    params: Vec::new(),
                };
                match conn.enqueue(&conn.requests.lock(), command, Operation::Execute) {
                    Ok(reply) => replies.push(reply),
                    Err(error) => {
                        assert_eq!(error, "SQLite write queue is full");
                        break;
                    }
                }
            }
            assert!((WRITE_QUEUE_CAPACITY..=WRITE_QUEUE_CAPACITY + 1).contains(&replies.len()));
            drop(database);
            let count = replies.len();
            for reply in replies {
                assert!(matches!(
                    cooperative_recv(&reply),
                    Ok(Ok(Reply::Changes(1)))
                ));
            }
            let order = query_cell(
                handle,
                "SELECT group_concat(n) AS n FROM (SELECT n FROM t ORDER BY rowid)",
                "n",
            );
            assert_eq!(
                order,
                (0..count)
                    .map(|n| n.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            );
            mesh_sqlite_close(handle);
        });
    }

    #[test]
    fn killing_an_actor_parked_on_a_write_leaves_the_connection_usable() {
        use crate::actor::{
            mesh_actor_exit, mesh_actor_spawn, mesh_rt_init_actor, process, ProcessId, ProcessState,
        };
        use std::time::{Duration, Instant};

        struct Write {
            handle: u64,
            finished: mpsc::Sender<()>,
        }

        extern "C-unwind" fn write(args: *const u8) {
            let write = unsafe { Box::from_raw(args.cast_mut().cast::<Write>()) };
            let _ = mesh_sqlite_execute(
                write.handle,
                mesh_str("INSERT INTO t VALUES (1)"),
                mesh_list_new(),
            );
            let _ = write.finished.send(());
        }

        mesh_rt_init();
        mesh_rt_init_actor(1);
        let handle = open_handle(":memory:");
        let conn = CONNECTIONS.lock().get(handle).unwrap();
        assert_eq!(
            result(mesh_sqlite_execute(
                handle,
                mesh_str("CREATE TABLE t(n INTEGER)"),
                mesh_list_new()
            ))
            .tag,
            0
        );
        let database = conn.state.database.lock();
        let (finished, receiver) = mpsc::channel();
        let args = Box::into_raw(Box::new(Write { handle, finished }));
        let pid = mesh_actor_spawn(write as *const u8, args.cast(), 0, 1);
        let actor = process(ProcessId(pid)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while actor.lock().state != ProcessState::Waiting {
            assert!(Instant::now() < deadline, "SQLite actor did not park");
            std::thread::sleep(Duration::from_millis(1));
        }
        mesh_actor_exit(pid, 2);
        drop(database);
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(5)),
            Err(mpsc::RecvTimeoutError::Disconnected)
        );
        assert_eq!(query_cell(handle, "SELECT n FROM t", "n"), "1");
        mesh_sqlite_close(handle);
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

    /// A statement that fails as it runs, a value the connection will not
    /// bind, a result past the row limit, a column past the value limit (as
    /// stored, or once its invalid UTF-8 is replaced) each come back as the
    /// error they are; a column name used twice keeps its last value.
    #[test]
    fn statements_and_results_fail_for_what_is_wrong_with_them() {
        mesh_rt_init();
        let handle =
            unsafe { unbox_u64_payload(result(mesh_sqlite_open(mk_str(b":memory:"))).value) };
        let run = |sql: &str, params: *mut u8| {
            mesh_sqlite_execute(handle, mk_str(sql.as_bytes()), params)
        };
        let rows = |sql: &str| mesh_sqlite_query(handle, mk_str(sql.as_bytes()), mesh_list_new());
        assert_eq!(
            result(run(
                "CREATE TABLE t (id INTEGER PRIMARY KEY)",
                mesh_list_new()
            ))
            .tag,
            0
        );
        assert_eq!(
            result(run("INSERT INTO t VALUES (1)", mesh_list_new())).tag,
            0
        );
        assert_eq!(
            error_text(run("INSERT INTO t VALUES (1)", mesh_list_new())),
            "UNIQUE constraint failed: t.id"
        );

        let conn = CONNECTIONS.lock().get(handle).unwrap();
        let set_length_limit = |limit| {
            let guard = conn.state.database.lock();
            let db = guard.as_ref().unwrap().db;
            unsafe { sqlite3_limit(db, SQLITE_LIMIT_LENGTH, limit) }
        };
        // SQLite lowers the length limit no further than 30 bytes.
        let limit = set_length_limit(30);
        assert_eq!(
            error_text(run("SELECT ?", texts(&[&"x".repeat(31)]))),
            "string or blob too big"
        );
        set_length_limit(limit);

        let many = format!(
            "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {}) SELECT i FROM n",
            MAX_SQLITE_ROWS + 1
        );
        assert_eq!(
            error_text(rows(&many)),
            format!("SQLite result exceeds {MAX_SQLITE_ROWS} row limit")
        );

        let twice = result(rows("SELECT 1 AS a, 2 AS a")).value;
        let row = crate::collections::list::mesh_list_get(twice, 0) as *mut u8;
        assert_eq!(crate::collections::map::mesh_map_size(row), 1);
        let a = crate::collections::map::mesh_map_get(row, mk_str(b"a") as u64);
        assert_eq!(unsafe { text_of(a as *const MeshString) }, "2");

        let too_big = format!("SQLite column 0 exceeds {MAX_DB_VALUE_BYTES} byte limit");
        let blob = format!("SELECT zeroblob({})", MAX_DB_VALUE_BYTES + 1);
        let typed = mesh_sqlite_query_values(handle, mk_str(blob.as_bytes()), mesh_list_new());
        assert_eq!(error_text(typed), too_big);
        // 0xFF bytes are invalid UTF-8, each replaced by a 3-byte U+FFFD.
        let invalid = format!(
            "SELECT CAST(unhex(replace(hex(zeroblob({})), '00', 'FF')) AS TEXT)",
            MAX_DB_VALUE_BYTES / 3 + 1
        );
        assert_eq!(error_text(rows(&invalid)), too_big);
        let replaced = result(rows("SELECT CAST(x'FF' AS TEXT) AS v")).value;
        let row = crate::collections::list::mesh_list_get(replaced, 0) as *mut u8;
        let v = crate::collections::map::mesh_map_get(row, mk_str(b"v") as u64);
        assert_eq!(unsafe { text_of(v as *const MeshString) }, "\u{fffd}");

        mesh_sqlite_close(handle);
    }
}
