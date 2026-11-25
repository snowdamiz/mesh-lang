"""Apply the implemented leader/follower timing rule to the existing modeled pairs."""
import json
from collections import defaultdict
from pathlib import Path

from cross_runner import EVIDENCE, adjacent_pairs, load_ledger, transactions


def arm(jobs):
    events, xid_upper = transactions(jobs)
    pairs = adjacent_pairs(events, xid_upper, span=1000, lag=250)
    prior = {}
    previous = {}
    for tx in sorted(events, key=lambda t: t.xid):
        previous[tx.xid] = prior.get(tx.owner, 0)
        prior[tx.owner] = tx.stamp
    effects = defaultdict(list)
    for job in jobs:
        effects[int(job["settled_xid"])].append(job["effect_committed_us"])

    def window(tx, ready_lag, receipt_lag):
        stamps = effects[tx.xid]
        assert len(stamps) == tx.width < 8
        available = max(max(stamps) + ready_lag, previous[tx.xid] + receipt_lag)
        deadline = min(stamps) + ready_lag + 8000
        return available, deadline

    results = []
    for ready_lag in [0, 250, 1000, 4000]:
        for receipt_lag in [250, 1000]:
            consistent = 0
            hits = {0: 0, 250: 0, 1000: 0}
            for a, b in pairs:
                aw = window(a, ready_lag, receipt_lag)
                bw = window(b, ready_lag, receipt_lag)
                # Under this schedule, each original partial cohort could only
                # have been sent at/after its normal deadline and availability.
                if max(aw) > xid_upper[a.xid] or max(bw) > xid_upper[b.xid]:
                    continue
                consistent += 1
                for delay in hits:
                    for leader, follower, tx in [(aw, bw, a), (bw, aw, b)]:
                        election = max(leader) + delay
                        if follower[0] <= election < follower[1] and election <= xid_upper[tx.xid]:
                            hits[delay] += 1
                            break
            results.append(dict(ready_lag_us=ready_lag, receipt_lag_us=receipt_lag,
                                consistent_with_both_stock_xid_upper_bounds=consistent,
                                pairs_with_open_offer_by_election_delay_us=hits))
    return dict(original_pairs=len(pairs), sensitivities=results)


def main():
    reference = json.loads(Path(__file__).with_name("cross-runner.json").read_text())
    results = {}
    for group, stock in reference.items():
        path, = EVIDENCE.glob(f"blocks/{int(group):02}-*/measured-B/*/raw.tar.gz")
        ledger, _ = load_ledger(path, stock["raw_sha256"])
        results[group] = dict(sha256=stock["raw_sha256"], **arm(ledger["jobs"]))
    print(json.dumps(results, indent=2))


if __name__ == "__main__":
    main()
