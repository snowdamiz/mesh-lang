"""Compare three ways of constructing the existing scalar enqueue receipt."""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import runpy
import tarfile
import tempfile

HERE = Path(__file__).resolve().parent
BASE = runpy.run_path(str(HERE.parent / "2026-11-23-large-arg-render/screen.py"))
ROOT = BASE["ROOT"]
SUPPORT = BASE["SUPPORT"]
ARMS = ("stock_a", "direct", "fields", "stock_b")
ORDERS = ((0, 1, 3, 2), (3, 2, 0, 1), (2, 3, 1, 0), (1, 0, 2, 3))
NAMESPACE = BASE["NS"]
RECEIPT = """result := jsonb_build_object(
        'id', job.id, 'effect_key', mesh_jobs.effect_key(job), 'state', job.state,
        'conflict', duplicate, 'inserted', NOT duplicate);"""
DIRECT = """RETURNING jsonb_build_object(
            'id', id, 'effect_key', mesh_jobs.effect_key(jobs), 'state', state,
            'conflict', false, 'inserted', true) INTO result;"""
CHECK = """SELECT j.id::text, j.position, r.request_key, r.fingerprint, pg_xact_commit_timestamp(r.xmin)::text,
      r.receipt = jsonb_build_object('id',j.id,'effect_key',mesh_jobs.effect_key(j),
          'state','available','conflict',false,'inserted',true)
    FROM mesh_jobs.jobs j JOIN mesh_jobs.receipts r
      ON r.namespace=j.namespace AND r.receipt->>'id'=j.id::text
    WHERE j.namespace=$1 ORDER BY j.position"""


def substitution(source, old, new):
    if source.count(old) != 1:
        raise ValueError(f"unexpected occurrence count for {old!r}")
    return source.replace(old, new)


def schema_for(schema, arm):
    if arm in ("stock_a", "stock_b"):
        return schema
    assert arm in ("direct", "fields")
    start = schema.index("CREATE OR REPLACE FUNCTION mesh_jobs.enqueue(")
    stop = schema.index("\n$$;", start)
    enqueue = schema[start:stop]
    if arm == "direct":
        enqueue = substitution(enqueue, "RETURNING * INTO job;", DIRECT)
        enqueue = substitution(enqueue, RECEIPT, "IF duplicate THEN\n        " + RECEIPT + "\n    END IF;")
    else:
        enqueue = substitution(enqueue, "RETURNING * INTO job;",
            "RETURNING id, run, state INTO job.id, job.run, job.state;")
    return schema[:start] + enqueue + schema[stop:]


def fixture():
    source = SUPPORT["SOURCE"]
    assert SUPPORT["sha"](source.read_bytes()) == BASE["FROZEN"]
    with tarfile.open(source) as archive:
        schema = "\n".join(archive.extractfile("ordinary-v1/compiler/mesh-rt/src/jobs/" + name).read().decode()
                           for name in ("schema.sql", "drain_schema.sql", "enqueue_schema.sql"))
    workloads = json.loads((ROOT / "scripts/jobs-parity-profiles.json").read_text())
    batch, = runpy.run_path(str(ROOT / "scripts/jobs-parity.py"))["inputs"](
        workloads["workloads"]["large"], 64, workloads["seed"])
    digest = SUPPORT["sha"]((json.dumps(batch, indent=2, sort_keys=True) + "\n").encode())
    assert digest == "30f91354336fab21dbe85f2933f3347ec53ab7634af3805169a892536747fa1c"
    jobs = batch["jobs"]
    return schema, jobs, SUPPORT["make_chunk"](jobs)[1], digest


def trial(pg, cluster, out, run_id, arm, schema, jobs, specs):
    name = f"receipt_{run_id}_{arm}"
    with cluster.connect("postgres") as admin:
        admin.params("CREATE DATABASE " + name)
    with cluster.connect(name) as db, cluster.connect(name) as obs:
        receipt = BASE["trial"](pg, db, obs, schema_for(schema, arm), jobs, specs, "stock_a")
        items = db.params(CHECK, NAMESPACE)[1]
        assert [r[0] for r in items] == receipt["receipt_ids"]
        for row, (_, key, finger) in zip(items, specs, strict=True):
            expected = (key, finger, receipt["committed"], "t")
            if tuple(row[2:]) != expected:
                raise ValueError(f"receipt mismatch for {key}: {row[2:]} != {expected}")
        replay = json.loads(pg.scalar(db, SUPPORT["SCALAR"], NAMESPACE, *specs[0]))
        assert replay == dict(id=items[0][0], effect_key=items[0][0]+"/1", state="available", inserted=True, conflict=False)
        receipt["fingerprints_sha256"] = hashlib.sha256("".join(r[3] for r in items).encode()).hexdigest()
        receipt["replayed_receipt"] = replay
        receipt["full_receipts_matched"] = len(items)
    pg.json_write(out / f"{run_id}-{arm}.json", receipt)
    cluster.drop(name)
    return receipt


def main(out):
    if out.exists() or not out.is_relative_to(ROOT / "target"):
        raise ValueError("use a fresh directory under target/")
    out.mkdir(parents=True)
    schema, jobs, specs, digest = fixture()
    with tempfile.TemporaryDirectory(dir=ROOT, prefix=".rp-") as tmp, (ROOT/"target/queue-performance.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        pg = SUPPORT["support"](Path(tmp))
        pg.json_write(out/"manifest.json", dict(orders=ORDERS, arms=ARMS, input=digest, source=BASE["FROZEN"],
                      source_by_arm={a:SUPPORT["sha"](schema_for(schema,a).encode()) for a in ARMS},
                      calibration=pg.calibrate_cpu()))
        cluster = pg.Cluster(Path(tmp), out)
        try:
            cluster.start()
            for group, order in enumerate(ORDERS, 1):
                for pos in order:
                    arm = ARMS[pos]
                    result = trial(pg, cluster, out, group, arm, schema, jobs, specs)
                    print(group, arm, round(sum(result["stages"]["enqueue"]["backend_cpu_ms"].values()), 1), flush=True)
        finally:
            cluster.stop()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("out",type=Path)
    main(parser.parse_args().out.resolve())
