"""Replay pinned-CPU counters strictly inside the recorded FILE producer interval."""
from hashlib import sha256
import json
from pathlib import Path
import sys
import tarfile

from replay import HERE, NATIVE


def window(path, digest, hz):
    assert sha256(path.read_bytes()).hexdigest() == digest
    with tarfile.open(path) as archive:
        samples = list(map(json.loads, archive.extractfile("host-samples.jsonl")))
        ledger = json.load(archive.extractfile("adapter/ledger.json"))
    txn, = ledger["transactions"]
    assert ledger["profile"] == "large" and len(ledger["jobs"]) == 64
    # The worker reads /proc/stat before stamping each sample. Use the preceding
    # timestamp as a conservative lower bound on the new sample's read time.
    selected = [point for prior, point in zip(samples, samples[1:])
                if txn["submitted_us"] <= prior["epoch_us"] < point["epoch_us"] <= txn["committed_us"]]
    record = dict(raw_sha256=digest, eligible_samples=len(selected),
                  enqueue_us=txn["committed_us"] - txn["submitted_us"])
    if len(selected) < 2:
        return record
    start, end = selected[0], selected[-1]
    duration = end["epoch_us"] - start["epoch_us"]
    assert abs(duration - (end["monotonic_ns"] - start["monotonic_ns"]) / 1000) < 1000
    assert all(set(s["cpu"]) == {"cpu0", "cpu1"} for s in selected)
    ticks = [sum(end["cpu"][c][i] - start["cpu"][c][i] for c in start["cpu"]) for i in range(8)]
    assert all(t >= 0 for t in ticks) and hz == 100
    return dict(record, observed_us=duration, ticks=ticks, idle_ms=ticks[3] * 1000 // hz,
                idle_percent=round(100 * ticks[3] / sum(ticks), 2))


def run(pilot):
    default = HERE.parent / "2026-11-24-public-large-file-session/resume/evidence/native/timing-adapter-path-retry"
    default_pins = json.loads((HERE.parent / "2026-11-25-public-large-claim-gap/peer-deadlines.json").read_bytes())
    external_pins = json.loads((HERE / "native.json").read_bytes())["blocks"]
    score = (pilot / "scored-before-release.json").read_bytes()
    assert sha256(score).hexdigest() == "e913615792bb6e7e6cc7da1ec991c99a23d1b11345d2f0e2246ab364e9953e71"
    sources = [
        ("default-file", default, [{"file": g["lanes"]["file"]["raw_sha256"]} for g in default_pins["blocks"]]),
        ("external-file", NATIVE, [{"file": external_pins[str(i)]["file"]["raw_sha256"]} for i in range(1, 6)]),
        ("default-factorial", pilot, [g["raw_sha256"]["measured"] for g in json.loads(score)["groups"]]),
    ]
    result = {}
    for name, series, groups in sources:
        hz = json.loads((series / "series.json").read_bytes())["config"]["clock_ticks"]
        records = []
        for group, references in enumerate(groups, 1):
            for label, digest in references.items():
                raw, = series.glob(f"blocks/{group:02d}-large.*/measured-{label}/*/raw.tar.gz")
                records.append(dict(group=group, lane=label, **window(raw, digest, hz)))
        result[name] = records
    return result


if __name__ == "__main__":
    print(json.dumps(run(Path(sys.argv[1])), indent=2, sort_keys=True))
