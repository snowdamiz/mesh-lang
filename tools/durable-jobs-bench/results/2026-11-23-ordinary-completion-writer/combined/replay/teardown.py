#!/usr/bin/env python3
"""Reconcile actual SQL state while a native actor termination callback is held."""
from copy import copy
import json
import os
from pathlib import Path
import shutil
import sys
import time

sys.path.insert(0, str(Path(os.environ["COMBINED_BULK_SOURCE"]) / "tools/durable-jobs-bench/native"))
import arm

h = arm.h
NAMESPACE = "parity_0123456789abcdef"


def run(out, broker, executable, mode):
    out.mkdir(mode=0o700)
    scratch = out / "scratch"
    scratch.mkdir(mode=0o700)
    gate = scratch / "gate"
    gate.mkdir(mode=0o700)
    db = arm.Cluster(scratch, out, 20)
    db.trusted_application = True
    db.runners, db.slots, db.lanes = 1, 4, 2
    process = None

    def observe():
        return json.loads(db.sql("mesh", f"""
            SELECT row_to_json(w) FROM (
                SELECT j.state, j.attempt, j.xmin::text AS job_xid,
                   a.xmin::text AS attempt_xid, e.xmin::text AS effect_xid
                FROM mesh_jobs.jobs j
                JOIN public.mesh_jobs_parity_attempts a ON a.namespace=j.namespace
                    AND a.job_key=j.args->>'key'
                JOIN public.mesh_jobs_parity_effects e ON e.namespace=j.namespace
                    AND e.job_key=j.args->>'key'
                WHERE j.namespace='{NAMESPACE}' AND j.args->>'key'='probe'
            ) w
        """))

    def wait(check, label):
        return h.wait_for(check, time.monotonic() + 5, label, 0.02)

    def verify_held():
        assert (gate / "entered").exists() and not (gate / "finished").exists()
        assert not (gate / "abnormal").exists() and not (gate / "timeout").exists()

    try:
        db.start()
        db.prepare_database("mesh")
        subject = arm.Subject("mesh", [str(broker)], broker.parent, db, db.env)
        env = subject.environment(NAMESPACE, 4, 4, "dense", 50)
        env.update(JOBS_PARITY_MESH_BATCH="1", JOBS_PARITY_MESH_LISTEN="1",
                   MESH_JOBS_SET_BASED_ENQUEUE="1", TRUSTED_ADMISSION_POLICY="success_cohort",
                   MESH_SCHEDULER_MIN_WORKERS="2", MESH_SCHEDULER_MAX_WORKERS="2",
                   MESH_EFFECT_LANES="2", MESH_EFFECT_APP_URL=db.url("mesh", "compiled_caller"))
        subject.call("migrate", env, out / "migrate.log")
        runner = copy(subject)
        runner.command = [str(executable)]
        runner.cwd = executable.parent
        runner_env = dict(env, COMBINED_TEARDOWN_DIR=str(gate))
        if mode == "product":
            runner_env["COMBINED_TEARDOWN_PUBLIC"] = "1"
        elif mode in {"zero", "r1"}:
            runner_env["TRUSTED_COMPLETION_WRITER_MS"] = "0"
        log = out / "runner.log"
        process = runner.run(runner_env, 0, log)
        wait(lambda: h.runner_ready([process], [log], track=False), "runner ready")
        batch = out / "input.json"
        batch.write_text(json.dumps(dict(request_id="probe", rollback=False,
                                         jobs=[dict(key="probe", hold_ms=0, fail_first=False, payload="callback")])))
        receipt, _ = subject.enqueue_receipt(env, batch, out / "producer.log")
        assert receipt["count"] == 1
        wait((gate / "entered").exists, "native terminate callback entered")

        if mode == "v2" or mode == "r1":
            held = wait(lambda: row if (row := observe())["state"] == "completed" else None,
                        "durable completion while the actor still owns the held callback")
        else:
            for _ in range(12):
                verify_held()
                held = observe()
                assert held["state"] == "executing", held
                time.sleep(0.025)
        verify_held()
        (gate / "release").touch()
        wait((gate / "finished").exists, "native terminate callback released")
        finished = wait(lambda: row if (row := observe())["state"] == "completed" else None,
                        "post-release durable settlement")
        assert finished["attempt"] == 1
        assert held["attempt_xid"] == finished["attempt_xid"]
        assert held["effect_xid"] == finished["effect_xid"]
        assert len({finished["attempt_xid"], finished["effect_xid"], finished["job_xid"],
                    str(int(receipt["xid"]) % 2**32)}) == 4
        h.stop_runners([process], require_clean=True)
        h.write_json(out / "teardown.json", dict(
            mode=mode, runner_sha256=h.digest(executable), broker_sha256=h.digest(broker),
            held_callback_state=held, after_callback_state=finished, normal_exit=True,
            callback_held_witness="entered present and finished absent before release"))
    finally:
        if process and process.poll() is None:
            (gate / "release").touch()
            h.stop_runners([process], require_clean=False)
        db.stop()
        shutil.rmtree(scratch)
        arm.redact(out, db.password)


if __name__ == "__main__":
    out, broker, runner = (Path(arg).resolve() for arg in sys.argv[1:4])
    mode = sys.argv[4]
    assert mode in {"v2", "r1", "product", "off", "zero"}
    run(out, broker, runner, mode)
