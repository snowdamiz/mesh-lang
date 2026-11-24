"""Replay instantaneous refill while retaining each job's measured post-claim duration."""
from hashlib import sha256
from itertools import groupby
import json
from pathlib import Path
import tarfile

HERE = Path(__file__).resolve().parent


def simulate(jobs, slots, latest):
    observed, ideal = [], []
    previous = None
    previous_ideal = None
    for stamp, batch in groupby(sorted(jobs, key=lambda row: row["claimed_us"]), key=lambda row: row["claimed_us"]):
        batch = list(batch)
        assert len(batch) <= slots
        limit = min(stamp + 999, min(row["entered_us"] for row in batch))
        required = len(observed) + len(batch) - slots
        if previous is None:
            assert len(batch) == slots
            previous = stamp
        recorded = max(previous, stamp, sorted(observed)[required - 1] if required > 0 else stamp)
        assert recorded <= limit
        if latest:
            recorded = limit
        if previous_ideal is None:
            previous_ideal = recorded
        earliest = previous_ideal
        if required > 0:
            earliest = max(earliest, sorted(ideal)[required - 1])
        assert sum(end > earliest for end in ideal) + len(batch) <= slots
        for job in batch:
            assert recorded <= job["entered_us"] <= job["effect_committed_us"] <= job["settled_us"]
            observed.append(job["settled_us"])
            ideal.append(earliest + job["settled_us"] - recorded)
        previous_ideal = earliest
        previous = recorded
    assert max(ideal) <= max(observed)
    return max(ideal)


def main():
    summary = json.loads((HERE / "evidence/summary.json").read_bytes())
    assert not summary["missing"] and len(summary["rotations"]) == 6
    rows = []
    for group in summary["rotations"]:
        sub = "measured-r2-d2/measured-mesh"
        path = HERE / "evidence/timing/blocks" / group["attempt"] / sub / "raw.tar.gz"
        assert sha256(path.read_bytes()).hexdigest() == group["raw_sha256"][sub]
        with tarfile.open(path) as ar:
            data = json.load(ar.extractfile("adapter/ledger.json"))
        assert data["slots"] == 16 and len(data["jobs"]) == 128
        tx, = data["transactions"]
        end = max(job["settled_us"] for job in data["jobs"])
        predicted = [simulate(data["jobs"], 16, latest) for latest in (False, True)]
        peers = {}
        for label in ["river-warm", "river", "oban", "apalis"]:
            peer = group["lanes"][label]
            peers[label] = {metric: [(stamp-tx[start])/1000 - peer[metric] for stamp in predicted]
                            for metric,start in [("submit_to_settlement_ms","submitted_us"),
                                                 ("commit_to_settlement_ms","committed_us")]}
        rows.append(dict(group=group["group"], savings_ms=[(end-stamp)/1000 for stamp in predicted], peer_delta_ms=peers))
    print(json.dumps(rows, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
