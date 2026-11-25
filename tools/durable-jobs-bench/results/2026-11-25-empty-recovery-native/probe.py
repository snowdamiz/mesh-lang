"""Recovery plans and transaction witnesses in a disposable clone of the native fixture."""
from contextlib import contextmanager
import json
import selectors
import subprocess

from contract import DEPTHS, body, digest, update

NS = "recovery_pilot"
CALL = """SELECT mesh_jobs.recover('recovery_pilot',16,'["default"]'::jsonb)"""
GUARD = "SELECT mesh_jobs.enqueue_batch_layout_v1()"
FUNCTION = "'mesh_jobs.recover(text,integer,jsonb)'::regprocedure"
UPDATE_FILTER = "query ~* '^\\s*WITH due AS ' AND query ILIKE '%UPDATE mesh_jobs.jobs j%'"
COUNTERS = """
queryid, query, calls, rows, total_exec_time, shared_blks_hit, shared_blks_read,
shared_blks_dirtied, shared_blks_written, wal_records, wal_fpi, wal_bytes
"""
SESSION = """
SET statement_timeout='10s';
SET lock_timeout='2s';
SET pg_stat_statements.track='all';
SET ROLE parity_subject;
"""
COLUMNS = """namespace, id, queue, worker, version, args, metadata, tags, state, priority,
    attempt, max_attempts, timeout_ms, backoff_base_ms, backoff_max_ms, inserted_at_ms,
    scheduled_at_ms, finished_at_ms"""
QUEUES = """
INSERT INTO mesh_jobs.queues(namespace,name)
VALUES ('recovery_pilot','default'), ('recovery_pilot','other'), ('other_namespace','default')
ON CONFLICT DO NOTHING;
"""


def history(depth):
    return f"""
BEGIN;
{QUEUES}
INSERT INTO mesh_jobs.jobs({COLUMNS})
SELECT '{NS}',md5('retained-' || n)::uuid,'default','history.v1',1,
       jsonb_build_object('payload', left(payload,1009)), '{{}}','[]','completed',0,
       1,2,60000,1000,1000,0,0,1
FROM generate_series(1,{depth}) n
CROSS JOIN LATERAL (
    SELECT string_agg(md5(n::text || ':' || p), '' ORDER BY p) AS payload
    FROM generate_series(1,32) p
) args;
COMMIT;
VACUUM (ANALYZE) mesh_jobs.jobs;
"""


def seed(count, mixed=False):
    extra = "" if not mixed else """
        UNION ALL SELECT 'recovery_pilot','other',1001,'scheduled',1
        UNION ALL SELECT 'other_namespace','default',1002,'scheduled',1
        UNION ALL SELECT 'recovery_pilot','default',1003,'scheduled',4102444800000
        UNION ALL SELECT 'recovery_pilot','default',1004,'available',1
    """
    return f"""
INSERT INTO mesh_jobs.jobs({COLUMNS})
SELECT n,md5('due-' || k)::uuid,q,'due.v1',1,jsonb_build_object('n',k),'{{}}','[]',
       state,0,0,2,60000,1000,1000,0,timest,NULL
FROM (
    SELECT '{NS}' AS n,'default' AS q,k,
           CASE WHEN k % 2 = 0 THEN 'retryable' ELSE 'scheduled' END AS state,k::bigint AS timest
    FROM generate_series(1,{count}) k
    {extra}
) due;
ANALYZE mesh_jobs.jobs;
"""


def proc_identity(sql, declaration):
    actual = sql(f"""
SELECT json_build_array(oid,proowner::regrole::text,proacl,prosecdef,provolatile,
    proconfig,prosrc,pg_get_function_arguments(oid),pg_get_function_result(oid))
FROM pg_proc WHERE oid={FUNCTION}
""")
    identity = json.loads(actual)
    assert identity[6] == body(declaration) and identity[4] == "v", identity
    identity[6] = digest(identity[6].encode())
    return identity


def signature(sql, depth):
    value = json.loads(sql(f"""
SELECT json_build_object(
    'rows', count(*),
    'distinct_ids', count(DISTINCT id),
    'json_text_min', min(octet_length(args::text)),
    'json_text_max', max(octet_length(args::text)),
    'args_storage_min', min(pg_column_size(args)),
    'args_storage_max', max(pg_column_size(args)),
    'committed', coalesce(bool_and(pg_xact_commit_timestamp(xmin) IS NOT NULL),true),
    'content_digest', md5(coalesce(string_agg(md5(args::text), '' ORDER BY id), '')),
    'heap_bytes', pg_relation_size('mesh_jobs.jobs'),
    'indexes_bytes', pg_indexes_size('mesh_jobs.jobs'),
    'total_bytes', pg_total_relation_size('mesh_jobs.jobs'))
FROM mesh_jobs.jobs WHERE namespace='{NS}' AND state='completed'
"""))
    assert value["rows"] == value["distinct_ids"] == depth and value["committed"], value
    assert value["json_text_min"] == value["json_text_max"] == (1024 if depth else None), value
    return value


