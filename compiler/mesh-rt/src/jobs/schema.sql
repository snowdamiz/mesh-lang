CREATE SCHEMA IF NOT EXISTS mesh_jobs;

CREATE TABLE IF NOT EXISTS mesh_jobs.schema_version (
    singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    version integer NOT NULL
);

INSERT INTO mesh_jobs.schema_version (version) VALUES (1)
ON CONFLICT (singleton) DO NOTHING;

DO $$
BEGIN
    IF NOT EXISTS (SELECT FROM mesh_jobs.schema_version WHERE version = 1) THEN
        RAISE EXCEPTION 'unsupported mesh_jobs schema version';
    END IF;
END
$$;

CREATE TABLE IF NOT EXISTS mesh_jobs.admissions (
    namespace text PRIMARY KEY,
    fence boolean NOT NULL DEFAULT false
);

CREATE TABLE IF NOT EXISTS mesh_jobs.queues (
    namespace text NOT NULL,
    name text NOT NULL,
    concurrency integer NOT NULL DEFAULT 1 CHECK (concurrency BETWEEN 0 AND 1024),
    state text NOT NULL DEFAULT 'active' CHECK (state IN ('active', 'paused', 'stopped')),
    PRIMARY KEY (namespace, name)
);

CREATE TABLE IF NOT EXISTS mesh_jobs.jobs (
    namespace text NOT NULL,
    id uuid NOT NULL DEFAULT pg_catalog.gen_random_uuid(),
    position bigint GENERATED ALWAYS AS IDENTITY,
    run bigint NOT NULL DEFAULT 1 CHECK (run > 0),
    worker text NOT NULL,
    version bigint NOT NULL CHECK (version BETWEEN 1 AND 4294967295),
    queue text NOT NULL,
    args jsonb NOT NULL CHECK (jsonb_typeof(args) = 'object'),
    metadata jsonb NOT NULL CHECK (jsonb_typeof(metadata) = 'object'),
    tags jsonb NOT NULL CHECK (jsonb_typeof(tags) = 'array'),
    state text NOT NULL CHECK (state IN
        ('available', 'scheduled', 'retryable', 'suspended', 'executing',
         'completed', 'cancelled', 'discarded')),
    priority integer NOT NULL CHECK (priority BETWEEN 0 AND 9),
    attempt integer NOT NULL DEFAULT 0 CHECK (attempt BETWEEN 0 AND max_attempts),
    max_attempts integer NOT NULL CHECK (max_attempts BETWEEN 1 AND 100),
    timeout_ms bigint NOT NULL CHECK (timeout_ms > 0),
    backoff_base_ms bigint NOT NULL CHECK (backoff_base_ms >= 0),
    backoff_max_ms bigint NOT NULL CHECK (backoff_max_ms >= backoff_base_ms),
    inserted_at_ms bigint NOT NULL,
    scheduled_at_ms bigint NOT NULL,
    attempted_at_ms bigint,
    finished_at_ms bigint,
    lease_token uuid,
    lease_expires_at_ms bigint,
    cancel_requested_at_ms bigint,
    errors jsonb NOT NULL DEFAULT '[]' CHECK (jsonb_typeof(errors) = 'array'),
    unique_key text,
    PRIMARY KEY (namespace, id),
    FOREIGN KEY (namespace, queue) REFERENCES mesh_jobs.queues(namespace, name),
    CHECK ((state = 'executing') = (lease_token IS NOT NULL)),
    CHECK (state <> 'executing' OR attempt > 0),
    CHECK ((state = 'executing') = (lease_expires_at_ms IS NOT NULL)),
    CHECK (state = 'executing' OR cancel_requested_at_ms IS NULL),
    CHECK ((state IN ('completed', 'cancelled', 'discarded')) = (finished_at_ms IS NOT NULL))
);

CREATE INDEX IF NOT EXISTS mesh_jobs_ready
    ON mesh_jobs.jobs (namespace, queue, priority, scheduled_at_ms, position)
    WHERE state IN ('available', 'scheduled', 'retryable');
CREATE INDEX IF NOT EXISTS mesh_jobs_due
    ON mesh_jobs.jobs (namespace, scheduled_at_ms, id) WHERE state IN ('scheduled', 'retryable');
