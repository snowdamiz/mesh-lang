"""Check, replay, or run the sealed dense scheduler pilot under the sole controller."""
import argparse
from copy import deepcopy
from datetime import datetime, timezone
from importlib import import_module
import json
from pathlib import Path
import re
import subprocess
import sys
import tarfile
import types

HERE = Path(__file__).resolve().parent
PRIOR = HERE.parent / "2026-11-24-dense-two-runner-checkout"
ID = HERE.name
PACKET = HERE / "controller-packet.json"
RESULTS = HERE / "evidence/native"
PREREQUISITE = HERE / "evidence/checkout-gate.json"
STAGE_PROOF = HERE / "evidence/linux-stage.json"
PILOT = [1, 2]
EXTENSION = [3, 4, 5, 6]
WORKERS = {"fixed2-a": 2, "fixed2-b": 2, "fixed4": 4}
ORDER = ["fixed2-a", "river-2x32", "fixed4", "oban-2x32", "fixed2-b", "apalis-2x32"]
CLOCKS = ("submit_ms", "drain_ms", "middle_820_ms")
DEFAULT_MOUNT = {"fstype": "ext4", "maj:min": "8:0", "source": "/dev/sda", "target": "/"}

sys.path.insert(0, str(PRIOR))
retry = import_module("import-retry")
dense = retry.dense
matrix, run, require = dense.matrix, dense.run, dense.require
DENSE_SCORE = PRIOR / "native-score.py"


def schedule():
    orders = []
    for shift in (0, 2, 4):
        order = ORDER[shift:] + ORDER[:shift]
        orders.extend((order, order[::-1]))
    return [dict(group=index, profile="noop", phases=["warmup", "measured"], order=order)
            for index, order in enumerate(orders, 1)]


def archived_gate():
    original, blocks, pins = retry.inputs()
    old_stages, old_score = dense.stages, matrix.replay_block
    try:
        dense.stages = retry.stages
        matrix.replay_block = dense.replay_block
        report = dense.summarize(retry.RESULTS, original, blocks, pins)
    finally:
        dense.stages, matrix.replay_block = old_stages, old_score
    require(list(report["blocks"]) == list(range(1, 7)) and not report["missing"],
            "offline HOLD: original six checkout groups are incomplete")
    release = dense.QUEUE / f"{retry.ID}-groups-3-6.released.json"
    record = run.read_json(release)
    require(record["groups"] == EXTENSION and record["id"] == retry.ID and record["exit_status"] == 0,
            "offline HOLD: final checkout controller release is missing")
    qualified = report["summary"]["d2"]
    return dict(
        checkout_series_sha256=run.digest(retry.RESULTS / "series.json"),
        original_plan_sha256=run.digest(PRIOR / "native-plan.json"),
        final_release_sha256=run.digest(release),
        qualified_rotations=report["qualified_rotations"],
        advancement_threshold=4,
        qualifying_rotations=qualified["meaningful_rotations"],
        advances=qualified["advances"],
        blocks={number: dict(attempt=block["attempt"], **block["direct"]["d2"],
                             raw_sha256={lane: row["raw_sha256"] for lane, row in block["raw_scores"].items()})
                for number, block in report["blocks"].items()},
        failures=report["failures"],
    )


def stages(config, label, remote):
    if label in dense.PEERS:
        return dense.stages(config, label, remote)
    files, inventory = dense.stages(config, "d2-on", remote)
    files = [p for p in files if p.name != "native-dense.sh"]
    launcher = HERE / "native-supply.sh"
    files.extend((launcher, HERE / "stage-arm.py"))
    spec = inventory["engines"]["mesh"]
    spec["argv"] = ["/bin/bash", str(remote / launcher.name), str(WORKERS[label]), "mesh"]
    del spec["verified_files"][str(remote / "native-dense.sh")]
    spec["verified_files"].update({str(remote / path.name): run.digest(path) for path in files})
    run.check_inventory(inventory)
    return files, inventory


