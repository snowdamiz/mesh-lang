//! Durable periodic scheduling in a job namespace.
//!
//! A `schedule` request takes `name`, `expression`, and a `job` enqueue template.
//! Requests have a 32 KiB limit. `timezone` defaults to `Etc/UTC`. The default
//! `misfire` policy is `skip`, which considers the current database minute.
//! `coalesce` emits the latest missed match and
//! `catch_up` emits missed matches in order. Both include the current minute and
//! drop matches outside `lookback_minutes` (default 60, maximum 1440).
//! Day-of-month and weekday both match, as in Oban. A repeated local minute
//! matches twice at distinct UTC instants. A skipped local minute has no match.
//!
//! `unschedule` takes `name`. `list_schedules` reads schedule definitions and
//! accepts `after` (exclusive schedule name) and `limit` (default 100, max 500).
//! Reapplying an identical definition leaves the cursor intact. A changed
//! definition drops outstanding backlog and keeps the schedule identity.
//! Removing and recreating a schedule creates a new identity.
//! The current minute is eligible when a schedule is first added.
//!
//! Producers call `tick` with a unique identity per live process. A namespace
//! starts a new activation epoch on its first tick or after at least 60 seconds
//! without a committed heartbeat from any producer. Poll at least every few
//! seconds. A replica joining an active namespace shares its epoch. A short
//! restart within the heartbeat interval also shares its epoch. Each `@reboot`
//! schedule can enqueue once in the current epoch, including when first added
//! mid-epoch. Epochs that elapse before a schedule can run are coalesced.

use super::store::{enqueue, Database};
use serde::Deserialize;
use serde_json::{json, Value};

const MINUTE_MS: i64 = 60_000;
const ACTIVATION_IDLE_MS: i64 = 60_000;
pub(super) const MAX_TICK_JOBS: u32 = 128;
const MAX_REQUEST_BYTES: usize = 32 * 1024;
const DEFAULT_LOOKBACK: u32 = 60;
const MAX_LOOKBACK: u32 = 1440;

const MONTHS: &[&str] = &[
    "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
];
const WEEKDAYS: &[&str] = &["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"];

#[derive(Debug, PartialEq, Eq)]
enum Expression {
    Reboot,
    Calendar([i64; 5]),
}

impl Expression {
    fn parse(input: &str) -> Result<Self, String> {
        let input = match input {
            "@reboot" => return Ok(Self::Reboot),
            "@yearly" | "@annually" => "0 0 1 1 *",
            "@monthly" => "0 0 1 * *",
            "@weekly" => "0 0 * * 0",
            "@daily" | "@midnight" => "0 0 * * *",
            "@hourly" => "0 * * * *",
            _ => input,
        };
        let fields: Vec<_> = input.split_ascii_whitespace().collect();
        let [minute, hour, day, month, weekday] = fields.as_slice() else {
            return Err("cron expression needs five fields or a supported alias".into());
        };
        let days = parse_field(day, 1, 31, &[])?;
        let months = parse_field(month, 1, 12, MONTHS)?;
        let month_lengths = [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
        let has_possible_day = month_lengths.iter().enumerate().any(|(index, length)| {
            months & (1 << (index + 1)) != 0 && days & ((1i64 << (length + 1)) - 1) != 0
        });
        if !has_possible_day {
            return Err("cron day cannot occur in any selected month".into());
        }
        let weekdays = parse_field(weekday, 0, 7, WEEKDAYS)?;
        Ok(Self::Calendar([
            parse_field(minute, 0, 59, &[])?,
            parse_field(hour, 0, 23, &[])?,
            days,
            months,
            (weekdays & 0b111_1111) | (weekdays >> 7),
        ]))
    }
}

fn parse_field(input: &str, min: u8, max: u8, names: &[&str]) -> Result<i64, String> {
    let invalid = || format!("invalid cron field: {input}");
    let value = |word: &str| {
        let named = names
            .iter()
            .position(|name| word.eq_ignore_ascii_case(name))
            .map(|position| position as u8 + min);
        named
            .or_else(|| {
                if word.bytes().all(|byte| byte.is_ascii_digit()) {
                    word.parse().ok()
                } else {
                    None
                }
            })
            .filter(|value| (min..=max).contains(value))
            .ok_or_else(invalid)
    };

    // Sunday can also be written as 7, but wildcard and open-ended steps
    // traverse the ordinary weekday range only once.
    let natural_max = if max == 7 { 6 } else { max };
    let mut mask = 0i64;
    for part in input.split(',') {
        let (range, step) = match part.split_once('/') {
            Some((range, step)) => {
                if !step.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err(invalid());
                }
                let step = step
                    .parse::<usize>()
                    .ok()
                    .filter(|step| (1..=99).contains(step))
                    .ok_or_else(invalid)?;
                (range, Some(step))
            }
            None => (part, None),
        };
        let (start, end) = match range {
            "*" => (min, natural_max),
            _ => match range.split_once('-') {
                Some((start, end)) => (value(start)?, value(end)?),
                None => {
                    let start = value(range)?;
                    (start, step.map_or(start, |_| natural_max.max(start)))
                }
            },
        };
        if start > end {
            return Err(invalid());
        }
        for value in (start..=end).step_by(step.unwrap_or(1)) {
            mask |= 1 << value;
        }
    }
    Ok(mask)
}

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Misfire {
    #[default]
    Skip,
    Coalesce,
    CatchUp,
}

