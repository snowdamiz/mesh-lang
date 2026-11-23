"""Reconcile V1 topology evidence and print its paired measurements."""

import argparse
from collections import Counter
import json
from pathlib import Path
import runpy
from statistics import median
import sys
import tarfile

from arm import INPUT, LAYOUTS, ORDERS, read, sha

PACKET = Path(__file__).resolve().parent
ROOT = PACKET.parents[3]
VERIFY = ROOT / "tools/durable-jobs-bench/verify.py"
sys.path.insert(0, str(VERIFY.parent))
from verify import PROFILES, reconcile, require, xid

CONTRACT = "ordinary-trusted-shared-process"
CLAIM_TURNS = PACKET.parent / "2026-11-23-ordinary-claim-turns/replay.py"
minimum_turns = runpy.run_path(str(CLAIM_TURNS))["minimum_turns"]
METRICS = (
    "submit_jobs_per_s", "commit_jobs_per_s", "first_submit_to_commit_ms",
    "runner_cpu_ms_per_job", "queue_calls", "broker_calls", "app_calls", "claim_calls",
    "admission_hints", "hint_percent", "grant_jobs_per_claim_call",
    "min_granting_calls", "max_granting_calls",
    "warm_queue_sessions", "warm_application_sessions", "active_application_backends_observed",
    "used_application_backends", "sampled_peak_slots", "claim_to_terminal_peak_slots",
    "mean_occupied_slots", "max_grants_per_runner", "min_grants_per_runner",
    "attempt_to_effect_commit_p50_ms", "effect_commit_to_settlement_p50_ms",
)


def intervals(jobs):
    events = [(job[key], change) for job in jobs
              if job["claimed_us"] < job["settled_us"]
              for key, change in (("claimed_us", 1), ("settled_us", -1))]
    occupancy = peak = 0
    for _, change in sorted(events):
        occupancy += change
        peak = max(peak, occupancy)
    return peak


def verify_app(files, jobs, namespace):
    records = files("application-records.json")
    statements = files("trusted-application-statements.json")
    for table, field in (("attempts", "attempt_xid"), ("effects", "effect_xid")):
        require({(record["engine"], record["namespace"], record["job_key"], record["xid"])
                 for record in records[table]} ==
                {("mesh", namespace, job["job_key"], job[field]) for job in jobs}
                and len(records[table]) == len(jobs), f"{table} do not match the work ledger")
        inserts = [entry for entry in statements
                   if entry["query"].startswith(f"INSERT INTO public.mesh_jobs_parity_{table}")]
        require(len(inserts) == 1 and inserts[0]["calls"] == inserts[0]["rows"] == len(jobs),
                f"{table} SQL counts differ from the work ledger")
    health = [entry for entry in statements if entry["query"] == "SELECT $1"]
    require(len(statements) == 3 and len(health) == 1
            and health[0]["calls"] == health[0]["rows"] == 2 * len(jobs),
            "unexpected application-side SQL")
    return sum(entry["execution_ms"] for entry in statements)


def verify_sessions(files, expected, first_submit, sample_active):
    snapshots = [files(name) for name in ("postgres-tls-sessions.json", "sessions-before.json",
                                         "sessions-end.json")]
    identities = []
    for sessions in snapshots:
        require(Counter(row["role"] for row in sessions) == expected and all(row["tls"] is True
                for row in sessions), "physical warm TLS allocations differ")
        by_pid = {row["backend"]: row["role"] for row in sessions}
        require(len(by_pid) == len(sessions), "duplicate PostgreSQL backend PID")
        identities.append(by_pid)
    require(identities[0] == identities[1] == identities[2],
            "warmed backend topology changed before terminal observation")

    before = {row["backend"]: (row["client_port"], row["query_us"])
              for row in snapshots[1] if row["role"] == "compiled_caller"}
    end = {row["backend"]: row for row in snapshots[2] if row["role"] == "compiled_caller"}
    require(all(row["client_port"] == before[pid][0] for pid, row in end.items()),
            "physical app client endpoints changed")
    used = sorted(pid for pid, row in end.items() if row["query_us"] != before[pid][1])
    require(bool(used) and all(end[pid]["query_us"] >= first_submit and
                end[pid]["query"].startswith("INSERT INTO public.mesh_jobs_parity_")
                for pid in used), "unexpected app session query during the measured window")
    require(set(sample_active) <= set(used), "sampled active app backend missing from used sessions")
    return used


