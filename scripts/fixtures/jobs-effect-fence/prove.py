"""Run through scripts/verify-postgres-jobs.sh --effect-fence."""

import json
import os
import re
import secrets
import select
import subprocess
import sys
import time
from contextlib import contextmanager
from pathlib import Path
from urllib.parse import unquote, urlsplit
from uuid import UUID


FIXTURE = Path(__file__).resolve().parent
ROOT = FIXTURE.parents[2]
SCHEMA = "compiler/mesh-rt/src/jobs"
BASE = "dbc55bb1018e0d59a1e68e622c11cee80c25a2bb"
LEGACY = "7c1620563614a04a573aef806a6d019a2af4dfe9"
NAMESPACE = "effect-fence-proof"
MIGRATOR, APP_OWNER, CALLER = "fence_migrator", "fence_app_owner", "fence_caller"
ROLE_GRANTS = {kind: f"fence_{kind}" for kind in ("producer", "worker", "operator", "inspector")}
OPERATOR = ROLE_GRANTS["operator"]
HELPER = "mesh_jobs_effects.fence_v1(text,uuid,bigint,integer,uuid)"
LEGACY_CLAIM = "mesh_jobs.claim(text,text,jsonb,bigint)"
WORKERS = '[{"worker":"proof","version":1}]'
SPEC = json.dumps(dict(
    worker="proof", version=1, queue="default", args={}, metadata={}, tags=[], priority=0,
    max_attempts=3, timeout_ms=60000, backoff_base_ms=0, backoff_max_ms=0,
    suspended=False, unique=None,
))


def expect(label, actual, expected):
    if actual != expected:
        raise AssertionError(f"{label}: expected {expected!r}, got {actual!r}")


def revision_sql(revision, *files):
    return "\n".join(subprocess.check_output(
        ["git", "show", f"{revision}:{SCHEMA}/{file}"], cwd=ROOT, text=True, timeout=5,
    ) for file in files)


class Proof:
    def __init__(self):
        self.url = urlsplit(os.environ["MESH_TEST_DATABASE_URL"])
        self.passwords = {role: secrets.token_hex(16) for role in (CALLER, OPERATOR)}

    def psql(self, login=None, *extra, application_name="jobs-effect-fence-proof"):
        user = login or self.url.username
        password = self.passwords[login] if login else unquote(self.url.password)
        url = self.url._replace(netloc=f"{user}@{self.url.hostname}:{self.url.port}")
        args = ["psql", "-XqAtw", "-v", "ON_ERROR_STOP=1", "-v", "VERBOSITY=verbose",
                url.geturl(), *extra]
        env = dict(os.environ, PGPASSWORD=password, PGCONNECT_TIMEOUT="4", PGAPPNAME=application_name,
                   PGOPTIONS="-c client_min_messages=warning -c statement_timeout=6s "
                             "-c idle_in_transaction_session_timeout=15s")
        return args, env

    def sql(self, query, *, role=None, login=None, state=None, extra=()):
        args, env = self.psql(login, *extra)
        if role:
            query = f"SET ROLE {role};\n{query}"
        try:
            result = subprocess.run(args, env=env, input=query + "\n", capture_output=True,
                                    text=True, timeout=10)
        except subprocess.TimeoutExpired:
            raise RuntimeError("psql exceeded the fence proof timeout") from None
        if state:
            found = re.search(r"^ERROR:\s+([0-9A-Z]{5}):", result.stderr, re.MULTILINE)
            expect("SQLSTATE", found.group(1) if found and result.returncode else None, state)
        elif result.returncode:
            raise RuntimeError(result.stderr.strip())
        return result.stdout.strip()

    @contextmanager
    def session(self, login, *args, application_name="jobs-effect-fence-proof"):
        command, env = self.psql(login, *args, application_name=application_name)
        with subprocess.Popen(command, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                              stderr=subprocess.PIPE, text=True) as process:
            try:
                yield process
            finally:
                if process.poll() is None:
                    process.kill()

    def migrate(self, revision, *files):
        return self.sql("BEGIN; SELECT pg_catalog.pg_advisory_xact_lock(1782495102, 1);\n"
                        + revision_sql(revision, *files) + "\nCOMMIT;", role=MIGRATOR)

    def enqueue(self):
        return UUID(self.sql(f"SELECT mesh_jobs.enqueue('{NAMESPACE}', '{SPEC}', '', '') ->> 'id';",
                             role=ROLE_GRANTS["producer"]))

    def claim(self, job):
        claim = self.sql(f"SELECT id, token FROM mesh_jobs.claim('{NAMESPACE}', 'default', "
                         f"'{WORKERS}', 60000);", role=ROLE_GRANTS["worker"])
        claimed, token = map(UUID, claim.split("|"))
        expect("claim ID", claimed, job)
        return token

    def expire(self, job):
        expect("expire exact lease", self.sql(f"""
            UPDATE mesh_jobs.jobs SET lease_expires_at_ms = mesh_jobs.now_ms()
            WHERE namespace = '{NAMESPACE}' AND id = '{job}' AND state = 'executing' RETURNING id;
        """, role=MIGRATOR), str(job))

    def recover(self):
        expect("recover exact lease", self.sql(f"SELECT mesh_jobs.recover('{NAMESPACE}', 1, NULL);",
                                               role=ROLE_GRANTS["worker"]), "1")

    def observe(self, job):
        return json.loads(self.sql(f"""
            SELECT coalesce(jsonb_agg(to_jsonb(e) ORDER BY e.stage), '[]')
            FROM fence_demo.effects AS e WHERE e.job_id = '{job}';
        """))

    def job(self, job):
        return self.sql(f"SELECT to_jsonb(j) FROM mesh_jobs.jobs AS j "
                        f"WHERE j.namespace = '{NAMESPACE}' AND j.id = '{job}';")


