"""Local representative PostgreSQL workload and lifecycle controls."""
import base64
from contextlib import ExitStack
import gzip
import importlib.util
import json
import os
from pathlib import Path
import secrets
import select
import sys
import time

from observe import Snapshot, start_sampler, stop_sampler

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("prior", HERE / "linux-roster-smoke.py")
prior = importlib.util.module_from_spec(spec)
spec.loader.exec_module(prior)
PG, WORK, ENV = prior.PG, prior.WORK, prior.ENV
pg, sql, require = prior.pg, prior.sql, prior.require
JOBS = 1024
MASK = sorted(prior.CPUS)


def emit(label, value):
    data = json.dumps(value, separators=(",", ":"), allow_nan=False).encode()
    encoded = base64.b64encode(gzip.compress(data, mtime=0)).decode()
    print(f"ROLE_EVIDENCE_BEGIN {label} {len(encoded)}", flush=True)
    for i in range(0, len(encoded), 2048):
        print(encoded[i:i+2048])
    print("ROLE_EVIDENCE_END", flush=True)


def identity(db):
    return json.loads(sql("postgres", f"""
       SELECT json_build_object('postmaster', pg_postmaster_start_time(), 'stats_reset', s.stats_reset,
         'sessions', s.sessions, 'observer_pid', pg_backend_pid(),
         'rows', (SELECT coalesce(json_agg(json_build_object('pid', a.pid,
                     'login', a.usename, 'start', a.backend_start, 'tls', t.ssl) ORDER BY a.pid), '[]')
                  FROM pg_stat_activity a LEFT JOIN pg_stat_ssl t USING(pid)
                  WHERE a.datname = '{db}' AND a.backend_type = 'client backend'))
         FROM pg_stat_database s WHERE datname = '{db}'
    """))


def client(stack, db, role):
    return prior.accept_client(stack, db, role)


def send(connection, statement):
    connection.process.stdin.write(statement + "\n")
    connection.process.stdin.flush()


def receive(connection, expected):
    pipe = connection.process.stdout
    # A runaway SQL command can't turn a readiness failure into a hung guest.
    require(select.select([pipe], [], [], 12)[0], "fixture query deadline")
    actual = pipe.readline().strip()
    require(actual == str(expected), f"fixture expected {expected} rows, got {actual!r}")


def materialize(table, low, high, effort):
    return (f"WITH w AS (INSERT INTO {table} "
            f"SELECT id,md5(s::text) FROM generate_series({low},{high}) id CROSS JOIN LATERAL "
            f"(SELECT sum(length(md5((id+n)::text))) s FROM generate_series(1,{effort}) n) x "
            "RETURNING 1) SELECT count(*) FROM w;")


def run_arm(db, mode, label, postmaster, force=None):
    before = after = run_status = None
    result = {"db": db, "mode": mode, "label": label, "force": force, "cpus": MASK,
              "hz": os.sysconf("SC_CLK_TCK")}
    trace = WORK / (label + ".jsonl")
    with ExitStack() as clients, ExitStack() as pins:
        producer = client(clients, db, "parity_producer")
        jobs_count = 32 if db == "mesh" else 80
        applications = [client(clients, db, "compiled_caller") for _ in range(32 if db == "mesh" else 0)]
        initial = 23 if force == "churn" else jobs_count
        jobs = [client(clients, db, "parity_subject") for _ in range(initial)]
        roots = {postmaster: "postmaster", producer.process.pid: "broker",
                 **{entry.process.pid: "runner" for entry in jobs[:8]}}
        if force:
            result["readiness"] = identity(db)
            jobs.extend(client(clients, db, "parity_subject") for _ in range(8))
            rogue = client(clients, db, "unclassified")
            result["unclassified_pid"] = rogue.backend
        start_cpu = [prior.process_clock(item.process.pid) for item in jobs[:8]]
        sampler = None
        try:
            if mode == "on":
                before = Snapshot(pins, postmaster, MASK, lambda: identity(db), roots)
                result["before"] = before.record
                sampler = start_sampler(HERE / "clock", trace, MASK, prior.SPARES[0], before)
                require(select.select([sampler.stderr], [], [], 10)[0], "sampler ready deadline")
                require(sampler.stderr.readline() == b"ready\n", "sampler startup failed")
            time.sleep(.030)
            submit = time.time_ns() // 1000
            for low in range(1, JOBS + 1, 64):
                send(producer, materialize("enqueued", low, low + 63, 550))
                receive(producer, 64)
            ack = time.time_ns() // 1000
            if force == "churn":
                victim = jobs[10]
                require(sql("postgres", f"SELECT pg_terminate_backend({victim.backend}, 2000)") == "t",
                        "forced backend exit failed")
                jobs[10] = client(clients, db, "parity_subject")
                late = client(clients, db, "parity_subject")
                jobs.append(late)
                with ExitStack() as fleeting:
                    ephemeral = client(fleeting, db, "parity_subject")
                    send(ephemeral, "SELECT pg_backend_pid();")
                    receive(ephemeral, ephemeral.backend)
                result.update(retired=victim.backend, replacement=jobs[10].backend,
                              late=late.backend, ephemeral=ephemeral.backend)
            for clients_for_stage, table, effort in (
                (jobs, "attempts", 900), (applications or jobs, "effects", 450), (jobs, "settled", 900),
            ):
                width = (JOBS + len(clients_for_stage) - 1) // len(clients_for_stage)
                chunks = []
                for i, connection in enumerate(clients_for_stage):
                    low = i * width + 1
                    high = min(JOBS, low + width - 1)
                    if low <= high:
                        send(connection, materialize(table, low, high, effort))
                        chunks.append((connection, high - low + 1))
                for connection, count in chunks:
                    receive(connection, count)
            time.sleep(.030)
            if sampler:
                after = Snapshot(pins, postmaster, MASK, lambda: identity(db), roots)
                result["after"] = after.record
        finally:
            if sampler:
                run_status = stop_sampler(sampler)
                result["sampler"] = run_status
                result["frames"] = [json.loads(line) for line in trace.read_text().splitlines()]
            if sys.exc_info()[0]:
                emit(label, result)
        end_cpu = [prior.process_clock(item.process.pid) for item in jobs[:8]]
        result["runner_timed_cpu_ms_per_job"] = (sum(end_cpu) - sum(start_cpu)) / 1e6 / JOBS
        result["ledger"] = json.loads(sql(db, """
          SELECT json_build_object(
            'enqueued', (SELECT json_agg(json_build_array(id, xmin::text,
                 (extract(epoch FROM pg_xact_commit_timestamp(xmin))*1000000)::bigint) ORDER BY id) FROM enqueued),
            'attempts', (SELECT json_agg(json_build_array(id, xmin::text,
                 (extract(epoch FROM pg_xact_commit_timestamp(xmin))*1000000)::bigint) ORDER BY id) FROM attempts),
            'effects', (SELECT json_agg(json_build_array(id, xmin::text,
                 (extract(epoch FROM pg_xact_commit_timestamp(xmin))*1000000)::bigint) ORDER BY id) FROM effects),
            'settled', (SELECT json_agg(json_build_array(id, xmin::text,
                 (extract(epoch FROM pg_xact_commit_timestamp(xmin))*1000000)::bigint) ORDER BY id) FROM settled))
        """))
        result.update(submit_us=submit, ack_us=ack)
    if mode == "on":
        result["after_close"] = identity(db)
    emit(label, result)
    require(run_status is None or run_status["exit"] == 0, "native collector exited unsuccessfully")
    sql(db, "TRUNCATE enqueued, attempts, effects, settled")


