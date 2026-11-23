"""Run a single stock Apalis/PostgreSQL arm against the frozen Jobs fixture."""

import argparse
from collections import Counter
from contextlib import ExitStack, suppress
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import select
import signal
import shutil
import subprocess
import sys
import tempfile
import time
import traceback

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
from verify import PROFILES, database_check, reconcile, require
ROOT = HERE.parents[2]
spec = importlib.util.spec_from_file_location("jobs_parity", ROOT / "scripts/jobs-parity.py")
parity = importlib.util.module_from_spec(spec)
spec.loader.exec_module(parity)

WORK = ROOT / "target/linode-apalis"
VERSION = "apalis 1.2.0 / apalis-postgres 1.2.0"
ENGINE = "apalis"
QUEUE = "parity"
SETTINGS = json.loads(parity.CONFIG.read_text())


class Cluster(parity.Cluster):
    def __init__(self, scratch, cap):
        super().__init__(scratch, scratch, cap, "17.11")

    def configure_tls(self):
        super().configure_tls()
        with (self.data / "postgresql.conf").open("a") as stream:
            stream.write(f"max_connections = {self.connection_cap}\n"
                         "superuser_reserved_connections = 0\nreserved_connections = 0\n"
                         "fsync = on\nfull_page_writes = on\n")

    def prepare(self):
        self.prepare_database(ENGINE)
        self.sql("postgres", "GRANT pg_read_all_stats TO parity_producer; "
                             "ALTER ROLE parity_producer CONNECTION LIMIT 1")
        self.sql(ENGINE, """
            CREATE EXTENSION pg_stat_statements;
            ALTER TABLE public.mesh_jobs_parity_attempts ADD COLUMN job_id text NOT NULL;
            ALTER TABLE public.mesh_jobs_parity_effects ADD COLUMN job_id text NOT NULL;
        """)


class Session:
    def __init__(self, process, deadline):
        self.process = process
        self.deadline = deadline

    def send(self, command):
        self.process.stdin.write(json.dumps(command, separators=(",", ":")).encode() + b"\n")
        self.process.stdin.flush()

    def receive(self, on_wait=None):
        while True:
            remaining = self.deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("Apalis producer/observer deadline expired")
            timeout = min(remaining, SETTINGS["observer_interval_ms"] / 1000) if on_wait else remaining
            if select.select([self.process.stdout], [], [], timeout)[0]:
                line = self.process.stdout.readline()
                require(line.endswith(b"\n"), "producer/observer exited or sent an incomplete line")
                return line
            if on_wait:
                on_wait()

    def query(self, verb):
        self.send({"verb": verb})
        return json.loads(self.receive())


def linux_process_table():
    rows = {}
    for path in Path("/proc").glob("[0-9]*/stat"):
        try:
            fields = path.read_text().rsplit(")", 1)[1].split()
        except (FileNotFoundError, ProcessLookupError, PermissionError):
            continue
        if fields[0] != "Z":
            rows[int(path.parent.name)] = (int(fields[1]), int(fields[21]) * os.sysconf("SC_PAGE_SIZE") // 1024,
                                           fields[19])
    return rows


if platform.system() == "Linux":
    parity.process_table = linux_process_table


def smoke_batches():
    return [
        {"request_id": request, "rollback": rollback, "jobs": [
            {"key": request, "payload": "a5" * 512, "hold_ms": 1, "fail_first": fail},
        ]}
        for request, rollback, fail in (
            ("rollback", True, False), ("success", False, False),
            ("retry", False, True), ("exhaust", False, True),
        )
    ]


def check_writes(jobs, transactions, writes, pids):
    by_id = {row["id"]: row["job_key"] for row in jobs}
    expected_ids = {identity: key for tx in transactions for key, identity in tx["enqueued_ids"].items()}
    require(by_id == expected_ids, "native IDs and enqueue receipts differ")
    require(
        Counter((row["engine"], row["namespace"], row["request_id"], row["xid"], row["committed_us"])
                for row in writes["product"])
        == Counter((ENGINE, QUEUE, tx["request_id"], tx["xid"], tx["committed_us"]) for tx in transactions),
        "product rows and enqueue receipts differ",
    )
    for category, has_attempt in (("attempts", True), ("effects", False)):
        identities = Counter()
        for row in writes[category]:
            key = by_id.get(row["job_id"])
            require(row["engine"] == ENGINE and row["namespace"] == QUEUE and key is not None
                    and row["job_key"] == row["effect_key"] == key, "unexpected application write identity")
            if has_attempt:
                require(row["worker_pid"] in pids, "unknown runner PID made an application write")
            identities[(row["job_id"], row.get("attempt"))] += 1
        expected = Counter((job["id"], number) for job in jobs
                           for number in (range(1, job["attempt"] + 1) if has_attempt else [None]))
        require(identities == expected, f"incorrect durable {category}")


