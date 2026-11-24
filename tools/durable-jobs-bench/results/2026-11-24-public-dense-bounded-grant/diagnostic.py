"""Exact grant statement replay and rollback-only checks on a disposable cluster."""

from contextlib import contextmanager
from hashlib import md5, sha256
import json
from pathlib import Path
import re
import select
import subprocess

HERE = Path(__file__).resolve().parent
DB = "dense_grant_probe"
SIGNATURE = "mesh_jobs.claim_batch_set_v1(text,text,jsonb,bigint,integer,bigint)"
STOCK = (HERE / "stock.sql").read_text()
JOIN = "FROM sized s, budget b WHERE b.fits AND j.namespace = ns AND j.id = s.id"
BOUND = JOIN + "\n            AND j.id = ANY (ARRAY(SELECT k.id FROM sized k))"
assert STOCK.count(JOIN) == 1
CANDIDATE = STOCK.replace(JOIN, BOUND)
WORKERS = """'[{"worker":"parity","version":1}]'::jsonb"""
MODES = ("force_custom_plan", "force_generic_plan", "auto")
SHAPES = {"head16": (1024, 0, 16), "mid16": (512, 32, 16),
          "mid8": (512, 48, 8), "tail8": (8, 8, 8), "empty": (0, 0, 8)}
EXPLAIN = "EXPLAIN (ANALYZE, BUFFERS, WAL, TIMING OFF, VERBOSE, FORMAT JSON)"


def digest(data):
    return sha256(data.encode()).hexdigest()


def body(ddl):
    return re.search(r"\bAS (\$\w*\$)(.*?)\1", ddl, re.S).group(2)


def definition(cluster, database):
    return cluster.sql(database, f"SELECT pg_get_functiondef('{SIGNATURE}'::regprocedure)")


def metadata(cluster, database):
    return json.loads(cluster.sql(database, f"""
        SELECT json_build_object('owner', proowner::regrole::text, 'acl', proacl,
            'security_definer', prosecdef, 'configuration', proconfig,
            'worker_execute', has_function_privilege('parity_subject', oid, 'EXECUTE'),
            'producer_execute', has_function_privilege('parity_producer', oid, 'EXECUTE'),
            'app_execute', has_function_privilege('compiled_caller', oid, 'EXECUTE'))
        FROM pg_proc WHERE oid='{SIGNATURE}'::regprocedure
    """))


def apply(cluster, database, candidate):
    assert database in {"mesh", DB} and cluster.data.is_dir()
    before = definition(cluster, database)
    assert body(before) == body(STOCK), "migrated grant source differs from the pinned source"
    acl = metadata(cluster, database)
    assert acl["owner"] == "parity_migrator" and not acl["security_definer"]
    assert acl["worker_execute"] and not acl["producer_execute"] and not acl["app_execute"]
    assert acl["configuration"] == ["search_path=pg_catalog, pg_temp", "lock_timeout=100ms"]
    expected = CANDIDATE if candidate else STOCK
    if candidate:
        # Replay the live declaration verbatim. CREATE OR REPLACE preserves
        # identity and grants, and the single replacement is in its body.
        assert before.count(JOIN) == 1
        cluster.sql(database, before.replace(JOIN, BOUND))
    after = definition(cluster, database)
    assert body(after) == body(expected) and metadata(cluster, database) == acl
    return dict(body_sha256=digest(body(after)), baseline_body_sha256=digest(body(before)), metadata=acl)


def grant_sql(ddl):
    # PL/pgSQL binds these five variables in the static SPI statement.
    # Match only bare variable references in the pinned statement.
    cte, = re.findall(r"    RETURN QUERY\n    (WITH picked AS MATERIALIZED .*?FROM granted g ORDER BY [^;]+);", ddl, re.S)
    params = {"ns": "$1", "queue_name": "$2", "workers": "$3",
              "lease_ms": "$4", "remaining": "$5", "due_at": "$6"}
    return re.sub(r"\b(ns|queue_name|workers|lease_ms|remaining|due_at)\b", lambda m: params[m[0]], cte)


