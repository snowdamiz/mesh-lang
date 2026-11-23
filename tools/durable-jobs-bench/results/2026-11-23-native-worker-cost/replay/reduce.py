"""Reconcile diagnostics with the stock durable ledgers and clock windows."""

import argparse
from collections import Counter, defaultdict
from decimal import Decimal
import json
from pathlib import Path
import re
from statistics import median
import struct
import tarfile

from controls import dense_stages, distribution, reconcile, require


def frames_between(frames, start, end):
    start *= 1000
    end *= 1000
    before = [f for f in frames if f["real_after_ns"] <= start]
    after = [f for f in frames if f["real_before_ns"] >= end]
    inside = [f for f in frames if f["real_before_ns"] >= start and f["real_after_ns"] <= end]
    require(before and after, "missing enclosing counter frames")
    lower, upper = before[-1], after[0]
    edge_ms = [(start - lower["real_before_ns"]) / 1e6, (upper["real_after_ns"] - end) / 1e6]
    return lower, upper, inside, edge_ms


def host_cpu(lower, upper, inside, edges, tick_hz, keys):
    def deltas(a, b):
        return [sum(b["cpu"][k][i] - a["cpu"][k][i] for k in keys) for i in range(8)]

    def busy(ticks):
        return sum(ticks[i] for i in (0, 1, 2, 5, 6)) * 1000 / tick_hz

    outer = deltas(lower, upper)
    inner = deltas(inside[0], inside[-1]) if len(inside) > 1 else [0] * 8
    require(min(outer) >= 0 and min(inner) >= 0, "host counters regressed")
    cores = 4 if "cpu" in keys else len(keys)
    tick_margin_ms = cores * 2 * 1000 / tick_hz
    return {
        "outer_ticks": outer, "inner_ticks": inner,
        "busy_cpu_ms_with_edge_allowance": [
            max(0, busy(inner) - tick_margin_ms, busy(outer) - tick_margin_ms - cores * sum(edges)),
            busy(outer) + tick_margin_ms],
    }


def backends(lower, upper, inside, edges, roles, pinned_cpu):
    def group(a, b, members):
        full = []
        missing = []
        for pid in members:
            x, y = a["backends"].get(pid), b["backends"].get(pid)
            if not isinstance(x, list) or not isinstance(y, list) or y[0] != x[0]:
                missing.append(int(pid))
                continue
            require(y[1] >= x[1] > 0, f"invalid schedstat at PID {pid}")
            full.append((y[1] - x[1]) / 1e6)
        return sum(full), missing

    result = {}
    for role in sorted(set(roles.values())):
        pids = [pid for pid, entry in roles.items() if entry == role]
        outer, missing = group(lower, upper, pids)
        stable = [pid for pid in pids if int(pid) not in missing]
        if not stable:
            result[role] = {"ready_snapshot_pids": len(pids), "missing_at_edges": missing}
            continue
        inner, inner_missing = group(inside[0], inside[-1], stable) if len(inside) > 1 else (0, [])
        # The entire role ran within the same two CPU mask. Use that tighter
        # upper bound on combined edge padding across all observed PIDs.
        result[role] = {
            "ready_snapshot_pids": len(pids), "stable_pids": len(stable),
            "missing_at_edges": missing,
            "inner_cpu_ms": inner, "outer_cpu_ms": outer,
            "busy_cpu_ms_bounds": [max(0, inner if not inner_missing else 0,
                                       outer - min(len(stable), 2) * sum(edges)), outer],
        }
    accounted = [row["busy_cpu_ms_bounds"] for row in result.values() if "busy_cpu_ms_bounds" in row]
    require(sum(row[0] for row in accounted) <= pinned_cpu[1],
            "stable backend CPU exceeds host busy capacity")
    return result


def elapsed(rows, key):
    return distribution([row[key] for row in rows])


