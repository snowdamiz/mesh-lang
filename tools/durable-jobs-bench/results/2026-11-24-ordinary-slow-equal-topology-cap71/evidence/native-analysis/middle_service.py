"""Account for 64 seats between the 64th and 448th first application commits."""

import hashlib
import json
import tarfile
from pathlib import Path


EVIDENCE = Path(__file__).resolve().parents[1]


def middle_service(jobs):
    first = sorted(job["attempt_committed_us"] for job in jobs)
    if len(first) != 512:
        raise ValueError("Expected 512 completed SLOW jobs")
    start, end = first[63], first[447]
    capacity = 64 * (end - start)

    def overlap(begin, finish):
        return max(0, min(finish, end) - max(begin, start))

    entry = required = excess = active = 0
    events = []
    for job in jobs:
        entered = job["entered_us"]
        committed = job["attempt_committed_us"]
        earliest_effect = committed + 100_000
        effect = job["effect_committed_us"]
        if job["hold_ms"] != 100 or not entered <= committed < earliest_effect <= effect:
            raise ValueError("Job timestamps violate the SLOW contract")
        entry += overlap(entered, committed)
        required += overlap(committed, earliest_effect)
        excess += overlap(earliest_effect, effect)
        active += overlap(entered, effect)
        events.extend(((entered, 1), (effect, -1)))
    seats = 0
    for _, delta in sorted(events):
        seats += delta
        if not 0 <= seats <= 64:
            raise ValueError("Entry-to-effect occupancy exceeds the physical slots")
    if entry + required + excess != active or active > capacity:
        raise ValueError("Middle window capacity accounting failed")
    return dict(window_us=end - start, entry_to_first_slot_us=entry,
                first_100ms_slot_us=required, after_100ms_to_effect_slot_us=excess,
                outside_entry_to_effect_slot_us=capacity - active)


def main():
    release = json.loads((EVIDENCE / "native-analysis/timing-replay.json").read_bytes())
    result = {}
    for block, lanes in sorted(release["qualified"].items()):
        result[block] = {}
        for engine in ("mesh", "go", "oban", "rust"):
            lane = "measured-" + engine
            archive = EVIDENCE / "native/sql-retry-timing/blocks" / block / lane / lane / "raw.tar.gz"
            with archive.open("rb") as stream:
                digest = hashlib.file_digest(stream, "sha256").hexdigest()
            if digest != lanes[lane]["raw_sha256"]:
                raise ValueError(f"Release digest mismatch for {archive}")
            with tarfile.open(archive) as raw:
                jobs = json.load(raw.extractfile("adapter/ledger.json"))["jobs"]
            result[block][engine] = dict(raw_sha256=digest, **middle_service(jobs))
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
