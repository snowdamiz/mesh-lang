"""Apply the selected overlay and verify worker semantics in a disposable database."""

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
PICK = "SELECT p.id, p.priority,"
KEY = "SELECT p.namespace AS grant_namespace, p.id, p.priority,"
JOIN = "WHERE b.fits AND j.namespace = ns AND j.id = s.id"
LOGICAL = "WHERE b.fits AND j.namespace = s.grant_namespace AND j.id = s.id"
WORKERS = """'[{"worker":"parity","version":1}]'::jsonb"""


def prototype():
    assert STOCK.count(PICK) == STOCK.count(JOIN) == 1
    return STOCK.replace(PICK, KEY).replace(JOIN, LOGICAL)


def validate(ddl):
    # The final cold-qualified candidate may tighten the target predicate.
    # Keep the function envelope, ordering, locks, fences, and all other SQL exact.
    prefix, suffix = prototype().split(LOGICAL, 1)
    assert ddl.startswith(prefix + LOGICAL) and ddl.endswith(suffix)
    extra = ddl[len(prefix + LOGICAL):-len(suffix)]
    assert not re.search(r";|--|/\*|\b(?:ctid|tid|any)\b", extra, re.I)
    assert not extra or re.match(r"\s+AND\s+", extra, re.I)


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


def apply(cluster, database, ddl=None):
    assert database in {"mesh", DB} and cluster.data.is_dir()
    before = definition(cluster, database)
    assert body(before) == body(STOCK), "migrated grant source differs from the pinned source"
    acl = metadata(cluster, database)
    assert acl["owner"] == "parity_migrator" and not acl["security_definer"]
    assert acl["worker_execute"] and not acl["producer_execute"] and not acl["app_execute"]
    assert acl["configuration"] == ["search_path=pg_catalog, pg_temp", "lock_timeout=100ms"]
    expected = STOCK if ddl is None else ddl
    if ddl is not None:
        validate(ddl)
        cluster.sql(database, before.replace(body(before), body(ddl), 1))
    after = definition(cluster, database)
    assert body(after) == body(expected) and metadata(cluster, database) == acl
    return dict(body_sha256=digest(body(after)), baseline_body_sha256=digest(body(before)), metadata=acl)


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


def seed(cluster):
    # Deterministic jobs with dense-sized payload and the same partial indexes.
    # Fixture states are seeded outside the EXPLAIN transaction.
    foreign = """
      INSERT INTO mesh_jobs.queues(namespace,name,concurrency) VALUES ('shadow','default',64);
      INSERT INTO mesh_jobs.jobs(namespace,id,worker,version,queue,args,metadata,tags,state,
          priority,max_attempts,timeout_ms,backoff_base_ms,backoff_max_ms,inserted_at_ms,scheduled_at_ms)
      VALUES ('shadow',md5('1')::uuid,'parity',1,'default','{}','{}','[]','available',0,3,1000,0,0,0,0);
    """
    psql(cluster, f"""
        TRUNCATE mesh_jobs.jobs, mesh_jobs.batch_context_blocks, mesh_jobs.queues RESTART IDENTITY;
        INSERT INTO mesh_jobs.queues(namespace,name,concurrency)
        VALUES ('dense','default',64), ('dense','other',64);
        INSERT INTO mesh_jobs.jobs(namespace,id,worker,version,queue,args,metadata,tags,state,
            priority,attempt,max_attempts,timeout_ms,backoff_base_ms,backoff_max_ms,
            inserted_at_ms,scheduled_at_ms,lease_token,lease_expires_at_ms,finished_at_ms)
        SELECT 'dense', md5(i::text)::uuid, 'parity', 1, 'default', jsonb_build_object('key', i),
            '{{}}', '[]', 'available', (ARRAY[1,0,0,0,0,0])[i], 0, 3, 1000, 0, 0, 0, 0,
            NULL, NULL, NULL
        FROM generate_series(1,6) i ORDER BY i;
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
    seed(cluster)
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



def qualify(cluster, ddl):
    cluster.sql("postgres", f"CREATE DATABASE {DB} TEMPLATE mesh")
    cluster.sql("postgres", f"REVOKE CONNECT ON DATABASE {DB} FROM PUBLIC; "
                f"GRANT CONNECT ON DATABASE {DB} TO parity_subject")
    receipt = apply(cluster, DB, ddl)
    return dict(overlay=receipt, checks=semantics(cluster))
