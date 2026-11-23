"""Verify the archived console projections and resource witnesses offline."""

import ast
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parent
EVIDENCE = ROOT / "evidence"


def require(condition, message):
    if not condition:
        raise ValueError(message)


def verify_dispatch():
    tree = ast.parse((ROOT / "replay/run.py").read_text())
    orders = next(ast.literal_eval(node.value) for node in tree.body
                  if isinstance(node, ast.Assign) and any(
                      isinstance(target, ast.Name) and target.id == "ORDERS" for target in node.targets))
    expected = ["Waiting for shared local timing lock"]
    for group, order in enumerate(orders, 1):
        profiles = ("short", "dense") if group % 2 else ("dense", "short")
        expected.extend(f"{group:02}-{profile} {label} exit 0"
                        for profile in profiles for label in order)
    observed = (EVIDENCE / "local-v1-dispatch.txt").read_text().splitlines()
    require(len(observed) == 41 and observed == expected, "dispatch order or exit status differs")
    finished = json.loads((EVIDENCE / "finished.json").read_text())
    require(finished["inputs_unchanged"] is True, "original input pin recheck failed")


def verify_receipt():
    receipt = json.loads((EVIDENCE / "performance-receipt.json").read_text())
    header, *lines = (EVIDENCE / "original-rollup-stdout.txt").read_text().splitlines()
    require(header == "failures [] pending [] groups {'dense': 5, 'short': 5}", "incomplete rollup")
    paired = receipt["paired_percent"]
    profile = None
    for line in lines:
        if line in ("short", "dense"):
            profile = line
        elif line.startswith("{"):
            require(ast.literal_eval(line) == paired["final_over_five_groups"][profile],
                    f"{profile} paired summary differs")
        else:
            label, columns = line.split(" ", 1)
            expected = receipt["aggregate_by_arm"][profile][label]
            actual = {}
            for column in columns.split("\t"):
                metric, stats = column.split("=")
                median, lo, hi = map(float, stats.split("/"))
                actual[metric] = dict(median=median, min=lo, max=hi)
            require(actual == expected, f"{profile} {label} rollup differs")

    for line in (EVIDENCE / "early-groups-stdout.txt").read_text().splitlines():
        if re.match(r"^(dense|short) \d+ \[", line):
            profile, group, arms = line.split(" ", 2)
            keys = ("submit_jobs_per_s", "commit_jobs_per_s", "runner_cpu_ms_per_job", "load_1m_at_start")
            observed = {label: dict(zip(keys, values, strict=True))
                        for label, *values in ast.literal_eval(arms)}
            require(observed == receipt["individual_arm_rows_from_early_stdout"][profile][group],
                    f"{profile} group {group} arm rows differ")
        elif line in ("short", "dense"):
            profile = line
        else:
            label, values = line.split(" ", 1)
            require(ast.literal_eval(values) == paired["early_group_values"][profile][label],
                    f"{profile} {label} early paired changes differ")


def verify_witnesses():
    lines = (EVIDENCE / "pg-activity-excerpts.txt").read_text().splitlines()
    require(len(lines) == 2, "expected both PostgreSQL snapshots")
    for line in lines:
        label, _, witness = line.split(" ", 2)
        sample = json.loads(witness)
        inserts = [row for row in sample["application"]
                   if row["query"].startswith("INSERT INTO public.mesh_jobs_parity_attempts")
                   and row["state"] == "active" and row["query_start_us"] <= sample["observed_us"]]
        require(len({row["pid"] for row in inserts}) == 3, f"{label} lacks simultaneous inserts")
        if label == "profile-dense-workers-8-01":
            require(all(row["wait_event_type"] == "LWLock" and row["wait_event"] == "WALWrite"
                        for row in inserts), "eight-worker WALWrite witness differs")
    arm = (EVIDENCE / "short04-workers4-records.stdout.txt").read_text().splitlines()
    tls = ast.literal_eval(next(line.partition(" ")[2] for line in arm if line.startswith("postgres-tls-sessions ")))
    require(sum(row["role"] == "parity_subject" and row["tls"] for row in tls) == 2,
            "early queue connection count differs")
    idle_line = next(line.partition(" ")[2] for line in arm if line.startswith("idle_samples "))
    idle = ast.literal_eval(idle_line.partition("] [")[0] + "]")
    require([sample["connections"] for sample in idle] == [13, 13], "warmed connection count differs")
    runner = json.loads((EVIDENCE / "short04-workers4-runner-0.log").read_text().splitlines()[1])
    require(runner["drained"] and runner["outcomes"]["completed"] == 128
            and runner["wakeups"]["notification_scans"] == 1, "short runner receipt differs")


if __name__ == "__main__":
    verify_dispatch()
    verify_receipt()
    verify_witnesses()
    print("Verified 40 dispatch records, the recovered rollups, and the selected resource witnesses.")