def verify_arm(files, log, label, pins, *, local, sample_activity):
    runners, per_runner, apps = LAYOUTS[label]
    ledger, row, raw, build, setup = [files(name + ".json")
                                      for name in ("ledger", "row", "raw", "build", "topology")]
    artifact = dict(version=pins["version"], artifact_sha256=pins["manifest_sha256"])
    require(build["files"]["mesh"] == pins["executable_sha256"] and
            build["version"] == artifact["version"], "ordinary V1 executable identity differs")
    require(all(row[key] == value for key, value in artifact.items())
            and row["input_sha256"] == raw["case_sha256"] == INPUT
            and ledger["database"] == files("postgres-settings.json"), "arm receipt identity differs")
    require(row["runners"] == ledger["runners"] == runners
            and row["slots"] == ledger["slots"] == 64 and row["connection_cap"] == 96,
            "runner or database slot budget differs")
    require(setup["runners"] == runners and setup["slots_per_runner"] == per_runner
            and setup["app_sessions_per_runner"] == apps and setup["label"] == label
            and setup["scheduler_min"] == setup["scheduler_max"] == 2
            and setup["sample_activity"] == sample_activity, "unexpected process or scheduler configuration")
    require(row["phase"] == ledger["phase"] == "diagnostic"
            and row["local_smoke"] is ledger["local_smoke"] is local
            and row["execution_contract"] == ledger["execution_contract"] ==
            build["execution_contract"] == CONTRACT,
            "ordinary V1 diagnostic and broker contract differs")
    broker = files("database.json")
    require(log("broker-output.log").splitlines()[0] == "SESSION_READY"
            and broker["user"] == "parity_producer" and broker["tls"] and broker["durable"]
            and broker["max_connections"] == broker["database_connection_limit"] == 96,
            "missing verified durable broker")
    # The canonical verifier fixes eight runners. Actual process, pool, and
    # per-runner budgets are validated here before projecting its ledger view.
    score = reconcile(dict(ledger, runners=PROFILES["noop"][0]), "mesh", "noop", artifact)
    jobs, transactions = ledger["jobs"], ledger["transactions"]
    require(len(jobs) == 1024 and len(transactions) == 1
            and transactions[0]["submitted_us"] <= transactions[0]["committed_us"]
            <= transactions[0]["acknowledged_us"], "wrong dense workload or producer acknowledgement")
    all_xids = [xid(job[key], key) for job in jobs
                for key in ("attempt_xid", "effect_xid", "settled_xid")]
    all_xids.append(xid(transactions[0]["xid"], "product xid"))
    require(len(set(all_xids)) == 3 * len(jobs) + 1, "work, terminal, or product XIDs overlap")
    namespace = raw["namespace"]
    app_sql_ms = verify_app(files, jobs, namespace)
    caps = {"compiled_caller": 32, "parity_subject": 4 * runners, "parity_producer": 1}
    sql = files("sql-capacity.json")
    require(sql["role_caps"] == caps
            and sql["sql_queues"] == [dict(name="default", state="active", namespace=namespace, concurrency=64)],
            "SQL admission or role capacity differs")
    require(row["peak_connections"] == ledger["peak_connections"] == sum(caps.values())
            and all(sample["connections"] == sum(caps.values()) for sample in raw["idle_samples"])
            and row["peak_processes"] == runners, "observed physical budget differs")
    samples = raw["samples"]
    require(bool(samples) and all(("app_active" in sample) == sample_activity for sample in samples),
            "sample mode differs")
    sampled_active = [set(sample.get("app_active", [])) for sample in samples]
    used = verify_sessions(files, caps, score["first_submit_us"], set.union(*sampled_active))

    pids = setup["runner_pids_by_index"]
    require(len(set(pids)) == len(pids) == runners
            and {job["worker_pid"] for job in jobs} <= set(pids), "unrecognized handler process")
    reports = [json.loads(log(f"runner-{index}.log").splitlines()[-1]) for index in range(runners)]
    grants, hints, peaks, lower_grants = [], [], [], []
    for pid, report in zip(pids, reports, strict=True):
        owned = [job for job in jobs if job["worker_pid"] == pid]
        queue = report["queues"]["default"]
        require(report["namespace"] == namespace and report["drained"] and report["last_error"] is None
                and not any(report[field] for field in ("undrained", "recovered", "stale_acks", "pending_claims"))
                and report["claimed"] == report["finished"] == report["outcomes"]["completed"] == len(owned)
                and all(count == 0 for outcome, count in report["outcomes"].items() if outcome != "completed")
                and queue["limit"] == per_runner and queue["in_flight"] == queue["pending_claims"] == 0,
                "runner error, work mismatch, or incorrect local capacity")
        require(type(report["batch_contention_hints"]) is int and report["batch_contention_hints"] >= 0,
                "missing non-grant hint count")
        grants.append(len(owned))
        hints.append(report["batch_contention_hints"])
        peaks.append(intervals(owned))
        lower_grants.append(minimum_turns(
            [(job["claimed_us"], job["entered_us"]) for job in owned], min(16, per_runner)))

    aggregate_peak = intervals(jobs)
    sampled_peak = max(item["executing"] for item in samples)
    # The lower-resolution claim stamp precedes admission commit and terminal
    # commit precedes the runtime's slot release. These intervals are proxies.
    require(sampled_peak == ledger["peak_executing"] <= 64 and aggregate_peak <= 64
            and all(peak <= per_runner for peak in peaks),
            "SQL observation exceeded executing capacity")
    counters = [files(f"counters-{number}.json") for number in (1, 2, 3)]
    require(len({item["dealloc"] for item in counters}) == 1
            and counters[1]["runner"] - counters[0]["runner"] == raw["raw_idle_sql_calls"],
            "counter eviction or idle accounting mismatch")
    for field, name in (("runner", "runner_sql"), ("enqueue", "enqueue_sql"),
                        ("application", "application_sql"), ("claims", "claim")):
        require(counters[2][field] - counters[1][field] == raw[f"raw_{name}_calls"],
                f"{field} counter delta mismatch")
    claim_calls = raw["raw_claim_calls"]
    # A runner can still consume one hinted claim reply after the last counter.
    minimum_window_hints = sum(hints) - sum(count > 0 for count in hints)
    max_granting_calls = claim_calls - minimum_window_hints
    require(0 < sum(lower_grants) <= max_granting_calls <= claim_calls
            and raw["raw_application_sql_calls"] == 4 * len(jobs)
            and raw["runner_cpu_seconds_including_startup_and_idle"] == ledger["runner_cpu_seconds"],
            "invalid claim, app call, or CPU accounting")
    drain_us = score["last_settlement_us"] - score["first_commit_us"]
    residence = sum(job["settled_us"] - max(job["claimed_us"], score["first_commit_us"]) for job in jobs)
    return {**score, "layout": LAYOUTS[label], "queue_calls": raw["raw_runner_sql_calls"],
            "broker_calls": raw["raw_enqueue_sql_calls"], "app_calls": raw["raw_application_sql_calls"],
            "claim_calls": claim_calls, "admission_hints": sum(hints),
            "hint_percent": 100 * sum(hints) / claim_calls,
            "minimum_hints_in_sql_window": minimum_window_hints,
            "min_granting_calls": sum(lower_grants), "max_granting_calls": max_granting_calls,
            "grant_jobs_per_claim_call": len(jobs) / claim_calls,
            "grants_by_runner_index": grants, "hints_by_runner_index": hints, "peak_slots_by_runner_index": peaks,
            "min_grants_per_runner": min(grants), "max_grants_per_runner": max(grants),
            "warm_queue_sessions": caps["parity_subject"], "warm_application_sessions": caps["compiled_caller"],
            "warm_broker_sessions": caps["parity_producer"], "used_application_backends": len(used),
            "used_application_backend_ids": used,
            "active_application_backends_observed": max(map(len, sampled_active)) if sample_activity else None,
            "sampled_peak_slots": sampled_peak, "claim_to_terminal_peak_slots": aggregate_peak,
            "mean_occupied_slots": residence / drain_us,
            "app_statement_execution_ms": app_sql_ms, "unique_product_work_terminal_xids": len(set(all_xids))}


