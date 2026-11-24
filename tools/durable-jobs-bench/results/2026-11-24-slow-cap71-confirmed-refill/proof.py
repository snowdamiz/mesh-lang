"""Untimed canonical-slow producer and two-client SQL fence proof."""
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

from runtime import arm, backend, capacity, INPUT, CAP, install, scope_refill

NS = "parity_a1708bf005e91a39"
WORKERS = '[{"worker":"parity","version":1}]'
CLAIM = f"SELECT status, id FROM mesh_jobs.claim_batch_set_v1('{NS}', 'default', '{WORKERS}', 30000, 16, NULL);"
ROLE = "SET ROLE parity_subject; "
OPERATOR = "SET ROLE parity_operator; "


def snapshot(db):
    return json.loads(db.sql("mesh", f"""
        SELECT json_object_agg(id, json_build_object('state', state,
            'token', lease_token, 'xid', xmin::text, 'attempt', attempt))
        FROM mesh_jobs.jobs WHERE namespace = '{NS}'
    """))


def running(rows):
    return sorted(key for key, row in rows.items() if row["state"] == "executing")


def refill(rows, keys):
    ids = ",".join(keys)
    tokens = ",".join(rows[key]["token"] for key in keys)
    return (f"SELECT status, coalesce(id, context) FROM mesh_jobs.complete_and_refill_v1("
            f"'{NS}', 'default', '{WORKERS}', 30000, 0, 16, '{{{ids}}}', '{{{tokens}}}');")


def check_refill(result, before, after, keys):
    lines = result.splitlines()
    assert lines[0] == "completed|" + "1" * len(keys), lines
    granted = [line.removeprefix("claimed|") for line in lines[1:]]
    assert len(granted) == len(keys) and all(line.startswith("claimed|") for line in lines[1:])
    assert all(before[k]["state"] == "available" and after[k]["state"] == "executing"
               and after[k]["attempt"] == 1 for k in granted)
    assert all(after[k]["state"] == "completed" and after[k]["token"] is None for k in keys)
    assert len({after[k]["xid"] for k in [*keys, *granted]}) == 1
    return granted


def contention(db, statement):
    args, env = db.psql("mesh")
    with subprocess.Popen(args, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                          stderr=subprocess.PIPE, text=True) as lock:
        try:
            lock.stdin.write(f"BEGIN;\nSELECT concurrency FROM mesh_jobs.queues "
                             f"WHERE namespace='{NS}' AND name='default' FOR NO KEY UPDATE;\n")
            lock.stdin.flush()
            assert lock.stdout.readline().rstrip() == "64"
            blocked = subprocess.run([*args, "-v", "VERBOSITY=verbose", "-c", ROLE + statement],
                                     env=env, capture_output=True, text=True, timeout=5)
            assert blocked.returncode != 0 and "55P03" in blocked.stderr, blocked.stderr
        finally:
            if lock.poll() is None:
                lock.stdin.write("ROLLBACK;\n\\q\n")
                lock.stdin.flush()
                lock.wait(timeout=5)
    return "55P03"


