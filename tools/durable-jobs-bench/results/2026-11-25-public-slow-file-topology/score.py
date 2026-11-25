"""Score real SLOW ledgers with per-arm topology and unchanged global slots."""

from collections import Counter
import json
from pathlib import Path
import sys
import tarfile
from types import FunctionType

HERE = Path(__file__).resolve().parent
SUITE = HERE.parents[1]
sys.path[:0] = [str(SUITE), str(HERE.parent / "2026-11-23-bulk-enqueue-v2-short/replay")]
import run
import verify
from interleave import host_cpu

PROFILE = "slow"
RUNNER = "/var/lib/durable-jobs-bench/native/builds/trusted-jobs-product-623c46b92cc5/mesh"
BROKER = "/var/lib/durable-jobs-bench/native/source-public-large-file-session-resume/broker"
ELF = "af2aca867a02d526cb6ba92d1867166ffc346f31e97a8f8aba0f19aa76b1d355"
BROKER_SHA = "a5a2d5667c2d7209234d7f1468c40dd4071818010be31364a4e82d04b7ff18a3"


def scorer(runners):
    assert runners in (1, 2)
    _, *global_fields = verify.PROFILES[PROFILE]
    profile = {**verify.PROFILES, PROFILE: (runners, *global_fields)}
    reconcile = FunctionType(verify.reconcile.__code__, dict(vars(verify), PROFILES=profile))
    return FunctionType(run.score_arm.__code__, dict(vars(run), reconcile=reconcile))


def runner_reports(archive, ledger, pids):
    """Read all logs and bind final per-runner limits to durable worker PIDs."""
    by_pid = Counter(job["worker_pid"] for job in ledger["jobs"])
    runners = ledger["runners"]
    logs = sorted(name for name in archive.getnames()
                  if name.startswith("adapter/runner-") and name.endswith(".log"))
    assert logs == [f"adapter/runner-{i}.log" for i in range(runners)]
    assert len(pids) == len(by_pid) == runners and set(pids) == by_pid.keys()
    reports = []
    for name in logs:
        lines = archive.extractfile(name).read().splitlines()
        assert b"READY" in lines
        reports.append(json.loads(lines[-1]))
    assert sum(r["queues"]["default"]["limit"] for r in reports) == ledger["slots"] == 64
    for pid, report in zip(pids, reports, strict=True):
        queue = report["queues"]["default"]
        assert report["claimed"] == report["finished"] == report["outcomes"]["completed"] == by_pid[pid]
        assert all(amount == 0 for outcome, amount in report["outcomes"].items() if outcome != "completed")
        assert report["state"] == queue["state"] == "stopped" and report["drained"] and queue["drained"]
        assert not any(report[k] for k in ("undrained", "recovered", "stale_acks", "pending_claims", "last_error"))
        assert queue["limit"] == 64 // runners and queue["in_flight"] == queue["pending_claims"] == 0
    return [by_pid[pid] for pid in pids]


def clocks(ledger, result):
    origin, = ledger["transactions"]
    commit = origin["committed_us"]
    jobs = ledger["jobs"]
    enqueue_xid = str(int(origin["xid"]) % (2**32))
    app_xids = {job[edge] for job in jobs for edge in ("attempt_xid", "effect_xid")}
    terminal = {job["settled_xid"] for job in jobs}
    assert len(jobs) == 512 and len(app_xids) == 1024 and app_xids.isdisjoint(terminal)
    assert enqueue_xid not in app_xids | terminal
    assert origin["submitted_us"] < commit <= origin.get("acknowledged_us", commit)
    first = sorted(job["attempt_committed_us"] for job in jobs)
    return dict(enqueue_ms=(commit - origin["submitted_us"]) / 1000,
                submit_ms=(result["last_settlement_us"] - origin["submitted_us"]) / 1000,
                drain_ms=(result["last_settlement_us"] - commit) / 1000,
                last_effect_ms=(max(job["effect_committed_us"] for job in jobs) - commit) / 1000,
                late_refill_ms=(first[-1] - first[63]) / 1000,
                terminal_xids=len(terminal))


def identity(record, verb, executable, sha, parent):
    assert record["argv"] == [executable, verb] and record["executable_sha256"] == sha
    assert record["cpus"] == [0, 1] and record["parent_pid"] == parent
    assert record["launch_start_ticks"] == record["start_ticks_before"] == record["start_ticks_after"] > 0
    assert (record["method"], record["observed_executable"], record["exe_errno"]) in (
        ("proc-exe", executable, None), ("owned-launch", None, 13))


