#!/usr/bin/env python3
"""Use the exact direct-file adapter on disposable PostgreSQL 17.11 TLS clusters."""

from contextlib import contextmanager
import ctypes
import hashlib
import json
import os
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import time
import traceback

from build import ADAPTER, DIRECT, STATE, check, digest
from component import HARNESS, LIMIT, ORDER, batch, bound, examples, job, write

sys.path.insert(0, str(ADAPTER))
import arm


class Cluster(arm.Cluster):
    def configure_tls(self):
        super().configure_tls()
        with (self.data / "postgresql.conf").open("a") as conf:
            conf.write("\nwork_mem = '16MB'\n")


class CPU:
    def __init__(self):
        if sys.platform == "darwin":
            lib = ctypes.CDLL("libproc.dylib", use_errno=True)
            self.sample = lib.proc_pid_rusage
            self.sample.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_void_p]
            self.sample.restype = ctypes.c_int
            base = (ctypes.c_uint32 * 2)()
            if ctypes.CDLL(None).mach_timebase_info(ctypes.byref(base)):
                raise OSError("mach_timebase_info")
            self.ns_per_unit = base[0] / base[1]
        else:
            self.ns_per_unit = 1e9 / os.sysconf("SC_CLK_TCK")

    def counters(self, pid):
        if sys.platform == "darwin":
            record = ctypes.create_string_buffer(512)
            if self.sample(pid, 2, record) != 0:  # RUSAGE_INFO_V2
                raise OSError(ctypes.get_errno(), f"proc_pid_rusage({pid})")
            return struct.unpack_from("=QQ", record.raw, offset=16)
        fields = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
        return int(fields[11]), int(fields[12])


def broker_pid(binary):
    ps = subprocess.check_output(["ps", "-A", "-o", "pid=,ppid=,args="], text=True)
    pid, = (int(pid) for line in ps.splitlines() if len(fields := line.split(None, 2)) == 3
            for pid, parent, command in [fields]
            if int(parent) == os.getpid() and command == f"{binary} session")
    return pid


@contextmanager
def pg(out):
    out.mkdir(parents=True)
    with tempfile.TemporaryDirectory(prefix="db-", dir=STATE) as folder:
        scratch = Path(folder)
        cluster = Cluster(scratch, out, 96)
        cluster.runners, cluster.slots, cluster.lanes = 8, 64, 4
        try:
            cluster.start()
            cluster.prepare_database("mesh")
            yield cluster, scratch
        except BaseException:
            (out / "failure.log").write_text(traceback.format_exc())
            raise
        finally:
            try:
                cluster.stop()
            finally:
                arm.redact(out, cluster.password)


def stats(cluster):
    return json.loads(cluster.sql("mesh", """
        SELECT coalesce(json_object_agg(q, n), '{}'::json)
        FROM (SELECT query q, sum(calls) n FROM pg_stat_statements
              WHERE userid = 'parity_producer'::regrole
                AND dbid = (SELECT oid FROM pg_database WHERE datname='mesh')
                AND (query LIKE 'SELECT mesh_jobs.enqueue(%' OR query LIKE 'INSERT INTO mesh_jobs.admissions AS a%')
              GROUP BY query) s
    """))


def counters(cluster):
    observed = stats(cluster)
    return (sum(n for txt, n in observed.items() if txt.startswith("SELECT mesh_jobs.enqueue(")),
            sum(n for txt, n in observed.items() if txt.startswith("INSERT INTO mesh_jobs.admissions AS a")), observed)


def rows(cluster, ns):
    return json.loads(cluster.sql("mesh", f"""
        SELECT coalesce(json_agg(row_to_json(r) ORDER BY r.position), '[]')
        FROM (SELECT args->>'key' AS key, id::text, position, xmin::text AS xid,
                     extract(epoch FROM pg_xact_commit_timestamp(xmin))*1000000 AS commit_us,
                     encode(sha256(convert_to(args->>'payload','UTF8')), 'hex') AS payload_sha256,
                     (args->>'hold_ms')::int AS hold_ms, (args->>'fail_first')::bool AS fail_first,
                     coalesce(pg_column_compression(args), 'none') AS compression,
                     pg_column_size(args) AS stored_bytes
              FROM mesh_jobs.jobs WHERE namespace='{ns}') r
    """))