def config_for(original):
    config = deepcopy(original)
    config["description"] = "R2 D2 hot checkout with 2 or 4 scheduler workers per runner"
    for key in ("mesh_broker", "mesh_runner", "mesh_spec"):
        config[key] = {"d2-on": original[key]["d2-on"]}
    for label in set(config["lanes"]) - set(dense.PEERS):
        del config["lanes"][label]
    for label in WORKERS:
        config["lanes"][label] = dict(engine="mesh", runners=2)
    return config


def old_inventory(label):
    return run.read_json(retry.RESULTS / "staging" / label / "inventory.json")


def check_stage_pins(config, gate):
    recorded = run.read_json(retry.RESULTS / "series.json")
    attempt = retry.RESULTS / "blocks" / gate["blocks"]["6"]["attempt"]
    proofs = {}
    for old_label in ("d2-on", *dense.PEERS):
        engine = recorded["config"]["lanes"][old_label]["engine"]
        remote = run.REMOTE / recorded["remotes"][old_label]
        archive_path = attempt / f"measured-{old_label}" / f"measured-{engine}" / "raw.tar.gz"
        with tarfile.open(archive_path, "r:gz") as archive:
            observed = run.file_json(archive, "host.json")
        old_spec = old_inventory(old_label)["engines"][engine]
        require(observed["verified_files"] == old_spec["verified_files"]
                and observed["affinity"] == [0, 1] and observed["pg_mount"] == DEFAULT_MOUNT,
                f"original stage does not match raw arm: {old_label}")
        labels = WORKERS if old_label == "d2-on" else (old_label,)
        for label in labels:
            files, inventory = stages(config, label, remote)
            spec = inventory["engines"][engine]
            require(spec["postgres_storage"] == old_spec["postgres_storage"]
                    and spec["artifact_sha256"] == old_spec["artifact_sha256"]
                    and spec["version"] == old_spec["version"], f"artifact or storage moved: {label}")
            if old_label == "d2-on":
                old_driver = {"native-dense-import-retry.sh"}
                new_driver = {"native-supply.sh", "stage-arm.py"}
                def unchanged(pins, drivers):
                    return {p: sha for p, sha in pins.items()
                            if p not in {str(remote / name) for name in drivers}}
                require(unchanged(spec["verified_files"], new_driver) ==
                        unchanged(old_spec["verified_files"], old_driver), "sealed Mesh stage changed")
            else:
                require(spec == old_spec, f"peer stage differs: {label}")
            require(all(spec["verified_files"][str(remote / p.name)] == run.digest(p) for p in files),
                    f"transferred file digest mismatch: {label}")
        proofs[old_label] = dict(raw_sha256=run.digest(archive_path), storage=old_spec["postgres_storage"],
                                 artifact_sha256=old_spec["artifact_sha256"],
                                 verified_files=old_spec["verified_files"], pg_mount=observed["pg_mount"])
    require(config["mesh_runner"]["d2-on"] == {
        "path": dense.BASE + "/builds/exit-bulk-d2-c7c50e726ca0",
        "elf": "972514efac688fd029821152c3bb70a7e62bf1c6227ab8b2d061dc69c47a2dc7",
        "manifest": "ab09bc77affff4496350202150dc6eb0b90d5c252f0a7552767e0070d615fe31"},
        "wrong R2 D2 executable")
    require(config["mesh_broker"]["d2-on"] == {
        "path": dense.V2,
        "elf": "55d7000d562a6ad395f5e64e33e0e973fac0084ed05ad6c30a33db2a3fc6303a",
        "manifest": "7f228c192630bb6d95cbf9e06c5e0132931f429c1cfa07391bb28cb6fdec8911"},
        "wrong bulk broker")
    return proofs


