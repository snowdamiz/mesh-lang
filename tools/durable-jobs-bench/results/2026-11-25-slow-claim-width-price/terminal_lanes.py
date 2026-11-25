"""Replay scalar terminal service and seat-limited refill on the released SLOW schedule."""
from collections import defaultdict
from hashlib import sha256
import json
import tarfile

from slow_middle import STOCK, replay as check_archive

SLOTS = 64


def project(rows, lanes, penalty, receipt_lag, exit_lag, latest):
    # Reconstruct each grant from the reported millisecond, observed entries and
    # already committed seats. Preserve waiting after the seat frontier.
    by_stamp = defaultdict(list)
    for row in rows:
        by_stamp[row["claimed_us"]].append(row)
    observed = []
    grants = []
    for stamp, batch in sorted(by_stamp.items()):
        needed = len(observed) + len(batch) - SLOTS
        previous = grants[-1][0] if grants else stamp
        frontier = max(previous, sorted(observed)[needed - 1] + receipt_lag) if needed > 0 else previous
        low = max(stamp, frontier)
        high = min(stamp + 999, min(j["entered_us"] for j in batch))
        if low > high:
            return None
        at = high if latest else low
        grants.append((at, at - frontier if grants else 0, batch))
        observed.extend(j["settled_us"] for j in batch)

    # Attribute the entire residual after readiness and preceding acknowledgement
    # to the serial service. This favors overlap when some of it was dispatcher work.
    ordered = sorted(rows, key=lambda row: row["settled_us"])
    service = []
    prior_ack = 0
    for j in ordered:
        ready = j["effect_committed_us"] + exit_lag
        duration = j["settled_us"] - max(ready, prior_ack)
        if duration < 0:
            return None
        service.append(duration)
        prior_ack = j["settled_us"] + receipt_lag

    next_grant = next_send = 0
    grant_at = grants[0][0]
    lane_free = [0] * lanes
    first_available_send = 0
    effects = {}
    commits = []
    while next_send < len(rows):
        pending_grant = None
        if next_grant < len(grants):
            observed_at, slack, batch = grants[next_grant]
            needed = len(effects) + len(batch) - SLOTS
            if needed <= len(commits):
                capacity_at = sorted(commits)[needed - 1] + receipt_lag if needed > 0 else grant_at
                pending_grant = max(capacity_at, grant_at) + slack
        j = ordered[next_send]
        lane = min(range(lanes), key=lane_free.__getitem__)
        send = None
        if j["id"] in effects:
            send = max(first_available_send, lane_free[lane], effects[j["id"]] + exit_lag)
        if pending_grant is not None and (send is None or pending_grant <= send):
            grant_at = pending_grant
            assert len(effects) + len(batch) - sum(t + receipt_lag <= grant_at for t in commits) <= SLOTS
            if lanes == 1 and penalty == 0:
                assert grant_at == observed_at
            for item in batch:
                effects[item["id"]] = grant_at + item["effect_committed_us"] - observed_at
            next_grant += 1
        else:
            assert send is not None
            commit = send + service[next_send] + penalty
            if lanes == 1 and penalty == 0:
                assert commit == j["settled_us"] and effects[j["id"]] == j["effect_committed_us"]
            commits.append(commit)
            lane_free[lane] = commit + receipt_lag
            first_available_send = send
            next_send += 1
    assert len(effects) == len(commits) == len(rows)
    return dict(last_terminal=max(commits), last_effect=max(effects.values()))


def replay():
    result = []
    for number, comparison in enumerate(check_archive(), 1):
        path, = STOCK.glob(f"blocks/{number:02}-*/measured-product/*/raw.tar.gz")
        assert sha256(path.read_bytes()).hexdigest() == comparison["product"]["raw_sha256"]
        with tarfile.open(path) as ar:
            ledger = json.load(ar.extractfile("adapter/ledger.json"))
            policy = json.load(ar.extractfile("adapter/queue-policy.json"))
        rows = ledger["jobs"]
        assert policy["terminal_xids"] == len({j["settled_us"] for j in rows}) == len(rows) == 512
        assert policy["completion_cohort_queries"] == 0
        loss = sum(comparison["product"]["elapsed"].values()) - sum(comparison["river"]["elapsed"].values())
        models = []
        for exit_lag, ack, extra in ((0, 0, 0), (0, 0, 100), (250, 100, 100), (250, 100, 250)):
            for latest in (False, True):
                baseline = project(rows, 1, 0, ack, exit_lag, latest)
                candidate = project(rows, 2, extra, ack, exit_lag, latest)
                saving = None if baseline is None else {k: baseline[k] - candidate[k] for k in baseline}
                models.append(dict(exit_us=exit_lag, receipt_us=ack, extra_us=extra, latest=latest,
                                   savings_us=saving))
        result.append(dict(group=number, mesh_sha256=comparison["product"]["raw_sha256"],
                           river_sha256=comparison["river"]["raw_sha256"], submit_loss_us=loss, models=models))
    return result


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2, sort_keys=True))
