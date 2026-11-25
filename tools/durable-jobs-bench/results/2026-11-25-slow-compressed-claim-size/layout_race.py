"""Replace effect_key while a real claim is waiting for its queue row."""
import subprocess


def run(command, claim_function, arguments, cursor_function):
    def sql(statement):
        return subprocess.check_output(command, input=statement, text=True).strip()

    original = sql("SELECT pg_get_functiondef('mesh_jobs.effect_key(mesh_jobs.jobs)'::regprocedure)")
    alternate = """CREATE OR REPLACE FUNCTION mesh_jobs.effect_key(j mesh_jobs.jobs)
        RETURNS text LANGUAGE SQL IMMUTABLE STRICT AS $$SELECT j.id::text || '/2'$$;"""
    # The holder waits for PostgreSQL's actual lock dependency, then
    # publishes the helper and releases the queue row in the same commit.
    release = """
        DO $$ BEGIN
            FOR i IN 1..5000 LOOP
                PERFORM pg_stat_clear_snapshot();
                IF EXISTS (SELECT FROM pg_stat_activity
                    WHERE application_name = 'raw_size_pending'
                    AND pg_backend_pid() = ANY(pg_blocking_pids(pid))) THEN RETURN; END IF;
                PERFORM pg_sleep(0.001);
            END LOOP;
            RAISE EXCEPTION 'claim never blocked on the holder';
        END $$;
    """ + alternate + "COMMIT;"

    try:
        with subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                              text=True) as holder:
            holder.stdin.write("BEGIN; SELECT 1 FROM mesh_jobs.queues "
                               "WHERE namespace='n' AND name='q' FOR NO KEY UPDATE;\n")
            holder.stdin.flush()
            if holder.stdout.readline().strip() != "1":
                raise RuntimeError("could not take queue row")
            query = f"""BEGIN; SET LOCAL application_name = 'raw_size_pending';
                SELECT coalesce(context::jsonb->>'effect_key',status)
                FROM {claim_function}{arguments} LIMIT 1; ROLLBACK;"""
            with subprocess.Popen([*command, "-c", query], stdout=subprocess.PIPE,
                                  text=True) as claimant:
                holder.communicate(release, timeout=15)
                reply, _ = claimant.communicate(timeout=5)
                if holder.returncode or claimant.returncode:
                    raise RuntimeError(f"queue holder or claimant failed: {holder.returncode}, {claimant.returncode}")
        cursor = sql(f"BEGIN; SELECT context::jsonb->>'effect_key' FROM {cursor_function}{arguments} "
                     "LIMIT 1; ROLLBACK;")
        return {"pending_claim_reply": reply.strip(), "subsequent_cursor_key": cursor}
    finally:
        sql(original)
