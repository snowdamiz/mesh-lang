"""Compare the retained completion only, using PostgreSQL results and untimed plans."""

from contextlib import contextmanager
import gzip
from hashlib import sha256
import json
import os
from pathlib import Path
import re
import select
import subprocess
import tempfile
import time

HERE = Path(__file__).resolve().parent
SIGNATURE = "mesh_jobs.complete_batch_v1(text,uuid[],uuid[])"
SCHEMA_SHA = "391b3bff16ef6894fe2030b2d2bbfcc767a2c0456e99e520dcb34e0431a56937"
START = "CREATE OR REPLACE FUNCTION mesh_jobs.complete_batch_v1("
JOIN = "JOIN mesh_jobs.jobs j ON j.namespace = ns AND (j.id, j.lease_token) = (p.id, p.token)"
CARDINALITY = "cardinality(ids) = cardinality(tokens) AND cardinality(ids) BETWEEN 1 AND 8"
ELIGIBLE = "j.state = 'executing' AND j.cancel_requested_at_ms IS NULL"
LOCK = "FOR NO KEY UPDATE OF j SKIP LOCKED"
CHECKED = "    ), checked AS MATERIALIZED ("
RACE_KEY = 67433918


def definition(schema):
    return START + schema.split(START, 1)[1].split("$$;", 1)[0] + "$$;\n"


def variants(stock):
    assert stock.count(JOIN) == stock.count(CARDINALITY) == 1
    held, suffix = stock.split(CHECKED)
    lateral = held[:held.index("        " + JOIN)] + f"""        CROSS JOIN LATERAL (
            SELECT j.id, j.lease_token, j.lease_expires_at_ms
            FROM mesh_jobs.jobs j
            WHERE j.namespace = ns AND j.id = p.id AND j.lease_token = p.token
                AND {ELIGIBLE}
            {LOCK}
        ) j
        WHERE {CARDINALITY}
"""
    return {
        "stock": stock,
        "array": held.replace(CARDINALITY, CARDINALITY + "\n            AND j.id = ANY(ids)") + CHECKED + suffix,
        "range": held.replace(JOIN, JOIN + """
            AND (j.namespace, j.id) BETWEEN (ns, p.id) AND (ns, p.id)""") + CHECKED + suffix,
        "lateral": lateral + CHECKED + suffix,
    }


class Server:
    def __init__(self, directory):
        self.dir = Path(directory)
        self.data = self.dir / "data"
        self.bin = Path(os.environ.get("PG17_BIN", "/opt/homebrew/opt/postgresql@17/bin"))
        self.psql = [self.bin / "psql", "-XqAt", "-h", str(self.dir), "-d", "postgres",
                     "-v", "ON_ERROR_STOP=1", "-v", "VERBOSITY=verbose"]

    def tool(self, name, *args):
        subprocess.run([self.bin / name, *args], check=True, capture_output=True, text=True)

    @contextmanager
    def start(self):
        self.tool("initdb", "-D", self.data, "-A", "trust", "--no-locale", "-E", "UTF8")
        try:
            self.tool("pg_ctl", "-D", self.data, "-l", self.dir / "server.log",
                      "-o", f"-c listen_addresses='' -k '{self.dir}'", "-w", "start")
            yield
        finally:
            if (self.data / "postmaster.pid").exists():
                self.tool("pg_ctl", "-D", self.data, "-m", "fast", "-w", "stop")

    def sql(self, sql, error=None):
        result = subprocess.run(self.psql, input=sql, capture_output=True, text=True, timeout=35)
        if error:
            assert result.returncode == 3 and f"ERROR:  {error}:" in result.stderr, result.stderr
        else:
            assert result.returncode == 0, result.stderr
        return result.stdout.strip()

    @contextmanager
    def session(self, sql):
        with subprocess.Popen(self.psql, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                              stderr=subprocess.PIPE, text=True) as proc:
            try:
                proc.stdin.write(sql.rstrip() + "\n")
                proc.stdin.flush()
                yield proc
            finally:
                if proc.poll() is None:
                    proc.kill()
                    proc.communicate()

    @contextmanager
    def hold(self, sql):
        with self.session("BEGIN;\n" + sql + "\n\\warn ready") as proc:
            assert select.select([proc.stderr], [], [], 10)[0], sql
            line = proc.stderr.readline()
            assert line.strip() == "ready", (sql, line)
            yield proc

    def wait_advisory(self, name):
        # Synchronization only. The replay never reports any elapsed time.
        for _ in range(300):
            if self.sql(f"""SELECT count(*) FROM pg_stat_activity WHERE application_name = '{name}'
                         AND wait_event_type = 'Lock' AND wait_event = 'advisory';""") == "1":
                return
            time.sleep(.02)
        raise AssertionError(f"{name} did not reach its registered gate")


