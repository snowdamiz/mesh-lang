"""Compare real PostgreSQL plans for the two Jobs due queries."""

import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
BASE = "3f69b150820fd01ba716d533be4f2eedea25ed27"
EPOCH = 5_000_000

spec = importlib.util.spec_from_file_location("parity", ROOT / "scripts/jobs-parity.py")
parity = importlib.util.module_from_spec(spec)
spec.loader.exec_module(parity)


def read_query(revision):
    source = subprocess.check_output(["git", "show", f"{revision}:compiler/mesh-rt/src/jobs/store.rs"],
                                     cwd=ROOT, text=True)
    return source.split('"WITH clock AS MATERIALIZED (SELECT mesh_jobs.now_ms()', 1)[1].split('"', 1)[0]


def probe(cluster, tail):
    query = 'WITH clock AS MATERIALIZED (SELECT mesh_jobs.now_ms()' + tail
    prepared = f"PREPARE due (text, text, text) AS {query};"
    execute = "EXECUTE due ('witness', 'jobs', '[{\"worker\":\"w\",\"version\":1}]')"
    value = cluster.sql("mesh", prepared + execute)
    plan = json.loads(cluster.sql("mesh", prepared + "EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) " + execute))
    return {"delay_ms": int(value) if value else None, "plan": plan[0]["Plan"]}


def run(cluster):
    schema = (ROOT / "compiler/mesh-rt/src/jobs/schema.sql").read_text()
    cluster.prepare_database("mesh")
    cluster.sql("mesh", schema)
    cluster.sql("mesh", f"""
       CREATE OR REPLACE FUNCTION mesh_jobs.now_ms() RETURNS bigint
       LANGUAGE sql VOLATILE SET search_path = pg_catalog, pg_temp AS $$ SELECT {EPOCH}::bigint $$;
       INSERT INTO mesh_jobs.queues(namespace, name, concurrency) VALUES ('witness', 'jobs', 64);
       INSERT INTO mesh_jobs.jobs (namespace, worker, version, queue, args, metadata, tags, state,
           priority, max_attempts, timeout_ms, backoff_base_ms, backoff_max_ms, inserted_at_ms, scheduled_at_ms)
       SELECT 'witness', 'w', 1, 'jobs', '{{}}', '{{}}', '[]', 'available', 0, 3, 1000, 0, 0,
           {EPOCH} - 1, {EPOCH} - 1 FROM generate_series(1, 1024);
       ANALYZE mesh_jobs.jobs;
    """)
    queries = {"baseline": read_query(BASE), "candidate": read_query((HERE / "revision.txt").read_text().strip())}
    ready = {name: probe(cluster, query) for name, query in queries.items()}
    cluster.sql("mesh", f"""
       INSERT INTO mesh_jobs.jobs (namespace, worker, version, queue, args, metadata, tags, state,
           priority, max_attempts, timeout_ms, backoff_base_ms, backoff_max_ms, inserted_at_ms, scheduled_at_ms)
       VALUES ('witness', 'ineligible', 1, 'jobs', '{{}}', '{{}}', '[]', 'scheduled', 0, 3, 1000, 0, 0,
           {EPOCH}, {EPOCH} + 500),
           ('witness', 'w', 1, 'jobs', '{{}}', '{{}}', '[]', 'retryable', 0, 3, 1000, 0, 0,
           {EPOCH}, {EPOCH} + 2000),
           ('witness', 'w', 1, 'jobs', '{{}}', '{{}}', '[]', 'scheduled', 0, 3, 1000, 0, 0,
           {EPOCH}, {EPOCH} + 3000);
       ANALYZE mesh_jobs.jobs;
    """)
    future = {name: probe(cluster, query) for name, query in queries.items()}
    assert all(result["delay_ms"] is None for result in ready.values()), ready
    assert all(result["delay_ms"] == 2000 for result in future.values()), future
    return {"version": cluster.sql("mesh", "SHOW server_version_num"), "ready_only": ready,
            "with_future_work": future}


if __name__ == "__main__":
    with tempfile.TemporaryDirectory(dir=HERE) as temporary:
        scratch = Path(temporary)
        evidence = scratch / "logs"
        evidence.mkdir()
        cluster = parity.Cluster(scratch, evidence, 96, "17.11")
        try:
            cluster.start()
            print(json.dumps(run(cluster), indent=2, sort_keys=True))
        finally:
            cluster.stop()
