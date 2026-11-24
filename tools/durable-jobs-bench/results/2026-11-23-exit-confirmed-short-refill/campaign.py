#!/usr/bin/env python3
"""Run the frozen combined short queue and replay its same-host comparison."""
import argparse
from collections import Counter
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
from statistics import median
import subprocess
import sys
import time

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
SCRATCH = ROOT / "target/exit-confirmed-short-refill"
COMBINED = HERE.parent / "2026-11-23-ordinary-completion-writer/combined"
ORIGINAL = ROOT / "target/ordinary-completion-writer/combined"
BROKER = ORIGINAL / "broker-artifacts"
BULK_SOURCE = ORIGINAL / "bulk-base/ordinary-v2"
BROKER_SHA256 = "25f1f9bb0bee23a14e93d0090a01f394f49ccea6515eb791ccd143dfd7942ba8"
ORDERS = (
    ("off-a", "zero-a", "refill", "off-b", "zero-b"),
    ("zero-b", "off-b", "refill", "zero-a", "off-a"),
    ("refill", "off-b", "zero-a", "zero-b", "off-a"),
    ("off-a", "zero-b", "zero-a", "off-b", "refill"),
    ("zero-a", "refill", "off-a", "zero-b", "off-b"),
    ("off-b", "zero-b", "off-a", "refill", "zero-a"),
)
ENDPOINTS = ("submit_to_terminal_ms", "commit_to_terminal_ms", "last_entry_ms",
             "last_first_write_ms", "last_second_write_ms")
SECONDARY = ("first_entry_ms", "entry_to_first_write_p99_ms", "first_write_to_second_p99_ms", "second_to_terminal_p99_ms",
             "terminal_tail_ms", "runner_cpu_ms", "peak_rss_bytes", "peak_connections",
             "peak_threads", "peak_processes")
FLOORS = dict.fromkeys(ENDPOINTS[:2], 8.0) | dict.fromkeys(ENDPOINTS[2:], 4.0)
ALLOWANCE = dict(first_entry_ms=1.0, entry_to_first_write_p99_ms=1.0, first_write_to_second_p99_ms=1.0,
                 second_to_terminal_p99_ms=1.0, terminal_tail_ms=1.0,
                 runner_cpu_ms=5.0, peak_rss_bytes=1024**2, peak_connections=0,
                 peak_threads=0, peak_processes=0)


def read(path):
    return json.loads(path.read_text())


def write(path, data):
    path.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n")


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def distribution(values):
    values = list(values)
    return dict(n=len(values), median=median(values), min=min(values), max=max(values))


