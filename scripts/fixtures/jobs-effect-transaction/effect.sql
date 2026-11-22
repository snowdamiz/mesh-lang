CREATE SCHEMA jobs_effect_proof;

CREATE TABLE jobs_effect_proof.effects (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    job_id uuid NOT NULL
);

CREATE TABLE jobs_effect_proof.receipts (
    job_id uuid NOT NULL,
    run bigint NOT NULL,
    operation_id text NOT NULL,
    effect_id bigint NOT NULL REFERENCES jobs_effect_proof.effects(id),
    PRIMARY KEY (job_id, run, operation_id)
);

CREATE FUNCTION jobs_effect_proof.record_demo_write(
    job_id uuid, job_run bigint, job_attempt integer, token uuid, operation_id text
) RETURNS text
LANGUAGE plpgsql VOLATILE STRICT SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    job mesh_jobs.jobs%ROWTYPE;
    effect_id bigint;
BEGIN
    BEGIN
        SELECT j.* INTO job FROM mesh_jobs.jobs j
        WHERE j.namespace = 'effect-transaction-proof' AND j.id = job_id
        FOR NO KEY UPDATE OF j NOWAIT;
        IF NOT FOUND THEN RETURN 'lost'; END IF;
    EXCEPTION WHEN lock_not_available THEN
        RETURN 'busy';
    END;

    IF job.state IS DISTINCT FROM 'executing'
        OR job.run IS DISTINCT FROM job_run
        OR job.attempt IS DISTINCT FROM job_attempt
        OR job.lease_token IS DISTINCT FROM token
    THEN
        RETURN 'lost';
    END IF;

    SELECT r.effect_id INTO effect_id FROM jobs_effect_proof.receipts r
    WHERE r.job_id = job.id AND r.run = job.run
        AND r.operation_id = record_demo_write.operation_id;

    -- The row lock stays held until the caller commits or rolls back.
    IF job.cancel_requested_at_ms IS NOT NULL THEN RETURN 'cancelled'; END IF;
    IF job.lease_expires_at_ms <= mesh_jobs.now_ms() THEN RETURN 'expired'; END IF;
    IF effect_id IS NOT NULL THEN RETURN 'confirmed:' || effect_id; END IF;

    INSERT INTO jobs_effect_proof.effects AS e (job_id) VALUES (job.id)
    RETURNING e.id INTO effect_id;
    INSERT INTO jobs_effect_proof.receipts VALUES (job.id, job.run, operation_id, effect_id);
    RETURN 'applied:' || effect_id;
END
$$;