def scorer(workers):
    # Reuse every predicate in the pinned scorer, parameterizing only its two
    # expected scheduler settings. The archived scorer and its seals stay intact.
    source = DENSE_SCORE.read_text()
    original = '"MESH_SCHEDULER_MIN_WORKERS": "2", "MESH_SCHEDULER_MAX_WORKERS": "2"'
    expected = f'"MESH_SCHEDULER_MIN_WORKERS": "{workers}", "MESH_SCHEDULER_MAX_WORKERS": "{workers}"'
    require(source.count(original) == 1, "frozen scheduler option check changed")
    module = types.ModuleType(f"dense_score_{workers}")
    exec(compile(source.replace(original, expected), str(DENSE_SCORE), "exec"), module.__dict__)
    return module


def check_checkout_receipt():
    current = archived_gate()
    require(run.read_json(PREREQUISITE) == json.loads(json.dumps(current)),
            "original checkout release or raw replay differs from sealed prerequisite")
    return current


def inputs():
    packet = run.read_json(PACKET)
    require(packet["id"] == ID and packet["controller"] == "linode-native-stepchange-controller-v1"
            and packet["cpus"] == [0, 1] and packet["input_sha256"] == dense.INPUT, "wrong request scope")
    for name, expected in packet["local_sha256"].items():
        require(run.digest(dense.ROOT / name) == expected, f"request input changed: {name}")
    _, _, original_pins = retry.inputs()
    original = run.read_json(retry.RESULTS / "series.json")["config"]
    config, gate = config_for(original), run.read_json(PREREQUISITE)
    require(packet["schedule"] == schedule() and packet["native_host"] == config["host"],
            "frozen schedule or host changed")
    proof = run.read_json(STAGE_PROOF)
    require(proof["uid"] == proof["gid"] == 65534 and proof["stage_mode"] == "0711"
            and proof["off_clock"] is True and proof["effective_caps"] == 0
            and proof["explicit_import_reached_cpu_guard"] == [2, 4]
            and proof["failed_plain_import"] is True and proof["proc_cpu_nobody_child"] is True
            and proof["sha256"].keys() == {"identity.py", "native-arm.py", "stage-arm.py"}
            and all(digest == run.digest((PRIOR if name in {"identity.py", "native-arm.py"} else HERE) / name)
                    for name, digest in proof["sha256"].items()), "Linux staging proof differs")
    return config, {**original_pins, **packet["local_sha256"], "request": run.digest(PACKET)}, gate


def grant(_=None):
    events = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD) (\S+) (\S+)(?: (\S+))?",
                        (dense.QUEUE / "STATUS.md").read_text(), flags=re.M)
    mood, request, start, end = events[-1]
    require((mood, request) == ("TIMING", ID)
            and datetime.fromisoformat(start) <= datetime.now(timezone.utc) < datetime.fromisoformat(end),
            "no active sole-controller TIMING grant")
    require(PACKET.read_bytes() == (dense.QUEUE / f"{ID}.json").read_bytes(), "controller packet seal differs")
    return dict(start=start, end=end, request_sha256=run.digest(PACKET))


def read_postgres_cpu(observation, clock_ticks, start_us, end_us):
    start, end = observation["postgres_before"], observation["postgres_after"]
    require(start["begin_us"] <= start["end_us"] <= start_us < end_us <= end["begin_us"] <= end["end_us"]
            <= observation["readback_completed_us"], "PostgreSQL frames do not bracket elapsed clocks")
    require(observation["clock_ticks"] == clock_ticks, "PostgreSQL USER_HZ differs")
    before, after = start["processes"], end["processes"]
    pg = str(observation["postgres_pid"])
    require(before[pg] is not None and after[pg] is not None
            and before[pg]["start_ticks"] == after[pg]["start_ticks"], "postmaster identity changed")
    user = system = 0
    missing = []
    for pid in sorted(before.keys() | after.keys()):
        lo, hi = before.get(pid), after.get(pid)
        if lo is None or hi is None or lo["start_ticks"] != hi["start_ticks"]:
            missing.append(pid)
            continue
        require(pid == pg or lo["parent"] == hi["parent"] == int(pg), "non-PostgreSQL process in PG frame")
        require(hi["user_ticks"] >= lo["user_ticks"] and hi["system_ticks"] >= lo["system_ticks"],
                "nonmonotonic PostgreSQL CPU counter")
        user += hi["user_ticks"] - lo["user_ticks"]
        system += hi["system_ticks"] - lo["system_ticks"]
    return dict(matched_processes=len(before.keys() | after.keys()) - len(missing),
                unmatched_pids=missing, user_ms=1000 * user / clock_ticks,
                system_ms=1000 * system / clock_ticks, tick_ms=1000 / clock_ticks,
                before_submit_slack_ms=(start_us - start["begin_us"]) / 1000,
                after_settlement_slack_ms=(end["end_us"] - end_us) / 1000)


