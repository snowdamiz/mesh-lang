"""Inspect physical storage for the released dense payloads on PostgreSQL 17.11."""
import argparse
import gzip
import json
import sys
from pathlib import Path
from tempfile import TemporaryDirectory

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "2026-11-25-dense-same-page"))
from prove import BATCH, INPUT, ROOT, batch_sql, frozen_sql, run, sha256

PHYSICAL = """
WITH tuples AS (
    SELECT h.lp_len, octet_length(h.t_attrs[8]) AS args_bytes
    FROM generate_series(0, pg_relation_size('{table}')::int /
        current_setting('block_size')::int - 1) AS pages(n)
    CROSS JOIN LATERAL heap_page_item_attrs(get_raw_page('{table}', n),
        '{table}'::regclass, false) h
    WHERE h.lp_flags = 1
)
SELECT json_build_object('rows', count(*), 'args_min', min(args_bytes),
    'args_max', max(args_bytes), 'tuple_min', min(lp_len), 'tuple_max', max(lp_len),
    'heap_bytes', pg_relation_size('{table}')) FROM tuples
"""


def inspect(bindir):
    payload = gzip.decompress(BATCH.read_bytes())
    assert sha256(sha256(payload).hexdigest().encode()).hexdigest() == INPUT
    jobs = json.loads(payload)["jobs"]
    assert len(jobs) == 1024 and all(len(job["payload"]) == 1024 for job in jobs)
    with TemporaryDirectory(dir=ROOT / "target", prefix="dt-") as tmp:
        scratch = Path(tmp)
        data = scratch / "pg"
        ctl = [bindir / "pg_ctl", "-D", data, "-w"]
        run([bindir / "initdb", "-D", data, "-A", "trust", "--no-locale", "-E", "UTF8", "-U", "owner"])
        run([*ctl, "-l", scratch / "pg.log", "-o", f"-c listen_addresses='' -k {scratch}", "start"])
        try:
            def query(sql):
                return run([bindir / "psql", "-XqAt", "-v", "ON_ERROR_STOP=1",
                            "-h", scratch, "-U", "owner", "-d", "postgres"], input=sql)

            assert query("SHOW server_version_num") == "170011"
            query("CREATE EXTENSION pageinspect;\n" + "\n".join(frozen_sql(name) for name in
                  ("schema.sql", "enqueue_schema.sql", "drain_schema.sql", "cron_schema.sql", "effect_fence_schema.sql")))
            guard = "SELECT mesh_jobs.enqueue_batch_layout_v1()"
            assert query(guard) == "t"
            batches = [batch_sql(jobs[i:i + 128]) for i in range(0, len(jobs), 128)]
            counts = query("BEGIN;\n" + "\n".join(
                f"SELECT count(*) FROM {batch} WHERE ordinal::int > 0;" for batch in batches) + "\nCOMMIT;")
            assert counts.splitlines() == ["128"] * 8
            assert query(f"BEGIN; ALTER TABLE mesh_jobs.jobs SET (toast_tuple_target=512); {guard}; ROLLBACK;") == "f"
            observations = {}
            for suffix, options in (("stock", ""), ("low", " WITH (toast_tuple_target=512)")):
                table = "public.physical_" + suffix
                query(f"CREATE TABLE {table} (LIKE mesh_jobs.jobs){options}; "
                      f"INSERT INTO {table} SELECT * FROM mesh_jobs.jobs ORDER BY position")
                dense = json.loads(query(PHYSICAL.format(table=table)))
                toast = query(f"SELECT reltoastrelid::regclass FROM pg_class WHERE oid='{table}'::regclass")
                dense["toast_chunks"] = int(query(f"SELECT count(*) FROM {toast}"))
                # An incompressible argument above the normal TOAST threshold exercises
                # the same table and raw-page reader with an external datum.
                query(f"TRUNCATE {table}; INSERT INTO {table} SELECT * FROM mesh_jobs.jobs LIMIT 1; "
                      f"UPDATE {table} SET args=(SELECT to_jsonb(string_agg(md5(n::text), '' ORDER BY n)) "
                      "FROM generate_series(1,256) n)")
                positive = int(query(f"SELECT count(*) FROM {toast}"))
                assert positive > 0
                observations[suffix] = {"dense": dense, "positive_control_chunks": positive}
            return observations
        finally:
            run([*ctl, "-m", "fast", "stop"])


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pg-bin", required=True, type=Path)
    print(json.dumps(inspect(parser.parse_args().pg_bin), indent=2, sort_keys=True))