CREATE INDEX IF NOT EXISTS mesh_jobs_running
    ON mesh_jobs.jobs (namespace, queue) WHERE state = 'executing';
CREATE INDEX IF NOT EXISTS mesh_jobs_expiring
    ON mesh_jobs.jobs (namespace, lease_expires_at_ms, id) WHERE state = 'executing';
CREATE INDEX IF NOT EXISTS mesh_jobs_finished
    ON mesh_jobs.jobs (namespace, finished_at_ms, id) WHERE finished_at_ms IS NOT NULL;
CREATE INDEX IF NOT EXISTS mesh_jobs_unique_candidates
    ON mesh_jobs.jobs (namespace, worker, queue, inserted_at_ms DESC);
CREATE INDEX IF NOT EXISTS mesh_jobs_unique_key
    ON mesh_jobs.jobs (namespace, unique_key) WHERE unique_key IS NOT NULL;

CREATE TABLE IF NOT EXISTS mesh_jobs.receipts (
    namespace text NOT NULL,
    request_key text NOT NULL,
    fingerprint text NOT NULL,
    receipt jsonb NOT NULL,
    PRIMARY KEY (namespace, request_key)
);

CREATE OR REPLACE FUNCTION mesh_jobs.now_ms() RETURNS bigint
LANGUAGE sql VOLATILE SET search_path = pg_catalog, pg_temp AS $$
    SELECT floor(extract(epoch FROM clock_timestamp()) * 1000)::bigint
$$;

CREATE OR REPLACE FUNCTION mesh_jobs.effect_key(j mesh_jobs.jobs) RETURNS text
LANGUAGE sql IMMUTABLE STRICT SET search_path = pg_catalog, pg_temp AS $$
    SELECT j.id::text || '/' || j.run::text
$$;

CREATE OR REPLACE FUNCTION mesh_jobs.job_json(j mesh_jobs.jobs) RETURNS jsonb
LANGUAGE sql STABLE STRICT SET search_path = pg_catalog, pg_temp AS $$
    SELECT (to_jsonb(j) - ARRAY['namespace', 'lease_token', 'unique_key', 'position']) ||
        jsonb_build_object('effect_key', mesh_jobs.effect_key(j))
$$;

CREATE OR REPLACE FUNCTION mesh_jobs.queue_json(q mesh_jobs.queues) RETURNS jsonb
LANGUAGE sql STABLE STRICT SET search_path = pg_catalog, pg_temp AS $$
    SELECT jsonb_build_object(
        'queue', q.name, 'concurrency', q.concurrency, 'state', q.state,
        'running', (SELECT count(*) FROM mesh_jobs.jobs j
                    WHERE j.namespace = q.namespace AND j.queue = q.name AND j.state = 'executing'))
$$;

CREATE OR REPLACE FUNCTION mesh_jobs.unique_values(payload jsonb, keys jsonb) RETURNS jsonb
LANGUAGE sql IMMUTABLE STRICT SET search_path = pg_catalog, pg_temp AS $$
    SELECT CASE WHEN keys = '[]' THEN payload ELSE coalesce((
        SELECT jsonb_object_agg(k, payload -> k)
        FROM jsonb_array_elements_text(keys) AS k
        WHERE payload ? k
    ), '{}'::jsonb) END
$$;

CREATE OR REPLACE FUNCTION mesh_jobs.enqueue(
    ns text, spec jsonb, request_id text, fingerprint text
) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    recorded mesh_jobs.receipts%ROWTYPE;
    job mesh_jobs.jobs%ROWTYPE;
    policy jsonb := spec -> 'unique';
    fields jsonb := policy -> 'fields';
    keys jsonb := policy -> 'keys';
    replacements jsonb;
    queue_name text := spec ->> 'queue';
    now_at bigint;
    run_at bigint;
    duplicate boolean := false;
    result jsonb;
