#!/usr/bin/env python3
"""Operation pricing on a fresh durable PG17.11 instance using libpq extended queries."""
import argparse
from collections import defaultdict
from contextlib import ExitStack, closing
import ctypes as C
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import statistics
import tempfile
from time import monotonic_ns

ROOT = Path.cwd()
PROOF = ROOT / "proof/jobs-parity/linux-regression"
FIXTURE = ROOT / "scripts/jobs-parity.py"
JOBS = "compiler/mesh-rt/src/jobs"
WORKERS = '[{"worker":"parity","version":1}]'
SPECS = ("stock", "strict", "cohort")


def module(path):
    spec = importlib.util.spec_from_file_location("fixture", path)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


def digest(data):
    return hashlib.sha256(data).hexdigest()


def backend_cpu(pid):
    if platform.system() == "Linux":
        return int(Path(f"/proc/{pid}/schedstat").read_text().split()[0])
    return darwin_cpu(pid)


def darwin_cpu(pid):
    # rusage_info_v0 uses Mach time units, and outlives individual requests.
    fields = (C.c_uint64 * 12)()
    assert PROC_RUSAGE(pid, 0, C.byref(fields)) == 0
    return (fields[2] + fields[3]) * TIMEBASE[0] // TIMEBASE[1]


if platform.system() == "Darwin":
    PROC_RUSAGE = C.CDLL("/usr/lib/libproc.dylib").proc_pid_rusage
    TIMEBASE = (C.c_uint32 * 2)()
    assert C.CDLL(None).mach_timebase_info(C.byref(TIMEBASE)) == 0


class Wire:
    def __init__(self, library):
        self.pq = C.CDLL(str(library))
        for name, restype, *signature in (
            ("connectdb", C.c_void_p, C.c_char_p),
            ("status", C.c_int, C.c_void_p),
            ("backendPID", C.c_int, C.c_void_p),
            ("sslInUse", C.c_int, C.c_void_p),
            ("exec", C.c_void_p, C.c_void_p, C.c_char_p),
            ("execParams", C.c_void_p, C.c_void_p, C.c_char_p, C.c_int,
             C.c_void_p, C.POINTER(C.c_char_p), C.c_void_p, C.c_void_p, C.c_int),
            ("resultStatus", C.c_int, C.c_void_p),
            ("resultErrorMessage", C.c_char_p, C.c_void_p),
            ("transactionStatus", C.c_int, C.c_void_p),
            ("ntuples", C.c_int, C.c_void_p),
            ("nfields", C.c_int, C.c_void_p),
            ("cmdStatus", C.c_char_p, C.c_void_p),
            ("getvalue", C.c_char_p, C.c_void_p, C.c_int, C.c_int),
            ("clear", None, C.c_void_p),
            ("finish", None, C.c_void_p),
        ):
            fn = getattr(self.pq, f"PQ{name}")
            fn.restype, fn.argtypes = restype, signature


class Session:
    def __init__(self, wire, url):
        self.pq = wire.pq
        self.conn = self.pq.PQconnectdb(url.encode())
        if self.pq.PQstatus(self.conn) or not self.pq.PQsslInUse(self.conn):
            self.close()
            raise RuntimeError("libpq verified TLS connection failed")
        self.pid = self.pq.PQbackendPID(self.conn)

    def close(self):
        self.pq.PQfinish(self.conn)

    def call(self, sql, params=None, *, tx=0):
        if params is None:
            result = self.pq.PQexec(self.conn, sql.encode())
        else:
            values = (C.c_char_p * len(params))(*(str(value).encode() for value in params))
            result = self.pq.PQexecParams(self.conn, sql.encode(), len(values), None,
                                          values, None, None, 0)
        try:
            status = self.pq.PQresultStatus(result)
            if status not in (1, 2):
                raise RuntimeError(self.pq.PQresultErrorMessage(result).decode())
            assert self.pq.PQtransactionStatus(self.conn) == tx, sql[:80]
            if status == 1:
                return self.pq.PQcmdStatus(result).decode()
            return [tuple(self.pq.PQgetvalue(result, row, col).decode()
                          for col in range(self.pq.PQnfields(result)))
                    for row in range(self.pq.PQntuples(result))]
        finally:
            self.pq.PQclear(result)


def commands(source):
    definitions = (source / JOBS / "store.rs").read_text()
    claim = definitions.split('"SELECT coalesce(status,', 1)[1].split('",', 1)[0]
    due = definitions.split('"WITH clock AS MATERIALIZED', 1)[1].split('",', 1)[0]
    return "SELECT coalesce(status," + claim, "WITH clock AS MATERIALIZED" + due


