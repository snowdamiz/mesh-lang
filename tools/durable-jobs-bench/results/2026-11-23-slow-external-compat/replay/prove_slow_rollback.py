#!/usr/bin/env python3
"""Prove rollback and retry using the exact timed 512-job V1 artifact."""
import argparse
import gzip
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

import layout
import storage

HERE = Path(__file__).resolve().parent
FROZEN = HERE.parent / "frozen"


def run(work, out):
    sys.path.insert(0, str(work / "base/tools/durable-jobs-bench/native"))
    import arm

    out.mkdir()
    binary = work / "artifacts-v1/mesh"
    batch = json.loads(gzip.decompress((FROZEN / "canonical-batch.json.gz").read_bytes()))
    pinned = json.loads((FROZEN / "local-pins.json").read_text())["executable_sha256"]
    if arm.h.digest(binary) != pinned:
        raise ValueError("rollback must use the timed binary")
    with tempfile.TemporaryDirectory(dir=work / "tmp", prefix="slow-rollback-") as directory:
        cluster = arm.Cluster(Path(directory), out, 68)
        cluster.trusted_application = True
        cluster.runners, cluster.slots, cluster.lanes, cluster.hold_ms = 1, 64, 8, 100
        engine = "mesh"
        try:
            cluster.start()
            cluster.prepare_database(engine)
            env = dict(cluster.env, MESH_JOBS_SET_BASED_ENQUEUE="1", JOBS_PARITY_MESH_LISTEN="1")
            subject = arm.Subject(engine, [str(binary)], work / "artifacts-v1", cluster, env)
            env = subject.environment("parity_0000000000000001", 64, 64, "burst", 250)
            subject.call("migrate", env, out / "migrate.log")
            witness = layout.install(cluster, engine, external=True, compatible=True)
            layout.check_install(witness, external=True, compatible=True)
            env["JOBS_PARITY_DB_URL"] = cluster.url(engine, "parity_producer")
            probe = Path(directory) / "probe.json"
            statements = """
                SELECT coalesce(jsonb_agg(to_jsonb(w) ORDER BY query), '[]') FROM (
                    SELECT query, calls, rows, wal_bytes FROM pg_stat_statements
                    WHERE dbid = (SELECT oid FROM pg_database WHERE datname = current_database())
                      AND userid = 'parity_producer'::regrole
                      AND (query LIKE 'SELECT ordinal, receipt FROM mesh_jobs.enqueue_batch_v1(%'
                        OR query LIKE 'SELECT mesh_jobs.enqueue(%')
                ) w
            """

            def fire(rollback):
                storage.write(probe, dict(batch, rollback=rollback))
                return json.loads(subprocess.check_output([str(binary), "enqueue"], env=dict(env, JOBS_PARITY_INPUT=str(probe)), timeout=40))

            def counts():
                return json.loads(cluster.sql(engine, """
                    SELECT json_build_array(
                        (SELECT count(*) FROM public.mesh_jobs_parity_product),
                        (SELECT count(*) FROM mesh_jobs.jobs), (SELECT count(*) FROM mesh_jobs.receipts))
                """))

            rolled = fire(True)
            after_rollback = counts()
            writer = json.loads(cluster.sql(engine, statements))
            assert rolled["count"] == 512 and after_rollback == [0, 0, 0]
            assert len(writer) == 2 and [(x["calls"], x["rows"]) for x in writer] == [(8, 8), (8, 504)]
            assert all(x["wal_bytes"] > 0 for x in writer)
            committed = fire(False)
            durable = counts()
            saved = json.loads(cluster.sql(engine, layout.RECEIPTS))
            assert committed["count"] == 512 and durable == [1, 512, 512]
            assert committed["ids"].keys() == rolled["ids"].keys()
            assert set(rolled["ids"].values()).isdisjoint(committed["ids"].values())
            assert {row["request_key"]: row["id"] for row in saved} == committed["ids"]
            storage.write(out / "receipt.json", dict(artifact_sha256=pinned, installer=witness, rolled=rolled,
                after_rollback=after_rollback, rollback_writer=writer, committed=committed,
                durable=durable, saved=saved))
            (out / "PASS").write_text("512 true-batch receipts rolled back and the same input committed once on retry\n")
        finally:
            try:
                cluster.stop()
            finally:
                arm.redact(out, cluster.password)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("work", type=Path)
    parser.add_argument("out", type=Path)
    args = parser.parse_args()
    run(args.work.resolve(), args.out.resolve())
