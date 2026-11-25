"""Replay closed diagnostic arms and price the hook against duplicate inactive arms."""
import argparse
from collections import Counter
import errno
import json
from pathlib import Path
import re
import sys
import tarfile

from boundary import MAX_OFFSET_CHANGE_NS, US, join, require, summarize
from packet import HOST, MANIFEST, ORDERS, RESULTS, RUNNER, RUNNER_SHA, SOURCE, checked, inventory, run

sys.path.insert(0, str(RESULTS / "2026-11-23-dense-matched-topology"))
from qualification import check_affinity, diagnostic_reconciler, mesh_helpers, text


def score_arm(folder, phase, expected, enabled):
    spec = expected["engines"]["mesh"]
    path = folder / "raw.tar.gz"
    require(run.read_json(folder / "dispatch.json").get("exit_code") == 0, "adapter dispatch failed")
    with tarfile.open(path, "r:gz") as archive:
        names = archive.getnames()
        require(len(set(names)) == len(names), "duplicate archive member")
        invoke = run.file_json(archive, "invocation.json")
        request = run.read_json(folder / "request.json")
        require(all(invoke[k] == request[k] == v for k, v in
                    dict(phase=phase, profile="noop", attempt=request["attempt"], engine="mesh").items())
                and run.file_json(archive, "status.json")["exit_code"] == 0, "failed or wrong arm")
        host = run.file_json(archive, "host.json")
        require([host["boot_id"], host["kernel"], host["pg_mount"]["maj:min"]] == HOST
                and host["architecture"] == "x86_64" and host["affinity"] == [0, 1]
                and len(host["available_cpus"]) == 4 and host["verified_files"] == spec["verified_files"],
                "host or staged bytes differ")
        require(text(archive, "adapter.log").splitlines().count("POSTFLUSH_STAGED_HELPERS_READABLE") == 1,
                "missing reduced-UID file access receipt")

        def files(name):
            return run.file_json(archive, "adapter/" + name)

        ledger, row, raw, identity = map(files, ("ledger.json", "row.json", "raw.json", "runner-identity.json"))
        artifact = dict(version=SOURCE, artifact_sha256=MANIFEST)
        for item in (ledger, row):
            require(all(item[k] == v for k, v in artifact.items()) and item["phase"] == phase
                    and item["local_smoke"] is False
                    and item["execution_contract"] == "ordinary-trusted-shared-process", "runner contract differs")
        score = diagnostic_reconciler(2)(ledger, "mesh", "noop", artifact)
        samples = [json.loads(line) for line in text(archive, "host-samples.jsonl").splitlines()]
        host_window = run.sample_window(host, samples, score)
        timed_offsets = [sample["epoch_us"] * US - sample["monotonic_ns"] for sample in samples
                         if score["first_submit_us"] <= sample["epoch_us"] <= score["last_settlement_us"]]
        require(timed_offsets and max(timed_offsets) - min(timed_offsets) <= MAX_OFFSET_CHANGE_NS,
                "host observer detected a clock offset change during the workload")
        require(score["completed"] == 1024 == files("ordered-writes.json")["ordered"]
                and raw["raw_application_sql_calls"] == 4096
                and ledger["database"] == files("postgres-settings.json")
                and row["connection_cap"] == 96, "dense ordered workload differs")
        capacities = {"compiled_caller": 32, "parity_subject": 8, "parity_producer": 1}
        capacity = files("sql-capacity.json")
        require(capacity["role_caps"] == capacities and capacity["sql_queues"] ==
                [dict(name="default", concurrency=64, state="active", namespace=raw["namespace"])]
                and row["peak_connections"] == ledger["peak_connections"] == sum(capacities.values()),
                "unexpected session, connection or queue capacity")
        broker = files("database.json")
        require(broker["user"] == "parity_producer" and broker["tls"] is broker["durable"] is True
                and text(archive, "adapter/broker-output.log").splitlines()[0] == "SESSION_READY", "wrong producer")
        statements = files("scalar-enqueues.json")
        require(len(statements) == 1 and all(q.startswith("SELECT mesh_jobs.enqueue(") and n == 1024
                for q, n in statements.items()), "producer did not issue the frozen scalar enqueues")
        helpers = mesh_helpers()
        used = helpers["verify_sessions"](files, capacities, ledger["transactions"][0]["submitted_us"], set())
        helpers["verify_app"](files, ledger["jobs"], raw["namespace"])

        require(identity["mode"] == "direct-observed" and identity["app_sessions_per_runner"] == 16
                and identity["queues_per_runner"] == 1 and identity["slots_per_runner"] == 32
                and identity["postgres_cap"] == 96 and identity["scheduler_min"] == identity["scheduler_max"] == 2
                and identity["batch"] == identity["listen"] == "1"
                and identity["hashes"] == {"mesh": RUNNER_SHA, "mesh.json": MANIFEST}, "direct runner geometry differs")
        runners = identity["runners"]
        pids = [entry["pid"] for entry in runners]
        require(len(set(pids)) == len(pids) == 2 and len({entry["parent_pid"] for entry in runners}) == 1,
                "invalid direct runner roster")
        exe = RUNNER + "/mesh"
        for entry in runners:
            require(entry["argv"] == [exe, "run"] and entry["executable"] == exe and entry["cpus"] == [0, 1]
                    and entry["executable_sha256"] == RUNNER_SHA and entry["parent_pid"] > 1
                    and entry["start_ticks_before"] == entry["start_ticks_after"] == entry["launch_start_ticks"] > 0
                    and (entry["method"], entry["observed_executable"], entry["exe_errno"]) in
                    {("proc-exe", exe, None), ("owned-launch", None, errno.EACCES)}, "unverified direct launch")
        check_affinity(files("process-affinity.json"), set(pids))
        logs = [text(archive, f"adapter/runner-{i}.log") for i in range(2)]
        per_pid = Counter(job["worker_pid"] for job in ledger["jobs"])
        for pid, log in zip(pids, logs, strict=True):
            lines = log.splitlines()
            report = json.loads(lines[-1])
            require(lines.count("READY") == 1 and "APP_POOL_REGISTRATION_OK" not in lines
                    and report["namespace"] == raw["namespace"] and report["state"] == "stopped"
                    and report["drained"] and report["last_error"] is None
                    and report["claimed"] == report["finished"] == report["outcomes"]["completed"] == per_pid[pid]
                    and not any(n for k, n in report["outcomes"].items() if k != "completed")
                    and not any(report[k] for k in ("undrained", "recovered", "stale_acks", "pending_claims")),
                    "runner failed to drain")
        metadata = files("postflush-arm.json")
        exports = {}
        for name in names:
            if name.startswith("adapter/postflush-") and name.endswith(".log"):
                match = re.fullmatch(r"adapter/postflush-([1-9][0-9]*)\.log", name)
                require(match is not None, "invalid backend export name")
                exports[int(match[1])] = text(archive, name)
        rows = join(ledger, pids, logs, files("sessions-before.json"), files("sessions-end.json"),
                    metadata, exports, enabled)
        if enabled:
            require({row["backend"] for row in rows} == set(used), "sampled backends differ from SQL session witnesses")
        settings = {key: metadata[key] for key in ("parallel_workers", "max_prepared_transactions",
                                                   "synchronous_standby_names", "wal_sync_method")}
        return dict(raw_sha256=run.digest(path), drain_us=score["last_settlement_us"] - score["first_commit_us"],
                    settings=settings, host_window=host_window, split=summarize(rows), writes=rows)


