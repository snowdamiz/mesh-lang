#!/usr/bin/env python3
"""Reconstruct paired wall budgets from retained Linode scores."""
import hashlib
import json
from pathlib import Path
import statistics

ROOT = Path(__file__).resolve().parents[1]
SERIES = [
    ("earlier_bulk_d2", "2026-11-23-ordinary-hot-checkout-bulk-v2/native-summary.json",
     "short", ["bulk-d2"], ["river"]),
    ("six_lane_bulk_d2",
     "2026-11-24-bulk-d2-short-peers/evidence/2026-11-24-bulk-d2-short-peers/summary.json",
     "short", ["bulk-d2"], ["river", "oban", "apalis"]),
    ("large_broker_once", "2026-11-23-ordinary-broker-once/evidence/native-summary.json",
     "large", ["ordinary-a", "ordinary-b"], ["river-external", "oban-external", "apalis-external"]),
]


def measured(s, jobs):
    return jobs * 1000 / s["submit_jobs_per_s"], s["first_submit_to_commit_ms"]


def build():
    report = {}
    for label, path, profile, stocks, peers in SERIES:
        data = (ROOT / path).read_bytes()
        receipt = json.loads(data)
        assert not receipt["issues"] and not receipt["missing"]
        rows = []
        for row in receipt["rows"][profile]:
            jobs = 128 if profile == "short" else 64
            scores = row["scores"]
            assert all(scores[k]["completed"] == jobs for k in [*stocks, *peers])
            estimates = [measured(scores[k], jobs) for k in stocks]
            elapsed = statistics.mean(x[0] for x in estimates)
            enqueue = statistics.mean(x[1] for x in estimates)
            rows.append({
                "group": row["group"], "stock_submit_to_commit_ms": [x[1] for x in estimates],
                "stock_duplicate_enqueue_spread_ms": round(max(x[1] for x in estimates) - min(x[1] for x in estimates), 3),
                "stock_full_submit_ms": [round(x[0], 3) for x in estimates],
                "saving_needed_ms": {
                    k: round(elapsed - measured(scores[k], jobs)[0], 3) for k in peers},
                "can_close_by_changing_enqueue_alone": {
                    k: 0 < elapsed - measured(scores[k], jobs)[0] <= enqueue for k in peers},
                "raw_sha256": row["raw_sha256"],
            })
        assert len(rows) == 5
        report[label] = {"profile": profile, "source": path, "host": receipt["host"],
                         "source_sha256": hashlib.sha256(data).hexdigest(), "rotations": rows}
    return report


if __name__ == "__main__":
    print(json.dumps(build(), indent=2, sort_keys=True))
