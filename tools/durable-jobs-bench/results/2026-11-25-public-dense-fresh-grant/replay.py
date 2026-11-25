"""Exercise the pinned overlay in a disposable PG17.11 cluster without timing it."""

from hashlib import sha256
import importlib.util
import json
from pathlib import Path
import re
import select
import tarfile
from tempfile import TemporaryDirectory
from time import monotonic, sleep

from build_overlay import definition, overlay

HERE = Path(__file__).resolve().parent
REJECTED = HERE.parent / "2026-11-25-public-dense-tid-grant"
SPEC = importlib.util.spec_from_file_location("prior_grant_replay", REJECTED / "replay.py")
BASE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BASE)
BENCH = HERE.parent
LIVE_GROUP = (BENCH / "2026-11-25-dense-live-grant-plans/procfd-recovery/evidence/native"
              / "blocks/01-noop")
KEY = 4738572


def live_evidence():
    archive = LIVE_GROUP / "sampled/measured-mesh/raw.tar.gz"
    with tarfile.open(archive) as raw:
        record = raw.extractfile("adapter/grant-plans.json").read()
    targets = []
    for capture in json.loads(record):
        if "granted AS" not in capture["query"] or "WITH picked" not in capture["query"]:
            continue
        modify, = [n for n in BASE.nodes(capture["plan"]) if n["Node Type"] == "ModifyTable"]
        if modify["Actual Rows"] == 0:
            continue
        scan, = [n for n in BASE.nodes(modify) if n["Node Type"] == "Index Scan"
                 and n.get("Relation Name") == "jobs"]
        assert scan["Index Name"] == "jobs_pkey" and ".id" not in scan["Index Cond"]
        assert any(".id" in n.get("Join Filter", "") for n in BASE.nodes(modify))
        targets.append(dict(query_id=capture["query_id"], loops=scan["Actual Loops"],
                            rows=scan["Actual Rows"], admitted=modify["Actual Rows"]))
    assert len(targets) == 25 and all(t["loops"] == 1 and t["rows"] == 1024 for t in targets), targets
    return dict(member_sha256=sha256(record).hexdigest(), nonempty=targets,
                target_rows=sum(t["rows"] * t["loops"] for t in targets))


def await_gate(db, child):
    deadline = monotonic() + 7
    while db.sql(f"""SELECT EXISTS (
        SELECT FROM pg_locks l JOIN pg_stat_activity a USING(pid)
        WHERE application_name='fresh_grant_reader' AND locktype='advisory'
            AND objid={KEY} AND NOT granted);""") != "t":
        assert monotonic() < deadline and child.poll() is None, "claim did not reach the advisory gate"
        sleep(0.01)


def assert_ready(child):
    assert select.select([child.stdout], [], [], 10)[0] and child.stdout.readline().strip() == "ready"