def psql(cluster, sql, database=DB, role="parity_subject", expect=None):
    args, env = cluster.psql(database)
    if role == "parity_subject":
        args[args.index("-U") + 1] = role
    else:
        assert role in {"parity_owner", "parity_operator"}
        if role == "parity_operator":
            sql = "SET ROLE parity_operator;\n" + sql
    result = subprocess.run(args + ["-v", "VERBOSITY=verbose"], input=sql, env=env, text=True,
                            capture_output=True, timeout=30)
    with (cluster.evidence / "grant-probe.log").open("a") as log:
        log.write(f"\n--- {role}, exit {result.returncode} ---\n{sql}\n{result.stdout}{result.stderr}")
    if expect:
        assert result.returncode == 3 and re.search(rf"ERROR:\s+{expect}:", result.stderr), result.stderr
    else:
        assert result.returncode == 0, result.stderr
    return result.stdout.strip()


@contextmanager
def hold(cluster, sql, *, role="parity_subject", commit=False):
    args, env = cluster.psql(DB)
    if role == "parity_subject":
        args[args.index("-U") + 1] = role
    command = ("SET ROLE parity_operator;\n" if role == "parity_operator" else "")
    command += "BEGIN;\n" + sql + "\n\\echo fenced_ready\n"
    with (cluster.evidence / "grant-probe.log").open("a") as log:
        log.write(f"\n--- held {role} ---\n{command}")
    with subprocess.Popen(args, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                          stderr=subprocess.STDOUT, text=True) as child:
        try:
            child.stdin.write(command)
            child.stdin.flush()
            assert select.select([child.stdout], [], [], 10)[0], "lock holder failed to become ready"
            # All holder statements discard output with \gset/DO.
            line = child.stdout.readline()
            assert line.strip() == "fenced_ready", line
        finally:
            # The holder is only released after the competing action.
            if child.poll() is not None:
                with (cluster.evidence / "grant-probe.log").open("a") as log:
                    log.write(child.stdout.read())
        try:
            yield
        finally:
            try:
                tail, _ = child.communicate("COMMIT;\n" if commit else "ROLLBACK;\n", timeout=15)
            except BaseException:
                child.kill()
                tail, _ = child.communicate()
                raise
            finally:
                with (cluster.evidence / "grant-probe.log").open("a") as log:
                    log.write(tail)
            assert child.returncode == 0, tail


def seed(cluster, ready, running, *, semantic=False):
    # Deterministic jobs with dense-sized payload and the same partial indexes.
    # Fixture states are seeded outside the EXPLAIN transaction.
    total = 6 if semantic else 1024
    foreign = """
      INSERT INTO mesh_jobs.queues(namespace,name,concurrency) VALUES ('shadow','default',64);
      INSERT INTO mesh_jobs.jobs(namespace,id,worker,version,queue,args,metadata,tags,state,
          priority,max_attempts,timeout_ms,backoff_base_ms,backoff_max_ms,inserted_at_ms,scheduled_at_ms)
      VALUES ('shadow',md5('1')::uuid,'parity',1,'default','{}','{}','[]','available',0,3,1000,0,0,0,0);
    """ if semantic else ""
    psql(cluster, f"""
        TRUNCATE mesh_jobs.jobs, mesh_jobs.batch_context_blocks, mesh_jobs.queues RESTART IDENTITY;
        INSERT INTO mesh_jobs.queues(namespace,name,concurrency)
        VALUES ('dense','default',64), ('dense','other',64);
        INSERT INTO mesh_jobs.jobs(namespace,id,worker,version,queue,args,metadata,tags,state,
            priority,attempt,max_attempts,timeout_ms,backoff_base_ms,backoff_max_ms,
            inserted_at_ms,scheduled_at_ms,lease_token,lease_expires_at_ms,finished_at_ms)
        SELECT 'dense', md5(i::text)::uuid, 'parity', 1, 'default', jsonb_build_object('key', i),
            '{{}}', '[]', CASE WHEN i<={ready} THEN 'available'
              WHEN i<={ready}+{running} THEN 'executing' ELSE 'completed' END,
            CASE WHEN {str(semantic).lower()} THEN (ARRAY[1,0,0,0,0,0])[i] ELSE 0 END,
            (i>{ready})::int, 3, 1000, 0, 0, 0, 0,
            CASE WHEN i>{ready} AND i<={ready}+{running} THEN gen_random_uuid() END,
            CASE WHEN i>{ready} AND i<={ready}+{running} THEN mesh_jobs.now_ms()+60000 END,
            CASE WHEN i>{ready}+{running} THEN 1 END
        FROM generate_series(1,{total}) i ORDER BY i;
        {foreign}
    """, role="parity_owner")


def claim(slots=16, cutoff="0"):
    return f"mesh_jobs.claim_batch_set_v1('dense','default',{WORKERS},60000,{slots},{cutoff})"


