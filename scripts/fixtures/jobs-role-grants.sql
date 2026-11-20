-- Apply as the Jobs object owner after Jobs.migrate.
-- Set producer, worker, operator, and inspector to existing role names with psql -v.
REVOKE EXECUTE ON ALL FUNCTIONS IN SCHEMA mesh_jobs FROM PUBLIC;
GRANT USAGE ON SCHEMA mesh_jobs TO :"producer", :"worker", :"operator", :"inspector";

GRANT SELECT, INSERT (namespace), UPDATE (fence) ON mesh_jobs.admissions TO :"producer";
GRANT SELECT, INSERT (namespace, name) ON mesh_jobs.queues TO :"producer";
GRANT SELECT, INSERT ON mesh_jobs.receipts TO :"producer";
GRANT SELECT ON mesh_jobs.jobs TO :"producer";
GRANT INSERT (namespace, worker, version, queue, args, metadata, tags, state, priority,
    max_attempts, timeout_ms, backoff_base_ms, backoff_max_ms, inserted_at_ms, scheduled_at_ms,
    unique_key) ON mesh_jobs.jobs TO :"producer";
GRANT UPDATE (args, metadata, priority, tags, max_attempts, queue, worker, version,
    scheduled_at_ms, state) ON mesh_jobs.jobs TO :"producer";
GRANT EXECUTE ON FUNCTION mesh_jobs.enqueue(text,jsonb,text,text),
    mesh_jobs.unique_values(jsonb,jsonb) TO :"producer";

GRANT SELECT ON mesh_jobs.schema_version, mesh_jobs.jobs, mesh_jobs.queues TO :"worker";
-- A row-locking SELECT needs UPDATE on at least one column.
GRANT UPDATE (concurrency) ON mesh_jobs.queues TO :"worker";
GRANT UPDATE (state, attempt, attempted_at_ms, scheduled_at_ms, metadata, finished_at_ms,
    lease_token, lease_expires_at_ms, cancel_requested_at_ms, errors)
    ON mesh_jobs.jobs TO :"worker";
GRANT SELECT, INSERT (namespace, id, row_version, row_location, encoding, context_bytes),
    UPDATE (row_version, row_location, encoding, context_bytes)
    ON mesh_jobs.batch_context_blocks TO :"worker";
GRANT EXECUTE ON FUNCTION mesh_jobs.claim(text,text,jsonb,bigint),
    mesh_jobs.claim_batch_v1(text,text,jsonb,bigint,integer,bigint),
    mesh_jobs.claim_drain_v1(text,text,jsonb,bigint,bigint,jsonb),
    mesh_jobs.renew(text,uuid,uuid,bigint), mesh_jobs.try_renew(text,uuid,uuid,bigint),
    mesh_jobs.settle(text,uuid,uuid,jsonb,boolean), mesh_jobs.settle_state(text,uuid,uuid,jsonb,boolean),
    mesh_jobs.recover(text,integer,jsonb) TO :"worker";

GRANT SELECT, INSERT ON mesh_jobs.queues TO :"operator";
GRANT UPDATE (concurrency, state) ON mesh_jobs.queues TO :"operator";
GRANT SELECT, DELETE ON mesh_jobs.jobs TO :"operator";
GRANT UPDATE (state, run, attempt, attempted_at_ms, scheduled_at_ms, cancel_requested_at_ms,
    finished_at_ms, args, metadata, tags, priority, timeout_ms) ON mesh_jobs.jobs TO :"operator";
GRANT SELECT, INSERT, DELETE ON mesh_jobs.cron_schedules TO :"operator";
GRANT UPDATE (expression, timezone, job, misfire, lookback_minutes, cursor_minute)
    ON mesh_jobs.cron_schedules TO :"operator";
GRANT EXECUTE ON FUNCTION mesh_jobs.queue_control(text,text,text,integer),
    mesh_jobs.set_namespace_paused(text,boolean),
    mesh_jobs.job_control(text,uuid,text,bigint,bigint), mesh_jobs.queue_json(mesh_jobs.queues),
    mesh_jobs.job_json(mesh_jobs.jobs) TO :"operator";

GRANT SELECT ON mesh_jobs.job_summaries, mesh_jobs.queue_summaries TO :"inspector", :"operator";
GRANT SELECT (namespace, name, state) ON mesh_jobs.queues TO :"inspector";
GRANT EXECUTE ON FUNCTION mesh_jobs.now_ms(), mesh_jobs.effect_key(mesh_jobs.jobs)
    TO :"producer", :"worker", :"operator";
