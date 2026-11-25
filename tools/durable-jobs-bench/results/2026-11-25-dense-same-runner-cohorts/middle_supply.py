"""Integrate per-runner ledger states between durable effects 256 and 768."""
import json
from collections import defaultdict

from cross_runner import EVIDENCE, adjacent_pairs, load_ledger, transactions


def handoff_pressure(jobs):
    events, cutoff = transactions(jobs)
    pairs = adjacent_pairs(events, cutoff, span=1000, lag=250)
    per_owner = defaultdict(list)
    for job in jobs:
        per_owner[int(job["worker_pid"])].append(job)
    for rows in per_owner.values():
        for stamp in {j["claimed_us"] for j in rows}:
            assert sum(j["claimed_us"] <= stamp and j["settled_us"] > stamp + 999
                       for j in rows) <= 8
    result = {}
    for delay in [0, 1000, 4000, 8000, 16000]:
        conflicts = 0
        for left, right in pairs:
            receipt = right.stamp + delay
            exceeds_capacity = False
            for tx in (left, right):
                rows = per_owner[tx.owner]
                grants = {j["claimed_us"] for j in rows
                          if tx.stamp <= j["claimed_us"] + 999 < receipt}
                for stamp in grants:
                    # Give unaffected settlements instant local credit and
                    # allow the latest possible stamp within the millisecond.
                    charged = sum(j["claimed_us"] <= stamp and
                                  (int(j["settled_xid"]) == tx.xid or
                                   j["settled_us"] > stamp + 999) for j in rows)
                    if charged > 8:
                        exceeds_capacity = True
                        break
            conflicts += exceeds_capacity
        result[delay] = conflicts
    return {"pairs": len(pairs), "pairs_over_capacity_at_original_grant": result}


def partition(rows, start, end, rounding):
    changes = defaultdict(lambda: [0] * 4)
    for job in rows:
        entry = job["entered_us"]
        claim = min(entry, job["claimed_us"] + rounding)
        assert job["claimed_us"] <= entry
        times = [claim, entry, job["attempt_committed_us"],
                 job["effect_committed_us"], job["settled_us"]]
        assert times == sorted(times)
        for i, (left, right) in enumerate(zip(times, times[1:])):
            left, right = max(left, start), min(right, end)
            if left < right:
                changes[left][i] += 1
                changes[right][i] -= 1
    counts, area = [0] * 4, [0] * 4
    empty = only_terminal = 0
    previous = start
    for stamp in sorted(set(changes) | {end}):
        dt = stamp - previous
        for i, n in enumerate(counts):
            area[i] += n * dt
        if sum(counts) == 0:
            empty += dt
        elif sum(counts[:3]) == 0:
            only_terminal += dt
        counts = [a + b for a, b in zip(counts, changes[stamp])]
        assert min(counts) >= 0
        previous = stamp
    assert counts == [0] * 4
    return area, empty, only_terminal


def measure(jobs):
    effects = sorted(j["effect_committed_us"] for j in jobs)
    start, end = effects[255], effects[767]
    owners = defaultdict(list)
    for j in jobs:
        owners[j["worker_pid"]].append(j)
    assert len(owners) == 8
    result = {"middle_us": end - start}
    for rounding in [0, 999]:
        parts = [partition(rows, start, end, rounding) for rows in owners.values()]
        result[f"stamp_plus_{rounding}_us"] = {
            "mean_jobs_by_stage": dict(zip(
                ["stamp_to_entry", "entry_to_first", "first_to_effect", "effect_to_terminal"],
                [sum(row) / (end - start) for row in zip(*(part[0] for part in parts))])),
            "mean_runners_with_no_stamped_unsettled_jobs": sum(p[1] for p in parts) / (end - start),
            "mean_runners_with_only_effected_unsettled_jobs": sum(p[2] for p in parts) / (end - start),
        }
    return result


def main():
    release = json.loads((EVIDENCE / "controller-release.json").read_text())
    assert release["exit_status"] == 0
    seals = json.loads((EVIDENCE / "summary.json").read_text())["raw_sha256_by_rotation"]
    result = {}
    for group, arms in seals.items():
        result[group] = {}
        for lane in ["B", "A1", "A2", "reference"]:
            arm = "measured-" + lane
            path, = EVIDENCE.glob(f"blocks/{int(group):02}-*/{arm}/*/raw.tar.gz")
            ledger, _ = load_ledger(path, arms[arm])
            result[group][lane] = dict(sha256=arms[arm], **measure(ledger["jobs"]))
            if lane == "B":
                result[group][lane]["paired_receipt_delay_us"] = handoff_pressure(ledger["jobs"])
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
