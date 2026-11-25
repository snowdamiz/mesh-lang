"""Reconcile selected public stock, FILE provenance and split V1 native evidence."""

import json
from pathlib import Path
import runpy
import subprocess
import sys
import tarfile

from prepare import HERE, LARGE, PRODUCT, RECOVERED, inputs
from score import BROKER_SHA, PROFILE, clocks, run, runner_reports, score_arm, scorer

ROOT = HERE.parents[3]
RESULTS = HERE.parent
BASELINE = PRODUCT / "evidence/native/timing-closure-retry"
STOCK = BASELINE / "slow"
INBOX = ROOT / "target/linode-jobs-stepchange/requests"
V1 = RESULTS / "2026-11-23-ordinary-slow-runner-concentration"
EXTERNAL = RESULTS / "2026-11-25-public-large-file-external/grant-parser-retry"
DEFAULT = LARGE / "resume"
PEERS = ("river", "oban", "apalis")
KEYS = ("enqueue_ms", "submit_ms", "drain_ms", "last_effect_ms", "late_refill_ms")


def files_for(folder, block, config):
    for phase in block["phases"]:
        for label in block["order"]:
            lane = config["lanes"][label]
            path = folder / f"{phase}-{label}" / f"{phase}-{lane['engine']}"
            yield phase, label, path


def stock_report():
    series = run.read_json(STOCK / "series.json")
    plan = run.read_json(BASELINE / "slow.inventory.json")
    assert plan["recipe"]["blocks"] == series["plan"] and plan["configuration"] == series["config"]
    archived = run.read_json(STOCK / "summary.json")
    config = plan["configuration"]
    releases = sorted((STOCK / "controller-evidence").glob("*.released.json"))
    assert releases and sum((run.read_json(p)["groups"] for p in releases), []) == [1, 2, 3, 4, 5]
    for path in releases:
        assert run.read_json(path)["exit_status"] == 0
        mirror = INBOX / path.name
        if mirror.exists():
            assert run.digest(mirror) == run.digest(path)
    output = []
    observed = set()
    for block in series["plan"]:
        folder, = (STOCK / "blocks").glob(f"{run.block_prefix(block)}.*")
        assert run.read_json(folder / "order.json") == block
        measured, hashes = {}, {}
        for phase, label, path in files_for(folder, block, config):
            score = score_arm(path, block, phase, config["lanes"][label], config)
            key = f"{phase}-{label}"
            assert score["raw_sha256"] not in observed
            observed.add(score["raw_sha256"])
            hashes[key] = score["raw_sha256"]
            if phase == "measured":
                measured[label] = {key: score[key] for key in (*KEYS, "peak_connections", "terminal_xids")}
        mesh = measured["product"]
        output.append(dict(group=block["group"], raw_sha256=hashes, measured=measured,
                           mesh_minus_peer_ms={p: {key: round(mesh[key] - measured[p][key], 3) for key in KEYS}
                                               for p in PEERS}))
    assert len(observed) == 40 and archived
    return output


def replay_file_pins(path, args, receipt):
    observed = subprocess.check_output([sys.executable, "-B", str(path), *args], cwd=ROOT)
    assert observed == receipt.read_bytes(), receipt
    groups = json.loads(observed)["groups"]
    return dict(receipt_sha256=run.digest(receipt), blocks=[
        dict(group=group["group"], effects=group.get("faster_duplicate_minus_file_ms",
                                                     group.get("faster_inline_minus_file_ms")))
        for group in groups])


def v1_report():
    # Run the original scorer for every arm and independently score the
    # unmodified split ledgers with the topology-scoped public verifier.
    sys.path.insert(0, str(V1))
    try:
        retry = runpy.run_path(str(V1 / "native-import-retry.py"))
        native = retry["native"]
        native.staged = retry["staged"]
        cfg = native.freeze("two")
        out = retry["OUT"]
        verified = native.replay(out, cfg, retry["pins"]())
        expected = out / "native-followup-replay.json"
        assert json.loads(json.dumps(verified)) == run.read_json(expected)
        series = run.read_json(out / "series.json")
        split = []
        for block in cfg["plan"]:
            path, = (out / "blocks").glob(f"{run.block_prefix(block)}.*")
            for phase, label, scope in files_for(path, block, cfg):
                if label not in ("one-a", "one-b", "two"):
                    continue
                runners = 2 if label == "two" else 1
                _, inventory = retry["staged"](cfg, label, run.REMOTE / series["remotes"][label])
                result = scorer(runners)(scope, block, phase, "mesh", inventory)
                with tarfile.open(scope / "raw.tar.gz") as archive:
                    ledger = run.file_json(archive, "adapter/ledger.json")
                    witnesses = run.file_json(archive, "adapter/topology.json")
                    pids = [int(pid) for pid in witnesses["runner_pids_by_index"]]
                    jobs = runner_reports(archive, ledger, pids)
                    metric = clocks(ledger, result)
                split.append(dict(group=block["group"], phase=phase, lane=label,
                                  jobs_by_runner=jobs, slots=ledger["slots"],
                                  raw_sha256=run.digest(scope / "raw.tar.gz"), **metric))
        return dict(receipt_sha256=run.digest(expected), arms=split, paired=verified["summary"]["paired"])
    finally:
        sys.path.pop(0)


def main():
    _, source = inputs()
    qualified = run.read_json(RECOVERED / "qualified.json")
    assert qualified["broker_sha256"] == BROKER_SHA
    with tarfile.open(RECOVERED / "raw.tar.gz") as raw:
        pins = raw.extractfile("./pins.txt").read().decode().splitlines()
        assert "broker-main.mpl: OK" in pins and "original-main.mpl: OK" in pins
    pending = []
    for pattern in ("*slow*.json", "*file*.json", "*concentration*.json"):
        pending.extend(path.name for path in INBOX.glob(pattern) if ".released." not in path.name)
    assert not any("public-slow-file" in name for name in pending)
    print(json.dumps(dict(stock=stock_report(), v1=v1_report(), source=source, inbox=sorted(set(pending)),
                          large_default=replay_file_pins(DEFAULT / "adapter-path-retry/retry.py", ["scores"],
                              DEFAULT / "evidence/native/timing-adapter-path-retry-scores.json"),
                          large_external=replay_file_pins(EXTERNAL / "retry.py", ["replay"],
                              EXTERNAL / "evidence/replay.json")),
                     sort_keys=True, indent=2, allow_nan=False))


if __name__ == "__main__":
    main()
