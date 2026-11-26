#!/usr/bin/env python3
"""Run the isolated one-slot compiled candidate against the disposable Jobs server."""

import json
import os
from pathlib import Path
import secrets
from select import select
import shutil
import signal
import subprocess
import sys
import time
from urllib.parse import quote, urlencode, urlsplit


FIXTURE = Path(__file__).resolve().parent
ROOT = FIXTURE.parents[2]
NS = "compiled-effect-proof"
ROLES = {"compiled_runner": 3, "compiled_caller": 1, "compiled_producer": 1}
WORKER_ARG = "--mesh-internal-job-attempt-effect-v1"


def wait_for(check, label, owner=None):
    deadline = time.monotonic() + 12
    while time.monotonic() < deadline:
        if owner is not None and owner.poll() is not None:
            raise AssertionError(f"runner exited before {label}")
        value = check()
        if value:
            return value
        time.sleep(0.025)
    raise AssertionError(f"timed out waiting for {label}")


def equal(actual, expected, label):
    if actual != expected:
        raise AssertionError(f"{label}: expected {expected!r}, got {actual!r}")


def literal(value):
    return "'" + value.replace("'", "''") + "'"


class Producer:
    """Keep exactly one producer and observer connection throughout the workload."""

    def __init__(self, process):
        self.process = process

    def query(self, sql):
        wire = (sql.strip().rstrip(";") + ";\n\\echo __mesh_effect_result__\n").encode()
        self.process.stdin.write(wire)
        deadline = time.monotonic() + 6
        output = bytearray()
        marker = b"__mesh_effect_result__\n"
        while not output.endswith(marker):
            left = deadline - time.monotonic()
            if left <= 0 or not select([self.process.stdout], [], [], left)[0]:
                raise RuntimeError("producer did not finish its SQL within six seconds")
            more = os.read(self.process.stdout.fileno(), 4096)
            if not more:
                raise RuntimeError("producer exited before its SQL result")
            output.extend(more)
        return output[:-len(marker)].decode().strip()


