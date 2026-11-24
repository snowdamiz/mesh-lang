#!/usr/bin/env python3
"""Witness sparse cold and empty-tail pickup on an enabled split 16-slot runner."""
import json
import shutil
import sys
import time
from pathlib import Path

from campaign import BROKER, BULK_SOURCE, read, sha, write

sys.path.insert(0, str(BULK_SOURCE / "tools/durable-jobs-bench/native"))
import arm

NS = "parity_0123456789abcdef"


def run(out, executable):
    out.mkdir()
    scratch = out / "scratch"
    scratch.mkdir()
    cluster = arm.Cluster(scratch, out, 20)
    cluster.trusted_application = True
    cluster.runners, cluster.slots, cluster.lanes = 1, 16, 8
    process = None
    try:
        cluster.start()
        cluster.prepare_database("mesh")
        subject = arm.Subject("mesh", [str(BROKER / "mesh")], BROKER, cluster, cluster.env)
        env = subject.environment(NS, 16, 16, "sparse", 250)
        env.update(JOBS_PARITY_MESH_BATCH="1", JOBS_PARITY_MESH_LISTEN="1", MESH_SHORT_REFILL="1",
                   MESH_JOBS_SET_BASED_ENQUEUE="1", TRUSTED_ADMISSION_POLICY="success_cohort",
                   TRUSTED_COMPLETION_WRITER_MS="0", MESH_SCHEDULER_MIN_WORKERS="2", MESH_SCHEDULER_MAX_WORKERS="2",
                   MESH_EFFECT_LANES="8", MESH_EFFECT_APP_URL=cluster.url("mesh", "compiled_caller"),
                   JOBS_PARITY_STATUS_PATH=str(scratch / "status"))
        subject.call("migrate", env, out / "migrate.log")
        runner = arm.Subject("mesh", [str(executable)], executable.parent, cluster, env)
        log = out / "runner.log"
        process = runner.run(env, 0, log)
        arm.h.wait_for(lambda: arm.h.runner_ready([process], [log], track=False), time.monotonic() + 10, "ready")
        assert "short refill enabled" in log.read_text().splitlines()
        status = scratch / "status"

        def observe():
            status.unlink(missing_ok=True)
            status.with_suffix(".request").touch()
            arm.h.wait_for(status.exists, time.monotonic() + 5, "status")
            return read(status)

        arm.h.wait_for(lambda: observe()["wakeups"]["subscription_scans"] > 0,
                       time.monotonic() + 5, "empty subscribed queue")
        cases = []
        with subject.warm_producer(env, out, time.monotonic() + 20) as submit:
            cluster.audit_tls("mesh")
            for index in range(3):
                label = f"sparse-{index}"
                before = observe()["wakeups"]["notification_scans"]
                path = out / f"{label}.json"
                write(path, dict(request_id=label, rollback=False, jobs=[dict(
                    key=label, hold_ms=1, fail_first=False, payload="sparse refill")]))
                receipt, _ = submit(path, out / f"{label}.log")
                assert receipt["count"] == 1
                identity = receipt["ids"][label]
                def settled():
                    result = cluster.sql("mesh", f"""
                        SELECT json_build_object('state',j.state,'attempt',j.attempt,
                          'enqueued_us', (extract(epoch FROM pg_xact_commit_timestamp(r.xmin)) * 1e6)::bigint,
                          'entered_us', {arm.ledger.epoch_us('a.entered_at')},'attempt_xid',a.xmin::text,'effect_xid',e.xmin::text,
                          'terminal_xid',j.xmin::text)
                        FROM mesh_jobs.jobs j
                        JOIN public.mesh_jobs_parity_product r ON r.engine='mesh' AND r.namespace=j.namespace AND r.request_id='{label}'
                        JOIN public.mesh_jobs_parity_attempts a ON a.namespace=j.namespace AND a.job_key='{label}'
                        JOIN public.mesh_jobs_parity_effects e ON e.namespace=j.namespace AND e.job_key='{label}'
                        WHERE j.id='{identity}' AND j.state='completed'
                    """).strip()
                    return result or None
                result = json.loads(arm.h.wait_for(settled, time.monotonic() + 3, label, 0.015))
                after = observe()
                assert after["wakeups"]["notification_scans"] == before + 1
                assert result["attempt"] == 1
                assert len({result[key] for key in ("attempt_xid", "effect_xid", "terminal_xid")}) == 3
                result["pickup_us"] = result["entered_us"] - result["enqueued_us"]
                assert 0 <= result["pickup_us"] < 100_000
                cases.append(result)
                assert after["claimed"] == index + 1 and after["last_error"] is None
                time.sleep(0.32)
        arm.h.stop_runners([process], require_clean=True)
        write(out / "receipt.json", dict(runner_sha256=sha(executable), cases=cases))
    finally:
        if process and process.poll() is None:
            arm.h.stop_runners([process], require_clean=False)
        cluster.stop()
        shutil.rmtree(scratch)
        arm.redact(out, cluster.password)


if __name__ == "__main__":
    run(*(Path(item).resolve() for item in sys.argv[1:]))
