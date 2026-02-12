//! PostgreSQL connection pool for the Snow runtime.
//!
//! Provides a bounded pool of PostgreSQL connections with Mutex+Condvar
//! synchronization. Multiple Snow actors share a fixed set of database
//! connections, preventing connection exhaustion.
//!
//! ## Functions
//!
//! - `snow_pool_open`: Create a pool with configurable min/max/timeout
//! - `snow_pool_checkout`: Borrow a connection (blocks with timeout)
//! - `snow_pool_checkin`: Return a connection (auto-ROLLBACK if dirty)
//! - `snow_pool_query`: Auto checkout-use-checkin for SELECT
//! - `snow_pool_execute`: Auto checkout-use-checkin for INSERT/UPDATE/DELETE
//! - `snow_pool_close`: Drain all connections, prevent new checkouts
//!
//! Pool handles are opaque u64 values (Box::into_raw), same pattern as
//! PgConn/SqliteConn handles.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use parking_lot::{Condvar, Mutex};

use super::pg::{
    pg_simple_command, snow_pg_close, snow_pg_connect, snow_pg_execute, snow_pg_query,
    snow_pg_query_as, PgConn,
};
use crate::io::alloc_result;
use crate::string::{snow_string_new, SnowString};

// ── Data Structures ──────────────────────────────────────────────────────

struct PooledConn {
    handle: u64,
    #[allow(dead_code)]
    last_used: Instant,
}

struct PoolInner {
    url: String,
    idle: VecDeque<PooledConn>,
    active_count: usize,
    total_created: usize,
    #[allow(dead_code)]
    min_conns: usize,
    max_conns: usize,
    checkout_timeout_ms: u64,
    closed: bool,
}

struct PgPool {
    inner: Mutex<PoolInner>,
    available: Condvar,
}

// ── Helpers ──────────────────────────────────────────────────────────────

/// Extract a Rust &str from a raw SnowString pointer.
unsafe fn snow_str_to_rust(s: *const SnowString) -> &'static str {
    (*s).as_str()
}

/// Create a SnowString from a Rust &str and return as *mut u8.
fn rust_str_to_snow(s: &str) -> *mut u8 {
    snow_string_new(s.as_ptr(), s.len() as u64) as *mut u8
}

/// Create an error SnowResult from a Rust string.
fn err_result(msg: &str) -> *mut u8 {
    let s = rust_str_to_snow(msg);
    alloc_result(1, s) as *mut u8
}

/// Create a new PG connection from a URL string.
/// Returns Ok(handle_u64) or Err(error_message).
unsafe fn create_connection(url: &str) -> Result<u64, String> {
    // Create a SnowString from the URL
    let url_snow = snow_string_new(url.as_ptr(), url.len() as u64);
    let result_ptr = snow_pg_connect(url_snow as *const SnowString);
    let r = &*(result_ptr as *const crate::io::SnowResult);
    if r.tag == 0 {
        Ok(r.value as u64)
    } else {
        // Extract the error message string from the SnowString value
        let err_str = &*(r.value as *const SnowString);
        Err(err_str.as_str().to_string())
    }
}

/// Perform a health check on a connection by sending SELECT 1.
/// Returns true if healthy, false if dead.
unsafe fn health_check(handle: u64) -> bool {
    let conn = &mut *(handle as *mut PgConn);
    pg_simple_command(conn, "SELECT 1").is_ok()
}

// ── Public API ───────────────────────────────────────────────────────────