BEGIN
    IF request_id <> '' THEN
        SELECT * INTO recorded FROM mesh_jobs.receipts r
        WHERE r.namespace = ns AND r.request_key = request_id;
        IF FOUND THEN
            IF recorded.fingerprint <> fingerprint THEN
                RAISE EXCEPTION 'mesh jobs request_key already bound to a different spec';
            END IF;
            RETURN recorded.receipt;
        END IF;
    END IF;

    now_at := mesh_jobs.now_ms();
    run_at := coalesce((spec ->> 'scheduled_at_ms')::bigint,
                       now_at + coalesce((spec ->> 'delay_ms')::bigint, 0));
    IF policy <> 'null'::jsonb THEN
        SELECT j.* INTO job FROM mesh_jobs.jobs j
        WHERE j.namespace = ns AND (policy -> 'states') ? j.state
            AND ((policy ->> 'key') IS NULL OR j.unique_key = policy ->> 'key')
            AND (NOT fields ? 'worker' OR j.worker = spec ->> 'worker')
            AND (NOT fields ? 'version' OR j.version = (spec ->> 'version')::bigint)
            AND (NOT fields ? 'queue' OR j.queue = queue_name)
            AND (NOT fields ? 'args' OR
                mesh_jobs.unique_values(j.args, keys) = mesh_jobs.unique_values(spec -> 'args', keys))
            AND (NOT fields ? 'metadata' OR
                mesh_jobs.unique_values(j.metadata, keys) = mesh_jobs.unique_values(spec -> 'metadata', keys))
            AND ((policy ->> 'period_ms') IS NULL OR
                (CASE policy ->> 'timestamp' WHEN 'scheduled_at' THEN j.scheduled_at_ms
                    ELSE j.inserted_at_ms END) > now_at - (policy ->> 'period_ms')::bigint)
        ORDER BY j.inserted_at_ms DESC, j.position DESC
        LIMIT 1 FOR NO KEY UPDATE;
        duplicate := FOUND;
    END IF;

    IF duplicate THEN
        replacements := (spec -> 'replace') -> job.state;
        IF replacements IS NOT NULL AND replacements <> '[]'::jsonb THEN
            IF replacements ? 'queue' AND job.state <> 'executing' THEN
                INSERT INTO mesh_jobs.queues(namespace, name) VALUES (ns, queue_name)
                ON CONFLICT DO NOTHING;
            END IF;
            UPDATE mesh_jobs.jobs j SET
                args = CASE WHEN replacements ? 'args' THEN spec -> 'args' ELSE j.args END,
                metadata = CASE WHEN replacements ? 'metadata' THEN spec -> 'metadata' ELSE j.metadata END,
                priority = CASE WHEN replacements ? 'priority' THEN (spec ->> 'priority')::int ELSE j.priority END,
                tags = CASE WHEN replacements ? 'tags' THEN spec -> 'tags' ELSE j.tags END,
                max_attempts = CASE WHEN replacements ? 'max_attempts'
                    THEN greatest(j.attempt + (j.state IN
                        ('available', 'scheduled', 'retryable', 'suspended'))::int,
                        (spec ->> 'max_attempts')::int) ELSE j.max_attempts END,
                queue = CASE WHEN replacements ? 'queue' AND j.state <> 'executing'
                    THEN queue_name ELSE j.queue END,
                worker = CASE WHEN replacements ? 'worker' AND j.state <> 'executing'
                    THEN spec ->> 'worker' ELSE j.worker END,
                version = CASE WHEN replacements ? 'worker' AND j.state <> 'executing'
                    THEN (spec ->> 'version')::bigint ELSE j.version END,
                scheduled_at_ms = CASE WHEN replacements ? 'scheduled_at_ms' AND j.state <> 'executing'
                    THEN run_at ELSE j.scheduled_at_ms END,
                state = CASE WHEN replacements ? 'scheduled_at_ms'
                           AND j.state IN ('available', 'scheduled', 'retryable')
                    THEN CASE WHEN run_at > now_at THEN 'scheduled' ELSE 'available' END
                    ELSE j.state END
            WHERE j.namespace = ns AND j.id = job.id RETURNING j.* INTO job;
        END IF;
    ELSE
        INSERT INTO mesh_jobs.queues(namespace, name) VALUES (ns, queue_name)
        ON CONFLICT DO NOTHING;
        INSERT INTO mesh_jobs.jobs (
            namespace, worker, version, queue, args, metadata, tags, state, priority,
            max_attempts, timeout_ms, backoff_base_ms, backoff_max_ms,
            inserted_at_ms, scheduled_at_ms, unique_key
        ) VALUES (
            ns, spec ->> 'worker', (spec ->> 'version')::bigint, queue_name,
            spec -> 'args', spec -> 'metadata', spec -> 'tags',
            CASE WHEN (spec ->> 'suspended')::boolean THEN 'suspended'
                 WHEN run_at > now_at THEN 'scheduled' ELSE 'available' END,
            (spec ->> 'priority')::int, (spec ->> 'max_attempts')::int,
            (spec ->> 'timeout_ms')::bigint, (spec ->> 'backoff_base_ms')::bigint,
            (spec ->> 'backoff_max_ms')::bigint, now_at, run_at, policy ->> 'key'
        ) RETURNING * INTO job;
    END IF;

    result := jsonb_build_object(
        'id', job.id, 'effect_key', mesh_jobs.effect_key(job), 'state', job.state,
        'conflict', duplicate, 'inserted', NOT duplicate);
    IF request_id <> '' THEN
        INSERT INTO mesh_jobs.receipts VALUES (ns, request_id, fingerprint, result);
    END IF;
    RETURN result;
