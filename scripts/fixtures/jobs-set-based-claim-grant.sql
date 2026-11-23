-- Apply as the Jobs object owner after Jobs.migrate on the new runtime.
-- Set worker with psql -v to the existing scoped batch-worker role.
GRANT EXECUTE ON FUNCTION mesh_jobs.claim_batch_set_v1(text,text,jsonb,bigint,integer,bigint)
    TO :"worker";
