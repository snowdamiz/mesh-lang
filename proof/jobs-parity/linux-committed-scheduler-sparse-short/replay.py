"""Replay the frozen sparse and short16 scorer from the compact proof."""

import gzip
from hashlib import file_digest, sha256
import importlib.util
import json
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
TOPOLOGY = {"sparse": (1, 1, 5, 1), "short": (1, 16, 20, 8)}
ARMS = ("ordinary", "layout", "oban")


def read(path):
    return json.loads(path.read_text())


def digest(path):
    with path.open("rb") as stream:
        return file_digest(stream, "sha256").hexdigest()


def scorer(root):
    spec = importlib.util.spec_from_file_location("canonical_jobs_parity", root / "scripts/jobs-parity.py")
    harness = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(harness)
    return harness, read(harness.CONFIG)


def verify_arm(h, settings, profile, label, receipt, ledger):
    _, slots, cap, lanes = TOPOLOGY[profile]
    engine = "oban" if label == "oban" else "mesh"
    workload = settings["workloads"][profile]
    expected = {job["key"]: (batch["request_id"], job)
                for batch in h.inputs(workload, slots, settings["seed"]) for job in batch["jobs"]}
    metrics, xids, attempts, samples = (ledger[key] for key in
                                       ("metrics", "xids", "attempts", "samples"))
    assert metrics == receipt["metrics"] and metrics["engine"] == engine
    assert metrics["input_sha256"] == receipt["input_sha256"]
    count = len(expected)
    assert count == metrics["completed"] == len(xids["jobs"]) == len(attempts)
    assert receipt["database"] == {
        "user": "parity_producer", "tls": True, "durable": True, "commit_timestamps": True,
        "max_connections": cap, "database_connection_limit": cap, "reserve": 0, "superuser_reserve": 0,
    }
    enqueues = {row["request"]: row for row in xids["enqueues"]}
    assert len(enqueues) == len(xids["enqueues"]) == len({request for request, _ in expected.values()})
    enqueue_xids = {row["xid"] for row in xids["enqueues"]}
    assert len(enqueue_xids) == len(enqueues)
    jobs = {row["key"]: row for row in xids["jobs"]}
    observed = {row["job_key"]: row for row in attempts}
    assert jobs.keys() == observed.keys() == expected.keys()
    application_xids, settlements = set(), set()
    for key, row in jobs.items():
        attempt = observed[key]
        request, job = expected[key]
        payload = job["payload"].encode()
        assert row["id"] == attempt["id"]
        assert attempt["payload_sha256"] == sha256(payload).hexdigest()
        assert row["payload_bytes"] == len(payload)
        assert attempt["state"] == "completed" and attempt["fail_first"] == job["fail_first"] is False
        assert attempt["hold_ms"] == job["hold_ms"]
        assert row["attempt"] == attempt["attempt"] == attempt["first_handler_attempt"] == 1
        assert attempt["attempts"] == attempt["effects"] == attempt["effect_keys"] == 1
        assert enqueues[request]["commit_ms"] == attempt["commit_ms"] <= row["attempt_commit_ms"]
        assert row["attempt_commit_ms"] < row["effect_commit_ms"] <= row["settlement_commit_ms"]
        assert (row["attempt_commit_ms"], row["effect_commit_ms"], row["settlement_commit_ms"]) == (
            attempt["recorded_ms"], attempt["effected_ms"], attempt["settled_ms"])
        tolerance = settings["clock_tolerance_ms"]
        assert attempt["pickup_ms"] == h.within_clock(attempt["entered_ms"] - attempt["commit_ms"], tolerance)
        assert attempt["commit_to_claim_ms"] == h.within_clock(attempt["claimed_ms"] - attempt["commit_ms"], tolerance)
        application_xids.update((row["attempt_xid"], row["effect_xid"]))
        settlements.add(row["settlement_xid"])
        if engine == "mesh":
            assert row["attempt_xid"] == row["first_named_xid"]
            assert row["effect_xid"] == row["second_named_xid"]
            assert row["job_run"] == row["second_job_run"] == 1
    assert len(application_xids) == 2 * count and len(settlements) == count
    assert len(enqueue_xids | application_xids | settlements) == len(enqueues) + 3 * count
    assert len(application_xids) == receipt["independent_application_commits"]
    assert len(enqueues) == receipt["enqueue_commits"] and len(settlements) == receipt["settlement_commits"]
    if engine == "mesh":
        assert len({row["pid"] for row in jobs.values()}) == count
        assert not {row["pid"] for row in jobs.values()} & {runner["pid"] for runner in receipt["runners"]}
        assert len(receipt["runners"]) == 1 and receipt["runners"][0]["tracer_pid"] == 0
        assert any(wake["notification_scans"] > 0 for wake in ledger["wakeups"])
        assert receipt["startup_resets"] == lanes and receipt["application_resets"] == 2 * count
        resets, = (row for row in ledger["statements"] if row["query"] == "DISCARD ALL")
        assert resets["calls"] == lanes + 2 * count
        for name in ("first", "second"):
            statement, = (row for row in ledger["statements"] if f'SELECT "compiled_demo"."{name}"(' in row["query"])
            assert statement["calls"] == statement["rows"] == count
        assert ledger["sql_counts"]["application"] == 4 * count
    pickup = [attempt["pickup_ms"] for attempt in attempts]
    assert h.percentile(pickup, 0.99) == metrics["pickup_p99_ms"]
    assert max(pickup) == metrics["pickup_max_ms"]
    assert h.percentile([a["commit_to_claim_ms"] for a in attempts], 0.99) == metrics["claim_p99_ms"]
    duration = (max(row["settlement_commit_ms"] for row in jobs.values()) -
                min(row["commit_ms"] for row in enqueues.values())) / 1000
    assert count / duration == metrics["jobs_per_second"]
    assert metrics["cpu_seconds_per_job"] == ledger["cpu_seconds"] / count
    for field in ("connections", "processes", "threads", "rss_bytes", "executing"):
        assert metrics[f"peak_{field}"] == max(sample[field] for sample in samples)
    assert metrics["peak_executing"] <= slots and metrics["peak_connections"] <= cap