def start_server():
    data = WORK / "data"
    password = secrets.token_hex(24)
    pwfile = WORK / "pw"
    pwfile.touch(mode=0o600)
    pwfile.write_text(password + "\n")
    try:
        pg("initdb", "-D", str(data), "--no-locale", "-A", "scram-sha-256",
           "-U", "nobody", f"--pwfile={pwfile}")
    finally:
        pwfile.unlink()
    ENV["PGPASSWORD"] = password
    (data / "server.crt").write_bytes((HERE / "server.crt").read_bytes())
    key = data / "server.key"
    key.write_bytes((WORK / "server.key").read_bytes())
    key.chmod(0o600)
    (data / "pg_hba.conf").write_text("hostssl all all 127.0.0.1/32 scram-sha-256\n")
    with (data / "postgresql.conf").open("a") as settings:
        settings.write("listen_addresses = '127.0.0.1'\nport = 55432\n"
                       "unix_socket_directories = ''\nssl = on\nmax_connections = 96\n"
                       "track_commit_timestamp = on\n")
    pg("pg_ctl", "-D", str(data), "-l", str(WORK / "postgres.log"), "-w", "start")
    return data, password


def main():
    require(os.getuid() == 65534 and len(prior.SPARES) >= 1, "expected unprivileged workload plus spare")
    os.sched_setaffinity(0, prior.CPUS)
    data, password = start_server()
    try:
        postmaster = int((data / "postmaster.pid").read_text().splitlines()[0])
        version, ssl, sync, fsync, full_pages, cap, track = sql(
            "postgres", "SELECT current_setting('server_version_num'), ssl, "
            "current_setting('synchronous_commit'), current_setting('fsync'), "
            "current_setting('full_page_writes'), current_setting('max_connections'), "
            "current_setting('track_commit_timestamp') FROM pg_stat_ssl WHERE pid=pg_backend_pid()").split("|")
        require((version, ssl, sync, fsync, full_pages, cap, track) ==
                ("170011", "t", "on", "on", "on", "96", "on"), "fixture settings mismatch")
        emit("platform", dict(version=version, auth="SCRAM", tls=ssl, sync=sync, fsync=fsync,
                              full_pages=full_pages, cap=cap, commit_timestamps=track, uname=list(os.uname()),
                              mask=MASK, spare=prior.SPARES[0], uid=os.getuid()))
        for login, cap in (("parity_producer", 1), ("parity_subject", -1),
                           ("compiled_caller", 32), ("unclassified", 1)):
            sql("postgres", f"CREATE ROLE {login} LOGIN PASSWORD '{password}' CONNECTION LIMIT {cap}")
        for db in ("mesh", "peer"):
            sql("postgres", f"CREATE DATABASE {db}")
            for table in ("enqueued", "attempts", "effects", "settled"):
                sql(db, f"CREATE TABLE {table} (id int primary key, proof text NOT NULL); "
                        f"GRANT INSERT, SELECT ON {table} TO parity_subject, parity_producer, compiled_caller")
        for db in ("mesh", "peer"):
            for i, mode in enumerate(("off", "on", "off"), 1):
                run_arm(db, mode, f"{db}-{i}-{mode}", postmaster)
        run_arm("mesh", "on", "mesh-churn", postmaster, force="churn")
    finally:
        pg("pg_ctl", "-D", str(data), "-m", "fast", "-w", "stop")


if __name__ == "__main__":
    main()
