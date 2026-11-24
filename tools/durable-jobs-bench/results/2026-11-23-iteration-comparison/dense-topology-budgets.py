"""Replay the dense crossover and price enqueue improvements with drain held fixed."""

import argparse
from pathlib import Path
from statistics import median
import sys

HERE = Path(__file__).resolve().parent
SOURCE = HERE.parent / "2026-11-23-dense-matched-topology"
sys.path.insert(0, str(SOURCE))
import matrix


def durations(row):
    count = row["completed"]
    if count != 1024 or row["failed"]:
        raise ValueError("expected the full 1,024-job dense workload")
    full, drain = (round(count * 1_000_000 / row[key])
                   for key in ("submit_jobs_per_s", "commit_jobs_per_s"))
    if full - drain != round(row["first_submit_to_commit_ms"] * 1000):
        raise ValueError("enqueue and settlement boundaries do not reconcile")
    return full, drain


def report(kind):
    config, schedule, pins = matrix.inputs(kind)
    summary = matrix.summarize(SOURCE / "evidence" / kind, config, schedule, pins)
    if summary["qualified_rotations"] != 5:
        raise ValueError(f"{kind} has {summary['qualified_rotations']}/5 archived rotations")
    peers = [name for name in config["lanes"] if not name.startswith("mesh-")]
    labels = {"river": "Go River", "oban": "Elixir Oban", "apalis": "Rust Apalis"}
    lines = ["# Dense submit and drain budgets at two Mesh runners", "",
             "Each rotation ran the frozen 1,024-job workload on two CPUs. Mesh used two "
             "32-slot runners. Each peer used " + ("eight 8-slot runners." if kind == "stock"
             else "two 32-slot runners.") + " The calculations use the archived, reconciled clocks.",
             "", "The full gap is Mesh submit-to-settlement time less the peer’s. "
             "The final column gives the minimum drain saving still needed if Mesh could enqueue "
             "instantaneously and preserve its measured drain. All values are milliseconds.", "",
             "| Rotation | Peer | Mesh enqueue | Full gap | Drain saving needed with free enqueue |",
             "|---|---|---:|---:|---:|"]
    residuals = {name: [] for name in peers}
    for block, arms in sorted(summary["blocks"].items()):
        full, drain = durations(arms["measured-mesh-v1-2x32"])
        for name in peers:
            label = labels[name.split("-")[0]]
            peer_full, _ = durations(arms[f"measured-{name}"])
            residual = max(0, drain - peer_full) / 1000
            residuals[name].append(residual)
            lines.append(f"| {block.split('-')[0]} | {label} | {(full-drain)/1000:.1f} | "
                         f"{(full-peer_full)/1000:+.1f} | {residual:.1f} |")
    lines.extend(["", "| Peer | Rotations requiring faster drain | Median minimum saving |",
                  "|---|---:|---:|"])
    for name, values in residuals.items():
        lines.append(f"| {labels[name.split('-')[0]]} | {sum(v > 0 for v in values)}/5 | {median(values):.1f} ms |")
    lines.extend(["", "A positive residual requires a consumer-side saving even under the free-enqueue "
                  "assumption. A measured change to the enqueue path can also change consumer behavior. "
                  "[Original crossover, controls and replay](../2026-11-23-dense-matched-topology/README.md)", ""])
    return "\n".join(lines)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("kind", choices=("stock", "equal"))
    args = parser.parse_args()
    print(report(args.kind), end="")
