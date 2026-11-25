"""Untimed PostgreSQL transaction and public-handler checks on a small fixture."""

from contextlib import contextmanager
import json
from pathlib import Path
import shutil
import time

from backend import Subject, arm
from wire import Relay

NS = "parity_0123456789abcdef"


def wait(label, probe):
    return arm.h.wait_for(probe, time.monotonic() + 12, label, .025)


@contextmanager
def cluster(out, executable, mode):
    out.mkdir(mode=0o700)
    scratch = out / "scratch"
    scratch.mkdir(mode=0o700)
    db = arm.Cluster(scratch, out, 20)
    db.trusted_application = True
    db.runners, db.slots, db.lanes = 1, 4, 1
    try:
        db.start()
        db.prepare_database("mesh")
        subject = Subject("mesh", [str(executable)], executable.parent, db, db.env)
        env = subject.environment(NS, db.slots, db.slots, mode, 50)
        env.update(JOBS_PARITY_MESH_BATCH="1", JOBS_PARITY_MESH_LISTEN="1",
                   MESH_EFFECT_LANES=str(db.lanes), MESH_SCHEDULER_MIN_WORKERS="2",
                   MESH_SCHEDULER_MAX_WORKERS="2", MESH_EFFECT_APP_URL=db.url("mesh", "compiled_caller"))
        subject.call("migrate", env, out / "migrate.log")
        yield db, subject, env
    finally:
        db.stop()
        shutil.rmtree(scratch)
        arm.redact(out, db.password)


def submit(out, subject, env, name, jobs):
    path = out / f"{name}.json"
    items = [dict(key=key, hold_ms=0, fail_first=False, payload="semantic") | extra
             for key, extra in jobs]
    path.write_text(json.dumps(dict(request_id=name, jobs=items, rollback=False)))
    receipt, _ = subject.enqueue_receipt(env, path, out / f"{name}.log")
    assert receipt["count"] == len(jobs)
    return receipt


def rows(db):
    return json.loads(db.sql("mesh", f"""
        SELECT coalesce(jsonb_object_agg(key, to_jsonb(r)), '{{}}') FROM (
            SELECT j.args->>'key' AS key, j.state, j.attempt, j.xmin::text AS xid,
                j.lease_token::text AS token, j.lease_expires_at_ms AS expiry,
                ARRAY(SELECT a.xmin::text FROM public.mesh_jobs_parity_attempts a
                    WHERE a.namespace=j.namespace AND a.job_key=j.args->>'key' ORDER BY a.attempt) AS attempts,
                (SELECT e.xmin::text FROM public.mesh_jobs_parity_effects e
                    WHERE e.namespace=j.namespace AND e.job_key=j.args->>'key') AS effect
            FROM mesh_jobs.jobs j WHERE j.namespace='{NS}'
        ) r
    """))


def handler_cut(out, executable, fault=None, hot=False):
    with cluster(out, executable, "final_attempt" if fault else "dense") as (db, subject, env), Relay(db) as relay:
        app_url = db.url("mesh", "compiled_caller", port=relay.port)
        gate = relay.arm(b"INSERT INTO public.mesh_jobs_parity_attempts", drop=True) if fault else None
        run_env = dict(env, MESH_EFFECT_APP_URL=app_url + ("&mesh_pool_hot_checkout=1" if hot else ""))
        process = subject.run(run_env, 0, out / "runner.log")
        try:
            wait("public runner ready", lambda: arm.h.runner_ready([process], [out / "runner.log"], track=False))
            receipt = submit(out, subject, env, "handler-cut", [("first", {})])
            if gate:
                assert gate.hit.wait(6)
                held = rows(db)["first"]
                assert held["state"] == "executing" and len(held["attempts"]) == 1 and held["effect"] is None
                gate.release.set()
                wait("failed ambiguous attempt", lambda: rows(db)["first"]["state"] == "discarded")
                assert rows(db)["first"]["attempts"] == held["attempts"]
                assert rows(db)["first"]["effect"] is None
                receipt = submit(out, subject, env, "after-loss", [("next", {})])
            key = "next" if gate else "first"
            wait("handler and settlement", lambda: rows(db)[key]["state"] == "completed")
            final = rows(db)[key]
            assert len(final["attempts"]) == 1
            assert len({receipt["xid"], final["attempts"][0], final["effect"], final["xid"]}) == 4
            arm.h.stop_runners([process], require_clean=True)
            inserts = [sql for _, sql in relay.requests if sql.startswith("INSERT INTO")]
            assert len(inserts) == (3 if gate else 2), inserts
            if gate:
                assert gate.witness == dict(command="INSERT 0 1", rows=0, idle=True, receipt_dropped=True)
                assert len({port for port, sql in relay.requests if sql.startswith("INSERT INTO")}) == 2
            arm.h.write_json(out / "result.json", dict(final=final, wire=gate.witness if gate else None,
                                                      app_insert_calls=len(inserts)))
        finally:
            if process.poll() is None:
                if gate:
                    gate.release.set()
                arm.h.stop_runners([process], require_clean=False)