def verify_supply(archive, label, ledger, score, config):
    def receipt(name):
        return run.file_json(archive, f"adapter/{name}.json")

    obs, launch, raw, row = map(receipt, ("scheduler-observation", "dense-launches", "raw", "row"))
    workers = WORKERS[label]
    pids = {str(runner["pid"]): runner["launch_start_ticks"] for runner in launch["runners"]}
    postgres_pid = str(obs["postgres_pid"])
    masks = receipt("process-affinity")
    require(postgres_pid not in pids and masks == dict.fromkeys({*pids, postgres_pid}, [0, 1]),
            "PostgreSQL CPU frame does not identify the pinned postmaster")
    for frame in (obs["ready"], obs["terminal"]):
        require(frame.keys() == pids.keys() and all(
            t["start_ticks"] == pids[pid] and t["cpus"] == [0, 1] and t["threads"] == workers + 6
            for pid, t in frame.items()), "actual scheduler thread supply differs")
    samples = raw["idle_samples"] + raw["samples"]
    peak = max(item["threads"] for item in samples)
    require(peak == row["peak_threads"] == 2 * (workers + 6)
            and raw["samples"][-1]["terminal"] == dense.PLAN["topology"]["jobs"]
            and all(item["connections"] == 41 and item["processes"] == 2 for item in samples),
            "sampled runner/thread/connection envelope differs")

    roster = {r["backend"]: r["client_port"] for r in launch["both_runners_tls"] if r["role"] == "compiled_caller"}
    final = obs["writers"]
    require(len(final) == 32 and {r["backend"]: r["client_port"] for r in final if r["tls"]} == roster,
            "warmed application TLS sockets changed before post-settlement readback")
    start, finish = score["first_submit_us"], score["last_settlement_us"]
    writers = {r["backend"] for r in final if r["last_was_application_write"]
               and start <= r["query_started_us"] <= finish}
    first = {r["backend"] for r in launch["first_runner_tls"] if r["role"] == "compiled_caller"}
    writer_sets = [sorted(writers & first), sorted(writers - first)]
    require(all(writer_sets), "each runner must have a witnessed PostgreSQL writer")
    proc_cpu = read_postgres_cpu(obs, config["clock_ticks"], start, finish)
    pg_clients = {str(r["backend"]) for r in launch["both_runners_tls"]}
    for field in ("postgres_before", "postgres_after"):
        require(pg_clients <= obs[field]["processes"].keys(), "PostgreSQL CPU frame omitted warmed sessions")
    return dict(scheduler_workers_each=workers, sampled_peak_runner_threads=peak,
                writer_backend_pids_by_runner=writer_sets, writer_backend_count=len(writers),
                postgres_process_cpu_window=proc_cpu,
                sql_calls={key.removeprefix("raw_"): value for key, value in raw.items()
                           if key.startswith("raw_") and key.endswith("_calls")},
                per_runner_hints=[json.loads(matrix.text(archive, f"adapter/runner-{i}.log")
                                             .splitlines()[-1])["batch_contention_hints"] for i in range(2)],
                row_cpu_seconds=ledger["runner_cpu_seconds"])


