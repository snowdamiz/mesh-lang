"""Replay the rejected PG17 claim overlay using disposable SQL fixtures only."""

from contextlib import contextmanager
from hashlib import sha256
import json
import os
from pathlib import Path
import re
import select
import subprocess
import tarfile
from tempfile import TemporaryDirectory
from time import monotonic, sleep

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
REVISION = "b3557273d72be3802bf189b284e7caaf5f3caa0f"
SCHEMA = "compiler/mesh-rt/src/jobs/schema.sql"
SIGNATURE = "mesh_jobs.claim_batch_set_v1(text,text,jsonb,bigint,integer,bigint)"
MODES = ("force_custom_plan", "force_generic_plan", "auto")
WORKERS = """'[{"worker":"a","version":1},{"worker":"b","version":2}]'::jsonb"""
PARAMETERS = "text,text,jsonb,bigint,integer,bigint"
GATE_KEY = 4738561
SETUP_ROLE = """
CREATE ROLE grant_worker;
GRANT USAGE ON SCHEMA mesh_jobs TO grant_worker;
GRANT SELECT, UPDATE ON mesh_jobs.jobs, mesh_jobs.queues TO grant_worker;
GRANT SELECT ON mesh_jobs.batch_context_blocks TO grant_worker;
GRANT EXECUTE ON FUNCTION """ + SIGNATURE + " TO grant_worker;"
FUNCTION_INFO = f"""
SELECT json_build_array(oid, proowner, proacl, prosecdef, proconfig, provolatile)
FROM pg_proc WHERE oid = '{SIGNATURE}'::regprocedure;
"""


def original(path):
    return subprocess.check_output(["git", "show", f"{REVISION}:{path}"], cwd=ROOT)


def definition(schema):
    return re.search(r"CREATE OR REPLACE FUNCTION mesh_jobs\.claim_batch_set_v1.*?^\$\$;",
                     schema, re.S | re.M)[0] + "\n"


def statement(ddl):
    query = re.search(r"RETURN QUERY\n    (WITH picked AS MATERIALIZED .*?FROM granted g ORDER BY [^;]+);",
                      ddl, re.S)[1]
    names = ("ns", "queue_name", "workers", "lease_ms", "remaining", "due_at")
    binds = {name: f"${i}" for i, name in enumerate(names, 1)}
    return re.sub(r"\b(" + "|".join(names) + r")\b", lambda m: binds[m[0]], query)


class Cluster:
    def __init__(self, directory):
        self.directory = directory
        self.bin = Path(os.environ.get("PG17_BIN", "/opt/homebrew/opt/postgresql@17/bin"))
        self.data = directory / "data"
        self.client = [self.bin / "psql", "-XqAt", "-h", directory, "-d", "postgres",
                       "-v", "ON_ERROR_STOP=1", "-v", "VERBOSITY=verbose"]

    def tool(self, name, *args):
        return subprocess.run([self.bin / name, *args], check=True, capture_output=True, text=True)

    @contextmanager
    def start(self):
        self.tool("initdb", "-A", "trust", "--no-locale", "-E", "UTF8", "-D", self.data)
        try:
            self.tool("pg_ctl", "-D", self.data, "-l", self.directory / "postgres.log",
                      "-o", f"-c listen_addresses='' -k '{self.directory}'", "-w", "start")
            yield
        finally:
            if (self.data / "postmaster.pid").exists():
                self.tool("pg_ctl", "-D", self.data, "-m", "fast", "-w", "stop")

    def sql(self, sql, *, worker=False, error=None):
        prefix = "SET ROLE grant_worker;\n" if worker else ""
        result = subprocess.run(self.client, input=prefix + sql, capture_output=True, text=True, timeout=15)
        if error:
            assert result.returncode == 3 and f"ERROR:  {error}:" in result.stderr, result.stderr
        else:
            assert result.returncode == 0, result.stderr
        return result.stdout.strip()

    @contextmanager
    def session(self, sql):
        with subprocess.Popen(self.client, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, text=True) as child:
            try:
                child.stdin.write(sql.rstrip() + "\n")
                child.stdin.flush()
                yield child
            finally:
                if child.poll() is None:
                    child.kill()
                    child.communicate()

    @contextmanager
    def hold(self, sql, finish="ROLLBACK;"):
        with self.session("BEGIN;\n" + sql + "\n\\echo ready") as child:
            assert select.select([child.stdout], [], [], 10)[0] and child.stdout.readline().strip() == "ready"
            yield
            output, _ = child.communicate(finish + "\n", timeout=10)
            assert child.returncode == 0, output


