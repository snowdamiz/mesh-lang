"""Validate the dense ledger and actual runner allocations for every engine."""

from collections import Counter
from functools import cache
import importlib.util
import json
from pathlib import Path
import re
import runpy
import sys

PACKET = Path(__file__).resolve().parent
SUITE = PACKET.parents[1]
ROOT = SUITE.parents[1]
TOPOLOGY = PACKET.parent / "2026-11-23-ordinary-runner-topology"
sys.path.insert(0, str(SUITE))
import run
from verify import PROFILES, reconcile, require, xid

INPUT = PROFILES["noop"][3]
JOBS = 1024
SLOTS = 64
CPUS = [0, 1]
CAP = 96
STOCK_MESH = "8306bc3eeeeae1e44959dd08a081898e36c466ff467c0f11fc680cb26c067094"
RUNNER_SHA = "faefaf34344958650a0fbdd19556d86d9f5126c5007b9c77225447004a6781e4"


@cache
def diagnostic_reconciler(runners):
    """Bind an isolated verifier to an explicit diagnostic runner count."""
    require(runners in (2, 8), "unexpected dense topology")
    spec = importlib.util.spec_from_file_location("dense_topology_verify", SUITE / "verify.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    module.PROFILES = {**module.PROFILES, "noop": (runners, *PROFILES["noop"][1:])}
    return module.reconcile


@cache
def mesh_helpers():
    sys.path.insert(0, str(TOPOLOGY))
    try:
        return runpy.run_path(str(TOPOLOGY / "replay.py"))
    finally:
        sys.path.remove(str(TOPOLOGY))


def text(archive, name):
    member = archive.getmember(name)
    require(member.isfile(), f"invalid archive member {name}")
    return archive.extractfile(member).read().decode()


def mesh_topology(files, logs, ledger, runners, phase, *, historical=False):
    slots = SLOTS // runners
    contract = "ordinary-trusted-shared-process"
    build, row, raw, setup, broker = map(files, (
        "build.json", "row.json", "raw.json", "topology.json", "database.json"))
    require(build["files"]["mesh"] == RUNNER_SHA and row["runners"] == runners
            and row["slots"] == SLOTS and row["connection_cap"] == CAP
            and row["execution_contract"] == ledger["execution_contract"] == build["execution_contract"] == contract
            and raw["case_sha256"] == INPUT and ledger["database"] == files("postgres-settings.json")
            and row["phase"] == phase and row["local_smoke"] is ledger["local_smoke"] is False,
            "ordinary V1 runner, profile or execution contract changed")
    pids = setup["runner_pids_by_index"]
    require(len(pids) == len(set(pids)) == runners and setup["runners"] == runners
            and setup["slots_per_runner"] == slots and setup["app_sessions_per_runner"] == 32 // runners
            and setup["scheduler_min"] == setup["scheduler_max"] == 2 and setup["sample_activity"] is False
            and setup["label"] == ("stock-a" if runners == 8 else "tuned-2x32"),
            "Mesh runner topology receipt differs")
    check_affinity(files("process-affinity.json"), set(pids))
    caps = {"parity_subject": 4 * runners, "compiled_caller": 32, "parity_producer": 1}
    capacity = files("sql-capacity.json")
    namespace = raw["namespace"]
    require(capacity["role_caps"] == caps
            and capacity["sql_queues"] == [dict(name="default", state="active", namespace=namespace, concurrency=SLOTS)]
            and row["peak_connections"] == ledger["peak_connections"] == sum(caps.values())
            and row["peak_processes"] == runners,
            "Mesh SQL queue, session, or process limits differ")
    receipts = ledger["transactions"]
    submit = receipts[0]
    require(logs("broker-output.log").splitlines()[0] == "SESSION_READY"
            and broker["user"] == "parity_producer" and broker["tls"] is broker["durable"] is True
            and submit["submitted_us"] < submit["committed_us"] <= submit["acknowledged_us"],
            "unproven warm Mesh broker or submit interval")
    helpers = mesh_helpers()
    used = helpers["verify_sessions"](files, caps, submit["submitted_us"], set())
    jobs = ledger["jobs"]
    app_sql_ms = helpers["verify_app"](files, jobs, namespace)
    counts = Counter(job["worker_pid"] for job in jobs)
    require(set(counts) <= set(pids), "unexpected Mesh worker process")
    hints = 0
    for index, pid in enumerate(pids):
        report = json.loads(logs(f"runner-{index}.log").splitlines()[-1])
        queue = report["queues"]["default"]
        require(report["namespace"] == namespace and report["drained"] and report["last_error"] is None
                and not any(report[key] for key in ("undrained", "recovered", "stale_acks", "pending_claims"))
                and report["claimed"] == report["finished"] == report["outcomes"]["completed"] == counts[pid]
                and all(count == 0 for key, count in report["outcomes"].items() if key != "completed")
                and queue["limit"] == slots and queue["in_flight"] == queue["pending_claims"] == 0
                and helpers["intervals"]([job for job in jobs if job["worker_pid"] == pid]) <= slots,
                "Mesh runner exceeded capacity or failed to drain")
        hints += report["batch_contention_hints"]
    require(raw["raw_application_sql_calls"] == 4 * JOBS, "Mesh app SQL call count differs")
    terminal = {xid(job["settled_xid"], "terminal") for job in jobs}
    independent = {xid(job[key], key) for job in jobs for key in ("attempt_xid", "effect_xid")}
    require(len(terminal | independent | {xid(submit["xid"], "producer")}) == 3 * JOBS + 1,
            "Mesh producer, app, and terminal transactions were not independent")
    report = dict(configured_runner_pool_max=caps["parity_subject"], warm_application_sessions=32,
                  used_application_sessions=len(used), warm_queue_sessions=caps["parity_subject"],
                  scheduler_workers_per_runner=2, jobs_by_runner=[counts[pid] for pid in pids],
                  app_sql_calls=raw["raw_application_sql_calls"], app_sql_ms=app_sql_ms,
                  claim_calls=raw["raw_claim_calls"], admission_hints=hints,
                  submit_boundary="before warm broker call", terminal_transactions=len(terminal))
    if not historical:
        require(ledger["artifact_sha256"] == STOCK_MESH, "unexpected V1 build manifest")
    return report


def check_affinity(masks, worker_pids):
    require(len(masks) == len(worker_pids) + 1 and worker_pids <= set(map(int, masks))
            and all(mask == CPUS for mask in masks.values()), "runner or PostgreSQL affinity differs")


def peer_topology(archive, files, logs, ledger, engine, runners):
    slots = SLOTS // runners
    submit = ledger["transactions"][0]
    require(submit["submitted_us"] < submit["committed_us"], "empty producer window")
    report = {}
    if engine == "go":
        runtime = files("river/config.json")
        raw = files("river/measured/ledger.json")
        require(runtime["profile_config"]["runners"] == runners
                and runtime["profile_config"]["slots"] == SLOTS
                and runtime["profile_config"]["physical_connection_cap"] == CAP
                and runtime["runner_pool_max"] == SLOTS + 2 * runners
                and runtime["submit_boundary"] == "before_batch_read" and runtime["visible_cpus"] == 2
                and files("runtime.json")["submit_boundary"] == "before_batch_read",
                "River topology or producer boundary differs")
        counts = Counter(row["worker_pid"] for row in raw["attempts"])
        capacity = [files(f"river/measured/runner-{index}-capacity.json") for index in range(runners)]
        require(len({row["pid"] for row in capacity}) == runners
                and set(counts) <= {row["pid"] for row in capacity}
                and all(row["slots"] == slots and 0 <= row["peak_concurrent_handlers"] <= slots and row["handler_calls"] == counts[row["pid"]]
                        for row in capacity), "River worker capacities or actual activity differ")
        require(submit["submitted_us"] <= submit["read_finished_us"] <= submit["decoded_us"]
                <= submit["enqueue_called_us"] <= submit["committed_us"] <= submit["returned_us"],
                "River producer read/submit/commit receipt differs")
        report.update(configured_runner_pool_max=runtime["runner_pool_max"],
                      jobs_by_runner=[row["handler_calls"] for row in capacity], scheduler_workers_per_runner=2,
                      submit_boundary="before batch read")
    elif engine == "oban":
        row = files("row.json")
        runtime = files("oban-emulator.json")
        pids = {entry["pid"] for entry in runtime}
        check_affinity(files("process-affinity.json"), pids)
        expected_logs = {f"adapter/runner-{index}.log" for index in range(runners)}
        require(row["runners"] == runners and row["slots"] == SLOTS and row["peak_processes"] >= runners
                and len(pids) == runners and all(entry["schedulers_online"] == 2 for entry in runtime)
                and expected_logs == {name for name in archive.getnames() if re.fullmatch(r"adapter/runner-\d+\.log", name)}
                and submit["committed_us"] <= submit["acknowledged_us"],
                "Oban runtime roster or producer acknowledgement differs")
        counts = Counter(job["worker_pid"] for job in ledger["jobs"])
        require(set(counts) <= pids, "Oban application worker outside measured process roster")
        sessions = files("postgres-tls-sessions.json")
        require(all(row["tls"] for row in sessions), "Oban warm sessions lack TLS")
        report.update(warm_sessions_by_role=dict(Counter(row["role"] for row in sessions)),
                      configured_runner_pool_max=SLOTS + 2 * runners, scheduler_workers_per_runner=2,
                      jobs_by_runner=[counts[entry["pid"]] for entry in runtime],
                      submit_boundary="before warm broker call")
    else:
        readiness = files("ready.json")
        runtime = files("runtime.json")
        names = {f"runner-{index}" for index in range(runners)}
        sessions = readiness["sessions"]
        require(set(readiness["workers"]) == names
                and {row["application_name"].removeprefix("apalis:") for row in sessions if row["listener"]} == names
                and all(row["ssl"] for row in sessions)
                and runtime["cpu_affinity"] == CPUS and runtime["runner_pool_size"] == "slots + 2"
                and runtime["runner_tokio_threads"] == 2
                and submit["committed_us"] <= submit["returned_us"],
                "Apalis worker LISTEN, pool or producer receipt differs")
        samples = files("samples.json")
        require(all(sample["processes"] == runners for sample in samples["idle"]) and samples["idle"],
                "Apalis runner process evidence differs")
        report.update(configured_runner_pool_max=SLOTS + 2 * runners,
                      warm_sessions_by_role=dict(Counter(row["user"] for row in sessions)),
                      scheduler_workers_per_runner=2, submit_boundary="before warm producer session call")
    terminal = {xid(job["settled_xid"], "terminal") for job in ledger["jobs"]}
    app = {xid(job[field], field) for job in ledger["jobs"] for field in ("effect_xid", "attempt_xid")}
    require(not terminal & (app | {xid(submit["xid"], "producer")}),
            "peer terminal transaction overlaps application or producer")
    return dict(**report, terminal_transactions=len(terminal))
