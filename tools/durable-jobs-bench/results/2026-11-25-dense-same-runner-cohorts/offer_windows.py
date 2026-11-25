"""Apply the implemented leader/follower timing rule to the existing modeled pairs."""
import json
from argparse import ArgumentParser
from collections import defaultdict
from pathlib import Path

from cross_runner import EVIDENCE, adjacent_pairs, load_ledger, transactions


def arm(jobs, include_sql=False):
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
            mix = {delay: [0, 0, 0] for delay in hits}
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
                            if include_sql:
                                mix[delay][int(a.width == 1) + int(b.width == 1)] += 1
                            break
            results.append(dict(ready_lag_us=ready_lag, receipt_lag_us=receipt_lag,
                                consistent_with_both_stock_xid_upper_bounds=consistent,
                                pairs_with_open_offer_by_election_delay_us=hits))
            if include_sql:
                results[-1]["pairs_by_scalar_count"] = mix
    return dict(original_pairs=len(pairs), sensitivities=results)


def sql_price(ledger, queue, stock, sensitivities):
    events, _ = transactions(ledger["jobs"])
    scalar, = [s for s in queue["queue_statements"] if s["query"].startswith("SELECT coalesce(mesh_jobs.settle_state")]
    batch, = [s for s in queue["queue_statements"] if s["query"].startswith("SELECT ordinal::text FROM mesh_jobs.complete_batch_v1")]
    assert scalar["calls"] == sum(e.width == 1 for e in events)
    assert batch["calls"] == len(events) - scalar["calls"] and batch["rows"] + scalar["calls"] == 1024
    scalar_ms = scalar["execution_ms"] / scalar["calls"]
    batch_ms = batch["execution_ms"] / batch["calls"]
    row, = [r for r in sensitivities if r["ready_lag_us"] == r["receipt_lag_us"] == 250]
    prices = {}
    for delay, counts in row["pairs_by_scalar_count"].items():
        pairs = sum(counts)
        assert pairs == row["pairs_with_open_offer_by_election_delay_us"][delay]
        scalar_calls = counts[1] + 2 * counts[2]
        old_ms = scalar_calls * scalar_ms + (2 * pairs - scalar_calls) * batch_ms
        prices[delay] = dict(pairs=pairs, batch_batch=counts[0], scalar_batch=counts[1],
                             scalar_scalar=counts[2], original_calls_at_class_means_ms=old_ms,
                             difference_allowing_one_mean_batch_per_pair_ms=old_ms - pairs * batch_ms)
    return dict(raw_sha256=stock["raw_sha256"], scalar_mean_ms=scalar_ms, batch_mean_ms=batch_ms,
                river_submit_gap_ms=stock["matched_peers"]["river"]["mesh_minus_peer_us"]["submit"] / 1000,
                by_election_delay_us=prices)


def main(include_sql):
    reference = json.loads(Path(__file__).with_name("cross-runner.json").read_text())
    results = {}
    for group, stock in reference.items():
        path, = EVIDENCE.glob(f"blocks/{int(group):02}-*/measured-B/*/raw.tar.gz")
        ledger, queue = load_ledger(path, stock["raw_sha256"], with_queue=include_sql)
        windows = arm(ledger["jobs"], include_sql)
        if include_sql:
            results[group] = sql_price(ledger, queue, stock, windows["sensitivities"])
        else:
            results[group] = dict(sha256=stock["raw_sha256"], **windows)
    print(json.dumps(results, indent=2))


if __name__ == "__main__":
    parser = ArgumentParser(description=__doc__)
    parser.add_argument("--sql", action="store_true", help="Price intact pairs at the two observed SQL class means")
    main(parser.parse_args().sql)
