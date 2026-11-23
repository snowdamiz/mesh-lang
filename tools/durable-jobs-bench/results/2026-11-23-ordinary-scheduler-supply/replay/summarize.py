"""Replay commit receipts, work records, process topology, and paired local scores."""

import argparse
from collections import Counter
import json
from pathlib import Path
from statistics import median
import sys

from arm import WORKERS, sha
from run import VERIFY

sys.path.insert(0, str(VERIFY.parent))
from verify import PROFILES, reconcile, require

CONTRACT = "ordinary-trusted-shared-process"
APP_CONNECTIONS = {"short": 8, "dense": 4}


def read(path):
    return json.loads(path.read_text())


def overlap(jobs, start, end, per_runner=False):
    events = [(job["worker_pid"] if per_runner else 0, job[name], delta)
              for job in jobs for name, delta in ((start, 1), (end, -1))
              if job[start] < job[end]]
    active = Counter()
    peak = 0
    for pid, _, delta in sorted(events):
        active[pid] += delta
        peak = max(peak, active[pid])
    return peak


def verify_records(path, jobs):
    records = read(path / "application-records.json")
    statements = read(path / "trusted-application-statements.json")
    for table, field in (("attempts", "attempt_xid"), ("effects", "effect_xid")):
        entries = {(record["job_key"], record["xid"]) for record in records[table]}
        require(len(records[table]) == len(jobs) and
                entries == {(job["job_key"], job[field]) for job in jobs},
                f"{table} raw identities differ from the settled ledger")
        inserts = [entry for entry in statements
                   if entry["query"].startswith(f"INSERT INTO public.mesh_jobs_parity_{table}")]
        require(len(inserts) == 1 and inserts[0]["calls"] == inserts[0]["rows"] == len(jobs),
                f"{table} statement counts differ from the settled ledger")
    return sum(statement["execution_ms"] for statement in statements) / len(jobs)


