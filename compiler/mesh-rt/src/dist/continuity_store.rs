//! Durable, bounded continuity storage with resumable snapshot chunks.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::ffi::{CStr, CString};
use std::os::raw::{c_int, c_void};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use libsqlite3_sys::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const SCHEMA_VERSION: u32 = 1;
const SQLITE_TRANSIENT_VALUE: isize = -1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoredContinuityPhase {
    Reserved,
    Replicating,
    Admitted,
    Started,
    Completed,
    Failed,
    Indeterminate,
    Expired,
    Tombstoned,
}

impl StoredContinuityPhase {
    fn as_str(self) -> &'static str {
        match self {
            Self::Reserved => "reserved",
            Self::Replicating => "replicating",
            Self::Admitted => "admitted",
            Self::Started => "started",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Indeterminate => "indeterminate",
            Self::Expired => "expired",
            Self::Tombstoned => "tombstoned",
        }
    }

    fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "reserved" => Ok(Self::Reserved),
            "replicating" => Ok(Self::Replicating),
            "admitted" => Ok(Self::Admitted),
            "started" => Ok(Self::Started),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "indeterminate" => Ok(Self::Indeterminate),
            "expired" => Ok(Self::Expired),
            "tombstoned" => Ok(Self::Tombstoned),
            _ => Err(format!("continuity_store_phase_invalid:{raw}")),
        }
    }

    pub const fn is_active(self) -> bool {
        matches!(
            self,
            Self::Reserved | Self::Replicating | Self::Admitted | Self::Started
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredContinuityRecord {
    pub operation_key: String,
    pub request_hash: String,
    #[serde(default)]
    pub request_body: Vec<u8>,
    /// Complete versioned runtime record used to rehydrate in-flight state.
    #[serde(default)]
    pub runtime_record: Vec<u8>,
    pub owner_node: String,
    pub ownership_generation: u64,
    pub attempts: Vec<String>,
    pub phase: StoredContinuityPhase,
    pub replica_set: Vec<String>,
    pub created_at_millis: u64,
    pub updated_at_millis: u64,
    pub terminal_at_millis: Option<u64>,
    pub expires_at_millis: Option<u64>,
    pub response_metadata: Vec<(String, String)>,
    pub response_body: Vec<u8>,
    pub control_term: u64,
    pub schema_version: u32,
    pub version: u64,
}

impl StoredContinuityRecord {
    pub fn validate(&self) -> Result<(), String> {
        if self.operation_key.is_empty()
            || self.request_hash.is_empty()
            || self.owner_node.is_empty()
            || self.attempts.iter().any(String::is_empty)
        {
            return Err("continuity_store_record_identity_invalid".to_string());
        }
        if self.schema_version != SCHEMA_VERSION || self.version == 0 {
            return Err("continuity_store_record_version_invalid".to_string());
        }
        let mut replicas = self.replica_set.clone();
        replicas.sort();
        replicas.dedup();
        if replicas.len() != self.replica_set.len()
            || replicas.iter().any(|replica| replica == &self.owner_node)
        {
            return Err("continuity_store_replica_set_invalid".to_string());
        }
        if self.phase.is_active() && self.terminal_at_millis.is_some() {
            return Err("continuity_store_active_record_terminal_timestamp".to_string());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ContinuityStoreLimits {
    pub terminal_retention_millis: u64,
    pub tombstone_retention_millis: u64,
    pub max_terminal_records: u64,
    pub max_disk_bytes: u64,
    pub compaction_batch_size: u32,
}

impl Default for ContinuityStoreLimits {
    fn default() -> Self {
        Self {
            terminal_retention_millis: 86_400_000,
            tombstone_retention_millis: 172_800_000,
            max_terminal_records: 1_000_000,
            max_disk_bytes: 8 * 1024 * 1024 * 1024,
            compaction_batch_size: 1_000,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompactionOutcome {
    pub records_tombstoned: u32,
    pub tombstones_deleted: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContinuityStoreStats {
    pub records: u64,
    pub active_records: u64,
    pub terminal_records: u64,
    pub tombstones: u64,
    pub log_entries: u64,
    pub high_water_mark: u64,
    pub disk_bytes: u64,
    #[serde(default)]
    pub replica_safe_point: Option<u64>,
    #[serde(default)]
    pub compaction_lag: u64,
    #[serde(default)]
    pub replication_lag: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContinuityNodeSafety {
    pub active_owned_records: u32,
    pub required_replica_responsibilities: u32,
    pub only_active_copy: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotChunk {
    pub snapshot_id: String,
    pub sequence: u32,
    pub final_chunk: bool,
    pub high_water_mark: u64,
    pub payload: Vec<u8>,
    pub checksum: [u8; 32],
    /// Digest of the ordered per-chunk digests for the complete snapshot.
    pub snapshot_checksum: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContinuityLogEntry {
    pub sequence: u64,
    pub operation_key: String,
    pub version: u64,
    pub record: StoredContinuityRecord,
    pub checksum: [u8; 32],
}

impl ContinuityLogEntry {
    pub fn verify(&self) -> bool {
        serde_json::to_vec(&self.record)
            .map(|encoded| <[u8; 32]>::from(Sha256::digest(encoded)) == self.checksum)
            .unwrap_or(false)
            && self.operation_key == self.record.operation_key
            && self.version == self.record.version
    }
}

impl SnapshotChunk {
    pub fn verify(&self) -> bool {
        <[u8; 32]>::from(Sha256::digest(&self.payload)) == self.checksum
    }
}

pub trait ContinuityStore: Send + Sync {
    fn get(&self, operation_key: &str) -> Result<Option<StoredContinuityRecord>, String>;
    fn upsert(&self, record: &StoredContinuityRecord) -> Result<(), String>;
    fn compact(&self, now_millis: u64) -> Result<CompactionOutcome, String>;
    fn snapshot_chunks(&self, chunk_bytes: usize) -> Result<Vec<SnapshotChunk>, String>;
    fn apply_snapshot_chunk(&self, chunk: &SnapshotChunk) -> Result<(), String>;
    fn high_water_mark(&self) -> Result<u64, String>;
    fn log_entries_after(
        &self,
        high_water_mark: u64,
        limit: u32,
    ) -> Result<Vec<ContinuityLogEntry>, String>;
    fn apply_log_entry(&self, entry: &ContinuityLogEntry) -> Result<(), String>;
    fn acknowledge_replica_safe_point(
        &self,
        replica_node: &str,
        high_water_mark: u64,
    ) -> Result<(), String>;
    fn compact_log_to_replica_safe_point(&self) -> Result<u64, String>;
}

#[derive(Debug)]
struct Connection {
    raw: *mut sqlite3,
}

unsafe impl Send for Connection {}

impl Drop for Connection {
    fn drop(&mut self) {
        // A Connection holds only a handle sqlite3_open_v2 opened.
        unsafe {
            sqlite3_close(self.raw);
        }
    }
}

struct Statement {
    database: *mut sqlite3,
    raw: *mut sqlite3_stmt,
}

impl Drop for Statement {
    fn drop(&mut self) {
        // Finalizing a NULL statement (SQL that prepares to nothing) is a
        // no-op.
        unsafe {
            sqlite3_finalize(self.raw);
        }
    }
}

impl Statement {
    fn bind_text(&mut self, index: c_int, value: &str) -> Result<(), String> {
        let value =
            CString::new(value).map_err(|_| "continuity_store_value_contains_nul".to_string())?;
        let result =
            unsafe { sqlite3_bind_text(self.raw, index, value.as_ptr(), -1, sqlite_transient()) };
        check_sqlite(self.database, result)
    }

    fn bind_i64(&mut self, index: c_int, value: i64) -> Result<(), String> {
        check_sqlite(self.database, unsafe {
            sqlite3_bind_int64(self.raw, index, value)
        })
    }

    fn bind_blob(&mut self, index: c_int, value: &[u8]) -> Result<(), String> {
        let length = c_int::try_from(value.len())
            .map_err(|_| "continuity_store_blob_too_large".to_string())?;
        check_sqlite(self.database, unsafe {
            sqlite3_bind_blob(
                self.raw,
                index,
                value.as_ptr().cast::<c_void>(),
                length,
                sqlite_transient(),
            )
        })
    }

    fn bind_optional_i64(&mut self, index: c_int, value: Option<u64>) -> Result<(), String> {
        match value {
            Some(value) => self.bind_i64(index, sqlite_integer(value)?),
            None => check_sqlite(self.database, unsafe { sqlite3_bind_null(self.raw, index) }),
        }
    }

    fn step(&mut self) -> Result<c_int, String> {
        let result = unsafe { sqlite3_step(self.raw) };
        if matches!(result, SQLITE_ROW | SQLITE_DONE) {
            Ok(result)
        } else {
            Err(sqlite_error(self.database))
        }
    }

    /// A text column's value: the bytes of its UTF-8 text.
    fn text(&self, column: c_int) -> String {
        String::from_utf8_lossy(&self.blob(column)).into_owned()
    }

    fn integer(&self, column: c_int) -> i64 {
        unsafe { sqlite3_column_int64(self.raw, column) }
    }

    fn optional_integer(&self, column: c_int) -> Option<i64> {
        (unsafe { sqlite3_column_type(self.raw, column) } != SQLITE_NULL)
            .then(|| self.integer(column))
    }

    fn blob(&self, column: c_int) -> Vec<u8> {
        let pointer = unsafe { sqlite3_column_blob(self.raw, column) };
        let length = unsafe { sqlite3_column_bytes(self.raw, column) }.max(0) as usize;
        if pointer.is_null() || length == 0 {
            Vec::new()
        } else {
            unsafe { std::slice::from_raw_parts(pointer.cast::<u8>(), length) }.to_vec()
        }
    }
}

fn sqlite_transient() -> Option<unsafe extern "C" fn(*mut c_void)> {
    unsafe {
        std::mem::transmute::<isize, Option<unsafe extern "C" fn(*mut c_void)>>(
            SQLITE_TRANSIENT_VALUE,
        )
    }
}

#[derive(Debug)]
pub struct SqliteContinuityStore {
    path: PathBuf,
    connection: Mutex<Connection>,
    limits: ContinuityStoreLimits,
}

struct PreparedContinuityRecord<'a> {
    record: &'a StoredContinuityRecord,
    serialized: Vec<u8>,
    checksum: Vec<u8>,
    attempts: String,
    replicas: String,
    response_metadata: String,
}

impl SqliteContinuityStore {
    pub fn open(path: &Path, limits: ContinuityStoreLimits) -> Result<Self, String> {
        if limits.max_terminal_records == 0
            || limits.max_disk_bytes == 0
            || limits.compaction_batch_size == 0
            || limits.tombstone_retention_millis <= limits.terminal_retention_millis
        {
            return Err("continuity_store_limits_invalid".to_string());
        }
        if path != Path::new(":memory:") {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|error| format!("continuity_store_directory_failed:{error}"))?;
            }
        }
        let c_path = CString::new(path.to_string_lossy().as_bytes())
            .map_err(|_| "continuity_store_path_contains_nul".to_string())?;
        let mut raw = ptr::null_mut();
        let result = unsafe {
            sqlite3_open_v2(
                c_path.as_ptr(),
                &mut raw,
                SQLITE_OPEN_READWRITE | SQLITE_OPEN_CREATE | SQLITE_OPEN_FULLMUTEX,
                ptr::null(),
            )
        };
        if result != SQLITE_OK {
            let error = sqlite_error(raw);
            if !raw.is_null() {
                unsafe { sqlite3_close(raw) };
            }
            return Err(error);
        }
        let store = Self {
            path: path.to_path_buf(),
            connection: Mutex::new(Connection { raw }),
            limits,
        };
        store.initialize_schema()?;
        if path != Path::new(":memory:") {
            secure_database_file(path)?;
        }
        Ok(store)
    }

    pub fn stats(&self) -> Result<ContinuityStoreStats, String> {
        let connection = self.connection.lock().unwrap();
        let count = |sql: &str| Self::query_u64(&connection, sql, None);
        let records = count("SELECT COUNT(*) FROM continuity_records")?;
        let active_records = count(
            "SELECT COUNT(*) FROM continuity_records
             WHERE phase IN ('reserved', 'replicating', 'admitted', 'started')",
        )?;
        let terminal_records = Self::terminal_records(&connection)?;
        let tombstones = count("SELECT COUNT(*) FROM continuity_tombstones")?;
        let log_entries = count("SELECT COUNT(*) FROM continuity_log")?;
        let high_water_mark = Self::high_water(&connection)?;
        let replica_safe_point = Self::replica_safe_point(&connection)?;
        // Log sequences start at 1: with no safe point nothing is eligible.
        let compaction_lag = Self::query_u64(
            &connection,
            "SELECT COUNT(*) FROM continuity_log WHERE sequence <= ?1",
            Some(replica_safe_point.unwrap_or(0)),
        )?;
        let replication_lag =
            replica_safe_point.map(|safe_point| high_water_mark.saturating_sub(safe_point));
        drop(connection);
        Ok(ContinuityStoreStats {
            records,
            active_records,
            terminal_records,
            tombstones,
            log_entries,
            high_water_mark,
            disk_bytes: self.disk_bytes(),
            replica_safe_point,
            compaction_lag,
            replication_lag,
        })
    }

    /// The one integer an aggregate query returns: it always has a row.
    fn query_u64(
        connection: &Connection,
        sql: &str,
        parameter: Option<u64>,
    ) -> Result<u64, String> {
        let mut statement = Self::prepare(connection, sql)?;
        if let Some(parameter) = parameter {
            statement.bind_i64(1, sqlite_integer(parameter)?)?;
        }
        statement.step()?;
        unsigned_integer(statement.integer(0))
    }

    fn high_water(connection: &Connection) -> Result<u64, String> {
        Self::query_u64(
            connection,
            "SELECT COALESCE(MAX(sequence), 0) FROM continuity_log",
            None,
        )
    }

    fn terminal_records(connection: &Connection) -> Result<u64, String> {
        Self::query_u64(
            connection,
            "SELECT COALESCE(MAX(terminal_record_count), 0) FROM continuity_store_counters",
            None,
        )
    }

    /// The lowest high-water mark every replica has acknowledged, once any
    /// replica has acknowledged one.
    fn replica_safe_point(connection: &Connection) -> Result<Option<u64>, String> {
        let replicas = Self::query_u64(
            connection,
            "SELECT COUNT(*) FROM continuity_replica_safe_points",
            None,
        )?;
        if replicas == 0 {
            return Ok(None);
        }
        Self::query_u64(
            connection,
            "SELECT MIN(high_water_mark) FROM continuity_replica_safe_points",
            None,
        )
        .map(Some)
    }

    fn prepare_upsert<'a>(
        record: &'a StoredContinuityRecord,
    ) -> Result<PreparedContinuityRecord<'a>, String> {
        record.validate()?;
        let serialized = serde_json::to_vec(record)
            .map_err(|error| format!("continuity_store_record_encode_failed:{error}"))?;
        let checksum = Sha256::digest(&serialized).to_vec();
        let attempts = serde_json::to_string(&record.attempts)
            .map_err(|_| "continuity_store_attempts_encode_failed".to_string())?;
        let replicas = serde_json::to_string(&record.replica_set)
            .map_err(|_| "continuity_store_replicas_encode_failed".to_string())?;
        let response_metadata = serde_json::to_string(&record.response_metadata)
            .map_err(|_| "continuity_store_response_metadata_encode_failed".to_string())?;
        Ok(PreparedContinuityRecord {
            record,
            serialized,
            checksum,
            attempts,
            replicas,
            response_metadata,
        })
    }

    fn apply_prepared_upsert(
        connection: &Connection,
        prepared: &PreparedContinuityRecord<'_>,
    ) -> Result<(), String> {
        let record = prepared.record;
        let mut tombstone = Self::prepare(
            connection,
            "SELECT version FROM continuity_tombstones WHERE operation_key = ?1",
        )?;
        tombstone.bind_text(1, &record.operation_key)?;
        if tombstone.step()? == SQLITE_ROW
            && unsigned_integer(tombstone.integer(0))? >= record.version
        {
            return Err("continuity_store_tombstone_fenced".to_string());
        }
        drop(tombstone);

        let mut statement = Self::prepare(
            connection,
            "INSERT INTO continuity_records(
               operation_key, request_hash, owner_node, ownership_generation,
               attempts_json, phase, replica_set_json, created_at_millis,
               updated_at_millis, terminal_at_millis, expires_at_millis,
               response_metadata_json, response_body, control_term, schema_version, version,
               request_body, runtime_record
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
             ON CONFLICT(operation_key) DO UPDATE SET
               request_hash=excluded.request_hash,
               owner_node=excluded.owner_node,
               ownership_generation=excluded.ownership_generation,
               attempts_json=excluded.attempts_json,
               phase=excluded.phase,
               replica_set_json=excluded.replica_set_json,
               updated_at_millis=excluded.updated_at_millis,
               terminal_at_millis=excluded.terminal_at_millis,
               expires_at_millis=excluded.expires_at_millis,
               response_metadata_json=excluded.response_metadata_json,
               response_body=excluded.response_body,
               control_term=excluded.control_term,
               schema_version=excluded.schema_version,
               version=excluded.version,
               request_body=excluded.request_body,
               runtime_record=excluded.runtime_record
             WHERE excluded.version > continuity_records.version
               AND excluded.ownership_generation >= continuity_records.ownership_generation",
        )?;
        statement.bind_text(1, &record.operation_key)?;
        statement.bind_text(2, &record.request_hash)?;
        statement.bind_text(3, &record.owner_node)?;
        statement.bind_i64(4, sqlite_integer(record.ownership_generation)?)?;
        statement.bind_text(5, &prepared.attempts)?;
        statement.bind_text(6, record.phase.as_str())?;
        statement.bind_text(7, &prepared.replicas)?;
        statement.bind_i64(8, sqlite_integer(record.created_at_millis)?)?;
        statement.bind_i64(9, sqlite_integer(record.updated_at_millis)?)?;
        statement.bind_optional_i64(10, record.terminal_at_millis)?;
        statement.bind_optional_i64(11, record.expires_at_millis)?;
        statement.bind_text(12, &prepared.response_metadata)?;
        statement.bind_blob(13, &record.response_body)?;
        statement.bind_i64(14, sqlite_integer(record.control_term)?)?;
        statement.bind_i64(15, i64::from(record.schema_version))?;
        statement.bind_i64(16, sqlite_integer(record.version)?)?;
        statement.bind_blob(17, &record.request_body)?;
        statement.bind_blob(18, &record.runtime_record)?;
        statement.step()?;
        let changed = unsafe { sqlite3_changes(connection.raw) } > 0;
        drop(statement);

        if !changed {
            return Ok(());
        }
        let mut log = Self::prepare(
            connection,
            "INSERT INTO continuity_log(operation_key, version, record_json, checksum)
             VALUES (?1, ?2, ?3, ?4)",
        )?;
        log.bind_text(1, &record.operation_key)?;
        log.bind_i64(2, sqlite_integer(record.version)?)?;
        log.bind_blob(3, &prepared.serialized)?;
        log.bind_blob(4, &prepared.checksum)?;
        log.step()?;
        Ok(())
    }

    fn upsert_batch(&self, records: &[StoredContinuityRecord]) -> Result<(), String> {
        if records.is_empty() {
            return Ok(());
        }
        let prepared = records
            .iter()
            .map(Self::prepare_upsert)
            .collect::<Result<Vec<_>, _>>()?;
        let mut new_records = 0u64;
        let mut new_terminal_records = 0u64;
        for record in records {
            if self.get(&record.operation_key)?.is_none() {
                new_records = new_records.saturating_add(1);
                if !record.phase.is_active() {
                    new_terminal_records = new_terminal_records.saturating_add(1);
                }
            }
        }
        if new_records > 0 {
            let estimated_growth = prepared.iter().fold(0u64, |total, record| {
                total.saturating_add(
                    u64::try_from(record.serialized.len())
                        .unwrap_or(u64::MAX)
                        .saturating_add(4_096),
                )
            });
            if self.disk_bytes().saturating_add(estimated_growth) > self.limits.max_disk_bytes {
                return Err("continuity_store_disk_limit_reached".to_string());
            }
            let mut terminal_records = self.terminal_record_count()?;
            if terminal_records.saturating_add(new_terminal_records)
                > self.limits.max_terminal_records
            {
                let _ = self.compact(SystemTimeMillis::now())?;
                terminal_records = self.terminal_record_count()?;
                if terminal_records.saturating_add(new_terminal_records)
                    > self.limits.max_terminal_records
                {
                    return Err("continuity_store_terminal_record_limit_reached".to_string());
                }
            }
        }

        let connection = self.connection.lock().unwrap();
        execute_batch(connection.raw, "BEGIN IMMEDIATE")?;
        let result = prepared
            .iter()
            .try_for_each(|record| Self::apply_prepared_upsert(&connection, record));
        match result {
            Ok(()) => execute_batch(connection.raw, "COMMIT"),
            Err(error) => {
                let _ = execute_batch(connection.raw, "ROLLBACK");
                Err(error)
            }
        }
    }

    fn initialize_schema(&self) -> Result<(), String> {
        let connection = self.connection.lock().unwrap();
        execute_batch(
            connection.raw,
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             PRAGMA busy_timeout = 5000;
             CREATE TABLE IF NOT EXISTS schema_migrations (
               version INTEGER PRIMARY KEY,
               name TEXT NOT NULL,
               applied_at_millis INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS continuity_records (
               operation_key TEXT PRIMARY KEY,
               request_hash TEXT NOT NULL,
               owner_node TEXT NOT NULL,
               ownership_generation INTEGER NOT NULL CHECK (ownership_generation >= 0),
               attempts_json TEXT NOT NULL,
               phase TEXT NOT NULL,
               replica_set_json TEXT NOT NULL,
               created_at_millis INTEGER NOT NULL,
               updated_at_millis INTEGER NOT NULL,
               terminal_at_millis INTEGER,
               expires_at_millis INTEGER,
               response_metadata_json TEXT NOT NULL,
               response_body BLOB NOT NULL,
               request_body BLOB NOT NULL DEFAULT X'',
               runtime_record BLOB NOT NULL DEFAULT X'',
               control_term INTEGER NOT NULL,
               schema_version INTEGER NOT NULL,
               version INTEGER NOT NULL CHECK (version > 0)
             );
             CREATE INDEX IF NOT EXISTS continuity_terminal_expiry
               ON continuity_records(expires_at_millis, updated_at_millis)
               WHERE terminal_at_millis IS NOT NULL;
             CREATE TABLE IF NOT EXISTS continuity_tombstones (
               operation_key TEXT PRIMARY KEY,
               version INTEGER NOT NULL,
               deleted_at_millis INTEGER NOT NULL,
               expires_at_millis INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS continuity_tombstone_expiry
               ON continuity_tombstones(expires_at_millis);
             CREATE TABLE IF NOT EXISTS continuity_log (
               sequence INTEGER PRIMARY KEY AUTOINCREMENT,
               operation_key TEXT NOT NULL,
               version INTEGER NOT NULL,
               record_json BLOB NOT NULL,
               checksum BLOB NOT NULL
             );
             CREATE TABLE IF NOT EXISTS continuity_replica_safe_points (
               replica_node TEXT PRIMARY KEY,
               high_water_mark INTEGER NOT NULL CHECK (high_water_mark >= 0),
               acknowledged_at_millis INTEGER NOT NULL
             );
             INSERT OR IGNORE INTO schema_migrations(version, name, applied_at_millis)
               VALUES (1, 'initial_continuity_store', 0);",
        )?;
        let mut column = Self::prepare(
            &connection,
            "SELECT COUNT(*) FROM pragma_table_info('continuity_records')
              WHERE name = 'request_body'",
        )?;
        let request_body_missing = column.step()? == SQLITE_ROW && column.integer(0) == 0;
        drop(column);
        if request_body_missing {
            execute_batch(
                connection.raw,
                "ALTER TABLE continuity_records
                   ADD COLUMN request_body BLOB NOT NULL DEFAULT X'';",
            )?;
        }
        execute_batch(
            connection.raw,
            "INSERT OR IGNORE INTO schema_migrations(version, name, applied_at_millis)
               VALUES (2, 'continuity_request_body', 0);",
        )?;
        let mut column = Self::prepare(
            &connection,
            "SELECT COUNT(*) FROM pragma_table_info('continuity_records')
              WHERE name = 'runtime_record'",
        )?;
        let runtime_record_missing = column.step()? == SQLITE_ROW && column.integer(0) == 0;
        drop(column);
        if runtime_record_missing {
            execute_batch(
                connection.raw,
                "ALTER TABLE continuity_records
                   ADD COLUMN runtime_record BLOB NOT NULL DEFAULT X'';",
            )?;
        }
        execute_batch(
            connection.raw,
            "INSERT OR IGNORE INTO schema_migrations(version, name, applied_at_millis)
               VALUES (3, 'continuity_runtime_record', 0);",
        )?;
        execute_batch(
            connection.raw,
            "CREATE TABLE IF NOT EXISTS continuity_store_counters (
               singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
               terminal_record_count INTEGER NOT NULL CHECK (terminal_record_count >= 0)
             );
             INSERT OR IGNORE INTO continuity_store_counters(singleton, terminal_record_count)
               SELECT 1, COUNT(*) FROM continuity_records WHERE terminal_at_millis IS NOT NULL;
             CREATE TRIGGER IF NOT EXISTS continuity_terminal_count_insert
               AFTER INSERT ON continuity_records
               WHEN NEW.terminal_at_millis IS NOT NULL
               BEGIN
                 UPDATE continuity_store_counters
                    SET terminal_record_count = terminal_record_count + 1
                  WHERE singleton = 1;
               END;
             CREATE TRIGGER IF NOT EXISTS continuity_terminal_count_delete
               AFTER DELETE ON continuity_records
               WHEN OLD.terminal_at_millis IS NOT NULL
               BEGIN
                 UPDATE continuity_store_counters
                    SET terminal_record_count = terminal_record_count - 1
                  WHERE singleton = 1;
               END;
             CREATE TRIGGER IF NOT EXISTS continuity_terminal_count_update
               AFTER UPDATE OF terminal_at_millis ON continuity_records
               WHEN (OLD.terminal_at_millis IS NULL) != (NEW.terminal_at_millis IS NULL)
               BEGIN
                 UPDATE continuity_store_counters
                    SET terminal_record_count = terminal_record_count
                      + CASE WHEN NEW.terminal_at_millis IS NULL THEN -1 ELSE 1 END
                  WHERE singleton = 1;
               END;
             INSERT OR IGNORE INTO schema_migrations(version, name, applied_at_millis)
               VALUES (4, 'continuity_constant_time_terminal_counter', 0);",
        )?;
        Ok(())
    }

    fn prepare(connection: &Connection, sql: &str) -> Result<Statement, String> {
        let sql = CString::new(sql).expect("static SQL contains no NUL");
        let mut statement = ptr::null_mut();
        check_sqlite(connection.raw, unsafe {
            sqlite3_prepare_v2(
                connection.raw,
                sql.as_ptr(),
                -1,
                &mut statement,
                ptr::null_mut(),
            )
        })?;
        Ok(Statement {
            database: connection.raw,
            raw: statement,
        })
    }

    fn record_from_statement(statement: &Statement) -> Result<StoredContinuityRecord, String> {
        Ok(StoredContinuityRecord {
            operation_key: statement.text(0),
            request_hash: statement.text(1),
            request_body: statement.blob(16),
            runtime_record: statement.blob(17),
            owner_node: statement.text(2),
            ownership_generation: unsigned_integer(statement.integer(3))?,
            attempts: serde_json::from_str(&statement.text(4))
                .map_err(|_| "continuity_store_attempts_corrupt".to_string())?,
            phase: StoredContinuityPhase::parse(&statement.text(5))?,
            replica_set: serde_json::from_str(&statement.text(6))
                .map_err(|_| "continuity_store_replicas_corrupt".to_string())?,
            created_at_millis: unsigned_integer(statement.integer(7))?,
            updated_at_millis: unsigned_integer(statement.integer(8))?,
            terminal_at_millis: statement
                .optional_integer(9)
                .map(unsigned_integer)
                .transpose()?,
            expires_at_millis: statement
                .optional_integer(10)
                .map(unsigned_integer)
                .transpose()?,
            response_metadata: serde_json::from_str(&statement.text(11))
                .map_err(|_| "continuity_store_response_metadata_corrupt".to_string())?,
            response_body: statement.blob(12),
            control_term: unsigned_integer(statement.integer(13))?,
            schema_version: statement
                .integer(14)
                .try_into()
                .map_err(|_| "continuity_store_schema_version_corrupt".to_string())?,
            version: unsigned_integer(statement.integer(15))?,
        })
    }

    fn all_records_from_connection(
        connection: &Connection,
    ) -> Result<Vec<StoredContinuityRecord>, String> {
        let mut statement = Self::prepare(
            connection,
            "SELECT operation_key, request_hash, owner_node, ownership_generation,
                    attempts_json, phase, replica_set_json, created_at_millis,
                    updated_at_millis, terminal_at_millis, expires_at_millis,
                    response_metadata_json, response_body, control_term,
                    schema_version, version, request_body, runtime_record
               FROM continuity_records ORDER BY operation_key",
        )?;
        let mut records = Vec::new();
        while statement.step()? == SQLITE_ROW {
            records.push(Self::record_from_statement(&statement)?);
        }
        Ok(records)
    }

    fn all_records(&self) -> Result<Vec<StoredContinuityRecord>, String> {
        let connection = self.connection.lock().unwrap();
        Self::all_records_from_connection(&connection)
    }

    fn update_response(&self, operation_key: &str, response: &[u8]) -> Result<(), String> {
        let metadata = serde_json::to_string(&vec![("replayable".to_string(), "true".to_string())])
            .map_err(|_| "continuity_store_response_metadata_encode_failed".to_string())?;
        let connection = self.connection.lock().unwrap();
        let mut statement = Self::prepare(
            &connection,
            "UPDATE continuity_records
                SET response_metadata_json = ?2, response_body = ?3, updated_at_millis = ?4
              WHERE operation_key = ?1",
        )?;
        statement.bind_text(1, operation_key)?;
        statement.bind_text(2, &metadata)?;
        statement.bind_blob(3, response)?;
        statement.bind_i64(4, sqlite_integer(SystemTimeMillis::now())?)?;
        statement.step()?;
        if unsafe { sqlite3_changes(connection.raw) } == 0 {
            return Err("continuity_response_record_missing".to_string());
        }
        Ok(())
    }

    fn terminal_record_count(&self) -> Result<u64, String> {
        Self::terminal_records(&self.connection.lock().unwrap())
    }

    fn node_safety(
        &self,
        node_id: &str,
        live_nodes: &BTreeSet<String>,
    ) -> Result<ContinuityNodeSafety, String> {
        let mut safety = ContinuityNodeSafety::default();
        for record in self
            .all_records()?
            .into_iter()
            .filter(|record| record.phase.is_active())
        {
            let owns = record.owner_node == node_id;
            let replicates = record.replica_set.iter().any(|replica| replica == node_id);
            if owns {
                safety.active_owned_records = safety.active_owned_records.saturating_add(1);
            }
            if replicates {
                safety.required_replica_responsibilities =
                    safety.required_replica_responsibilities.saturating_add(1);
            }
            if owns || replicates {
                let live_copies = std::iter::once(&record.owner_node)
                    .chain(record.replica_set.iter())
                    .filter(|holder| live_nodes.contains(*holder))
                    .count();
                safety.only_active_copy |= live_copies <= 1;
            }
        }
        Ok(safety)
    }

    fn snapshot_state(&self) -> Result<(u64, Vec<StoredContinuityRecord>), String> {
        let connection = self.connection.lock().unwrap();
        execute_batch(connection.raw, "BEGIN")?;
        let result = (|| {
            let high_water = Self::high_water(&connection)?;
            let records = Self::all_records_from_connection(&connection)?;
            Ok((high_water, records))
        })();
        match result {
            Ok(state) => {
                execute_batch(connection.raw, "COMMIT")?;
                Ok(state)
            }
            Err(error) => {
                let _ = execute_batch(connection.raw, "ROLLBACK");
                Err(error)
            }
        }
    }

    fn disk_bytes(&self) -> u64 {
        if self.path == Path::new(":memory:") {
            return 0;
        }
        let sidecar = |suffix: &str| {
            let mut path = self.path.as_os_str().to_os_string();
            path.push(suffix);
            PathBuf::from(path)
        };
        let paths = [self.path.clone(), sidecar("-wal"), sidecar("-shm")];
        paths
            .into_iter()
            .filter_map(|path| std::fs::metadata(path).ok())
            .map(|metadata| metadata.len())
            .sum()
    }
}

const RECORD_COLUMNS: &str =
    "operation_key, request_hash, owner_node, ownership_generation, attempts_json,
     phase, replica_set_json, created_at_millis, updated_at_millis, terminal_at_millis,
     expires_at_millis, response_metadata_json, response_body, control_term, schema_version, version,
     request_body, runtime_record";

impl ContinuityStore for SqliteContinuityStore {
    fn get(&self, operation_key: &str) -> Result<Option<StoredContinuityRecord>, String> {
        let connection = self.connection.lock().unwrap();
        let sql =
            format!("SELECT {RECORD_COLUMNS} FROM continuity_records WHERE operation_key = ?1");
        let mut statement = Self::prepare(&connection, &sql)?;
        statement.bind_text(1, operation_key)?;
        if statement.step()? == SQLITE_ROW {
            Ok(Some(Self::record_from_statement(&statement)?))
        } else {
            Ok(None)
        }
    }

    fn upsert(&self, record: &StoredContinuityRecord) -> Result<(), String> {
        self.upsert_batch(std::slice::from_ref(record))
    }

    fn compact(&self, now_millis: u64) -> Result<CompactionOutcome, String> {
        let connection = self.connection.lock().unwrap();
        execute_batch(connection.raw, "BEGIN IMMEDIATE")?;
        let result = (|| {
            let mut select = Self::prepare(
                &connection,
                "SELECT operation_key, version FROM continuity_records
                   WHERE terminal_at_millis IS NOT NULL AND expires_at_millis <= ?1
                   ORDER BY expires_at_millis LIMIT ?2",
            )?;
            select.bind_i64(1, sqlite_integer(now_millis)?)?;
            select.bind_i64(2, i64::from(self.limits.compaction_batch_size))?;
            let mut expired = Vec::new();
            while select.step()? == SQLITE_ROW {
                expired.push((select.text(0), unsigned_integer(select.integer(1))?));
            }
            drop(select);
            for (operation_key, version) in &expired {
                let mut tombstone = Self::prepare(
                    &connection,
                    "INSERT INTO continuity_tombstones(operation_key, version, deleted_at_millis, expires_at_millis)
                     VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(operation_key) DO UPDATE SET
                       version=MAX(version, excluded.version),
                       deleted_at_millis=excluded.deleted_at_millis,
                       expires_at_millis=MAX(expires_at_millis, excluded.expires_at_millis)",
                )?;
                tombstone.bind_text(1, operation_key)?;
                tombstone.bind_i64(2, sqlite_integer(*version)?)?;
                tombstone.bind_i64(3, sqlite_integer(now_millis)?)?;
                tombstone.bind_i64(
                    4,
                    sqlite_integer(
                        now_millis.saturating_add(self.limits.tombstone_retention_millis),
                    )?,
                )?;
                tombstone.step()?;
                let mut delete = Self::prepare(
                    &connection,
                    "DELETE FROM continuity_records WHERE operation_key = ?1 AND version <= ?2",
                )?;
                delete.bind_text(1, operation_key)?;
                delete.bind_i64(2, sqlite_integer(*version)?)?;
                delete.step()?;
            }
            let mut delete_tombstones = Self::prepare(
                &connection,
                "DELETE FROM continuity_tombstones WHERE expires_at_millis <= ?1",
            )?;
            delete_tombstones.bind_i64(1, sqlite_integer(now_millis)?)?;
            delete_tombstones.step()?;
            let deleted = unsafe { sqlite3_changes(connection.raw) }.max(0) as u32;
            Ok(CompactionOutcome {
                records_tombstoned: expired.len().try_into().unwrap_or(u32::MAX),
                tombstones_deleted: deleted,
            })
        })();
        match result {
            Ok(outcome) => {
                execute_batch(connection.raw, "COMMIT")?;
                // PASSIVE never waits for readers and keeps WAL growth bounded
                // without turning compaction into an unbounded maintenance job.
                let _ = execute_batch(connection.raw, "PRAGMA wal_checkpoint(PASSIVE)");
                Ok(outcome)
            }
            Err(error) => {
                let _ = execute_batch(connection.raw, "ROLLBACK");
                Err(error)
            }
        }
    }

    fn snapshot_chunks(&self, chunk_bytes: usize) -> Result<Vec<SnapshotChunk>, String> {
        if chunk_bytes < 128 {
            return Err("continuity_snapshot_chunk_bound_too_small".to_string());
        }
        let (high_water_mark, records) = self.snapshot_state()?;
        let snapshot_id = format!("snapshot-{high_water_mark}-{}", records.len());
        let mut payloads: Vec<Vec<u8>> = Vec::new();
        let mut current = vec![b'['];
        for record in records {
            let encoded = serde_json::to_vec(&record)
                .map_err(|error| format!("continuity_snapshot_encode_failed:{error}"))?;
            if encoded.len() > chunk_bytes - 2 {
                return Err("continuity_snapshot_record_exceeds_chunk_bound".to_string());
            }
            // Include the comma and closing bracket in the chunk bound.
            if current.len() > 1
                && current
                    .len()
                    .saturating_add(encoded.len())
                    .saturating_add(2)
                    > chunk_bytes
            {
                current.push(b']');
                payloads.push(current);
                current = vec![b'['];
            }
            if current.len() > 1 {
                current.push(b',');
            }
            current.extend_from_slice(&encoded);
        }
        if current.len() > 1 || payloads.is_empty() {
            current.push(b']');
            payloads.push(current);
        }
        let final_sequence = payloads.len().saturating_sub(1);
        let checksums: Vec<[u8; 32]> = payloads
            .iter()
            .map(|payload| Sha256::digest(payload).into())
            .collect();
        let mut snapshot_hasher = Sha256::new();
        for checksum in &checksums {
            snapshot_hasher.update(checksum);
        }
        let snapshot_checksum: [u8; 32] = snapshot_hasher.finalize().into();
        Ok(payloads
            .into_iter()
            .enumerate()
            .map(|(sequence, payload)| SnapshotChunk {
                snapshot_id: snapshot_id.clone(),
                sequence: sequence.try_into().unwrap_or(u32::MAX),
                final_chunk: sequence == final_sequence,
                high_water_mark,
                checksum: checksums[sequence],
                snapshot_checksum,
                payload,
            })
            .collect())
    }

    fn apply_snapshot_chunk(&self, chunk: &SnapshotChunk) -> Result<(), String> {
        if !chunk.verify() {
            return Err("continuity_snapshot_checksum_mismatch".to_string());
        }
        let records: Vec<StoredContinuityRecord> = serde_json::from_slice(&chunk.payload)
            .map_err(|error| format!("continuity_snapshot_decode_failed:{error}"))?;
        // One durable transaction per chunk, not per record: with
        // `synchronous = FULL` each commit is an fsync, which made joining a
        // 10,000-record snapshot take seconds.
        self.upsert_batch(&records)
    }

    fn high_water_mark(&self) -> Result<u64, String> {
        Self::high_water(&self.connection.lock().unwrap())
    }

    fn log_entries_after(
        &self,
        high_water_mark: u64,
        limit: u32,
    ) -> Result<Vec<ContinuityLogEntry>, String> {
        if limit == 0 {
            return Err("continuity_log_batch_limit_zero".to_string());
        }
        let connection = self.connection.lock().unwrap();
        let mut statement = Self::prepare(
            &connection,
            "SELECT sequence, operation_key, version, record_json, checksum
               FROM continuity_log WHERE sequence > ?1 ORDER BY sequence LIMIT ?2",
        )?;
        statement.bind_i64(1, sqlite_integer(high_water_mark)?)?;
        statement.bind_i64(2, i64::from(limit))?;
        let mut entries = Vec::new();
        while statement.step()? == SQLITE_ROW {
            let encoded = statement.blob(3);
            let record: StoredContinuityRecord = serde_json::from_slice(&encoded)
                .map_err(|error| format!("continuity_log_record_decode_failed:{error}"))?;
            let checksum: [u8; 32] = statement
                .blob(4)
                .try_into()
                .map_err(|_| "continuity_log_checksum_invalid".to_string())?;
            let entry = ContinuityLogEntry {
                sequence: unsigned_integer(statement.integer(0))?,
                operation_key: statement.text(1),
                version: unsigned_integer(statement.integer(2))?,
                record,
                checksum,
            };
            if !entry.verify() {
                return Err("continuity_log_entry_checksum_mismatch".to_string());
            }
            entries.push(entry);
        }
        Ok(entries)
    }

    fn apply_log_entry(&self, entry: &ContinuityLogEntry) -> Result<(), String> {
        if !entry.verify() {
            return Err("continuity_log_entry_checksum_mismatch".to_string());
        }
        self.upsert(&entry.record)
    }

    fn acknowledge_replica_safe_point(
        &self,
        replica_node: &str,
        high_water_mark: u64,
    ) -> Result<(), String> {
        if replica_node.trim().is_empty() {
            return Err("continuity_replica_safe_point_node_missing".to_string());
        }
        let connection = self.connection.lock().unwrap();
        let mut statement = Self::prepare(
            &connection,
            "INSERT INTO continuity_replica_safe_points(
               replica_node, high_water_mark, acknowledged_at_millis
             ) VALUES (?1, ?2, ?3)
             ON CONFLICT(replica_node) DO UPDATE SET
               high_water_mark=MAX(high_water_mark, excluded.high_water_mark),
               acknowledged_at_millis=excluded.acknowledged_at_millis",
        )?;
        statement.bind_text(1, replica_node)?;
        statement.bind_i64(2, sqlite_integer(high_water_mark)?)?;
        statement.bind_i64(3, sqlite_integer(SystemTimeMillis::now())?)?;
        statement.step()?;
        Ok(())
    }

    fn compact_log_to_replica_safe_point(&self) -> Result<u64, String> {
        let connection = self.connection.lock().unwrap();
        execute_batch(connection.raw, "BEGIN IMMEDIATE")?;
        let result = (|| {
            let Some(safe_point) = Self::replica_safe_point(&connection)? else {
                return Ok(0);
            };
            let mut delete = Self::prepare(
                &connection,
                "DELETE FROM continuity_log WHERE sequence IN (
                   SELECT sequence FROM continuity_log
                    WHERE sequence <= ?1 ORDER BY sequence LIMIT ?2
                 )",
            )?;
            delete.bind_i64(1, sqlite_integer(safe_point)?)?;
            delete.bind_i64(2, i64::from(self.limits.compaction_batch_size))?;
            delete.step()?;
            Ok(unsafe { sqlite3_changes(connection.raw) }.max(0) as u64)
        })();
        match result {
            Ok(deleted) => {
                execute_batch(connection.raw, "COMMIT")?;
                Ok(deleted)
            }
            Err(error) => {
                let _ = execute_batch(connection.raw, "ROLLBACK");
                Err(error)
            }
        }
    }
}

fn execute_batch(database: *mut sqlite3, sql: &str) -> Result<(), String> {
    let sql = CString::new(sql).expect("static SQL contains no NUL");
    check_sqlite(database, unsafe {
        sqlite3_exec(
            database,
            sql.as_ptr(),
            None,
            ptr::null_mut(),
            ptr::null_mut(),
        )
    })
}

fn check_sqlite(database: *mut sqlite3, result: c_int) -> Result<(), String> {
    if result == SQLITE_OK {
        Ok(())
    } else {
        Err(sqlite_error(database))
    }
}

fn sqlite_error(database: *mut sqlite3) -> String {
    // sqlite3_errmsg never returns NULL; for a NULL handle (open ran out
    // of memory) it reports that.
    format!(
        "continuity_store_database_error:{}",
        unsafe { CStr::from_ptr(sqlite3_errmsg(database)) }.to_string_lossy()
    )
}

fn sqlite_integer(value: u64) -> Result<i64, String> {
    value
        .try_into()
        .map_err(|_| "continuity_store_integer_out_of_range".to_string())
}

fn unsigned_integer(value: i64) -> Result<u64, String> {
    value
        .try_into()
        .map_err(|_| "continuity_store_negative_integer".to_string())
}

#[cfg(unix)]
fn secure_database_file(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("continuity_store_permissions_failed:{error}"))
}

#[cfg(not(unix))]
fn secure_database_file(_path: &Path) -> Result<(), String> {
    Ok(())
}

static CONFIGURED_STORE: OnceLock<Option<Arc<SqliteContinuityStore>>> = OnceLock::new();
static DURABLE_WRITER: OnceLock<DurableWriter> = OnceLock::new();

const DURABLE_WRITE_QUEUE_ITEMS: usize = 8_192;
const DURABLE_WRITE_BATCH_ITEMS: usize = 128;
const DURABLE_WRITE_BATCH_WINDOW: Duration = Duration::from_millis(2);

struct DurableWrite {
    record: StoredContinuityRecord,
    reply: crate::actor::CooperativeSender<Result<(), String>>,
}

const MAX_REPLAY_RESPONSES: usize = 10_000;
const MAX_REPLAY_BYTES: usize = 128 * 1024 * 1024;

#[derive(Default)]
struct ResponseReplayCache {
    responses: BTreeMap<String, Vec<u8>>,
    insertion_order: VecDeque<String>,
    bytes: usize,
}

impl ResponseReplayCache {
    fn insert(&mut self, operation_key: &str, response: &[u8]) {
        if response.len() > MAX_REPLAY_BYTES {
            return;
        }
        if let Some(previous) = self.responses.remove(operation_key) {
            self.bytes = self.bytes.saturating_sub(previous.len());
            self.insertion_order.retain(|key| key != operation_key);
        }
        self.bytes = self.bytes.saturating_add(response.len());
        self.responses
            .insert(operation_key.to_string(), response.to_vec());
        self.insertion_order.push_back(operation_key.to_string());
        // Each key is queued once, and the newest response alone fits the
        // byte bound: evicting older ones always ends inside the bounds.
        while self.responses.len() > MAX_REPLAY_RESPONSES || self.bytes > MAX_REPLAY_BYTES {
            let oldest = self
                .insertion_order
                .pop_front()
                .expect("a cache over its bounds holds an older response");
            let removed = self
                .responses
                .remove(&oldest)
                .expect("every queued key has its response");
            self.bytes = self.bytes.saturating_sub(removed.len());
        }
    }
}

static RESPONSE_REPLAY_CACHE: OnceLock<Mutex<ResponseReplayCache>> = OnceLock::new();

fn response_replay_cache() -> &'static Mutex<ResponseReplayCache> {
    RESPONSE_REPLAY_CACHE.get_or_init(|| Mutex::new(ResponseReplayCache::default()))
}

/// Reads one deployment environment variable: the runtime passes
/// `std::env::var`, tests a table of their own.
type EnvironmentLookup<'a> = &'a dyn Fn(&str) -> Option<String>;

fn process_environment(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

pub fn configured_continuity_store() -> Option<&'static Arc<SqliteContinuityStore>> {
    CONFIGURED_STORE
        .get_or_init(|| {
            let embedded = super::autonomous::embedded_autonomous_config();
            let node_name = super::node::node_state().map(|state| state.name.as_str());
            let path = continuity_database_path(embedded, &process_environment, node_name)?;
            open_configured_store(&path, &runtime_continuity_config(embedded))
        })
        .as_ref()
}

fn open_configured_store(
    path: &Path,
    config: &super::autonomous::RuntimeContinuityConfig,
) -> Option<Arc<SqliteContinuityStore>> {
    let limits = ContinuityStoreLimits {
        terminal_retention_millis: config.terminal_retention_millis,
        tombstone_retention_millis: config.tombstone_retention_millis,
        max_terminal_records: config.max_terminal_records,
        max_disk_bytes: config.max_disk_bytes,
        compaction_batch_size: 1_000,
    };
    match SqliteContinuityStore::open(path, limits) {
        Ok(store) => {
            let store = Arc::new(store);
            start_continuity_compactor(Arc::clone(&store));
            Some(store)
        }
        Err(error) => {
            eprintln!("mesh continuity: durable_store_open_failed reason={error}");
            None
        }
    }
}

fn runtime_continuity_config(
    embedded: Option<&super::autonomous::RuntimeAutonomousConfig>,
) -> super::autonomous::RuntimeContinuityConfig {
    embedded
        .map(|config| config.continuity.clone())
        .unwrap_or_default()
}

/// Where this node keeps its durable continuity store, if anywhere:
/// MESH_CONTINUITY_DB when set; otherwise, in autonomous mode with durable
/// continuity, the manifest's path or a node-private default.
fn continuity_database_path(
    embedded: Option<&super::autonomous::RuntimeAutonomousConfig>,
    env: EnvironmentLookup<'_>,
    node_name: Option<&str>,
) -> Option<PathBuf> {
    if let Some(path) = env("MESH_CONTINUITY_DB") {
        return (!path.trim().is_empty()).then(|| PathBuf::from(path));
    }
    // Manual mode preserves the historical opt-in behavior.
    let embedded = embedded.filter(|autonomous| autonomous.features.durable_continuity)?;
    if let Some(path) = embedded
        .continuity
        .path
        .as_ref()
        .filter(|path| !path.as_os_str().is_empty())
    {
        return Some(path.clone());
    }
    let stable_id = env("MESH_STABLE_NODE_ID")
        .filter(|value| !value.trim().is_empty())
        .or_else(|| node_name.map(str::to_string))
        .unwrap_or_else(|| "mesh-local-node".to_string());
    let digest = Sha256::digest(stable_id.as_bytes());
    let suffix: String = digest[..12]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let directory = env("MESH_DATA_DIR").map_or_else(|| PathBuf::from(".mesh"), PathBuf::from);
    Some(directory.join(format!("continuity-{suffix}.db")))
}

fn start_continuity_compactor(store: Arc<SqliteContinuityStore>) {
    let _ = std::thread::Builder::new()
        .name("mesh-continuity-compactor".to_string())
        .spawn(move || loop {
            std::thread::park_timeout(std::time::Duration::from_secs(30));
            compaction_pass(&store);
        });
}

/// One compactor pass: expire terminal records and tombstones, then trim
/// the replication log every replica has acknowledged.
fn compaction_pass(store: &SqliteContinuityStore) {
    match store.compact(SystemTimeMillis::now()) {
        Ok(outcome) => {
            if outcome.records_tombstoned > 0 || outcome.tombstones_deleted > 0 {
                eprintln!(
                    "mesh continuity: transition=compacted records_tombstoned={} tombstones_deleted={}",
                    outcome.records_tombstoned, outcome.tombstones_deleted
                );
            }
            let _ = store.compact_log_to_replica_safe_point();
        }
        Err(error) => {
            eprintln!("mesh continuity: compaction_failed reason={error}");
        }
    }
}

pub(crate) fn runtime_snapshot_chunk_bytes() -> usize {
    snapshot_chunk_bytes(
        &process_environment,
        &runtime_continuity_config(super::autonomous::embedded_autonomous_config()),
    )
}

fn snapshot_chunk_bytes(
    env: EnvironmentLookup<'_>,
    config: &super::autonomous::RuntimeContinuityConfig,
) -> usize {
    env("MESH_CONTINUITY_SNAPSHOT_CHUNK_BYTES")
        .and_then(|value| value.parse::<usize>().ok())
        // The bounds the manifest's `snapshot_chunk_bytes` has: a chunk must
        // fit in one transport frame.
        .filter(|value| (128..16 * 1024 * 1024).contains(value))
        .unwrap_or_else(|| config.snapshot_chunk_bytes.try_into().unwrap_or(usize::MAX))
}

pub(crate) fn degraded_durability_enabled() -> bool {
    durability_degraded(
        &process_environment,
        &runtime_continuity_config(super::autonomous::embedded_autonomous_config()),
    )
}

fn durability_degraded(
    env: EnvironmentLookup<'_>,
    config: &super::autonomous::RuntimeContinuityConfig,
) -> bool {
    env("MESH_CONTINUITY_DURABILITY")
        .map(|value| value.trim().eq_ignore_ascii_case("degraded"))
        .unwrap_or(!config.strict_durability)
}

pub(crate) fn continuity_node_safety(
    node_id: &str,
    live_nodes: &BTreeSet<String>,
) -> Result<ContinuityNodeSafety, String> {
    if node_id.is_empty() {
        return Err("continuity_safety_node_missing".to_string());
    }
    match configured_continuity_store() {
        Some(store) => store.node_safety(node_id, live_nodes),
        None => Ok(registry_node_safety(
            super::continuity::continuity_registry(),
            node_id,
            live_nodes,
        )),
    }
}

/// Manual/non-durable mode retains the same conservative safety contract
/// using its single-replica in-memory record shape. Arbitrary replica sets
/// require the configured durable store.
fn registry_node_safety(
    registry: &super::continuity::ContinuityRegistry,
    node_id: &str,
    live_nodes: &BTreeSet<String>,
) -> ContinuityNodeSafety {
    let mut safety = ContinuityNodeSafety::default();
    for record in registry
        .snapshot()
        .records
        .into_iter()
        .filter(|record| record.phase == super::continuity::ContinuityPhase::Submitted)
    {
        let owns = record.owner_node == node_id;
        let replicates = record.replica_node == node_id;
        if owns {
            safety.active_owned_records = safety.active_owned_records.saturating_add(1);
        }
        if replicates {
            safety.required_replica_responsibilities =
                safety.required_replica_responsibilities.saturating_add(1);
        }
        if owns || replicates {
            let live_copies = [&record.owner_node, &record.replica_node]
                .into_iter()
                .filter(|holder| !holder.is_empty() && live_nodes.contains(*holder))
                .count();
            safety.only_active_copy |= live_copies <= 1;
        }
    }
    safety
}

pub(crate) fn persist_runtime_response(operation_key: &str, response: &[u8]) -> Result<(), String> {
    if operation_key.is_empty() || response.is_empty() {
        return Err("continuity_response_invalid".to_string());
    }
    response_replay_cache()
        .lock()
        .unwrap()
        .insert(operation_key, response);
    configured_continuity_store().map_or(Ok(()), |store| {
        store.update_response(operation_key, response)
    })
}

pub(crate) fn replay_runtime_response(operation_key: &str) -> Result<Option<Vec<u8>>, String> {
    let cached = response_replay_cache()
        .lock()
        .unwrap()
        .responses
        .get(operation_key)
        .cloned();
    match (cached, configured_continuity_store()) {
        (Some(response), _) => Ok(Some(response)),
        (None, Some(store)) => store.replay_response(operation_key),
        (None, None) => Ok(None),
    }
}

pub(crate) fn load_runtime_records() -> Result<Vec<Vec<u8>>, String> {
    configured_continuity_store().map_or(Ok(Vec::new()), |store| store.runtime_records())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotResumeProof {
    pub records: usize,
    pub chunks: usize,
    pub acknowledged_before_interruption: usize,
    pub resumed_from_sequence: u32,
    pub final_high_water_mark: u64,
}

/// Exercises the production SQLite snapshot format across an interrupted
/// transfer and a receiver-store reopen. The Docker release proof calls this
/// directly so snapshot resume is captured in the same evidence bundle as
/// horizontal scaling.
pub fn prove_interrupted_snapshot_resume() -> Result<SnapshotResumeProof, String> {
    let limits = ContinuityStoreLimits::default();
    let source = SqliteContinuityStore::open(Path::new(":memory:"), limits)?;
    const RECORDS: usize = 64;
    for index in 0..RECORDS {
        let body = vec![(index % 251) as u8; 192];
        source.upsert(&StoredContinuityRecord {
            operation_key: format!("snapshot-proof-{index:04}"),
            request_hash: format!("hash-{index:04}"),
            request_body: body.clone(),
            runtime_record: body,
            owner_node: "snapshot-source".to_string(),
            ownership_generation: 1,
            attempts: vec![format!("attempt-{index:04}")],
            phase: StoredContinuityPhase::Completed,
            replica_set: vec!["snapshot-target".to_string()],
            created_at_millis: 1,
            updated_at_millis: 2,
            terminal_at_millis: Some(2),
            expires_at_millis: Some(4_102_444_800_000),
            response_metadata: vec![("status".to_string(), "200".to_string())],
            response_body: b"ok".to_vec(),
            control_term: 1,
            schema_version: SCHEMA_VERSION,
            version: 1,
        })?;
    }
    let chunks = source.snapshot_chunks(2_048)?;
    if chunks.len() < 2 {
        return Err("continuity_snapshot_proof_not_chunked".to_string());
    }
    let acknowledged_before_interruption = chunks.len() / 2;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "mesh-snapshot-resume-proof-{}-{stamp}.db",
        std::process::id()
    ));
    let result = (|| {
        {
            let target = SqliteContinuityStore::open(&path, limits)?;
            for chunk in &chunks[..acknowledged_before_interruption] {
                target.apply_snapshot_chunk(chunk)?;
            }
        }
        let target = SqliteContinuityStore::open(&path, limits)?;
        for chunk in &chunks[acknowledged_before_interruption..] {
            target.apply_snapshot_chunk(chunk)?;
        }
        let records = target.all_records()?.len();
        if records != RECORDS {
            return Err(format!(
                "continuity_snapshot_resume_record_mismatch:expected={RECORDS}:actual={records}"
            ));
        }
        Ok(SnapshotResumeProof {
            records,
            chunks: chunks.len(),
            acknowledged_before_interruption,
            resumed_from_sequence: chunks[acknowledged_before_interruption].sequence,
            final_high_water_mark: chunks
                .first()
                .map(|chunk| chunk.high_water_mark)
                .unwrap_or(0),
        })
    })();
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
    result
}

impl SqliteContinuityStore {
    /// A completed record's stored response, now also in the replay cache.
    fn replay_response(&self, operation_key: &str) -> Result<Option<Vec<u8>>, String> {
        let response = self
            .get(operation_key)?
            .filter(|record| record.phase == StoredContinuityPhase::Completed)
            .map(|record| record.response_body)
            .filter(|response| !response.is_empty());
        if let Some(response) = &response {
            response_replay_cache()
                .lock()
                .unwrap()
                .insert(operation_key, response);
        }
        Ok(response)
    }

    /// The versioned runtime records that rehydrate in-flight state.
    fn runtime_records(&self) -> Result<Vec<Vec<u8>>, String> {
        Ok(self
            .all_records()?
            .into_iter()
            .map(|record| record.runtime_record)
            .filter(|encoded| !encoded.is_empty())
            .collect())
    }

    fn stored_runtime_record(
        &self,
        record: &super::continuity::ContinuityRecord,
    ) -> Result<StoredContinuityRecord, String> {
        let now = SystemTimeMillis::now();
        let phase = match record.phase {
            super::continuity::ContinuityPhase::Submitted => StoredContinuityPhase::Started,
            super::continuity::ContinuityPhase::Completed => StoredContinuityPhase::Completed,
            super::continuity::ContinuityPhase::Rejected => StoredContinuityPhase::Failed,
        };
        let terminal = (!phase.is_active()).then_some(now);
        let runtime_record = super::continuity::encode_record_payload(record)
            .map_err(|error| format!("durable_store_encode_failed:{error}"))?;
        let existing = self
            .get(&record.request_key)
            .map_err(|error| format!("durable_store_read_failed:{error}"))?;
        let cached_response = response_replay_cache()
            .lock()
            .unwrap()
            .responses
            .get(&record.request_key)
            .cloned();
        let (response_metadata, response_body) = match existing.as_ref() {
            Some(stored) => (
                stored.response_metadata.clone(),
                stored.response_body.clone(),
            ),
            None => (
                cached_response
                    .as_ref()
                    .map(|_| vec![("replayable".to_string(), "true".to_string())])
                    .unwrap_or_default(),
                cached_response.unwrap_or_default(),
            ),
        };
        Ok(StoredContinuityRecord {
            operation_key: record.request_key.clone(),
            request_hash: record.payload_hash.clone(),
            request_body: record.request_payload().to_vec(),
            runtime_record,
            owner_node: record.owner_node.clone(),
            ownership_generation: record.promotion_epoch,
            attempts: vec![record.attempt_id.clone()],
            phase,
            replica_set: record.acknowledged_replica_nodes().to_vec(),
            created_at_millis: existing
                .as_ref()
                .map_or(now, |stored| stored.created_at_millis),
            updated_at_millis: now,
            terminal_at_millis: terminal,
            expires_at_millis: terminal.map(|time| {
                time.saturating_add(
                    runtime_continuity_config(super::autonomous::embedded_autonomous_config())
                        .terminal_retention_millis,
                )
            }),
            response_metadata,
            response_body,
            control_term: record.promotion_epoch,
            schema_version: SCHEMA_VERSION,
            version: record.record_version,
        })
    }
}

/// Writes a store's records in group commits: one transaction for every
/// write that arrives within a short window, from one writer thread.
struct DurableWriter {
    sender: crossbeam_channel::Sender<DurableWrite>,
}

impl DurableWriter {
    fn start(store: Arc<SqliteContinuityStore>) -> Self {
        let (sender, receiver) =
            crossbeam_channel::bounded::<DurableWrite>(DURABLE_WRITE_QUEUE_ITEMS);
        std::thread::Builder::new()
            .name("mesh-continuity-group-commit".to_string())
            .spawn(move || {
                while let Ok(first) = receiver.recv() {
                    let mut writes = Vec::with_capacity(DURABLE_WRITE_BATCH_ITEMS);
                    writes.push(first);
                    let deadline = Instant::now() + DURABLE_WRITE_BATCH_WINDOW;
                    while writes.len() < DURABLE_WRITE_BATCH_ITEMS {
                        let remaining = deadline.saturating_duration_since(Instant::now());
                        match receiver.recv_timeout(remaining) {
                            Ok(write) => writes.push(write),
                            Err(_) => break,
                        }
                    }
                    commit_durable_writes(&store, writes);
                }
            })
            .expect("failed to spawn continuity group-commit thread");
        Self { sender }
    }

    /// Persists `record` with the next group commit and waits for it;
    /// `operation` names the caller in errors.
    fn persist(&self, record: StoredContinuityRecord, operation: &str) -> Result<(), String> {
        let (reply, result) = crate::actor::cooperative_channel();
        self.sender
            .try_send(DurableWrite { record, reply })
            .map_err(|error| match error {
                crossbeam_channel::TrySendError::Full(_) => {
                    format!("continuity_{operation}_group_commit_queue_full")
                }
                crossbeam_channel::TrySendError::Disconnected(_) => {
                    format!("continuity_{operation}_group_commit_unavailable")
                }
            })?;
        crate::actor::cooperative_recv_timeout(&result, Duration::from_secs(4)).map_err(
            |error| match error {
                mpsc::RecvTimeoutError::Timeout => {
                    format!("continuity_{operation}_group_commit_timeout")
                }
                mpsc::RecvTimeoutError::Disconnected => {
                    format!("continuity_{operation}_group_commit_unavailable")
                }
            },
        )?
    }
}

fn commit_durable_writes(store: &SqliteContinuityStore, writes: Vec<DurableWrite>) {
    let records: Vec<_> = writes.iter().map(|write| write.record.clone()).collect();
    match store.upsert_batch(&records) {
        Ok(()) => {
            for write in writes {
                let _ = write.reply.send(Ok(()));
            }
        }
        Err(error) if writes.len() == 1 => {
            let _ = writes[0].reply.send(Err(error));
        }
        Err(_) => {
            // A stale or fenced record must not roll back unrelated writes
            // that merely arrived during the same group-commit window.
            for write in writes {
                let result = store.upsert(&write.record);
                let _ = write.reply.send(result);
            }
        }
    }
}

/// Persists `record` through `store`'s group commit.
fn persist_runtime(
    store: &Arc<SqliteContinuityStore>,
    writer: &DurableWriter,
    record: &super::continuity::ContinuityRecord,
    operation: &str,
) -> Result<(), String> {
    writer.persist(store.stored_runtime_record(record)?, operation)
}

fn configured_writer(store: &Arc<SqliteContinuityStore>) -> &'static DurableWriter {
    DURABLE_WRITER.get_or_init(|| DurableWriter::start(Arc::clone(store)))
}

pub(crate) fn persist_replica_prepare(
    record: &super::continuity::ContinuityRecord,
) -> Result<(), String> {
    let Some(store) = configured_continuity_store() else {
        return Ok(());
    };
    persist_runtime(store, configured_writer(store), record, "prepare")
}

pub(crate) fn persist_runtime_record(
    _watermark: u64,
    record: &super::continuity::ContinuityRecord,
) {
    let Some(store) = configured_continuity_store() else {
        return;
    };
    if let Err(error) = persist_runtime(store, configured_writer(store), record, "runtime") {
        eprintln!(
            "mesh continuity: durable_store_write_failed operation={} reason={}",
            record.request_key, error
        );
    }
}

struct SystemTimeMillis;

impl SystemTimeMillis {
    fn now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(key: &str, version: u64, phase: StoredContinuityPhase) -> StoredContinuityRecord {
        let terminal = (!phase.is_active()).then_some(10);
        StoredContinuityRecord {
            operation_key: key.to_string(),
            request_hash: "hash".to_string(),
            request_body: b"request".to_vec(),
            runtime_record: Vec::new(),
            owner_node: "owner".to_string(),
            ownership_generation: 1,
            attempts: vec!["attempt-1".to_string()],
            phase,
            replica_set: vec!["replica".to_string()],
            created_at_millis: 1,
            updated_at_millis: 10,
            terminal_at_millis: terminal,
            expires_at_millis: terminal.map(|_| 20),
            response_metadata: vec![("status".to_string(), "200".to_string())],
            response_body: b"ok".to_vec(),
            control_term: 1,
            schema_version: SCHEMA_VERSION,
            version,
        }
    }

    fn store() -> SqliteContinuityStore {
        SqliteContinuityStore::open(Path::new(":memory:"), ContinuityStoreLimits::default())
            .expect("in-memory store")
    }

    #[test]
    fn upsert_rejects_stale_version_without_replacing_record() {
        let store = store();
        store
            .upsert(&record("operation", 2, StoredContinuityPhase::Completed))
            .expect("new record");
        store
            .upsert(&record("operation", 1, StoredContinuityPhase::Failed))
            .expect("stale upsert is idempotently ignored");

        assert_eq!(
            store
                .get("operation")
                .expect("lookup")
                .expect("record")
                .phase,
            StoredContinuityPhase::Completed
        );
    }

    #[test]
    fn batch_upsert_commits_every_record_and_replication_log_entry() {
        let store = store();
        let records: Vec<_> = (0..128)
            .map(|index| {
                record(
                    &format!("operation-{index}"),
                    1,
                    StoredContinuityPhase::Started,
                )
            })
            .collect();

        store.upsert_batch(&records).expect("batch upsert");

        let stats = store.stats().expect("batch stats");
        assert_eq!(stats.records, 128);
        assert_eq!(stats.active_records, 128);
        assert_eq!(stats.log_entries, 128);
        for item in records {
            assert_eq!(
                store
                    .get(&item.operation_key)
                    .expect("batch lookup")
                    .expect("batch record"),
                item
            );
        }
    }

    #[test]
    fn batch_upsert_rolls_back_every_record_when_one_is_fenced() {
        let store = store();
        store
            .upsert(&record("fenced", 2, StoredContinuityPhase::Completed))
            .expect("terminal record");
        store.compact(30).expect("create tombstone");
        let log_entries_before = store.stats().expect("pre-batch stats").log_entries;

        let records = vec![
            record("would-have-committed", 1, StoredContinuityPhase::Started),
            record("fenced", 1, StoredContinuityPhase::Started),
        ];
        assert_eq!(
            store.upsert_batch(&records),
            Err("continuity_store_tombstone_fenced".to_string())
        );

        assert!(store
            .get("would-have-committed")
            .expect("rolled-back lookup")
            .is_none());
        let stats = store.stats().expect("rolled-back stats");
        assert_eq!(stats.records, 0);
        assert_eq!(stats.log_entries, log_entries_before);
        assert_eq!(stats.tombstones, 1);
    }

    #[test]
    fn group_commit_isolates_a_fenced_record_from_valid_writes() {
        let store = store();
        store
            .upsert(&record("fenced", 2, StoredContinuityPhase::Completed))
            .expect("terminal record");
        store.compact(30).expect("create tombstone");
        let (valid_reply, valid_result) = crate::actor::cooperative_channel();
        let (fenced_reply, fenced_result) = crate::actor::cooperative_channel();

        commit_durable_writes(
            &store,
            vec![
                DurableWrite {
                    record: record("valid", 1, StoredContinuityPhase::Started),
                    reply: valid_reply,
                },
                DurableWrite {
                    record: record("fenced", 1, StoredContinuityPhase::Started),
                    reply: fenced_reply,
                },
            ],
        );

        assert_eq!(
            (
                valid_result.recv().expect("valid reply"),
                fenced_result.recv().expect("fenced reply").is_err(),
                store.get("valid").expect("valid lookup").is_some(),
            ),
            (Ok(()), true, true)
        );
    }

    #[test]
    fn compaction_never_removes_active_record() {
        let store = store();
        store
            .upsert(&record("active", 1, StoredContinuityPhase::Started))
            .expect("active record");
        store
            .upsert(&record("terminal", 1, StoredContinuityPhase::Completed))
            .expect("terminal record");
        let outcome = store.compact(30).expect("compaction");

        assert_eq!(outcome.records_tombstoned, 1);
        assert!(store.get("active").expect("lookup").is_some());
    }

    #[test]
    fn stats_distinguish_active_terminal_tombstone_and_log_state() {
        let store = store();
        store
            .upsert(&record("active", 1, StoredContinuityPhase::Started))
            .expect("active record");
        store
            .upsert(&record("terminal", 1, StoredContinuityPhase::Completed))
            .expect("terminal record");
        let before = store.stats().expect("stats before compaction");
        assert_eq!(before.records, 2);
        assert_eq!(before.active_records, 1);
        assert_eq!(before.terminal_records, 1);
        assert_eq!(before.log_entries, 2);

        store.compact(30).expect("compaction");
        let after = store.stats().expect("stats after compaction");
        assert_eq!(after.records, 1);
        assert_eq!(after.active_records, 1);
        assert_eq!(after.terminal_records, 0);
        assert_eq!(after.tombstones, 1);
    }

    #[test]
    fn disk_limit_rejects_new_work_without_evicting_active_records() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("continuity.db");
        {
            let store = SqliteContinuityStore::open(&path, ContinuityStoreLimits::default())
                .expect("initial store");
            store
                .upsert(&record("active", 1, StoredContinuityPhase::Started))
                .expect("active record");
        }
        let store = SqliteContinuityStore::open(
            &path,
            ContinuityStoreLimits {
                max_disk_bytes: 1,
                ..ContinuityStoreLimits::default()
            },
        )
        .expect("limited store");

        assert_eq!(
            store.upsert(&record("new", 1, StoredContinuityPhase::Started)),
            Err("continuity_store_disk_limit_reached".to_string())
        );
        assert!(store.get("active").expect("lookup active").is_some());
        assert!(store.get("new").expect("lookup rejected").is_none());
    }

    #[test]
    fn drain_safety_uses_durable_owner_and_complete_replica_set() {
        let store = store();
        let mut active = record("active", 1, StoredContinuityPhase::Started);
        active.owner_node = "worker-a".to_string();
        active.replica_set = vec!["worker-b".to_string(), "worker-c".to_string()];
        store.upsert(&active).expect("active record");
        store
            .upsert(&record("terminal", 1, StoredContinuityPhase::Completed))
            .expect("terminal record");

        let all_live = BTreeSet::from([
            "worker-a".to_string(),
            "worker-b".to_string(),
            "worker-c".to_string(),
        ]);
        let owner = store
            .node_safety("worker-a", &all_live)
            .expect("owner safety");
        assert_eq!(owner.active_owned_records, 1);
        assert_eq!(owner.required_replica_responsibilities, 0);
        assert!(!owner.only_active_copy);

        let replica = store
            .node_safety("worker-b", &BTreeSet::from(["worker-b".to_string()]))
            .expect("replica safety");
        assert_eq!(replica.active_owned_records, 0);
        assert_eq!(replica.required_replica_responsibilities, 1);
        assert!(replica.only_active_copy);
    }

    #[test]
    fn tombstone_prevents_delayed_record_resurrection() {
        let store = store();
        store
            .upsert(&record("operation", 2, StoredContinuityPhase::Completed))
            .expect("terminal record");
        store.compact(30).expect("compaction");

        assert_eq!(
            store.upsert(&record("operation", 1, StoredContinuityPhase::Started)),
            Err("continuity_store_tombstone_fenced".to_string())
        );
    }

    #[test]
    fn interrupted_snapshot_resumes_from_next_verified_chunk() {
        let source = store();
        for index in 0..8 {
            source
                .upsert(&record(
                    &format!("operation-{index}"),
                    1,
                    StoredContinuityPhase::Completed,
                ))
                .expect("source record");
        }
        let chunks = source.snapshot_chunks(512).expect("snapshot chunks");
        assert!(chunks.len() > 1);
        let target = store();
        target
            .apply_snapshot_chunk(&chunks[0])
            .expect("first chunk");
        for chunk in &chunks[1..] {
            target.apply_snapshot_chunk(chunk).expect("resumed chunk");
        }

        assert_eq!(target.all_records().expect("target records").len(), 8);
    }

    #[test]
    fn snapshot_chunks_preserve_payloads_and_checksums_at_boundaries() {
        for count in [0, 1, 6] {
            let source = store();
            for index in 0..count {
                let mut value = record(
                    &format!("operation-{index}"),
                    1,
                    StoredContinuityPhase::Completed,
                );
                value.request_hash = "quotes\"\\\n\té🦀".to_string();
                value.response_body = vec![0, 10, 127, 255];
                source.upsert(&value).unwrap();
            }
            let records = source.all_records().unwrap();
            let single_bound = records
                .iter()
                .map(|record| serde_json::to_vec(record).unwrap().len() + 2)
                .max()
                .unwrap_or(128);
            let pair_bound = serde_json::to_vec(&records[..records.len().min(2)])
                .unwrap()
                .len()
                .max(single_bound);
            let full_bound = serde_json::to_vec(&records).unwrap().len().max(128);
            for bound in [
                single_bound,
                single_bound + 1,
                pair_bound,
                full_bound,
                usize::MAX,
            ] {
                // The previous algorithm's greedy serialization is the wire-format oracle.
                let mut expected = Vec::new();
                let mut current = Vec::new();
                for record in &records {
                    current.push(record);
                    if serde_json::to_vec(&current).unwrap().len() > bound {
                        current.pop();
                        expected.push(serde_json::to_vec(&current).unwrap());
                        current.clear();
                        current.push(record);
                    }
                }
                if !current.is_empty() || expected.is_empty() {
                    expected.push(serde_json::to_vec(&current).unwrap());
                }
                let checksums: Vec<[u8; 32]> = expected
                    .iter()
                    .map(|payload| Sha256::digest(payload).into())
                    .collect();
                let snapshot_checksum: [u8; 32] = Sha256::digest(checksums.concat()).into();
                let chunks = source.snapshot_chunks(bound).unwrap();
                assert_eq!(chunks.len(), expected.len());
                for (sequence, chunk) in chunks.iter().enumerate() {
                    assert_eq!(chunk.payload, expected[sequence]);
                    assert!(chunk.payload.len() <= bound);
                    assert_eq!(chunk.checksum, checksums[sequence]);
                    assert_eq!(chunk.snapshot_checksum, snapshot_checksum);
                    assert_eq!(chunk.snapshot_id, format!("snapshot-{count}-{count}"));
                    assert_eq!(chunk.high_water_mark, count);
                    assert_eq!(chunk.sequence as usize, sequence);
                    assert_eq!(chunk.final_chunk, sequence + 1 == chunks.len());
                }
            }
        }
    }

    #[test]
    fn snapshot_rejects_small_bounds_and_oversized_records_at_every_position() {
        assert_eq!(
            store().snapshot_chunks(127),
            Err("continuity_snapshot_chunk_bound_too_small".to_string())
        );
        for oversized in 0..3 {
            let source = store();
            for index in 0..3 {
                let mut value = record(
                    &format!("operation-{index}"),
                    1,
                    StoredContinuityPhase::Completed,
                );
                if index == oversized {
                    value.response_body = vec![255; 1024];
                }
                source.upsert(&value).unwrap();
            }
            assert_eq!(
                source.snapshot_chunks(1024),
                Err("continuity_snapshot_record_exceeds_chunk_bound".to_string()),
                "oversized record at position {oversized}"
            );
        }
    }

    #[test]
    fn release_snapshot_resume_proof_reopens_receiver_store() {
        let proof = prove_interrupted_snapshot_resume().expect("snapshot resume proof");

        assert_eq!(proof.records, 64);
        assert!(proof.chunks > 1);
        assert_eq!(
            proof.resumed_from_sequence as usize,
            proof.acknowledged_before_interruption
        );
    }

    #[test]
    fn snapshot_rejects_corrupted_chunk() {
        let source = store();
        source
            .upsert(&record("operation", 1, StoredContinuityPhase::Completed))
            .expect("record");
        let mut chunk = source.snapshot_chunks(1024).expect("snapshot").remove(0);
        chunk.payload.push(0);

        assert_eq!(
            store().apply_snapshot_chunk(&chunk),
            Err("continuity_snapshot_checksum_mismatch".to_string())
        );
    }

    #[test]
    fn replication_log_compacts_only_through_every_acknowledged_safe_point() {
        let store = store();
        for index in 0..3 {
            store
                .upsert(&record(
                    &format!("operation-{index}"),
                    1,
                    StoredContinuityPhase::Completed,
                ))
                .expect("record");
        }
        store
            .acknowledge_replica_safe_point("replica-a", 3)
            .expect("first replica safe point");
        store
            .acknowledge_replica_safe_point("replica-b", 1)
            .expect("lagging replica safe point");
        let lagging = store.stats().expect("lagging replica telemetry");
        assert_eq!(lagging.replica_safe_point, Some(1));
        assert_eq!(lagging.replication_lag, Some(2));
        assert_eq!(lagging.compaction_lag, 1);
        assert_eq!(store.compact_log_to_replica_safe_point().unwrap(), 1);
        assert_eq!(store.log_entries_after(0, 10).unwrap().len(), 2);

        store
            .acknowledge_replica_safe_point("replica-b", 3)
            .expect("caught-up replica safe point");
        let caught_up = store.stats().expect("caught-up replica telemetry");
        assert_eq!(caught_up.replica_safe_point, Some(3));
        assert_eq!(caught_up.replication_lag, Some(0));
        assert_eq!(caught_up.compaction_lag, 2);
        assert_eq!(store.compact_log_to_replica_safe_point().unwrap(), 2);
        assert!(store.log_entries_after(0, 10).unwrap().is_empty());
    }

    /// Runs `sql` on the store's own connection, to age or damage its data.
    fn execute(store: &SqliteContinuityStore, sql: &str) {
        execute_batch(store.connection.lock().unwrap().raw, sql).expect(sql);
    }

    #[test]
    fn every_phase_round_trips_through_its_stored_name() {
        let phases = [
            StoredContinuityPhase::Reserved,
            StoredContinuityPhase::Replicating,
            StoredContinuityPhase::Admitted,
            StoredContinuityPhase::Started,
            StoredContinuityPhase::Completed,
            StoredContinuityPhase::Failed,
            StoredContinuityPhase::Indeterminate,
            StoredContinuityPhase::Expired,
            StoredContinuityPhase::Tombstoned,
        ];
        for phase in phases {
            assert_eq!(StoredContinuityPhase::parse(phase.as_str()), Ok(phase));
        }
        assert_eq!(
            StoredContinuityPhase::parse("paused"),
            Err("continuity_store_phase_invalid:paused".to_string())
        );
    }

    #[test]
    fn records_are_refused_for_each_broken_invariant() {
        let base = record("operation", 1, StoredContinuityPhase::Started);
        let cases: [(fn(&mut StoredContinuityRecord), &str); 6] = [
            (
                |record| record.attempts.push(String::new()),
                "continuity_store_record_identity_invalid",
            ),
            (
                |record| record.version = 0,
                "continuity_store_record_version_invalid",
            ),
            (
                |record| record.schema_version = SCHEMA_VERSION + 1,
                "continuity_store_record_version_invalid",
            ),
            (
                |record| record.replica_set.push("replica".to_string()),
                "continuity_store_replica_set_invalid",
            ),
            (
                |record| record.replica_set.push("owner".to_string()),
                "continuity_store_replica_set_invalid",
            ),
            (
                |record| record.terminal_at_millis = Some(5),
                "continuity_store_active_record_terminal_timestamp",
            ),
        ];
        for (change, expected) in cases {
            let mut broken = base.clone();
            change(&mut broken);
            assert_eq!(broken.validate(), Err(expected.to_string()));
            assert_eq!(store().upsert(&broken), Err(expected.to_string()));
        }
    }

    #[test]
    fn stores_open_only_with_valid_limits_and_paths() {
        let invalid = ContinuityStoreLimits {
            compaction_batch_size: 0,
            ..ContinuityStoreLimits::default()
        };
        assert_eq!(
            SqliteContinuityStore::open(Path::new(":memory:"), invalid).err(),
            Some("continuity_store_limits_invalid".to_string())
        );
        assert_eq!(
            SqliteContinuityStore::open(Path::new("bad\0path"), ContinuityStoreLimits::default())
                .err(),
            Some("continuity_store_path_contains_nul".to_string())
        );
        let directory = tempfile::tempdir().expect("tempdir");
        assert!(
            SqliteContinuityStore::open(directory.path(), ContinuityStoreLimits::default())
                .expect_err("a directory is no database")
                .starts_with("continuity_store_database_error:")
        );
        let blocked = directory.path().join("file");
        std::fs::write(&blocked, "").unwrap();
        assert!(SqliteContinuityStore::open(
            &blocked.join("continuity.db"),
            ContinuityStoreLimits::default()
        )
        .expect_err("a path under a file")
        .starts_with("continuity_store_directory_failed:"));
    }

    #[test]
    fn an_older_store_gains_its_later_columns_and_counter() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("continuity.db");
        {
            let store = SqliteContinuityStore::open(&path, ContinuityStoreLimits::default())
                .expect("store");
            store
                .upsert(&record("terminal", 1, StoredContinuityPhase::Completed))
                .expect("terminal record");
            // Back to the first schema: no request body, runtime record, or
            // terminal counter.
            execute(
                &store,
                "DROP TRIGGER continuity_terminal_count_insert;
                 DROP TRIGGER continuity_terminal_count_delete;
                 DROP TRIGGER continuity_terminal_count_update;
                 DROP TABLE continuity_store_counters;
                 ALTER TABLE continuity_records DROP COLUMN request_body;
                 ALTER TABLE continuity_records DROP COLUMN runtime_record;",
            );
        }
        let store =
            SqliteContinuityStore::open(&path, ContinuityStoreLimits::default()).expect("reopen");
        let migrated = store.get("terminal").expect("lookup").expect("record");
        assert!(migrated.request_body.is_empty() && migrated.runtime_record.is_empty());
        assert_eq!(store.stats().expect("stats").terminal_records, 1);
    }

    #[test]
    fn terminal_record_limit_compacts_before_refusing() {
        let limits = ContinuityStoreLimits {
            max_terminal_records: 1,
            ..ContinuityStoreLimits::default()
        };
        let store = SqliteContinuityStore::open(Path::new(":memory:"), limits).expect("store");
        // Its retention ended long ago: compaction makes room.
        store
            .upsert(&record("expired", 1, StoredContinuityPhase::Completed))
            .expect("first terminal record");
        store
            .upsert(&record("second", 1, StoredContinuityPhase::Completed))
            .expect("room after compaction");
        assert!(store.get("expired").expect("lookup").is_none());

        execute(
            &store,
            "UPDATE continuity_records SET expires_at_millis = 9223372036854775807",
        );
        assert_eq!(
            store.upsert(&record("third", 1, StoredContinuityPhase::Completed)),
            Err("continuity_store_terminal_record_limit_reached".to_string())
        );
    }

    #[test]
    fn responses_are_stored_on_their_record_only() {
        let store = store();
        store
            .upsert(&record("operation", 1, StoredContinuityPhase::Completed))
            .expect("record");
        store
            .update_response("operation", b"response")
            .expect("update");
        let updated = store.get("operation").unwrap().unwrap();
        assert_eq!(updated.response_body, b"response");
        assert_eq!(
            updated.response_metadata,
            [("replayable".to_string(), "true".to_string())]
        );
        assert_eq!(
            store.update_response("missing", b"response"),
            Err("continuity_response_record_missing".to_string())
        );
    }

    #[test]
    fn damaged_rows_are_refused_and_reads_roll_back() {
        let damages = [
            (
                "UPDATE continuity_records SET attempts_json = 'x'",
                "continuity_store_attempts_corrupt",
            ),
            (
                "UPDATE continuity_records SET replica_set_json = 'x'",
                "continuity_store_replicas_corrupt",
            ),
            (
                "UPDATE continuity_records SET response_metadata_json = 'x'",
                "continuity_store_response_metadata_corrupt",
            ),
            (
                "UPDATE continuity_records SET phase = 'paused'",
                "continuity_store_phase_invalid:paused",
            ),
            (
                "UPDATE continuity_records SET schema_version = -1",
                "continuity_store_schema_version_corrupt",
            ),
            (
                "UPDATE continuity_records SET created_at_millis = -1",
                "continuity_store_negative_integer",
            ),
        ];
        for (damage, expected) in damages {
            let store = store();
            store
                .upsert(&record("operation", 1, StoredContinuityPhase::Completed))
                .expect("record");
            execute(&store, damage);
            assert_eq!(store.snapshot_chunks(1024), Err(expected.to_string()));
            // The snapshot's read transaction was rolled back.
            execute(&store, "BEGIN; COMMIT;");
        }
    }

    /// Each statement's failure is the operation's error. A table dropped
    /// under the store stands in for the I/O and corruption errors SQLite
    /// reports at the statement that meets them.
    #[test]
    fn a_statement_that_fails_fails_its_operation() {
        let active = || record("operation", 1, StoredContinuityPhase::Started);
        let missing = |error: Result<(), String>, table: &str| {
            let error = error.expect_err(table);
            assert!(
                error.contains(&format!("no such table: {table}")),
                "{error}"
            );
        };
        let without = |table: &str| {
            let store = self::store();
            execute(&store, &format!("DROP TABLE {table}"));
            store
        };

        let store = without("continuity_replica_safe_points");
        missing(store.stats().map(drop), "continuity_replica_safe_points");
        missing(
            store.acknowledge_replica_safe_point("replica", 1),
            "continuity_replica_safe_points",
        );
        let store = without("continuity_tombstones");
        missing(store.upsert(&active()), "continuity_tombstones");
        let store = without("continuity_records");
        missing(store.upsert(&active()), "continuity_records");
        missing(store.all_records().map(drop), "continuity_records");
        missing(
            store.update_response("operation", b"ok"),
            "continuity_records",
        );
        missing(store.compact(10).map(drop), "continuity_records");
        let store = without("continuity_log");
        missing(store.upsert(&active()), "continuity_log");
        assert_eq!(store.get("operation"), Ok(None));
        missing(store.log_entries_after(0, 1).map(drop), "continuity_log");

        // A write a store refuses fails when it steps, and changes nothing.
        let store = self::store();
        execute(&store, "PRAGMA query_only = ON");
        for refused in [
            store.upsert(&active()),
            store.acknowledge_replica_safe_point("replica", 1),
        ] {
            let refused = refused.expect_err("read-only store");
            assert!(refused.contains("readonly"), "{refused}");
        }
        execute(&store, "PRAGMA query_only = OFF");
        assert_eq!(store.get("operation"), Ok(None));

        // A tombstone's expiry past SQLite's integers is refused.
        let store = self::store();
        store
            .upsert(&record("expired", 1, StoredContinuityPhase::Completed))
            .unwrap();
        assert!(store.compact(i64::MAX as u64).is_err());
        assert!(store.get("expired").unwrap().is_some());
    }

    #[test]
    fn concurrent_writes_share_a_group_commit() {
        let store = Arc::new(store());
        let writer = Arc::new(DurableWriter::start(Arc::clone(&store)));
        let start = Arc::new(std::sync::Barrier::new(8));
        let writers: Vec<_> = (0..8)
            .map(|index| {
                let (writer, start) = (Arc::clone(&writer), Arc::clone(&start));
                std::thread::spawn(move || {
                    start.wait();
                    writer.persist(
                        record(
                            &format!("batched-{index}"),
                            1,
                            StoredContinuityPhase::Started,
                        ),
                        "runtime",
                    )
                })
            })
            .collect();
        for writer in writers {
            assert_eq!(writer.join().unwrap(), Ok(()));
        }
        assert_eq!(store.stats().unwrap().records, 8);
    }

    #[test]
    fn a_failed_compaction_changes_nothing() {
        let store = store();
        store
            .upsert(&record("terminal", 1, StoredContinuityPhase::Completed))
            .expect("record");
        execute(&store, "DROP TABLE continuity_tombstones");
        assert!(store
            .compact(30)
            .unwrap_err()
            .starts_with("continuity_store_database_error:"));
        assert!(store.get("terminal").expect("lookup").is_some());
        // So does a failed log compaction.
        store.acknowledge_replica_safe_point("replica", 1).unwrap();
        execute(&store, "DROP TABLE continuity_log");
        assert!(store
            .compact_log_to_replica_safe_point()
            .unwrap_err()
            .starts_with("continuity_store_database_error:"));
        execute(&store, "BEGIN; COMMIT;");
    }

    #[test]
    fn compaction_extends_tombstones_and_expires_them() {
        let store = store();
        store
            .upsert(&record("operation", 1, StoredContinuityPhase::Completed))
            .expect("record");
        store.compact(30).expect("tombstone");
        // A later version terminates again: its tombstone is extended.
        execute(&store, "DELETE FROM continuity_tombstones");
        store
            .upsert(&record("operation", 2, StoredContinuityPhase::Completed))
            .expect("newer record");
        execute(
            &store,
            "INSERT INTO continuity_tombstones VALUES ('operation', 1, 0, 40)",
        );
        let outcome = store.compact(30).expect("extend tombstone");
        assert_eq!(outcome.records_tombstoned, 1);
        let far = ContinuityStoreLimits::default().tombstone_retention_millis + 31;
        assert_eq!(
            store.compact(far).expect("expire tombstone"),
            CompactionOutcome {
                records_tombstoned: 0,
                tombstones_deleted: 1,
            }
        );
    }

    #[test]
    fn the_replication_log_refuses_what_does_not_verify() {
        let store = store();
        store
            .upsert(&record("operation", 1, StoredContinuityPhase::Completed))
            .expect("record");
        assert_eq!(store.high_water_mark(), Ok(1));
        assert_eq!(
            store.log_entries_after(0, 0),
            Err("continuity_log_batch_limit_zero".to_string())
        );
        let entry = store.log_entries_after(0, 10).unwrap().remove(0);
        assert!(entry.verify());
        let target = self::store();
        target.apply_log_entry(&entry).expect("apply");
        assert_eq!(target.get("operation").unwrap().unwrap(), entry.record);
        for tampered in [
            ContinuityLogEntry {
                version: 2,
                ..entry.clone()
            },
            ContinuityLogEntry {
                operation_key: "other".to_string(),
                ..entry.clone()
            },
            ContinuityLogEntry {
                checksum: [0; 32],
                ..entry.clone()
            },
        ] {
            assert!(!tampered.verify());
            assert_eq!(
                target.apply_log_entry(&tampered),
                Err("continuity_log_entry_checksum_mismatch".to_string())
            );
        }
        let damages = [
            (
                "UPDATE continuity_log SET checksum = X'00'",
                "continuity_log_checksum_invalid",
            ),
            (
                "UPDATE continuity_log SET checksum = zeroblob(32)",
                "continuity_log_entry_checksum_mismatch",
            ),
            (
                "UPDATE continuity_log SET record_json = X'00'",
                "continuity_log_record_decode_failed:",
            ),
        ];
        for (damage, expected) in damages {
            execute(&store, damage);
            assert!(
                store
                    .log_entries_after(0, 10)
                    .unwrap_err()
                    .starts_with(expected),
                "{expected}"
            );
        }
        assert_eq!(
            store.acknowledge_replica_safe_point(" ", 1),
            Err("continuity_replica_safe_point_node_missing".to_string())
        );
        assert_eq!(store.compact_log_to_replica_safe_point(), Ok(0));
    }

    #[test]
    fn the_replay_cache_ignores_oversized_responses_and_evicts_the_oldest() {
        let mut cache = ResponseReplayCache::default();
        // Zeroed and never touched: no memory is committed for it.
        cache.insert("huge", &vec![0; MAX_REPLAY_BYTES + 1]);
        assert!(cache.responses.is_empty());
        cache.insert("first", b"one");
        cache.insert("first", b"three");
        assert_eq!(cache.bytes, 5);
        assert_eq!(cache.insertion_order.len(), 1);
        for index in 0..MAX_REPLAY_RESPONSES {
            cache.insert(&format!("response-{index}"), b"x");
        }
        assert_eq!(cache.responses.len(), MAX_REPLAY_RESPONSES);
        assert!(!cache.responses.contains_key("first"));
        assert_eq!(cache.bytes, MAX_REPLAY_RESPONSES);
    }

    fn lookup<'a>(table: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            table
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.to_string())
        }
    }

    fn autonomous(
        durable_continuity: bool,
        path: Option<&str>,
    ) -> super::super::autonomous::RuntimeAutonomousConfig {
        let mut config: super::super::autonomous::RuntimeAutonomousConfig =
            serde_json::from_value(serde_json::json!({
                "schema_version": 4,
                "enabled": true,
                "policy_revision": 1,
                "policy": crate::dist::scaling::ScalingPolicy::default(),
                "gateway_nodes": 0,
                "template_revision": "v1",
                "reconcile_interval_millis": 1,
                "startup_timeout_millis": 1,
                "drain_timeout_millis": 1,
                "termination_timeout_millis": 1,
                "driver": {"kind": "disabled"},
            }))
            .expect("config");
        config.features.durable_continuity = durable_continuity;
        config.continuity.path = path.map(PathBuf::from);
        config
    }

    #[test]
    fn the_store_path_follows_the_environment_then_the_manifest() {
        let explicit = [("MESH_CONTINUITY_DB", "/data/explicit.db")];
        assert_eq!(
            continuity_database_path(None, &lookup(&explicit), None),
            Some(PathBuf::from("/data/explicit.db"))
        );
        assert_eq!(
            continuity_database_path(None, &lookup(&[("MESH_CONTINUITY_DB", " ")]), None),
            None
        );
        // Manual mode has no store unless one is named.
        assert_eq!(continuity_database_path(None, &lookup(&[]), None), None);
        let without_durability = autonomous(false, None);
        assert_eq!(
            continuity_database_path(Some(&without_durability), &lookup(&[]), None),
            None
        );
        let declared = autonomous(true, Some("/data/declared.db"));
        assert_eq!(
            continuity_database_path(Some(&declared), &lookup(&[]), None),
            Some(PathBuf::from("/data/declared.db"))
        );
        // Otherwise a node-private default, named for the node's identity.
        let default = autonomous(true, Some(""));
        let by_stable_id = continuity_database_path(
            Some(&default),
            &lookup(&[
                ("MESH_STABLE_NODE_ID", "stable"),
                ("MESH_DATA_DIR", "/var/mesh"),
            ]),
            Some("node@host"),
        )
        .unwrap();
        assert!(by_stable_id.starts_with("/var/mesh"));
        let by_name = continuity_database_path(Some(&default), &lookup(&[]), Some("node@host"));
        let unnamed = continuity_database_path(Some(&default), &lookup(&[]), None).unwrap();
        assert!(unnamed.starts_with(".mesh"));
        assert_ne!(by_name, Some(unnamed.clone()));
        assert_ne!(by_name, Some(by_stable_id.clone()));
        assert_eq!(
            unnamed.file_name().unwrap().len(),
            "continuity-".len() + 24 + ".db".len()
        );
    }

    #[test]
    fn chunk_size_and_durability_come_from_the_environment_or_config() {
        let config = super::super::autonomous::RuntimeContinuityConfig::default();
        assert_eq!(
            snapshot_chunk_bytes(
                &lookup(&[("MESH_CONTINUITY_SNAPSHOT_CHUNK_BYTES", "4096")]),
                &config
            ),
            4096
        );
        for ignored in ["64", "not-a-number", "999999999"] {
            assert_eq!(
                snapshot_chunk_bytes(
                    &lookup(&[("MESH_CONTINUITY_SNAPSHOT_CHUNK_BYTES", ignored)]),
                    &config
                ),
                1024 * 1024
            );
        }
        assert!(!durability_degraded(&lookup(&[]), &config));
        assert!(durability_degraded(
            &lookup(&[("MESH_CONTINUITY_DURABILITY", " Degraded ")]),
            &config
        ));
        let relaxed = super::super::autonomous::RuntimeContinuityConfig {
            strict_durability: false,
            ..config
        };
        assert!(durability_degraded(&lookup(&[]), &relaxed));
        assert!(!durability_degraded(
            &lookup(&[("MESH_CONTINUITY_DURABILITY", "strict")]),
            &relaxed
        ));
    }

    fn runtime_record(
        key: &str,
        phase: super::super::continuity::ContinuityPhase,
    ) -> super::super::continuity::ContinuityRecord {
        use super::super::continuity::*;
        ContinuityRecord {
            request_key: key.to_string(),
            payload_hash: "hash".to_string(),
            record_version: 1,
            request_payload: b"payload".to_vec(),
            attempt_id: attempt_id_from_token(1),
            phase,
            result: if phase == ContinuityPhase::Completed {
                ContinuityResult::Succeeded
            } else {
                ContinuityResult::Pending
            },
            ingress_node: "owner-node".to_string(),
            owner_node: "owner-node".to_string(),
            replica_nodes: vec!["replica-node".to_string()],
            acknowledged_replica_nodes: vec!["replica-node".to_string()],
            replica_node: "replica-node".to_string(),
            replication_count: 2,
            replica_status: ReplicaStatus::Mirrored,
            cluster_role: ContinuityClusterRole::Primary,
            promotion_epoch: 0,
            replication_health: ReplicationHealth::Healthy,
            execution_node: "owner-node".to_string(),
            routed_remotely: false,
            fell_back_locally: false,
            error: String::new(),
            declared_handler_runtime_name: "Api.handle".to_string(),
        }
    }

    #[test]
    fn non_durable_safety_counts_submitted_records_of_the_node() {
        use super::super::continuity::ContinuityPhase;
        let registry = super::super::continuity::ContinuityRegistry::new();
        registry
            .merge_remote_record(2, runtime_record("submitted", ContinuityPhase::Submitted))
            .expect("submitted record");
        registry
            .merge_remote_record(3, runtime_record("completed", ContinuityPhase::Completed))
            .expect("completed record");
        let both = BTreeSet::from(["owner-node".to_string(), "replica-node".to_string()]);
        assert_eq!(
            registry_node_safety(&registry, "owner-node", &both),
            ContinuityNodeSafety {
                active_owned_records: 1,
                required_replica_responsibilities: 0,
                only_active_copy: false,
            }
        );
        let replica_alone = BTreeSet::from(["replica-node".to_string()]);
        assert_eq!(
            registry_node_safety(&registry, "replica-node", &replica_alone),
            ContinuityNodeSafety {
                active_owned_records: 0,
                required_replica_responsibilities: 1,
                only_active_copy: true,
            }
        );
        assert_eq!(
            registry_node_safety(&registry, "elsewhere", &both),
            ContinuityNodeSafety::default()
        );
    }

    #[test]
    fn stores_replay_responses_and_rebuild_runtime_records() {
        use super::super::continuity::ContinuityPhase;
        let store = store();
        let submitted = runtime_record("stored-submitted", ContinuityPhase::Submitted);
        let stored = store.stored_runtime_record(&submitted).expect("stored");
        assert_eq!(stored.phase, StoredContinuityPhase::Started);
        assert_eq!(stored.terminal_at_millis, None);
        assert!(stored.response_body.is_empty() && stored.response_metadata.is_empty());
        store.upsert(&stored).expect("upsert");
        assert_eq!(store.replay_response("stored-submitted"), Ok(None));

        // A response cached before the record was stored is kept with it.
        let completed = runtime_record("stored-completed", ContinuityPhase::Completed);
        response_replay_cache()
            .lock()
            .unwrap()
            .insert("stored-completed", b"cached response");
        let stored = store.stored_runtime_record(&completed).expect("stored");
        assert_eq!(stored.phase, StoredContinuityPhase::Completed);
        assert!(stored.terminal_at_millis.is_some() && stored.expires_at_millis.is_some());
        assert_eq!(stored.response_body, b"cached response");
        store.upsert(&stored).expect("upsert");
        response_replay_cache()
            .lock()
            .unwrap()
            .responses
            .remove("stored-completed");
        assert_eq!(
            store.replay_response("stored-completed"),
            Ok(Some(b"cached response".to_vec()))
        );
        assert_eq!(
            replay_runtime_response("stored-completed"),
            Ok(Some(b"cached response".to_vec()))
        );
        // A later version keeps what the stored record had.
        let mut rejected = runtime_record("stored-completed", ContinuityPhase::Rejected);
        rejected.record_version = 2;
        let restored = store.stored_runtime_record(&rejected).expect("stored");
        assert_eq!(restored.phase, StoredContinuityPhase::Failed);
        assert_eq!(restored.response_body, b"cached response");

        let records = store.runtime_records().expect("runtime records");
        assert_eq!(records.len(), 2);
        assert_eq!(
            super::super::continuity::decode_record_payload(&records[1]).unwrap(),
            submitted
        );
    }

    #[test]
    fn the_group_commit_writer_answers_each_write() {
        use super::super::continuity::ContinuityPhase;
        let store = Arc::new(store());
        let writer = DurableWriter::start(Arc::clone(&store));
        persist_runtime(
            &store,
            &writer,
            &runtime_record("persisted", ContinuityPhase::Submitted),
            "runtime",
        )
        .expect("persist");
        assert!(store.get("persisted").unwrap().is_some());
        let fenced = record("persisted", 1, StoredContinuityPhase::Failed);
        store.upsert(&fenced).expect("stale version ignored");
        assert_eq!(
            writer.persist(
                StoredContinuityRecord {
                    version: 0,
                    ..fenced
                },
                "runtime"
            ),
            Err("continuity_store_record_version_invalid".to_string())
        );

        // A writer whose queue takes nothing more, or is gone.
        let (sender, receiver) = crossbeam_channel::bounded(0);
        let full = DurableWriter { sender };
        let entry = || record("queued", 1, StoredContinuityPhase::Started);
        assert_eq!(
            full.persist(entry(), "prepare"),
            Err("continuity_prepare_group_commit_queue_full".to_string())
        );
        drop(receiver);
        assert_eq!(
            full.persist(entry(), "prepare"),
            Err("continuity_prepare_group_commit_unavailable".to_string())
        );
        // A writer that drops the write unanswered.
        let (sender, receiver) = crossbeam_channel::bounded::<DurableWrite>(1);
        let dropping = std::thread::spawn(move || drop(receiver.recv()));
        assert_eq!(
            DurableWriter { sender }.persist(entry(), "runtime"),
            Err("continuity_runtime_group_commit_unavailable".to_string())
        );
        dropping.join().unwrap();
    }

    #[test]
    fn a_write_the_group_commit_never_answers_times_out() {
        let (sender, receiver) = crossbeam_channel::bounded::<DurableWrite>(1);
        let writer = DurableWriter { sender };
        assert_eq!(
            writer.persist(
                record("unanswered", 1, StoredContinuityPhase::Started),
                "runtime"
            ),
            Err("continuity_runtime_group_commit_timeout".to_string())
        );
        drop(receiver);
    }

    #[test]
    fn configured_stores_open_compact_and_report_failures() {
        let directory = tempfile::tempdir().expect("tempdir");
        let config = super::super::autonomous::RuntimeContinuityConfig::default();
        let store =
            open_configured_store(&directory.path().join("configured.db"), &config).expect("open");
        store
            .upsert(&record("terminal", 1, StoredContinuityPhase::Completed))
            .expect("record");
        store.acknowledge_replica_safe_point("replica", 1).unwrap();
        compaction_pass(&store);
        let stats = store.stats().unwrap();
        assert_eq!(
            (stats.records, stats.tombstones, stats.log_entries),
            (0, 1, 0)
        );
        compaction_pass(&store);
        execute(&store, "DROP TABLE continuity_tombstones");
        // A failed pass is reported, not fatal.
        compaction_pass(&store);

        let blocked = directory.path().join("file");
        std::fs::write(&blocked, "").unwrap();
        assert!(open_configured_store(&blocked.join("configured.db"), &config).is_none());
    }

    /// Nothing in this process configures a durable store.
    #[test]
    fn without_a_configured_store_the_runtime_keeps_memory_only_state() {
        use super::super::continuity::ContinuityPhase;
        assert!(configured_continuity_store().is_none());
        assert_eq!(
            continuity_node_safety("", &BTreeSet::new()),
            Err("continuity_safety_node_missing".to_string())
        );
        assert!(continuity_node_safety("unknown-node", &BTreeSet::new()).is_ok());
        assert_eq!(
            persist_runtime_response("", b"x"),
            Err("continuity_response_invalid".to_string())
        );
        assert_eq!(persist_runtime_response("memory-only", b"response"), Ok(()));
        assert_eq!(
            replay_runtime_response("memory-only"),
            Ok(Some(b"response".to_vec()))
        );
        assert_eq!(replay_runtime_response("never-stored"), Ok(None));
        assert_eq!(load_runtime_records(), Ok(Vec::new()));
        let record = runtime_record("memory-only", ContinuityPhase::Submitted);
        assert_eq!(persist_replica_prepare(&record), Ok(()));
        persist_runtime_record(1, &record);
        assert!(runtime_snapshot_chunk_bytes() >= 128);
        let _ = degraded_durability_enabled();
    }
}
