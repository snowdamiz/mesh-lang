"""Count distinct older terminal commits strictly inside each SLOW claim-to-entry interval."""
from collections import Counter, defaultdict
from hashlib import sha256
import json
import tarfile

from replay import STOCK


def replay():
    summary = (STOCK / "summary.json").read_bytes()
    assert sha256(summary).hexdigest() == "763b9cb39bec01492a9cf6ab6240e5758ef90573345c6571fd5e2fb851d3eaa8"
    results = []
    for number, row in enumerate(json.loads(summary)["groups"], 1):
        raw, = STOCK.glob(f"blocks/{number:02}-*/measured-product/*/raw.tar.gz")
        digest = sha256(raw.read_bytes()).hexdigest()
        assert digest == row["product"]["raw_sha256"]
        with tarfile.open(raw) as f:
            assert json.load(f.extractfile("status.json"))["exit_code"] == 0
            ledger = json.load(f.extractfile("adapter/ledger.json"))
            queue = json.load(f.extractfile("adapter/queue-policy.json"))
        jobs = ledger["jobs"]
        assert ledger["runners"] == 1 and ledger["slots"] == 64
        assert len(jobs) == len({j["settled_xid"] for j in jobs}) == 512
        assert len({j[k] for j in jobs for k in ("attempt_xid", "effect_xid")}) == 1024
        groups = defaultdict(list)
        for j in jobs:
            assert j["state"] == "completed" and j["attempts"] == j["effects"] == 1
            assert j["claimed_us"] % 1000 == 0
            assert j["claimed_us"] <= j["entered_us"] <= j["attempt_committed_us"]
            assert j["attempt_committed_us"] + 100000 <= j["effect_committed_us"] <= j["settled_us"]
            groups[j["claimed_us"]].append(j)
        calls, = [s for s in queue["queue_statements"] if "FROM mesh_jobs.claim_batch" in s["query"]]
        positive_upper = calls["calls"] - (calls["rows"] - len(jobs))
        assert len(groups) <= positive_upper
        seen = set()
        windows = []
        for stamp, group in sorted(groups.items()):
            first = min(j["entered_us"] for j in group)
            older = [j for j in jobs if j["claimed_us"] < stamp and stamp + 999 < j["settled_us"] < first]
            seen.update(j["settled_xid"] for j in older)
            times = [j["settled_us"] for j in older]
            windows.append(dict(width=len(group), terminal_count=len(times),
                                terminal_span_us=max(times) - min(times) if times else 0,
                                stamp_end_to_entry_us=max(0, first - stamp - 999)))
        results.append(dict(raw_sha256=digest, stamp_count=len(groups), positive_call_upper=positive_upper,
                            different_older_terminals=len(seen),
                            windows_by_terminal_count=dict(Counter(w["terminal_count"] for w in windows)),
                            most_terminals=max(windows, key=lambda w: w["terminal_count"])))
    assert len(results) == 5
    return results


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2, sort_keys=True))
