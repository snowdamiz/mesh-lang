"""Inspect consecutive claim stamps in released one-runner SLOW ledgers."""
from collections import Counter, defaultdict
import hashlib
import json
from pathlib import Path
import tarfile

RESULTS = Path(__file__).resolve().parents[1]
STOCK = RESULTS / "2026-11-24-trusted-jobs-product-composition/evidence/native/timing-closure-retry/slow"
RECENT = RESULTS / "2026-11-25-public-slow-file-topology/roster-retry/evidence/native/pilot"
PRICES = RESULTS / "2026-11-25-public-slow-hot-checkout-price/evidence/prices.json"


def independent_pairs(weights):
    """Best sum across adjacent pairs that share no grant stamp."""
    previous, best = 0, 0
    for weight in weights:
        previous, best = best, max(best, previous + weight)
    return best


def measure(path, digest):
    assert hashlib.sha256(path.read_bytes()).hexdigest() == digest
    with tarfile.open(path) as raw:
        def member(name):
            return json.load(raw.extractfile(name))
        assert member("status.json")["exit_code"] == 0
        ledger = member("adapter/ledger.json")
        policy = member("adapter/queue-policy.json")
    assert ledger["runners"] == 1 and ledger["slots"] == 64 and ledger["database"]["max_connections"] == 68
    assert ledger["phase"] == "measured" and ledger["queue_policy"] == "set_based"
    jobs = ledger["jobs"]
    assert len(jobs) == len({j["id"] for j in jobs}) == len({j["settled_xid"] for j in jobs}) == 512
    assert len({j[k] for j in jobs for k in ("attempt_xid", "effect_xid")}) == 1024
    assert len({j["worker_pid"] for j in jobs}) == 1 and policy["terminal_xids"] == 512
    enqueue, = ledger["transactions"]
    cohorts = defaultdict(list)
    for j in jobs:
        stamp = j["claimed_us"]
        assert stamp % 1000 == 0
        assert j["attempt"] == j["attempts"] == j["effects"] == 1 and j["hold_ms"] == 100
        assert enqueue["committed_us"] <= stamp + 999 and stamp <= j["entered_us"] <= j["attempt_committed_us"]
        assert j["attempt_committed_us"] + 100000 <= j["effect_committed_us"] <= j["settled_us"]
        cohorts[stamp].append(j)
    groups = sorted(cohorts.items())
    widths = Counter(len(group) for _, group in groups)
    assert max(widths) == 16
    pairs = []
    for (stamp, group), (later, following) in zip(groups, groups[1:]):
        # Count earlier grants even when their settlement happened later in this millisecond.
        in_use = sum(j["claimed_us"] < stamp and j["settled_us"] >= stamp for j in jobs)
        headroom = max(0, 64 - in_use - len(group))
        seats = min(headroom, len(following)) if len(group) == 16 else 0
        pairs.append(dict(stamp=stamp, gap_us=later - stamp, seats=seats,
                          whole=seats == len(following), width=len(group)))
    weighted = [r["seats"] * max(0, r["gap_us"] - 1000) for r in pairs]
    first = [dict(width=len(group), stamp_us=stamp-enqueue["committed_us"],
                  first_entry_us=min(j["entered_us"] for j in group)-enqueue["committed_us"])
             for stamp, group in groups[:4]]
    return dict(raw_sha256=digest, stamped_groups=len(groups), width_counts=dict(sorted(widths.items())),
                first_four=first, full_with_room_and_following=sum(r["seats"] > 0 for r in pairs),
                fully_absorbable_disjoint_pairs=independent_pairs([int(r["whole"]) for r in pairs]),
                disjoint_normalized_slot_us=independent_pairs(weighted) / 64)


def replay():
    prices = json.loads(PRICES.read_bytes())
    rows = {}
    for r in prices["canonical_slow"]:
        key = r["group"]
        path, = STOCK.glob(f"blocks/{key:02}-*/measured-product/*/raw.tar.gz")
        rows[f"stock-{key}"] = measure(path, r["raw_sha256"])
    for r in prices["recent_cold_pilot"]:
        for lane, score in r["measured"].items():
            key = r["group"]
            path, = RECENT.glob(f"blocks/{key:02}-*/measured-{lane}/*/raw.tar.gz")
            rows[f"recent-{key}-{lane}"] = measure(path, score["raw_sha256"])
    return rows


if __name__ == "__main__":
    print(json.dumps(replay(), sort_keys=True, indent=2))
