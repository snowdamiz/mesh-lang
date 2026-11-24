"""Attach identity, TLS and statement receipts to the frozen native Mesh arm."""
from collections import Counter
from contextlib import contextmanager
from copy import copy
import inspect
import json
import os
from pathlib import Path
import sys
from textwrap import dedent
import time

from identity import observe, pinned_file, process_stamp

INPUT = "9afb598e8e80b56fc96689cfbe7a7061d82f8e5e3c33a2c534d680ce3db2528a"
TRIAL = "db8e121ab9b1f6aed2293b26c79a4fbfa8ac8aba6ff228be54b360bd32048a8e"
V1 = ("faefaf34344958650a0fbdd19556d86d9f5126c5007b9c77225447004a6781e4",
      "8306bc3eeeeae1e44959dd08a081898e36c466ff467c0f11fc680cb26c067094")
BROKER = ("55d7000d562a6ad395f5e64e33e0e973fac0084ed05ad6c30a33db2a3fc6303a",
          "7f228c192630bb6d95cbf9e06c5e0132931f429c1cfa07391bb28cb6fdec8911")
R2 = ("93fef96704ae412cf3c55fae73cc2aac405c8b423670826b4876954a44c8b010",
      "361f10d2c2866f694989198bb8359d29c91dd9928dcd95e8f92293293f226d29")
MODES = ("d2-off", "d2-on", "r2-off", "r2-zero", "v1")
ROLES = {"compiled_caller": 32, "parity_subject": 8, "parity_producer": 1}
ROSTER = """
SELECT coalesce(json_agg(json_build_object(
  'role', a.usename, 'backend', a.pid, 'tls', coalesce(s.ssl, false),
  'client_port', a.client_port) ORDER BY a.pid), '[]')
FROM pg_stat_activity a LEFT JOIN pg_stat_ssl s ON s.pid = a.pid
WHERE a.datname = current_database() AND a.usename IN
  ('parity_subject', 'compiled_caller', 'parity_producer')
"""
SQL = """
SELECT coalesce(json_object_agg(p.query, p.calls), '{}') FROM pg_stat_statements p
WHERE p.dbid = (SELECT oid FROM pg_database WHERE datname = current_database())
  AND p.userid = 'parity_producer'::regrole
  AND (p.query LIKE 'SELECT ordinal, receipt FROM mesh_jobs.enqueue_batch_v1(%'
       OR p.query LIKE 'SELECT mesh_jobs.enqueue(%')
"""


def check(condition, message):
    if not condition:
        raise ValueError(message)


def install_broker_hook(arm):
    # The same pinned direct broker launch anchor used by the short native proofs.
    original = arm.regression.Subject.warm_producer.__wrapped__
    path = Path(original.__code__.co_filename)
    check(arm.h.digest(path) == TRIAL, "frozen direct broker source changed")
    source = dedent(inspect.getsource(original))
    anchor = '    ) as broker, output.open("rb") as reader:\n'
    check(source.count(anchor) == 1, "frozen direct broker launch changed")
    scope = {}
    exec(compile(source.replace(anchor, anchor + "        self.record_broker(broker)\n"),
                 path, "exec"), original.__globals__, scope)
    arm.regression.Subject.warm_producer = scope["warm_producer"]


