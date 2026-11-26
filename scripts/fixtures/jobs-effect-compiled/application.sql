BEGIN;
CREATE SCHEMA compiled_demo;
CREATE TABLE compiled_demo.effects (
    job_id uuid NOT NULL,
    job_run bigint NOT NULL,
    job_attempt integer NOT NULL,
    stage integer NOT NULL,
    label text NOT NULL,
    xid xid8 NOT NULL DEFAULT pg_catalog.pg_current_xact_id(),
    backend integer NOT NULL DEFAULT pg_catalog.pg_backend_pid(),
    PRIMARY KEY (job_id, job_run, stage)
);
GRANT USAGE ON SCHEMA mesh_jobs_effects TO compiled_definer;
GRANT EXECUTE ON FUNCTION mesh_jobs_effects.fence_v1(text, uuid, bigint, integer, uuid)
    TO compiled_definer;
GRANT USAGE, CREATE ON SCHEMA compiled_demo TO compiled_definer;
GRANT USAGE ON SCHEMA compiled_demo TO compiled_caller, compiled_producer;
GRANT SELECT, INSERT ON compiled_demo.effects TO compiled_definer;
GRANT SELECT ON compiled_demo.effects TO compiled_producer;

SET LOCAL ROLE compiled_definer;

CREATE FUNCTION compiled_demo.first(ns text, job_id uuid, job_run bigint, job_attempt integer, token uuid, label text)
RETURNS boolean LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
BEGIN
    IF ns <> 'compiled-effect-proof' THEN
        RAISE EXCEPTION 'unexpected application namespace';
    END IF;
    PERFORM mesh_jobs_effects.fence_v1(ns, job_id, job_run, job_attempt, token);
    -- This is a session advisory lock. Only reset (or closing the session)
    -- can remove it after the transaction commits.
    PERFORM pg_catalog.pg_advisory_lock(1782495102, 421);
    INSERT INTO compiled_demo.effects VALUES (job_id, job_run, job_attempt, 1, label)
        ON CONFLICT DO NOTHING;
    RETURN FOUND;
END
$$;

CREATE FUNCTION compiled_demo.second(ns text, job_id uuid, job_run bigint, job_attempt integer, token uuid, label text)
RETURNS boolean LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
BEGIN
    IF ns <> 'compiled-effect-proof' THEN
        RAISE EXCEPTION 'unexpected application namespace';
    END IF;
    PERFORM mesh_jobs_effects.fence_v1(ns, job_id, job_run, job_attempt, token);
    IF EXISTS (SELECT 1 FROM pg_catalog.pg_locks
               WHERE pid = pg_catalog.pg_backend_pid() AND locktype = 'advisory') THEN
        RAISE EXCEPTION 'warm session retained the first operation advisory lock';
    END IF;
    IF label = 'stage disconnect' AND job_attempt = 1 THEN
        PERFORM pg_catalog.pg_advisory_xact_lock(1782495102, 422);
        PERFORM pg_catalog.pg_sleep(2);
    END IF;
    INSERT INTO compiled_demo.effects VALUES (job_id, job_run, job_attempt, 2, label)
        ON CONFLICT DO NOTHING;
    RETURN FOUND;
END
$$;
REVOKE ALL ON ALL FUNCTIONS IN SCHEMA compiled_demo FROM PUBLIC;
GRANT EXECUTE ON ALL FUNCTIONS IN SCHEMA compiled_demo TO compiled_caller;
COMMIT;
