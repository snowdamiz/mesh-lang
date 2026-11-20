#!/usr/bin/env python3
"""Run the pinned Mesh and Oban Jobs fixtures against an owned PostgreSQL cluster."""

import argparse
import contextlib
import hashlib
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
REFERENCE = ("db606548", "59f6a7b3", "7c162056")
POSTGRES_VERSION = re.compile(r"[0-9]+\.[0-9]+")
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


class QueueMismatch(Exception):
    pass


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
        if row["state"] != "completed" or row["attempt"] != 1:
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
        claimed, entered, settled = (row[field] for field in ("claimed_ms", "entered_ms", "settled_ms"))
        if any(value is None for value in (claimed, entered, settled)):
            raise FixtureError(f"{key}: missing clock sample")
        within_clock(entered - claimed, tolerance)
        within_clock(settled - entered, tolerance)
        samples.append({"job_key": key, "commit_ms": committed, **row,
                        "commit_to_claim_ms": within_clock(claimed - committed, tolerance),
                        "pickup_ms": within_clock(entered - committed, tolerance),
                        "commit_to_settlement_ms": within_clock(settled - committed, tolerance)})
    return samples


def validate_pairs(trials, pairs):
    booked = [(trial["pair"], trial["engine"]) for trial in trials]
    expected = {(i, engine) for i in range(pairs) for engine in ("mesh", "oban")}
    if len(booked) != len(expected) or set(booked) != expected:
        raise FixtureError("incomplete or duplicate paired trials")
    if len({trial["input_sha256"] for trial in trials}) != 1:
        raise FixtureError("paired trials received different inputs")
    required = ("jobs_per_second", "pickup_p99_ms", "claim_p99_ms", "settlement_p99_ms",
                "sql_calls_per_job", "claim_calls_per_job", "idle_queries_per_minute", "peak_executing",
                "peak_connections", "peak_rss_bytes", "cpu_seconds_per_job",
                "peak_processes", "peak_threads")
    for trial in trials:
        for key in required:
            value = trial.get(key)
            if not isinstance(value, (int, float)) or not math.isfinite(value) or value < 0:
                raise FixtureError(f"missing or invalid {key} in pair {trial['pair']}")


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
        self.sql("postgres", f"CREATE ROLE parity_subject LOGIN CONNECTION LIMIT {self.connection_cap}"
                 f" PASSWORD '{self.password}'")

    def prepare_database(self, engine):
        self.sql("postgres", f"DROP DATABASE IF EXISTS {engine}")
        self.sql("postgres", f"CREATE DATABASE {engine} OWNER parity_subject")
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
SELECT json_build_object('total', coalesce(sum(calls),0),
                         'claims', coalesce(sum(calls) FILTER (WHERE {claim_query}),0))
FROM pg_stat_statements
WHERE dbid = (SELECT oid FROM pg_database WHERE datname = '{engine}')
  AND userid = (SELECT oid FROM pg_roles WHERE rolname = 'parity_subject')
"""))

    def observe(self, engine, namespace):
        relation, scope = job_relation(engine, namespace)
        return json.loads(self.sql(engine, f"""
SELECT json_build_object(
 'connections', (SELECT count(*) FROM pg_stat_activity WHERE usename = 'parity_subject'
                    AND datname = current_database()),
 'executing', count(*) FILTER (WHERE state = 'executing'),
 'terminal', count(*) FILTER (WHERE state IN ('completed','discarded','cancelled'))
)
FROM {relation} WHERE {scope}
"""))

    def rows(self, engine, namespace):
        relation, scope = job_relation(engine, namespace)
        claim = "j.attempted_at_ms" if engine == "mesh" else "extract(epoch FROM j.attempted_at) * 1000"
        return json.loads(self.sql(engine, f"""