def check_smoke(jobs, transactions, rollback, writes):
    expected = {"success": ("Done", 1), "retry": ("Done", 2), "exhaust": ("Failed", 1)}
    require({row["job_key"]: (row["native_state"], row["attempt"]) for row in jobs} == expected,
            "unexpected smoke settlement")
    require(rollback["committed_us"] is None and rollback["request_id"] not in
            {row["request_id"] for row in writes["product"]}, "rollback survived")
    require(not set(rollback["enqueued_ids"].values()) & {row["id"] for row in jobs},
            "a rolled-back task became visible")
    commits = {key: tx["committed_us"] for tx in transactions for key in tx["enqueued_ids"]}
    for job in jobs:
        key = job["job_key"]
        require(job["attempt"] == job["attempts"] and job["effects"] == 1
                and job["attempt_xid"] != job["effect_xid"]
                and commits[key] <= job["entered_us"] <= job["attempt_committed_us"]
                < job["effect_committed_us"] <= job["settled_us"], f"{key}: invalid smoke write proof")
        if key == "exhaust":
            require(job["max_attempts"] == 1 and "fixture_fail_first" in json.dumps(job["last_result"]),
                    "native failure result missing")


def run_arm(profile, output, binary, cluster, driver_profile, build):
    runners, slots, cap, frozen, _ = PROFILES[driver_profile]
    workload = SETTINGS["workloads"][driver_profile]
    smoke = profile == "smoke"
    sparse = workload["mode"] == "sparse"
    batches = smoke_batches() if smoke else parity.inputs(workload, slots, SETTINGS["seed"])
    paths = parity.batch_files(cluster.scratch, batches)
    hashes = [parity.digest(path) for path in paths]
    input_sha = hashlib.sha256("".join(hashes).encode()).hexdigest()
    require(smoke or input_sha == frozen, "frozen input hash does not match parity")
    parity.write_json(output / "input.json", {"input_sha256": input_sha, "batch_digests": dict(zip(
        (batch["request_id"] for batch in batches), hashes, strict=True)), "profile": profile})

    worker_env = dict(cluster.env, APALIS_BENCH_DB_URL=cluster.url(ENGINE))
    producer_env = dict(cluster.env, APALIS_BENCH_DB_URL=cluster.url(ENGINE, "parity_producer"))
    parity.checked([str(binary), "migrate"], env=worker_env, log=cluster.scratch / "migrate.log")
    cluster.sql("postgres", "SELECT pg_stat_statements_reset()")
    deadline = time.monotonic() + SETTINGS["trial_timeout_seconds"]
    interval = SETTINGS["observer_interval_ms"] / 1000
    allocation = parity.allocate_slots(runners, slots)
    names = [f"runner-{index}" for index in range(runners)]
    processes, samples, idle_samples, transactions = [], [], [], []
    start = time.monotonic()
    rollback = None
    success = False

    def check_processes():
        # Do not reap runners here. wait4 collects CPU at shutdown.
        for proc in processes:
            require(os.waitid(os.P_PID, proc.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT) is None,
                    f"runner {proc.pid} exited unexpectedly")

    with ExitStack() as files:
        broker_log = files.enter_context((cluster.scratch / "broker-errors.log").open("xb"))
        process = subprocess.Popen([str(binary), "session"], env=producer_env,
                                   stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=broker_log)
        session = Session(process, deadline)

        def sample():
            check_processes()
            captured = dict(offset_s=time.monotonic() - start, host_load_average_1m=os.getloadavg()[0],
                            **session.query("sample"), **parity.process_sample(processes))
            require(captured["all_tls"] is True, "runner or producer connection lacks TLS")
            require(captured["processes"] == runners, "runner process sample is incomplete")
            return captured

        def settled(count):
            observed = sample()
            samples.append(observed)
            require(smoke or not observed["failed"], "Apalis recorded an unexpected terminal failure")
            require(observed["terminal"] <= count, "Apalis settled an unexpected task")
            return observed["terminal"] == count

        try:
            require(session.receive() == b"SESSION_READY\n", "Apalis producer/observer did not start")
            database = session.query("database")
            parity.write_json(output / "database.json", database)
            database_check(database, cap)
            poll_ms = SETTINGS["poll_ms"]
            for name, count in zip(names, allocation, strict=True):
                log = files.enter_context((cluster.scratch / f"{name}.log").open("xb"))
                processes.append(parity.RunnerProcess(
                    [str(binary), "worker", name, str(count), str(poll_ms)], env=worker_env,
                    stdin=subprocess.PIPE, stdout=log, stderr=log, start_new_session=True,
                ))

            def ready():
                check_processes()
                status = session.query("sessions")
                listeners = Counter(item["application_name"] for item in status["sessions"] if item["listener"])
                return status if sorted(status["workers"]) == sorted(names) and listeners == {
                    f"apalis:{name}": 1 for name in names
                } else None

            readiness = parity.wait_for(ready, min(deadline, time.monotonic() + 45), "Apalis LISTEN readiness")
            parity.write_json(output / "ready.json", readiness)
            before = session.query("counters")
            idle_samples.append(sample())
            time.sleep(SETTINGS["idle_seconds"])
            idle = session.query("counters")
            idle_samples.append(sample())
            start = time.monotonic()
            expected_count = 0

            def observe_busy_producer():
                # The same session cannot serve SQL while the transaction is in flight.
                # Capture runner resources without submitting a SQL request or delaying its commit.
                check_processes()
                samples.append(dict(offset_s=time.monotonic() - start,
                                    host_load_average_1m=os.getloadavg()[0],
                                    **parity.process_sample(processes)))

            for path, batch in zip(paths, batches, strict=True):
                max_attempts = 1 if smoke and batch["request_id"] == "exhaust" else 3
                submitted_us = time.time_ns() // 1000
                session.send({"verb": "enqueue", "path": str(path), "max_attempts": max_attempts})
                receipt = json.loads(session.receive(observe_busy_producer))
                returned_us = time.time_ns() // 1000
                receipt.update(submitted_us=submitted_us, returned_us=returned_us)
                require(receipt["request_id"] == batch["request_id"]
                        and set(receipt["enqueued_ids"]) == {job["key"] for job in batch["jobs"]},
                        "enqueue receipt does not describe the requested batch")
                if batch["rollback"]:
                    rollback = receipt
                    require(sample()["total"] == 0, "rolled-back tasks reached the queue")
                    continue
                transactions.append(receipt)
                stamped = receipt["committed_us"]
                require(stamped is not None and submitted_us <= stamped <= returned_us,
                        "PostgreSQL producer commit is outside the submitted/returned window")
                expected_count += len(batch["jobs"])
                if sparse:
                    parity.wait_for(lambda: settled(expected_count), deadline, batch["request_id"], interval)
                    time.sleep(poll_ms / 1000)
            parity.wait_for(lambda: settled(expected_count), deadline, "native durable settlements", interval)
            after = session.query("counters")
            parity.write_json(output / "counters.json", {"before_idle": before, "after_idle": idle, "after": after})
            require(before["dealloc"] == idle["dealloc"] == after["dealloc"],
                    "PostgreSQL evicted statement counters")
            jobs = session.query("jobs")
            parity.write_json(output / "raw-jobs.json", jobs)
            writes = session.query("writes")
            parity.write_json(output / "raw-writes.json", writes)
            final_sessions = session.query("sessions")
            parity.write_json(output / "final-sessions.json", final_sessions)
            check_writes(jobs, transactions, writes, {proc.pid for proc in processes})
            require(all(row["job_type"] == QUEUE for row in jobs), "unexpected Apalis queue")
            success = True
        finally:
            try:
                cpu = parity.stop_runners(processes, require_clean=success)
            finally:
                parity.write_json(output / "samples.json", {"idle": idle_samples, "timed": samples})
                parity.write_json(output / "transactions.json", {"committed": transactions, "rollback": rollback})
                if process.poll() is None:
                    with suppress(BrokenPipeError):
                        session.send({"verb": "stop"})
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait()
                process.stdin.close()
                process.stdout.close()
            if success:
                require(process.returncode == 0, "producer/observer did not exit cleanly")

    errors = [line for name in names for line in (cluster.scratch / f"{name}.log").read_text().splitlines()]
    if smoke:
        check_smoke(jobs, transactions, rollback, writes)
        require(errors == ["handler: fixture_fail_first"] * 2, "unexpected Apalis smoke worker errors")
        parity.write_json(output / "smoke.json", {"verified": True, "completed": 2, "failed": 1,
                                                  "rolled_back": 1, "retried": 1})
        return

    require(not errors, "runner logged an unexpected error")
    all_samples = idle_samples + samples
    ledger = dict(engine="rust", profile=profile, version=VERSION, artifact_sha256=build["artifact_sha256"],
                  input_sha256=input_sha, runners=runners, slots=slots, database=database,
                  runner_cpu_seconds=cpu, peak_executing=max(p.get("executing", 0) for p in all_samples),
                  peak_connections=max(p.get("connections", 0) for p in all_samples),
                  peak_runner_rss_bytes=max(p["rss_bytes"] for p in all_samples),
                  jobs=jobs, transactions=transactions)
    metrics = reconcile(ledger, "rust", profile,
                        {"artifact_sha256": build["artifact_sha256"], "version": VERSION})
    parity.write_json(output / "metrics.json", metrics)
    parity.write_json(output / "ledger.json", ledger)


