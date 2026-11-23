"""Replay the sealed producer on fresh clones and reconcile each durable transaction."""
import argparse
import contextlib
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import re
import signal
import shutil
import statistics
import subprocess
import tempfile
import traceback

HERE = Path(__file__).resolve().parent
SOURCE = Path(os.environ["ORDERED_SOURCE"])
BASE = Path(os.environ["ORDERED_BASE"])
STOCK = Path(os.environ.get("ORDERED_STOCK", BASE / "builds/mesh-v1/mesh"))
CANDIDATE = Path(os.environ["ORDERED_ARTIFACT"]) / "mesh"


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


h = load("parity", SOURCE / "scripts/jobs-parity.py")
# Run the original relay/session protocol against the same sealed harness.
r = load("producer_protocol", HERE / "producer_protocol.py")
r.parity = h


def host():
    return {
        "affinity": sorted(os.sched_getaffinity(0)) if hasattr(os, "sched_getaffinity") else None,
        "kernel": platform.release(),
        "loadavg": os.getloadavg(),
        **{name: Path(path).read_text() for name, path in {
            "cpu": "/proc/stat", "cpu_pressure": "/proc/pressure/cpu",
            "io_pressure": "/proc/pressure/io", "boot_id": "/proc/sys/kernel/random/boot_id",
        }.items() if Path(path).is_file()},
    }


def witness(cluster, database, responses, batches, component, out, rolled_back_count=0):
    doc = json.loads(cluster.sql(database, """
        SELECT json_build_object(
           'commits', (SELECT json_object_agg(request_id, json_build_array(xmin::text,
               floor(extract(epoch FROM pg_xact_commit_timestamp(xmin))*1000000)::bigint,
               engine, namespace) ORDER BY request_id) FROM public.mesh_jobs_parity_product),
           'jobs', (SELECT json_agg(json_build_object(
                      'key', j.args->>'key', 'payload_sha256',
                      encode(sha256(convert_to(j.args->>'payload','UTF8')), 'hex'),
                      'hold_ms', j.args->'hold_ms', 'fail_first', j.args->'fail_first',
                      'id',j.id::text, 'ordinal',j.position, 'namespace',j.namespace,
                      'job_xid', j.xmin::text, 'receipt_xid', r.xmin::text, 'receipt',r.receipt,
                      'job_commit_us', floor(extract(epoch FROM pg_xact_commit_timestamp(j.xmin))*1000000)::bigint,
                      'receipt_commit_us', floor(extract(epoch FROM pg_xact_commit_timestamp(r.xmin))*1000000)::bigint
                   ) ORDER BY j.position)
               FROM mesh_jobs.jobs j JOIN mesh_jobs.receipts r
                 ON (r.namespace=j.namespace AND r.request_key=j.args->>'key')),
           'counts', ARRAY[(SELECT count(*) FROM mesh_jobs.jobs),
                           (SELECT count(*) FROM mesh_jobs.receipts),
                           (SELECT count(*) FROM public.mesh_jobs_parity_product),
                           (SELECT count(*) FROM public.mesh_jobs_parity_attempts),
                           (SELECT count(*) FROM public.mesh_jobs_parity_effects)])
    """))
    h.write_json(out / "durable-ledger.json", doc)
    calls = json.loads(cluster.sql(database, """
        SELECT json_object_agg(query,json_build_object('calls',calls,'execute_ms',total_exec_time))
          FROM pg_stat_statements
         WHERE dbid=(SELECT oid FROM pg_database WHERE datname=current_database())
           AND userid=(SELECT oid FROM pg_roles WHERE rolname='parity_producer')
    """))
    h.write_json(out / "pg-stat-statements.json", calls)
    expected = [job for batch in batches for job in batch["jobs"]]
    assert doc["counts"] == [len(expected), len(expected), len(batches), 0, 0]
    expected_args = [(job["key"], hashlib.sha256(job["payload"].encode()).hexdigest(),
                      job["hold_ms"], job["fail_first"]) for job in expected]
    fields = ("key", "payload_sha256", "hold_ms", "fail_first")
    assert [tuple(row[key] for key in fields) for row in doc["jobs"]] == expected_args
    ids = [row["id"] for row in doc["jobs"]]
    ordinals = [row["ordinal"] for row in doc["jobs"]]
    assert len(set(ids)) == len(ids) and sorted(set(ordinals)) == ordinals
    producer_xids = set()
    subxid_owner = {}
    offset = 0
    for timing, batch in zip(responses, batches, strict=True):
        raw = timing["response"]
        count = len(batch["jobs"])
        rows = doc["jobs"][offset:offset + count]
        xid, stamp, engine, namespace = doc["commits"][batch["request_id"]]
        assert engine == "mesh" and namespace == r.NS and xid not in producer_xids and int(xid) > 2
        producer_xids.add(xid)
        assert raw["ids"] == {row["key"]: row["id"] for row in rows}
        assert raw["count"] == count and raw["xid"] == xid
        assert timing["submitted_us"] <= stamp <= timing["acknowledged_us"]
        timing["commit_ms"] = (stamp - timing["submitted_us"]) / 1000
        timing["commit_us"] = stamp
        for row in rows:
            assert row["namespace"] == r.NS
            assert row["job_xid"] == row["receipt_xid"] and int(row["job_xid"]) > 2
            assert subxid_owner.setdefault(row["job_xid"], xid) == xid
            assert row["job_commit_us"] == row["receipt_commit_us"] == stamp
            job_id = row["id"]
            assert row["receipt"] == {"id": job_id, "effect_key": f"{job_id}/1", "state": "available",
                                      "conflict": False, "inserted": True}
        if component:
            assert raw["receipts"] == [row["receipt"] for row in rows]
        offset += count
    assert sum(v["calls"] for sql, v in calls.items() if sql.startswith("SELECT mesh_jobs.enqueue(")) == len(expected) + rolled_back_count


