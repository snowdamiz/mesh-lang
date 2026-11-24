"""Validate native Mesh arm receipts independently of the live adapter."""
from collections import Counter
import json
import tarfile

from matrix import CAP, CPUS, INPUT, JOBS, diagnostic_reconciler, require, run, text
from interleave import host_cpu


V1 = "mesh-v1-2x32"
MODES = {"d2-off-a": "d2-off", "d2-off-b": "d2-off", "d2-on": "d2-on",
         "r2-off-a": "r2-off", "r2-off-b": "r2-off", "r2-zero": "r2-zero", V1: "v1"}
ROLES = {"compiled_caller": 32, "parity_subject": 8, "parity_producer": 1}


def child_identity(record, file, pin, verb, adapter):
    require(record["argv"] == [str(file), verb] and record["executable_sha256"] == pin
            and record["cpus"] == CPUS and record["parent_pid"] == adapter
            and record["launch_start_ticks"] == record["start_ticks_before"] == record["start_ticks_after"] > 0,
            "direct process identity differs")
    require((record["method"], record["observed_executable"], record["exe_errno"])
            in (("proc-exe", str(file), None), ("owned-launch", None, 13)),
            "unexpected kernel executable fallback")


def score(archive, phase, label, config, spec):
    def files(name):
        return run.file_json(archive, f"adapter/{name}")

    def logs(name):
        return text(archive, f"adapter/{name}")

    ledger, raw, row, launch = map(files, ("ledger.json", "raw.json", "row.json", "dense-launches.json"))
    v1 = label == V1
    contract = "ordinary-trusted-shared-process" + ("" if v1 else "-v2")
    require(ledger["phase"] == row["phase"] == phase and ledger["local_smoke"] is row["local_smoke"] is False
            and ledger["execution_contract"] == row["execution_contract"] == contract
            and ledger["database"] == files("postgres-settings.json") and raw["case_sha256"] == INPUT
            and row["completed"] == row["reconciled_jobs"] == JOBS and row["committed_work_records"] == 2 * JOBS
            and row["durable_enqueue_transactions"] == 1 and row["connection_cap"] == CAP
            and row["peak_processes"] == 2 and row["poll_ms"] == 250, "dense input, durability or topology differs")
    result = diagnostic_reconciler(2)(ledger, "mesh", "noop", {key: spec[key] for key in ("version", "artifact_sha256")})
    origin, = ledger["transactions"]
    require(launch["observed_us"] < origin["submitted_us"] < origin["committed_us"] <= origin["acknowledged_us"]
            and launch["mode"] == MODES[label] and launch["cpus"] == CPUS, "pre-submit or arm mode differs")
    broker, runner = config["mesh_broker"][label], config["mesh_runner"][label]
    build = files("build.json")
    require(launch["broker_manifest"] == spec["artifact_sha256"] == broker["manifest"]
            and build["files"]["mesh"] == broker["elf"] and launch["runner_manifest"] == runner["manifest"],
            "broker or runner manifest differs")
    child_identity(launch["broker"], broker["path"] + "/mesh", broker["elf"], "session", launch["adapter_pid"])
    require(logs("broker-output.log").splitlines()[0] == "SESSION_READY", "no warm broker")
    db = files("database.json")
    require(db == dict(commit_timestamps=True, database_connection_limit=96, durable=True, max_connections=96,
                       reserve=0, superuser_reserve=0, tls=True, user="parity_producer"), "broker durability differs")

    first = {r["backend"]: r for r in launch["first_runner_tls"]}
    both = {r["backend"]: r for r in launch["both_runners_tls"]}
    new = both.keys() - first.keys()
    require(len(first) == len(launch["first_runner_tls"]) == 21
            and len(both) == len(launch["both_runners_tls"]) == 41
            and len({r["client_port"] for r in both.values()}) == 41
            and all(r["tls"] for r in both.values())
            and all(both.get(pid) == r for pid, r in first.items())
            and Counter(r["role"] for r in first.values()) == {
                "compiled_caller": 16, "parity_subject": 4, "parity_producer": 1}
            and Counter(both[pid]["role"] for pid in new) == {"compiled_caller": 16, "parity_subject": 4}
            and Counter(r["role"] for r in both.values()) == ROLES
            and {(r["role"], r["backend"], r["tls"]) for r in both.values()} ==
                {(r["role"], r["backend"], r["tls"]) for r in files("postgres-tls-sessions.json")}
            and row["peak_connections"] == result["peak_connections"] == 41,
            "warm TLS lanes or physical peak changed")

    jobs = ledger["jobs"]
    by_pid = Counter(job["worker_pid"] for job in jobs)
    launch_pids = [item["pid"] for item in launch["runners"]]
    require(len(launch_pids) == len(by_pid) == 2 and set(launch_pids) == by_pid.keys()
            and launch["broker"]["pid"] not in by_pid, "actual runner PIDs differ")
    affinity = files("process-affinity.json")
    require(len(affinity) == 3 and set(map(int, affinity)) >= by_pid.keys()
            and all(mask == CPUS for mask in affinity.values()), "runner or PostgreSQL affinity changed")
    for index, record in enumerate(launch["runners"]):
        child_identity(record, runner["path"] + "/mesh", runner["elf"], "run", launch["adapter_pid"])
        require(record["options"] == {
            "MESH_PG_HOT_CHECKOUT": "1" if label == "d2-on" else None,
            "TRUSTED_COMPLETION_WRITER_MS": "0" if label == "r2-zero" else None,
            "MESH_EFFECT_LANES": "16", "JOBS_PARITY_SLOTS": "32",
            "MESH_SCHEDULER_MIN_WORKERS": "2", "MESH_SCHEDULER_MAX_WORKERS": "2",
            "JOBS_PARITY_MESH_BATCH": "1", "JOBS_PARITY_MESH_LISTEN": "1",
        }, "runner child options differ")
        report = json.loads(logs(f"runner-{index}.log").splitlines()[-1])
        queue = report["queues"]["default"]
        require(report["namespace"] == raw["namespace"] and report["state"] == queue["state"] == "stopped"
                and report["drained"] and queue["drained"] and report["last_error"] is None
                and report["claimed"] == report["finished"] == report["outcomes"]["completed"] == by_pid[record["pid"]]
                and all(n == 0 for name, n in report["outcomes"].items() if name != "completed")
                and not any(report[k] for k in ("undrained", "recovered", "stale_acks", "pending_claims"))
                and queue["limit"] == 32 and queue["in_flight"] == queue["pending_claims"] == 0,
                "runner failed to drain")

    cohort = Counter(int(job["settled_xid"]) for job in jobs)
    work = {int(job[k]) for job in jobs for k in ("attempt_xid", "effect_xid")}
    require(len(work) == 2 * JOBS and work.isdisjoint(cohort)
            and int(origin["xid"]) % (2**32) not in work | cohort.keys(),
            "producer, application or settlement transactions share an XID")
    if v1:
        require(len(cohort) == JOBS and max(cohort.values()) == 1, "V1 stock must settle individually")
        admitted = 0
    else:
        queue = files("queue-policy.json")
        admitted = queue["admitted_queries"]
        require(ledger["queue_policy"] == row["queue_policy"] == queue["queue_policy"] == "success_cohort"
                and len(cohort) == queue["terminal_xids"]
                and 1 <= max(cohort.values()) == queue["largest_terminal_cohort"] <= 8 and admitted > 0,
                "V2 cohort evidence changed")
    producer = files("producer-writer.json")
    bulk = sum(n for query, n in producer["statements"].items()
               if query.startswith("SELECT ordinal, receipt FROM mesh_jobs.enqueue_batch_v1("))
    scalar = sum(n for query, n in producer["statements"].items() if query.startswith("SELECT mesh_jobs.enqueue("))
    require((bulk, scalar) == (producer["bulk"], producer["scalar"]) == ((0, JOBS) if v1 else (8, 0))
            and producer["enabled"] is (not v1), "producer SQL route differs")

    statements, records = map(files, ("trusted-application-statements.json", "application-records.json"))
    for table, field in (("attempts", "attempt_xid"), ("effects", "effect_xid")):
        require(len(records[table]) == JOBS
                and {(r["engine"], r["namespace"], r["job_key"], r["xid"]) for r in records[table]}
                    == {("mesh", raw["namespace"], j["job_key"], j[field]) for j in jobs}, "application rows differ")
        stmt, = [s for s in statements if s["query"].startswith("INSERT INTO public.mesh_jobs_parity_" + table)]
        require(stmt["calls"] == stmt["rows"] == JOBS, "application SQL write counts differ")
    health = [r for r in statements if r["query"] == "SELECT $1"]
    probes = sum(r["calls"] for r in health)
    require(len(health) <= 1 and len(statements) == 2 + len(health)
            and probes == sum(r["rows"] for r in health)
            and raw["raw_application_sql_calls"] == 2 * JOBS + probes
            and (0 <= probes < 2 * JOBS if label == "d2-on" else probes == 2 * JOBS),
            "actual application checkout probes differ")
    return dict(result, checkout_probes=probes, enqueues_bulk=bulk, enqueues_scalar=scalar,
                terminal_xids=len(cohort), max_terminal_cohort=max(cohort.values()), admitted_queries=admitted,
                last_effect_us=max(j["effect_committed_us"] for j in jobs),
                probe_sql_exec_ms=sum(r["execution_ms"] for r in health),
                application_sql_exec_ms=sum(r["execution_ms"] for r in statements))


def raw_score(folder, phase, label, config, inventory):
    with tarfile.open(folder / "raw.tar.gz", "r:gz") as archive:
        request = run.read_json(folder / "request.json")
        invocation = run.file_json(archive, "invocation.json")
        for key, expected in dict(profile="noop", engine="mesh", phase=phase, attempt=request["attempt"]).items():
            require(invocation[key] == request[key] == expected, f"native invocation differs for {key}")
        require(run.file_json(archive, "status.json")["exit_code"] == 0, "native adapter failed")
        host = run.file_json(archive, "host.json")
        spec = inventory["engines"]["mesh"]
        require([host["boot_id"], host["kernel"], host["pg_mount"]["maj:min"]] == config["host"]
                and host["affinity"] == CPUS and len(host["available_cpus"]) == 4
                and host["verified_files"] == spec["verified_files"], "native host or executable pins differ")
        result = score(archive, phase, label, config, spec)
        with archive.extractfile("host-samples.jsonl") as stream:
            result.update(run.sample_window(host, [json.loads(line) for line in stream], result))
    return dict(result, **host_cpu(folder, result, config["clock_ticks"]),
                raw_sha256=run.digest(folder / "raw.tar.gz"))
