"""Compare full-submit parity budgets using the six sealed hot-pool rotations."""
from statistics import median

from native_report import LANES, NAMES, replay


def budget(mesh, peer):
    produce = mesh["submit"] - mesh["drain"]
    peer_produce = peer["submit"] - peer["drain"]
    middle = mesh["writes"][921] - mesh["writes"][102]
    peer_middle = peer["writes"][921] - peer["writes"][102]
    gap = mesh["submit"] - peer["submit"]
    return {
        "enqueue_gap": produce - peer_produce,
        "drain_gap": mesh["drain"] - peer["drain"],
        "gap_after_free_enqueue": mesh["drain"] - peer["submit"],
        "middle_saving": middle - peer_middle,
        "gap_after_matching_middle": gap - (middle - peer_middle),
        "gap": gap,
    }


def report():
    trials, _ = replay()
    lines = [
        "# Dense producer and middle-drain budgets",
        "",
        "These six 2×32 Linode rotations used 1,024 jobs, PostgreSQL 17.11 and the grouped-settlement "
        "public hot-pool executable. Times are paired differences in milliseconds. Positive gaps mean "
        "Mesh finished later. Each counterfactual holds every other interval in that arm fixed.",
        "",
        "| Peer | Full-submit gap | Enqueue gap | Drain gap | Residual with free Mesh enqueue, range | "
        "Residual after matching the peer’s middle writes, range |",
        "| --- | ---: | ---: | ---: | ---: | ---: |",
    ]
    for label, name in zip(LANES[3:], NAMES[3:], strict=True):
        rows = [budget(trial["hot"], trial[label]) for trial in trials]
        cells = []
        for key in ("gap", "enqueue_gap", "drain_gap", "gap_after_free_enqueue", "gap_after_matching_middle"):
            values = [row[key] for row in rows]
            cells.append(f"{min(values):+.1f} to {max(values):+.1f}" if key.startswith("gap_after")
                         else f"{median(values):+.1f}")
        lines.append("| " + " | ".join([name, *cells]) + " |")
    lines.extend([
        "",
        "Free enqueue credits the full time from first submit to durable enqueue commit. Matching the "
        "peer’s middle writes replaces only the elapsed interval between its 103rd and 922nd durable "
        "second writes. The two counterfactuals are independent. Neither attributes a saving to a "
        "specific implementation. Medians of separate columns can come from different pairs.",
        "",
        "| Rotation | Peer | Full-submit gap | Residual with free enqueue | Mesh middle minus peer middle | "
        "Residual after matching middle writes |",
        "| ---: | --- | ---: | ---: | ---: | ---: |",
    ])
    for group, trial in enumerate(trials, 1):
        for label, name in zip(LANES[3:], NAMES[3:], strict=True):
            row = budget(trial["hot"], trial[label])
            cells = [f"{row[key]:+.1f}" for key in ("gap", "gap_after_free_enqueue", "middle_saving",
                                                      "gap_after_matching_middle")]
            lines.append("| " + " | ".join([str(group), name, *cells]) + " |")
    lines.extend(["", "Run `python3 -B producer_drain_budget.py` in this directory. The underlying replay "
                  "checks 72 raw archive digests, the two durable writes per job, and both clocks. "
                  "[Original four-engine results](NATIVE-RESULTS.md)", ""])
    return "\n".join(lines)


if __name__ == "__main__":
    print(report(), end="")
