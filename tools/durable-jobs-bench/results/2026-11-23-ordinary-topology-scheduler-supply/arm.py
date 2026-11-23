"""Collect fixed-scheduler topology arms with the sealed ordinary V1 adapter."""

import argparse
from contextlib import contextmanager
import ctypes
from pathlib import Path
import runpy
import subprocess
import sys
import time

PACKET = Path(__file__).resolve().parent
ROOT = PACKET.parents[3]
PRIOR = PACKET.parent / "2026-11-23-ordinary-runner-topology"
LANE = ROOT / "target/ordinary-topology-scheduler-supply"
LOCK = ROOT / "target/ordinary-runner-topology/local-topology.lock"
CASES = {
    "stock-a": (8, 8, 4, 2),
    "stock-b": (8, 8, 4, 2),
    "two-w2": (2, 32, 16, 2),
    "two-w8": (2, 32, 16, 8),
    "one-w2": (1, 64, 32, 2),
    "one-w16": (1, 64, 32, 16),
    "two-w1": (2, 32, 16, 1),
}
PRIMARY = "two-w8"
BASE_ORDER = ("stock-a", "two-w2", "one-w16", "two-w1", "stock-b", "two-w8", "one-w2")
ORDERS = (BASE_ORDER, BASE_ORDER[::-1],
          BASE_ORDER[2:] + BASE_ORDER[:2], (BASE_ORDER[2:] + BASE_ORDER[:2])[::-1],
          BASE_ORDER[4:] + BASE_ORDER[:4])
ACTIVITY = ("stock-a", "two-w2", "two-w8", "one-w16", "one-w2", "two-w1", "stock-b")

ACTIVITY_COLUMNS = """
    'claim_active', (SELECT coalesce(json_agg(pid ORDER BY pid), '[]')
        FROM pg_stat_activity WHERE usename = 'parity_subject'
          AND datname = current_database() AND state = 'active'
          AND query LIKE '%FROM mesh_jobs.claim_batch_v1%'),
    'app_insert_active', (SELECT coalesce(json_agg(pid ORDER BY pid), '[]')
        FROM pg_stat_activity WHERE usename = 'compiled_caller'
          AND datname = current_database() AND state = 'active'
          AND query LIKE 'INSERT INTO public.mesh_jobs_parity_%'),
    'executing', count(*) FILTER
"""


class DarwinRusageV0(ctypes.Structure):
    _fields_ = [("uuid", ctypes.c_uint8 * 16)] + [
        (name, ctypes.c_uint64) for name in (
            "user_ns", "system_ns", "idle_wakeups", "interrupt_wakeups", "pageins",
            "wired_size", "resident_size", "physical_footprint", "start_abstime", "exit_abstime"
        )
    ]


def cpu_reader():
    libproc = ctypes.CDLL("/usr/lib/libproc.dylib", use_errno=True)
    rusage = libproc.proc_pid_rusage
    rusage.argtypes = (ctypes.c_int, ctypes.c_int, ctypes.c_void_p)
    rusage.restype = ctypes.c_int

    def read(pids):
        point = {"begin_us": time.time_ns() // 1000, "pids": {}}
        for pid in sorted(pids):
            usage = DarwinRusageV0()
            if rusage(pid, 0, ctypes.byref(usage)) != 0:
                raise OSError(ctypes.get_errno(), f"proc_pid_rusage failed for runner {pid}")
            point["pids"][pid] = {key: getattr(usage, key) for key in ("start_abstime", "user_ns", "system_ns")}
        point["end_us"] = time.time_ns() // 1000
        return point

    return read


def run(source, label, out, mode):
    if sys.platform != "darwin":
        raise RuntimeError("live runner CPU recorder requires Darwin")
    base_run = runpy.run_path(str(PRIOR / "arm.py"))["run"]
    original_load = base_run.__globals__["load_arm"]
    workers = CASES[label][3]
    read_cpu = cpu_reader()

    def load(path):
        arm = original_load(path)

        def threads(pids):
            rows = subprocess.check_output(
                ["ps", "-M", "-p", ",".join(map(str, sorted(pids)))], text=True)
            return arm.h.mac_thread_counts(rows, set(pids))

        class SupplyCluster(arm.Cluster):
            cpu_end = None

            def sql(self, database, query):
                anchor = "'executing', count(*) FILTER"
                if anchor in query:
                    query = query.replace(anchor, ACTIVITY_COLUMNS, 1)
                return super().sql(database, query)

            def observe(self, engine, namespace):
                sample = super().observe(engine, namespace)
                if sample["terminal"] == 1024 and self.cpu_end is None:
                    # Capture close to the SQL observation, ahead of ps and the
                    # after-run bookkeeping. The exact elapsed end is the XID.
                    self.cpu_end = read_cpu(self.runner_pids)
                    arm.h.write_json(self.evidence / "runner-cpu-window.json",
                                     {"before_submit": self.cpu_start,
                                      "terminal_observed": self.cpu_end})
                    arm.h.write_json(self.evidence / "scheduler-terminal.json",
                                     threads(self.runner_pids))
                return sample

        class SupplySubject(arm.Subject):
            def __init__(self, engine, command, cwd, cluster, env):
                overrides = dict(MESH_SCHEDULER_MIN_WORKERS=str(workers),
                                 MESH_SCHEDULER_MAX_WORKERS=str(workers))
                super().__init__(engine, command, cwd, cluster, dict(env, **overrides))

            def capture_runners(self, processes, logs):
                super().capture_runners(processes, logs)
                pids = {process.pid for process in processes}
                arm.h.write_json(self.cluster.evidence / "scheduler-ready.json", threads(pids))
                if mode == "activity":
                    # The diagnostic keeps the same broker and adds no
                    # connections. Reuse the ready process sample instead of
                    # invoking ps several times on each 5 ms observation.
                    ready_sample = arm.h.process_sample(processes)
                    arm.h.process_sample = lambda _processes: ready_sample

            @contextmanager
            def warm_producer(self, env, evidence, deadline):
                with super().warm_producer(env, evidence, deadline) as enqueue:
                    def before_submit(path, log):
                        self.cluster.cpu_start = read_cpu(self.cluster.runner_pids)
                        return enqueue(path, log)
                    yield before_submit

        if mode == "activity":
            run_trial = arm.h.run_trial

            def faster_samples(subject, allocation, workload, batches, scratch, evidence, settings, pair):
                return run_trial(subject, allocation, workload, batches, scratch, evidence,
                                 dict(settings, observer_interval_ms=5), pair)

            arm.h.run_trial = faster_samples
        arm.Cluster, arm.Subject = SupplyCluster, SupplySubject
        return arm

    base_run.__globals__.update(load_arm=load, LAYOUTS={label: CASES[label][:3]})
    base_run(source, label, out, local=True, sample_activity=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("label", choices=CASES)
    parser.add_argument("out", type=Path)
    parser.add_argument("mode", choices=("timing", "activity"))
    args = parser.parse_args()
    run(args.source.resolve(), args.label, args.out.resolve(), args.mode)
