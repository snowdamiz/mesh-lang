CREATE OR REPLACE FUNCTION mesh_jobs.recover(ns text, page_size integer, queue_filter jsonb) RETURNS bigint
LANGUAGE plpgsql VOLATILE SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    job record;
    recovered bigint := 0;
    cutoff bigint := mesh_jobs.now_ms();
BEGIN
    FOR job IN SELECT id, lease_token FROM mesh_jobs.jobs
        WHERE namespace = ns AND state = 'executing' AND lease_expires_at_ms <= cutoff
            AND (queue_filter IS NULL OR queue_filter ? queue)
        ORDER BY lease_expires_at_ms, id LIMIT page_size FOR NO KEY UPDATE SKIP LOCKED
    LOOP
        IF mesh_jobs.settle(ns, job.id, job.lease_token,
            '{"action":"fail","reason":"lease expired"}'::jsonb, true)
        THEN recovered := recovered + 1; END IF;
    END LOOP;

    WITH due AS (
        SELECT id FROM mesh_jobs.jobs
        WHERE namespace = ns AND state IN ('scheduled', 'retryable') AND scheduled_at_ms <= cutoff
            AND (queue_filter IS NULL OR queue_filter ? queue)
        ORDER BY scheduled_at_ms, id LIMIT page_size FOR NO KEY UPDATE SKIP LOCKED
    )
    UPDATE mesh_jobs.jobs j SET state = 'available'
    FROM due WHERE j.namespace = ns AND j.id = due.id
        AND EXISTS (SELECT 1 FROM due);
    RETURN recovered;
END
$$;
