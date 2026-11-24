"""Reconcile native clocks with fixture commits and replay the original gates."""
import argparse
import base64
from collections import Counter
import gzip
import json
from pathlib import Path
from statistics import median
import sys

HERE = Path(__file__).resolve().parent
ROOT = next(p for p in HERE.parents if (p / "tools/durable-jobs-bench/verify.py").is_file())
RECOVERY = ROOT / "tools/durable-jobs-bench/results/2026-11-23-ordinary-native-cpu-recovery/replay"
sys.path.insert(0, str(RECOVERY))
from recover import MARGIN_NS, observer_window, phase_budget, verdict
from reduce import MAX_FRAME_NS, MAX_GAP_NS

PHASES = ("producer", "drain", "ack_drain", "full")
PG_ROLES = {"pg_jobs", "pg_application", "pg_producer_shared"}
ROLES = sorted(PG_ROLES | {"postmaster", "pg_background", "runner", "broker"})


def require(condition, message):
    if not condition:
        raise ValueError(message)


def read_serial(file):
    entries = {}
    label = None
    for line in file.read_text().splitlines():
        if line.startswith("ROLE_EVIDENCE_BEGIN "):
            require(label is None, "unfinished entry in serial")
            _, label, size = line.split()
            chunks = []
        elif line == "ROLE_EVIDENCE_END":
            require(label and label not in entries, "duplicate or unexpected serial entry")
            payload = "".join(chunks)
            require(len(payload) == int(size), f"truncated payload for {label}")
            entries[label] = json.loads(gzip.decompress(base64.b64decode(payload, validate=True)))
            label = None
        elif label:
            chunks.append(line)
    require(label is None, "serial evidence ended mid-record")
    return entries


def fixture_windows(record):
    ledger = record["ledger"]
    job_ids = list(range(1, 1025))
    tables = ("enqueued", "attempts", "effects", "settled")
    require(set(ledger) == set(tables), "missing fixture durable stage")
    prior = None
    for table in tables:
        rows = ledger[table]
        require([row[0] for row in rows] == job_ids
                and all(row[1].isdigit() and row[2] > 0 for row in rows), "fixture ledger gaps")
        if prior is not None:
            require(max(row[2] for row in prior) < min(row[2] for row in rows)
                    and {row[1] for row in prior}.isdisjoint(row[1] for row in rows),
                    "fixture commits overlapped")
        prior = rows
    submit = record["submit_us"]
    commit = max(row[2] for row in ledger["enqueued"])
    ack = record["ack_us"]
    terminal = max(row[2] for row in ledger["settled"])
    require(submit < commit <= ack < terminal, "fixture durable window order")
    return dict(zip(PHASES, ((submit, commit), (commit, terminal), (ack, terminal),
                            (submit, terminal)), strict=True))


def snapshot_id(row):
    return (row["pid"], row["start"], row["login"], row["tls"])


def roster(record):
    before, after = (record[key] for key in ("before", "after"))
    left = {row["pid"]: row for row in before["postgres"]["rows"]}
    right = {row["pid"]: row for row in after["postgres"]["rows"]}
    pre = {row["pid"]: row for row in before["roster"]}
    post = {row["pid"]: row for row in after["roster"]}
    common = {pid for pid in pre.keys() & post.keys()
              if pre[pid] == post[pid] and
              (pre[pid]["role"] not in PG_ROLES or snapshot_id(left[pid]) == snapshot_id(right[pid]))}
    reused = {pid for pid in left.keys() & right.keys()
              if snapshot_id(left[pid]) != snapshot_id(right[pid]) or
              (pid in pre and pid in post and pre[pid]["start"] != post[pid]["start"])}
    late = {pid for pid in right if pid not in left} | reused
    retired = {pid for pid in left if pid not in right} | reused
    unknown = set(map(int, before["unknown"])) | set(map(int, after["unknown"]))
    # The one terminal ledger connection is known and opens after the
    # closing census. A positive remainder can include pending pre stats.
    db = (before["postgres"], after["postgres"], record["after_close"])
    consistent = all((d["postmaster"], d["stats_reset"]) ==
                     (db[0]["postmaster"], db[0]["stats_reset"]) for d in db)
    unexplained_sessions = db[2]["sessions"] - db[0]["sessions"] - len(late) - 1 if consistent else None
    expect = Counter({"pg_jobs": 32 if record["db"] == "mesh" else 80,
                      "pg_producer_shared": 1, "runner": 8, "broker": 1, "postmaster": 1})
    if record["db"] == "mesh":
        expect["pg_application"] = 32
    candidates = Counter(row["role"] for pid, row in pre.items() if pid in common)
    counts_match = candidates == expect
    complete = (counts_match and unexplained_sessions == 0 and not db[2]["rows"] and
                not any((late, retired, unknown, before["errors"], after["errors"])))
    return common, {
        "stable_candidates": dict(candidates), "expected": dict(expect), "pre": dict(Counter(r["role"] for r in pre.values())),
        "post": dict(Counter(r["role"] for r in post.values())),
        "late_pids": sorted(late), "retired_pids": sorted(retired), "reused_pids": sorted(reused),
        "unknown_pids": sorted(unknown), "pre_open_errors": before["errors"], "post_open_errors": after["errors"],
        "unexplained_session_delta": unexplained_sessions, "clean_snapshot_screen": complete,
    }


