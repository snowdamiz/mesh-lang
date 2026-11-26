#!/usr/bin/env python3
"""Run and reconcile matched frozen 128-job short arms in private TLS clusters."""
import argparse
from collections import Counter, defaultdict
import fcntl
import hashlib
import json
import os
from pathlib import Path
from statistics import median
import subprocess
import sys
import time

import verify

RESULTS = Path(__file__).resolve().parents[1]
ROOT = RESULTS.parents[3]
LANE = ROOT / "target/ordinary-completion-writer"
V2 = ROOT / "target/ordinary-trusted-mesh-queue-v2"
SOURCE = Path(os.environ.get("COMPLETION_WRITER_SOURCE", LANE / "source"))
CANDIDATE_ARTIFACTS = Path(os.environ.get("COMPLETION_WRITER_ARTIFACTS", LANE / "artifacts"))
V2_ARTIFACTS = Path(os.environ.get("COMPLETION_WRITER_V2_ARTIFACTS", V2 / "local-artifacts"))
PROFILES = SOURCE / "scripts/jobs-parity-profiles.json"
ARM = SOURCE / "tools/durable-jobs-bench/native/arm.py"
MODES = {"v2": None, "off": None, "zero": 0, "hold": 1, "off-repeat": None, "scalar": None}
ORDER = ["v2", "off", "zero", "hold", "off-repeat"]
PREFIX = "completion_writer "
V2_VERSION = "3828f20d2d6beb683868cce33e6dd1958365914b"
WRITER_VERSION = hashlib.sha256(Path(__file__).with_name("completion-writer.patch").read_bytes()).hexdigest()[:40]


def read(path):
    return json.loads(path.read_text())


def save(path, value):
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")


def describe(values):
    return dict(n=len(values), median=median(values), min=min(values), max=max(values))


def peak_occupancy(windows):
    active = peak = 0
    for _, delta in sorted((stamp, delta) for enter, settle in windows
                           for stamp, delta in ((enter, 1), (settle, -1))):
        active += delta
        peak = max(active, peak)
    return peak


def trace_stats(path, ledger, terminal):
    notes = [json.loads(line[len(PREFIX):])
             for line in (path / "runner-0.log").read_text().splitlines() if line.startswith(PREFIX)]
    if not notes:
        return None
    jobs = {job["id"]: job for job in ledger["jobs"]}
    seen, group_spreads, collection, handoff, io = set(), [], [], [], []
    for note in notes:
        assert note["handoff_us"] <= note["write_us"] <= note["response_us"]
        members = note["members"]
        assert 1 <= len(members) <= 8
        xids = {jobs[member["id"]]["settled_xid"] for member in members}
        assert len(xids) == 1
        assert len(members) == terminal[next(iter(xids))]
        for member in members:
            assert member["id"] not in seen and member["result"] == "completed"
            seen.add(member["id"])
        times = [member["ready_us"] for member in members]
        assert max(times) <= note["handoff_us"]
        group_spreads.append(max(times) - min(times))
        collection.append(note["handoff_us"] - min(times))
        handoff.append(note["write_us"] - note["handoff_us"])
        io.append(note["response_us"] - note["write_us"])
    assert seen == set(jobs)
    return dict(groups=len(notes), ready_spread_us=describe(group_spreads),
                oldest_return_to_handoff_us=describe(collection),
                handoff_to_write_us=describe(handoff), write_to_confirm_us=describe(io))


def diagnostic_occupancy(path, ledger):
    transitions = path / "committed-transitions.json"
    if not transitions.exists():
        return None
    jobs = {row["id"]: row for row in ledger["jobs"]}
    per_job = defaultdict(list)
    for row in read(transitions):
        per_job[row["job"]].append(row)
    assert set(per_job) == set(jobs)
    windows = []
    for identity, (admitted, completed) in per_job.items():
        job = jobs[identity]
        assert (admitted["prior"], admitted["next"]) == ("available", "executing")
        assert (completed["prior"], completed["next"]) == ("executing", "completed")
        assert (completed["xid"], completed["committed_us"]) == (job["settled_xid"], job["settled_us"])
        assert admitted["committed_us"] <= job["entered_us"] <= completed["committed_us"]
        windows.append((admitted["committed_us"], completed["committed_us"]))
    peak = peak_occupancy(windows)
    assert 0 < peak <= 16
    return peak