def active(sql):
    return json.loads(sql("""
SELECT coalesce(jsonb_object_agg(args->>'n',state),'{}'::jsonb)
FROM mesh_jobs.jobs WHERE worker='due.v1'
"""))


@contextmanager
def worker(command, environment):
    with subprocess.Popen(command, env=environment, text=True,
                          stdin=subprocess.PIPE, stdout=subprocess.PIPE) as process:
        def exchange(statement, expected):
            process.stdin.write(statement + "\n")
            process.stdin.flush()
            with selectors.DefaultSelector() as selector:
                selector.register(process.stdout, selectors.EVENT_READ)
                assert selector.select(15), "worker did not produce a SQL receipt"
            assert process.stdout.readline().strip() == expected

        try:
            yield exchange
        finally:
            process.stdin.close()
            try:
                assert process.wait(timeout=10) == 0
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
                raise


def semantics(sql, command, environment):
    """Commit two independent recoveries while the first still holds its locks."""
    sql(seed(16))
    with worker(command, environment) as first:
        first(SESSION + "BEGIN; " + CALL + ";", "0")
        assert sql(SESSION + CALL) == "0"
        assert set(active(sql).values()) == {"scheduled", "retryable"}
        first("ROLLBACK; SELECT 'rolled back';", "rolled back")
    assert set(active(sql).values()) == {"scheduled", "retryable"}
    sql("DELETE FROM mesh_jobs.jobs WHERE worker='due.v1';" + seed(32))
    with worker(command, environment) as first:
        first(SESSION + "BEGIN; " + CALL + ";", "0")
        assert sql(SESSION + CALL) == "0"
        visible = active(sql)
        assert {int(k) for k,v in visible.items() if v == "available"} == set(range(17,33))
        first("COMMIT; SELECT 'committed';", "committed")
    assert active(sql) == {str(n): "available" for n in range(1,33)}
    commits = int(sql("""
SELECT count(DISTINCT xmin::text) FROM mesh_jobs.jobs
WHERE worker='due.v1' AND pg_xact_commit_timestamp(xmin) IS NOT NULL
"""))
    assert commits == 2
    sql("""
UPDATE mesh_jobs.jobs SET state='executing', attempt=max_attempts,
    lease_token=md5('recovery-pilot-token')::uuid, lease_expires_at_ms=1
WHERE worker='due.v1' AND args->>'n'='1';
""")
    assert sql(SESSION + CALL) == "1"
    assert sql("""
SELECT state='discarded' AND lease_token IS NULL AND lease_expires_at_ms IS NULL
    AND errors->-1->>'reason'='lease expired'
FROM mesh_jobs.jobs WHERE worker='due.v1' AND args->>'n'='1'
""") == "t"
    # Keep the mutation and transition-table trigger local to this transaction.
    receipt = sql(f"""
BEGIN;
CREATE TABLE public.recovery_trigger_seen(n bigint);
CREATE FUNCTION public.recovery_trigger() RETURNS trigger LANGUAGE plpgsql AS $observer$
BEGIN INSERT INTO public.recovery_trigger_seen SELECT count(*) FROM promoted; RETURN NULL; END
$observer$;
CREATE TRIGGER recovery_trigger AFTER UPDATE ON mesh_jobs.jobs
REFERENCING NEW TABLE AS promoted FOR EACH STATEMENT EXECUTE FUNCTION public.recovery_trigger();
GRANT INSERT ON public.recovery_trigger_seen TO parity_subject;
SET ROLE parity_subject;
{CALL};
RESET ROLE;
SELECT json_agg(n) FROM public.recovery_trigger_seen;
ROLLBACK;
""")
    assert receipt.splitlines() == ["0", "[0]"], receipt
    assert sql(GUARD) == "t"
    return dict(locked_empty=True, split_two_workers=True, two_committed_update_xids=commits,
                expired_lease_return=1, statement_trigger_transition_rows=0, guard_enabled=True)


def plan(sql, statement, mode):
    value, = json.loads(sql(f"""
BEGIN;
SET LOCAL plan_cache_mode='{mode}';
SET LOCAL ROLE parity_subject;
PREPARE recovery(text,integer,bigint,jsonb) AS {statement};
EXPLAIN (ANALYZE, BUFFERS, WAL, TIMING OFF, SUMMARY OFF, FORMAT JSON)
EXECUTE recovery('{NS}',16,1700000000000,'["default"]');
ROLLBACK;
"""))
    return value


def target_scans(node):
    if node["Node Type"].endswith("Scan") and node.get("Relation Name") == "jobs" and node.get("Alias") == "j":
        yield node
    for child in node.get("Plans", []):
        yield from target_scans(child)


