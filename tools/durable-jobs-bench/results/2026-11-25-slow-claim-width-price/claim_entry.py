"""Reconstruct one-runner grant cohorts from the released SLOW query and job ledgers."""
from collections import defaultdict
from hashlib import sha256
import json
from statistics import median
import tarfile

from replay import PRICES, RECENT, STOCK


def read_arm(path, digest, engine="mesh"):
    assert sha256(path.read_bytes()).hexdigest() == digest
    with tarfile.open(path) as archive:
        def load(member):
            return json.load(archive.extractfile(member))
        assert load("status.json")["exit_code"] == 0
        ledger = load("adapter/ledger.json")
        if engine == "mesh":
            policy = load("adapter/queue-policy.json")
    assert ledger["engine"] == engine
    jobs = ledger["jobs"]
    assert ledger["runners"] == 1 and ledger["slots"] == 64
    assert len(jobs) == len({j["id"] for j in jobs}) == 512
    assert len({j[k] for j in jobs for k in ("attempt_xid", "effect_xid")}) == 1024
    assert all(j["attempts"] == j["effects"] == 1 and j["hold_ms"] == 100 and
               j["effect_committed_us"] - j["attempt_committed_us"] >= 100_000 for j in jobs)
    groups = defaultdict(list)
    for job in jobs:
        stamp = job["claimed_us"]
        assert stamp <= job["entered_us"] <= job["attempt_committed_us"]
        groups[stamp].append(job["entered_us"])
    if engine == "mesh":
        assert ledger["phase"] == "measured" and len({j["settled_xid"] for j in jobs}) == 512
        queries = policy["queue_statements"]
        (grant,) = [q for q in queries if "mesh_jobs.claim_batch_set_v1(" in q["query"]]
        (settle,) = [q for q in queries if "mesh_jobs.settle_state(" in q["query"]]
        assert settle["calls"] == settle["rows"] == policy["terminal_xids"] == 512
        assert not any("cursor" in q["query"].lower() for q in queries)
        # The stock SQL returns one sentinel row on ordinary misses. Agreement
        # also checks the millisecond partition against the SQL call/row counts.
        sentinels = grant["rows"] - len(jobs)
        assert sentinels >= 0 and grant["calls"] - sentinels == len(groups)
        assert max(map(len, groups.values())) == 16
        assert all(stamp % 1000 == 0 for stamp in groups)
        sql = {"additional_query_rows": sentinels, "whole_arm_claim_exec_ms": grant["execution_ms"],
               "whole_arm_settle_exec_ms": settle["execution_ms"]}
    else:
        sql = {}
    common = stagger = 0
    widths = defaultdict(list)
    for stamp, entries in groups.items():
        first = min(entries)
        width = len(entries)
        common += width * (first - stamp)
        stagger += sum(entry - first for entry in entries)
        widths[width].append(first - stamp)
    assert common + stagger == sum(j["entered_us"] - j["claimed_us"] for j in jobs)
    return dict(raw_sha256=digest, stamp_groups=len(groups), **sql,
                common_stamp_to_first_entry_slot_us=common / 64, subsequent_entry_slot_us=stagger / 64,
                by_width={k: {"cohorts": len(v), "median_stamp_to_first_entry_us": median(v)}
                          for k, v in sorted(widths.items())})


def replay():
    prices = json.loads(PRICES.read_text())
    result = {}
    for row in prices["canonical_slow"]:
        group = row["group"]
        path, = STOCK.glob(f"blocks/{group:02}-*/measured-product/*/raw.tar.gz")
        result[f"stock-{group}"] = read_arm(path, row["raw_sha256"])
    summary = (STOCK / "summary.json").read_bytes()
    assert sha256(summary).hexdigest() == "763b9cb39bec01492a9cf6ab6240e5758ef90573345c6571fd5e2fb851d3eaa8"
    for group, block in enumerate(json.loads(summary)["groups"], 1):
        path, = STOCK.glob(f"blocks/{group:02}-*/measured-river/*/raw.tar.gz")
        result[f"river-{group}"] = read_arm(path, block["river"]["raw_sha256"], "go")
    for row in prices["recent_cold_pilot"]:
        for lane, score in row["measured"].items():
            group = row["group"]
            path, = RECENT.glob(f"blocks/{group:02}-*/measured-{lane}/*/raw.tar.gz")
            result[f"recent-{group}-{lane}"] = read_arm(path, score["raw_sha256"])
    assert len(result) == 14
    return result


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2, sort_keys=True))