def seed(db, ready, running=0, total=1024):
    db.sql(f"""
    TRUNCATE mesh_jobs.jobs, mesh_jobs.batch_context_blocks, mesh_jobs.queues RESTART IDENTITY;
    INSERT INTO mesh_jobs.queues(namespace,name,concurrency)
    VALUES ('dense','default',64), ('dense','other',64), ('shadow','default',64);
    INSERT INTO mesh_jobs.jobs(namespace,id,worker,version,queue,args,metadata,tags,state,
        priority,attempt,max_attempts,timeout_ms,backoff_base_ms,backoff_max_ms,
        inserted_at_ms,scheduled_at_ms,lease_token,lease_expires_at_ms,finished_at_ms)
    SELECT 'dense',md5(i::text)::uuid,'a',1,'default',jsonb_build_object('i',i,'p',repeat('x',1024)),
        '{{}}','[]',CASE WHEN i <= {ready} THEN 'available'
          WHEN i <= {ready}+{running} THEN 'executing' ELSE 'completed' END,
        0,(i > {ready})::int,3,1000,0,0,0,0,
        CASE WHEN i > {ready} AND i <= {ready}+{running} THEN gen_random_uuid() END,
        CASE WHEN i > {ready} AND i <= {ready}+{running} THEN 60000 END,
        CASE WHEN i > {ready}+{running} THEN 1 END
    FROM generate_series(1,{total}) AS i ORDER BY i;
    INSERT INTO mesh_jobs.jobs(namespace,id,worker,version,queue,args,metadata,tags,state,
        priority,max_attempts,timeout_ms,backoff_base_ms,backoff_max_ms,inserted_at_ms,scheduled_at_ms)
    VALUES ('shadow',md5('1')::uuid,'a',1,'default','{{}}','{{}}','[]','available',0,3,1000,0,0,0,0);
    ANALYZE mesh_jobs.jobs;
    """)


def claim(slots=16):
    return f"mesh_jobs.claim_batch_set_v1('dense','default',{WORKERS},60000,{slots},0)"


def rows(db, *, slots=16, pre="", finish="ROLLBACK", error=None):
    output = db.sql(f"""BEGIN; {pre}
        SELECT coalesce(json_agg(r ORDER BY r.ordinality),'[]') FROM {claim(slots)} WITH ORDINALITY r;
        {finish};""", worker=True, error=error)
    return None if error else json.loads(output)


def ids(rows_):
    return [json.loads(r["context"])["args"]["i"] for r in rows_]