def mesh_receipt(archive, lane, phase, mode=None):
    def get(name):
        return run.file_json(archive, f"adapter/{name}.json")

    ledger, row, raw = [get(k) for k in ("ledger", "row", "raw")]
    runners, slots = ledger["runners"], ledger["slots"]
    assert row["slots"] == slots == 64 and row["runners"] == row["peak_processes"] == runners
    assert (ledger["phase"], row["phase"]) == (phase, phase)
    assert ledger["local_smoke"] is row["local_smoke"] is False
    assert row["connection_cap"] == 68 and row["poll_ms"] == 250
    assert row["completed"] == row["reconciled_jobs"] == 512
    assert row["committed_work_records"] == 1024 and row["durable_enqueue_transactions"] == 1
    assert raw["case_sha256"] == ledger["input_sha256"]
    assert get("build")["files"]["mesh"] == ELF
    assert get("database") == dict(commit_timestamps=True, database_connection_limit=68, durable=True,
                                   max_connections=68, reserve=0, superuser_reserve=0,
                                   tls=True, user="parity_producer")
    storage = get("storage-topology")
    assert storage["max_connections"] == 68 and storage["storage"] == "default"
    assert storage["args_storage"] == "x" and storage["work_mem"] == "4MB"
    assert storage["topology"] == ("stock" if runners == 1 else "two-runner")
    broker, *children = get("process-identity")
    parent = broker["parent_pid"]
    identity(broker, "session", BROKER if mode else RUNNER, BROKER_SHA if mode else ELF, parent)
    for runner in children:
        identity(runner, "run", RUNNER, ELF, parent)
    pids = [runner["pid"] for runner in children]
    finished = runner_reports(archive, ledger, pids)
    masks = get("process-affinity")
    assert set(map(int, masks)) >= set(pids) and len(masks) == runners + 1
    assert all(mask == [0, 1] for mask in masks.values())
    output = archive.extractfile("adapter/broker-output.log").read().splitlines()
    assert output[0] == b"SESSION_READY" and output[-1] == b"{}"
    assert all(value <= 64 for value in (row["estimated_peak_executing"], row["peak_executing"]))
    assert len(raw["wakeups"]) == runners
    assert raw["idle_seconds"] >= 3 and len(raw["idle_samples"]) == 2
    assert any(wakeup["notification_scans"] > 0 for wakeup in raw["wakeups"])
    assert all(not count for dropped in raw["event_drops"] for count in dropped.values())
    tls = get("postgres-tls-sessions")
    assert len(tls) == len({r["backend"] for r in tls}) <= ledger["peak_connections"] <= 68
    assert all(r["tls"] for r in tls)
    assert Counter(r["role"] for r in tls) == dict(
        parity_subject=4 * runners, parity_producer=1, compiled_caller=8)
    assert all(job["effect_committed_us"] - job["attempt_committed_us"] >= 100_000 for job in ledger["jobs"])
    ordered = get("ordered-writes")
    assert ordered["ordered"] == ordered["jobs"] == 512 and ordered["worker_pids"] == runners
    queue = get("queue-policy")
    assert queue["queue_policy"] == ledger["queue_policy"] == lane["queue_policy"] == "set_based"
    assert queue["terminal_xids"] == 512 and queue["largest_terminal_cohort"] == 1
    assert queue["completion_cohort_queries"] == 0 and queue["admitted_queries"] > 0
    statements = queue["queue_statements"]
    def matching(prefix):
        matches = [s for s in statements if s["query"].startswith(prefix)]
        return sum(s["calls"] for s in matches), sum(s["rows"] for s in matches)
    assert matching("SELECT coalesce(mesh_jobs.settle_state(") == (512, 512)
    assert matching("BEGIN ISOLATION LEVEL READ COMMITTED")[0] == matching("COMMIT")[0] == queue["admitted_queries"]
    assert sum(row["calls"] for row in statements if "FROM mesh_jobs.claim_batch_set_v1(" in row["query"]) == queue["admitted_queries"]
    writes = get("trusted-application-statements")
    assert len(writes) == 3
    for suffix in ("attempts", "effects"):
        item, = [s for s in writes if s["query"].startswith("INSERT INTO public.mesh_jobs_parity_" + suffix)]
        assert item["calls"] == item["rows"] == 512
    probes, = [r for r in writes if r["query"] == "SELECT $1"]
    assert probes["calls"] == probes["rows"] == 1024
    producer = get("producer-writer")
    assert (producer["enabled"], producer["bulk"], producer["scalar"]) == (True, 8, 8)
    if mode:
        receipt = get("slow-transfer")
        assert receipt["queue_capacity"] == 64 and receipt["slots_per_runner"] == 64 // runners
        assert receipt["session_mode"] == mode
        assert [(s["calls"], s["rows"]) for s in receipt["producer_statements"]] == [(8, 8), (8, 504)]
        assert all(s["wal_bytes"] > 0 for s in receipt["producer_statements"])
        assert set(receipt["warm_apps_by_runner"]) == set(map(str, pids))
        assert all(len(sessions) == 8 // runners for sessions in receipt["warm_apps_by_runner"].values())
        owned = [s for sessions in receipt["warm_apps_by_runner"].values() for s in sessions]
        assert len({s["backend"] for s in owned}) == len({s["client_port"] for s in owned}) == 8
        assert sorted((s["backend"], s["client_port"]) for s in owned) == sorted(
            (s["backend"], s["client_port"]) for s in tls if s["role"] == "compiled_caller")
    return dict(jobs_by_runner=finished, probes=1024, checked_claim_calls=queue["admitted_queries"],
                app_sql_execution_ms=sum(s["execution_ms"] for s in writes),
                queue_sql_execution_ms=sum(s["execution_ms"] for s in statements),
                minimum_hold_ms=ordered["minimum_hold_ms"])


def score_arm(folder, block, phase, lane, config):
    engine = lane["engine"]
    runners = lane.get("runners", 1)
    score = scorer(runners)(folder, block, phase, engine, lane["inventory"])
    assert score["completed"] == 512 and score["failed"] == 0
    assert [score["host_boot_id"], score["host_kernel"], score["pg_mount"]["maj:min"]] == config["host"]
    path = folder / "raw.tar.gz"
    with tarfile.open(path) as archive:
        ledger = run.file_json(archive, "adapter/ledger.json")
        score.update(clocks(ledger, score))
        if engine == "mesh":
            score.update(mesh_receipt(archive, lane, phase, lane.get("session_mode")))
    score.update(host_cpu(folder, score, config["clock_ticks"]))
    return dict(score, raw_sha256=run.digest(path))
