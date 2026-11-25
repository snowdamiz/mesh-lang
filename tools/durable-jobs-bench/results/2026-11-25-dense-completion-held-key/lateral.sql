CREATE OR REPLACE FUNCTION mesh_jobs.complete_batch_v1(
    ns text, ids uuid[], tokens uuid[]
) RETURNS TABLE(ordinal bigint)
LANGUAGE sql VOLATILE SET search_path = pg_catalog, pg_temp AS $$
    WITH held AS MATERIALIZED (
        SELECT j.id, j.lease_token, j.lease_expires_at_ms, p.ordinal
        FROM unnest(ids, tokens) WITH ORDINALITY AS p(id, token, ordinal)
        CROSS JOIN LATERAL (
            SELECT j.id, j.lease_token, j.lease_expires_at_ms
            FROM mesh_jobs.jobs j
            WHERE j.namespace = ns AND j.id = p.id AND j.lease_token = p.token
                AND j.state = 'executing' AND j.cancel_requested_at_ms IS NULL
            FOR NO KEY UPDATE OF j SKIP LOCKED
        ) j
        WHERE cardinality(ids) = cardinality(tokens) AND cardinality(ids) BETWEEN 1 AND 8
    ), checked AS MATERIALIZED (
        SELECT h.*, mesh_jobs.now_ms() AS finished_at FROM held h
    ), completed AS (
        UPDATE mesh_jobs.jobs j SET state = 'completed', finished_at_ms = h.finished_at,
            lease_token = NULL, lease_expires_at_ms = NULL
        FROM checked h WHERE j.namespace = ns AND j.id = h.id AND j.lease_token = h.lease_token
            AND h.lease_expires_at_ms > h.finished_at
        RETURNING h.ordinal
    )
    SELECT c.ordinal FROM completed c
$$;
