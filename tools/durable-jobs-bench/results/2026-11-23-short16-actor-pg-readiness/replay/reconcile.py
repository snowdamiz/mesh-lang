"""Verify every local ledger and compute paired timing and overlap from raw receipts."""
import argparse
from collections import Counter
import hashlib
import importlib.util
import json
import re
from pathlib import Path
from statistics import median
import sys

from arm import CONTRACT, LABELS, READINESS, ROLES
from campaign import ORDERS

GUARD = Path(__file__).with_name("guard.json")


def read(path):
    return json.loads(path.read_text())


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def load_verifier(path, source):
    spec = importlib.util.spec_from_file_location("verify", path)
    verify = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(verify)
    verify.ROOT = source
    return verify


def app_sessions(path):
    sessions = read(path)
    assert Counter(row["role"] for row in sessions if row["tls"]) == ROLES
    assert len(sessions) == len({row["backend"] for row in sessions}) == 13
    return {row["backend"]: row for row in sessions if row["role"] == "compiled_caller"}


def arm(path, label, pins, manifest, diagnostic, verify):
    def get(name):
        return read(path / f"{name}.json")

    assert read(path.with_suffix(".status.json"))["exit_code"] == 0
    ledger, row, build = get("ledger"), get("row"), get("build")
    assert build["files"]["mesh"] == pins["mesh"]
    assert build == manifest
    experiment = get("experiment")
    assert (experiment["label"], experiment["sampler"], experiment["readiness"]) == (
        label, diagnostic, label == "readiness")
    assert experiment["scheduler_min"] == experiment["scheduler_max"] == 2
    assert experiment["execution_contract"] == (READINESS if label == "readiness" else CONTRACT)
    assert ledger["local_smoke"] and ledger["phase"] == "diagnostic"
    assert ledger["execution_contract"] == row["execution_contract"] == build["execution_contract"] == CONTRACT
    score = verify.reconcile(ledger, "mesh", "short",
                             dict(version=build["version"], artifact_sha256=pins["mesh.json"]))
    assert score["completed"] == 128 and score["peak_connections"] == 13
    tx, = ledger["transactions"]
    jobs = ledger["jobs"]
    assert tx["committed_us"] <= tx["acknowledged_us"]
    assert all(job["worker_pid"] == experiment["runner_pid"] for job in jobs)
    assert all(tx["committed_us"] <= job["entered_us"] <= job["attempt_committed_us"] for job in jobs)
    assert min(job["effect_committed_us"] - job["attempt_committed_us"] for job in jobs) >= 1000
    raw = get("raw")
    persisted = get("application-records")
    for table, field in (("attempts", "attempt_xid"), ("effects", "effect_xid")):
        entries = persisted[table]
        assert len(entries) == 128
        assert {item["job_key"]: item["xid"] for item in entries} == {
            job["job_key"]: job[field] for job in jobs}
        assert all(item["namespace"] == raw["namespace"] and item["engine"] == "mesh"
                   and item["job_key"] == item["effect_key"] for item in entries)
    assert get("ordered-writes")["ordered"] == 128

    policy = get("queue-policy")
    terminal = Counter(job["settled_xid"] for job in jobs)
    app_xids = {job[field] for job in jobs for field in ("attempt_xid", "effect_xid")}
    assert not terminal.keys() & (app_xids | {str(int(tx["xid"]) % (2**32))})
    assert len(app_xids) == 256 and len(terminal) == policy["terminal_xids"]
    assert max(terminal.values()) == policy["largest_terminal_cohort"] <= 8
    assert row["queue_policy"] == ledger["queue_policy"] == policy["queue_policy"] == "success_cohort"
    assert policy["admitted_queries"] > 0
    assert max(terminal.values()) == 1 or policy["completion_cohort_queries"] > 0

    writer = get("producer-writer")
    assert writer["enabled"] is True and (writer["bulk"], writer["scalar"]) == (1, 0)
    app = get("trusted-application-statements")
    app_calls = sum(statement["calls"] for statement in app)
    for table in ("attempts", "effects"):
        insert, = [q for q in app if q["query"].startswith("INSERT INTO public.mesh_jobs_parity_" + table)]
        assert insert["calls"] == insert["rows"] == 128
    # PostgreSQL normalizes checkout SELECT 1 to SELECT $1 for this app role.
    probes = sum(q["calls"] for q in app if q["query"] == "SELECT $1")
    resets = sum(q["calls"] for q in app if q["query"].upper().startswith(
        ("ROLLBACK", "DISCARD", "RESET")))
    assert probes == 256 and resets == 0 and app_calls == 512
    assert raw["raw_application_sql_calls"] == app_calls and raw["statement_counter_deallocations"] == 0
    before, after = get("counters-2"), get("counters-3")
    for role, raw_key in (("enqueue", "raw_enqueue_sql_calls"), ("runner", "raw_runner_sql_calls"),
                          ("claims", "raw_claim_calls"), ("application", "raw_application_sql_calls")):
        assert after[role] - before[role] == raw[raw_key]
    warm = app_sessions(path / "sessions-warm.json")
    start = app_sessions(path / "sessions-before.json")
    end = app_sessions(path / "sessions-end.json")
    assert set(warm) == set(start) == set(end)
    assert all(item["state"] == "idle" for item in end.values())
    used = [pid for pid, session in end.items()
            if session["query"].startswith("INSERT INTO public.mesh_jobs_parity_")
            and score["first_commit_us"] <= session["query_start_us"] <= score["last_settlement_us"]]
    assert used
    status = json.loads((path / "runner-0.log").read_text().splitlines()[-1])
    assert status["state"] == "stopped" and status["drained"] is True
    assert status["claimed"] == status["finished"] == status["outcomes"]["completed"] == 128
    assert status["undrained"] == status["pending_claims"] == status["stale_acks"] == 0
    assert status["last_error"] is None
    assert sum(status["outcomes"].values()) == 128
    assert all(sample["processes"] == 1 for sample in raw["samples"] + raw["idle_samples"])

    submitted, committed, settled = (score[key] for key in
                                      ("first_submit_us", "first_commit_us", "last_settlement_us"))
    entries = sorted(job["entered_us"] for job in jobs)
    boundaries = sorted((job[key], delta) for job in jobs
                        for key, delta in (("entered_us", 1), ("settled_us", -1)))
    active = peak = 0
    for _, delta in boundaries:
        active += delta
        peak = max(peak, active)
    assert active == 0 and 0 < peak <= ledger["slots"]
    final_effect = max(job["effect_committed_us"] for job in jobs)
    metrics = dict(
        submit_ms=(settled - submitted) / 1000, drain_ms=(settled - committed) / 1000,
        rank_1_ms=(entries[0] - committed) / 1000, rank_64_ms=(entries[63] - committed) / 1000,
        rank_128_ms=(entries[-1] - committed) / 1000, second_half_ms=(entries[-1] - entries[63]) / 1000,
        final_effect_to_terminal_ms=(settled - final_effect) / 1000,
        peak_occupied_slots_from_entry=peak, runner_cpu_ms=ledger["runner_cpu_seconds"] * 1000, sampled_runner_threads=row["peak_threads"],
        application_probe_calls=probes, application_reset_calls=resets, application_sql_calls=app_calls,
        application_pg_execution_ms=sum(q["execution_ms"] for q in app),
        queue_sql_calls=raw["raw_runner_sql_calls"], producer_sql_calls=raw["raw_enqueue_sql_calls"],
        queue_claim_calls=policy["admitted_queries"], completion_cohort_queries=policy["completion_cohort_queries"],
        terminal_xids=len(terminal), largest_cohort=max(terminal.values()),
        distinct_durable_xids=1 + len(app_xids) + len(terminal),
        warmed_application_backends=len(warm), used_application_backends=len(used),
        used_application_backend_pids=used)
    if diagnostic:
        points = []
        for line in (path / "activity.log").read_text().splitlines():
            if line.lstrip().startswith("{"):
                item = json.loads(line)
                if committed <= item["epoch_us"] <= settled:
                    points.append(item)
        assert points and all(sample["app"] == 8 for sample in points)
        assert all(set(sample["active_writes"]) <= set(warm) for sample in points)
        assert not (path / "activity-errors.log").read_text().strip()
        log = (path / "runner-0.log").read_text()
        stacks = (path / "warm-runner-stacks.txt").read_text().split("Total number in stack")[0]
        threads = re.split(r"(?m)^    [0-9]+ Thread_[0-9]+", stacks)[1:]
        workers = sum("scheduler11worker_loop" in thread for thread in threads)
        assert workers == 2
        metrics.update(active_write_samples=len(points), live_warm_actor_workers=workers,
                       peak_simultaneous_backend_writes=max(len(sample["active_writes"]) for sample in points),
                       samples_with_simultaneous_writes=sum(len(sample["active_writes"]) >= 2 for sample in points),
                       peer_socket_read_waits=log.count("postgres socket wait read"),
                       peer_socket_write_waits=log.count("postgres socket wait write"))
    return {**score, **metrics}


