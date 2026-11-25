"""Split wall time by occupied slot time on the canonical four-engine SLOW workload."""
import hashlib
import json
import tarfile
from collections import Counter

from replay import STOCK


def budget(path, sha):
    assert hashlib.sha256(path.read_bytes()).hexdigest() == sha
    with tarfile.open(path) as raw:
        assert json.load(raw.extractfile("status.json"))["exit_code"] == 0
        ledger = json.load(raw.extractfile("adapter/ledger.json"))
    assert ledger["slots"] == ledger["peak_executing"] == 64 and ledger["runners"] == 1
    jobs = ledger["jobs"]
    tx, = ledger["transactions"]
    assert len(jobs) == len({j["id"] for j in jobs}) == 512
    assert len({j[k] for j in jobs for k in ("effect_xid", "attempt_xid")}) == 1024
    last = max(j["settled_us"] for j in jobs)
    drain = last - tx["committed_us"]
    changes = Counter()
    entry_work = 0
    hold_excess = 0
    for j in jobs:
        entered, first, second = (j[k] for k in ("entered_us", "attempt_committed_us", "effect_committed_us"))
        assert tx["committed_us"] <= entered <= first <= second <= j["settled_us"]
        assert j["state"] == "completed" and j["effects"] == j["attempts"] == 1
        assert j["hold_ms"] == 100 and second - first >= 100_000
        changes[entered] += 1
        changes[second] -= 1
        entry_work += first - entered
        hold_excess += second - first - 100_000
    active = 0
    previous = tx["committed_us"]
    outside = 0
    for stamp, step in sorted(changes.items()):
        outside += (64 - active) * (stamp - previous)
        active += step
        assert 0 <= active <= 64
        previous = stamp
    assert active == 0
    outside += 64 * (last - previous)
    hold = len(jobs) * 100_000
    assert outside + entry_work + hold + hold_excess == 64 * drain
    result = dict(raw_sha256=sha, submit_to_enqueue_us=tx["committed_us"]-tx["submitted_us"],
                  drain_us=drain, normalized_entry_to_first_us=entry_work/64,
                  normalized_required_hold_us=hold/64, normalized_between_writes_excess_us=hold_excess/64,
                  normalized_outside_entry_to_second_us=outside/64)
    if ledger["engine"] == "mesh":
        claim_to_entry = sum(j["entered_us"]-j["claimed_us"] for j in jobs)
        effect_to_terminal = sum(j["settled_us"]-j["effect_committed_us"] for j in jobs)
        assert all(j["claimed_us"] % 1000 == 0 and j["claimed_us"] <= j["entered_us"] for j in jobs)
        result.update(normalized_claim_stamp_to_entry_us=claim_to_entry/64,
                      normalized_second_to_terminal_us=effect_to_terminal/64,
                      normalized_outside_claim_stamp_to_terminal_us=(outside-claim_to_entry-effect_to_terminal)/64)
    return result


def replay():
    data = (STOCK / "summary.json").read_bytes()
    assert hashlib.sha256(data).hexdigest() == "763b9cb39bec01492a9cf6ab6240e5758ef90573345c6571fd5e2fb851d3eaa8"
    out = {}
    for i, group in enumerate(json.loads(data)["groups"], 1):
        out[i] = {}
        for lane, row in group.items():
            path, = STOCK.glob(f"blocks/{i:02}-*/measured-{lane}/*/raw.tar.gz")
            out[i][lane] = budget(path, row["raw_sha256"])
    return out


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2, sort_keys=True))
