"""Reconcile dense arms and compare elapsed times within each rotation."""
from collections import Counter
import json

from transfer import (CPUS, JOBS, ROLES, child_identity, diagnostic_reconciler,
                      require, xid)
from qualification import mesh_helpers

SETTLE = "SELECT coalesce(mesh_jobs.settle_state("
CLAIM = "FROM mesh_jobs.claim_batch_set_v1("
CURSOR = "FROM mesh_jobs.claim_batch_v1("
APP_PREFIX = "INSERT INTO public.mesh_jobs_parity_"
CONTRACT = "ordinary-trusted-shared-process-v2"


def lifecycle(ledger, individual):
    jobs = ledger["jobs"]
    origin, = ledger["transactions"]
    effects = {xid(job[k], k) for job in jobs for k in ("attempt_xid", "effect_xid")}
    terminals = Counter(xid(job["settled_xid"], "terminal") for job in jobs)
    enqueue = xid(origin["xid"], "enqueue") % (2**32)
    require(len(jobs) == JOBS and len(effects) == 2 * JOBS
            and (not individual or len(terminals) == JOBS)
            and not (effects & terminals.keys()) and enqueue not in effects | terminals.keys(),
            "incorrect producer, independent application or individual terminal transactions")
    return len(terminals)


def intervals(ledger):
    origin, = ledger["transactions"]
    settled = sorted(job["settled_us"] for job in ledger["jobs"])
    writes = sorted(job["effect_committed_us"] for job in ledger["jobs"])
    # Same 103rd through 922nd completion ranks as the historical dense report.
    return dict(submit_us=settled[-1] - origin["submitted_us"],
                drain_us=settled[-1] - origin["committed_us"],
                middle_effect_us=writes[921] - writes[102],
                middle_settlement_us=settled[921] - settled[102])


