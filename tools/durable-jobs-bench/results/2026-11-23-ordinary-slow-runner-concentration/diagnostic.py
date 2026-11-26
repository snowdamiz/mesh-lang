"""Price the refill phases and their SQL on real ledgers, even when elapsed is noisy."""

import argparse
import json
from pathlib import Path

from replay import distribution, require
from arm import read


def sql_cost(path):
    before, after = (read(path / f"statements-{state}.json") for state in ("before", "end"))
    queries = {
        "claim": lambda q: "FROM mesh_jobs.claim_batch_v1(" in q,
        "due_probe": lambda q: "SELECT (j.scheduled_at_ms -" in q,
        "settlement": lambda q: "mesh_jobs.settle_state(" in q,
        "recover": lambda q: "SELECT mesh_jobs.recover(" in q,
    }
    costs = {}
    for name, matching in queries.items():
        def total(statements, field):
            return sum(row[field] for row in statements
                       if row["role"] == "parity_subject" and matching(row["query"]))
        costs[name] = {field: total(after, field) - total(before, field)
                       for field in ("calls", "rows", "total_exec_time")}
        require(costs[name]["calls"] >= 0 and costs[name]["total_exec_time"] >= 0, f"{name} counters rolled back")
    return costs


def compare(path, series):
    rows = []
    for group in series["groups"]:
        data = {}
        for label in group["order"]:
            score = group["arms"][label]
            arm = path / f"{group['group']:02}" / label
            costs = sql_cost(arm)
            # The detailed snapshots bracket the canonical counters. Late
            # idle scans can enter the wider snapshot but cannot subtract work.
            require(costs["claim"]["calls"] >= score["claim_calls"]
                    and costs["settlement"]["calls"] == score["completed"], "claim or terminal SQL count differs")
            data[label] = dict(sql=costs, load_at_launch=read(arm / "host-before.json")["loadavg"][0],
                               extra_claims_in_sql_window=costs["claim"]["calls"] - score["claim_calls"])
        rows.append(dict(group=group["group"], arms=data))
    return dict(groups=rows, rollup={
        label: dict(
            load_at_launch=distribution([row["arms"][label]["load_at_launch"] for row in rows]),
            sql={metric: {field: distribution([row["arms"][label]["sql"][metric][field] for row in rows])
                          for field in ("calls", "total_exec_time")}
                 for metric in ("claim", "due_probe", "settlement", "recover")})
        for label in series["by_label"]})


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("series", type=Path)
    parser.add_argument("summary", type=Path)
    args = parser.parse_args()
    print(json.dumps(compare(args.series, read(args.summary)), indent=2, sort_keys=True))