ROLES = f"""
CREATE ROLE completion_migrator;
CREATE ROLE completion_worker;
CREATE ROLE completion_legacy;
CREATE ROLE completion_app;
REVOKE EXECUTE ON FUNCTION mesh_jobs.claim(text,text,jsonb,bigint),
    mesh_jobs.settle_state(text,uuid,uuid,jsonb,boolean) FROM PUBLIC;
GRANT USAGE ON SCHEMA mesh_jobs TO completion_migrator, completion_worker, completion_legacy, completion_app;
GRANT SELECT, UPDATE ON mesh_jobs.jobs TO completion_worker, completion_legacy;
GRANT SELECT, UPDATE ON mesh_jobs.queues TO completion_legacy;
GRANT EXECUTE ON FUNCTION mesh_jobs.claim(text,text,jsonb,bigint),
    mesh_jobs.settle_state(text,uuid,uuid,jsonb,boolean) TO completion_legacy;
ALTER FUNCTION {SIGNATURE} OWNER TO completion_migrator;
GRANT EXECUTE ON FUNCTION {SIGNATURE} TO completion_worker;
GRANT CREATE ON SCHEMA mesh_jobs TO completion_migrator;
"""

META = f"""
SELECT json_build_object('oid', oid, 'owner', proowner::regrole::text, 'acl', proacl,
    'invoker', NOT prosecdef, 'volatility', provolatile, 'settings', proconfig,
    'worker', has_function_privilege('completion_worker', oid, 'EXECUTE'),
    'legacy', has_function_privilege('completion_legacy', oid, 'EXECUTE'),
    'app', has_function_privilege('completion_app', oid, 'EXECUTE'))
FROM pg_proc WHERE oid = '{SIGNATURE}'::regprocedure;
"""


def seed(db, running, layout="scattered", total=1024):
    running_filter = f"i <= {running}" if layout == "clustered" else f"(i * 37) % 1024 < {running}"
    db.sql(f"""
    TRUNCATE mesh_jobs.jobs, mesh_jobs.batch_context_blocks, mesh_jobs.queues RESTART IDENTITY;
    INSERT INTO mesh_jobs.queues(namespace,name,concurrency)
    VALUES ('dense','default',64), ('shadow','default',64);
    WITH fixture AS (
        SELECT ns, i, (ns = 'shadow' OR {running_filter}) AS active
        FROM (VALUES ('dense'), ('shadow')) n(ns)
        CROSS JOIN LATERAL generate_series(1, CASE WHEN ns = 'dense' THEN {total} ELSE 8 END) g(i)
    )
    INSERT INTO mesh_jobs.jobs(namespace,id,worker,version,queue,args,metadata,tags,state,
        priority,attempt,max_attempts,timeout_ms,backoff_base_ms,backoff_max_ms,
        inserted_at_ms,scheduled_at_ms,lease_token,lease_expires_at_ms)
    SELECT ns,md5(i::text)::uuid,'a',1,'default',jsonb_build_object('i',i,'p',repeat('x',1024)),
        '{{}}','[]',CASE WHEN active THEN 'executing' ELSE 'available' END,
        0,active::int,3,1000,0,0,0,0,
        CASE WHEN active THEN md5('lease-' || i)::uuid END,
        CASE WHEN active THEN 9000000000000 + i END
    FROM fixture ORDER BY ns, i;
    ANALYZE mesh_jobs.jobs;
    """)


def ident(i):
    return "NULL" if i is None else f"md5('{i}')::uuid"


def where(i):
    return f"namespace='dense' AND id={ident(i)}"


