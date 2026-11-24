"""Exercise replay, replacement and a payload-reading effect-key helper."""
import argparse
import fcntl
import json
from pathlib import Path
import tempfile

from screen import ROOT, SUPPORT, fixture, schema_for

CUSTOM = """CREATE OR REPLACE FUNCTION mesh_jobs.effect_key(j mesh_jobs.jobs) RETURNS text
LANGUAGE sql IMMUTABLE STRICT SET search_path = pg_catalog, pg_temp AS $$
    SELECT j.id::text || '/' || j.run::text || '/' || coalesce(j.args->>'key','missing')
$$;"""


def check(pg, db, variant, schema, original):
    namespace = "receipt_projection"
    db.simple(schema_for(schema, variant))
    db.simple(CUSTOM)
    spec = json.loads(original)
    spec["args"] = {"key":"before", "nested":["λ", 7]}
    spec["unique"] = dict(key="custom", fields=["worker"], keys=[], states=["available"],
                          timestamp="inserted_at", period_ms=60000)
    def enqueue(key):
        text = SUPPORT["compact"](spec)
        return json.loads(pg.scalar(db, SUPPORT["SCALAR"], namespace, text, key, SUPPORT["sha"](text.encode())))
    db.params("BEGIN")
    first = enqueue("a")
    replay = enqueue("a")
    assert first == replay and first["inserted"] and not first["conflict"]
    spec["args"]["key"] = "after"
    spec["replace"] = {"available":["args"]}
    second = enqueue("b")
    assert second["conflict"] and not second["inserted"] and first["id"] == second["id"]
    assert second["effect_key"] == first["id"] + "/1/after"
    assert pg.scalar(db, "SELECT args->>'key' FROM mesh_jobs.jobs WHERE namespace=$1",namespace) == "after"
    assert enqueue("b") == second
    db.params("SAVEPOINT rollback_one")
    spec["unique"] = None
    extra = enqueue("c")
    db.params("ROLLBACK TO SAVEPOINT rollback_one")
    db.params("COMMIT")
    assert pg.scalar(db, "SELECT count(*)::text FROM mesh_jobs.jobs") == "1"
    assert pg.scalar(db, "SELECT count(*)::text FROM mesh_jobs.receipts") == "2" and extra["id"] != first["id"]
    suffix = "missing" if variant == "fields" else "before"
    assert first["effect_key"] == first["id"] + "/1/" + suffix
    return dict(first=first,replay=replay,replacement=second,
                reflects_inserted_args=(suffix=="before"),rolled_back_id=extra["id"])


def main(out):
    if out.exists() or not out.is_relative_to(ROOT/"target"):
        raise ValueError("use a fresh directory under target/")
    out.mkdir(parents=True)
    schema, _, specs, _ = fixture()
    with tempfile.TemporaryDirectory(dir=ROOT,prefix=".rp-") as tmp, (ROOT/"target/queue-performance.lock").open("a") as lock:
        fcntl.flock(lock,fcntl.LOCK_EX)
        pg = SUPPORT["support"](Path(tmp))
        cluster = pg.Cluster(Path(tmp),out)
        result = {}
        try:
            cluster.start()
            for arm in ("stock_a","direct","fields"):
                name = "sem_" + arm
                with cluster.connect("postgres") as admin:
                    admin.params("CREATE DATABASE " + name)
                with cluster.connect(name) as db:
                    result[arm] = check(pg,db,arm,schema,specs[0][0])
                cluster.drop(name)
        finally:
            cluster.stop()
    pg.json_write(out/"semantics.json", result)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("out",type=Path)
    main(parser.parse_args().out.resolve())
