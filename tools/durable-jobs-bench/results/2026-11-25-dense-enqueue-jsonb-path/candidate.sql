CREATE OR REPLACE FUNCTION mesh_jobs.enqueue_batch_v1(ns text, batch jsonb)
RETURNS TABLE(ordinal text, receipt text)
LANGUAGE plpgsql VOLATILE SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    processed integer;
BEGIN
    -- The caller holds the same namespace fence as scalar and older producers.
    -- Ordinal zero is reserved for this branch, before the first batch write.
    IF jsonb_array_length(batch) NOT BETWEEN 16 AND 128
       OR NOT mesh_jobs.enqueue_batch_layout_v1()
       OR EXISTS (SELECT FROM jsonb_array_elements(batch) x
                  WHERE x #> '{spec,unique}' IS DISTINCT FROM 'null'::jsonb
                     OR x #> '{spec,replace}' IS DISTINCT FROM '{}'::jsonb)
       OR EXISTS (SELECT FROM jsonb_array_elements(batch) x
                  WHERE x->>'request_key' <> ''
                  GROUP BY x->>'request_key' HAVING count(*) > 1)
       OR EXISTS (SELECT FROM jsonb_array_elements(batch) x JOIN mesh_jobs.receipts r
                  ON r.namespace = ns AND r.request_key = x->>'request_key'
                  WHERE x->>'request_key' <> '') THEN
        RETURN QUERY SELECT '0'::text, 'stock'::text;
        RETURN;
    END IF;

    INSERT INTO mesh_jobs.queues(namespace, name)
    SELECT ns, x.value #>> '{spec,queue}'
    FROM jsonb_array_elements(batch) WITH ORDINALITY AS x(value, i)
    GROUP BY x.value #>> '{spec,queue}' ORDER BY min(x.i)
    ON CONFLICT DO NOTHING;

    RETURN QUERY
    WITH inserted AS (
        MERGE INTO mesh_jobs.jobs j USING (
            SELECT x.i, x.value->'spec' AS s, x.value->>'request_key' AS rk,
                x.value->>'fingerprint' AS fp, mesh_jobs.now_ms() AS ts
            FROM jsonb_array_elements(batch) WITH ORDINALITY AS x(value, i)
            ORDER BY x.i
        ) input ON false
        WHEN NOT MATCHED THEN INSERT (
            namespace, worker, version, queue, args, metadata, tags, state, priority,
            max_attempts, timeout_ms, backoff_base_ms, backoff_max_ms,
            inserted_at_ms, scheduled_at_ms, unique_key
        ) VALUES (
            ns, s->>'worker', (s->>'version')::bigint, s->>'queue',
            s->'args', s->'metadata', s->'tags',
            CASE WHEN (s->>'suspended')::boolean THEN 'suspended'
                WHEN coalesce((s->>'scheduled_at_ms')::bigint, ts + coalesce((s->>'delay_ms')::bigint, 0)) > ts
                THEN 'scheduled' ELSE 'available' END,
            (s->>'priority')::integer, (s->>'max_attempts')::integer, (s->>'timeout_ms')::bigint,
            (s->>'backoff_base_ms')::bigint, (s->>'backoff_max_ms')::bigint,
            ts, coalesce((s->>'scheduled_at_ms')::bigint, ts + coalesce((s->>'delay_ms')::bigint, 0)),
            NULL
        )
        RETURNING input.i, input.rk, input.fp, jsonb_build_object(
            'id', j.id, 'effect_key', mesh_jobs.effect_key(j), 'state', j.state,
            'conflict', false, 'inserted', true
        ) AS result
    ), saved AS (
        INSERT INTO mesh_jobs.receipts(namespace, request_key, fingerprint, receipt)
        SELECT ns, i.rk, i.fp, i.result FROM inserted i WHERE i.rk <> '' ORDER BY i.i
        RETURNING request_key
    )
    SELECT i.i::text, i.result::text FROM inserted i LEFT JOIN saved r ON r.request_key = i.rk
    WHERE i.rk = '' OR r.request_key IS NOT NULL
    ORDER BY i.i;

    GET DIAGNOSTICS processed = ROW_COUNT;
    IF processed <> jsonb_array_length(batch) THEN
        RAISE EXCEPTION 'mesh jobs incomplete set-based enqueue';
    END IF;
END
$$;
