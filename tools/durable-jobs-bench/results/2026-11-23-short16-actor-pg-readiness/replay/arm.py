"""Add application-pool opt-in and observation to the frozen V2 arm."""
from collections import Counter
from contextlib import contextmanager
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys

LABELS = ("off-a", "off-b", "readiness")
CONTRACT = "ordinary-trusted-shared-process-v2"
READINESS = CONTRACT + "-cooperative-app-pg"
ROLES = {"parity_subject": 4, "parity_producer": 1, "compiled_caller": 8}
SESSIONS = """
SELECT coalesce(json_agg(json_build_object(
 'role', a.usename, 'backend', a.pid, 'tls', coalesce(s.ssl, false),
 'state', a.state, 'wait', a.wait_event, 'query', a.query,
 'query_start_us', (extract(epoch FROM a.query_start) * 1000000)::bigint)
 ORDER BY a.pid), '[]')
FROM pg_stat_activity a LEFT JOIN pg_stat_ssl s USING (pid)
WHERE a.datname = current_database()
  AND a.usename IN ('parity_subject', 'parity_producer', 'compiled_caller')
"""
ACTIVITY = """
SELECT json_build_object(
 'epoch_us', (extract(epoch FROM clock_timestamp()) * 1000000)::bigint,
 'app', (SELECT count(*) FROM pg_stat_activity
           WHERE datname=current_database() AND usename='compiled_caller'),
 'active_writes', (SELECT coalesce(json_agg(pid ORDER BY pid), '[]')
   FROM pg_stat_activity WHERE datname=current_database() AND usename='compiled_caller'
     AND state='active' AND query LIKE 'INSERT INTO public.mesh_jobs_parity_%'))
"""


def load(source):
    path = source / "tools/durable-jobs-bench/native/arm.py"
    sys.path.insert(0, str(path.parent))
    spec = importlib.util.spec_from_file_location("frozen_v2_arm", path)
    arm = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(arm)
    return arm


def run(source, label, out, diagnostic):
    arm = load(source)
    write = arm.h.write_json
    if arm.TRUSTED_CONTRACT != CONTRACT or arm.PROFILES["short"][:4] != (1, 16, 8, 20):
        raise ValueError("wrong frozen V2 contract or short topology")

    class ObservedCluster(arm.Cluster):
        counter_step = 0
        observer = None

        def sessions(self, engine):
            return json.loads(self.sql(engine, SESSIONS))

        def calls(self, engine):
            self.counter_step += 1
            counters = super().calls(engine)
            write(self.evidence / f"counters-{self.counter_step}.json", counters)
            if self.counter_step == 2:
                write(self.evidence / "sessions-before.json", self.sessions(engine))
                if diagnostic:
                    self.start_observer(engine)
            elif self.counter_step == 3:
                self.stop_observer()
                write(self.evidence / "sessions-end.json", self.sessions(engine))
            return counters

        def start_observer(self, engine):
            # Extra TLS owner session and rapid activity observer only in
            # diagnostic arms. Preserve the original timed-arm observation.
            args, env = self.psql(engine)
            with (self.evidence / "activity.log").open("wb") as out, \
                 (self.evidence / "activity-errors.log").open("wb") as err:
                self.observer = subprocess.Popen(args, env=env, stdin=subprocess.PIPE,
                                                 stdout=out, stderr=err)
            self.observer.stdin.write((ACTIVITY + "\\watch 0.001\n").encode())
            self.observer.stdin.flush()

        def stop_observer(self):
            if self.observer is not None:
                if self.observer.poll() is None:
                    self.observer.terminate()
                self.observer.communicate(timeout=5)
                self.observer = None

    class ObservedSubject(arm.Subject):
        def __init__(self, engine, command, artifacts, cluster, env):
            super().__init__(engine, command, artifacts, cluster, env)
            if label == "readiness":
                self.env["MESH_EFFECT_APP_URL"] += "&mesh_cooperative_pool=1"
            if diagnostic:
                self.env["MESH_PG_TRACE_WAITS"] = "1"

        def capture_runners(self, processes, logs):
            super().capture_runners(processes, logs)
            (runner,) = processes
            sessions = self.cluster.sessions(self.engine)
            if Counter(row["role"] for row in sessions if row["tls"]) != ROLES or len(sessions) != 13:
                raise ValueError("missing exact eight app, four queue or one broker verified-TLS sessions")
            write(self.cluster.evidence / "sessions-warm.json", sessions)
            write(self.cluster.evidence / "experiment.json", dict(
                label=label, runner_pid=runner.pid, readiness=label == "readiness",
                execution_contract=READINESS if label == "readiness" else CONTRACT,
                scheduler_min=int(self.env["MESH_SCHEDULER_MIN_WORKERS"]),
                scheduler_max=int(self.env["MESH_SCHEDULER_MAX_WORKERS"]),
                sampler=diagnostic, producer_mode="one warm shared producer/observer process"))
            if diagnostic and sys.platform == "darwin":
                destination = self.cluster.evidence / "warm-runner-stacks.txt"
                with (self.cluster.evidence / "warm-runner-stacks.log").open("wb") as log:
                    result = subprocess.run(
                        ["sample", str(runner.pid), "1", "10", "-file", str(destination)],
                        stdout=log, stderr=subprocess.STDOUT, timeout=10)
                if result.returncode:
                    raise ValueError("could not sample live scheduler stacks")

        @contextmanager
        def warm_producer(self, env, evidence, deadline):
            with super().warm_producer(env, evidence, deadline) as enqueue:
                try:
                    yield enqueue
                finally:
                    self.cluster.stop_observer()

    arm.Cluster, arm.Subject = ObservedCluster, ObservedSubject
    sys.argv = [str(source), "--local-smoke", "mesh", "short", str(out)]
    arm.main()


if __name__ == "__main__":
    source, label, out, *extras = sys.argv[1:]
    assert label in LABELS and extras in ([], ["--diagnostic"])
    assert os.environ["MESH_JOBS_SET_BASED_ENQUEUE"] == "1"
    assert os.environ["TRUSTED_ADMISSION_POLICY"] == "success_cohort"
    run(Path(source).resolve(), label, Path(out).resolve(), bool(extras))