def validate_samples(frames, size):
    previous = [None] * size
    dead = set()
    for frame in frames:
        require(len(frame["process"]) == size, "trace column count differs from pinned roster")
        edge = frame["host_end"][2]
        for i, (begin, cpu, end, error) in enumerate(frame["process"]):
            require(edge <= begin <= end <= frame["end"][0], "process read escaped monotonic bracket")
            require(error >= 0 and (not error or cpu == 0), "failed process read carries CPU")
            edge = end
            if error:
                dead.add(i)
                continue
            require(i not in dead, "process read recovered after lifetime or permission loss")
            require(previous[i] is None or previous[i] <= cpu, "process clock decreased")
            previous[i] = cpu


def bounds(record, windows):
    raw = record["frames"]
    pre = record["before"]["roster"]
    validate_samples(raw, len(pre))
    cpus = [f"cpu{cpu}" for cpu in record["cpus"]]
    frames = [{**frame, "host": {"begin": frame["host_begin"], "end": frame["host_end"],
                               "ticks": dict(zip(cpus, frame["ticks"], strict=True))},
               "reporter_thread_cpu_ns": frame["self_cpu"]} for frame in raw]
    stable, coverage = roster(record)
    widths = [(f["end"][2] - f["begin"][0]) / 1e6 for f in frames]
    gaps = [(b["begin"][0] - a["end"][2]) / 1e6 for a, b in zip(frames, frames[1:])]
    metrics = {"frames": len(frames), "median_frame_ms": median(widths), "max_frame_ms": max(widths),
               "max_gap_ms": max(gaps), "over_width": sum(width > MAX_FRAME_NS / 1e6 for width in widths),
               "over_gap": sum(gap > MAX_GAP_NS / 1e6 for gap in gaps),
               "recorder_interval_cpu_ms": (frames[-1]["self_cpu"] - frames[0]["self_cpu"]) / 1e6,
               "error_codes": dict(Counter(str(r[3]) for f in frames for r in f["process"] if r[3]))}
    budgets = {}
    for name, edges in windows.items():
        try:
            budgets[name] = window(record, frames, stable, coverage, cpus, edges)
        except ValueError as error:
            budgets[name] = {"error": str(error)}
    return {"cadence": metrics, "coverage": coverage, "budgets": budgets}