def race(db, ddl, *, total, when, stock=False, mixed=False):
    BASE.seed(db, total, total=total)
    original = db.sql("SELECT xmin,ctid FROM mesh_jobs.jobs WHERE position=1")
    if when == "before":
        hook = "AND j.attempt < j.max_attempts"
        variant = ddl.replace(hook, hook + " AND mesh_jobs.fresh_probe_gate(j.position)", 1)
    else:
        hook = "            WITH granted AS ("
        assert ddl.count(hook) == 1
        variant = ddl.replace(hook, "            PERFORM mesh_jobs.fresh_probe_gate(1);\n" + hook, 1)
    assert variant != ddl
    db.sql(variant)
    with db.session(f"BEGIN; SELECT pg_advisory_xact_lock({KEY}) AS held \\gset\n\\echo ready") as writer:
        assert_ready(writer)
        with db.session(f"""
            SET ROLE grant_worker;
            SET application_name='fresh_grant_reader';
            SET statement_timeout='10s';
            SELECT json_agg(r ORDER BY r.ordinality)
            FROM {BASE.claim(total)} WITH ORDINALITY r;
            \\q
        """) as reader:
            await_gate(db, reader)
            if when == "before":
                version = ", worker='b', version=2" if mixed else ""
                release = f"""
                    UPDATE mesh_jobs.jobs SET args=args || '{{"new":true}}'{version}
                    WHERE position=1 RETURNING xmin,ctid;
                    COMMIT;"""
            else:
                lock = "SET lock_timeout='20ms';"
                db.sql(lock + "UPDATE mesh_jobs.jobs SET args='{}' WHERE position=1;", error="55P03")
                db.sql(lock + """SELECT mesh_jobs.job_control(
                    'dense',md5('1')::uuid,'cancel',NULL,NULL);""", error="55P03")
                assert db.sql(lock + "UPDATE mesh_jobs.jobs SET args='{}' WHERE namespace='shadow';") == ""
                assert db.sql(f"""SELECT * FROM mesh_jobs.claim('dense','default',{BASE.WORKERS},60000);""",
                              worker=True) == ""
                assert db.sql("SELECT count(*) FROM mesh_jobs.jobs WHERE state='executing'") == "0"
                release = "COMMIT;"
            replacement, _ = writer.communicate(release + "\n", timeout=10)
            assert writer.returncode == 0, replacement
            output = reader.stdout.read().strip()
            assert reader.wait(timeout=12) == 0, output
    db.sql(ddl)
    returned = json.loads(output)
    expected = [2] if mixed else list(range(1, total + 1))
    assert BASE.ids(returned) == expected, (stock, mixed, returned)
    first = json.loads(returned[0]["context"])
    if when == "before":
        assert all(a != b for a, b in zip(original.split("|"), replacement.strip().split("|"), strict=True))
        if mixed:
            assert db.sql("SELECT state='available' AND worker='b' AND version=2 "
                          "FROM mesh_jobs.jobs WHERE position=1") == "t"
        else:
            assert first["args"]["new"] is True
        assert (first["worker"], first["version"]) == ("a", 1)
    else:
        assert "new" not in first["args"] and first["attempt"] == 1
        cancelled = db.sql("""SELECT mesh_jobs.job_control(
            'dense',md5('1')::uuid,'cancel',NULL,NULL)->>'state';""")
        assert cancelled == "executing"
        assert db.sql("SELECT cancel_requested_at_ms IS NOT NULL FROM mesh_jobs.jobs WHERE position=1") == "t"
    for row in returned:
        assert db.sql(f"""
            SELECT state='executing' AND lease_token='{row["token"]}'::uuid
                AND lease_expires_at_ms=attempted_at_ms+60000
            FROM mesh_jobs.jobs WHERE namespace='dense' AND id='{row["id"]}'::uuid;
        """) == "t"
    return dict(variant="stock" if stock else "fresh", gate=when, original=original,
                replacement=replacement.strip(), order=BASE.ids(returned), changed_handler=mixed,
                first_worker=first["worker"], first_version=first["version"])