def stats_query(db):
    return f"""
SELECT json_build_object(
 'dealloc',(SELECT dealloc FROM pg_stat_statements_info),
 'reset',(SELECT stats_reset FROM pg_stat_statements_info),
 'statements',(SELECT coalesce(json_agg(s ORDER BY queryid),'[]') FROM
    (SELECT {COUNTERS}
     FROM pg_stat_statements
     WHERE dbid=(SELECT oid FROM pg_database WHERE datname='{db}')
       AND userid='parity_subject'::regrole AND NOT toplevel AND {UPDATE_FILTER}) s))
"""


def measured(sql, admin, db, scenario, due_count):
    # Reset only this disposable database's entries. These executions are
    # separate from EXPLAIN and rollbacks give every due call the same prefix.
    admin(f"SELECT pg_stat_statements_reset(0,(SELECT oid FROM pg_database WHERE datname='{db}'),0)")
    repeats = 128 if scenario == "empty" else 12
    result = sql(SESSION + (f"BEGIN; {CALL}; ROLLBACK;\n" * repeats))
    assert result.splitlines() == ["0"] * repeats
    snapshot = json.loads(admin(stats_query(db)))
    row, = snapshot["statements"]
    assert not snapshot["dealloc"]
    assert (row["calls"], row["rows"]) == (repeats, repeats * due_count), snapshot
    return snapshot


def capture_case(sql, declaration, variant, n, record, write):
    for mode in ("force_custom_plan", "force_generic_plan"):
        captured = plan(sql, update(declaration), mode)
        record["plans"][mode] = captured
        write()
        scans = list(target_scans(captured["Plan"]))
        assert scans, captured
        if variant == "candidate" and not n:
            assert all(s["Actual Loops"] == 0 for s in scans), captured
        if n:
            assert any(s["Actual Loops"] > 0 for s in scans), captured


def run(cluster, definitions, write):
    db = "recovery_pilot"
    admin = lambda statement: cluster.sql("postgres", statement)
    admin(f"CREATE DATABASE {db} TEMPLATE mesh")
    command, environment = cluster.psql(db)
    def sql(statement):
        # stdin gives each semicolon-delimited command its own transaction
        # unless the script explicitly opens one.
        result = subprocess.run(command, input=statement, text=True,
                                env=environment, capture_output=True, timeout=45)
        if result.returncode:
            raise RuntimeError(result.stderr.strip())
        return result.stdout.strip()

    probe = {}
    try:
        stock = proc_identity(sql, definitions["stock"])
        for variant, depths in (("stock", DEPTHS), ("candidate", DEPTHS[::-1])):
            sql("SET ROLE parity_migrator;\n" + definitions[variant])
            identity = proc_identity(sql, definitions[variant])
            assert identity[:6] == stock[:6] and identity[7:] == stock[7:], identity
            record = probe[variant] = dict(identity=identity, fixtures={})
            assert sql(GUARD) == "t"
            reset = "TRUNCATE mesh_jobs.batch_context_blocks, mesh_jobs.jobs;\n"
            sql(reset + history(0))
            record["concurrency"] = semantics(sql, command, environment)
            for depth in depths:
                sql(reset + history(depth))
                receipt = record["fixtures"][str(depth)] = dict(history=signature(sql, depth), cases={})
                for scenario, n in (("empty", 0), ("full", 16), ("mixed", 5)):
                    if n:
                        sql("DELETE FROM mesh_jobs.jobs WHERE worker='due.v1';\n" + seed(n, scenario == "mixed"))
                    before = active(sql)
                    cases = receipt["cases"]
                    cases[scenario] = dict(plans={}, before=before)
                    capture_case(sql, definitions[variant], variant, n, cases[scenario], lambda: write(probe))
                    cases[scenario]["stats"] = measured(sql, admin, db, scenario, n)
                    assert active(sql) == before
                    # Commit the due call as well and prove excluded rows remain.
                    assert sql(SESSION + CALL) == "0"
                    after = active(sql)
                    assert after == {k: ("available" if 1 <= int(k) <= n else v) for k,v in before.items()}
                    cases[scenario]["after"] = after
                    write(probe)
                sql("DELETE FROM mesh_jobs.jobs WHERE worker='due.v1';" + seed(16))
                with worker(command, environment) as first:
                    first(SESSION + "BEGIN; " + CALL + ";", "0")
                    before = active(sql)
                    held = receipt["cases"]["held"] = dict(plans={}, before=before)
                    capture_case(sql, definitions[variant], variant, 0, held, lambda: write(probe))
                    held["stats"] = measured(sql, admin, db, "held", 0)
                    assert active(sql) == before
                    first("ROLLBACK; SELECT 'rolled back';", "rolled back")
                held["after"] = active(sql)
                assert held["after"] == held["before"]
                write(probe)
                assert signature(sql, depth)["content_digest"] == receipt["history"]["content_digest"]
        for depth in DEPTHS:
            a,b = (probe[v]["fixtures"][str(depth)]["history"] for v in ("stock", "candidate"))
            assert all(a[k] == b[k] for k in a if k != "total_bytes"), (a, b)
        write(probe)
    finally:
        admin(f"DROP DATABASE {db}")
    return probe