class Proof:
    def __init__(self, scratch):
        self.path = scratch / "compiled-effects"
        self.path.mkdir()
        self.url = urlsplit(os.environ["MESH_TEST_DATABASE_URL"])
        self.passwords = {role: secrets.token_hex(16) for role in ROLES}
        self.ca = scratch / "ca.crt"
        target = ROOT / os.environ.get("CARGO_TARGET_DIR", "target")
        self.meshc = target / "debug/meshc"
        self.binary = self.path / "proof"
        self.listen = os.environ.get("MESH_EFFECT_LISTEN") == "1"
        self.env = dict(
            os.environ,
            MESH_EFFECT_JOBS_URL=self.connection("compiled_runner", True),
            MESH_EFFECT_APP_URL=self.connection("compiled_caller", True),
            MESH_EFFECT_MARKS=str(self.path),
            MESH_EFFECT_MIGRATE="0",
        )

    def connection(self, role, with_password=False):
        credentials = role
        if with_password:
            credentials += ":" + quote(self.passwords[role], safe="")
        query = urlencode({"sslmode": "verify-full", "sslrootcert": str(self.ca)})
        return self.url._replace(
            netloc=f"{credentials}@{self.url.hostname}:{self.url.port}", query=query
        ).geturl()

    def sql(self, query, private=False):
        command = ["psql", "-XqAtw", "-v", "ON_ERROR_STOP=1", self.url.geturl()]
        result = subprocess.run(
            command, input=query, env=dict(os.environ, PGCONNECT_TIMEOUT="4"),
            capture_output=True, text=True, timeout=8,
        )
        if result.returncode:
            raise RuntimeError("disposable role setup failed" if private else result.stderr.strip())
        return result.stdout.strip()

    def prepare(self):
        subprocess.run(
            ["cargo", "build", "--locked", "-p", "meshc", "-p", "mesh-rt"],
            cwd=ROOT, check=True,
        )
        shutil.copyfile(FIXTURE / "main.mpl", self.path / "main.mpl")
        subprocess.run(
            [str(self.meshc), "build", ".", "--output", str(self.binary)],
            cwd=self.path, check=True,
            env=dict(os.environ, MESH_RT_LIB_PATH=str(self.meshc.with_name("libmesh_rt.a"))),
        )
        bootstrap = subprocess.run(
            [self.binary, WORKER_ARG], cwd=self.path, env={}, stdin=subprocess.DEVNULL,
            capture_output=True, timeout=5,
        )
        equal(bootstrap.returncode, 174, "worker without a control socket failed bootstrap")
        equal(bootstrap.stdout, b"", "failed bootstrap did not publish a reply")
        print("Compiled Jobs bootstrap: rejected an invalid control descriptor")

        logins = "\n".join(
            f"CREATE ROLE {role} LOGIN CONNECTION LIMIT {limit} PASSWORD '{self.passwords[role]}';"
            for role, limit in ROLES.items()
        )
        self.sql(
            logins + """
            CREATE ROLE compiled_definer NOLOGIN;
            ALTER DATABASE mesh_jobs_proof CONNECTION LIMIT 5;
            GRANT CREATE ON DATABASE mesh_jobs_proof TO compiled_runner;
            GRANT pg_read_all_stats, pg_signal_backend TO compiled_producer;
            """,
            private=True,
        )
        subprocess.run(
            [self.binary], env=dict(self.env, MESH_EFFECT_MIGRATE="1"),
            cwd=self.path, check=True, timeout=15,
        )
        fence = ROOT / "compiler/mesh-rt/src/jobs/effect_fence_schema.sql"
        self.sql("SET ROLE compiled_runner;\n" + fence.read_text() + """
            GRANT USAGE ON SCHEMA mesh_jobs TO compiled_producer;
            GRANT SELECT, INSERT, UPDATE ON ALL TABLES IN SCHEMA mesh_jobs TO compiled_producer;
            GRANT USAGE ON ALL SEQUENCES IN SCHEMA mesh_jobs TO compiled_producer;
            """)
        self.sql("SET ROLE compiled_runner;\n" + (FIXTURE / "application.sql").read_text())

    def producer(self, query):
        return self.producer_session.query(query)

    def status(self):
        try:
            return json.loads((self.path / "status").read_text())
        except (FileNotFoundError, json.JSONDecodeError):
            return {}

    def enqueue(self, owner, marks, label, max_attempts=1, timeout_ms=40_000):
        spec = json.dumps(dict(
            worker="proof", version=1, queue="default", args={"path": str(marks), "label": label},
            metadata={}, tags=[], priority=0, max_attempts=max_attempts, timeout_ms=timeout_ms,
            backoff_base_ms=0, backoff_max_ms=0, suspended=False, unique=None,
        ))
        sql = f"SELECT mesh_jobs.enqueue('{NS}', {literal(spec)}::jsonb, '', '') ->> 'id'"
        if not self.listen:
            return self.producer(sql)

        def subscribed():
            report = self.status()
            wakeups = report.get("wakeups", {})
            if (wakeups.get("subscribed") and wakeups["subscription_scans"] > 0
                    and report["pending_claims"] == 0):
                return wakeups
            return None

        before = wait_for(subscribed, "initial LISTEN subscription scan", owner)
        job = self.producer(f"""
            BEGIN;
            SET LOCAL mesh_jobs.publish_wakeups = on;
            {sql};
            COMMIT
            """)

        def notified():
            wakeups = self.status().get("wakeups", {})
            if wakeups.get("notification_scans", 0) > before["notification_scans"]:
                return wakeups
            return None

        after = wait_for(notified, "LISTEN notification scan after enqueue", owner)
        equal(after["fallback_scans"], 0, "no fallback polling before LISTEN proof")
        print(
            f"Compiled Jobs LISTEN: {before['subscription_scans']} subscription scans, "
            f"notification scans {before['notification_scans']} -> {after['notification_scans']}, "
            f"{after['fallback_scans']} fallback scans"
        )
        return job

    def state(self, job):
        return self.producer(
            f"SELECT state || ':' || attempt FROM mesh_jobs.jobs WHERE id = {literal(job)}::uuid"
        )

    def effects(self, job):
        return json.loads(self.producer(f"""
            SELECT COALESCE(jsonb_agg(to_jsonb(e) ORDER BY stage), '[]'::jsonb)
            FROM compiled_demo.effects e WHERE job_id = {literal(job)}::uuid
            """))

    def warm_idle(self):
        rows = json.loads(self.producer("""
            SELECT jsonb_agg(jsonb_build_object('login', usename, 'pid', pid,
                'idle', state = 'idle' AND xact_start IS NULL,
                'tls', (SELECT ssl FROM pg_catalog.pg_stat_ssl WHERE pid = a.pid)))
            FROM pg_catalog.pg_stat_activity a
            WHERE datname = current_database() AND backend_type = 'client backend'
            """))
        if sorted(row["login"] for row in rows) != sorted(
            ["compiled_runner"] * 3 + ["compiled_caller", "compiled_producer"]
        ):
            return None
        effect = next(row for row in rows if row["login"] == "compiled_caller")
        return effect["pid"] if effect["idle"] and effect["tls"] else None

    def children(self, owner):
        children = set()
        for path in Path(f"/proc/{owner.pid}/task").glob("*/children"):
            try:
                children.update(map(int, path.read_text().split()))
            except FileNotFoundError:
                pass
        return children

    def run(self):
        self.prepare()
        producer_role = "compiled_producer"
        command = ["psql", "-XqAtw", "-v", "ON_ERROR_STOP=1", self.connection(producer_role)]
        env = dict(os.environ, PGPASSWORD=self.passwords[producer_role], PGCONNECT_TIMEOUT="4",
                   PGOPTIONS="-c statement_timeout=4000")
        with subprocess.Popen(command, env=env, stdin=subprocess.PIPE,
                              stdout=subprocess.PIPE, bufsize=0) as producer:
            self.producer_session = Producer(producer)
            try:
                self.run_compiled(self.reconcile)
                self.run_compiled(self.timeout)
                self.run_compiled(self.stage_disconnect)
                self.run_compiled(self.reconcile_retry)
                self.run_compiled(self.parent_disconnect, expected_exit=-signal.SIGKILL)
            finally:
                if producer.poll() is None:
                    producer.terminate()

    def run_compiled(self, check, expected_exit=0):
        for name in ("ready", "status", "stop"):
            (self.path / name).unlink(missing_ok=True)
        with (self.path / "runner.log").open("w+") as log:
            with subprocess.Popen([self.binary], cwd=self.path, env=self.env, stdout=log, stderr=log) as owner:
                try:
                    check(owner)
                    if expected_exit == 0:
                        (self.path / "stop").touch()
                    owner.wait(timeout=10)
                    equal(owner.returncode, expected_exit, "runner exit")
                    wait_for(lambda: self.producer("""
                        SELECT count(*) = 0 FROM pg_catalog.pg_stat_activity
                        WHERE datname = current_database() AND usename IN ('compiled_runner', 'compiled_caller')
                    """) == "t", "runner and effect backends released")
                except BaseException:
                    if owner.poll() is None:
                        owner.terminate()
                        try:
                            owner.wait(timeout=5)
                        except subprocess.TimeoutExpired:
                            owner.kill()
                    log.flush()
                    log.seek(0)
                    print(log.read(), file=sys.stderr, end="")
                    raise

    def reconcile(self, owner):
        wait_for((self.path / "ready").exists, "compiled runner admission", owner)
        backend = wait_for(self.warm_idle, "cap-five prewarm", owner)
        marks = self.path / "positive"
        marks.mkdir()
        job = self.enqueue(owner, marks, "typed value")
        wait_for((marks / "committed-1").exists, "first effect receipt", owner)
        children = self.children(owner)
        equal(len(children), 1, "one fresh exec'd worker")
        child = children.pop()
        environ = Path(f"/proc/{child}/environ").read_bytes().split(b"\0")
        expected_env = {b"MESH_SCHEDULER_MIN_WORKERS=1", b"MESH_SCHEDULER_MAX_WORKERS=1"}
        if self.env.get("MESH_DIAGNOSTIC_JOB_DISABLE_THP") == "1":
            expected_env.add(b"MESH_DIAGNOSTIC_JOB_DISABLE_THP=1")
        if set(environ[:-1]) != expected_env or environ[-1] != b"":
            raise AssertionError("worker environment did not match the launch allowlist")
        equal(
            Path(f"/proc/{child}/cmdline").read_bytes().split(b"\0")[1:],
            [WORKER_ARG.encode(), b""],
            "worker command line",
        )
        first = self.effects(job)
        equal([effect["stage"] for effect in first], [1], "one committed operation during hold")
        equal(first[0]["backend"], backend, "first write used the admitted warm session")
        equal(self.warm_idle(), backend, "warm session idle across first handler hold")
        equal(self.state(job), "executing:1", "claim still executing after first commit")
        (marks / "release-1").touch()
        wait_for((marks / "committed-2").exists, "second effect receipt", owner)
        second = self.effects(job)
        equal([effect["stage"] for effect in second], [1, 2], "two committed operations")
        equal([effect["job_run"] for effect in second], [1, 1], "one run")
        equal([effect["job_attempt"] for effect in second], [1, 1], "one attempt")
        equal([effect["backend"] for effect in second], [backend, backend], "exact reused session")
        equal([effect["label"] for effect in second], ["typed value", "typed value"], "declared bind values")
        equal(len({effect["xid"] for effect in second}), 2, "independent transactions")
        equal(self.children(owner), {child}, "same exact child across both commits")
        equal(self.warm_idle(), backend, "warm session idle across second handler hold")
        equal(self.state(job), "executing:1", "claim still executing with child held")
        (marks / "release-2").touch()
        wait_for(lambda: self.state(job) == "completed:1", "durable settlement", owner)
        equal(self.children(owner), set(), "worker reaped at settlement")
        print("Compiled Jobs effects: cap 5, same verified warm backend, two commits, one reaped child")

    def timeout(self, owner):
        wait_for((self.path / "ready").exists, "deadline runner admission", owner)
        wait_for(self.warm_idle, "deadline session prewarm", owner)
        marks = self.path / "timeout"
        marks.mkdir()
        job = self.enqueue(owner, marks, "held past deadline", timeout_ms=3000)
        wait_for((marks / "committed-1").exists, "effect before the handler deadline", owner)
        wait_for(lambda: self.state(job) == "discarded:1", "durable timeout settlement", owner)
        equal(self.children(owner), set(), "timed-out child reaped at settlement")
        equal([row["stage"] for row in self.effects(job)], [1], "no effect after the deadline")
        equal(self.producer(
            f"SELECT errors->0->>'reason' FROM mesh_jobs.jobs WHERE id = {literal(job)}::uuid"
        ), "job handler timed out", "deadline outcome")
        print("Compiled Jobs deadline: reaped the held child and preserved its first commit")

    def stage_disconnect(self, owner):
        wait_for((self.path / "ready").exists, "fault runner admission", owner)
        backend = wait_for(self.warm_idle, "fault session prewarm", owner)
        self.fault_marks = self.path / "disconnect"
        self.fault_marks.mkdir()
        self.fault_job = self.enqueue(owner, self.fault_marks, "stage disconnect", max_attempts=2)
        wait_for((self.fault_marks / "committed-1").exists, "first fault-job effect", owner)
        self.first_effect = self.effects(self.fault_job)
        equal([row["stage"] for row in self.first_effect], [1], "first effect across disconnect")
        (self.fault_marks / "release-1").touch()

        def active_operation():
            return self.producer(f"""
                SELECT pid FROM pg_catalog.pg_locks
                WHERE pid = {backend} AND locktype = 'advisory' AND granted
                  AND classid = 1782495102 AND objid = 422
                """)

        wait_for(active_operation, "second transaction in progress", owner)
        equal(self.producer(f"SELECT pg_catalog.pg_terminate_backend({backend})"), "t", "lost receipt")
        wait_for((self.fault_marks / "ignored-unknown").exists, "handler ignored the unconfirmed effect", owner)
        wait_for(lambda: self.state(self.fault_job) == "retryable:1", "one staged attempt", owner)
        equal(self.children(owner), set(), "failed worker reaped at settlement")

        def retired():
            report = self.status()
            return report.get("state") == "stopped" and report["last_error"] == "Jobs effect session retired"

        wait_for(retired, "runner stopped new claims after retirement", owner)
        equal(self.state(self.fault_job), "retryable:1", "retired runner did not consume the next attempt")
        equal(self.effects(self.fault_job), self.first_effect, "no second durable effect before restart")
        if (self.fault_marks / "committed-2").exists():
            raise AssertionError("the disconnected attempt received a second commit receipt")
        (self.fault_marks / "release-1").unlink()
        (self.fault_marks / "committed-1").unlink()

    def reconcile_retry(self, owner):
        wait_for((self.fault_marks / "committed-1").exists, "replacement attempt reconciled first effect", owner)
        equal(self.state(self.fault_job), "executing:2", "replacement claim")
        equal(self.effects(self.fault_job), self.first_effect, "first durable effect kept its original xid")
        (self.fault_marks / "release-1").touch()
        wait_for((self.fault_marks / "committed-2").exists, "replacement committed second effect", owner)
        records = self.effects(self.fault_job)
        equal(records[:1], self.first_effect, "first effect stayed idempotent")
        equal([(r["stage"], r["job_attempt"]) for r in records], [(1, 1), (2, 2)], "only the later attempt recorded stage two")
        (self.fault_marks / "release-2").touch()
        wait_for(lambda: self.state(self.fault_job) == "completed:2", "replacement settlement", owner)
        equal(self.children(owner), set(), "replacement worker reaped")
        print("Compiled Jobs fault: one interrupted submission, two durable effects after a replacement attempt")

    def parent_disconnect(self, owner):
        wait_for((self.path / "ready").exists, "parent-death runner admission", owner)
        marks = self.path / "parent-death"
        marks.mkdir()
        job = self.enqueue(owner, marks, "runner disconnected")
        wait_for((marks / "committed-1").exists, "effect before parent death", owner)
        children = self.children(owner)
        equal(len(children), 1, "one held child before parent death")
        child = children.pop()
        stat = Path(f"/proc/{child}/stat")

        def identity():
            fields = stat.read_text().rsplit(")", 1)[1].split()
            return fields[0], fields[19]

        state, start_time = identity()
        if state == "Z":
            raise AssertionError("the held child already exited")
        owner.kill()

        def child_exited():
            try:
                state, current_start = identity()
            except FileNotFoundError:
                return True
            return state == "Z" or current_start != start_time

        wait_for(child_exited, "control EOF stopped the held child")
        equal(self.state(job), "executing:1", "no settlement after runner death")
        equal([row["stage"] for row in self.effects(job)], [1], "only the first effect committed")
        print("Compiled Jobs parent death: control EOF stopped the exact held child")


if __name__ == "__main__":
    Proof(Path(sys.argv[1]).resolve().parent).run()
