#!/usr/bin/env python3
"""Replay matched broker-only arms through the retained matrix and verify commits."""
from collections import Counter
from contextlib import redirect_stdout
import io
import importlib.util
import json
from statistics import median
from pathlib import Path
import sys
import tarfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
SUITE = ROOT / "tools/durable-jobs-bench"
sys.path.insert(0, str(SUITE))
import run
from rotation import DECISION_MS
MATRICES = {"short": ROOT / "target/ordinary-hot-checkout/bulk-v2/native/interleave.py",
            "large": ROOT / "target/ordinary-trusted-large-assembly/replay-retry/interleave.py"}


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


def durable(folder, config, phase, label, score=None):
    """Require the persisted XID and all independent writes from the ordinary lane."""
    profile, = config["profiles"]
    lane = config["lanes"][label]["inventory"]
    mesh = lane["engines"]["mesh"]
    if score is None:
        score = run.score_arm(folder, {"profile": profile}, phase, "mesh", lane)
    with tarfile.open(folder / "raw.tar.gz") as archive:
        def read(name):
            return run.file_json(archive, f"adapter/{name}.json")
        ledger = read("ledger")
        transaction, = ledger["transactions"]
        count = 128 if profile == "short" else 64
        jobs = ledger["jobs"]
        assert len(jobs) == count and {job["id"] for job in jobs} == set(transaction["enqueued_ids"].values())
        assert transaction["xid"] and transaction["committed_us"] >= transaction["submitted_us"]
        xids = Counter()
        for job in jobs:
            assert job["attempt"] == job["attempts"] == job["effects"] == 1 and job["state"] == "completed"
            assert job["attempt_committed_us"] >= transaction["committed_us"]
            assert job["attempt_committed_us"] <= job["effect_committed_us"] <= job["settled_us"]
            xids.update([job["attempt_xid"], job["effect_xid"]])
        assert len(xids) == 2 * count and transaction["xid"] not in xids
        assert ledger["execution_contract"] == config["contract"]
        assert ledger["database"]["synchronous_commit"] and ledger["database"]["track_commit_timestamp"]
        if profile == "short":
            identity = read("producer-identity")
            assert (identity["path"], identity["sha256"]) == tuple(mesh["argv"][-3:-1])
            assert identity["invocations"] == 1
            writer = read("producer-writer")
            assert (writer["enabled"], writer["bulk"], writer["scalar"]) == (True, 1, 0)
        else:
            assembly = read("assembly")
            workmem = read("workmem")
            assert workmem["after"]["settings"]["work_mem"]["value"] == "16384"
            broker_sha = assembly["broker_sha256"]
            if label == "candidate":
                path, = [p for p in mesh["verified_files"] if "/enqueue-spec-preparation-large-" in p and p.endswith("/broker")]
            else:
                path, = [p for p in mesh["verified_files"] if p.endswith("/mesh-direct-producer-v3/broker")]
            assert broker_sha == mesh["verified_files"][path]
            storage = read("payload-storage")
            assert storage["policy"] == "EXTERNAL" and storage["persisted"]["metadata"]["attstorage"] == "e"
            assert sorted((row["job_key"], row["payload_sha256"]) for row in jobs) == sorted(
                (row["job_key"], row["payload_sha256"]) for row in storage["persisted"]["rows"]
                if row["payload_bytes"] == 524288)
            assert all(mask == [0, 1] for mask in assembly["affinity"].values())
    return {"committed_jobs": count, "application_commits": len(xids), "enqueue_xid": transaction["xid"],
            "first_submit_to_commit_ms": score["first_submit_to_commit_ms"], "raw_sha256": run.digest(folder / "raw.tar.gz")}