END
$$;

CREATE OR REPLACE FUNCTION mesh_jobs.claim(
    ns text, queue_name text, workers jsonb, lease_ms bigint
) RETURNS TABLE(id text, token text, context text, timeout_ms text)
LANGUAGE plpgsql VOLATILE SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    q mesh_jobs.queues%ROWTYPE;
    job mesh_jobs.jobs%ROWTYPE;
    due_at bigint;
    started_at bigint;
BEGIN
    -- Serialize capacity admission with queue controls without blocking enqueue's FK checks.
    SELECT * INTO q FROM mesh_jobs.queues
    WHERE namespace = ns AND name = queue_name FOR NO KEY UPDATE SKIP LOCKED;
    IF NOT FOUND OR q.state <> 'active' OR q.concurrency = 0 THEN RETURN; END IF;

    IF (SELECT count(*) FROM mesh_jobs.jobs j
        WHERE j.namespace = ns AND j.queue = queue_name AND j.state = 'executing') >= q.concurrency
    THEN RETURN; END IF;

    due_at := mesh_jobs.now_ms();
    SELECT j.* INTO job FROM mesh_jobs.jobs j
    WHERE j.namespace = ns AND j.queue = queue_name
        AND j.state IN ('available', 'scheduled', 'retryable') AND j.scheduled_at_ms <= due_at
        AND j.attempt < j.max_attempts
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

CREATE OR REPLACE FUNCTION mesh_jobs.claim_batch_v1(
    ns text, queue_name text, workers jsonb, lease_ms bigint, slots integer, cutoff_ms bigint
) RETURNS TABLE(status text, id text, token text, context text, timeout_ms text, context_bytes text)
LANGUAGE plpgsql VOLATILE SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    q mesh_jobs.queues%ROWTYPE;
    job mesh_jobs.jobs%ROWTYPE;
    due_at bigint;
    started_at bigint;
    remaining integer;
    oversized_count integer := 0;
    value_bytes integer;
    row_bytes integer;
    -- Reserve 256 bytes for the six-column RowDescription and the other reply frames.
    reply_bytes integer := 32 * 1024 * 1024 - 256;
    wire_encoding name := current_setting('client_encoding');
    same_encoding boolean := wire_encoding = current_setting('server_encoding');
    candidates NO SCROLL CURSOR FOR
        SELECT j.* FROM mesh_jobs.jobs j
        WHERE j.namespace = ns AND j.queue = queue_name
            AND j.state IN ('available', 'scheduled', 'retryable') AND j.scheduled_at_ms <= due_at
            AND j.attempt < j.max_attempts
            AND EXISTS (
                SELECT FROM jsonb_to_recordset(workers) AS w(worker text, version bigint)
                WHERE w.worker = j.worker AND w.version = j.version
            )
        ORDER BY j.priority, j.scheduled_at_ms, j.position
        FOR NO KEY UPDATE OF j SKIP LOCKED;
