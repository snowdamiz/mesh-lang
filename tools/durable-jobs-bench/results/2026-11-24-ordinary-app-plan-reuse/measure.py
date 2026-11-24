#!/usr/bin/env python3
"""Compare unnamed and named extended-protocol autocommit writes on loopback."""
import argparse
from concurrent.futures import ThreadPoolExecutor
from contextlib import ExitStack, closing
import ctypes as C
import hashlib
import itertools
import json
import os
from pathlib import Path
import platform
import re
import signal
import subprocess
import sys
import tarfile
from tempfile import TemporaryDirectory
from threading import Barrier
from time import monotonic_ns, process_time_ns, time_ns

HERE = Path(__file__).resolve().parent
RESULTS = HERE.parent
ROOT = HERE.parents[3]
TARGET = ROOT / "target/app-plan-reuse"
SUPPORT = RESULTS / "2026-11-23-admission-v3/replay/price.py"
BASE_CLUSTER = ROOT / "scripts/jobs-parity.py"
FIXTURE = RESULTS / "2026-11-23-ordinary-trusted-v1/ordinary-trusted-v1.tar.gz"
DENSE = RESULTS / "2026-11-23-dense-matched-topology/evidence/stock/blocks"
DENSE_INPUT = "9afb598e8e80b56fc96689cfbe7a7061d82f8e5e3c33a2c534d680ce3db2528a"
SOURCE_HASH = "e742644d89b872a41b4d9d241091a2349491f3de6d393fe77c4cbdbc05f6a58c"
DENSE_ARCHIVE_HASH = "8a5b8ece64d96ea296940c291c48e0e8c84c509e47935b40742497400edec4ec"
WORKERS = 4
JOBS = 1024
ORDERS = list(itertools.permutations(("stock-a", "named", "stock-b")))
NAMES = (b"app_attempt", b"app_effect")
MINIMUM_MS = 75
WORKER = ROOT / "tools/durable-jobs-bench/worker.py"
PG_BIN = Path("/var/lib/durable-jobs-bench/native/toolchain/pgsql-17.11/bin")
PG_SHA = "b70863988c4762b6b914ac54d194fa5a1723201100d8155caf7781edba29faa1"
CPUS = {0, 1}
HOST = ("90ec1c24-08c8-4bc1-aa28-79b6db1bd13a", "6.8.0-149-generic", "8:0")

SETTINGS = """SELECT json_object_agg(name, setting) FROM pg_settings
    WHERE name IN ('server_version_num', 'ssl', 'synchronous_commit', 'fsync',
      'full_page_writes', 'track_commit_timestamp', 'pg_stat_statements.track_planning',
      'pg_stat_statements.track', 'plan_cache_mode', 'wal_sync_method', 'track_wal_io_timing',
      'max_connections', 'reserved_connections', 'superuser_reserved_connections')"""
COUNTERS = """SELECT coalesce(json_agg(s ORDER BY s.query), '[]') FROM (
    SELECT query, calls, plans, rows, total_plan_time, total_exec_time, wal_records, wal_bytes
    FROM pg_stat_statements
    WHERE userid = 'compiled_caller'::regrole AND dbid = (SELECT oid FROM pg_database WHERE datname = 'mesh')
) s"""
ROWS = """SELECT json_build_object(
    'attempts', (SELECT json_agg(json_build_array(namespace, engine, job_key, effect_key, job_id,
        job_generation, attempt, (extract(epoch FROM entered_at)*1000000)::bigint, worker_pid,
        xmin::text, (extract(epoch FROM pg_xact_commit_timestamp(xmin))*1000000)::bigint)
        ORDER BY job_key) FROM public.mesh_jobs_parity_attempts),
    'effects', (SELECT json_agg(json_build_array(namespace, engine, job_key, effect_key, xmin::text,
        (extract(epoch FROM pg_xact_commit_timestamp(xmin))*1000000)::bigint)
        ORDER BY job_key) FROM public.mesh_jobs_parity_effects))"""


def write(path, data):
    with path.open("x") as stream:
        json.dump(data, stream, separators=(",", ":"), sort_keys=True, allow_nan=False)
        stream.write("\n")


def archive_json(archive, name):
    return json.load(archive.extractfile(name))