def metrics(ledger, score):
    writes = sorted(job["effect_committed_us"] for job in ledger["jobs"])
    require(len(writes) == 1024, "middle-write clock needs the full input")
    start, commit, end = (score[k] for k in ("first_submit_us", "first_commit_us", "last_settlement_us"))
    return dict(submit_ms=(end - start) / 1000, drain_ms=(end - commit) / 1000,
                submit_last_write_ms=(writes[-1] - start) / 1000,
                drain_last_write_ms=(writes[-1] - commit) / 1000,
                middle_820_ms=(writes[921] - writes[102]) / 1000)


def score_arm(folder, phase, label, config, remote):
    _, inventory = stages(config, label, remote)
    engine = config["lanes"][label]["engine"]
    if label in WORKERS:
        score = scorer(WORKERS[label]).raw_score(folder, phase, "d2-on", config, inventory)
    else:
        score = dense.base_score(folder, engine, 2, phase, inventory, config)
    with tarfile.open(folder / "raw.tar.gz", "r:gz") as archive:
        ledger = run.file_json(archive, "adapter/ledger.json")
        require(score["pg_mount"] == DEFAULT_MOUNT and ledger["slots"] == 64
                and ledger["database"]["max_connections"] == ledger["database"]["database_connection_limit"] == 96
                and ledger["peak_connections"] <= 96 and ledger["peak_executing"] <= 64,
                "storage, PostgreSQL physical cap, or slot cap differs")
        if label in WORKERS:
            extra = verify_supply(archive, label, ledger, score, config)
        elif engine == "oban":
            raw = run.file_json(archive, "adapter/raw.json")
            extra = {"sql_calls": {key.removeprefix("raw_"): value for key, value in raw.items()
                                   if key.startswith("raw_") and key.endswith("_calls")}}
        else:
            extra = {"sql_calls": run.file_json(archive, "adapter/counters.json")}
        submit = score["first_submit_us"]
        settle = score["last_settlement_us"]
        with archive.extractfile("host-samples.jsonl") as stream:
            points = [json.loads(line)["epoch_us"] for line in stream]
        limits = dict(before_submit_ms=(submit - max(t for t in points if t <= submit)) / 1000,
                      after_settle_ms=(min(t for t in points if t >= settle) - settle) / 1000)
    return dict(score, **metrics(ledger, score), **extra, host_cpu_window_overhang_ms=limits,
                runner_rss_bytes=ledger["peak_runner_rss_bytes"])


def replay_block(path, block, config, remotes):
    require(run.read_json(path / "order.json") == block, "attempt changed frozen order")
    return {f"{phase}-{label}": score_arm(folder, phase, label, config, run.REMOTE / remotes[label])
            for phase, label, _, folder in matrix.paths(path, config, block)}


def gate_for(arms):
    a, b, candidate = (arms[label] for label in ("fixed2-a", "fixed2-b", "fixed4"))
    gates = {}
    for key in CLOCKS:
        faster = min(a[key], b[key])
        threshold = max(abs(a[key] - b[key]), .05 * faster)
        gain = faster - candidate[key]
        gates[key] = dict(gain_ms=gain, threshold_ms=threshold, passes=gain > threshold)
    gates["last_write"] = dict(passes=candidate["drain_last_write_ms"] <
                               min(a["drain_last_write_ms"], b["drain_last_write_ms"]))
    return dict(metrics=gates, passes=all(row["passes"] for row in gates.values()))


