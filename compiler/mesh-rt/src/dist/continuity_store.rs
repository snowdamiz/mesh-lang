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
        if !self.raw.is_null() {
            unsafe {
                sqlite3_close(self.raw);
            }
        }
    }
}

struct Statement {
    database: *mut sqlite3,
    raw: *mut sqlite3_stmt,
}

impl Drop for Statement {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            unsafe {
                sqlite3_finalize(self.raw);
            }
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

    fn text(&self, column: c_int) -> Result<String, String> {
        let pointer = unsafe { sqlite3_column_text(self.raw, column) };
        if pointer.is_null() {
            return Ok(String::new());
        }
        Ok(unsafe { CStr::from_ptr(pointer.cast()) }
            .to_string_lossy()
            .into_owned())
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
        let count = |sql: &str| -> Result<u64, String> {
            let mut statement = Self::prepare(&connection, sql)?;
            if statement.step()? != SQLITE_ROW {
                return Ok(0);
            }
            unsigned_integer(statement.integer(0))
        };
        let records = count("SELECT COUNT(*) FROM continuity_records")?;
        let active_records = count(
            "SELECT COUNT(*) FROM continuity_records
             WHERE phase IN ('reserved', 'replicating', 'admitted', 'started')",
        )?;
        let terminal_records = count(
            "SELECT terminal_record_count FROM continuity_store_counters WHERE singleton = 1",
        )?;
        let tombstones = count("SELECT COUNT(*) FROM continuity_tombstones")?;
        let log_entries = count("SELECT COUNT(*) FROM continuity_log")?;
        let high_water_mark = count("SELECT COALESCE(MAX(sequence), 0) FROM continuity_log")?;
        let mut safe_point = Self::prepare(
            &connection,
            "SELECT COALESCE(MIN(high_water_mark), 0), COUNT(*)
               FROM continuity_replica_safe_points",
        )?;
        let replica_safe_point = if safe_point.step()? == SQLITE_ROW && safe_point.integer(1) > 0 {
            Some(unsigned_integer(safe_point.integer(0))?)
        } else {
            None
        };
        drop(safe_point);
        let compaction_lag = if let Some(replica_safe_point) = replica_safe_point {
            let mut eligible = Self::prepare(
                &connection,
                "SELECT COUNT(*) FROM continuity_log WHERE sequence <= ?1",
            )?;
            eligible.bind_i64(1, sqlite_integer(replica_safe_point)?)?;
            let count = if eligible.step()? == SQLITE_ROW {
                unsigned_integer(eligible.integer(0))?
            } else {
                0
            };
            drop(eligible);
            count
        } else {
            0
        };
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
            operation_key: statement.text(0)?,
            request_hash: statement.text(1)?,
            request_body: statement.blob(16),
            runtime_record: statement.blob(17),
            owner_node: statement.text(2)?,
            ownership_generation: unsigned_integer(statement.integer(3))?,
            attempts: serde_json::from_str(&statement.text(4)?)
                .map_err(|_| "continuity_store_attempts_corrupt".to_string())?,
            phase: StoredContinuityPhase::parse(&statement.text(5)?)?,
            replica_set: serde_json::from_str(&statement.text(6)?)
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
            response_metadata: serde_json::from_str(&statement.text(11)?)
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
        let connection = self.connection.lock().unwrap();
        let mut statement = Self::prepare(
            &connection,
            "SELECT terminal_record_count
               FROM continuity_store_counters WHERE singleton = 1",
        )?;
        if statement.step()? != SQLITE_ROW {
            return Ok(0);
        }
        unsigned_integer(statement.integer(0))
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
            let mut high_water = Self::prepare(
                &connection,
                "SELECT COALESCE(MAX(sequence), 0) FROM continuity_log",
            )?;
            let high_water = if high_water.step()? == SQLITE_ROW {
                unsigned_integer(high_water.integer(0))?
            } else {
                0
            };
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
                expired.push((select.text(0)?, unsigned_integer(select.integer(1))?));
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
        let mut current: Vec<StoredContinuityRecord> = Vec::new();
        for record in records {
            let mut candidate = current.clone();
            candidate.push(record.clone());
            let encoded = serde_json::to_vec(&candidate)
                .map_err(|error| format!("continuity_snapshot_encode_failed:{error}"))?;
            if encoded.len() > chunk_bytes && !current.is_empty() {
                payloads.push(
                    serde_json::to_vec(&current)
                        .map_err(|error| format!("continuity_snapshot_encode_failed:{error}"))?,
                );
                current = vec![record];
            } else if encoded.len() > chunk_bytes {
                return Err("continuity_snapshot_record_exceeds_chunk_bound".to_string());
            } else {
                current = candidate;
            }
        }
        if !current.is_empty() || payloads.is_empty() {
            payloads.push(
                serde_json::to_vec(&current)
                    .map_err(|error| format!("continuity_snapshot_encode_failed:{error}"))?,
            );
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
        for record in records {
            self.upsert(&record)?;
        }
        Ok(())
    }

    fn high_water_mark(&self) -> Result<u64, String> {
        let connection = self.connection.lock().unwrap();
        let mut statement = Self::prepare(
            &connection,
            "SELECT COALESCE(MAX(sequence), 0) FROM continuity_log",
        )?;
        if statement.step()? != SQLITE_ROW {
            return Ok(0);
        }
        unsigned_integer(statement.integer(0))
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
                operation_key: statement.text(1)?,
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
            let mut safe_point = Self::prepare(
                &connection,
                "SELECT MIN(high_water_mark), COUNT(*)
                   FROM continuity_replica_safe_points",
            )?;
            if safe_point.step()? != SQLITE_ROW || safe_point.integer(1) == 0 {
                return Ok(0);
            }
            let safe_point = unsigned_integer(safe_point.integer(0))?;
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
    if database.is_null() {
        return "continuity_store_database_error".to_string();
    }
    let pointer = unsafe { sqlite3_errmsg(database) };
    if pointer.is_null() {
        "continuity_store_database_error".to_string()
    } else {
        format!(
            "continuity_store_database_error:{}",
            unsafe { CStr::from_ptr(pointer) }.to_string_lossy()
        )
    }
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
static DURABLE_WRITER: OnceLock<crossbeam_channel::Sender<DurableWrite>> = OnceLock::new();

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
        while self.responses.len() > MAX_REPLAY_RESPONSES || self.bytes > MAX_REPLAY_BYTES {
            let Some(oldest) = self.insertion_order.pop_front() else {
                break;
            };
            if let Some(removed) = self.responses.remove(&oldest) {
                self.bytes = self.bytes.saturating_sub(removed.len());
            }
        }
    }
}

static RESPONSE_REPLAY_CACHE: OnceLock<Mutex<ResponseReplayCache>> = OnceLock::new();

fn response_replay_cache() -> &'static Mutex<ResponseReplayCache> {
    RESPONSE_REPLAY_CACHE.get_or_init(|| Mutex::new(ResponseReplayCache::default()))
}

