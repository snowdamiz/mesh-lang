"""Replay canonical ledger and conservative phase brackets for the role diagnostic."""
from collections import Counter, defaultdict
from pathlib import Path
import json
from statistics import median
import sys
import tarfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(ROOT / "tools/durable-jobs-bench"))
import run

HOST_FIELDS = {"busy": (0, 1, 2, 5, 6), "idle": (3,), "iowait": (4,), "steal": (7,)}
CLOCK_TOLERANCE_NS = 1_000_000
MAX_FRAME_NS = 25_000_000
MAX_GAP_NS = 100_000_000
MAX_EDGE_NS = 100_000_000


def phase_times(ledger):
    transactions = ledger["transactions"]
    submit = min(tx["submitted_us"] for tx in transactions)
    commit = max(tx["committed_us"] for tx in transactions)
    ack = max(tx.get("returned_us", tx.get("acknowledged_us")) for tx in transactions)
    terminal = max(job["settled_us"] for job in ledger["jobs"])
    run.require(submit < commit <= ack < terminal, "ledger phase clocks are inconsistent")
    return {"producer": (submit, commit), "drain": (commit, terminal),
            "ack_drain": (ack, terminal), "full": (submit, terminal)}


def clock_check(frames):
    stamps = [stamp for frame in frames for stamp in (frame["begin"], frame["host"]["begin"],
              frame["host"]["end"], frame["end"])]
    run.require(all(0 <= stamp[2] - stamp[0] <= CLOCK_TOLERANCE_NS for stamp in stamps),
                "wall clock read is not bounded by monotonic reads")
    run.require(all(left[2] <= right[0] and left[1] <= right[1]
                    for left, right in zip(stamps, stamps[1:])), "capture clocks moved backwards")
    run.require(max(s[1] - s[2] for s in stamps)
                <= min(s[1] - s[0] for s in stamps) + CLOCK_TOLERANCE_NS,
                "monotonic-to-realtime offsets disagree")
    run.require(all(f["end"][2] - f["begin"][0] <= MAX_FRAME_NS for f in frames),
                "frame spans over 25 ms in the phase bracket")
    run.require(all(right["begin"][0] - left["end"][2] <= MAX_GAP_NS
                    for left, right in zip(frames, frames[1:])), "missed frame in the phase bracket")


def frame_bounds(frames, start_us, end_us):
    start, end = start_us * 1000, end_us * 1000
    # Include uncertainty of the microsecond ledger timestamps and the
    # accepted monotonic/realtime mapping.
    margin = CLOCK_TOLERANCE_NS + 1000
    before = [i for i, frame in enumerate(frames) if frame["end"][1] <= start - margin]
    after = [i for i, frame in enumerate(frames) if frame["begin"][1] >= end + margin]
    run.require(before and after and before[-1] < after[0], "phase has no enclosing complete frames")
    outer = frames[before[-1]:after[0] + 1]
    clock_check(outer)
    inner = [frame for frame in outer if start + margin <= frame["begin"][1]
             and frame["end"][1] <= end - margin]
    outer_ns = [start - outer[0]["begin"][1], outer[-1]["end"][1] - end]
    inner_ns = ([inner[0]["end"][1] - start, end - inner[-1]["begin"][1]]
                if len(inner) >= 2 else None)
    run.require(max(outer_ns) <= MAX_EDGE_NS, "outer bracket exceeds 100 ms at an edge")
    if inner_ns:
        run.require(max(inner_ns) <= MAX_EDGE_NS, "inner bracket exceeds 100 ms at an edge")
    meta = {"outer_edge_ms": [value / 1e6 for value in outer_ns],
            "inner_edge_ms": [value / 1e6 for value in inner_ns] if inner_ns else None,
            "max_frame_ms": max(f["end"][2] - f["begin"][0] for f in outer) / 1e6,
            "frames": len(outer)}
    if "reporter_thread_cpu_ns" in outer[0]:
        deltas = [b["reporter_thread_cpu_ns"] - a["reporter_thread_cpu_ns"]
                  for a, b in zip(outer, outer[1:])]
        run.require(all(delta >= 0 for delta in deltas), "reporter CPU clock moved backwards")
        meta["bracket_reporter_cpu_ms"] = sum(deltas) / 1e6
    return outer, inner, meta


def tick_interval(pairs, hz):
    diffs = [end - start for start, end in pairs]
    run.require(all(delta >= 0 for delta in diffs), "CPU counter moved backwards")
    return sum(max(0, delta - 1) for delta in diffs) / hz, sum(delta + 1 for delta in diffs) / hz