def main(mode, source, runner_dir, out):
    check(sys.platform == "linux" and sorted(os.sched_getaffinity(0)) == [0, 1], "native CPU envelope changed")
    check(mode in MODES, "unknown dense arm")
    sys.path.insert(0, str(source / "tools/durable-jobs-bench/native"))
    import arm

    check(arm.PROFILES["noop"] == (8, 64, 4, 96, 2, INPUT), "frozen dense profile changed")
    arm.PROFILES["noop"] = (2, 64, 16, 96, 2, INPUT)
    install_broker_hook(arm)
    base = arm.Subject
    runner = runner_dir / "mesh"
    expected_runner = (os.environ["DENSE_D2_ELF"], os.environ["DENSE_D2_MANIFEST"]) if mode in (
        "d2-off", "d2-on") else (V1 if mode == "v1" else R2)
    expected_broker = V1 if mode == "v1" else BROKER
    check(arm.h.digest(runner_dir / "mesh.json") == expected_runner[1], "runner manifest seal changed")
    runner_build = json.loads((runner_dir / "mesh.json").read_text())
    check(runner_build["files"]["mesh"] == expected_runner[0]
          and (runner_build["system"], runner_build["architecture"]) == ("Linux", "x86_64"),
          "native runner build differs")

    class Cluster(arm.Cluster):
        def roster(self, engine, expected):
            def ready():
                rows = json.loads(self.sql(engine, ROSTER))
                if Counter(row["role"] for row in rows) == expected and all(row["tls"] for row in rows):
                    return rows
                return None

            return arm.h.wait_for(ready, time.monotonic() + 12, "physical TLS lanes", 0.05)

        def reconcile_trial(self, engine, namespace, enqueued_ids, handler_attempts, commits):
            result = super().reconcile_trial(engine, namespace, enqueued_ids, handler_attempts, commits)
            if mode == "v1":
                statements = json.loads(self.sql(engine, SQL))
                arm.h.write_json(out / "producer-writer.json", dict(enabled=False,
                    bulk=sum(n for q, n in statements.items() if q.startswith("SELECT ordinal, receipt")),
                    scalar=sum(n for q, n in statements.items() if q.startswith("SELECT mesh_jobs.enqueue(")),
                    statements=statements))
            return result

    class DenseSubject(base):
        def __init__(self, engine, command, cwd, cluster, env, **options):
            super().__init__(engine, command, cwd, cluster, env, **options)
            self.broker = Path(command[0])
            check(command == [str(self.broker)] and
                  arm.h.digest(self.broker.with_name("mesh.json")) == expected_broker[1],
                  "broker manifest or direct command differs")
            self.broker_process = None
            self.runner_info = {}
            self.first_roster = None

        @contextmanager
        def warm_producer(self, env, evidence, deadline):
            self.broker_stamp = pinned_file(self.broker, expected_broker[0])
            with super().warm_producer(env, evidence, deadline) as enqueue:
                yield enqueue

        def record_broker(self, process):
            check(self.broker_process is None, "second broker in frozen arm")
            self.broker_process = (process, process_stamp(process))

        def run(self, env, index, log):
            if index == 1:
                # Only runner zero exists at this point. The delta after
                # runner one starts proves individual ownership from SQL.
                self.first_roster = self.cluster.roster(self.engine,
                    {"compiled_caller": 16, "parity_subject": 4, "parity_producer": 1})
            selected = copy(self)
            selected.command = [str(runner)]
            selected.cwd = runner_dir
            child_env = dict(env)
            child_env.pop("TRUSTED_COMPLETION_WRITER_MS", None)
            child_env.pop("MESH_PG_HOT_CHECKOUT", None)
            if mode == "r2-zero":
                child_env["TRUSTED_COMPLETION_WRITER_MS"] = "0"
            if mode == "d2-on":
                child_env["MESH_PG_HOT_CHECKOUT"] = "1"
            clean = {key: child_env.get(key) for key in (
                "TRUSTED_COMPLETION_WRITER_MS", "MESH_PG_HOT_CHECKOUT", "MESH_EFFECT_LANES",
                "JOBS_PARITY_SLOTS", "MESH_SCHEDULER_MIN_WORKERS", "MESH_SCHEDULER_MAX_WORKERS",
                "JOBS_PARITY_MESH_BATCH", "JOBS_PARITY_MESH_LISTEN")}
            stamp = pinned_file(runner, expected_runner[0])
            process = base.run(selected, child_env, index, log)
            try:
                self.runner_info[process] = (stamp, process_stamp(process), clean)
            except BaseException:
                arm.h.stop_runners([process], require_clean=False)
                raise
            return process

        def capture_runners(self, processes, logs):
            second = self.cluster.roster(self.engine, ROLES)
            first = {row["backend"]: row for row in self.first_roster}
            all_rows = {row["backend"]: row for row in second}
            check(len(processes) == 2 and set(processes) == self.runner_info.keys()
                  and len(all_rows) == 41 and len(first) == 21
                  and all(all_rows.get(pid) == row for pid, row in first.items()),
                  "warmed physical TLS roster changed")
            base.capture_runners(self, processes, logs)
            standard = json.loads((out / "postgres-tls-sessions.json").read_text())
            check({(r["role"], r["backend"], r["tls"]) for r in standard} ==
                  {(r["role"], r["backend"], r["tls"]) for r in second}, "standard TLS roster changed")
            cpus = sorted(os.sched_getaffinity(0))
            broker, start = self.broker_process
            broker_observed = observe(broker, self.broker, "session", expected_broker[0],
                                      self.broker_stamp, start, cpus)
            launches = []
            for process in processes:
                stamp, start, options = self.runner_info[process]
                launches.append(dict(observe(process, runner, "run", expected_runner[0], stamp, start, cpus),
                                     options=options))
            arm.h.write_json(out / "dense-launches.json", dict(
                mode=mode, adapter_pid=os.getpid(), cpus=cpus, broker=broker_observed,
                runner_manifest=expected_runner[1], broker_manifest=expected_broker[1],
                runners=launches, first_runner_tls=self.first_roster, both_runners_tls=second,
                observed_us=time.time_ns() // 1000))

    arm.Subject, arm.Cluster = DenseSubject, Cluster
    sys.argv = [sys.argv[0], "mesh", "noop", str(out)]
    arm.main()


if __name__ == "__main__":
    mode, source, runner, output = sys.argv[1:]
    main(mode, Path(source), Path(runner), Path(output))
