CREATE OR REPLACE FUNCTION mesh_jobs.claim_batch_set_v1(
    ns text, queue_name text, workers jsonb, lease_ms bigint, slots integer, cutoff_ms bigint
) RETURNS TABLE(status text, id text, token text, context text, timeout_ms text, context_bytes text)
LANGUAGE plpgsql VOLATILE SET search_path = pg_catalog, pg_temp SET lock_timeout = '100ms' AS $$
DECLARE
    q mesh_jobs.queues%ROWTYPE;
    remaining integer := least(coalesce(slots, 0), 16);
    due_at bigint;
BEGIN
    IF lease_ms IS NULL OR lease_ms NOT BETWEEN 1 AND 31536000000 THEN
        RAISE EXCEPTION 'mesh jobs admitted claim lease_ms must be positive and at most 365 days';
    END IF;
    IF remaining <= 0 THEN RETURN; END IF;
    IF current_setting('transaction_isolation') <> 'read committed'
        OR current_setting('client_encoding') <> current_setting('server_encoding') THEN
        RETURN QUERY SELECT 'fallback'::text, NULL::text, NULL::text, NULL::text, NULL::text, NULL::text;
        RETURN;
    END IF;

    -- Serialize with single claims, cursor batches and queue controls.
    -- Namespace pause takes its advisory lock first. Never wait for it
    -- while holding a queue row, including when a legacy claimant is present.
    SELECT * INTO q FROM mesh_jobs.queues
    WHERE namespace = ns AND name = queue_name FOR NO KEY UPDATE;
    IF NOT FOUND OR q.state <> 'active' OR q.concurrency = 0 THEN RETURN; END IF;
    IF NOT pg_try_advisory_xact_lock_shared(hashtextextended(ns, 1782495106)) THEN RETURN; END IF;
    IF EXISTS (SELECT FROM mesh_jobs.queues WHERE namespace = ns AND name = '' AND state = 'paused')
    THEN RETURN; END IF;

    -- This command takes a fresh snapshot after acquiring the capacity lock.
    remaining := least(remaining, q.concurrency - (SELECT count(*) FROM mesh_jobs.jobs j
        WHERE j.namespace = ns AND j.queue = queue_name AND j.state = 'executing'));
    IF remaining <= 0 THEN RETURN; END IF;

    due_at := coalesce(cutoff_ms, mesh_jobs.now_ms());
    RETURN QUERY
    WITH picked AS MATERIALIZED (
        SELECT j.* FROM mesh_jobs.jobs j
        WHERE j.namespace = ns AND j.queue = queue_name
            AND j.state IN ('available', 'scheduled', 'retryable') AND j.scheduled_at_ms <= due_at
            AND j.attempt < j.max_attempts
            AND EXISTS (SELECT FROM jsonb_to_recordset(workers) AS w(worker text, version bigint)
                        WHERE (w.worker, w.version) = (j.worker, j.version))
            AND NOT EXISTS (
                SELECT FROM mesh_jobs.batch_context_blocks b
                WHERE b.namespace = j.namespace AND b.id = j.id
                    AND b.row_version = j.xmin AND b.row_location = j.ctid
                    AND b.encoding = current_setting('client_encoding')::name)
        ORDER BY j.priority, j.scheduled_at_ms, j.position
        LIMIT remaining FOR NO KEY UPDATE OF j SKIP LOCKED
    ), rendered AS MATERIALIZED (
        -- Keep speculative memory bounded before detoasting. Compressed and
        -- larger values go to the released exact-prefix cursor on a new turn.
        SELECT p.namespace AS grant_namespace, p.id, p.priority, p.scheduled_at_ms, p.position, p.timeout_ms,
            gen_random_uuid() AS grant_token,
            CASE WHEN pg_column_size(p.args)::bigint + pg_column_size(p.metadata)
                    + pg_column_size(p.tags) <= 65536
                    AND pg_column_compression(p.args) IS NULL
                    AND pg_column_compression(p.metadata) IS NULL
                    AND pg_column_compression(p.tags) IS NULL
                    AND octet_length(p.worker) + octet_length(p.queue) <= 1024
            THEN jsonb_build_object(
                'id', p.id, 'effect_key', p.id::text || '/' || p.run::text, 'worker', p.worker,
                'version', p.version, 'args', p.args, 'attempt', p.attempt + 1,
                'max_attempts', p.max_attempts, 'queue', p.queue, 'metadata', p.metadata,
                'tags', p.tags, 'scheduled_at_ms', p.scheduled_at_ms
            )::text END AS body
        FROM picked p
    ), sized AS MATERIALIZED (
        SELECT r.*, octet_length(r.body) AS bytes FROM rendered r
    ), budget AS MATERIALIZED (
        SELECT coalesce(bool_and(s.bytes IS NOT NULL AND s.bytes <= 16 * 1024 * 1024)
                     AND sum(s.bytes + 128) <= 32 * 1024 * 1024 - 1024, true) AS fits,
               mesh_jobs.now_ms() AS started_at
        FROM sized s
    ), granted AS (
        UPDATE mesh_jobs.jobs j SET state = 'executing', attempt = j.attempt + 1,
            attempted_at_ms = b.started_at, lease_token = s.grant_token,
            lease_expires_at_ms = b.started_at + lease_ms
        FROM sized s, budget b WHERE b.fits AND j.namespace = s.grant_namespace AND j.id = s.id
            AND (j.namespace, j.id) BETWEEN (s.grant_namespace, s.id)
                                        AND (s.grant_namespace, s.id)
        RETURNING j.id AS job_id, s.grant_token, s.body, s.timeout_ms, s.bytes,
                  s.priority, s.scheduled_at_ms, s.position
    )
    SELECT 'claimed'::text, g.job_id::text, g.grant_token::text, g.body, g.timeout_ms::text, g.bytes::text
    FROM granted g ORDER BY g.priority, g.scheduled_at_ms, g.position;

    IF FOUND THEN RETURN; END IF;
    -- The speculative transaction made no state transitions. Determine whether
    -- exact sizing or a busy eligible row needs the existing claim protocol.
    IF EXISTS (
        SELECT FROM mesh_jobs.jobs j
        WHERE j.namespace = ns AND j.queue = queue_name
            AND j.state IN ('available', 'scheduled', 'retryable') AND j.scheduled_at_ms <= due_at
            AND j.attempt < j.max_attempts
            AND EXISTS (SELECT FROM jsonb_to_recordset(workers) AS w(worker text, version bigint)
                        WHERE (w.worker, w.version) = (j.worker, j.version))
            AND NOT EXISTS (
                SELECT FROM mesh_jobs.batch_context_blocks b
                WHERE b.namespace = j.namespace AND b.id = j.id
                    AND b.row_version = j.xmin AND b.row_location = j.ctid
                    AND b.encoding = current_setting('client_encoding')::name)
    ) THEN
        RETURN QUERY SELECT 'fallback'::text, NULL::text, NULL::text, NULL::text, NULL::text, NULL::text;
    ELSE
        -- Use the exact scan cutoff to cover a deadline crossing after this
        -- transaction and before the separate next-due read.
        RETURN QUERY SELECT 'empty'::text, NULL::text, NULL::text, NULL::text, due_at::text, NULL::text;
    END IF;
END
$$;
