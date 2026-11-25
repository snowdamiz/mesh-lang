"""Drive the immutable logger-off adapter with one or two public dispatchers."""
from contextlib import contextmanager
import hashlib
import inspect
import json
import os
from pathlib import Path
import sys
from textwrap import dedent
import time

BOUNDS = ("MESH_SCHEDULER_MIN_WORKERS", "MESH_SCHEDULER_MAX_WORKERS")
SESSION_SQL = """
SELECT json_build_object(
  'applications', (SELECT coalesce(json_agg(json_build_object(
      'backend', a.pid, 'tls', coalesce(s.ssl, false),
      'started_us', (extract(epoch FROM a.query_start) * 1000000)::bigint,
      'wrote', ltrim(a.query) ~ '^INSERT INTO public[.]mesh_jobs_parity_(attempts|effects)([[:space:]]|$)'
    ) ORDER BY a.pid), '[]')
    FROM pg_stat_activity a LEFT JOIN pg_stat_ssl s USING (pid)
    WHERE a.datname = current_database() AND a.usename = 'compiled_caller'),
  'queues', (SELECT json_agg(json_build_array(namespace, name, concurrency, state) ORDER BY namespace, name)
    FROM mesh_jobs.queues),
  'claim_body', (SELECT prosrc FROM pg_proc WHERE oid =
    'mesh_jobs.claim_batch_set_v1(text,text,jsonb,bigint,integer,bigint)'::regprocedure),
  'claim_config', (SELECT proconfig FROM pg_proc WHERE oid =
    'mesh_jobs.claim_batch_set_v1(text,text,jsonb,bigint,integer,bigint)'::regprocedure)
)
"""


def process(pid, bounds=None):
    proc = Path("/proc") / str(pid)
    stat = (proc / "stat").read_text().rsplit(")", 1)[1].split()
    row = dict(pid=pid, parent=int(stat[1]), start=int(stat[19]), threads=int(stat[17]),
               cpu_ticks=int(stat[11]) + int(stat[12]))
    if bounds is not None:
        # Retain only these two non-secret values from the live child environment.
        row["launch_bounds"] = bounds
        try:
            values = dict(entry.split(b"=", 1) for entry in (proc / "environ").read_bytes().split(b"\0")
                          if b"=" in entry)
        except PermissionError as error:
            row["environ_errno"] = error.errno
        else:
            row["live_bounds"] = {key: values[key.encode()].decode() for key in BOUNDS}
        row["cpus"] = sorted(os.sched_getaffinity(pid))
    return row


