"""Validate, dispatch through the sole Linode controller, or replay the WAL diagnostic."""
import argparse
from bisect import bisect_left, bisect_right
from copy import deepcopy
from datetime import datetime, timezone
from importlib import import_module
import json
from math import isfinite
from pathlib import Path
import re
import subprocess
import sys
import tarfile

from wal_capture import SETTINGS

HERE = Path(__file__).resolve().parent
PRIOR = HERE.parent / "2026-11-24-dense-two-runner-checkout"
PACKET = HERE / "controller-packet.json"
ID = HERE.name
RESULTS = HERE / "evidence/native"
ORDER = ("d2-on", "river-2x32", "oban-2x32", "apalis-2x32")
BLOCKS = [dict(group=i, profile="noop", phases=["warmup", "measured"], order=list(order))
          for i, order in enumerate((ORDER, ORDER[::-1]), 1)]
WAL = ("wal_records", "wal_fpi", "wal_bytes", "wal_buffers_full",
       "wal_write", "wal_sync", "wal_write_time", "wal_sync_time")
MOUNT = dict(fstype="ext4", source="/dev/sda", target="/", **{"maj:min": "8:0"})
sys.path.insert(0, str(PRIOR))
retry = import_module("import-retry")
dense = retry.dense
matrix, run, require = dense.matrix, dense.run, dense.require
EQUAL = dense.EQUAL
GRANTS = dense.QUEUE / "STATUS.md"


def stages(config, label, remote):
    files, inventory = retry.stages(config, label, remote)
    engine = config["lanes"][label]["engine"]
    spec = inventory["engines"][engine]
    if engine == "mesh":
        _, previous, mode, *args = spec["argv"]
        require(mode == "d2-on", "wrong R2 arm")
        del spec["verified_files"][previous]
        files = [path for path in files if path.name != Path(previous).name]
    elif engine == "oban":
        _, _, _, _, *args = spec["argv"]
        args.append("oban")
    else:
        _, _, _, original_engine, *args = spec["argv"]
        require(engine == original_engine, "wrong equal peer adapter")
    if engine != "mesh":
        files.append(EQUAL / "peer-arm.py")
    files.extend((HERE / "native-wal.sh", HERE / "wal_capture.py"))
    spec["argv"] = ["/bin/bash", str(remote / "native-wal.sh"), engine, *args]
    spec["verified_files"].update({str(remote / path.name): run.digest(path) for path in files})
    if "wal-*.json" not in spec["evidence_globs"]:
        spec["evidence_globs"] = [*spec["evidence_globs"], "wal-*.json", "wal-readback.log"]
    run.check_inventory(inventory)
    return files, inventory


def inputs():
    doc = run.read_json(PACKET)
    require(doc["id"] == ID and doc["controller"] == "linode-native-stepchange-controller-v1"
            and doc["schedule"] == BLOCKS and doc["cpus"] == [0, 1]
            and doc["input_sha256"] == dense.INPUT, "diagnostic seal or schedule differs")
    for name, sha in doc["local_sha256"].items():
        require(run.digest(dense.ROOT / name) == sha, f"changed diagnostic input: {name}")
    original, _, old_pins = retry.inputs()
    config = deepcopy(original)
    config["lanes"] = {label: original["lanes"][label] for label in ORDER}
    require(config["host"] == doc["native_host"], "diagnostic host pin differs")
    config["description"] = "Dense R2 hot and stock peers with off-clock PostgreSQL WAL readback"
    return config, dict(old_pins, **doc["local_sha256"], request=run.digest(PACKET))


def grant(_=None):
    events = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD) (\S+) (\S+)(?: (\S+))?",
                        GRANTS.read_text(), re.MULTILINE)
    require(bool(events), "missing controller grant log")
    phase, ident, begin, end = events[-1]
    require((phase, ident) == ("TIMING", ID)
            and datetime.fromisoformat(begin) <= datetime.now(timezone.utc) < datetime.fromisoformat(end),
            "no active sole-controller TIMING grant")
    require(PACKET.read_bytes() == (dense.QUEUE / f"{ID}.json").read_bytes(), "intake seal differs")
    return dict(start=begin, end=end, request_sha256=run.digest(PACKET))


