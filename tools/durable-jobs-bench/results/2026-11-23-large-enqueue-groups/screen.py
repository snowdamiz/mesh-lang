"""Price ordered multi-spec enqueue calls on the frozen large PostgreSQL workload."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import runpy
import tarfile
import tempfile
import time

HERE = Path(__file__).resolve().parent
SUPPORT = runpy.run_path(str(HERE.parent/"2026-11-23-slow-payload-storage-cost/screen.py"))
ROOT = SUPPORT["ROOT"]
POLICIES = ("external", "default")
ARMS = ("scalar_a", "group16", "ordered16", "merge16", "scalar_b")
ORDERS = ((0,1,4,2,3),(1,2,0,3,4),(2,3,1,4,0),(3,4,2,0,1),(4,0,3,1,2))
NS = SUPPORT["NS"]

# This diagnostic helper preserves invocation order even for two operations on the same row.
WRITER = """
CREATE FUNCTION public.ordered_enqueue(ns text, specs text[], keys text[], fingerprints text[])
RETURNS SETOF text LANGUAGE plpgsql VOLATILE AS $$
DECLARE
    item text;
    pos integer := 1;
BEGIN
    FOREACH item IN ARRAY specs LOOP
        RETURN NEXT mesh_jobs.enqueue(ns, item::jsonb, keys[pos], fingerprints[pos])::text;
        pos := pos + 1;
    END LOOP;
