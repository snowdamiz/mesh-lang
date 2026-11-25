# Two ordinary SQL calls, each committed by PostgreSQL's autocommit.
pub fn record_attempt(pool :: PoolHandle, ns :: String, key :: String, identity :: String,
                      generation :: String, attempt :: Int, entered :: Int, pid :: Int) -> Int!String do
  Pool.execute(pool, """
    INSERT INTO public.mesh_jobs_parity_attempts
      (engine, namespace, job_key, effect_key, job_id, job_generation, attempt, entered_at, worker_pid)
    VALUES ('mesh', $1, $2, $2, $3::uuid, $4, $5::integer,
      to_timestamp(($6::bigint / 1000000)::double precision)
        + ($6::bigint % 1000000) * interval '1 microsecond', $7::integer)
    ON CONFLICT (namespace, job_id, job_generation, attempt) DO NOTHING
    """, [ns, key, identity, generation, String.from(attempt), String.from(entered), String.from(pid)])
end

pub fn record_effect(pool :: PoolHandle, ns :: String, key :: String) -> Int!String do
  Pool.execute(pool, """
    INSERT INTO public.mesh_jobs_parity_effects
      (engine, namespace, job_key, effect_key, effected_at)
    VALUES ('mesh', $1, $2, $2, clock_timestamp())
    ON CONFLICT (engine, namespace, effect_key) DO NOTHING
    """, [ns, key])
end