def product(cluster, ns):
    return json.loads(cluster.sql("mesh", f"""
        SELECT coalesce(json_agg(row_to_json(r) ORDER BY r.request_id), '[]')
        FROM (SELECT request_id, xmin::text AS xid, extract(epoch FROM pg_xact_commit_timestamp(xmin))*1000000 AS commit_us
              FROM public.mesh_jobs_parity_product WHERE engine='mesh' AND namespace='{ns}') r
    """))


def receipts(cluster, ns):
    return json.loads(cluster.sql("mesh", f"""
        SELECT coalesce(json_object_agg(request_key, receipt->>'id'), '{{}}'::json)
        FROM mesh_jobs.receipts WHERE namespace='{ns}'
    """))


def prepare(cluster, scratch, out, binary, ns, mode, schema=True):
    migrator = DIRECT / "local-stock/mesh"
    subject = arm.Subject("mesh", [str(migrator)], migrator.parent, cluster,
                          dict(cluster.env, JOBS_PARITY_MESH_BATCH="1", JOBS_PARITY_MESH_LISTEN="1"))
    subject.direct_producer = str(binary)
    subject.fixture_dir = scratch
    env = subject.environment(ns, 8, 64, mode, 250)
    if schema:
        subject.call("migrate", env, out / "migrate.log")
        cluster.sql("mesh", "ALTER TABLE mesh_jobs.jobs ALTER COLUMN args SET STORAGE EXTERNAL")
    else:
        subprocess.run([migrator, "migrate"], env=env, check=True, capture_output=True)
    settings = json.loads(cluster.sql("mesh", """
        SELECT json_build_object(
            'version', current_setting('server_version_num'), 'work_mem', current_setting('work_mem'),
            'args_storage', (SELECT attstorage::text FROM pg_attribute WHERE
                             attrelid='mesh_jobs.jobs'::regclass AND attname='args'),
            'durability', json_build_array(current_setting('synchronous_commit'),
                                           current_setting('fsync'),current_setting('full_page_writes')))
    """))
    assert settings == dict(version="170011", work_mem="16MB", args_storage="e",
                            durability=["on", "on", "on"]), settings
    return subject, env, settings


def accepted(cluster, ns, fixture, direct, committed):
    jobs = fixture["jobs"]
    observed, persisted = rows(cluster, ns), receipts(cluster, ns)
    stamps = product(cluster, ns)
    by_key = {row["key"]: row for row in observed}
    assert [row["key"] for row in observed][-len(jobs):] == [job["key"] for job in jobs]
    assert direct["count"] == len(jobs)
    stamp, = (row for row in stamps if row["request_id"] == fixture["request_id"])
    assert stamp["xid"] == direct["xid"] and abs(stamp["commit_us"] / 1000 - committed) < 0.001
    for j in jobs:
        row = by_key[j["key"]]
        assert row["id"] == direct["ids"][j["key"]] == persisted[j["key"]]
        assert row["payload_sha256"] == hashlib.sha256(j["payload"].encode()).hexdigest()
        assert (row["hold_ms"], row["fail_first"], row["compression"]) == (j["hold_ms"], j["fail_first"], "none")
        assert row["commit_us"] == stamp["commit_us"]
    return stamp, observed, persisted


