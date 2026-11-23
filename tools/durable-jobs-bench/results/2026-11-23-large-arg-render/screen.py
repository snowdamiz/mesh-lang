"""Charge PostgreSQL for both preparing and consuming canonical job argument text."""
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
SUPPORT = runpy.run_path(str(HERE.parent / "2026-11-23-slow-payload-storage-cost/screen.py"))
ROOT = SUPPORT["ROOT"]
ARMS = ("stock_a", "live_text", "stored_text", "stock_b")
ORDERS = ((0,1,3,2), (1,2,0,3), (2,3,1,0), (3,0,2,1))
NS = "argument-render"
FROZEN = "542925005370032c71db1a2aa488c2d35d16f14f9fdc8f54ee0afbb81f80fc82"


def rendering(schema, arm):
    start = schema.index("context := jsonb_build_object(")
    end = schema.index(")::text;", start) + len(")::text")
    expression = schema[start + len("context := "):end]
    assert schema.count(expression) == 1
    if not arm.startswith("stock"):
        fields = expression.removeprefix("jsonb_build_object(").removesuffix(")::text")
        fields = fields.replace("'id', job.id, ", "", 1).replace("'args', job.args, ", "", 1)
        other = "jsonb_build_object(" + fields + ")::text"
        args = "job.args::text" if arm == "live_text" else "job.rendered_args"
        new = """'{"id": "' || job.id::text || '", "args": ' || """ + args + " || ', ' || substr(" + other + ", 2)"
        schema = schema.replace(expression, new)
    return schema, expression


def install_cache(db, arm):
    if arm == "stored_text":
        db.simple("""ALTER TABLE mesh_jobs.jobs ADD COLUMN rendered_args text
                     GENERATED ALWAYS AS (args::text) STORED;
                     ALTER TABLE mesh_jobs.jobs ALTER COLUMN rendered_args SET STORAGE EXTERNAL""")
    elif arm == "on_args_write":
        db.simple("""ALTER TABLE mesh_jobs.jobs ADD COLUMN rendered_args text;
                     ALTER TABLE mesh_jobs.jobs ALTER COLUMN rendered_args SET STORAGE EXTERNAL;
                     CREATE FUNCTION mesh_jobs.capture_rendered_args() RETURNS trigger
                     LANGUAGE plpgsql AS $f$ BEGIN
                         NEW.rendered_args := NEW.args::text;
                         RETURN NEW;
                     END $f$;
                     CREATE TRIGGER capture_rendered_args BEFORE INSERT OR UPDATE OF args
                     ON mesh_jobs.jobs FOR EACH ROW EXECUTE FUNCTION mesh_jobs.capture_rendered_args()""")


def trial(pg, db, observer, schemas, jobs, specs, arm):
    schema, source_expr = rendering(schemas, arm)
    db.simple(schema)
    db.simple("ALTER TABLE mesh_jobs.jobs ALTER COLUMN args SET STORAGE EXTERNAL")
    install_cache(db, arm)
    db.simple("SET work_mem='16MB'")
    pg.set_queue(db, NS)
    settings = pg.settings(db)
    assert settings["server_version_num"] == "170011" and settings["work_mem"] == "16384"
    assert settings["fsync"] == settings["synchronous_commit"] == "on"
    ids = []
    def submit():
        db.params("BEGIN")
        for spec, key, fingerprint in specs:
            db.params("SAVEPOINT mesh_jobs_enqueue")
            db.params(SUPPORT["FENCE"], NS)
            receipt = json.loads(pg.scalar(db, SUPPORT["SCALAR"], NS, spec, key, fingerprint))
            assert receipt["inserted"] and not receipt["conflict"]
            ids.append(receipt["id"])
            db.params("RELEASE SAVEPOINT mesh_jobs_enqueue")
        xid = pg.scalar(db, "SELECT pg_current_xact_id()::text")
        db.params("COMMIT")
        return xid

    def claim():
        rows = []
        for _ in range(8):
            db.params("BEGIN ISOLATION LEVEL READ COMMITTED")
            part = db.params(pg.CLAIM, NS, "default", pg.WORKERS, 600000, 8, "", "false")[1]
            assert [int(r[-1]) for r in part] == list(range(1, 9))
            for r in part:
                assert r[0] == "claimed" and len(r[3].encode()) == int(r[5])
            rows.extend(part)
            db.params("COMMIT")
        return rows

    def mark():
        return pg.scalar(observer, "SELECT pg_current_wal_insert_lsn()::text")

    def collect(name, action):
        lsn = mark()
        result, stages[name] = pg.measured(db, observer, action)
        stages[name]["cluster_wal_bytes"] = int(pg.scalar(observer,
            "SELECT pg_wal_lsn_diff(pg_current_wal_insert_lsn(),$1::pg_lsn)::text", lsn))
        return result

    stages = {}
    xid = collect("enqueue", submit)
    committed = pg.scalar(db, "SELECT pg_xact_commit_timestamp($1::xid)::text", xid)
    assert committed and len(ids) == len(set(ids)) == 64
    assert set(db.params("SELECT DISTINCT pg_xact_commit_timestamp(xmin)::text FROM mesh_jobs.jobs")[1]) == {(committed,)}
    assert dict(db.params("SELECT request_key,receipt->>'id' FROM mesh_jobs.receipts")[1]) == dict(zip([j["key"] for j in jobs], ids))
    rows = collect("claim", claim)
    assert [row[1] for row in rows] == ids
    expected = db.params("SELECT job.id::text, " + source_expr.replace("job.attempt + 1", "job.attempt") +
                         " FROM mesh_jobs.jobs job ORDER BY position")[1]
    digests = []
    for row, (ident, body), job in zip(rows, expected, jobs, strict=True):
        assert row[1] == ident and row[3] == body and json.loads(body)["args"] == job
        digests.append(hashlib.sha256(body.encode()).hexdigest())
    collect("settle", lambda: [settle(pg, db, r) for r in rows])
    terminal = db.params("""SELECT state, attempt, count(*), count(DISTINCT xmin::text)
                         FROM mesh_jobs.jobs GROUP BY state,attempt""")[1]
    assert terminal == (("completed","1","64","64"),)
    footprint = db.params("""SELECT pg_relation_size(reltoastrelid), pg_total_relation_size(oid)
                      FROM pg_class WHERE oid='mesh_jobs.jobs'::regclass""")[1][0]
    return dict(settings=settings, stages=stages, contexts_sha256=digests, receipt_ids=ids,
                submitted_xid=xid, committed=committed, terminal=terminal,
                toast_heap_bytes=int(footprint[0]), total_jobs_bytes=int(footprint[1]), load=os.getloadavg())


