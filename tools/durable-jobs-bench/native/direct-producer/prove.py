#!/usr/bin/env python3
"""Exercise the warm direct broker with the stock migrator on an owned TLS PG."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import sys
import tempfile
import time

from prepare import digest


def prove(args, arm):
    h = arm.h
    evidence = args.evidence
    evidence.mkdir()
    scratch = Path(tempfile.mkdtemp(prefix="producer-proof-", dir=args.state))
    cluster = arm.Cluster(scratch, evidence, 96)
    cluster.runners, cluster.slots, cluster.lanes = 8, 64, 4
    try:
        cluster.start()
        cluster.prepare_database("mesh")
        manifest = json.loads((args.artifact / "mesh.json").read_text())
        stock = Path(manifest["stock_manifest"]).parent / "mesh"
        assert digest(stock) == manifest["stock_runner_sha256"]
        assert digest(args.artifact / "broker") == manifest["files"]["broker"]
        subject = arm.Subject("mesh", [str(stock)], stock.parent, cluster,
                              dict(cluster.env, JOBS_PARITY_MESH_BATCH="1", JOBS_PARITY_MESH_LISTEN="1"))
        subject.direct_producer = str(args.artifact / "broker")
        subject.fixture_dir = scratch.resolve()
        namespace = "parity_direct_producer"
        env = subject.environment(namespace, 8, 64, "dense", 250)
        subject.call("migrate", env, evidence / "migrate.log")
        settings = json.loads(h.CONFIG.read_text())
        batch, = h.inputs(settings["workloads"]["large"], 64, settings["seed"])
        large_path = scratch / f'{batch["request_id"]}.json'

        def lookup():
            return json.loads(cluster.sql("mesh", f"""
                SELECT coalesce(json_agg(json_build_object(
                    'key', args->>'key', 'id', id::text,
                    'payload', encode(sha256(convert_to(args->>'payload', 'UTF8')), 'hex')
                ) ORDER BY args->>'key'), '[]') AS value
                FROM mesh_jobs.jobs WHERE namespace = '{namespace}'
            """))

        def product():
            return json.loads(cluster.sql("mesh", f"""
                SELECT coalesce(json_agg(request_id ORDER BY request_id), '[]') AS value
                FROM public.mesh_jobs_parity_product WHERE engine = 'mesh' AND namespace = '{namespace}'
            """))

        def scope(name):
            directory = evidence / name
            directory.mkdir()
            return subject.warm_producer(env, directory, time.monotonic() + 30)

        def job(key, hold=0):
            return dict(key=key, hold_ms=hold, fail_first=False, payload=f"fixture-{key}")

        with scope("accepted-and-rollback") as send:
            # The filename does not exist even after SESSION_READY. The warm
            # process can only obtain its contents during the marked request.
            assert not large_path.exists()
            created_after_ready_us = time.time_ns() // 1000
            h.write_json(large_path, batch)
            receipt, committed = send(large_path, evidence / "large.log")
            accepted = lookup()
            expected = [
                dict(key=row["key"], id=receipt["ids"][row["key"]],
                     payload=hashlib.sha256(row["payload"].encode()).hexdigest())
                for row in batch["jobs"]
            ]
            expected.sort(key=lambda row: row["key"])
            assert committed is not None and receipt["count"] == len(expected) == 64
            assert accepted == expected and product() == [batch["request_id"]]
            rollback, = h.batch_files(scratch, [dict(request_id="batch_000001", rollback=True,
                                                        jobs=[job("rollback")])])
            abort, abort_commit = send(rollback, evidence / "rollback.log")
            assert abort["count"] == 1 and abort_commit is None
            assert lookup() == accepted and product() == [batch["request_id"]]

        failures = [
            ("invalid-json", "batch_000002", b'{"jobs": [', "EOF while parsing"),
            ("wrong-request", "batch_000003", json.dumps(dict(request_id="batch_555555",
                rollback=False, jobs=[job("wrong")])).encode(), "batch file does not match request ID"),
            # The frozen batch loop writes the first 128 jobs. Validation of
            # the 129th happens inside the same transaction and must roll back.
            ("after-first-chunk", "batch_000004", json.dumps(dict(request_id="batch_000004",
                rollback=False, jobs=[job(f"partial_{i:03d}") for i in range(128)] +
                [job("invalid", -1)])).encode(), "hold_ms must be nonnegative"),
        ]
        for label, request, data, message in failures:
            path = scratch / f"{request}.json"
            path.write_bytes(data)
            try:
                with scope(label) as send:
                    send(path, evidence / f"{label}.log")
            except RuntimeError as error:
                assert message.lower() in str(error).lower(), repr(error)
            else:
                raise AssertionError(f"{label} incorrectly returned an enqueue receipt")
            assert lookup() == accepted and product() == [batch["request_id"]]

        with scope("input-scope") as send:
            alias = scratch / "batch_000005.json"
            alias.symlink_to(large_path)
            for invalid in (Path(large_path.name), alias, evidence / large_path.name):
                try:
                    send(invalid, evidence / "out-of-scope.log")
                except ValueError as error:
                    assert "private trial scratch" in str(error)
                else:
                    raise AssertionError(f"{invalid} incorrectly accepted")

        h.write_json(evidence / "proof.json", {
            "direct_broker_sha256": manifest["files"]["broker"],
            "stock_migrator_sha256": digest(stock),
            "input_created_after_ready_us": created_after_ready_us,
            "input_bytes": large_path.stat().st_size,
            "input_sha256": digest(large_path),
            "accepted": accepted,
            "transactions": cluster.transactions,
            "explicit_rollback_xid": abort["xid"],
            "rejections": [label for label, *_ in failures] + ["relative", "symlink", "outside-scratch"],
            "surviving_product_requests": product(),
        })
    finally:
        try:
            cluster.stop()
            shutil.rmtree(scratch)
        finally:
            arm.redact(evidence, cluster.password)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("adapter", "artifact", "state", "evidence"):
        parser.add_argument(name, type=lambda value: Path(value).resolve())
    args = parser.parse_args()
    os.umask(0o077)
    sys.path.insert(0, str(args.adapter))
    import arm
    prove(args, arm)


if __name__ == "__main__":
    main()
