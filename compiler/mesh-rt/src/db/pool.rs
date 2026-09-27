//! PostgreSQL connection pool for the Mesh runtime.
//!
//! Provides a bounded pool of PostgreSQL connections with Mutex+Condvar
//! synchronization. Multiple Mesh actors share a fixed set of database
//! connections, preventing connection exhaustion.
//!
//! ## Functions
//!
//! - `mesh_pool_open`: Create a pool with configurable min/max/timeout
//! - Runtime-internal checkout/checkin with lease provenance validation
//! - `mesh_pool_query`: Auto checkout-use-checkin for SELECT
//! - `mesh_pool_execute`: Auto checkout-use-checkin for INSERT/UPDATE/DELETE
//! - `mesh_pool_close`: Drain all connections, prevent new checkouts
//!
//! Pool handles are opaque u64 values (Box::into_raw), same pattern as
//! PgConn/SqliteConn handles.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use parking_lot::{Condvar, Mutex};

use super::pg::{
    mesh_pg_close, mesh_pg_execute, mesh_pg_execute_values, mesh_pg_query, mesh_pg_query_values,
    open, pg_simple_command, PgConn, ANSWER_TIMEOUT,
};
use crate::io::{alloc_result, box_scalar, err_result};
use crate::string::text_of;
use crate::string::MeshString;

// ── Data Structures ──────────────────────────────────────────────────────

struct PoolInner {
    url: String,
    /// Connections ready to lend.
    idle: Vec<u64>,
    /// Handles currently checked out by this exact pool.
    leased: HashSet<u64>,
    /// Connections open or being opened, idle or lent: at most `max_conns`.
    total_created: usize,
    max_conns: usize,
    checkout_timeout_ms: u64,
    closed: bool,
}

struct PgPool {
    inner: Mutex<PoolInner>,
    available: Condvar,
}

// ── Helpers ──────────────────────────────────────────────────────────────

pub(crate) unsafe fn unbox_u64_payload(ptr: *mut u8) -> u64 {
    *(ptr as *const u64)
}

/// Whether an idle connection still answers `SELECT 1`, within a bounded
/// time: a connection whose server went silent is dead too.
fn health_check(handle: u64) -> bool {
    let conn = unsafe { &mut *(handle as *mut PgConn) };
    conn.set_read_timeout(Some(ANSWER_TIMEOUT));
    let healthy = pg_simple_command(conn, "SELECT 1").is_ok();
    conn.set_read_timeout(None);
    healthy
}

/// A connection leased to the caller until it is checked in: an idle one
/// that still answers, else a new one while the pool is below its maximum,
/// else the first of either within the checkout timeout.
fn checkout(pool: &PgPool) -> Result<u64, String> {
    let mut inner = pool.inner.lock();
    // One deadline for the whole checkout: a wakeup that finds nothing to
    // take waits only what is left of it.
    let deadline = Instant::now() + Duration::from_millis(inner.checkout_timeout_ms);
    loop {
        if inner.closed {
            return Err("pool is closed".to_string());
        }
        if let Some(handle) = inner.idle.pop() {
            inner.leased.insert(handle);
            // Checked without the lock: a connection slow to answer holds up
            // only this checkout, not every other checkout and checkin.
            drop(inner);
            if health_check(handle) {
                return Ok(handle);
            }
            mesh_pg_close(handle);
            inner = pool.inner.lock();
            inner.leased.remove(&handle);
            inner.total_created -= 1;
            continue;
        }
        if inner.total_created < inner.max_conns {
            inner.total_created += 1;
            let url = inner.url.clone();
            // Connected without the lock.
            drop(inner);
            let opened = open(&url);
            inner = pool.inner.lock();
            match opened {
                Ok(handle) if !inner.closed => {
                    inner.leased.insert(handle);
                    return Ok(handle);
                }
                Ok(handle) => {
                    inner.total_created -= 1;
                    drop(inner);
                    mesh_pg_close(handle);
                    return Err("pool is closed".to_string());
                }
                Err(error) => {
                    inner.total_created -= 1;
                    drop(inner);
                    // The slot is free again, for a checkout waiting on one.
                    pool.available.notify_one();
                    return Err(format!("pool connect: {error}"));
                }
            }
        }
        if pool.available.wait_until(&mut inner, deadline).timed_out() {
            return Err("pool checkout timeout".to_string());
        }
    }
}