def check_stages(config):
    prior = run.read_json(retry.RESULTS / "series.json")
    evidence = {}
    for label in ORDER:
        engine = config["lanes"][label]["engine"]
        dest = run.REMOTE / prior["remotes"][label]
        old = run.read_json(retry.RESULTS / "staging" / label / "inventory.json")
        require(old == retry.stages(config, label, dest)[1], f"stock stage changed: {label}")
        files, edited = stages(config, label, dest)
        new_spec, old_spec = (inv["engines"][engine] for inv in (edited, old))
        require(all(new_spec[key] == old_spec[key]
                    for key in ("artifact_sha256", "version", "postgres_storage")),
                f"diagnostic replaced stock binary or storage: {label}")
        old_pins = dict(old_spec["verified_files"])
        if engine == "mesh":
            del old_pins[old_spec["argv"][1]]
        require(old_pins.items() <= new_spec["verified_files"].items(),
                f"diagnostic lost native file pins: {label}")
        require(all(new_spec["verified_files"][str(dest / path.name)] == run.digest(path)
                    for path in files), f"unsealed diagnostic stage: {label}")
        evidence[label] = dict(argv=new_spec["argv"], artifact_sha256=new_spec["artifact_sha256"],
                               postgres_storage=new_spec["postgres_storage"])
    subprocess.run(["bash", "-n", HERE / "native-wal.sh"], check=True)
    return evidence


def lsns(frame):
    return tuple((int(hi, 16) << 32) + int(lo, 16)
                 for name in ("flush", "write", "insert")
                 for hi, lo in (frame["lsn"][name].split("/"),))


def verify_wal(before, after, closure, score):
    restored = closure["restored"]
    before_f, after_f, restored_f = (sample["frame"] for sample in (before, after, restored))
    spans = [before["begin"], before["end"], after["begin"], after["end"],
             closure["stop_begin"], closure["stop_end"], restored["begin"], restored["end"]]
    for clock in ("epoch_us", "monotonic_ns"):
        values = [t[clock] for t in spans]
        require(values == sorted(values), f"invalid off-clock ordering: {clock}")
    submit, settled = score["first_submit_us"], score["last_settlement_us"]
    require(before["end"]["epoch_us"] <= submit < settled <= after["begin"]["epoch_us"],
            "diagnostic SQL overlapped benchmark clocks")
    require(before_f["postmaster_started_us"] == after_f["postmaster_started_us"]
            < closure["stop_end"]["epoch_us"] <= restored_f["postmaster_started_us"]
            and len({f["system_id"] for f in (before_f, after_f, restored_f)}) == 1
            and re.fullmatch(r"[a-f0-9]{64}", closure["stats_file_sha256"]),
            "clean readback does not identify the original cluster")
    for sample in (before, after, restored):
        frame = sample["frame"]
        require(frame["settings"] == SETTINGS and frame["observer_tls"] is True
                and frame["other_clients"] == frame["wal_senders"] == 0
                and sample["begin"]["epoch_us"] <= frame["observed_us"] <= sample["end"]["epoch_us"],
                "offline PostgreSQL frame settings, TLS, clients, or clocks differ")
        require(list(lsns(frame)) == sorted(lsns(frame)), "invalid insert/write/flush LSN")
    require(all(lo <= mid <= hi for lo, mid, hi in zip(
        lsns(before_f), lsns(after_f), lsns(restored_f), strict=True)), "WAL LSN reset or backwards")
    old, live, final = (f["wal"] for f in (before_f, after_f, restored_f))
    require(old["stats_reset"] == live["stats_reset"] == final["stats_reset"],
            "WAL statistics were reset or lost")
    for key in WAL:
        values = [r[key] for r in (old, live, final)]
        require(all(isfinite(v) and v >= 0 for v in values) and values == sorted(values),
                f"invalid or nonmonotonic PostgreSQL counter: {key}")
    # Reporting can lag either live read. The complete post-shutdown delta is
    # an upper bound on drain work, including any unpublished pre-window work.
    # Do not impute exact drain costs from either live snapshot.
    require(final["wal_sync"] > old["wal_sync"] and final["wal_sync_time"] > old["wal_sync_time"]
            and final["wal_write_time"] > old["wal_write_time"], "missing PostgreSQL WAL I/O charges")
    statements = after["statements"]
    statement_scope = "whole-workload top-level statements, partial classification"
    if statements["dealloc"] == 0:
        for name in ("attempt", "effect"):
            rows = [row for row in statements["rows"] if row["kind"] == name]
            if sum(row["calls"] for row in rows) != 1024 or sum(row["rows"] for row in rows) != 1024:
                break
        else:
            statement_scope = "whole-workload top-level application INSERTs verified"
    return dict(scope="whole workload, including warmup/setup and delayed publication plus shutdown/readback",
                timing_bounds=dict(pre_submit_overhang_ms=(submit - before["begin"]["epoch_us"]) / 1000,
                                   post_settlement_to_live_ms=(after["end"]["epoch_us"] - settled) / 1000,
                                   post_settlement_to_readback_ms=(restored["end"]["epoch_us"] - settled) / 1000),
                initial_counters=old, live_delta={k: live[k] - old[k] for k in WAL},
                whole_workload_delta={k: final[k] - old[k] for k in WAL},
                after_live_publication_shutdown_readback={k: final[k] - live[k] for k in WAL},
                post_live_lsn_advance=dict(zip(("flush", "write", "insert"),
                                              (b - a for a, b in zip(lsns(after_f), lsns(restored_f), strict=True)))),
                checkpoint_at_baseline=before_f["checkpoint"], checkpoint_at_live=after_f["checkpoint"],
                checkpoint_restored=restored_f["checkpoint"], statement_scope=statement_scope,
                statement_wal=statements)


