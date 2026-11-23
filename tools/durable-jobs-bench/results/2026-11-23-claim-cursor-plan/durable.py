#!/usr/bin/env python3
"""Confirm lock boundaries and compare an uninstrumented, durable SQL workload."""
import argparse
import ctypes as C
from contextlib import contextmanager
from itertools import permutations
import json
import os
from pathlib import Path
import select
import subprocess
import tempfile
from time import monotonic, monotonic_ns, process_time_ns

from replay import (Cluster, FIXTURE, ROOT, SEP, TRACE, collect_plans, definition, load_harness,
                    load_inputs, save_payload, sha, wrapper)

PROOF = ROOT / "proof/jobs-parity/linux-regression"
WORKERS = '[{"worker":"parity","version":1}]'
EDGE = Path(__file__).with_name("edge.sql")
END = b"screen_end_of_command\n"


class Usage(C.Structure):
    # struct rusage_info_v0 from the macOS SDK. The CPU fields use Mach ticks here.
    _fields_ = [("uuid", C.c_uint8 * 16), ("user", C.c_uint64), ("system", C.c_uint64),
                ("unused", C.c_uint64 * 6), ("start", C.c_uint64), ("exit", C.c_uint64)]


PROC_USAGE = C.CDLL("/usr/lib/libproc.dylib", use_errno=True).proc_pid_rusage
PROC_USAGE.argtypes = (C.c_int, C.c_int, C.POINTER(Usage))
PROC_USAGE.restype = C.c_int
TIMEBASE = (C.c_uint32 * 2)()
assert C.CDLL(None).mach_timebase_info(C.byref(TIMEBASE)) == 0
NS_PER_TICK = TIMEBASE[0] / TIMEBASE[1]


def usage(pid):
    sample = Usage()
    if PROC_USAGE(pid, 0, C.byref(sample)):
        raise OSError(C.get_errno(), "proc_pid_rusage")
    assert sample.start and not sample.exit, pid
    return sample.start, sample.user, sample.system


def cpu_delta_ns(before, after):
    assert before[0] == after[0]
    return ((after[1] - before[1]) + (after[2] - before[2])) * NS_PER_TICK


def calibrate_clock():
    before = usage(os.getpid())
    start = process_time_ns()
    while process_time_ns() - start < 50_000_000:
        pass
    expected = process_time_ns() - start
    observed = cpu_delta_ns(before, usage(os.getpid()))
    assert abs(observed / expected - 1) < 0.05, (observed, expected)
    return {"mach_timebase": list(TIMEBASE), "reference_cpu_ns": expected, "observed_cpu_ns": round(observed)}