def frozen_inputs():
    dense = next(DENSE.glob("01-*/measured-mesh-v1-2x32/measured-mesh/raw.tar.gz"))
    assert hashlib.sha256(dense.read_bytes()).hexdigest() == DENSE_ARCHIVE_HASH
    assert hashlib.sha256(FIXTURE.read_bytes()).hexdigest() == SOURCE_HASH
    with tarfile.open(FIXTURE) as archive:
        source = archive.extractfile("fixture/writes.mpl").read().decode()
        policy = archive.extractfile(
            "proof/jobs-parity/linux-regression/trusted-application.sql").read().decode()
    with tarfile.open(dense) as archive:
        dense_ledger = archive_json(archive, "adapter/ledger.json")
        records = archive_json(archive, "adapter/application-records.json")
        native_statements = archive_json(archive, "adapter/trusted-application-statements.json")
    assert dense_ledger["input_sha256"] == DENSE_INPUT and dense_ledger["runners"] == 2
    assert dense_ledger["slots"] == 64 and len(dense_ledger["jobs"]) == JOBS
    namespaces = {r["namespace"] for rows in records.values() for r in rows}
    namespace, = namespaces
    assert re.fullmatch(r"parity_[0-9a-f]{16}", namespace)
    jobs = sorted(dense_ledger["jobs"], key=lambda j: j["job_key"])
    assert all(j["job_key"] == f"job_{i:06d}" and j["attempt"] == j["attempts"] == j["effects"] == 1
               and j["hold_ms"] == 0 and not j["fail_first"] for i, j in enumerate(jobs))
    assert len({j["id"] for j in jobs}) == JOBS and len({j["worker_pid"] for j in jobs}) == 2
    heredocs = re.findall(r'Pool\.execute\(pool, """\n(.*?)\n([ ]*)"""', source, re.S)
    assert len(heredocs) == len(NAMES)
    statements = []
    for index, ((body, indent), relation) in enumerate(zip(heredocs, ("attempts", "effects"), strict=True)):
        # Mesh strips the two delimiter lines and the closing delimiter's indentation.
        lines = body.splitlines()
        assert indent and all(line.startswith(indent) for line in lines)
        statement = "\n".join(line[len(indent):] for line in lines)
        native, = [r["query"] for r in native_statements
                   if r["query"].startswith(f"INSERT INTO public.mesh_jobs_parity_{relation}")]
        normalized_params = iter(range(8 if index == 0 else 3, 12))
        normalized = re.sub(r"'mesh'|1000000|'1 microsecond'", lambda _: f"${next(normalized_params)}", statement)
        assert normalized == native
        statements.append(statement.encode())
    bindings = [(namespace, j["job_key"], j["id"], j["id"] + "/1",
                 j["attempt"], j["entered_us"], j["worker_pid"]) for j in jobs]
    return statements, policy.replace("@NAMESPACE@", namespace), bindings