def write(stage, job, token, *, run=1, attempt=1):
    return f"SELECT fence_demo.write_{stage}('{job}', {run}, {attempt}, '{token}');"


def marker(session, query, message):
    session.stdin.write(f"{query}\n\\warn {message}\n")
    session.stdin.flush()
    if not select.select([session.stderr], [], [], 8)[0]:
        raise AssertionError(f"psql never reached {message}")
    return session.stderr.readline().strip()


def setup(proof):
    roles = [MIGRATOR, APP_OWNER, CALLER, *ROLE_GRANTS.values()]
    for role in roles:
        password = proof.passwords.get(role)
        options = f"LOGIN PASSWORD '{password}'" if password else "NOLOGIN"
        proof.sql(f"CREATE ROLE {role} {options};")
    proof.sql(f"GRANT CREATE ON DATABASE mesh_jobs_proof TO {MIGRATOR};")
    proof.migrate(BASE, "schema.sql", "drain_schema.sql")
    proof.migrate(BASE, "cron_schema.sql")

    fence = (ROOT / SCHEMA / "effect_fence_schema.sql").read_text()
    proof.sql(fence, state="P0001")  # The connection superuser does not own Jobs objects.
    proof.sql("ALTER DEFAULT PRIVILEGES GRANT EXECUTE ON FUNCTIONS TO fence_worker;", role=MIGRATOR)
    proof.sql(fence, role=MIGRATOR, state="P0001")
    expect("failed install left no effect schema", proof.sql(
        "SELECT to_regnamespace('mesh_jobs_effects') IS NULL;"), "t")
    proof.sql("ALTER DEFAULT PRIVILEGES REVOKE EXECUTE ON FUNCTIONS FROM fence_worker;", role=MIGRATOR)
    proof.sql(fence, role=MIGRATOR)
    proof.sql((FIXTURE / "application.sql").read_text())

    # A stock legacy function still has PUBLIC EXECUTE. The separate namespace
    # must keep that ACL from becoming effective for the application owner.
    expect("legacy PUBLIC execute still exists", proof.sql(
        f"SELECT has_function_privilege('{APP_OWNER}', '{LEGACY_CLAIM}', 'EXECUTE');"), "t")
    direct_claim = f"SELECT mesh_jobs.claim('{NAMESPACE}', 'default', '{WORKERS}', 60000);"
    proof.sql(direct_claim, role=APP_OWNER, state="42501")
    proof.sql(direct_claim, login=CALLER, state="42501")

    options = tuple(arg for kind, role in ROLE_GRANTS.items() for arg in ("-v", f"{kind}={role}"))
    proof.sql((ROOT / "scripts/fixtures/jobs-role-grants.sql").read_text(), role=MIGRATOR, extra=options)