def verify_contract(db):
    seed(db, 10, total=10)
    before = db.sql("SELECT xmin,ctid FROM mesh_jobs.jobs WHERE namespace='dense' AND position=2;")
    db.sql("""
      UPDATE mesh_jobs.jobs SET worker='b',version=2,args=args || '{"latest":true}' WHERE position=2;
      UPDATE mesh_jobs.jobs SET priority=1 WHERE position=1;
      UPDATE mesh_jobs.jobs SET scheduled_at_ms=1 WHERE position=4;
      UPDATE mesh_jobs.jobs SET version=3 WHERE position=5;
      UPDATE mesh_jobs.jobs SET queue='other' WHERE position=6;
      UPDATE mesh_jobs.jobs SET attempt=max_attempts WHERE position=7;
      UPDATE mesh_jobs.jobs SET state='suspended' WHERE position=8;
      INSERT INTO mesh_jobs.batch_context_blocks
      SELECT namespace,id,xmin,ctid,current_setting('client_encoding')::name,1
      FROM mesh_jobs.jobs WHERE position=9;
    """)
    after = db.sql("SELECT xmin,ctid FROM mesh_jobs.jobs WHERE namespace='dense' AND position=2;")
    assert all(a != b for a, b in zip(before.split("|"), after.split("|"), strict=True))
    selected = rows(db, finish="COMMIT")
    assert ids(selected) == [2, 3, 10, 1]
    context = json.loads(selected[0]["context"])
    assert (context["worker"], context["version"], context["args"]["latest"]) == ("b", 2, True)
    for row in selected:
        context = json.loads(row["context"])
        assert row["status"] == "claimed" and row["timeout_ms"] == "1000"
        assert row["id"] == context["id"] and context["effect_key"] == row["id"] + "/1"
        assert context["attempt"] == 1 and int(row["context_bytes"]) == len(row["context"].encode())
        assert db.sql(f"""
        SELECT state='executing' AND attempt=1 AND lease_token='{row["token"]}'::uuid
          AND lease_expires_at_ms=attempted_at_ms+60000
        FROM mesh_jobs.jobs WHERE namespace='dense' AND id='{row["id"]}'::uuid;
        """) == "t"
    assert db.sql("SELECT state FROM mesh_jobs.jobs WHERE namespace='shadow';") == "available"
    assert rows(db)[0]["status"] == "empty"

    seed(db, 2, total=2)
    assert ids(rows(db)) == [1, 2]
    assert db.sql("SELECT count(*) FROM mesh_jobs.jobs WHERE state='executing';") == "0"
    with db.hold("SELECT id AS held FROM mesh_jobs.jobs WHERE position=1 FOR NO KEY UPDATE \\gset"):
        assert ids(rows(db)) == [2]
        db.sql("UPDATE mesh_jobs.jobs SET scheduled_at_ms=1 WHERE position=2;")
        assert [r["status"] for r in rows(db)] == ["fallback"]
    db.sql("UPDATE mesh_jobs.jobs SET args=jsonb_build_object('i',1,'p',repeat('x',70000)) WHERE position=1;")
    assert [r["status"] for r in rows(db)] == ["fallback"]
    assert db.sql("SELECT count(*) FROM mesh_jobs.jobs WHERE state='executing';") == "0"
    assert [r["status"] for r in rows(db, pre="SET TRANSACTION ISOLATION LEVEL REPEATABLE READ;")] == ["fallback"]

    seed(db, 20, total=20)
    assert rows(db, slots=0) == []
    assert len(rows(db, slots=64)) == 16
    db.sql("UPDATE mesh_jobs.queues SET concurrency=1 WHERE namespace='dense' AND name='default';")
    with db.hold(f"SET ROLE grant_worker;\nSELECT id AS first FROM {claim(20)} \\gset", "COMMIT;"):
        rows(db, error="55P03")
    assert rows(db) == []
    assert db.sql("SELECT count(*) FROM mesh_jobs.jobs WHERE state='executing';") == "1"
    db.sql("UPDATE mesh_jobs.queues SET concurrency=2 WHERE namespace='dense' AND name='default';")
    assert ids(rows(db, finish="COMMIT")) == [2]
    assert db.sql("SELECT count(DISTINCT lease_token) FROM mesh_jobs.jobs WHERE state='executing';") == "2"
    with db.hold("SELECT pg_advisory_xact_lock(hashtextextended('dense',1782495106)) AS lock \\gset"):
        assert rows(db) == []
    db.sql("""UPDATE mesh_jobs.queues SET concurrency=64 WHERE namespace='dense' AND name='default';
        INSERT INTO mesh_jobs.queues VALUES ('dense','',1,'paused');""")
    assert rows(db) == []
    return ["mixed handlers, due/state/attempt/queue eligibility and row-version block",
            "same ID in another namespace stays available", "ordered context and committed lease",
            "pre-statement row update uses new xmin/ctid", "rollback", "skip locked; sole busy row fallback",
            "oversize and isolation fallback", "slots, 16 cap, queue capacity and two claimers",
            "namespace lock and pause fences"]


def layout(db):
    return json.loads(db.sql("""
        SELECT json_build_object('kind',c.relkind,'am',a.amname,'has_subclass',c.relhassubclass,
            'children',(SELECT count(*) FROM pg_inherits WHERE inhparent=c.oid),
            'row_security',c.relrowsecurity,
            'triggers',(SELECT json_agg(t.tgname ORDER BY t.tgname)
                FROM pg_trigger t WHERE t.tgrelid=c.oid AND NOT t.tgisinternal))
        FROM pg_class c LEFT JOIN pg_am a ON a.oid=c.relam WHERE c.oid='mesh_jobs.jobs'::regclass;
    """))


def supported(info):
    return (info["kind"], info["am"], info["has_subclass"], info["children"], info["row_security"]) == (
        "r", "heap", False, 0, False)


