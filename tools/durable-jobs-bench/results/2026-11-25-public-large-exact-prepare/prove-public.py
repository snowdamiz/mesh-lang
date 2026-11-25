"""Untimed TLS replay of the compiled trusted runner and FILE broker."""

import hashlib
import json
import os
from pathlib import Path
import sys
from tempfile import TemporaryDirectory
from time import monotonic

sys.path.insert(0, str(Path(__file__).resolve().parent / "adapter"))
from backend import Cluster, Subject, arm, install, install_broker_observer


def prove(scratch, out, runner, mode):
    install()
    install_broker_observer()
    h = arm.h
    settings = json.loads(h.CONFIG.read_bytes())
    workload = settings["workloads"]["large"]
    cluster = Cluster(scratch, out, 96)
    cluster.trusted_application = True
    cluster.queue_policy = "set_based"
    cluster.runners, cluster.slots, cluster.lanes, cluster.hold_ms = 8, 64, 4, workload["hold_ms"]
    workers = []
    try:
        cluster.start()
        cluster.prepare_database("mesh")
        subject = Subject("mesh", [str(runner)], runner.parent, cluster, dict(
            cluster.env, MESH_SCHEDULER_MIN_WORKERS="2", MESH_SCHEDULER_MAX_WORKERS="2",
            JOBS_PARITY_MESH_BATCH="1", JOBS_PARITY_MESH_LISTEN="1", MESH_EFFECT_LANES="4",
            MESH_EFFECT_APP_URL=cluster.url("mesh", "compiled_caller"),
            MESH_JOBS_SET_BASED_ENQUEUE="1"))
        namespace = "parity_0102030405060708"
        env = subject.environment(namespace, 8, 64, workload["mode"], settings["poll_ms"])
        subject.call("migrate", env, out / "migrate.log")
        batches = h.inputs(workload, 64, settings["seed"])
        payload, = h.batch_files(scratch, batches)
        digest = hashlib.sha256(h.digest(payload).encode()).hexdigest()
        assert digest == arm.PROFILES["large"][-1]
        logs = [out / f"runner-{i}.log" for i in range(8)]
        deadline = monotonic() + 120
        with subject.warm_producer(env, out, deadline) as enqueue:
            try:
                for i, log in enumerate(logs):
                    workers.append(subject.run(env, i, log))
                h.wait_for(lambda: h.runner_ready(workers, logs), deadline, "runner readiness")
                subject.capture_runners(workers, logs)
                receipt, commit = enqueue(payload, out / "enqueue.log")
                assert receipt["count"] == len(receipt["ids"]) == 64
                h.wait_for(lambda: cluster.sql("mesh", f"""
                    SELECT count(*) FROM mesh_jobs.jobs WHERE namespace='{namespace}' AND state='completed'
                """) == "64", deadline, "64 durable jobs")
            finally:
                h.stop_runners(workers, require_clean=sys.exc_info()[0] is None)
                workers.clear()
        expected = dict.fromkeys(receipt["ids"], (1,))
        cluster.reconcile_trial("mesh", namespace, receipt["ids"], expected, {payload.stem: commit})
        reports = [h.mesh_shutdown_report(log) for log in logs]
        assert sum(r["claimed"] for r in reports) == sum(r["finished"] for r in reports) == 64
        assert all(r["drained"] and r["last_error"] is None for r in reports)
        assert all(("batch_protocol" in r) == (mode == "prepared") for r in reports)
        assert all((r["last_batch_confirmed_us"] > 0) == (r["claimed"] > 0) for r in reports)
        assert len({r[key] for r in cluster.jobs for key in
                    ("settled_xid", "attempt_xid", "effect_xid")}) == 192
        return dict(mode=mode, jobs=len(cluster.jobs), runners=8, slots=64,
                    input_sha256=digest, runner_sha256=h.digest(runner),
                    file_broker_sha256=h.digest(subject.session_executable),
                    tls=True, independent_terminal_transactions=64, application_transactions=128)
    finally:
        h.stop_runners(workers, require_clean=False)
        cluster.stop()


if __name__ == "__main__":
    runner, folder = (Path(x).resolve() for x in sys.argv[1:])
    folder.mkdir()
    try:
        with TemporaryDirectory(dir=folder.parent, prefix="pg-") as scratch:
            result = prove(Path(scratch), folder, runner, os.environ["PRODUCT_CLAIM_MODE"])
        (folder / "result.json").write_text(json.dumps(result, sort_keys=True, indent=2) + "\n")
        # These logs are diagnostic only and can contain the disposable login.
        for path in folder.glob("*.log"):
            path.unlink()
    except BaseException:
        for path in folder.glob("*.log"):
            path.unlink()
        raise
