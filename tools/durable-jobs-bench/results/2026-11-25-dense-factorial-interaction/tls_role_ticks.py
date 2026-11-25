"""Attribute the archived PostgreSQL clock ticks to census-identified backend PIDs."""
from collections import Counter
import json

from settlement_scans import FIXED8, RESULTS, archive, drain, read

FACTORIAL = RESULTS / "2026-11-25-dense-fixed8-logical-factorial/evidence/native-retry"
TLS_COUNTS = {"parity_subject": 8, "parity_producer": 1, "compiled_caller": 32}


def summarize(raw):
    enqueue, terminal, _ = drain(raw)
    transaction, = read(raw, "adapter/ledger.json")["transactions"]
    submit = transaction["submitted_us"]
    proof = read(raw, "adapter/pg-cpu-bracket.json")
    roster = read(raw, "adapter/postgres-tls-sessions.json")
    first, last = proof["before"], proof["after"]
    parent = proof["postmaster"]
    assert first["begin_us"] <= first["end_us"] <= submit <= enqueue < terminal <= last["begin_us"] <= last["end_us"]
    assert proof["clock_ticks"] == 100
    assert read(raw, "adapter/process-affinity.json")[str(parent)] == [0, 1]
    roles = {str(row["backend"]): row["role"] for row in roster if row["tls"] is True}
    counts = Counter(roles.values())
    assert len(roster) == len(roles) and counts.keys() == TLS_COUNTS.keys()
    assert counts["compiled_caller"] == 32 and counts["parity_producer"] == 1
    assert 0 < counts["parity_subject"] <= 8
    stable = {}
    unpaired = []
    for pid in sorted(first["rows"].keys() | last["rows"].keys(), key=int):
        a, b = first["rows"].get(pid), last["rows"].get(pid)
        if a is None or b is None or a[:2] != b[:2]:
            unpaired.append(pid)
            continue
        assert int(pid) == parent or a[0] == parent
        assert b[2] >= a[2]
        stable[pid] = b[2] - a[2]
    assert roles.keys() <= stable.keys() and str(parent) in stable
    buckets = {}
    for role in TLS_COUNTS:
        counts = [stable[p] for p in roles if roles[p] == role]
        buckets[role] = dict(ticks=sum(counts), per_backend_ticks=sorted(counts, reverse=True))
    # At either endpoint the sum of the two published counters can omit
    # less than two ticks. All out-of-window work shares the two CPUs.
    slack_us = submit - first["begin_us"] + last["end_us"] - terminal
    ticks = buckets["parity_subject"]["per_backend_ticks"]
    measured_queue_cpu_us = [
        max(0, 10000 * sum(max(0, t - 2) for t in ticks) - 2 * slack_us),
        10000 * sum(t + 2 for t in ticks),
    ]
    other = sum(t for p, t in stable.items() if p not in roles)
    return dict(roster_complete=counts == TLS_COUNTS, roles=buckets,
                census_queue_submit_to_settle_cpu_bounds_us=measured_queue_cpu_us, other_stable_ticks=other, total_stable_ticks=sum(stable.values()),
                unpaired_pids=unpaired, submit_lead_us=submit-first["begin_us"],
                terminal_lag_us=last["end_us"]-terminal)


def replay(root, receipt, lanes_by_group):
    groups = json.loads((root / receipt).read_bytes())["groups"]
    result = {}
    for group, lanes in lanes_by_group.items():
        result[group] = {}
        for lane in lanes:
            score = groups[str(group)]["measured"][lane]
            with archive(root, group, lane, score["raw_sha256"]) as raw:
                row = summarize(raw)
            if "pg_cpu_ms" in score:
                assert row["total_stable_ticks"] * 10 == score["pg_cpu_ms"]
            row["raw_sha256"] = score["raw_sha256"]
            result[group][lane] = row
    return result


if __name__ == "__main__":
    pilot = ["stock-a", "stock-b", "fixed8", "key", "combined"]
    blocks = {i: pilot if i < 3 else ["combined"] for i in range(1, 8)}
    print(json.dumps(dict(factorial=replay(FACTORIAL, "final-replay.json", blocks),
                          readiness=replay(FIXED8, "replay-rest.json", {i: ["combined"] for i in range(1, 7)})),
                     sort_keys=True, indent=2))
