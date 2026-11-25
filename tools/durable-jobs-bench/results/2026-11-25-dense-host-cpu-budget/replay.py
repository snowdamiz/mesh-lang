"""Bound two-core host work using archived /proc/stat reads and committed job times."""
import hashlib
import json
import tarfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
PACKET = HERE.parent / "2026-11-24-dense-individual-hot-pool/logger-off-retry"
NATIVE = PACKET / "evidence/native"
BUSY = (0, 1, 2, 5, 6)  # user, nice, system, IRQ, soft IRQ
CPUS = ("cpu0", "cpu1")
MARGIN_US = 2_000
WAIT_US = 200_000


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def frame_bounds(points):
    offsets = [p["epoch_us"] - p["monotonic_ns"] // 1000 for p in points]
    assert max(offsets) - min(offsets) <= MARGIN_US, "unstable clocks"
    frames = []
    # The first point precedes process launch. The final wait can return early.
    for previous, current in zip(points, points[1:-1]):
        earliest = previous["epoch_us"] + WAIT_US - MARGIN_US
        latest = current["epoch_us"] + MARGIN_US
        assert earliest <= latest and set(current["cpu"]) == set(CPUS)
        frames.append((earliest, latest, current["cpu"]))
    return frames


def work_floor(frames, start, end, tick_us):
    """Tick-accounted CPU floor, allowing both edges to straddle the interval."""
    assert start < end
    assert frames[0][1] < start < end < frames[-1][0]
    best = 0
    around_window = [f for f in frames if f[1] >= start - 2 * WAIT_US and f[0] <= end + 2 * WAIT_US]
    for i, (early, _, first) in enumerate(around_window):
        for _, late, last in around_window[i + 1:]:
            ticks = [last[cpu][k] - first[cpu][k] for cpu in CPUS for k in BUSY]
            assert min(ticks) >= 0
            # Quantized cumulative endpoints lose at most one tick per difference.
            busy = sum(max(0, n - 1) for n in ticks) * tick_us
            spill = len(CPUS) * (max(0, start - early) + max(0, late - end))
            best = max(best, busy - spill)
    assert best <= len(CPUS) * (end - start), "tick accounting exceeds physical window"
    return best


def replay():
    official = json.loads((NATIVE / "summary-1795570643531732000.json").read_text())
    assert official["qualified_rotations"] == 6 and not official["failures"] and not official["missing"]
    config = json.loads((NATIVE / "series.json").read_text())["config"]
    assert config["clock_ticks"] == 100 and config["cpus"] == [0, 1]
    worker_path = "tools/durable-jobs-bench/worker.py"
    expected = json.loads((PACKET / "timing.json").read_text())["local_sha256"][worker_path]
    assert expected == "16d1e01d19ee7b325908f07a69c6cdc772caf7d034ad79a6583494d2e107993b"
    assert digest(HERE.parents[1] / "worker.py") == expected
    groups = {}
    for group, block in sorted(official["blocks"].items()):
        lanes = {}
        for key, score in sorted(block["scores"].items()):
            if not key.startswith("measured-"):
                continue
            raw, = (NATIVE / "blocks" / block["attempt"] / key).glob("*/raw.tar.gz")
            assert digest(raw) == score["raw_sha256"]
            with tarfile.open(raw) as archive:
                points = [json.loads(line) for line in archive.extractfile("host-samples.jsonl")]
                ledger = json.load(archive.extractfile("adapter/ledger.json"))
                host = json.load(archive.extractfile("host.json"))
            assert host["affinity"] == [0, 1] and host["boot_id"] == config["host"][0]
            assert score["completed"] == len(ledger["jobs"]) == 1024 and not score["failed"]
            frames = frame_bounds(points)
            first_submit = min(t["submitted_us"] for t in ledger["transactions"])
            first_commit = min(t["committed_us"] for t in ledger["transactions"])
            terminal = max(j["settled_us"] for j in ledger["jobs"])
            assert (first_submit, first_commit, terminal) == (
                score["first_submit_us"], score["first_commit_us"], score["last_settlement_us"])
            row = {"raw_sha256": score["raw_sha256"]}
            for label, start in (("drain", first_commit), ("submit", first_submit)):
                row[label + "_us"] = terminal - start
                row[label + "_busy_floor_cpu_us"] = work_floor(frames, start, terminal, 10_000)
            lanes[key.removeprefix("measured-")] = row
        assert len(lanes) == 6
        groups[group] = lanes
    return groups


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2, sort_keys=True))
