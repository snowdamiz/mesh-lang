"""Apply fixed scheduler bounds to runner children of the frozen D2 arm."""
from contextlib import contextmanager
import importlib.util
import json
import os
from pathlib import Path
import sys
import time

HERE = Path(__file__).resolve().parent
KEYS = ("MESH_SCHEDULER_MIN_WORKERS", "MESH_SCHEDULER_MAX_WORKERS")
WRITERS = """
SELECT coalesce(json_agg(json_build_object(
    'backend', a.pid, 'tls', coalesce(s.ssl, false), 'client_port', a.client_port,
    'query_started_us', (extract(epoch FROM a.query_start) * 1000000)::bigint,
    'last_was_application_write',
    ltrim(a.query) ~ '^INSERT INTO public[.]mesh_jobs_parity_(attempts|effects)([[:space:]]|$)'
  ) ORDER BY a.pid), '[]')
FROM pg_stat_activity a LEFT JOIN pg_stat_ssl s ON s.pid = a.pid
WHERE a.datname = current_database() AND a.usename = 'compiled_caller'
"""


def sibling(name, filename):
    spec = importlib.util.spec_from_file_location(name, HERE / filename)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


def runner_env(env, workers):
    if workers not in {"2", "4"} or any(env.get(key) != "2" for key in KEYS):
        raise ValueError("unexpected original or experimental scheduler bounds")
    return {**env, **dict.fromkeys(KEYS, workers)}


def proc(pid):
    fields = (Path("/proc") / str(pid) / "stat").read_text().rsplit(")", 1)[1].split()
    return dict(start_ticks=int(fields[19]), parent=int(fields[1]), threads=int(fields[17]),
                user_ticks=int(fields[11]), system_ticks=int(fields[12]))


def pg_cpu(postmaster):
    begin_us = time.time_ns() // 1000
    children = (Path("/proc") / str(postmaster) / "task" / str(postmaster) / "children")
    rows = {}
    for pid in [postmaster, *map(int, children.read_text().split())]:
        try:
            rows[str(pid)] = proc(pid)
        except FileNotFoundError:
            rows[str(pid)] = None
    return dict(begin_us=begin_us, end_us=time.time_ns() // 1000, processes=rows)


def main(workers, source, runner, output):
    identity = sibling("identity", "identity.py")
    dense = sibling("dense_checkout_arm", "native-arm.py")
    dense.check(sys.platform == "linux" and os.sched_getaffinity(0) == {0, 1},
                "native CPU envelope changed")
    dense.check(workers in {"2", "4"} and not any(key in os.environ for key in KEYS),
                "scheduler bounds must enter through runner children")
    sys.path.insert(0, str(source / "tools/durable-jobs-bench/native"))
    import arm

    start_arm = arm.main

    def measured_arm():
        # dense.main has installed the sealed D2 broker, cluster, and Subject.
        class Cluster(arm.Cluster):
            counter_reads = 0

            def calls(self, engine):
                result = super().calls(engine)
                self.counter_reads += 1
                if self.counter_reads == 3:
                    # The original harness has just observed all 1,024 terminal
                    # rows and frozen its counters. The workers are still live.
                    after = pg_cpu(self.postmaster)
                    threads = self.runner_threads()
                    writers = json.loads(self.sql(engine, WRITERS))
                    arm.h.write_json(output / "scheduler-observation.json", dict(
                        ready=self.ready_threads, terminal=threads, writers=writers,
                        postgres_pid=self.postmaster, clock_ticks=os.sysconf("SC_CLK_TCK"),
                        postgres_before=self.pg_before, postgres_after=after,
                        readback_completed_us=time.time_ns() // 1000))
                return result

            def runner_threads(self):
                rows = {}
                for process in self.processes:
                    _, startup = identity.process_stamp(process)
                    record = proc(process.pid)
                    record["cpus"] = sorted(os.sched_getaffinity(process.pid))
                    dense.check(record["start_ticks"] == startup and record["cpus"] == [0, 1],
                                "live scheduler process or CPU mask changed")
                    rows[str(process.pid)] = record
                return rows

        class Subject(arm.Subject):
            def run(self, env, index, log):
                return super().run(runner_env(env, workers), index, log)

            def capture_runners(self, processes, logs):
                super().capture_runners(processes, logs)
                cluster = self.cluster
                cluster.processes = tuple(processes)
                cluster.postmaster = int((cluster.data / "postmaster.pid").read_text().splitlines()[0])
                cluster.ready_threads = cluster.runner_threads()

            @contextmanager
            def warm_producer(self, env, evidence, deadline):
                dense.check(all(env.get(key) == "2" for key in KEYS), "broker scheduler options changed")
                with super().warm_producer(env, evidence, deadline) as enqueue:
                    def before_submit(path, log):
                        self.cluster.pg_before = pg_cpu(self.cluster.postmaster)
                        return enqueue(path, log)

                    yield before_submit

        arm.Cluster, arm.Subject = Cluster, Subject
        start_arm()

    arm.main = measured_arm
    dense.main("d2-on", source, runner, output)


if __name__ == "__main__":
    workers, source, runner, output = sys.argv[1:]
    main(workers, Path(source), Path(runner), Path(output))
