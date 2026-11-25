"""Verify a logical composite-key grant without timing the workstation."""
from hashlib import sha256
from importlib.util import module_from_spec, spec_from_file_location
import json
from pathlib import Path
from tempfile import TemporaryDirectory

HERE = Path(__file__).resolve().parent
spec = spec_from_file_location("grant_fixture", HERE.parent / "2026-11-25-public-dense-tid-grant/replay.py")
fixture = module_from_spec(spec)
spec.loader.exec_module(fixture)


def overlay(stock, *, bound=True):
    replacements = [
        ("SELECT p.id, p.priority,", "SELECT p.namespace AS grant_namespace, p.id, p.priority,"),
        ("WHERE b.fits AND j.namespace = ns AND j.id = s.id",
         "WHERE b.fits AND j.namespace = s.grant_namespace AND j.id = s.id"),
    ]
    for old, new in replacements:
        assert stock.count(old) == 1
        stock = stock.replace(old, new)
    if bound:
        point = "WHERE b.fits AND j.namespace = s.grant_namespace AND j.id = s.id"
        assert stock.count(point) == 1
        # The row bounds describe the same single non-null key as the two
        # equalities. Keeping the pair together gives the btree planner
        # bounds on the full key even with an empty-queue cached estimate.
        stock = stock.replace(point, point + """
            AND (j.namespace, j.id) BETWEEN (s.grant_namespace, s.id)
                                        AND (s.grant_namespace, s.id)""")
    return stock


def key_plans(plans):
    return {shape: {mode: row["target"] for mode, row in modes.items()} for shape, modes in plans.items()}


def cold_plans(db, stock, candidate):
    sql = ["SET plan_cache_mode = auto;", "SET ROLE grant_worker;"]
    names = {"stock": stock, "namespace_only_change": overlay(stock, bound=False), "composite": candidate}
    def call(name):
        return fixture.execute(16).replace("grant_replay", name)
    for name, ddl in names.items():
        sql.append(f"PREPARE {name}({fixture.PARAMETERS}) AS {fixture.statement(ddl)};")
        sql.extend(call(name) + ";" for _ in range(6))
    sql += ["RESET ROLE;", """
        INSERT INTO mesh_jobs.queues(namespace, name, concurrency) VALUES ('dense','default',64);
        INSERT INTO mesh_jobs.jobs(namespace,worker,version,queue,args,metadata,tags,state,
            priority,max_attempts,timeout_ms,backoff_base_ms,backoff_max_ms,inserted_at_ms,scheduled_at_ms)
        SELECT 'dense','a',1,'default',jsonb_build_object('i',i,'p',repeat('x',1024)),
            '{}','[]','available',0,3,1000,0,0,0,0 FROM generate_series(1,1024) i;
    """, "SET ROLE grant_worker;"]
    for name in names:
        sql.extend([f"\\echo PLAN_{name}", f"EXPLAIN (VERBOSE, COSTS OFF, FORMAT JSON) {call(name)};"])
    out = db.sql("\n".join(sql))
    output = {}
    for name in names:
        fragment = out.split(f"PLAN_{name}\n")[1].split("PLAN_", 1)[0]
        output[name] = fixture.target_plan(json.loads(fragment)[0]["Plan"])
    return output


def point_lookup(target):
    return (len(target) == 1 and target[0].get("Index Name") == "jobs_pkey"
            and ".namespace = s_1.grant_namespace" in target[0].get("Index Cond", "")
            and ".id = s_1.id" in target[0].get("Index Cond", ""))


def verify():
    schema = fixture.original(fixture.SCHEMA).decode()
    stock = fixture.definition(schema)
    candidate = overlay(stock)
    assert (HERE / "candidate.sql").read_bytes() == candidate.encode()
    results = {}
    with TemporaryDirectory(prefix="mesh-grant-key-", dir="/tmp") as private:
        db = fixture.Cluster(Path(private))
        with db.start():
            version = db.sql("SHOW server_version")
            assert version.split()[0] == "17.11"
            db.sql(schema + fixture.SETUP_ROLE)
            db.sql(f"""CREATE FUNCTION mesh_jobs.grant_replay_gate(row_position bigint) RETURNS boolean
                LANGUAGE plpgsql VOLATILE AS $$
                BEGIN
                    IF row_position = 1 THEN PERFORM pg_advisory_xact_lock({fixture.GATE_KEY}); END IF;
                    RETURN true;
                END $$;""")
            cold = cold_plans(db, stock, candidate)
            props = db.sql(fixture.FUNCTION_INFO)
            for label, ddl in {"stock": stock, "composite": candidate}.items():
                db.sql(ddl)
                assert db.sql(fixture.FUNCTION_INFO) == props
                checks = fixture.verify_contract(db)
                query = fixture.statement(ddl)
                races = {n: fixture.race(db, query, total=n)["claimed"] for n in (1, 2)}
                plans = key_plans(fixture.plans(db, query))
                results[label] = dict(contract=checks, concurrent_claims=races, plans=plans)
    assert results["stock"]["concurrent_claims"] == results["composite"]["concurrent_claims"] == {
        1: [{"i": 1, "new_context": True}],
        2: [{"i": 1, "new_context": True}, {"i": 2, "new_context": False}],
    }
    assert results["stock"]["contract"] == results["composite"]["contract"]
    assert cold["stock"][0]["Index Cond"] == "(j_1.namespace = $1)"
    assert len(cold["namespace_only_change"]) == 1
    assert ".id" not in cold["namespace_only_change"][0]["Index Cond"]
    assert point_lookup(cold["composite"])
    assert all(point_lookup(target) for shape in results["composite"]["plans"].values()
               for target in shape.values())
    return dict(source=fixture.REVISION, postgres=version, cpu_timing=False, cold=cold,
                stock_sha256=sha256(stock.encode()).hexdigest(),
                candidate_sha256=sha256(candidate.encode()).hexdigest(), variants=results)


if __name__ == "__main__":
    print(json.dumps(verify(), sort_keys=True, indent=2))