def device(archive, score, shutdown_begin):
    with archive.extractfile("host-samples.jsonl") as stream:
        points = [json.loads(line) for line in stream]
    stamps = [point["epoch_us"] for point in points]
    counters = []
    for point in points:
        row, = [line.split() for line in point["diskstats"] if line.split()[:3] == ["8", "0", "sda"]]
        counters.append((int(row[18]), int(row[19])))
    require(stamps == sorted(set(stamps)) and all(a <= b for lo, hi in zip(counters, counters[1:])
                                                 for a, b in zip(lo, hi)), "device clock or counters moved back")
    start, end = score["first_commit_us"], score["last_settlement_us"]
    inner_start, inner_end = bisect_left(stamps, start), bisect_right(stamps, end) - 2
    outer_start, outer_end = bisect_right(stamps, start) - 2, bisect_left(stamps, end)
    require(0 <= outer_start < outer_end < len(stamps), "device samples do not enclose the drain")
    return dict(scope="whole ext4 device across enqueue commit to settlement",
                quantum_ms=1, inner=[b - a for a, b in zip(counters[inner_start], counters[inner_end])]
                if inner_start <= inner_end else [0, 0],
                outer=[b - a for a, b in zip(counters[outer_start], counters[outer_end])],
                fields=["completed_flushes", "flush_time_ms"],
                outer_may_include_shutdown=not (outer_end + 1 < len(stamps)
                                                 and stamps[outer_end + 1] <= shutdown_begin))


def score_arm(folder, phase, label, config, remote):
    _, inventory = stages(config, label, remote)
    engine = config["lanes"][label]["engine"]
    if label == "d2-on":
        score = dense.native_score.raw_score(folder, phase, label, config, inventory)
    else:
        score = dense.base_score(folder, engine, 2, phase, inventory, config)
    with tarfile.open(folder / "raw.tar.gz", "r:gz") as archive:
        require(score["pg_mount"] == MOUNT, "diagnostic disk differs")
        ledger = run.file_json(archive, "adapter/ledger.json")
        jobs = ledger["jobs"]
        require(len(jobs) == 1024 and len({j[key] for j in jobs
                for key in ("attempt_xid", "effect_xid")}) == 2048, "application XIDs are not independent")
        before, after, closure = [run.file_json(archive, "adapter/wal-" + name + ".json")
                                  for name in ("before", "live-after", "restored")]
        wal = verify_wal(before, after, closure, score)
        disks = device(archive, score, closure["stop_begin"]["epoch_us"])
        effects = sorted(job["effect_committed_us"] for job in jobs)
        middle_ms = (effects[921] - effects[102]) / 1000
    return dict(score, postgres_wal=wal, disk=disks, middle_write_ms=middle_ms,
                raw_sha256=run.digest(folder / "raw.tar.gz"))


