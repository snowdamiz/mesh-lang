"""Drive the sealed run_arm and wire paths against an untimed deterministic fixture."""
from collections import Counter
from contextlib import redirect_stdout
import hashlib
import importlib.util
from io import StringIO
from itertools import count
import json
from pathlib import Path
import sys
from tempfile import TemporaryDirectory
from types import SimpleNamespace
from unittest.mock import patch

PACKET = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(PACKET))
import measure


def load_source(name, path, content):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    exec(compile(content, str(path), "exec"), module.__dict__)
    return module


class DeferredPool:
    """Keep submission, barrier release, and future consumption in call order."""

    def __init__(self, workers):
        assert workers == measure.WORKERS

    def __enter__(self):
        return self

    def __exit__(self, *_):
        pass

    def map(self, fn, args):
        return map(fn, args)

    def submit(self, fn, *args):
        def result(*, timeout):
            assert timeout == 60
            return fn(*args)
        return SimpleNamespace(result=result)


class Fixture:
    def __init__(self, statements, bindings):
        self.statements = dict(zip(measure.NAMES, statements, strict=True))
        self.binding_by_key = {values[1]: values for values in bindings}
        self.pids = list(range(100, 100 + measure.WORKERS))
        self.events = []
        self.rows = {"attempts": [], "effects": []}
        self.xid = count(10)
        self.tick = count(1_000_000, 1_000)
        self.backend_tick = count(100)
        self.sessions = iter(self.pids)
        self.by_pid = {}
        self.release = False

    def url(self, database, role):
        assert (database, role) == ("mesh", "compiled_caller")
        return "postgresql://fixture?sslmode=verify-full"

    def sql(self, database, query):
        if query.startswith("TRUNCATE"):
            assert database == "mesh" and not self.by_pid
            return ""
        if query.startswith("SELECT pg_stat_statements_reset"):
            assert database == "postgres" and not self.by_pid
            return ""
        if "FROM pg_stat_activity" in query:
            assert database == "mesh" and list(self.by_pid) == self.pids
            return json.dumps([[pid, True, "TLSv1.3", "fixture"] for pid in self.pids])
        assert all(session.closed for session in self.by_pid.values())
        if query == measure.ROWS:
            return json.dumps({table: sorted(rows, key=lambda row: row[2]) for table, rows in self.rows.items()})
        if query == measure.COUNTERS:
            return "[]"
        assert query == "SELECT row_to_json(w) FROM pg_stat_wal w"
        return "{}"

    def session(self, wire, url):
        assert wire is self and "sslmode=verify-full&connect_timeout=" in url
        pid = next(self.sessions)
        session = Session(self, pid)
        self.by_pid[pid] = session
        return session

    def barrier(self, size):
        assert size == measure.WORKERS + 1

        def wait(*, timeout):
            assert timeout == 10
            self.release = True
        return SimpleNamespace(wait=wait)

    def sample(self, cpus):
        assert cpus == {"cpu0", "cpu1"}
        boundary = "after" if self.release else "before"
        self.events.append(("sample", boundary))
        return {"fixture_boundary": boundary}

    def backend_cpu(self, pid):
        assert pid in self.pids
        return next(self.backend_tick)

    def insert(self, path, pid, name, values):
        assert self.release
        params = tuple(v.decode() for v in values)
        ns, key, identity, generation, attempt, entered, runner_pid = self.binding_by_key[params[1]]
        self.events.append((path, pid, name.decode(), key))
        xid = str(next(self.xid))
        if name == b"app_attempt":
            assert params == (ns, key, identity, generation, str(attempt), str(entered), str(runner_pid))
            self.rows["attempts"].append([ns, "mesh", key, key, identity, generation, attempt, entered,
                                          runner_pid, xid, 1_000_000])
        else:
            assert name == b"app_effect" and params == (ns, key)
            self.rows["effects"].append([ns, "mesh", key, key, xid, 1_000_001])
        return b"INSERT 0 1"


class Session:
    def __init__(self, fixture, pid):
        self.fixture = fixture
        self.pid = self.conn = pid
        self.pq = self
        self.names = {}
        self.executed = Counter()
        self.closed = False

    def close(self):
        self.closed = True

    def call(self, sql):
        if "FROM pg_stat_ssl" in sql:
            return [("compiled_caller", '"$user", public', "true", "TLSv1.3", "fixture")]
        if sql == "SELECT 1":
            assert self.fixture.release
            self.fixture.events.append(("health", self.pid))
            return [("1",)]
        assert "FROM pg_prepared_statements" in sql
        assert self.fixture.events[-1] == ("sample", "after")
        return [(name.decode(), str(self.executed[name] - 5), "5", "f") for name in sorted(self.names)]

    def PQprepare(self, conn, name, sql, nparams, types):
        assert conn == self.pid and nparams == 0 and types is None and name not in self.names
        assert self.fixture.statements[name] == sql and not self.fixture.release
        self.names[name] = sql
        self.fixture.events.append(("prepare", conn, name.decode()))
        return object()

    def PQexecParams(self, conn, sql, count_, types, values, lengths, formats, result_format):
        assert conn == self.pid and types is lengths is formats is None and result_format == 0
        assert count_ == len(values) and not self.names
        name, = [name for name, text in self.fixture.statements.items() if text == sql]
        return self.fixture.insert("unnamed", conn, name, values)

    def PQexecPrepared(self, conn, name, count_, values, lengths, formats, result_format):
        assert conn == self.pid and lengths is formats is None and result_format == 0
        assert count_ == len(values) and self.names[name] == self.fixture.statements[name]
        self.executed[name] += 1
        return self.fixture.insert("named", conn, name, values)

    @staticmethod
    def PQresultStatus(_):
        return 1

    @staticmethod
    def PQcmdStatus(result):
        return result if isinstance(result, bytes) else b""

    @staticmethod
    def PQtransactionStatus(_):
        return 0

    @staticmethod
    def PQclear(_):
        pass


