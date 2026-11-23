WITH input AS (
    SELECT j.*, j.xmin AS settlement_xid, convert_from(j.job, 'UTF8')::jsonb AS args
    FROM apalis.jobs j
)
SELECT coalesce(jsonb_agg(to_jsonb(result) ORDER BY result.job_key), '[]'::jsonb)
FROM (
    SELECT j.id, j.job_type, j.args->>'key' AS job_key,
           encode(sha256(convert_to(j.args->>'payload', 'UTF8')), 'hex') AS payload_sha256,
           (j.args->>'hold_ms')::integer AS hold_ms,
           (j.args->>'fail_first')::boolean AS fail_first,
           CASE WHEN j.status = 'Done' THEN 'completed' ELSE lower(j.status) END AS state,
           j.status AS native_state, j.attempts AS attempt, j.max_attempts, j.last_result,
           j.lock_by, (extract(epoch FROM j.lock_at)*1000000)::bigint AS last_lock_us,
           a.attempts, e.effects,
           a.entered_us, a.attempt_xid, e.effect_xid, j.settlement_xid::text AS settled_xid,
           a.attempt_committed_us, e.effect_committed_us,
           (extract(epoch FROM pg_xact_commit_timestamp(j.settlement_xid))*1000000)::bigint AS settled_us
    FROM input j
    CROSS JOIN LATERAL (
        SELECT count(*) AS attempts, min((extract(epoch FROM entered_at)*1000000)::bigint) AS entered_us,
               min(xmin::text) FILTER (WHERE attempt = 1) AS attempt_xid,
               min((extract(epoch FROM pg_xact_commit_timestamp(xmin))*1000000)::bigint)
                   FILTER (WHERE attempt = 1) AS attempt_committed_us
        FROM public.mesh_jobs_parity_attempts
        WHERE (engine, namespace, job_key) = ('apalis', 'parity', j.args->>'key')
          AND job_id = j.id
    ) a
    CROSS JOIN LATERAL (
        SELECT count(*) AS effects, min(xmin::text) AS effect_xid,
               min((extract(epoch FROM pg_xact_commit_timestamp(xmin))*1000000)::bigint) AS effect_committed_us
        FROM public.mesh_jobs_parity_effects
        WHERE (engine, namespace, effect_key) = ('apalis', 'parity', j.args->>'key')
          AND job_id = j.id
    ) e
) result