def check_layout_fallback(db, stock, candidate):
    # Installation admission runs on an isolated schema. There is no runtime DDL fence.
    seed(db, 1, total=1)
    db.sql("""
        CREATE TABLE mesh_jobs.inherited_jobs () INHERITS (mesh_jobs.jobs);
        INSERT INTO mesh_jobs.inherited_jobs OVERRIDING SYSTEM VALUE
            SELECT * FROM ONLY mesh_jobs.jobs WHERE namespace='dense';
        DELETE FROM ONLY mesh_jobs.jobs WHERE namespace='dense';
        GRANT SELECT, UPDATE ON mesh_jobs.inherited_jobs TO grant_worker;
    """)
    inherited = layout(db)
    installed = stock if not supported(inherited) else candidate
    db.sql(installed)
    assert installed == stock and ids(rows(db, finish="COMMIT")) == [1]
    assert db.sql("SELECT state FROM mesh_jobs.inherited_jobs;") == "executing"
    # Partitioned parent is also rejected. This layout is outside the pinned schema.
    db.sql("""
        ALTER TABLE mesh_jobs.jobs RENAME TO jobs_original;
        CREATE TABLE mesh_jobs.jobs (id uuid) PARTITION BY HASH(id);
        CREATE TABLE mesh_jobs.jobs_p0 PARTITION OF mesh_jobs.jobs FOR VALUES WITH (modulus 1,remainder 0);
    """)
    partitioned = layout(db)
    assert not supported(partitioned)
    db.sql("DROP TABLE mesh_jobs.jobs; ALTER TABLE mesh_jobs.jobs_original RENAME TO jobs;")
    return {"inheritance": inherited, "partitioned_parent": partitioned}


def execute(slots=16):
    return f"EXECUTE grant_replay('dense','default',{WORKERS},60000,{slots},0)"


def nodes(plan):
    yield plan
    for child in plan.get("Plans", []):
        yield from nodes(child)


def target_plan(plan, *, actual=False):
    modify, = [n for n in nodes(plan) if n["Node Type"] == "ModifyTable"]
    fields = ("Node Type", "Index Name", "Index Cond", "TID Cond", "Recheck Cond", "Filter")
    if actual:
        fields += ("Actual Rows", "Actual Loops", "Rows Removed by Filter", "Shared Hit Blocks")
    return [
        {key: n[key] for key in fields if key in n}
        for n in nodes(modify)
        if (n.get("Relation Name") == "jobs" and n["Node Type"] != "ModifyTable")
        or n["Node Type"] == "Bitmap Index Scan"
    ]


def verify_tid_plan(plan):
    modify, = [node for node in nodes(plan) if node["Node Type"] == "ModifyTable"]
    assert [n["Node Type"] for n in target_plan(plan)] == ["Tid Scan"]
    for node in nodes(modify):
        inputs = node.get("Plans", [])
        if len(inputs) != 2:
            continue
        outer, inner = inputs
        if (node["Node Type"] == "Nested Loop" and outer.get("CTE Name") == "sized"
                and inner["Node Type"] == "Tid Scan"):
            assert "row_location" in inner["TID Cond"] and ".ctid" in inner["TID Cond"]
            return "one TID per sized row"

    join, = [node for node in modify["Plans"] if node["Parent Relationship"] == "Outer"]
    build, budget = join["Plans"]
    assert join["Node Type"] == "Nested Loop" and budget.get("CTE Name") == "budget"
    scan, hashed = build["Plans"]
    assert build["Node Type"] == "Hash Join" and scan["Node Type"] == "Tid Scan"
    assert "ANY" in scan["TID Cond"] and hashed["Plans"][0].get("CTE Name") == "sized"
    init, = [node for node in modify["Plans"] if node["Parent Relationship"] == "InitPlan"]
    assert {node.get("CTE Name") for node in init["Plans"]} == {"budget", "sized"}
    assert any(node.get("Filter") == "gate.fits" for node in init["Plans"])
    return "one scan of the budgeted TID array"