def triggers_and_atomicity(db, stock, candidate):
    db.sql("""
        CREATE TABLE mesh_jobs.fresh_events (updated bigint NOT NULL);
        CREATE FUNCTION mesh_jobs.fresh_count_update() RETURNS trigger
            LANGUAGE plpgsql AS $$ BEGIN
                INSERT INTO mesh_jobs.fresh_events SELECT count(*) FROM transitioned;
                RETURN NULL;
            END $$;
        CREATE TRIGGER fresh_count_update AFTER UPDATE ON mesh_jobs.jobs
            REFERENCING NEW TABLE AS transitioned FOR EACH STATEMENT
            EXECUTE FUNCTION mesh_jobs.fresh_count_update();
        GRANT INSERT ON mesh_jobs.fresh_events TO grant_worker;
    """)
    observed = {}
    for key, ddl in {"stock": stock, "fresh": candidate}.items():
        db.sql(ddl)
        BASE.seed(db, 0, total=2)
        db.sql("TRUNCATE mesh_jobs.fresh_events;")
        empty, = BASE.rows(db, finish="COMMIT")
        assert (empty["status"], empty["timeout_ms"]) == ("empty", "0")
        assert db.sql("SELECT updated FROM mesh_jobs.fresh_events;") == "0"
        BASE.seed(db, 2, total=2)
        db.sql("""UPDATE mesh_jobs.jobs SET args=jsonb_build_object('i',1,'oversized',repeat('x',70000))
                  WHERE position=1; TRUNCATE mesh_jobs.fresh_events;""")
        fallback, = BASE.rows(db, finish="COMMIT")
        assert fallback["status"] == "fallback"
        assert db.sql("SELECT updated FROM mesh_jobs.fresh_events;") == "0"
        BASE.seed(db, 2, total=2)
        assert BASE.ids(BASE.rows(db, finish="COMMIT")) == [1, 2]
        assert db.sql("SELECT updated FROM mesh_jobs.fresh_events ORDER BY updated;") == "0\n2"
        # Caller settings route the unchanged stock statement or marker.
        BASE.seed(db, 2, total=2)
        assert BASE.ids(BASE.rows(db, pre="SET LOCAL enable_tidscan=off;")) == [1, 2]
        assert BASE.rows(db, pre="SET LOCAL client_encoding='LATIN1';")[0]["status"] == "fallback"
        observed[key] = {"empty_statement_updates": 0, "oversized_statement_updates": 0,
                         "successful_statement_updates": 2}
    BASE.seed(db, 2, total=2)
    db.sql("""
        TRUNCATE mesh_jobs.fresh_events;
        CREATE FUNCTION mesh_jobs.fresh_suppress() RETURNS trigger
            LANGUAGE plpgsql AS $$ BEGIN
                IF OLD.position=1 THEN RETURN NULL; END IF;
                RETURN NEW;
            END $$;
        CREATE TRIGGER fresh_suppress BEFORE UPDATE ON mesh_jobs.jobs
            FOR EACH ROW EXECUTE FUNCTION mesh_jobs.fresh_suppress();
    """)
    BASE.rows(db, finish="COMMIT", error="55000")
    assert db.sql("SELECT count(*) FROM mesh_jobs.fresh_events;") == "0"
    assert db.sql("SELECT count(*) FROM mesh_jobs.jobs WHERE state='executing';") == "0"
    db.sql("""
        DROP TRIGGER fresh_suppress ON mesh_jobs.jobs;
        DROP TRIGGER fresh_count_update ON mesh_jobs.jobs;
    """)
    return observed | {"suppressed_first_row_aborts_all": True}


def layout(db, schema, candidate):
    BASE.seed(db, 1, total=1)
    db.sql("""
        ALTER TABLE mesh_jobs.jobs ENABLE ROW LEVEL SECURITY;
        CREATE POLICY fresh_probe_scope ON mesh_jobs.jobs TO grant_worker
            USING (namespace='dense') WITH CHECK (namespace='dense');
    """)
    marker = "    ) THEN\n        WITH picked AS MATERIALIZED ("
    assert candidate.count(marker) == 1
    db.sql(candidate.replace(marker, "    ) THEN\n        RAISE EXCEPTION 'unexpected fast layout';\n"
                            "        WITH picked AS MATERIALIZED ("))
    assert BASE.ids(BASE.rows(db, finish="COMMIT")) == [1]
    db.sql("DROP POLICY fresh_probe_scope ON mesh_jobs.jobs; "
           "ALTER TABLE mesh_jobs.jobs DISABLE ROW LEVEL SECURITY;")
    BASE.seed(db, 1, total=1)
    db.sql("""
        CREATE TABLE mesh_jobs.inherited_jobs () INHERITS (mesh_jobs.jobs);
        INSERT INTO mesh_jobs.inherited_jobs SELECT * FROM ONLY mesh_jobs.jobs WHERE namespace='dense';
        DELETE FROM ONLY mesh_jobs.jobs WHERE namespace='dense';
        GRANT SELECT, UPDATE ON mesh_jobs.inherited_jobs TO grant_worker;
    """)
    db.sql(candidate)
    assert BASE.ids(BASE.rows(db, finish="COMMIT")) == [1]
    assert db.sql("SELECT state FROM mesh_jobs.inherited_jobs") == "executing"
    db.sql("DROP TABLE mesh_jobs.inherited_jobs;")
    partitioned = schema.replace(
        "\n);\n\nCREATE INDEX IF NOT EXISTS mesh_jobs_ready",
        "\n) PARTITION BY HASH(namespace);\n"
        "CREATE TABLE mesh_jobs.jobs_p0 PARTITION OF mesh_jobs.jobs FOR VALUES WITH (modulus 2, remainder 0);\n"
        "CREATE TABLE mesh_jobs.jobs_p1 PARTITION OF mesh_jobs.jobs FOR VALUES WITH (modulus 2, remainder 1);\n\n"
        "CREATE INDEX IF NOT EXISTS mesh_jobs_ready",
        1,
    )
    assert partitioned != schema
    db.sql("DROP SCHEMA mesh_jobs CASCADE;\n" + partitioned +
           BASE.SETUP_ROLE.replace("CREATE ROLE grant_worker;", ""))
    BASE.seed(db, 2, total=2)
    db.sql(candidate)
    assert BASE.ids(BASE.rows(db, finish="COMMIT")) == [1, 2]
    assert db.sql("SELECT count(*) FROM mesh_jobs.jobs WHERE state='executing' AND "
                  "tableoid <> 'mesh_jobs.jobs'::regclass") == "2"
    return ["row security", "inherited job", "jobs in a declarative hash partition"]


