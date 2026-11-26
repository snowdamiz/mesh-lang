#!/usr/bin/env python3
"""Run the exact compiled V1 producer and worker on a disposable PostgreSQL 17.11 arm."""
import argparse
from contextlib import contextmanager
import gzip
import json
import os
from pathlib import Path
import subprocess
import sys
import time

import cpu_sampler
import matrix_verify
import storage

HERE = Path(__file__).resolve().parent
PROFILE = "slow"
POLICIES = {
    "stock-a": ("pglz", "default", "1"),
    "stock-b": ("pglz", "default", "1"),
    "lz4": ("lz4", "default", "1"),
    "scalar": ("pglz", "default", "0"),
    "external": ("pglz", "external", "1"),
    "column-lz4": ("pglz", "column-lz4", "1"),
}


def batch_data(parity, control):
    settings = json.loads(parity.CONFIG.read_text())
    batches = parity.inputs(settings["workloads"][PROFILE], 64, settings["seed"])
    if control:
        for batch in batches:
            for job in batch["jobs"]:
                job["payload"] = "abcd" * 4096
    encoded = [(json.dumps(batch, indent=2, sort_keys=True, allow_nan=False) + "\n").encode()
               for batch in batches]
    digest = storage.sha("".join(map(storage.sha, encoded)).encode())
    return batches, encoded, digest


def sampler_anchor(executable):
    # The warm session is already ready, and its runner has yet to launch.
    children = subprocess.check_output(["ps", "-ax", "-o", "pid=,ppid=,command="], text=True)
    pids = [int(pid) for line in children.splitlines()
            for pid, parent, command in (line.split(None, 2),)
            if int(parent) == os.getpid() and command == f"{executable} session"]
    if len(pids) != 1:
        raise ValueError(f"expected one compiled producer child, found {pids}")
    return pids[0]


def run(work, out, label, control):
    launcher = work / "base/tools/durable-jobs-bench/native"
    sys.path.insert(0, str(launcher))
    import arm

    policy, column_policy, enabled = POLICIES[label]
    os.environ["MESH_JOBS_SET_BASED_ENQUEUE"] = enabled
    artifacts = work / "artifacts-v1"
    executable = artifacts / "mesh"
    parity = arm.h
    batches, encoded, case = batch_data(parity, control)
    frozen = HERE.parent / "frozen" / ("compressible-batch.json.gz" if control else "canonical-batch.json.gz")
    if b"".join(encoded) != gzip.decompress(frozen.read_bytes()):
        raise ValueError("run input differs from the packaged bytes")
    original = arm.PROFILES[PROFILE]
    if not control and case != original[-1]:
        raise ValueError("canonical slow input drifted")
    arm.PROFILES[PROFILE] = (*original[:-1], case)
    if control:
        parity.inputs = lambda *_: batches
    cpu_samples = {}
    raw_atoms = {}

    class Cluster(arm.Cluster):
        def configure_tls(self):
            super().configure_tls()
            if policy == "lz4":
                with (self.data / "postgresql.conf").open("a") as stream:
                    stream.write("default_toast_compression = 'lz4'\n")

        def reconcile_trial(self, engine, namespace, enqueued_ids, handler_attempts, commits):
            result = super().reconcile_trial(engine, namespace, enqueued_ids, handler_attempts, commits)
            raw_atoms["persisted"] = json.loads(self.sql(engine, storage.snapshot(engine)))
            raw_atoms["statements"] = json.loads(self.sql(engine, """
                SELECT jsonb_agg(to_jsonb(w) ORDER BY w.query) FROM (
                  SELECT query, calls, rows, total_exec_time, wal_bytes
                  FROM pg_stat_statements
                  WHERE dbid = (SELECT oid FROM pg_database WHERE datname = current_database())
                    AND userid = 'parity_producer'::regrole
                ) w
            """))
            return result

    class Subject(arm.Subject):
        def call(self, action, env, log):
            result = super().call(action, env, log)
            if action == "migrate":
                query = storage.metadata("mesh_jobs.jobs", "args")
                before = json.loads(self.cluster.sql("mesh", query))
                ddl = {
                    "external": "SET STORAGE EXTERNAL",
                    "column-lz4": "SET COMPRESSION lz4",
                }.get(column_policy)
                if ddl:
                    self.cluster.sql("mesh", f"ALTER TABLE mesh_jobs.jobs ALTER COLUMN args {ddl}")
                after = json.loads(self.cluster.sql("mesh", query))
                raw_atoms["setup"] = dict(before=before, after=after, ddl=ddl,
                    count=int(self.cluster.sql("mesh", "SELECT count(*) FROM mesh_jobs.jobs")))
            return result

        @contextmanager
        def warm_producer(self, env, evidence, deadline):
            with super().warm_producer(env, evidence, deadline) as submit:
                sql = """SELECT json_build_object('backend', pg_backend_pid(),
                         'policy', current_setting('default_toast_compression'))"""
                identity = json.loads(self.cluster.sql("mesh", sql))
                producer = sampler_anchor(executable)
                raw_atoms["producer"] = identity
                if identity["policy"] != policy:
                    raise ValueError("producer inherited the wrong TOAST policy")

                def measured(path, log):
                    pg_start = cpu_sampler._ms(identity["backend"])
                    producer_start = cpu_sampler._ms(producer)
                    monotonic_start = time.monotonic_ns()
                    result = submit(path, log)
                    wall_ms = (time.monotonic_ns() - monotonic_start) / 1e6
                    cpu_samples[path.stem] = dict(
                        wall_ms=wall_ms, producer_ms=cpu_sampler._ms(producer) - producer_start,
                        backend_ms=cpu_sampler._ms(identity["backend"]) - pg_start,
                    )
                    return result

                yield measured

    arm.Cluster, arm.Subject = Cluster, Subject
    out.mkdir()
    (work / "state/scratch").mkdir(parents=True, exist_ok=True)
    os.umask(0o077)
    args = argparse.Namespace(local_smoke=True, engine="mesh", profile=PROFILE)
    row, ledger = arm.run_arm(args, out, work / "state", artifacts)
    storage.write(out / "ledger.json", ledger)
    storage.write(out / "row.json", row)
    storage.write(out / "payload-storage.json", raw_atoms)
    storage.write(out / "producer-cpu.json", cpu_samples)
    score_arm(out, label, batches, case, control)


