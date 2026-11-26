"""Price a 32-row ceiling at the original Direct4 16-row grant boundaries."""
import json
from collections import defaultdict

from direct4_idle_budget import blob
from direct4_peer_deadlines import claim_room, ledger


def assess(raw, digest):
    jobs, txn = ledger(raw, digest)
    stock = claim_room(raw, jobs)
    batches = defaultdict(list)
    for job in jobs:
        batches[job["claimed_us"], job["worker_pid"]].append(job)
    assert len(batches) == stock["positive_grants"]
    order = sorted(batches)
    first_entry = {key: min(j["entered_us"] for j in rows) for key, rows in batches.items()}
    effects = sorted(job["effect_committed_us"] for job in jobs)
    rows = []
    for index, key in enumerate(order):
        stamp, pid = key
        if len(batches[key]) != 16:
            continue
        # Give the owner every visible commit through the end of the stamp bin.
        # Preserve the other owner's grants in that same bin.
        live = [j for j in jobs if j["claimed_us"] < stamp and j["settled_us"] >= stamp + 1000]
        simultaneous = sum(len(items) for (s, p), items in batches.items() if s == stamp and p != pid)
        future_rows = sum(j["claimed_us"] > stamp for j in jobs)
        room = min(16, 32 - 16 - sum(j["worker_pid"] == pid for j in live),
                   64 - 16 - len(live) - simultaneous, future_rows)
        assert 0 <= room <= 16
        row = dict(stamp_from_enqueue_commit_us=stamp-txn["committed_us"], pid=pid,
                   phase="early" if stamp < effects[255] else "late" if stamp > effects[767] else "middle",
                   extra_rows_ceiling=room)
        rows.append(row)
        following = next((k for k in order[index + 1:] if k[1] == pid), None)
        if following is None or room < len(batches[following]):
            continue
        begin, end = first_entry[key], first_entry[following]
        assert begin < end
        known = [j for k, batch in batches.items() if k[1] == pid and k[0] <= stamp
                 and first_entry[k] <= begin for j in batch]
        # Only grants already proved received at the first entry of the leader.
        # Each subsequent durable effect retires one job from that known set.
        stops = sorted((j["effect_committed_us"] for j in known), reverse=True)
        assert len(stops) >= 16
        peer_between = any(stamp <= s <= following[0] and p != pid for s, p in order)
        row.update(next_width=len(batches[following]), next_stamp_gap_us=following[0]-stamp,
                   another_owner_inclusive=peer_between, entry_interval_us=end-begin,
                   below_four_effects_outstanding_us=max(0, end-max(begin, stops[3])),
                   zero_effects_outstanding_us=max(0, end-max(begin, stops[0])))
    assert len(rows) == stock["width16"]
    return rows


def replay():
    result = {}
    for group in (1, 2):
        path = f"blocks/{group:02}-noop"
        score = json.loads(blob(f"{path}/reconciled.json"))["measured"]["direct4"]
        raw = blob(f"{path}/measured-direct4/measured-mesh/raw.tar.gz")
        result[group] = dict(raw_sha256=score["raw_sha256"], full_grants=assess(raw, score["raw_sha256"]))
    return result


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2))
