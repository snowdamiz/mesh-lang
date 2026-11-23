"""Replay native-02 workload scores, host windows and partial process evidence."""
import argparse
from collections import Counter, defaultdict
import json
from pathlib import Path
import sys
import tarfile

HERE = Path(__file__).resolve().parent
FROZEN = HERE.parents[1] / "2026-11-23-ordinary-native-cpu" / "replay"
sys.path.insert(0, str(FROZEN))
from collector import ROLES
from diagnose import LANES, config_for_replay, layout, replay as original_replay
from offline import host_interval
from reduce import (
    CLOCK_TOLERANCE_NS, HOST_FIELDS, call_counts, phase_budget, phase_times,
    run, verdict,
)

MARGIN_NS = CLOCK_TOLERANCE_NS + 1000
IDENTITY = ("start", "parent", "comm")


def lines(archive, name):
    with archive.extractfile(name) as stream:
        return [json.loads(line) for line in stream]


def validate_host_rows(rows, field, cpus):
    previous = None
    for row in rows:
        ticks = row[field]
        run.require(set(ticks) == set(cpus) and all(len(v) == 8 for v in ticks.values()),
                    "host CPU mask or counter shape changed")
        if previous is not None:
            run.require(all(b >= a for cpu in cpus
                            for a, b in zip(previous[cpu], ticks[cpu], strict=True)),
                        "host CPU counter moved backwards")
        previous = ticks


def sparse_budget(points, edges, hz, cpus, jobs):
    validate_host_rows(points, "cpu", cpus)
    bound = host_interval(points, edges, hz, cpus, jobs)
    elapsed = (edges[1] - edges[0]) / 1000
    return {
        "boundary_epoch_us": list(edges), "duration_ms": elapsed,
        "capacity_ms_per_job": bound["capacity_ms_per_job"],
        "host_cpu_ms_per_job": bound["host_ms_per_job"],
        "outer_edge_ms": bound["outer_edge_max_ms"], "inner_frames": bound["inner_frames"],
    }


def host_projection(frames, cpus):
    # The captured host stamps entirely enclose the /proc/stat operation.
    # Repeating an endpoint for the unused outer frame stamps is idempotent
    # for the original clock-order and mapping checks (using a point interval).
    hosts = []
    for frame in frames:
        host = frame["host"]
        begin, end = host["begin"], host["end"]
        run.require(frame["begin"][2] <= begin[0] <= end[2] <= frame["end"][0],
                    "host sample escaped its frame")
        hosts.append({"begin": [begin[0], begin[1], begin[0]], "host": host,
                      "end": [end[2], end[1], end[2]]})
    validate_host_rows([frame["host"] for frame in hosts], "ticks", cpus)
    return hosts


def verify_frame_order(frames):
    stamps = [stamp for frame in frames for stamp in (
        frame["begin"], frame["host"]["begin"], frame["host"]["end"], frame["end"])]
    run.require(all(0 <= s[2] - s[0] <= CLOCK_TOLERANCE_NS for s in stamps)
                and all(a[2] <= b[0] and a[1] <= b[1] for a, b in zip(stamps, stamps[1:])),
                "capture clocks moved backwards or have unresolved read latency")
    run.require(max(s[1] - s[2] for s in stamps)
                <= min(s[1] - s[0] for s in stamps) + CLOCK_TOLERANCE_NS,
                "monotonic-to-realtime offsets disagree")
    run.require(all(a["reporter_thread_cpu_ns"] <= b["reporter_thread_cpu_ns"]
                    for a, b in zip(frames, frames[1:])), "reporter CPU clock moved backwards")


def observer_window(frames, edges):
    start, end = (stamp * 1000 for stamp in edges)
    # The CPU read follows frame.end and precedes the next frame.begin.
    reporter = [frame["reporter_thread_cpu_ns"] for frame in frames]
    outer_left = [i for i in range(len(frames) - 1)
                  if frames[i + 1]["begin"][1] <= start - MARGIN_NS]
    outer_right = [i for i, frame in enumerate(frames) if frame["end"][1] >= end + MARGIN_NS]
    inner = [i for i in range(len(frames) - 1)
             if frames[i]["end"][1] >= start + MARGIN_NS
             and frames[i + 1]["begin"][1] <= end - MARGIN_NS]
    run.require(outer_left and outer_right, "observer CPU clock does not bracket the phase")
    lo = reporter[inner[-1]] - reporter[inner[0]] if len(inner) >= 2 else 0
    hi = reporter[outer_right[0]] - reporter[outer_left[-1]]
    run.require(0 <= lo <= hi, "invalid observer CPU interval")
    return [lo / 1e6, hi / 1e6]