def distribution(values):
    return dict(n=len(values), median=median(values), min=min(values), max=max(values))


def summarize(groups):
    labels = list(LAYOUTS)
    rollup = {label: {} for label in labels}
    paired = {}
    for label in labels:
        for key in METRICS:
            values = [group["arms"][label][key] for group in groups
                      if label in group["arms"] and group["arms"][label][key] is not None]
            if values:
                rollup[label][key] = distribution(values)
    for label in labels[1:]:
        paired[label] = {}
        for key in ("submit_jobs_per_s", "commit_jobs_per_s", "runner_cpu_ms_per_job"):
            values = []
            for group in groups:
                arms = group["arms"]
                if not all(name in arms for name in ("stock-a", "stock-b", label)):
                    continue
                control = arms["stock-a"][key] if label == "stock-b" else median(
                    arms[name][key] for name in ("stock-a", "stock-b"))
                values.append(100 * (arms[label][key] / control - 1))
            if values:
                paired[label][key] = dict(distribution(values), by_group=values,
                                          positive=sum(value > 0 for value in values))
    return dict(rollup=rollup, paired_percent=paired)


def local(path):
    manifest = read(path / "series.json")
    pins = manifest["pins"]
    require(pins == read(PACKET / "local-pins.json") and sha(VERIFY) == pins["verifier_sha256"],
            "local source pin or verifier differs")
    require(manifest["input_sha256"] == INPUT and manifest["orders"] ==
            [list(order) for order in ORDERS[:len(manifest["orders"])]], "unexpected paired design")
    require(sha(PACKET / "arm.py") == manifest["arm_sha256"]
            and sha(PACKET / "local.py") == manifest["dispatch_sha256"]
            and read(path / "finished.json")["inputs_unchanged"], "series source or input drift")
    pins = dict(pins, version="3828f20d2d6beb683868cce33e6dd1958365914b")
    failures = []
    groups = []
    for number, order in enumerate(manifest["orders"], 1):
        arms = {}
        for label in order:
            arm = path / f"{number:02}" / label
            try:
                status = read(arm.with_suffix(".status.json"))
                require(status["exit_code"] == 0, f"arm exit {status['exit_code']}")
                arms[label] = verify_arm(lambda name: read(arm / name),
                                         lambda name: (arm / name).read_text(),
                                         label, pins, local=True, sample_activity=manifest["sample_activity"])
                arms[label]["host_load_1m"] = status["load_before"][0]
                arms[label]["arm_wall_s"] = status["wall_s"]
            except (ValueError, TypeError, OSError, KeyError, IndexError) as exc:
                failures.append(dict(group=number, label=label, error=str(exc)))
        groups.append(dict(number=number, order=order, arms=arms))
    return dict(groups=groups, failures=failures, **summarize(groups))