def rows(cluster, slots=16, prefix="", expect=None):
    sql = f"""BEGIN; {prefix}
       SELECT coalesce(json_agg(row_to_json(r) ORDER BY r.ordinality),'[]')
       FROM {claim(slots)} WITH ORDINALITY r;
       ROLLBACK;
    """
    output = psql(cluster, sql, expect=expect)
    return json.loads(output) if not expect else None


def fixture_id(i):
    s = md5(str(i).encode()).hexdigest()
    return f"{s[:8]}-{s[8:12]}-{s[12:16]}-{s[16:20]}-{s[20:]}"


def semantics(cluster):
    seed(cluster, 6, 0, semantic=True)
    who = json.loads(psql(cluster, """
      SELECT json_build_array(current_user, session_user,
      (SELECT ssl FROM pg_stat_ssl WHERE pid=pg_backend_pid()));
    """))
    assert who == ["parity_subject", "parity_subject", True]
    # Seed a due boundary, unsupported version and wrong queue. Only the
    # worker role runs admission. The first eligible prefix is 2, 3, 1.
    psql(cluster, """UPDATE mesh_jobs.jobs SET scheduled_at_ms=1 WHERE namespace='dense' AND args->>'key'='4';
        UPDATE mesh_jobs.jobs SET version=2 WHERE namespace='dense' AND args->>'key'='5';
        UPDATE mesh_jobs.jobs SET queue='other' WHERE namespace='dense' AND args->>'key'='6';""",
         role="parity_owner")
    first = rows(cluster)
    assert [r["id"] for r in first] == [fixture_id(i) for i in (2, 3, 1)]
    for ordinal, row in enumerate(first, 1):
        context = json.loads(row["context"])
        assert (row["status"], row["ordinality"], row["timeout_ms"]) == ("claimed", ordinal, "1000")
        assert row["id"] == context["id"] and int(row["context_bytes"]) == len(row["context"].encode())
        assert context["attempt"] == 1 and context["effect_key"] == row["id"] + "/1"
        assert row["token"]
    assert psql(cluster, "SELECT count(*) FROM mesh_jobs.jobs WHERE state='executing';") == "0"
    with hold(cluster, """SELECT id AS held_id FROM mesh_jobs.jobs WHERE namespace='dense'
                 AND id=md5('2')::uuid FOR NO KEY UPDATE \n\\gset"""):
        assert [r["id"] for r in rows(cluster)] == [fixture_id(i) for i in (3, 1)]
    psql(cluster, """UPDATE mesh_jobs.jobs SET args=jsonb_build_object('blob',repeat('x',70000))
                     WHERE namespace='dense' AND id=md5('2')::uuid;""", role="parity_operator")
    assert [r["status"] for r in rows(cluster)] == ["fallback"]
    assert [r["status"] for r in rows(cluster, prefix="SET TRANSACTION ISOLATION LEVEL REPEATABLE READ;")] == ["fallback"]
    assert psql(cluster, "SELECT count(*) FROM mesh_jobs.jobs WHERE state='executing';") == "0"
    psql(cluster, """UPDATE mesh_jobs.jobs SET args='{"key":2}' WHERE namespace='dense' AND id=md5('2')::uuid;""",
         role="parity_operator")
    psql(cluster, """SELECT mesh_jobs.queue_control('dense','default','scale',1);""", role="parity_operator")
    with hold(cluster, f"""SELECT mesh_jobs.claim('dense','default',{WORKERS},60000) AS legacy
                          \\gset""", commit=True):
        rows(cluster, expect="55P03")
    assert rows(cluster) == []
    assert psql(cluster, """SELECT count(*) FROM mesh_jobs.jobs WHERE state='executing' AND id=md5('2')::uuid;""") == "1"
    with hold(cluster, """SELECT mesh_jobs.queue_control('dense','default','scale',4) AS scaled
                         \\gset""", role="parity_operator", commit=True):
        rows(cluster, expect="55P03")
    assert [r["id"] for r in rows(cluster)] == [fixture_id(i) for i in (3, 1)]
    with hold(cluster, """SELECT mesh_jobs.set_namespace_paused('dense',true) AS paused
                         \\gset""", role="parity_operator", commit=True):
        assert rows(cluster) == []
        psql(cluster, """UPDATE mesh_jobs.jobs SET state='executing', attempt=1,
           lease_token=gen_random_uuid(), lease_expires_at_ms=60000 WHERE namespace='dense'
           AND id=md5('3')::uuid;""", expect="55000")
    assert rows(cluster) == []
    psql(cluster, """SELECT mesh_jobs.set_namespace_paused('dense',false);""", role="parity_operator")
    assert rows(cluster, slots=0) == []
    with hold(cluster, f"""SELECT token AS token FROM {claim(1)}
                          \\gset""", commit=True):
        assert psql(cluster, """SELECT state FROM mesh_jobs.jobs
                 WHERE namespace='dense' AND id=md5('3')::uuid;""") == "available"
    assert psql(cluster, """SELECT state FROM mesh_jobs.jobs
             WHERE namespace='dense' AND id=md5('3')::uuid;""") == "executing"
    assert psql(cluster, """SELECT mesh_jobs.settle_state('dense',md5('3')::uuid,
                gen_random_uuid(),'{"action":"complete"}',false) IS NULL;""") == "t"
    assert psql(cluster, """SELECT mesh_jobs.settle_state('dense',id,lease_token,
                '{"action":"complete"}',false) FROM mesh_jobs.jobs
                WHERE namespace='dense' AND id=md5('3')::uuid;""") == "completed"
    assert psql(cluster, "SELECT state FROM mesh_jobs.jobs WHERE namespace='shadow';") == "available"
    return dict(worker_tls=who, ordered_rows=[r["id"] for r in first], context_and_ordinality=True,
                rollback=True, skip_locked=True, oversized_and_isolation_fallback=True,
                legacy_capacity=True, admin_fence=True, namespace_and_trigger_fence=True,
                commit_visibility_and_token=True, isolated_namespace=True)


