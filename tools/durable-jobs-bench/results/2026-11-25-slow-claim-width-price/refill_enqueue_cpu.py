"""Read pinned host CPU inside the cap68 producer transaction using the worker's wait interval."""
from hashlib import sha256
import json
from pathlib import Path
import sys
import tarfile

WORKER = Path(__file__).resolve().parents[2] / "worker.py"
WORKER_SHA = "16d1e01d19ee7b325908f07a69c6cdc772caf7d034ad79a6583494d2e107993b"
RELEASE_SHA = "1bd34ec7973f01b2d57c7ba93e27741e03906f088d1d9962f99eedf09cdebde1"
CLOCK_GUARD_US = 1000


def read(path, digest):
    assert sha256(path.read_bytes()).hexdigest() == digest
    with tarfile.open(path) as raw:
        tx, = json.load(raw.extractfile("adapter/ledger.json"))["transactions"]
        frames = list(map(json.loads, raw.extractfile("host-samples.jsonl")))
    start, end = tx["submitted_us"], tx["committed_us"]
    samples = []
    drift = []
    for previous, current in zip(frames, frames[1:]):
        wall_us = current["epoch_us"] - previous["epoch_us"]
        mono_us = (current["monotonic_ns"] - previous["monotonic_ns"]) / 1000
        assert wall_us > 0 and mono_us > 0
        drift.append(abs(wall_us - mono_us))
    # The endpoint after process exit need not have waited out the timeout.
    for previous, current in zip(frames, frames[1:-1]):
        assert current["monotonic_ns"] - previous["monotonic_ns"] >= 200_000_000
        early = previous["epoch_us"] + 200_000 - CLOCK_GUARD_US
        late = current["epoch_us"] + CLOCK_GUARD_US
        if start <= early <= late <= end:
            samples.append((early, late, current["cpu"]))
    assert max(drift) < CLOCK_GUARD_US
    result = dict(raw_sha256=digest, enqueue_us=end-start, interior_cpu_reads=len(samples),
                  max_adjacent_clock_delta_difference_us=round(max(drift), 3))
    if len(samples) < 2:
        return result
    first, last = samples[0], samples[-1]
    assert set(first[2]) == set(last[2]) == {"cpu0", "cpu1"}
    ticks = [sum(last[2][cpu][k] - first[2][cpu][k] for cpu in first[2]) for k in range(8)]
    assert all(count >= 0 for count in ticks)
    return dict(result, read_span_bounds_us=[last[0]-first[1], last[1]-first[0]],
                start_read_after_submit_us=[first[0]-start, first[1]-start],
                end_read_after_submit_us=[last[0]-start, last[1]-start],
                ticks=ticks, idle_ticks=ticks[3], total_ticks=sum(ticks))


def replay(directory):
    assert sha256(WORKER.read_bytes()).hexdigest() == WORKER_SHA
    release = (directory / "controller-release.json").read_bytes()
    assert sha256(release).hexdigest() == RELEASE_SHA
    archives = json.loads(release)["raw_archives"]
    assert len(archives) == 20
    return {name: read(directory / name, digest) for name, digest in sorted(archives.items())}


if __name__ == "__main__":
    print(json.dumps(replay(Path(sys.argv[1])), indent=2, sort_keys=True))
