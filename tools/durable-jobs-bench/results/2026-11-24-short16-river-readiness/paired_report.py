"""Render matched elapsed-time differences from the six reconciled Linode rotations."""
import json
from pathlib import Path
from statistics import median

HERE = Path(__file__).resolve().parent
PARTS = (
    "enqueue_ms", "second_write_first16_ms", "second_write_tail112_ms",
    "settlement_after_last_write_ms",
)
LANES = (
    ("river-warm", "river-cold"), ("river-cold", "river"),
    ("r2-d2", "river"), ("r2-d2", "river-warm"), ("r2-d2", "oban"),
    ("r2-d2", "apalis"), ("r2-d2", "r2-off"),
)


def report(summary):
    rows = summary["rotations"]
    if summary["missing"] or [row["group"] for row in rows] != list(range(1, 7)):
        raise ValueError("all six reconciled groups are required")
    lines = [
        "# Short jobs with stock and prewarmed River",
        "",
        "Six matched Linode rotations measured 128 jobs per arm. Each job committed two independent application writes.",
        "All durations below are milliseconds. Negative differences mean the first lane finished sooner.",
        "",
        "| First lane minus second | Submit to settle | Drain from commit | Enqueue | Commit to 16th write pair | Remaining 112 write pairs | Last write to settlement | Wins submit / drain |",
        "|---|---:|---:|---:|---:|---:|---:|---:|",
    ]
    for first, second in LANES:
        vectors = []
        for row in rows:
            a, b = (row["lanes"][k] for k in (first, second))
            delta = [a[k] - b[k] for k in ("submit_to_settlement_ms", "commit_to_settlement_ms", *PARTS)]
            if abs(delta[0] - sum(delta[2:])) > 0.001 or abs(delta[1] - sum(delta[3:])) > 0.001:
                raise ValueError(f"group {row['group']} has inconsistent durable endpoints")
            vectors.append(delta)
        cols = list(zip(*vectors))
        numbers = " | ".join(f"{median(col):+.2f}" for col in cols)
        wins = " / ".join(str(sum(v < 0 for v in col)) for col in cols[:2])
        lines.append(f"| {first} minus {second} | {numbers} | {wins} |")
    lines.extend([
        "",
        "Each column is the paired median. The component intervals sum to the elapsed difference within each rotation. Their medians can come from different rotations.",
        "Write cohorts use second application commit order. River warm and cold use one executable. The separate river lane uses the original stock executable.",
        "R2 D2 and R2 off use the exit-confirmed, writer-off Mesh runner with hot and checked application pool checkout respectively.",
        "",
        "[Per-rotation data and executable evidence](evidence/summary.json)",
    ])
    return "\n".join(lines) + "\n"


if __name__ == "__main__":
    print(report(json.loads((HERE / "evidence/summary.json").read_bytes())), end="")
