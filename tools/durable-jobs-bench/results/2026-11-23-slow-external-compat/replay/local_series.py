#!/usr/bin/env python3
"""Run and replay local canonical and compressible crossover blocks."""
import argparse
import gzip
import json
from pathlib import Path
from statistics import median
import subprocess
import sys

import local_mesh
import storage

RESULT = Path(__file__).resolve().parents[1]
DEFAULT_A, DEFAULT_B, EXTERNAL, FALLBACK = "default-a", "default-b", "external-bulk", "external-fallback"
ORDERS = (
    (DEFAULT_A, EXTERNAL, FALLBACK, DEFAULT_B),
    (EXTERNAL, DEFAULT_B, DEFAULT_A, FALLBACK),
    (DEFAULT_B, FALLBACK, EXTERNAL, DEFAULT_A),
    (FALLBACK, DEFAULT_A, DEFAULT_B, EXTERNAL),
    (DEFAULT_A, DEFAULT_B, EXTERNAL, FALLBACK),
    (FALLBACK, EXTERNAL, DEFAULT_B, DEFAULT_A),
)
METRICS = (
    "first_submit_to_commit_ms", "producer_ack_ms", "producer_cpu_ms", "producer_backend_cpu_ms",
    "producer_sql_exec_ms", "producer_sql_wal_bytes", "submit_to_last_settlement_ms",
    "commit_to_last_settlement_ms", "commit_to_pickup_p99_ms",
    "effect_commit_to_settlement_p99_ms", "runner_cpu_ms_per_job", "peak_executing",
    "compressed_rows", "payload_column_bytes", "toast_heap_bytes", "total_bytes", "peak_committed_holds",
)


def distribution(values):
    return dict(n=len(values), median=median(values), min=min(values), max=max(values))


def arms(out, orders):
    for number, order in enumerate(orders, 1):
        for label in order:
            yield number, label, out / f"{number:02d}-{label}"


def summarize(out, orders):
    batches = [json.loads(gzip.decompress(
        (RESULT / "frozen/canonical-batch.json.gz").read_bytes()
    ))]
    batch_bytes = (json.dumps(batches[0], indent=2, sort_keys=True, allow_nan=False) + "\n").encode()
    case = storage.sha(storage.sha(batch_bytes).encode())
    scores, missing = {}, []
    for number, label, arm in arms(out, orders):
        if not (arm / "score.json").is_file():
            missing.append(arm.name)
            continue
        before = json.loads((arm / "score.json").read_text())
        replayed = local_mesh.score_arm(arm, label, batches, case)
        if replayed != before:
            raise ValueError(f"{arm}: replayed score differs from the recorded score")
        scores.setdefault(number, {})[label] = replayed
    labels = {label for order in orders for label in order}
    distributions = {
        label: {metric: distribution(samples) for metric in METRICS if
                (samples := [block[label][metric] for block in scores.values() if label in block])}
        for label in sorted(labels)
    }
    paired = {}
    rows = [block for block in scores.values() if block.keys() == labels]
    if rows:
        for metric in METRICS:
            stock = [min(b[DEFAULT_A][metric], b[DEFAULT_B][metric]) - b[EXTERNAL][metric] for b in rows]
            fallback = [b[FALLBACK][metric] - b[EXTERNAL][metric] for b in rows]
            paired[metric] = {
                "default_a_minus_b": distribution([b[DEFAULT_A][metric] - b[DEFAULT_B][metric] for b in rows]),
                "faster_default_minus_external": distribution(stock),
                "fallback_minus_external": distribution(fallback),
                "external_wins_against_both_defaults": sum(x > 0 for x in stock),
                "external_wins_against_fallback": sum(x > 0 for x in fallback),
            }
    summary = dict(order=orders, missing=missing, by_group=scores,
                   distributions=distributions, paired=paired)
    storage.write(out / "summary.json", summary)
    return bool(missing)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("run", "replay"))
    parser.add_argument("work", type=Path)
    args = parser.parse_args()
    orders = ORDERS
    out = RESULT / "evidence/rotations"
    out.mkdir(exist_ok=True)
    if args.action == "run":
        for number, label, arm in arms(out, orders):
            if (arm / "score.json").is_file():
                continue
            if arm.exists():
                raise RuntimeError(f"preserve incomplete {arm} under a distinct name before retrying")
            log = out / f"{arm.name}.log"
            argv = [sys.executable, "-B", str(RESULT / "replay/local_mesh.py"),
                    str(args.work), str(arm), label]
            with log.open("xb") as stream:
                subprocess.run(argv, stdout=stream, stderr=subprocess.STDOUT, check=True, timeout=120)
            print(arm.name, flush=True)
            if label == orders[number - 1][-1]:
                summarize(out, orders)
    return summarize(out, orders)


if __name__ == "__main__":
    sys.exit(main())
