"""Track the canonical text and JSONB argument values through one real job."""
import argparse
from pathlib import Path
import json
import runpy
import tarfile
import tempfile

HERE = Path(__file__).resolve().parent
screen = runpy.run_path(str(HERE / "screen.py"))
SUPPORT, ROOT = screen["SUPPORT"], screen["ROOT"]


def scan(pg, db):
    relation = pg.scalar(db, """SELECT reltoastrelid::regclass::text FROM pg_class
                                WHERE oid='mesh_jobs.jobs'::regclass""")
    prefix, name = relation.split(".")
    assert prefix == "pg_toast" and name.removeprefix("pg_toast_").isdigit()
    raw = db.params(f"""SELECT chunk_id, sum(octet_length(chunk_data)), count(*),
                 encode(sha256(string_agg(chunk_data,''::bytea ORDER BY chunk_seq)), 'hex')
                 FROM {relation} GROUP BY chunk_id ORDER BY chunk_id""")[1]
    assert len(raw) == 2
    rendered = pg.scalar(db, "SELECT encode(sha256(convert_to(args::text,'UTF8')),'hex') FROM mesh_jobs.jobs")
    assert sum(digest == rendered for _, _, _, digest in raw) == 1
    return {("rendered_text" if digest == rendered else "jsonb"): dict(id=int(key), bytes=int(size),
                                                         chunks=int(count), sha256=digest)
            for key, size, count, digest in raw}


def main(out, arm):
    if out.exists() or not out.is_relative_to(ROOT/"target"):
        raise ValueError("use a fresh output directory under target/")
    out.mkdir(parents=True)
    with tarfile.open(SUPPORT["SOURCE"]) as t:
        assert SUPPORT["sha"](SUPPORT["SOURCE"].read_bytes()) == screen["FROZEN"]
        prefix = "ordinary-v1/compiler/mesh-rt/src/jobs/"
        schema = "\n".join(t.extractfile(prefix+f).read().decode() for f in ("schema.sql","drain_schema.sql","enqueue_schema.sql"))
    schema, _ = screen["rendering"](schema, arm)
    config = json.loads((ROOT/"scripts/jobs-parity-profiles.json").read_text())
    batch, = runpy.run_path(str(ROOT/"scripts/jobs-parity.py"))["inputs"](config["workloads"]["large"], 64, config["seed"])
    _, (spec,) = SUPPORT["make_chunk"](batch["jobs"][:1])
    with tempfile.TemporaryDirectory(dir=ROOT,prefix=".tp-") as temp:
        pg = SUPPORT["support"](Path(temp))
        cl = pg.Cluster(Path(temp), out)
        try:
            cl.start()
            with cl.connect("postgres") as db:
                ns = screen["NS"]
                db.simple(schema)
                db.simple("ALTER TABLE mesh_jobs.jobs ALTER args SET STORAGE EXTERNAL")
                screen["install_cache"](db, arm)
                pg.set_queue(db, ns)
                receipt = json.loads(pg.scalar(db, SUPPORT["SCALAR"], ns, *spec))
                result = {"enqueue": scan(pg,db)}
                claim, = db.params(pg.CLAIM,ns,"default",pg.WORKERS,600000,1,"","false")[1]
                assert claim[1] == receipt["id"] and json.loads(claim[3])["args"] == batch["jobs"][0]
                result["claim"] = scan(pg,db)
                screen["settle"](pg,db,claim)
                result["settle"] = scan(pg,db)
                db.params("UPDATE mesh_jobs.jobs SET priority=priority")
                result["priority_self_assignment"] = scan(pg,db)
                for kind in ("jsonb","rendered_text"):
                    assert len({stage[kind]["sha256"] for stage in result.values()}) == 1
                if arm == "on_args_write":
                    for field in ("jsonb", "rendered_text"):
                        assert len({r[field]["id"] for r in result.values()}) == 1
                    db.params("UPDATE mesh_jobs.jobs SET args=args || jsonb_build_object('extra', 7)")
                    assert pg.scalar(db, "SELECT (rendered_args=args::text)::text FROM mesh_jobs.jobs") == "true"
                    result["explicit_args_edit"] = scan(pg, db)
                    for field in ("jsonb", "rendered_text"):
                        assert result["enqueue"][field]["sha256"] != result["explicit_args_edit"][field]["sha256"]
                pg.json_write(out/"toast.json", result)
        finally:
            cl.stop()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("out", type=Path)
    parser.add_argument("--on-args-write", action="store_true")
    args = parser.parse_args()
    main(args.out.resolve(), "on_args_write" if args.on_args_write else "stored_text")
