"""Measure the actual PostgreSQL batch writer on the owner's EXTERNAL argument layout."""
import argparse
import hashlib
import json
from pathlib import Path
import runpy
import tarfile
import tempfile

HERE = Path(__file__).resolve().parent
SUPPORT = runpy.run_path(str(HERE.parent/"2026-11-23-slow-payload-storage-cost/screen.py"))
ROOT = SUPPORT["ROOT"]
EXTERNAL = "ALTER TABLE mesh_jobs.jobs ALTER COLUMN args SET STORAGE EXTERNAL;"
ARMS = ("default_a","external_scalar","external_batch","default_b")
ORDERS = ((0,1,3,2),(1,2,0,3),(2,3,1,0),(3,0,2,1))

def layout(db):
    return db.params("SELECT mesh_jobs.enqueue_batch_layout_v1()::int")[1] == (("1",),)

def migration(db, ddl, original, normalized):
    db.simple(ddl+original)
    assert layout(db)
    db.simple(EXTERNAL)
    assert not layout(db)
    db.simple(normalized)
    assert layout(db)
    checks = {}
    for name, change in {
        "unrelated_column_storage":"ALTER TABLE mesh_jobs.jobs ALTER COLUMN metadata SET STORAGE EXTERNAL",
        "different_argument_policy":"ALTER TABLE mesh_jobs.jobs ALTER COLUMN args SET STORAGE MAIN",
        "row_security":"ALTER TABLE mesh_jobs.jobs ENABLE ROW LEVEL SECURITY",
        "additional_index":"CREATE INDEX extra_jobs_queue ON mesh_jobs.jobs(queue)",
        "changed_constraint":"ALTER TABLE mesh_jobs.jobs ADD CHECK (priority < 999)",
    }.items():
        db.params("BEGIN")
        db.params(change)
        checks[name] = not layout(db)
        db.params("ROLLBACK")
        assert checks[name] and layout(db)
    return checks

def main(dest):
    if dest.exists() or not dest.is_relative_to(ROOT):
        raise ValueError("use a fresh output directory inside the checkout")
    dest.mkdir(parents=True)
    archive=SUPPORT["SOURCE"]
    assert SUPPORT["sha"](archive.read_bytes())=="542925005370032c71db1a2aa488c2d35d16f14f9fdc8f54ee0afbb81f80fc82"
    with tarfile.open(archive) as src:
        prefix="ordinary-v1/compiler/mesh-rt/src/jobs/"
        ddl="\n".join(src.extractfile(prefix+f).read().decode() for f in ("schema.sql","drain_schema.sql"))
        original=src.extractfile(prefix+"enqueue_schema.sql").read().decode()
    before="a.attgenerated, a.attstorage, a.attcompression"
    after="""a.attgenerated, CASE WHEN t.oid='mesh_jobs.jobs'::regclass AND a.attname='args'
              AND a.attstorage='e' THEN 'x'::"char" ELSE a.attstorage END, a.attcompression"""
    assert original.count(before)==1
    normalized=original.replace(before,after)
    profile=json.loads((ROOT/"scripts/jobs-parity-profiles.json").read_text())
    batch,=runpy.run_path(str(ROOT/"scripts/jobs-parity.py"))["inputs"](profile["workloads"]["slow"],64,profile["seed"])
    digest=hashlib.sha256((json.dumps(batch,indent=2,sort_keys=True)+"\n").encode()).hexdigest()
    assert hashlib.sha256(digest.encode()).hexdigest()=="42bb9e2be94c17b36110081430faf2647a04f3a3e0f2decf00e7c216fed462fd"
    jobs=batch["jobs"]
    chunks=[SUPPORT["make_chunk"](jobs[i:i+63]) for i in range(0,512,63)]
    with tempfile.TemporaryDirectory(dir=ROOT,prefix=".xb-") as temp:
        folder=Path(temp)
        pg=SUPPORT["support"](folder)
        cluster=pg.Cluster(folder,dest)
        try:
            cluster.start()
            with cluster.connect("postgres") as db:
                checks=migration(db,ddl,original,normalized)
            pg.json_write(dest/"manifest.json",dict(arms=ARMS,orders=ORDERS,checks=checks,batch_sha256=digest,
                          source_sha256=SUPPORT["sha"](original.encode()),candidate_sha256=SUPPORT["sha"](normalized.encode()),
                          calibration=pg.calibrate_cpu()))
            for g,order in enumerate(ORDERS,1):
                for i in order:
                    arm=ARMS[i]
                    name=f"storage_{g}_{arm}"
                    with cluster.connect("postgres") as db:
                        db.simple("CREATE DATABASE "+name)
                    with cluster.connect(name) as db:
                        physical=(EXTERNAL if arm.startswith("external_") else "")
                        specific=(original if arm=="external_scalar" else normalized)
                        schema=ddl+specific+physical
                        row=SUPPORT["trial"](pg,db,schema,"external" if arm=="external_scalar" else "pglz_a",jobs,chunks)
                        attr=db.params("SELECT attstorage::text FROM pg_attribute WHERE attrelid='mesh_jobs.jobs'::regclass AND attname='args'")[1][0][0]
                        assert attr==("e" if physical else "x")
                    pg.json_write(dest/f"{g}-{arm}.json",dict(row,argument_attstorage=attr))
                    with cluster.connect("postgres") as db:
                        db.simple("DROP DATABASE "+name)
                    print(g,arm,round(row["backend_cpu_ms"],2),round(row["wall_ms"],2),flush=True)
        finally:
            cluster.stop()

if __name__=="__main__":
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("dest",type=Path)
    main(parser.parse_args().dest.resolve())
