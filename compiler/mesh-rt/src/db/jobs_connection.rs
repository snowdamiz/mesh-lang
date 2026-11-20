use std::time::Duration;

use super::pg::{commit, connect, pg_simple_command, pg_text_query, PgConn};
use super::pool::{connection_url, NativeLease};

const IO_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) struct PgJobDb<'a>(&'a mut PgConn);

/// A runner session with a dedicated socket outside the application pool.
pub(crate) struct JobSession {
    url: String,
    connection: Option<PgConn>,
}

impl JobSession {
    pub(crate) unsafe fn from_pool(pool: u64) -> Result<Self, String> {
        Ok(Self {
            url: connection_url(pool)?,
            connection: None,
        })
    }

    pub(crate) fn with<T>(
        &mut self,
        run: impl FnOnce(&mut PgJobDb<'_>) -> Result<T, String>,
    ) -> Result<T, String> {
        if self.connection.is_none() {
            self.connection = Some(connect(&self.url)?);
        }
        let connection = self.connection.as_mut().expect("connected session");
        let result = run(&mut PgJobDb(connection));
        if connection.is_broken() || connection.txn_status != b'I' {
            self.connection = None;
        }
        result
    }
}

impl PgJobDb<'_> {
    pub(crate) fn query(
        &mut self,
        sql: &str,
        params: &[&str],
    ) -> Result<Vec<Vec<(String, String)>>, String> {
        self.timed(|conn| pg_text_query(conn, sql, params))
    }

    pub(crate) fn simple(&mut self, sql: &str) -> Result<(), String> {
        self.timed(|conn| pg_simple_command(conn, sql))
    }

    pub(crate) fn in_transaction(&self) -> bool {
        self.0.txn_status == b'T'
    }

    pub(crate) fn transaction<T>(
        &mut self,
        run: impl FnOnce(&mut Self) -> Result<T, String>,
    ) -> Result<T, String> {
        if self.0.txn_status != b'I' {
            return Err("Jobs requires a PostgreSQL connection outside a transaction".into());
        }
        self.simple("BEGIN")?;
        let result = run(self).and_then(|value| {
            if !self.in_transaction() {
                return Err("Jobs operation did not leave a committable transaction".into());
            }
            self.timed(commit).map(|()| value)
        });
        if result.is_err() && self.0.txn_status != b'I' {
            let _ = self.simple("ROLLBACK");
        }
        result
    }

    fn timed<T>(
        &mut self,
        run: impl FnOnce(&mut PgConn) -> Result<T, String>,
    ) -> Result<T, String> {
        let (read_timeout, write_timeout) = self.0.socket_timeouts();
        self.0.set_read_timeout(Some(IO_TIMEOUT));
        self.0.set_write_timeout(Some(IO_TIMEOUT));
        let result = run(self.0);
        self.0.set_read_timeout(read_timeout);
        self.0.set_write_timeout(write_timeout);
        result
    }
}

pub(crate) unsafe fn with_pool<T>(
    pool: u64,
    run: impl FnOnce(&mut PgJobDb<'_>) -> Result<T, String>,
) -> Result<T, String> {
    let lease = NativeLease::checkout(pool)?;
    with_connection(lease.connection, run)
}

pub(crate) unsafe fn with_connection<T>(
    connection: u64,
    run: impl FnOnce(&mut PgJobDb<'_>) -> Result<T, String>,
) -> Result<T, String> {
    if connection == 0 {
        return Err("invalid PostgreSQL connection".into());
    }
    run(&mut PgJobDb(&mut *(connection as *mut PgConn)))
}
