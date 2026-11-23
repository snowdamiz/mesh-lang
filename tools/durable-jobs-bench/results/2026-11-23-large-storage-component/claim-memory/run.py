"""Price two independent changes to the stock large-claim query on PG 17.11."""
import argparse
import fcntl
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shlex
import subprocess
import tarfile
import tempfile

HERE = Path(__file__).resolve().parent
ROOT = Path.cwd()
SOURCE = ROOT / "tools/durable-jobs-bench/results/2026-11-23-large-storage-component"
ORDERS = [
    ("stock-a", "header", "memory", "stock-b"),
    ("stock-b", "memory", "header", "stock-a"),
    ("header", "stock-b", "stock-a", "memory"),
    ("memory", "stock-a", "stock-b", "header"),
]


def restore(work):
    manifest = json.loads((SOURCE / "manifest.json").read_text())
    with tarfile.open(SOURCE / "evidence.tar.gz") as archive:
        for name, digest in manifest["files"].items():
            if not name.startswith("support/target/"):
                continue
            body = archive.extractfile(name).read()
            assert hashlib.sha256(body).hexdigest() == digest
            dest = work / name.removeprefix("support/target/")
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_bytes(body)
    frozen = work / "large-claim-perf/frozen"
    frozen.mkdir(parents=True)
    for name in ("scripts/jobs-parity.py", "scripts/jobs-parity-profiles.json"):
        body = (ROOT / name).read_bytes()
        assert hashlib.sha256(body).hexdigest() == manifest["repo_inputs"][name]
        (frozen / Path(name).name).write_bytes(body)
    spec = importlib.util.spec_from_file_location("pricing", work / "large-args-domain/replay.py")
    pg = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(pg)
    return pg


def trial(pg, cl, database, kind, module, schema, jobs):
    with cl.connect("postgres") as admin:
        admin.params(f"CREATE DATABASE {database}")
    try:
        with cl.connect(database) as session, cl.connect(database) as observer:
            session.simple(schema)
            session.simple("ALTER TABLE mesh_jobs.jobs ALTER args SET STORAGE EXTERNAL")
            if kind == "memory":
                session.simple("SET work_mem = '16MB'")
            if kind == "header":
                library = str(module).replace("'", "''")
                session.simple(f"""CREATE FUNCTION mesh_jobs.arg_is_object(jsonb) RETURNS boolean
                    AS '{library}', 'mesh_jsonb_is_object' LANGUAGE C IMMUTABLE STRICT PARALLEL SAFE;
                    ALTER TABLE mesh_jobs.jobs DROP CONSTRAINT jobs_args_check,
                    ADD CONSTRAINT jobs_args_check CHECK (mesh_jobs.arg_is_object(args));""")
                same_type = pg.scalar(session, """SELECT bool_and(
                    mesh_jobs.arg_is_object(v) IS NOT DISTINCT FROM (jsonb_typeof(v)='object'))
                    FROM (VALUES (NULL::jsonb), ('null'), ('42'), ('true'), ('"a"'), ('{}'),
                        ('[]'), ('{"x":[false]}'), ('[{"x":1}]'), (to_jsonb(repeat('a',524288))),
                        (jsonb_build_object('value',repeat('b',524288)))) AS cases(v)""")
                assert same_type == "t"
            result = pg.run_lifecycle(session, observer, jobs)
            return dict(result, settings=pg.settings(session), load=os.getloadavg())
    finally:
        cl.drop(database)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("out", type=Path)
    args = parser.parse_args()
    out = args.out.resolve()
    out.mkdir()
    include = subprocess.check_output(["pg_config", "--includedir-server"], text=True).strip()
    module = out / "object.so"
    flags = shlex.split(subprocess.check_output(["pg_config", "--cppflags"], text=True))
    subprocess.run(["cc", "-O2", "-Wall", "-Werror", *flags, "-I"+include, "-bundle", "-undefined",
                    "dynamic_lookup", HERE / "object.c", "-o", module], check=True)
    with tempfile.TemporaryDirectory(dir=ROOT / "target", prefix="jc-") as name:
        work = Path(name)
        pg = restore(work)
        jobs, digest = pg.profile_jobs()
        schema = (ROOT / "compiler/mesh-rt/src/jobs/schema.sql").read_text()
        pg.json_write(out / "inputs.json", {"orders": ORDERS, "large_sha256": digest,
            "schema_sha256": hashlib.sha256(schema.encode()).hexdigest(), "clock": pg.calibrate_cpu()})
        with (ROOT / "target/queue-performance.lock").open("a") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            cl = pg.Cluster(work, out)
            try:
                cl.start()
                for group, order in enumerate(ORDERS, 1):
                    for idx, kind in enumerate(order):
                        sample = trial(pg, cl, f"component_{group}_{idx}", kind, module, schema, jobs["large"])
                        pg.json_write(out / f"{group}-{kind}.json", sample)
            finally:
                cl.stop()


if __name__ == "__main__":
    main()