def check_grants(proof, job, token):
    for statement in (
        f"SELECT mesh_jobs_effects.fence_v1('{NAMESPACE}', '{job}', 1, 1, '{token}');",
        "SELECT args, lease_token FROM mesh_jobs.jobs;",
        "UPDATE mesh_jobs.jobs SET priority = 1;",
        "SELECT * FROM fence_demo.effects;",
        f"INSERT INTO fence_demo.effects(job_id, job_run, stage) VALUES ('{job}', 1, 1);",
        "SET ROLE fence_app_owner;",
    ):
        proof.sql(statement, login=CALLER, state="42501")
    proof.sql("SELECT args, lease_token FROM mesh_jobs.jobs;", role=APP_OWNER, state="42501")
    proof.sql("CREATE TABLE fence_demo.ungranted (id integer);", role=APP_OWNER, state="42501")
    proof.sql("UPDATE fence_demo.effects SET stage = 1;", role=APP_OWNER, state="42501")
    for role in ROLE_GRANTS.values():
        expect(f"no effect grants for {role}", proof.sql(f"""
            SELECT has_schema_privilege('{role}', 'mesh_jobs_effects', 'USAGE'),
                   has_function_privilege('{role}', '{HELPER}', 'EXECUTE'),
                   has_schema_privilege('{role}', 'fence_demo', 'USAGE'),
                   has_table_privilege('{role}', 'fence_demo.effects', 'SELECT,INSERT,UPDATE,DELETE'),
                   has_function_privilege('{role}', 'fence_demo.write_first(uuid,bigint,integer,uuid)', 'EXECUTE'),
                   has_function_privilege('{role}', 'fence_demo.write_second(uuid,bigint,integer,uuid)', 'EXECUTE');
        """), "f|f|f|f|f|f")


def check_owner_replay(proof, job, token):
    snapshot = f"""
        SELECT p.oid, p.proowner, p.proacl, p.prosrc, p.prosecdef, p.proconfig
        FROM pg_proc AS p WHERE p.oid IN ('{HELPER}'::regprocedure, '{LEGACY_CLAIM}'::regprocedure)
        ORDER BY p.oid;
    """
    before = proof.sql(snapshot)
    job_before = proof.job(job)
    for revision, queue_files in ((LEGACY, ("schema.sql",)), (BASE, ("schema.sql", "drain_schema.sql"))):
        proof.migrate(revision, *queue_files)
        proof.migrate(revision, "cron_schema.sql")
        expect(f"function ACLs and definitions after owner replay {revision[:8]}", proof.sql(snapshot), before)
        expect("live attempt unchanged by replay", proof.job(job) == job_before, True)
        check_grants(proof, job, token)
    expect("legacy worker can still renew", proof.sql(f"""
        SELECT mesh_jobs.renew('{NAMESPACE}', '{job}', '{token}', 60000);
    """, role=ROLE_GRANTS["worker"]), "active")


def check_writes(proof, job, old_token):
    first_attempt = write("first", job, old_token)
    proof.expire(job)
    proof.sql(first_attempt, login=CALLER, state="55000")
    proof.recover()
    proof.sql(first_attempt, login=CALLER, state="55000")  # Recovered, before the next claim.
    token = proof.claim(job)
    expect("replacement token changed", token != old_token, True)
    before = proof.job(job)
    for query in (
        write("first", job, old_token, attempt=2),
        write("first", job, token),
        write("first", job, token, run=2, attempt=2),
        write("first", UUID(int=0), token, attempt=2),
        f"SELECT fence_demo.write_first('{job}', 1, 2, NULL);",
        f"SET mesh_jobs_effects.token = '{token}';\n" + write("first", job, old_token, attempt=2),
    ):
        proof.sql(query, login=CALLER, state="55000")
    proof.sql(f"SELECT mesh_jobs_effects.fence_v1('other-namespace', '{job}', 1, 2, '{token}');",
              role=APP_OWNER, state="55000")
    first = write("first", job, token, attempt=2)
    second = write("second", job, token, attempt=2)
    for level in ("REPEATABLE READ", "SERIALIZABLE"):
        proof.sql(f"BEGIN ISOLATION LEVEL {level};\n{first}", login=CALLER, state="25000")
    expect("denied statements leave no effects", proof.observe(job), [])

    with proof.session(CALLER) as caller:
        expect("first autocommit reply", marker(caller, first, "first_committed"), "first_committed")
        first_receipt, = proof.observe(job)
        expect("first stage committed independently", first_receipt["stage"], 1)
        expect("second autocommit reply", marker(caller, second, "second_committed"), "second_committed")
        after = proof.observe(job)
        expect("first receipt preserved", after[0], first_receipt)
        expect("effect keys", [(row["job_id"], row["job_run"], row["stage"]) for row in after],
               [(str(job), 1, 1), (str(job), 1, 2)])
        expect("distinct commits on the same backend",
               (after[0]["xid"] != after[1]["xid"], after[0]["backend"] == after[1]["backend"]), (True, True))
    expect("successful and denied calls leave the queue row unchanged", proof.job(job) == before, True)

    proof.expire(job)
    proof.recover()
    replacement = proof.claim(job)
    proof.sql(first, login=CALLER, state="55000")
    for stage in ("first", "second"):
        proof.sql(write(stage, job, replacement, attempt=3), login=CALLER)
    expect("replacement replays both durable effect keys", proof.observe(job), after)
    expect("legacy worker can still settle", proof.sql(f"""
        SELECT mesh_jobs.settle('{NAMESPACE}', '{job}', '{replacement}', '{{"action":"complete"}}', false);
    """, role=ROLE_GRANTS["worker"]), "t")


