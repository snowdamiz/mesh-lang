"""Check the pinned logical grant against the individual-settlement product schema."""
import argparse
from hashlib import sha256
import json
import os
from pathlib import Path
import subprocess
from tempfile import TemporaryDirectory

import proof

fixture = proof.fixture
REVISION = "1e70f18d624a841740d3bbb3d0fd09b269257f68"
HERE = Path(__file__).resolve().parent


def check(binary_dir, expected_version):
    os.environ["PG17_BIN"] = str(binary_dir.resolve())
    stock_schema = subprocess.check_output(
        ["git", "show", f"{REVISION}:{fixture.SCHEMA}"], cwd=fixture.ROOT, text=True)
    stock = fixture.definition(stock_schema)
    candidate = (HERE / "candidate.sql").read_text()
    assert candidate == proof.overlay(stock) and stock_schema.count(stock) == 1
    patched = stock_schema.replace(stock, candidate)
    with TemporaryDirectory(prefix="mesh-grant-tier-", dir="/tmp") as scratch:
        db = fixture.Cluster(Path(scratch))
        with db.start():
            version = db.sql("SHOW server_version").split()[0]
            assert version == expected_version
            db.sql(stock_schema + fixture.SETUP_ROLE)
            props = db.sql(fixture.FUNCTION_INFO)
            source = f"SELECT prosrc FROM pg_proc WHERE oid='{fixture.SIGNATURE}'::regprocedure;"
            before = db.sql(source)
            db.sql(patched)
            after = db.sql(source)
            assert db.sql(fixture.FUNCTION_INFO) == props and before != after
            assert after == candidate.split('AS $$',1)[1].rsplit('$$;',1)[0].strip()
            cold = proof.cold_plans(db, stock, candidate)
            assert proof.point_lookup(cold["composite"])
            assert cold["stock"][0]["Index Cond"] == "(j_1.namespace = $1)"
            db.sql(f"""CREATE FUNCTION mesh_jobs.grant_replay_gate(row_position bigint) RETURNS boolean
                LANGUAGE plpgsql VOLATILE AS $$
                BEGIN
                    IF row_position = 1 THEN PERFORM pg_advisory_xact_lock({fixture.GATE_KEY}); END IF;
                    RETURN true;
                END $$;""")
            controls = {}
            for label, ddl in (("stock", stock), ("logical", candidate)):
                db.sql(ddl)
                semantics = fixture.verify_contract(db)
                query = fixture.statement(ddl)
                epq = {str(n): fixture.race(db, query, total=n)["claimed"] for n in (1,2)}
                controls[label] = dict(semantics=semantics, epq=epq)
            assert controls["logical"] == controls["stock"]
            assert controls["logical"]["epq"]["2"] == [
                {"i": 1, "new_context": True}, {"i": 2, "new_context": False}]
            # Simulate a subsequent migration by either binary on the same schema.
            for ddl, expected in ((stock_schema,before), (patched,after), (patched,after)):
                db.sql(ddl)
                assert db.sql(source) == expected and db.sql(fixture.FUNCTION_INFO) == props
    return dict(postgres=version, product_revision=REVISION, schema_sha256=sha256(stock_schema.encode()).hexdigest(),
                overlay_sha256=sha256(candidate.encode()).hexdigest(), controls=controls, cold_plans=cold,
                repeated_migration=True, old_migration_restores_stock=True, retained_catalog_properties=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("bin", type=Path)
    parser.add_argument("version")
    args = parser.parse_args()
    print(json.dumps(check(args.bin, args.version), sort_keys=True, indent=2))
