"""Replace effect_key while a real claim is waiting for its queue row."""
import subprocess


def receipt_lock(command, stock_function, candidate_function, cursor_function, arguments):
    """Keep an unrelated receipt-table SHARE lock through both claim routes."""
    def query(statement):
        return subprocess.check_output(command, input=statement, text=True).strip()

    with subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True) as holder:
        try:
            holder.stdin.write("BEGIN; LOCK TABLE ONLY mesh_jobs.receipts IN SHARE MODE; SELECT 'held';\n")
            holder.stdin.flush()
            if holder.stdout.readline().strip() != "held":
                raise RuntimeError("receipt-table holder did not acquire its lock")
            fallback = query(f"BEGIN; SELECT status FROM {stock_function}{arguments}; ROLLBACK;")
            granted = query(f"BEGIN; SELECT count(*) FROM {cursor_function}{arguments}; ROLLBACK;")
            # Use PL/pgSQL's structured diagnostics so a different SQL error fails the proof.
            outcome = query(f"""BEGIN; CREATE TEMP TABLE proof_lock_error(code text, detail text);
                DO $$ DECLARE ctx text; BEGIN
                    PERFORM * FROM {candidate_function}{arguments};
                EXCEPTION WHEN lock_not_available THEN
                    GET STACKED DIAGNOSTICS ctx = PG_EXCEPTION_CONTEXT;
                    INSERT INTO proof_lock_error VALUES (SQLSTATE, ctx);
                END $$;
                SELECT code, position('ONLY mesh_jobs.receipts' IN detail)>0
                    FROM proof_lock_error; ROLLBACK;""")
            if (fallback, granted, outcome) != ("fallback", "16", "55P03|t"):
                raise AssertionError((fallback, granted, outcome))
            return {"stock_status": fallback, "cursor_grants": int(granted),
                    "postlock_guard_sqlstate": "55P03", "failed_in_layout_table_lock": True}
        finally:
            holder.communicate("ROLLBACK;", timeout=5)


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
