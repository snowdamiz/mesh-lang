"""Locate both public/private drain differences on the immutable transfer captures."""
import hashlib
import io
import json
from pathlib import Path
from runpy import run_path
import subprocess
import tarfile

COMMIT = "f0ae2105488f4630d77406c9fd5f739597d023ad"
RESULTS = Path(__file__).resolve().parent.parent
PACKET = "tools/durable-jobs-bench/results/2026-11-25-public-dense-3c4032d-transfer/logger-off-recovery/evidence/pilot"
interval = run_path(str(RESULTS / "2026-11-25-dense-fixed8-pilot-review/replay.py"))["interval"]


def blob(name):
    return subprocess.check_output(["git", "show", f"{COMMIT}:{PACKET}/{name}"], cwd=RESULTS)


def capture(content, score):
    with tarfile.open(fileobj=io.BytesIO(content)) as archive:
        def read(name):
            return json.load(archive.extractfile(name))
        assert read("status.json")["exit_code"] == 0
        host = read("host.json")
        assert host["pg_mount"]["maj:min"] == "8:0" and host["affinity"] == [0, 1]
        samples = [json.loads(line) for line in archive.extractfile("host-samples.jsonl")]
        ledger = read("adapter/ledger.json")
        stats = read("adapter/raw.json")
        policy = read("adapter/queue-policy.json")
    tx, = ledger["transactions"]
    jobs = ledger["jobs"]
    assert len(jobs) == len(tx["enqueued_ids"]) == score["completed"] == 1024
    assert len({j[k] for j in jobs for k in ("attempt_xid", "effect_xid")}) == 2048
    start = tx["committed_us"]
    effects = sorted(j["effect_committed_us"] for j in jobs)
    finish = max(j["settled_us"] for j in jobs)
    marks = [start, *effects[127::128], finish]
    spans = [b - a for a, b in zip(marks, marks[1:])]
    assert min(spans) >= 0 and sum(spans) == score["drain_us"]
    assert len({j["settled_xid"] for j in jobs}) == policy["terminal_xids"]
    bracket = interval(samples, start, finish)["counts"]
    return dict(drain_us=sum(spans), effect_128_windows_us=spans[:-1], terminal_tail_us=spans[-1],
                terminal_xids=policy["terminal_xids"], runner_sql_calls=stats["raw_runner_sql_calls"],
                claim_calls=stats["raw_claim_calls"], runner_cpu_ms=score["runner_cpu_ms"],
                device_busy_ms=bracket["busy_ms"], device_flush_ms=bracket["flush_ms"])


def replay():
    assert json.loads(blob("controller-release.json"))["exit_status"] == 0
    raw = blob("replay.json")
    assert raw == blob("independent-replay.json")
    verdict = json.loads(raw)
    assert [g["passed"] for g in verdict["paired_gates"].values()] == [True, False]
    groups = {}
    for path, digest in verdict["raw_sha256"].items():
        _, folder, cell, _, _ = path.split("/")
        phase, lane = cell.split("-")
        group = str(int(folder.split("-")[0]))
        data = blob(path)
        assert hashlib.sha256(data).hexdigest() == digest
        groups.setdefault(group, {}).setdefault(phase, {})[lane] = capture(data, verdict["groups"][group][phase][lane])
    assert len(verdict["raw_sha256"]) == 16 and set(groups) == {"1", "2"}
    for group in groups.values():
        assert set(group) == {"warmup", "measured"}
        for cells in group.values():
            assert set(cells) == {"A1", "B", "C", "A2"}
    return groups


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2, sort_keys=True))