def arm_result(folder, mode):
    sys.path.insert(0, str(COMBINED / "replay"))
    from proof import check

    load = lambda name: read(folder / name)
    row, ledger, raw, build, split = map(load, ("row.json", "ledger.json", "raw.json",
                                               "runner-build.json", "split-executables.json"))
    writer = "off" if mode.startswith("off") else "zero"
    stats = check(load, writer, dict(elf=build["files"]["mesh"],
                  manifest=sha(folder / "runner-build.json"), version=build["version"]), native=False)
    assert split["broker_sha256"] == BROKER_SHA256
    assert row["runners"] == row["peak_processes"] == 1 and row["peak_threads"] <= 12
    assert row["input_sha256"] == "ac0bcbf2465ea6d6807e8bfcdcfc3b1dacfa18b775e3f4c074257d66bfbba9d1"
    db = ledger["database"]
    assert db["max_connections"] == db["database_connection_limit"] == 20 and db["version"] == "17.11"
    assert all(db[key] is True for key in ("tls", "fsync", "full_page_writes", "synchronous_commit",
                                            "track_commit_timestamp"))
    log = (folder / "runner-0.log").read_text().splitlines()
    assert ("short refill enabled" in log) == (mode == "refill")
    report = next(json.loads(line) for line in reversed(log) if line.startswith('{"batch_contention_hints"'))
    assert report["last_error"] is None and report["drained"] and report["claimed"] == 128
    assert report["stale_acks"] == 0 and report["outcomes"]["completed"] == 128
    assert not any(value for key, value in report["outcomes"].items() if key != "completed")
    app = load("trusted-application-statements.json")
    for table in ("attempts", "effects"):
        stmt, = (item for item in app if item["query"].startswith("INSERT INTO public.mesh_jobs_parity_" + table))
        assert stmt["calls"] == stmt["rows"] == 128
    receipt, = ledger["transactions"]
    submit, commit = (receipt[key] for key in ("submitted_us", "committed_us"))
    entry, first, second, terminal = (sorted(job[key] for job in ledger["jobs"])
                                      for key in ("entered_us", "attempt_committed_us",
                                                  "effect_committed_us", "settled_us"))
    assert submit <= commit <= entry[0] and len({job["worker_pid"] for job in ledger["jobs"]}) == 1
    def p99(later, earlier):
        return sorted(job[later] - job[earlier] for job in ledger["jobs"])[126] / 1000

    queue = load("queue-policy.json")
    return dict(stats, runner_sha256=split["runner_sha256"], claim_calls=raw["raw_claim_calls"],
                claim_sql_ms=sum(query["execution_ms"] for query in queue["queue_statements"]
                                 if "FROM mesh_jobs.claim_admitted_v3(" in query["query"]),
                admitted_queries=queue["admitted_queries"],
                submit_to_terminal_ms=(terminal[-1] - submit) / 1000,
                commit_to_terminal_ms=(terminal[-1] - commit) / 1000,
                producer_ms=(commit - submit) / 1000, first_entry_ms=(entry[0] - commit) / 1000,
                last_entry_ms=(entry[-1] - commit) / 1000,
                last_first_write_ms=(first[-1] - commit) / 1000,
                last_second_write_ms=(second[-1] - commit) / 1000,
                entry_64_to_128_ms=(entry[-1] - entry[63]) / 1000,
                terminal_tail_ms=(terminal[-1] - second[-1]) / 1000,
                entry_to_first_write_p99_ms=p99("attempt_committed_us", "entered_us"),
                first_write_to_second_p99_ms=p99("effect_committed_us", "attempt_committed_us"),
                second_to_terminal_p99_ms=p99("settled_us", "effect_committed_us"),
                runner_cpu_ms=ledger["runner_cpu_seconds"] * 1000,
                **{key: row[key] for key in ("peak_rss_bytes", "peak_connections", "peak_threads",
                                             "peak_processes", "peak_host_load_average_1m")})


def summarize(out):
    groups, failures = [], {}
    for index, order in enumerate(ORDERS, 1):
        group = {}
        for mode in order:
            arm = out / f"{index:02}" / mode
            try:
                assert read(arm.with_suffix(".status.json"))["returncode"] == 0
                group[mode] = arm_result(arm, mode)
            except (AssertionError, OSError, ValueError, KeyError, StopIteration) as error:
                failures[f"{index:02}/{mode}"] = repr(error)
        groups.append(group)
    complete = [group for group in groups if len(group) == len(ORDERS[0])]
    hashes = {run["runner_sha256"] for group in complete for run in group.values()}
    summary = dict(groups=groups, failures=failures, complete_groups=len(complete), runner_sha256=sorted(hashes))
    if not complete:
        return summary | {"qualified": False}
    metrics = (*ENDPOINTS, *SECONDARY, "producer_ms", "entry_64_to_128_ms", "claim_calls",
               "claim_sql_ms", "admitted_queries", "terminal_xids", "peak_host_load_average_1m")
    summary["modes"] = {mode: {key: distribution(group[mode][key] for group in complete)
                              for key in metrics} for mode in ORDERS[0]}
    comparisons = {}
    for key in (*ENDPOINTS, *SECONDARY, "claim_calls", "entry_64_to_128_ms"):
        zero = [abs(g["zero-a"][key] - g["zero-b"][key]) for g in complete]
        off = [abs(g["off-a"][key] - g["off-b"][key]) for g in complete]
        noise = max(median(off), median(zero))
        gain = [(g["zero-a"][key] + g["zero-b"][key]) / 2 - g["refill"][key] for g in complete]
        wins = sum(g["refill"][key] < min(g["zero-a"][key], g["zero-b"][key]) for g in complete)
        result = dict(gain_ms_or_resource_units=distribution(gain), paired_gains=gain,
                      duplicate_zero_spread=distribution(zero), duplicate_off_spread=distribution(off),
                      wins_over_both_zero=wins)
        if key in ENDPOINTS:
            result["passes"] = wins >= 5 and median(gain) >= FLOORS[key] and median(gain) > noise
        elif key in SECONDARY:
            result["passes"] = median(gain) >= -max(noise, ALLOWANCE[key])
        comparisons[key] = result
    summary["comparisons"] = comparisons
    summary["qualified"] = len(complete) == len(ORDERS) and len(hashes) == 1 and all(
        comparisons[key]["passes"] for key in (*ENDPOINTS, *SECONDARY))
    return summary


