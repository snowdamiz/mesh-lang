#!/usr/bin/env python3
"""Run the pinned Mesh and Oban Jobs fixtures against an owned PostgreSQL cluster."""

import argparse
import contextlib
import errno
import hashlib
import importlib.util
import json
import math
import os
import platform
import random
import re
import secrets
import shutil
import signal
import socket
import stat
import statistics
import subprocess
import sys
import tempfile
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from urllib.parse import quote

ROOT = Path(__file__).resolve().parents[1]
OBAN = ROOT / "scripts/fixtures/jobs-parity-oban"
MESH_SOURCE = ROOT / "tests/e2e/durable_jobs/.parity"
CONFIG = Path(__file__).with_name("jobs-parity-profiles.json")
EVIDENCE = ROOT / "target/jobs-parity"
HARNESS_PATHS = (
    "scripts/jobs-parity.py", "scripts/jobs-parity-profiles.json", "scripts/lib/jobs_parity_fault.py",
    str(OBAN.relative_to(ROOT)), str(MESH_SOURCE.relative_to(ROOT)),
)
REFERENCE = ("db606548", "59f6a7b3", "7c162056")
POSTGRES_VERSION = re.compile(r"[0-9]+\.[0-9]+")
EVENT_BUFFER_SINKS = ("reader", "logger")
EVENT_BUFFER_METRICS = ("sampled_peak_bytes", "upper_bound_bytes")
LISTENER_POLL_MS = 1000
LISTENER_OUTAGE_PROFILE = ("sparse", 1, 16)
SCHEMA = """
CREATE TABLE public.mesh_jobs_parity_attempts (
    engine text NOT NULL, namespace text NOT NULL, job_key text NOT NULL,
    effect_key text NOT NULL, attempt integer NOT NULL, entered_at timestamptz NOT NULL,
    worker_pid integer NOT NULL
);
CREATE INDEX ON public.mesh_jobs_parity_attempts (engine, namespace, job_key);
CREATE TABLE public.mesh_jobs_parity_effects (
    engine text NOT NULL, namespace text NOT NULL, job_key text NOT NULL,
    effect_key text NOT NULL, effected_at timestamptz NOT NULL,
    PRIMARY KEY (engine, namespace, effect_key)
);
CREATE TABLE public.mesh_jobs_parity_product (
    engine text NOT NULL, namespace text NOT NULL, request_id text NOT NULL,
    PRIMARY KEY (engine, namespace, request_id)
);
"""


class FixtureError(Exception):
    pass


class RunnerCleanupError(FixtureError):
    pass


class QueueMismatch(Exception):
    pass


class RunnerProcess(subprocess.Popen):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self.observed_workers = {}
        self.fixture_executable = None