def check(path):
    ledger, row, policy = (read(path / name) for name in
                           ("ledger.json", "row.json", "queue-policy.json"))
    build_path = path / "build.json"
    build = read(build_path)
    expected_version = V2_VERSION if path.name == "v2" else WRITER_VERSION
    assert build["version"] == row["version"] == expected_version
    if row["local_smoke"] and path.is_relative_to(RESULTS / "evidence"):
        manifest_path = V2 / "local-artifacts/mesh.json" if path.name == "v2" else RESULTS / "local-build.json"
        if manifest_path.exists():
            assert build == read(manifest_path)
            assert row["artifact_sha256"] == hashlib.sha256(manifest_path.read_bytes()).hexdigest()
    proof = verify.reconcile(ledger, "mesh", "short", {
        name: row[name] for name in ("version", "artifact_sha256")})
    expected_policy = "set_based" if path.name == "scalar" else "success_cohort"
    assert row["queue_policy"] == expected_policy and policy["admitted_queries"] > 0
    terminal = Counter(job["settled_xid"] for job in ledger["jobs"])
    limit = 1 if expected_policy == "set_based" else 8
    assert max(terminal.values()) <= limit and len(terminal) == policy["terminal_xids"]
    terminal_calls = [query for query in policy["queue_statements"]
                      if "mesh_jobs.complete_admitted_v3(" in query["query"]
                      or "mesh_jobs.settle_state(" in query["query"]]
    assert sum(query["rows"] for query in terminal_calls) == proof["completed"]
    assert sum(query["calls"] for query in terminal_calls) == len(terminal)
    for table in ("attempts", "effects"):
        counts = [q for q in read(path / "trusted-application-statements.json")
                  if q["query"].startswith("INSERT INTO public.mesh_jobs_parity_" + table)]
        assert len(counts) == 1 and counts[0]["calls"] == counts[0]["rows"] == proof["completed"]
    tls = Counter(session["role"] for session in read(path / "postgres-tls-sessions.json"))
    assert tls == {"parity_subject": 4, "compiled_caller": 8, "parity_producer": 1}, tls
    assert row["peak_connections"] == 13
    # The stock estimate uses attempted_at_ms from inside the claim transaction.
    # Entry-to-settlement is an observed lower bound on committed occupancy.
    occupancy = peak_occupancy((job["entered_us"], job["settled_us"]) for job in ledger["jobs"])
    assert 0 < occupancy <= 16 and 0 <= row["peak_executing"] <= 16
    return dict(**proof, queue_sql_per_job=row["runner_sql_calls_per_job"],
                terminal_xids=len(terminal), terminal_groups_by_size=Counter(terminal.values()),
                occupancy_lower_bound=occupancy,
                estimated_peak_executing=row["estimated_peak_executing"],
                physical_roles=tls,
                exact_diagnostic_occupancy=diagnostic_occupancy(path, ledger),
                trace=trace_stats(path, ledger, terminal))


