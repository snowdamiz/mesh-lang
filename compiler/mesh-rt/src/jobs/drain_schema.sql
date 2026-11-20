CREATE OR REPLACE FUNCTION mesh_jobs.claim_drain_v1(
    ns text, queue_name text, workers jsonb, lease_ms bigint,
    cutoff_ms bigint, initial_ids jsonb
) RETURNS TABLE(id text, token text, context text, timeout_ms text)
LANGUAGE plpgsql VOLATILE SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    q mesh_jobs.queues%ROWTYPE;
    job mesh_jobs.jobs%ROWTYPE;
    due_at bigint;
    started_at bigint;
BEGIN
    IF current_setting('transaction_isolation') <> 'read committed' THEN
        RAISE EXCEPTION 'mesh jobs claim requires READ COMMITTED';
    END IF;
    IF NOT pg_try_advisory_xact_lock_shared(hashtextextended(ns, 1782495106)) THEN RETURN; END IF;
    IF EXISTS (SELECT FROM mesh_jobs.queues WHERE namespace = ns AND name = '' AND state = 'paused')
    THEN RETURN; END IF;

    -- Serialize capacity admission with queue controls without blocking enqueue's FK checks.
    SELECT * INTO q FROM mesh_jobs.queues
    WHERE namespace = ns AND name = queue_name FOR NO KEY UPDATE SKIP LOCKED;
    IF NOT FOUND OR q.state <> 'active' OR q.concurrency = 0 THEN RETURN; END IF;

    IF (SELECT count(*) FROM mesh_jobs.jobs j
        WHERE j.namespace = ns AND j.queue = queue_name AND j.state = 'executing') >= q.concurrency
    THEN RETURN; END IF;

    due_at := coalesce(cutoff_ms, mesh_jobs.now_ms());
    SELECT j.* INTO job FROM mesh_jobs.jobs j
    WHERE j.namespace = ns AND j.queue = queue_name
        AND j.state IN ('available', 'scheduled', 'retryable') AND j.scheduled_at_ms <= due_at
        AND j.attempt < j.max_attempts
        AND (initial_ids IS NULL OR initial_ids ? j.id::text)
        AND EXISTS (
            SELECT FROM jsonb_to_recordset(workers) AS w(worker text, version bigint)
            WHERE w.worker = j.worker AND w.version = j.version
        )
    ORDER BY j.priority, j.scheduled_at_ms, j.position
    LIMIT 1 FOR NO KEY UPDATE OF j SKIP LOCKED;
    IF NOT FOUND THEN RETURN; END IF;

    started_at := mesh_jobs.now_ms();
    UPDATE mesh_jobs.jobs j SET state = 'executing', attempt = j.attempt + 1,
        attempted_at_ms = started_at, lease_token = gen_random_uuid(),
        lease_expires_at_ms = started_at + lease_ms
    WHERE j.namespace = ns AND j.id = job.id RETURNING j.* INTO job;

    RETURN QUERY SELECT job.id::text, job.lease_token::text, jsonb_build_object(
        'id', job.id, 'effect_key', mesh_jobs.effect_key(job), 'worker', job.worker,
        'version', job.version, 'args', job.args, 'attempt', job.attempt,
        'max_attempts', job.max_attempts, 'queue', job.queue, 'metadata', job.metadata,
        'tags', job.tags, 'scheduled_at_ms', job.scheduled_at_ms
    )::text, job.timeout_ms::text;
END
$$;