def prove_arm(module, statements, bindings, arm, output, expected_failure):
    fixture = Fixture(statements, bindings)
    support = SimpleNamespace(Session=fixture.session, backend_cpu=fixture.backend_cpu)
    env = SimpleNamespace(sched_getaffinity=lambda pid: {0, 1}, getloadavg=lambda: (0, 0, 0))
    label = ("original-" if expected_failure else "retry-") + arm
    with redirect_stdout(StringIO()), patch.multiple(
                        module, os=env, ThreadPoolExecutor=DeferredPool, Barrier=fixture.barrier,
                        monotonic_ns=lambda: next(fixture.tick), process_time_ns=lambda: next(fixture.tick),
                        time_ns=lambda: next(fixture.tick)):
        try:
            module.run_arm(fixture, support, fixture, fixture, statements, bindings, label, arm, output)
        except AttributeError as error:
            assert expected_failure and str(error) == "'int' object has no attribute 'sample'"
            assert not any(event[0] == "sample" for event in fixture.events)
            assert not any(fixture.rows.values())
            return {"arm": arm, "failure": str(error), "sample_calls": 0}
    assert not expected_failure
    path = "named" if arm == "named" else "unnamed"
    events = fixture.events
    before = events.index(("sample", "before"))
    after = events.index(("sample", "after"))
    assert events[-1] == ("sample", "after")
    assert [event for event in events if event[0] == "sample"] == [("sample", "before"), ("sample", "after")]
    setup = events[:before]
    assert len(setup) == (2 * measure.WORKERS if arm == "named" else 0)
    assert all(event[0] == "prepare" for event in setup)
    writes = events[before + 1:after]
    assert len(writes) == 4 * len(bindings)
    assert all((event[0], event[1]) == ("health", insert[1]) and insert[0] == path
               for event, insert in zip(writes[::2], writes[1::2], strict=True))
    insert_counts = Counter(event[1] for event in writes[1::2])
    assert set(insert_counts) == set(fixture.pids) and set(insert_counts.values()) == {12}
    receipt = json.loads((output / f"{label}.json").read_bytes())
    assert receipt["host_before"] == {"fixture_boundary": "before"}
    assert receipt["host_after"] == {"fixture_boundary": "after"}
    assert len(receipt["rows"]["attempts"]) == len(receipt["rows"]["effects"]) == len(bindings)
    return {"arm": arm, "protocol": path, "prepares": len(setup),
            "writers": len(insert_counts), "writes": sum(insert_counts.values()),
            "sample_order": ["before", "after"]}


def main():
    retry_path = PACKET / "native-sampler-retry.py"
    retry = load_source("native_sampler_retry", retry_path, retry_path.read_bytes())
    original = retry.MEASURE.read_bytes()
    corrected = retry.corrected_measure()
    expected = original.replace(b"for worker, offset in zip(workers", b"for worker_pid, offset in zip(workers")
    expected = expected.replace(b"if b[-1] == worker):", b"if b[-1] == worker_pid):")
    assert corrected == expected and original != corrected
    packet = retry.native.run.read_json(retry.PACKET)
    corrected_hash = hashlib.sha256(corrected).hexdigest()
    assert packet["local_sha256"][retry.ARM_PATH] == corrected_hash
    patched = load_source("patched_measure", retry.MEASURE, corrected)
    statements, _, all_bindings = measure.frozen_inputs()
    pids = sorted({values[-1] for values in all_bindings})
    selected = {pid: [values for values in all_bindings if values[-1] == pid][:12] for pid in pids}
    bindings = sorted((v for pairs in selected.values() for v in pairs), key=lambda v: v[1])
    with TemporaryDirectory(dir=measure.TARGET, prefix="sampler-proof-") as scratch:
        original_result = [prove_arm(measure, statements, bindings, arm, Path(scratch), True)
                           for arm in ("stock-a", "named")]
        patched_result = [prove_arm(patched, statements, bindings, arm, Path(scratch), False)
                          for arm in ("stock-a", "named", "stock-b")]
    print(json.dumps({
        "method": "deterministic fixture clocks and synthetic libpq, PostgreSQL, and host responses",
        "performance_samples": 0,
        "original_measure_sha256": hashlib.sha256(original).hexdigest(),
        "corrected_measure_sha256": corrected_hash,
        "original": original_result,
        "retry": patched_result,
    }, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