/// `run` on a connection checked out of the pool and checked back in after:
/// what it returns, or why no connection could be checked out.
fn with_connection(pool_handle: u64, run: impl FnOnce(u64) -> *mut u8) -> *mut u8 {
    match checkout(unsafe { &*(pool_handle as *const PgPool) }) {
        Ok(conn_handle) => {
            let result = run(conn_handle);
            mesh_pool_checkin(pool_handle, conn_handle);
            result
        }
        Err(error) => err_result(&error),
    }
}

// ── Public scoped API and runtime-internal leasing ───────────────────────

/// Create a PostgreSQL connection pool.
///
/// # Signature
///
/// `mesh_pool_open(url: *const MeshString, min_conns: i64, max_conns: i64,
///     timeout_ms: i64) -> *mut u8 (MeshResult<u64, String>)`
///
/// Pre-creates `min_conns` connections. Returns MeshResult with tag 0 (Ok)
/// containing the pool handle as u64, or tag 1 (Err) with error message.
#[no_mangle]
pub extern "C" fn mesh_pool_open(
    url: *const MeshString,
    min_conns: i64,
    max_conns: i64,
    timeout_ms: i64,
) -> *mut u8 {
    let url = unsafe { text_of(url) };
    // Clamp parameters to reasonable values
    let min = min_conns.max(0) as usize;
    let max = (max_conns.max(1) as usize).max(min);

    // Pre-create min_conns connections
    let mut idle = Vec::with_capacity(min);
    for _ in 0..min {
        match open(url) {
            Ok(handle) => idle.push(handle),
            Err(error) => {
                // Close all already-created connections
                for handle in idle {
                    mesh_pg_close(handle);
                }
                return err_result(&format!("pool open: {error}"));
            }
        }
    }

    let pool = Box::new(PgPool {
        inner: Mutex::new(PoolInner {
            url: url.to_string(),
            total_created: idle.len(),
            idle,
            leased: HashSet::new(),
            max_conns: max,
            checkout_timeout_ms: timeout_ms.max(100) as u64,
            closed: false,
        }),
        available: Condvar::new(),
    });
    alloc_result(0, box_scalar(Box::into_raw(pool) as u64)) as *mut u8
}

/// Check out a connection from the pool.
///
/// # Signature
///
/// `mesh_pool_checkout(pool_handle: u64) -> *mut u8 (MeshResult<u64, String>)`
///
/// Returns an idle connection, creates a new one if under max, or blocks
/// with timeout if pool is exhausted. Performs health check on idle
/// connections before returning them.
pub(crate) fn mesh_pool_checkout(pool_handle: u64) -> *mut u8 {
    match checkout(unsafe { &*(pool_handle as *const PgPool) }) {
        Ok(handle) => alloc_result(0, box_scalar(handle)) as *mut u8,
        Err(error) => err_result(&error),
    }
}

/// Return a connection to the pool.
///
/// # Signature
///
/// `mesh_pool_checkin(pool_handle: u64, conn_handle: u64)`
///
/// If the connection has an active transaction (txn_status != 'I'),
/// sends ROLLBACK to clean it up. If ROLLBACK fails, the connection
/// is destroyed instead of returned to idle.
pub(crate) fn mesh_pool_checkin(pool_handle: u64, conn_handle: u64) {
    let pool = unsafe { &*(pool_handle as *const PgPool) };
    // Reject foreign and already-returned handles before dereferencing
    // them. This is the runtime backstop for pool provenance.
    if !pool.inner.lock().leased.remove(&conn_handle) {
        return;
    }

    // Transaction cleanup (POOL-05): ROLLBACK if not idle
    let conn = unsafe { &mut *(conn_handle as *mut PgConn) };
    let usable = !conn.is_broken()
        && (conn.txn_status == b'I' || pg_simple_command(conn, "ROLLBACK").is_ok());
    let kept = {
        let mut inner = pool.inner.lock();
        let kept = usable && !inner.closed;
        if kept {
            inner.idle.push(conn_handle);
        } else {
            inner.total_created -= 1;
        }
        kept
    };
    if !kept {
        mesh_pg_close(conn_handle);
    }
    pool.available.notify_one();
}

