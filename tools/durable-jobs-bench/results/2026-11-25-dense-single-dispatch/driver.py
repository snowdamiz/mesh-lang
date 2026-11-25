"""Reconcile a matched dispatcher pilot from raw receipts or run it under a controller grant."""
import argparse
from collections import Counter
from copy import deepcopy
import errno
import importlib.util
import inspect
import json
from pathlib import Path
import runpy
import subprocess
import sys
import tarfile

HERE = Path(__file__).resolve().parent
ORIGINAL = HERE.parent / "2026-11-24-dense-individual-hot-pool/logger-off-retry"
sys.path.insert(0, str(ORIGINAL))
import retry
from topology import BOUNDS

base, run, dense, require = retry.base, retry.run, retry.dense, retry.require
matrix = base.matrix
retry.configure()
PACKET = HERE / "timing.json"
ID = HERE.name
OUTPUT = HERE / "evidence/native"
LANES = {"stock-a": (2, 2), "single-four": (1, 4), "single-two": (1, 2), "stock-b": (2, 2)}
ORDER = list(LANES)
SCHEDULE = [dict(group=g, profile="noop", order=order, phases=["warmup", "measured"])
            for g, order in enumerate((ORDER, ORDER[::-1]), 1)]
CLAIM_BODY = "f2d4720701661a141bc3f8d6591b76dd607a19fb60a9c46826766d99df168f6c"
FROZEN_STAGES = base.stages


def check():
    packet = run.read_json(PACKET)
    require(packet["id"] == ID and packet["schedule"] == SCHEDULE
            and packet["source_sha"] == base.REV and packet["build_free"] is True,
            "pilot contract differs")
    for name, digest in packet["local_sha256"].items():
        require(run.digest(dense.ROOT / name) == digest, f"changed pinned input: {name}")
    binding = retry.relink_binding()
    setup = base.setup_raw(binding)
    release = run.read_json(retry.OUT / "setup/controller-release.json")
    require(setup == run.read_json(retry.OUT / "setup/verified.json")
            and release["exit_status"] == 0 and release["id"] == retry.ID + "-setup"
            and release["released_at"], "original logger-off SETUP did not release")
    require(packet["elf"] == binding["manifest"]["files"]["mesh"]
            and packet["manifest"] == binding["manifest_sha256"], "pilot uses the wrong public ELF")
    cfg = base.config(binding)
    historical = retry.OUT / "native"
    previous = run.read_json(historical / "series.json")
    remote = run.REMOTE / previous["remotes"]["hot"]
    _, inv = FROZEN_STAGES(cfg, "hot", remote)
    require(inv == run.read_json(historical / "staging/hot/inventory.json"), "archived hot inventory differs")
    # Independently replay a measured hot control with the unchanged scorer.
    group = run.read_json(ORIGINAL / "evidence/final-replay.json")["blocks"]["1"]
    original = historical / "blocks" / group["attempt"] / "measured-hot/measured-mesh"
    score = base.score_arm(original, "measured", "hot", cfg, remote)
    require(score == group["scores"]["measured-hot"], "historical hot replay differs")
    cfg["lanes"] = {label: dict(engine="mesh", runners=runners) for label, (runners, _) in LANES.items()}
    cfg["description"] = "Public dense hot individual-terminal fixed-total-four dispatcher pilot"
    cfg["dispatch_lanes"] = {label: list(settings) for label, settings in LANES.items()}
    return cfg, dict(packet["local_sha256"], request=run.digest(PACKET), setup=setup["raw_sha256"])


def stages(cfg, label, remote):
    files, inventory = FROZEN_STAGES(cfg, "hot", remote)
    inventory = deepcopy(inventory)
    spec = inventory["engines"]["mesh"]
    argv = spec["argv"]
    require(argv[:2] == ["/usr/bin/env", "HOT_CHECKOUT=1"] and argv[-1] == "mesh",
            "frozen hot invocation differs")
    extra = [HERE / "native-topology.sh", HERE / "topology.py"]
    spec["argv"] = [*argv[:4], "/bin/bash", str(remote / extra[0].name),
                    *map(str, LANES[label]), "mesh"]
    spec["verified_files"].update({str(remote / path.name): run.digest(path) for path in extra})
    run.check_inventory(inventory)
    return [*files, *extra], inventory


