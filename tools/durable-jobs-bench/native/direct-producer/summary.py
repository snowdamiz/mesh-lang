#!/usr/bin/env python3
"""Report paired latency, throughput, and resource distributions from matrix replay."""

import json
from statistics import median
import sys

CANDIDATE = "mesh-direct-file"
CONTROLS = ("mesh-stock-a", "mesh-stock-b")
TIMES = {"submit_to_commit_ms": ("first_submit_us", "first_commit_us"),
         "submit_to_terminal_ms": ("first_submit_us", "last_settlement_us"),
         "commit_to_terminal_ms": ("first_commit_us", "last_settlement_us")}
CAPS = {"large": (8, 64, 96), "noop": (8, 64, 96), "short": (1, 16, 20), "sparse": (1, 1, 5)}


def distribution(values):
    return {"n": len(values), "median": median(values), "min": min(values), "max": max(values)}


def profile_summary(profile, blocks):
    runners, slots, pg_cap = CAPS[profile]
    lanes = list(blocks[0]["scores"])
    by_group = {}
    for block in blocks:
        population = {}
        for lane, score in block["scores"].items():
            if score["failed"] != 0 or score["peak_connections"] > pg_cap or score["peak_executing"] > slots:
                raise ValueError(f"{profile} {lane}: enqueues, executions or caps failed")
            population[lane] = {metric: (score[end] - score[start]) / 1000
                                for metric, (start, end) in TIMES.items()}
            population[lane].update({field: score[field] for field in (
                "completed", "submit_jobs_per_s", "commit_jobs_per_s", "peak_connections",
                "peak_executing", "peak_runner_rss_bytes", "runner_cpu_ms_per_job",
                "pinned_user_cpu_ms_per_job", "pinned_kernel_cpu_ms_per_job",
                "cpu_steal_percent", "cpu_iowait_percent",
            )})
        by_group[block["group"]] = population

    def score(lane):
        return {name: distribution([group[lane][name] for group in by_group.values()])
                for name in next(iter(by_group.values()))[lane]}

    def paired(candidate, reference):
        measures = {}
        for metric in TIMES:
            saved = [group[reference][metric] - group[candidate][metric]
                     for group in by_group.values()]
            measures[metric] = dict(distribution(saved), wins=sum(value > 0 for value in saved),
                                    ties=sum(value == 0 for value in saved),
                                    milliseconds_saved_by_group=dict(zip(by_group, saved, strict=True)))
        return measures

    return dict(caps=dict(runners=runners, slots=slots, postgres_connections=pg_cap),
                groups=by_group, lanes={lane: score(lane) for lane in lanes},
                direct_vs={lane: paired(CANDIDATE, lane) for lane in lanes if lane != CANDIDATE},
                stock_b_vs_stock_a=paired(CONTROLS[1], CONTROLS[0]))


def main():
    source = json.load(sys.stdin)
    result = dict(cpus=source["cpus"], host=source["host"], missing=source["missing"], issues=source["issues"],
                  profiles={profile: profile_summary(profile, blocks)
                            for profile, blocks in source["rows"].items()})
    print(json.dumps(result, indent=2, allow_nan=False))
    return 1 if result["missing"] else 0


if __name__ == "__main__":
    sys.exit(main())
