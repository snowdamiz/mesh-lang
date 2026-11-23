"""Vary only scheduler workers of the warmed runners in the sealed V1 arm."""

import contextlib
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import subprocess
import sys
import time

LANE = Path(__file__).resolve().parent
BASE = LANE.parent / "ordinary-trusted-mesh-queue"
SOURCE = BASE / "source"
ORIGINAL_ARM = SOURCE / "tools/durable-jobs-bench/native/arm.py"
ARTIFACTS = BASE / "local-artifacts-v2"
WORKERS = {"stock-a": 2, "stock-b": 2, "workers-4": 4, "workers-8": 8}


def sha(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def module(path):
    sys.path.insert(0, str(path.parent))
    spec = importlib.util.spec_from_file_location("sealed_v1_arm", path)
    arm = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(arm)
    return arm


def thread_counts(harness, processes):
    pids = {process.pid for process in processes}
    if platform.system() == "Darwin":
        output = subprocess.check_output(["ps", "-M", "-p", ",".join(map(str, sorted(pids)))], text=True)
        return harness.mac_thread_counts(output, pids)
    return {pid: len(list(Path(f"/proc/{pid}/task").iterdir())) for pid in pids}


def measure(pinned_arm, label, profile, evidence, local):
    arm = module(pinned_arm)
    source_subject = arm.Subject
    workers = WORKERS[label]

    class SchedulerSubject(source_subject):
        @contextlib.contextmanager
        def warm_producer(self, env, evidence, deadline):
            start = time.monotonic()
            with super().warm_producer(env, evidence, deadline) as enqueue:
                self.broker_ready_seconds = time.monotonic() - start
                yield enqueue

        def run(self, env, index, log):
            if index == 0:
                self.runner_launch = time.monotonic()
            runner_env = dict(env, MESH_SCHEDULER_MIN_WORKERS=str(workers),
                              MESH_SCHEDULER_MAX_WORKERS=str(workers))
            return super().run(runner_env, index, log)

        def capture_runners(self, processes, logs):
            ready = time.monotonic()
            counts = thread_counts(arm.h, processes)
            if set(counts) != {process.pid for process in processes}:
                raise ValueError("missing ready runner thread count")
            super().capture_runners(processes, logs)
            arm.h.write_json(self.cluster.evidence / "scheduler.json", {
                "label": label, "runner_min_workers": workers, "runner_max_workers": workers,
                "runner_pids": [process.pid for process in processes],
                "runner_os_threads_at_ready": counts,
                "runner_launch_to_all_ready_s": ready - self.runner_launch,
                "broker_launch_to_ready_and_db_audit_s": self.broker_ready_seconds,
                "slots_per_runner": arm.h.allocate_slots(self.cluster.runners, self.cluster.slots),
                "app_connections_per_runner": self.cluster.lanes,
                "postgres_cap": self.cluster.connection_cap,
            })

    arm.Subject = SchedulerSubject
    sys.argv = [str(pinned_arm), *(["--local-smoke"] if local else []), "mesh", profile, str(evidence)]
    arm.main()


if __name__ == "__main__":
    _, label, profile, out = sys.argv
    measure(ORIGINAL_ARM, label, profile, Path(out), local=True)
