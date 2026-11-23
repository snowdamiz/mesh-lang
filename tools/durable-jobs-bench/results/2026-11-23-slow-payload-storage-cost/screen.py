"""Run the PostgreSQL part of a 512-job batch against four TOAST policies on macOS."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import runpy
import sys
import tarfile
import tempfile
import time

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
RESULTS = HERE.parent
SOURCE = RESULTS / "2026-11-23-set-based-enqueue/native/bulk-enqueue-ce5e551d079e.tar.gz"
PG_SUPPORT = RESULTS / "2026-11-23-large-storage-component/replay.py"
POLICIES = ("pglz_a", "lz4", "pglz_b", "external")
ORDERS = ((0, 1, 3, 2), (1, 2, 0, 3), (2, 3, 1, 0), (3, 0, 2, 1))
SCALAR = "SELECT mesh_jobs.enqueue($1,$2::jsonb,$3,$4)::text"
BULK = "SELECT ordinal,receipt FROM mesh_jobs.enqueue_batch_v1($1,$2::jsonb) ORDER BY ordinal::int"
FENCE = """INSERT INTO mesh_jobs.admissions AS a(namespace) VALUES ($1)
           ON CONFLICT(namespace) DO UPDATE SET fence=NOT a.fence"""
NS = "slow-storage"

def sha(data):
    return hashlib.sha256(data).hexdigest()

def compact(obj):
    return json.dumps(obj, sort_keys=True, separators=(",", ":"))

def support(temp):
    blobs, _ = runpy.run_path(str(PG_SUPPORT))["files"]()
    for name, data in blobs.items():
        if name.startswith("support/target/"):
            path = temp / name.removeprefix("support/target/")
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
    spec = importlib.util.spec_from_file_location("storage_pg", temp / "large-args-domain/replay.py")
    instance = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(instance)
    return instance

def make_chunk(jobs):
    values = []
    for job in jobs:
        spec = dict(worker="parity", version=1, args=job, queue="default", metadata={}, tags=[],
                    priority=0, scheduled_at_ms=None, delay_ms=None, max_attempts=3,
                    timeout_ms=60000, backoff_base_ms=1000, backoff_max_ms=1000,
                    suspended=False, unique=None, replace={})
        values.append(dict(spec=spec, request_key=job["key"], fingerprint=sha(compact(spec).encode())))
    return compact(values), [(compact(v["spec"]),v["request_key"],v["fingerprint"]) for v in values]

def trial(pg, db, ddl, policy, jobs, chunks):
    db.simple(ddl)
    if policy == "external":
        db.simple("ALTER TABLE mesh_jobs.jobs ALTER COLUMN args SET STORAGE EXTERNAL")
    db.simple("SET default_toast_compression TO " + ("lz4" if policy == "lz4" else "pglz"))
    pg.set_queue(db, NS)
    settings = pg.settings(db)
    assert settings["fsync"] == settings["synchronous_commit"] == "on"
    layout = pg.scalar(db, "SELECT mesh_jobs.enqueue_batch_layout_v1()::int") == "1"
    assert layout == (policy != "external")
    pid = db.pq.PQbackendPID(db.conn)
    counts = {"bulk_grants":0, "scalar_grants":0, "bulk_calls":0, "layout_fallbacks":0}
    ids = {}
    start_ticks = sum(pg.backend_cpu(pid))
    start = time.monotonic_ns()
    db.params("BEGIN")
    for batch, values in chunks:
        db.params("SAVEPOINT mesh_jobs_enqueue")
        db.params(FENCE, NS)
        output = []
        if len(values) >= 16:
            counts["bulk_calls"] += 1
            rows = db.params(BULK, NS, batch)[1]
            if rows == (("0","stock"),):
                counts["layout_fallbacks"] += 1
            else:
                assert [int(row[0]) for row in rows] == list(range(1,len(values)+1))
                output = [json.loads(row[1]) for row in rows]
                counts["bulk_grants"] += len(output)
        if not output:
            output = [json.loads(pg.scalar(db, SCALAR, NS, *v)) for v in values]
            counts["scalar_grants"] += len(output)
        for values_row, receipt in zip(values, output, strict=True):
            assert receipt["inserted"] and not receipt["conflict"] and receipt["state"] == "available"
            ids[values_row[1]] = receipt["id"]
        db.params("RELEASE SAVEPOINT mesh_jobs_enqueue")
    top_xid = pg.scalar(db, "SELECT pg_current_xact_id()::text")
    db.params("COMMIT")
    wall = (time.monotonic_ns() - start) / 1e6
    elapsed_ticks = sum(pg.backend_cpu(pid)) - start_ticks
    assert counts["bulk_grants"] == (504 if layout else 0) and counts["bulk_grants"] + counts["scalar_grants"] == 512
    rows = db.params("""SELECT id::text,args::text,coalesce(pg_column_compression(args),'none'),
                        pg_column_size(args),pg_xact_commit_timestamp(xmin)::text FROM mesh_jobs.jobs ORDER BY args->>'key'""")[1]
    storage = {}
    assert len(rows) == len(ids) == len(set(ids.values())) == len(jobs) == 512
    commit = pg.scalar(db, "SELECT pg_xact_commit_timestamp($1::xid)::text", top_xid)
    assert commit and {r[4] for r in rows} == {commit}
    for (ident, args, compression, size, _), job in zip(rows,jobs,strict=True):
        assert ids[job["key"]] == ident and json.loads(args) == job
        n, total = storage.get(compression,(0,0))
        storage[compression] = (n+1,total+int(size))
    receipt_rows = db.params("SELECT request_key, receipt->>'id' FROM mesh_jobs.receipts")[1]
    assert dict(receipt_rows) == ids
    return dict(wall_ms=wall, backend_ticks=elapsed_ticks, backend_cpu_ms=elapsed_ticks*pg.TIMEBASE[0]/pg.TIMEBASE[1]/1e6,
                settings=settings, layout=layout, calls=counts, jobs=len(rows), receipt_rows=len(receipt_rows),
                storage=storage, top_xid=top_xid, commit=commit, load=os.getloadavg())

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("destination", type=Path)
    args = parser.parse_args()
    out = args.destination.resolve()
    if not out.is_relative_to(ROOT) or out.exists():
        raise ValueError("provide a fresh destination inside the checkout")
    out.mkdir(parents=True)
    settings = json.loads((ROOT/"scripts/jobs-parity-profiles.json").read_text())
    batch, = runpy.run_path(str(ROOT/"scripts/jobs-parity.py"))["inputs"](settings["workloads"]["slow"],64,settings["seed"])
    batch_sha = sha((json.dumps(batch,indent=2,sort_keys=True)+"\n").encode())
    verifier = runpy.run_path(str(HERE.parents[1]/"verify.py"))
    assert sha(batch_sha.encode()) == verifier["PROFILES"]["slow"][3]
    assert sha(SOURCE.read_bytes()) == "542925005370032c71db1a2aa488c2d35d16f14f9fdc8f54ee0afbb81f80fc82"
    with tarfile.open(SOURCE) as archive:
        sources = [archive.extractfile("ordinary-v1/compiler/mesh-rt/src/jobs/"+name).read()
                   for name in ("schema.sql","drain_schema.sql","enqueue_schema.sql")]
    ddl = "\n".join(source.decode() for source in sources)
    with tempfile.TemporaryDirectory(dir=ROOT,prefix=".sp-") as tmp:
        root = Path(tmp)
        pg = support(root)
        manifest = dict(batch_sha256=batch_sha, schema_sha256=[sha(s) for s in sources],
                        archive_sha256=sha(SOURCE.read_bytes()), calibration=pg.calibrate_cpu(),
                        policies=POLICIES, orders=ORDERS, chunks=[63]*8+[8])
        pg.json_write(out/"manifest.json",manifest)
        cluster = pg.Cluster(root,out)
        try:
            cluster.start()
            for kind in ("canonical", "compressible"):
                jobs = batch["jobs"] if kind=="canonical" else [dict(j,payload="a"*len(j["payload"])) for j in batch["jobs"]]
                chunks = [make_chunk(jobs[i:i+63]) for i in range(0,512,63)]
                for group,order in enumerate(ORDERS,1):
                    for index in order:
                        policy = POLICIES[index]
                        dbname = f"slow_{kind}_{group}_{policy}"
                        with cluster.connect("postgres") as admin:
                            admin.simple("CREATE DATABASE "+dbname)
                        with cluster.connect(dbname) as db:
                            row = trial(pg,db,ddl,policy,jobs,chunks)
                        with cluster.connect("postgres") as admin:
                            admin.simple("DROP DATABASE "+dbname)
                        pg.json_write(out/f"{kind}-{group}-{policy}.json",row)
                        print(kind,group,policy,row["backend_cpu_ms"],row["wall_ms"],flush=True)
        finally:
            cluster.stop()

if __name__ == "__main__":
    main()