BEGIN
    -- Capacity must be read with a fresh snapshot after taking the same lock as claim().
    IF current_setting('transaction_isolation') <> 'read committed' THEN
        RAISE EXCEPTION 'mesh jobs batch claim requires READ COMMITTED';
    END IF;
    IF lease_ms IS NULL OR lease_ms NOT BETWEEN 1 AND 31536000000 THEN
        RAISE EXCEPTION 'mesh jobs batch claim lease_ms must be positive and at most 365 days';
    END IF;
    remaining := least(coalesce(slots, 0), 16);
    IF remaining <= 0 THEN RETURN; END IF;

    SELECT * INTO q FROM mesh_jobs.queues
    WHERE namespace = ns AND name = queue_name FOR NO KEY UPDATE SKIP LOCKED;
    IF NOT FOUND OR q.state <> 'active' OR q.concurrency = 0 THEN RETURN; END IF;

    remaining := least(remaining, q.concurrency - (SELECT count(*) FROM mesh_jobs.jobs j
        WHERE j.namespace = ns AND j.queue = queue_name AND j.state = 'executing'));
    IF remaining <= 0 THEN RETURN; END IF;

    due_at := coalesce(cutoff_ms, mesh_jobs.now_ms());
    -- Fetch one row at a time so a cursor prefetch cannot lock work beyond the prefix.
    OPEN candidates;
    WHILE remaining > 0 LOOP
        FETCH candidates INTO job;
        EXIT WHEN NOT FOUND;
        id := job.id::text;
        context := jsonb_build_object(
            'id', job.id, 'effect_key', mesh_jobs.effect_key(job), 'worker', job.worker,
            'version', job.version, 'args', job.args, 'attempt', job.attempt + 1,
            'max_attempts', job.max_attempts, 'queue', job.queue, 'metadata', job.metadata,
            'tags', job.tags, 'scheduled_at_ms', job.scheduled_at_ms
        )::text;
        value_bytes := CASE WHEN same_encoding THEN octet_length(context)
            ELSE octet_length(convert_to(context, wire_encoding)) END;
        context_bytes := value_bytes::text;

        -- DataRow adds a 5-byte frame, a 2-byte count, and six 4-byte field lengths.
        row_bytes := 31 + octet_length(id) + octet_length(context_bytes);
        IF value_bytes > 16 * 1024 * 1024 THEN
            status := 'oversized';
            row_bytes := row_bytes + octet_length(status);
            -- Bound diagnostics independently of slots while continuing past oversized jobs.
            IF oversized_count < 16 AND row_bytes <= reply_bytes THEN
                token := NULL;
                context := NULL;
                timeout_ms := NULL;
                RETURN NEXT;
                reply_bytes := reply_bytes - row_bytes;
                oversized_count := oversized_count + 1;
            END IF;
            CONTINUE;
        END IF;

        status := 'claimed';
        token := gen_random_uuid()::text;
        timeout_ms := job.timeout_ms::text;
        row_bytes := row_bytes + octet_length(status) + octet_length(token) +
            value_bytes + octet_length(timeout_ms);
        -- Keep the ordered prefix of claimable jobs when the next one cannot fit.
        EXIT WHEN row_bytes > reply_bytes;

        started_at := mesh_jobs.now_ms();
        UPDATE mesh_jobs.jobs j SET state = 'executing', attempt = j.attempt + 1,
            attempted_at_ms = started_at, lease_token = token::uuid,
            lease_expires_at_ms = started_at + lease_ms
        WHERE j.namespace = ns AND j.id = job.id;

        RETURN NEXT;
        reply_bytes := reply_bytes - row_bytes;
        remaining := remaining - 1;
    END LOOP;
    CLOSE candidates;
END
$$;

CREATE OR REPLACE FUNCTION mesh_jobs.try_renew(
    ns text, job_id uuid, token uuid, lease_ms bigint
) RETURNS text
LANGUAGE plpgsql VOLATILE SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    job mesh_jobs.jobs%ROWTYPE;
    now_at bigint;
BEGIN
    -- A busy attempt must not delay lease renewals for the runner's other jobs.
    SELECT * INTO job FROM mesh_jobs.jobs
    WHERE namespace = ns AND id = job_id AND lease_token = token FOR NO KEY UPDATE NOWAIT;
    IF NOT FOUND THEN RETURN 'lost'; END IF;
    now_at := mesh_jobs.now_ms();
    IF job.lease_expires_at_ms <= now_at THEN RETURN 'lost'; END IF;
    IF job.cancel_requested_at_ms IS NOT NULL THEN RETURN 'cancelled'; END IF;

    UPDATE mesh_jobs.jobs SET lease_expires_at_ms = greatest(lease_expires_at_ms, now_at + lease_ms)
    WHERE namespace = ns AND id = job_id;
    RETURN 'active';
