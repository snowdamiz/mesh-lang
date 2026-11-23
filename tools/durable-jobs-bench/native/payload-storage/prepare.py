"""Stage a storage-policy variant beside a pinned Mesh benchmark adapter."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import tempfile

AFTER_MIGRATION = '''
    def call(self, action, env, log):
        result = super().call(action, env, log)
        if action == "migrate":
            if self.engine != "mesh":
                raise RuntimeError("the storage-policy adapter requires Mesh")
            database = self.cluster
            settings = """SELECT json_build_object(
                'attstorage', a.attstorage, 'attcompression', a.attcompression,
                'column_type', a.atttypid::regtype::text, 'rows', (SELECT count(*) FROM mesh_jobs.jobs))
                FROM pg_attribute a WHERE a.attrelid='mesh_jobs.jobs'::regclass AND a.attname='args'"""
            before = json.loads(database.sql("mesh", settings))
            if before["rows"] or before["column_type"] != "jsonb" or before["attstorage"] != "x":
                raise RuntimeError(f"unexpected initial Jobs storage {before}")
            ddl = "ALTER TABLE mesh_jobs.jobs ALTER COLUMN args SET STORAGE EXTERNAL"
            started = time.monotonic_ns()
            database.sql("mesh", ddl)
            elapsed = time.monotonic_ns() - started
            after = json.loads(database.sql("mesh", settings))
            if after != dict(before, attstorage="e"):
                raise RuntimeError(f"wrong Jobs storage readback {after}")
            h.write_json(database.evidence / "payload-storage-before.json",
                         dict(before=before, after=after, ddl=ddl, ddl_client_ns=elapsed))
        return result
'''

AFTER_RECONCILE = '''
        rows = json.loads(self.sql(engine, """
            SELECT coalesce(json_agg(row_to_json(s) ORDER BY s.id), '[]')
            FROM (
                SELECT id::text, args->>'key' AS job_key, pg_column_size(args) AS stored_bytes,
                       pg_column_compression(args) AS compression, state, attempt,
                       encode(sha256(convert_to(args->>'payload','UTF8')), 'hex') AS payload_sha256
                FROM mesh_jobs.jobs
            ) s
        """))
        expected = {j["id"]: (j["job_key"], j["payload_sha256"]) for j in self.jobs}
        actual = {r["id"]: (r["job_key"], r["payload_sha256"]) for r in rows}
        if (actual != expected or not rows or len(rows) != len(expected)
                or any(r["compression"] is not None or r["state"] != "completed" or r["attempt"] != 1
                       for r in rows)):
            raise RuntimeError("stored payloads did not reconcile to all completed one-attempt jobs")
        h.write_json(self.evidence / "payload-storage-after.json", rows)
'''

def digest(data):
    return hashlib.sha256(data).hexdigest()

def transformed(source):
    markers = [
        ("class Subject(regression.Subject):\n", AFTER_MIGRATION),
        ("            self, engine, namespace, self.transactions, handler_attempts, relation, scope\n        )\n",
         AFTER_RECONCILE),
    ]
    for marker, insert in markers:
        if source.count(marker) != 1:
            raise ValueError(f"expected a single adapter anchor {marker!r}")
        source = source.replace(marker, marker + insert)
    compile(source, "arm.py", "exec")
    return source

def prepare(config):
    pins = {}
    for lane in config["lanes"]:
        directory = Path(lane["source"])
        output = Path(lane["output"])
        if output.parent != directory.parent or output == directory:
            raise ValueError("overlay must be a separate sibling of the source adapter")
        with tempfile.TemporaryDirectory(dir=output.parent) as tmp:
            staged = Path(tmp)/"adapter"
            staged.mkdir(mode=0o755)
            for name, checksum in lane["files"].items():
                if Path(name).name != name:
                    raise ValueError("unexpected adapter path")
                original = (directory/name).read_bytes()
                if digest(original) != checksum:
                    raise ValueError(f"wrong source adapter {directory/name}")
                data = transformed(original.decode()).encode() if name == "arm.py" else original
                path = staged/name
                path.write_bytes(data)
                path.chmod(0o755 if name == "run" else 0o644)
                pins[str(output/name)] = digest(data)
            if output.exists():
                if sorted(p.name for p in output.iterdir()) != sorted(lane["files"]):
                    raise ValueError(f"inconsistent existing directory {output}")
                for name in lane["files"]:
                    if (output/name).read_bytes() != (staged/name).read_bytes():
                        raise ValueError(f"different prior adapter bytes {output/name}")
            else:
                os.rename(staged,output)
    return pins

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("config")
    print(json.dumps(prepare(json.loads(parser.parse_args().config)), sort_keys=True))