/// Create a PostgreSQL connection pool.
///
/// # Signature
///
/// `snow_pool_open(url: *const SnowString, min_conns: i64, max_conns: i64,
///     timeout_ms: i64) -> *mut u8 (SnowResult<u64, String>)`
///
/// Pre-creates `min_conns` connections. Returns SnowResult with tag 0 (Ok)
/// containing the pool handle as u64, or tag 1 (Err) with error message.
#[no_mangle]
pub extern "C" fn snow_pool_open(
    url: *const SnowString,
    min_conns: i64,
    max_conns: i64,
    timeout_ms: i64,
) -> *mut u8 {
    unsafe {
        let url_str = snow_str_to_rust(url);

        // Clamp parameters to reasonable values
        let min = (min_conns.max(0)) as usize;
        let max = (max_conns.max(1)) as usize;
        let max = max.max(min.max(1));
        let timeout = (timeout_ms.max(100)) as u64;

        // Pre-create min_conns connections
        let mut idle = VecDeque::with_capacity(min);
        for _ in 0..min {
            match create_connection(url_str) {
                Ok(handle) => {
                    idle.push_back(PooledConn {
                        handle,
                        last_used: Instant::now(),
                    });
                }
                Err(e) => {
                    // Close all already-created connections
                    for c in idle.drain(..) {
                        snow_pg_close(c.handle);
                    }
                    return err_result(&format!("pool open: {}", e));
                }
            }
        }

        let pool = Box::new(PgPool {
            inner: Mutex::new(PoolInner {
                url: url_str.to_string(),
                idle,
                active_count: 0,
                total_created: min,
                min_conns: min,
                max_conns: max,
                checkout_timeout_ms: timeout,
                closed: false,
            }),
            available: Condvar::new(),
        });

        let handle = Box::into_raw(pool) as u64;
        alloc_result(0, handle as *mut u8) as *mut u8
    }
}

/// Check out a connection from the pool.
///
/// # Signature
///
/// `snow_pool_checkout(pool_handle: u64) -> *mut u8 (SnowResult<u64, String>)`
///
/// Returns an idle connection, creates a new one if under max, or blocks
/// with timeout if pool is exhausted. Performs health check on idle
/// connections before returning them.
#[no_mangle]
pub extern "C" fn snow_pool_checkout(pool_handle: u64) -> *mut u8 {
    unsafe {
        let pool = &*(pool_handle as *const PgPool);
        let timeout = Duration::from_millis({
            let inner = pool.inner.lock();
            inner.checkout_timeout_ms
        });

        let mut inner = pool.inner.lock();

        loop {
            // Check if pool is closed
            if inner.closed {
                return err_result("pool is closed");
            }

            // Try to get an idle connection
            if let Some(conn) = inner.idle.pop_front() {
                // Health check: validate connection before returning
                if health_check(conn.handle) {
                    inner.active_count += 1;
                    return alloc_result(0, conn.handle as *mut u8) as *mut u8;
                } else {
                    // Connection is dead -- close it and try next
                    snow_pg_close(conn.handle);
                    inner.total_created -= 1;
                    continue;
                }
            }

            // No idle connections -- can we create a new one?
            if inner.total_created < inner.max_conns {
                // Optimistically reserve a slot
                inner.total_created += 1;
                inner.active_count += 1;
                let url = inner.url.clone();
                // Drop the lock before doing I/O (connection creation)
                drop(inner);

                match create_connection(&url) {
                    Ok(handle) => {
                        return alloc_result(0, handle as *mut u8) as *mut u8;
                    }
                    Err(e) => {
                        // Undo the reservation
                        let mut inner = pool.inner.lock();
                        inner.total_created -= 1;
                        inner.active_count -= 1;
                        return err_result(&format!("pool connect: {}", e));
                    }
                }
            }

            // All connections busy, wait with timeout
            let wait_result = pool.available.wait_for(&mut inner, timeout);
            if wait_result.timed_out() {
                return err_result("pool checkout timeout");
            }
            // Loop back to try again after being notified
        }
    }
}

/// Return a connection to the pool.
///
/// # Signature
///
/// `snow_pool_checkin(pool_handle: u64, conn_handle: u64)`
///
/// If the connection has an active transaction (txn_status != 'I'),
/// sends ROLLBACK to clean it up. If ROLLBACK fails, the connection
/// is destroyed instead of returned to idle.
#[no_mangle]
pub extern "C" fn snow_pool_checkin(pool_handle: u64, conn_handle: u64) {
    unsafe {
        let pool = &*(pool_handle as *const PgPool);

        {
            let inner = pool.inner.lock();
            if inner.closed {
                // Pool is closed -- just destroy the connection
                snow_pg_close(conn_handle);
                // Note: total_created/active_count will be cleaned up by close
                return;
            }
        }

        // Transaction cleanup (POOL-05): ROLLBACK if not idle
        let conn = &mut *(conn_handle as *mut PgConn);
        if conn.txn_status != b'I' {
            if pg_simple_command(conn, "ROLLBACK").is_err() {
                // Connection is broken -- close it instead of returning to idle
                snow_pg_close(conn_handle);
                let mut inner = pool.inner.lock();
                inner.total_created -= 1;
                inner.active_count -= 1;
                pool.available.notify_one();
                return;
            }
        }

        // Return to idle
        let mut inner = pool.inner.lock();
        inner.active_count -= 1;
        inner.idle.push_back(PooledConn {
            handle: conn_handle,
            last_used: Instant::now(),
        });
        // Drop the lock before notifying
        drop(inner);
        pool.available.notify_one();
    }
}

