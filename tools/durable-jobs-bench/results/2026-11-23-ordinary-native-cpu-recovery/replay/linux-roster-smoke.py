"""Reproduce native-02's roster failures against real PostgreSQL 17.11."""
from collections import Counter
from contextlib import ExitStack
import json
import os
from pathlib import Path
import secrets
import select
from statistics import median
import subprocess
import sys
import time

from collector import PROC, process_clock, publish_registry, verify_roster
from dynamic_roster import Roster, SessionReader

ROOT = Path("/roster")
PG = Path("/jobs-parity/postgres-17.11")
WORK = Path("/fixture")
PYTHON = "/usr/bin/python3"
AVAILABLE_CPUS = sorted(os.sched_getaffinity(0))
CPUS = set(AVAILABLE_CPUS[:2])
SPARES = AVAILABLE_CPUS[2:]
FRAME_NS = int((ROOT / "max-frame-ns").read_text())
CAP = 96
ENV = {
    **os.environ, "PATH": f"{PG / 'bin'}:/bin",
    "LD_LIBRARY_PATH": str(PG / "lib"),
    "LC_ALL": "C", "PGSSLMODE": "verify-full", "PGSSLROOTCERT": str(ROOT / "server.crt"),
}


def require(passed, message):
    if not passed:
        raise RuntimeError(message)


def pg(tool, *args):
    return subprocess.check_output([PG / "bin" / tool, *args], env=ENV, text=True, timeout=30)


def psql(db, user):
    return [str(PG / "bin/psql"), "-X", "-qAtw", "-v", "ON_ERROR_STOP=1",
            "-h", "127.0.0.1", "-p", "55432", "-d", db, "-U", user]


def sql(db, command):
    return subprocess.check_output([*psql(db, "nobody"), "-c", command],
                                   env=ENV, text=True, timeout=10).strip()