pub fn configured_continuity_store() -> Option<&'static Arc<SqliteContinuityStore>> {
    CONFIGURED_STORE
        .get_or_init(|| {
            let config = runtime_continuity_config();
            let path = continuity_database_path(&config)?;
            let limits = ContinuityStoreLimits {
                terminal_retention_millis: config.terminal_retention_millis,
                tombstone_retention_millis: config.tombstone_retention_millis,
                max_terminal_records: config.max_terminal_records,
                max_disk_bytes: config.max_disk_bytes,
                compaction_batch_size: 1_000,
            };
            SqliteContinuityStore::open(&path, limits)
                .map(Arc::new)
                .inspect(|store| {
                    start_continuity_compactor(Arc::clone(store));
                })
                .map_err(|error| {
                    eprintln!("mesh continuity: durable_store_open_failed reason={error}");
                    error
                })
                .ok()
        })
        .as_ref()
}

fn runtime_continuity_config() -> super::autonomous::RuntimeContinuityConfig {
    super::autonomous::embedded_autonomous_config()
        .map(|config| config.continuity.clone())
        .unwrap_or_default()
}

fn continuity_database_path(
    config: &super::autonomous::RuntimeContinuityConfig,
) -> Option<PathBuf> {
    if let Ok(path) = std::env::var("MESH_CONTINUITY_DB") {
        return (!path.trim().is_empty()).then(|| PathBuf::from(path));
    }
    if super::autonomous::embedded_autonomous_config()
        .is_some_and(|autonomous| !autonomous.features.durable_continuity)
    {
        return None;
    }
    if let Some(path) = config
        .path
        .as_ref()
        .filter(|path| !path.as_os_str().is_empty())
    {
        return Some(path.clone());
    }
    // Manual mode preserves the historical opt-in behavior. Autonomous mode
    // always gets a node-private default store when no path is declared.
    super::autonomous::embedded_autonomous_config()?;
    let stable_id = std::env::var("MESH_STABLE_NODE_ID")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| super::node::node_state().map(|state| state.name.clone()))
        .unwrap_or_else(|| "mesh-local-node".to_string());
    let digest = Sha256::digest(stable_id.as_bytes());
    let suffix: String = digest[..12]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let directory = std::env::var_os("MESH_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".mesh"));
    Some(directory.join(format!("continuity-{suffix}.db")))
}