def run(args, output):
    binary = args.binary.resolve(strict=True)
    build = json.loads(binary.with_name("build.json").read_text())
    require(build["artifact_sha256"] == parity.digest(binary), "executed artifact differs from build manifest")
    for name, checksum in build["source_sha256"].items():
        require(checksum == parity.digest(HERE / name), f"source differs from locked build: {name}")
    dependencies = binary.with_name("dependencies.txt")
    require(build["cargo_tree_sha256"] == parity.digest(dependencies), "dependency tree differs from build manifest")
    parity.write_json(output / "build.json", build)
    shutil.copyfile(dependencies, output / "dependencies.txt")
    profile = "sparse" if args.profile == "smoke" else args.profile
    cap = PROFILES[profile][2]
    if args.pg_bin:
        os.environ["PG_BIN_DIR"] = str(args.pg_bin.resolve(strict=True))
    scratch_root = args.scratch.resolve()
    scratch_root.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="arm-", dir=scratch_root) as location:
        cluster = Cluster(Path(location), cap)
        try:
            cluster.start()
            cluster.prepare()
            parity.write_json(output / "runtime.json", {
                "phase": os.environ.get("DURABLE_JOBS_PHASE", "diagnostic"),
                "platform": platform.platform(), "architecture": platform.machine(),
                "cpu_affinity": sorted(os.sched_getaffinity(0)) if hasattr(os, "sched_getaffinity") else None,
                "load_before": os.getloadavg(), "runner_tokio_threads": 2, "runner_pool_size": "slots + 2",
                "queue": QUEUE, "queue_config": "lock_tasks + persist_results + LISTEN + 250 ms fallback",
                "pg_tls": "verify-full", "pg_ca_sha256": parity.digest(cluster.certificate),
                "versions": {"apalis": "1.2.0", "apalis-postgres": "1.2.0"},
                "fixture_sha256": {str(path.relative_to(ROOT)): parity.digest(path) for path in
                                   (Path(spec.origin), parity.CONFIG, HERE.parent / "verify.py")},
            })
            run_arm(args.profile, output, binary, cluster, profile, build)
        except Exception:
            (output / "failure.log").write_text(traceback.format_exc().replace(cluster.password, "[redacted]"))
            with suppress(OSError, ValueError, AttributeError, parity.FixtureError, subprocess.TimeoutExpired):
                producer_env = dict(cluster.env, APALIS_BENCH_DB_URL=cluster.url(ENGINE, "parity_producer"))
                snapshot = parity.checked([str(binary), "snapshot"], env=producer_env, timeout=10,
                                          log=cluster.scratch / "failure-snapshot.log")
                parity.write_json(output / "failure-snapshot.json", json.loads(snapshot.splitlines()[-1]))
            raise RuntimeError(f"Apalis arm failed. See {output / 'failure.log'}") from None
        finally:
            try:
                cluster.stop()
            finally:
                for log in cluster.scratch.glob("*.log"):
                    # PostgreSQL may print a statement on setup errors.
                    (output / log.name).write_text(log.read_text(errors="replace").replace(
                        cluster.password, "[redacted]"))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("profile", choices=[*PROFILES, "smoke"])
    parser.add_argument("output", type=Path)
    parser.add_argument("--binary", type=Path, default=WORK / "build/release/durable-jobs-apalis-bench")
    parser.add_argument("--scratch", type=Path, default=WORK / "scratch")
    parser.add_argument("--pg-bin", type=Path, help="directory with PostgreSQL 17.11 tools")
    args = parser.parse_args()
    if os.getuid() == 0:
        parser.error("run as the existing unprivileged benchmark user")
    output = args.output.resolve()
    os.umask(0o077)
    output.mkdir(parents=True, exist_ok=True)
    require(not any(output.iterdir()), "use a new or empty evidence directory")
    signal.signal(signal.SIGTERM, lambda *_: exit_for_signal())
    try:
        run(args, output)
    except Exception as error:
        if not (output / "failure.log").exists():
            (output / "failure.log").write_text(traceback.format_exc())
        parser.exit(1, f"{error}\n")


def exit_for_signal():
    raise RuntimeError("Apalis arm interrupted by SIGTERM")


if __name__ == "__main__":
    main()