def stale_manifest(db, candidate):
    BASE.seed(db, 2, total=2)
    marker = "            WITH granted AS ("
    assert candidate.count(marker) == 1
    db.sql(candidate.replace(
        marker,
        "            UPDATE ONLY mesh_jobs.jobs SET args='{}' WHERE namespace='dense' AND position=1;\n" + marker,
        1,
    ))
    BASE.rows(db, finish="COMMIT", error="55000")
    assert db.sql("SELECT count(*) FROM mesh_jobs.jobs WHERE namespace='dense' AND state='available' "
                  "AND args ? 'i'") == "2"
    db.sql(candidate)
    return "stale tuple after same-transaction DML aborts all changes"


def clock(db, stock, candidate):
    db.sql("""
        CREATE SEQUENCE mesh_jobs.grant_test_clock MINVALUE 1 START WITH 7000;
        GRANT USAGE ON SEQUENCE mesh_jobs.grant_test_clock TO grant_worker;
        CREATE OR REPLACE FUNCTION mesh_jobs.now_ms() RETURNS bigint
        LANGUAGE sql VOLATILE SET search_path = pg_catalog, pg_temp
        AS $$ SELECT nextval('mesh_jobs.grant_test_clock') $$;
    """)
    for ddl in (stock, candidate):
        db.sql(ddl)
        for explicit in (True, False):
            BASE.seed(db, 2, total=2)
            db.sql("ALTER SEQUENCE mesh_jobs.grant_test_clock RESTART WITH 7000;")
            call = BASE.claim(2) if explicit else BASE.claim(2).removesuffix(",0)") + ",NULL)"
            assert db.sql(f"SELECT count(*) FROM {call};", worker=True) == "2"
            started = 7000 if explicit else 7001
            assert db.sql(f"SELECT count(*) FROM mesh_jobs.jobs WHERE attempted_at_ms={started} "
                          f"AND lease_expires_at_ms={started + 60000}") == "2"
            assert db.sql("SELECT last_value FROM mesh_jobs.grant_test_clock") == str(started)
    db.sql("""
        CREATE OR REPLACE FUNCTION mesh_jobs.now_ms() RETURNS bigint
        LANGUAGE sql VOLATILE SET search_path = pg_catalog, pg_temp
        AS $$ SELECT floor(extract(epoch FROM clock_timestamp()) * 1000)::bigint $$;
    """)
    return "explicit cutoff skips the cutoff clock; implicit cutoff and one post-render lease clock match stock"


def update_query(candidate):
    query = re.search(r"(WITH granted AS \(\n                UPDATE ONLY.*?FROM granted g);", candidate, re.S)[1]
    query = re.sub(r"\n            INTO updated_ids, valid\n", "\n", query)
    names = ("ns", "queue_name", "lease_ms", "target_rel", "locations", "job_ids", "versions",
             "tokens", "started_at", "next_attempts")
    params = {name: f"${index}" for index, name in enumerate(names, 1)}
    return re.sub(r"(?<!\.)\b(" + "|".join(names) + r")\b", lambda m: params[m[0]], query)