def time_arm(out, bins, treatment, fixture, rotation, cpu):
    ns = "broker_once_timing"
    binary = bins / treatment
    with pg(out) as (cluster, scratch):
        subject, env, settings = prepare(cluster, scratch, out, binary, ns, "dense")
        with subject.warm_producer(env, out, time.monotonic() + 90) as send:
            path, = HARNESS["batch_files"](scratch, [fixture])
            input_sha = digest(path)
            producer, backend = broker_pid(binary), int(cluster.sql("mesh", "SELECT pg_backend_pid()"))
            p0, b0 = cpu.counters(producer), cpu.counters(backend)
            load_before = os.getloadavg()
            start = time.monotonic_ns()
            direct, committed = send(path, out / "receipt.log")
            finish = time.monotonic_ns()
            p1, b1 = cpu.counters(producer), cpu.counters(backend)
            load_after = os.getloadavg()
            stamp, jobs, persisted = accepted(cluster, ns, fixture, direct, committed)
            sql, chunks, queries = counters(cluster)
            timing = cluster.transactions[fixture["request_id"]]
            assert timing["submitted_us"] <= stamp["commit_us"] <= timing["acknowledged_us"]
            assert len(jobs) == sql == chunks == len({j["xid"] for j in jobs}) == 64, queries
            assert len(persisted) == 64
            receipt = dict(arm=treatment, rotation=rotation, input_sha256=input_sha, jobs=jobs,
                           binary_sha256=digest(binary), producer=direct, database_product=stamp,
                           sql_counts=queries, committed_receipts=persisted,
                           submit_to_commit_ms=(stamp["commit_us"] - timing["submitted_us"]) / 1000,
                           adapter_round_trip_ms=(finish-start)/1e6, adapter_timing=timing,
                           load_before=load_before, load_after=load_after,
                           cpu_counters=dict(producer=[producer, p0, p1], backend=[backend, b0, b1],
                                             ns_per_unit=cpu.ns_per_unit), settings=settings)
            write(out / "result.json", receipt)
    return receipt