END
$$;
"""

def enqueue(pg, db, jobs, mode, allow_merge):
    width = 1 if mode.startswith("scalar_") else 16
    chunks = [SUPPORT["make_chunk"](jobs[i:i+width]) for i in range(0,len(jobs),width)]
    params = [f"ARRAY[{','.join(f'${2+k*width+i}::text' for i in range(width))}]" for k in range(3)]
    ordered = "SELECT * FROM public.ordered_enqueue($1,"+",".join(params)+")"
    ids = {}
    calls = {"merge":0,"ordered":0,"scalar":0,"savepoints":0,"merge_declines":0}
    pid = db.pq.PQbackendPID(db.conn)
    cpu = sum(pg.backend_cpu(pid))
    begin = time.monotonic_ns()
    db.params("BEGIN")
    for batch, values in chunks:
        db.params("SAVEPOINT mesh_jobs_enqueue")
        calls["savepoints"] += 1
        db.params(SUPPORT["FENCE"],NS)
        receipts = []
        if mode == "merge16":
            rows = db.params(SUPPORT["BULK"],NS,batch)[1]
            if allow_merge:
                assert [int(r[0]) for r in rows] == list(range(1,len(values)+1))
                receipts = [json.loads(r[1]) for r in rows]
                calls["merge"] += 1
            else:
                assert rows == (("0","stock"),)
                calls["merge_declines"] += 1
        if mode == "ordered16":
            fields = [val for field in zip(*values,strict=True) for val in field]
            receipts = [json.loads(r[0]) for r in db.params(ordered,NS,*fields)[1]]
            calls["ordered"] += 1
        if not receipts:
            receipts = [json.loads(pg.scalar(db,SUPPORT["SCALAR"],NS,*v)) for v in values]
            calls["scalar"] += len(values)
        for v,r in zip(values,receipts,strict=True):
            assert r["inserted"] and not r["conflict"] and r["state"]=="available"
            ids[v[1]]=r["id"]
        db.params("RELEASE SAVEPOINT mesh_jobs_enqueue")
    xid = pg.scalar(db,"SELECT pg_current_xact_id()::text")
    db.params("COMMIT")
    elapsed = time.monotonic_ns()-begin
    ticks = sum(pg.backend_cpu(pid))-cpu
    rows = db.params("""SELECT id::text,args::text,pg_xact_commit_timestamp(xmin)::text,
                        coalesce(pg_column_compression(args),'none'), pg_column_size(args)
                        FROM mesh_jobs.jobs ORDER BY position""")[1]
    committed = pg.scalar(db,"SELECT pg_xact_commit_timestamp($1::xid)::text",xid)
    assert len(rows)==len(ids)==len(set(ids.values()))==len(jobs)==64 and committed
    storage = {}
    for row,expected in zip(rows,jobs,strict=True):
        ident,body,stamp,codec,size=row
        assert json.loads(body)==expected and ids[expected["key"]]==ident and stamp==committed
        n,total=storage.get(codec,(0,0))
        storage[codec]=(n+1,total+int(size))
    assert ids==dict(db.params("SELECT request_key, receipt->>'id' FROM mesh_jobs.receipts")[1])
    return dict(calls=calls, xid=xid, committed=committed, jobs=len(ids), storage=storage,
                backend_ticks=ticks, cpu_ms=ticks*pg.TIMEBASE[0]/pg.TIMEBASE[1]/1e6, wall_ms=elapsed/1e6, load=os.getloadavg())

def main(out):
    if out.exists() or not out.is_relative_to(ROOT):
        raise ValueError("provide a fresh output directory inside the checkout")
    out.mkdir(parents=True)
    source = SUPPORT["SOURCE"]
    assert SUPPORT["sha"](source.read_bytes())=="542925005370032c71db1a2aa488c2d35d16f14f9fdc8f54ee0afbb81f80fc82"
    with tarfile.open(source) as archive:
        ddl="\n".join(archive.extractfile("ordinary-v1/compiler/mesh-rt/src/jobs/"+file).read().decode()
                      for file in ("schema.sql","drain_schema.sql","enqueue_schema.sql"))
    profile=json.loads((ROOT/"scripts/jobs-parity-profiles.json").read_text())
    batch,=runpy.run_path(str(ROOT/"scripts/jobs-parity.py"))["inputs"](profile["workloads"]["large"],64,profile["seed"])
    digest=hashlib.sha256((json.dumps(batch,indent=2,sort_keys=True)+"\n").encode()).hexdigest()
    assert hashlib.sha256(digest.encode()).hexdigest()=="9d7f130f2cc690487582f73c9bea5fbb1f2661f8e416beb2f1574a6f868cc5c1"
    with tempfile.TemporaryDirectory(dir=ROOT,prefix=".le-") as tmp:
        folder=Path(tmp)
        pg=SUPPORT["support"](folder)
        pg.json_write(out/"manifest.json",dict(calibration=pg.calibrate_cpu(),batch_sha256=digest,orders=ORDERS,arms=ARMS,
                                                ddl_sha256=SUPPORT["sha"](ddl.encode()),policies=POLICIES))
        cluster=pg.Cluster(folder,out)
        try:
            cluster.start()
            for g,order in enumerate(ORDERS,1):
                for storage in (POLICIES if g%2 else POLICIES[::-1]):
                    for i in order:
                        arm=ARMS[i]
                        name=f"price_{g}_{storage}_{arm}"
                        with cluster.connect("postgres") as admin:
                            admin.simple("CREATE DATABASE "+name)
                        with cluster.connect(name) as db:
                            db.simple(ddl)
                            db.simple(WRITER)
                            if storage=="external":
                                db.simple("ALTER TABLE mesh_jobs.jobs ALTER COLUMN args SET STORAGE EXTERNAL")
                            pg.set_queue(db,NS)
                            settings=pg.settings(db)
                            assert settings["server_version_num"]=="170011" and settings["fsync"]==settings["synchronous_commit"]=="on"
                            layout=pg.scalar(db,"SELECT mesh_jobs.enqueue_batch_layout_v1()::int")=="1"
                            assert layout==(storage=="default")
                            metrics=enqueue(pg,db,batch["jobs"],arm,layout)
                        with cluster.connect("postgres") as admin:
                            admin.simple("DROP DATABASE "+name)
                        pg.json_write(out/f"{g}-{storage}-{arm}.json",dict(metrics,settings=settings,layout=layout))
                        print(g,storage,arm,round(metrics["cpu_ms"],1),round(metrics["wall_ms"],1),flush=True)
        finally:
            cluster.stop()

if __name__=="__main__":
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("out",type=Path)
    main(parser.parse_args().out.resolve())