def public_score(get, log, phase, cfg, count):
    # Keep every original semantic assertion, adjusting only expected runner topology.
    frozen = retry.BASE_MESH
    source = inspect.getsource(frozen)
    for before, after in (
        ("row[\"peak_processes\"] == 2", 'row["peak_processes"] == count'),
        ("diagnostic_reconciler(2)", "reconciler"),
        ("len(pids) == 2", "len(pids) == count"),
        ("len(affinity) == 3", "len(affinity) == count + 1"),
        ('slots["limit"] == 32', 'slots["limit"] == 64 // count'),
        ('runner["pid"]]) <= 32', 'runner["pid"]]) <= 64 // count'),
        ('4 <= counts["parity_subject"] <= 8', 'counts["parity_subject"] == 4 * count'),
        ("confined to two live runners", "confined to the live runners"),
    ):
        require(source.count(before) == 1, f"original scorer anchor differs: {before}")
        source = source.replace(before, after)
    spec = importlib.util.spec_from_file_location("dispatch_verify", dense.SUITE / "verify.py")
    verify = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(verify)
    verify.PROFILES = dict(verify.PROFILES, noop=(count, *verify.PROFILES["noop"][1:]))
    scope = dict(frozen.__globals__, count=count, reconciler=verify.reconcile)
    exec(compile(source, frozen.__code__.co_filename, "exec"), scope)
    for index in range(count):
        retry.verify_runner_log(log(f"runner-{index}.log"))
    raw = get("raw")
    require(raw["runner_event_buffer_bytes"] == [None] * count
            and raw["event_drops"] == [{"logger": None, "reader": None}] * count,
            "frozen harness observed a telemetry sink")
    return scope["mesh"](get, log, phase, True, cfg["product"], retry.ARTIFACT / "mesh")


def pg_cpu(observed, start, stop):
    before, after = observed["before"], observed["after"]
    require(before["begin_us"] <= before["end_us"] <= start < stop <= after["begin_us"] <= after["end_us"]
            and observed["clock_ticks"] == 100, "invalid PostgreSQL CPU bracket")
    pid = observed["postgres_pid"]

    def frames(frame):
        result = {row["pid"]: row for row in frame["rows"]}
        require(pid in result and len(result) == len(frame["rows"])
                and all(row["parent"] == pid for key, row in result.items() if key != pid),
                "PostgreSQL process roster differs")
        return result

    first, last = frames(before), frames(after)
    shared = {key for key in first.keys() & last.keys() if first[key]["start"] == last[key]["start"]}
    require(pid in shared, "postmaster changed across arm")
    ticks = [last[k]["cpu_ticks"] - first[k]["cpu_ticks"] for k in shared]
    require(min(ticks) >= 0, "PostgreSQL CPU ticks went backwards")
    slack_us = (start - before["begin_us"]) + (after["end_us"] - stop)
    return dict(cpu_ms=sum(ticks) * 10, paired_pids=sorted(shared),
                unmatched_pids=sorted((first.keys() | last.keys()) - shared),
                bracket_slack_ms=slack_us / 1000,
                cpu_floor_ms=max(0, sum(max(0, tick - 1) for tick in ticks) * 10 - 2 * slack_us / 1000))