def waits(archive, prefix, ledger, frames, window, parents):
    # Clock brackets are sampled from the same Linux monotonic and wall clocks.
    # A common offset must satisfy first/last readings across the full arm.
    min_offset = max(f["real_before_ns"] - f["mono_after_ns"] for f in frames)
    max_offset = min(f["real_after_ns"] - f["mono_before_ns"] for f in frames)
    require(min_offset <= max_offset, "monotonic/epoch offsets are incompatible")
    offset = (min_offset + max_offset) // 2
    uncertainty_ns = (max_offset - min_offset + 1) // 2
    spawn = {}
    consume = {}
    parent_cpu = {}
    slot = struct.Struct("<Q12q")
    for filename in sorted(name for name in archive.getnames() if
                           re.fullmatch(re.escape(prefix) + r"/adapter/wait-\d+\.bin", name)):
        data = archive.extractfile(filename).read()
        version, runner, count, dropped, user_us, system_us = struct.unpack_from("<6Q", data)
        require(version == 3 and not dropped and len(data) == 48 + count * slot.size,
                f"malformed or dropped wait records in {filename}")
        require(runner not in parent_cpu and runner in parents, "wait file runner identity differs")
        parent_cpu[runner] = {"user_ms": user_us / 1000, "system_ms": system_us / 1000}
        for mono, event, pid, requested, status, flags, user, system, end, minflt, majflt, nv, niv in (
                slot.iter_unpack(data[48:])):
            require(pid > 0 and status == 0 and event in {1, 2},
                    f"unexpected or failed child operation in {filename}")
            owner = spawn if event == 1 else consume
            require(pid not in owner, f"duplicate spawn or consuming wait for {pid}")
            require(event != 2 or (requested == pid and flags in (0, 1)), "non-exact wait")
            owner[pid] = (runner, mono, end, user, system, minflt, majflt, nv, niv)
    witness = {job["worker_pid"]: job for job in ledger["jobs"]}
    require(len(parent_cpu) == ledger["runners"] and len(witness) == len(ledger["jobs"])
            and spawn.keys() == consume.keys() == witness.keys(), "spawn, wait and durable PIDs differ")
    rows = []
    for pid, job in witness.items():
        runner, start, spawned, *_ = spawn[pid]
        reaper, stop, _, user, system, minflt, majflt, nv, niv = consume[pid]
        epoch_start = (start + offset) / 1000
        epoch_stop = (stop + offset) / 1000
        tolerance = uncertainty_ns / 1000
        require(runner == reaper and start <= spawned <= stop
                and window[0] - tolerance <= epoch_start <= job["entered_us"] + 5000 + tolerance
                and job["effect_committed_us"] - tolerance <= epoch_stop <= job["settled_us"] + tolerance
                and min(user, system, minflt, majflt, nv, niv) >= 0,
                f"worker lifecycle outside ledger for PID {pid}")
        rows.append({"pid": pid, "runner": runner, "job_key": job["job_key"], "queue_id": job["id"],
                     "attempt": job["attempt"], "attempt_xid": job["attempt_xid"], "effect_xid": job["effect_xid"],
                     "settled_xid": job["settled_xid"], "spawn_ns": start, "reap_ns": stop,
                     "spawn_call_ms": (spawned - start) / 1e6,
                     "spawn_to_enter_ms": (job["entered_us"] - epoch_start) / 1000,
                     "child_span_ms": (stop - start) / 1e6,
                     "effect_commit_to_wait_ms": (epoch_stop - job["effect_committed_us"]) / 1000,
                     "wait_to_settlement_ms": (job["settled_us"] - epoch_stop) / 1000,
                     "user_cpu_ms": user / 1000, "system_cpu_ms": system / 1000,
                     "minor_faults": minflt, "major_faults": majflt,
                     "voluntary_switches": nv, "involuntary_switches": niv})
    keys = list(rows[0])[10:]
    exact_child_total = sum(row["user_cpu_ms"] + row["system_cpu_ms"] for row in rows)
    exact_parent_total = sum(sum(row.values()) for row in parent_cpu.values())
    lifecycle_total = ledger["runner_cpu_seconds"] * 1000
    # wait4 may also include runtimes or thread exit work after the destructor.
    require(exact_child_total + exact_parent_total <= lifecycle_total + 1,
            "per-child and self rusage exceed the official runner-tree wait4 total")
    return {"clock_offset_ns": offset, "clock_uncertainty_ns": uncertainty_ns,
            "child_total_cpu_ms": exact_child_total, "parent_cpu_at_destructor_ms": parent_cpu,
            "parent_total_at_destructor_ms": exact_parent_total,
            "runner_lifecycle_unmatched_ms": lifecycle_total - exact_child_total - exact_parent_total,
            "workers": len(rows), "per_runner_worker_count": Counter(row["runner"] for row in rows),
            "distribution": {key: elapsed(rows, key) for key in keys}, "jobs": rows}