def capture(cluster, ddl, remaining, mode):
    params = f"'dense','default',{WORKERS},60000,{remaining},0"
    execute = f"EXECUTE grant_cte({params});"
    warm = (f"SAVEPOINT reuse; {execute} ROLLBACK TO reuse; RELEASE reuse;\n" * 5
            if mode == "auto" else "")
    output = psql(cluster, f"""
      BEGIN;
      SET LOCAL search_path = pg_catalog, pg_temp;
      SET LOCAL lock_timeout = '100ms';
      SET LOCAL plan_cache_mode = {mode};
      PREPARE grant_cte(text,text,jsonb,bigint,integer,bigint) AS {grant_sql(ddl)};
      SELECT concurrency AS qcap FROM mesh_jobs.queues
        WHERE namespace='dense' AND name='default' FOR NO KEY UPDATE
      \\gset
      SELECT pg_try_advisory_xact_lock_shared(hashtextextended('dense',1782495106)) AS locked
      \\gset
      {warm}
      \\echo PLAN_BEGIN
      {EXPLAIN} {execute}
      \\echo PLAN_END
      SELECT json_build_array(generic_plans, custom_plans)
      FROM pg_prepared_statements WHERE name='grant_cte';
      ROLLBACK;
    """)
    _, rest = output.split("PLAN_BEGIN\n", 1)
    encoded, counters = rest.split("\nPLAN_END\n", 1)
    generic, custom = json.loads(counters)
    assert generic + custom == (6 if mode == "auto" else 1)
    assert mode != "force_generic_plan" or generic == 1
    assert mode != "force_custom_plan" or custom == 1
    plan, = json.loads(encoded)
    return dict(plan=plan, generic=generic, custom=custom)


def nodes(node):
    yield node
    for child in node.get("Plans", []):
        yield from nodes(child)


def visits(node):
    return node["Actual Loops"] * sum(node.get(k, 0) for k in
                                    ("Actual Rows", "Rows Removed by Filter", "Rows Removed by Index Recheck"))


def plan_metrics(execution, expected):
    plan = execution["plan"]["Plan"]
    update, = [n for n in nodes(plan) if n.get("Subplan Name") == "CTE granted"]
    assert update["Node Type"] == "ModifyTable" and update["Operation"] == "Update"
    assert update["Actual Rows"] == plan["Actual Rows"] == expected
    picked, = [n for n in nodes(plan) if n.get("Subplan Name") == "CTE picked"]
    assert picked["Actual Rows"] == expected
    assert any(n["Node Type"] == "LockRows" for n in nodes(picked))
    fences = [trigger for trigger in execution["plan"].get("Triggers", [])
              if trigger["Trigger Name"] == "mesh_jobs_fence_claim"]
    assert sum(trigger["Calls"] for trigger in fences) == expected
    scans = [n for n in nodes(update) if n.get("Relation Name") == "jobs" and n is not update]
    assert scans
    keys = ("Node Type", "Index Name", "Index Cond", "Recheck Cond", "Filter",
            "Actual Rows", "Actual Loops", "Rows Removed by Filter", "Shared Hit Blocks", "Shared Read Blocks")
    return dict(
        update_visits=sum(map(visits, scans)),
        update_scans=[{key: n[key] for key in keys if key in n} for n in scans],
        picked_visits=sum(visits(n) for n in nodes(picked) if n.get("Relation Name") == "jobs"),
        grant_buffers=plan.get("Shared Hit Blocks", 0) + plan.get("Shared Read Blocks", 0),
        wal_records=plan.get("WAL Records", 0),
        actual_rows=expected, generic=execution["generic"], custom=execution["custom"],
    )