fn start_continuity_compactor(store: Arc<SqliteContinuityStore>) {
    let _ = std::thread::Builder::new()
        .name("mesh-continuity-compactor".to_string())
        .spawn(move || loop {
            std::thread::park_timeout(std::time::Duration::from_secs(30));
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
        });
}

pub(crate) fn runtime_snapshot_chunk_bytes() -> usize {
    std::env::var("MESH_CONTINUITY_SNAPSHOT_CHUNK_BYTES")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value >= 128)
        .unwrap_or_else(|| {
            runtime_continuity_config()
                .snapshot_chunk_bytes
                .try_into()
                .unwrap_or(usize::MAX)
        })
}

pub(crate) fn degraded_durability_enabled() -> bool {
    std::env::var("MESH_CONTINUITY_DURABILITY")
        .ok()
        .map(|value| value.trim().eq_ignore_ascii_case("degraded"))
        .unwrap_or_else(|| !runtime_continuity_config().strict_durability)
}

pub(crate) fn continuity_node_safety(
    node_id: &str,
    live_nodes: &BTreeSet<String>,
) -> Result<ContinuityNodeSafety, String> {
    if node_id.is_empty() {
        return Err("continuity_safety_node_missing".to_string());
    }
    if let Some(store) = configured_continuity_store() {
        return store.node_safety(node_id, live_nodes);
    }

    // Manual/non-durable mode retains the same conservative safety contract
    // using its single-replica in-memory record shape. Arbitrary replica sets
    // require the configured durable store and fail closed here when absent.
    let mut safety = ContinuityNodeSafety::default();
    for record in super::continuity::continuity_registry()
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
    Ok(safety)
}

pub(crate) fn persist_runtime_response(operation_key: &str, response: &[u8]) -> Result<(), String> {
    if operation_key.is_empty() || response.is_empty() {
        return Err("continuity_response_invalid".to_string());
    }
    response_replay_cache()
        .lock()
        .unwrap()
        .insert(operation_key, response);
    let Some(store) = configured_continuity_store() else {
        return Ok(());
    };
    store.update_response(operation_key, response)
}