CPU_HEADER = re.compile(
    r"^\s*(?P<comm>.*?)\s+(?P<pid>-?\d+)\s*/\s*(?P<tid>-?\d+)\s+\[\d+\]\s+"
    r"(?P<time>\d+\.\d+):\s+(?:(?P<period>\d+)\s+)?cpu-clock(?:[:/][a-z]*)?:\s*(?P<leaf>.*)$")
TOP_FRAME = re.compile(r"^\s*(?:0x)?[a-f0-9]+\s+(\S+).*?\(([^()]*(?:\([^()]*\)[^()]*)*)\)\s*$")
LIFE_HEADER = re.compile(r"^\s*.*?\s+(-?\d+)\s*/\s*(-?\d+)\s+\[\d+\]\s+"
                         r"(\d+\.\d+):\s+sched:sched_process_(\w+):\s*(.*)$")


def role_of(pid, comm, roles, parents, children):
    if pid in children:
        return "mesh_attempt_child"
    if pid in parents:
        return "runner"
    if str(pid) in roles:
        return roles[str(pid)]
    if comm.startswith("postgres"):
        return "other_postgres"
    return "other"


def lifecycle_events(lines, window_ns, children, parents):
    started = {}
    exited = defaultdict(list)
    counts = Counter()
    clones = Counter()
    failed = []
    for line in lines.splitlines():
        if "PERF_RECORD_LOST" in line or "LOST" in line[:20]:
            failed.append(line)
        match = LIFE_HEADER.match(line)
        if not match:
            continue
        pid, tid = map(int, match.groups()[:2])
        when, event, args = match.groups()[2:]
        ns = int(Decimal(when) * 1_000_000_000)
        if not window_ns[0] <= ns <= window_ns[1]:
            continue
        counts[event] += 1
        if event == "exec" and pid in children:
            started.setdefault(pid, []).append(ns)
        if event == "exit" and pid in children and tid == pid:
            exited[pid].append(ns)
        if event == "fork":
            if pid in children or pid in parents:
                clones["mesh_child" if pid in children else "runner"] += 1
    missing = [pid for pid in children if len(started.get(pid, [])) != 1 or len(exited.get(pid, [])) != 1]
    return {"events": counts, "stock_child_exec_exit_missing_pids": missing,
            "fork_tracepoint_by_creator_group": clones, "lost_records": failed}


def cpu_samples(lines, window_ns, roles, parents, children):
    counts = Counter()
    periods = Counter()
    leafs = defaultdict(Counter)
    buckets = defaultdict(Counter)
    thread_samples = defaultdict(Counter)
    failed = []
    record = None
    frames = []

    def finish():
        if not record:
            return
        ns = int(Decimal(record["time"]) * 1_000_000_000)
        if not window_ns[0] <= ns <= window_ns[1]:
            return
        comm, pid = record["comm"], int(record["pid"])
        role = "idle" if not pid else role_of(pid, comm, roles, parents, children)
        counts[role] += 1
        periods[role] += int(record["period"] or 0)
        symbol, dso = frames[0] if frames else ("[unresolved]", "")
        leafs[role][f"{symbol} ({dso})"] += 1
        thread_samples[role][comm] += 1
        kernel_names = " ".join(symbol for symbol, module in frames
                                if "kernel.kallsyms" in module or "vmlinux" in module)
        if not kernel_names:
            bucket = "user_or_unresolved"
        elif re.search(r"copy_process|kernel_clone|pthread_create|do_exit|exit_mm|mmput|"
                       r"exit_mmap|exit_group|do_execve|bprm_execve|load_elf_binary|"
                       r"alloc_thread_stack|free_thread_stack", kernel_names):
            bucket = "kernel_exec_clone_exit"
        elif re.search(r"filemap_fault|filemap_map_pages|do_read_fault", kernel_names):
            bucket = "kernel_file_backed_fault"
        elif re.search(r"do_anonymous_page|do_wp_page|wp_page_copy", kernel_names):
            bucket = "kernel_anon_or_cow_fault"
        elif re.search(r"handle_mm_fault|do_user_addr_fault|do_fault", kernel_names):
            bucket = "kernel_unresolved_fault"
        elif re.search(r"futex|try_to_wake_up|finish_task_switch|__schedule|ep_poll|poll_schedule",
                       kernel_names):
            bucket = "kernel_scheduler_sync"
        elif re.search(r"ext4_|xfs_|file_write|fsync|file_read|blk_mq|submit_bio|io_schedule",
                       kernel_names):
            bucket = "kernel_storage"
        else:
            bucket = "kernel_other"
        buckets[role][bucket] += 1

    for line in lines.splitlines():
        if "PERF_RECORD_LOST" in line or "LOST" in line[:20]:
            failed.append(line)
        match = CPU_HEADER.match(line)
        if match:
            finish()
            record = match
            frames = []
            if leaf := TOP_FRAME.search(match["leaf"]):
                frames.append(leaf.groups())
        elif record and (frame := TOP_FRAME.search(line)):
            frames.append(frame.groups())
    finish()
    return {"samples": counts, "period_ns": periods, "lost_records": failed,
            "kernel_stack_bucket_samples": dict(buckets),
            "top_leaves": {role: counter.most_common(30) for role, counter in leafs.items()},
            "thread_comm_samples": {role: counter.most_common() for role, counter in thread_samples.items()}}