def params(ids, tokens=None, ns="'dense'"):
    if tokens is None:
        tokens = [None if i is None else f"lease-{i}" for i in ids]
    def array(values):
        return "ARRAY[" + ",".join(map(ident, values)) + "]::uuid[]" if values is not None else "NULL::uuid[]"
    return ", ".join((ns, array(ids), array(tokens)))


def call(ids, tokens=None, ns="'dense'"):
    return f"SELECT ordinal FROM mesh_jobs.complete_batch_v1({params(ids, tokens, ns)}) ORDER BY ordinal;"


def worker(sql):
    return "SET ROLE completion_worker;\n" + sql


def parse_lines(value):
    return [int(x) for x in value.splitlines()]


def nodes(node):
    yield node
    for child in node.get("Plans", []):
        yield from nodes(child)


def scan_fields(plan):
    keys = ("Node Type", "Index Name", "Index Cond", "Filter", "Actual Rows", "Actual Loops",
            "Rows Removed by Filter", "Shared Hit Blocks", "Shared Read Blocks")
    return {k: plan[k] for k in keys if k in plan}


def prepare(ddl):
    body = ddl.split("$$")[1]
    body = re.sub(r"\b(ns|ids|tokens)\b", lambda m: {"ns": "$1", "ids": "$2", "tokens": "$3"}[m[0]], body)
    return "PREPARE completion(text,uuid[],uuid[]) AS " + body.rstrip() + ";"


def plan_matrix(db, ddls):
    rows = []
    options = "ANALYZE, TIMING OFF, SUMMARY OFF, BUFFERS, COSTS OFF, VERBOSE, FORMAT JSON"
    for layout in ("clustered", "scattered"):
        for running in (0, 1, 4, 8, 16, 32, 48, 64):
            seed(db, running, layout)
            for mode in ("force_custom_plan", "force_generic_plan"):
                for variant, ddl in ddls.items():
                    commands = [worker(f"SET plan_cache_mode={mode};"), prepare(ddl)]
                    good = [i for i in range(1, 1025)
                            if (i <= running if layout == "clustered" else (i * 37) % 1024 < running)]
                    picks = good + [2000 + i for i in range(8)]
                    for width in range(1, 9):
                        args = params(picks[:width])
                        commands += [f"\\echo PLAN_{width}", "BEGIN;",
                                     f"EXPLAIN ({options}) EXECUTE completion({args});", "ROLLBACK;"]
                    output = db.sql("\n".join(commands))
                    for width, raw in enumerate(re.split(r"PLAN_\d+\n", output)[1:], 1):
                        plan = json.loads(raw)[0]["Plan"]
                        held, = [n for n in nodes(plan) if n.get("Subplan Name") == "CTE held"]
                        upd, = [n for n in nodes(plan) if n.get("Subplan Name") == "CTE completed"]
                        assert plan["Actual Rows"] == min(running, width)
                        rows.append(dict(layout=layout, running=running, mode=mode, width=width, variant=variant,
                                         output=plan["Actual Rows"], held_root=scan_fields(held),
                                         held=[scan_fields(n) for n in nodes(held) if n.get("Relation Name") == "jobs"],
                                         update=[scan_fields(n) for n in nodes(upd) if n.get("Relation Name") == "jobs"
                                                 and n["Node Type"] != "ModifyTable"]))
    return rows



def state(db, i):
    return db.sql(f"""SELECT state || ':' || (lease_token IS NULL)::text || ':' ||
                  (finished_at_ms IS NOT NULL)::text FROM mesh_jobs.jobs WHERE {where(i)};""")


def install(db, ddl, metadata):
    db.sql("SET ROLE completion_migrator;\n" + ddl)
    actual = json.loads(db.sql(META))
    assert actual == metadata and actual["invoker"] and actual["worker"]
    assert not actual["legacy"] and not actual["app"] and actual["volatility"] == "v"