def setup(cluster, libpq, source, stock, payloads):
    cluster.prepare_database("mesh")
    schema = "".join((source / JOBS / name).read_text() for name in (
        "schema.sql", "cron_schema.sql", "drain_schema.sql"))
    cluster.sql("mesh", "SET ROLE parity_subject;\n" + schema)
    cluster.sql("postgres", f"""CREATE ROLE compiled_definer NOLOGIN;
        CREATE ROLE compiled_caller LOGIN PASSWORD '{cluster.password}';
        CREATE ROLE admission_worker LOGIN PASSWORD '{cluster.password}';""")
    cluster.sql("postgres", "ALTER SYSTEM SET max_connections = '96'")
    cluster.sql("postgres", "ALTER SYSTEM SET superuser_reserved_connections = '0'")
    cluster.call("pg_ctl", "-D", str(cluster.data), "-w", "-t", "20",
                 "-l", str(cluster.evidence / "postgres.log"), "restart")
    cluster.sql("mesh", "SET ROLE parity_subject;\n" + (PROOF / "fence.sql").read_text())
    cluster.sql("mesh", (PROOF / "application.sql").read_text())
    with closing(Session(Wire(libpq), cluster.url("mesh", "parity_subject"))) as db:
        db.call("INSERT INTO mesh_jobs.queues(namespace,name,concurrency) VALUES ('price','default',64);")
        assert db.call("BEGIN", tx=2) == "BEGIN"
        db.call("INSERT INTO public.mesh_jobs_parity_product VALUES ('mesh','price','batch_000000');", tx=2)
        for n, args in enumerate(payloads):
            spec = dict(worker="parity", version=1, queue="default", args=args, metadata={}, tags=[],
                        priority=0, max_attempts=3, timeout_ms=60000, backoff_base_ms=1000,
                        backoff_max_ms=1000, suspended=False)
            inserted = db.call("SELECT mesh_jobs.enqueue('price',$1::jsonb,'','')",
                               (json.dumps(spec),), tx=2)
            assert json.loads(inserted[0][0])["inserted"], n
        assert db.call("COMMIT") == "COMMIT"
    worker_acl = (stock / "scripts/fixtures/jobs-role-grants.sql").read_text()
    for role in ("worker", "producer", "operator", "inspector"):
        worker_acl = worker_acl.replace(f':"{role}"', "admission_worker" if role == "worker" else "parity_owner")
    cluster.sql("mesh", "SET ROLE parity_subject;\n" + worker_acl)
    if source != stock:
        cluster.sql("mesh", """REVOKE EXECUTE ON FUNCTION mesh_jobs.claim_admitted_v3(text,text,jsonb,bigint,integer,bigint),
                   mesh_jobs.complete_admitted_v3(text,uuid[],uuid[]) FROM PUBLIC;
            GRANT EXECUTE ON FUNCTION mesh_jobs.claim_admitted_v3(text,text,jsonb,bigint,integer,bigint),
                   mesh_jobs.complete_admitted_v3(text,uuid[],uuid[]) TO admission_worker;""")
    cluster.sql("mesh", "ANALYZE")
    return json.loads(cluster.sql("mesh", """SELECT json_build_object(
        'server', current_setting('server_version'), 'max_connections', current_setting('max_connections'),
        'reserved', current_setting('reserved_connections'), 'superuser_reserved', current_setting('superuser_reserved_connections'),
        'fsync', current_setting('fsync'), 'synchronous_commit', current_setting('synchronous_commit'),
        'full_page_writes', current_setting('full_page_writes'), 'track_commit_timestamp',current_setting('track_commit_timestamp'))"""))