def phase_host(phases, points, hz, cpus, jobs, frames=None):
    projection = host_projection(frames, cpus) if frames else None
    phases_out = {}
    for name, edges in phases.items():
        scaled_jobs = 820 if name == "settled_102_to_922" else jobs
        sparse = sparse_budget(points, edges, hz, cpus, scaled_jobs)
        decision = {"denominator_jobs": scaled_jobs, "canonical_sparse_host": sparse}
        if projection:
            fine = phase_budget(projection, edges, hz, cpus, scaled_jobs)
            overlap = {}
            for field in HOST_FIELDS:
                a, b = sparse["host_cpu_ms_per_job"][field], fine["host_cpu_ms_per_job"][field]
                clipped = [max(a[0], b[0]), min(a[1], b[1])]
                run.require(clipped[0] <= clipped[1], f"host captures disagree on {name} {field}")
                overlap[field] = clipped
            decision.update(independent_fine_host=fine, intersection_cpu_ms_per_job=overlap,
                            reporter_thread_cpu_ms=observer_window(frames, edges))
        phases_out[name] = decision
    return phases_out


def covered(frame, start, end):
    return frame["begin"][1] >= start + MARGIN_NS and frame["end"][1] <= end - MARGIN_NS


def archived_processes(frames, published, masks, broker, ledger_workers, cpus):
    registries = [frame["registry"] for frame in frames if "registry" in frame]
    run.require(len(registries) <= 1, "multiple frozen process registries")
    if not registries:
        run.require(not any(frame.get("processes") for frame in frames),
                    "process data exists without a sealed registry")
        return {}, {}, []
    registry, = registries
    run.require({str(row["backend"]): ROLES[row["role"]] for row in published}
                == registry["backend_roles"] and len(published) == len(registry["backend_roles"]),
                "role registry differs from the TLS census")
    runners = set(registry["runner_pids"])
    run.require(len(runners) == 8 and ledger_workers <= runners
                and set(masks) == runners | {registry["postmaster"]}
                and all(mask == cpus for mask in masks.values())
                and registry["broker_pid"] == str(broker), "participant witness mismatch")
    expected = {**registry["backend_roles"], **dict.fromkeys(runners, "runner"),
                registry["broker_pid"]: "broker", registry["postmaster"]: "postmaster"}
    run.require(len(expected) == len(published) + 10, "overlapping process roles")
    tracks = defaultdict(list)
    complete = []
    for frame in frames:
        if "roster_error" in frame or "processes" not in frame:
            # Failed frames can contain partially populated dictionaries.
            continue
        run.require({pid: row["role"] for pid, row in frame["processes"].items()} == expected,
                    "unexpected successful process roster")
        for pid, row in frame["processes"].items():
            if row["role"].startswith("pg_"):
                run.require(row["comm"] == "postgres" and row["threads"] == 1
                            and str(row["parent"]) == registry["postmaster"], "invalid client backend identity")
            if row["role"] in ("runner", "broker"):
                run.require(row["parent"] == registry["broker_witness"]["parent_pid"],
                            "invalid runner/broker parent")
            if tracks[pid]:
                prior = tracks[pid][-1][1]
                run.require(all(row[key] == prior[key] for key in IDENTITY)
                            and all(row[key] >= prior[key] for key in ("cpu_ns", "user", "system")),
                            "observed process changed identity or CPU moved backwards")
            tracks[pid].append((frame, row))
        complete.append(frame)
    return expected, tracks, complete


