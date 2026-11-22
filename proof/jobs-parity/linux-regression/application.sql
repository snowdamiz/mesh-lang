BEGIN;
CREATE SCHEMA compiled_demo;
ALTER TABLE public.mesh_jobs_parity_attempts
  ADD COLUMN job_id uuid NOT NULL,
  ADD COLUMN job_run bigint NOT NULL,
  ADD COLUMN xid xid8 NOT NULL DEFAULT pg_current_xact_id(),
  ADD COLUMN backend integer NOT NULL DEFAULT pg_backend_pid(),
  ADD UNIQUE (namespace, job_id, job_run);
ALTER TABLE public.mesh_jobs_parity_effects
  ADD COLUMN job_id uuid NOT NULL,
  ADD COLUMN job_run bigint NOT NULL,
  ADD COLUMN attempt integer NOT NULL,
  ADD COLUMN xid xid8 NOT NULL DEFAULT pg_current_xact_id(),
  ADD COLUMN backend integer NOT NULL DEFAULT pg_backend_pid(),
  ADD UNIQUE (namespace, job_id, job_run);
GRANT USAGE, CREATE ON SCHEMA compiled_demo TO compiled_definer;
GRANT USAGE ON SCHEMA compiled_demo TO compiled_caller;
GRANT USAGE ON SCHEMA mesh_jobs_effects TO compiled_definer;
GRANT EXECUTE ON FUNCTION mesh_jobs_effects.fence_v1(text, uuid, bigint, integer, uuid)
    TO compiled_definer;
GRANT INSERT ON public.mesh_jobs_parity_attempts, public.mesh_jobs_parity_effects TO compiled_definer;
GRANT SELECT (namespace, job_id, job_run)
    ON public.mesh_jobs_parity_attempts, public.mesh_jobs_parity_effects TO compiled_definer;

SET LOCAL ROLE compiled_definer;
CREATE FUNCTION compiled_demo.first(ns text, id uuid, run bigint, attempt_no integer, token uuid,
    key text, entered_us bigint, worker bigint, witness boolean)
RETURNS boolean LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
BEGIN
    IF witness AND EXISTS (SELECT 1 FROM pg_locks
            WHERE pid = pg_backend_pid() AND locktype = 'advisory') THEN
        RAISE EXCEPTION 'previous checkout leaked its advisory lock';
    END IF;
    PERFORM mesh_jobs_effects.fence_v1(ns, id, run, attempt_no, token);
    IF witness THEN
        -- Distinct session locks witness exclusivity without blocking any peer.
        PERFORM pg_advisory_lock(1782495102, worker::integer);
    END IF;
    INSERT INTO public.mesh_jobs_parity_attempts
      (engine, namespace, job_key, effect_key, attempt, entered_at, worker_pid, job_id, job_run)
    VALUES ('mesh', ns, key, id::text || '/' || run, attempt_no,
      to_timestamp((entered_us / 1000000)::double precision)
        + (entered_us % 1000000) * interval '1 microsecond',
      worker::integer, id, run)
    ON CONFLICT (namespace, job_id, job_run) DO NOTHING;
    RETURN FOUND;
END
$$;

CREATE FUNCTION compiled_demo.second(ns text, id uuid, run bigint, attempt_no integer, token uuid,
    key text, witness boolean)
RETURNS boolean LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
BEGIN
    IF witness AND EXISTS (SELECT 1 FROM pg_locks
            WHERE pid = pg_backend_pid() AND locktype = 'advisory') THEN
        RAISE EXCEPTION 'full reset failed to remove the previous session advisory lock';
    END IF;
    PERFORM mesh_jobs_effects.fence_v1(ns, id, run, attempt_no, token);
    INSERT INTO public.mesh_jobs_parity_effects
      (engine, namespace, job_key, effect_key, effected_at, job_id, job_run, attempt)
    VALUES ('mesh', ns, key, id::text || '/' || run, clock_timestamp(), id, run, attempt_no)
    ON CONFLICT (namespace, job_id, job_run) DO NOTHING;
    RETURN FOUND;
END
$$;
REVOKE ALL ON ALL FUNCTIONS IN SCHEMA compiled_demo FROM PUBLIC;
GRANT EXECUTE ON ALL FUNCTIONS IN SCHEMA compiled_demo TO compiled_caller;
COMMIT;
