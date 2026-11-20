use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

use crate::db::pg::JobsAckError;

pub(super) trait Database {
    fn query(&mut self, sql: &str, params: &[&str]) -> Result<Vec<Vec<(String, String)>>, String>;
    fn acknowledge(
        &mut self,
        sql: &str,
        params: &[&str],
    ) -> Result<Vec<Vec<(String, String)>>, JobsAckError>;
    fn simple(&mut self, sql: &str) -> Result<(), String>;
    fn in_transaction(&self) -> bool;
}

#[derive(Debug)]
pub(super) struct Claim {
    pub(super) id: String,
    pub(super) token: String,
    pub(super) context: String,
    pub(super) timeout_ms: u64,
    pub(super) observed_at_ms: Option<u64>,
}

pub(super) const MAX_CLAIM_BATCH: usize = 16;

#[derive(Debug, Deserialize)]
pub(super) struct OversizedClaim {
    pub(super) id: String,
    pub(super) context_bytes: usize,
}

#[derive(Debug, Default)]
pub(super) struct ClaimBatch {
    pub(super) claims: Vec<Claim>,
    pub(super) oversized: Vec<OversizedClaim>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LeaseStatus {
    Active,
    Cancelled,
    Lost,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub(super) enum Outcome {
    Complete,
    Fail {
        reason: String,
    },
    RetryAfter {
        delay_ms: u64,
        reason: String,
    },
    Snooze {
        delay_ms: u64,
    },
    Cancel {
        reason: String,
    },
    Discard {
        reason: String,
    },
    #[serde(skip)]
    ReleaseUnstarted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Settlement {
    Completed,
    Cancelled,
    Discarded,
    Retryable,
    Scheduled,
    ReleasedUnstarted,
    Stale,
}

impl Settlement {
    fn from_sql(state: &str) -> Result<Self, JobsAckError> {
        match state {
            "completed" => Ok(Self::Completed),
            "cancelled" => Ok(Self::Cancelled),
            "discarded" => Ok(Self::Discarded),
            "retryable" => Ok(Self::Retryable),
            "scheduled" => Ok(Self::Scheduled),
            "released_unstarted" => Ok(Self::ReleasedUnstarted),
            "stale" => Ok(Self::Stale),
            _ => Err(JobsAckError::Unconfirmed(
                "mesh jobs invalid settlement state from PostgreSQL".into(),
            )),
        }
    }
}

const MAX_INPUT: usize = 1024 * 1024;
const MAX_BATCH: usize = 128;
const MAX_DURATION_MS: u64 = 365 * 24 * 60 * 60 * 1000;
pub(super) const MAX_TIMESTAMP_MS: u64 = 253_402_300_799_999;
const MAX_CONCURRENCY: u32 = 1024;
const MAX_PAGE: u32 = 1000;

pub(super) fn decode<T: DeserializeOwned>(input: &str) -> Result<T, String> {
    if input.len() > MAX_INPUT {
        return Err("mesh jobs JSON exceeds 1 MiB".into());
    }
    serde_json::from_str(input).map_err(|error| error.to_string())
}

pub(super) fn name(value: &str, field: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.len() > 255 || value.contains('\0') {
        return Err(format!(
            "mesh jobs {field} must be nonblank text of at most 255 bytes"
        ));
    }
    Ok(())
}

fn duration(value: u64, field: &str) -> Result<(), String> {
    if value > MAX_DURATION_MS {
        return Err(format!("mesh jobs {field} exceeds 365 days"));
    }
    Ok(())
}

fn lease_duration(value: u64) -> Result<(), String> {
    if value == 0 {
        return Err("mesh jobs lease_ms must be positive".into());
    }
    duration(value, "lease_ms")
}

fn scheduling(scheduled_at_ms: Option<u64>, delay_ms: Option<u64>) -> Result<(), String> {
    if let Some(timestamp) = scheduled_at_ms {
        if delay_ms.is_some() || timestamp > MAX_TIMESTAMP_MS {
            return Err("mesh jobs scheduled_at_ms must be a UTC timestamp through year 9999 and cannot be combined with delay_ms".into());
        }
    }
    duration(delay_ms.unwrap_or_default(), "delay_ms")
}

pub(super) fn response(
    db: &mut impl Database,
    sql: &str,
    params: &[&str],
) -> Result<String, String> {
    let mut rows = db.query(sql, params)?;
    if rows.len() != 1 || rows[0].len() != 1 {
        return Err("mesh jobs expected one result cell from PostgreSQL".into());
    }
    Ok(rows.remove(0).remove(0).1)
}

fn standalone(db: &impl Database) -> Result<(), String> {
    if db.in_transaction() {
        return Err("mesh jobs worker or migration requires an idle database connection".into());
    }
    Ok(())
}

fn rollback_error(error: String, rollback: Result<(), String>) -> String {
    match rollback {
        Ok(()) => error,
        Err(cleanup) => format!("{error}; mesh jobs rollback failed: {cleanup}"),
    }
}

pub(super) fn migrate(db: &mut impl Database) -> Result<(), String> {
    standalone(db)?;
    db.simple("BEGIN ISOLATION LEVEL READ COMMITTED")?;
    let result = db
        .simple("SELECT pg_catalog.pg_advisory_xact_lock(1782495102, 1)")
        .and_then(|()| db.simple(include_str!("schema.sql")))
        .and_then(|()| db.simple(include_str!("drain_schema.sql")))
        .and_then(|()| db.simple("COMMIT"));
    result.map_err(|error| rollback_error(error, db.simple("ROLLBACK")))
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub(super) enum State {
    Available,
    Scheduled,
    Retryable,
    Suspended,
    Executing,
    Completed,
    Cancelled,
    Discarded,
}

const INCOMPLETE: &[State] = &[
    State::Available,
    State::Scheduled,
    State::Retryable,
    State::Suspended,
    State::Executing,
];

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
enum UniqueField {
    Worker,
    Version,
    Queue,
    Args,
    #[serde(alias = "meta")]
    Metadata,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum StateGroup {
    All,
    Incomplete,
    Scheduled,
    Successful,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum States {
    Group(StateGroup),
    List(Vec<State>),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Period {
    Milliseconds(u64),
    Infinity(Infinity),
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Infinity {
    Infinity,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum UniqueTimestamp {
    #[default]
    InsertedAt,
    ScheduledAt,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct UniqueOptions {
    key: Option<String>,
    fields: Option<Vec<UniqueField>>,
    #[serde(default)]
    keys: Vec<String>,
    states: Option<States>,
    period_ms: Option<Period>,
    #[serde(default)]
    timestamp: UniqueTimestamp,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum UniqueInput {
    Enabled(bool),
    Key(String),
    Options(UniqueOptions),
}

impl UniqueInput {
    fn policy(self) -> Result<Value, String> {
        let mut options = match self {
            Self::Enabled(false) => return Ok(Value::Null),
            Self::Enabled(true) => UniqueOptions {
                states: Some(States::Group(StateGroup::All)),
                period_ms: Some(Period::Infinity(Infinity::Infinity)),
                ..UniqueOptions::default()
            },
            Self::Key(key) => UniqueOptions {
                key: Some(key),
                states: Some(States::Group(StateGroup::All)),
                period_ms: Some(Period::Infinity(Infinity::Infinity)),
                ..UniqueOptions::default()
            },
            Self::Options(options) => options,
        };
        if let Some(key) = &options.key {
            name(key, "unique key")?;
        }
        let mut fields = options.fields.unwrap_or_else(|| {
            let mut fields = vec![UniqueField::Worker, UniqueField::Queue];
            if options.key.is_none() {
                fields.push(UniqueField::Args);
            }
            fields
        });
        if fields.is_empty() && options.key.is_none() {
            return Err("mesh jobs unique fields cannot be empty without a unique key".into());
        }
        fields.sort_unstable();
        fields.dedup();

        let mut states = match options
            .states
            .unwrap_or(States::Group(StateGroup::Successful))
        {
            States::List(states) => states,
            States::Group(StateGroup::Incomplete) => INCOMPLETE.to_vec(),
            States::Group(StateGroup::Scheduled) => vec![State::Scheduled],
            States::Group(StateGroup::Successful) => [INCOMPLETE, &[State::Completed]].concat(),
            States::Group(StateGroup::All) => [
                INCOMPLETE,
                &[State::Completed, State::Cancelled, State::Discarded],
            ]
            .concat(),
        };
        if states.is_empty() {
            return Err("mesh jobs unique states cannot be empty".into());
        }
        states.sort_unstable();
        states.dedup();

        if options.keys.len() > 128 {
            return Err("mesh jobs accepts at most 128 uniqueness keys".into());
        }
        for key in &options.keys {
            name(key, "unique projection key")?;
        }
        options.keys.sort_unstable();
        options.keys.dedup();
        let period_ms = match options.period_ms.unwrap_or(Period::Milliseconds(60_000)) {
            Period::Milliseconds(0) => {
                return Err("mesh jobs unique period_ms must be positive".into())
            }
            Period::Milliseconds(value) => {
                duration(value, "unique period_ms")?;
                Some(value)
            }
            Period::Infinity(_) => None,
        };
        Ok(json!({
            "key": options.key, "fields": fields, "keys": options.keys, "states": states,
            "period_ms": period_ms, "timestamp": options.timestamp,
        }))
    }
}

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
enum ReplaceField {
    Args,
    MaxAttempts,
    #[serde(alias = "meta")]
    Metadata,
    Priority,
    Queue,
    #[serde(alias = "scheduled_at")]
    ScheduledAtMs,
    Tags,
    Worker,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Replace {
    All(Vec<ReplaceField>),
    ByState(BTreeMap<State, Vec<ReplaceField>>),
}

impl Replace {
    fn by_state(self) -> BTreeMap<State, Vec<ReplaceField>> {
        let mut rules = match self {
            Self::All(fields) => INCOMPLETE
                .iter()
                .copied()
                .chain([State::Completed, State::Cancelled, State::Discarded])
                .map(|state| (state, fields.clone()))
                .collect(),
            Self::ByState(rules) => rules,
        };
        for fields in rules.values_mut() {
            fields.sort_unstable();
            fields.dedup();
        }
        rules
    }
}

#[derive(Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct JobSpec {
    worker: String,
    version: u32,
    args: Value,
    queue: String,
    #[serde(alias = "meta")]
    metadata: Value,
    priority: u8,
    tags: Vec<String>,
    scheduled_at_ms: Option<u64>,
    delay_ms: Option<u64>,
    max_attempts: u32,
    timeout_ms: u64,
    backoff_base_ms: u64,
    backoff_max_ms: u64,
    suspended: bool,
    #[serde(skip_serializing)]
    request_key: Option<String>,
    #[serde(skip_serializing)]
    unique: Option<UniqueInput>,
    #[serde(skip_serializing)]
    replace: Option<Replace>,
}

impl Default for JobSpec {
    fn default() -> Self {
        Self {
            worker: String::new(),
            version: 1,
            args: json!({}),
            queue: "default".into(),
            metadata: json!({}),
            priority: 0,
            tags: Vec::new(),
            scheduled_at_ms: None,
            delay_ms: None,
            max_attempts: 20,
            timeout_ms: 60_000,
            backoff_base_ms: 1_000,
            backoff_max_ms: 3_600_000,
            suspended: false,
            request_key: None,
            unique: None,
            replace: None,
        }
    }
}

impl JobSpec {
    fn prepare(&mut self) -> Result<Admission, String> {
        name(&self.worker, "worker")?;
        name(&self.queue, "queue")?;
        if self.version == 0 || !(1..=100).contains(&self.max_attempts) || self.priority > 9 {
            return Err(
                "mesh jobs requires version > 0, max_attempts 1..=100 and priority 0..=9".into(),
            );
        }
        if !self.args.is_object() || !self.metadata.is_object() {
            return Err("mesh jobs args and metadata must be JSON objects".into());
        }
        if self.tags.len() > 128 {
            return Err("mesh jobs accepts at most 128 tags".into());
        }
        for tag in &self.tags {
            name(tag, "tag")?;
        }
        if self.timeout_ms == 0 || self.backoff_base_ms > self.backoff_max_ms {
            return Err(
                "mesh jobs requires timeout_ms > 0 and backoff_base_ms <= backoff_max_ms".into(),
            );
        }
        duration(self.timeout_ms, "timeout_ms")?;
        duration(self.backoff_max_ms, "backoff_max_ms")?;
        scheduling(self.scheduled_at_ms, self.delay_ms)?;

        let request_key = match self.request_key.take() {
            Some(key) => {
                name(&key, "request_key")?;
                key
            }
            None => String::new(),
        };
        let unique = self.unique.take().map(UniqueInput::policy).transpose()?;
        let replace = self
            .replace
            .take()
            .map(Replace::by_state)
            .unwrap_or_default();
        if unique.as_ref().is_none_or(Value::is_null) && !replace.is_empty() {
            return Err("mesh jobs replace requires a unique policy".into());
        }
        let mut spec = serde_json::to_value(self).map_err(|error| error.to_string())?;
        spec["unique"] = unique.unwrap_or(Value::Null);
        spec["replace"] = json!(replace);
        let spec = spec.to_string();
        let fingerprint = format!("{:x}", Sha256::digest(spec.as_bytes()));
        Ok(Admission {
            spec,
            request_key,
            fingerprint,
        })
    }
}

struct Admission {
    spec: String,
    request_key: String,
    fingerprint: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Enqueue {
    Batch(Vec<JobSpec>),
    Single(Box<JobSpec>),
}

pub(super) fn test_context(spec: &str, now_ms: u64) -> Result<String, String> {
    let mut spec = decode::<JobSpec>(spec)?;
    spec.prepare()?;
    let scheduled_at_ms = spec
        .scheduled_at_ms
        .unwrap_or_else(|| now_ms + spec.delay_ms.unwrap_or_default());
    scheduling(Some(scheduled_at_ms), None)?;
    let id = crate::crypto::uuid4();
    Ok(json!({
        "id": id,
        "effect_key": format!("{id}/1"),
        "worker": spec.worker,
        "version": spec.version,
        "args": spec.args,
        "attempt": 1,
        "max_attempts": spec.max_attempts,
        "queue": spec.queue,
        "metadata": spec.metadata,
        "tags": spec.tags,
        "scheduled_at_ms": scheduled_at_ms
    })
    .to_string())
}

pub(super) fn validate_enqueue_spec(spec: &str) -> Result<(), String> {
    decode::<JobSpec>(spec)?.prepare().map(drop)
}

pub(super) fn enqueue(
    db: &mut impl Database,
    namespace: &str,
    specs: &str,
) -> Result<String, String> {
    if !db.in_transaction() {
        return Err("mesh jobs enqueue requires an outer transaction".into());
    }
    name(namespace, "namespace")?;
    let (batch, specs) = match decode(specs)? {
        Enqueue::Single(spec) => (false, vec![*spec]),
        Enqueue::Batch(specs) => (true, specs),
    };
    if specs.is_empty() || specs.len() > MAX_BATCH {
        return Err("mesh jobs enqueue requires 1 to 128 specs".into());
    }
    let admissions = specs
        .into_iter()
        .map(|mut spec| spec.prepare())
        .collect::<Result<Vec<_>, _>>()?;

    db.simple("SAVEPOINT mesh_jobs_enqueue")?;
    let result = (|| {
        // Broad uniqueness policies can match any insert in this namespace. A new
        // row version also forces a retry when an outer repeatable-read snapshot is stale.
        db.query(
            "INSERT INTO mesh_jobs.admissions AS a (namespace) VALUES ($1)
             ON CONFLICT (namespace) DO UPDATE SET fence = NOT a.fence",
            &[namespace],
        )?;
        let mut receipts = Vec::with_capacity(admissions.len());
        for admission in admissions {
            receipts.push(response(
                db,
                "SELECT mesh_jobs.enqueue($1, $2::jsonb, $3, $4)::text AS receipt",
                &[
                    namespace,
                    &admission.spec,
                    &admission.request_key,
                    &admission.fingerprint,
                ],
            )?);
        }
        db.simple("RELEASE SAVEPOINT mesh_jobs_enqueue")?;
        Ok(if batch {
            format!("[{}]", receipts.join(","))
        } else {
            receipts.remove(0)
        })
    })();
    result.map_err(|error| {
        rollback_error(
            error,
            db.simple("ROLLBACK TO SAVEPOINT mesh_jobs_enqueue")
                .and_then(|()| db.simple("RELEASE SAVEPOINT mesh_jobs_enqueue")),
        )
    })
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Worker {
    worker: String,
    version: u32,
}

#[cfg(test)]
pub(super) fn claim(
    db: &mut impl Database,
    namespace: &str,
    queue: &str,
    workers_json: &str,
    lease_ms: u64,
) -> Result<Option<Claim>, String> {
    claim_with_timing(db, namespace, queue, workers_json, lease_ms, false)
}

pub(super) fn claim_with_timing(
    db: &mut impl Database,
    namespace: &str,
    queue: &str,
    workers_json: &str,
    lease_ms: u64,
    observe: bool,
) -> Result<Option<Claim>, String> {
    standalone(db)?;
    name(namespace, "namespace")?;
    name(queue, "queue")?;
    lease_duration(lease_ms)?;
    let workers: Vec<Worker> = decode(workers_json)?;
    for worker in &workers {
        name(&worker.worker, "worker")?;
        if worker.version == 0 {
            return Err("mesh jobs worker version must be positive".into());
        }
    }
    if workers.is_empty() {
        return Ok(None);
    }
    let sql = if observe {
        "SELECT id, token, context, timeout_ms,
                floor(extract(epoch FROM pg_catalog.clock_timestamp()) * 1000)::bigint::text
         FROM mesh_jobs.claim($1, $2, $3::jsonb, $4::bigint)"
    } else {
        "SELECT id, token, context, timeout_ms FROM mesh_jobs.claim($1, $2, $3::jsonb, $4::bigint)"
    };
    claim_one_transaction(
        db,
        sql,
        &[namespace, queue, workers_json, &lease_ms.to_string()],
        observe,
    )
}

pub(super) fn claim_one_transaction(
    db: &mut impl Database,
    sql: &str,
    params: &[&str],
    observe: bool,
) -> Result<Option<Claim>, String> {
    standalone(db)?;
    // The capacity query must see the preceding winner after the queue lock,
    // including on databases configured to default to repeatable read.
    db.simple("BEGIN ISOLATION LEVEL READ COMMITTED")?;
    let result = (|| {
        let rows = db.query(sql, params)?;
        let claim = match rows.as_slice() {
            [] => None,
            [row] => {
                let mut fields = row.iter().map(|(_, value)| value);
                let (Some(id), Some(token), Some(context), Some(timeout_ms)) =
                    (fields.next(), fields.next(), fields.next(), fields.next())
                else {
                    return Err("mesh jobs invalid claim result from PostgreSQL".into());
                };
                let observed_at_ms = if observe {
                    fields.next().and_then(|value| value.parse().ok())
                } else {
                    None
                };
                if fields.next().is_some() {
                    return Err("mesh jobs invalid claim result from PostgreSQL".into());
                }
                Some(Claim {
                    id: id.clone(),
                    token: token.clone(),
                    context: context.clone(),
                    timeout_ms: timeout_ms
                        .parse::<u64>()
                        .map_err(|error| error.to_string())?,
                    observed_at_ms,
                })
            }
            _ => return Err("mesh jobs PostgreSQL returned multiple claims".into()),
        };
        db.simple("COMMIT")?;
        Ok(claim)
    })();
    result.map_err(|error| rollback_error(error, db.simple("ROLLBACK")))
}

#[cfg(test)]
pub(super) fn claim_batch(
    db: &mut impl Database,
    namespace: &str,
    queue: &str,
    workers_json: &str,
    lease_ms: u64,
    slots: usize,
    cutoff_ms: Option<u64>,
) -> Result<ClaimBatch, String> {
    claim_batch_with_timing(
        db,
        namespace,
        queue,
        workers_json,
        lease_ms,
        slots,
        cutoff_ms,
        false,
    )
}

pub(super) fn claim_batch_with_timing(
    db: &mut impl Database,
    namespace: &str,
    queue: &str,
    workers_json: &str,
    lease_ms: u64,
    slots: usize,
    cutoff_ms: Option<u64>,
    observe: bool,
) -> Result<ClaimBatch, String> {
    standalone(db)?;
    name(namespace, "namespace")?;
    name(queue, "queue")?;
    lease_duration(lease_ms)?;
    scheduling(cutoff_ms, None)?;
    let workers: Vec<Worker> = decode(workers_json)?;
    for worker in &workers {
        name(&worker.worker, "worker")?;
        if worker.version == 0 {
            return Err("mesh jobs worker version must be positive".into());
        }
    }
    if workers.is_empty() || slots == 0 {
        return Ok(ClaimBatch::default());
    }

    let slots = slots.min(MAX_CLAIM_BATCH);
    let cutoff = cutoff_ms.map_or_else(String::new, |ms| ms.to_string());
    // Force a fresh capacity snapshot even when the session defaults to repeatable read.
    db.simple("BEGIN ISOLATION LEVEL READ COMMITTED")?;
    let result = (|| {
        let rows = db.query(
            "SELECT status, id, token, context, timeout_ms, context_bytes,
                    CASE WHEN $7::boolean AND status = 'claimed'
                    THEN floor(extract(epoch FROM pg_catalog.clock_timestamp()) * 1000)::bigint::text END
             FROM mesh_jobs.claim_batch_v1($1, $2, $3::jsonb, $4::bigint, $5::integer,
                 nullif($6, '')::bigint) WITH ORDINALITY
             ORDER BY ordinality",
            &[
                namespace,
                queue,
                workers_json,
                &lease_ms.to_string(),
                &slots.to_string(),
                &cutoff,
                if observe { "true" } else { "false" },
            ],
        )?;
        let mut batch = ClaimBatch::default();
        let invalid = || "mesh jobs invalid batch claim result from PostgreSQL".to_string();
        for row in rows {
            let fields = row.into_iter().map(|(_, value)| value).collect::<Vec<_>>();
            let [status, id, token, context, timeout_ms, context_bytes, observed_at_ms]: [String;
                7] = fields.try_into().map_err(|_| invalid())?;
            let context_bytes = context_bytes.parse::<usize>().map_err(|_| invalid())?;
            match status.as_str() {
                "claimed"
                    if context_bytes == context.len()
                        && context_bytes <= crate::db::pg::MAX_DB_VALUE_BYTES
                        && batch.claims.len() < slots =>
                {
                    batch.claims.push(Claim {
                        id,
                        token,
                        context,
                        timeout_ms: timeout_ms.parse().map_err(|_| invalid())?,
                        observed_at_ms: if observe {
                            observed_at_ms.parse().ok()
                        } else {
                            None
                        },
                    });
                }
                "oversized"
                    if context_bytes > crate::db::pg::MAX_DB_VALUE_BYTES
                        && token.is_empty()
                        && context.is_empty()
                        && timeout_ms.is_empty()
                        && batch.oversized.len() < MAX_CLAIM_BATCH =>
                {
                    batch.oversized.push(OversizedClaim { id, context_bytes });
                }
                _ => return Err(invalid()),
            }
        }
        if !db.in_transaction() {
            return Err("mesh jobs batch claim did not leave a committable transaction".into());
        }
        // The driver waits for COMMIT's ReadyForQuery before any claim can escape.
        db.simple("COMMIT")?;
        Ok(batch)
    })();
    result.map_err(|error| rollback_error(error, db.simple("ROLLBACK")))
}

#[derive(Deserialize)]
pub(super) struct DrainWork {
    pub(super) pending: bool,
    pub(super) oversized: Option<OversizedClaim>,
}

pub(super) fn drain_work(
    db: &mut impl Database,
    namespace: &str,
    queues: &str,
    workers: &str,
    batch: bool,
) -> Result<DrainWork, String> {
    let (blocked_filter, diagnostic) = if batch {
        (
            "AND NOT EXISTS (SELECT FROM mesh_jobs.batch_context_blocks b
                WHERE b.namespace = j.namespace AND b.id = j.id
                    AND b.row_version = j.xmin AND b.row_location = j.ctid
                    AND b.encoding = current_setting('client_encoding'))",
            "(SELECT json_build_object('id', b.id::text, 'context_bytes', b.context_bytes)
              FROM mesh_jobs.batch_context_blocks b JOIN mesh_jobs.jobs j
                ON j.namespace = b.namespace AND j.id = b.id
                    AND j.xmin = b.row_version AND j.ctid = b.row_location
              JOIN mesh_jobs.queues q ON q.namespace = j.namespace AND q.name = j.queue
              CROSS JOIN clock
              WHERE b.namespace = $1 AND $2::jsonb ? j.queue
                  AND b.encoding = current_setting('client_encoding')
                  AND q.state = 'active' AND q.concurrency > 0
                  AND j.state IN ('available', 'scheduled', 'retryable')
                  AND j.scheduled_at_ms <= now_ms AND j.attempt < j.max_attempts
                  AND EXISTS (SELECT FROM jsonb_to_recordset($3::jsonb) AS w(worker text, version bigint)
                              WHERE w.worker = j.worker AND w.version = j.version)
              LIMIT 1)",
        )
    } else {
        ("", "NULL")
    };
    // A skipped lock, or a queue at capacity, can also produce an empty claim.
    response(
        db,
        &format!("WITH clock AS MATERIALIZED (SELECT mesh_jobs.now_ms() AS now_ms)
         SELECT json_build_object('pending', EXISTS (
             SELECT FROM mesh_jobs.jobs j JOIN mesh_jobs.queues q
             ON q.namespace = j.namespace AND q.name = j.queue CROSS JOIN clock
             WHERE j.namespace = $1 AND $2::jsonb ? j.queue
                 AND ((j.state = 'executing' AND j.lease_expires_at_ms <= now_ms)
                     OR (q.state = 'active' AND q.concurrency > 0
                         AND j.state IN ('available', 'scheduled', 'retryable')
                         AND j.scheduled_at_ms <= now_ms AND j.attempt < j.max_attempts
                         {blocked_filter}
                         AND EXISTS (SELECT FROM jsonb_to_recordset($3::jsonb) AS w(worker text, version bigint)
                                     WHERE w.worker = j.worker AND w.version = j.version)))
         ), 'oversized', {diagnostic})::text"),
        &[namespace, queues, workers],
    )
    .and_then(|outcome| decode(&outcome))
}

pub(super) fn renew(
    db: &mut impl Database,
    namespace: &str,
    id: &str,
    token: &str,
    lease_ms: u64,
) -> Result<LeaseStatus, String> {
    standalone(db)?;
    name(namespace, "namespace")?;
    lease_duration(lease_ms)?;
    match response(
        db,
        "SELECT mesh_jobs.try_renew($1, $2::uuid, $3::uuid, $4::bigint) AS status",
        &[namespace, id, token, &lease_ms.to_string()],
    )?
    .as_str()
    {
        "active" => Ok(LeaseStatus::Active),
        "cancelled" => Ok(LeaseStatus::Cancelled),
        "lost" => Ok(LeaseStatus::Lost),
        _ => Err("mesh jobs invalid lease status from PostgreSQL".into()),
    }
}

pub(super) fn finish(
    db: &mut impl Database,
    namespace: &str,
    id: &str,
    token: &str,
    mut outcome: Outcome,
) -> Result<Settlement, JobsAckError> {
    standalone(db).map_err(JobsAckError::Rejected)?;
    name(namespace, "namespace").map_err(JobsAckError::Rejected)?;
    if matches!(outcome, Outcome::ReleaseUnstarted) {
        return acknowledge(
            db,
            "WITH held AS MATERIALIZED (
                 SELECT lease_expires_at_ms, cancel_requested_at_ms FROM mesh_jobs.jobs
                 WHERE namespace = $1 AND id = $2::uuid AND lease_token = $3::uuid
                 FOR NO KEY UPDATE NOWAIT
             ), released AS (
                 UPDATE mesh_jobs.jobs j SET state = 'available', attempt = j.attempt - 1,
                     lease_token = NULL, lease_expires_at_ms = NULL
                 FROM held WHERE j.namespace = $1 AND j.id = $2::uuid AND j.lease_token = $3::uuid
                     AND held.cancel_requested_at_ms IS NULL
                     AND held.lease_expires_at_ms > mesh_jobs.now_ms()
                 RETURNING j.state
             ) SELECT coalesce(
                 CASE WHEN EXISTS (SELECT 1 FROM held WHERE cancel_requested_at_ms IS NOT NULL)
                     THEN mesh_jobs.settle_state($1, $2::uuid, $3::uuid, '{\"action\":\"cancel\"}'::jsonb, false)
                     ELSE (SELECT CASE state WHEN 'available' THEN 'released_unstarted' ELSE state END
                           FROM released)
                 END, 'stale')",
            &[namespace, id, token],
        );
    }
    match &mut outcome {
        Outcome::RetryAfter { delay_ms, .. } | Outcome::Snooze { delay_ms } => {
            duration(*delay_ms, "outcome delay_ms").map_err(JobsAckError::Rejected)?;
        }
        _ => {}
    }
    match &mut outcome {
        Outcome::Fail { reason }
        | Outcome::RetryAfter { reason, .. }
        | Outcome::Cancel { reason }
        | Outcome::Discard { reason } => {
            *reason = reason
                .chars()
                .take(2048)
                .map(|c| if c == '\0' { '\u{fffd}' } else { c })
                .collect();
        }
        _ => {}
    }
    let outcome = serde_json::to_string(&outcome)
        .map_err(|error| JobsAckError::Rejected(error.to_string()))?;
    acknowledge(
        db,
        "SELECT coalesce(mesh_jobs.settle_state($1, $2::uuid, $3::uuid, $4::jsonb, false), 'stale')",
        &[namespace, id, token, &outcome],
    )
}

fn acknowledge(
    db: &mut impl Database,
    sql: &str,
    params: &[&str],
) -> Result<Settlement, JobsAckError> {
    let rows = db.acknowledge(sql, params)?;
    if let [row] = rows.as_slice() {
        if let [(_, state)] = row.as_slice() {
            return Settlement::from_sql(state);
        }
    }
    Err(JobsAckError::Unconfirmed(
        "mesh jobs expected one settlement cell from PostgreSQL".into(),
    ))
}

pub(super) fn recover(
    db: &mut impl Database,
    namespace: &str,
    limit: u32,
    queues: Option<&str>,
) -> Result<u64, String> {
    standalone(db)?;
    name(namespace, "namespace")?;
    if limit > MAX_PAGE {
        return Err("mesh jobs recovery limit exceeds 1000".into());
    }
    response(
        db,
        "SELECT mesh_jobs.recover($1, $2::integer, nullif($3, '')::jsonb)::text AS recovered",
        &[namespace, &limit.to_string(), queues.unwrap_or_default()],
    )?
    .parse::<u64>()
    .map_err(|error| error.to_string())
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    queue: Option<String>,
    worker: Option<String>,
    state: Option<State>,
    limit: Option<u32>,
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum Control {
    Configure {
        queue: String,
        #[serde(alias = "limit")]
        concurrency: u32,
    },
    Get {
        id: Option<String>,
        queue: Option<String>,
    },
    List {
        queue: Option<String>,
        state: Option<State>,
        worker: Option<String>,
        #[serde(alias = "after")]
        after_id: Option<String>,
        limit: Option<u32>,
        #[serde(default)]
        queues: bool,
    },
    ListQueues {
        after: Option<String>,
        limit: Option<u32>,
    },
    QueueStats {
        queue: String,
    },
    Pause {
        queue: String,
    },
    Resume {
        queue: Option<String>,
        id: Option<String>,
    },
    Scale {
        queue: String,
        #[serde(alias = "limit")]
        concurrency: u32,
    },
    Stop {
        queue: String,
    },
    Start {
        queue: String,
        #[serde(alias = "limit")]
        concurrency: Option<u32>,
    },
    Cancel {
        id: String,
    },
    Retry {
        id: String,
    },
    CancelAll {
        #[serde(default)]
        filter: Selection,
    },
    RetryAll {
        #[serde(default)]
        filter: Selection,
    },
    Discard {
        id: String,
    },
    Reschedule {
        id: String,
        scheduled_at_ms: Option<u64>,
        delay_ms: Option<u64>,
    },
    Suspend {
        id: String,
    },
    Delete {
        id: String,
    },
    Prune {
        queue: Option<String>,
        #[serde(alias = "older_than_ms")]
        before_ms: Option<u64>,
        max_age_ms: Option<u64>,
        limit: Option<u32>,
    },
}

fn queue_control(
    db: &mut impl Database,
    namespace: &str,
    queue: &str,
    action: &str,
    concurrency: Option<u32>,
) -> Result<String, String> {
    name(queue, "queue")?;
    if concurrency.is_some_and(|limit| limit > MAX_CONCURRENCY) {
        return Err("mesh jobs concurrency exceeds 1024".into());
    }
    response(
        db,
        "SELECT mesh_jobs.queue_control($1, $2, $3, nullif($4, '')::integer)::text",
        &[namespace, queue, action, &optional_number(concurrency)],
    )
}

fn job_control(
    db: &mut impl Database,
    namespace: &str,
    id: &str,
    action: &str,
    scheduled_at_ms: Option<u64>,
    delay_ms: Option<u64>,
) -> Result<String, String> {
    scheduling(scheduled_at_ms, delay_ms)?;
    response(
        db,
        "SELECT mesh_jobs.job_control($1, $2::uuid, $3, nullif($4, '')::bigint, nullif($5, '')::bigint)::text",
        &[namespace, id, action, &optional_number(scheduled_at_ms), &optional_number(delay_ms)],
    )
}

fn optional_number<T: ToString>(number: Option<T>) -> String {
    number.map(|value| value.to_string()).unwrap_or_default()
}

fn bulk_control(
    db: &mut impl Database,
    namespace: &str,
    operation: &str,
    filter: Selection,
) -> Result<String, String> {
    let state = serde_json::to_value(filter.state).map_err(|error| error.to_string())?;
    response(
        db,
        "WITH eligible AS MATERIALIZED (
             SELECT id FROM mesh_jobs.jobs
             WHERE namespace = $1 AND ($3 = '' OR queue = $3)
                 AND ($4 = '' OR worker = $4) AND ($5 = '' OR state = $5)
                 AND CASE $2 WHEN 'cancel' THEN finished_at_ms IS NULL
                     AND cancel_requested_at_ms IS NULL ELSE finished_at_ms IS NOT NULL END
             ORDER BY id LIMIT $6::integer FOR NO KEY UPDATE SKIP LOCKED
         ), changed AS MATERIALIZED (
             SELECT id, mesh_jobs.job_control($1, id, $2, NULL, NULL) AS job
             FROM eligible
         ) SELECT jsonb_build_object('changed', count(job),
             'ids', coalesce(jsonb_agg(id ORDER BY id), '[]'::jsonb))::text FROM changed",
        &[
            namespace,
            operation,
            filter.queue.as_deref().unwrap_or_default(),
            filter.worker.as_deref().unwrap_or_default(),
            state.as_str().unwrap_or_default(),
            &page(filter.limit)?,
        ],
    )
}

fn page(limit: Option<u32>) -> Result<String, String> {
    let limit = limit.unwrap_or(100);
    if limit == 0 || limit > MAX_PAGE {
        return Err("mesh jobs page limit must be 1..=1000".into());
    }
    Ok(limit.to_string())
}

fn list_queues(
    db: &mut impl Database,
    namespace: &str,
    after: Option<&str>,
    limit: Option<u32>,
) -> Result<String, String> {
    response(
        db,
        "SELECT coalesce(jsonb_agg(mesh_jobs.queue_json(q) ORDER BY q.name), '[]')::text
         FROM (SELECT * FROM mesh_jobs.queues
               WHERE namespace = $1 AND name > $2 ORDER BY name LIMIT $3::integer) q",
        &[namespace, after.unwrap_or_default(), &page(limit)?],
    )
}

pub(super) fn prune(
    db: &mut impl Database,
    namespace: &str,
    queue: Option<&str>,
    before_ms: Option<u64>,
    max_age_ms: Option<u64>,
    limit: Option<u32>,
) -> Result<u64, String> {
    scheduling(before_ms, max_age_ms)?;
    response(
        db,
        "WITH cutoff AS MATERIALIZED (
             SELECT coalesce(nullif($2, '')::bigint,
                 mesh_jobs.now_ms() - coalesce(nullif($3, '')::bigint, 86400000)) AS at_ms
         ), stale AS (
             SELECT id FROM mesh_jobs.jobs CROSS JOIN cutoff
             WHERE namespace = $1 AND finished_at_ms <= cutoff.at_ms AND ($4 = '' OR queue = $4)
             ORDER BY finished_at_ms, id LIMIT $5::integer FOR UPDATE OF jobs SKIP LOCKED
         ), removed AS (
             DELETE FROM mesh_jobs.jobs j USING stale
             WHERE j.namespace = $1 AND j.id = stale.id RETURNING j.id
         ) SELECT count(*)::text FROM removed",
        &[
            namespace,
            &optional_number(before_ms),
            &optional_number(max_age_ms),
            queue.unwrap_or_default(),
            &page(limit)?,
        ],
    )?
    .parse()
    .map_err(|_| "invalid Jobs prune count".into())
}

pub(super) fn control(
    db: &mut impl Database,
    namespace: &str,
    request: &str,
) -> Result<String, String> {
    name(namespace, "namespace")?;
    let mut request: Value = decode(request)?;
    if let Some(object) = request.as_object_mut() {
        if let Some(action) = object.remove("op") {
            if object.insert("action".into(), action).is_some() {
                return Err("mesh jobs control accepts action or op once".into());
            }
        }
    }
    let request: Control = serde_json::from_value(request).map_err(|error| error.to_string())?;
    match request {
        Control::Configure { queue, concurrency } => {
            queue_control(db, namespace, &queue, "configure", Some(concurrency))
        }
        Control::Get {
            id: Some(id),
            queue: None,
        } => response(
            db,
            "SELECT coalesce((SELECT mesh_jobs.job_json(j) FROM mesh_jobs.jobs j
             WHERE namespace = $1 AND id = $2::uuid), 'null')::text",
            &[namespace, &id],
        ),
        Control::Get {
            id: None,
            queue: Some(queue),
        } => response(
            db,
            "SELECT coalesce((SELECT mesh_jobs.queue_json(q) FROM mesh_jobs.queues q
             WHERE namespace = $1 AND name = $2), 'null')::text",
            &[namespace, &queue],
        ),
        Control::List {
            queue,
            state,
            worker,
            after_id,
            limit,
            queues: false,
        } => {
            let state = serde_json::to_value(state).map_err(|error| error.to_string())?;
            response(
                db,
                "SELECT coalesce(jsonb_agg(
                     mesh_jobs.job_json(j) - ARRAY['args', 'metadata', 'tags', 'errors'] ORDER BY j.id
                 ), '[]')::text
                 FROM (SELECT * FROM mesh_jobs.jobs
                       WHERE namespace = $1 AND ($2 = '' OR queue = $2)
                           AND ($3 = '' OR state = $3) AND ($4 = '' OR worker = $4)
                           AND ($5 = '' OR id > nullif($5, '')::uuid)
                       ORDER BY id LIMIT $6::integer) j",
                &[
                    namespace,
                    queue.as_deref().unwrap_or_default(),
                    state.as_str().unwrap_or_default(),
                    worker.as_deref().unwrap_or_default(),
                    after_id.as_deref().unwrap_or_default(),
                    &page(limit)?,
                ],
            )
        }
        Control::List {
            queue: None,
            state: None,
            worker: None,
            after_id,
            limit,
            queues: true,
        } => list_queues(db, namespace, after_id.as_deref(), limit),
        Control::ListQueues { after, limit } => list_queues(db, namespace, after.as_deref(), limit),
        Control::QueueStats { queue } => response(
            db,
            "WITH clock AS MATERIALIZED (SELECT mesh_jobs.now_ms() AS now_ms), counts AS (
                 SELECT state, count(*) AS count, min(scheduled_at_ms) FILTER (
                     WHERE state IN ('available', 'scheduled', 'retryable') AND scheduled_at_ms <= now_ms
                 ) AS oldest_due_ms
                 FROM mesh_jobs.jobs CROSS JOIN clock WHERE namespace = $1 AND queue = $2 GROUP BY state
             ) SELECT jsonb_build_object(
                 'counts', coalesce(jsonb_object_agg(state, count), '{}'::jsonb),
                 'oldest_due_ms', min(oldest_due_ms),
                 'oldest_due_age_ms', (SELECT now_ms FROM clock) - min(oldest_due_ms)
             )::text FROM counts",
            &[namespace, &queue],
        ),
        Control::Pause { queue } => queue_control(db, namespace, &queue, "pause", None),
        Control::Resume {
            queue: Some(queue),
            id: None,
        } => queue_control(db, namespace, &queue, "resume", None),
        Control::Resume {
            queue: None,
            id: Some(id),
        } => job_control(db, namespace, &id, "resume_job", None, None),
        Control::Scale { queue, concurrency } => {
            queue_control(db, namespace, &queue, "scale", Some(concurrency))
        }
        Control::Stop { queue } => queue_control(db, namespace, &queue, "stop", None),
        Control::Start { queue, concurrency } => {
            queue_control(db, namespace, &queue, "start", concurrency)
        }
        Control::Cancel { id } => job_control(db, namespace, &id, "cancel", None, None),
        Control::Retry { id } => job_control(db, namespace, &id, "retry", None, None),
        Control::CancelAll { filter } => bulk_control(db, namespace, "cancel", filter),
        Control::RetryAll { filter } => bulk_control(db, namespace, "retry", filter),
        Control::Discard { id } => job_control(db, namespace, &id, "discard", None, None),
        Control::Reschedule {
            id,
            scheduled_at_ms,
            delay_ms,
        } => job_control(db, namespace, &id, "reschedule", scheduled_at_ms, delay_ms),
        Control::Suspend { id } => job_control(db, namespace, &id, "suspend", None, None),
        Control::Delete { id } => job_control(db, namespace, &id, "delete", None, None),
        Control::Prune {
            queue,
            before_ms,
            max_age_ms,
            limit,
        } => Ok(json!({
            "deleted": prune(db, namespace, queue.as_deref(), before_ms, max_age_ms, limit)?
        })
        .to_string()),
        _ => Err("mesh jobs control target is missing or ambiguous".into()),
    }
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod store_tests;
