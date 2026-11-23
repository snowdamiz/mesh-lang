#!/usr/bin/env python3
"""Run paired trials of compiled Mesh database benchmark fixtures."""

import argparse
from collections import Counter
from contextlib import closing, contextmanager, nullcontext
import csv
from datetime import datetime, timezone
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import random
import selectors
import shutil
import signal
import socket
import socketserver
import sqlite3
import statistics
import subprocess
import tempfile
import threading
import time
from urllib.parse import parse_qsl, unquote, urlencode, urlsplit
from uuid import uuid4


ROOT = Path(__file__).resolve().parents[2]
ITERATIONS = {
    "pg_point": 100, "pg_typed": 100, "pg_write": 32, "pg_rows": 1,
    "pg_sleep": 4, "sqlite_point": 100, "sqlite_write": 32, "sqlite_rows": 1,
}
ACTORS = (1, 16, 64)
ROWS = 10_000
COLUMNS = "id integer PRIMARY KEY, a integer, b integer, c integer, d integer, e integer, f integer, g integer"
CREATE_ROWS = f"CREATE TABLE mesh_db_bench_rows ({COLUMNS})"
CREATE_WRITES = "CREATE TABLE mesh_db_bench_writes (id integer PRIMARY KEY)"
ROW_CHECKSUM = sum(sum((column + 1) * (row + column) for column in range(8))
                   for row in range(1, ROWS + 1))
MIN_BLOCKS = 5


class TrialError(Exception):
    pass


class CallErrors(TrialError):
    def __init__(self, errors):
        self.counts = Counter(errors)
        super().__init__(f"{len(errors)} database calls failed: {dict(self.counts)}")


class FrontendMessages:
    """Count completed PostgreSQL frontend frames across arbitrary TCP reads."""

    def __init__(self):
        self.buffer = bytearray()
        self.startup = True
        self.counts = Counter()

    def feed(self, data):
        self.buffer.extend(data)
        offset = 0
        while len(self.buffer) - offset >= (4 if self.startup else 5):
            tag_size = 0 if self.startup else 1
            start = offset + tag_size
            length = int.from_bytes(self.buffer[start:start + 4], "big")
            if length < 4:
                raise TrialError("invalid PostgreSQL frontend frame length")
            end = start + length
            if end > len(self.buffer):
                break
            if self.startup:
                self.startup = False
            else:
                tag = chr(self.buffer[offset])
                if tag in ("P", "Q", "S"):
                    self.counts[tag] += 1
            offset = end
        del self.buffer[:offset]