def perturbation(lanes):
    metrics = {"drain_us": lambda s: s["drain_us"]}
    for kind in ("attempt", "effect"):
        metrics[kind + "_record_to_return_p95_us"] = (
            lambda s, kind=kind: s["split"][kind]["record_to_return_us"]["p95"])
    result = {}
    for key, metric in metrics.items():
        a, b, on = (metric(lanes[lane]) for lane in ("off-a", "off-b", "on"))
        center, spread = (a + b) / 2, abs(a - b)
        delta = on - center
        result[key] = dict(off_a=a, off_b=b, on=on, signed_delta=delta, off_spread=spread,
                           pass_gate=center > 0 and spread <= 0.05 * center
                           and abs(delta) <= max(0.03 * center, spread))
    return result


def replay(root, setup, stages, groups):
    checked()
    require(groups in ([1], [1, 2, 3]), "replay the screen or all three groups")
    require(set(stages) == set(ORDERS[0]), "wrong stage roster")
    expected = {lane: inventory(setup, stage, lane) for lane, stage in stages.items()}
    blocks = []
    for group in groups:
        order = ORDERS[group - 1]
        block = root / f"{group:02}-noop"
        require(run.read_json(block / "order.json") ==
                dict(group=group, profile="noop", phases=["warmup", "measured"], order=list(order)), "wrong arm order")
        trials = {}
        for phase in ("warmup", "measured"):
            trials[phase] = {lane: score_arm(block / f"{phase}-{lane}" / f"{phase}-mesh", phase,
                                            expected[lane], lane == "on") for lane in order}
        settings = [score["settings"] for arms in trials.values() for score in arms.values()]
        require(all(s == settings[0] for s in settings), "server settings differ between otherwise identical arms")
        costs = perturbation(trials["measured"])
        blocks.append(dict(group=group, trials=trials, perturbation=costs,
                           pass_gate=all(m["pass_gate"] for m in costs.values())))
        if group == 1 and not blocks[0]["pass_gate"]:
            break
    qualified = len(blocks) == 3 and all(block["pass_gate"] for block in blocks)
    shares = [b["trials"]["measured"]["on"]["split"]["both"]["conservative_share"] for b in blocks]
    classification = "inconclusive"
    if qualified and all(low > 0.55 for low, high in shares):
        classification = "record-to-callback dominated"
    if qualified and all(high < 0.45 for low, high in shares):
        classification = "callback-to-return dominated"
    return dict(blocks=blocks, screen_pass=blocks[0]["pass_gate"], qualified=qualified, classification=classification)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path)
    parser.add_argument("setup", type=Path)
    parser.add_argument("stages", type=Path, help="JSON map from lane to remote stage")
    parser.add_argument("groups", nargs="+", type=int)
    args = parser.parse_args()
    result = replay(args.root, run.read_json(args.setup), run.read_json(args.stages), args.groups)
    print(json.dumps(result))
    sys.exit(0 if result["screen_pass"] and (args.groups == [1] or result["qualified"]) else 1)