WITH attempts AS (
 SELECT job_key, count(*) AS attempts, min(entered_at) AS entered_at,
        count(DISTINCT effect_key) AS effect_keys
 FROM public.mesh_jobs_parity_attempts
 WHERE engine = '{engine}' AND namespace = '{namespace}' GROUP BY job_key
), effects AS (
 SELECT job_key, count(*) AS effects FROM public.mesh_jobs_parity_effects
 WHERE engine = '{engine}' AND namespace = '{namespace}' GROUP BY job_key
)
SELECT coalesce(json_agg(summary ORDER BY summary.job_key), '[]') FROM (
 SELECT j.id::text AS id, j.args->>'key' AS job_key, j.state, j.attempt,
   encode(sha256(convert_to(j.args->>'payload', 'UTF8')), 'hex') AS payload_sha256,
   (j.args->>'hold_ms')::integer AS hold_ms, (j.args->>'fail_first')::boolean AS fail_first,
   coalesce(a.attempts,0) AS attempts, coalesce(e.effects,0) AS effects,
   coalesce(a.effect_keys,0) AS effect_keys, ({claim}) AS claimed_ms,
   extract(epoch FROM a.entered_at)*1000 AS entered_ms,
   extract(epoch FROM pg_xact_commit_timestamp(j.xmin))*1000 AS settled_ms
 FROM {relation} j
 LEFT JOIN attempts a ON a.job_key = j.args->>'key'
 LEFT JOIN effects e ON e.job_key = j.args->>'key'
 WHERE {scope}
) summary
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
    def __init__(self, engine, command, cwd, cluster, env):
        self.engine = engine
        self.command = command
        self.cwd = cwd
        self.cluster = cluster
        self.env = env

    def environment(self, namespace, slots, total_slots, mode, poll_ms):
        return dict(self.env, JOBS_PARITY_DB_URL=self.cluster.url(self.engine),
                    JOBS_PARITY_NAMESPACE=namespace, JOBS_PARITY_SLOTS=str(slots),
                    JOBS_PARITY_TOTAL_SLOTS=str(total_slots), JOBS_PARITY_MODE=mode,
                    JOBS_PARITY_POLL_MS=str(poll_ms), JOBS_PARITY_RUNNER="controller")

    def call(self, action, env, log):
        return checked([*self.command, action], cwd=self.cwd, env=env, log=log)

    def enqueue(self, env, path, log):
        result = self.call("enqueue", dict(env, JOBS_PARITY_INPUT=str(path)), log)
        try:
            receipt = json.loads(result.splitlines()[-1])
            if not re.fullmatch(r"\d+", receipt["xid"]) or type(receipt["count"]) is not int:
                raise ValueError("invalid enqueue receipt")
        except (ValueError, KeyError, IndexError, TypeError) as error:
            raise FixtureError(f"invalid enqueue receipt in {log}") from error
        committed = self.cluster.sql(self.engine, "SELECT extract(epoch FROM "
                                     f"pg_xact_commit_timestamp('{receipt['xid']}'::xid))*1000")
        return receipt["count"], float(committed) if committed else None

    def run(self, env, index, log):
        with log.open("wb") as stream:
            process = subprocess.Popen([*self.command, "run"], cwd=self.cwd, start_new_session=True,
                                       env=dict(env, JOBS_PARITY_RUNNER=f"runner_{index}"),
                                       stdin=subprocess.PIPE if self.engine == "oban" else subprocess.DEVNULL,
                                       stdout=stream, stderr=stream)
        return process


def wait_for(observe, deadline, description, interval=0.1):
    while time.monotonic() < deadline:
        result = observe()
        if result:
            return result
        time.sleep(interval)
    raise FixtureError(f"timed out waiting for {description}")


def runner_ready(processes, logs):
    for process, log in zip(processes, logs, strict=True):
        if process.poll() is not None:
            raise FixtureError(f"runner exited {process.returncode}; see {log}")
        # Keep enough output for diagnostics without reading an unbounded stream.
        with log.open("rb") as stream:
            if b"READY" not in stream.read(32768).splitlines():
                return False
    return True


def process_sample(processes):
    roots = {process.pid for process in processes}
    table = {}
    for line in checked(["ps", "-A", "-o", "pid=,ppid=,rss="]).splitlines():
        pid, parent, rss = map(int, line.split())
        table[pid] = (parent, rss)
    owned = roots & table.keys()
    while True:
        children = {pid for pid, (parent, _) in table.items() if parent in owned}
        if children <= owned:
            break
        owned |= children
    if not owned:
        return {"processes": 0, "threads": 0, "rss_bytes": 0}
    if sys.platform == "darwin":
        command = ["ps", "-M", "-p", ",".join(map(str, sorted(owned)))]
        result = subprocess.run(command, capture_output=True, text=True, timeout=5)
        threads = sum(fields[1].isdigit() and int(fields[1]) in owned
                      for line in result.stdout.splitlines() if len(fields := line.split()) >= 2)
    else:
        threads = sum(len(list(Path(f"/proc/{pid}/task").glob("*"))) for pid in owned)
    return {"processes": len(owned), "threads": threads,
            "rss_bytes": 1024 * sum(table[pid][1] for pid in owned)}


