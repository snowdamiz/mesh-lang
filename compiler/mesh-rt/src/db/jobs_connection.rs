use std::net::{Shutdown, TcpStream};
use std::time::Duration;

use md5::{Digest, Md5};
use parking_lot::{Condvar, Mutex};
use socket2::{SockRef, TcpKeepalive};

use super::pg::{
    commit, connect, connect_with_socket, pg_jobs_ack, pg_simple_command, pg_text_query,
    JobsAckError, PgConn, PgHandle, PgNotifications,
};
use super::pool::{connection_url, NativeLease};

const IO_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) struct PgJobDb<'a>(&'a mut PgConn);

pub(crate) fn wake_key(name: &str) -> String {
    format!("{:x}", Md5::digest(name.as_bytes()))
}

#[derive(Default)]
pub(crate) struct ListenerStop {
    state: Mutex<ListenerSocket>,
    changed: Condvar,
}

#[derive(Default)]
struct ListenerSocket {
    stopped: bool,
    socket: Option<TcpStream>,
}

impl ListenerStop {
    fn attach(&self, socket: &TcpStream) -> Result<(), String> {
        let mut state = self.state.lock();
        if state.stopped {
            return Err("Jobs listener stopped".into());
        }
        state.socket = Some(socket.try_clone().map_err(|error| error.to_string())?);
        let keepalive = TcpKeepalive::new().with_time(Duration::from_secs(30));
        #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
        let keepalive = keepalive.with_interval(Duration::from_secs(5));
        let _ = SockRef::from(socket).set_tcp_keepalive(&keepalive);
        Ok(())
    }

    pub(crate) fn stop(&self) {
        let mut state = self.state.lock();
        state.stopped = true;
        if let Some(socket) = state.socket.take() {
            // Also interrupts TLS/authentication and a pending LISTEN.
            let _ = socket.shutdown(Shutdown::Both);
        }
        self.changed.notify_all();
    }

    fn stopped(&self) -> bool {
        self.state.lock().stopped
    }

    fn wait_to_retry(&self, delay: Duration) {
        let mut state = self.state.lock();
        state.socket = None;
        if !state.stopped {
            self.changed.wait_for(&mut state, delay);
        }
    }
}

pub(crate) struct JobListener {
    url: String,
    channel: String,
}

pub(crate) enum ListenEvent<'a> {
    Subscribed,
    Disconnected,
    Hint(&'a str),
}

impl JobListener {
    pub(crate) unsafe fn from_pool(pool: u64, namespace: &str) -> Result<Self, String> {
        Ok(Self::new(connection_url(pool)?, namespace))
    }

    pub(super) fn new(url: String, namespace: &str) -> Self {
        Self {
            url,
            channel: format!("mesh_jobs_wake_{}", wake_key(namespace)),
        }
    }

    pub(crate) fn run(self, stop: &ListenerStop, mut on_event: impl FnMut(ListenEvent<'_>)) {
        let mut retry = Duration::from_millis(100);
        while !stop.stopped() {
            let subscribed = connect_with_socket(&self.url, |socket| stop.attach(socket))
                .and_then(|conn| PgNotifications::subscribe(conn, &self.channel));
            if let Ok(mut listener) = subscribed {
                if stop.stopped() {
                    return;
                }
                on_event(ListenEvent::Subscribed);
                retry = Duration::from_millis(100);
                while !stop.stopped() {
                    match listener.read() {
                        Ok(Some(note)) if note.channel == self.channel => {
                            on_event(ListenEvent::Hint(&note.payload));
                        }
                        Ok(_) => {}
                        Err(_) => break,
                    }
                }
                on_event(ListenEvent::Disconnected);
            }
            stop.wait_to_retry(retry);
            retry = (retry * 2).min(Duration::from_secs(5));
        }
    }
}

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

    pub(crate) fn acknowledge(
        &mut self,
        sql: &str,
        params: &[&str],
    ) -> Result<Vec<Vec<(String, String)>>, JobsAckError> {
        self.timed(|conn| pg_jobs_ack(conn, sql, params))
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

    fn timed<T, E>(&mut self, run: impl FnOnce(&mut PgConn) -> Result<T, E>) -> Result<T, E> {
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
    // A native Jobs operation holds the socket for the whole database call,
    // including its per-request deadlines and transaction boundaries.
    PgHandle::from_handle(connection).direct(|conn| run(&mut PgJobDb(conn)))
}