def claim_loss(out, executable):
    with cluster(out, executable, "fault") as (db, subject, env), Relay(db) as relay:
        gate = relay.arm(b"COMMIT", drop=True)
        run_env = dict(env, JOBS_PARITY_POOL_URL=db.url("mesh", port=relay.port))
        log = out / "runner.log"
        process = subject.run(run_env, 0, log)
        try:
            wait("runner ready", lambda: arm.h.runner_ready([process], [log], track=False))
            submit(out, subject, env, "claim-loss", [("claim", {})])
            assert gate.hit.wait(6), "no claimed transaction COMMIT"
            held = rows(db)["claim"]
            assert held["state"] == "executing" and held["attempt"] == 1
            assert held["attempts"] == [] and held["effect"] is None
            assert gate.witness == dict(command="COMMIT", rows=0, idle=True, receipt_dropped=True)
            gate.release.set()
            wait("lease recovery", lambda: rows(db)["claim"]["state"] == "completed")
            done = rows(db)["claim"]
            commit_ms = int(db.sql("mesh", f"""SELECT (extract(epoch FROM
                pg_xact_commit_timestamp(xmin)) * 1000)::bigint
                FROM public.mesh_jobs_parity_attempts WHERE namespace='{NS}' AND job_key='claim'"""))
            assert done["attempt"] == 2 and len(done["attempts"]) == 1
            assert commit_ms >= held["expiry"]
            arm.h.stop_runners([process], require_clean=True)
            assert done["effect"] and done["token"] is None
            assert len({held["xid"], done["xid"], done["attempts"][0], done["effect"]}) == 4
            arm.h.write_json(out / "result.json", dict(held=held, done=done, wire=gate.witness,
                             first_app_commit_ms=commit_ms, claim_responses=relay.claim_responses))
        finally:
            gate.release.set()
            if process.poll() is None:
                arm.h.stop_runners([process], require_clean=False)


def teardown(out, executable, witness, forced):
    with cluster(out, executable, "dense") as (db, subject, env):
        gate = out / "gate"
        gate.mkdir()
        runner = Subject("mesh", [str(witness)], witness.parent, db, db.env)
        log = out / "runner.log"
        child = runner.run(dict(env, COMBINED_TEARDOWN_DIR=str(gate)), 0, log)
        try:
            wait("runner ready", lambda: arm.h.runner_ready([child], [log], track=False))
            receipt = submit(out, subject, env, "teardown", [("owned", {})])
            wait("termination callback entered", (gate / "entered").exists)
            held = rows(db)["owned"]
            assert held["state"] == "executing" and len(held["attempts"]) == 1 and held["effect"]
            assert len({receipt["xid"], held["attempts"][0], held["effect"], held["xid"]}) == 4
            for _ in range(5):
                assert not (gate / "finished").exists()
                assert rows(db)["owned"] == held
            if forced:
                child.kill()
                assert child.wait(timeout=5) == -9
                assert rows(db)["owned"] == held
                done = held
            else:
                (gate / "release").touch()
                wait("termination callback released", (gate / "finished").exists)
                wait("post-exit settlement", lambda: rows(db)["owned"]["state"] == "completed")
                done = rows(db)["owned"]
                assert held["attempts"] == done["attempts"] and held["effect"] == done["effect"]
                assert done["xid"] not in (held["xid"], held["attempts"][0], held["effect"], receipt["xid"])
                arm.h.stop_runners([child], require_clean=True)
            arm.h.write_json(out / "result.json", dict(held=held, done=done, forced=forced,
                             runner_sha256=arm.h.digest(witness)))
        finally:
            if child.poll() is None:
                (gate / "release").touch()
                arm.h.stop_runners([child], require_clean=False)
