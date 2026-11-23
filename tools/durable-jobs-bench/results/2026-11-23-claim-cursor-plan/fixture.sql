SET ROLE parity_subject;
INSERT INTO mesh_jobs.queues (namespace, name, concurrency) VALUES ('probe', 'default', :capacity);
CREATE TEMP TABLE fixture_args (n integer, args jsonb);
\i :payload_file

INSERT INTO mesh_jobs.jobs (namespace, id, worker, version, queue, args, metadata, tags,
    state, priority, max_attempts, timeout_ms, backoff_base_ms, backoff_max_ms,
    inserted_at_ms, scheduled_at_ms, attempt)
SELECT 'probe', lpad(to_hex(n), 32, '0')::uuid,
    CASE WHEN :mixed::boolean AND n IN (12,14,20) THEN 'alternate'
         WHEN :mixed::boolean AND n = 13 THEN 'unregistered' ELSE 'parity' END,
    CASE WHEN :mixed::boolean AND n IN (12,14,18,20) THEN 2 ELSE 1 END,
    'default', args, '{}', '[]',
    CASE WHEN :is_sparse::boolean AND n > 11 AND (NOT :mixed::boolean OR n > 21)
         THEN 'scheduled' ELSE 'available' END,
    0, 3, 60000, 1000, 1000, 1700000000000 + n,
    CASE WHEN :is_sparse::boolean AND n > 11 AND (NOT :mixed::boolean OR n > 21)
         THEN 1700000010000 + n ELSE 1700000000000 + n END,
    CASE WHEN :mixed::boolean AND n = 16 THEN 3 ELSE 0 END
FROM fixture_args ORDER BY n;

-- Preserve heap/index churn from prior claiming and settling, without measuring
-- synthetic handlers. At late stages, keep a tail of ready jobs.
UPDATE mesh_jobs.jobs SET state = 'executing', attempt = 1,
    lease_token = gen_random_uuid(), attempted_at_ms = 1700000000500,
    lease_expires_at_ms = 1700000060500 WHERE position <= :done + :running;
UPDATE mesh_jobs.jobs SET state = 'completed', lease_token = NULL,
    lease_expires_at_ms = NULL, finished_at_ms = 1700000000510 WHERE position <= :done;

INSERT INTO mesh_jobs.batch_context_blocks (namespace, id, row_version, row_location, encoding, context_bytes)
SELECT namespace, id, xmin, ctid, 'UTF8', 16777217
FROM mesh_jobs.jobs WHERE :mixed::boolean AND position = 17;

ANALYZE mesh_jobs.jobs;
ANALYZE mesh_jobs.queues;
ANALYZE mesh_jobs.batch_context_blocks;