def plans(db, query):
    output = {}
    for shape, (ready, running, slots) in {
        "head16": (1024, 0, 16), "mid16": (512, 32, 16), "mid8": (512, 48, 8),
        "tail8": (8, 8, 8), "empty": (0, 0, 8),
    }.items():
        seed(db, ready, running)
        output[shape] = {}
        for mode in MODES:
            warm = ""
            if mode == "auto":
                warm = (f"SAVEPOINT undo; {execute(slots)} \\g /dev/null\nROLLBACK TO SAVEPOINT undo;\n") * 5
            raw = db.sql(f"""
                SET plan_cache_mode={mode};
                PREPARE grant_replay({PARAMETERS}) AS {query};
                BEGIN;
                {warm}
                EXPLAIN (VERBOSE, COSTS OFF, FORMAT JSON) {execute(slots)};
                SELECT json_build_array(custom_plans,generic_plans) FROM pg_prepared_statements
                    WHERE name='grant_replay';
                ROLLBACK;
            """, worker=True)
            plan, counts = raw.rsplit("\n", 1)
            parsed = json.loads(plan)[0]["Plan"]
            output[shape][mode] = {
                "custom_generic": json.loads(counts),
                "target": target_plan(parsed),
                "plan": parsed,
            }
    return output


def historical():
    native = HERE.parent / "2026-11-24-public-dense-bounded-grant/evidence/native"
    archive, = native.glob("blocks/00-*/measured-qualify/measured-mesh/raw.tar.gz")
    with tarfile.open(archive) as raw:
        entry = raw.extractfile("adapter/grant-plans.json").read()
    retained = json.loads(entry)
    matrix = {}
    for shape, modes in retained["plans"].items():
        matrix[shape] = {}
        for mode, arms in modes.items():
            stock = arms["stock"]
            target = target_plan(stock["plan"]["Plan"], actual=True)
            matrix[shape][mode] = {"custom_generic": [stock["custom"], stock["generic"]], "target": target}
    head = matrix["head16"]
    forced, = [n for n in head["force_custom_plan"]["target"] if n["Node Type"] == "Bitmap Index Scan"]
    assert (forced["Index Name"], forced["Actual Rows"], forced["Actual Loops"]) == (
        "mesh_jobs_unique_candidates", 1032, 16)
    auto, = head["auto"]["target"]
    assert (auto["Index Name"], auto["Actual Rows"], auto["Actual Loops"]) == ("jobs_pkey", 1, 16)
    return {"source": str(archive.relative_to(ROOT)) + "#adapter/grant-plans.json",
            "member_sha256": sha256(entry).hexdigest(), "stock": matrix}


def race(db, query, *, witness=False, total=2):
    # Block the snapshot-visible old tuple before LockRows. The other session
    # commits a replacement at the same moment it releases the advisory gate.
    seed(db, total, total=total)
    old = db.sql("SELECT xmin,ctid FROM mesh_jobs.jobs WHERE position=1;")
    with db.session(f"BEGIN; SELECT pg_advisory_xact_lock({GATE_KEY}) AS held \\gset\n\\echo ready") as writer:
        assert select.select([writer.stdout], [], [], 10)[0] and writer.stdout.readline().strip() == "ready"
        label = "tid_grant_epq_reader"
        gated, count = re.subn(r"AND j.attempt < j.max_attempts",
                               "AND j.attempt < j.max_attempts AND mesh_jobs.grant_replay_gate(j.position)",
                               query, count=1)
        assert count == 1
        if witness:
            gated = gated.split("), rendered AS MATERIALIZED", 1)[0]
            gated += """)
                SELECT p.row_version,p.row_location,p.args->>'new',p.row_relation='mesh_jobs.jobs'::regclass
                FROM picked p WHERE p.position=1"""
        with db.session(f"""
            SET ROLE grant_worker;
            SET application_name = '{label}';
            SET statement_timeout = '10s';
            PREPARE grant_replay({PARAMETERS}) AS {gated};
            BEGIN;
            SELECT name AS held FROM mesh_jobs.queues WHERE namespace='dense' AND name='default'
                FOR NO KEY UPDATE \\gset
            SELECT pg_try_advisory_xact_lock_shared(hashtextextended('dense',1782495106)) AS fence \\gset
            {execute(total)};
            ROLLBACK;
            \\q
        """) as reader:
            deadline = monotonic() + 6
            while True:
                if db.sql(f"""
                    SELECT EXISTS (SELECT FROM pg_locks l JOIN pg_stat_activity a USING(pid)
                        WHERE application_name='{label}' AND locktype='advisory'
                            AND objid={GATE_KEY} AND NOT granted);
                """) == "t":
                    break
                assert monotonic() < deadline and reader.poll() is None, "reader failed to reach pre-lock gate"
                sleep(0.01)
            output, _ = writer.communicate("""
                UPDATE mesh_jobs.jobs SET args=args || '{"new":true}' WHERE position=1 RETURNING xmin,ctid;
                COMMIT;
                """, timeout=10)
            assert writer.returncode == 0, output
            new = output.strip()
            output = reader.stdout.read()
            assert reader.wait(timeout=12) == 0, output
    assert all(a != b for a, b in zip(old.split("|"), new.split("|"), strict=True))
    assert db.sql("SELECT count(*) FROM mesh_jobs.jobs WHERE state='executing';") == "0"
    if witness:
        assert output.strip() == new + "|true|t", output
        return {"old": old, "committed": new, "locked": "|".join(output.strip().split("|")[:2]),
                "locked_new_context": True, "locked_relation_matches": True}
    results = [line.split("|") for line in output.splitlines()]
    return {"old": old, "committed": new,
            "claimed": [{"i": json.loads(fields[3])["args"]["i"],
                         "new_context": json.loads(fields[3])["args"].get("new", False)}
                        for fields in results]}