def fault(cluster, database, env, batch, out, kind):
    actions = {
        "sql-error": "RAISE EXCEPTION 'ordered enqueue diagnostic marker';",
        "disconnect": "PERFORM pg_terminate_backend(pg_backend_pid()); PERFORM pg_sleep(0.05);",
    }
    cluster.sql(database, f"""
        CREATE SEQUENCE public.ordered_attempt_count;
        CREATE FUNCTION public.ordered_attempt_probe() RETURNS trigger
        LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $probe$
        BEGIN
            PERFORM nextval('public.ordered_attempt_count');
            IF NEW.args->>'key' = 'job_000003' THEN
                {actions[kind]}
            END IF;
            RETURN NEW;
        END
        $probe$;
        CREATE TRIGGER ordered_attempt_probe BEFORE INSERT ON mesh_jobs.jobs
        FOR EACH ROW EXECUTE FUNCTION public.ordered_attempt_probe();
    """)
    request = json.dumps({"verb": "enqueue", "batch": batch}, separators=(",", ":"))
    with (out / "fault-stdout.log").open("x") as stdout, (out / "fault-stderr.log").open("x") as stderr:
        process = subprocess.run([CANDIDATE, "session"], env=env, input=request + "\n", text=True,
                                 stdout=stdout, stderr=stderr, timeout=30)
    result = json.loads(cluster.sql(database, """
        SELECT json_build_object(
            'attempts', (SELECT last_value FROM public.ordered_attempt_count),
            'jobs', (SELECT count(*) FROM mesh_jobs.jobs),
            'receipts', (SELECT count(*) FROM mesh_jobs.receipts),
            'product', (SELECT count(*) FROM public.mesh_jobs_parity_product))
    """))
    result["exit"] = process.returncode
    h.write_json(out / "fault.json", result)
    assert process.returncode == 1 and (out / "fault-stdout.log").read_text() == "SESSION_READY\n"
    assert result == dict(attempts=4, jobs=0, receipts=0, product=0, exit=1)
    marker = "ordered enqueue diagnostic marker" if kind == "sql-error" else "PostgreSQL connection is unusable"
    assert marker in (out / "fault-stderr.log").read_text()


