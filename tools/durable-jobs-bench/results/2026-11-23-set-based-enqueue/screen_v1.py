#!/usr/bin/env python3
"""Screen the ordinary trusted V1 producer: pinned original V1 (A), same-compiler stock relink (A'), set-based (B)."""
import argparse
import fcntl
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import select
import statistics
import subprocess
import sys
import tempfile
import time

LANE = Path(__file__).resolve().parent
ROOT = LANE.parents[1]
SRC = LANE / "source"
ORIGINAL = LANE / "artifacts/v1-original/mesh"  # copy of ordinary-trusted-mesh-queue/local-artifacts-v1, sha256 43c091c8…
STOCK = LANE / "artifacts/v1-stock/mesh"
NEW = LANE / "artifacts/v1-bulk/mesh"
EXE = dict(stock_a=ORIGINAL, stock_b=STOCK, bulk=NEW)
sys.path.insert(0, str(SRC / "scripts"))

def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module

parity = load("jobs_parity", SRC / "scripts/jobs-parity.py")
mach = load("bulk_cpu", LANE / "cpu_sampler.py") if sys.platform == "darwin" else None
ROLES = SRC / "scripts/fixtures/jobs-role-grants.sql"
MODE = "MESH_JOBS_SET_BASED_ENQUEUE"
NS = "bulk_integration"
ORDERS = [
    ("stock_a", "stock_b", "bulk"),
    ("bulk", "stock_a", "stock_b"),
    ("stock_b", "bulk", "stock_a"),
    ("stock_b", "stock_a", "bulk"),
    ("bulk", "stock_b", "stock_a"),
    ("stock_a", "bulk", "stock_b"),
]

def json_out(path, value):
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")

def scope(cluster, db):
    cluster.sql(db, """
       CREATE ROLE bulk_worker;
       CREATE ROLE bulk_operator;
       CREATE ROLE bulk_inspector;
       REVOKE parity_subject FROM parity_producer;
       GRANT INSERT ON public.mesh_jobs_parity_product TO parity_producer;
    """)
    cmd, env = cluster.psql(db)
    subprocess.run([*cmd, "-v", "producer=parity_producer", "-v", "worker=bulk_worker",
                    "-v", "operator=bulk_operator", "-v", "inspector=bulk_inspector",
                    "-f", str(ROLES)], env=env, check=True, timeout=10)

def pid_cpu(pid):
    return sum(mach.backend_cpu(pid)) * mach.TIMEBASE[0] / mach.TIMEBASE[1] / 1e6

def session(executable, env, requests, out):
    with (out / "producer-stderr.log").open("x") as stderr:
        with subprocess.Popen([str(executable), "session"], env=env, text=True,
                              stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=stderr) as proc:
            def read():
                if not select.select([proc.stdout], [], [], 30)[0]:
                    raise TimeoutError("producer did not respond")
                line = proc.stdout.readline()
                if not line:
                    raise RuntimeError(f"producer output closed with status {proc.poll()}")
                return line.rstrip("\n")

            def send(value):
                proc.stdin.write(value + "\n")
                proc.stdin.flush()
                return read()

            try:
                assert read() == "SESSION_READY"
                sql = """SELECT json_build_object('pid',pg_backend_pid(),'role',current_user,
                           'ssl',(SELECT ssl FROM pg_stat_ssl WHERE pid=pg_backend_pid()),
                           'settings', ARRAY[current_setting('server_version_num'),
                             current_setting('synchronous_commit'),current_setting('fsync'),
                             current_setting('full_page_writes'),current_setting('track_commit_timestamp')])
                           AS value"""
                identity = json.loads(send(json.dumps(dict(verb="query", sql=sql))))
                json_out(out / "session.json", identity)
                assert identity["role"] == "parity_producer" and identity["ssl"]
                assert identity["settings"] == ["170011", "on", "on", "on", "on"]
                receipts = []
                for index, request in enumerate(requests):
                    pg_before, proc_before = pid_cpu(identity["pid"]), pid_cpu(proc.pid)
                    submitted_us = time.time_ns() // 1000
                    start = time.perf_counter_ns()
                    raw = send(request)
                    wall = (time.perf_counter_ns() - start) / 1e6
                    acknowledged_us = time.time_ns() // 1000
                    pg_after, proc_after = pid_cpu(identity["pid"]), pid_cpu(proc.pid)
                    result = dict(submitted_us=submitted_us, acknowledged_us=acknowledged_us,
                                  wall_ms=wall, backend_cpu_ms=pg_after-pg_before,
                                  producer_cpu_ms=proc_after-proc_before, response=json.loads(raw))
                    receipts.append(result)
                    json_out(out / f"response-{index+1:03}.json", result)
                assert send("stop") == "{}"
                assert proc.wait(timeout=5) == 0
                return receipts
            finally:
                if proc.poll() is None:
                    proc.kill()
                    proc.wait(timeout=5)