def assess(evidence):
    assert evidence["semantics"]["stock"] == evidence["semantics"]["candidate"]
    scores = {}
    for shape, (ready, running, remaining) in SHAPES.items():
        expected = min(ready, remaining)
        scores[shape] = {}
        for mode in MODES:
            pair = {kind: plan_metrics(evidence["plans"][shape][mode][kind], expected)
                    for kind in ("stock", "candidate")}
            scores[shape][mode] = pair
    dense = [scores[shape]["auto"] for shape in ("head16", "mid16", "mid8")]
    stock_visits = sum(pair["stock"]["update_visits"] for pair in dense)
    candidate_visits = sum(pair["candidate"]["update_visits"] for pair in dense)
    stock_buffers = sum(pair["stock"]["grant_buffers"] for pair in dense)
    candidate_buffers = sum(pair["candidate"]["grant_buffers"] for pair in dense)
    checks = dict(
        bounded=all(pair["candidate"]["update_visits"] <= 2 * pair["candidate"]["actual_rows"]
                    for shape in scores.values() for pair in shape.values()),
        update_visits=stock_visits > 0 and candidate_visits <= stock_visits / 4,
        grant_buffers=stock_buffers > 0 and candidate_buffers <= 0.75 * stock_buffers,
    )
    return dict(passed=all(checks.values()), checks=checks, plans=scores)


def qualify(cluster):
    assert cluster.expected_version == "17.11" and cluster.data.is_dir()
    cluster.sql("postgres", f"CREATE DATABASE {DB} TEMPLATE mesh")
    cluster.sql("postgres", f"REVOKE CONNECT ON DATABASE {DB} FROM PUBLIC; "
                f"GRANT CONNECT ON DATABASE {DB} TO parity_subject")
    setup, behaviors, plans = {}, {}, {}
    stock_definition = definition(cluster, DB)
    data = dict(overlay=setup, semantics=behaviors, plans=plans,
                statement_hashes={kind: digest(grant_sql(ddl)) for kind, ddl in
                                  (("stock", STOCK), ("candidate", CANDIDATE))})
    try:
        for kind, ddl in (("stock", stock_definition), ("candidate", stock_definition.replace(JOIN, BOUND))):
            cluster.sql(DB, ddl)
            installed = definition(cluster, DB)
            setup[kind] = dict(body_sha256=digest(body(installed)), metadata=metadata(cluster, DB))
            assert setup[kind]["body_sha256"] == digest(body(STOCK if kind == "stock" else CANDIDATE))
            behaviors[kind] = semantics(cluster)
        # Each stock/candidate pair sees the same committed backlog. The
        # prepared statement runs under worker TLS with the real row trigger.
        for shape, (ready, running, remaining) in SHAPES.items():
            plans[shape] = {}
            for mode in MODES:
                pair = plans[shape][mode] = {}
                for kind, ddl in (("stock", STOCK), ("candidate", CANDIDATE)):
                    seed(cluster, ready, running)
                    pair[kind] = capture(cluster, ddl, remaining, mode)
                    counts = json.loads(psql(cluster, """SELECT json_build_array(
                        count(*) FILTER (WHERE state='available'),
                        count(*) FILTER (WHERE state='executing'))
                        FROM mesh_jobs.jobs WHERE namespace='dense';"""))
                    assert counts == [ready, running], "EXPLAIN transaction leaked a grant"
        assert setup["stock"]["metadata"] == setup["candidate"]["metadata"]
        data["gate"] = assess(data)
    finally:
        (cluster.evidence / "grant-plans.json").write_text(json.dumps(data, indent=2, sort_keys=True) + "\n")
    assert data["gate"]["passed"], data["gate"]["checks"]