def verify_rows(db):
    seed(db, 8, "clustered")
    db.sql(f"""
      UPDATE mesh_jobs.jobs SET lease_expires_at_ms=0 WHERE {where(2)};
      UPDATE mesh_jobs.jobs SET cancel_requested_at_ms=1 WHERE {where(3)};
    """)
    checks = {}
    def probe(name, ids, tokens, expected, ns="'dense'"):
        checks[name] = got = parse_lines(db.sql(worker("BEGIN;\n" + call(ids, tokens, ns) + "\nROLLBACK;")))
        assert got == expected, (name, got)

    probe("mixed_fences", [2, 1, 3, 4, 5, 100, 6, None],
          ["lease-2", "lease-1", "lease-3", "foreign", None, "lease-100", "lease-6", "lease-6"], [2, 7])
    probe("duplicate_mixed_tokens", [4, 4, 4], ["foreign", "lease-4", "foreign"], [2])
    probe("mismatched_lengths", [1, 4], ["lease-1"], [])
    probe("oversized", list(range(1, 10)), None, [])
    probe("empty", [], [], [])
    probe("null_array", None, ["lease-1"], [])
    probe("null_namespace", [1], None, [], "NULL")
    lower_bounds = """SELECT ordinal FROM mesh_jobs.complete_batch_v1('dense',
      array_fill(md5('1')::uuid, ARRAY[1], ARRAY[7]),
      array_fill(md5('lease-1')::uuid, ARRAY[1], ARRAY[-3]));"""
    assert db.sql(worker("BEGIN;\n" + lower_bounds + "\nROLLBACK;")) == "1"
    duplicate = parse_lines(db.sql(worker("BEGIN;\n" + call([5, 4, 5, 4]) + "\nROLLBACK;")))
    assert len(duplicate) == 2 and len({[5, 4, 5, 4][n-1] for n in duplicate}) == 2
    checks["identical_duplicates_returned_ordinals"] = duplicate

    with db.hold(worker(call([1, 4], ["lease-1"]))) as conn:
        db.sql(f"SELECT id FROM mesh_jobs.jobs WHERE {where(1)} FOR NO KEY UPDATE NOWAIT;")
        output, err = conn.communicate("ROLLBACK;\n", timeout=10)
        assert conn.returncode == 0, err
        assert not output.strip()
    checks["rejected_cardinality_keeps_no_row_lock"] = True

    for role in ("completion_legacy", "completion_app"):
        db.sql(f"SET ROLE {role};\n" + call([1]), error="42501")

    with db.hold(f"SELECT id FROM mesh_jobs.jobs WHERE {where(1)} FOR NO KEY UPDATE \\gset"):
        checks["locked_prefix"] = got = parse_lines(db.sql(worker(call([1, 6, 7]))))
        assert got == [2, 3] and state(db, 1) == "executing:false:false"
    assert state(db, 6) == state(db, 7) == "completed:true:true"

    with db.hold(worker(call([1, 8]) + "RESET ROLE;")) as conn:
        assert state(db, 1) == state(db, 8) == "executing:false:false"
        assert db.sql(worker(call([1, 8]))) == ""
        output, err = conn.communicate("ROLLBACK;\n", timeout=10)
        assert conn.returncode == 0, err
        assert parse_lines(output) == [1, 2]
    checks["completion_retry_after_rollback"] = got = parse_lines(db.sql(worker(call([1, 8]))))
    assert got == [1, 2] and state(db, 1) == state(db, 8) == "completed:true:true"
    assert db.sql(worker(call([1, 8]))) == ""
    assert db.sql("SELECT count(*) FROM mesh_jobs.jobs WHERE namespace='shadow' AND state='executing';") == "8"

    legacy = "SET ROLE completion_legacy;\n"
    claimed = db.sql(legacy + """SELECT id, token FROM mesh_jobs.claim('dense','default',
        '[{"worker":"a","version":1}]'::jsonb, 600000);""")
    job, token = claimed.split("|")
    assert db.sql(worker(f"SELECT ordinal FROM mesh_jobs.complete_batch_v1('dense', ARRAY['{job}']::uuid[],"
                         f" ARRAY['{token}']::uuid[]);")) == "1"
    assert db.sql(legacy + f"""SELECT mesh_jobs.settle_state('dense','{job}','{token}',
        '{{"action":"complete"}}'::jsonb,false) IS NULL;""") == "t"
    assert db.sql(legacy + f"""SELECT mesh_jobs.settle_state('dense',{ident(3)},{ident('lease-3')},
        '{{"action":"complete"}}'::jsonb,false);""") == "cancelled"
    checks["legacy_claim_group_settlement_and_scalar_cancel"] = True
    return checks