def prove(out, candidate, public):
    out.mkdir()
    scratch = out / "scratch"
    scratch.mkdir()
    db = backend.Cluster(scratch, out, CAP)
    db.trusted_application = True
    db.runners, db.slots, db.lanes, db.hold_ms = 2, 64, 4, 100
    try:
        db.start()
        db.prepare_database("mesh")
        settings = json.loads(arm.h.CONFIG.read_text())
        batch, = arm.h.inputs(settings["workloads"]["slow"], 64, settings["seed"])
        assert len(batch["jobs"]) == 512 and all(j["hold_ms"] == 100 for j in batch["jobs"])
        paths = arm.h.batch_files(scratch, [batch])
        path, = paths
        digest = arm.h.digest(path)
        assert hashlib.sha256(digest.encode()).hexdigest() == INPUT
        migrator = backend.Subject("mesh", [str(candidate)], candidate.parent, db, db.env)
        producer = backend.Subject("mesh", [str(public)], public.parent, db, db.env)
        env = migrator.environment(NS, 32, 64, "dense", settings["poll_ms"])
        env.update(JOBS_PARITY_MESH_LISTEN="1", MESH_JOBS_SET_BASED_ENQUEUE="1")
        migrator.call("migrate", env, out / "migrate.log")
        if candidate != public:
            scope_refill(db)
        before_capacity = capacity(db)
        with producer.warm_producer(env, out, time.monotonic() + 30) as enqueue:
            receipt, committed = enqueue(path, out / "enqueue.log")
        assert committed is not None
        assert set(receipt["ids"]) == {row["key"] for row in batch["jobs"]}
        counts = json.loads(db.sql("mesh", f"""
            SELECT json_build_object('jobs', count(*), 'compressed', count(pg_column_compression(args)),
                'min_arg_bytes', min(pg_column_size(args)), 'max_arg_bytes', max(pg_column_size(args)),
                'hold_min', min((args->>'hold_ms')::int), 'hold_max', max((args->>'hold_ms')::int),
                'row_xids', count(DISTINCT xmin::text),
                'enqueue_commits', count(DISTINCT pg_xact_commit_timestamp(xmin)))
            FROM mesh_jobs.jobs WHERE namespace='{NS}'
        """))
        arm.h.write_json(out / "persisted-input.json", counts)
        assert counts["jobs"] == 512 and counts["compressed"] == 0 and counts["enqueue_commits"] == 1, counts
        assert counts["max_arg_bytes"] < 65536 and counts["hold_min"] == counts["hold_max"] == 100
        producer_calls = json.loads(db.sql("mesh", """
            SELECT json_build_object(
                'bulk_calls', sum(calls) FILTER (WHERE query LIKE 'SELECT ordinal, receipt FROM mesh_jobs.enqueue_batch_v1(%'),
                'bulk_rows', sum(rows) FILTER (WHERE query LIKE 'SELECT ordinal, receipt FROM mesh_jobs.enqueue_batch_v1(%'),
                'scalar_calls', sum(calls) FILTER (WHERE query LIKE 'SELECT mesh_jobs.enqueue(%'))
            FROM pg_stat_statements WHERE userid = 'parity_producer'::regrole
        """))
        assert producer_calls == {"bulk_calls": 8, "bulk_rows": 504, "scalar_calls": 8}
        for _ in range(4):
            claimed = db.sql("mesh", ROLE + CLAIM).splitlines()
            assert len(claimed) == 16 and all(row.startswith("claimed|") for row in claimed)
        before = snapshot(db)
        assert db.sql("mesh", ROLE + CLAIM) == "" and len(running(before)) == 64
        cuts = {}
        if candidate != public:
            cohort_a, cohort_b = running(before)[:8], running(before)[8:16]
            first = refill(before, cohort_a)
            second = refill(before, cohort_b)
            cuts["lock_timeout_sqlstate"] = contention(db, first)
            assert snapshot(db) == before, "contention committed a partial completion or grant"
            with ThreadPoolExecutor(max_workers=2) as executor:
                results = list(executor.map(lambda sql: db.sql("mesh", ROLE + sql), [first, second]))
            joint = snapshot(db)
            grants = [check_refill(result, before, joint, keys)
                      for result, keys in zip(results, (cohort_a, cohort_b), strict=True)]
            assert set(grants[0]).isdisjoint(grants[1]) and len(running(joint)) == 64
            cuts["two_worker_clients"] = {"completed": 16, "replacements": 16, "executing_after": 64}

            queue = f"SELECT mesh_jobs.queue_control('{NS}','default',"
            namespace = f"SELECT mesh_jobs.set_namespace_paused('{NS}',"
            db.sql("mesh", OPERATOR + namespace + "true);")
            keys = running(joint)[:1]
            assert db.sql("mesh", ROLE + refill(joint, keys)) == "completed|1"
            assert len(running(snapshot(db))) == 63
            db.sql("mesh", OPERATOR + namespace + "false);" + queue + "'pause',NULL);")
            paused = snapshot(db)
            assert db.sql("mesh", ROLE + refill(paused, running(paused)[:1])) == "completed|1"
            assert len(running(snapshot(db))) == 62
            db.sql("mesh", OPERATOR + queue + "'resume',NULL);" + queue + "'configure',61);")
            lowered = snapshot(db)
            assert db.sql("mesh", ROLE + refill(lowered, running(lowered)[:1])) == "completed|1"
            assert len(running(snapshot(db))) == 61
            assert db.sql("mesh", ROLE + CLAIM) == ""
            db.sql("mesh", OPERATOR + queue + "'configure',64);")
            assert len(db.sql("mesh", ROLE + CLAIM).splitlines()) == 3
            assert len(running(snapshot(db))) == 64
            cuts.update(namespace_pause=True, queue_pause=True, lower_capacity=61, restored_capacity=64)
        assert db.sql("mesh", ROLE + CLAIM) == ""
        assert capacity(db)["role_caps"] == before_capacity["role_caps"]
        arm.h.write_json(out / "proof.json", dict(
            image_sha256=arm.h.digest(candidate), producer_sha256=arm.h.digest(public),
            input_sha256=INPUT, persisted_input=counts, producer_sql=producer_calls,
            cap=CAP, queue_capacity=64, cuts=cuts))
    finally:
        db.stop()
        shutil.rmtree(scratch)
        arm.redact(out, db.password)


if __name__ == "__main__":
    out, candidate, public = [Path(value).resolve() for value in sys.argv[1:]]
    os.environ.update(PRODUCT_COMPOSITION="product-group", PRODUCT_STORAGE="default",
                      PRODUCT_TOPOLOGY="slow-equal-cap71")
    install()
    prove(out, candidate, public)
