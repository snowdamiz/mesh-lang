"""Compare complete durable rank curves from the 16 MiB render SETUP duplicates."""
import hashlib
import json
from pathlib import Path
import sys
import tarfile

RAW_SHA = "5ddc191ae55fa1b2280a2e0850307a1d843fe192027a8712a2cac744fc7b6097"
LANES = ("stock-16a", "stock-16b", "span-16a", "span-16b")
FIELDS = ("claimed_us", "entered_us", "attempt_committed_us", "effect_committed_us", "settled_us")


def read_lane(archive, lane):
    def read(name):
        return json.load(archive.extractfile(f"./{lane}/{name}.json"))

    ledger = read("ledger")
    transaction, = ledger["transactions"]
    start, zero = transaction["submitted_us"], transaction["committed_us"]
    jobs = ledger["jobs"]
    assert ledger["runners"] == 8 and ledger["slots"] == 64
    assert len(jobs) == len(set(transaction["enqueued_ids"].values())) == 64
    assert transaction["enqueued_ids"] == {j["job_key"]: j["id"] for j in jobs}
    assert all(j["state"] == "completed" and j["attempts"] == j["effects"] == 1 for j in jobs)
    for job in jobs:
        milestones = [zero, *(job[field] for field in FIELDS)]
        assert milestones == sorted(milestones)
    assert len({j[f] for j in jobs for f in ("attempt_xid", "effect_xid", "settled_xid")}) == 192
    curves = {f: sorted(j[f] - zero for j in jobs) for f in FIELDS}
    return dict(case_sha256=read("raw")["case_sha256"], input_sha256=ledger["input_sha256"],
                enqueue_us=zero - start, drain_us=curves["settled_us"][-1],
                submit_to_settlement_us=curves["settled_us"][-1] + zero - start), curves


def replay(raw):
    assert hashlib.sha256(raw.read_bytes()).hexdigest() == RAW_SHA
    arms, curves = {}, {}
    with tarfile.open(raw) as archive:
        for lane in LANES:
            arms[lane], curves[lane] = read_lane(archive, lane)
    for key in ("input_sha256", "case_sha256"):
        assert len({a[key] for a in arms.values()}) == 1
    margins = {}
    for field in FIELDS:
        stocks = zip(curves["stock-16a"][field], curves["stock-16b"][field])
        spans = zip(curves["span-16a"][field], curves["span-16b"][field])
        gains = [min(stock) - max(span) for stock, span in zip(stocks, spans)]
        margins[field] = dict(positive_ranks=sum(g > 0 for g in gains),
                              lagging_or_tied_ranks=[i for i, g in enumerate(gains, 1) if g <= 0],
                              min_us=min(gains), max_us=max(gains),
                              gain_us_at_rank={i: gains[i - 1] for i in (1, 8, 16, 32, 48, 56, 64)})
    return dict(raw_sha256=RAW_SHA, arms=arms, faster_stock_minus_slower_span_at_each_rank=margins)


if __name__ == "__main__":
    print(json.dumps(replay(Path(sys.argv[1])), indent=2))