def main():
    schema_bytes = original(SCHEMA)
    schema = schema_bytes.decode()
    stock = definition(schema)
    prior = HERE.parent / "2026-11-24-public-dense-bounded-grant/stock.sql"
    assert stock == prior.read_text()
    with TemporaryDirectory(prefix="tid-grant-", dir="/tmp") as private:
        temp = Path(private)
        baseline = temp / "stock.sql"
        baseline.write_text(stock)
        subprocess.run(["patch", "-s", "-o", str(temp / "tid.sql"), str(baseline),
                        str(HERE / "grant.patch")], cwd=temp, check=True)
        candidate = (temp / "tid.sql").read_text()
        db = Cluster(temp)
        with db.start():
            version = db.sql("SHOW server_version;")
            assert version.split()[0] == "17.11", version
            db.sql(schema + SETUP_ROLE)
            contract = db.sql(FUNCTION_INFO)
            heap = layout(db)
            assert supported(heap)
            checks, estimates = {}, {}
            for name, ddl in {"stock": stock, "tid": candidate}.items():
                db.sql(ddl)
                assert db.sql(FUNCTION_INFO) == contract
                checks[name] = verify_contract(db)
                estimates[name] = plans(db, statement(ddl))
            for modes in estimates["tid"].values():
                for observed in modes.values():
                    observed["target_bound"] = verify_tid_plan(observed["plan"])
            db.sql(f"""
                CREATE FUNCTION mesh_jobs.grant_replay_gate(row_position bigint) RETURNS boolean
                LANGUAGE plpgsql VOLATILE AS $$
                BEGIN
                    IF row_position = 1 THEN PERFORM pg_advisory_xact_lock({GATE_KEY}); END IF;
                    RETURN true;
                END $$;
            """)
            locked = race(db, statement(candidate), witness=True)
            raced = {str(total): {name: race(db, statement(ddl), total=total)
                                  for name, ddl in {"stock": stock, "tid": candidate}.items()}
                     for total in (1, 2)}
            assert raced["1"]["stock"]["claimed"] == [{"i": 1, "new_context": True}]
            assert raced["1"]["tid"]["claimed"] == []
            assert raced["2"]["stock"]["claimed"] == [{"i": 1, "new_context": True}, {"i": 2, "new_context": False}]
            assert raced["2"]["tid"]["claimed"] == [{"i": 2, "new_context": False}]
            rejected_layouts = check_layout_fallback(db, stock, candidate)
    result = {
        "postgres": version, "pinned_revision": REVISION,
        "schema_sha256": sha256(schema_bytes).hexdigest(),
        "function_sha256": {name: sha256(ddl.encode()).hexdigest()
                            for name, ddl in {"stock": stock, "tid": candidate}.items()},
        "pinned_local_layout": heap, "layout_rejections": rejected_layouts,
        "routine_correctness": checks, "ordinary_explain": estimates,
        "locked_row_witness": locked, "snapshot_races": raced, "historical_native": historical(),
        "disposition": "reject overlay; no native qualification or timing",
    }
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