def captured_role_bounds(tracks, expected, complete, edges, jobs, host, hz, wait4, frames):
    start, end = (stamp * 1000 for stamp in edges)
    inside = [f for f in complete if covered(f, start, end)]
    left = [f for f in complete if f["end"][1] <= start - MARGIN_NS]
    right = [f for f in complete if f["begin"][1] >= end + MARGIN_NS]
    data = {}
    for pid, track in tracks.items():
        # The frame brackets each individual PID clock, so an interior
        # endpoint delta needs no interpolation even across missing frames.
        window = [(f, row) for f, row in track if covered(f, start, end)]
        before = [(f, row) for f, row in track if f["end"][1] <= start - MARGIN_NS]
        after = [(f, row) for f, row in track if f["begin"][1] >= end + MARGIN_NS]

        def interval(a, b):
            lo_ticks = sum(max(0, b[field] - a[field] - 1) for field in ("user", "system"))
            hi_ticks = sum(b[field] - a[field] + 1 for field in ("user", "system"))
            ticks_ms = [lo_ticks * 1000 / hz, hi_ticks * 1000 / hz]
            cpu_ms = (b["cpu_ns"] - a["cpu_ns"]) / 1e6
            return cpu_ms, ticks_ms

        lower = interval(window[0][1], window[-1][1]) if len(window) >= 2 else None
        upper = interval(before[-1][1], after[0][1]) if before and after else None
        if lower and upper:
            run.require(lower[0] <= upper[1][1] and lower[1][0] <= upper[0],
                        "independent process counters contradict the phase bracket")
        data[pid] = {"role": expected[pid], "start_ticks": track[0][1]["start"],
                     "inside_cpu_ms": lower[0] if lower else None,
                     "enclosing_cpu_ms": upper[0] if upper else None,
                     "inside_stat_ms": lower[1] if lower else None,
                     "enclosing_stat_ms": upper[1] if upper else None}
    groups = {}
    known_low = sum(row["inside_cpu_ms"] or 0 for row in data.values()) / jobs
    run.require(known_low <= host[1], "observed participants exceed pinned-host upper bound")
    for role in sorted(set(expected.values()) | {"runner", "broker", "postmaster",
                                                "pg_jobs", "pg_application", "pg_producer"}):
        rows = [row for row in data.values() if row["role"] == role]
        lo = sum(row["inside_cpu_ms"] or 0 for row in rows) / jobs
        hi = host[1] - (known_low - lo)
        sealed_upper = (sum(row["enclosing_cpu_ms"] for row in rows) / jobs
                        if rows and all(row["enclosing_cpu_ms"] is not None for row in rows) else None)
        if role == "runner":
            hi = min(hi, wait4)
        run.require(lo <= hi + 1e-8, "role lower bound exceeds the host residual or wait4")
        groups[role] = {
            "probed_participants": len(rows),
            "captured_pid_clock_lower_ms_per_job": lo if any(
                row["inside_cpu_ms"] is not None for row in rows) else None,
            "captured_pid_clock_upper_ms_per_job": sealed_upper,
            "whole_role_safe_bounds_ms_per_job": [lo, max(lo, hi)],
        }
    return {
        "complete_inside": len(inside),
        "complete_covering": bool(left and right),
        "inside_with_roster_error": sum("roster_error" in f and covered(f, start, end) for f in frames),
        "partial_frames_excluded": sum("roster_error" in f and "processes" in f for f in frames),
        "max_complete_inside_frame_ms": max(((f["end"][2] - f["begin"][0]) / 1e6
                                             for f in inside), default=None),
        "inner_edge_ms": ([(inside[0]["end"][1] - start) / 1e6,
                           (end - inside[-1]["begin"][1]) / 1e6] if len(inside) >= 2 else None),
        "outer_edge_ms": ([(start - left[-1]["begin"][1]) / 1e6,
                           (right[0]["end"][1] - end) / 1e6] if left and right else None),
        "known_pid_clock_lower_ms_per_job": known_low, "roles": groups, "pids": data,
    }