/// Execute a read query (SELECT) with automatic checkout-use-checkin.
///
/// # Signature
///
/// `mesh_pool_query(pool_handle: u64, sql: *const MeshString, params: *mut u8)
///     -> *mut u8 (MeshResult<List<Map<String, String>>, String>)`
#[no_mangle]
pub extern "C" fn mesh_pool_query(
    pool_handle: u64,
    sql: *const MeshString,
    params: *mut u8,
) -> *mut u8 {
    with_connection(pool_handle, |conn| mesh_pg_query(conn, sql, params))
}

/// Execute a write statement (INSERT/UPDATE/DELETE) with automatic checkout-use-checkin.
///
/// # Signature
///
/// `mesh_pool_execute(pool_handle: u64, sql: *const MeshString, params: *mut u8)
///     -> *mut u8 (MeshResult<Int, String>)`
#[no_mangle]
pub extern "C" fn mesh_pool_execute(
    pool_handle: u64,
    sql: *const MeshString,
    params: *mut u8,
) -> *mut u8 {
    with_connection(pool_handle, |conn| mesh_pg_execute(conn, sql, params))
}

#[no_mangle]
pub extern "C" fn mesh_pool_query_values(
    pool_handle: u64,
    sql: *const MeshString,
    params: *mut u8,
) -> *mut u8 {
    with_connection(pool_handle, |conn| mesh_pg_query_values(conn, sql, params))
}

#[no_mangle]
pub extern "C" fn mesh_pool_execute_values(
    pool_handle: u64,
    sql: *const MeshString,
    params: *mut u8,
) -> *mut u8 {
    with_connection(pool_handle, |conn| {
        mesh_pg_execute_values(conn, sql, params)
    })
}

/// Execute a SELECT query with automatic checkout-use-checkin and map rows through a callback.
///
/// # Signature
///
/// `mesh_pool_query_as(pool_handle: u64, sql: *mut u8, params: *mut u8,
///     from_row_fn: *mut u8) -> *mut u8 (MeshResult<List<MeshResult>, String>)`
///
/// Same checkout/query_as/checkin pattern as `mesh_pool_query` but delegates to
/// `mesh_pg_query_as` for struct mapping.
#[no_mangle]
pub extern "C" fn mesh_pool_query_as(
    pool_handle: u64,
    sql: *mut u8,
    params: *mut u8,
    fn_ptr: *mut u8,
    env_ptr: *mut u8,
) -> *mut u8 {
    // The rows are decoded after the connection is back: a decoder that
    // queries the pool itself then finds it there.
    let query_result = mesh_pool_query(pool_handle, sql as *const MeshString, params);
    unsafe { crate::db::pg::decode_rows(query_result, fn_ptr, env_ptr) }
}

