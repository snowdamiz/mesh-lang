use super::store::{claim_one_transaction, response, Claim, Database};

pub(super) struct Selection<'a> {
    pub(super) namespace: &'a str,
    pub(super) queues: &'a str,
    pub(super) workers: &'a str,
    pub(super) cutoff_ms: String,
}

const ELIGIBLE: &str = "
    FROM mesh_jobs.jobs j JOIN mesh_jobs.queues q
        ON q.namespace = j.namespace AND q.name = j.queue
    WHERE j.namespace = $1 AND $2::jsonb ? j.queue
        AND q.state = 'active' AND q.concurrency > 0
        AND j.scheduled_at_ms <= coalesce(nullif($4, '')::bigint, mesh_jobs.now_ms())
        AND EXISTS (SELECT FROM jsonb_to_recordset($3::jsonb) AS w(worker text, version bigint)
                    WHERE w.worker = j.worker AND w.version = j.version)";

const READY: &str = "
    j.state IN ('available', 'scheduled', 'retryable') AND j.attempt < j.max_attempts
    AND NOT EXISTS (SELECT FROM mesh_jobs.queues paused
                    WHERE paused.namespace = $1 AND paused.name = '' AND paused.state = 'paused')";

impl Selection<'_> {
    pub(super) fn initial_ids(
        &self,
        db: &mut impl Database,
        limit: u64,
    ) -> Result<Vec<String>, String> {
        let rows = db.query(
            &format!(
                "SELECT j.id::text {ELIGIBLE} AND {READY}
                 ORDER BY j.priority, j.scheduled_at_ms, j.position LIMIT $5::bigint"
            ),
            &[
                self.namespace,
                self.queues,
                self.workers,
                &self.cutoff_ms,
                &limit.to_string(),
            ],
        )?;
        rows.into_iter()
            .map(|row| match row.as_slice() {
                [(_, id)] => Ok(id.clone()),
                _ => Err("invalid job id from PostgreSQL".into()),
            })
            .collect()
    }

    pub(super) fn claim(
        &self,
        db: &mut impl Database,
        queue: &str,
        lease_ms: u64,
        initial_ids: Option<&[String]>,
        observe: bool,
    ) -> Result<Option<Claim>, String> {
        let ids = initial_ids
            .map(|ids| serde_json::json!(ids).to_string())
            .unwrap_or_default();
        let timing = if observe {
            ", floor(extract(epoch FROM pg_catalog.clock_timestamp()) * 1000)::bigint::text"
        } else {
            ""
        };
        claim_one_transaction(
            db,
            &format!(
                "SELECT id, token, context, timeout_ms{timing} FROM mesh_jobs.claim_drain_v1(
                $1, $2, $3::jsonb, $4::bigint, nullif($5, '')::bigint, nullif($6, '')::jsonb
             )"
            ),
            &[
                self.namespace,
                queue,
                self.workers,
                &lease_ms.to_string(),
                &self.cutoff_ms,
                &ids,
            ],
            observe,
        )
    }

    pub(super) fn recover(&self, db: &mut impl Database, limit: u64) -> Result<u64, String> {
        response(
            db,
            &format!(
                "WITH expired AS MATERIALIZED (
                     SELECT j.id, j.lease_token {ELIGIBLE}
                     AND j.state = 'executing' AND j.lease_expires_at_ms <= mesh_jobs.now_ms()
                     ORDER BY j.lease_expires_at_ms, j.id LIMIT $5::bigint FOR NO KEY UPDATE OF j SKIP LOCKED
                 ), settled AS MATERIALIZED (
                     SELECT mesh_jobs.settle_state($1, id, lease_token,
                         '{{\"action\":\"fail\",\"reason\":\"lease expired\"}}'::jsonb, true) AS state
                     FROM expired
                 ) SELECT count(state)::text FROM settled"
            ),
            &[self.namespace, self.queues, self.workers, &self.cutoff_ms, &limit.to_string()],
        )?
        .parse()
        .map_err(|_| "invalid PostgreSQL drain recovery count".into())
    }

    pub(super) fn remaining(&self, db: &mut impl Database) -> Result<bool, String> {
        response(
            db,
            &format!(
                "SELECT EXISTS (SELECT {ELIGIBLE} AND (({READY}) OR j.state = 'executing'))::text"
            ),
            &[self.namespace, self.queues, self.workers, &self.cutoff_ms],
        )
        .map(|remaining| remaining == "true")
    }
}