def safe_evidence(out, password, nonce):
    # Scope evidence to public fixture inputs, numeric server data and text logs.
    # PostgreSQL data, password files, paths to private keys, and process env
    # stay in scratch. Redact bearer strings before the remote archive.
    for file in out.rglob("*"):
        if not file.is_file() or file.is_symlink():
            continue
        data = file.read_bytes()
        sanitized = data.replace(password.encode(), b"<redacted>").replace(nonce.encode(), b"<redacted>")
        if sanitized != data:
            file.write_bytes(sanitized)


def measure(args):
    out, profile = args.evidence, args.profile
    settings = json.loads(h.CONFIG.read_text())
    workload = settings["workloads"]["noop" if profile in ("dense", "component", "faults") else profile]
    slots = 1 if profile == "sparse" else 64
    cap = 5 if profile == "sparse" else 96
    batches = h.inputs(workload, slots, settings["seed"])
    inputs = out / "frozen-input"
    inputs.mkdir()
    paths = h.batch_files(inputs, batches)
    digest = hashlib.sha256("".join(h.digest(p) for p in paths).encode()).hexdigest()
    assert digest == r.PROFILE_HASHES["dense" if profile in ("component", "faults") else profile]
    h.write_json(out / "inputs.json", {"profile": profile, "digest": digest, "files": {p.name: h.digest(p) for p in paths}})
    if profile in ("component", "faults"):
        batches = [{**batches[0], "jobs": batches[0]["jobs"][:128]}]
        h.write_json(out / "component.json", batches[0])
    requests = [json.dumps({"verb": "enqueue", "batch": b}, separators=(",", ":")) + "\n" for b in batches]
    scratch = Path(tempfile.mkdtemp(dir=os.environ["ORDERED_SCRATCH"], prefix=f"{profile}-"))
    with contextlib.ExitStack() as evidence:
        cluster = h.Cluster(scratch, out, cap, "17.11")
        evidence.callback(safe_evidence, out, cluster.password, cluster.nonce)
        configure_tls = cluster.configure_tls

        def configure():
            configure_tls()
            with (cluster.data / "postgresql.conf").open("a") as stream:
                stream.write(f"max_connections = {cap}\nsuperuser_reserved_connections = 0\n"
                             "reserved_connections = 0\nfsync = on\nfull_page_writes = on\n")

        cluster.configure_tls = configure
        results = []
        try:
            cluster.start()
            template = "ordered_template"
            cluster.prepare_database(template)
            cluster.sql(template, "CREATE EXTENSION pg_stat_statements")
            env = dict(cluster.env, MESH_SCHEDULER_MIN_WORKERS="2", MESH_SCHEDULER_MAX_WORKERS="2",
                       MESH_DIAGNOSTIC_JOB_DISABLE_THP="1", MESH_EFFECT_LANES="1" if profile == "sparse" else "4",
                       JOBS_PARITY_NAMESPACE=r.NS, JOBS_PARITY_TOTAL_SLOTS=str(slots),
                       JOBS_PARITY_MESH_BATCH="1", JOBS_PARITY_MESH_LISTEN="1",
                       JOBS_PARITY_MODE="producer_commit" if profile in ("component", "faults") else workload["mode"])
            h.checked([str(STOCK), "migrate"], env=dict(env, JOBS_PARITY_DB_URL=cluster.url(template)),
                      log=out / "template-migrate.log")
            layouts = [("stock-a", "candidate", "stock-b"), ("stock-b", "candidate", "stock-a"),
                       ("candidate", "stock-a", "stock-b"), ("stock-b", "stock-a", "candidate"),
                       ("stock-a", "candidate", "stock-b"), ("stock-b", "candidate", "stock-a")]
            executions = enumerate(
                ((group, arm) for group, order in enumerate(layouts[:args.groups], 1) for arm in order), 1)
            if profile == "faults":
                executions = enumerate([(1, "sql-error"), (1, "disconnect"), (1, "candidate")], 1)
            build_hashes = {str(exe): h.digest(exe) for exe in (STOCK, CANDIDATE)}
            postmaster = int((cluster.data / "postmaster.pid").read_text().splitlines()[0])
            if platform.system() == "Linux":
                assert sorted(os.sched_getaffinity(postmaster)) == sorted(os.sched_getaffinity(0)) == [0, 1]
            for index, (group, arm) in executions:
                trial = out / f"{index:02}-{group:02}-{arm}"
                trial.mkdir()
                database = f"ordered_{index}"
                cluster.sql("postgres", f"CREATE DATABASE {database} WITH TEMPLATE {template} "
                            f"OWNER parity_subject CONNECTION LIMIT {cap}")
                cluster.sql("postgres", "SELECT pg_stat_statements_reset()")
                h.write_json(trial / "host-before.json", host())
                producer_env = dict(env, JOBS_PARITY_DB_URL=cluster.url(database, "parity_producer"))
                try:
                    if arm in ("sql-error", "disconnect"):
                        fault(cluster, database, producer_env, batches[0], trial, arm)
                        continue
                    executable = CANDIDATE if arm == "candidate" else STOCK
                    relay_ctx = r.RecordRelay(cluster.port) if args.wire else contextlib.nullcontext()
                    sent = requests
                    if profile == "faults":
                        rolled_back = {**batches[0], "rollback": True}
                        sent = [json.dumps({"verb": "enqueue", "batch": rolled_back}) + "\n", *requests]
                    with relay_ctx as relay:
                        if relay:
                            producer_env["JOBS_PARITY_DB_URL"] = cluster.url(database, "parity_producer", relay.port)
                        responses = r.session(str(executable), producer_env, sent, trial, relay)
                    if profile == "faults":
                        rolled_back, *responses = responses
                        xid = int(rolled_back["response"]["xid"])
                        stamp = cluster.sql(database, f"SELECT pg_xact_commit_timestamp('{xid}'::xid)")
                        assert not stamp
                        assert not set(rolled_back["response"]["ids"].values()).intersection(
                            responses[0]["response"]["ids"].values())
                        h.write_json(trial / "explicit-rollback.json", {"xid": xid, "commit_timestamp": None,
                            "recovered_same_session": True})
                    h.write_json(trial / "host-after.json", host())
                    witness(cluster, database, responses, batches, profile in ("component", "faults"), trial,
                            len(batches[0]["jobs"]) if profile == "faults" else 0)
                    summary = dict(group=group, arm=arm, pid=postmaster, executable_sha256=build_hashes[str(executable)],
                                   commit_ms=statistics.median(x["commit_ms"] for x in responses),
                                   ack_ms=statistics.median(x["ack_ms"] for x in responses),
                                   requests=[{k: v for k, v in x.items() if k != "response"} for x in responses])
                    h.write_json(trial / "run.json", summary)
                    results.append(summary)
                except BaseException:
                    (trial / "failure.log").write_text(traceback.format_exc())
                    raise
                finally:
                    cluster.sql("postgres", f"DROP DATABASE {database}")
            h.write_json(out / "series.json", results)
        finally:
            cluster.stop()
            shutil.rmtree(scratch)


def term(signum, frame):
    raise SystemExit(128 + signum)


if __name__ == "__main__":
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("profile", choices=("component", "dense", "large", "sparse", "faults"))
    p.add_argument("evidence", type=Path)
    p.add_argument("--groups", type=int, choices=range(1, 7), default=6)
    p.add_argument("--wire", action="store_true")
    signal.signal(signal.SIGTERM, term)
    measure(p.parse_args())