/// Close a connection pool.
///
/// # Signature
///
/// `mesh_pool_close(pool_handle: u64)`
///
/// Sets pool to closed state, drains all idle connections, and wakes
/// all blocked checkouts so they return "pool is closed" errors.
/// Active connections will be closed when checked in.
#[no_mangle]
pub extern "C" fn mesh_pool_close(pool_handle: u64) {
    let pool = unsafe { &*(pool_handle as *const PgPool) };
    let idle = {
        let mut inner = pool.inner.lock();
        inner.closed = true;
        inner.total_created -= inner.idle.len();
        std::mem::take(&mut inner.idle)
    };
    // Close idle connections outside the lock
    for handle in idle {
        mesh_pg_close(handle);
    }
    // Wake all blocked checkouts so they see closed=true
    pool.available.notify_all();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collections::list::{mesh_list_append, mesh_list_length, mesh_list_new};
    use crate::collections::map::mesh_map_size;
    use crate::gc::mesh_rt_init;
    use crate::io::MeshResult;
    use crate::string::mesh_string_new;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::mpsc;
    use std::thread;

    fn mk_str(s: &[u8]) -> *mut MeshString {
        mesh_string_new(s.as_ptr(), s.len() as u64)
    }

    /// A pool over `url` holding `idle` connections and lending `leased`.
    fn pool_of(
        url: &str,
        idle: Vec<u64>,
        leased: &[u64],
        max_conns: usize,
        timeout_ms: u64,
    ) -> u64 {
        Box::into_raw(Box::new(PgPool {
            inner: Mutex::new(PoolInner {
                url: url.to_string(),
                total_created: idle.len() + leased.len(),
                idle,
                leased: leased.iter().copied().collect(),
                max_conns,
                checkout_timeout_ms: timeout_ms,
                closed: false,
            }),
            available: Condvar::new(),
        })) as u64
    }

    fn pool(handle: u64) -> &'static PgPool {
        unsafe { &*(handle as *const PgPool) }
    }

    /// AuthenticationOk and ReadyForQuery: a finished handshake.
    const AUTHENTICATED: &[u8] = b"R\0\0\0\x08\0\0\0\0Z\0\0\0\x05I";
    /// The reply to `SELECT 1` (the health check).
    const SELECTED: &[u8] = b"C\0\0\0\x0dSELECT 1\0Z\0\0\0\x05I";

    /// The tag of the next message the client sends, or `None` once it goes.
    fn next_request(socket: &mut TcpStream) -> Option<u8> {
        let mut header = [0; 5];
        socket.read_exact(&mut header).ok()?;
        let length = u32::from_be_bytes(header[1..].try_into().unwrap()) as usize;
        socket.read_exact(&mut vec![0; length - 4]).ok()?;
        Some(header[0])
    }

    /// A PostgreSQL stand-in: `session` gets each connection it accepts,
    /// numbered from 0, once the client's startup message is read.
    fn fake_server(session: impl Fn(usize, TcpStream) + Send + Sync + 'static) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "postgres://u@{}/d?sslmode=disable",
            listener.local_addr().unwrap()
        );
        let session = std::sync::Arc::new(session);
        thread::spawn(move || {
            for (index, socket) in listener.incoming().enumerate() {
                let mut socket = socket.unwrap();
                let session = session.clone();
                thread::spawn(move || {
                    let mut length = [0; 4];
                    socket.read_exact(&mut length).unwrap();
                    let mut startup = vec![0; u32::from_be_bytes(length) as usize - 4];
                    socket.read_exact(&mut startup).unwrap();
                    session(index, socket);
                });
            }
        });
        url
    }

    /// Authenticate, then answer every query as `SELECT 1` until the
    /// client goes.
    fn serve(mut socket: TcpStream) {
        socket.write_all(AUTHENTICATED).unwrap();
        serve_queries(socket);
    }

    fn serve_queries(mut socket: TcpStream) {
        while let Some(tag) = next_request(&mut socket) {
            if tag == b'Q' {
                socket.write_all(SELECTED).unwrap();
            }
        }
    }

    #[test]
    fn checkin_discards_connection_after_partial_wire_response() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket.write_all(&[b'D', 0, 0, 0, 8, 0]).unwrap();
        });
        let mut conn = PgConn::from_test_stream(TcpStream::connect(address).unwrap());

        assert!(conn.read_wire_message().is_err());
        assert!(conn.is_broken());
        assert_eq!(
            conn.read_wire_message().unwrap_err(),
            "PostgreSQL connection is unusable"
        );
        server.join().unwrap();

        let conn_handle = Box::into_raw(Box::new(conn)) as u64;
        let pool_handle = pool_of("", Vec::new(), &[conn_handle], 1, 100);

        mesh_pool_checkin(pool_handle, conn_handle);

        let inner = pool(pool_handle).inner.lock();
        assert_eq!(inner.total_created, 0);
        assert!(inner.leased.is_empty());
        assert!(inner.idle.is_empty());
        drop(inner);
        unsafe { drop(Box::from_raw(pool_handle as *mut PgPool)) };
    }

    #[test]
    fn checkin_ignores_connection_not_leased_by_pool() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || listener.accept().unwrap());
        let conn = PgConn::from_test_stream(TcpStream::connect(address).unwrap());
        let conn_handle = Box::into_raw(Box::new(conn)) as u64;
        drop(server.join().unwrap());

        let pool_handle = pool_of("", Vec::new(), &[], 1, 100);

        mesh_pool_checkin(pool_handle, conn_handle);

        let (total_created, idle) = {
            let inner = pool(pool_handle).inner.lock();
            (inner.total_created, inner.idle.len())
        };
        mesh_pg_close(conn_handle);
        unsafe { drop(Box::from_raw(pool_handle as *mut PgPool)) };

        assert_eq!(total_created, 0);
        assert_eq!(idle, 0);
    }

    /// A checkout waiting for the only slot gets it when the connection
    /// that held it fails to open.
    #[test]
    fn a_failed_connection_attempt_frees_its_slot_for_a_waiting_checkout() {
        let (held, held_signal) = mpsc::channel::<()>();
        let (release, released) = mpsc::channel::<()>();
        let released = parking_lot::Mutex::new(released);
        let url = fake_server(move |index, socket| {
            if index == 0 {
                // Keep the first connection waiting, then refuse it.
                held.send(()).unwrap();
                released.lock().recv().unwrap();
                drop(socket);
            } else {
                serve(socket);
            }
        });
        let handle = pool_of(&url, Vec::new(), &[], 1, 5000);
        let first = thread::spawn(move || checkout(pool(handle)));
        held_signal.recv().unwrap();
        let second = thread::spawn(move || checkout(pool(handle)));
        // Long enough for the second checkout to be waiting.
        thread::sleep(Duration::from_millis(200));
        release.send(()).unwrap();

        let refused = first.join().unwrap().unwrap_err();
        assert!(refused.starts_with("pool connect: "), "{refused}");
        let second = second.join().unwrap().expect("the freed slot was taken");
        mesh_pool_checkin(handle, second);
        mesh_pool_close(handle);
    }

    /// A checkout waits its timeout in all, however often it wakes to find
    /// nothing to take.
    #[test]
    fn a_checkout_waits_no_longer_than_its_timeout_however_often_it_wakes() {
        let url = fake_server(|_, socket| serve(socket));
        let handle = pool_of(&url, Vec::new(), &[], 1, 1000);
        let held = checkout(pool(handle)).unwrap();
        let waiter = thread::spawn(move || {
            let started = Instant::now();
            (checkout(pool(handle)), started.elapsed())
        });
        thread::sleep(Duration::from_millis(700));
        pool(handle).available.notify_one();

        let (result, waited) = waiter.join().unwrap();
        assert_eq!(result.unwrap_err(), "pool checkout timeout");
        assert!(waited < Duration::from_millis(1500), "waited {waited:?}");
        mesh_pool_checkin(handle, held);
        mesh_pool_close(handle);
    }

    /// An idle connection slow to answer its health check holds up only
    /// the checkout that took it.
    #[test]
    fn an_idle_connection_is_checked_without_holding_the_pool() {
        let (asked, asked_signal) = mpsc::channel::<()>();
        let (answer, answered) = mpsc::channel::<()>();
        let answered = parking_lot::Mutex::new(answered);
        let url = fake_server(move |_, mut socket| {
            socket.write_all(AUTHENTICATED).unwrap();
            assert_eq!(next_request(&mut socket), Some(b'Q'));
            asked.send(()).unwrap();
            answered.lock().recv().unwrap();
            socket.write_all(SELECTED).unwrap();
            while next_request(&mut socket).is_some() {}
        });
        let handle = pool_of(&url, vec![open(&url).unwrap()], &[], 1, 5000);
        let taker = thread::spawn(move || checkout(pool(handle)));
        asked_signal.recv().unwrap();

        let (closed, closed_signal) = mpsc::channel();
        thread::spawn(move || {
            mesh_pool_close(handle);
            closed.send(()).unwrap();
        });
        let closed_promptly = closed_signal.recv_timeout(Duration::from_secs(2)).is_ok();
        answer.send(()).unwrap();
        let conn = taker.join().unwrap().unwrap();
        // Back in a closed pool, it is closed.
        mesh_pool_checkin(handle, conn);

        assert!(closed_promptly, "closing waited for the health check");
        assert_eq!(pool(handle).inner.lock().total_created, 0);
    }

    /// An idle connection whose server went away is closed at checkout and
    /// a new one opened in its place.
    #[test]
    fn an_idle_connection_that_died_is_replaced() {
        let url = fake_server(|index, mut socket| {
            socket.write_all(AUTHENTICATED).unwrap();
            if index > 0 {
                serve_queries(socket);
            }
        });
        let handle = pool_of(&url, vec![open(&url).unwrap()], &[], 1, 5000);

        let conn = checkout(pool(handle)).unwrap();

        assert_eq!(pool(handle).inner.lock().total_created, 1);
        mesh_pool_checkin(handle, conn);
        mesh_pool_close(handle);
    }

    /// A connection that opens after its pool closed is closed, not lent.
    #[test]
    fn a_connection_opened_after_its_pool_closed_is_not_lent() {
        let (held, held_signal) = mpsc::channel::<()>();
        let (release, released) = mpsc::channel::<()>();
        let released = parking_lot::Mutex::new(released);
        let url = fake_server(move |_, socket| {
            held.send(()).unwrap();
            released.lock().recv().unwrap();
            serve(socket);
        });
        let handle = pool_of(&url, Vec::new(), &[], 1, 5000);
        let taker = thread::spawn(move || checkout(pool(handle)));
        held_signal.recv().unwrap();
        mesh_pool_close(handle);
        release.send(()).unwrap();

        assert_eq!(taker.join().unwrap().unwrap_err(), "pool is closed");
        assert_eq!(pool(handle).inner.lock().total_created, 0);
    }

    fn result(pointer: *mut u8) -> (u8, *mut u8) {
        let result = unsafe { &*(pointer as *const MeshResult) };
        (result.tag, result.value)
    }

    fn error_text(pointer: *mut u8) -> String {
        let (tag, value) = result(pointer);
        assert_eq!(tag, 1, "expected an error");
        unsafe {
            (*(value as *const crate::string::MeshString))
                .as_str()
                .to_string()
        }
    }

    fn test_database_url() -> String {
        std::env::var("MESH_TEST_DATABASE_URL").expect("MESH_TEST_DATABASE_URL is set")
    }

    fn open_pool(url: &str, min: i64, max: i64, timeout_ms: i64) -> *mut u8 {
        mesh_pool_open(mk_str(url.as_bytes()), min, max, timeout_ms)
    }

    /// A pool opens connections as they are asked for, up to its maximum;
    /// past it a checkout waits its timeout out; a connection that broke
    /// while lent is closed at checkin and replaced; a closed pool lends
    /// nothing.
    #[test]
    #[ignore = "requires MESH_TEST_DATABASE_URL (the coverage run starts a database)"]
    fn a_pool_grows_waits_and_replaces_a_dead_connection() {
        mesh_rt_init();
        let (tag, pool) = result(open_pool(&test_database_url(), 0, 1, 100));
        assert_eq!(tag, 0);
        let pool = unsafe { unbox_u64_payload(pool) };

        let (tag, first) = result(mesh_pool_checkout(pool));
        assert_eq!(tag, 0, "a connection is opened for the first checkout");
        let first = unsafe { unbox_u64_payload(first) };
        assert_eq!(
            error_text(mesh_pool_checkout(pool)),
            "pool checkout timeout"
        );

        // The connection ends its own session, and comes back broken.
        let terminate = mk_str(b"SELECT pg_terminate_backend(pg_backend_pid())");
        let _ = crate::db::pg::mesh_pg_execute(first, terminate, mesh_list_new());
        mesh_pool_checkin(pool, first);
        let (tag, second) = result(mesh_pool_checkout(pool));
        assert_eq!(tag, 0, "the dead connection is replaced");
        let second = unsafe { unbox_u64_payload(second) };
        let (tag, _) = result(crate::db::pg::mesh_pg_execute(
            second,
            mk_str(b"SELECT 1"),
            mesh_list_new(),
        ));
        assert_eq!(tag, 0);
        mesh_pool_checkin(pool, second);

        mesh_pool_close(pool);
        assert_eq!(error_text(mesh_pool_checkout(pool)), "pool is closed");
        assert_eq!(
            error_text(mesh_pool_execute(
                pool,
                mk_str(b"SELECT 1"),
                mesh_list_new()
            )),
            "pool is closed"
        );
    }

    /// Opening a pool whose connections cannot all be made closes the ones
    /// that were, and says why.
    #[test]
    #[ignore = "requires MESH_TEST_DATABASE_URL (the coverage run starts a database)"]
    fn a_pool_that_cannot_open_every_connection_closes_the_rest() {
        mesh_rt_init();
        let url = test_database_url();
        let (tag, admin) = result(open_pool(&url, 1, 1, 5000));
        assert_eq!(tag, 0);
        let admin = unsafe { unbox_u64_payload(admin) };
        for sql in [
            "DROP ROLE IF EXISTS mesh_pool_one_connection",
            "CREATE ROLE mesh_pool_one_connection LOGIN PASSWORD 'one' CONNECTION LIMIT 1",
        ] {
            let (tag, _) = result(mesh_pool_execute(
                admin,
                mk_str(sql.as_bytes()),
                mesh_list_new(),
            ));
            assert_eq!(tag, 0, "{sql}");
        }
        let (scheme, rest) = url.split_once("://").unwrap();
        let (_, host) = rest.split_once('@').unwrap();
        let limited = format!("{scheme}://mesh_pool_one_connection:one@{host}");

        let refused = error_text(open_pool(&limited, 2, 2, 5000));
        assert!(refused.starts_with("pool open: "), "{refused}");

        let _ = mesh_pool_execute(
            admin,
            mk_str(b"DROP ROLE mesh_pool_one_connection"),
            mesh_list_new(),
        );
        mesh_pool_close(admin);
    }

    #[test]
    #[ignore = "requires MESH_TEST_DATABASE_URL (the coverage run starts a database)"]
    fn test_pool_execute_postgres_round_trip() {
        mesh_rt_init();

        let database_url = std::env::var("MESH_TEST_DATABASE_URL")
            .expect("MESH_TEST_DATABASE_URL must be set for test_pool_execute_postgres_round_trip");
        let url = mk_str(database_url.as_bytes());
        let open_result = mesh_pool_open(url, 1, 2, 5000);
        let open = unsafe { &*(open_result as *const MeshResult) };
        assert_eq!(open.tag, 0, "pool open should succeed");
        let pool = unsafe { unbox_u64_payload(open.value) };

        let create_sql = mk_str(
            b"CREATE TEMP TABLE IF NOT EXISTS mesh_pool_smoke (id INTEGER PRIMARY KEY, name TEXT)",
        );
        let empty_params = mesh_list_new();
        let create_result = mesh_pool_execute(pool, create_sql, empty_params);
        let create = unsafe { &*(create_result as *const MeshResult) };
        assert_eq!(create.tag, 0, "pool execute should succeed");

        let insert_sql = mk_str(b"INSERT INTO mesh_pool_smoke (id, name) VALUES ($1, $2)");
        let mut insert_params = mesh_list_new();
        insert_params = mesh_list_append(insert_params, mk_str(b"1") as u64);
        insert_params = mesh_list_append(insert_params, mk_str(b"mesh") as u64);
        let insert_result = mesh_pool_execute(pool, insert_sql, insert_params);
        let insert = unsafe { &*(insert_result as *const MeshResult) };
        assert_eq!(insert.tag, 0, "pool insert should succeed");
        assert_eq!(
            unsafe { *(insert.value as *const i64) },
            1,
            "pool insert should affect one row"
        );

        mesh_pool_close(pool);
    }

    /// A connection left in a transaction goes back idle once rolled back,
    /// and is closed when it cannot be.
    #[test]
    fn checkin_rolls_back_a_transaction_left_open() {
        let url = fake_server(|index, mut socket| {
            socket.write_all(AUTHENTICATED).unwrap();
            while let Some(tag) = next_request(&mut socket) {
                if tag == b'Q' {
                    let reply: &[u8] = if index == 0 {
                        b"C\0\0\0\x0dROLLBACK\0Z\0\0\0\x05I"
                    } else {
                        b"E\0\0\0\x0eMrefused\0\0Z\0\0\0\x05E"
                    };
                    socket.write_all(reply).unwrap();
                }
            }
        });
        let kept = open(&url).unwrap();
        let refused = open(&url).unwrap();
        for conn in [kept, refused] {
            unsafe { (*(conn as *mut PgConn)).txn_status = b'T' };
        }
        let handle = pool_of(&url, Vec::new(), &[kept, refused], 2, 100);

        mesh_pool_checkin(handle, kept);
        mesh_pool_checkin(handle, refused);

        let inner = pool(handle).inner.lock();
        assert_eq!(
            (inner.idle.as_slice(), inner.total_created),
            (&[kept][..], 1)
        );
        assert_eq!(unsafe { (*(kept as *const PgConn)).txn_status }, b'I');
    }

    extern "C-unwind" fn column_count(row: u64) -> u64 {
        mesh_map_size(row as *mut u8) as u64
    }

    /// Each scoped call checks a connection out and back in; one left in a
    /// transaction comes back rolled back.
    #[test]
    #[ignore = "requires MESH_TEST_DATABASE_URL (the coverage run starts a database)"]
    fn the_scoped_api_lends_a_connection_for_each_call() {
        mesh_rt_init();
        let (tag, handle) = result(open_pool(&test_database_url(), 1, 1, 5000));
        assert_eq!(tag, 0);
        let handle = unsafe { unbox_u64_payload(handle) };
        let sql = mk_str(b"SELECT 1 AS a, 2 AS b");
        for query in [mesh_pool_query, mesh_pool_query_values] {
            let (tag, rows) = result(query(handle, sql, mesh_list_new()));
            assert_eq!((tag, mesh_list_length(rows)), (0, 1));
        }
        let (tag, decoded) = result(mesh_pool_query_as(
            handle,
            sql as *mut u8,
            mesh_list_new(),
            column_count as *mut u8,
            std::ptr::null_mut(),
        ));
        assert_eq!(tag, 0);
        assert_eq!(crate::collections::list::mesh_list_get(decoded, 0), 2);
        let (tag, count) = result(mesh_pool_execute_values(handle, sql, mesh_list_new()));
        assert_eq!((tag, unsafe { unbox_u64_payload(count) }), (0, 1));

        let (tag, _) = result(mesh_pool_execute(handle, mk_str(b"BEGIN"), mesh_list_new()));
        assert_eq!(tag, 0);
        let conn = checkout(pool(handle)).unwrap();
        assert_eq!(unsafe { (*(conn as *const PgConn)).txn_status }, b'I');
        mesh_pool_checkin(handle, conn);
        assert_eq!(pool(handle).inner.lock().idle.len(), 1);
        mesh_pool_close(handle);
    }
}