def window(record, frames, stable, coverage, cpus, edges):
    budget = phase_budget(frames, edges, record["hz"], cpus, 1024)
    start, end = (v * 1000 for v in edges)
    before = max(i for i, f in enumerate(frames) if f["end"][1] <= start - MARGIN_NS)
    after = min(i for i, f in enumerate(frames) if f["begin"][1] >= end + MARGIN_NS)
    outer = frames[before:after+1]
    inner = [f for f in outer if start + MARGIN_NS <= f["begin"][1] and f["end"][1] <= end - MARGIN_NS]
    scale = 1e-6 / 1024
    captured = {role: [] for role in ROLES}
    errors = []
    for i, row in enumerate(record["before"]["roster"]):
        if any(f["process"][i][3] for f in outer):
            errors.append(row["pid"])
            continue
        if row["pid"] not in stable:
            continue
        high = (outer[-1]["process"][i][1] - outer[0]["process"][i][1]) * scale
        low = (inner[-1]["process"][i][1] - inner[0]["process"][i][1]) * scale if len(inner) >= 2 else None
        captured[row["role"]].append((low, high))
    budget["process_errors"] = errors
    output = {}
    for role, pairs in captured.items():
        output[role] = None if not pairs or pairs[0][0] is None else [
            sum(pair[0] for pair in pairs), sum(pair[1] for pair in pairs)]
    total = [sum(pair[i] for pair in output.values() if pair is not None) for i in range(2)]
    busy = budget["host_cpu_ms_per_job"]["busy"]
    if total[0] > busy[1] + 1e-6:
        return {**budget, "error": "captured role CPU exceeds host busy upper bound",
                "contradiction": {"captured_inner_cpu_ms": total[0] * 1024,
                                  "host_upper_cpu_ms": busy[1] * 1024}}
    require(total[0] <= total[1], "inverted captured CPU interval")
    budget["captured_pid_cpu_ms_per_job"] = output
    budget["qualified_role_cpu_ms_per_job"] = dict.fromkeys(ROLES)
    budget["all_attributed_cpu_ms_per_job"] = total if len(inner) >= 2 else None
    budget["unattributed_busy_cpu_ms_per_job"] = [max(0, busy[0] - total[1]),
                                                 max(0, busy[1] - total[0])] if len(inner) >= 2 else None
    budget["snapshot_screen_only"] = coverage["clean_snapshot_screen"] and not errors and len(inner) >= 2
    try:
        budget["observer_cpu_ms"] = observer_window(frames, edges)
    except ValueError as error:
        budget["observer_cpu_ms"] = None
        budget["observer_cpu_error"] = str(error)
    return budget


def replay(entries):
    require(entries["platform"]["version"] == "170011" and entries["platform"]["tls"] == "t",
            "unexpected Linux PG TLS platform")
    rows = []
    controls = []
    for label, arm in entries.items():
        if label == "platform" or "ledger" not in arm:
            continue
        windows = fixture_windows(arm)
        phase_ms = {key: (end - start) / 1000 for key, (start, end) in windows.items()}
        row = {"label": label, "lane": arm["db"], "mode": arm["mode"], "phase_ms": phase_ms,
               "score": {"runner_cpu_ms_per_job": arm["runner_timed_cpu_ms_per_job"]}, "order": "forward",
               "boundaries_us": windows, "durable_rows_per_stage": 1024}
        if arm["mode"] == "on":
            require(arm["sampler"]["exit"] == 0, "sampler did not exit cleanly")
            row.update(bounds(arm, windows))
        if arm.get("force"):
            row["force"] = verify_forced(arm, row)
        else:
            controls.append(row)
        rows.append(row)
    comparisons = verdict(controls)
    for lane in comparisons.values():
        for pair in lane["pairs"]:
            for kind in ("probe", "control", "valid"):
                key = f"runner_wait4_{kind}"
                if key in pair:
                    pair[f"timed_client_cpu_{kind}"] = pair.pop(key)
            pair["whole_runner_wait4_valid"] = None
    for row in rows:
        row["timed_client_cpu_ms_per_job"] = row.pop("score")["runner_cpu_ms_per_job"]
    return {"platform": entries["platform"], "rows": rows, "control_gates": comparisons,
            "control_scope": "fixture elapsed and timed client root CPU only",
            "hold": "representative fixture, one order, original control gates failed"}


def verify_forced(arm, row):
    c = row["coverage"]
    ready = sum(s["login"] == "parity_subject" for s in arm["readiness"]["rows"])
    require(ready == 23 and c["pre"]["pg_jobs"] == 31, "post-readiness attachments missed")
    require(arm["retired"] in c["retired_pids"]
            and {arm["replacement"], arm["late"]} <= set(c["late_pids"]), "forced churn missed")
    require(arm["unclassified_pid"] in c["unknown_pids"], "unknown login classified")
    require(arm["ephemeral"] not in c["late_pids"] + c["retired_pids"] and
            c["unexplained_session_delta"] == 1, "transient session escaped count guard")
    require(row["cadence"]["error_codes"].get("3", 0) > 0 and not c["clean_snapshot_screen"],
            "exit did not invalidate role coverage")
    return dict(readiness_jobs=ready, missing_transient_cpu=None,
                **{name: arm[name] for name in ("retired", "replacement", "late", "ephemeral", "unclassified_pid")})


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("serial", type=Path)
    args = parser.parse_args()
    print(json.dumps(replay(read_serial(args.serial)), indent=2, sort_keys=True))