def verify_arm(path, profile, label, pins):
    status = read(path.with_suffix(".status.json"))
    if status["exit_code"]:
        failure = read(path / "failure.json")
        raise ValueError(f"exit {status['exit_code']}: {failure}")
    policy = WORKERS[label]
    scheduler = read(path / "scheduler.json")
    row = read(path / "row.json")
    ledger = read(path / "ledger.json")
    jobs = ledger["jobs"]
    canonical = "noop" if profile == "dense" else profile
    runners, slots, cap, expected_input, _ = PROFILES[canonical]
    artifact = dict(version="3828f20d2d6beb683868cce33e6dd1958365914b",
                    artifact_sha256=pins["manifest_sha256"])
    build = read(path / "build.json")
    require(build["files"]["mesh"] == pins["executable_sha256"]
            and all(row[key] == value for key, value in artifact.items()),
            "different ordinary executable")
    require(row["execution_contract"] == ledger["execution_contract"] ==
            build["execution_contract"] == CONTRACT
            and row["local_smoke"] and ledger["local_smoke"]
            and ledger["phase"] == row["phase"] == "diagnostic"
            and row["input_sha256"] == expected_input, "unexpected execution contract, phase, or input")
    score = reconcile(dict(ledger, profile=canonical), "mesh", canonical, artifact)
    require(row["profile"] == ledger["profile"] == profile,
            "adapter profile label differs from the request")
    require(scheduler["label"] == label and
            scheduler["runner_min_workers"] == scheduler["runner_max_workers"] == policy,
            "different scheduler count")
    require(scheduler["slots_per_runner"] == [slots // runners] * runners
            and scheduler["app_connections_per_runner"] == APP_CONNECTIONS[profile]
            and scheduler["postgres_cap"] == cap
            and row["peak_processes"] == runners, "different runner, slot, or database topology")
    threads = scheduler["runner_os_threads_at_ready"]
    pids = set(scheduler["runner_pids"])
    require(len(pids) == runners and {int(pid) for pid in threads} == pids
            and {job["worker_pid"] for job in jobs} <= pids,
            "jobs ran outside the warmed runner processes")
    require(set(threads.values()) == {policy + 6},
            "ready runner OS threads did not match the configured scheduler plus six other threads")
    require(row["peak_threads"] >= sum(threads.values()), "peak thread sample below ready inventory")

    reports = [next(json.loads(line) for line in log.read_text().splitlines()[::-1]
                    if line.startswith("{")) for log in sorted(path.glob("runner-*.log"))]
    require(len(reports) == runners and all(
        report["drained"] and report["last_error"] is None and report["recovered"] == 0
        and report["stale_acks"] == 0 and report["undrained"] == 0
        and report["claimed"] == report["finished"] == report["outcomes"]["completed"]
        and report["queues"]["default"]["limit"] == slots // runners
        and not any(value for outcome, value in report["outcomes"].items() if outcome != "completed")
        for report in reports) and sum(report["claimed"] for report in reports) == len(jobs),
        "runner reports show an error, retry, recovery, or changed queue limit")
    require(len({job["settled_xid"] for job in jobs}) == len(jobs),
            "ordinary V1 no longer settles each job in a separate transaction")
    pg_exec = verify_records(path, jobs)
    tls = read(path / "postgres-tls-sessions.json")
    tls_counts = Counter(session["role"] for session in tls if session["tls"])
    require(sum(tls_counts.values()) == len(tls)
            and tls_counts["compiled_caller"] == runners * APP_CONNECTIONS[profile]
            and runners <= tls_counts["parity_subject"] <= runners * 4
            and tls_counts["parity_producer"] == 1,
            f"TLS connection topology differs: {tls_counts}")
    warmed_connections = runners * (APP_CONNECTIONS[profile] + 4) + 1
    raw = read(path / "raw.json")
    require([sample["connections"] for sample in raw["idle_samples"]] == [warmed_connections] * 2
            and row["peak_connections"] == warmed_connections,
            "measured connection allocation differs from the frozen topology")

    handler_peak = overlap(jobs, "entered_us", "effect_committed_us")
    in_runner_peak = overlap(jobs, "entered_us", "effect_committed_us", per_runner=True)
    write_span_peak = overlap(jobs, "attempt_committed_us", "effect_committed_us")
    require(in_runner_peak > 1 and write_span_peak > 1, "missing overlapping durable handlers")
    return dict(score, runner_os_threads_ready=sum(threads.values()), runner_os_threads_peak=row["peak_threads"],
                runner_count_with_work=len({job["worker_pid"] for job in jobs}),
                handler_peak_enter_to_effect=handler_peak,
                max_single_runner_handler_peak_enter_to_effect=in_runner_peak,
                peak_interleaved_committed_write_spans=write_span_peak,
                pg_application_statement_exec_ms_per_job=pg_exec,
                application_sql_calls_per_job=row["application_sql_calls_per_job"],
                queue_sql_calls_per_job=row["runner_sql_calls_per_job"],
                claim_calls_per_job=row["claim_calls_per_job"],
                startup_ms=1000 * scheduler["runner_launch_to_all_ready_s"],
                broker_startup_and_db_audit_ms=1000 * scheduler["broker_launch_to_ready_and_db_audit_s"],
                arm_wall_s=status["wall_s"], host_load_1m=status["load_before"][0],
                peak_host_load_1m=row["peak_host_load_average_1m"],
                peak_ledger_claim_to_terminal=row["estimated_peak_executing"],
                receipt_hashes={name: sha(path / name) for name in (
                    "ledger.json", "raw.json", "scheduler.json", "application-records.json",
                    "row.json", "build.json", "postgres-tls-sessions.json", "postgres-settings.json",
                    "trusted-application-statements.json")},
                ready_connections_by_role=dict(tls_counts))


def distribution(values):
    return dict(n=len(values), median=median(values), min=min(values), max=max(values))


def summarize(out):
    manifest = read(out / "series.json")
    require(sha(VERIFY) == manifest["pins"]["verifier_sha256"], "ledger verifier drifted")
    scores, pending, failures = {"short": [], "dense": []}, [], []
    for block in manifest["plan"]:
        directory = out / f"{block['group']:02}-{block['profile']}"
        results = {}
        for label in block["order"]:
            path = directory / label
            if not path.with_suffix(".status.json").exists():
                pending.append(str(path.relative_to(out)))
                continue
            try:
                results[label] = verify_arm(path, block["profile"], label, manifest["pins"])
            except (OSError, ValueError, KeyError, TypeError, StopIteration) as error:
                failures.append(dict(arm=str(path.relative_to(out)), error=str(error)))
        if len(results) == len(WORKERS):
            scores[block["profile"]].append(dict(block, arms=results))

    metrics = ("submit_jobs_per_s", "commit_jobs_per_s", "runner_cpu_ms_per_job", "peak_runner_rss_bytes",
               "runner_os_threads_peak", "peak_connections", "peak_executing",
               "peak_ledger_claim_to_terminal", "handler_peak_enter_to_effect",
               "max_single_runner_handler_peak_enter_to_effect", "peak_interleaved_committed_write_spans",
               "runner_count_with_work", "startup_ms", "broker_startup_and_db_audit_ms", "arm_wall_s",
               "host_load_1m", "peak_host_load_1m", "first_submit_to_commit_ms",
               "commit_to_pickup_p99_ms", "commit_to_settlement_p99_ms", "claim_to_entry_p99_ms",
               "entry_to_attempt_commit_p50_ms", "attempt_to_effect_commit_p50_ms",
               "effect_commit_to_settlement_p50_ms", "effect_commit_to_settlement_p99_ms",
               "pg_application_statement_exec_ms_per_job", "application_sql_calls_per_job",
               "queue_sql_calls_per_job", "claim_calls_per_job")
    rollup, paired = {}, {}
    for profile, blocks in scores.items():
        if not blocks:
            continue
        rollup[profile] = {label: {key: distribution([b["arms"][label][key] for b in blocks])
                                  for key in metrics} for label in WORKERS}
        paired[profile] = {}
        for label in ("stock-b", "workers-4", "workers-8"):
            paired[profile][label] = {}
            for key in ("submit_jobs_per_s", "commit_jobs_per_s", "runner_cpu_ms_per_job"):
                values = [100 * (b["arms"][label][key] / stock - 1) for b in blocks
                          for stock in (b["arms"]["stock-a"][key] if label == "stock-b" else median(
                              b["arms"][control][key] for control in ("stock-a", "stock-b")),)]
                paired[profile][label][key] = dict(distribution(values), by_group=values)
    return dict(local_affinity=manifest["host_affinity"], scores=scores, rollup=rollup, paired_percent=paired,
                pending=pending, failures=failures)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("out", type=Path)
    args = parser.parse_args()
    result = summarize(args.out)
    print(json.dumps(result, indent=2, sort_keys=True, allow_nan=False))
    sys.exit(bool(result["pending"] or result["failures"]))
