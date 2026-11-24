"""Create a compact native scorecard from a qualified matrix replay."""

import argparse
import json
from pathlib import Path
from statistics import median

from matrix import require

WINDOWS = ("submit_jobs_per_s", "commit_jobs_per_s")
MESH = "mesh-v1-2x32"
ENGINES = (MESH, "mesh-v1-8x8", "oban", "river", "apalis")
NAMES = {"mesh": "Mesh V1", "oban": "Oban", "river": "River", "apalis": "Apalis"}


def display(label):
    topology = label.rsplit("-", 1)[1].replace("x", " × ")
    return f"{NAMES[label.split('-', 1)[0]]} {topology}"


def format_rate(rate):
    return f"{rate['median']:.2f} [{rate['min']:.2f}–{rate['max']:.2f}]"


def scorecard(summary, output):
    require(summary["qualified_rotations"] == 5 and not summary["missing"], "five rotations are required")
    print("| Deployment | Submit jobs/s | Drain jobs/s | Submit wins by Mesh | Drain wins by Mesh |", file=output)
    print("| --- | ---: | ---: | ---: | ---: |", file=output)
    order = [label for name in ENGINES for label in summary["scores"]
             if label == name or label.startswith(f"{name}-")]
    for label in order:
        scores = summary["scores"][label]
        pairs = [(row[f"measured-{MESH}"], row[f"measured-{label}"]) for row in summary["blocks"].values()]
        wins = [sum(mesh[window] > peer[window] for mesh, peer in pairs) for window in WINDOWS]
        values = [format_rate(scores[metric]) for metric in WINDOWS]
        markers = ["—", "—"] if label == MESH else [f"{count}/5" for count in wins]
        print("| " + " | ".join([display(label), *values, *markers]) + " |", file=output)
    print(file=output)
    print("| Deployment | Median runner CPU (ms/job) | Physical conn. peak [range] | PG pool cap (runner roles) | Warm app | Used app [range] | Terminal transactions |",
          file=output)
    print("| --- | ---: | ---: | ---: | ---: | ---: | ---: |", file=output)
    for label in order:
        scores = summary["scores"][label]
        topology = [row[f"measured-{label}"]["topology"] for row in summary["blocks"].values()]
        require(all(item["configured_runner_pool_max"] == topology[0]["configured_runner_pool_max"]
                    for item in topology), "configured runner pool changed")
        runner = topology[0]["configured_runner_pool_max"]
        if label.startswith("mesh"):
            runner = f"{runner} queue + 32 app"
        app = topology[0].get("warm_application_sessions", "shared")
        used = [item["used_application_sessions"] for item in topology if "used_application_sessions" in item]
        usage = f"{min(used)}–{max(used)}" if used else "—"
        cpu = scores["runner_cpu_ms_per_job"]["median"]
        peaks = scores["peak_connections"]
        terminal = median([item["terminal_transactions"] for item in topology])
        print(f"| {display(label)} | {cpu:.3f} | {peaks['median']} [{peaks['min']}–{peaks['max']}] | "
              f"{runner} | {app} | {usage} | {terminal:g} |", file=output)


if __name__ == "__main__":
    import sys

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("summary", type=Path)
    args = parser.parse_args()
    scorecard(json.loads(args.summary.read_text()), sys.stdout)
