"""Bracket device flush completions during the official enqueue-commit to settlement clock."""
import argparse
import json
import tarfile
from hashlib import sha256
from io import BytesIO
from pathlib import Path

from direct4_peer_deadlines import SERIES, clocks, ledger, warm_river
from direct4_quiet_cost import RELEASE, REPLAY, read


RECORDER = "16d1e01d19ee7b325908f07a69c6cdc772caf7d034ad79a6583494d2e107993b"


def measure(path, digest, drain_us, boot):
    data = read(path, digest)
    jobs, transaction = ledger(data, digest)
    assert clocks(jobs, transaction)["drain_us"] == drain_us
    start = transaction["committed_us"]
    stop = start + drain_us
    with tarfile.open(fileobj=BytesIO(data)) as raw:
        host = json.load(raw.extractfile("host.json"))
        assert host["boot_id"] == boot and host["affinity"] == [0, 1]
        assert host["pg_mount"]["maj:min"] == "8:0"
        assert [v for k, v in host["verified_files"].items() if k.endswith("/worker.py")] == [RECORDER]
        samples = [json.loads(row) for row in raw.extractfile("host-samples.jsonl")]
    offsets = [s["epoch_us"] - s["monotonic_ns"] // 1000 for s in samples]
    assert max(offsets) - min(offsets) <= 2000
    frames = []
    # diskstats follows the epoch stamp. Except for the final sample, the next
    # read follows a full 200 ms wait. Exclude the pair ending in process exit.
    for sample, following in zip(samples, samples[1:-1]):
        first = sample["epoch_us"] - 2000
        last = following["epoch_us"] - 200_000 + 2000
        assert first <= last
        device, = [row.split()[3:] for row in sample["diskstats"] if row.split()[:2] == ["8", "0"]]
        frames.append((first, last, int(device[15]), int(device[16])))
    assert all(a[2] <= b[2] and a[3] <= b[3] for a, b in zip(frames, frames[1:]))
    inside = [f for f in frames if f[0] >= start and f[1] <= stop]
    assert len(inside) >= 2
    before = max(f for f in frames if f[1] <= start)
    after = next(f for f in frames if f[0] >= stop)
    result = {"drain_us": drain_us, "raw_sha256": digest}
    for k, field in ((2, "completed_flushes"), (3, "flush_ms")):
        result[field] = [inside[-1][k] - inside[0][k], after[k] - before[k]]
    return result


def replay(directory):
    worker = Path(__file__).resolve().parents[2] / "worker.py"
    assert sha256(worker.read_bytes()).hexdigest() == RECORDER
    release = json.loads(read(directory / "controller-release.json", RELEASE))
    request = json.loads(read(directory / "request.json", release["request_sha256"]))
    report = json.loads(read(directory / "controller/replay.json", REPLAY))
    assert release["exit_status"] == 0 and report["complete"] and not report["errors"]
    boot = request["host"][0]
    results = {}
    for block, phases in report["arms"].items():
        for lane, score in phases["measured"].items():
            name = f"{block}/measured-{lane}/measured-mesh/raw.tar.gz"
            results[name] = measure(directory / name, report["evidence_sha256s"][name], score["drain_us"], boot)
    for group, score in warm_river().items():
        path, = SERIES.glob(f"blocks/{group:02}-*/measured-river-warm/*/raw.tar.gz")
        results[str(path.relative_to(SERIES))] = measure(path, score["raw_sha256"], score["drain_us"], boot)
    assert len(results) == 14
    return results


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("pilot_directory", type=Path)
    print(json.dumps(replay(parser.parse_args().pilot_directory), indent=2, sort_keys=True))
