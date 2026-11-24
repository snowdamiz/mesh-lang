"""Format the fully reconciled controller replay as a Markdown timing report."""
import json
import sys

LABELS = ("hot", "checked-a", "checked-b", "river-2x32", "oban-2x32", "apalis-2x32")
METRICS = ("submit_us", "drain_us", "middle_effect_us", "middle_settlement_us")


def elapsed(value):
    return f"{value['median'] / 1000:.1f} [{value['min'] / 1000:.1f}, {value['max'] / 1000:.1f}]"


def report(result):
    if result["missing"] or result["qualified_rotations"] != 6:
        raise ValueError("A timing report requires all six qualified rotations")
    lines = [
        "# Public dense Jobs with individual settlement",
        "",
        "Six native rotations ran 1,024 jobs per arm on CPUs 0 and 1 with two 32-slot runners and "
        "PostgreSQL 17.11. Each Mesh job had two independent application XIDs and its own terminal XID. "
        "The three Mesh lanes ran the same Jobs proof ELF. The hot lane enabled the application pool URL option.",
        "",
        "Times are median [min, max] in milliseconds. The middle intervals span completion ranks 103–922 "
        "separately for second application commits and terminal commits.",
        "",
        "| Lane | Submit to terminal | Enqueue commit to terminal | Middle second writes | Middle terminals |",
        "|---|---:|---:|---:|---:|",
    ]
    for label in LABELS:
        values = [elapsed(result["elapsed_us"][label][key]) for key in METRICS]
        lines.append("| " + " | ".join([label, *values]) + " |")
    lines.extend([
        "", "Paired millisecond differences are hot minus the named lane. Negative differences favor hot. "
        "Parentheses show the number of rotations hot won.",
        "", "| Comparison lane | Submit to terminal | Enqueue commit to terminal | Middle second writes | Middle terminals |",
        "|---|---:|---:|---:|---:|",
    ])
    for label in LABELS[1:]:
        row = result["paired_hot_minus_us"][label]
        values = [f"{elapsed(row[key])} ({row[key]['hot_wins']}/6)" for key in METRICS]
        lines.append("| " + " | ".join([label, *values]) + " |")
    lines.append("")
    for label in LABELS[:3]:
        arms = [r["scores"][f"measured-{label}"] for r in result["blocks"].values()]
        probes = [arm["checkout_probes"] for arm in arms]
        peaks = [arm["peak_connections"] for arm in arms]
        lines.append(f"- {label} recorded {min(probes)}–{max(probes)} checkout probes per arm "
                     f"and {min(peaks)}–{max(peaks)} peak PostgreSQL connections.")
    decision = "passed" if result["advances"] else "missed"
    lines += ["", f"Hot {decision} the preregistered four-of-six duplicate-control gate "
              f"with {len(result['qualifying_groups'])}/6 qualifying rotations.", ""]
    return "\n".join(lines)


if __name__ == "__main__":
    print(report(json.load(sys.stdin)), end="")