END
$$;

CREATE OR REPLACE FUNCTION mesh_jobs.settle_state(
    ns text, job_id uuid, token uuid, outcome jsonb, recovering boolean
) RETURNS text
LANGUAGE plpgsql VOLATILE SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    job mesh_jobs.jobs%ROWTYPE;
    action text := outcome ->> 'action';
    reason text := outcome ->> 'reason';
    next_state text;
    delay_ms bigint := (outcome ->> 'delay_ms')::bigint;
    now_at bigint;
BEGIN
    SELECT * INTO job FROM mesh_jobs.jobs
    WHERE namespace = ns AND id = job_id AND lease_token = token FOR NO KEY UPDATE NOWAIT;
    IF NOT FOUND THEN RETURN NULL; END IF;
    now_at := mesh_jobs.now_ms();
    IF (job.lease_expires_at_ms <= now_at) <> recovering THEN RETURN NULL; END IF;

    IF job.cancel_requested_at_ms IS NOT NULL THEN
        next_state := 'cancelled';
        reason := 'cancelled by operator';
    ELSE
        next_state := CASE action
            WHEN 'complete' THEN 'completed'
            WHEN 'cancel' THEN 'cancelled'
            WHEN 'discard' THEN 'discarded'
            WHEN 'snooze' THEN 'scheduled'
            WHEN 'fail' THEN CASE WHEN job.attempt < job.max_attempts
                THEN 'retryable' ELSE 'discarded' END
            WHEN 'retry_after' THEN CASE WHEN job.attempt < job.max_attempts
                THEN 'retryable' ELSE 'discarded' END
        END;
    END IF;
    IF next_state = 'retryable' AND delay_ms IS NULL THEN
        delay_ms := least(job.backoff_max_ms::numeric,
            job.backoff_base_ms::numeric * power(2::numeric, job.attempt - 1))::bigint;
    END IF;

    UPDATE mesh_jobs.jobs SET
        state = next_state,
        attempt = CASE WHEN next_state = 'scheduled' THEN job.attempt - 1 ELSE job.attempt END,
        scheduled_at_ms = CASE WHEN next_state IN ('scheduled', 'retryable')
            THEN now_at + delay_ms ELSE job.scheduled_at_ms END,
        finished_at_ms = CASE WHEN next_state IN ('completed', 'cancelled', 'discarded') THEN now_at END,
        metadata = CASE WHEN next_state = 'scheduled' THEN job.metadata || jsonb_build_object(
            'snoozed', 1 + CASE WHEN jsonb_typeof(job.metadata -> 'snoozed') = 'number' THEN
                greatest(0, least(2147483646, trunc((job.metadata ->> 'snoozed')::numeric)))
                ELSE 0 END)
            ELSE job.metadata END,
        lease_token = NULL, lease_expires_at_ms = NULL, cancel_requested_at_ms = NULL,
        errors = CASE WHEN reason IS NULL THEN job.errors ELSE
            CASE WHEN jsonb_array_length(job.errors) >= 100 THEN job.errors - 0 ELSE job.errors END ||
            jsonb_build_array(jsonb_build_object('at_ms', now_at, 'attempt', job.attempt, 'reason', reason))
            END
    WHERE namespace = ns AND id = job_id AND lease_token = token
    RETURNING state INTO next_state;
    RETURN next_state;
END
$$;

