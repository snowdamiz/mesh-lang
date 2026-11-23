"""Price statement reuse on the two actual ordinary handler writes."""

import argparse
from concurrent.futures import ThreadPoolExecutor
from contextlib import ExitStack
from ctypes import c_char_p
import fcntl
import hashlib
import importlib.util
from itertools import permutations
import json
import os
from pathlib import Path
import re
import subprocess
import tarfile
from tempfile import TemporaryDirectory
from time import monotonic_ns
from uuid import UUID

ROOT = Path.cwd()
RESULTS = ROOT / "tools/durable-jobs-bench/results"
WORKERS = 8
COUNT = 1024
NAMESPACE = "ordinary-prepare"


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def inputs():
    bundle = RESULTS / "2026-11-23-ordinary-trusted-v1/ordinary-trusted-v1.tar.gz"
    assert hashlib.sha256(bundle.read_bytes()).hexdigest() == "e742644d89b872a41b4d9d241091a2349491f3de6d393fe77c4cbdbc05f6a58c"
    with tarfile.open(bundle) as archive:
        body = archive.extractfile("fixture/writes.mpl").read().decode()
        policy = archive.extractfile("proof/jobs-parity/linux-regression/trusted-application.sql").read().decode()
    attempt, effect = re.findall(r'Pool\.execute\(pool, """(.*?)"""', body, re.S)
    return [attempt, effect], policy.replace("@NAMESPACE@", NAMESPACE)


def write_jobs(item):
    index, db, statements, named = item
    for sequence in range(index, COUNT, WORKERS):
        key = f"job_{sequence:06d}"
        values = [(NAMESPACE, key, str(UUID(int=sequence+1)), "1", "1", "1700000000000000", str(index+1)),
                  (NAMESPACE, key)]
        for i, (sql, params) in enumerate(zip(statements, values, strict=True)):
            assert db.simple("SELECT 1") == (("SELECT 1", (("1",),)),)
            if named:
                label = f"app_{i}"
                if sequence == index:
                    db.prepare(label, sql)
                args = (c_char_p * len(params))(*(p.encode() for p in params))
                result = db.result(db.pq.PQexecPrepared(db.conn, label.encode(), len(args), args, None, None, 0))
            else:
                result = db.params(sql, *params)
            assert result == ("INSERT 0 1", ())
            db.idle()


def run_arm(cluster, library, cpu, statements, folder, name):
    cluster.sql("mesh", "TRUNCATE public.mesh_jobs_parity_attempts, public.mesh_jobs_parity_effects")
    with ExitStack() as resources:
        sessions = [resources.enter_context(library.connect(cluster, "mesh", "compiled_caller"))
                    for _ in range(WORKERS)]
        pids = [int(db.params("SELECT pg_backend_pid()")[1][0][0]) for db in sessions]
        assert len(set(pids)) == WORKERS
        before = [cpu.backend_cpu(pid) for pid in pids]
        load = os.getloadavg()
        start = monotonic_ns()
        with ThreadPoolExecutor(WORKERS) as executor:
            list(executor.map(write_jobs, [(i, db, statements, name.endswith("named"))
                                         for i, db in enumerate(sessions)]))
        wall = monotonic_ns() - start
        after = [cpu.backend_cpu(pid) for pid in pids]
        plans = [db.params("SELECT name, generic_plans, custom_plans FROM pg_prepared_statements ORDER BY name")[1]
                 for db in sessions]
        for rows in plans:
            if name.endswith("named"):
                assert [r[0] for r in rows] == ["app_0", "app_1"]
                assert all(int(generic) + int(custom) == COUNT // WORKERS for _, generic, custom in rows)
            else:
                assert not rows
    entries = json.loads(cluster.sql("mesh", """SELECT json_agg(json_build_array(
        a.job_key, a.job_id, a.xmin::text, e.xmin::text,
        (extract(epoch FROM pg_xact_commit_timestamp(a.xmin))*1000000)::bigint,
        (extract(epoch FROM pg_xact_commit_timestamp(e.xmin))*1000000)::bigint) ORDER BY a.job_key)
        FROM public.mesh_jobs_parity_attempts a JOIN public.mesh_jobs_parity_effects e
        USING (engine, namespace, job_key, effect_key)"""))
    assert len(entries) == COUNT and len({xid for e in entries for xid in e[2:4]}) == 2 * COUNT
    assert all(row[0] == f"job_{i:06d}" and UUID(row[1]).int == i+1 and row[4] <= row[5]
               for i, row in enumerate(entries))
    doc = dict(arm=name, pids=pids, before=before, after=after, timebase=list(cpu.TIMEBASE),
               load=load, wall_ns=wall, plans=plans, jobs=entries)
    (folder / (name + ".json")).write_text(json.dumps(doc, separators=(",", ":"))+"\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("out", type=Path)
    args = parser.parse_args()
    out = args.out.resolve()
    out.mkdir()
    parity = module("parity", ROOT / "scripts/jobs-parity.py")
    archived = module("support", RESULTS / "2026-11-23-large-storage-component/claim-memory/run.py")
    statements, policy = inputs()
    with (ROOT / "target/queue-performance.lock").open("a") as lock, TemporaryDirectory(dir=ROOT/"target") as temp:
        fcntl.flock(lock, fcntl.LOCK_EX)
        work = Path(temp)
        cpu = archived.restore(work)
        bindir = parity.postgres_tools()
        libdir = subprocess.check_output([str(bindir/"pg_config"), "--libdir"], text=True).strip()
        library = cpu.Libpq(Path(libdir)/"libpq.dylib")
        cluster = parity.Cluster(work, out, 96, "17.11")
        try:
            cluster.start()
            cluster.sql("postgres", f"CREATE ROLE compiled_caller LOGIN PASSWORD '{cluster.password}'")
            cluster.prepare_database("mesh")
            cluster.sql("mesh", policy)
            settings = json.loads(cluster.sql("mesh", """SELECT json_object_agg(name,setting) FROM pg_settings
                WHERE name IN ('fsync','synchronous_commit','full_page_writes','ssl','track_commit_timestamp',
                               'server_version_num','plan_cache_mode')"""))
            assert settings.pop("server_version_num") == "170011" and settings.pop("plan_cache_mode") == "auto"
            assert set(settings.values()) == {"on"}
            (out/"environment.json").write_text(json.dumps(dict(settings=settings, calibration=cpu.calibrate_cpu(),
                version="17.11", plan_cache_mode="auto", connections=WORKERS, jobs=COUNT))+"\n")
            for group, order in enumerate(permutations(("stock-a", "named", "stock-b")), 1):
                for arm in order:
                    label = f"{group:02}-{arm}"
                    run_arm(cluster, library, cpu, statements, out, label)
                    print(label, flush=True)
        finally:
            cluster.stop()


if __name__ == "__main__":
    main()
