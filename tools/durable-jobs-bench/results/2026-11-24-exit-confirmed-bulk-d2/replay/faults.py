#!/usr/bin/env python3
"""Run writer-off faults against disposable PostgreSQL and the real split runner."""
from copy import copy
import json
import os
from pathlib import Path
import shutil
import sys
import time

from wire import Relay

sys.path.insert(0, str(Path(os.environ["COMBINED_BULK_SOURCE"]) / "tools/durable-jobs-bench/native"))
import arm

NS = "parity_0123456789abcdef"
ATTEMPT = "INSERT INTO public.mesh_jobs_parity_attempts"
EFFECT = "INSERT INTO public.mesh_jobs_parity_effects"
CASES = ("stale-session", "lost-application-reply", "lost-claim-reply", "lost-terminal-reply")


def wait(predicate, description, timeout=12):
    return arm.h.wait_for(predicate, time.monotonic() + timeout, description, 0.015)


def exercise(out, broker, executable, checkout, case):
    out.mkdir(mode=0o700)
    scratch = out / "scratch"
    scratch.mkdir(mode=0o700)
    db = arm.Cluster(scratch, out, 20)
    db.trusted_application = True
    db.runners, db.slots, db.lanes = 1, 4, 1
    app_fault = case in CASES[:2]
    log = out / "runner.log"
    process = None

    def rows(key):
        return json.loads(db.sql("mesh", f"""
            SELECT row_to_json(w) FROM (
                SELECT state, attempt, j.xmin::text AS job_xid,
                       lease_expires_at_ms,
                       (SELECT coalesce(json_agg(a.xmin::text ORDER BY a.attempt), '[]')
                          FROM public.mesh_jobs_parity_attempts a
                         WHERE a.namespace=j.namespace AND a.job_key=j.args->>'key') AS attempts,
                       (SELECT e.xmin::text FROM public.mesh_jobs_parity_effects e
                         WHERE e.namespace=j.namespace AND e.job_key=j.args->>'key') AS effect_xid
                  FROM mesh_jobs.jobs j WHERE j.namespace='{NS}' AND j.args->>'key'='{key}'
            ) w
        """) or "null")

    def at(key, state):
        return wait(lambda: row if (row := rows(key)) and row["state"] == state else None,
                    f"{key} is {state}")

    def submit(key):
        batch = out / f"input-{key}.json"
        batch.write_text(json.dumps(dict(request_id=key, rollback=False,
                                         jobs=[dict(key=key, hold_ms=0, fail_first=False, payload="fault")])) + "\n")
        receipt, _ = subject.enqueue_receipt(env, batch, out / f"enq-{key}.log")
        assert receipt["count"] == 1
        return str(int(receipt["xid"]) % 2**32)

    def complete(key, enqueue_xid, attempt=1):
        row = at(key, "completed")
        assert row["attempt"] == attempt and len(row["attempts"]) == 1 and row["effect_xid"]
        assert len({row["job_xid"], enqueue_xid, row["attempts"][0], row["effect_xid"]}) == 4
        return row

    def app_port():
        return int(db.sql("mesh", """
            SELECT client_port FROM pg_stat_activity
            WHERE datname=current_database() AND usename='compiled_caller'
        """))

    def observe(needle):
        with relay.lock:
            return [(port, query) for port, query in relay.requests if query.startswith(needle)]

    try:
        db.start()
        db.prepare_database("mesh")
        subject = arm.Subject("mesh", [str(broker)], broker.parent, db, db.env)
        mode = "final_attempt" if case == "lost-application-reply" else "fault"
        env = subject.environment(NS, db.slots, db.slots, mode, 50)
        env.update(JOBS_PARITY_MESH_BATCH="1", JOBS_PARITY_MESH_LISTEN="1",
                   JOBS_PARITY_MESH_EVENTS="logged", MESH_JOBS_SET_BASED_ENQUEUE="1",
                   TRUSTED_ADMISSION_POLICY="success_cohort", MESH_EFFECT_LANES=str(db.lanes),
                   MESH_SCHEDULER_MIN_WORKERS="2", MESH_SCHEDULER_MAX_WORKERS="2",
                   MESH_EFFECT_APP_URL=db.url("mesh", "compiled_caller"))
        subject.call("migrate", env, out / "migrate.log")
        with Relay(db) as relay:
            runner = copy(subject)
            runner.command = [str(executable)]
            runner.cwd = executable.parent
            runner_env = dict(env)
            runner_env.pop("TRUSTED_COMPLETION_WRITER_MS", None)
            runner_env.pop("MESH_PG_HOT_CHECKOUT", None)
            if checkout == "d2":
                runner_env["MESH_PG_HOT_CHECKOUT"] = "1"
            if app_fault:
                runner_env["MESH_EFFECT_APP_URL"] = db.url("mesh", "compiled_caller", port=relay.port)
            else:
                runner_env["JOBS_PARITY_POOL_URL"] = db.url("mesh", port=relay.port)
            process = runner.run(runner_env, 0, log)
            try:
                wait(lambda: arm.h.runner_ready([process], [log], track=False), "runner readiness")
                if case == "stale-session":
                    retired = app_port()
                    time.sleep(0.55)
                    assert db.sql("mesh", f"""
                        SELECT pg_terminate_backend(pid, 5000) FROM pg_stat_activity
                         WHERE datname=current_database() AND usename='compiled_caller' AND client_port={retired}
                    """) == "t"
                    final = complete("fault", submit("fault"))
                    replacement = app_port()
                    assert replacement != retired
                    assert [port for port, query in relay.requests if query == "SELECT 1" and port == retired] == [retired]
                    assert [port for port, _ in observe(ATTEMPT)] == [replacement]
                    assert [port for port, _ in observe(EFFECT)] == [replacement]
                    result = dict(retired_port=retired, replacement_port=replacement, final=final)
                elif case == "lost-application-reply":
                    original = app_port()
                    gate = relay.arm(ATTEMPT.encode(), drop=True)
                    enqueue_xid = submit("fault")
                    assert gate.hit.wait(6), "application INSERT did not reach the commit boundary"
                    before = at("fault", "executing")
                    assert len(before["attempts"]) == 1 and before["effect_xid"] is None
                    assert before["attempts"][0] != enqueue_xid
                    assert gate.witness == dict(command="INSERT 0 1", rows=0, idle=True, receipt_dropped=True)
                    gate.release.set()
                    final = at("fault", "discarded")
                    assert final["attempt"] == 1 and final["attempts"] == before["attempts"]
                    assert final["effect_xid"] is None
                    assert [port for port, _ in observe(ATTEMPT)] == [original]
                    assert not observe(EFFECT)
                    assert "trusted application write failed" in log.read_text()
                    next_job = complete("next", submit("next"))
                    replacement = app_port()
                    assert replacement != original
                    assert [port for port, _ in observe(ATTEMPT)] == [original, replacement]
                    assert [port for port, _ in observe(EFFECT)] == [replacement]
                    result = dict(original_port=original, replacement_port=replacement,
                                  before=before, final=final, next_job=next_job, wire=gate.witness,
                                  retransmitted_application_writes=0)
                else:
                    claim = case == "lost-claim-reply"
                    gate = relay.arm(b"COMMIT" if claim else b"mesh_jobs.settle_state($1", drop=True)
                    enqueue_xid = submit("fault")
                    assert gate.hit.wait(6), "queue acknowledgement did not reach the commit boundary"
                    before = at("fault", "executing" if claim else "completed")
                    if claim:
                        assert gate.witness == dict(command="COMMIT", rows=0, idle=True, receipt_dropped=True)
                        assert before["attempt"] == 1 and before["attempts"] == [] and before["effect_xid"] is None
                    else:
                        assert gate.witness["idle"] and gate.witness["receipt_dropped"] and gate.witness["rows"] == 1
                    gate.release.set()
                    final = complete("fault", enqueue_xid, attempt=2 if claim else 1)
                    recovery = {}
                    if claim:
                        recovered = int(db.sql("mesh", f"""
                            SELECT (extract(epoch FROM pg_xact_commit_timestamp(xmin))*1000)::bigint
                              FROM public.mesh_jobs_parity_attempts WHERE namespace='{NS}' AND job_key='fault'
                        """))
                        assert recovered >= before["lease_expires_at_ms"]
                        recovery["recovered_attempt_commit_ms"] = recovered
                    else:
                        assert final == before
                    next_job = complete("next", submit("next"))
                    result = dict(before=before, final=final, next_job=next_job, wire=gate.witness, **recovery)
                arm.h.stop_runners([process], require_clean=True)
                arm.h.write_json(out / "result.json", dict(case=case, checkout=checkout, result=result,
                                 requests=relay.requests, runner_sha256=arm.h.digest(executable),
                                 broker_sha256=arm.h.digest(broker)))
            finally:
                if process.poll() is None:
                    for pending in relay.gates:
                        pending.release.set()
                    arm.h.stop_runners([process], require_clean=False)
    finally:
        db.stop()
        shutil.rmtree(scratch)
        arm.redact(out, db.password)


if __name__ == "__main__":
    out, broker, runner = (Path(item).resolve() for item in sys.argv[1:4])
    checkout, *cases = sys.argv[4:]
    assert checkout in ("d2", "off") and set(cases) <= set(CASES)
    out.mkdir(mode=0o700)
    for case in cases or CASES:
        exercise(out / case, broker, runner, checkout, case)
        print(case, checkout, "passed", flush=True)
