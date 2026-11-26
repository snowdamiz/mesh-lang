"""Bracket device activity during the official enqueue-commit to settlement clock."""
import argparse
import json
import tarfile
from hashlib import sha256
from io import BytesIO
from pathlib import Path

from direct4_peer_deadlines import SERIES, clocks, ledger, warm_river
from direct4_quiet_cost import RELEASE, REPLAY, read


RECORDER = "16d1e01d19ee7b325908f07a69c6cdc772caf7d034ad79a6583494d2e107993b"
DISK_FIELDS = {"reads": 0, "writes": 4, "sectors_written": 6, "write_ms": 7,
               "busy_ms": 9, "completed_flushes": 15, "flush_ms": 16}


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
        frames.append((first, last, {name: int(device[k]) for name, k in DISK_FIELDS.items()}))
    for left, right in zip(frames, frames[1:]):
        assert all(left[2][name] <= right[2][name] for name in DISK_FIELDS)
    inside = [f for f in frames if f[0] >= start and f[1] <= stop]
    assert len(inside) >= 2
    before = max(f for f in frames if f[1] <= start)
    after = next(f for f in frames if f[0] >= stop)
    result = {"drain_us": drain_us, "raw_sha256": digest}
    for name in DISK_FIELDS:
        result[name] = [inside[-1][2][name] - inside[0][2][name], after[2][name] - before[2][name]]
    return result


def measured_archives(directory, fixed_workers=None, protected=None):
    worker = Path(__file__).resolve().parents[2] / "worker.py"
    assert sha256(worker.read_bytes()).hexdigest() == RECORDER
    release = json.loads(read(directory / "controller-release.json", RELEASE))
    request = json.loads(read(directory / "request.json", release["request_sha256"]))
    report = json.loads(read(directory / "controller/replay.json", REPLAY))
    assert release["exit_status"] == 0 and report["complete"] and not report["errors"]
    boot = request["host"][0]
    for block, phases in report["arms"].items():
        for lane, score in phases["measured"].items():
            name = f"{block}/measured-{lane}/measured-mesh/raw.tar.gz"
            yield name, directory / name, report["evidence_sha256s"][name], score["drain_us"], boot
    for group, score in warm_river().items():
        path, = SERIES.glob(f"blocks/{group:02}-*/measured-river-warm/*/raw.tar.gz")
        yield str(path.relative_to(SERIES)), path, score["raw_sha256"], score["drain_us"], boot
    if fixed_workers is not None:
        fixed = json.loads(read(fixed_workers / "controller-release.json",
                                "529bc600a6d74400a17565b22582353bcde94033084bc76e40f8a63c7bf25001"))
        score = json.loads(read(fixed_workers / "score-before-release.json", fixed["score_sha256"]))
        assert fixed["exit_status"] == 0 and score["complete"] and len(score["arms"]) == 16
        measured = {n: a for n, a in score["arms"].items() if n.endswith("/measured-mesh")}
        assert len(measured) == 8
        for name, arm in measured.items():
            yield "fixed/" + name, fixed_workers / name / "raw.tar.gz", arm["raw_sha256"], arm["drain_us"], boot
    if protected is not None:
        pilot = json.loads(read(protected / "controller-release.json",
                                "8e7ae41878b0b386dbb8531d020d6c8372ec897ee49afd588d7387ef3505def0"))
        score = json.loads(read(protected / "controller/replay.json",
                                "9b2c3668ddbdbaa798e5f05f59e5b0c95fcb66c5828e7cc5d974cfc2ce466d95"))
        assert pilot["exit_status"] == 0 and score["complete"] and not score["errors"]
        assert score["request_sha256"] == pilot["request_sha256"]
        assert len(score["arms"]) == 2
        for block, phases in score["arms"].items():
            assert set(phases["measured"]) == {"original", "off-a", "off-b", "on"}
            for lane, arm in phases["measured"].items():
                name = f"{block}/measured-{lane}/measured-mesh/raw.tar.gz"
                yield "protected/" + name, protected / name, score["evidence_sha256s"][name], arm["drain_us"], boot


def replay(directory, fixed_workers=None, protected=None):
    inputs = measured_archives(directory, fixed_workers, protected)
    results = {name: measure(path, digest, drain_us, boot) for name, path, digest, drain_us, boot in inputs}
    assert len(results) == 14 + 8 * (fixed_workers is not None) + 8 * (protected is not None)
    return results


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("pilot_directory", type=Path)
    parser.add_argument("--fixed-workers", type=Path)
    parser.add_argument("--protected", type=Path)
    args = parser.parse_args()
    print(json.dumps(replay(args.pilot_directory, args.fixed_workers, args.protected), indent=2, sort_keys=True))