DO $legacy$
BEGIN
    -- Standalone v1 bodies support workers with grants on only the legacy functions.
    IF to_regprocedure('mesh_jobs.renew(text,uuid,uuid,bigint)') IS NULL THEN
        CREATE FUNCTION mesh_jobs.renew(
            ns text, job_id uuid, token uuid, lease_ms bigint
        ) RETURNS text
        LANGUAGE plpgsql VOLATILE SET search_path = pg_catalog, pg_temp AS $body$
        DECLARE
            job mesh_jobs.jobs%ROWTYPE;
            now_at bigint;
        BEGIN
            SELECT * INTO job FROM mesh_jobs.jobs
            WHERE namespace = ns AND id = job_id AND lease_token = token FOR NO KEY UPDATE;
            IF NOT FOUND THEN RETURN 'lost'; END IF;
            now_at := mesh_jobs.now_ms();
            IF job.lease_expires_at_ms <= now_at THEN RETURN 'lost'; END IF;
            IF job.cancel_requested_at_ms IS NOT NULL THEN RETURN 'cancelled'; END IF;

            UPDATE mesh_jobs.jobs SET lease_expires_at_ms = greatest(lease_expires_at_ms, now_at + lease_ms)
            WHERE namespace = ns AND id = job_id;
            RETURN 'active';
        END
        $body$;
    END IF;

    IF to_regprocedure('mesh_jobs.settle(text,uuid,uuid,jsonb,boolean)') IS NULL THEN
        CREATE FUNCTION mesh_jobs.settle(
            ns text, job_id uuid, token uuid, outcome jsonb, recovering boolean
        ) RETURNS boolean
        LANGUAGE plpgsql VOLATILE SET search_path = pg_catalog, pg_temp AS $body$
        DECLARE
            job mesh_jobs.jobs%ROWTYPE;
            action text := outcome ->> 'action';
            reason text := outcome ->> 'reason';
            next_state text;
            delay_ms bigint := (outcome ->> 'delay_ms')::bigint;
            now_at bigint;
        BEGIN
            SELECT * INTO job FROM mesh_jobs.jobs
            WHERE namespace = ns AND id = job_id AND lease_token = token FOR NO KEY UPDATE;
            IF NOT FOUND THEN RETURN false; END IF;
            now_at := mesh_jobs.now_ms();
            IF (job.lease_expires_at_ms <= now_at) <> recovering THEN RETURN false; END IF;

            IF job.cancel_requested_at_ms IS NOT NULL THEN
                next_state := 'cancelled';
                reason := 'cancelled by operator';
            ELSE
                next_state := CASE action
                    WHEN 'complete' THEN 'completed'
                    WHEN 'cancel' THEN 'cancelled'
                    WHEN 'discard' THEN 'discarded'
                    WHEN 'snooze' THEN 'scheduled'
                    WHEN 'fail' THEN CASE WHEN job.attempt < job.max_attempts
                        THEN 'retryable' ELSE 'discarded' END
                    WHEN 'retry_after' THEN CASE WHEN job.attempt < job.max_attempts
                        THEN 'retryable' ELSE 'discarded' END
                END;
            END IF;
            IF next_state = 'retryable' AND delay_ms IS NULL THEN
                delay_ms := least(job.backoff_max_ms::numeric,
                    job.backoff_base_ms::numeric * power(2::numeric, job.attempt - 1))::bigint;
            END IF;

            UPDATE mesh_jobs.jobs SET
                state = next_state,
                attempt = CASE WHEN next_state = 'scheduled' THEN job.attempt - 1 ELSE job.attempt END,
                scheduled_at_ms = CASE WHEN next_state IN ('scheduled', 'retryable')
                    THEN now_at + delay_ms ELSE job.scheduled_at_ms END,
                finished_at_ms = CASE WHEN next_state IN ('completed', 'cancelled', 'discarded') THEN now_at END,
                metadata = CASE WHEN next_state = 'scheduled' THEN job.metadata || jsonb_build_object(
                    'snoozed', 1 + CASE WHEN jsonb_typeof(job.metadata -> 'snoozed') = 'number' THEN
                        greatest(0, least(2147483646, trunc((job.metadata ->> 'snoozed')::numeric)))
                        ELSE 0 END)
                    ELSE job.metadata END,
                lease_token = NULL, lease_expires_at_ms = NULL, cancel_requested_at_ms = NULL,
                errors = CASE WHEN reason IS NULL THEN job.errors ELSE
                    CASE WHEN jsonb_array_length(job.errors) >= 100 THEN job.errors - 0 ELSE job.errors END ||
                    jsonb_build_array(jsonb_build_object('at_ms', now_at, 'attempt', job.attempt, 'reason', reason))
                    END
            WHERE namespace = ns AND id = job_id;
            RETURN true;
        END
        $body$;
    END IF;
END
$legacy$;

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
    FROM due WHERE j.namespace = ns AND j.id = due.id;
    RETURN recovered;
END
$$;