def score(h, settings, profile, groups, label):
    runners, slots, cap, _ = TOPOLOGY[profile]
    pairs = [dict(group["measured"][arm]["metrics"], pair=i)
             for i, group in enumerate(groups) for arm in (label, "oban")]
    h.validate_pairs(pairs, len(groups), slots)
    summary = h.profile_summary(pairs, profile, settings["workloads"][profile], settings,
                                runners, slots, ("mesh", "oban"), True, True, None)
    assert summary["budgets"]["peak_connections"] == cap
    breaches = [f"{scope}.{key}" for scope, metrics in summary.items() for key, result in metrics.items()
                if isinstance(result, dict)
                and (result.get("within_limit") is False or result.get("within_budget") is False)]
    return {"group_count": len(groups), "breaches": breaches, "summary": summary}


def verify_oban(group, guest):
    assert group["oban_versions"] == {"oban": "2.24.2", "erlang": "29.1.1", "elixir": "1.20.4"}
    arm = group["measured"]["oban"]
    runner, = arm["runners"]
    assert runner["pid"] != arm["broker"]["pid"]
    for role, report in (("preflight", group["oban_preflight"]), ("run", runner), ("session", arm["broker"])):
        assert report["role"] == role and report["lookup"] == ["file"]
        assert report["loopback_addresses"] == ["127.0.0.1"]
        assert report["erl_flags"] == guest["oban_erl_flags"]
        assert report["erl_inetrc"] == guest["oban_erl_inetrc"]
        assert report["inetrc_sha256"] == guest["inetrc_sha256"]
        assert report["schedulers"] == report["schedulers_online"] == report["async_threads"] == 2
        assert report["dirty_cpu_schedulers"] == report["dirty_cpu_schedulers_online"] == 1
        assert report["dirty_io_schedulers"] == 1
        if role != "preflight":
            assert report["live_environment"] == {
                "ERL_FLAGS": report["erl_flags"], "ERL_INETRC": report["erl_inetrc"],
            }
            beam, = (p for p in report["processes"] if p["pid"] == report["pid"])
            assert beam["comm"] == "beam.smp" and beam["exe"] == "/opt/otp/erts-17.1/bin/beam.smp"


