SET ROLE parity_subject;
INSERT INTO mesh_jobs.queues(namespace, name, concurrency)
VALUES ('nonfit', 'default', 16), ('markers', 'default', 16);
INSERT INTO mesh_jobs.jobs (namespace, id, worker, version, queue, args, metadata, tags,
    state, priority, max_attempts, timeout_ms, backoff_base_ms, backoff_max_ms,
    inserted_at_ms, scheduled_at_ms)
SELECT ns, lpad(to_hex(n), 32, '0')::uuid, 'parity', 1, 'default',
    jsonb_build_object('payload', repeat('x', bytes)), '{}', '[]', 'available', 0,
    3, 60000, 1000, 1000, 1700000000000, 1700000000000
FROM (VALUES ('nonfit',1,16775700), ('nonfit',2,16775700), ('nonfit',3,8192),
    ('nonfit',4,32), ('markers',1,16777216), ('markers',2,16777216),
    ('markers',3,16777216), ('markers',4,16777216), ('markers',5,32)) AS args(ns,n,bytes)
ORDER BY ns,n;
ANALYZE mesh_jobs.jobs;
ANALYZE mesh_jobs.queues;
ANALYZE mesh_jobs.batch_context_blocks;