def distribution(values):
    return dict(median=median(values), min=min(values), max=max(values))


def report(evidence, verify):
    plan = read(evidence / "scored/plan.json")
    assert plan["orders"] == [list(order) for order in ORDERS] and not plan["diagnostic"]
    manifest_path = evidence / "mesh-build.json"
    assert digest(manifest_path) == plan["pins"]["mesh.json"]
    manifest = read(manifest_path)
    rows = []
    for index, order in enumerate(plan["orders"], 1):
        scores = {label: arm(evidence / "scored" / f"{index:02}" / label, label,
                             plan["pins"], manifest, False, verify) for label in order}
        rows.append(dict(group=index, order=order, scores=scores))
    summary_fields = [key for key, value in rows[0]["scores"]["off-a"].items()
                      if type(value) in (int, float) and not key.endswith("_us")]
    summary = {label: {field: distribution([row["scores"][label][field] for row in rows])
                       for field in summary_fields} for label in LABELS}
    paired = {}
    for field in ("drain_ms", "submit_ms", "second_half_ms", "runner_cpu_ms"):
        details = []
        for row in rows:
            a, b, candidate = [row["scores"][label][field] for label in LABELS]
            midpoint = (a + b) / 2
            details.append(dict(group=row["group"], candidate=candidate, off_a=a, off_b=b,
                                gain=midpoint - candidate, gain_pct=100 * (1 - candidate / midpoint),
                                duplicate_spread=abs(a-b), beats_both=candidate < min(a, b)))
        paired[field] = dict(groups=details, wins_both=sum(item["beats_both"] for item in details),
                             median_gain_percent=median(item["gain_pct"] for item in details),
                             median_gain_ms=median(item["gain"] for item in details),
                             median_duplicate_spread_ms=median(item["duplicate_spread"] for item in details))
    guard = read(GUARD)
    drain = paired["drain_ms"]
    passed = (len(rows) == guard["groups"]
              and drain["wins_both"] >= guard["minimum_groups_faster_than_both_off_controls"]
              and drain["median_gain_percent"] >= guard["minimum_median_paired_drain_improvement_percent"]
              and drain["median_gain_ms"] > drain["median_duplicate_spread_ms"]
              and paired["submit_ms"]["median_gain_percent"] >= guard["minimum_median_paired_submit_improvement_percent"]
              and paired["second_half_ms"]["median_gain_percent"] >= guard["minimum_median_paired_second_half_improvement_percent"])
    diagnostics = {}
    if (evidence / "diagnostic/plan.json").is_file():
        diagnostic = read(evidence / "diagnostic/plan.json")
        assert diagnostic["pins"] == plan["pins"] and diagnostic["diagnostic"]
        for label in diagnostic["orders"][0]:
            diagnostics[label] = arm(evidence / "diagnostic/01" / label, label,
                                     plan["pins"], manifest, True, verify)
    return dict(pins=plan["pins"], rows=rows, summary=summary, paired=paired,
                diagnostic=diagnostics, timing_guard=guard, timing_guard_passed=passed)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("evidence", type=Path)
    parser.add_argument("verifier", type=Path)
    parser.add_argument("frozen_v2_source", type=Path)
    opts = parser.parse_args()
    output = report(opts.evidence, load_verifier(opts.verifier, opts.frozen_v2_source))
    json.dump(output, sys.stdout, indent=2, sort_keys=True, allow_nan=False)
    print()