CREATE OR REPLACE FUNCTION mesh_jobs.queue_control(
    ns text, queue_name text, operation text, capacity integer
) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    q mesh_jobs.queues%ROWTYPE;
BEGIN
    IF operation = 'configure' OR operation = 'start' THEN
        INSERT INTO mesh_jobs.queues(namespace, name, concurrency)
            VALUES (ns, queue_name, coalesce(capacity, 1))
        ON CONFLICT (namespace, name) DO UPDATE SET
            concurrency = coalesce(capacity, queues.concurrency),
            state = CASE WHEN operation = 'start' THEN 'active' ELSE queues.state END
        RETURNING * INTO q;
    ELSE
        UPDATE mesh_jobs.queues SET
            concurrency = CASE WHEN operation = 'scale' THEN capacity ELSE concurrency END,
            state = CASE operation
                WHEN 'pause' THEN CASE WHEN state = 'active' THEN 'paused' ELSE state END
                WHEN 'resume' THEN CASE WHEN state = 'paused' THEN 'active' ELSE state END
                WHEN 'stop' THEN 'stopped'
                ELSE state END
        WHERE namespace = ns AND name = queue_name RETURNING * INTO q;
    END IF;
    RETURN coalesce(mesh_jobs.queue_json(q), 'null'::jsonb);
END
$$;

CREATE OR REPLACE FUNCTION mesh_jobs.job_control(
    ns text, job_id uuid, operation text, absolute_ms bigint, delay_ms bigint
) RETURNS jsonb
LANGUAGE plpgsql VOLATILE SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    job mesh_jobs.jobs%ROWTYPE;
    now_at bigint;
    run_at bigint;
BEGIN
    SELECT * INTO job FROM mesh_jobs.jobs
    WHERE namespace = ns AND id = job_id FOR NO KEY UPDATE;
    IF NOT FOUND THEN RETURN 'null'::jsonb; END IF;

    now_at := mesh_jobs.now_ms();
    run_at := coalesce(absolute_ms, now_at + coalesce(delay_ms, 0));
    CASE operation
        WHEN 'cancel' THEN
            UPDATE mesh_jobs.jobs SET
                state = CASE WHEN job.state = 'executing' THEN 'executing' ELSE 'cancelled' END,
                cancel_requested_at_ms = CASE WHEN job.state = 'executing' THEN now_at END,
                finished_at_ms = CASE WHEN job.state <> 'executing' THEN now_at END
            WHERE namespace = ns AND id = job_id AND job.finished_at_ms IS NULL
                AND job.cancel_requested_at_ms IS NULL;
        WHEN 'retry' THEN
            UPDATE mesh_jobs.jobs SET state = 'available', run = job.run + 1, attempt = 0,
                attempted_at_ms = NULL, finished_at_ms = NULL, scheduled_at_ms = now_at
            WHERE namespace = ns AND id = job_id AND job.finished_at_ms IS NOT NULL;
        WHEN 'discard' THEN
            UPDATE mesh_jobs.jobs SET state = 'discarded', finished_at_ms = now_at
            WHERE namespace = ns AND id = job_id AND job.finished_at_ms IS NULL AND job.state <> 'executing';
        WHEN 'reschedule' THEN
            UPDATE mesh_jobs.jobs SET scheduled_at_ms = run_at, state = CASE
                WHEN job.state = 'suspended' THEN 'suspended'
                WHEN run_at > now_at THEN 'scheduled' ELSE 'available' END
            WHERE namespace = ns AND id = job_id AND job.finished_at_ms IS NULL AND job.state <> 'executing';
        WHEN 'suspend' THEN
            UPDATE mesh_jobs.jobs SET state = 'suspended'
            WHERE namespace = ns AND id = job_id AND job.finished_at_ms IS NULL AND job.state <> 'executing';
        WHEN 'resume_job' THEN
            UPDATE mesh_jobs.jobs SET state = CASE
                WHEN job.scheduled_at_ms > now_at THEN 'scheduled' ELSE 'available' END
            WHERE namespace = ns AND id = job_id AND job.state = 'suspended';
        WHEN 'delete' THEN
            DELETE FROM mesh_jobs.jobs WHERE namespace = ns AND id = job_id AND job.state <> 'executing';
            RETURN jsonb_build_object('id', job_id, 'deleted', FOUND);
    END CASE;

    SELECT * INTO job FROM mesh_jobs.jobs WHERE namespace = ns AND id = job_id;
    RETURN mesh_jobs.job_json(job);
END
$$;