class Client:
    def __init__(self, database, role, application=None):
        self.role = role
        env = ENV if application is None else {**ENV, "PGAPPNAME": application}
        self.process = subprocess.Popen(psql(database, role), env=env,
                                        stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        self.process.stdin.write("SELECT pg_backend_pid();\n")
        self.process.stdin.flush()
        self.backend = int(self.process.stdout.readline())

    def close(self):
        self.process.stdin.close()
        self.process.wait(timeout=3)
        self.process.stdout.close()


def accept_client(stack, db, role, application=None):
    client = Client(db, role, application)
    stack.callback(client.close)
    return client


def census(db, evidence):
    data = sql("postgres", f"""SELECT json_agg(json_build_object('role', a.usename, 'backend', a.pid,
        'tls', coalesce(s.ssl, false))) FROM pg_stat_activity a LEFT JOIN pg_stat_ssl s USING(pid)
        WHERE a.datname = '{db}'
        AND a.usename IN ('parity_subject', 'parity_producer', 'compiled_caller');""")
    (evidence / "postgres-tls-sessions.json").write_text(data)
    return Counter(row["role"] for row in json.loads(data))


def expected_failure(fn, reason, label="LEGACY REJECTED"):
    try:
        fn()
    except ValueError as error:
        require(str(error).startswith(reason), f"unexpected old failure: {error}")
        print(label, error, flush=True)
    else:
        raise RuntimeError("legacy implementation accepted the invalid fixture")


def trusted_frame(roster):
    frame = roster.frame()
    for pid in frame["processes"]:
        if not frame["processes"][pid]["role"].startswith("pg_"):
            continue
        first = frame["left"]["rows"][pid]
        last = frame["right"]["rows"][pid]
        require(roster.identity(first) == roster.identity(last) and first[3] is True,
                "classified process lacked the two-ended TLS identity")
    return frame


def fixture(postmaster, engine, jobs, runners, broker):
    evidence = WORK / engine
    (evidence / "process-affinity.json").write_text(json.dumps(
        {str(pid): sorted(CPUS) for pid in [postmaster, *runners]}))
    (evidence / "oban-broker-emulator.json").write_text(json.dumps({"pid": broker}))
    with ExitStack() as sessions:
        accept_client(sessions, engine, "parity_producer")
        app_count = 32 if engine == "mesh" else 0
        for _ in range(app_count):
            accept_client(sessions, engine, "compiled_caller")
        clients = [accept_client(sessions, engine, "parity_subject") for _ in range(jobs)]
        counts = census(engine, evidence)
        require(counts["parity_subject"] == jobs and counts["compiled_caller"] == app_count
                and counts["parity_producer"] == 1, "incorrect live fixture census")
        legacy = lambda: publish_registry(evidence, engine, sorted(CPUS), PYTHON, PROC)
        if engine == "mesh":
            expected_failure(legacy, "unexpected TLS role census")
        else:
            registry = legacy()
            verify_roster(registry, engine, PROC)
            retired = clients[0].backend
            # Close via SQL so program teardown can remain idempotent.
            # Start a replacement authenticated connection while the count stays fixed.
            sql("postgres", f"SELECT pg_terminate_backend({retired}, 2000)")
            replacement = accept_client(sessions, engine, "parity_subject").backend
            require(retired != replacement, "fixture did not change backend PIDs")
            expected_failure(lambda: verify_roster(registry, engine, PROC),
                             "live PostgreSQL client PIDs or role titles differ")
        # The package's proposed observer is isolated from workload roles.
        # A same-named unrelated client has no self-backend witness.
        rogue = accept_client(sessions, engine, "unclassified", "native_role_observer")
        if SPARES:
            os.sched_setaffinity(0, {SPARES[0]})
            sessions.callback(os.sched_setaffinity, 0, CPUS)
        reader = SessionReader(psql(engine, "nobody"), ENV, "native_role_observer")
        sessions.callback(reader.close)
        reader.read(time.monotonic_ns() + 2_000_000_000, CAP)
        roster = Roster(CPUS, postmaster, os.getpid(), runners, broker, CAP, reader, FRAME_NS)
        sessions.callback(roster.close)
        initial = trusted_frame(roster)
        require(initial["unclassified"].keys() == {rogue.backend}, "unknown login received a charged role")
        require(rogue.backend not in initial["processes"], "unknown login entered CPU attribution")
        require(initial["processes"][reader.observer_pid]["role"] == "pg_observer",
                "observer was charged to the workload")
        if engine == "mesh":
            for _ in range(3):
                accept_client(sessions, engine, "parity_subject")
            require(census(engine, evidence)["parity_subject"] == 26, "reverse fixture is missing sessions")
            expected_failure(legacy, "unexpected TLS role census")
            step = trusted_frame(roster)
            require(sum(row["role"] == "pg_jobs" for row in step["processes"].values()) == 26,
                    "additional attached sessions were not observed")
            for _ in range(6):
                accept_client(sessions, engine, "parity_subject")
        else:
            sql("postgres", f"SELECT pg_terminate_backend({clients[1].backend}, 2000)")
            accept_client(sessions, engine, "parity_subject")
        current = trusted_frame(roster)
        expected_jobs = 32 if engine == "mesh" else 80
        observed = Counter(row["role"] for row in current["processes"].values())
        require(observed == Counter({"pg_jobs": expected_jobs, "pg_application": app_count,
                                     "pg_producer": 1, "pg_observer": 1, "runner": 8,
                                     "broker": 1, "postmaster": 1}), f"wrong chargeable identities: {observed}")
        require(len(roster.sessions) == expected_jobs + app_count + 2, "retired pidfd leaked")
        # Retire a session across the two observations of a single frame.
        # Use a warmed connection in a different DB, outside the role census.
        victim = next(pid for pid, row in current["processes"].items() if row["role"] == "pg_jobs")
        terminator = accept_client(sessions, "postgres", "nobody")
        captured_read = reader.read
        calls = 0

        def raced_read(deadline, cap):
            nonlocal calls
            calls += 1
            if calls == 2:
                pipe = terminator.process
                pipe.stdin.write(f"SELECT pg_terminate_backend({victim});\n")
                pipe.stdin.flush()
                require(pipe.stdout.readline().strip() == "t", "retirement was not observed")
                subject = next(item for item in roster.sessions.values() if item.pid == victim)
                remaining = max(0, (deadline - time.monotonic_ns()) / 1e9)
                require(select.select([subject.fd], [], [], remaining)[0],
                        "retired process did not exit during the covered frame")
            return captured_read(deadline, cap)

        reader.read = raced_read
        race = trusted_frame(roster)
        require(victim in race["changed_sessions"] and victim not in race["processes"],
                "between-census retirement charged a PID")
        reader.read = captured_read
        later = trusted_frame(roster)
        require(not any(key[0] == victim for key in roster.sessions), "retired pidfd was retained")
        require(later["processes"].keys() == current["processes"].keys() - {victim},
                "retiring one backend lost stable witnesses")
        active = accept_client(sessions, engine, "parity_subject")
        active.process.stdin.write("SELECT pg_sleep(0.2);\n")
        active.process.stdin.flush()
        proof = None
        for _ in range(8):
            frame = trusted_frame(roster)
            if all(frame[edge]["rows"].get(active.backend, [None] * 5)[4] == "active"
                   for edge in ("left", "right")):
                proof = frame
                break
        require(proof is not None and proof["processes"][active.backend]["role"] == "pg_jobs",
                "live executing query was missing from one census endpoint")
        active.process.stdout.readline()
        frame_ms = []
        pace_ms = []
        cpu_ms = []
        client_cpu_ms = []
        backend_cpu_ms = []
        rejected = []
        for index in range(60):
            start_cpu = time.thread_time_ns()
            start_client = process_clock(reader.process.pid)
            start_backend = process_clock(reader.observer_pid)
            start_wall = time.monotonic_ns()
            try:
                frame = trusted_frame(roster)
                frame_ms.append((frame["end"][2] - frame["begin"][0]) / 1e6)
            except TimeoutError as error:
                rejected.append([index + 1, str(error), reader.failed])
            pace_ms.append((time.monotonic_ns() - start_wall) / 1e6)
            cpu_ms.append((time.thread_time_ns() - start_cpu) / 1e6)
            client_cpu_ms.append((process_clock(reader.process.pid) - start_client) / 1e6)
            backend_cpu_ms.append((process_clock(reader.observer_pid) - start_backend) / 1e6)
            if reader.failed:
                break
            time.sleep(max(0, .02 - pace_ms[-1] / 1000))
        print("DYNAMIC", engine, "uid", os.getuid(), "charges", dict(sorted(observed.items())),
              "accepted_wall_ms_p50_max", ([round(median(frame_ms), 3), round(max(frame_ms), 3)]
                                           if frame_ms else None),
              "recorder_cpu_ms_total", round(sum(cpu_ms), 3),
              "psql_cpu_ms_total", round(sum(client_cpu_ms), 3),
              "observer_backend_cpu_ms_total", round(sum(backend_cpu_ms), 3),
              "attempted_frames", len(pace_ms), "accepted_frames", len(frame_ms),
              "rejected_frames", rejected, flush=True)
        if not reader.failed:
            try:
                reader.read(time.monotonic_ns() - 1, CAP)
            except TimeoutError:
                pass
            else:
                raise RuntimeError("a late SQL snapshot was accepted")
        expected_failure(lambda: reader.read(time.monotonic_ns() + FRAME_NS, CAP),
                         "census connection has an unread response", "DYNAMIC REJECTED")


def main():
    print("PLATFORM", os.uname().machine, os.uname().release, sys.version.split()[0], flush=True)
    require(os.getuid() == 65534 and len(CPUS) == 2, "unprivileged two-CPU workload fixture required")
    print("AFFINITY", "workload", sorted(CPUS), "recorder", SPARES[:1] or sorted(CPUS), flush=True)
    os.sched_setaffinity(0, CPUS)
    data = WORK / "data"
    password = secrets.token_hex(24)
    pwfile = WORK / "pw"
    pwfile.touch(mode=0o600)
    pwfile.write_text(password + "\n")
    try:
        print(pg("initdb", "-D", str(data), "--no-locale", "-A", "scram-sha-256",
                 "-U", "nobody", f"--pwfile={pwfile}").splitlines()[0])
    finally:
        pwfile.unlink()
    ENV["PGPASSWORD"] = password
    (data / "server.crt").write_bytes((ROOT / "server.crt").read_bytes())
    key = data / "server.key"
    key.write_bytes((WORK / "server.key").read_bytes())
    key.chmod(0o600)
    (data / "pg_hba.conf").write_text("hostssl all all 127.0.0.1/32 scram-sha-256\n")
    with (data / "postgresql.conf").open("a") as settings:
        settings.write("listen_addresses = '127.0.0.1'\nport = 55432\n"
                       "unix_socket_directories = ''\nssl = on\nmax_connections = 96\n")
    pg("pg_ctl", "-D", str(data), "-l", str(WORK / "postgres.log"), "-w", "start")
    try:
        print("SERVER", sql("postgres", "SHOW server_version"), flush=True)
        for login in ("parity_producer", "parity_subject", "compiled_caller", "unclassified"):
            sql("postgres", f"CREATE ROLE {login} LOGIN PASSWORD '{password}'")
        for engine in ("mesh", "oban"):
            sql("postgres", f"CREATE DATABASE {engine}")
        with ExitStack() as processes:
            (WORK / "run").write_text("import time\ntime.sleep(60)\n")
            (WORK / "session").write_text("import time\ntime.sleep(60)\n")

            def participant(arg, stdout=None):
                child = subprocess.Popen([PYTHON, arg], cwd=WORK, stdout=stdout)
                processes.callback(child.wait, timeout=3)
                processes.callback(child.terminate)
                return child.pid

            runners = [participant("run") for _ in range(8)]
            postmaster = int((data / "postmaster.pid").read_text().splitlines()[0])
            for engine, jobs in (("mesh", 23), ("oban", 80)):
                evidence = WORK / engine
                evidence.mkdir()
                with (evidence / "broker-output.log").open("wb") as sink:
                    broker = subprocess.Popen([PYTHON, "session"], cwd=WORK, stdout=sink)
                    try:
                        fixture(postmaster, engine, jobs, runners, broker.pid)
                    finally:
                        broker.terminate()
                        broker.wait(timeout=3)
    finally:
        pg("pg_ctl", "-D", str(data), "-m", "immediate", "-w", "stop")


if __name__ == "__main__":
    main()
