"""Run through scripts/verify-postgres-jobs.sh --effect-transaction."""

import json
import os
import select
import subprocess
import sys
from pathlib import Path
from uuid import UUID


NAMESPACE = "effect-transaction-proof"
SESSION_LIMITS = """
SET lock_timeout = '250ms';
SET statement_timeout = '5s';
SET idle_in_transaction_session_timeout = '20s';
"""


def psql():
    return ["psql", "-XqAtw", "-v", "ON_ERROR_STOP=1", os.environ["MESH_TEST_DATABASE_URL"]]


def sql(query):
    try:
        result = subprocess.run(
            psql(), input=SESSION_LIMITS + query + "\n", stdout=subprocess.PIPE,
            text=True, timeout=12,
        )
    except subprocess.TimeoutExpired:
        raise RuntimeError("psql exceeded the fixture timeout") from None
    if result.returncode:
        raise RuntimeError(f"psql exited with status {result.returncode}")
    return result.stdout.strip()


def expect(label, actual, expected):
    if actual != expected:
        raise AssertionError(f"{label}: expected {expected!r}, got {actual!r}")


def observe(job_id):
    # Every observation uses a new backend and sees only committed rows.
    return json.loads(sql(f"""
        SELECT jsonb_build_object(
            'job', (SELECT to_jsonb(j) FROM mesh_jobs.jobs j
                WHERE j.namespace = '{NAMESPACE}' AND j.id = '{job_id}'),
            'effects', (SELECT coalesce(jsonb_agg(to_jsonb(e) ORDER BY e.id), '[]')
                FROM jobs_effect_proof.effects e),
            'receipts', (SELECT coalesce(jsonb_agg(to_jsonb(r) ORDER BY r.operation_id), '[]')
                FROM jobs_effect_proof.receipts r),
            'write_calls', (SELECT CASE WHEN is_called THEN last_value ELSE 0 END
                FROM jobs_effect_proof.effects_id_seq)
        );
    """))


def claim(job_id, lease_ms):
    claimed_id, token = sql(f"""
        SELECT id, token FROM mesh_jobs.claim(
            '{NAMESPACE}', 'default', '[{{"worker":"proof","version":1}}]', {lease_ms});
    """).split("|")
    expect("claimed job ID", UUID(claimed_id), job_id)
    return UUID(token)


def expect_claim(observed, job_id, token, attempt):
    job = observed["job"]
    expect(
        "attempt identity",
        (job["id"], job["state"], job["run"], job["attempt"],
         job["lease_token"], job["cancel_requested_at_ms"]),
        (str(job_id), "executing", 1, attempt, str(token), None),
    )


def record(job_id, run, attempt, token, operation):
    return f"""
        SELECT jobs_effect_proof.record_demo_write(
            '{job_id}', {run}, {attempt}, '{token}', '{operation}');
    """


def check_contention(job_id, token):
    with subprocess.Popen(
        psql(), stdin=subprocess.PIPE, stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE, text=True,
    ) as locker:
        try:
            locker.stdin.write(SESSION_LIMITS + f"""
                BEGIN;
                SELECT j.id FROM mesh_jobs.jobs j
                WHERE j.namespace = '{NAMESPACE}' AND j.id = '{job_id}'
                FOR NO KEY UPDATE OF j
                \\gset
                DO $$ BEGIN RAISE NOTICE 'effect proof row locked'; END $$;
            """)
            locker.stdin.flush()
            ready, _, _ = select.select([locker.stderr], [], [], 8)
            if not ready:
                raise RuntimeError("Row locker did not report readiness")
            notice = locker.stderr.readline().strip()
            expect("row lock readiness", notice.partition(":")[2].strip(), "effect proof row locked")
            # Only NOWAIT should report busy while the peer holds the row.
            # The statement timeout still bounds a blocking regression.
            contended = "SET lock_timeout = 0;\n" + record(job_id, 1, 2, token, "contended")
            expect("contended write", sql(contended), "busy")
        finally:
            try:
                _, errors = locker.communicate("ROLLBACK;\n", timeout=5)
            except subprocess.TimeoutExpired:
                locker.kill()
                locker.communicate()
                raise RuntimeError("Row locker exceeded the shutdown timeout") from None
            if locker.returncode:
                raise RuntimeError(f"Row locker failed: {errors.strip()}")


