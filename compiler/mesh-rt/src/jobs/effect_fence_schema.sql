-- Apply once as the Jobs object owner after the normal Jobs migration.
BEGIN ISOLATION LEVEL READ COMMITTED;
SET LOCAL search_path = pg_catalog, pg_temp;
SELECT pg_catalog.pg_advisory_xact_lock(1782495102, 1);

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT FROM pg_catalog.pg_class
        WHERE oid = pg_catalog.to_regclass('mesh_jobs.jobs')
            AND pg_catalog.pg_get_userbyid(relowner) = current_user
    ) OR NOT EXISTS (
        SELECT FROM pg_catalog.pg_proc
        WHERE oid = pg_catalog.to_regprocedure('mesh_jobs.now_ms()')
            AND pg_catalog.pg_get_userbyid(proowner) = current_user
    ) THEN
        RAISE EXCEPTION 'install the Jobs effect fence as the Jobs object owner';
    END IF;
END
$$;

-- Isolate schema USAGE from the legacy functions in mesh_jobs.
CREATE SCHEMA mesh_jobs_effects;
REVOKE ALL ON SCHEMA mesh_jobs_effects FROM PUBLIC;

CREATE FUNCTION mesh_jobs_effects.fence_v1(
    ns text, job_id uuid, job_run bigint, job_attempt integer, token uuid
) RETURNS void
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    expires_at bigint;
BEGIN
    IF current_setting('transaction_isolation') <> 'read committed' THEN
        RAISE EXCEPTION 'mesh jobs effect fence requires READ COMMITTED' USING ERRCODE = '25000';
    END IF;

    SELECT j.lease_expires_at_ms INTO expires_at
    FROM mesh_jobs.jobs AS j
    WHERE j.namespace = ns AND j.id = job_id AND j.run = job_run AND j.attempt = job_attempt
        AND j.lease_token = token AND j.state = 'executing' AND j.cancel_requested_at_ms IS NULL
    FOR NO KEY UPDATE OF j NOWAIT;

    -- The lock protects the matching row through the application's commit or rollback.
    IF NOT FOUND OR (expires_at > mesh_jobs.now_ms()) IS NOT TRUE THEN
        RAISE EXCEPTION 'mesh jobs effect fence denied' USING ERRCODE = '55000';
    END IF;
END
$$;
REVOKE ALL ON FUNCTION mesh_jobs_effects.fence_v1(text, uuid, bigint, integer, uuid) FROM PUBLIC;

-- Fail the installation if the owner's custom default ACLs would opt other roles in.
DO $$
BEGIN
    IF EXISTS (
        SELECT FROM pg_catalog.pg_namespace AS n
        CROSS JOIN LATERAL pg_catalog.aclexplode(n.nspacl) AS acl
        WHERE n.nspname = 'mesh_jobs_effects' AND acl.grantee <> n.nspowner
        UNION ALL
        SELECT FROM pg_catalog.pg_proc AS p
        CROSS JOIN LATERAL pg_catalog.aclexplode(p.proacl) AS acl
        WHERE p.oid = 'mesh_jobs_effects.fence_v1(text,uuid,bigint,integer,uuid)'::regprocedure
            AND acl.grantee <> p.proowner
    ) THEN
        RAISE EXCEPTION 'Jobs effect fence requires owner-only installation privileges';
    END IF;
END
$$;

COMMIT;