pub(crate) fn replay_runtime_response(operation_key: &str) -> Result<Option<Vec<u8>>, String> {
    if let Some(response) = response_replay_cache()
        .lock()
        .unwrap()
        .responses
        .get(operation_key)
        .cloned()
    {
        return Ok(Some(response));
    }
    let Some(store) = configured_continuity_store() else {
        return Ok(None);
    };
    let response = store
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

pub(crate) fn load_runtime_records() -> Result<Vec<Vec<u8>>, String> {
    let Some(store) = configured_continuity_store() else {
        return Ok(Vec::new());
    };
    Ok(store
        .all_records()?
        .into_iter()
        .map(|record| record.runtime_record)
        .filter(|encoded| !encoded.is_empty())
        .collect())
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

fn stored_runtime_record(
    store: &SqliteContinuityStore,
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
    let existing = store
        .get(&record.request_key)
        .map_err(|error| format!("durable_store_read_failed:{error}"))?;
    let cached_response = response_replay_cache()
        .lock()
        .unwrap()
        .responses
        .get(&record.request_key)
        .cloned();
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
        expires_at_millis: terminal
            .map(|time| time.saturating_add(runtime_continuity_config().terminal_retention_millis)),
        response_metadata: existing.as_ref().map_or_else(
            || {
                cached_response
                    .as_ref()
                    .map(|_| vec![("replayable".to_string(), "true".to_string())])
                    .unwrap_or_default()
            },
            |stored| stored.response_metadata.clone(),
        ),
        response_body: existing.as_ref().map_or_else(
            || cached_response.unwrap_or_default(),
            |stored| stored.response_body.clone(),
        ),
        control_term: record.promotion_epoch,
        schema_version: SCHEMA_VERSION,
        version: record.record_version,
    })
}

fn durable_writer(
    store: &Arc<SqliteContinuityStore>,
) -> &'static crossbeam_channel::Sender<DurableWrite> {
    DURABLE_WRITER.get_or_init(|| {
        let (sender, receiver) =
            crossbeam_channel::bounded::<DurableWrite>(DURABLE_WRITE_QUEUE_ITEMS);
        let store = Arc::clone(store);
        std::thread::Builder::new()
            .name("mesh-continuity-group-commit".to_string())
            .spawn(move || {
                while let Ok(first) = receiver.recv() {
                    let mut writes = Vec::with_capacity(DURABLE_WRITE_BATCH_ITEMS);
                    writes.push(first);
                    let deadline = Instant::now() + DURABLE_WRITE_BATCH_WINDOW;
                    while writes.len() < DURABLE_WRITE_BATCH_ITEMS {
                        let Some(remaining) = deadline.checked_duration_since(Instant::now())
                        else {
                            break;
                        };
                        match receiver.recv_timeout(remaining) {
                            Ok(write) => writes.push(write),
                            Err(crossbeam_channel::RecvTimeoutError::Timeout) => break,
                            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                        }
                    }
                    commit_durable_writes(&store, writes);
                }
            })
            .expect("failed to spawn continuity group-commit thread");
        sender
    })
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
            let Some(write) = writes.into_iter().next() else {
                return;
            };
            let _ = write.reply.send(Err(error));
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

fn persist_with_group_commit(
    store: &Arc<SqliteContinuityStore>,
    record: StoredContinuityRecord,
    operation: &str,
) -> Result<(), String> {
    let (reply, result) = crate::actor::cooperative_channel();
    durable_writer(store)
        .try_send(DurableWrite { record, reply })
        .map_err(|error| match error {
            crossbeam_channel::TrySendError::Full(_) => {
                format!("continuity_{operation}_group_commit_queue_full")
            }
            crossbeam_channel::TrySendError::Disconnected(_) => {
                format!("continuity_{operation}_group_commit_unavailable")
            }
        })?;
    crate::actor::cooperative_recv_timeout(&result, Duration::from_secs(4)).map_err(|error| {
        match error {
            mpsc::RecvTimeoutError::Timeout => {
                format!("continuity_{operation}_group_commit_timeout")
            }
            mpsc::RecvTimeoutError::Disconnected => {
                format!("continuity_{operation}_group_commit_unavailable")
            }
        }
    })?
}

pub(crate) fn persist_replica_prepare(
    record: &super::continuity::ContinuityRecord,
) -> Result<(), String> {
    let Some(store) = configured_continuity_store() else {
        return Ok(());
    };
    let stored = stored_runtime_record(store, record)?;
    persist_with_group_commit(store, stored, "prepare")
}

pub(crate) fn persist_runtime_record(
    _watermark: u64,
    record: &super::continuity::ContinuityRecord,
) {
    let Some(store) = configured_continuity_store() else {
        return;
    };
    let result = stored_runtime_record(store, record)
        .and_then(|stored| persist_with_group_commit(store, stored, "runtime"));
    if let Err(error) = result {
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
}