def host_span(frames, fields, hz, cpus):
    if len(frames) < 2:
        return 0.0, 0.0
    before, after = (frame["host"]["ticks"] for frame in (frames[0], frames[-1]))
    run.require(set(before) == set(after) == set(cpus), "host CPU counter mask changed")
    return tick_interval([(before[cpu][field], after[cpu][field]) for cpu in cpus for field in fields], hz)


def process_span(frames, group, hz):
    if len(frames) < 2:
        return {"ticks_user": (0., 0.), "ticks_system": (0., 0.), "cpu_clock": (0., 0.)}
    before, after = (frame["processes"] for frame in (frames[0], frames[-1]))
    pids = [pid for pid, row in before.items() if row["role"] in group]
    result = {}
    for field in ("user", "system"):
        result[f"ticks_{field}"] = tick_interval([(before[pid][field], after[pid][field]) for pid in pids], hz)
    deltas = [after[pid]["cpu_ns"] - before[pid]["cpu_ns"] for pid in pids]
    run.require(all(delta >= 0 for delta in deltas), "process CPU clock moved backwards")
    precise = sum(deltas) / 1e9
    result["cpu_clock"] = (precise, precise)
    return result


def verify_frames(frames, registry, ledger):
    expected = {**registry["backend_roles"], **dict.fromkeys(registry["runner_pids"], "runner"),
                registry["broker_pid"]: "broker", registry["postmaster"]: "postmaster"}
    run.require(len(expected) == len(registry["backend_roles"]) + len(registry["runner_pids"]) + 2,
                "overlapping role membership")
    pinned = set(registry["runner_pids"])
    if ledger.get("execution_contract") == "ordinary-trusted-shared-process":
        used = {str(job["worker_pid"]) for job in ledger["jobs"]}
        run.require(bool(used) and used <= pinned, "ledger worker PID is outside the probed runners")
    identity = None
    previous = None
    for frame in frames:
        run.require("roster_error" not in frame and "processes" in frame,
                    f"invalid role frame: {frame.get('roster_error', 'missing roster')}")
        processes = frame["processes"]
        run.require({pid: row["role"] for pid, row in processes.items()} == expected,
                    "incomplete or changed role membership")
        current = {pid: (row["start"], row["parent"], row["comm"]) for pid, row in processes.items()}
        if identity is None:
            identity = current
        run.require(identity == current, "PID start time, parent, or process name changed")
        if previous is not None:
            run.require(all(row[field] >= previous[pid][field]
                            for pid, row in processes.items() for field in ("user", "system", "cpu_ns")),
                        "role CPU counter moved backwards between frames")
        previous = processes


def phase_budget(frames, edges, hz, cpus, jobs, registry=None, ledger=None):
    outer, inner, meta = frame_bounds(frames, *edges)
    wall_s = (edges[1] - edges[0]) / 1e6
    capacity = wall_s * len(cpus)

    def report(lo, hi):
        run.require(0 <= lo <= hi + 1e-8 and lo <= capacity + 1e-8,
                    "CPU lower bound contradicts interval or core capacity")
        return [lo * (1000 / jobs), min(hi, capacity) * (1000 / jobs)]

    mask = set(cpus)
    prior = None
    for frame in outer:
        ticks = frame["host"]["ticks"]
        run.require(set(ticks) == mask and all(len(cpu) == 8 for cpu in ticks.values()),
                    "host CPU counter shape or mask changed")
        if prior is not None:
            run.require(all(b >= a for cpu in mask for a, b in zip(prior[cpu], ticks[cpu], strict=True)),
                        "host CPU counter moved backwards between frames")
        prior = ticks
    host = {name: report(host_span(inner, fields, hz, cpus)[0], host_span(outer, fields, hz, cpus)[1])
            for name, fields in HOST_FIELDS.items()}
    scale = 1000 / jobs
    mid = {
        "duration_ms": wall_s * 1000, "boundary_epoch_us": list(edges), "brackets": meta,
        "capacity_ms_per_job": capacity * scale, "host_cpu_ms_per_job": host,
        "steal_pct_upper": 100 * host["steal"][1] / (capacity * scale),
        "iowait_pct_upper": 100 * host["iowait"][1] / (capacity * scale),
    }
    if registry is None:
        return mid
    verify_frames(outer, registry, ledger)
    isolated = {}
    groups = {role: {role} for role in {"runner", "broker", "postmaster", *registry["backend_roles"].values()}}
    groups["all_attributed"] = set(groups)
    for name, group in sorted(groups.items()):
        inside, outside = (process_span(frames, group, hz) for frames in (inner, outer))
        clock_low, clock_high = inside["cpu_clock"][0], outside["cpu_clock"][1]
        tick_low = inside["ticks_user"][0] + inside["ticks_system"][0]
        tick_high = outside["ticks_user"][1] + outside["ticks_system"][1]
        run.require(clock_low <= tick_high + 1e-8 and tick_low <= clock_high + 1e-8,
                    f"{name} independent process CPU counters disagree")
        isolated[name] = {
            **{field: report(inside[field][0], outside[field][1])
               for field in ("ticks_user", "ticks_system", "cpu_clock")},
            "total": report(max(clock_low, tick_low), min(clock_high, tick_high)),
        }
    known = isolated["all_attributed"]["total"]
    busy = host["busy"]
    run.require(known[0] <= busy[1] + 1e-6, "attributed process CPU exceeds host busy upper bound")
    mid["role_cpu_ms_per_job"] = isolated
    mid["unattributed_busy_cpu_ms_per_job"] = [max(0, busy[0] - known[1]), busy[1] - known[0]]
    return mid


