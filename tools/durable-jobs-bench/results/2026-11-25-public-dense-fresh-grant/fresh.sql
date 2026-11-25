    target_rel oid := 'mesh_jobs.jobs'::regclass;
    job_ids uuid[];
    locations tid[];
    versions xid[];
    tokens uuid[];
    next_attempts integer[];
    bodies text[];
    timeouts bigint[];
    sizes integer[];
    started_at bigint;
    same_relation boolean;
    updated_ids uuid[];
    valid boolean;
-- section
    -- Capacity already read and locked this relation against rewrites. Keep
    -- the released statement for inherited/partitioned heaps and other AMs.
    IF current_setting('enable_tidscan') = 'on' AND EXISTS (
        SELECT FROM pg_class c
        WHERE c.oid = target_rel AND c.relkind = 'r' AND NOT c.relhassubclass
            AND NOT c.relrowsecurity AND NOT c.relhasrules
            AND c.relam = (SELECT a.oid FROM pg_am a WHERE a.amname = 'heap')
    ) THEN
-- section
    )
    SELECT array_agg(s.id ORDER BY s.priority,s.scheduled_at_ms,s.position) FILTER (WHERE b.fits AND s.id IS NOT NULL),
           array_agg(s.loc ORDER BY s.priority,s.scheduled_at_ms,s.position) FILTER (WHERE b.fits AND s.id IS NOT NULL),
           array_agg(s.version_xid ORDER BY s.priority,s.scheduled_at_ms,s.position) FILTER (WHERE b.fits AND s.id IS NOT NULL),
           array_agg(s.grant_token ORDER BY s.priority,s.scheduled_at_ms,s.position) FILTER (WHERE b.fits AND s.id IS NOT NULL),
           array_agg(s.next_attempt ORDER BY s.priority,s.scheduled_at_ms,s.position) FILTER (WHERE b.fits AND s.id IS NOT NULL),
           array_agg(s.body ORDER BY s.priority,s.scheduled_at_ms,s.position) FILTER (WHERE b.fits AND s.id IS NOT NULL),
           array_agg(s.timeout_ms ORDER BY s.priority,s.scheduled_at_ms,s.position) FILTER (WHERE b.fits AND s.id IS NOT NULL),
           array_agg(s.bytes ORDER BY s.priority,s.scheduled_at_ms,s.position) FILTER (WHERE b.fits AND s.id IS NOT NULL),
           b.started_at, coalesce(bool_and(s.rel = target_rel), true)
    INTO job_ids, locations, versions, tokens, next_attempts, bodies, timeouts, sizes, started_at, same_relation
    FROM budget b LEFT JOIN sized s ON true
    GROUP BY b.started_at;
-- section
    -- The pick includes descendants. Detect a child that was attached
    -- between the catalog check and pick before interpreting any TIDs.
    IF same_relation THEN
        -- A VOLATILE function gets a fresh READ COMMITTED snapshot here.
        -- An empty array still executes one statement for statement triggers.
        WITH granted AS (
            UPDATE ONLY mesh_jobs.jobs j
            SET state = 'executing', attempt = j.attempt + 1,
                attempted_at_ms = started_at,
                lease_token = tokens[array_position(locations, j.ctid)],
                lease_expires_at_ms = started_at + lease_ms
            WHERE j.ctid = ANY (coalesce(locations, ARRAY[]::tid[]))
                AND j.namespace = ns AND j.tableoid = target_rel
                AND j.id = job_ids[array_position(locations, j.ctid)]
                AND j.xmin = versions[array_position(locations, j.ctid)]
            RETURNING j.id AS job_id, j.namespace = ns AND j.tableoid = target_rel
                AND j.queue = queue_name AND j.state = 'executing'
                AND j.attempt = next_attempts[array_position(job_ids, j.id)]
                AND j.lease_token = tokens[array_position(job_ids, j.id)]
                AND j.attempted_at_ms = started_at
                AND j.lease_expires_at_ms = started_at + lease_ms AS valid_grant
        )
        SELECT array_agg(g.job_id ORDER BY array_position(job_ids,g.job_id)),
            coalesce(bool_and(g.valid_grant IS TRUE), true)
        INTO updated_ids, valid
        FROM granted g;

        -- Any mismatch aborts all writes and queued notifications. The
        -- existing caller may only observe a full ordered batch or an error.
        IF NOT valid OR updated_ids IS DISTINCT FROM job_ids THEN
            RAISE EXCEPTION 'mesh jobs claim lost or changed a locked grant row' USING ERRCODE = '55000';
        END IF;
-- section
        IF job_ids IS NOT NULL THEN
            RETURN QUERY SELECT 'claimed'::text, r.job_id::text, r.lease_token::text,
                r.body, r.timeout_ms::text, r.bytes::text
            FROM unnest(job_ids,tokens,bodies,timeouts,sizes)
                WITH ORDINALITY AS r(job_id,lease_token,body,timeout_ms,bytes,ord)
            ORDER BY r.ord;
            RETURN;
        END IF;
    END IF;
