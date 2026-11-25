"""Read the released 2x32 Mesh and River ledgers and emit paired phase times."""

import hashlib
import json
import tarfile
from pathlib import Path


EVIDENCE = Path(__file__).resolve().parents[1]


def read_arm(path, raw_sha, engine):
    if hashlib.sha256(path.read_bytes()).hexdigest() != raw_sha:
        raise ValueError(f"Archive digest mismatch: {path}")
    with tarfile.open(path) as archive:
        ledger = json.load(archive.extractfile("adapter/ledger.json"))
        roster = None
        if engine == "go":
            samples = [json.loads(line) for line in archive.extractfile(
                "adapter/river/measured/samples.jsonl")]
            idle = [sample["connections"] for sample in samples if sample["idle"]]
            roster = dict(idle_min=min(idle), idle_max=max(idle),
                          peak=max(sample["connections"] for sample in samples))

    transaction, = ledger["transactions"]
    jobs = ledger["jobs"]
    if len(jobs) != 512 or any(job["hold_ms"] != 100 for job in jobs):
        raise ValueError(f"Unexpected SLOW workload: {path}")
    first = sorted(job["attempt_committed_us"] for job in jobs)
    low, high = first[63], first[447]
    last_effect = max(job["effect_committed_us"] for job in jobs)
    last_settled = max(job["settled_us"] for job in jobs)
    marks = [transaction["submitted_us"], transaction["committed_us"], low,
             high, last_effect, last_settled]
    phases = dict(zip(("enqueue", "commit_to_first_64", "first_64_to_first_448",
                       "first_448_to_last_effect", "terminal_tail"),
                      (end - start for start, end in zip(marks, marks[1:]))))
    held = sum(max(0, min(high, job["effect_committed_us"]) -
                   max(low, job["attempt_committed_us"])) for job in jobs)
    if not 0 <= held <= 64 * (high - low):
        raise ValueError(f"Committed hold exposure exceeds physical capacity: {path}")
    return dict(raw_sha256=raw_sha, phase_us=phases,
                middle_hold_slot_us=held,
                middle_unoccupied_hold_slot_us=64 * (high - low) - held,
                river_connections=roster)


def main():
    scores = json.loads((EVIDENCE / "native-analysis/timing-replay.json").read_bytes())
    results = {}
    for block, lanes in sorted(scores["qualified"].items()):
        rows = {}
        for engine in ("mesh", "go"):
            lane = "measured-" + engine
            path = EVIDENCE / "native/sql-retry-timing/blocks" / block / lane / lane / "raw.tar.gz"
            rows[engine] = read_arm(path, lanes[lane]["raw_sha256"], engine)
        differences = {key: value - rows["mesh"]["phase_us"][key]
                       for key, value in rows["go"]["phase_us"].items()}
        results[block] = dict(arms=rows, river_minus_mesh_us=differences)
    print(json.dumps(results, indent=2))


if __name__ == "__main__":
    main()