def pg_frame(pid):
    begin = time.time_ns() // 1000
    children = (Path("/proc") / str(pid) / "task" / str(pid) / "children").read_text().split()
    rows = []
    for child in (pid, *map(int, children)):
        try:
            rows.append(process(child))
        except FileNotFoundError:
            if child == pid:
                raise
    return dict(begin_us=begin, end_us=time.time_ns() // 1000, rows=rows)


def runner_env(env, runners, workers):
    if any(env.get(key) != "2" for key in BOUNDS):
        raise ValueError("frozen scheduler defaults changed")
    if (env["JOBS_PARITY_SLOTS"], env["MESH_EFFECT_LANES"]) != (str(64 // runners), str(32 // runners)):
        raise ValueError("unexpected dispatcher allocation")
    return {**env, **dict.fromkeys(BOUNDS, str(workers))}


def install(runners, workers, stock):
    # The staged directory is searchable but cannot be listed by nobody.
    # This stock directory already supports the original unprivileged imports.
    sys.path.insert(0, str(stock))
    import adapter as frozen

    arm = frozen.arm
    frozen.install()
    _, slots, _, cap, memory, digest = arm.PROFILES["noop"]
    if (slots, cap) != (64, 96):
        raise ValueError("frozen dense slot or connection cap changed")
    arm.PROFILES["noop"] = (runners, slots, 32 // runners, cap, memory, digest)

    # Hold the original worker-role limit as well as the physical DB limit.
    prepare = arm.regression.Cluster.prepare_database
    source = frozen.replace_once(dedent(inspect.getsource(prepare)), "self.runners * 4", "8")
    parent = arm.regression.Cluster
    source = frozen.replace_once(source, "super().prepare_database", "super(_frozen_parent, self).prepare_database")
    scope = dict(prepare.__globals__, _frozen_parent=parent)
    exec(compile(source, prepare.__code__.co_filename, "exec"), scope)
    arm.regression.Cluster.prepare_database = scope["prepare_database"]

    class Cluster(frozen.Cluster):
        counter_reads = 0

        def scheduler_frame(self):
            return [process(pid, bounds) for pid, bounds in self.launch_bounds.items()]

        def calls(self, engine):
            self.counter_reads += 1
            if self.counter_reads != 3:
                return super().calls(engine)
            after = pg_frame(self.postmaster)
            terminal = self.scheduler_frame()
            result = super().calls(engine)
            audit = json.loads(arm.h.Cluster.sql(self, engine, SESSION_SQL))
            arm.h.write_json(self.evidence / "dispatcher-observation.json", dict(
                ready=self.ready, terminal=terminal,
                postgres_pid=self.postmaster, clock_ticks=os.sysconf("SC_CLK_TCK"),
                before=self.before, after=after, **audit))
            return result

    class Subject(frozen.Subject):
        def run(self, env, index, log):
            selected = runner_env(env, runners, workers)
            child = super().run(selected, index, log)
            child.scheduler_bounds = {key: selected[key] for key in BOUNDS}
            return child

        def capture_runners(self, processes, logs):
            super().capture_runners(processes, logs)
            db = self.cluster
            db.postmaster = int((db.data / "postmaster.pid").read_text().splitlines()[0])
            db.launch_bounds = {self.broker_process[0].pid: self.broker_bounds}
            db.launch_bounds.update({child.pid: child.scheduler_bounds for child in processes})
            db.ready = db.scheduler_frame()

        @contextmanager
        def warm_producer(self, env, evidence, deadline):
            if any(env.get(key) != "2" for key in BOUNDS):
                raise ValueError("broker scheduler bounds changed")
            self.broker_bounds = {key: env[key] for key in BOUNDS}
            with super().warm_producer(env, evidence, deadline) as enqueue:
                def record(path, log):
                    self.cluster.before = pg_frame(self.cluster.postmaster)
                    return enqueue(path, log)
                yield record

    arm.Cluster, arm.Subject = Cluster, Subject
    return arm


def main():
    runners, workers = map(int, sys.argv[1:3])
    if ((runners, workers) not in {(2, 2), (1, 2), (1, 4)}
            or sys.argv[3:5] != ["mesh", "noop"] or os.getuid() != 65534
            or os.sched_getaffinity(0) != {0, 1}
            or any(key in os.environ for key in BOUNDS)):
        raise ValueError("unexpected topology, privilege or CPU envelope")
    artifact = Path(os.environ["NATIVE_JOBS_ARTIFACTS"])
    manifest = (artifact / "mesh.json").read_bytes()
    if (hashlib.sha256(manifest).hexdigest() != os.environ["PRODUCT_MANIFEST_SHA256"]
            or json.loads(manifest)["files"]["mesh"] != os.environ["PRODUCT_ELF_SHA256"]):
        raise ValueError("immutable logger-off manifest differs")
    os.environ.update(TRUSTED_ADMISSION_POLICY="set_based", MESH_JOBS_SET_BASED_ENQUEUE="1")
    arm = install(runners, workers, Path(os.environ["NATIVE_JOBS_STATE"]))
    if sys.argv[6:] == ["--stage-check"]:
        print(json.dumps(dict(uid=os.getuid(), cpus=sorted(os.sched_getaffinity(0)),
                              profile=arm.PROFILES["noop"], runner_workers=workers, broker_workers=2)))
        return
    if len(sys.argv) != 6:
        raise ValueError("unexpected launcher arguments")
    sys.argv = [sys.argv[0], *sys.argv[3:]]
    arm.main()


if __name__ == "__main__":
    main()