def semantic_arm(out, binary, component):
    ns = "broker_once_semantics"
    with pg(out) as (cluster, scratch):
        subject, env, settings = prepare(cluster, scratch, out, binary, ns, "dense")

        @contextmanager
        def session(name, environment=env):
            evidence = out / name
            evidence.mkdir()
            with subject.warm_producer(environment, evidence, time.monotonic() + 120) as send:
                yield lambda path: send(path, evidence / f"{path.stem}.log")

        examples_by_label = dict(examples())
        attempt = 0
        proof = {"successes": {}, "failures": {}, "settings": settings}
        for label in ("mixed", "exact", "count", "slow", "dense", "sparse"):
            group_ns = f"{ns}_{label}"
            subject, group_env, _ = prepare(cluster, scratch, out, binary, group_ns, "dense", schema=False)
            start_sql, start_chunks, _ = counters(cluster)
            with session(label, group_env) as send:
                expected_sql = expected_chunks = 0
                for index, fixture in enumerate(examples_by_label[label]):
                    fixture = batch(attempt, fixture["jobs"])
                    attempt += 1
                    path, = HARNESS["batch_files"](scratch, [fixture])
                    result, committed = send(path)
                    stamp, persisted, _ = accepted(cluster, group_ns, fixture, result, committed)
                    observed = component[label][index]["once"]
                    positions = persisted[-len(fixture["jobs"]):]
                    xids_in_order = []
                    offset = 0
                    for chunk in observed["chunks"]:
                        group = positions[offset:offset + len(chunk["keys"])]
                        assert [r["key"] for r in group] == chunk["keys"]
                        xid, = {r["xid"] for r in group}
                        xids_in_order.append(xid)
                        offset += len(group)
                    assert len(set(xids_in_order)) == len(observed["chunks"])
                    expected_sql += len(fixture["jobs"])
                    expected_chunks += len(observed["chunks"])
                    proof["successes"].setdefault(label, []).append(dict(receipt=result, product=stamp,
                        input_sha256=digest(path), chunk_xids=xids_in_order, chunks=observed["chunks"],
                        jobs=positions))
                end_sql, end_chunks, observed_queries = counters(cluster)
                observed_calls = end_sql - start_sql, end_chunks - start_chunks
                assert observed_calls == (expected_sql, expected_chunks), (
                    label, observed_calls, expected_sql, expected_chunks, observed_queries)

        subject, env, _ = prepare(cluster, scratch, out, binary, ns, "dense", schema=False)
        with session("rollback", env) as send:
            before = (rows(cluster, ns), product(cluster, ns), receipts(cluster, ns))
            abort, = HARNESS["batch_files"](scratch, [batch(attempt, [job("rollback")], True)])
            attempt += 1
            receipt, committed = send(abort)
            assert receipt["count"] == 1 and committed is None
            assert (rows(cluster, ns), product(cluster, ns), receipts(cluster, ns)) == before
            proof["explicit_rollback"] = receipt
            alias = scratch / "batch_999999.json"
            alias.symlink_to(abort)
            for name, invalid in (("relative", Path(abort.name)), ("symlink", alias), ("outside", out / abort.name)):
                try:
                    send(invalid)
                except ValueError as error:
                    assert "private trial scratch" in str(error)
                    proof["failures"][name] = str(error)
                else:
                    raise AssertionError(name)

        failures = [
            ("malformed", b'{"jobs":[', "EOF while parsing", 0, 0, None),
            ("request_id", batch(777777, [job("wrong")]), "batch file does not match request ID", 0, 0, None),
            ("late_invalid", [job(f"bad_{i:03d}") for i in range(128)] + [job("bad", hold_ms=-1)],
             "hold_ms must be nonnegative", 128, 1, None),
            ("invalid_lookahead", [bound("fits", LIMIT-2), job("bad", hold_ms=-1)],
             "hold_ms must be nonnegative", 0, 0, None),
            ("late_oversize", [job("writes_first"), bound("too_wide", LIMIT-1)],
             "encoded job exceeds the Jobs.enqueue_in input limit", 1, 1, None),
            ("first_oversize", [bound("too_wide", LIMIT-1)],
             "encoded job exceeds the Jobs.enqueue_in input limit", 0, 0, None),
            ("probe_limit", examples_by_label["exact"][0]["jobs"],
             "producer commit probe input exceeds the enqueue batch limit", 0, 0, "producer_commit"),
        ]
        for name, data, message, sql_expected, chunks_expected, mode in failures:
            path = scratch / f"batch_{attempt:06d}.json"
            if isinstance(data, list):
                write(path, batch(attempt, data))
            elif isinstance(data, dict):
                write(path, data)
            else:
                path.write_bytes(data)
            attempt += 1
            prior_sql, prior_chunks, _ = counters(cluster)
            try:
                with session(name, dict(env, JOBS_PARITY_MODE=mode or "dense")) as send:
                    send(path)
            except (RuntimeError, BrokenPipeError) as error:
                assert message.lower() in str(error).lower(), repr(error)
                after_sql, after_chunks, _ = counters(cluster)
                assert (after_sql-prior_sql, after_chunks-prior_chunks) == (sql_expected, chunks_expected), name
                proof["failures"][name] = dict(message=str(error), scalar_calls=sql_expected,
                                                enqueue_calls=chunks_expected, input_sha256=digest(path))
            else:
                raise AssertionError(name)
            assert (rows(cluster, ns), product(cluster, ns), receipts(cluster, ns)) == before
        with session("probe-exact", dict(env, JOBS_PARITY_MODE="producer_commit")) as send:
            path, = HARNESS["batch_files"](scratch, [batch(attempt, [bound("probe_exact", LIMIT-2)])])
            probe, committed = send(path)
            assert probe["count"] == len(probe["receipts"]) == 1 and committed is not None
            assert probe["receipts"][0]["id"] == probe["ids"]["probe_exact"]
            proof["probe_exact"] = probe
        proof["durable"] = dict(jobs=rows(cluster, ns), products=product(cluster, ns), receipts=receipts(cluster, ns))
        xids = [r["xid"] for r in proof["durable"]["products"]]
        xids.extend(run["product"]["xid"] for group in proof["successes"].values() for run in group)
        assert len(set(xids)) == len(xids) and proof["explicit_rollback"]["xid"] not in xids
        write(out / "proof.json", proof)


def main(command, out, bins, component):
    out.mkdir()
    pins = json.loads((bins / "pins.json").read_text())
    for label in ("A", "B", "once"):
        check(bins / label, pins[label])
    if command == "proof":
        trace = json.loads((component / "counts-and-membership.json").read_text())
        for label in ("A", "once"):
            semantic_arm(out / label, bins / label, trace)
    else:
        fixture, = dict(examples())["large"]
        cpu = CPU()
        report = []
        for n, order in enumerate(ORDER, 1):
            for label in order:
                row = time_arm(out / f"{n:02d}-{label}", bins, label, fixture, n, cpu)
                report.append(row)
                print(n, label, row["submit_to_commit_ms"], row["load_before"], flush=True)
        write(out / "measurements.json", report)


if __name__ == "__main__":
    os.umask(0o077)
    main(sys.argv[1], *(Path(arg).resolve() for arg in sys.argv[2:]))