def check_cancellation(proof):
    job = proof.enqueue()
    token = proof.claim(job)
    first, second = (write(stage, job, token) for stage in ("first", "second"))
    cancel = f"""SELECT (mesh_jobs.job_control('{NAMESPACE}', '{job}', 'cancel', NULL, NULL)
                        ->> 'cancel_requested_at_ms') IS NOT NULL;"""
    activity_name = "jobs-effect-fence-cancel"
    with proof.session(CALLER) as writer:
        ready = marker(writer, f"BEGIN;\n{first}\nSELECT pg_backend_pid() AS pid \\gset", "authorized :pid")
        label, pid = ready.split()
        expect("writer reached its barrier", label, "authorized")
        pid = int(pid)
        proof.sql(second, login=CALLER, state="55P03")
        with proof.session(OPERATOR, "-c", cancel, application_name=activity_name) as canceller:
            blocked = f"""
                SELECT EXISTS (SELECT FROM pg_stat_activity WHERE application_name = '{activity_name}'
                    AND usename = '{OPERATOR}' AND {pid} = ANY(pg_blocking_pids(pg_stat_activity.pid)));
            """
            deadline = time.monotonic() + 4
            while proof.sql(blocked) != "t":
                if canceller.poll() is not None or time.monotonic() >= deadline:
                    raise AssertionError("operator did not wait for the authorized writer's row lock")
                time.sleep(0.02)
            expect("authorized write still uncommitted", proof.observe(job), [])
            expect("blocked cancellation still uncommitted", json.loads(proof.job(job))["cancel_requested_at_ms"], None)
            expect("authorized write commits", marker(writer, "COMMIT;", "released"), "released")
            result, errors = canceller.communicate(timeout=8)
            expect(f"operator cancellation exit status ({errors.strip()})", canceller.returncode, 0)
            expect("operator cancellation committed", result.strip(), "t")

    cancelled = json.loads(proof.job(job))
    expect("committed cancellation recorded", isinstance(cancelled["cancel_requested_at_ms"], int), True)
    expect("cancelled attempt otherwise remains eligible", proof.sql(f"""
        SELECT state = 'executing' AND run = 1 AND attempt = 1 AND lease_token = '{token}'
            AND lease_expires_at_ms > mesh_jobs.now_ms()
        FROM mesh_jobs.jobs WHERE namespace = '{NAMESPACE}' AND id = '{job}';
    """), "t")
    after = proof.observe(job)
    expect("only the authorized write committed", [row["stage"] for row in after], [1])
    proof.sql(second, login=CALLER, state="55000")
    expect("cancellation prevents the next write", proof.observe(job), after)


def main(data_dir):
    proof = Proof()
    expect("owned PostgreSQL backend over TLS", proof.sql("""
        SELECT current_database(), current_setting('data_directory'), ssl
        FROM pg_stat_ssl WHERE pid = pg_backend_pid();
    """), f"mesh_jobs_proof|{data_dir.resolve()}|t")
    setup(proof)
    job = proof.enqueue()
    token = proof.claim(job)
    check_owner_replay(proof, job, token)
    check_writes(proof, job, token)
    check_cancellation(proof)
    print("Jobs effect fence proof passed. Two independent commits, owner replay, and cancellation ordering verified.")


if __name__ == "__main__":
    main(Path(sys.argv[1]))
