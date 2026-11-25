"""Price earlier handler entry on the released R2 D2 ledgers."""
import hashlib
import json
from itertools import groupby
from pathlib import Path
import tarfile

REFERENCE = Path(__file__).resolve().parent.parent / "2026-11-24-short16-river-readiness"


def read_ledger(group, lane):
    member = f"measured-{lane}/measured-{'go' if lane == 'river-warm' else 'mesh'}"
    path = REFERENCE / "evidence/timing/blocks" / group["attempt"] / member / "raw.tar.gz"
    digest = hashlib.sha256(path.read_bytes()).hexdigest()
    if digest != group["raw_sha256"][member]:
        raise ValueError(f"archive hash mismatch: {path}")
    with tarfile.open(path) as raw:
        return json.load(raw.extractfile("adapter/ledger.json")), digest


def clocks(effects, ends, tx):
    ordered = sorted(effects)
    return {
        "submit": max(ends) - tx["submitted_us"],
        "drain": max(ends) - tx["committed_us"],
        "middle112": ordered[-1] - ordered[15],
    }


def observed(ledger):
    jobs = ledger["jobs"]
    tx, = ledger["transactions"]
    if len(jobs) != 128 or any(job["hold_ms"] != 1 for job in jobs):
        raise ValueError("expected SHORT16")
    return clocks([job["effect_committed_us"] for job in jobs],
                  [job["settled_us"] for job in jobs], tx)


def earlier_entry(ledger, late, entry_floor):
    jobs = sorted(ledger["jobs"], key=lambda job: job["claimed_us"])
    tx, = ledger["transactions"]
    previous = predicted_previous = 0
    original_ends, ends, effects = [], [], []
    for stamp, rows in groupby(jobs, key=lambda job: job["claimed_us"]):
        batch = list(rows)
        first_entry = min(job["entered_us"] for job in batch)
        needed = len(ends) + len(batch) - 16
        original_open = sorted(original_ends)[needed - 1] if needed > 0 else 0
        predicted_open = sorted(ends)[needed - 1] if needed > 0 else 0
        lower = max(stamp, previous, original_open)
        upper = min(stamp + 999, first_entry)
        if lower > upper or len(batch) > 16:
            raise ValueError("infeasible observed stamp cohort")
        original = upper if late else lower
        # Preserve the lag after both the previous grant and enough terminal commits.
        lag = original - max(previous, original_open)
        predicted = max(predicted_previous, predicted_open) + lag
        for job in batch:
            floor = {"same_stamp_first_entry": first_entry, "claim_stamp": original,
                     "unchanged": job["entered_us"]}[entry_floor]
            advance = original - predicted + job["entered_us"] - floor
            original_ends.append(job["settled_us"])
            ends.append(job["settled_us"] - advance)
            effects.append(job["effect_committed_us"] - advance)
        previous, predicted_previous = original, predicted
    return clocks(effects, ends, tx)


def main():
    summary = json.loads((REFERENCE / "evidence/summary.json").read_bytes())
    result = []
    for group in summary["rotations"]:
        mesh, mesh_sha = read_ledger(group, "r2-d2")
        river, river_sha = read_ledger(group, "river-warm")
        stock, peer = observed(mesh), observed(river)
        cases = {}
        for late in (False, True):
            if earlier_entry(mesh, late, "unchanged") != stock:
                raise ValueError("zero-change clocks do not match")
            for floor in ("same_stamp_first_entry", "claim_stamp"):
                changed = earlier_entry(mesh, late, floor)
                cases[f"{floor}_{'late' if late else 'early'}"] = {
                    "saved_us": {k: stock[k] - changed[k] for k in stock},
                    "minus_river_us": {k: changed[k] - peer[k] for k in stock},
                }
        result.append({"group": group["group"], "mesh_sha256": mesh_sha,
                       "river_sha256": river_sha, "observed_mesh_us": stock,
                       "observed_river_us": peer, "scenarios": cases})
    if len(result) != 6:
        raise ValueError("expected six rotations")
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