class Session:
    def __init__(self, cluster, role, label, trace=False):
        self.cluster, self.role, self.label, self.trace = cluster, role, label, trace

    def __enter__(self):
        cluster = self.cluster
        self.process = subprocess.Popen(cluster.psql("arm"), env=dict(cluster.env, PGAPPNAME=self.label),
                                        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        try:
            self.call((TRACE if self.trace else "") + f"SET ROLE {self.role};")
            self.pid = int(self.call("SELECT pg_backend_pid();"))
        except BaseException:
            self.close()
            raise
        return self

    def close(self):
        self.process.stdin.close()
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait(timeout=5)
        self.process.stdout.close()
        self.process.stderr.close()

    def __exit__(self, *_):
        self.close()

    def call(self, sql):
        self.process.stdin.write(sql.encode() + b"\n\\echo " + END)
        self.process.stdin.flush()
        output = bytearray()
        deadline = monotonic() + 20
        while not output.endswith(END):
            readable, _, _ = select.select([self.process.stdout], [], [], max(0, deadline - monotonic()))
            if not readable:
                raise TimeoutError(sql[:150])
            chunk = os.read(self.process.stdout.fileno(), 1 << 20)
            if not chunk:
                raise RuntimeError(self.process.stderr.read().decode())
            output.extend(chunk)
        return output[:-len(END)].decode().rstrip("\n")

    def begin(self):
        assert self.call("BEGIN ISOLATION LEVEL READ COMMITTED;") == "BEGIN"

    def commit(self):
        # psql only emits this tag after libpq received COMMIT and ReadyForQuery.
        assert self.call("COMMIT;") == "COMMIT"


def install_app(cluster, schema):
    cluster.sql("postgres", "CREATE ROLE compiled_definer NOLOGIN; CREATE ROLE compiled_caller NOLOGIN;")
    cluster.sql("base", f"SET ROLE parity_subject;\n{schema}\n{(PROOF / 'fence.sql').read_text()}")
    cluster.sql("base", (PROOF / "application.sql").read_text())


def fetch(session, ns, slots, cutoff=""):
    sql = f"EXECUTE claim('{ns}', 'default', '{WORKERS}', '60000', '{slots}', '{cutoff}', 'false');"
    rows = []
    for record in session.call(sql).splitlines():
        status, id_, token, context, timeout, size, observed, ordinal = record.split(SEP)
        assert not observed and int(ordinal) == len(rows) + 1
        if status == "claimed":
            assert token and int(timeout) == 60000 and int(size) == len(context.encode())
        else:
            assert status == "oversized" and int(size) > 16 * 1024**2 and not (token or context or timeout)
        rows.append((status, id_, token, context, int(size)))
    return rows


def lockable(cluster, namespace, position):
    # Separate connection proves the lock footprint while the claimant remains open.
    value = cluster.sql("arm", f"""
SET ROLE parity_subject;
SELECT count(*) FROM (
    SELECT id FROM mesh_jobs.jobs WHERE namespace = '{namespace}' AND id = '{position:032x}'::uuid
    FOR NO KEY UPDATE SKIP LOCKED
) AS probe;
""").splitlines()[-1]
    return value == "1"


def normalize(rows):
    return [{"status": status, "position": int(id_.replace("-", ""), 16),
             "context_sha256": sha(context.encode()) if context else None, "context_bytes": size}
            for status, id_, _, context, size in rows]


@contextmanager
def arm(cluster, template, variant):
    cluster.clone("arm", template)
    try:
        cluster.sql("arm", "SET ROLE parity_subject;\n" + definition(variant))
        cluster.warm("arm")
        yield
    finally:
        cluster.drop("arm")


def edges(cluster):
    cluster.clone("edge", "base")
    cluster.sql("edge", EDGE.read_text())
    results = {}
    for variant in ("stock", "limit20", "limit_remaining"):
        with arm(cluster, "edge", variant), Session(cluster, "parity_subject", "edge") as jobs:
            assert jobs.call(wrapper()) == "PREPARE"
            jobs.begin()
            rows = fetch(jobs, "nonfit", 8)
            assert [status for status, *_ in rows] == ["claimed", "claimed"]
            assert not lockable(cluster, "nonfit", 3) and lockable(cluster, "nonfit", 4)
            jobs.commit()
            assert lockable(cluster, "nonfit", 3)
            jobs.begin()
            parked = fetch(jobs, "markers", 1)
            assert [status for status, *_ in parked] == ["oversized"] * 4
            assert lockable(cluster, "markers", 5)
            jobs.commit()
            result = cluster.sql("arm", """SELECT count(*) FROM mesh_jobs.batch_context_blocks
                WHERE namespace = 'markers' AND pg_xact_commit_timestamp(xmin) IS NOT NULL;""")
            assert result == "4\n"
            jobs.begin()
            follow = fetch(jobs, "markers", 1)
            jobs.commit()
            result = {"nonfit": normalize(rows), "markers": normalize(parked), "next": normalize(follow),
                      "first_nonfit_locked": True, "next_unvisited_free": True, "marker_next_free": True}
            assert [r["position"] for r in result["nonfit"]] == [1, 2]
            assert [r["position"] for r in result["markers"]] == [1, 2, 3, 4]
            assert [r["position"] for r in result["next"]] == [5]
            results[variant] = result
    assert results["stock"] == results["limit20"] == results["limit_remaining"]
    cluster.drop("edge")
    return results


def effects(session, id_, token, key):
    session.begin()
    assert session.call(f"SELECT compiled_demo.first('probe','{id_}',1,1,'{token}',"
                        f"'{key}',(extract(epoch FROM clock_timestamp())*1000000)::bigint,"
                        "pg_backend_pid()::bigint,false);") == "t"
    session.commit()
    session.begin()
    assert session.call(f"SELECT compiled_demo.second('probe','{id_}',1,1,'{token}','{key}',false);") == "t"
    session.commit()


def reconcile(cluster):
    return int(cluster.sql("arm", """
        SELECT count(*) FROM mesh_jobs.jobs j
        JOIN public.mesh_jobs_parity_attempts a ON (a.namespace,a.job_id,a.job_run) = (j.namespace,j.id,j.run)
        JOIN public.mesh_jobs_parity_effects e ON (e.namespace,e.job_id,e.job_run) = (j.namespace,j.id,j.run)
        WHERE j.state = 'completed' AND j.attempt = 1 AND a.attempt = 1 AND e.attempt = 1
          AND a.job_key = j.args->>'key' AND e.job_key = a.job_key
          AND a.effect_key = mesh_jobs.effect_key(j) AND e.effect_key = a.effect_key
          AND pg_xact_commit_timestamp(a.xmin) <= pg_xact_commit_timestamp(e.xmin)
          AND pg_xact_commit_timestamp(e.xmin) <= pg_xact_commit_timestamp(j.xmin)
          AND a.xid < e.xid;
"""))


def drain(cluster, payload, label, trace=False):
    with (Session(cluster, "parity_subject", label, trace=trace) as jobs,
          Session(cluster, "compiled_caller", "app") as app):
        assert jobs.call(wrapper()) == "PREPARE"
        # The ordered and fully committed workload starts with five 1-slot
        # turns, then drains 8 at a time on the same reusable claimant session.
        # This exercises cache reuse without any aborted tuple versions.
        before = {s.pid: usage(s.pid) for s in (jobs, app)}
        start = monotonic_ns()
        n = 0
        turns = []
        served = []
        for turn in range(13):
            slots = 1 if turn < 5 else min(8, len(payload) - n)
            assert slots > 0
            if trace:
                jobs.call(f"SET application_name = '{label}_{turn}';")
            jobs.begin()
            rows = fetch(jobs, "probe", slots)
            assert len(rows) == slots
            turns.append(len(rows))
            for status, id_, _, context, _ in rows:
                assert status == "claimed" and int(id_.replace("-", ""), 16) == n + len(served) + 1
                body = json.loads(context)
                assert body["id"] == id_ and body["attempt"] == 1 and body["args"] == payload[n + len(served)]
                served.append(body["args"]["key"])
            jobs.commit()
            for (_, id_, token, _, _), key in zip(rows, served, strict=True):
                effects(app, id_, token, key)
            jobs.begin()
            ids = ", ".join(f"('{id_}'::uuid,'{token}'::uuid)" for _, id_, token, *_ in rows)
            finished = jobs.call("SELECT count(*) FILTER (WHERE mesh_jobs.settle("
                                 "'probe',p.id,p.token,'{\"action\":\"complete\"}',false))"
                                 f" FROM (VALUES {ids}) AS p(id,token);")
            assert int(finished) == len(rows)
            jobs.commit()
            n += len(rows)
            served.clear()
        wall_ns = monotonic_ns() - start
        after = {s.pid: usage(s.pid) for s in (jobs, app)}
        cpu = {}
        for scope, session in (("jobs_backend", jobs), ("app_backend", app)):
            cpu[scope + "_ms"] = round(cpu_delta_ns(before[session.pid], after[session.pid]) / 1e6, 3)
        assert n == len(payload) == reconcile(cluster)
        return {"wall_ms": round(wall_ns / 1e6, 3), "cpu": cpu, "turns": turns,
                "settled_jobs": n, "committed_application_records": n * 2}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pg-bin", type=Path, required=True)
    parser.add_argument("--output", type=Path, default=ROOT / "target/jobs-claim-cursor-plan")
    args = parser.parse_args()
    output = args.output.resolve()
    with (tempfile.TemporaryDirectory(dir=output, prefix="d") as path,
          tempfile.TemporaryDirectory(dir=ROOT / "target", prefix=".claim-s-") as socket_dir):
        cluster = Cluster(Path(path), Path(socket_dir), args.pg_bin.resolve())
        try:
            cluster.start()
            report = {"cpu_clock_control": calibrate_clock(), "lock_and_marker_edges": edges(cluster)}
            print("Nonfit and marker lock witnesses passed in all three variants", flush=True)
            harness, _ = load_harness()
            install_app(cluster, harness.SCHEMA)
            cluster.clone("large", "base")
            payload = load_inputs()["large"]
            payload_file = cluster.work / "large.sql"
            save_payload(payload_file, payload)
            cluster.sql("large", FIXTURE.read_text(), variables={
                "capacity": 64, "payload_file": payload_file, "done": 0, "running": 0,
                "is_sparse": "false", "mixed": "false"})
            report["rounds"] = []
            variants = {"stock_a": "stock", "limit20": "limit20", "stock_b": "stock"}
            for order in permutations(variants):
                group = {"order": order, "load_average_before": os.getloadavg()[0], "measurements": {}}
                for label in order:
                    with arm(cluster, "large", variants[label]):
                        group["measurements"][label] = drain(cluster, payload, "timing")
                report["rounds"].append(group)
                print("Confirmed 64 jobs in each arm of", ", ".join(order), flush=True)
            for label in variants:
                with arm(cluster, "large", variants[label]):
                    drain(cluster, payload, "probe_" + label, trace=True)
        finally:
            cluster.stop()
        plans = collect_plans(cluster.data / "log/screen.csv")
        assert len(plans) == 39 and all("cursor" in entry for entry in plans.values()), plans.keys()
        report["trace_only_plans"] = {k: v["cursor"] for k, v in plans.items()}
        report["effect_fence_sha256"] = sha((PROOF / "fence.sql").read_bytes())
        report["application_sql_sha256"] = sha((PROOF / "application.sql").read_bytes())
        report["postgres_binary_sha256"] = sha((cluster.bindir / "postgres").read_bytes())
        (output / "durable.json").write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    main()
