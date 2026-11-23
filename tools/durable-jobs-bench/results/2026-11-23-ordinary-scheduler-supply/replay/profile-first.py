"""Untimed dense controls with OS stacks and actual application INSERT activity."""

import argparse
from contextlib import contextmanager
import fcntl
import json
import os
from pathlib import Path
import subprocess
import sys
import time

import arm
from run import LOCK, PG_BIN, check_pins

original_module = arm.module


def instrument(path):
    original = original_module(path)
    base_subject, base_cluster = original.Subject, original.Cluster
    run_trial = original.h.run_trial

    class ProfileCluster(base_cluster):
        def observe(self, engine, namespace):
            observed = super().observe(engine, namespace)
            if not getattr(self, "profile_live", False):
                return observed
            active_sql = self.sql(engine, """
                SELECT json_build_object(
                  'observed_us', (extract(epoch FROM clock_timestamp()) * 1000000)::bigint,
                  'application', coalesce(json_agg(json_build_object(
                    'pid', pid, 'wait_event_type', wait_event_type, 'wait_event', wait_event,
                    'query_start_us', (extract(epoch FROM query_start) * 1000000)::bigint,
                    'state', state, 'query', left(query, 100))
                    ORDER BY pid) FILTER (WHERE usename = 'compiled_caller'), '[]'),
                  'queue', coalesce(json_agg(json_build_object(
                    'pid', pid, 'wait_event_type', wait_event_type, 'wait_event', wait_event,
                    'state', state, 'query', left(query, 100))
                    ORDER BY pid) FILTER (WHERE usename = 'parity_subject'), '[]'))
                FROM pg_stat_activity WHERE datname = current_database()
                    AND usename IN ('compiled_caller', 'parity_subject') AND state = 'active'
            """)
            with (self.evidence / "pg-active.jsonl").open("a") as stream:
                stream.write(json.dumps(json.loads(active_sql), separators=(",", ":")) + "\n")
            return observed

    class ProfileSubject(base_subject):
        def capture_runners(self, processes, logs):
            self.profile_pid = processes[0].pid
            super().capture_runners(processes, logs)

        @contextmanager
        def warm_producer(self, env, evidence, deadline):
            profiler = None
            try:
                with super().warm_producer(env, evidence, deadline) as enqueue:
                    with (evidence / "sample.log").open("xb") as log:
                        def record(path, batch_log):
                            nonlocal profiler
                            original.h.write_json(evidence / "profile-window.json", dict(
                                first_runner_pid=self.profile_pid, sampling_start_us=time.time_ns() // 1000))
                            profiler = subprocess.Popen(
                                ["/usr/bin/sample", str(self.profile_pid), "4", "1", "-mayDie", "-file",
                                 str(evidence / "runner-sample.txt")], stdout=log, stderr=subprocess.STDOUT)
                            self.cluster.profile_live = True
                            return enqueue(path, batch_log)
                        yield record
            finally:
                self.cluster.profile_live = False
                if profiler is not None:
                    try:
                        code = profiler.wait(timeout=8)
                    except subprocess.TimeoutExpired:
                        profiler.kill()
                        code = profiler.wait(timeout=5)
                    original.h.write_json(evidence / "sample-status.json", dict(exit_code=code))

    def faster_observation(subject, allocation, workload, batches, scratch, evidence, settings, pair):
        return run_trial(subject, allocation, workload, batches, scratch, evidence,
                         dict(settings, observer_interval_ms=10), pair)

    original.Subject = ProfileSubject
    original.Cluster = ProfileCluster
    original.h.run_trial = faster_observation
    return original


def main(label, out):
    check_pins()
    state = arm.LANE / "local-state"
    temp = state / "tmp"
    os.environ.update(TMPDIR=str(temp) + "/", PG_BIN_DIR=str(PG_BIN), DURABLE_JOBS_PHASE="diagnostic",
                      NATIVE_JOBS_STATE=str(state), NATIVE_JOBS_ARTIFACTS=str(arm.ARTIFACTS))
    os.environ.pop("JOBS_PARITY_MESH_EVENTS", None)
    arm.module = instrument
    with LOCK.open("r") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        arm.measure(arm.ORIGINAL_ARM, label, "dense", out, local=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("label", choices=("stock-a", "workers-8"))
    parser.add_argument("out", type=Path)
    args = parser.parse_args()
    sys.exit(main(args.label, args.out.resolve()))