def optional(archive, name):
    try:
        return run.file_json(archive, name)
    except KeyError:
        return None


def call_counts(archive, completed):
    raw = optional(archive, "adapter/raw.json")
    groups = {}
    if raw is not None:
        groups["measured_plus_terminal_poll"] = {
            key: {"calls": value, "calls_per_job": value / completed}
            for key, value in raw.items() if key.startswith("raw_") and key.endswith("_calls")}
        groups["dealloc"] = raw["statement_counter_deallocations"]
    application = optional(archive, "adapter/trusted-application-statements.json")
    if application is not None:
        groups["application"] = application
    peer = optional(archive, "adapter/counters.json")
    if peer is not None:
        groups["peer_raw_counters"] = peer
    return groups


def arm(folder, lane, config, phase, mode):
    engine = lane["engine"]
    score = run.score_arm(folder, {"profile": "noop"}, phase, engine, lane["inventory"])
    run.require([score["host_boot_id"], score["host_kernel"], score["pg_mount"]["maj:min"]] == config["host"],
                "host boot, kernel, or storage changed")
    with tarfile.open(folder / "raw.tar.gz", "r:gz") as archive:
        ledger = run.file_json(archive, "adapter/ledger.json")
        workers_with_jobs = None
        if ledger.get("execution_contract") == "ordinary-trusted-shared-process":
            used = {str(job["worker_pid"]) for job in ledger["jobs"]}
            masks = run.file_json(archive, "adapter/process-affinity.json")
            run.require(bool(used) and len(used) <= 8 and used <= masks.keys()
                        and len(masks) == 9 and all(mask == config["cpus"] for mask in masks.values()),
                        "ordinary ledger worker PID or published runner affinity changed")
            workers_with_jobs = len(used)
        phases = phase_times(ledger)
        if engine == "mesh" and all("claimed_us" in job for job in ledger["jobs"]):
            a, b = phases["drain"]
            # Mesh exposes millisecond-rounded claim events. The true start
            # can be as late as claim+999us, but always precedes handler entry.
            residence = sum(max(0, job["settled_us"] -
                            max(a, min(job["claimed_us"] + 999, job["entered_us"])))
                            for job in ledger["jobs"])
        else:
            residence = None
        jobs = score["completed"]
        row = {"mode": mode, "score": score, "raw_sha256": run.digest(folder / "raw.tar.gz"),
               "phase_boundaries_us": phases, "enqueue_receipts": ledger["transactions"],
               "phase_ms": {key: (b - a) / 1000 for key, (a, b) in phases.items()},
               "statement_counts": call_counts(archive, jobs)}
        if workers_with_jobs is not None:
            row["ordinary_runners_with_jobs"] = workers_with_jobs
        if residence is not None:
            row["drain_mean_occupied_slots_lower"] = residence / (b - a)
        if mode != "on":
            return row
        with archive.extractfile("role-frames.jsonl") as stream:
            header = json.loads(next(stream))
            frames = [json.loads(line) for line in stream]
        run.require(header["format"] == 1 and header["hz"] == config["clock_ticks"]
                    and header["cpus"] == config["cpus"] and header["spare_cpu"] not in config["cpus"]
                    and header["engine"] == engine, "recorder header differs from pins")
        run.require(all(a["begin"][1] <= b["begin"][1] and a["begin"][0] < b["begin"][0]
                        for a, b in zip(frames, frames[1:])), "trace time order changed")
        registry = None
        if engine in {"mesh", "oban"}:
            registry, = [frame["registry"] for frame in frames if "registry" in frame]
            roles = Counter(registry["backend_roles"].values())
            expected = ({"pg_jobs": 32, "pg_application": 32, "pg_producer": 1} if engine == "mesh"
                        else {"pg_jobs": 80, "pg_producer": 1})
            published = run.file_json(archive, "adapter/postgres-tls-sessions.json")
            from collector import ROLES
            run.require(len(published) == len(registry["backend_roles"])
                        and all(entry["tls"] is True for entry in published)
                        and {str(entry["backend"]): ROLES[entry["role"]] for entry in published}
                            == registry["backend_roles"], "recorded roles differ from adapter TLS census")
            masks = run.file_json(archive, "adapter/process-affinity.json")
            run.require(roles == expected and len(registry["runner_pids"]) == 8
                        and set(masks) == set(registry["runner_pids"]) | {registry["postmaster"]}
                        and all(mask == config["cpus"] for mask in masks.values()), "invalid role registry")
            if engine == "oban":
                broker = run.file_json(archive, "adapter/oban-broker-emulator.json")["pid"]
                run.require(str(broker) == registry["broker_pid"], "Oban broker PID differs from its report")
            else:
                witness = registry["broker_witness"]
                verified = lane["inventory"]["engines"][engine]["verified_files"]
                run.require(witness["argv"][0] in verified and witness["argv"][0].endswith("/mesh")
                            and witness["argv"][1:] == ["session"], "V1 broker was not the pinned session")
            row["registry"] = registry
        cpus = [f"cpu{cpu}" for cpu in config["cpus"]]
        row["budgets"] = {key: phase_budget(frames, edges, header["hz"], cpus, jobs, registry, ledger)
                          for key, edges in phases.items()}
        if registry:
            runner = row["budgets"]["full"]["role_cpu_ms_per_job"]["runner"]["total"]
            whole = score["runner_cpu_ms_per_job"]
            run.require(runner[0] <= whole + 1e-6, "runner phase lower bound exceeds whole runner wait4 rusage")
            row["runner_wait4_ms_per_job"] = whole
            row["runner_wait4_outside_full_phase_ms_per_job"] = [max(0, whole - runner[1]), whole - runner[0]]
    return row


