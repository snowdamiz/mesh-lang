#!/usr/bin/env python3
"""Extract bounded attribution from the five already-reconciled stock native blocks."""
import json
from pathlib import Path
import sys

from local_series import distribution
import storage

HERE = Path(__file__).resolve().parents[1]
ROOT = HERE.parents[3]
SERIES = HERE.parent / "2026-11-23-set-based-enqueue/evidence/bulk-enqueue-v1-slow"
FIELDS = (
    "commit_to_pickup_p99_ms", "effect_commit_to_settlement_p99_ms",
    "first_claim_from_commit_ms", "claim_to_last_settlement_ms", "mean_occupied_slots_lower",
    "mean_slot_residence_lower_ms", "runner_cpu_ms_per_job", "pinned_user_cpu_ms_per_job",
    "pinned_kernel_cpu_ms_per_job", "producer_commit_ms", "commit_drain_ms", "submit_drain_ms",
    "peak_executing", "peak_connections",
)


def main():
    data = []
    for entry in sorted((SERIES / "blocks").glob("*/reconciled.json")):
        original = json.loads(entry.read_text())
        measured = {}
        for label, score in original["scores"].items():
            if score["completed"] != 512 or score["failed"] or score["ordered_independent_writes"] != 512:
                raise ValueError(f"{entry}: incomplete prior arm {label}")
            row = dict(score,
                producer_commit_ms=(score["first_commit_us"] - score["first_submit_us"]) / 1000,
                commit_drain_ms=(score["last_settlement_us"] - score["first_commit_us"]) / 1000,
                submit_drain_ms=(score["last_settlement_us"] - score["first_submit_us"]) / 1000,
            )
            measured[label] = {metric: row[metric] for metric in FIELDS if row.get(metric) is not None}
        data.append(dict(path=str(entry.relative_to(ROOT)), sha256=storage.sha(entry.read_bytes()),
                         raw_sha256=original["raw_sha256"], measured=measured))
    if len(data) != 5:
        raise ValueError("expected the five frozen measured native blocks")
    distributions = {}
    for label, score in data[0]["measured"].items():
        distributions[label] = {field: distribution([block["measured"][label][field] for block in data])
                                for field in score}
    gaps = {
        label: {field: distribution([block["measured"]["bulk"][field] - block["measured"][label][field]
                                     for block in data]) for field in
                ("producer_commit_ms", "commit_drain_ms", "submit_drain_ms")}
        for label in ("river", "oban", "apalis")
    }
    output = dict(blocks=data, distributions=distributions, bulk_minus_peer=gaps,
                  scope="Prior five stock-policy Linux blocks. Summary and original raw hashes only.")
    storage.write(HERE / "evidence/retained-native-v1-slow.json", output)


if __name__ == "__main__":
    sys.exit(main())