def settle(pg, db, row):
    assert pg.settle(db, NS, row[1], row[2]) == "completed"


def main(out, arms=ARMS):
    if out.exists() or not out.is_relative_to(ROOT / "target"):
        raise ValueError("use a new output directory under target/")
    out.mkdir(parents=True)
    source = SUPPORT["SOURCE"]
    assert SUPPORT["sha"](source.read_bytes()) == FROZEN
    with tarfile.open(source) as archive:
        schema = "\n".join(archive.extractfile("ordinary-v1/compiler/mesh-rt/src/jobs/" + f).read().decode()
                           for f in ("schema.sql", "drain_schema.sql", "enqueue_schema.sql"))
    profiles = json.loads((ROOT / "scripts/jobs-parity-profiles.json").read_text())
    batch, = runpy.run_path(str(ROOT / "scripts/jobs-parity.py"))["inputs"](profiles["workloads"]["large"],64,profiles["seed"])
    digest = SUPPORT["sha"]((json.dumps(batch, indent=2, sort_keys=True) + "\n").encode())
    assert digest == "30f91354336fab21dbe85f2933f3347ec53ab7634af3805169a892536747fa1c"
    jobs = batch["jobs"]
    _, specs = SUPPORT["make_chunk"](jobs)
    with tempfile.TemporaryDirectory(dir=ROOT,prefix=".ar-") as temp, (ROOT/"target/queue-performance.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        pg = SUPPORT["support"](Path(temp))
        pg.json_write(out/"manifest.json", dict(arms=arms, orders=ORDERS, frozen=FROZEN, input_sha256=digest,
                                                schema_sha256=SUPPORT["sha"](schema.encode()), calibration=pg.calibrate_cpu()))
        cluster = pg.Cluster(Path(temp), out)
        try:
            cluster.start()
            for g, order in enumerate(ORDERS,1):
                for i in order:
                    arm = arms[i]
                    name = f"render_{g}_{arm}"
                    with cluster.connect("postgres") as admin:
                        admin.params("CREATE DATABASE " + name)
                    with cluster.connect(name) as db, cluster.connect(name) as observer:
                        row = trial(pg, db, observer, schema, jobs, specs, arm)
                    pg.json_write(out/f"{g}-{arm}.json", row)
                    print(g, arm, [(stage, round(sum(r["backend_cpu_ms"].values()))) for stage,r in row["stages"].items()],flush=True)
                    cluster.drop(name)
        finally:
            cluster.stop()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("out",type=Path)
    parser.add_argument("--on-args-write", action="store_true", help="charge a cache refreshed on explicit argument writes")
    args = parser.parse_args()
    arms = tuple("on_args_write" if arm == "stored_text" else arm for arm in ARMS) if args.on_args_write else ARMS
    main(args.out.resolve(), arms)