/// Execute a read query (SELECT) with automatic checkout-use-checkin.
///
/// # Signature
///
/// `snow_pool_query(pool_handle: u64, sql: *const SnowString, params: *mut u8)
///     -> *mut u8 (SnowResult<List<Map<String, String>>, String>)`
#[no_mangle]
pub extern "C" fn snow_pool_query(
    pool_handle: u64,
    sql: *const SnowString,
    params: *mut u8,
) -> *mut u8 {
    unsafe {
        // Checkout
        let checkout_result = snow_pool_checkout(pool_handle);
        let r = &*(checkout_result as *const crate::io::SnowResult);
        if r.tag != 0 {
            return checkout_result; // propagate checkout error
        }
        let conn_handle = r.value as u64;

        // Use
        let query_result = snow_pg_query(conn_handle, sql, params);

        // Checkin (always, even on error)
        snow_pool_checkin(pool_handle, conn_handle);

        query_result
    }
}

/// Execute a write statement (INSERT/UPDATE/DELETE) with automatic checkout-use-checkin.
///
/// # Signature
///
/// `snow_pool_execute(pool_handle: u64, sql: *const SnowString, params: *mut u8)
///     -> *mut u8 (SnowResult<Int, String>)`
#[no_mangle]
pub extern "C" fn snow_pool_execute(
    pool_handle: u64,
    sql: *const SnowString,
    params: *mut u8,
) -> *mut u8 {
    unsafe {
        // Checkout
        let checkout_result = snow_pool_checkout(pool_handle);
        let r = &*(checkout_result as *const crate::io::SnowResult);
        if r.tag != 0 {
            return checkout_result; // propagate checkout error
        }
        let conn_handle = r.value as u64;

        // Use
        let exec_result = snow_pg_execute(conn_handle, sql, params);

        // Checkin (always, even on error)
        snow_pool_checkin(pool_handle, conn_handle);

        exec_result
    }
}

/// Execute a SELECT query with automatic checkout-use-checkin and map rows through a callback.
///
/// # Signature
///
/// `snow_pool_query_as(pool_handle: u64, sql: *mut u8, params: *mut u8,
///     from_row_fn: *mut u8) -> *mut u8 (SnowResult<List<SnowResult>, String>)`
///
/// Same checkout/query_as/checkin pattern as `snow_pool_query` but delegates to
/// `snow_pg_query_as` for struct mapping.
#[no_mangle]
pub extern "C" fn snow_pool_query_as(
    pool_handle: u64,
    sql: *mut u8,
    params: *mut u8,
    from_row_fn: *mut u8,
) -> *mut u8 {
    unsafe {
        // Checkout
        let checkout_result = snow_pool_checkout(pool_handle);
        let r = &*(checkout_result as *const crate::io::SnowResult);
        if r.tag != 0 {
            return checkout_result; // propagate checkout error
        }
        let conn_handle = r.value as u64;

        // Use
        let query_result = snow_pg_query_as(conn_handle, sql, params, from_row_fn);

        // Checkin (always, even on error)
        snow_pool_checkin(pool_handle, conn_handle);

        query_result
    }
}

/// Close a connection pool.
///
/// # Signature
///
/// `snow_pool_close(pool_handle: u64)`
///
/// Sets pool to closed state, drains all idle connections, and wakes
/// all blocked checkouts so they return "pool is closed" errors.
/// Active connections will be closed when checked in.
#[no_mangle]
pub extern "C" fn snow_pool_close(pool_handle: u64) {
    unsafe {
        let pool = &*(pool_handle as *const PgPool);
        let idle_conns: Vec<u64>;

        {
            let mut inner = pool.inner.lock();
            inner.closed = true;
            // Drain all idle connections
            idle_conns = inner.idle.drain(..).map(|c| c.handle).collect();
        }

        // Close idle connections outside the lock
        for handle in idle_conns {
            snow_pg_close(handle);
        }

        // Wake all blocked checkouts so they see closed=true
        pool.available.notify_all();
    }
}