def full_submit_decision(profile, summary):
    """Use matched full-submit clocks, with enqueue commit as a secondary clock."""
    blocks = summary["rows"].get(profile, [])
    threshold = DECISION_MS[profile]
    if summary["missing"] or len(blocks) != 5:
        return {"status": "incomplete", "qualified_rotations": len(blocks)}
    rows = []
    for block in blocks:
        assert block["durable_broker"]["measured"].keys() == {"stock-a", "stock-b", "candidate"}
        assert all(row["completed"] == (128 if profile == "short" else 64) for row in block["scores"].values())
        full = {label: (row["last_settlement_us"] - row["first_submit_us"]) / 1000
                for label, row in block["scores"].items()}
        enqueue = {label: row["first_submit_to_commit_ms"] for label, row in block["scores"].items()}
        assert all(duration > 0 and 0 <= enqueue[label] <= duration for label, duration in full.items())
        spread = abs(full["stock-a"] - full["stock-b"])
        gain = min(full["stock-a"], full["stock-b"]) - full["candidate"]
        rows.append({
            "attempt": block["attempt"], "full_submit_ms": full, "submit_to_enqueue_commit_ms": enqueue,
            "saving_vs_faster_duplicate_ms": gain, "duplicate_spread_ms": spread,
            "candidate_margin_vs_peers_ms": {
                label: duration - full["candidate"] for label, duration in full.items()
                if label not in {"stock-a", "stock-b", "candidate"}
            },
        })
    median_gain = median(row["saving_vs_faster_duplicate_ms"] for row in rows)
    wins = sum(row["saving_vs_faster_duplicate_ms"] > row["duplicate_spread_ms"] for row in rows)
    return {
        "status": "material_gain" if median_gain >= threshold and wins >= 4 else "below_material_gain_gate",
        "primary_clock": "first producer submit to last durable settlement",
        "minimum_median_saving_ms": threshold, "median_saving_vs_faster_duplicate_ms": median_gain,
        "rotations_above_duplicate_spread": wins, "rotations": rows,
    }


def checked_config(profile, series, path):
    from broker import assemble
    from controller import ID, check, grant

    check()
    assert series == f"{ID}-{profile}-matched"
    grant("TIMING", series)
    path = path.resolve()
    attempts = (HERE / "evidence").resolve()
    assert path.parent.name == "raw" and path.parent.parent.parent == attempts
    assert path.name == f"{profile}-trial.json"
    evidence = path.parent.parent
    status = run.read_json(evidence / "status.json")
    assert (status["action"], status["request_id"], status["exit_status"]) == (
        "broker", f"{ID}-{profile}-broker", 0)
    exported = evidence / "raw.tar.gz"
    assert run.digest(exported) == status["raw_sha256"]
    with tarfile.open(exported) as archive:
        for name in (path.name, "link.json"):
            assert archive.extractfile(name).read() == (path.parent / name).read_bytes()
    link = run.read_json(path.parent / "link.json")
    identities = run.read_json(HERE / "candidate-identities.json")
    assert link["source_identity"] == identities[profile]
    assert link["profile"] == profile and assemble(profile, link) == run.read_json(path)
    return run.read_json(path)


def main():
    first, *arguments = sys.argv[1:]
    if first in MATRICES:
        profile = first
        action, *arguments = arguments
    else:
        action = first
        cfg = Path(arguments[arguments.index("--config") + 1])
        profile, = json.loads(cfg.read_text())["profiles"]
    stock_matrix = module(f"{profile}_matrix", MATRICES[profile])
    matrix_replay = stock_matrix.replay

    def replay(block_path, config, block):
        result = matrix_replay(block_path, config, block)
        evidence = {}
        for phase, label, folder in stock_matrix.arms(block_path, config, block):
            if label in {"stock-a", "stock-b", "candidate"}:
                score = result["scores"][label] if phase == "measured" else None
                evidence.setdefault(phase, {})[label] = durable(folder, config, phase, label, score)
        return dict(result, durable_broker=evidence)

    if action == "offline-example":
        from broker import assemble
        path = f"/var/lib/durable-jobs-bench/native/builds/enqueue-spec-preparation-{profile}-0123456789ab/" + ("mesh" if profile == "short" else "broker")
        launch = {path: "1" * 64}
        if profile == "large":
            launch["/var/lib/durable-jobs-bench/native/enqueue-spec-preparation-large-0123456789ab/mesh-run"] = "2" * 64
        config = assemble(profile, {"broker": path, "launch_files": launch})
        stock_matrix.validate(config)
        assert config["lanes"]["stock-a"] == config["lanes"]["stock-b"]
        print("Conditional plan validates with its stock matrix and duplicate controls")
        return
    stock_matrix.replay = replay
    directory = HERE / "evidence" / arguments[0]
    if action == "dispatch":
        import argparse
        parser = argparse.ArgumentParser()
        parser.add_argument("series")
        parser.add_argument("--config", required=True, type=Path)
        parser.add_argument("--group", action="append", type=int, choices=range(1, 6))
        args = parser.parse_args(arguments)
        config = checked_config(profile, args.series, args.config)
        return stock_matrix.dispatch(directory, config, args.group)
    if action == "summarize":
        result = io.StringIO()
        with redirect_stdout(result):
            status = stock_matrix.summarize(directory)
        summary = json.loads(result.getvalue())
        summary["full_submit_decision"] = full_submit_decision(profile, summary)
        print(json.dumps(summary, indent=2, sort_keys=True, allow_nan=False))
        return status
    raise ValueError(f"unknown trial action {action}")


if __name__ == "__main__":
    sys.exit(main())
