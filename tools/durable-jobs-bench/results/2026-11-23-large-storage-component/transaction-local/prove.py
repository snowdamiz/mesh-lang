import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path.cwd()
loader = importlib.util.spec_from_file_location("parity", ROOT / "scripts/jobs-parity.py")
parity = importlib.util.module_from_spec(loader)
loader.loader.exec_module(parity)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("binary", type=Path)
    parser.add_argument("evidence", type=Path)
    args = parser.parse_args()
    binary, out = args.binary.resolve(), args.evidence.resolve()
    out.mkdir()
    with tempfile.TemporaryDirectory(dir=ROOT / "target", prefix="storage-scope-") as temp:
        scratch = Path(temp)
        db = parity.Cluster(scratch, out, 8, expected_version="17.11")
        try:
            db.start()
            db.sql("postgres", "CREATE DATABASE storage OWNER parity_subject")
            db.sql("storage", "SET ROLE parity_subject; CREATE TABLE storage_products(key text PRIMARY KEY)")
            payload = ("søme Unicode, punctuation, and repeated fields. " * 4096)
            source = scratch / "input.txt"
            source.write_text(payload)
            env = dict(os.environ, MESH_STORAGE_OWNER_URL=db.url("storage"),
                       MESH_STORAGE_PRODUCER_URL=db.url("storage", "parity_producer"),
                       MESH_STORAGE_DATA=str(source))
            proc = subprocess.run([binary], env=env, capture_output=True, text=True, timeout=30)
            if proc.returncode:
                raise RuntimeError(proc.stderr.replace(db.password, "[redacted]")[:1200])
            printed = [json.loads(line) for line in proc.stdout.splitlines()]
            assert [(r["key"], r["policy"], r["restored"], r["rollback"]) for r in printed] == [
                ("default", "pglz", "pglz", False), ("local-rollback", "lz4", "pglz", True),
                ("local-commit", "lz4", "pglz", False), ("default-again", "pglz", "pglz", False)]
            rows = json.loads(db.sql("storage", """SELECT json_agg(row_to_json(s) ORDER BY key) FROM (
                SELECT j.args->>'key' AS key, pg_column_compression(j.args) AS codec,
                    pg_column_size(j.args) AS bytes, length(j.args->>'payload') AS characters,
                    encode(sha256(convert_to(j.args->>'payload','UTF8')),'hex') AS sha256,
                    j.xmin::text AS job_xid, p.xmin::text AS product_xid, j.id::text,
                    pg_xact_commit_timestamp(j.xmin) AS job_commit,
                    pg_xact_commit_timestamp(p.xmin) AS product_commit
                FROM mesh_jobs.jobs j FULL JOIN storage_products p ON p.key=j.args->>'key'
            ) s"""))
            assert [r["key"] for r in rows] == ["default", "default-again", "local-commit"]
            assert [r["codec"] for r in rows] == ["pglz", "pglz", "lz4"]
            ids = {r["key"]: json.loads(r["receipt"])["id"] for r in printed}
            transaction = {r["key"]: r["xid"] for r in printed}
            digest = hashlib.sha256(payload.encode()).hexdigest()
            assert len({r["job_xid"] for r in rows}) == 3
            assert all(r["sha256"] == digest and r["characters"] == len(payload)
                       and r["id"] == ids[r["key"]] and r["product_xid"] == transaction[r["key"]]
                       and r["job_commit"] and r["job_commit"] == r["product_commit"] for r in rows)
            assert db.sql("storage", """SELECT a.attstorage='x' AND a.attcompression=''::"char"
                         AND a.atttypid='jsonb'::regtype FROM pg_attribute a
                         WHERE a.attrelid='mesh_jobs.jobs'::regclass AND a.attname='args'""") == "t"
            versions = """SELECT json_agg(ARRAY[j.ctid::text,j.xmin::text,
                pg_column_compression(args)::text] ORDER BY id) FROM mesh_jobs.jobs j"""
            before = json.loads(db.sql("storage", versions))
            observed = []
            for mode, code in (("EXTERNAL", "e"), ("EXTENDED", "x")):
                db.sql("storage", f"SET ROLE parity_subject; ALTER TABLE mesh_jobs.jobs ALTER COLUMN args SET STORAGE {mode}")
                proc = subprocess.run([binary], env=dict(env, MESH_STORAGE_MIGRATE_ONLY="yes"), capture_output=True,
                                      text=True, timeout=30)
                assert proc.returncode == 0, proc.stderr.replace(db.password, "[redacted]")[:1200]
                stored = db.sql("storage", """SELECT attstorage FROM pg_attribute
                    WHERE attrelid='mesh_jobs.jobs'::regclass AND attname='args'""")
                assert stored == code and json.loads(db.sql("storage", versions)) == before
                observed.append(stored)
            parity.write_json(out / "receipt.json", dict(binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                              server_version=db.sql("storage", "SHOW server_version"), fresh=tls_state(db),
                              produced=printed, committed=rows, storage_after_remigration=observed,
                              existing_tuple_versions_and_codecs=before, input_utf8_bytes=len(payload.encode())))
        finally:
            db.stop()


def tls_state(db):
    return json.loads(db.sql("storage", """SELECT json_build_object(
        'tls', (SELECT ssl FROM pg_stat_ssl WHERE pid=pg_backend_pid()),
        'available_codecs', (SELECT enumvals FROM pg_settings WHERE name='default_toast_compression'),
        'fsync', current_setting('fsync'), 'synchronous_commit', current_setting('synchronous_commit')
    )"""))


if __name__ == "__main__":
    main()