class Proxy(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True

    def __init__(self, upstream, fragment):
        super().__init__(("127.0.0.1", 0), Forward)
        self.upstream = upstream
        self.fragment = fragment
        self.counts = Counter()
        self.lock = threading.Lock()

    def add(self, counts):
        with self.lock:
            self.counts.update(counts)

    def snapshot(self):
        with self.lock:
            return self.counts.copy()


class Forward(socketserver.BaseRequestHandler):
    def handle(self):
        parser = FrontendMessages()
        try:
            with socket.create_connection(self.server.upstream, timeout=10) as upstream:
                upstream.settimeout(None)
                self.request.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
                upstream.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
                self.forward(upstream, parser)
        except (OSError, TrialError):
            # The fixture sees a dropped connection and invalidates the trial.
            return

    def forward(self, upstream, parser):
        with selectors.DefaultSelector() as selector:
            selector.register(self.request, selectors.EVENT_READ, upstream)
            selector.register(upstream, selectors.EVENT_READ, self.request)
            while selector.get_map():
                for key, _ in selector.select():
                    data = key.fileobj.recv(65536)
                    if not data:
                        selector.unregister(key.fileobj)
                        key.data.shutdown(socket.SHUT_WR)
                        continue
                    if key.fileobj is self.request:
                        parser.feed(data)
                        if parser.counts:
                            self.server.add(parser.counts)
                            parser.counts.clear()
                        key.data.sendall(data)
                    elif self.server.fragment:
                        for byte in data:
                            key.data.sendall(bytes([byte]))
                    else:
                        key.data.sendall(data)


@contextmanager
def running_proxy(url, fragment):
    parts = urlsplit(url)
    with Proxy((parts.hostname, parts.port or 5432), fragment) as proxy:
        thread = threading.Thread(target=proxy.serve_forever, daemon=True)
        thread.start()
        try:
            credentials = parts.netloc.rpartition("@")[0]
            authority = f"{credentials}@" if credentials else ""
            proxied = parts._replace(netloc=f"{authority}127.0.0.1:{proxy.server_address[1]}")
            yield proxied.geturl(), proxy
        finally:
            proxy.shutdown()
            thread.join()


def percentile(samples, percentage):
    return sorted(samples)[math.ceil(len(samples) * percentage / 100) - 1]


def validate(mode, actors, iterations, result, counts):
    if errors := result.get("call_errors"):
        raise CallErrors(errors)
    elapsed = result["elapsed_ns"]
    checksums = result["checksums"]
    calls = result["calls_ns"]
    ticker = result["ticker_ns"]
    operations = 0 if mode == "idle" else actors * iterations
    if type(elapsed) is not int or elapsed <= 0:
        raise TrialError("missing or invalid wall time")
    if len(calls) != operations or any(type(sample) is not int or sample <= 0 for sample in calls):
        raise TrialError(f"expected {operations} positive call samples")
    if not ticker or any(type(sample) is not int or sample < 0 for sample in ticker):
        raise TrialError("missing or invalid ticker sample")
    if mode.endswith("point") or mode in ("pg_typed", "sqlite_mixed"):
        expected = [(actor * 13 + iteration) % ROWS + 2
                    for actor in range(actors) for iteration in range(iterations)]
    elif mode.endswith("rows"):
        expected = [ROW_CHECKSUM] * operations
    else:
        expected = [1] * operations
    if any(type(checksum) is not int for checksum in checksums) or Counter(checksums) != Counter(expected):
        raise TrialError(f"checksum mismatch ({len(checksums)} of {operations} results)")
    trips = counts["Q"] + counts["S"]
    if mode.startswith("pg_") and trips < operations:
        raise TrialError(f"proxy counted {trips} round trips for {operations} calls")
    rows = operations * (ROWS if mode.endswith("rows") else 1)
    return {
        "elapsed_ns": elapsed, "operations": operations, "rows": rows,
        "checksum": sum(checksums), "p50_ns": percentile(calls, 50) if calls else None,
        "p99_ns": percentile(calls, 99) if calls else None,
        "calls_per_second": operations * 1e9 / elapsed,
        "rows_per_second": rows * 1e9 / elapsed,
        "ticker_p99_ns": percentile(ticker, 99), "ticker_max_ns": max(ticker),
        "ticker_samples": len(ticker), "round_trips": trips, "parses": counts["P"],
        "round_trips_per_call": trips / operations if operations else None,
        "parses_per_call": counts["P"] / operations if operations else None,
    }


def pg_tool(tool):
    if directory := os.environ.get("PG_BIN_DIR"):
        search_path = directory
    elif found := shutil.which(tool):
        return found
    else:
        search_path = subprocess.check_output(["pg_config", "--bindir"], text=True).strip()
    if found := shutil.which(tool, path=search_path):
        return found
    raise TrialError(f"could not find {tool} in {search_path}")


def pg_sql(url, sql, timeout=30):
    parts = urlsplit(url)
    env = os.environ.copy()
    for key in ("PGHOSTADDR", "PGSERVICE", "PGOPTIONS"):
        env.pop(key, None)
    if parts.password is not None:
        env["PGPASSWORD"] = unquote(parts.password)
        host = parts.netloc.rsplit("@", 1)[1]
        user = parts.netloc.split(":", 1)[0]
        url = parts._replace(netloc=f"{user}@{host}").geturl()
    with subprocess.Popen([pg_tool("psql"), "-XwAtq", "-d", url, "-v", "ON_ERROR_STOP=1", "-c", sql],
                          env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True) as proc:
        try:
            output, error = proc.communicate(timeout=timeout)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.communicate()
            raise TrialError("PostgreSQL setup or validation timed out") from None
    if proc.returncode:
        raise TrialError(error.strip().replace(url, f"postgresql://{parts.hostname}/…"))
    return output.strip()


@contextmanager
def local_postgres():
    scratch_parent = ROOT / ".tmp"
    scratch_parent.mkdir(exist_ok=True)
    pg_ctl = pg_tool("pg_ctl")
    initdb = pg_tool("initdb")
    scratch = Path(tempfile.mkdtemp(prefix="db-bench-", dir=scratch_parent))
    data = scratch / "data"
    ctl = [pg_ctl, "-D", str(data)]
    log = scratch / "postgres.log"
    try:
        subprocess.run([initdb, "-D", str(data), "-U", "mesh_db_bench",
                        "--auth=trust", "--no-locale", "--encoding=UTF8"], check=True,
                       stdout=subprocess.DEVNULL)
        for _ in range(8):
            port = random.randrange(49152, 65536)
            options = f"-h 127.0.0.1 -p {port} -c unix_socket_directories=''"
            result = subprocess.run([*ctl, "-l", str(log), "-o", options, "-t", "10", "-w", "start"],
                                    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            if result.returncode == 0:
                break
            if subprocess.run([*ctl, "status"], stdout=subprocess.DEVNULL).returncode == 0:
                raise TrialError(log.read_text())
        else:
            raise TrialError(log.read_text())
        yield f"postgresql://mesh_db_bench@127.0.0.1:{port}/postgres?sslmode=disable"
    finally:
        if subprocess.run([*ctl, "status"], stdout=subprocess.DEVNULL).returncode == 0:
            for mode in ("fast", "immediate"):
                if subprocess.run([*ctl, "-m", mode, "-t", "10", "stop"],
                                  stdout=subprocess.DEVNULL).returncode == 0:
                    break
            else:
                raise TrialError(f"could not stop PostgreSQL, preserved {scratch}")
        shutil.rmtree(scratch)


@contextmanager
def postgres_database(external_url):
    with (nullcontext(external_url) if external_url else local_postgres()) as admin:
        parts = urlsplit(admin)
        name = f"mesh_db_bench_{uuid4().hex}"
        options = dict(parse_qsl(parts.query))
        options["sslmode"] = "disable"
        url = parts._replace(path=f"/{name}", query=urlencode(options)).geturl()
        pg_sql(admin, f"CREATE DATABASE {name}")
        try:
            pg_sql(url, f"{CREATE_ROWS}; {CREATE_WRITES}; INSERT INTO mesh_db_bench_rows "
                   f"SELECT id, id+1, id+2, id+3, id+4, id+5, id+6, id+7 FROM generate_series(1,{ROWS}) AS id;")
            pg_sql(url, "ANALYZE mesh_db_bench_rows")
            yield url, pg_sql(url, "SHOW server_version")
        finally:
            pg_sql(admin, f"DROP DATABASE {name} WITH (FORCE)")


def prepare_sqlite(path):
    with closing(sqlite3.connect(path)) as db, db:
        db.execute(CREATE_ROWS)
        db.execute(CREATE_WRITES)
        db.executemany("INSERT INTO mesh_db_bench_rows VALUES (?,?,?,?,?,?,?,?)",
                       (tuple(range(row, row + 8)) for row in range(1, ROWS + 1)))


def wait_for_fixture(proc, deadline):
    if not hasattr(os, "wait4"):
        proc.wait(timeout=max(0, deadline - time.monotonic()))
        return None

    while True:
        pid, status, usage = os.wait4(proc.pid, os.WNOHANG)
        if pid:
            proc.returncode = os.waitstatus_to_exitcode(status)
            return usage.ru_maxrss * (1 if platform.system() == "Darwin" else 1024)
        if time.monotonic() >= deadline:
            raise TrialError("fixture timed out after warmup")
        time.sleep(0.002)


def run_fixture(binary, env, scratch, proxy, timeout):
    stdout_path = scratch / "stdout"
    stderr_path = scratch / "stderr"
    deadline = time.monotonic() + timeout
    with stdout_path.open("w") as stdout, stderr_path.open("w") as stderr:
        with subprocess.Popen([binary], env=env, stdout=stdout, stderr=stderr) as proc:
            try:
                while not (scratch / "ready").exists():
                    if proc.poll() is not None:
                        raise TrialError(f"fixture exited with status {proc.returncode} before warmup completed")
                    if time.monotonic() >= deadline:
                        raise TrialError("fixture timed out in warmup")
                    time.sleep(0.002)
                before = proxy.snapshot() if proxy else Counter()
                (scratch / "go").touch()
                peak_rss = wait_for_fixture(proc, deadline)
                if proc.returncode:
                    raise TrialError(f"fixture exited with status {proc.returncode}")
            except (TrialError, subprocess.TimeoutExpired) as error:
                raise TrialError(f"{error}\n{stdout_path.read_text()}{stderr_path.read_text()}".strip()) from None
            finally:
                if proc.poll() is None:
                    proc.kill()
                    proc.wait()
    after = proxy.snapshot() if proxy else Counter()
    return json.loads(stdout_path.read_text()), after - before, peak_rss


def trial(binary, mode, actors, args, url, proxy):
    iterations = args.iterations or (1500 if mode == "idle" else ITERATIONS.get(mode, 1))
    warmup = 0 if mode == "idle" else args.warmup
    if mode == "pg_write":
        pg_sql(url, "TRUNCATE mesh_db_bench_writes")
    with tempfile.TemporaryDirectory(prefix="trial-", dir=args.out) as name:
        scratch = Path(name)
        database = scratch / "bench.sqlite3"
        if mode.startswith("sqlite_"):
            prepare_sqlite(database)
        env = dict(os.environ, DB_BENCH_MODE=mode, DB_BENCH_ACTORS=str(actors),
                   DB_BENCH_ITERATIONS=str(iterations), DB_BENCH_WARMUP=str(warmup),
                   DB_BENCH_POOL_SIZE=str(args.pool_size or actors), DB_BENCH_GATE=str(scratch),
                   DB_BENCH_SQLITE=str(database))
        if proxy:
            env["DB_BENCH_URL"] = args.proxy_url
        env["DB_BENCH_PG_SLEEP_SPACING_MS"] = str(args.pg_sleep_spacing_ms or 0)
        env["DB_BENCH_SQLITE_WRITE_WORK"] = str(args.sqlite_write_work or "")
        if args.workers:
            env.update(MESH_SCHEDULER_MIN_WORKERS=str(args.workers),
                       MESH_SCHEDULER_MAX_WORKERS=str(args.workers))
        result, counts, peak_rss = run_fixture(
            binary, env, scratch, proxy if mode.startswith("pg_") else None, args.timeout)
        if mode == "sqlite_mixed":
            with closing(sqlite3.connect(database)) as conn:
                committed = conn.execute("SELECT id FROM mesh_db_bench_writes").fetchall()
            if committed != [(0,)]:
                raise TrialError("mixed writer's row did not commit")
        metrics = validate(mode, actors, iterations, result, counts)
        if mode == "sqlite_mixed" and result.get("readers_before_commit") != actors:
            raise TrialError("mixed readers did not finish while the writer held its transaction")
        if mode.endswith("_write"):
            sql = "SELECT count(*) FROM mesh_db_bench_writes WHERE id > 0"
            if mode == "pg_write":
                persisted = int(pg_sql(url, sql))
            else:
                with closing(sqlite3.connect(database)) as conn:
                    persisted = conn.execute(sql).fetchone()[0]
            if persisted != metrics["operations"]:
                raise TrialError(f"persisted {persisted} of {metrics['operations']} writes")
        return {**metrics, "peak_rss_bytes": peak_rss, "samples": result, "wire_messages": dict(counts)}


def run_blocks(args, url, proxy):
    rng = random.Random(args.seed)
    trials = []
    for block in range(1, args.blocks + 1):
        scenarios = [(mode, actors) for mode in args.mode for actors in args.actors]
        rng.shuffle(scenarios)
        for mode, actors in scenarios:
            sides = [("baseline", args.baseline), ("candidate", args.candidate)]
            rng.shuffle(sides)
            for side, binary in sides:
                record = dict(block=block, mode=mode, actors=actors, side=side)
                try:
                    record.update(trial(binary, mode, actors, args, url, proxy))
                except (OSError, TrialError, ValueError, KeyError, TypeError) as error:
                    record["error"] = str(error).replace(args.proxy_url or "\0", "<database URL>")
                    if isinstance(error, CallErrors):
                        record["call_errors"] = {
                            message.replace(args.proxy_url or "\0", "<database URL>"): count
                            for message, count in error.counts.items()
                        }
                    print(f"invalid {mode}/{actors} {side} block {block}: {record['error']}", flush=True)
                trials.append(record)
    return trials


METRICS = ("elapsed_ns", "p50_ns", "p99_ns", "calls_per_second", "rows_per_second",
           "ticker_p99_ns", "ticker_max_ns", "ticker_samples", "round_trips_per_call", "parses_per_call",
           "peak_rss_bytes")


def exceeds_noise_limit(baseline, candidate):
    return (baseline is not None and candidate is not None
            and abs(candidate - baseline) > baseline * 0.05)


def compare(args, trials, metadata):
    columns = ("mode", "actors", "side", "valid_blocks", "failed_blocks", "sha256", "host", "postgres")
    fields = [*columns, *(f"{metric}_{summary}" for metric in METRICS
                         for summary in ("median", "min", "max"))]
    sufficient = True
    stable = True
    with (args.out / "comparison.csv").open("w", newline="") as stream:
        writer = csv.DictWriter(stream, fields)
        writer.writeheader()
        for mode in args.mode:
            print(f"\n{mode:16} {'actors':>6} {'side':>9} {'valid':>5} {'p50 µs':>10} {'p99 µs':>10} "
                  f"{'calls/s':>10} {'trips/call':>10} {'Parse/call':>10} {'ticker p99 ms':>13}")
            for actors in args.actors:
                side_rows = []
                for side in ("baseline", "candidate"):
                    group = [trial for trial in trials if (trial["mode"], trial["actors"], trial["side"])
                             == (mode, actors, side)]
                    valid = [trial for trial in group if "error" not in trial]
                    sufficient &= len(valid) >= MIN_BLOCKS
                    row = dict(mode=mode, actors=actors, side=side, valid_blocks=len(valid),
                               failed_blocks=len(group) - len(valid), sha256=metadata["binaries"][side],
                               host=metadata["host"], postgres=metadata["postgres"])
                    for metric in METRICS:
                        values = [trial[metric] for trial in valid if trial[metric] is not None]
                        if values:
                            row[f"{metric}_median"] = statistics.median(values)
                            row[f"{metric}_min"] = min(values)
                            row[f"{metric}_max"] = max(values)
                    writer.writerow(row)
                    side_rows.append(row)
                    def display(metric, scale=1):
                        value = row.get(f"{metric}_median")
                        return f"{value / scale:.3f}" if value is not None else "-"
                    print(f"{'':16} {actors:6} {side:>9} {len(valid):5} {display('p50_ns', 1000):>10} "
                          f"{display('p99_ns', 1000):>10} {display('calls_per_second'):>10} "
                          f"{display('round_trips_per_call'):>10} {display('parses_per_call'):>10} "
                          f"{display('ticker_p99_ns', 1_000_000):>13}")
                if args.check_noise:
                    for metric in ("elapsed_ns", "p50_ns", "p99_ns", "ticker_p99_ns"):
                        a, b = (row.get(f"{metric}_median") for row in side_rows)
                        if exceeds_noise_limit(a, b):
                            print(f"{mode}/{actors} {metric} exceeded the 5% noise limit", flush=True)
                            stable = False
    return sufficient and stable


def self_test():
    assert percentile([8, 2, 1, 3], 50) == 2
    assert percentile(list(range(1, 101)), 99) == 99
    assert percentile([4], 99) == 4
    assert not exceeds_noise_limit(100, 105)
    assert exceeds_noise_limit(100, 106)
    assert exceeds_noise_limit(0, 1)
    assert not exceeds_noise_limit(None, 1)

    parser = FrontendMessages()
    startup = bytes.fromhex("0000000800030000")
    exchange = startup + b"P\0\0\0\x05\0S\0\0\0\x04Q\0\0\0\x05\0X\0\0\0\x04"
    for byte in exchange:
        parser.feed(bytes([byte]))
    assert parser.counts == {"P": 1, "S": 1, "Q": 1}
    assert not parser.buffer

    result = dict(elapsed_ns=20, calls_ns=[15], ticker_ns=[0], checksums=[2])
    assert validate("pg_point", 1, 1, result, parser.counts)["round_trips"] == 2
    for update in ({"calls_ns": []}, {"checksums": [3]}, {"ticker_ns": []}):
        try:
            validate("pg_point", 1, 1, {**result, **update}, parser.counts)
        except TrialError:
            continue
        raise AssertionError(f"accepted invalid sample: {update}")
    try:
        validate("sqlite_mixed", 1, 1, {**result, "call_errors": ["database is locked"]}, Counter())
    except CallErrors as error:
        assert error.counts == {"database is locked": 1}
    else:
        raise AssertionError("accepted mixed trial with a failed read")
    print("benchmark self-test passed")


def positive(value):
    value = int(value)
    if value <= 0:
        raise argparse.ArgumentTypeError("expected a positive integer")
    return value


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--baseline", type=Path, help="compiled baseline Mesh fixture")
    parser.add_argument("--candidate", type=Path, help="compiled candidate Mesh fixture")
    parser.add_argument("--out", type=Path, default=ROOT / "target" / "db-performance",
                        help="directory for run.json and comparison.csv")
    parser.add_argument("--mode", action="append", choices=(*ITERATIONS, "idle", "sqlite_mixed"))
    parser.add_argument("--actors", type=int, nargs="+", choices=ACTORS, default=ACTORS)
    parser.add_argument("--blocks", type=positive, default=MIN_BLOCKS)
    parser.add_argument("--seed", type=int, default=89)
    parser.add_argument("--iterations", type=positive, help="override calls per actor (milliseconds for idle)")
    parser.add_argument("--warmup", type=int, default=1, help="warmup calls per actor")
    parser.add_argument("--workers", type=positive, help="fix the number of Mesh scheduler workers")
    parser.add_argument("--pool-size", type=positive, help="override the PostgreSQL pool size")
    parser.add_argument("--pg-sleep-spacing-ms", type=positive,
                        help="schedule pg_sleep calls at this spacing across all actors")
    parser.add_argument("--sqlite-write-work", type=positive, help="recursive SQL steps per SQLite insert")
    parser.add_argument("--check-noise", action="store_true", help="fail if latency medians differ by over 5%%")
    parser.add_argument("--timeout", type=positive, default=120, help="fixture timeout in seconds")
    parser.add_argument("--fragment", action="store_true", help="forward PostgreSQL replies one byte at a time")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0
    if not args.baseline or not args.candidate or args.blocks < MIN_BLOCKS or args.warmup < 0:
        parser.error("comparison requires both binaries, at least five blocks, and nonnegative warmup")
    if args.warmup >= 1_000_000 or (args.iterations or 0) >= 1_000_000:
        parser.error("keep calls per actor below one million to avoid overlapping write keys")
    args.mode = tuple(dict.fromkeys(args.mode or ITERATIONS))
    args.actors = tuple(dict.fromkeys(args.actors))
    args.out = args.out.resolve()
    args.out.mkdir(exist_ok=True, parents=True)
    binaries = {}
    for side in ("baseline", "candidate"):
        binary = getattr(args, side).resolve()
        if not binary.is_file() or not os.access(binary, os.X_OK):
            parser.error(f"{side} is not an executable file: {binary}")
        with binary.open("rb") as stream:
            binaries[side] = hashlib.file_digest(stream, "sha256").hexdigest()
        setattr(args, side, binary)
    pg = any(mode.startswith("pg_") for mode in args.mode)
    database = postgres_database(os.environ.get("MESH_TEST_DATABASE_URL")) if pg else nullcontext((None, None))
    with database as (url, version):
        proxy_cm = running_proxy(url, args.fragment) if pg else nullcontext((None, None))
        with proxy_cm as (args.proxy_url, proxy):
            metadata = dict(started_utc=datetime.now(timezone.utc).isoformat(),
                            host=platform.node(), platform=platform.platform(), postgres=version,
                            logical_cpus=os.cpu_count(), load_average=os.getloadavg(), binaries=binaries,
                            seed=args.seed, workers=args.workers, pool_size=args.pool_size,
                            inherited_scheduler_workers={
                                key: os.environ[key] for key in (
                                    "MESH_SCHEDULER_MIN_WORKERS", "MESH_SCHEDULER_MAX_WORKERS"
                                ) if key in os.environ
                            },
                            iterations=args.iterations, warmup=args.warmup, fragment=args.fragment,
                            pg_sleep_spacing_ms=args.pg_sleep_spacing_ms,
                            sqlite_write_work=args.sqlite_write_work, check_noise=args.check_noise)
            trials = run_blocks(args, url, proxy)
            (args.out / "run.json").write_text(json.dumps(dict(metadata=metadata, trials=trials), indent=2) + "\n")
            return 0 if compare(args, trials, metadata) else 1


if __name__ == "__main__":
    def terminate(_signum, _frame):
        raise KeyboardInterrupt

    signal.signal(signal.SIGTERM, terminate)
    try:
        raise SystemExit(main())
    except KeyboardInterrupt:
        raise SystemExit(130) from None
    except (OSError, TrialError, subprocess.CalledProcessError) as error:
        raise SystemExit(str(error)) from None