def recover_arm(raw, lane, mode, config, path):
    engine = lane["engine"]
    dispatch = run.read_json(raw / "dispatch.json")
    run.require(dispatch["exit_code"] == 0, f"native adapter failed: {path}")
    score = run.score_arm(raw, {"profile": "noop"}, "diagnostic", engine, lane["inventory"])
    run.require([score["host_boot_id"], score["host_kernel"], score["pg_mount"]["maj:min"]]
                == config["host"], "host boot, kernel or storage changed")
    cpus = [f"cpu{cpu}" for cpu in config["cpus"]]
    hz = config["clock_ticks"]
    with tarfile.open(raw / "raw.tar.gz") as archive:
        ledger = run.file_json(archive, "adapter/ledger.json")
        phases = phase_times(ledger)
        run.require(list(phases["full"]) == [score["first_submit_us"], score["last_settlement_us"]],
                    "reconciled ledger uses another full window")
        settlements = sorted(row["settled_us"] for row in ledger["jobs"])
        phases["settled_102_to_922"] = (settlements[101], settlements[921])
        points = lines(archive, "host-samples.jsonl")
        frames = None
        if mode == "on":
            header, *frames = lines(archive, "role-frames.jsonl")
            run.require(header == {"format": 1, "hz": hz, "spare_cpu": 2, "cpus": config["cpus"],
                                  "engine": engine, "period_ns": 20_000_000,
                                  "canonical_worker_sha256": run.digest(FROZEN / "canonical_worker.py")},
                        "recorder header differs from the sealed native-02 pins")
            verify_frame_order(frames)
        else:
            run.require("role-frames.jsonl" not in archive.getnames(), "control has role instrumentation")
        jobs = score["completed"]
        row = {"path": path, "mode": mode, "score": score, "dispatch": dispatch,
               "raw_sha256": run.digest(raw / "raw.tar.gz"),
               "phase_ms": {key: (b - a) / 1000 for key, (a, b) in phases.items()},
               "statement_counts": call_counts(archive, jobs),
               "host_windows": phase_host(phases, points, hz, cpus, jobs, frames)}
        if engine != "go":
            census = run.file_json(archive, "adapter/postgres-tls-sessions.json")
            run.require(all(item["tls"] is True for item in census)
                        and len({item["backend"] for item in census}) == len(census),
                        "invalid simultaneous TLS census")
            activity = run.file_json(archive, "adapter/raw.json")
            row["sessions"] = {
                "startup_tls_by_role": dict(Counter(item["role"] for item in census)),
                "canonical_later_idle_totals": [s["connections"] for s in activity["idle_samples"]],
                "canonical_measured_totals": [s["connections"] for s in activity["samples"]],
            }
        if frames:
            row["recorder"] = {
                "frames": len(frames),
                "elapsed_between_first_last_frame_ms": (frames[-1]["end"][2] - frames[0]["begin"][0]) / 1e6,
                "reporter_cpu_between_first_last_frames_ms":
                    (frames[-1]["reporter_thread_cpu_ns"] - frames[0]["reporter_thread_cpu_ns"]) / 1e6,
                "roster_errors": dict(Counter(frame["roster_error"] for frame in frames if "roster_error" in frame)),
                "complete_process_frames": sum("processes" in f and "roster_error" not in f for f in frames),
                "guard_error_offsets_from_settlement_ms": [
                    (f["begin"][1] - phases["full"][1] * 1000) / 1e6 for f in frames
                    if "live PostgreSQL client PIDs or role titles differ" in f.get("roster_error", "")
                ],
            }
            # Host decisions above are independent of the optional role join.
            expected, tracks, complete = {}, {}, []
            if engine != "go":
                masks = run.file_json(archive, "adapter/process-affinity.json")
                used = {str(job["worker_pid"]) for job in ledger["jobs"]}
                broker = (run.file_json(archive, "adapter/oban-broker-emulator.json")["pid"]
                          if engine == "oban" else None)
                expected, tracks, complete = archived_processes(
                    frames, census, masks, broker, used, config["cpus"])
                row["ledger_worker_pids"] = sorted(used)
            row["role_windows"] = {
                key: captured_role_bounds(
                    tracks, expected, complete, phases[key], window["denominator_jobs"],
                    window["intersection_cpu_ms_per_job"]["busy"], hz,
                    score["runner_cpu_ms_per_job"] * jobs / window["denominator_jobs"], frames)
                for key, window in row["host_windows"].items()
            }
        row["runner_lifetime_cpu_ms_per_job"] = score["runner_cpu_ms_per_job"]
        row["drain_busy_outside_runner_lower_ms_per_job"] = max(
            0, row["host_windows"]["drain"].get(
                "intersection_cpu_ms_per_job",
                row["host_windows"]["drain"]["canonical_sparse_host"]["host_cpu_ms_per_job"]
            )["busy"][0] - score["runner_cpu_ms_per_job"])
        return row


def replay(series):
    config = config_for_replay()
    request = run.read_json(series / "intake.json")["request"]
    pins = request["local_sha256"]
    for filename in ("canonical_worker.py", "collector.py", "diagnose.py", "inventory.json", "reduce.py"):
        run.require(run.digest(FROZEN / filename) == pins[f"target/ordinary-native-cpu/{filename}"],
                    f"frozen replay input changed: {filename}")
    for filename in ("run.py", "verify.py"):
        run.require(run.digest(run.SUITE / filename) == pins[f"tools/durable-jobs-bench/{filename}"],
                    f"canonical scorer source changed: {filename}")
    for label in LANES:
        for mode, source in (("on", "collector.py"), ("off", "canonical_worker.py")):
            stage = series / "staging" / label / mode
            run.require(run.digest(stage / "worker.py") == pins[f"target/ordinary-native-cpu/{source}"],
                        "staged worker differs from sealed source")
            if mode == "on":
                run.require(run.digest(stage / "canonical_worker.py")
                            == pins["target/ordinary-native-cpu/canonical_worker.py"],
                            "staged canonical worker differs from sealed source")
    original = original_replay(series, config)
    run.require(json.loads(json.dumps(original)) == run.read_json(series / "summary.json"),
                "original role decisions drifted")
    rows = []
    for order, label, mode, _, raw in layout(series, config):
        row = recover_arm(raw, config["lanes"][label], mode, config, str(raw.relative_to(series)))
        row.update(order=order, lane=label)
        rows.append(row)
    controls = verdict(rows)
    return {
        "qualification": original["controls"],
        "original_arm_decisions": [{k: row[k] for k in ("order", "lane", "mode", "path", "raw_sha256", "error")
                                    if k in row} for row in original["arms"]],
        "original_comparison": original["comparison"],
        "host_only_matched_controls_same_gates": controls,
        "arms": rows,
    }


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("series", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    run.write_json(args.output, replay(args.series))
