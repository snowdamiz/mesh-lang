BEGIN;
CREATE SCHEMA fence_demo AUTHORIZATION fence_migrator;

CREATE TABLE fence_demo.effects (
    job_id uuid NOT NULL,
    job_run bigint NOT NULL,
    stage integer NOT NULL,
    xid xid8 NOT NULL DEFAULT pg_catalog.pg_current_xact_id(),
    backend integer NOT NULL DEFAULT pg_catalog.pg_backend_pid(),
    PRIMARY KEY (job_id, job_run, stage)
);
ALTER TABLE fence_demo.effects OWNER TO fence_migrator;

SET LOCAL ROLE fence_migrator;
GRANT USAGE ON SCHEMA mesh_jobs_effects TO fence_app_owner;
GRANT EXECUTE ON FUNCTION mesh_jobs_effects.fence_v1(text, uuid, bigint, integer, uuid)
    TO fence_app_owner;

GRANT USAGE ON SCHEMA fence_demo TO fence_app_owner, fence_caller;
GRANT SELECT, INSERT ON fence_demo.effects TO fence_app_owner;
GRANT CREATE ON SCHEMA fence_demo TO fence_app_owner;

SET LOCAL ROLE fence_app_owner;
CREATE FUNCTION fence_demo.write_first(uuid, bigint, integer, uuid) RETURNS void
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
BEGIN
    PERFORM mesh_jobs_effects.fence_v1('effect-fence-proof', $1, $2, $3, $4);
    INSERT INTO fence_demo.effects (job_id, job_run, stage) VALUES ($1, $2, 1)
    ON CONFLICT (job_id, job_run, stage) DO NOTHING;
END
$$;
CREATE FUNCTION fence_demo.write_second(uuid, bigint, integer, uuid) RETURNS void
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
BEGIN
    PERFORM mesh_jobs_effects.fence_v1('effect-fence-proof', $1, $2, $3, $4);
    INSERT INTO fence_demo.effects (job_id, job_run, stage) VALUES ($1, $2, 2)
    ON CONFLICT (job_id, job_run, stage) DO NOTHING;
END
$$;

REVOKE ALL ON FUNCTION fence_demo.write_first(uuid, bigint, integer, uuid),
    fence_demo.write_second(uuid, bigint, integer, uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION fence_demo.write_first(uuid, bigint, integer, uuid),
    fence_demo.write_second(uuid, bigint, integer, uuid) TO fence_caller;

SET LOCAL ROLE fence_migrator;
REVOKE CREATE ON SCHEMA fence_demo FROM fence_app_owner;
COMMIT;