def native(path):
    manifest = read(path / "series.json")
    pins = read(PACKET / "native-pins.json")
    groups, failures = [], []
    for number, order in enumerate(manifest["orders"], 1):
        require(order == list(ORDERS[number - 1]), "unexpected native paired design")
        arms = {}
        for label in order:
            outer = path / f"{number:02}" / label / "diagnostic-mesh"
            try:
                require(read(outer / "dispatch.json").get("exit_code") == 0, "native dispatch failed")
                with tarfile.open(outer / "raw.tar.gz", "r:gz") as archive:
                    def content(name):
                        member = archive.getmember(name)
                        require(member.isfile(), f"invalid member {name}")
                        return archive.extractfile(member).read().decode()

                    request = read(outer / "request.json")
                    require(json.loads(content("invocation.json"))["attempt"] == request["attempt"]
                            and json.loads(content("status.json"))["exit_code"] == 0, "native exit mismatch")
                    host = json.loads(content("host.json"))
                    require(host["affinity"] == pins["cpus"] and
                            len(host["available_cpus"]) == 4
                            and host["boot_id"] == pins["host"][0] and host["kernel"] == pins["host"][1]
                            and host["pg_mount"]["maj:min"] == pins["host"][2]
                            and all(host["verified_files"].get(name) == digest
                                    for name, digest in pins["inventory"]["engines"]["mesh"]["verified_files"].items()),
                            "native host or artifact changed")
                    affinity = json.loads(content("adapter/process-affinity.json"))
                    require(bool(affinity) and all(mask == pins["cpus"] for mask in affinity.values()),
                            "native runner or PostgreSQL mask differs")
                    arms[label] = verify_arm(lambda name: json.loads(content("adapter/" + name)),
                                             lambda name: content("adapter/" + name),
                                             label, pins, local=False, sample_activity=False)
            except (ValueError, TypeError, OSError, KeyError, IndexError, tarfile.TarError) as exc:
                failures.append(dict(group=number, label=label, error=str(exc)))
        groups.append(dict(number=number, order=order, arms=arms))
    return dict(groups=groups, failures=failures, **summarize(groups))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("platform", choices=("local", "native"))
    parser.add_argument("path", type=Path)
    args = parser.parse_args()
    result = (local if args.platform == "local" else native)(args.path)
    print(json.dumps(result, indent=2, sort_keys=True, allow_nan=False))
    sys.exit(bool(result["failures"]))
