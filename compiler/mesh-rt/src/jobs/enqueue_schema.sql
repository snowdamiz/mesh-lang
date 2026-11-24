-- Installed on PostgreSQL 17, whose MERGE RETURNING exposes the source ordinal.
CREATE OR REPLACE FUNCTION mesh_jobs.enqueue_batch_layout_v1() RETURNS boolean
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
BEGIN
    -- A column-scoped INSERT grant cannot take these locks directly. This
    -- helper only locks and inspects the fixed tables; the writes use invoker ACLs.
    LOCK TABLE ONLY mesh_jobs.admissions, ONLY mesh_jobs.queues,
        ONLY mesh_jobs.jobs, ONLY mesh_jobs.receipts
        IN ROW EXCLUSIVE MODE;

    IF NOT EXISTS (SELECT FROM mesh_jobs.schema_version WHERE singleton AND version = 1) THEN
        RETURN false;
    END IF;

    -- Compare with schema.sql on PostgreSQL 17. In particular, a statement
    -- trigger fires even for a zero-row write, and a row trigger can inspect,
    -- change or suppress a job or receipt. Decide before issuing any batch DML.
    RETURN (WITH tables AS (
        SELECT c.*, a.amname, t.reloptions AS toast_options
        FROM pg_class c
        LEFT JOIN pg_am a ON a.oid = c.relam
        LEFT JOIN pg_class t ON t.oid = c.reltoastrelid
        WHERE c.oid IN ('mesh_jobs.jobs'::regclass, 'mesh_jobs.queues'::regclass,
                        'mesh_jobs.receipts'::regclass, 'mesh_jobs.admissions'::regclass)
    ), layout(item) AS (
        SELECT jsonb_build_array('table', relname, relkind, relpersistence,
            amname, relrowsecurity, relforcerowsecurity, relispartition, relhassubclass,
            relhasrules, reloptions, toast_options) FROM tables
        UNION ALL
        SELECT jsonb_build_array('column', t.relname, a.attnum, a.attname,
            a.atttypid, a.atttypmod, a.attcollation, a.attnotnull, a.attidentity,
            a.attgenerated,
            -- Treat only the jobs argument's two supported physical formats
            -- as the same layout. Compression and all other fields still match.
            CASE WHEN t.oid = 'mesh_jobs.jobs'::regclass AND a.attname = 'args'
                      AND a.attstorage IN ('x', 'e') THEN 'x'::"char"
                 ELSE a.attstorage END,
            a.attcompression, a.attisdropped,
            pg_get_expr(d.adbin, d.adrelid))
        FROM tables t JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum > 0
        LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum
        UNION ALL
        SELECT jsonb_build_array('constraint', t.relname, c.conname, c.convalidated,
            pg_get_constraintdef(c.oid))
        FROM tables t JOIN pg_constraint c ON c.conrelid = t.oid
        UNION ALL
        SELECT jsonb_build_array('index', t.relname, i.indisvalid, i.indisready,
            pg_get_indexdef(i.indexrelid))
        FROM tables t JOIN pg_index i ON i.indrelid = t.oid
        UNION ALL
        SELECT jsonb_build_array('trigger', t.relname, tr.tgenabled,
            CASE WHEN tr.tgisinternal THEN tr.tgtype::text ELSE pg_get_triggerdef(tr.oid) END)
        FROM tables t JOIN pg_trigger tr ON tr.tgrelid = t.oid
        UNION ALL
        SELECT jsonb_build_array('function', pg_get_functiondef(p.oid))
        FROM pg_proc p
        WHERE p.oid IN ('mesh_jobs.enqueue(text,jsonb,text,text)'::regprocedure,
                        'mesh_jobs.now_ms()'::regprocedure,
                        'mesh_jobs.effect_key(mesh_jobs.jobs)'::regprocedure,
                        'mesh_jobs.publish_wake_v1()'::regprocedure,
                        'mesh_jobs.fence_claim_v1()'::regprocedure)
    )
    SELECT md5(string_agg(item::text, E'\n' ORDER BY item::text COLLATE "C")) = 'd40b27c60eed17d561be929bc792f074'
    FROM layout);
END
$$;

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
                  WHERE x->'spec'->'unique' IS DISTINCT FROM 'null'::jsonb
                     OR x->'spec'->'replace' IS DISTINCT FROM '{}'::jsonb)
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
    SELECT ns, x.value->'spec'->>'queue'
    FROM jsonb_array_elements(batch) WITH ORDINALITY AS x(value, i)
    GROUP BY x.value->'spec'->>'queue' ORDER BY min(x.i)
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

REVOKE EXECUTE ON FUNCTION mesh_jobs.enqueue_batch_layout_v1(),
    mesh_jobs.enqueue_batch_v1(text,jsonb) FROM PUBLIC;