def checked(args, *, cwd=ROOT, env=None, timeout=120, log=None):
    result = subprocess.run(args, cwd=cwd, env=env, text=True, timeout=timeout,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if log:
        log.write_text(result.stdout + result.stderr)
    if result.returncode:
        # Fixture failures can include connection arguments. Keep the diagnostic local.
        raise FixtureError(f"{Path(args[0]).name} exited {result.returncode}"
                           + (f"; see {log}" if log else f": {result.stderr.strip()}"))
    return result.stdout.strip()


def write_json(path, data):
    with path.open("x") as stream:
        json.dump(data, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write("\n")


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def allocate_slots(runners, slots):
    return [slots // runners + (index < slots % runners) for index in range(runners)]


def inputs(workload, slots, seed, smoke=False):
    count = max(workload["minimum_jobs"], slots * workload["jobs_per_slot"])
    if smoke:
        count = 4
    rng = random.Random(seed)
    batch_size = 1 if workload["mode"] == "sparse" else count
    batches = []
    for start in range(0, count, batch_size):
        jobs = [{
            "key": f"job_{index:06d}",
            "payload": rng.randbytes((workload["payload_bytes"] + 1) // 2).hex()[:workload["payload_bytes"]],
            "hold_ms": workload["hold_ms"],
            "fail_first": False,
        } for index in range(start, min(start + batch_size, count))]
        batches.append({"request_id": f"batch_{start:06d}", "jobs": jobs, "rollback": False})
    return batches


def percentile(samples, rank):
    if not samples or not all(math.isfinite(sample) for sample in samples):
        raise FixtureError("missing or nonfinite latency sample")
    return sorted(samples)[math.ceil(len(samples) * rank) - 1]


def within_clock(value, tolerance):
    if not isinstance(value, (float, int)) or not math.isfinite(value) or value < -tolerance:
        raise FixtureError(f"invalid clock sample: {value!r} ms (tolerance {tolerance} ms)")
    return max(0.0, value)


def account(rows, expected, commits, tolerance):
    keys = [row["job_key"] for row in rows]
    if len(keys) != len(set(keys)) or set(keys) != set(expected):
        raise QueueMismatch("persisted job keys do not match the input manifest")
    samples = []
    for row in rows:
        key = row["job_key"]
        if row["state"] != "completed" or row["attempt"] != 1 or row["first_handler_attempt"] != 1:
            raise QueueMismatch(f"{key}: expected one completed attempt")
        if row["attempts"] != 1 or row["effects"] != 1 or row["effect_keys"] != 1:
            raise QueueMismatch(f"{key}: expected one raw attempt and one deduplicated effect")
        request_id, job = expected[key]
        fields = ("payload_sha256", "hold_ms", "fail_first")
        expected_fields = (hashlib.sha256(job["payload"].encode()).hexdigest(),
                           job["hold_ms"], job["fail_first"])
        if tuple(row[field] for field in fields) != expected_fields:
            raise QueueMismatch(f"{key}: persisted job inputs differ from the fixture manifest")
        committed = commits[request_id]
        claimed, entered, recorded, effected, settled = (row[field] for field in (
            "claimed_ms", "entered_ms", "recorded_ms", "effected_ms", "settled_ms",
        ))
        if any(value is None for value in (claimed, entered, recorded, effected, settled)):
            raise FixtureError(f"{key}: missing clock sample")
        within_clock(settled - entered, tolerance)
        samples.append({"job_key": key, "commit_ms": committed, **row,
                        "commit_to_claim_ms": within_clock(claimed - committed, tolerance),
                        "pickup_ms": within_clock(entered - committed, tolerance),
                        "claim_to_entry_ms": within_clock(entered - claimed, tolerance),
                        "entry_to_attempt_commit_ms": within_clock(recorded - entered, tolerance),
                        "attempt_to_effect_commit_ms": within_clock(effected - recorded, tolerance),
                        "effect_commit_to_settlement_ms": within_clock(settled - effected, tolerance),
                        "commit_to_settlement_ms": within_clock(settled - committed, tolerance)})
    return samples


def validate_pairs(trials, pairs, slots, variants=("mesh", "oban")):
    booked = [(trial["pair"], trial["engine"]) for trial in trials]
    expected = {(i, engine) for i in range(pairs) for engine in variants}
    if len(booked) != len(expected) or set(booked) != expected:
        raise FixtureError("incomplete or duplicate paired trials")
    if len({trial["input_sha256"] for trial in trials}) != 1:
        raise FixtureError("paired trials received different inputs")
    positive = ("completed", "jobs_per_second", "sql_calls_per_job", "runner_sql_calls_per_job",
                "enqueue_sql_calls_per_job", "claim_calls_per_job",
                "estimated_peak_executing", "peak_connections", "peak_rss_bytes", "cpu_seconds_per_job",
                "peak_processes", "peak_threads")
    nonnegative = ("pickup_p99_ms", "claim_p99_ms", "settlement_p99_ms", "idle_queries_per_minute",
                   "peak_executing")
    for trial in trials:
        for key in (*positive, *nonnegative):
            value = trial.get(key)
            if (type(value) not in (int, float) or not math.isfinite(value)
                    or value < 0 or (key in positive and value == 0)):
                raise FixtureError(f"missing or invalid {key} in pair {trial['pair']}")
        if trial["peak_threads"] < trial["peak_processes"]:
            raise FixtureError(f"missing threads in pair {trial['pair']}")
        if type(trial["peak_executing"]) is not int or trial["peak_executing"] > slots:
            raise QueueMismatch(f"observed occupancy exceeds {slots} slots in pair {trial['pair']}")


def postgres_tools():
    configured = os.environ.get("PG_BIN_DIR")
    if configured:
        bindir = Path(configured)
    elif shutil.which("initdb"):
        bindir = Path(shutil.which("initdb")).parent
    else:
        bindir = Path(checked(["pg_config", "--bindir"]))
    for name in ("initdb", "pg_ctl", "psql", "createdb"):
        if not (bindir / name).is_file():
            raise FixtureError(f"set PG_BIN_DIR to a PostgreSQL installation containing {name}")
    return bindir


class Cluster:
    def __init__(self, scratch, evidence, connection_cap, expected_version=None):
        self.scratch = scratch
        self.evidence = evidence
        self.bin = postgres_tools()
        self.data = scratch / "pgdata"
        self.nonce = secrets.token_hex(16)
        self.password = secrets.token_hex(16)
        self.connection_cap = connection_cap
        self.expected_version = expected_version
        self.env = {k: v for k, v in os.environ.items() if not k.startswith("PG")}
        self.env["TMPDIR"] = str(scratch)
        with socket.socket() as port:
            port.bind(("127.0.0.1", 0))
            self.port = port.getsockname()[1]

    def call(self, name, *args, **kwargs):
        return checked([str(self.bin / name), *args], env=self.env, **kwargs)

    def start(self):
        password_file = self.scratch / "password"
        password_file.touch(mode=0o600)
        password_file.write_text(self.password + "\n")
        try:
            self.call("initdb", "-D", str(self.data), "-U", "parity_owner", "--no-locale",
                      "--encoding=UTF8", "--auth-local=trust", "--auth-host=scram-sha-256",
                      f"--pwfile={password_file}", log=self.evidence / "initdb.log")
        finally:
            password_file.unlink()
        with (self.data / "postgresql.conf").open("a") as stream:
            stream.write(f"""
listen_addresses = '127.0.0.1'
port = {self.port}
unix_socket_directories = ''
shared_preload_libraries = 'pg_stat_statements'
pg_stat_statements.track = 'top'
track_commit_timestamp = on
synchronous_commit = on
max_connections = 128
timezone = 'UTC'
""")
        if self.expected_version:
            self.configure_tls()
        self.call("pg_ctl", "-D", str(self.data), "-w", "-t", "20", "-l",
                  str(self.evidence / "postgres.log"), "start")
        if self.expected_version:
            expected = tuple(map(int, self.expected_version.split(".")))
            observed = int(self.sql("postgres", "SHOW server_version_num"))
            if observed != expected[0] * 10_000 + expected[1]:
                raise FixtureError(f"expected PostgreSQL {self.expected_version}, got {observed}")
            if self.sql("postgres", "SELECT ssl FROM pg_stat_ssl WHERE pid=pg_backend_pid()") != "t":
                raise FixtureError("the compatibility observer must use TLS")
        self.sql("postgres", "CREATE EXTENSION pg_stat_statements")
        self.sql("postgres", f"CREATE ROLE parity_subject LOGIN PASSWORD '{self.password}'")
        self.sql("postgres", f"CREATE ROLE parity_producer LOGIN IN ROLE parity_subject "
                 f"PASSWORD '{self.password}'")

    def prepare_database(self, engine):
        self.sql("postgres", f"DROP DATABASE IF EXISTS {engine}")
        self.sql("postgres", f"CREATE DATABASE {engine} OWNER parity_subject "
                 f"CONNECTION LIMIT {self.connection_cap}")
        self.sql(engine, f"SET ROLE parity_subject; {SCHEMA}")
        self.sql(engine, f"CREATE TABLE public.mesh_jobs_parity_authority"
                 f" (nonce text NOT NULL); INSERT INTO public.mesh_jobs_parity_authority VALUES ('{self.nonce}')")

    def configure_tls(self):
        self.certificate = self.scratch / "ca.crt"
        ca_key = self.scratch / "ca.key"
        server_cert = self.data / "server.crt"
        request = self.scratch / "server.csr"
        key = self.data / "server.key"
        extensions = self.scratch / "server.ext"
        extensions.write_text("subjectAltName=IP:127.0.0.1\n")
        checked(["openssl", "req", "-new", "-x509", "-newkey", "rsa:2048", "-nodes",
                 "-days", "1", "-subj", "/CN=jobs-parity-ca", "-keyout", str(ca_key),
                 "-out", str(self.certificate)], env=self.env, log=self.evidence / "openssl-ca.log")
        checked(["openssl", "req", "-new", "-newkey", "rsa:2048", "-nodes", "-subj", "/CN=localhost",
                 "-keyout", str(key), "-out", str(request)], env=self.env,
                log=self.evidence / "openssl-csr.log")
        checked(["openssl", "x509", "-req", "-in", str(request), "-CA", str(self.certificate),
                 "-CAkey", str(ca_key), "-CAcreateserial", "-days", "1", "-extfile", str(extensions),
                 "-out", str(server_cert)], env=self.env, log=self.evidence / "openssl-server.log")
        key.chmod(0o600)
        with (self.data / "postgresql.conf").open("a") as stream:
            stream.write("ssl = on\n")
        (self.data / "pg_hba.conf").write_text("hostssl all all 127.0.0.1/32 scram-sha-256\n")

    def stop(self):
        status = subprocess.run([str(self.bin / "pg_ctl"), "-D", str(self.data), "status"],
                                env=self.env, capture_output=True)
        if status.returncode == 0:
            try:
                self.call("pg_ctl", "-D", str(self.data), "-m", "fast", "-t", "10", "-w", "stop")
            except FixtureError:
                self.call("pg_ctl", "-D", str(self.data), "-m", "immediate", "-t", "10", "-w", "stop")

    def url(self, database, user="parity_subject", port=None):
        options = (f"sslmode=verify-full&sslrootcert={quote(str(self.certificate))}"
                   if self.expected_version else "sslmode=disable")
        return f"postgresql://{user}:{self.password}@127.0.0.1:{port or self.port}/{database}?{options}"

    def sql(self, database, query):
        env = dict(self.env, PGPASSWORD=self.password)
        env["PGSSLMODE"] = "verify-full" if self.expected_version else "disable"
        if self.expected_version:
            env["PGSSLROOTCERT"] = str(self.certificate)
        return checked([str(self.bin / "psql"), "-X", "-w", "-qAt", "-v", "ON_ERROR_STOP=1",
                        "-h", "127.0.0.1", "-p", str(self.port), "-U", "parity_owner",
                        "-d", database, "-c", query], env=env, timeout=15)

    def calls(self, engine):
        claim_query = ("query LIKE '%FROM mesh_jobs.claim%'" if engine == "mesh"
                       else "query LIKE '%oban_jobs%' AND query ILIKE '%FOR UPDATE SKIP LOCKED%'")
        return json.loads(self.sql("postgres", f"""
SELECT json_build_object(
 'runner', coalesce(sum(calls) FILTER (WHERE rolname='parity_subject'),0),
 'enqueue', coalesce(sum(calls) FILTER (WHERE rolname='parity_producer'),0),
 'claims', coalesce(sum(calls) FILTER (WHERE rolname='parity_subject' AND ({claim_query})),0),
 'dealloc', (SELECT dealloc FROM pg_stat_statements_info)
)
FROM pg_stat_statements JOIN pg_roles ON userid = pg_roles.oid
WHERE dbid = (SELECT oid FROM pg_database WHERE datname = '{engine}')
  AND rolname IN ('parity_subject', 'parity_producer')
"""))

    def observe(self, engine, namespace):
        relation, scope = job_relation(engine, namespace)
        return json.loads(self.sql(engine, f"""
SELECT json_build_object(
 'connections', (SELECT count(*) FROM pg_stat_activity WHERE usename IN ('parity_subject', 'parity_producer')
                    AND datname = current_database()),
 'executing', count(*) FILTER (WHERE state = 'executing'),
 'terminal', count(*) FILTER (WHERE state IN ('completed','discarded','cancelled'))
)
FROM {relation} WHERE {scope}
"""))

    def rows(self, engine, namespace):
        relation, scope = job_relation(engine, namespace)
        claim = "j.attempted_at_ms" if engine == "mesh" else "extract(epoch FROM j.attempted_at) * 1000"
        effect_key = "j.id::text || '/1'" if engine == "mesh" else "j.args->>'key'"
        return json.loads(self.sql(engine, f"""
WITH attempts AS (
 SELECT job_key, count(*) AS attempts, min(attempt) AS first_handler_attempt, min(entered_at) AS entered_at,
        min(pg_xact_commit_timestamp(xmin)) AS recorded_at, count(DISTINCT effect_key) AS effect_keys
 FROM public.mesh_jobs_parity_attempts
 WHERE engine = '{engine}' AND namespace = '{namespace}' GROUP BY job_key
), effects AS (
 SELECT job_key, count(*) AS effects, min(pg_xact_commit_timestamp(xmin)) AS committed_at
 FROM public.mesh_jobs_parity_effects
 WHERE engine = '{engine}' AND namespace = '{namespace}' GROUP BY job_key
)
SELECT coalesce(json_agg(summary ORDER BY summary.job_key), '[]') FROM (
 SELECT j.id::text AS id, j.args->>'key' AS job_key, j.state, j.attempt, {effect_key} AS effect_key,
   encode(sha256(convert_to(j.args->>'payload', 'UTF8')), 'hex') AS payload_sha256,
   (j.args->>'hold_ms')::integer AS hold_ms, (j.args->>'fail_first')::boolean AS fail_first,
   coalesce(a.attempts,0) AS attempts, coalesce(e.effects,0) AS effects,
   a.first_handler_attempt,
   coalesce(a.effect_keys,0) AS effect_keys, ({claim}) AS claimed_ms,
   extract(epoch FROM a.entered_at)*1000 AS entered_ms,
   extract(epoch FROM a.recorded_at)*1000 AS recorded_ms,
   extract(epoch FROM e.committed_at)*1000 AS effected_ms,
   extract(epoch FROM pg_xact_commit_timestamp(j.xmin))*1000 AS settled_ms
 FROM {relation} j
 LEFT JOIN attempts a ON a.job_key = j.args->>'key'
 LEFT JOIN effects e ON e.job_key = j.args->>'key'
 WHERE {scope}
) summary
"""))

    def reconcile_trial(self, engine, namespace, enqueued_ids, handler_attempts, commits):
        relation, _ = job_relation(engine, namespace)
        if engine == "oban":
            schemas = json.loads(self.sql(engine, """
SELECT coalesce(json_agg(n.nspname ORDER BY n.nspname), '[]')
FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON c.relnamespace = n.oid
WHERE c.relname = 'oban_jobs' AND c.relkind IN ('r', 'p')
"""))
            if schemas != [namespace]:
                raise QueueMismatch("unexpected Oban job relations in the trial database")
        scope = "namespace" if engine == "mesh" else f"'{namespace}'"
        observed = json.loads(self.sql(engine, f"""
SELECT json_build_object(
 'jobs', (SELECT coalesce(json_agg(json_build_array({scope}, id::text, args->>'key')), '[]') FROM {relation}),
 'business', (SELECT coalesce(json_agg(json_build_array(engine, namespace, request_id)), '[]')
    FROM public.mesh_jobs_parity_product),
 'attempts', (SELECT coalesce(json_agg(json_build_array(engine, namespace, job_key, attempt, effect_key)), '[]')
    FROM public.mesh_jobs_parity_attempts),
 'effects', (SELECT coalesce(json_agg(json_build_array(engine, namespace, job_key, effect_key)), '[]')
    FROM public.mesh_jobs_parity_effects)
)
"""))
        effects = {key: f"{identity}/1" if engine == "mesh" else key for key, identity in enqueued_ids.items()}
        expected = {
            "jobs": [[namespace, identity, key] for key, identity in enqueued_ids.items()],
            "business": [[engine, namespace, request_id] for request_id in commits],
            "attempts": [[engine, namespace, key, attempt, effects[key]]
                         for key, attempts in handler_attempts.items() for attempt in attempts],
            "effects": [[engine, namespace, key, effects[key]]
                        for key, attempts in handler_attempts.items() if attempts],
        }
        for kind, values in expected.items():
            if sorted(observed[kind], key=repr) != sorted(values, key=repr):
                raise QueueMismatch(f"database {kind} identities do not match the completed trial")
        return observed

    def reconcile_writes(self, engine, namespace, expected):
        if not re.fullmatch(r"parity_[a-f0-9]{16}", namespace):
            raise FixtureError("invalid write observation namespace")
        writes = json.loads(self.sql(engine, """
SELECT json_build_object(
 'attempts', (SELECT coalesce(json_agg(a ORDER BY engine, namespace, job_key), '[]') FROM (
    SELECT engine, namespace, job_key, count(*)
    FROM public.mesh_jobs_parity_attempts GROUP BY engine, namespace, job_key
 ) a),
 'effects', (SELECT coalesce(json_agg(e ORDER BY engine, namespace, job_key), '[]') FROM (
    SELECT engine, namespace, job_key, count(*)
    FROM public.mesh_jobs_parity_effects GROUP BY engine, namespace, job_key
 ) e)
)
"""))
        for kind, expected_jobs in expected.items():
            observed = {(row["engine"], row["namespace"], row["job_key"]): row["count"]
                        for row in writes[kind]}
            if observed != {(engine, namespace, key): count for key, count in expected_jobs.items()}:
                raise QueueMismatch(f"database {kind} do not match expected writes: {writes[kind]}")
        return writes

    def claim_identity(self, engine, namespace, label):
        relation, scope = job_relation(engine, namespace)
        if not re.fullmatch(r"[a-z_]+", label):
            raise FixtureError("invalid claim observation job key")
        fence = "lease_token::text" if engine == "mesh" else "attempted_at::text"
        return json.loads(self.sql(engine, f"""
SELECT coalesce((SELECT json_build_object('id', id::text, 'state', state,
                                          'attempt', attempt, 'fence', {fence})
                 FROM {relation} WHERE {scope} AND args->>'key'='{label}'), 'null')
"""))

    def worker_pids(self, namespace):
        _, scope = job_relation("mesh", namespace)
        return json.loads(self.sql("mesh", f"""
SELECT coalesce(json_agg(worker_pid), '[]') FROM public.mesh_jobs_parity_attempts
WHERE engine = 'mesh' AND {scope}
"""))

    def owned(self, engine):
        return self.sql(engine, "SELECT nonce FROM public.mesh_jobs_parity_authority") == self.nonce


def job_relation(engine, namespace):
    if not re.fullmatch(r"parity_[a-f0-9]{16}", namespace):
        raise FixtureError("invalid observation namespace")
    if engine == "mesh":
        return "mesh_jobs.jobs", f"namespace = '{namespace}'"
    return f"{namespace}.oban_jobs", "true"


class Subject:
    def __init__(self, engine, command, cwd, cluster, env, *, variant=None, poll_ms=None):
        self.engine = engine
        self.variant = variant or engine
        self.command = command
        self.cwd = cwd
        self.cluster = cluster
        self.env = env
        self.poll_ms = poll_ms

    def environment(self, namespace, slots, total_slots, mode, poll_ms):
        return dict(self.env, JOBS_PARITY_DB_URL=self.cluster.url(self.engine),
                    JOBS_PARITY_POOL_URL="", JOBS_PARITY_STATUS_PATH="",
                    JOBS_PARITY_NAMESPACE=namespace, JOBS_PARITY_SLOTS=str(slots),
                    JOBS_PARITY_TOTAL_SLOTS=str(total_slots), JOBS_PARITY_MODE=mode,
                    JOBS_PARITY_POLL_MS=str(self.poll_ms or poll_ms), JOBS_PARITY_RUNNER="controller")

    def call(self, action, env, log):
        return checked([*self.command, action], cwd=self.cwd, env=env, log=log)

    def enqueue(self, env, path, log):
        receipt, committed = self.enqueue_receipt(env, path, log)
        return receipt["count"], committed

    def enqueue_receipt(self, env, path, log):
        result = self.call("enqueue", dict(env, JOBS_PARITY_INPUT=str(path),
                            JOBS_PARITY_DB_URL=self.cluster.url(self.engine, "parity_producer")), log)
        return self.read_receipt(result, log)

    def read_receipt(self, result, log):
        try:
            receipt = json.loads(result.splitlines()[-1])
            ids = receipt["ids"]
            id_pattern = r"[0-9a-f-]{36}" if self.engine == "mesh" else r"[1-9][0-9]*"
            if (not re.fullmatch(r"\d+", receipt["xid"]) or type(receipt["count"]) is not int
                    or type(ids) is not dict or len(ids) != receipt["count"]
                    or any(not isinstance(value, str) or not re.fullmatch(id_pattern, value) for value in ids.values())
                    or len(set(ids.values())) != len(ids)):
                raise ValueError("invalid enqueue receipt")
        except (ValueError, KeyError, IndexError, TypeError) as error:
            raise FixtureError(f"invalid enqueue receipt in {log}") from error
        committed = self.cluster.sql(self.engine, "SELECT extract(epoch FROM "
                                     f"pg_xact_commit_timestamp('{receipt['xid']}'::xid))*1000")
        return receipt, float(committed) if committed else None

    @contextlib.contextmanager
    def warm_producer(self, env, evidence, deadline):
        stdout = evidence / "producer-output.log"
        stderr = evidence / "producer-errors.log"
        producer_env = dict(env, JOBS_PARITY_INPUT="-",
                            JOBS_PARITY_DB_URL=self.cluster.url(self.engine, "parity_producer"))
        with stdout.open("wb") as output, stderr.open("wb") as errors:
            process = RunnerProcess([*self.command, "enqueue"], cwd=self.cwd, env=producer_env,
                                    start_new_session=True, stdin=subprocess.PIPE,
                                    stdout=output, stderr=errors)
        complete = False
        try:
            with stdout.open("rb") as output:
                def receive():
                    position = output.tell()
                    line = output.readline()
                    if line.endswith(b"\n"):
                        return line.decode()
                    output.seek(position)
                    if process.poll() is not None:
                        raise FixtureError(f"warm producer exited with {process.returncode}; see {stderr}")
                    return None

                ready = wait_for(receive, deadline, "warm producer readiness", 0.01)
                if ready != "PRODUCER_READY\n":
                    raise FixtureError(f"missing warm producer ready marker; see {stdout}")

                def enqueue(path, log):
                    try:
                        process.stdin.write(os.fsencode(path) + b"\n")
                        process.stdin.flush()
                    except BrokenPipeError as error:
                        raise FixtureError(f"warm producer pipe closed; see {stderr}") from error
                    result = wait_for(receive, deadline, f"warm producer receipt for {path.name}", 0.01)
                    log.write_text(result)
                    return self.read_receipt(result, log)

                yield enqueue
                complete = True
        finally:
            stop_runners([process], require_clean=complete)

    def run(self, env, index, log):
        with log.open("wb") as stream:
            process = RunnerProcess([*self.command, "run"], cwd=self.cwd, start_new_session=True,
                                    env=dict(env, JOBS_PARITY_RUNNER=f"runner_{index}"),
                                    stdin=subprocess.PIPE if self.engine == "oban" else subprocess.DEVNULL,
                                    stdout=stream, stderr=stream)
        process.fixture_executable = str(Path(self.command[0]).resolve()) if self.engine == "mesh" else None
        return process


def wait_for(observe, deadline, description, interval=0.1):
    while time.monotonic() < deadline:
        result = observe()
        if result:
            return result
        time.sleep(interval)
    raise FixtureError(f"timed out waiting for {description}")


def runner_ready(processes, logs, *, track=True):
    if track:
        remember_workers(processes, process_table())
    for process, log in zip(processes, logs, strict=True):
        if process.poll() is not None:
            raise FixtureError(f"runner exited {process.returncode}; see {log}")
        # Keep enough output for diagnostics without reading an unbounded stream.
        with log.open("rb") as stream:
            if b"READY" not in stream.read(32768).splitlines():
                return False
    return True


def mac_thread_counts(output, owned):
    header, _, body = output.partition("\n")
    rows = body.splitlines()
    _, label, _ = header.partition("PID")
    if not label:
        raise FixtureError("cannot read macOS thread sample without PID column")
    pid_end = header.index("PID") + len("PID")
    counts = {}
    for row in rows:
        fields = row[:pid_end].split()
        if not fields or not fields[-1].isdigit():
            continue
        pid = int(fields[-1])
        if pid in owned:
            counts[pid] = counts.get(pid, 0) + 1
    return counts


def process_table():
    table = {}
    for line in checked(["ps", "-A", "-o", "pid=,ppid=,rss=,stat=,lstart="]).splitlines():
        fields = line.split(maxsplit=4)
        if len(fields) != 5:
            continue
        pid, parent, rss, state, started = fields
        if not state.startswith("Z"):
            table[int(pid)] = (int(parent), int(rss), started)
    return table


def process_tree(table, roots):
    owned = roots & table.keys()
    while True:
        children = {pid for pid, (parent, _, _) in table.items() if parent in owned}
        if children <= owned:
            return owned
        owned |= children


def remember_workers(processes, table):
    for process in processes:
        if process.returncode is None:
            workers = process_tree(table, {process.pid}) - {process.pid}
            process.observed_workers.update({pid: table[pid][2] for pid in workers})


def fixture_executables(processes):
    return {process.fixture_executable for process in processes if process.fixture_executable}


def fixture_workers(table, executables):
    if not executables:
        return {}
    if sys.platform == "darwin":
        paths = {}
        for line in checked(["ps", "-A", "-ww", "-o", "pid=,comm="]).splitlines():
            fields = line.strip().split(maxsplit=1)
            if len(fields) == 2:
                paths[int(fields[0])] = fields[1]
    else:
        paths = {}
        for pid in table:
            with contextlib.suppress(OSError):
                paths[pid] = os.readlink(f"/proc/{pid}/exe")
    return {pid: started for pid, (_, _, started) in table.items() if paths.get(pid) in executables}


def wait_for_worker_exit(pids, executables=()):
    deadline = time.monotonic() + 5
    while pids or executables:
        live = process_table()
        pids = {pid: started for pid, started in pids.items()
                if pid in live and live[pid][2] == started}
        # A worker can be reparented before the observer sees it. The unique
        # compiled fixture path identifies its processes after the runner exits.
        pids.update(fixture_workers(live, executables))
        if not pids:
            return
        if time.monotonic() >= deadline:
            raise RunnerCleanupError(f"observed worker processes survived runner shutdown: {sorted(pids)}")
        time.sleep(0.05)


def process_sample(processes):
    for _ in range(3):
        table = process_table()
        remember_workers(processes, table)
        owned = process_tree(table, {process.pid for process in processes})
        if not owned:
            return {"processes": 0, "threads": 0, "rss_bytes": 0}
        if sys.platform == "darwin":
            command = ["ps", "-M", "-p", ",".join(map(str, sorted(owned)))]
            result = subprocess.run(command, capture_output=True, text=True, timeout=5)
            counts = mac_thread_counts(result.stdout, owned) if result.stdout else {}
        else:
            counts = {pid: len(list(Path(f"/proc/{pid}/task").glob("*"))) for pid in owned}
        after = process_table()
        stable = {pid for pid in owned if pid in after and after[pid][2] == table[pid][2]}
        if all(counts.get(pid, 0) > 0 for pid in stable):
            # A process captured by the thread sampler can exit before the
            # second process snapshot. Exclude recycled PIDs, not those exits.
            sampled = {pid for pid in owned if counts.get(pid, 0) > 0 and (pid in stable or pid not in after)}
            return {"processes": len(sampled), "threads": sum(counts[pid] for pid in sampled),
                    "rss_bytes": 1024 * sum(table[pid][1] for pid in sampled)}
    raise FixtureError("runner thread capture omitted a live process")


def stop_runners(processes, *, require_clean):
    cpu_seconds = 0.0
    error = None
    remember_workers(processes, process_table())
    for process in processes:
        if process.returncode is None:
            if process.stdin is not None:
                with contextlib.suppress(BrokenPipeError):
                    with process.stdin:
                        process.stdin.write(b"stop\n")
            else:
                with contextlib.suppress(ProcessLookupError):
                    os.kill(process.pid, signal.SIGTERM)
    for process in processes:
        if process.returncode is not None:
            if require_clean:
                error = FixtureError(f"runner exited prematurely with {process.returncode}")
            continue
        deadline = time.monotonic() + 20
        while True:
            pid, status, usage = os.wait4(process.pid, os.WNOHANG)
            if pid:
                process.returncode = os.waitstatus_to_exitcode(status)
                cpu_seconds += usage.ru_utime + usage.ru_stime
                if require_clean and process.returncode != 0:
                    error = FixtureError(f"runner shutdown returned {process.returncode}")
                break
            if time.monotonic() >= deadline:
                with contextlib.suppress(ProcessLookupError):
                    os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=5)
                error = FixtureError("runner did not stop before the cleanup deadline")
                break
            time.sleep(0.05)
    observed = {pid: started for process in processes for pid, started in process.observed_workers.items()}
    wait_for_worker_exit(observed, fixture_executables(processes))
    if error:
        raise error
    return cpu_seconds


def batch_files(scratch, batches):
    paths = []
    for batch in batches:
        path = scratch / f"{batch['request_id']}.json"
        write_json(path, batch)
        paths.append(path)
    return paths


def take_sample(cluster, engine, namespace, processes, start):
    return {
        "offset_s": time.monotonic() - start,
        "host_load_average_1m": os.getloadavg()[0],
        **cluster.observe(engine, namespace),
        **process_sample(processes),
    }


def estimated_peak_executing(rows):
    active = peak = 0
    events = [(stamp, delta) for row in rows if row["claimed_ms"] < row["settled_ms"]
              for stamp, delta in ((row["claimed_ms"], 1), (row["settled_ms"], -1))]
    for _, delta in sorted(events):
        active += delta
        peak = max(peak, active)
    return peak


class ListenerTrial:
    """Observe the disposable runner's established sessions and wake source."""

    def __init__(self, cluster, namespace, status_path, proxy, disconnected):
        self.cluster = cluster
        self.status_path = status_path
        self.proxy = proxy
        self.disconnected = disconnected
        channel = hashlib.md5(namespace.encode(), usedforsecurity=False).hexdigest()
        self.listen_sql = f'LISTEN "mesh_jobs_wake_{channel}"'
        self.scope = """a.datname = current_database() AND a.usename = 'parity_subject'
                        AND a.client_addr = inet '127.0.0.1' AND a.backend_type = 'client backend'"""

    def sessions(self):
        return json.loads(self.cluster.sql("mesh", f"""
SELECT coalesce(json_agg(json_build_object('pid', a.pid, 'port', a.client_port,
    'tls', s.ssl, 'listener', coalesce(a.query = '{self.listen_sql}' AND a.state = 'idle', false))
    ORDER BY a.pid), '[]')
FROM pg_stat_activity a JOIN pg_stat_ssl s USING (pid)
WHERE {self.scope}
"""))

    def status(self, deadline):
        self.status_path.unlink(missing_ok=True)
        self.status_path.with_suffix(".request").touch(exist_ok=False)

        def read():
            if self.status_path.is_file():
                try:
                    return json.loads(self.status_path.read_text())
                except ValueError as error:
                    raise FixtureError("invalid live Mesh runner report") from error

        report = wait_for(read, deadline, "live Mesh wake status", 0.025)
        if (not isinstance(report, dict) or report.get("state") != "running"
                or "last_error" not in report or report["last_error"] is not None):
            raise FixtureError("Mesh runner reported a storage or execution error in the listener trial")
        wake = report.get("wakeups")
        fields = ("subscriptions", "subscription_scans", "notification_scans", "due_scans", "fallback_scans")
        if (not isinstance(wake, dict) or type(wake.get("subscribed")) is not bool
                or any(type(wake.get(field)) is not int or wake[field] < 0 for field in fields)):
            raise FixtureError("invalid Mesh wake status in the listener trial")
        return wake

    def arm(self, deadline):
        def startup():
            wake = self.status(deadline)
            if wake["subscribed"] and wake["subscription_scans"] > 0 and wake["fallback_scans"] > 0:
                return wake

        self.initial_wake = wait_for(startup, deadline, "subscription and initial fallback scan")
        self.initial_sessions = self.sessions()
        listeners = [row for row in self.initial_sessions if row["listener"]]
        if (len(listeners) != 1 or len(self.initial_sessions) != 4
                or not all(row["tls"] is True for row in self.initial_sessions)
                or self.initial_wake["subscriptions"] != 1 or self.initial_wake["notification_scans"] != 0):
            raise FixtureError("expected one TLS listener and three established TLS runner sessions")
        if not self.disconnected:
            return

        # Batch claim and lifecycle sessions are already connected. The pool
        # serves maintenance, and handlers and producers use the direct URL.
        listener = listeners[0]
        self.proxy.hold.set()
        terminated = self.cluster.sql("mesh", f"""
SELECT pg_terminate_backend(a.pid)
FROM pg_stat_activity a JOIN pg_stat_ssl s USING (pid)
WHERE {self.scope} AND s.ssl AND a.state = 'idle' AND a.query = '{self.listen_sql}'
  AND a.pid = {listener['pid']} AND a.client_port = {listener['port']}
  AND EXISTS (SELECT FROM public.mesh_jobs_parity_authority WHERE nonce = '{self.cluster.nonce}')
""")
        if terminated != "t":
            raise FixtureError("could not terminate the owned PostgreSQL listener")
        wait_for(self.proxy.reconnect_arrived.is_set, deadline, "listener reconnect at the loopback gate")

    def observe(self, deadline):
        wake = self.status(deadline)
        sessions = self.sessions()
        expected = [row for row in self.initial_sessions if not (self.disconnected and row["listener"])]
        if sessions != expected or wake["subscribed"] == self.disconnected:
            raise FixtureError(f"expected TLS sessions {expected}, observed {sessions}; "
                               f"runner subscribed={wake['subscribed']}")
        if (wake["subscriptions"] != self.initial_wake["subscriptions"]
                or wake["subscription_scans"] != self.initial_wake["subscription_scans"]):
            raise FixtureError("listener unexpectedly subscribed again")
        return {"postgres_sessions": sessions, "wakeups": wake}

    def verify(self, before, after, count):
        start, end = before["wakeups"], after["wakeups"]
        if self.disconnected:
            if (end["notification_scans"] != 0 or end["due_scans"] != start["due_scans"]
                    or end["fallback_scans"] - start["fallback_scans"] < count):
                raise FixtureError("disconnected trial did not account for the sparse fallback scans")
        elif end["notification_scans"] <= start["notification_scans"]:
            raise FixtureError("healthy listener trial did not record notification-triggered scans")
        return {
            "initial_postgres_sessions": self.initial_sessions,
            "initial_wakeups": self.initial_wake,
            "reconnect_held": self.proxy.reconnect_arrived.is_set(),
            "before": before,
            "after": after,
        }


def run_trial(subject, allocation, workload, batches, scratch, evidence, settings, pair, listener_proxy=None):
    engine = subject.engine
    cluster = subject.cluster
    cluster.prepare_database(engine)
    namespace = f"parity_{secrets.token_hex(8)}"
    env = subject.environment(namespace, allocation[0], sum(allocation),
                              workload["mode"], settings["poll_ms"])
    subject.call("migrate", env, evidence / "migrate.log")
    # The observer table is cluster-wide and retains entries for dropped databases.
    cluster.sql("postgres", "SELECT pg_stat_statements_reset()")
    expected = {job["key"]: (batch["request_id"], job) for batch in batches for job in batch["jobs"]}
    paths = batch_files(scratch, batches)
    case = hashlib.sha256("".join(digest(path) for path in paths).encode()).hexdigest()
    logs = [evidence / f"runner-{index}.log" for index in range(len(allocation))]
    processes = []
    completed = False
    relay = contextlib.ExitStack()
    listener = listener_evidence = None
    try:
        with contextlib.ExitStack() as producer_scope:
            if listener_proxy:
                if not cluster.expected_version or not cluster.owned(engine):
                    raise FixtureError("listener probe requires an owned PostgreSQL cluster with verified TLS")
                proxy = relay.enter_context(listener_proxy(cluster.port))
                status_path = scratch / "wake-status"
                listener = ListenerTrial(cluster, namespace, status_path, proxy, subject.variant == "disconnected")
                env = dict(env, JOBS_PARITY_POOL_URL=cluster.url(engine, port=proxy.port),
                           JOBS_PARITY_STATUS_PATH=str(status_path))
            deadline = time.monotonic() + settings["trial_timeout_seconds"]
            sparse = workload["mode"] == "sparse"
            enqueue_batch = (producer_scope.enter_context(subject.warm_producer(env, evidence, deadline))
                             if sparse else lambda path, log: subject.enqueue_receipt(env, path, log))
            for index, (slots, log) in enumerate(zip(allocation, logs, strict=True)):
                processes.append(subject.run(dict(env, JOBS_PARITY_SLOTS=str(slots)), index, log))
            wait_for(lambda: runner_ready(processes, logs), deadline, "runner readiness")
            if listener:
                listener.arm(min(deadline, time.monotonic() + 15))
                listener_before = listener.observe(deadline)
            idle_start = time.monotonic()
            idle_calls_before = cluster.calls(engine)
            idle_samples = [take_sample(cluster, engine, namespace, processes, idle_start)]
            time.sleep(settings["idle_seconds"])
            idle_calls_after = cluster.calls(engine)
            idle_s = time.monotonic() - idle_start
            idle_samples.append(take_sample(cluster, engine, namespace, processes, idle_start))
            commits = {}
            enqueued_ids = {}
            start = time.monotonic()
            samples = []
            interval = settings["observer_interval_ms"] / 1000

            def settled_count(count):
                if not runner_ready(processes, logs, track=False):
                    raise FixtureError("runner readiness lost during observation")
                sample = take_sample(cluster, engine, namespace, processes, start)
                samples.append(sample)
                return sample["terminal"] >= count

            expected_total = 0
            with ThreadPoolExecutor(max_workers=1) as producer:
                for path, batch in zip(paths, batches, strict=True):
                    request_id = batch["request_id"]
                    enqueue = producer.submit(enqueue_batch, path, evidence / f"enqueue-{request_id}.log")
                    while not enqueue.done() and time.monotonic() < deadline:
                        settled_count(len(expected))
                        time.sleep(interval)
                    receipt, commit_ms = enqueue.result(timeout=max(0.0, deadline - time.monotonic()))
                    if set(receipt["ids"]) != {job["key"] for job in batch["jobs"]} or commit_ms is None:
                        raise QueueMismatch(f"{request_id} did not confirm the expected enqueue transaction")
                    enqueued_ids.update(receipt["ids"])
                    commits[request_id] = commit_ms
                    expected_total += receipt["count"]
                    if sparse:
                        wait_for(lambda: settled_count(expected_total), deadline, request_id, interval)
                        time.sleep(settings["poll_ms"] / 1000)
            wait_for(lambda: settled_count(len(expected)), deadline, "durable settlements", interval)
            after = cluster.calls(engine)
            counter_snapshots = (idle_calls_before, idle_calls_after, after)
            if len({snapshot["dealloc"] for snapshot in counter_snapshots}) != 1:
                raise FixtureError("PostgreSQL evicted statement counters during the trial")
            runner_calls = after["runner"] - idle_calls_after["runner"]
            enqueue_calls = after["enqueue"] - idle_calls_after["enqueue"]
            calls = runner_calls + enqueue_calls
            claim_calls = after["claims"] - idle_calls_after["claims"]
            idle_calls = idle_calls_after["runner"] - idle_calls_before["runner"]
            if listener:
                listener_evidence = listener.verify(listener_before, listener.observe(deadline), len(expected))
            completed = True
    finally:
        try:
            cpu = stop_runners(processes, require_clean=completed)
        finally:
            relay.close()

    observed_peak = max(sample["executing"] for sample in samples + idle_samples)
    if observed_peak > sum(allocation):
        raise QueueMismatch(f"observed {observed_peak} executing jobs above {sum(allocation)} slots")
    rows = account(cluster.rows(engine, namespace), expected, commits, settings["clock_tolerance_ms"])
    writes = cluster.reconcile_trial(engine, namespace, enqueued_ids, dict.fromkeys(expected, (1,)), commits)
    duration = (max(row["settled_ms"] for row in rows) - min(commits.values())) / 1000
    if duration <= 0:
        raise FixtureError("nonpositive committed observation interval")

    queued_context = []
    wakeups = []
    event_drops = []
    event_buffers = []
    event_mode = env.get("JOBS_PARITY_MESH_EVENTS") if engine == "mesh" else None
    if engine == "mesh":
        for log in logs:
            status = mesh_shutdown_report(log)
            if listener and (status.get("last_error") is not None or status.get("claimed") != len(rows)):
                raise FixtureError(f"unexpected runner errors or claim count in {log}")
            peak = status.get("peak_queued_context_bytes")
            if peak is not None and (type(peak) is not int or peak < 0):
                raise FixtureError(f"invalid queued context peak in {log}")
            queued_context.append(peak)
            wakeups.append(status.get("wakeups"))
            drops = {"reader": status.get("events_dropped"), "logger": status.get("event_logs_dropped")}
            if event_mode in ("logged", "backpressured") and any(
                type(count) is not int or count < 0 for count in drops.values()
            ):
                raise FixtureError(f"enabled telemetry did not report nonnegative event drop counters in {log}")
            event_drops.append(drops)
            event_buffers.append(mesh_event_buffer_report(status, event_mode, log))

    listener_enabled = env.get("JOBS_PARITY_MESH_LISTEN") == "1" and engine == "mesh"
    if listener_enabled and not listener and not any(
        report and report.get("notification_scans", 0) > 0 for report in wakeups
    ):
        raise FixtureError("listener trial did not record any notification-triggered scans")

    raw = {"engine": engine, "variant": subject.variant, "namespace": namespace, "pair": pair, "case_sha256": case,
           "producer_mode": "warm process" if sparse else "one process per batch",
           "transactions": commits, "enqueued_ids": enqueued_ids, "attempts": rows, "namespace_writes": writes,
           "samples": samples, "idle_samples": idle_samples,
           "poll_ms": int(env["JOBS_PARITY_POLL_MS"]), "wakeups": wakeups,
           "event_drops": event_drops, "runner_event_buffer_bytes": event_buffers,
           "runner_peak_queued_context_bytes": queued_context,
           "runner_cpu_seconds_including_startup_and_idle": cpu, "raw_sql_calls": calls,
           "statement_counter_deallocations": after["dealloc"],
           "raw_runner_sql_calls": runner_calls, "raw_enqueue_sql_calls": enqueue_calls,
           "raw_claim_calls": claim_calls, "raw_idle_sql_calls": idle_calls, "idle_seconds": idle_s}
    if listener:
        raw["listener_trial"] = listener_evidence
    write_json(evidence / "raw.json", raw)
    return {
        "pair": pair, "engine": subject.variant, "input_sha256": case, "completed": len(rows),
        "poll_ms": int(env["JOBS_PARITY_POLL_MS"]),
        "jobs_per_second": len(rows) / duration,
        "pickup_p99_ms": percentile([row["pickup_ms"] for row in rows], 0.99),
        **({"pickup_max_ms": max(row["pickup_ms"] for row in rows)} if listener else {}),
        "claim_p99_ms": percentile([row["commit_to_claim_ms"] for row in rows], 0.99),
        "settlement_p99_ms": percentile([row["commit_to_settlement_ms"] for row in rows], 0.99),
        "sql_calls_per_job": calls / len(rows),
        "runner_sql_calls_per_job": runner_calls / len(rows),
        "enqueue_sql_calls_per_job": enqueue_calls / len(rows),
        "claim_calls_per_job": claim_calls / len(rows),
        "idle_queries_per_minute": idle_calls * 60 / idle_s,
        "cpu_seconds_per_job": cpu / len(rows),
        "peak_executing": observed_peak,
        "estimated_peak_executing": estimated_peak_executing(rows),
        "peak_queued_context_bytes": max(queued_context) if queued_context and None not in queued_context else None,
        "events_dropped": sum(counts["reader"] or 0 for counts in event_drops),
        "event_logs_dropped": sum(counts["logger"] or 0 for counts in event_drops),
        "event_buffer_bytes": max_event_buffer_bytes(event_buffers),
        "peak_host_load_average_1m": max(
            sample["host_load_average_1m"] for sample in samples + idle_samples),
        **{f"peak_{key}": max(sample[key] for sample in samples + idle_samples)
           for key in ("connections", "rss_bytes", "processes", "threads")},
    }


def mesh_reports(log):
    for line in log.read_text().splitlines():
        try:
            status = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(status, dict):
            yield status


def mesh_shutdown_report(log):
    for status in reversed(list(mesh_reports(log))):
        if status.get("state") == "stopped" and status.get("drained") is True:
            return status
    raise FixtureError(f"missing drained Mesh shutdown report in {log}")


def native_worker_starts(log):
    prefix = b"PARITY_WORKER_START "
    with log.open("rb") as stream:
        return [int(line[len(prefix):]) for line in stream if line.startswith(prefix)]


def mesh_event_buffer_report(status, mode, log):
    if "event_buffer_bytes" not in status:
        return None
    report = status["event_buffer_bytes"]
    expected_sink = {"logged": "logger", "backpressured": "reader"}.get(mode)
    if type(report) is not dict or set(report) != set(EVENT_BUFFER_SINKS) or expected_sink is None:
        raise FixtureError(f"invalid event buffer report in {log}")
    for sink, values in report.items():
        if sink != expected_sink and values is None:
            continue
        if (sink != expected_sink or type(values) is not dict or set(values) != set(EVENT_BUFFER_METRICS)
                or any(type(value) is not int for value in values.values())):
            raise FixtureError(f"invalid {sink} event buffer bytes in {log}")
        sampled, bound = (values[key] for key in EVENT_BUFFER_METRICS)
        if not (0 <= sampled <= bound and bound > 0):
            raise FixtureError(f"invalid {sink} event buffer bytes in {log}")
        if sink == "reader" and status.get("events_dropped", 0) > 0 and sampled != bound:
            raise FixtureError(f"backpressured reader dropped events without reporting a full buffer in {log}")
    return report


def max_event_buffer_bytes(reports):
    if not reports or any(report is None for report in reports):
        return None
    return {
        sink: {key: max(report[sink][key] for report in reports) for key in EVENT_BUFFER_METRICS}
        if all(report[sink] is not None for report in reports) else None
        for sink in EVENT_BUFFER_SINKS
    }


def run_fault(subject, scratch, evidence, settings):
    engine = subject.engine
    cluster = subject.cluster
    cluster.prepare_database(engine)
    namespace = f"parity_{secrets.token_hex(8)}"
    env = subject.environment(namespace, 1, 1, "fault", settings["poll_ms"])
    subject.call("migrate", env, evidence / "migrate.log")
    if not cluster.owned(engine):
        raise FixtureError("fault injection requires the driver's nonce in a fresh database")

    def enqueue(batch_id, rollback, fail_first, hold_ms):
        batch = {"request_id": batch_id, "rollback": rollback, "jobs": [{
            "key": batch_id, "payload": "synthetic", "hold_ms": hold_ms, "fail_first": fail_first}]}
        path = batch_files(scratch, [batch])[0]
        receipt, committed = subject.enqueue_receipt(env, path, evidence / f"{batch_id}.log")
        if set(receipt["ids"]) != {batch_id}:
            raise QueueMismatch(f"unexpected enqueue receipt for {batch_id}")
        return receipt["ids"], committed

    rolled_back, rollback_timestamp = enqueue("rolled_back", True, False, 0)
    if rollback_timestamp is not None:
        raise QueueMismatch("rollback xid had a commit timestamp")
    process = subject.run(env, 0, evidence / "initial-runner.log")
    replacement = None
    completed = False
    try:
        deadline = time.monotonic() + settings["trial_timeout_seconds"]
        wait_for(lambda: runner_ready([process], [evidence / "initial-runner.log"]),
                 deadline, "fault runner readiness")
        enqueued, committed = enqueue("interrupted", False, True, 30_000)
        if committed is None:
            raise QueueMismatch("fault fixture enqueue did not commit")

        def barrier():
            relation, scope = job_relation(engine, namespace)
            where = "WHERE " + scope
            result = json.loads(cluster.sql(engine, f"""
SELECT json_build_object(
 'claim', (SELECT state FROM {relation} {where} AND args->>'key'='interrupted'),
 'effects', (SELECT count(*) FROM public.mesh_jobs_parity_effects
             WHERE engine = '{engine}' AND namespace = '{namespace}')
)
"""))
            return result if result == {"claim": "executing", "effects": 1} else None

        before = wait_for(barrier, deadline, "committed effect on an active claim")
        if process.poll() is not None or not cluster.owned(engine):
            raise FixtureError("fault runner or database authority changed before injection")
        claim = cluster.claim_identity(engine, namespace, "interrupted")
        if claim is None or claim["state"] != "executing" or claim["attempt"] != 1 or not claim["fence"]:
            raise FixtureError("missing original claim identity at fault barrier")
        if claim["id"] != enqueued["interrupted"]:
            raise QueueMismatch("the original claim did not use the enqueue ID")
        remember_workers([process], process_table())
        os.kill(process.pid, signal.SIGKILL)
        process.wait(timeout=10)
        if process.stdin is not None:
            process.stdin.close()
        wait_for_worker_exit(process.observed_workers, fixture_executables([process]))
        gate = scratch / "replacement-gate"
        replacement = subject.run(dict(env, JOBS_PARITY_RETRY_GATE=str(gate)), 1,
                                  evidence / "replacement-runner.log")
        wait_for(lambda: runner_ready([replacement], [evidence / "replacement-runner.log"]),
                 deadline, "replacement runner readiness")

        def reclaimed():
            value = cluster.claim_identity(engine, namespace, "interrupted")
            return value if value and value["state"] == "executing" and value["attempt"] == 2 else None

        recovered = wait_for(reclaimed, deadline, "active replacement claim")
        if recovered["id"] != claim["id"] or not recovered["fence"] or recovered["fence"] == claim["fence"]:
            raise QueueMismatch("recovery changed job identity or reused its old claim fence")
        stale_ack_rejected = None
        if engine == "mesh":
            if not all(re.fullmatch(r"[0-9a-f-]{36}", claim[field]) for field in ("id", "fence")):
                raise FixtureError("invalid Mesh claim values at fault barrier")
            applied = cluster.sql(engine, f"""
SELECT mesh_jobs.settle('{namespace}', '{claim['id']}', '{claim['fence']}',
    '{{"action":"complete"}}'::jsonb, false)
""")
            stale_ack_rejected = applied == "f" and reclaimed() == recovered
            if not stale_ack_rejected:
                raise QueueMismatch("the original Mesh token could change the active replacement claim")
        gate.touch()

        wait_for(lambda: cluster.observe(engine, namespace)["terminal"] == 1,
                 deadline, "recovered settlement")
        completed = True
    finally:
        if replacement is not None:
            gate.touch(exist_ok=True)
            stop_runners([replacement], require_clean=completed)
        stop_runners([process], require_clean=False)

    rows = cluster.rows(engine, namespace)
    if len(rows) != 1 or rows[0]["state"] != "completed" or rows[0]["attempt"] != 2:
        raise QueueMismatch(f"recovery did not reconcile: {rows}")
    writes = cluster.reconcile_trial(engine, namespace, enqueued, {"interrupted": (1, 2)}, ("interrupted",))
    receipt = {
        "engine": engine, "before_signal": before, "after_recovery": rows[0],
        "fault": "runner SIGKILL after committed effect and before settlement",
        "enqueue_commit_ms": committed, "rollback_commit_ms": rollback_timestamp,
        "enqueued_ids": enqueued, "rolled_back_ids": rolled_back,
        "stale_ack_rejected": stale_ack_rejected,
        "job_id_preserved": True, "replacement_fence_changed": True, "namespace_writes": writes,
    }
    write_json(evidence / "fault.json", receipt)
    return receipt


def run_settlement_fault(subject, scratch, evidence, settings, placement, proxy_type):
    engine = subject.engine
    cluster = subject.cluster
    cluster.prepare_database(engine)
    namespace = f"parity_{secrets.token_hex(8)}"
    env = subject.environment(namespace, 1, 1, "fault", settings["poll_ms"])
    subject.call("migrate", env, evidence / "migrate.log")
    if not cluster.owned(engine):
        raise FixtureError("settlement fault requires the driver's nonce in a fresh database")
    if cluster.expected_version:
        raise FixtureError("settlement fault requires the driver's SCRAM-only loopback mode")

    def matches(sql):
        if engine == "mesh":
            return "coalesce(mesh_jobs.settle_state(" in sql or sql.startswith("SELECT mesh_jobs.settle(")
        return sql.startswith("UPDATE ") and f'"{namespace}"."oban_jobs"' in sql and '"completed_at"' in sql

    input_file = batch_files(scratch, [{"request_id": "settlement", "rollback": False, "jobs": [
        {"key": "settlement", "hold_ms": 200, "fail_first": False, "payload": "synthetic"}]}])[0]
    completed = False
    log = evidence / "runner.log"
    inspect_events = engine == "mesh" and env.get("JOBS_PARITY_MESH_EVENTS") == "logged"

    def events_for_job():
        return [report["event"] for report in mesh_reports(log)
                if ("event" in report and isinstance(report.get("attempt"), dict)
                    and report["attempt"].get("job_id") == enqueued["settlement"])]

    with proxy_type(cluster.port, matches, placement) as proxy:
        process = subject.run(dict(env, JOBS_PARITY_DB_URL=cluster.url(engine, port=proxy.port)), 0, log)
        try:
            deadline = time.monotonic() + settings["trial_timeout_seconds"]
            wait_for(lambda: runner_ready([process], [log]), deadline, "settlement-fault runner")
            receipt, committed = subject.enqueue_receipt(env, input_file, evidence / "enqueue.log")
            enqueued = receipt["ids"]
            if committed is None or set(enqueued) != {"settlement"}:
                raise QueueMismatch("settlement-fault enqueue did not commit")
            fault = wait_for(lambda: proxy.fault, deadline, f"{placement} transport cut")
            wait_for(lambda: cluster.observe(engine, namespace)["terminal"] == 1,
                     deadline, f"settlement following {placement} cut")
            if inspect_events:
                expected = {"ack_uncertain"}
                if placement == "before_commit":
                    expected.add("settlement_committed")
                wait_for(lambda: expected.issubset(events_for_job()), deadline,
                         f"expected acknowledgement events for {placement}")
            completed = True
        finally:
            stop_runners([process], require_clean=completed)
    rows = cluster.rows(engine, namespace)
    accepted = {1} if placement == "after_commit" else {1, 2}
    if len(rows) != 1 or rows[0]["state"] != "completed" or rows[0]["attempt"] not in accepted:
        raise QueueMismatch(f"unexpected durable rows after {placement}: {rows}")
    attempts = {"settlement": range(1, rows[0]["attempt"] + 1)}
    writes = cluster.reconcile_trial(engine, namespace, enqueued, attempts, ("settlement",))
    receipt = {"engine": engine, "fault": fault, "rows": rows, "enqueued_ids": enqueued, "namespace_writes": writes}
    if inspect_events:
        events = events_for_job()
        confirmed = events.count("settlement_committed")
        if confirmed != int(placement == "before_commit"):
            raise QueueMismatch(f"ambiguous settlement produced {confirmed} commit confirmations: {placement}")
        receipt["job_events"] = events
    write_json(evidence / "fault.json", receipt)


def run_producer_commit_fault(subject, scratch, evidence, settings, placement, proxy_type):
    cluster = subject.cluster
    cluster.prepare_database("mesh")
    namespace = f"parity_{secrets.token_hex(8)}"
    env = subject.environment(namespace, 1, 1, "fault", settings["poll_ms"])
    subject.call("migrate", env, evidence / "migrate.log")
    if cluster.expected_version or not cluster.owned("mesh"):
        raise FixtureError("producer commit fault requires a fresh owned SCRAM-only database")

    keys = ("producer_first", "producer_second")
    request = "producer_commit"
    input_file = batch_files(scratch, [{"request_id": request, "rollback": False, "jobs": [
        {"key": key, "hold_ms": 0, "fail_first": False, "payload": "synthetic"} for key in keys]}])[0]

    def producer_state():
        return json.loads(cluster.sql("mesh", """
SELECT json_build_object(
 'business', (SELECT coalesce(json_agg(json_build_array(engine, namespace, request_id, xmin::text,
                                                       pg_xact_commit_timestamp(xmin))), '[]')
              FROM public.mesh_jobs_parity_product),
 'jobs', (SELECT coalesce(json_agg(json_build_array(namespace, args->>'key', id::text, xmin::text,
                                                   pg_xact_commit_timestamp(xmin))
                                ORDER BY args->>'key'), '[]') FROM mesh_jobs.jobs),
 'receipts', (SELECT coalesce(json_agg(json_build_array(namespace, request_key, receipt, xmin::text,
                                                       pg_xact_commit_timestamp(xmin))
                                    ORDER BY request_key), '[]') FROM mesh_jobs.receipts)
)
"""))

    def reconcile(state):
        ids = {key: receipt["id"] for _, key, receipt, _, _ in state["receipts"]}
        if (set(ids) != set(keys) or len(state["receipts"]) != len(keys)
                or len(state["business"]) != 1
                or state["business"][0][:3] != ["mesh", namespace, request]):
            raise QueueMismatch("producer transaction did not persist one business row and both receipts")
        # Enqueue uses a savepoint. Its tuple XIDs can differ from the business row.
        committed_at = state["business"][0][4]
        if committed_at is None or any(
            ns != namespace or receipt["effect_key"] != f"{ids[key]}/1" or tick != committed_at
            for ns, key, receipt, _, tick in state["receipts"]
        ):
            raise QueueMismatch("enqueue receipt identities or commit times differ from the producer transaction")
        jobs = [[ns, key, identity, tick] for ns, key, identity, _, tick in state["jobs"]]
        if jobs != [[namespace, key, ids[key], committed_at] for key in sorted(keys)]:
            raise QueueMismatch("enqueue job identities or commit times differ from the producer transaction")
        cluster.reconcile_trial("mesh", namespace, ids, {}, (request,))
        return ids

    with proxy_type(
        cluster.port, lambda sql: sql.strip().upper() == "COMMIT", placement,
        prior_query=lambda sql: sql == "SELECT pg_current_xact_id()::text AS xid",
    ) as proxy:
        result = subprocess.run(
            [*subject.command, "enqueue"], cwd=subject.cwd, text=True, capture_output=True,
            env=dict(env, JOBS_PARITY_INPUT=str(input_file),
                     JOBS_PARITY_DB_URL=cluster.url("mesh", "parity_producer", proxy.port)),
            timeout=settings["trial_timeout_seconds"],
        )
        log = evidence / "lost-reply.log"
        log.write_text(result.stdout + result.stderr)
        if result.returncode != 1 or result.stdout.strip() or not result.stderr.startswith("parity error:"):
            raise FixtureError(f"producer did not return an error without a receipt; see {log}")
        fault = wait_for(lambda: proxy.fault, time.monotonic() + 5, f"{placement} producer commit cut")

    first = producer_state()
    if placement == "before_commit":
        if any(first.values()):
            raise QueueMismatch("producer transaction was visible despite the pre-commit cut")
        cluster.reconcile_trial("mesh", namespace, {}, {}, ())
    else:
        reconcile(first)

    receipt, committed = subject.enqueue_receipt(env, input_file, evidence / "replay.log")
    replayed = producer_state()
    ids = reconcile(replayed)
    if committed is None or receipt["ids"] != ids:
        raise QueueMismatch("producer replay did not return the durable job identities")
    if placement != "before_commit" and replayed != first:
        raise QueueMismatch("producer replay changed a committed business write or enqueue receipt")
    if placement == "before_commit" and replayed["business"][0][3] != receipt["xid"]:
        raise QueueMismatch("the replay did not create the transaction after the rolled-back attempt")

    log = evidence / "runner.log"
    process = subject.run(env, 0, log)
    completed = False
    try:
        deadline = time.monotonic() + settings["trial_timeout_seconds"]
        wait_for(lambda: runner_ready([process], [log]), deadline, "producer replay runner")
        wait_for(lambda: cluster.observe("mesh", namespace)["terminal"] == len(keys),
                 deadline, "settlement of replayed jobs")
        completed = True
    finally:
        stop_runners([process], require_clean=completed)
    rows = cluster.rows("mesh", namespace)
    if any(row["state"] != "completed" or row["attempt"] != 1 for row in rows):
        raise QueueMismatch(f"replayed producer jobs did not complete at attempt one: {rows}")
    writes = cluster.reconcile_trial("mesh", namespace, ids, dict.fromkeys(keys, (1,)), (request,))
    write_json(evidence / "fault.json", {
        "engine": "mesh", "fault": fault, "producer_returncode": result.returncode,
        "input_sha256": digest(input_file), "after_cut": first, "after_replay": replayed,
        "replay_receipt": receipt, "rows": rows, "namespace_writes": writes,
    })


def run_batch_claim_fault(subject, scratch, evidence, settings, placement, proxy_type, *, final_attempt=False):
    cluster = subject.cluster
    if cluster.expected_version:
        raise FixtureError("batch claim fault requires the driver's SCRAM-only loopback mode")
    cluster.prepare_database("mesh")
    namespace = f"parity_{secrets.token_hex(8)}"
    env = subject.environment(namespace, 1, 1, "fault", settings["poll_ms"])
    if any(env.get(key) != "2" for key in ("MESH_SCHEDULER_MIN_WORKERS", "MESH_SCHEDULER_MAX_WORKERS")):
        raise FixtureError("batch claim startup marker requires a 2/2 coordinator scheduler")
    subject.call("migrate", env, evidence / "migrate.log")
    if not cluster.owned("mesh"):
        raise FixtureError("batch claim fault requires the driver's nonce in a fresh database")
    input_file = batch_files(scratch, [{"request_id": "claim", "rollback": False, "jobs": [
        {"key": "claim", "hold_ms": 0, "fail_first": False, "payload": "synthetic"}]}])[0]
    producer_env = dict(env, JOBS_PARITY_MODE="final_attempt") if final_attempt else env
    receipt, committed = subject.enqueue_receipt(producer_env, input_file, evidence / "enqueue.log")
    enqueued = receipt["ids"]
    if set(enqueued) != {"claim"} or committed is None:
        raise QueueMismatch("batch claim fault requires one committed job")
    before = cluster.claim_identity("mesh", namespace, "claim")
    if (not before or before["id"] != enqueued["claim"] or before["attempt"] != 0
            or before["fence"] is not None):
        raise QueueMismatch(f"fault input has an unexpected initial state: {before}")

    log = evidence / "runner.log"
    completed = False
    with proxy_type(
        cluster.port, lambda sql: sql.strip().upper() == "COMMIT", placement,
        prior_query=lambda sql: "FROM mesh_jobs.claim_batch_v1(" in sql,
    ) as proxy:
        process = subject.run(dict(env, JOBS_PARITY_DB_URL=cluster.url("mesh", port=proxy.port),
                                   JOBS_PARITY_MARK_WORKER_START="1"), 0, log)
        try:
            deadline = time.monotonic() + settings["trial_timeout_seconds"]
            wait_for(lambda: runner_ready([process], [log]), deadline, "batch claim fault runner")
            fault = wait_for(lambda: proxy.fault, deadline, f"{placement} batch commit cut")
            wait_for(lambda: cluster.observe("mesh", namespace)["terminal"] == 1,
                     deadline, "settlement following an ambiguous claim")
            completed = True
        finally:
            stop_runners([process], require_clean=completed)
    discarded = final_attempt and placement != "before_commit"
    expected_attempt = 1 if final_attempt or placement == "before_commit" else 2
    after = cluster.claim_identity("mesh", namespace, "claim")
    if after != dict(before, state="discarded" if discarded else "completed", attempt=expected_attempt):
        raise QueueMismatch(f"unexpected job identity or lease fence after {placement}: {after}")
    rows = cluster.rows("mesh", namespace)
    handler_attempts = {"claim": () if discarded else (expected_attempt,)}
    writes = cluster.reconcile_trial("mesh", namespace, enqueued, handler_attempts, ("claim",))
    workers = cluster.worker_pids(namespace)
    if len(workers) != int(not discarded) or process.pid in workers or native_worker_starts(log) != workers:
        raise QueueMismatch(f"unexpected native worker starts after {placement}; see {log}")
    result = {
        "engine": "mesh", "fault": fault, "rows": rows, "namespace_writes": writes,
        "enqueued_ids": enqueued, "worker_start_pids": workers,
    }
    if discarded:
        lease = json.loads(cluster.sql("mesh", """
SELECT json_build_object('max_attempts', max_attempts, 'elapsed_ms', finished_at_ms - attempted_at_ms,
                         'lease_expires_at_ms', lease_expires_at_ms, 'errors', errors)
FROM mesh_jobs.jobs
"""))
        if (lease["max_attempts"] != 1 or lease["elapsed_ms"] < 1500 or lease["lease_expires_at_ms"] is not None
                or [(error["attempt"], error["reason"]) for error in lease["errors"]] != [(1, "lease expired")]):
            raise QueueMismatch(f"the final claim did not discard through lease recovery: {lease}")
        result["lease_recovery"] = lease
    write_json(evidence / "fault.json", result)


def startup_case(subject, scratch, evidence, keys, *, mode):
    cluster = subject.cluster
    cluster.prepare_database("mesh")
    namespace = f"parity_{secrets.token_hex(8)}"
    env = subject.environment(namespace, len(keys), len(keys), mode, subject.poll_ms)
    env["JOBS_PARITY_MARK_WORKER_START"] = "1"
    if any(env.get(key) != "2" for key in ("MESH_SCHEDULER_MIN_WORKERS", "MESH_SCHEDULER_MAX_WORKERS")):
        raise FixtureError("the startup marker requires a 2/2 coordinator scheduler")
    subject.call("migrate", env, evidence / f"{mode}-migrate.log")
    if not cluster.owned("mesh"):
        raise FixtureError("startup fault requires the driver's nonce in a fresh database")
    path = batch_files(scratch, [{"request_id": mode, "rollback": False, "jobs": [
        {"key": key, "hold_ms": 0, "fail_first": False, "payload": "synthetic"} for key in keys]}])[0]
    receipt, commit = subject.enqueue_receipt(env, path, evidence / f"{mode}-enqueue.log")
    if commit is None or set(receipt["ids"]) != set(keys):
        raise QueueMismatch(f"{mode} startup fault enqueue did not commit all jobs")
    return namespace, env, receipt["ids"]


def held_native_worker(process, log):
    starts = native_worker_starts(log)
    if len(starts) > 1:
        raise QueueMismatch(f"duplicate native startup at the gate: {starts}; see {log}")
    if process.poll() is not None:
        raise FixtureError(f"the runner exited before the startup barrier; see {log}")
    table = process_table()
    remember_workers([process], table)
    return starts[0] if starts and starts[0] in table and table[starts[0]][0] == process.pid else None


def check_startup_claims(claims, enqueued, attempt):
    if set(claims) != set(enqueued) or any(
        row["id"] != enqueued[key] or row["state"] != "executing"
        or row["attempt"] != attempt or not row["fence"]
        for key, row in claims.items()
    ) or len({row["fence"] for row in claims.values()}) != len(enqueued):
        raise QueueMismatch(f"unexpected claims at startup barrier: {claims}")


def run_confirmed_launch_failure(subject, scratch, evidence):
    cluster = subject.cluster
    keys = ("started", "denied", "skipped")
    namespace, env, enqueued = startup_case(subject, scratch, evidence, keys, mode="dense")
    # Distinct due times make the launch order independent of the enqueue clock.
    cluster.sql("mesh", f"""
UPDATE mesh_jobs.jobs SET scheduled_at_ms = CASE args->>'key'
    WHEN 'started' THEN 1 WHEN 'denied' THEN 2 ELSE 3 END
WHERE namespace = '{namespace}'
""")
    executable = Path(subject.command[0])
    mode = stat.S_IMODE(executable.stat().st_mode)
    spawn_gate, worker_gate = scratch / "second-spawn", scratch / "first-worker"
    log = evidence / "confirmed-runner.log"
    runner = subject.run(dict(env, JOBS_PARITY_SECOND_SPAWN_GATE=str(spawn_gate),
                              JOBS_PARITY_WORKER_START_GATE=str(worker_gate)), 0, log)
    stopped = False
    try:
        deadline = time.monotonic() + 12
        wait_for(lambda: b"PARITY_SECOND_SPAWN_WAIT" in log.read_bytes().splitlines(),
                 deadline, "the second OS spawn call at its gate")
        worker = wait_for(lambda: held_native_worker(runner, log), deadline, "the first native worker")
        batch = json.loads(cluster.sql("mesh", f"""
SELECT json_object_agg(args->>'key', json_build_object(
    'id', id::text, 'state', state, 'attempt', attempt, 'fence', lease_token::text, 'xid', xmin::text))
FROM mesh_jobs.jobs WHERE namespace = '{namespace}'
"""))
        check_startup_claims(batch or {}, enqueued, 1)
        if len({row["xid"] for row in batch.values()}) != 1:
            raise QueueMismatch(f"the three claims did not commit in one transaction: {batch}")
        cluster.reconcile_writes("mesh", namespace, {"attempts": {}, "effects": {}})
        executable.chmod(mode & ~0o111)
        spawn_gate.touch()
        denial = f"PARITY_SPAWN_RESULT 2 {errno.EACCES}".encode()
        wait_for(lambda: denial in log.read_bytes().splitlines(), deadline, "libc execute permission denial")
        executable.chmod(mode)
        worker_gate.touch()
        wait_for(lambda: any(event.get("event") == "runner_stopped" for event in mesh_reports(log)),
                 deadline, "runner drain after failed launch")
        stopped = True
    finally:
        try:
            executable.chmod(mode)
            spawn_gate.touch(exist_ok=True)
            worker_gate.touch(exist_ok=True)
        finally:
            stop_runners([runner], require_clean=stopped)

    spawn_results = [line for line in log.read_bytes().splitlines() if line.startswith(b"PARITY_SPAWN_RESULT ")]
    if spawn_results != [b"PARITY_SPAWN_RESULT 1 0", denial] or native_worker_starts(log) != [worker]:
        raise QueueMismatch(f"expected one native start and exactly two OS spawn calls; see {log}")
    report = mesh_shutdown_report(log)
    queue = report["queues"]["default"]
    if (report["claimed"] != 3 or report["outcomes"]["released_unstarted"] != 2
            or report["outcomes"]["completed"] != 1 or report["stale_acks"] != 0
            or report["pending_claims"] != 0 or queue["pending_claims"] != 0
            or queue["in_flight"] != 0 or not queue["drained"]
            or "could not start job handler process" not in (report["last_error"] or "")):
        raise QueueMismatch(f"unexpected launch-failure shutdown report: {report}")
    refunds = [event for event in mesh_reports(log)
               if event.get("event") == "settlement_committed" and event.get("settlement") == "released_unstarted"]
    expected_refunds = [(enqueued["denied"], 1, "worker_failure"), (enqueued["skipped"], 1, "shutdown")]
    if sorted((event["attempt"]["job_id"], event["attempt"]["attempt_number"], event["failure_kind"])
              for event in refunds) != sorted(expected_refunds):
        raise QueueMismatch(f"unexpected jobs or causes in the unstarted settlements: {refunds}")
    after = {key: cluster.claim_identity("mesh", namespace, key) for key in keys}
    expected = {key: dict(id=enqueued[key], state="available", attempt=0, fence=None) for key in keys}
    expected["started"].update(state="completed", attempt=1)
    if after != expected or cluster.worker_pids(namespace) != [worker]:
        raise QueueMismatch(f"only the denied and skipped claims should refund: {after}")
    fault_writes = cluster.reconcile_writes(
        "mesh", namespace, {"attempts": {"started": 1}, "effects": {"started": 1}})

    recovery_log = evidence / "confirmed-recovery.log"
    recovery = subject.run(env, 1, recovery_log)
    completed = False
    try:
        wait_for(lambda: cluster.observe("mesh", namespace)["terminal"] == 3,
                 time.monotonic() + 15, "refunded jobs to settle with returned capacity")
        completed = True
    finally:
        stop_runners([recovery], require_clean=completed)
    finished = {key: cluster.claim_identity("mesh", namespace, key) for key in keys}
    if finished != {key: dict(row, state="completed", attempt=1) for key, row in expected.items()}:
        raise QueueMismatch(f"launch recovery changed job identity or consumed refunded attempts: {finished}")
    starts = native_worker_starts(recovery_log)
    if (len(starts) != 2 or recovery.pid in starts
            or sorted([worker, *starts]) != sorted(cluster.worker_pids(namespace))):
        raise QueueMismatch(f"unexpected native starts during launch recovery: {starts}")
    writes = cluster.reconcile_trial("mesh", namespace, enqueued, {key: (1,) for key in keys}, ("dense",))
    return {"committed_batch": batch, "first_runner": report, "refund_events": refunds,
            "after_refund": after, "fault_writes": fault_writes,
            "after_recovery": finished, "writes": writes, "worker_start_pids": [worker, *starts],
            "runner_pids": [runner.pid, recovery.pid],
            "spawn_results": [[1, 0], [2, errno.EACCES]]}


def run_ambiguous_startup(subject, scratch, evidence):
    cluster = subject.cluster
    namespace, env, enqueued = startup_case(subject, scratch, evidence, ("uncertain",), mode="fault")

    def claim():
        return cluster.claim_identity("mesh", namespace, "uncertain")

    no_writes = {"attempts": {}, "effects": {}}
    gate = scratch / "uncertain-worker"
    log = evidence / "ambiguous-runner.log"
    runner = subject.run(dict(env, JOBS_PARITY_WORKER_START_GATE=str(gate)), 0, log)
    try:
        deadline = time.monotonic() + 12
        old_pid = wait_for(lambda: held_native_worker(runner, log), deadline, "old child at its constructor gate")
        old = claim()
        check_startup_claims({"uncertain": old} if old else {}, enqueued, 1)
        cluster.reconcile_writes("mesh", namespace, no_writes)
        runner.kill()
        if runner.wait(timeout=5) != -signal.SIGKILL:
            raise FixtureError("the startup runner did not die from SIGKILL")
        after_death = claim()
        if after_death != old:
            raise QueueMismatch(f"runner death changed the uncertain claim: {after_death}")
        # Keep the old image before runtime entry until its durable lease ends.
        # It then proceeds through the real bootstrap and must exit without a handler write.
        lease_expiry = wait_for(lambda: cluster.sql("mesh", f"""
WITH observation AS MATERIALIZED (SELECT mesh_jobs.now_ms() AS now_ms)
SELECT json_build_object('expires_at_ms', lease_expires_at_ms, 'observed_at_ms', now_ms)
FROM mesh_jobs.jobs, observation WHERE namespace = '{namespace}' AND id = '{old["id"]}'
  AND lease_token = '{old["fence"]}' AND lease_expires_at_ms <= now_ms
"""), deadline, "the interrupted claim's original lease to expire")
        if process_table().get(old_pid, (None, None, None))[2] != runner.observed_workers[old_pid]:
            raise FixtureError("the constructor gate did not hold the original worker through lease expiry")
        if claim() != old:
            raise QueueMismatch("the expired startup claim changed before any recovery runner started")
    finally:
        try:
            gate.touch(exist_ok=True)
        finally:
            stop_runners([runner], require_clean=False)
    if native_worker_starts(log) != [old_pid] or b"PARITY_WORKER_START_GATE_TIMEOUT" in log.read_bytes().splitlines():
        raise QueueMismatch(f"unexpected native worker startup or gate timeout; see {log}")
    fault_writes = cluster.reconcile_writes("mesh", namespace, no_writes)

    gate = scratch / "recovered-worker"
    log = evidence / "ambiguous-recovery.log"
    recovery = subject.run(dict(env, JOBS_PARITY_WORKER_START_GATE=str(gate)), 1, log)
    completed = False
    try:
        deadline = time.monotonic() + 12
        new_pid = wait_for(lambda: held_native_worker(recovery, log), deadline,
                           "replacement child at its constructor gate")
        fresh = claim()
        check_startup_claims({"uncertain": fresh} if fresh else {}, enqueued, 2)
        if fresh["fence"] == old["fence"]:
            raise QueueMismatch("the replacement claim reused the interrupted token")
        stale = cluster.sql("mesh", f"""
SELECT coalesce(mesh_jobs.settle_state('{namespace}', '{old["id"]}', '{old["fence"]}',
    '{{"action":"complete"}}'::jsonb, false), 'stale')
""")
        if stale != "stale" or claim() != fresh:
            raise QueueMismatch("a stale settle changed the live replacement claim")
        cluster.reconcile_writes("mesh", namespace, no_writes)
        gate.touch()
        wait_for(lambda: cluster.observe("mesh", namespace)["terminal"] == 1,
                 deadline, "recovered startup settlement")
        completed = True
    finally:
        try:
            gate.touch(exist_ok=True)
        finally:
            stop_runners([recovery], require_clean=completed)
    finished = claim()
    if finished != dict(fresh, state="completed", fence=None):
        raise QueueMismatch(f"unexpected final startup recovery claim: {finished}")
    errors = json.loads(cluster.sql("mesh", f"""
SELECT errors FROM mesh_jobs.jobs WHERE namespace = '{namespace}' AND id = '{old["id"]}'
"""))
    if ([(error["attempt"], error["reason"]) for error in errors] != [(1, "lease expired")]
            or native_worker_starts(log) != [new_pid] or cluster.worker_pids(namespace) != [new_pid]):
        raise QueueMismatch(f"the interrupted startup did not recover exactly once by lease; see {log}")
    writes = cluster.reconcile_trial("mesh", namespace, enqueued, {"uncertain": (2,)}, ("fault",))
    return {"interrupted_claim": old, "after_runner_death": after_death, "runner_exit": runner.returncode,
            "lease_expiry": json.loads(lease_expiry), "runner_pids": [runner.pid, recovery.pid],
            "fault_writes": fault_writes, "replacement_claim": fresh, "stale_settle": stale,
            "after_recovery": finished, "errors": errors, "writes": writes, "worker_start_pids": [old_pid, new_pid]}


def materialize_commit(ref, path, archive, paths=()):
    path.mkdir()
    with archive.open("wb") as stream:
        subprocess.run(["git", "archive", ref, *paths], cwd=ROOT, stdout=stream, check=True)
    checked(["tar", "-xf", str(archive), "-C", str(path)])
    archive.unlink()


def snapshot_harness(scratch, evidence):
    head = checked(["git", "rev-parse", "HEAD"])
    root = scratch / "harness"
    materialize_commit(head, root, scratch / "harness.tar", HARNESS_PATHS)
    files = {str(path.relative_to(root)): digest(path) for path in root.rglob("*") if path.is_file()}
    for path, checksum in files.items():
        if not (ROOT / path).is_file() or digest(ROOT / path) != checksum:
            raise FixtureError(f"working harness differs from {head}: {path}")
    write_json(evidence / "harness.json", {"sha": head, "sha256": files})
    return root, head, files


def build_mesh(ref, scratch, evidence, env, release, mesh_fixture, toolchains, *, startup_faults=False):
    source = scratch / "mesh-source"
    materialize_commit(ref, source, scratch / "mesh-source.tar")
    profile = "release" if release else "debug"
    cargo_target = (toolchains or EVIDENCE / "build") / ref
    build = cargo_target / profile
    rustc = checked(["rustc", "-vV"], env=env)
    rust_target = next(line.removeprefix("host: ") for line in rustc.splitlines()
                       if line.startswith("host: "))
    pin = build / "jobs-parity-toolchain.json"
    if toolchains is None:
        options = ["--release"] if release else []
        checked(["cargo", "build", "--locked", "-p", "meshc", "-p", "mesh-rt", *options],
                cwd=source, env=dict(env, CARGO_TARGET_DIR=str(cargo_target)), timeout=2400,
                log=evidence / "meshc-build.log")
    actual = {"mesh_sha": ref, "profile": profile, "rustc": rustc,
              "files_sha256": {name: digest(build / name) for name in ("meshc", "libmesh_rt.a")}}
    if toolchains is not None:
        try:
            recorded = json.loads(pin.read_text())
        except (OSError, ValueError) as error:
            raise FixtureError(f"cannot read prebuilt toolchain pin: {pin}") from error
        if recorded != actual:
            raise FixtureError(f"prebuilt toolchain differs from its source and artifact pin: {pin}")
    else:
        with tempfile.NamedTemporaryFile(mode="w", dir=build, delete=False) as manifest:
            json.dump(actual, manifest, indent=2)
        Path(manifest.name).replace(pin)
    write_json(evidence / "mesh-toolchain.json", actual)
    project = scratch / "mesh-workload"
    shutil.copytree(mesh_fixture, project)
    native_flags = ["-DMESH_PARITY_J2_LANE_7"] if startup_faults else []
    checked(["cc", "-std=c11", "-O2", *native_flags, "-c", "worker_identity.c", "-o", "worker_identity.o"],
            cwd=project, env=env, log=evidence / "mesh-native-compile.log")
    checked(["ar", "rcs", "libworker_identity.a", "worker_identity.o"], cwd=project, env=env)
    manifest = (project / "mesh.toml.in").read_text().replace("@TARGET@", rust_target)
    (project / "mesh.toml").write_text(manifest.replace("@SHA256@", digest(project / "libworker_identity.a")))
    executable = scratch / "mesh-parity-fixture"
    checked([str(build / "meshc"), "build", str(project), "--target", rust_target,
             "--output", str(executable), "--opt-level", "2" if release else "0"], cwd=source,
            env=dict(env, MESH_RT_LIB_PATH=str(build / "libmesh_rt.a")),
            timeout=180, log=evidence / "mesh-fixture-build.log")
    if any(digest(build / name) != checksum for name, checksum in actual["files_sha256"].items()):
        raise FixtureError("toolchain artifacts changed while compiling the fixture")
    return executable


def compile_oban(scratch, evidence, env, oban_fixture):
    mix_state = EVIDENCE / "build" / "oban"
    mix_state.mkdir(parents=True, exist_ok=True)
    env = dict(env, MIX_ENV="prod", MIX_HOME=str(mix_state / "mix"), HEX_HOME=str(mix_state / "hex"),
               MIX_BUILD_PATH=str(scratch / "oban-build"), MIX_DEPS_PATH=str(scratch / "oban-deps"),
               ERL_CRASH_DUMP=str(scratch / "erl_crash.dump"))
    if not list((mix_state / "mix").glob("archives/hex-*")):
        checked(["mix", "local.hex", "--force"], cwd=oban_fixture, env=env, log=evidence / "hex-bootstrap.log")
    lock_hash = digest(oban_fixture / "mix.lock")
    checked(["mix", "deps.get", "--check-locked"], cwd=oban_fixture, env=env,
            log=evidence / "oban-setup.log")
    checked(["mix", "compile"], cwd=oban_fixture, env=env, timeout=300, log=evidence / "oban-compile.log")
    if digest(oban_fixture / "mix.lock") != lock_hash:
        raise FixtureError("Mix modified the pinned lockfile")
    command = ["mix", "run", "--no-start", "--no-compile", "fixture.exs", "--"]
    try:
        versions = json.loads(checked([*command, "versions"], cwd=oban_fixture, env=env).splitlines()[-1])
    except (ValueError, IndexError) as error:
        raise FixtureError("Oban fixture did not report version metadata") from error
    if versions.get("oban") != "2.24.2":
        raise FixtureError("Oban version differs from pin")
    # Runtime connections use the disposable cluster's literal loopback host.
    # Keep the default resolver available while fetching pinned dependencies.
    env = dict(env, ERL_INETRC=str(oban_fixture / "inetrc"))
    return command, env, {**versions, "oban_resolver": "fixture loopback hosts file"}


def selection(args, settings):
    workloads = args.workload or list(settings["workloads"])
    runners = [args.runners] if args.runners else settings["runner_counts"]
    slots = [args.slots] if args.slots else settings["total_slots"]
    return [(name, r, s) for name in workloads for r in runners for s in slots if s >= r]


def profile_summary(trials, name, workload, settings, runners, slots, variants, mesh_batch, mesh_listen, mesh_events):
    keys = ("jobs_per_second", "pickup_p99_ms", "claim_p99_ms", "settlement_p99_ms",
            "sql_calls_per_job", "runner_sql_calls_per_job",
            "enqueue_sql_calls_per_job", "claim_calls_per_job", "idle_queries_per_minute",
            "cpu_seconds_per_job", "peak_rss_bytes", "events_dropped", "event_logs_dropped",
            "peak_host_load_average_1m", "pickup_max_ms")
    summary = {}
    for engine in variants:
        values = [trial for trial in trials if trial["engine"] == engine]
        summary[engine] = {
            key: {"min": min(series), "median": statistics.median(series), "max": max(series)}
            for key in keys if (series := [trial[key] for trial in values if key in trial])
        }
        if mesh_events:
            summary[engine]["event_buffer_bytes"] = max_event_buffer_bytes(
                [trial.get("event_buffer_bytes") for trial in values])
    limits = {}
    if "oban" in summary:
        for metric, limit in (
                ("cpu_seconds_per_job", settings["relative_cpu_seconds_per_job"]),
                ("peak_rss_bytes", settings["relative_peak_rss"])):
            mesh = summary["mesh"][metric]["median"]
            oban = summary["oban"][metric]["median"]
            limits[metric] = {"mesh_median": mesh, "oban_median": oban, "relative_limit": limit,
                              "within_limit": mesh <= oban * limit}
    limits.update({
        "peak_connections": slots + runners * settings["connection_slots_per_runner"],
        "peak_processes": slots * settings["processes_per_slot"],
        "peak_threads": slots * settings["threads_per_slot"]
                        + runners * settings["threads_per_runner"] + settings["threads_per_host"],
    })
    summary["budgets"] = limits
    for metric in ("peak_connections", "peak_processes", "peak_threads"):
        for engine in variants:
            observed = max(row[metric] for row in trials if row["engine"] == engine)
            summary[engine][metric] = {"observed": observed, "limit": limits[metric],
                                       "within_limit": observed <= limits[metric]}
    if budget := workload.get("pickup_p99_budget_ms"):
        for engine in variants:
            if engine == "disconnected":
                continue
            summary[engine]["pickup_p99_ms"]["budget"] = budget
            summary[engine]["pickup_p99_ms"]["within_budget"] = (
                summary[engine]["pickup_p99_ms"]["max"] <= budget
            )
    if mesh_batch:
        peaks = [row["peak_queued_context_bytes"] for row in trials if row["engine"] == "mesh"]
        observed = max(peaks) if None not in peaks else None
        limit = settings["batch_queued_context_bytes_max"]
        summary["mesh"]["peak_queued_context_bytes"] = {
            "observed": observed, "limit": limit,
            "within_limit": observed is not None and 0 < observed <= limit,
        }
    if "oban" in summary:
        checks = [
            ("jobs_per_second", "oban", "core_throughput_ratio_min"),
            ("pickup_p99_ms", "oban", "core_pickup_p99_ratio_max"),
        ]
    elif mesh_batch and name == "short":
        checks = [("claim_calls_per_job", "baseline", "batch_claim_calls_ratio_max")]
    elif mesh_batch and workload["mode"] == "sparse" and "baseline" in summary:
        checks = [("pickup_p99_ms", "baseline", "batch_sparse_pickup_p99_ratio_max")]
    elif mesh_events == "disabled":
        checks = [("jobs_per_second", "baseline", "events_disabled_throughput_ratio_min"),
                  ("pickup_p99_ms", "baseline", "events_disabled_pickup_ratio_max")]
    elif mesh_events and name == "short":
        checks = [("jobs_per_second", "baseline", "events_enabled_throughput_ratio_min")]
    else:
        checks = []
    comparison = {}
    for metric, reference, rule in checks:
        mesh = summary["mesh"][metric]["median"]
        baseline = summary[reference][metric]["median"]
        limit = settings[rule]
        within_limit = mesh >= baseline * limit if rule.endswith("_min") else mesh <= baseline * limit
        comparison[metric] = {"mesh_median": mesh, "reference": reference, "reference_median": baseline,
                              "ratio_limit": limit, "within_limit": within_limit}
    if mesh_listen and workload["mode"] == "sparse":
        observed = summary["mesh"]["claim_p99_ms"]["max"]
        limit = settings["listener_claim_p99_ms_max"]
        comparison["listener_claim_p99_ms"] = {"observed": observed, "limit": limit,
                                               "within_limit": observed <= limit}
        baseline_polls = {trial["poll_ms"] for trial in trials if trial["engine"] == "baseline"}
        if baseline_polls == {settings["poll_ms"]}:
            metric = "idle_queries_per_minute"
            observed = summary["mesh"][metric]["median"]
            baseline = summary["baseline"][metric]["median"]
            limit = settings["listener_idle_query_ratio_max"]
            comparison[metric] = {"mesh_median": observed, "reference_median": baseline,
                                  "ratio_limit": limit, "within_limit": observed <= baseline * limit}
    if "disconnected" in summary:
        observed = summary["disconnected"]["pickup_max_ms"]["max"]
        limit = 2 * LISTENER_POLL_MS
        comparison["fallback_pickup_max_ms"] = {"observed": observed, "limit": limit,
                                               "within_limit": observed <= limit}
    if comparison:
        summary["comparison"] = comparison
    return summary


@contextlib.contextmanager
def workspace(evidence):
    scratch = Path(tempfile.mkdtemp(prefix="work-", dir=evidence))
    workers_live = False
    try:
        yield scratch
    except RunnerCleanupError as error:
        workers_live = True
        raise RunnerCleanupError(f"{error}. Preserved {scratch} for recovery.") from error
    finally:
        if (scratch / "pgdata/postmaster.pid").is_file():
            raise FixtureError(f"PostgreSQL did not confirm shutdown. Preserved {scratch} for recovery.")
        if not workers_live:
            shutil.rmtree(scratch)


def record(args):
    settings = json.loads(CONFIG.read_text())
    release = args.load or args.listener_outage or args.release
    poll_ms = LISTENER_POLL_MS if args.listener_outage else args.poll_ms or settings["poll_ms"]
    mesh_batch = args.mesh_batch or args.listener_outage
    mesh_listen = args.mesh_listen or args.listener_outage
    mesh_events = "logged" if args.j2_lane_7 else args.mesh_events
    baseline_poll_ms = args.baseline_poll_ms or poll_ms
    profiles = [(args.workload[0] if args.workload else "short", 1, 1)] if args.smoke else selection(args, settings)
    if args.listener_outage:
        profiles = [LISTENER_OUTAGE_PROFILE]
    if not profiles:
        raise FixtureError("no valid profiles for the chosen runner counts and slots")
    if args.faults or args.j2_lane_7:
        profiles = []
    ref = checked(["git", "rev-parse", "--verify", "--end-of-options", f"{args.mesh_ref}^{{commit}}"])
    baseline_ref = None
    if args.against_ref:
        baseline_ref = checked(["git", "rev-parse", "--verify", "--end-of-options",
                                f"{args.against_ref}^{{commit}}"])
    base = EVIDENCE / ref
    base.mkdir(parents=True, exist_ok=True)
    lane = "fault" if args.faults else "smoke" if args.smoke else "load"
    if args.listener_outage:
        lane = "listener-outage"
    if args.j2_lane_7:
        lane = "j2-lane-7"
    prefix = lane if args.j2_lane_7 else f"j1-{lane}"
    evidence = Path(tempfile.mkdtemp(prefix=f"{prefix}-", dir=base))
    budget_breaches = []
    write_json(evidence / "settings.json", settings)
    print(evidence, flush=True)
    try:
        with workspace(evidence) as scratch:
            harness, harness_sha, harness_hashes = snapshot_harness(scratch, evidence)
            oban_fixture = harness / OBAN.relative_to(ROOT)
            env = dict(os.environ, TMPDIR=str(scratch), TZ="UTC", LC_ALL="C",
                       MESH_SCHEDULER_MIN_WORKERS=str(settings["runner_schedulers"]),
                       MESH_SCHEDULER_MAX_WORKERS=str(settings["runner_schedulers"]),
                       JOBS_PARITY_MESH_BATCH="1" if mesh_batch else "0",
                       JOBS_PARITY_MESH_LISTEN="1" if mesh_listen else "0",
                       JOBS_PARITY_MESH_EVENTS=mesh_events or "",
                       ERL_FLAGS=settings["oban_erl_flags"])
            env.pop("MESH_RT_LIB_PATH", None)
            env.pop("RUSTC_WRAPPER", None)
            env.pop("RUSTC_WORKSPACE_WRAPPER", None)
            env.pop("JOBS_PARITY_SECOND_SPAWN_GATE", None)
            env.pop("JOBS_PARITY_WORKER_START_GATE", None)
            executable = build_mesh(ref, scratch, evidence, env, release,
                                    harness / MESH_SOURCE.relative_to(ROOT), args.toolchains,
                                    startup_faults=args.j2_lane_7)
            baseline = executable
            if baseline_ref and baseline_ref != ref:
                baseline_dir = scratch / "baseline"
                baseline_dir.mkdir()
                baseline_evidence = evidence / "baseline-build"
                baseline_evidence.mkdir()
                baseline = build_mesh(baseline_ref, baseline_dir, baseline_evidence, env, release,
                                      harness / MESH_SOURCE.relative_to(ROOT), args.toolchains)
            listener_proxy = None
            versions = {}
            if args.listener_outage or args.faults:
                proxy_spec = importlib.util.spec_from_file_location(
                    "parity_fault_probe", harness / "scripts/lib/jobs_parity_fault.py")
                proxy_module = importlib.util.module_from_spec(proxy_spec)
                proxy_spec.loader.exec_module(proxy_module)
            if args.listener_outage:
                listener_proxy = proxy_module.ListenerProxy
            elif not args.j2_lane_7 and not baseline_ref:
                command, mix_env, versions = compile_oban(scratch, evidence, env, oban_fixture)
            max_connections = max((s + r * settings["connection_slots_per_runner"]
                                   for _, r, s in profiles), default=7 if args.j2_lane_7 else 5)
            cluster = Cluster(scratch, evidence, max_connections, args.postgres_version)
            try:
                cluster.start()
                pg_version = cluster.sql("postgres", "SHOW server_version_num")
                actual_settings = cluster.sql("postgres", """
SELECT json_object_agg(name, setting) FROM pg_settings
WHERE name IN ('server_version','server_version_num','shared_buffers','max_connections',
               'track_commit_timestamp','synchronous_commit','pg_stat_statements.track',
               'pg_stat_statements.max','timezone','fsync','full_page_writes')
""")
                write_json(evidence / "manifest.json", {
                    "mesh_sha": ref, "mesh_executable_sha256": digest(executable), **versions,
                    "baseline_mesh_sha": baseline_ref,
                    "baseline_executable_sha256": digest(baseline) if baseline_ref else None,
                    "harness_sha": harness_sha, "harness_sha256": harness_hashes,
                    "postgres_settings": json.loads(actual_settings), "postgres_version_num": pg_version,
                    "host": platform.platform(), "logical_cpus": os.cpu_count(),
                    "load_average_at_start": os.getloadavg(),
                    "session_transport": "SCRAM with verified TLS" if args.postgres_version else "SCRAM",
                    "reference_shas": {name: checked(["git", "rev-parse", name]) for name in REFERENCE},
                    "seed": settings["seed"], "profiles": profiles,
                    "mesh_build_profile": "release" if release else "debug",
                    "mesh_batch": mesh_batch,
                    "mesh_listen": mesh_listen,
                    "mesh_events": mesh_events,
                    "poll_ms": poll_ms,
                    "baseline_poll_ms": baseline_poll_ms if baseline_ref else None,
                    "sparse_producer": "one warm process with database connections scoped to each batch",
                    "mode": lane, "claim_timestamps": {"mesh": "database attempted_at_ms",
                                                       "oban": "Basic engine attempted_at"},
                    "settlement_timestamp": "durable commit timestamp of the terminal row version",
                    "resource_scope": "runner process trees; sampled RSS, processes, threads and connections",
                })
                subjects = {"mesh": Subject("mesh", [str(executable)], ROOT, cluster, env, poll_ms=poll_ms)}
                if args.j2_lane_7:
                    confirmed = run_confirmed_launch_failure(subjects["mesh"], scratch, evidence)
                    ambiguous = run_ambiguous_startup(subjects["mesh"], scratch, evidence)
                    write_json(evidence / "j2-lane-7.log", {"confirmed": confirmed, "ambiguous": ambiguous})
                elif args.listener_outage:
                    subjects["disconnected"] = Subject("mesh", [str(executable)], ROOT, cluster, env,
                                                       variant="disconnected", poll_ms=poll_ms)
                elif baseline_ref:
                    baseline_env = dict(env, JOBS_PARITY_MESH_BATCH="0", JOBS_PARITY_MESH_LISTEN="0",
                                        JOBS_PARITY_MESH_EVENTS="")
                    subjects["baseline"] = Subject("mesh", [str(baseline)], ROOT, cluster, baseline_env,
                                                   variant="baseline", poll_ms=baseline_poll_ms)
                else:
                    subjects["oban"] = Subject("oban", command, oban_fixture, cluster, mix_env, poll_ms=poll_ms)
                variants = tuple(subjects)
                if args.faults:
                    recovery_receipts = {}
                    for engine, subject in subjects.items():
                        case_evidence = evidence / engine
                        case_evidence.mkdir()
                        case_scratch = scratch / f"fault-{engine}"
                        case_scratch.mkdir()
                        recovery_receipts[engine] = run_fault(subject, case_scratch, case_evidence, settings)
                        if not cluster.expected_version:
                            for placement in ("before_commit", "after_commit"):
                                placement_evidence = case_evidence / placement
                                placement_evidence.mkdir()
                                placement_scratch = case_scratch / placement
                                placement_scratch.mkdir()
                                run_settlement_fault(subject, placement_scratch, placement_evidence,
                                                     settings, placement, proxy_module.SettlementProxy)
                    if baseline_ref:
                        fields = ("job_key", "state", "attempt", "attempts", "effects", "effect_keys",
                                  "first_handler_attempt", "payload_sha256", "hold_ms", "fail_first")
                        candidate = recovery_receipts["mesh"]["after_recovery"]
                        reference = recovery_receipts["baseline"]["after_recovery"]
                        if any(candidate[field] != reference[field] for field in fields):
                            raise QueueMismatch("main and candidate recovered different durable results")
                        write_json(evidence / "j1-lane-1.log", {
                            "harness_sha": harness_sha, "mesh_sha": ref, "baseline_mesh_sha": baseline_ref,
                            "postgres_version_num": pg_version,
                            "matching_result": {field: candidate[field] for field in fields},
                            "receipts": recovery_receipts,
                        })
                    if not cluster.expected_version:
                        for placement in ("before_commit", "after_commit"):
                            case = evidence / "mesh" / f"producer-{placement}"
                            case.mkdir()
                            inputs_dir = scratch / case.name
                            inputs_dir.mkdir()
                            run_producer_commit_fault(subjects["mesh"], inputs_dir, case, settings,
                                                      placement, proxy_module.SettlementProxy)
                    if args.mesh_batch and not cluster.expected_version:
                        for placement, final_attempt in (
                            ("before_commit", False), ("after_commit", False), ("after_command_complete", False),
                            ("after_commit", True),
                        ):
                            name = f"claim-{placement}" + ("-final-attempt" if final_attempt else "")
                            case = evidence / "mesh" / name
                            case.mkdir()
                            inputs_dir = scratch / name
                            inputs_dir.mkdir()
                            run_batch_claim_fault(subjects["mesh"], inputs_dir, case, settings,
                                                  placement, proxy_module.SettlementProxy, final_attempt=final_attempt)
                for name, runners, slots in profiles:
                    profile = f"{name}-{runners}r-{slots}s"
                    case_evidence = evidence / profile
                    case_evidence.mkdir()
                    workload = settings["workloads"][name]
                    allocation = allocate_slots(runners, slots)
                    cap = slots + runners * settings["connection_slots_per_runner"]
                    cluster.connection_cap = cap
                    trials = []
                    pair_count = 1 if args.smoke else settings["trial_pairs"]
                    for pair in range(-1, pair_count):
                        batches = inputs(workload, slots, settings["seed"], smoke=args.smoke or pair < 0)
                        order = variants if pair % 2 == 0 else tuple(reversed(variants))
                        for engine in order:
                            run_name = f"{pair if pair >= 0 else 'warmup'}-{engine}"
                            run_evidence = case_evidence / run_name
                            run_evidence.mkdir()
                            run_scratch = scratch / f"{profile}-{run_name}"
                            run_scratch.mkdir()
                            result = run_trial(subjects[engine], allocation, workload, batches, run_scratch,
                                               run_evidence, settings, pair, listener_proxy)
                            write_json(run_evidence / "metrics.json", result)
                            shutil.rmtree(run_scratch)
                            if pair >= 0:
                                trials.append(result)
                    validate_pairs(trials, pair_count, slots, variants)
                    write_json(case_evidence / "trials.json", trials)
                    summary = profile_summary(trials, name, workload, settings, runners, slots, variants,
                                              mesh_batch, mesh_listen, mesh_events)
                    write_json(case_evidence / "summary.json", summary)
                    for scope, values in summary.items():
                        for metric, verdict in values.items():
                            if isinstance(verdict, dict) and (
                                verdict.get("within_limit") is False or verdict.get("within_budget") is False
                            ):
                                budget_breaches.append({"profile": profile, "scope": scope, "metric": metric})
            finally:
                cluster.stop()
    except (FixtureError, QueueMismatch, OSError, subprocess.SubprocessError) as error:
        kind = "queue-mismatch" if isinstance(error, QueueMismatch) else "fixture-error"
        write_json(evidence / "result.json", {"status": kind, "reason": str(error)})
        raise
    write_json(evidence / "result.json", {"status": "passed", "mode": lane,
                                         "five_paired_trials": args.load or args.listener_outage,
                                         "budget_breaches": budget_breaches})


def self_check():
    import unittest

    class DriverChecks(unittest.TestCase):
        def test_duplicate_native_marker_cannot_pass_the_startup_gate(self):
            from unittest.mock import mock_open, Mock

            log = Mock()
            log.open = mock_open(read_data=b"READY\nPARITY_WORKER_START 42\nPARITY_WORKER_START 42\n")
            self.assertEqual(native_worker_starts(log), [42, 42])
            with self.assertRaisesRegex(QueueMismatch, "duplicate native startup"):
                held_native_worker(Mock(), log)

        def test_inputs_and_slot_allocation(self):
            settings = json.loads(CONFIG.read_text())
            workload = settings["workloads"]["slow"]
            left = inputs(workload, 16, 9)
            self.assertEqual(left, inputs(workload, 16, 9))
            self.assertNotEqual(left, inputs(workload, 16, 10))
            self.assertEqual(sum(len(batch["jobs"]) for batch in left), 128)
            self.assertEqual(len(left[0]["jobs"][0]["payload"].encode()), 16_384)
            self.assertEqual(allocate_slots(4, 17), [5, 4, 4, 4])

        def test_latency_order_and_reconciliation(self):
            row = {"job_key": "a", "state": "completed", "attempt": 1, "attempts": 1,
                   "first_handler_attempt": 1,
                   "effects": 1, "effect_keys": 1, "claimed_ms": 99, "entered_ms": 103,
                   "recorded_ms": 111, "effected_ms": 115,
                   "settled_ms": 125, "payload_sha256": hashlib.sha256(b"test").hexdigest(),
                   "hold_ms": 0, "fail_first": False}
            expected = {"a": ("batch", {"payload": "test", "hold_ms": 0, "fail_first": False})}
            result = account([row], expected, {"batch": 100}, 5)
            self.assertEqual(result[0]["pickup_ms"], 3)
            self.assertEqual(result[0]["commit_to_claim_ms"], 0)
            self.assertEqual(result[0]["claim_to_entry_ms"], 4)
            self.assertEqual(result[0]["entry_to_attempt_commit_ms"], 8)
            self.assertEqual(result[0]["attempt_to_effect_commit_ms"], 4)
            self.assertEqual(result[0]["effect_commit_to_settlement_ms"], 10)
            self.assertEqual(result[0]["commit_to_settlement_ms"], 25)
            self.assertEqual(percentile(list(range(1, 101)), 0.99), 99)
            with self.assertRaises(QueueMismatch):
                account([], expected, {"batch": 100}, 5)
            with self.assertRaises(QueueMismatch):
                account([dict(row, hold_ms=1)], expected, {"batch": 100}, 5)
            with self.assertRaises(QueueMismatch):
                account([dict(row, first_handler_attempt=99)], expected, {"batch": 100}, 5)
            with self.assertRaises(FixtureError):
                account([dict(row, entered_ms=80)], expected, {"batch": 100}, 5)
            with self.assertRaises(FixtureError):
                account([dict(row, recorded_ms=None)], expected, {"batch": 100}, 5)
            with self.assertRaises(FixtureError):
                account([dict(row, effected_ms=131)], expected, {"batch": 100}, 5)
            with self.assertRaises(FixtureError):
                account([dict(row, entered_ms=115, recorded_ms=111, effected_ms=107, settled_ms=103)],
                        expected, {"batch": 100}, 5)
            with self.assertRaises(FixtureError):
                percentile([], 0.99)

        def test_warm_producer_keeps_one_process_across_enqueues(self):
            from unittest.mock import Mock

            script = """
import json, os, sys
assert sys.argv[1] == 'enqueue' and os.environ['JOBS_PARITY_INPUT'] == '-'
print('PRODUCER_READY', flush=True)
for number, line in enumerate(sys.stdin, 1):
    if line == 'stop\\n':
        break
    assert line.startswith('/')
    print(json.dumps({'count': 1, 'ids': {'job': str(number)}, 'xid': str(number), 'pid': os.getpid()}), flush=True)
"""
            with tempfile.TemporaryDirectory(prefix="parity-stream-test-", dir=ROOT) as directory:
                directory = Path(directory)
                cluster = Mock()
                cluster.url.return_value = "postgresql://localhost/test"
                cluster.sql.side_effect = ["1.25", "2.50"]
                subject = Subject("oban", [sys.executable, "-c", script], ROOT, cluster, os.environ)
                with subject.warm_producer(os.environ, directory, time.monotonic() + 5) as send:
                    first, first_commit = send(directory / "one.json", directory / "one.log")
                    second, second_commit = send(directory / "two.json", directory / "two.log")
                self.assertEqual(first["pid"], second["pid"])
                self.assertEqual([first["ids"], second["ids"]], [{"job": "1"}, {"job": "2"}])
                self.assertEqual([first_commit, second_commit], [1.25, 2.50])

        def test_trial_validation(self):
            with self.assertRaisesRegex(FixtureError, "incomplete"):
                validate_pairs([], 5, 1)
            rows = [{"pair": pair, "engine": engine, "input_sha256": "shared"}
                    for engine in ("mesh", "oban") for pair in range(5)]
            with self.assertRaisesRegex(FixtureError, "completed"):
                validate_pairs(rows, 5, 1)
            rows[-1]["input_sha256"] = "mismatch"
            with self.assertRaisesRegex(FixtureError, "different inputs"):
                validate_pairs(rows, 5, 1)

        def test_event_buffer_report_accepts_legacy_status_and_validates_backpressure(self):
            from unittest.mock import Mock

            legacy = {"state": "stopped", "drained": True, "events_dropped": 7, "event_logs_dropped": 0}
            log = Mock()
            log.read_text.return_value = f'READY\n{{"event":"runner_stopped"}}\n{json.dumps(legacy)}\n'
            status = mesh_shutdown_report(log)
            self.assertIsNone(mesh_event_buffer_report(status, "backpressured", log))
            self.assertIsNone(mesh_event_buffer_report({"state": "stopped", "drained": True}, "", log))

            reader = {"sampled_peak_bytes": 256, "upper_bound_bytes": 256}
            report = {"reader": reader, "logger": None}
            status["event_buffer_bytes"] = report
            self.assertEqual(mesh_event_buffer_report(status, "backpressured", log), report)
            for sampled in (-1, True, 257, 128):
                with self.subTest(sampled=sampled), self.assertRaises(FixtureError):
                    status["event_buffer_bytes"] = dict(report, reader=dict(reader, sampled_peak_bytes=sampled))
                    mesh_event_buffer_report(status, "backpressured", log)
            with self.assertRaises(FixtureError):
                mesh_event_buffer_report(dict(status, event_buffer_bytes=None), "backpressured", log)

        def test_event_buffer_summary_keeps_estimate_bound_and_legacy_distinct(self):
            settings = json.loads(CONFIG.read_text())
            workload = settings["workloads"]["short"]
            base = {"jobs_per_second": 100, "peak_connections": 1, "peak_processes": 1, "peak_threads": 1}
            logged = {"reader": None, "logger": {"sampled_peak_bytes": 32, "upper_bound_bytes": 256}}
            first = mesh_event_buffer_report({"event_buffer_bytes": logged}, "logged", "runner.log")
            second = dict(logged, logger=dict(logged["logger"], sampled_peak_bytes=64))
            trial = max_event_buffer_bytes([first, second])
            self.assertEqual(trial, second)
            self.assertIsNone(max_event_buffer_bytes([first, None]))
            rows = [dict(base, engine="mesh", event_buffer_bytes=first),
                    dict(base, engine="mesh", event_buffer_bytes=trial),
                    dict(base, engine="baseline")]
            args = ("short", workload, settings, 1, 1, ("mesh", "baseline"), False, False, "logged")
            summary = profile_summary(rows, *args)
            self.assertEqual(summary["mesh"]["event_buffer_bytes"], second)
            self.assertIsNone(summary["baseline"]["event_buffer_bytes"])
            partial = profile_summary(rows[1:] + [dict(base, engine="mesh")], *args)
            self.assertIsNone(partial["mesh"]["event_buffer_bytes"])

        def test_outage_budget_uses_worst_observed_pickup(self):
            settings = json.loads(CONFIG.read_text())
            workload = settings["workloads"]["sparse"]
            base = {"peak_connections": 4, "peak_processes": 2, "peak_threads": 6,
                    "peak_queued_context_bytes": 1024, "poll_ms": 1000, "claim_p99_ms": 15}
            rows = [dict(base, engine="mesh", pickup_p99_ms=25, pickup_max_ms=30),
                    dict(base, engine="disconnected", pickup_p99_ms=1100, pickup_max_ms=1100)]
            args = ("sparse", workload, settings, 1, 16, ("mesh", "disconnected"), True, True, None)
            for pickup, passes in ((2000, True), (2001, False)):
                with self.subTest(pickup=pickup):
                    slower = dict(rows[-1], pickup_max_ms=pickup)
                    summary = profile_summary(rows + [slower], *args)
                    self.assertEqual(summary["comparison"]["fallback_pickup_max_ms"],
                                     {"observed": pickup, "limit": 2000, "within_limit": passes})
                    self.assertEqual(summary["mesh"]["pickup_p99_ms"]["budget"], 750)

        def test_trial_reconciliation_rejects_stray_rows_and_wrong_effect_keys(self):
            from unittest.mock import patch

            namespace = "parity_0123456789abcdef"
            enqueued = {"key": "test-id"}
            attempts = {"key": (1,)}
            observed = {
                "jobs": [[namespace, "test-id", "key"]],
                "business": [["mesh", namespace, "batch"]],
                "attempts": [["mesh", namespace, "key", 1, "test-id/1"]],
                "effects": [["mesh", namespace, "key", "test-id/1"]],
            }
            cluster = Cluster.__new__(Cluster)
            cases = (
                ("jobs", ["stray", "test-id-2", "key"]),
                ("business", ["mesh", "stray", "batch"]),
                ("attempts", ["mesh", namespace, "key", 99, "test-id/2"]),
                ("effects", ["mesh", namespace, "key", "test-id/2"]),
            )
            with patch.object(cluster, "sql", side_effect=lambda *_: json.dumps(observed)):
                cluster.reconcile_trial("mesh", namespace, enqueued, attempts, ["batch"])
                for kind, stray in cases:
                    with self.subTest(kind=kind):
                        observed[kind].append(stray)
                        with self.assertRaisesRegex(QueueMismatch, f"{kind} identities"):
                            cluster.reconcile_trial("mesh", namespace, enqueued, attempts, ["batch"])
                        observed[kind].pop()
                pending = dict(observed, attempts=[], effects=[])
                with patch.object(cluster, "sql", side_effect=lambda *_: json.dumps(pending)):
                    cluster.reconcile_trial("mesh", namespace, enqueued, {"key": ()}, ["batch"])
                    pending["effects"] = observed["effects"]
                    with self.assertRaisesRegex(QueueMismatch, "effects identities"):
                        cluster.reconcile_trial("mesh", namespace, enqueued, {"key": ()}, ["batch"])
                observed["jobs"] = [[namespace, "replacement-id", "key"]]
                observed["attempts"] = [["mesh", namespace, "key", 1, "replacement-id/1"]]
                observed["effects"] = [["mesh", namespace, "key", "replacement-id/1"]]
                with self.assertRaisesRegex(QueueMismatch, "jobs identities"):
                    cluster.reconcile_trial("mesh", namespace, enqueued, attempts, ["batch"])

        def test_direct_peak_cannot_exceed_slots(self):
            metrics = dict.fromkeys(("completed", "jobs_per_second", "sql_calls_per_job",
                                     "runner_sql_calls_per_job", "enqueue_sql_calls_per_job", "claim_calls_per_job",
                                     "peak_connections", "peak_rss_bytes", "cpu_seconds_per_job", "peak_processes",
                                     "peak_threads", "pickup_p99_ms", "claim_p99_ms", "settlement_p99_ms",
                                     "idle_queries_per_minute", "peak_executing"), 1)
            rows = [dict(metrics, pair=pair, engine=engine, input_sha256="shared", estimated_peak_executing=2)
                    for engine in ("mesh", "oban") for pair in range(5)]
            validate_pairs(rows, 5, 1)
            rows[0]["peak_executing"] = 2
            with self.assertRaisesRegex(QueueMismatch, "exceeds 1 slots"):
                validate_pairs(rows, 5, 1)

        def test_settlement_fault_reads_state_after_stopping_runner(self):
            from types import SimpleNamespace
            from unittest.mock import Mock, patch

            subject = Mock(engine="mesh")
            subject.environment.return_value = {}
            subject.enqueue_receipt.return_value = ({"ids": {"settlement": "enqueued"}}, 100)
            cluster = subject.cluster
            cluster.expected_version = None
            cluster.observe.return_value = {"terminal": 1}
            cluster.rows.return_value = [{"id": "enqueued", "state": "completed", "attempt": 1,
                                          "attempts": 1, "effects": 1, "effect_keys": 1}]

            def late_write(*_, **__):
                cluster.rows.return_value[0] = dict(cluster.rows.return_value[0], attempt=99)

            with patch(__name__ + ".runner_ready", return_value=True), patch(
                __name__ + ".batch_files", return_value=[Path("input.json")]
            ), patch(__name__ + ".stop_runners", side_effect=late_write), patch(__name__ + ".write_json") as output:
                proxy = lambda *_: contextlib.nullcontext(SimpleNamespace(port=1234, fault={"cut": True}))
                with self.assertRaisesRegex(QueueMismatch, "unexpected durable rows"):
                    run_settlement_fault(subject, Path("scratch"), Path("receipt"),
                                         {"poll_ms": 250, "trial_timeout_seconds": 1}, "after_commit", proxy)
                output.assert_not_called()

        def test_macos_thread_sample_counts_continuation_rows(self):
            report = """USER      PID  TT  %CPU
user       10  ??   0.0
           10       0.0
           10       0.0
user       20  ??   0.0
other      30  ??   0.0
           30       0.0
"""
            self.assertEqual(mac_thread_counts(report, {10, 20}), {10: 3, 20: 1})
            self.assertEqual(mac_thread_counts(report, {10, 40}), {10: 3})

        @unittest.skipUnless(sys.platform == "darwin", "requires macOS ps -M")
        def test_macos_thread_count_against_live_threads(self):
            fixture = """
import sys
from threading import Event, Thread
stop = Event()
threads = [Thread(target=stop.wait) for _ in range(4)]
for thread in threads:
    thread.start()
print('ready', flush=True)
sys.stdin.read(1)
stop.set()
"""
            with subprocess.Popen([sys.executable, "-c", fixture], stdin=subprocess.PIPE,
                                  stdout=subprocess.PIPE, text=True) as worker:
                try:
                    self.assertEqual(worker.stdout.readline().strip(), "ready")
                    report = checked(["ps", "-M", "-p", str(worker.pid)])
                    self.assertEqual(mac_thread_counts(report, {worker.pid}), {worker.pid: 5})
                finally:
                    worker.communicate("q", timeout=5)

        def test_peak_executing_includes_jobs_between_observer_samples(self):
            rows = [{"claimed_ms": 101, "settled_ms": 110}, {"claimed_ms": 103, "settled_ms": 109}]
            self.assertEqual(estimated_peak_executing(rows), 2)
            rows[1]["claimed_ms"] = 110
            rows[1]["settled_ms"] = 120
            self.assertEqual(estimated_peak_executing(rows), 1)

        def test_cleanup_retains_workers_seen_before_the_runner_exited(self):
            from unittest.mock import patch

            class ExitedRunner:
                pid = 10
                returncode = -9
                observed_workers = {11: "original start"}
                fixture_executable = "/fixture"

            with patch(__name__ + ".process_table", return_value={}), patch(
                __name__ + ".wait_for_worker_exit"
            ) as wait:
                stop_runners([ExitedRunner()], require_clean=False)
                wait.assert_called_once_with({11: "original start"}, {"/fixture"})

        def test_cleanup_discovers_worker_reparented_before_any_snapshot(self):
            from unittest.mock import patch

            with patch(__name__ + ".process_table", side_effect=[{42: (1, 0, "start")}, {}]) as observed, patch(
                __name__ + ".fixture_workers", side_effect=[{42: "start"}, {}]
            ) as owned:
                wait_for_worker_exit({}, {"/fixture"})
                self.assertEqual(observed.call_count, 2)
                self.assertEqual(owned.call_args_list[0].args, ({42: (1, 0, "start")}, {"/fixture"}))

    suite = unittest.defaultTestLoader.loadTestsFromTestCase(DriverChecks)
    return unittest.TextTestRunner(verbosity=2).run(suite).wasSuccessful()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--self-check", action="store_true")
    mode.add_argument("--smoke", action="store_true", help="small integration probe, not a benchmark")
    mode.add_argument("--load", action="store_true", help="five alternating trials per selected profile")
    mode.add_argument("--listener-outage", action="store_true",
                      help="pair healthy and disconnected listeners on the frozen sparse profile")
    mode.add_argument("--faults", action="store_true", help="owned runner-death and rollback probe")
    mode.add_argument("--j2-lane-7", action="store_true", help="macOS batch launch denial and interrupted startup proof")
    parser.add_argument("--mesh-ref", default="HEAD", help="Git commit from which to compile Mesh")
    parser.add_argument("--toolchains", type=lambda value: Path(value).resolve(),
                        help="reuse verified compiler/runtime pairs from a previous Jobs comparison build")
    parser.add_argument("--release", action="store_true", help="use a release toolchain for smoke and fault probes")
    parser.add_argument("--against-ref", help="compare with a second Mesh commit using single claims")
    parser.add_argument("--mesh-batch", action="store_true", help="enable batch claims for the Mesh runner")
    parser.add_argument("--mesh-listen", action="store_true", help="enable Mesh listener and transactional publishing")
    parser.add_argument("--mesh-events", choices=("disabled", "logged", "backpressured"),
                        help="measure Mesh telemetry with a log sink or an unread 128-event buffer")
    parser.add_argument("--poll-ms", type=int, help="override the dispatch poll interval")
    parser.add_argument("--baseline-poll-ms", type=int, help="override polling for the Mesh reference only")
    parser.add_argument("--postgres-version", metavar="MAJOR.PATCH",
                        help="require exact server version and SCRAM over verified TLS")
    settings = json.loads(CONFIG.read_text())
    parser.add_argument("--workload", action="append", choices=list(settings["workloads"]))
    parser.add_argument("--runners", type=int, choices=settings["runner_counts"])
    parser.add_argument("--slots", type=int, choices=settings["total_slots"])
    args = parser.parse_args()
    if args.self_check:
        return 0 if self_check() else 1
    if any(interval is not None and not 1 <= interval <= 60_000 for interval in (args.poll_ms, args.baseline_poll_ms)):
        parser.error("poll intervals must be between 1 and 60000 ms")
    if args.baseline_poll_ms is not None and not args.against_ref:
        parser.error("--baseline-poll-ms requires --against-ref")
    if args.postgres_version and not POSTGRES_VERSION.fullmatch(args.postgres_version):
        parser.error("--postgres-version requires MAJOR.PATCH")
    if args.listener_outage and (not args.postgres_version or args.workload or args.runners or args.slots
                                or args.against_ref or args.poll_ms or args.mesh_events):
        parser.error("--listener-outage requires --postgres-version and uses a fixed sparse profile")
    if not args.load and (args.runners is not None or args.slots is not None):
        parser.error("--runners and --slots require --load")
    if args.workload and (args.faults or args.j2_lane_7 or (args.smoke and len(args.workload) > 1)):
        parser.error("select one workload for --smoke or use --load for several")
    if args.against_ref and args.j2_lane_7:
        parser.error("--against-ref is unavailable for --j2-lane-7")
    if args.j2_lane_7 and (sys.platform != "darwin" or not args.mesh_batch):
        parser.error("--j2-lane-7 requires macOS and --mesh-batch")

    def interrupt(_signal, _frame):
        raise InterruptedError("parity run received SIGTERM")

    signal.signal(signal.SIGTERM, interrupt)
    try:
        record(args)
    except QueueMismatch as error:
        print(f"Queue mismatch: {error}", file=sys.stderr)
        return 1
    except (FixtureError, OSError, subprocess.SubprocessError) as error:
        print(f"Fixture failure: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