def stop_runners(processes, *, require_clean):
    cpu_seconds = 0.0
    error = None
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
        **cluster.observe(engine, namespace),
        **process_sample(processes),
    }


def run_trial(subject, allocation, workload, batches, scratch, evidence, settings, pair):
    engine = subject.engine
    cluster = subject.cluster
    cluster.prepare_database(engine)
    namespace = f"parity_{secrets.token_hex(8)}"
    env = subject.environment(namespace, allocation[0], sum(allocation),
                              workload["mode"], settings["poll_ms"])
    subject.call("migrate", env, evidence / "migrate.log")
    expected = {job["key"]: (batch["request_id"], job) for batch in batches for job in batch["jobs"]}
    paths = batch_files(scratch, batches)
    case = hashlib.sha256("".join(digest(path) for path in paths).encode()).hexdigest()
    logs = [evidence / f"runner-{index}.log" for index in range(len(allocation))]
    processes = []
    completed = False
    try:
        for index, (slots, log) in enumerate(zip(allocation, logs, strict=True)):
            processes.append(subject.run(dict(env, JOBS_PARITY_SLOTS=str(slots)), index, log))
        deadline = time.monotonic() + settings["trial_timeout_seconds"]
        wait_for(lambda: runner_ready(processes, logs), deadline, "runner readiness")
        idle_start = time.monotonic()
        idle_calls_before = cluster.calls(engine)
        idle_samples = [take_sample(cluster, engine, namespace, processes, idle_start)]
        time.sleep(settings["idle_seconds"])
        idle_calls_after = cluster.calls(engine)
        idle_s = time.monotonic() - idle_start
        idle_samples.append(take_sample(cluster, engine, namespace, processes, idle_start))
        commits = {}
        start = time.monotonic()
        samples = []
        interval = settings["observer_interval_ms"] / 1000

        def settled_count(count):
            if not runner_ready(processes, logs):
                raise FixtureError("runner readiness lost during observation")
            sample = take_sample(cluster, engine, namespace, processes, start)
            samples.append(sample)
            return sample["terminal"] >= count

        expected_total = 0
        with ThreadPoolExecutor(max_workers=1) as producer:
            for path, batch in zip(paths, batches, strict=True):
                request_id = batch["request_id"]
                enqueue = producer.submit(subject.enqueue, env, path, evidence / f"enqueue-{request_id}.log")
                while not enqueue.done() and time.monotonic() < deadline:
                    settled_count(len(expected))
                    time.sleep(interval)
                count, commit_ms = enqueue.result(timeout=max(0.0, deadline - time.monotonic()))
                if count != len(batch["jobs"]) or commit_ms is None:
                    raise QueueMismatch(f"{request_id} did not confirm the expected enqueue transaction")
                commits[request_id] = commit_ms
                expected_total += count
                if workload["mode"] == "sparse":
                    wait_for(lambda: settled_count(expected_total), deadline, request_id, interval)
                    time.sleep(settings["poll_ms"] / 1000)
        wait_for(lambda: settled_count(len(expected)), deadline, "durable settlements", interval)
        after = cluster.calls(engine)
        calls = after["total"] - idle_calls_after["total"]
        claim_calls = after["claims"] - idle_calls_after["claims"]
        idle_calls = idle_calls_after["total"] - idle_calls_before["total"]
        rows = account(cluster.rows(engine, namespace), expected, commits, settings["clock_tolerance_ms"])
        product = cluster.sql(engine, f"""
SELECT coalesce(json_agg(request_id ORDER BY request_id), '[]')
FROM public.mesh_jobs_parity_product
WHERE engine = '{engine}' AND namespace = '{namespace}'
""")
        if json.loads(product) != sorted(commits):
            raise QueueMismatch("business transactions did not match committed batches")
        duration = (max(row["settled_ms"] for row in rows) - min(commits.values())) / 1000
        if duration <= 0:
            raise FixtureError("nonpositive committed observation interval")
        completed = True
    finally:
        cpu = stop_runners(processes, require_clean=completed)

    raw = {"engine": engine, "namespace": namespace, "pair": pair, "case_sha256": case,
           "transactions": commits, "attempts": rows, "samples": samples, "idle_samples": idle_samples,
           "runner_cpu_seconds_including_startup_and_idle": cpu, "raw_sql_calls": calls,
           "raw_claim_calls": claim_calls, "raw_idle_sql_calls": idle_calls, "idle_seconds": idle_s}
    write_json(evidence / "raw.json", raw)
    return {
        "pair": pair, "engine": engine, "input_sha256": case, "completed": len(rows),
        "jobs_per_second": len(rows) / duration,
        "pickup_p99_ms": percentile([row["pickup_ms"] for row in rows], 0.99),
        "claim_p99_ms": percentile([row["commit_to_claim_ms"] for row in rows], 0.99),
        "settlement_p99_ms": percentile([row["commit_to_settlement_ms"] for row in rows], 0.99),
        "sql_calls_per_job": calls / len(rows),
        "claim_calls_per_job": claim_calls / len(rows),
        "idle_queries_per_minute": idle_calls * 60 / idle_s,
        "cpu_seconds_per_job": cpu / len(rows),
        **{f"peak_{key}": max(sample[key] for sample in samples + idle_samples)
           for key in ("executing", "connections", "rss_bytes", "processes", "threads")},
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
        return subject.enqueue(env, path, evidence / f"{batch_id}.log")

    _, rollback_timestamp = enqueue("rolled_back", True, False, 0)
    if rollback_timestamp is not None:
        raise QueueMismatch("rollback xid had a commit timestamp")
    process = subject.run(env, 0, evidence / "initial-runner.log")
    replacement = None
    completed = False
    try:
        deadline = time.monotonic() + settings["trial_timeout_seconds"]
        wait_for(lambda: runner_ready([process], [evidence / "initial-runner.log"]),
                 deadline, "fault runner readiness")
        _, committed = enqueue("interrupted", False, True, 30_000)
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
        if engine == "mesh":
            claim = json.loads(cluster.sql(engine, f"""
SELECT json_build_object('id',id,'token',lease_token)
FROM mesh_jobs.jobs WHERE namespace='{namespace}' AND args->>'key'='interrupted'
"""))
            for field in ("id", "token"):
                if not re.fullmatch(r"[0-9a-f-]{36}", claim.get(field) or ""):
                    raise FixtureError(f"missing Mesh claim {field} at fault barrier")
        os.kill(process.pid, signal.SIGKILL)
        process.wait(timeout=10)
        if process.stdin is not None:
            process.stdin.close()
        replacement = subject.run(env, 1, evidence / "replacement-runner.log")
        wait_for(lambda: runner_ready([replacement], [evidence / "replacement-runner.log"]),
                 deadline, "replacement runner readiness")
        wait_for(lambda: cluster.observe(engine, namespace)["terminal"] == 1,
                 deadline, "recovered settlement")
        rows = cluster.rows(engine, namespace)
        if len(rows) != 1:
            raise QueueMismatch("rollback or recovery left unexpected durable jobs")
        row = rows[0]
        if (row["job_key"], row["state"], row["attempts"], row["effects"], row["effect_keys"]) != (
                "interrupted", "completed", 2, 1, 1):
            raise QueueMismatch(f"recovery did not reconcile: {row}")
        product = cluster.sql(engine, f"""
SELECT request_id FROM public.mesh_jobs_parity_product
WHERE engine = '{engine}' AND namespace = '{namespace}'
""")
        if product != "interrupted":
            raise QueueMismatch("unexpected business writes after rollback and recovery")
        stale_ack_rejected = None
        if engine == "mesh":
            applied = cluster.sql(engine, f"""
SELECT mesh_jobs.settle('{namespace}', '{claim['id']}', '{claim['token']}',
    '{{"action":"complete"}}'::jsonb, false)
""")
            stale_ack_rejected = applied == "f" and cluster.rows(engine, namespace) == rows
            if not stale_ack_rejected:
                raise QueueMismatch("the original Mesh token could change durable state after recovery")
        write_json(evidence / "fault.json", {
            "engine": engine, "before_signal": before, "after_recovery": row,
            "fault": "runner SIGKILL after committed effect and before settlement",
            "enqueue_commit_ms": committed, "rollback_commit_ms": rollback_timestamp,
            "stale_ack_rejected": stale_ack_rejected,
        })
        completed = True
    finally:
        if replacement is not None:
            stop_runners([replacement], require_clean=completed)
        if process.returncode is None:
            stop_runners([process], require_clean=False)


def run_settlement_fault(subject, scratch, evidence, settings, placement):
    from lib.jobs_parity_fault import SettlementProxy

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
    with SettlementProxy(cluster.port, matches, placement) as proxy:
        process = subject.run(dict(env, JOBS_PARITY_DB_URL=cluster.url(engine, port=proxy.port)), 0, log)
        try:
            deadline = time.monotonic() + settings["trial_timeout_seconds"]
            wait_for(lambda: runner_ready([process], [log]), deadline, "settlement-fault runner")
            _, committed = subject.enqueue(env, input_file, evidence / "enqueue.log")
            if committed is None:
                raise QueueMismatch("settlement-fault enqueue did not commit")
            fault = wait_for(lambda: proxy.fault, deadline, f"{placement} transport cut")
            wait_for(lambda: cluster.observe(engine, namespace)["terminal"] == 1,
                     deadline, f"settlement following {placement} cut")
            rows = cluster.rows(engine, namespace)
            accepted_attempt_counts = {1} if placement == "after_commit" else {1, 2}
            if (len(rows) != 1 or rows[0]["state"] != "completed"
                    or rows[0]["attempts"] not in accepted_attempt_counts
                    or rows[0]["attempt"] != rows[0]["attempts"]
                    or rows[0]["effects"] != 1 or rows[0]["effect_keys"] != 1):
                raise QueueMismatch(f"unexpected durable rows after {placement}: {rows}")
            write_json(evidence / "fault.json", {"engine": engine, "fault": fault, "rows": rows})
            completed = True
        finally:
            stop_runners([process], require_clean=completed)


def build_mesh(ref, scratch, evidence, env, release):
    source = scratch / "mesh-source"
    source.mkdir()
    archive = scratch / "mesh-source.tar"
    with archive.open("wb") as stream:
        subprocess.run(["git", "archive", ref], cwd=ROOT, stdout=stream, check=True)
    checked(["tar", "-xf", str(archive), "-C", str(source)])
    archive.unlink()
    cargo_target = EVIDENCE / "build" / ref
    build = cargo_target / ("release" if release else "debug")
    rust_target = next(line.removeprefix("host: ") for line in checked(["rustc", "-vV"]).splitlines()
                       if line.startswith("host: "))
    options = ["--release"] if release else []
    compiler_env = dict(env, CARGO_TARGET_DIR=str(cargo_target))
    checked(["cargo", "build", "--locked", "-p", "meshc", "-p", "mesh-rt", *options],
            cwd=source, env=compiler_env, timeout=2400, log=evidence / "meshc-build.log")
    project = scratch / "mesh-workload"
    shutil.copytree(MESH_SOURCE, project)
    checked(["cc", "-std=c11", "-O2", "-c", "worker_identity.c", "-o", "worker_identity.o"],
            cwd=project, env=env, log=evidence / "mesh-native-compile.log")
    checked(["ar", "rcs", "libworker_identity.a", "worker_identity.o"], cwd=project, env=env)
    manifest = (project / "mesh.toml.in").read_text().replace("@TARGET@", rust_target)
    (project / "mesh.toml").write_text(manifest.replace("@SHA256@", digest(project / "libworker_identity.a")))
    executable = scratch / "mesh-parity-fixture"
    checked([str(build / "meshc"), "build", str(project), "--target", rust_target,
             "--output", str(executable), "--opt-level", "2" if release else "0"], cwd=source,
            env=dict(env, MESH_RT_LIB_PATH=str(build / "libmesh_rt.a")),
            timeout=180, log=evidence / "mesh-fixture-build.log")
    return executable


def compile_oban(scratch, evidence, env):
    mix_state = EVIDENCE / "build" / "oban"
    mix_state.mkdir(parents=True, exist_ok=True)
    env = dict(env, MIX_ENV="prod", MIX_HOME=str(mix_state / "mix"), HEX_HOME=str(mix_state / "hex"),
               MIX_BUILD_PATH=str(mix_state / "build"), MIX_DEPS_PATH=str(mix_state / "deps"),
               ERL_CRASH_DUMP=str(scratch / "erl_crash.dump"))
    if not list((mix_state / "mix").glob("archives/hex-*")):
        checked(["mix", "local.hex", "--force"], cwd=OBAN, env=env, log=evidence / "hex-bootstrap.log")
    lock_hash = digest(OBAN / "mix.lock")
    checked(["mix", "deps.get", "--check-locked"], cwd=OBAN, env=env,
            log=evidence / "oban-setup.log")
    checked(["mix", "compile"], cwd=OBAN, env=env, timeout=300, log=evidence / "oban-compile.log")
    if digest(OBAN / "mix.lock") != lock_hash:
        raise FixtureError("Mix modified the pinned lockfile")
    command = ["mix", "run", "--no-start", "--no-compile", "fixture.exs", "--"]
    try:
        versions = json.loads(checked([*command, "versions"], cwd=OBAN, env=env).splitlines()[-1])
    except (ValueError, IndexError) as error:
        raise FixtureError("Oban fixture did not report version metadata") from error
    if versions.get("oban") != "2.24.2":
        raise FixtureError("Oban version differs from pin")
    return command, env, versions


def selection(args, settings):
    workloads = args.workload or list(settings["workloads"])
    runners = [args.runners] if args.runners else settings["runner_counts"]
    slots = [args.slots] if args.slots else settings["total_slots"]
    return [(name, r, s) for name in workloads for r in runners for s in slots if s >= r]


def profile_summary(trials, workload, settings, runners, slots):
    keys = ("jobs_per_second", "pickup_p99_ms", "sql_calls_per_job", "claim_calls_per_job", "idle_queries_per_minute",
            "cpu_seconds_per_job", "peak_rss_bytes")
    summary = {}
    for engine in ("mesh", "oban"):
        values = [trial for trial in trials if trial["engine"] == engine]
        summary[engine] = {
            key: {"min": min(series), "median": statistics.median(series), "max": max(series)}
            for key in keys if (series := [trial[key] for trial in values])
        }
    limits = {}
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
        for engine in ("mesh", "oban"):
            observed = max(row[metric] for row in trials if row["engine"] == engine)
            summary[engine][metric] = {"observed": observed, "limit": limits[metric],
                                       "within_limit": observed <= limits[metric]}
    if budget := workload.get("pickup_p99_budget_ms"):
        for engine in ("mesh", "oban"):
            summary[engine]["pickup_p99_ms"]["budget"] = budget
            summary[engine]["pickup_p99_ms"]["within_budget"] = (
                summary[engine]["pickup_p99_ms"]["max"] <= budget
            )
    return summary


@contextlib.contextmanager
def workspace(evidence):
    scratch = Path(tempfile.mkdtemp(prefix="work-", dir=evidence))
    try:
        yield scratch
    finally:
        if (scratch / "pgdata/postmaster.pid").is_file():
            raise FixtureError(f"PostgreSQL did not confirm shutdown. Preserved {scratch} for recovery.")
        shutil.rmtree(scratch)


def record(args):
    settings = json.loads(CONFIG.read_text())
    profiles = [(args.workload[0] if args.workload else "short", 1, 1)] if args.smoke else selection(args, settings)
    if not profiles:
        raise FixtureError("no valid profiles for the chosen runner counts and slots")
    if args.faults:
        profiles = []
    ref = checked(["git", "rev-parse", "--verify", "--end-of-options", f"{args.mesh_ref}^{{commit}}"])
    base = EVIDENCE / ref
    base.mkdir(parents=True, exist_ok=True)
    lane = "fault" if args.faults else "smoke" if args.smoke else "load"
    evidence = Path(tempfile.mkdtemp(prefix=f"j1-{lane}-", dir=base))
    write_json(evidence / "settings.json", settings)
    print(evidence, flush=True)
    try:
        with workspace(evidence) as scratch:
            env = dict(os.environ, TMPDIR=str(scratch), TZ="UTC", LC_ALL="C",
                       MESH_SCHEDULER_MIN_WORKERS=str(settings["runner_schedulers"]),
                       MESH_SCHEDULER_MAX_WORKERS=str(settings["runner_schedulers"]),
                       JOBS_PARITY_MESH_BATCH="1" if args.mesh_batch else "0",
                       ERL_FLAGS=settings["oban_erl_flags"])
            env.pop("MESH_RT_LIB_PATH", None)
            env.pop("RUSTC_WRAPPER", None)
            env.pop("RUSTC_WORKSPACE_WRAPPER", None)
            executable = build_mesh(ref, scratch, evidence, env, release=args.load)
            command, mix_env, versions = compile_oban(scratch, evidence, env)
            max_connections = max((s + r * settings["connection_slots_per_runner"]
                                   for _, r, s in profiles), default=5)
            cluster = Cluster(scratch, evidence, max_connections, args.postgres_version)
            try:
                cluster.start()
                pg_version = cluster.sql("postgres", "SHOW server_version_num")
                actual_settings = cluster.sql("postgres", """
SELECT json_object_agg(name, setting) FROM pg_settings
WHERE name IN ('server_version','server_version_num','shared_buffers','max_connections',
               'track_commit_timestamp','synchronous_commit','pg_stat_statements.track',
               'timezone','fsync','full_page_writes')
""")
                fixture_files = [*MESH_SOURCE.iterdir(), OBAN / "mix.lock", OBAN / "mix.exs",
                                 OBAN / "fixture.exs", *sorted((OBAN / "lib").glob("*.ex"))]
                write_json(evidence / "manifest.json", {
                    "mesh_sha": ref, "mesh_executable_sha256": digest(executable), **versions,
                    "git_head": checked(["git", "rev-parse", "HEAD"]),
                    "postgres_settings": json.loads(actual_settings), "postgres_version_num": pg_version,
                    "host": platform.platform(), "logical_cpus": os.cpu_count(),
                    "load_average_at_start": os.getloadavg(),
                    "session_transport": "SCRAM with verified TLS" if args.postgres_version else "SCRAM",
                    "fixture_hashes": {str(path.relative_to(ROOT)): digest(path) for path in fixture_files
                                       if path.is_file()},
                    "reference_shas": {name: checked(["git", "rev-parse", name]) for name in REFERENCE},
                    "driver_sha256": digest(Path(__file__)), "seed": settings["seed"], "profiles": profiles,
                    "mesh_build_profile": "release" if args.load else "debug",
                    "mesh_batch": args.mesh_batch,
                    "mode": lane, "claim_timestamps": {"mesh": "database attempted_at_ms",
                                                       "oban": "Basic engine attempted_at"},
                    "settlement_timestamp": "durable commit timestamp of the terminal row version",
                    "resource_scope": "runner process trees; sampled RSS, processes, threads and connections",
                })
                subjects = {
                    "mesh": Subject("mesh", [str(executable)], ROOT, cluster, env),
                    "oban": Subject("oban", command, OBAN, cluster, mix_env),
                }
                if args.faults:
                    for engine, subject in subjects.items():
                        case_evidence = evidence / engine
                        case_evidence.mkdir()
                        case_scratch = scratch / engine
                        case_scratch.mkdir()
                        run_fault(subject, case_scratch, case_evidence, settings)
                        if not cluster.expected_version:
                            for placement in ("before_commit", "after_commit"):
                                placement_evidence = case_evidence / placement
                                placement_evidence.mkdir()
                                placement_scratch = case_scratch / placement
                                placement_scratch.mkdir()
                                run_settlement_fault(subject, placement_scratch, placement_evidence,
                                                     settings, placement)
                for name, runners, slots in profiles:
                    profile = f"{name}-{runners}r-{slots}s"
                    case_evidence = evidence / profile
                    case_evidence.mkdir()
                    workload = settings["workloads"][name]
                    allocation = allocate_slots(runners, slots)
                    cap = slots + runners * settings["connection_slots_per_runner"]
                    cluster.sql("postgres", f"ALTER ROLE parity_subject CONNECTION LIMIT {cap}")
                    trials = []
                    pair_count = 1 if args.smoke else settings["trial_pairs"]
                    for pair in range(-1, pair_count):
                        batches = inputs(workload, slots, settings["seed"], smoke=args.smoke or pair < 0)
                        order = ("mesh", "oban") if pair % 2 == 0 else ("oban", "mesh")
                        for engine in order:
                            run_name = f"{pair if pair >= 0 else 'warmup'}-{engine}"
                            run_evidence = case_evidence / run_name
                            run_evidence.mkdir()
                            run_scratch = scratch / f"{profile}-{run_name}"
                            run_scratch.mkdir()
                            result = run_trial(subjects[engine], allocation, workload, batches, run_scratch,
                                               run_evidence, settings, pair)
                            write_json(run_evidence / "metrics.json", result)
                            if pair >= 0:
                                trials.append(result)
                    validate_pairs(trials, pair_count)
                    write_json(case_evidence / "trials.json", trials)
                    write_json(case_evidence / "summary.json",
                               profile_summary(trials, workload, settings, runners, slots))
            finally:
                cluster.stop()
    except (FixtureError, QueueMismatch, OSError, subprocess.SubprocessError) as error:
        kind = "queue-mismatch" if isinstance(error, QueueMismatch) else "fixture-error"
        write_json(evidence / "result.json", {"status": kind, "reason": str(error)})
        raise
    write_json(evidence / "result.json", {"status": "passed", "mode": lane,
                                         "five_paired_trials": lane == "load"})


def self_check():
    import unittest

    class DriverChecks(unittest.TestCase):
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
                   "effects": 1, "effect_keys": 1, "claimed_ms": 99, "entered_ms": 103,
                   "settled_ms": 125, "payload_sha256": hashlib.sha256(b"test").hexdigest(),
                   "hold_ms": 0, "fail_first": False}
            expected = {"a": ("batch", {"payload": "test", "hold_ms": 0, "fail_first": False})}
            result = account([row], expected, {"batch": 100}, 5)
            self.assertEqual(result[0]["pickup_ms"], 3)
            self.assertEqual(result[0]["commit_to_claim_ms"], 0)
            self.assertEqual(result[0]["commit_to_settlement_ms"], 25)
            self.assertEqual(percentile(list(range(1, 101)), 0.99), 99)
            with self.assertRaises(QueueMismatch):
                account([], expected, {"batch": 100}, 5)
            with self.assertRaises(QueueMismatch):
                account([dict(row, hold_ms=1)], expected, {"batch": 100}, 5)
            with self.assertRaises(FixtureError):
                account([dict(row, entered_ms=80)], expected, {"batch": 100}, 5)
            with self.assertRaises(FixtureError):
                percentile([], 0.99)

        def test_trial_validation(self):
            with self.assertRaisesRegex(FixtureError, "incomplete"):
                validate_pairs([], 5)
            rows = [{"pair": pair, "engine": engine, "input_sha256": "shared"}
                    for engine in ("mesh", "oban") for pair in range(5)]
            with self.assertRaisesRegex(FixtureError, "metric|jobs_per_second"):
                validate_pairs(rows, 5)
            rows[-1]["input_sha256"] = "mismatch"
            with self.assertRaisesRegex(FixtureError, "different inputs"):
                validate_pairs(rows, 5)

    suite = unittest.defaultTestLoader.loadTestsFromTestCase(DriverChecks)
    return unittest.TextTestRunner(verbosity=2).run(suite).wasSuccessful()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--self-check", action="store_true")
    mode.add_argument("--smoke", action="store_true", help="small integration probe, not a benchmark")
    mode.add_argument("--load", action="store_true", help="five alternating trials per selected profile")
    mode.add_argument("--faults", action="store_true", help="owned runner-death and rollback probe")
    parser.add_argument("--mesh-ref", default="HEAD", help="Git commit from which to compile Mesh")
    parser.add_argument("--mesh-batch", action="store_true", help="enable batch claims for the Mesh runner")
    parser.add_argument("--postgres-version", metavar="MAJOR.PATCH",
                        help="require exact server version and SCRAM over verified TLS")
    settings = json.loads(CONFIG.read_text())
    parser.add_argument("--workload", action="append", choices=list(settings["workloads"]))
    parser.add_argument("--runners", type=int, choices=settings["runner_counts"])
    parser.add_argument("--slots", type=int, choices=settings["total_slots"])
    args = parser.parse_args()
    if args.self_check:
        return 0 if self_check() else 1
    if args.postgres_version and not POSTGRES_VERSION.fullmatch(args.postgres_version):
        parser.error("--postgres-version requires MAJOR.PATCH")
    if not args.load and (args.runners is not None or args.slots is not None):
        parser.error("--runners and --slots require --load")
    if args.workload and (args.faults or (args.smoke and len(args.workload) > 1)):
        parser.error("select one workload for --smoke or use --load for several")

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