def price(cluster, libpq, stock, payload, arm, width):
    claim, due = commands(stock)
    wire = Wire(libpq)
    summary = defaultdict(lambda: dict(calls=0, backend_cpu_ns=0, client_wall_ns=0))
    with ExitStack() as stack:
        jobs = stack.enter_context(closing(Session(wire, cluster.url("mesh", "admission_worker"))))
        app = stack.enter_context(closing(Session(wire, cluster.url("mesh", "compiled_caller"))))

        def call(label, session, sql, params=None, *, tx=0):
            before = backend_cpu(session.pid)
            start = monotonic_ns()
            result = session.call(sql, params, tx=tx)
            wall = monotonic_ns() - start
            summary[label]["backend_cpu_ns"] += backend_cpu(session.pid) - before
            summary[label]["client_wall_ns"] += wall
            summary[label]["calls"] += 1
            return result

        advance = 0
        for offset in range(0, len(payload), width):
            slots = min(width, len(payload) - offset)
            if arm == "stock":
                assert call("probe", jobs, due, ("price", "default", WORKERS)) == [("",)]
                assert call("admit_begin", jobs, "BEGIN ISOLATION LEVEL READ COMMITTED", tx=2) == "BEGIN"
                rows = call("admit_query", jobs, claim, ("price", "default", WORKERS, 60000, slots, "", "false"), tx=2)
                assert call("admit_commit", jobs, "COMMIT") == "COMMIT"
            else:
                assert call("admit_begin", jobs, "BEGIN ISOLATION LEVEL READ COMMITTED", tx=2) == "BEGIN"
                rows = call("admit_query", jobs,
                            "SELECT * FROM mesh_jobs.claim_admitted_v3($1,$2,$3,$4,$5,NULL) WITH ORDINALITY",
                            ("price", "default", WORKERS, 60000, slots), tx=2)
                assert len(rows) == slots
                for slot, row in enumerate(rows, 1):
                    assert row[0] == "claimed" and int(row[-1]) == slot
                    assert json.loads(row[3])["id"] == row[1] and len(row[3].encode()) == int(row[5])
                assert call("admit_commit", jobs, "COMMIT") == "COMMIT"
            assert len(rows) == slots
            for n, (status, id_, token, context, timeout, size, *_) in enumerate(rows, offset):
                job = json.loads(context)
                assert status == "claimed" and job["args"] == payload[n] and job["attempt"] == 1
                assert job["id"] == id_ and int(timeout) == 60000 and len(context.encode()) == int(size)
                key = job["args"]["key"]
                for op, statement, args in (
                    ("first", "SELECT compiled_demo.first('price',$1,1,1,$2,$3,"
                     "(extract(epoch FROM clock_timestamp())*1000000)::bigint,pg_backend_pid()::bigint,true)", (id_, token, key)),
                    ("second", "SELECT compiled_demo.second('price',$1,1,1,$2,$3,true)", (id_, token, key)),
                ):
                    assert call(f"app_{op}_begin", app, "BEGIN", tx=2) == "BEGIN"
                    assert call(f"app_{op}_write", app, statement, args, tx=2) == [("t",)]
                    assert call(f"app_{op}_commit", app, "COMMIT") == "COMMIT"
                    assert call(f"app_{op}_reset", app, "DISCARD ALL") == "DISCARD ALL"
                advance += 1
            if arm in ("stock", "strict"):
                for _, id_, token, *_ in rows:
                    assert call("settle_commit_and_query", jobs,
                                "SELECT coalesce(mesh_jobs.settle_state($1, $2::uuid, $3::uuid, $4::jsonb, false), 'stale')",
                                ("price", id_, token, '{"action":"complete"}')) == [("completed",)]
            else:
                ids = "{" + ",".join(row[1] for row in rows) + "}"
                tokens = "{" + ",".join(row[2] for row in rows) + "}"
                result = call("settle_commit_and_query", jobs,
                              "SELECT * FROM mesh_jobs.complete_admitted_v3($1,$2,$3)",
                              ("price", ids, tokens))
                assert sorted(int(row[0]) for row in result) == list(range(1, len(rows) + 1))
        assert advance == len(payload)
        # Price renewals in an isolated namespace with identical ready rows. The
        # production no-op profile may finish before the first renewal is due.
        observer = stack.enter_context(closing(Session(wire, cluster.url("mesh", "parity_subject"))))
        observer.call("""INSERT INTO mesh_jobs.queues(namespace,name,concurrency) VALUES ('renew','default',64);
            INSERT INTO mesh_jobs.jobs (namespace,worker,version,queue,args,metadata,tags,state,priority,
                max_attempts,timeout_ms,backoff_base_ms,backoff_max_ms,inserted_at_ms,scheduled_at_ms)
            SELECT 'renew',worker,version,queue,args,metadata,tags,'available',priority,
                max_attempts,timeout_ms,backoff_base_ms,backoff_max_ms,inserted_at_ms,scheduled_at_ms
            FROM mesh_jobs.jobs WHERE namespace='price' ORDER BY position LIMIT 64""")
        assert jobs.call("BEGIN ISOLATION LEVEL READ COMMITTED", tx=2) == "BEGIN"
        renewal_rows = jobs.call(claim, ("renew", "default", WORKERS, 60000, 16, "", "false"), tx=2)
        assert jobs.call("COMMIT") == "COMMIT"
        assert len(renewal_rows) == 16
        for _ in range(4):
            for _, id_, token, *_ in renewal_rows:
                assert call("renew_commit_and_query", jobs,
                            "SELECT mesh_jobs.try_renew($1, $2::uuid, $3::uuid, $4::bigint) AS status",
                            ("renew", id_, token, 120000)) == [("active",)]
    witness = json.loads(cluster.sql("mesh", """
        SELECT json_build_object('rows', count(*), 'terminal_xids', count(distinct j.xmin::text),
            'distinct_app_xids', count(distinct a.xid) + count(distinct e.xid))
        FROM mesh_jobs.jobs j
        JOIN public.mesh_jobs_parity_attempts a ON (a.namespace,a.job_id,a.job_run)=(j.namespace,j.id,j.run)
        JOIN public.mesh_jobs_parity_effects e ON (e.namespace,e.job_id,e.job_run)=(j.namespace,j.id,j.run)
        WHERE j.namespace='price' AND j.state='completed' AND j.attempt=a.attempt AND j.attempt=e.attempt
            AND j.attempt=1 AND a.job_key=j.args->>'key' AND e.job_key=a.job_key
            AND a.effect_key=mesh_jobs.effect_key(j) AND e.effect_key=a.effect_key
            AND pg_xact_commit_timestamp(a.xmin) < pg_xact_commit_timestamp(e.xmin)
            AND pg_xact_commit_timestamp(e.xmin) < pg_xact_commit_timestamp(j.xmin) AND a.xid < e.xid
    """))
    expected = len(payload) if arm != "cohort" else (len(payload) + width - 1) // width
    assert witness == dict(rows=len(payload), terminal_xids=expected, distinct_app_xids=2 * len(payload)), witness
    return dict(witness=witness, operations=summary)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--stock", type=Path, required=True)
    parser.add_argument("--candidate", type=Path)
    parser.add_argument("--libpq", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--scratch", type=Path, required=True)
    parser.add_argument("--rounds", type=int, default=5)
    parser.add_argument("--arms", choices=SPECS, nargs="+", default=["stock"])
    parser.add_argument("--width", type=int, default=8, choices=(1, 4, 8, 16))
    args = parser.parse_args()
    h = module(FIXTURE)
    config = json.loads(FIXTURE.with_name("jobs-parity-profiles.json").read_text())
    batches = h.inputs(config["workloads"]["noop"], 64, config["seed"])
    hashes = [digest((json.dumps(batch, indent=2, sort_keys=True) + "\n").encode()) for batch in batches]
    input_digest = digest("".join(hashes).encode())
    assert input_digest == "9afb598e8e80b56fc96689cfbe7a7061d82f8e5e3c33a2c534d680ce3db2528a"
    payload = [job for batch in batches for job in batch["jobs"]]
    report = dict(input_sha256=input_digest, host=platform.platform(), cpus=os.cpu_count(),
                  postgres_sha256=digest((h.postgres_tools() / "postgres").read_bytes()),
                  schema_sha256={label: digest((path / JOBS / "schema.sql").read_bytes())
                    for label, path in (("stock", args.stock), ("candidate", args.candidate)) if path},
                  application_sha256=digest((PROOF / "application.sql").read_bytes()),
                  fence_sha256=digest((PROOF / "fence.sql").read_bytes()), tls="verify-full",
                  width=args.width, rounds=[])
    for index in range(args.rounds):
        for arm in (args.arms if index % 2 == 0 else reversed(args.arms)):
            with tempfile.TemporaryDirectory(dir=args.scratch, prefix="price-") as scratch:
                work = Path(scratch)
                source = args.stock if arm == "stock" else args.candidate
                cluster = h.Cluster(work, work, 96, expected_version="17.11")
                try:
                    cluster.start()
                    settings = setup(cluster, args.libpq, source, args.stock, payload)
                    result = price(cluster, args.libpq, args.stock, payload, arm, args.width)
                finally:
                    cluster.stop()
                report["rounds"].append(dict(arm=arm, load=os.getloadavg(), settings=settings, **result))
                print(f"confirmed {arm} round {index + 1}: {result['witness']}", flush=True)
            report["median_backend_ms_per_call"] = {
                label: {op: statistics.median(row["operations"][op]["backend_cpu_ns"] /
                         row["operations"][op]["calls"] / 1e6 for row in report["rounds"] if row["arm"] == label)
                        for op in next(row["operations"] for row in report["rounds"] if row["arm"] == label)}
                for label in {row["arm"] for row in report["rounds"]}
            }
            args.output.write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    main()