def replay(directory, root):
    plan = read(directory / "plan.json")
    receipt = read(directory / "receipt.json")
    assert receipt["plan_sha256"] == digest(directory / "plan.json")
    assert receipt["ledgers_sha256"] == digest(directory / "ledgers.ndjson.gz")
    inputs = receipt["inputs"]
    for key in ("relink", "source_verification", "layout_trace"):
        record = inputs[key]
        assert digest(root / record["path"]) == record["sha256"], key
    for record in inputs["transfer_and_scorer"]:
        if not record["path"].startswith("target/"):
            assert digest(root / record["path"]) == record["sha256"], record["path"]
    source = read(root / inputs["source_verification"]["path"])
    assert source["verified_source_commit"] == plan["source_commit"]
    assert len(source["gates"]) == 19 and all(gate["exit"] == 0 for gate in source["gates"].values())
    dense = receipt["dense_prerequisite"]
    assert dense["event"] == "verified_stop" and dense["stop"]["kind"] == "stop"
    assert receipt["freeze"]["sha256"]["plan.json"] == receipt["plan_sha256"]
    assert receipt["freeze"]["monotonic"] < dense["monotonic"]
    assert all(event["monotonic"] >= dense["monotonic"] for event in receipt["run_events"])
    for label, witness in receipt["lifecycle"].items():
        if witness is None:
            continue
        assert witness["elf"]["sha256"] == inputs["images"][label]["sha256"]
        assert witness["constructor"]["worker_exit_without_effect_channel"] == 174
        assert len(witness["lifecycle"]) == 3
        pids = {tuple(sorted(sequence)) for sequence in witness["lifecycle"].values()}
        cohort, = pids
        assert len(cohort) == len(set(cohort)) == witness["elf"]["consuming_waits"] == 64
    with gzip.open(directory / "ledgers.ndjson.gz", "rt") as stream:
        ledgers = {tuple(row["key"]): row for row in map(json.loads, stream)}
    h, settings = scorer(root)
    if guest := receipt["common_guest"]:
        assert guest["vcpus"] == guest["memory_gib"] == 2
        assert guest["oban_erl_flags"] == settings["oban_erl_flags"]
        assert guest["oban_revision"] == "8526a5bd70a1cb37f8fc27611021c90a44a89590"
        assert guest["oban_erl_inetrc"] == "/proof/oban.inetrc"
        assert guest["inetrc_sha256"] == "ea95be6f379d135ab873d2da91daeb4d6fcbb593e27db22914bf52aca8543a9c"
    results = {}
    completed = {event["group"] for event in receipt["run_events"]
                 if event["event"] == "result" and event["exit"] == 0}
    for profile, groups in receipt["profiles"].items():
        complete = len(groups) == len(plan["arms"][profile]) == settings["trial_pairs"]
        names = set()
        for group in groups:
            assert group["name"] not in names and group["eligible"] is True
            completed.remove(group["name"])
            names.add(group["name"])
            assert group["order"] == plan["arms"][profile][group["index"] - 1]
            assert set(group["order"]) == set(group["measured"]) == set(ARMS)
            assert group["postgres"] == "17.11 / aarch64-unknown-linux-gnu"
            verify_oban(group, guest)
            for label, arm in group["measured"].items():
                if label != "oban":
                    assert arm["elf_sha256"] == inputs["images"][label]["sha256"]
                verify_arm(h, settings, profile, label, arm, ledgers.pop((group["name"], label)))
        assert len({group["index"] for group in groups}) == len(groups)
        results[profile] = {
            "qualification_group_count_met": complete,
            "aggregate": {label: score(h, settings, profile, groups, label) for label in ARMS[:2]} if groups else {},
            "same_boot_breaches": {group["name"]: {
                label: score(h, settings, profile, [group], label)["breaches"] for label in ARMS[:2]}
                for group in groups},
        }
    assert not ledgers and not completed
    return results


if __name__ == "__main__":
    assert "proof" in HERE.parts, "Run the copy under the committed proof directory"
    print(json.dumps(replay(HERE, HERE.parents[2]), indent=2))