def dispatcher_score(get, log, score, count, workers):
    observation = get("dispatcher-observation")
    broker, *runners = get("process-identity")
    expected = {row["pid"]: (row, workers) for row in runners}
    expected[broker["pid"]] = broker, 2
    for frame in ("ready", "terminal"):
        rows = observation[frame]
        require(len(rows) == count + 1 and {row["pid"] for row in rows} == expected.keys(),
                f"{frame}: scheduler PID inventory changed")
        for row in rows:
            launch, n = expected[row["pid"]]
            bounds = dict.fromkeys(BOUNDS, str(n))
            require(row["launch_bounds"] == bounds and row["cpus"] == [0, 1]
                    and row["start"] == launch["launch_start_ticks"] and row["parent"] == launch["parent_pid"]
                    and (row.get("live_bounds") == bounds or row.get("environ_errno") in (errno.EACCES, errno.EPERM))
                    and (row["pid"] == broker["pid"] or row["threads"] == n + 6),
                    f"{frame}: scheduler launch, live process or worker count differs for {row['pid']}")
    require(observation["ready"][0]["threads"] == observation["terminal"][0]["threads"],
            "broker thread count changed")
    ready = {row["backend"] for row in get("postgres-tls-sessions") if row["role"] == "compiled_caller"}
    applications = observation["applications"]
    require(len(applications) == 32 and {row["backend"] for row in applications} == ready
            and all(row["tls"] for row in applications), "application TLS backend roster changed")
    writers = [row["backend"] for row in applications if row["wrote"]]
    require(0 < len(writers) <= count * workers
            and all(score["first_submit_us"] <= row["started_us"] <= score["last_effect_us"]
                    for row in applications if row["wrote"]), "unexpected writing backends")
    reports = [json.loads(log(f"runner-{i}.log").splitlines()[-1]) for i in range(count)]
    namespace, = {report["namespace"] for report in reports}
    require([row for row in observation["queues"] if row[1] == "default"] == [[namespace, "default", 64, "active"]]
            and all(row[0] == namespace for row in observation["queues"]), "global queue scope or limit differs")
    require(dense.digest(observation["claim_body"].encode()) == CLAIM_BODY
            and set(observation["claim_config"]) == {"search_path=pg_catalog, pg_temp", "lock_timeout=100ms"},
            "live SQL claim lock or function differs from public source")

    jobs = get("ledger")["jobs"]
    app = get("application-records")
    for table, field in (("attempts", "attempt_xid"), ("effects", "effect_xid")):
        require(len(app[table]) == len(jobs)
                and {(row["engine"], row["namespace"], row["job_key"], row["xid"]) for row in app[table]}
                == {("mesh", namespace, row["job_key"], row[field]) for row in jobs},
                "application XIDs or work scope differs")
    occupancy = dense.matrix.mesh_helpers()["intervals"]
    sql = get("queue-policy")["queue_statements"]
    claim, = (row for row in sql if "FROM mesh_jobs.claim_batch_set_v1(" in row["query"])
    productive_floor = len({(row["worker_pid"], row["claimed_us"]) for row in jobs})
    require(productive_floor <= claim["calls"] and claim["rows"] >= len(jobs), "missing real grant work")
    return dict(
        runner_cpu_ms=score["runner_cpu_ms_per_job"] * len(jobs),
        pg_cpu=pg_cpu(observation, score["first_submit_us"], score["last_settlement_us"]),
        writer_backend_lower_bound=len(writers), writer_backends=sorted(writers),
        runner_threads=[row["threads"] for row in observation["ready"] if row["pid"] != broker["pid"]],
        broker_threads=observation["ready"][0]["threads"], queue_scope=namespace,
        lock_function_body_sha256=CLAIM_BODY, global_queue_capacity=64,
        claimed_to_terminal_peak=occupancy(jobs),
        claimed_to_terminal_peak_by_pid={
            str(pid): occupancy([row for row in jobs if row["worker_pid"] == pid]) for pid in expected if pid != broker["pid"]
        },
        claim_calls=claim["calls"], claim_rows=claim["rows"], productive_grant_lower_bound=productive_floor,
        contention_hints=sum(r["batch_contention_hints"] for r in reports),
        queue_statement_elapsed_ms=sum(row["execution_ms"] for row in sql),
        claim_statement_elapsed_ms=claim["execution_ms"],
        last_effect_from_commit_us=score["last_effect_us"] - score["first_commit_us"])


def score_arm(folder, phase, label, cfg, remote):
    count, workers = LANES[label]
    _, inventory = stages(cfg, label, remote)
    with tarfile.open(folder / "raw.tar.gz") as raw:
        get = lambda name: run.file_json(raw, f"adapter/{name}.json")
        log = lambda name: dense.text(raw, f"adapter/{name}")
        host = run.file_json(raw, "host.json")
        request = run.read_json(folder / "request.json")
        invocation = run.file_json(raw, "invocation.json")
        require(all(invocation[k] == request[k] == v for k, v in
                    dict(profile="noop", phase=phase, engine="mesh", attempt=request["attempt"]).items())
                and run.file_json(raw, "status.json")["exit_code"] == 0, "wrong or failed invocation")
        require([host["boot_id"], host["kernel"], host["pg_mount"]["maj:min"]] == cfg["host"]
                and host["affinity"] == [0, 1] and len(host["available_cpus"]) == 4
                and host["verified_files"] == inventory["engines"]["mesh"]["verified_files"], "host or pins differ")
        score = public_score(get, log, phase, cfg, count)
        score.update(dispatcher_score(get, log, score, count, workers))
        score.update(run.sample_window(host, [json.loads(line) for line in raw.extractfile("host-samples.jsonl")], score))
    return dict(score, **dense.host_cpu(folder, score, cfg["clock_ticks"]),
                raw_sha256=run.digest(folder / "raw.tar.gz"))


def replay_block(path, block, cfg, remotes):
    require(run.read_json(path / "order.json") == block, "recorded order differs")
    return {f"{phase}-{label}": score_arm(folder, phase, label, cfg, run.REMOTE / remotes[label])
            for phase, label, _, folder in matrix.paths(path, cfg, block)}