RLS = f"""
CREATE FUNCTION mesh_jobs.completion_race_gate(job_id uuid) RETURNS boolean
LANGUAGE plpgsql VOLATILE SET search_path = pg_catalog, pg_temp AS $$
BEGIN
    IF current_setting('mesh_jobs.completion_probe', true) = 'active' AND job_id = {ident(1)}
    THEN PERFORM pg_advisory_xact_lock({RACE_KEY}); END IF;
    RETURN true;
END
$$;
ALTER TABLE mesh_jobs.jobs ENABLE ROW LEVEL SECURITY;
CREATE POLICY replay_select ON mesh_jobs.jobs FOR SELECT TO completion_worker
USING (mesh_jobs.completion_race_gate(id));
CREATE POLICY replay_update ON mesh_jobs.jobs FOR UPDATE TO completion_worker
USING (true) WITH CHECK (true);
"""


def verify_races(db):
    seed(db, 8, "clustered")
    db.sql(RLS)
    expected = {
        "renewed_after_snapshot": ("lease_expires_at_ms=9000000000000", [1, 2]),
        "cancelled_after_snapshot": ("cancel_requested_at_ms=1", [2]),
        "rotated_after_snapshot": (f"lease_token={ident('new-lease')}", [2]),
        "expired_after_snapshot": ("lease_expires_at_ms=0", [2]),
    }
    result = {}
    try:
        for label, (change, ordinals) in expected.items():
            seed(db, 8, "clustered")
            if label == "renewed_after_snapshot":
                db.sql(f"UPDATE mesh_jobs.jobs SET lease_expires_at_ms=0 WHERE {where(1)};")
            with db.hold(f"SELECT pg_advisory_xact_lock({RACE_KEY}) \\gset") as gate:
                query = worker(f"""SET application_name = '{label}';
                    SET mesh_jobs.completion_probe = 'active';
                    SET statement_timeout = '10s';
                    {call([1, 2])}""")
                with db.session(query) as running:
                    db.wait_advisory(label)
                    db.sql(f"UPDATE mesh_jobs.jobs SET {change} WHERE {where(1)};")
                    _, err = gate.communicate("COMMIT;\n", timeout=10)
                    assert gate.returncode == 0, err
                    output, err = running.communicate(timeout=10)
                    assert running.returncode == 0, err
            got = parse_lines(output)
            assert got == ordinals, (label, got, ordinals)
            result[label] = dict(ordinals=got, job1=state(db, 1), job2=state(db, 2))
    finally:
        db.sql("""
           DROP POLICY replay_select ON mesh_jobs.jobs;
           DROP POLICY replay_update ON mesh_jobs.jobs;
           ALTER TABLE mesh_jobs.jobs DISABLE ROW LEVEL SECURITY;
           DROP FUNCTION mesh_jobs.completion_race_gate(uuid);
        """)
    return result


def verify_clock_after_lock(db):
    seed(db, 2, "clustered")
    db.sql(f"""
        ALTER FUNCTION mesh_jobs.now_ms() RENAME TO completion_original_clock;
        CREATE FUNCTION mesh_jobs.now_ms() RETURNS bigint
        LANGUAGE plpgsql VOLATILE SET search_path = pg_catalog, pg_temp AS $$
        BEGIN
            IF current_setting('mesh_jobs.completion_probe', true) = 'active' THEN
                PERFORM pg_advisory_xact_lock({RACE_KEY});
                RETURN 200;
            END IF;
            RETURN mesh_jobs.completion_original_clock();
        END $$;
        UPDATE mesh_jobs.jobs SET lease_expires_at_ms=200 WHERE {where(1)};
        UPDATE mesh_jobs.jobs SET lease_expires_at_ms=201 WHERE {where(2)};
    """)
    try:
        with db.hold(f"SELECT pg_advisory_xact_lock({RACE_KEY}) \\gset") as gate:
            with db.session(worker(f"""SET application_name='clock_gate';
                SET mesh_jobs.completion_probe='active'; SET statement_timeout='10s';
                {call([1, 2])}""")) as running:
                db.wait_advisory("clock_gate")
                db.sql(f"SELECT id FROM mesh_jobs.jobs WHERE {where(1)} FOR KEY SHARE NOWAIT;")
                db.sql(f"SELECT id FROM mesh_jobs.jobs WHERE {where(1)} FOR NO KEY UPDATE NOWAIT;", error="55P03")
                _, err = gate.communicate("COMMIT;\n", timeout=10)
                assert gate.returncode == 0, err
                output, err = running.communicate(timeout=10)
                assert running.returncode == 0, err
        assert parse_lines(output) == [2] and state(db, 2) == "completed:true:true"
        assert db.sql(f"SELECT finished_at_ms FROM mesh_jobs.jobs WHERE {where(2)};") == "200"
        assert state(db, 1) == "executing:false:false"
    finally:
        db.sql("""DROP FUNCTION mesh_jobs.now_ms();
                  ALTER FUNCTION mesh_jobs.completion_original_clock() RENAME TO now_ms;""")
    return {"clock": 200, "expires_at_clock": "executing", "expires_after_clock": "completed"}