def canonical_job(job):
    return dict(worker="parity", version=1, args=job, queue="default", metadata={},
                tags=[], priority=0, scheduled_at_ms=None, delay_ms=None, max_attempts=3,
                timeout_ms=60000, backoff_base_ms=1000, backoff_max_ms=1000,
                suspended=False, unique=None, replace={})

def verify(cluster, db, batches, results, fast, profile, out):
    ledger = json.loads(cluster.sql(db, """
    SELECT json_build_object(
      'counts', ARRAY[(SELECT count(*) FROM mesh_jobs.jobs),(SELECT count(*) FROM mesh_jobs.receipts)],
      'commits', (SELECT json_object_agg(p.request_id,json_build_array(p.xmin::text,
           floor(extract(epoch FROM pg_xact_commit_timestamp(p.xmin))*1000000)::bigint))
         FROM public.mesh_jobs_parity_product p),
      'jobs', (SELECT json_agg(json_build_object(
         'key',j.args->>'key','id',j.id,'payload',encode(sha256(convert_to(j.args->>'payload','UTF8')),'hex'),
         'position',j.position,'fp',r.fingerprint,'receipt',r.receipt,'effect',mesh_jobs.effect_key(j),
         'run',j.run,'state',j.state,'attempt',j.attempt,
         'clock',j.scheduled_at_ms=j.inserted_at_ms,
         'job_commit',floor(extract(epoch FROM pg_xact_commit_timestamp(j.xmin))*1000000)::bigint,
         'receipt_commit',floor(extract(epoch FROM pg_xact_commit_timestamp(r.xmin))*1000000)::bigint
       ) ORDER BY j.position)
       FROM mesh_jobs.jobs j JOIN mesh_jobs.receipts r ON r.namespace=j.namespace AND r.request_key=j.args->>'key')
    )"""))
    json_out(out / "ledger.json", ledger)
    count = sum(len(batch["jobs"]) for batch in batches)
    assert ledger["counts"] == [count,count] and len(ledger["jobs"]) == count
    assert len(ledger["commits"]) == len(results) == len(batches)
    assert [row["position"] for row in ledger["jobs"]] == list(range(1,count+1))
    assert len({row["id"] for row in ledger["jobs"]}) == count
    offset = 0
    for batch, result in zip(batches, results, strict=True):
        xid, commit = ledger["commits"][batch["request_id"]]
        assert result["submitted_us"] <= commit <= result["acknowledged_us"]
        result["commit_ms"] = (commit - result["submitted_us"]) / 1000
        stored = ledger["jobs"][offset:offset+len(batch["jobs"])]
        response = result["response"]
        assert response["xid"] == xid and response["count"] == len(stored)
        assert response["ids"] == {r["key"]:r["id"] for r in stored}
        if "receipts" in response:
            assert response["receipts"] == [r["receipt"] for r in stored]
        for row, job in zip(stored, batch["jobs"], strict=True):
            spec = json.dumps(canonical_job(job), sort_keys=True, separators=(",", ":"), ensure_ascii=False)
            assert row["fp"] == hashlib.sha256(spec.encode()).hexdigest()
            assert row["key"] == job["key"]
            assert row["payload"] == hashlib.sha256(job["payload"].encode()).hexdigest()
            assert row["job_commit"] == row["receipt_commit"] == commit
            assert row["clock"] and (row["attempt"],row["run"],row["state"]) == (0,1,"available")
            assert row["effect"] == row["id"] + "/1"
            assert row["receipt"] == dict(id=row["id"],effect_key=row["effect"],
                                          inserted=True,conflict=False,state="available")
        offset += len(stored)
    statements = json.loads(cluster.sql(db, """
        SELECT json_object_agg(query,json_build_object('calls', calls, 'rows', rows, 'exec_ms', total_exec_time))
          FROM pg_stat_statements
         WHERE dbid=(SELECT oid FROM pg_database WHERE datname=current_database())
           AND userid=(SELECT oid FROM pg_roles WHERE rolname='parity_producer')"""))
    json_out(out / "pg-stat-statements.json", statements)
    def count_calls(prefix):
        return sum(v["calls"] for k,v in statements.items() if k.startswith(prefix))
    bulk = count_calls("SELECT ordinal, receipt FROM mesh_jobs.enqueue_batch_v1(")
    scalar = count_calls("SELECT mesh_jobs.enqueue(")
    if fast and profile == "slow":
        # 16 KiB specs reach the producer's 1 MiB enqueue_in cap at about 63 jobs.
        # A trailing remainder below MIN_SET_BASED_BATCH (16) keeps the ordered writer.
        bulk_rows = sum(v["rows"] for k,v in statements.items()
                        if k.startswith("SELECT ordinal, receipt FROM mesh_jobs.enqueue_batch_v1("))
        assert bulk >= count // 64 and bulk_rows + scalar == count and scalar < 16, statements
    else:
        assert (bulk,scalar) == ((count//128 if profile == "noop" else 1, 0)
                                 if fast and profile in ("noop","short") else (0,count)), statements
    return dict(bulk_calls=bulk, scalar_calls=scalar)

def run(out, profiles, groups):
    out.mkdir(parents=True)
    settings = json.loads(parity.CONFIG.read_text())
    template = "bulk_template"
    cap = 16
    workloads = {profile:parity.inputs(settings["workloads"][profile], slots, settings["seed"])
                 for profile,slots in (("noop",64),("short",16),("large",64),("sparse",1),("slow",64))}
    workloads["small"] = [{**workloads["short"][0], "jobs":workloads["short"][0]["jobs"][:2]}]
    json_out(out / "manifest.json", dict(orders=ORDERS[:groups], profiles=profiles,
             calibration=mach.calibrate_cpu(), host=list(os.uname()), load=os.getloadavg(),
             executable_sha256={arm:[str(binary),parity.digest(binary)] for arm,binary in EXE.items()},
             inputs={p:[hashlib.sha256(json.dumps(b,sort_keys=True,separators=(",", ":")).encode()).hexdigest()
                        for b in workloads[p]] for p in profiles}))
    with (LANE.parent / "queue-performance.lock").open("r") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        with tempfile.TemporaryDirectory(dir=LANE/"tmp", prefix="screen-") as scratch:
            cluster = parity.Cluster(Path(scratch), out, cap, "17.11")
            try:
                cluster.start()
                cluster.prepare_database(template)
                cluster.sql(template, "CREATE EXTENSION pg_stat_statements")
                env = dict(cluster.env,JOBS_PARITY_NAMESPACE=NS,JOBS_PARITY_TOTAL_SLOTS="16",
                           JOBS_PARITY_MESH_LISTEN="1",MESH_SCHEDULER_MIN_WORKERS="2",
                           MESH_SCHEDULER_MAX_WORKERS="2")
                parity.checked([str(NEW),"migrate"], env=dict(env,JOBS_PARITY_DB_URL=cluster.url(template)),
                               log=out/"migrate.log")
                scope(cluster,template)
                series = []
                for profile in profiles:
                    batches = workloads[profile]
                    requests = [json.dumps(dict(verb="enqueue",batch=b),separators=(",", ":")) for b in batches]
                    for i,order in enumerate(ORDERS[:groups]):
                        for arm in order:
                            trial = out/f"{profile}-{i+1:02}-{arm}"
                            trial.mkdir()
                            db = "bulk_trial"
                            cluster.sql("postgres", f"CREATE DATABASE {db} WITH TEMPLATE {template} OWNER parity_subject")
                            try:
                                load = os.getloadavg()
                                fast = arm == "bulk"
                                trial_env = dict(env, JOBS_PARITY_DB_URL=cluster.url(db,"parity_producer"),
                                                 JOBS_PARITY_MODE="producer_commit" if profile in ("small","short","sparse") else "dense")
                                trial_env[MODE] = "1" if fast else "0"
                                receipts = session(EXE[arm],trial_env,requests,trial)
                                calls = verify(cluster,db,batches,receipts,fast,profile,trial)
                                timings = {key:sum(rec[key] for rec in receipts)
                                           for key in ("wall_ms","commit_ms","backend_cpu_ms","producer_cpu_ms")}
                                row = dict(profile=profile,group=i+1,arm=arm,load=load,**calls,**timings)
                                series.append(row)
                                json_out(trial/"run.json",row)
                                print(profile,i+1,arm,timings,flush=True)
                            finally:
                                cluster.sql("postgres",f"DROP DATABASE {db}")
                json_out(out/"series.json",series)
                def stats(vals):
                    return dict(min=min(vals), median=statistics.median(vals), max=max(vals))
                summary = {p:{arm:{key:stats([r[key] for r in series if r["profile"]==p and r["arm"]==arm])
                                    for key in ("wall_ms","commit_ms","backend_cpu_ms","producer_cpu_ms")}
                              for arm in ORDERS[0]} for p in profiles}
                json_out(out/"summary.json",summary)
            finally:
                cluster.stop()

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("label")
    parser.add_argument("--groups",type=int,choices=range(1,len(ORDERS)+1),default=5)
    parser.add_argument("--profiles",nargs="+",choices=["short","noop","large","sparse","small","slow"],
                        default=["slow"])
    args = parser.parse_args()
    run(LANE/"evidence"/args.label,args.profiles,args.groups)
