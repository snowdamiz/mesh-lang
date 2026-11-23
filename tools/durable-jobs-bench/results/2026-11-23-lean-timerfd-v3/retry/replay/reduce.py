"""Reduce exact waits and descendant-free component parent CPU separately."""
import json
from pathlib import Path
from statistics import median
import sys

from component import WAVES, WIDTH
from measure import ORDERS

fields = ("child_cpu_ms", "child_system_ms", "parent_cpu_ms", "tree_cpu_ms",
          "wall_ms", "minor_faults", "major_faults", "child_rss_kib")
all_pids = set()
arms = []
pairs = []
stock = []
rows = [json.loads(line) for line in Path(sys.argv[1]).read_text().splitlines()]
samples = [row for row in rows if row["kind"] == "sample"]
assert [(row["rotation"], row["arm"]) for row in samples] == [
    (i, arm) for i, order in enumerate(ORDERS, 1) for arm in order]
for row in samples:
    jobs = row["rows"]
    n = len(jobs)
    assert n == WAVES * WIDTH
    assert all(job["wait_status"] == 0 and len(job["effects"]) == 2 for job in jobs)
    cohort = {job["pid"] for job in jobs}
    assert len(cohort) == n and not (all_pids & cohort)
    all_pids |= cohort
    sys_ms = sum(job["system_us"] for job in jobs) / n / 1000
    child = sum(job["user_us"] for job in jobs) / n / 1000 + sys_ms
    parent = (row["parent"]["user_us"] + row["parent"]["system_us"]) / n / 1000
    arms.append(dict(rotation=row["rotation"], arm=row["arm"], n=n, child_cpu_ms=child,
                     child_system_ms=sys_ms, parent_cpu_ms=parent, tree_cpu_ms=child + parent,
                     wall_ms=row["wall_ns"] / 1e6,
                     minor_faults=sum(job["minor_faults"] for job in jobs) / n,
                     major_faults=sum(job["major_faults"] for job in jobs) / n,
                     child_rss_kib=median(job["peak_rss_kib"] for job in jobs)))
for i in range(1, len(ORDERS) + 1):
    group = {row["arm"]: row for row in arms if row["rotation"] == i}
    pairs.append(dict(rotation=i, **{
        f"saved_{field}": (group["lean-a"][field] + group["lean-b"][field]) / 2 - group["timerfd"][field]
        for field in fields
    }, **{f"spread_{field}": abs(group["lean-a"][field] - group["lean-b"][field]) for field in fields}))
    stock.append(group["stock"]["child_cpu_ms"])
summary = {field: {arm: median(row[field] for row in arms if row["arm"] == arm)
                   for arm in ("stock", "lean-a", "lean-b", "timerfd")}
           for field in fields}
deltas = {field: median(row[f"saved_{field}"] for row in pairs) for field in fields}
spread = {field: max(row[f"spread_{field}"] for row in pairs) for field in fields}
decision = {
    "signal_target_child_cpu_ms": 0.3,
    "strong_child_and_tree_signal": (
        deltas["child_cpu_ms"] >= 0.3 and deltas["tree_cpu_ms"] >= 0.3
        and deltas["child_cpu_ms"] > spread["child_cpu_ms"]
        and deltas["tree_cpu_ms"] > spread["tree_cpu_ms"]
        and all(pair["saved_child_cpu_ms"] > 0 and pair["saved_tree_cpu_ms"] > 0 for pair in pairs)
    ),
    "incremental_system_cpu_ms": deltas["child_system_ms"],
    "incremental_parent_cpu_ms": deltas["parent_cpu_ms"],
    "incremental_runner_tree_cpu_ms": deltas["tree_cpu_ms"],
}
print(json.dumps(dict(arms=arms, pairs=pairs, median=summary, savings=deltas, spread=spread,
                      decision=decision), indent=2))