def summarize(directory, config, blocks, pins):
    manifest = run.read_json(directory / "series.json")
    require(all(manifest[k] == v for k, v in dict(config=config, schedule=blocks, pins=pins).items()),
            "series inventory differs")
    failures, selected, missing = {}, {}, []
    for block in blocks:
        number = block["group"]
        found = matrix.pick(directory, block, config, manifest["remotes"], failures)
        if not found:
            missing.append(number)
            continue
        attempt, rows = found
        measured = {label: rows[f"measured-{label}"] for label in ORDER}
        resources = ("runner_cpu_ms_per_job", "runner_rss_bytes",
                     "pinned_user_cpu_ms_per_job", "pinned_kernel_cpu_ms_per_job")
        fixed2 = {key: (measured["fixed2-a"][key] + measured["fixed2-b"][key]) / 2 for key in resources}
        comparators = {"fixed2_duplicate_mean": fixed2, **{label: measured[label] for label in dense.PEERS}}
        tradeoffs = {name: {key: measured["fixed4"][key] - arm[key] for key in resources}
                     for name, arm in comparators.items()}
        selected[number] = dict(attempt=attempt, arms=rows, gate=gate_for(measured),
                                fixed4_resource_deltas=tradeoffs,
                                ordering={key: sorted(measured, key=lambda lane: measured[lane][key])
                                          for key in (*CLOCKS, "drain_last_write_ms")})
    pilot_passes = all(number in selected and selected[number]["gate"]["passes"] for number in PILOT)
    measures = (*CLOCKS, "drain_last_write_ms", "runner_cpu_ms_per_job", "runner_rss_bytes",
                "pinned_user_cpu_ms_per_job", "pinned_kernel_cpu_ms_per_job")
    aggregate = {}
    for label in ORDER:
        per_group = [row["arms"][f"measured-{label}"] for row in selected.values()]
        if per_group:
            aggregate[label] = {k: dense.dist([arm[k] for arm in per_group]) for k in measures}
    return dict(blocks=selected, missing=missing, failures=failures, pilot_passes=pilot_passes,
                extension_eligible=pilot_passes, aggregate=aggregate,
                aggregate_ordering={k: sorted(aggregate, key=lambda lane: aggregate[lane][k]["median"])
                                    for k in (*CLOCKS, "drain_last_write_ms")})


def timing(config, pins, groups):
    require(groups in (PILOT, EXTENSION), "use either the pilot or the balanced extension")
    require(check_checkout_receipt()["advances"], "offline HOLD: checkout did not clear 4/6")
    grant()
    if groups == EXTENSION:
        require(summarize(RESULTS, config, schedule(), pins)["pilot_passes"], "HOLD: pilot gate did not pass")
    matrix.stages, matrix.grant = stages, grant
    matrix.replay_block, matrix.summarize = replay_block, summarize
    matrix.SERIES[ID] = ID
    send = run.dispatch_arm

    def checked_dispatch(path, remote, block, phase, engine):
        send(path, remote, block, phase, engine)
        folder = path / f"{phase}-{engine}"
        require(run.read_json(folder / "dispatch.json").get("exit_code") == 0,
                f"review retained failed arm: {folder}")
        score_arm(folder, phase, path.name.removeprefix(phase + "-"), config, remote)

    run.dispatch_arm = checked_dispatch
    matrix.dispatch(RESULTS, config, ID, schedule(), pins, groups)
    report = summarize(RESULTS, config, schedule(), pins)
    return bool(set(groups).intersection(report["missing"]))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("gate", "check", "timing", "replay"))
    parser.add_argument("groups", type=int, nargs="*")
    args = parser.parse_args()
    if args.action == "gate":
        status = archived_gate()
        print(json.dumps(status, sort_keys=True, indent=2))
        return 0 if status["advances"] else 1
    config, pins, gate = inputs()
    if args.action == "check":
        subprocess.run(["bash", "-n", HERE / "native-supply.sh"], check=True)
        check_checkout_receipt()
        stage_pins = check_stage_pins(config, gate)
        print(json.dumps(dict(eligible=gate["advances"], archived_stages=stage_pins), indent=2, sort_keys=True))
        return 0
    if args.action == "timing":
        return timing(config, pins, args.groups)
    matrix.replay_block = replay_block
    print(json.dumps(summarize(RESULTS, config, schedule(), pins), indent=2, sort_keys=True, allow_nan=False))
    return 0


if __name__ == "__main__":
    sys.exit(main())
