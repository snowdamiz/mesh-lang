"""Local two-runner dense completion experiment using the sealed split runner."""
from collections import Counter
import fcntl
import hashlib
import itertools
import json
import os
from pathlib import Path
import subprocess
import sys
import time

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
WORK = ROOT / "target/dense-r2-write"
BASE = ROOT / "target/ordinary-completion-writer/combined"
SPLIT = HERE.parent / "2026-11-23-ordinary-completion-writer/combined/replay"
FROZEN = "9afb598e8e80b56fc96689cfbe7a7061d82f8e5e3c33a2c534d680ce3db2528a"
EXPECTED = {"compiled_caller": 32, "parity_producer": 1, "parity_subject": 8}
ORDERS = list(itertools.permutations(("off-a", "zero", "off-b")))


def write(path, value):
    with path.open("x") as stream:
        json.dump(value, stream, indent=2, sort_keys=True)
        stream.write("\n")


def setup():
    os.environ.update(COMBINED_BULK_SOURCE=str(BASE / "bulk-base/ordinary-v2"),
                      COMBINED_RUNNER_ARTIFACTS=str(BASE / "runner-artifacts"),
                      NATIVE_JOBS_ARTIFACTS=str(BASE / "broker-artifacts"),
                      NATIVE_JOBS_STATE=str(WORK / "state"),
                      PG_BIN_DIR="/opt/homebrew/opt/postgresql@17/bin",
                      MESH_JOBS_SET_BASED_ENQUEUE="1", TRUSTED_ADMISSION_POLICY="success_cohort",
                      TMPDIR=str(WORK / "tmp"), DURABLE_JOBS_PHASE="diagnostic")
    for name in ("TRUSTED_COMPLETION_WRITER_MS", "MESH_PG_HOT_CHECKOUT"):
        os.environ.pop(name, None)


def arm_run(label, out):
    setup()
    os.environ["COMBINED_WRITER_MODE"] = "zero" if label == "zero" else "off"
    sys.path.insert(0, str(SPLIT))
    from split_arm import arm, BaseSubject, SplitSubject

    if arm.PROFILES["noop"] != (8, 64, 4, 96, 2, FROZEN):
        raise ValueError("dense fixture or original resource envelope changed")
    arm.PROFILES["noop"] = (2, 64, 16, 96, 2, FROZEN)

    class CheckedSubject(SplitSubject):
        def capture_runners(self, processes, logs):
            def warm():
                allocation = self.cluster.sql(self.engine, """
                    SELECT json_object_agg(rol, n) FROM (
                        SELECT a.usename AS rol, count(*) AS n FROM pg_stat_activity a
                        JOIN pg_stat_ssl s ON s.pid = a.pid AND s.ssl
                        WHERE a.datname = current_database() AND a.usename IN
                              ('parity_subject', 'compiled_caller', 'parity_producer')
                        GROUP BY a.usename
                    ) AS totals
                """)
                return json.loads(allocation) == EXPECTED

            arm.h.wait_for(warm, time.monotonic() + 10, "41 warmed TLS sessions", 0.05)
            BaseSubject.capture_runners(self, processes, logs)
            rows = json.loads((out / "postgres-tls-sessions.json").read_text())
            if Counter(r["role"] for r in rows if r["tls"]) != EXPECTED:
                raise RuntimeError("TLS allocation changed between preflight and capture")
            if len(processes) != 2 or any(Path(p.fixture_executable).resolve() != self.runner_file for p in processes):
                raise RuntimeError("runner executable mismatch")
            if self.env["MESH_SCHEDULER_MIN_WORKERS"] != self.env["MESH_SCHEDULER_MAX_WORKERS"] or self.env["MESH_SCHEDULER_MIN_WORKERS"] != "2":
                raise RuntimeError("scheduler supply changed")
            write(out / "split-executables.json", dict(broker=self.broker_sha, runner=self.runner_sha,
                  runner_manifest=self.runner_manifest_sha, broker_manifest=self.broker_manifest_sha,
                  mode=self.mode, pids=[p.pid for p in processes], slots_per_runner=32,
                  scheduler_workers_per_runner=2, tls_counts=EXPECTED))

    arm.Subject = CheckedSubject
    sys.argv = [sys.argv[0], "--local-smoke", "mesh", "noop", str(out)]
    arm.main()


def campaign(name, smoke):
    (WORK / "state/scratch").mkdir(parents=True, exist_ok=True)
    (WORK / "tmp").mkdir(exist_ok=True)
    out = WORK / name
    out.mkdir()
    orders = [("zero",)] if smoke else ORDERS
    write(out / "freeze.json", dict(orders=orders, input_sha256=FROZEN,
          source_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
          decision="Advance only with submit and drain faster than both controls beyond their absolute wall spread in at least four of six groups, plus earlier last application commits and reconciled ledgers."))
    with (ROOT / "target/queue-performance.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        for index, order in enumerate(orders, 1):
            for label in order:
                directory = out / f"{index:02}-{label}"
                load = os.getloadavg()
                with directory.with_suffix(".log").open("x") as log:
                    result = subprocess.run([sys.executable, "-B", __file__, "arm", label, str(directory)],
                                            stdout=log, stderr=subprocess.STDOUT)
                write(directory.with_suffix(".status.json"), dict(exit=result.returncode, load_before=load,
                      load_after=os.getloadavg()))
                print(directory.name, result.returncode, flush=True)
                result.check_returncode()


if __name__ == "__main__":
    if sys.argv[1] == "arm":
        arm_run(sys.argv[2], Path(sys.argv[3]))
    else:
        campaign(sys.argv[2], sys.argv[1] == "smoke")