def json_member(archive, filename):
    return json.load(archive.extractfile(filename))


def reduce_arm(archive, prefix, engine, spec, tick_hz):
    invocation = json_member(archive, prefix + "/invocation.json")
    status = json_member(archive, prefix + "/status.json")
    require(status["adapter_exit"] == 0 and invocation["engine"] == engine, "arm failed or mismatched")
    ledger = json_member(archive, prefix + "/adapter/ledger.json")
    score = reconcile(ledger, engine, "noop", {field: spec[field] for field in ("artifact_sha256", "version")})
    app = json_member(archive, prefix + "/adapter/application-records.json")
    for label, field in (("attempts", "attempt_xid"), ("effects", "effect_xid")):
        require({(j["job_key"], j[field]) for j in ledger["jobs"]} ==
                {(row["job_key"], row["xid"]) for row in app[label]}
                and len(app[label]) == score["completed"], f"app writes differ for {label}")
    stage = dense_stages(ledger, score)
    roles = status["backend_roles"]
    with archive.extractfile(prefix + "/frames.jsonl") as stream:
        frames = [json.loads(line) for line in stream]
    require(all(x["real_after_ns"] <= y["real_before_ns"] and x["mono_after_ns"] < y["mono_before_ns"]
                for x, y in zip(frames, frames[1:])), "counter frame clocks do not reconcile")
    drain = [score["first_commit_us"], score["last_settlement_us"]]
    windows = {}
    for name, (start, end) in (("drain", drain), ("middle80", stage["middle90_minus_10_us"])):
        lower, upper, inner, edges = frames_between(frames, start, end)
        pinned = host_cpu(lower, upper, inner, edges, tick_hz, ("cpu0", "cpu1"))
        windows[name] = {"epoch_us": [start, end], "wall_ms": (end - start) / 1000, "edge_ms": edges,
                         "pinned": pinned, "all_guest": host_cpu(lower, upper, inner, edges, tick_hz, ("cpu",)),
                         "stable_backends": backends(lower, upper, inner, edges, roles,
                                                     pinned["busy_cpu_ms_with_edge_allowance"])}
    peers = json_member(archive, prefix + "/adapter/process-affinity.json")
    require(len(peers) == ledger["runners"] + 1 and all(mask == [0, 1] for mask in peers.values()),
            "runner/PostgreSQL affinity changed")
    # Mesh wait headers authenticate which of these PIDs are runners. Oban's
    # archived emulator report includes the runtime process, which may be a
    # descendant of the wrapper. Keep unresolved samples in the raw receipt.
    parents = {int(pid) for pid, role in roles.items() if role == "engine_runner_root"}
    if not parents:
        parents = {int(pid) for pid in peers}
    children = {row["worker_pid"] for row in ledger["jobs"]} if engine == "mesh" else set()
    result = {"score": score, "stages": stage, "windows": windows, "capture_issues": []}
    issues = result["capture_issues"]
    if invocation["mode"] == "on":
        if any(row.get("missing_at_edges") for w in windows.values() for row in w["stable_backends"].values()):
            issues.append("some ready-snapshot PostgreSQL PIDs are not stable across both windows")
        if not roles:
            issues.append("no PostgreSQL roles captured")
        if sum(role == "engine_runner_root" for role in roles.values()) != ledger["runners"]:
            issues.append("could not resolve all runner CPU clocks")
        if engine == "mesh":
            if invocation["reap_shim"]:
                try:
                    heard = waits(archive, prefix, ledger, frames, drain, parents)
                    result["exact_child_waits"] = heard
                    parents = set(heard["per_runner_worker_count"])
                except (ValueError, KeyError, struct.error) as error:
                    issues.append(f"exact waits rejected: {error}")
            else:
                issues.append("exact wait observer unavailable")
        # Child attempts are disjoint from the stable role estimates.
        for window in windows.values():
            accounted = sum(role["busy_cpu_ms_bounds"][0] for role in window["stable_backends"].values()
                            if "busy_cpu_ms_bounds" in role)
            maximum = window["pinned"]["busy_cpu_ms_with_edge_allowance"][1]
            exact = result.get("exact_child_waits")
            if exact and window is windows["drain"]:
                accounted += exact["child_total_cpu_ms"]
            require(accounted <= maximum + 2, "disjoint-role CPU exceeds the pinned host busy budget")
            window["unattributed_pinned_busy_cpu_ms_upper"] = max(0, maximum - accounted)
        for label in ("cpu", "lifecycle"):
            path = f"{prefix}/{label}"
            try:
                state = json_member(archive, path + ".status.json")
                raw = archive.extractfile(path + ".perf.txt").read().decode()
                require(state["script"] == 0 and state["record"] in (0, 130, -2), "perf collector failed")
                if "exact_child_waits" in result:
                    offset = result["exact_child_waits"]["clock_offset_ns"]
                else:
                    offset = median(f["real_before_ns"] - f["mono_before_ns"] for f in frames)
                for name, window in windows.items():
                    when = [int(us * 1000 - offset) for us in window["epoch_us"]]
                    if label == "cpu":
                        window["cpu_profile"] = cpu_samples(raw, when, roles, parents, children)
                        require(sum(window["cpu_profile"]["samples"].values()) > 0
                                and not window["cpu_profile"]["lost_records"], "missing/lost CPU samples")
                    elif name == "drain":
                        window["lifecycle"] = lifecycle_events(raw, when, children, parents)
                        require(window["lifecycle"]["events"] and not window["lifecycle"]["lost_records"],
                                "missing/lost lifecycle records")
                        require(not window["lifecycle"]["stock_child_exec_exit_missing_pids"],
                                "perf exec/exit counts differ from durable Mesh worker PIDs")
            except (KeyError, ValueError, UnicodeError) as error:
                issues.append(f"{label} profile unavailable or rejected: {error}")
    return result