def functional_matrix(db, ddls):
    metadata = json.loads(db.sql(META))
    checks = {}
    for variant, ddl in ddls.items():
        install(db, ddl, metadata)
        checks[variant] = dict(contract=verify_rows(db), race=verify_races(db), clock=verify_clock_after_lock(db))
    duplicate_key = "identical_duplicates_returned_ordinals"
    assert {name: row["contract"][duplicate_key] for name, row in checks.items()} == {
        "stock": [3, 4], "array": [1, 2], "range": [3, 4], "lateral": [1, 2],
    }
    stock = checks["stock"]
    for row in checks.values():
        assert row["race"] == stock["race"] and row["clock"] == stock["clock"]
        assert {k: v for k, v in row["contract"].items() if k != duplicate_key} == {
            k: v for k, v in stock["contract"].items() if k != duplicate_key
        }
    return checks


def summarize(rows):
    dimensions = ("layout", "running", "mode", "width")
    stock = {tuple(row[k] for k in dimensions): row for row in rows if row["variant"] == "stock"}

    def hits(row):
        return sum(n.get("Shared Hit Blocks", 0) for n in row["held"])

    result = {}
    for row in rows:
        if row["variant"] == "stock":
            continue
        own = hits(row)
        baseline = hits(stock[tuple(row[k] for k in dimensions)])
        direction = "more" if own > baseline else "less" if own < baseline else "equal"
        counts = result.setdefault(row["variant"], dict(more=0, less=0, equal=0, dense_wide_more=0,
                                                        dense_wide_less=0, dense_wide_equal=0,
                                                        point_key=0, generic_point_key=0))
        counts[direction] += 1
        if row["running"] >= 16 and row["width"] >= 4:
            counts["dense_wide_" + direction] += 1
        if any(n.get("Index Name") == "jobs_pkey"
               and ".namespace =" in n.get("Index Cond", "")
               and ".id =" in n.get("Index Cond", "") for n in row["held"]):
            counts["point_key"] += 1
            if row["mode"] == "force_generic_plan":
                counts["generic_point_key"] += 1
    return result


def main():
    schema = gzip.decompress((HERE / "schema-b3557273.sql.gz").read_bytes())
    assert sha256(schema).hexdigest() == SCHEMA_SHA
    ddls = variants(definition(schema.decode()))
    for variant, ddl in ddls.items():
        assert (HERE / f"{variant}.sql").read_text() == ddl, variant
    with tempfile.TemporaryDirectory(prefix="mesh-held-", dir="/tmp") as directory:
        db = Server(directory)
        with db.start():
            version = db.sql("SHOW server_version_num")
            assert version == "170011", version
            db.sql(schema.decode() + ROLES)
            contract = functional_matrix(db, ddls)
            result = plan_matrix(db, ddls)
    print(json.dumps(dict(
        postgres=version, contract=contract, counts=summarize(result), plans=result,
        schema_sha256=SCHEMA_SHA, sql_sha256={k: sha256(v.encode()).hexdigest() for k, v in ddls.items()},
    ), sort_keys=True, separators=(",", ":")))


if __name__ == "__main__":
    main()