def explain(db, candidate):
    query = update_query(candidate)
    cases = {"head16": (1024, 0, 16), "mid16": (512, 32, 16), "mid8": (512, 48, 8),
             "tail8": (8, 8, 8), "empty": (0, 0, 0)}
    result = {}
    for shape, (ready, running, slots) in cases.items():
        BASE.seed(db, ready, running)
        db.sql("""UPDATE mesh_jobs.jobs SET metadata='{"revised":1}' WHERE position % 2 = 0;
                  UPDATE mesh_jobs.jobs SET metadata='{"revised":2}' WHERE position % 3 = 0;
                  ANALYZE mesh_jobs.jobs;""")
        result[shape] = {}
        for mode in BASE.MODES:
            invocation = ("EXECUTE fresh_target('dense','default',60000,'mesh_jobs.jobs'::regclass,"
                          ":'locations',:'ids',:'versions',:'tokens',1,:'attempts')")
            warm = (f"EXPLAIN {invocation} \\g /dev/null\n") * (5 if mode == "auto" else 0)
            raw = db.sql(f"""
                SET plan_cache_mode={mode};
                SELECT coalesce(array_agg(ctid ORDER BY position)::text,'{{}}') AS locations,
                    coalesce(array_agg(id ORDER BY position)::text,'{{}}') AS ids,
                    coalesce(array_agg(xmin ORDER BY position)::text,'{{}}') AS versions,
                    coalesce(array_agg(gen_random_uuid() ORDER BY position)::text,'{{}}') AS tokens,
                    coalesce(array_agg(attempt + 1 ORDER BY position)::text,'{{}}') AS attempts
                FROM mesh_jobs.jobs WHERE namespace='dense' AND position <= {slots} \\gset
                PREPARE fresh_target(text,text,bigint,oid,tid[],uuid[],xid[],uuid[],bigint,integer[]) AS {query};
                {warm}
                EXPLAIN (FORMAT JSON,VERBOSE,COSTS OFF) {invocation};
                SELECT json_build_array(custom_plans,generic_plans) FROM pg_prepared_statements
                    WHERE name='fresh_target';
            """, worker=True)
            plan, counts = raw.rsplit("\n", 1)
            nodes = list(BASE.nodes(json.loads(plan)[0]["Plan"]))
            target, = [n for n in nodes if n.get("Relation Name") == "jobs" and n["Node Type"] != "ModifyTable"]
            assert target["Node Type"] == "Tid Scan" and "ANY" in target["TID Cond"], (shape, mode, target)
            result[shape][mode] = dict(mode_counts=json.loads(counts), target=target)
    return result


def main():
    schema = BASE.original(BASE.SCHEMA).decode()
    stock = definition(schema)
    candidate = overlay(stock)
    real = live_evidence()
    with TemporaryDirectory(prefix="fresh-grant-", dir="/tmp") as private:
        db = BASE.Cluster(Path(private))
        with db.start():
            assert db.sql("SHOW server_version_num") == "170011"
            db.sql(schema + BASE.SETUP_ROLE)
            contract = db.sql(BASE.FUNCTION_INFO)
            db.sql(candidate)
            assert db.sql(BASE.FUNCTION_INFO) == contract
            checks = BASE.verify_contract(db)
            db.sql(f"""
                CREATE FUNCTION mesh_jobs.fresh_probe_gate(picked_position bigint) RETURNS boolean
                LANGUAGE plpgsql VOLATILE SET lock_timeout='0' AS $$
                BEGIN
                    IF picked_position=1 THEN PERFORM pg_advisory_xact_lock({KEY}); END IF;
                    RETURN true;
                END $$;
            """)
            races = [race(db, ddl, total=total, when="before", stock=ddl == stock)
                     for total in (1, 2) for ddl in (stock, candidate)]
            races.extend(race(db, ddl, total=2, when="before", stock=ddl == stock, mixed=True)
                         for ddl in (stock, candidate))
            races.append(race(db, candidate, total=2, when="after"))
            trigger_checks = triggers_and_atomicity(db, stock, candidate)
            stale = stale_manifest(db, candidate)
            clocks = clock(db, stock, candidate)
            plans = explain(db, candidate)
            layouts = layout(db, schema, candidate)
    print(json.dumps(dict(version="17.11", real_stock=real, overlay_sha256=sha256(candidate.encode()).hexdigest(),
                          routine_checks=checks, races=races, triggers=trigger_checks, plans=plans,
                          stale_manifest=stale, clock=clocks, layout_fallback=layouts), indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