def score_arm(out, label, batches, case, control):
    def read(name):
        return json.loads((out / f"{name}.json").read_text())

    policy, column_policy, enabled = POLICIES[label]
    ledger, raw_atoms, cpu = read("ledger"), read("payload-storage"), read("producer-cpu")
    original = matrix_verify.PROFILES[PROFILE]
    verified_input = matrix_verify.expected_inputs
    if control:
        matrix_verify.PROFILES[PROFILE] = (*original[:3], case, original[-1])
    expected = {
        batch["request_id"]: {job["key"]: (
            storage.sha(job["payload"].encode()), job["hold_ms"], job["fail_first"])
            for job in batch["jobs"]}
        for batch in batches
    }
    matrix_verify.expected_inputs = lambda _: expected
    try:
        score = matrix_verify.reconcile(ledger, "mesh", PROFILE, {})
        if ledger["input_sha256"] != case:
            raise ValueError("wrong frozen batch digest")
    finally:
        matrix_verify.PROFILES[PROFILE] = original
        matrix_verify.expected_inputs = verified_input

    persisted = raw_atoms["persisted"]
    storage.verify_snapshot("mesh", persisted, ledger["jobs"], policy, column_policy)
    if raw_atoms["setup"]["count"] or raw_atoms["setup"]["after"] != persisted["metadata"]:
        raise ValueError("payload layout was populated at setup or changed during the timed arm")
    if raw_atoms["producer"]["policy"] != policy:
        raise ValueError("wrong producer session policy")

    statements = raw_atoms["statements"]
    bulk = [line for line in statements if line["query"].startswith(
        "SELECT ordinal, receipt FROM mesh_jobs.enqueue_batch_v1(")]
    scalar = [line for line in statements if line["query"].startswith("SELECT mesh_jobs.enqueue(")]
    bulk_calls = sum(line["calls"] for line in bulk)
    batch_rows = sum(line["rows"] for line in bulk)
    scalar_calls = sum(line["calls"] for line in scalar)
    scalar_rows = sum(line["rows"] for line in scalar)
    if enabled == "0":
        expected_writer = (0, 0, 512, 512)
    elif column_policy == "default":
        expected_writer = (8, 504, 8, 8)
    else:
        # The presence of a batch-shaped statement alone cannot prove bulk
        # writes. The server returns one stock marker per rejected batch.
        expected_writer = (8, 8, 512, 512)
        if any(line["wal_bytes"] != 0 for line in bulk):
            raise ValueError("layout fallback unexpectedly wrote WAL in the batch statement")
    if (bulk_calls, batch_rows, scalar_calls, scalar_rows) != expected_writer:
        raise ValueError(f"producer writer differs from the selected layout {bulk=} {scalar=}")

    transaction, = ledger["transactions"]
    timing, = cpu.values()
    compressed_rows = sum(row["compression"] is not None for row in persisted["rows"])
    if control and compressed_rows != (0 if column_policy == "external" else 512):
        raise ValueError("compressible control did not exercise the selected physical policy")
    report = dict(label=label, control=control, case_sha256=case, bulk_calls=bulk_calls, bulk_rows=batch_rows,
        scalar_calls=scalar_calls, scalar_rows=scalar_rows, compressed_rows=compressed_rows,
        payload_column_bytes=sum(row["stored_column_bytes"] for row in persisted["rows"]),
        producer_ack_ms=(transaction["acknowledged_us"] - transaction["submitted_us"]) / 1000,
        producer_wall_ms=timing["wall_ms"], producer_cpu_ms=timing["producer_ms"],
        producer_backend_cpu_ms=timing["backend_ms"],
        producer_sql_exec_ms=sum(line["total_exec_time"] for line in bulk + scalar),
        producer_sql_wal_bytes=sum(line["wal_bytes"] for line in bulk + scalar),
        submit_to_last_settlement_ms=(score["last_settlement_us"] - score["first_submit_us"]) / 1000,
        commit_to_last_settlement_ms=(score["last_settlement_us"] - score["first_commit_us"]) / 1000,
        **{name: persisted[name] for name in ("heap_bytes", "toast_heap_bytes", "total_bytes")}, **score)
    storage.write(out / "score.json", report)
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("work", type=Path)
    parser.add_argument("out", type=Path)
    parser.add_argument("label", choices=POLICIES)
    parser.add_argument("--control", action="store_true")
    args = parser.parse_args()
    run(args.work.resolve(), args.out.resolve(), args.label, args.control)