def mesh(get, log, phase, hot, binding, executable):
    build, ledger, row, settings, queue = map(get, ("build", "ledger", "row", "postgres-settings", "queue-policy"))
    manifest = binding["manifest"]
    require(build == manifest and ledger["phase"] == row["phase"] == phase
            and ledger["local_smoke"] is row["local_smoke"] is False
            and ledger["execution_contract"] == row["execution_contract"] == CONTRACT
            and ledger["database"] == settings and row["runners"] == row["peak_processes"] == 2
            and row["slots"] == 64 and row["poll_ms"] == 250 and row["connection_cap"] == 96
            and row["completed"] == row["reconciled_jobs"] == JOBS
            and row["committed_work_records"] == 2 * JOBS and row["durable_enqueue_transactions"] == 1,
            "unexpected Mesh artifact, topology or workload")
    score = diagnostic_reconciler(2)(ledger, "mesh", "noop",
                                    dict(version=manifest["version"], artifact_sha256=binding["manifest_sha256"]))
    require(get("dense-setup") == dict(storage=dict(args_storage="x", work_mem="4MB", max_connections=96),
                                       limits=ROLES), "unexpected storage or connection limits")
    require(get("privileges") == {
        "before": dict(worker_set=False, producer_set=False, application_set=False,
                       worker_cursor=True, worker_may_insert_job=False),
        "after": dict(worker_set=True, producer_set=False, application_set=False,
                      worker_cursor=True, worker_may_insert_job=False),
    }, "protected admission grant differs")
    require(get("database") == dict(commit_timestamps=True, database_connection_limit=96, durable=True,
                                   max_connections=96, reserve=0, superuser_reserve=0, tls=True,
                                   user="parity_producer"), "broker or database contract changed")
    origin, = ledger["transactions"]
    require(origin["submitted_us"] < origin["committed_us"] <= origin["acknowledged_us"], "broker clocks differ")

    broker, *runners = get("process-identity")
    for record, verb in [(broker, "session"), *((runner, "run") for runner in runners)]:
        child_identity(record, executable, manifest["files"]["mesh"], verb, broker["parent_pid"])
    pids = {runner["pid"] for runner in runners}
    affinity = get("process-affinity")
    require(len(runners) == len(pids) == 2 and broker["pid"] not in pids
            and len(affinity) == 3 and pids <= set(map(int, affinity))
            and all(mask == CPUS for mask in affinity.values()), "runner or PostgreSQL affinity changed")

    tls = get("postgres-tls-sessions")
    counts = Counter(item["role"] for item in tls)
    require(all(item["tls"] for item in tls) and len({r["backend"] for r in tls}) == len(tls)
            and set(counts) == set(ROLES) and counts["compiled_caller"] == 32 and counts["parity_producer"] == 1
            and 4 <= counts["parity_subject"] <= 8
            and len(tls) <= row["peak_connections"] == score["peak_connections"] <= 96,
            "TLS readiness or measured peak changed")
    jobs = ledger["jobs"]
    by_pid = Counter(job["worker_pid"] for job in jobs)
    require(set(by_pid) == pids, "handlers were not confined to two live runners")
    peak = mesh_helpers()["intervals"]
    for index, runner in enumerate(runners):
        report = json.loads(log(f"runner-{index}.log").splitlines()[-1])
        slots = report["queues"]["default"]
        require(report["drained"] and report["state"] == slots["state"] == "stopped"
                and report["last_error"] is None
                and not any(report[k] for k in ("undrained", "recovered", "stale_acks", "pending_claims"))
                and report["claimed"] == report["finished"] == report["outcomes"]["completed"] == by_pid[runner["pid"]]
                and all(n == 0 for key, n in report["outcomes"].items() if key != "completed")
                and slots["limit"] == 32 and slots["drained"] and slots["in_flight"] == slots["pending_claims"] == 0
                and peak([job for job in jobs if job["worker_pid"] == runner["pid"]]) <= 32,
                "runner exceeded its cap or did not cleanly drain")

    terminals = lifecycle(ledger, individual=True)
    sql = queue["queue_statements"]
    admitted = sum(stmt["calls"] for stmt in sql if CLAIM in stmt["query"])
    require(queue["queue_policy"] == ledger["queue_policy"] == row["queue_policy"] == "set_based"
            and 0 < admitted == queue["admitted_queries"] and queue["terminal_xids"] == terminals
            and queue["largest_terminal_cohort"] == 1 and queue["completion_cohort_queries"] == 0
            and not any(CURSOR in item["query"] or "complete_batch_v1(" in item["query"] for item in sql)
            and sum(item["calls"] for item in sql if item["query"].startswith(SETTLE)) == JOBS,
            "checked set admission or individual settlement changed")
    writer = get("producer-writer")
    require(writer["enabled"] and (writer["bulk"], writer["scalar"]) == (8, 0), "dense producer writer changed")
    statements = get("trusted-application-statements")
    for suffix in ("attempts", "effects"):
        writes, = (stmt for stmt in statements if stmt["query"].startswith(APP_PREFIX + suffix))
        require(writes["calls"] == writes["rows"] == JOBS, "application statement counts differ")
    health = [stmt for stmt in statements if stmt["query"] == "SELECT $1"]
    probes = sum(stmt["calls"] for stmt in health)
    require(len(health) <= 1 and len(statements) == 2 + len(health) and probes == sum(stmt["rows"] for stmt in health)
            and get("raw")["raw_application_sql_calls"] == 2 * JOBS + probes
            and (0 <= probes < 2 * JOBS if hot else probes == 2 * JOBS), "checkout probe count differs")
    return dict(score, **intervals(ledger), checkout_probes=probes, terminal_xids=terminals,
                application_xids=2 * JOBS, admitted_claim_queries=admitted,
                probe_sql_exec_ms=sum(stmt["execution_ms"] for stmt in health),
                physical_tls_by_role=dict(counts), jobs_by_runner=[by_pid[r["pid"]] for r in runners],
                last_effect_us=max(j["effect_committed_us"] for j in jobs))