def analyze(evidence, pin):
    result = {"archive": str(evidence), "arms": {}, "observer_perturbation": {}}
    with tarfile.open(evidence, "r:gz") as archive:
        code = int(archive.extractfile("slot-exit.txt").read())
        prefixes = sorted({name.split("/")[0] for name in archive.getnames()
                           if re.match(r"^\d\d-(mesh|oban)-(on|off)/", name)})
        result["slot_exit"] = code
        if not prefixes:
            return result
        tick_hz = json_member(archive, "host.json")["clk_tck"]
        for prefix in prefixes:
            engine = prefix.split("-")[1]
            try:
                result["arms"][prefix] = reduce_arm(archive, prefix, engine, pin["engines"][engine], tick_hz)
            except (KeyError, ValueError, struct.error) as error:
                result["arms"][prefix] = {"rejected": str(error)}
    for engine in ("mesh", "oban"):
        rows = [(name, row) for name, row in result["arms"].items()
                if name.split("-")[1] == engine and "rejected" not in row]
        off = [row for name, row in rows if name.endswith("-off")]
        on = [row for name, row in rows if name.endswith("-on")]
        if len(off) != 2 or len(on) != 1:
            continue
        comparison = {}
        for window in ("drain", "middle80"):
            ranges = sorted(row["windows"][window]["wall_ms"] for row in off)
            duration = on[0]["windows"][window]["wall_ms"]
            middle = median(ranges)
            spread = (ranges[1] - ranges[0]) / middle
            delta = (duration - middle) / middle
            comparison[window] = {"off_range_ms": ranges, "on_ms": duration, "off_spread": spread,
                                  "on_delta_vs_off_midpoint": delta,
                                  "within_diagnostic_screen": spread <= .05 and abs(delta) <= .05}
        result["observer_perturbation"][engine] = comparison
    return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("evidence", type=Path)
    args = parser.parse_args()
    pin_file = Path(__file__).with_name("pinned.json")
    print(json.dumps(analyze(args.evidence, json.loads(pin_file.read_text())), indent=2, allow_nan=False))