def execute(out, orders, trace):
    out.mkdir()
    save(out / "plan.json", dict(orders=orders, trace=trace, cpu_count=os.cpu_count(),
                                host_scope="Darwin host, two Mesh schedulers per runner"))
    with (ROOT / "target/queue-performance.lock").open("r") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        for ordinal, order in enumerate(orders, 1):
            group = out / f"{ordinal:02}"
            group.mkdir()
            for mode in order:
                path = group / mode
                artifact = V2_ARTIFACTS if mode == "v2" else CANDIDATE_ARTIFACTS
                env = dict(os.environ, NATIVE_JOBS_STATE=str(LANE / "state"),
                           COMPLETION_WRITER_SOURCE=str(SOURCE),
                           NATIVE_JOBS_ARTIFACTS=str(artifact),
                           TRUSTED_ADMISSION_POLICY="set_based" if mode == "scalar" else "success_cohort")
                for option in ("TRUSTED_COMPLETION_WRITER_MS", "MESH_JOBS_COMPLETION_TRACE"):
                    env.pop(option, None)
                if MODES[mode] is not None:
                    env["TRUSTED_COMPLETION_WRITER_MS"] = str(MODES[mode])
                    if trace:
                        env["MESH_JOBS_COMPLETION_TRACE"] = "1"
                before = os.getloadavg()
                started = time.monotonic()
                with path.with_suffix(".log").open("xb") as log:
                    command = Path(__file__).with_name("diagnostic-arm.py") if trace else ARM
                    code = subprocess.run([sys.executable, "-B", command, "--local-smoke", "mesh",
                                           "short", path], env=env, stdout=log,
                                          stderr=subprocess.STDOUT, check=False).returncode
                save(path.with_suffix(".status.json"), dict(exit_code=code, load_before=before,
                     load_after=os.getloadavg(), elapsed_s=time.monotonic() - started))
                if code:
                    print(ordinal, mode, f"failed ({code})", flush=True)
                    continue
                try:
                    stats = check(path)
                    print(ordinal, mode, round(stats["commit_jobs_per_s"]), stats["terminal_xids"],
                          flush=True)
                except (AssertionError, OSError, ValueError, KeyError) as error:
                    print(ordinal, mode, repr(error), flush=True)
    return summarize(out)


def summarize(out):
    # Keep replay anchored to the untouched V2 frozen inputs when available.
    verify.ROOT = SOURCE if PROFILES.exists() else ROOT
    plan = read(out / "plan.json")
    groups, failures = {}, {}
    for ordinal, order in enumerate(plan["orders"], 1):
        group = {}
        for mode in order:
            try:
                path = out / f"{ordinal:02}" / mode
                group[mode] = check(path)
            except (AssertionError, OSError, ValueError, KeyError) as error:
                failures[f"{ordinal:02}/{mode}"] = repr(error)
        groups[ordinal] = group
    summary = {}
    for mode in {mode for group in groups.values() for mode in group}:
        arms = [group[mode] for group in groups.values() if mode in group]
        stats = {name: describe([arm[name] for arm in arms]) for name in (
            "commit_jobs_per_s", "submit_jobs_per_s", "terminal_xids", "queue_sql_per_job",
            "effect_commit_to_settlement_p99_ms", "peak_executing", "occupancy_lower_bound")}
        for metric in ("commit_jobs_per_s", "submit_jobs_per_s", "terminal_xids"):
            controls = {"hold": ("off", "off-repeat", "v2", "zero"),
                        "zero": ("off", "off-repeat", "v2"),
                        "off": ("v2", "off-repeat")}.get(mode, ())
            for control in controls:
                ratios = [group[mode][metric] / group[control][metric] for group in groups.values()
                          if mode in group and control in group]
                if ratios:
                    stats[f"{metric}_ratio_to_{control}"] = dict(**describe(ratios),
                                                               above_one=sum(r > 1 for r in ratios))
        summary[mode] = stats
    save(out / "summary.json", dict(plan=plan, groups=groups, summary=summary, failures=failures))
    return int(bool(failures))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("run", "summarize"))
    parser.add_argument("out", type=Path)
    parser.add_argument("--groups", default=5, type=int, choices=range(1, 6))
    parser.add_argument("--trace", action="store_true")
    parser.add_argument("--pilot", action="store_true")
    parser.add_argument("--scalar", action="store_true")
    args = parser.parse_args()
    verify.ROOT = SOURCE if PROFILES.exists() else ROOT
    if args.action == "summarize":
        sys.exit(summarize(args.out.resolve()))
    orders = [ORDER[index:] + ORDER[:index] for index in range(args.groups)]
    if args.trace:
        orders = [["v2", "off", "zero", "hold"]]
    if args.pilot:
        orders = [["v2", "off", "zero", "hold", "off-repeat"]]
    if args.scalar:
        orders = [["scalar"]]
    sys.exit(execute(args.out.resolve(), orders, args.trace or args.scalar))