def historical_gap():
    data = []
    for block in sorted(DENSE.glob("[0-9]*-noop.*")):
        arms = {}
        for name, folder in (("mesh", "measured-mesh-v1-2x32/measured-mesh"),
                             ("river", "measured-river-8x8/measured-go"),
                             ("oban", "measured-oban-8x8/measured-oban")):
            path = block / folder / "raw.tar.gz"
            with tarfile.open(path) as archive:
                ledger = archive_json(archive, "adapter/ledger.json")
            assert ledger["input_sha256"] == DENSE_INPUT and len(ledger["jobs"]) == JOBS
            first, = ledger["transactions"]
            elapsed_us = max(j["effect_committed_us"] for j in ledger["jobs"]) - first["committed_us"]
            arms[name] = {"enqueue_to_last_effect_us": elapsed_us,
                          "archive_sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
        gap_us = arms["mesh"]["enqueue_to_last_effect_us"] - arms["river"]["enqueue_to_last_effect_us"]
        data.append({"block": block.name, "arms": arms, "mesh_minus_river_us": gap_us})
    assert len(data) == 5
    return data


def host_identity(worker):
    assert platform.system() == "Linux" and platform.machine() == "x86_64" and os.geteuid() != 0
    assert os.sched_getaffinity(0) == CPUS and os.cpu_count() == 4
    assert Path(os.environ["PG_BIN_DIR"]) == PG_BIN
    boot_id = Path("/proc/sys/kernel/random/boot_id").read_text().strip()
    mount = worker.filesystem(TARGET)
    assert (boot_id, platform.release(), mount["maj:min"]) == HOST and mount["fstype"] == "ext4"
    with (PG_BIN / "postgres").open("rb") as binary:
        sha = hashlib.file_digest(binary, "sha256").hexdigest()
    assert sha == PG_SHA and not worker.builders()
    return dict(boot_id=boot_id, kernel=platform.release(), mount=mount,
                affinity=sorted(os.sched_getaffinity(0)), postgres_sha256=sha)


def register_prepared(wire):
    ptr = C.c_void_p
    wire.pq.PQprepare.restype = ptr
    wire.pq.PQprepare.argtypes = (ptr, C.c_char_p, C.c_char_p, C.c_int, ptr)
    wire.pq.PQexecPrepared.restype = ptr
    wire.pq.PQexecPrepared.argtypes = (ptr, C.c_char_p, C.c_int, C.POINTER(C.c_char_p), ptr, ptr, C.c_int)


def check_command(db, result, expected):
    pq = db.pq
    if not result:
        raise RuntimeError("libpq returned no command result")
    try:
        if pq.PQresultStatus(result) != 1:
            raise RuntimeError(pq.PQresultErrorMessage(result).decode())
        assert pq.PQcmdStatus(result) == expected
        assert pq.PQtransactionStatus(db.conn) == 0  # ReadyForQuery, outside a transaction.
    finally:
        pq.PQclear(result)


def prepare(db, statements):
    for name, sql in zip(NAMES, statements, strict=True):
        check_command(db, db.pq.PQprepare(db.conn, name, sql, 0, None), b"")


def bind_values(values):
    return (C.c_char_p * len(values))(*(str(v).encode() for v in values))


def write_pairs(db, statements, bindings, arm, ready):
    ready.wait(timeout=10)
    pq, conn = db.pq, db.conn
    start = monotonic_ns()
    for pair in bindings:
        for sql, name, values in zip(statements, NAMES, pair, strict=True):
            assert db.call("SELECT 1") == [("1",)]
            if arm == "named":
                result = pq.PQexecPrepared(conn, name, len(values), values, None, None, 0)
            else:
                result = pq.PQexecParams(conn, sql, len(values), None, values, None, None, 0)
            check_command(db, result, b"INSERT 0 1")
    return {"elapsed_ns": monotonic_ns() - start, "pairs": len(bindings), "errors": 0}


def verify_rows(rows, bindings, start_us):
    assert len(rows["attempts"]) == len(rows["effects"]) == len(bindings)
    xids = set()
    for attempt, effect, values in zip(rows["attempts"], rows["effects"], bindings, strict=True):
        ns, key, identity, generation, number, entered, worker = values
        assert attempt[:9] == [ns, "mesh", key, key, identity, generation, number, entered, worker]
        assert effect[:4] == [ns, "mesh", key, key]
        assert effect[5] >= attempt[10] >= start_us and attempt[9] != effect[4]
        xids.update((attempt[9], effect[4]))
    assert len(xids) == len(bindings) * 2
    return xids


def run_arm(cluster, support, worker, wire, statements, bindings, label, arm, output):
    cluster.sql("mesh", "TRUNCATE public.mesh_jobs_parity_attempts, public.mesh_jobs_parity_effects")
    cluster.sql("postgres", "SELECT pg_stat_statements_reset(); SELECT pg_stat_reset_shared('wal')")
    typed = [(bind_values(b), bind_values(b[:2])) for b in bindings]
    # The two native worker PIDs each drive two sessions. The same lanes repeat every arm.
    lanes = [[], [], [], []]
    workers = sorted({b[-1] for b in bindings})
    for worker, offset in zip(workers, (0, 2), strict=True):
        for index, pair in enumerate(p for b, p in zip(bindings, typed, strict=True) if b[-1] == worker):
            lanes[offset + index % 2].append(pair)
    with ExitStack() as resources:
        sessions = [resources.enter_context(closing(support.Session(
            wire, cluster.url("mesh", "compiled_caller") + "&connect_timeout=5&options=-cstatement_timeout%3D8000")))
            for _ in range(WORKERS)]
        pids = [db.pid for db in sessions]
        assert len(set(pids)) == WORKERS
        assert all(os.sched_getaffinity(pid) == CPUS for pid in pids)
        for db in sessions:
            assert db.call("""SELECT current_user, current_setting('search_path'), ssl::text, version, cipher
                FROM pg_stat_ssl WHERE pid=pg_backend_pid()""")[0][:3] == ("compiled_caller", '"$user", public', "true")
        tls = json.loads(cluster.sql("mesh", """SELECT json_agg(json_build_array(a.pid, s.ssl, s.version, s.cipher)
            ORDER BY a.pid) FROM pg_stat_activity a JOIN pg_stat_ssl s USING(pid)
            WHERE a.datname='mesh' AND a.usename='compiled_caller'"""))
        assert {r[0] for r in tls if r[1]} == set(pids)
        prepare_wall_ns = 0
        if arm == "named":
            before = monotonic_ns()
            with ThreadPoolExecutor(WORKERS) as pool:
                list(pool.map(lambda db: prepare(db, statements), sessions))
            prepare_wall_ns = monotonic_ns() - before
        ready = Barrier(WORKERS + 1)
        with ThreadPoolExecutor(WORKERS) as pool:
            futures = [pool.submit(write_pairs, db, statements, lane, arm, ready)
                       for db, lane in zip(sessions, lanes, strict=True)]
            observed_cpus = {f"cpu{cpu}" for cpu in CPUS}
            host_before = worker.sample(observed_cpus)
            load_before = os.getloadavg()
            backend_before = [support.backend_cpu(pid) for pid in pids]
            client_before = process_time_ns()
            start_us = time_ns() // 1000
            start = monotonic_ns()
            ready.wait(timeout=10)
            writers = [future.result(timeout=60) for future in futures]
            wall_ns = monotonic_ns() - start
            client_cpu_ns = process_time_ns() - client_before
            backend_after = [support.backend_cpu(pid) for pid in pids]
            host_after = worker.sample(observed_cpus)
        prepared = [db.call("""SELECT name, generic_plans, custom_plans, from_sql
            FROM pg_prepared_statements ORDER BY name""") for db in sessions]
        assert sum(w["pairs"] for w in writers) == len(bindings)
        for rows, lane in zip(prepared, lanes, strict=True):
            if arm == "named":
                assert [r[0].encode() for r in rows] == list(NAMES)
                assert all(int(g) + int(c) == len(lane) and int(g) > 0 and source == "f"
                           for _, g, c, source in rows)
            else:
                assert rows == []
    counters = json.loads(cluster.sql("postgres", COUNTERS))
    rows = json.loads(cluster.sql("mesh", ROWS))
    verify_rows(rows, bindings, start_us)
    wal = json.loads(cluster.sql("postgres", "SELECT row_to_json(w) FROM pg_stat_wal w"))
    write(output / f"{label}.json", dict(
        arm=arm, label=label, sslmode="verify-full", tls=tls, pids=pids,
        host_before=host_before, host_after=host_after,
        load_before=load_before, load_after=os.getloadavg(), start_us=start_us,
        wall_ns=wall_ns, client_cpu_ns=client_cpu_ns, backend_before=backend_before,
        backend_after=backend_after, prepare_wall_ns=prepare_wall_ns, writers=writers,
        prepared=prepared, counters=counters, wal=wal, rows=rows))
    print(f"{label} {wall_ns / 1e6:.1f} ms", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--check", action="store_true", help="verify frozen inputs without running PostgreSQL")
    mode.add_argument("--native", metavar="NAME", help="controller-launched Linode run")
    args = parser.parse_args()
    if args.native and platform.system() != "Linux":
        parser.error("timing is reserved for the sole controller on the pinned Linode")
    statements, policy, bindings = frozen_inputs()
    gaps = historical_gap()
    if args.check:
        print(json.dumps(dict(writers=WORKERS, pairs=JOBS, orders=ORDERS, measured_arms=0,
                              statements=[s.decode() for s in statements], historical=gaps), indent=2))
        return
    if not re.fullmatch(r"[a-zA-Z0-9_-]+", args.native):
        parser.error("name must use ASCII letters, digits, underscores or hyphens")
    os.chdir(ROOT)
    sys.path.insert(0, str(SUPPORT.parent))
    import price as support
    parity = support.module(BASE_CLUSTER)
    worker = support.module(WORKER)
    identity = host_identity(worker)
    output = TARGET / args.native
    output.mkdir()
    bindir = parity.postgres_tools()
    libdir = subprocess.check_output([bindir / "pg_config", "--libdir"], text=True).strip()
    library = Path(libdir) / "libpq.so"
    wire = support.Wire(library)
    register_prepared(wire)
    write(output / "freeze.json", dict(
        orders=ORDERS, minimum_ms=MINIMUM_MS, jobs=len(bindings), workers=WORKERS,
        advance="median gain versus faster stock >= minimum_ms and gain > duplicate spread in at least four rotations",
        source_sha256=SOURCE_HASH, dense_archive_sha256=DENSE_ARCHIVE_HASH, dense_input_sha256=DENSE_INPUT,
        historical_gap=gaps, commands=sys.argv,
        files={str(path.relative_to(ROOT)): hashlib.sha256(path.read_bytes()).hexdigest()
               for path in (Path(__file__).resolve(), SUPPORT, BASE_CLUSTER, WORKER)},
        statements=[s.decode() for s in statements], policy_sha256=hashlib.sha256(policy.encode()).hexdigest()))
    class NativeCluster(parity.Cluster):
        def configure_tls(self):
            super().configure_tls()
            with (self.data / "postgresql.conf").open("a") as config:
                config.write("max_connections = 96\nreserved_connections = 0\nsuperuser_reserved_connections = 0\n")

    def interrupt(signum, _frame):
        raise SystemExit(128 + signum)

    for signum in (signal.SIGTERM, signal.SIGHUP):
        signal.signal(signum, interrupt)
    with TemporaryDirectory(dir=TARGET, prefix="cluster-") as scratch:
        cluster = NativeCluster(Path(scratch), output, 96, "17.11")
        try:
            cluster.start()
            cluster.sql("postgres", f"CREATE ROLE compiled_caller LOGIN PASSWORD '{cluster.password}'")
            cluster.prepare_database("mesh")
            cluster.sql("mesh", policy)
            cluster.sql("postgres", "ALTER SYSTEM SET pg_stat_statements.track_planning = 'on'")
            cluster.sql("postgres", "ALTER SYSTEM SET track_wal_io_timing = 'on'")
            cluster.call("pg_ctl", "-D", str(cluster.data), "reload")
            settings = json.loads(cluster.sql("mesh", SETTINGS))
            assert settings["server_version_num"] == "170011"
            assert settings["max_connections"] == "96"
            assert settings["reserved_connections"] == settings["superuser_reserved_connections"] == "0"
            assert settings["plan_cache_mode"] == "auto" and settings["pg_stat_statements.track"] == "top"
            assert all(settings[key] == "on" for key in (
                "ssl", "synchronous_commit", "fsync", "full_page_writes", "track_commit_timestamp",
                "pg_stat_statements.track_planning", "track_wal_io_timing"))
            cpu_before, process_before = support.backend_cpu(os.getpid()), process_time_ns()
            while process_time_ns() - process_before < 80_000_000:
                pass
            cpu_elapsed = support.backend_cpu(os.getpid()) - cpu_before
            process_elapsed = process_time_ns() - process_before
            assert 0.90 < cpu_elapsed / process_elapsed < 1.10
            write(output / "environment.json", dict(
                system=platform.system(), machine=platform.machine(), kernel=platform.release(),
                logical_cores=os.cpu_count(), python=sys.version, libpq=str(library.resolve()),
                libpq_sha256=hashlib.sha256(library.read_bytes()).hexdigest(), native_host=identity,
                bindir=str(bindir.resolve()), settings=settings, cpu_calibration_ns=cpu_elapsed,
                cpu_process_clock_ns=process_elapsed, initial_load=os.getloadavg()))
            for group, order in enumerate(ORDERS, 1):
                for arm in order:
                    run_arm(cluster, support, worker, wire, statements, bindings, f"{group:02}-{arm}", arm, output)
        finally:
            cluster.stop()


if __name__ == "__main__":
    main()