impl Misfire {
    fn as_str(self) -> &'static str {
        match self {
            Self::Skip => "skip",
            Self::Coalesce => "coalesce",
            Self::CatchUp => "catch_up",
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    Schedule {
        name: String,
        #[serde(alias = "cron")]
        expression: String,
        timezone: Option<String>,
        #[serde(default)]
        misfire: Misfire,
        lookback_minutes: Option<u32>,
        job: Value,
    },
    Unschedule {
        name: String,
    },
    ListSchedules {
        after: Option<String>,
        limit: Option<u32>,
    },
}

#[derive(Deserialize)]
struct Schedule {
    name: String,
    expression: String,
    timezone: String,
    misfire: Misfire,
    lookback_minutes: i64,
    job: Value,
    cursor_minute: i64,
}

#[derive(Deserialize)]
struct Activation {
    epoch: i64,
    activated_at_ms: i64,
}

type Rows = Vec<Vec<(String, String)>>;

fn column(row: &[(String, String)]) -> Result<&str, String> {
    row.first()
        .map(|(_, value)| value.as_str())
        .ok_or_else(|| "invalid cron database reply".to_string())
}

fn only_value(rows: &Rows) -> Result<&str, String> {
    match rows.as_slice() {
        [row] => column(row),
        _ => Err("invalid cron database reply".into()),
    }
}

fn check_text(text: &str, label: &str, maximum: usize) -> Result<(), String> {
    if text.trim().is_empty() || text.len() > maximum || text.contains('\0') {
        return Err(format!("invalid cron {label}"));
    }
    Ok(())
}

/// Install cron storage after the queue store migration.
pub(super) fn migrate(db: &mut impl Database) -> Result<(), String> {
    db.simple(include_str!("cron_schema.sql"))
}

pub(super) fn control(
    db: &mut impl Database,
    namespace: &str,
    request: &str,
) -> Result<String, String> {
    check_text(namespace, "namespace", 255)?;
    if db.in_transaction() {
        return Err("cron control needs an idle database connection".into());
    }
    if request.len() > MAX_REQUEST_BYTES {
        return Err("cron control JSON exceeds 32 KiB".into());
    }
    let request: Request = serde_json::from_str(request).map_err(|err| err.to_string())?;
    match request {
        Request::Schedule {
            name,
            expression,
            timezone,
            misfire,
            lookback_minutes,
            job,
        } => {
            check_text(&name, "name", 128)?;
            check_text(&expression, "expression", 1024)?;
            let expression = expression.trim();
            Expression::parse(expression)?;
            let timezone = timezone.as_deref().unwrap_or("Etc/UTC");
            check_text(timezone, "timezone", 128)?;
            let lookback = lookback_minutes.unwrap_or(DEFAULT_LOOKBACK);
            if !(1..=MAX_LOOKBACK).contains(&lookback) {
                return Err(format!(
                    "cron lookback_minutes must be in 1..={MAX_LOOKBACK}"
                ));
            }
            let job = validate_template(job)?;
            let rows = db.query(
                "INSERT INTO mesh_jobs.cron_schedules AS s
                    (namespace, name, expression, timezone, misfire, lookback_minutes, job, cursor_minute)
                 SELECT $1, $2, $3, name, $5, $6::integer, $7::jsonb, mesh_jobs.now_ms() / 60000 - 1
                 FROM pg_catalog.pg_timezone_names WHERE name = $4
                 ON CONFLICT (namespace, name) DO UPDATE SET
                    expression = excluded.expression, timezone = excluded.timezone,
                    misfire = excluded.misfire, lookback_minutes = excluded.lookback_minutes,
                    job = excluded.job,
                    cursor_minute = CASE WHEN (s.expression, s.timezone, s.misfire, s.lookback_minutes, s.job)
                        IS DISTINCT FROM (excluded.expression, excluded.timezone, excluded.misfire,
                                          excluded.lookback_minutes, excluded.job)
                        THEN greatest(s.cursor_minute, excluded.cursor_minute) ELSE s.cursor_minute END
                 RETURNING (to_jsonb(s) - 'namespace')::text",
                &[
                    namespace, &name, expression, timezone, misfire.as_str(),
                    &lookback.to_string(), &job.to_string(),
                ],
            )?;
            if rows.is_empty() {
                return Err("unknown IANA cron timezone".into());
            }
            Ok(only_value(&rows)?.to_owned())
        }
        Request::Unschedule { name } => {
            check_text(&name, "name", 128)?;
            let rows = db.query(
                "DELETE FROM mesh_jobs.cron_schedules WHERE namespace = $1 AND name = $2 RETURNING name",
                &[namespace, &name],
            )?;
            Ok(json!({"removed": !rows.is_empty()}).to_string())
        }
        Request::ListSchedules { after, limit } => {
            let after = after.as_deref().unwrap_or("");
            if !after.is_empty() {
                check_text(after, "after", 128)?;
            }
            let limit = limit.unwrap_or(100);
            if !(1..=500).contains(&limit) {
                return Err("cron list limit must be in 1..=500".into());
            }
            let rows = db.query(
                "SELECT COALESCE(jsonb_agg(to_jsonb(s) - 'namespace' ORDER BY s.name), '[]')::text
                 FROM (SELECT * FROM mesh_jobs.cron_schedules
                       WHERE namespace = $1 AND ($2 = '' OR name > $2)
                       ORDER BY name LIMIT $3::integer) s",
                &[namespace, after, &limit.to_string()],
            )?;
            Ok(only_value(&rows)?.to_owned())
        }
    }
}

fn validate_template(job: Value) -> Result<Value, String> {
    let spec = job.as_object().ok_or("cron job must be an object")?;
    for reserved in ["request_key", "scheduled_at_ms", "delay_ms"] {
        if spec.contains_key(reserved) {
            return Err(format!("cron job cannot set {reserved}"));
        }
    }
    // The queue store owns the job options and their defaults.
    super::store::validate_enqueue_spec(&job.to_string())?;
    Ok(job)
}

/// Poll on an idle connection and return the number of committed occurrences.
/// This includes occurrences resolved by a job's uniqueness policy.
pub(super) fn tick(
    db: &mut impl Database,
    namespace: &str,
    owner: &str,
    limit: u32,
    schedule_limit: u32,
) -> Result<u32, String> {
    tick_with_clock(db, namespace, owner, limit, schedule_limit, None)
}

fn tick_with_clock(
    db: &mut impl Database,
    namespace: &str,
    owner: &str,
    limit: u32,
    schedule_limit: u32,
    now_ms: Option<i64>,
) -> Result<u32, String> {
    check_text(namespace, "namespace", 255)?;
    check_text(owner, "producer identity", 256)?;
    if db.in_transaction() {
        return Err("cron tick needs an idle database connection".into());
    }
    db.simple("BEGIN ISOLATION LEVEL READ COMMITTED")?;
    let result = poll(
        db,
        namespace,
        owner,
        limit.min(MAX_TICK_JOBS),
        schedule_limit.min(MAX_TICK_JOBS),
        now_ms,
    )
    .and_then(|count| {
        if !db.in_transaction() {
            return Err("cron tick did not leave a committable transaction".into());
        }
        db.simple("COMMIT").map(|()| count)
    });
    result.map_err(|error| match db.simple("ROLLBACK") {
        Ok(()) => error,
        Err(cleanup) => format!("{error}; cron rollback failed ({cleanup})"),
    })
}

fn poll(
    db: &mut impl Database,
    namespace: &str,
    owner: &str,
    limit: u32,
    schedule_limit: u32,
    now_ms_override: Option<i64>,
) -> Result<u32, String> {
    // A transaction-scoped lock fences namespace reactivation and scheduling.
    // A losing producer can keep polling its queues without waiting for cron.
    let lock = db.query(
        "SELECT pg_catalog.pg_try_advisory_xact_lock(pg_catalog.hashtextextended($1, 1461749648))::text",
        &[namespace],
    )?;
    if only_value(&lock)? == "false" {
        return Ok(0);
    }
    let now = db.query(
        "SELECT coalesce(nullif($1, '')::bigint, mesh_jobs.now_ms())::text",
        &[&now_ms_override
            .map(|value| value.to_string())
            .unwrap_or_default()],
    )?;
    let now_ms: i64 = only_value(&now)?
        .parse()
        .map_err(|_| "invalid cron clock")?;
    let activation = db.query(
        "INSERT INTO mesh_jobs.cron_activation AS a
            (namespace, epoch, activated_at_ms, activated_by, seen_at_ms)
         VALUES ($1, 1, $2::bigint, $3, $2::bigint)
         ON CONFLICT (namespace) DO UPDATE SET
            epoch = a.epoch + (a.seen_at_ms <= excluded.seen_at_ms - $4::bigint)::integer,
            activated_at_ms = CASE WHEN a.seen_at_ms <= excluded.seen_at_ms - $4::bigint
                THEN excluded.activated_at_ms ELSE a.activated_at_ms END,
            activated_by = CASE WHEN a.seen_at_ms <= excluded.seen_at_ms - $4::bigint
                THEN excluded.activated_by ELSE a.activated_by END,
            seen_at_ms = greatest(a.seen_at_ms, excluded.seen_at_ms)
         RETURNING to_jsonb(a)::text",
        &[
            namespace,
            &now_ms.to_string(),
            owner,
            &ACTIVATION_IDLE_MS.to_string(),
        ],
    )?;
    let activation: Activation = serde_json::from_str(only_value(&activation)?)
        .map_err(|err| format!("invalid cron activation: {err}"))?;
    if limit == 0 {
        return Ok(0);
    }
    let minute = now_ms.div_euclid(MINUTE_MS);
    let candidates = db.query(
        "SELECT (to_jsonb(s) - 'namespace')::text FROM mesh_jobs.cron_schedules s
         WHERE namespace = $1 AND (
            (expression = '@reboot' AND reboot_epoch < $2::bigint)
            OR (expression <> '@reboot' AND cursor_minute < $3::bigint))
         ORDER BY (expression = '@reboot') DESC, cursor_minute, name
         LIMIT $4::integer FOR UPDATE SKIP LOCKED",
        &[
            namespace,
            &activation.epoch.to_string(),
            &minute.to_string(),
            &schedule_limit.to_string(),
        ],
    )?;
    let mut count = 0;
    for row in &candidates {
        let schedule: Schedule = serde_json::from_str(column(row)?)
            .map_err(|err| format!("invalid cron schedule: {err}"))?;
        match Expression::parse(&schedule.expression)? {
            Expression::Reboot => {
                enqueue_occurrence(db, namespace, &schedule, activation.activated_at_ms)?;
                db.query(
                    "UPDATE mesh_jobs.cron_schedules SET reboot_epoch = $3::bigint
                     WHERE namespace = $1 AND name = $2",
                    &[namespace, &schedule.name, &activation.epoch.to_string()],
                )?;
                count += 1;
            }
            Expression::Calendar(masks) => {
                count += run_calendar(db, namespace, &schedule, minute, &masks, limit - count)?;
            }
        }
        if count == limit {
            break;
        }
    }
    if !candidates.is_empty() && now_ms_override.is_none() {
        db.query(
            "UPDATE mesh_jobs.cron_activation SET seen_at_ms = greatest(seen_at_ms, mesh_jobs.now_ms())
             WHERE namespace = $1",
            &[namespace],
        )?;
    }
    Ok(count)
}

fn run_calendar(
    db: &mut impl Database,
    namespace: &str,
    schedule: &Schedule,
    minute: i64,
    masks: &[i64; 5],
    remaining: u32,
) -> Result<u32, String> {
    let (first, latest, max_matches) = match schedule.misfire {
        Misfire::Skip => (minute, false, 1),
        Misfire::Coalesce => (minute - schedule.lookback_minutes + 1, true, 1),
        Misfire::CatchUp => (minute - schedule.lookback_minutes + 1, false, remaining + 1),
    };
    let masks = masks.map(|mask| mask.to_string());
    let matches = db.query(
        "SELECT utc_minute::text
         FROM (SELECT m AS utc_minute, pg_catalog.timezone($3::text, pg_catalog.to_timestamp(m * 60.0)) AS local_time
               FROM generate_series($1::bigint, $2::bigint) m) minutes
         WHERE ($4::bigint & (1::bigint << extract(minute FROM local_time)::integer)) <> 0
           AND ($5::bigint & (1::bigint << extract(hour FROM local_time)::integer)) <> 0
           AND ($6::bigint & (1::bigint << extract(day FROM local_time)::integer)) <> 0
           AND ($7::bigint & (1::bigint << extract(month FROM local_time)::integer)) <> 0
           AND ($8::bigint & (1::bigint << extract(dow FROM local_time)::integer)) <> 0
         ORDER BY CASE WHEN $9::boolean THEN -utc_minute ELSE utc_minute END
         LIMIT $10::integer",
        &[
            &first.max(schedule.cursor_minute + 1).to_string(), &minute.to_string(),
            &schedule.timezone, &masks[0], &masks[1], &masks[2], &masks[3], &masks[4],
            if latest { "true" } else { "false" }, &max_matches.to_string(),
        ],
    )?;
    let count = (matches.len() as u32).min(remaining);
    let mut cursor = minute;
    for row in matches.iter().take(count as usize) {
        let occurrence: i64 = column(row)?
            .parse()
            .map_err(|_| "invalid cron occurrence")?;
        enqueue_occurrence(db, namespace, schedule, occurrence * MINUTE_MS)?;
        if matches.len() > count as usize {
            cursor = occurrence;
        }
    }
    db.query(
        "UPDATE mesh_jobs.cron_schedules SET cursor_minute = $3::bigint
         WHERE namespace = $1 AND name = $2",
        &[namespace, &schedule.name, &cursor.to_string()],
    )?;
    Ok(count)
}

fn enqueue_occurrence(
    db: &mut impl Database,
    namespace: &str,
    schedule: &Schedule,
    scheduled_at_ms: i64,
) -> Result<(), String> {
    let mut job = schedule.job.clone();
    job["scheduled_at_ms"] = json!(scheduled_at_ms);
    enqueue(db, namespace, &job.to_string())?;
    Ok(())
}

#[cfg(test)]
#[path = "cron_tests.rs"]
mod tests;