def run_arm(out, runner, mode):
    env = dict(os.environ)
    for key in ("MESH_JOBS_COMPLETION_TRACE", "MESH_SHORT_REFILL", "JOBS_PARITY_MESH_EVENTS",
                "TRUSTED_COMPLETION_WRITER_MS", "JOBS_PARITY_STATUS_PATH"):
        env.pop(key, None)
    env.update(TMPDIR=str(SCRATCH / "tmp"), PG_BIN_DIR=env.get("PG_BIN_DIR", "/opt/homebrew/opt/postgresql@17/bin"),
               COMBINED_BULK_SOURCE=str(BULK_SOURCE), COMBINED_RUNNER_ARTIFACTS=str(runner),
               COMBINED_WRITER_MODE="off" if mode.startswith("off") else "zero",
               MESH_JOBS_SET_BASED_ENQUEUE="1", TRUSTED_ADMISSION_POLICY="success_cohort",
               NATIVE_JOBS_STATE=str(SCRATCH / "state"), NATIVE_JOBS_ARTIFACTS=str(BROKER),
               DURABLE_JOBS_PHASE="diagnostic")
    if mode == "refill":
        env["MESH_SHORT_REFILL"] = "1"
    started = time.monotonic()
    before = os.getloadavg()
    assert sha(BROKER / "mesh") == BROKER_SHA256
    with out.with_suffix(".log").open("xb") as log:
        result = subprocess.run([sys.executable, "-B", COMBINED / "replay/split_arm.py", "--local-smoke",
                                 "mesh", "short", out], env=env, stdout=log, stderr=subprocess.STDOUT)
    out.mkdir(exist_ok=True)
    build = read(runner / "mesh.json")
    assert sha(runner / "mesh") == build["files"]["mesh"]
    shutil.copyfile(runner / "mesh.json", out / "runner-build.json")
    write(out.with_suffix(".status.json"), dict(returncode=result.returncode, mode=mode,
          refill_enabled=mode == "refill", elapsed_s=time.monotonic() - started,
          load_before=before, load_after=os.getloadavg(), runner_manifest_sha256=sha(runner / "mesh.json")))
    print(out.relative_to(SCRATCH), result.returncode, flush=True)
    return result.returncode


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("probe", "timing", "summarize"))
    parser.add_argument("out", type=Path)
    parser.add_argument("--runner", type=Path)
    args = parser.parse_args()
    out = args.out.resolve()
    if args.action == "summarize":
        print(json.dumps(summarize(out), indent=2, sort_keys=True))
        return
    assert out.is_relative_to(SCRATCH) and args.runner.resolve().is_relative_to(SCRATCH)
    with (ROOT / "target/queue-performance.lock").open("r") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        if args.action == "probe":
            sys.exit(run_arm(out, args.runner.resolve(), "probe"))
        out.mkdir()
        write(out / "plan.json", dict(orders=ORDERS, floors_ms=FLOORS, secondary_allowance=ALLOWANCE,
              minimum_wins_over_both_zero=5, same_runner_rebuild_for_all=True))
        for index, order in enumerate(ORDERS, 1):
            group = out / f"{index:02}"
            group.mkdir()
            for mode in order:
                run_arm(group / mode, args.runner.resolve(), mode)
    write(out / "summary.json", summarize(out))


if __name__ == "__main__":
    main()