def clock(a, candidate, b):
    faster = min(a, b)
    gain = faster - candidate
    spread = abs(a - b)
    threshold = max(spread, 0.05 * faster)
    return dict(gain_us=gain, spread_us=spread, threshold_us=threshold, passes=gain > threshold)


def contrast(scores):
    a, c, d, b = (scores[f"measured-{label}"] for label in ORDER)
    clocks = {key: clock(a[key], c[key], b[key]) for key in ("submit_us", "drain_us")}
    earlier = c["last_effect_from_commit_us"] < min(a["last_effect_from_commit_us"], b["last_effect_from_commit_us"])
    runner_cpu = min(a["runner_cpu_ms"], b["runner_cpu_ms"]) - c["runner_cpu_ms"]
    claim_delta = min(a["claim_calls"], b["claim_calls"]) - c["claim_calls"]
    combined = lambda arm: arm["runner_cpu_ms"] + arm["pg_cpu"]["cpu_ms"]
    combined_delta = min(combined(a), combined(b)) - combined(c)
    return dict(clocks=clocks, earlier_last_effect=earlier, runner_cpu_saved_ms=runner_cpu,
                combined_bracket_cpu_saved_ms=combined_delta, claim_calls_reduced=claim_delta,
                runner_cpu_saved_share_of_historical_target=[runner_cpu / 512, runner_cpu / 337],
                single_four_minus_single_two={key: c[key] - d[key] for key in (
                    "submit_us", "drain_us", "last_effect_from_commit_us", "runner_cpu_ms", "claim_calls")},
                passes=earlier and runner_cpu > 0 and combined_delta > 0
                and all(row["passes"] for row in clocks.values()))


def summarize(directory, cfg, schedule, pins):
    series = run.read_json(directory / "series.json")
    require(all(series[k] == v for k, v in dict(config=cfg, schedule=schedule, pins=pins).items()),
            "series contract differs")
    blocks, missing, failures = {}, [], {}
    for block in schedule:
        selected = matrix.pick(directory, block, cfg, series["remotes"], failures)
        if selected is None:
            missing.append(block["group"])
        else:
            attempt, scores = selected
            starts = [row["first_submit_us"] for row in scores.values()]
            ends = [row["last_settlement_us"] for row in scores.values()]
            require(all(end < start for end, start in zip(ends, starts[1:])), "arm windows overlap or reorder")
            blocks[str(block["group"])] = dict(attempt=attempt, scores=scores, contrast=contrast(scores))
    return dict(blocks=blocks, missing=missing, failures=failures,
                advance_to_peers=not missing and len(blocks) == 2
                and all(row["contrast"]["passes"] for row in blocks.values()))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("check", "timing", "replay", "intake"))
    parser.add_argument("seal", nargs="?")
    args = parser.parse_args()
    cfg, pins = check()
    if args.action == "check":
        print(json.dumps(dict(source=base.REV, elf=cfg["product"]["manifest"]["files"]["mesh"],
                              setup=pins["setup"], schedule=SCHEDULE, request=pins["request"]), indent=2))
        return 0
    if args.action == "intake":
        require(args.seal == pins["request"], "controller seal differs")
        require((matrix.QUEUE / f"{ID}.json").read_bytes() == PACKET.read_bytes(), "sealed intake missing")
        released = matrix.QUEUE / f"{ID}-groups-1-2.released.json"
        if released.exists():
            return run.read_json(released)["exit_status"]
        events = [line for line in (matrix.QUEUE / "STATUS.md").read_text().splitlines()
                  if line.startswith(("SETUP ", "TIMING ", "RELEASED ", "HOLD "))]
        require(events[-1].startswith("RELEASED "), "controller must release the previous grant first")
        packet = run.read_json(PACKET)
        packet.update(timing_grant_id=ID, evidence_destination=str(OUTPUT))
        helper = matrix.QUEUE.parent / "completion-writer-r2.py"
        return runpy.run_path(str(helper))["measure"](packet, "timing_command", [1, 2])

    matrix.stages, matrix.replay_block, matrix.summarize = stages, replay_block, summarize
    matrix.ERRORS += (AssertionError,)
    if args.action == "timing":
        base.grant(PACKET)
        subprocess.run(["bash", "-n", HERE / "native-topology.sh"], check=True)
        matrix.grant = lambda _: base.grant(PACKET)
        matrix.SERIES[ID] = ID
        return matrix.dispatch(OUTPUT, cfg, ID, SCHEDULE, pins, [1, 2])
    result = summarize(OUTPUT, cfg, SCHEDULE, pins)
    print(json.dumps(result, indent=2, sort_keys=True, allow_nan=False))
    return bool(result["missing"])


if __name__ == "__main__":
    sys.exit(main())