def verdict(rows):
    """Use matched duplicates in both lane orders. No pooled pass can hide a bad pair."""
    comparison = {}
    for label in sorted({row["lane"] for row in rows}):
        pairs = []
        for order in ("forward", "reverse"):
            group = [row for row in rows if row["lane"] == label and row["order"] == order]
            if [row.get("mode") for row in group] != ["off", "on", "off"] or any("error" in row for row in group):
                pairs.append({"order": order, "valid": False, "reason": "incomplete or failed matched triple"})
                continue
            by_phase = {}
            for key in ("full", "drain", "producer"):
                off_before, on, off_after = (row["phase_ms"][key] for row in group)
                center = median((off_before, off_after))
                spread = abs(off_before - off_after) / center
                shift = (on - center) / center
                accepted = spread <= .10 and abs(shift) <= max(.05, spread / 2)
                by_phase[key] = {"control_ms": [off_before, off_after], "probe_ms": on,
                                 "control_spread_pct": 100 * spread, "probe_shift_pct": 100 * shift,
                                 "valid": accepted}
            rusage_before, rusage_on, rusage_after = (row["score"]["runner_cpu_ms_per_job"] for row in group)
            center = median((rusage_before, rusage_after))
            rusage_spread = abs(rusage_before - rusage_after) / center
            rusage_shift = (rusage_on - center) / center
            rusage_valid = rusage_spread <= .15 and abs(rusage_shift) <= max(.10, rusage_spread / 2)
            pairs.append({"order": order, "phases": by_phase, "runner_wait4_probe": rusage_on,
                          "runner_wait4_control": [rusage_before, rusage_after],
                          "runner_wait4_valid": rusage_valid,
                          "valid": rusage_valid and all(data["valid"] for data in by_phase.values())})
        comparison[label] = {"pairs": pairs, "valid": all(pair["valid"] for pair in pairs),
                             "drain_valid": all(pair.get("phases", {}).get("drain", {}).get("valid", False)
                                                and pair.get("runner_wait4_valid", False) for pair in pairs)}
    return comparison