def main(data_dir):
    expect("owned PostgreSQL backend over TLS", sql("""
        SELECT current_database(), current_setting('data_directory'), ssl
        FROM pg_catalog.pg_stat_ssl WHERE pid = pg_backend_pid();
    """), f"mesh_jobs_proof|{data_dir.resolve()}|t")
    fixture = Path(__file__).resolve().parent
    schema = fixture.parents[2] / "compiler/mesh-rt/src/jobs/schema.sql"
    sql("BEGIN;\n" + schema.read_text() + "\n" + (fixture / "effect.sql").read_text() + "\nCOMMIT;")

    job_id = UUID(sql(f"""
        SELECT mesh_jobs.enqueue('{NAMESPACE}',
            '{{"worker":"proof","version":1,"queue":"default","args":{{}},"metadata":{{}},
              "tags":[],"priority":0,"max_attempts":3,"timeout_ms":60000,
              "backoff_base_ms":0,"backoff_max_ms":0,"suspended":false,"unique":null}}',
            '', '') ->> 'id';
    """))
    job_filter = f"WHERE namespace = '{NAMESPACE}' AND id = '{job_id}'"
    old_token = claim(job_id, 5)
    sql(f"""
        SELECT pg_sleep(greatest(0, lease_expires_at_ms - mesh_jobs.now_ms()) / 1000.0)
        FROM mesh_jobs.jobs {job_filter};
    """)
    expect("database clock reached the lease deadline", sql(f"""
        SELECT lease_expires_at_ms <= mesh_jobs.now_ms() FROM mesh_jobs.jobs {job_filter};
    """), "t")
    before_expired = observe(job_id)
    expect_claim(before_expired, job_id, old_token, 1)
    expect("expired lease", sql(record(job_id, 1, 1, old_token, "expired")), "expired")
    expect("expired lease leaves no changes", observe(job_id), before_expired)

    expect("recovered claims", sql(f"SELECT mesh_jobs.recover('{NAMESPACE}', 1, NULL);"), "1")
    token = claim(job_id, 60000)
    expect("replacement token changed", token != old_token, True)
    before = observe(job_id)
    expect_claim(before, job_id, token, 2)
    expect("replacement lease is live", sql(f"""
        SELECT lease_expires_at_ms > mesh_jobs.now_ms() FROM mesh_jobs.jobs {job_filter};
    """), "t")
    expect("replaced claim", sql(record(job_id, 1, 1, old_token, "replaced")), "lost")
    expect("stale token", sql(record(job_id, 1, 2, old_token, "stale-token")), "lost")
    expect("stale attempt", sql(record(job_id, 1, 1, token, "stale-attempt")), "lost")
    expect("wrong run", sql(record(job_id, 2, 2, token, "wrong-run")), "lost")
    expect("missing job", sql(record(UUID(int=0), 1, 2, token, "missing")), "lost")
    expect("invalid claims leave no changes", observe(job_id),
           dict(job=before["job"], effects=[], receipts=[], write_calls=0))

    commit = record(job_id, 1, 2, token, "demo-write")
    rollback = record(job_id, 1, 2, token, "rollback")
    expect("commit, rollback and replay on one backend", sql(
        f"BEGIN; {commit} COMMIT;\n"
        f"BEGIN; {rollback} ROLLBACK;\n"
        f"BEGIN; {commit} COMMIT;"
    ).splitlines(), ["applied:1", "applied:2", "confirmed:1"])
    # The sequence also detects an attempted insert that receipt replay rolled back.
    durable = dict(
        job=before["job"],
        effects=[dict(id=1, job_id=str(job_id))],
        receipts=[dict(job_id=str(job_id), run=1, operation_id="demo-write", effect_id=1)],
        write_calls=2,
    )
    expect("committed effect and receipt", observe(job_id), durable)

    check_contention(job_id, token)
    expect("row lock contention leaves no changes", observe(job_id), durable)

    # Expire the live fixture lease after the effect has committed. The runner
    # can lose a reply before learning that the operation already succeeded.
    expect("expire the effect writer's lease", sql(f"""
        UPDATE mesh_jobs.jobs SET lease_expires_at_ms = mesh_jobs.now_ms()
        {job_filter} AND state = 'executing' AND lease_token = '{token}'
            AND lease_expires_at_ms > mesh_jobs.now_ms()
        RETURNING id;
    """), str(job_id))
    expect("recover the effect writer", sql(f"SELECT mesh_jobs.recover('{NAMESPACE}', 1, NULL);"), "1")
    replacement_token = claim(job_id, 60000)
    expect("effect writer replacement gets a new token", replacement_token != token, True)
    replacement = observe(job_id)
    expect_claim(replacement, job_id, replacement_token, 3)
    durable = dict(durable, job=replacement["job"])
    expect("replacement preserves the effect and receipt", replacement, durable)
    expect("old writer cannot resolve the operation with a stale lease", sql(commit), "lost")
    same_operation = record(job_id, 1, 3, replacement_token, "demo-write")
    expect("new lease resolves the original committed effect", sql(same_operation), "confirmed:1")
    expect("replay across leases leaves one effect and no extra insertion", observe(job_id), durable)

    expect("committed cancellation request", sql(f"""
        SELECT (mesh_jobs.job_control('{NAMESPACE}', '{job_id}', 'cancel', NULL, NULL)
                ->> 'cancel_requested_at_ms') IS NOT NULL;
    """), "t")
    expect("cancelled attempt still has an unexpired lease", sql(f"""
        SELECT lease_expires_at_ms > mesh_jobs.now_ms() FROM mesh_jobs.jobs {job_filter};
    """), "t")
    cancelled = observe(job_id)
    cancelled_at = cancelled["job"]["cancel_requested_at_ms"]
    expect("cancellation timestamp recorded", isinstance(cancelled_at, int), True)
    expect("cancellation preserves the attempt identity", cancelled,
           dict(durable, job=dict(durable["job"], cancel_requested_at_ms=cancelled_at)))
    expect("cancelled lease", sql(record(job_id, 1, 3, replacement_token, "cancelled")), "cancelled")
    expect("cancelled lease leaves the confirmed effect and attempt intact", observe(job_id), cancelled)
    print("Jobs effect transaction proof passed. One committed effect and one matching receipt.")


if __name__ == "__main__":
    main(Path(sys.argv[1]))