def replay_block(path, block, config, remotes):
    require(run.read_json(path / "order.json") == block, "diagnostic order differs")
    return {f"{phase}-{label}": score_arm(folder, phase, label, config, run.REMOTE / remotes[label])
            for phase, label, _, folder in matrix.paths(path, config, block)}


def summary(directory, config, blocks, pins):
    series = run.read_json(directory / "series.json")
    require(all(series[k] == v for k, v in dict(config=config, schedule=blocks, pins=pins).items()),
            "diagnostic series stamp differs")
    failures, missing, groups = {}, [], {}
    for block in blocks:
        number = block["group"]
        selected = matrix.pick(directory, block, config, series["remotes"], failures)
        if not selected:
            missing.append(number)
            continue
        attempt, arms = selected
        stock = arms["measured-d2-on"]
        upper = stock["postgres_wal"]["whole_workload_delta"]
        sync_and_write = upper["wal_sync_time"] + upper["wal_write_time"]
        comparisons = {}
        for label in ORDER[1:]:
            peer = arms[f"measured-{label}"]
            gap = stock["middle_write_ms"] - peer["middle_write_ms"]
            comparisons[label] = dict(middle_gap_ms=gap, mesh_total_pg_wal_upper_ms=sync_and_write,
                                      entire_mesh_wal_upper_less_than_gap=0 < sync_and_write < gap,
                                      whole_workload_sync_count_gap=upper["wal_sync"] -
                                      peer["postgres_wal"]["whole_workload_delta"]["wal_sync"])
        groups[number] = dict(attempt=attempt, arms=arms, comparisons=comparisons)
    return dict(groups=groups, missing=missing, failures=failures,
                attribution="PG17 per-process instrumented WAL I/O sums, cluster-wide and off-clock bracketed. "
                "No per-role sync attribution or equivalent Linux flush mapping.")


def configure_matrix():
    matrix.stages, matrix.grant = stages, grant
    matrix.replay_block, matrix.summarize = replay_block, summary
    matrix.SERIES[ID] = ID


def timing(config, pins):
    grant()
    configure_matrix()
    stage = run.stage

    def stage_public(path, remote, log):
        stage(path, remote, log)
        if path.name in {"native-wal.sh", "wal_capture.py", "peer-arm.py", "native-arm.py", "identity.py"}:
            run.ssh(["chmod", "0644", remote / path.name], stdout=log, stderr=log, timeout=12).check_returncode()
    run.stage = stage_public

    dispatch = run.dispatch_arm

    def checked(path, remote, block, phase, engine):
        dispatch(path, remote, block, phase, engine)
        folder = path / f"{phase}-{engine}"
        require(run.read_json(folder / "dispatch.json").get("exit_code") == 0,
                f"review retained failed diagnostic arm: {folder}")
        score_arm(folder, phase, path.name.removeprefix(phase + "-"), config, remote)
    run.dispatch_arm = checked
    return matrix.dispatch(RESULTS, config, ID, BLOCKS, pins, [1, 2])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("check", "timing", "replay"))
    args = parser.parse_args()
    config, pins = inputs()
    if args.action == "check":
        print(json.dumps(check_stages(config), indent=2, sort_keys=True))
        return 0
    if args.action == "timing":
        return timing(config, pins)
    configure_matrix()
    result = summary(RESULTS, config, BLOCKS, pins)
    print(json.dumps(result, indent=2, sort_keys=True, allow_nan=False))
    return bool(result["missing"])


if __name__ == "__main__":
    sys.exit(main())
