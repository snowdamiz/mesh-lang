#!/usr/bin/env python3
"""Stage, qualify and replay the separately labeled Linode short16 sensitivity."""

import argparse
from copy import deepcopy
from datetime import datetime, timezone
import fcntl
import gzip
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import shlex
import sys
import tarfile
from uuid import uuid4

from build import HERE, HISTORICAL, ROOT, STOCK_ELF, SUITE, sha
from score import CLOCKS, connection_samples, distribution, require, stage_times, startup

sys.path.insert(0, str(SUITE))
original_replay = HISTORICAL / "replay"
sys.path.insert(0, str(original_replay))
original_spec = importlib.util.spec_from_file_location("original_short_controller", original_replay / "controller.py")
original = importlib.util.module_from_spec(original_spec)
original_spec.loader.exec_module(original)
sys.path.remove(str(original_replay))
matrix, run = original.matrix, original.run
base_validate = matrix.validate
ID = HERE.name
EVIDENCE = HERE / "evidence"
SETUP, TIMING = EVIDENCE / "setup", EVIDENCE / "timing"
REQUESTS = ROOT / "target/linode-jobs-stepchange/requests"
HISTORY = HISTORICAL / "evidence/native-timing" / HISTORICAL.name
RIVER_PREFIX = "/var/lib/durable-jobs-bench/river/builds"
CANDIDATES = {"river-cold": "cold", "river-warm": "warm"}
RIVERS = ("river", *CANDIDATES)
ORDERS = (
    ("r2-off", "river-cold", "oban", "river", "r2-d2", "river-warm", "apalis"),
    ("river-warm", "r2-d2", "apalis", "oban", "river-cold", "r2-off", "river"),
    ("oban", "apalis", "river", "r2-off", "river-warm", "r2-d2", "river-cold"),
)
CONTROLLER_ID = "linode-native-stepchange-controller-v1"


def schedule(profiles, lanes):
    require(profiles == ["short"] and set(lanes) == set(ORDERS[0]), "unexpected sensitivity lane")
    for group, order in enumerate((order for row in ORDERS for order in (row, row[::-1])), 1):
        yield dict(group=group, order=list(order), profile="short", phases=["measured"])


def validate(config):
    require(set(config["lanes"]) == set(ORDERS[0]), "unexpected seven-lane configuration")
    for omit in ("river", "river-warm"):
        base_validate(dict(config, lanes={k: v for k, v in config["lanes"].items() if k != omit}))


def historical_config():
    accepted = run.read_json(HISTORICAL / "recovery/evidence/qualification.json")
    recovered = HISTORICAL / "recovery/evidence" / accepted["attempt"] / "qualified-timing.json"
    require(sha(recovered) == accepted["inventory_sha256"], "original qualified inventory changed")
    manifest = run.read_json(HISTORY / "series.json")
    require(manifest["config"] == run.read_json(recovered), "original timing did not use recovered inventory")
    require(manifest["plan"] == list(original.plan(manifest["config"]["profiles"], manifest["config"]["lanes"])),
            "original five-rotation plan changed")
    return manifest["config"], manifest["plan"]


def arm_detail(folder, engine, score, clk_tck, mode=None):
    with tarfile.open(folder / "raw.tar.gz") as archive:
        detail = stage_times(run.file_json(archive, "adapter/ledger.json"), score)
        detail["physical_connections"] = connection_samples(archive, engine)
        if mode is not None:
            detail["startup"] = startup(archive, mode, score, clk_tck)
            require(detail["physical_connections"]["last_idle"] == detail["startup"]["physical_at_submit"],
                    "standard idle sample and startup census disagree")
    return detail


def historical():
    config, plan = historical_config()
    recorded = run.read_json(HISTORY / "summary.json")
    source_rows = recorded["rows"]["short"]
    require(not recorded["missing"] and len(source_rows) == len(plan) == 5, "incomplete original rotations")
    rows = []
    for row, block in zip(source_rows, plan, strict=True):
        require(all(row[key] == value for key, value in block.items()), "original order changed")
        path = HISTORY / "blocks" / row["attempt"]
        replayed = original.replay(path, config, block)
        require(all(row[key] == value for key, value in replayed.items()), "original archive replay differs")
        details = {label: arm_detail(folder, config["lanes"][label]["engine"], replayed["scores"][label],
                                     config["clock_ticks"])
                   for _, label, folder in matrix.arms(path, config, block)}
        for lane, expected in (("river", (4, 18)), ("r2-off", (13, 13)), ("r2-d2", (13, 13)),
                               ("oban", (19, 19)), ("apalis", (19, 19))):
            observed = details[lane]["physical_connections"]
            require((observed["last_idle"], observed["peak_during_load"]) == expected,
                    f"rotation {block['group']} {lane} connection receipt differs")
        rows.append(dict(group=block["group"], order=block["order"], attempt=row["attempt"],
                         raw_sha256=replayed["raw_sha256"], lanes=details))
    comparisons = paired(rows, "r2-d2", "river")
    require(all(comparisons[key]["target_faster"] == 3 for key in CLOCKS), "stock peer wins differ")
    return dict(series=HISTORICAL.name, rows=rows, paired_r2_hot_vs_stock_river=comparisons)


def build_receipt():
    receipt = run.read_json(EVIDENCE / "build.json")
    compressed = EVIDENCE / "river-linux-amd64.gz"
    require(receipt["stock_rebuild_sha256"] == STOCK_ELF and sha(compressed) == receipt["compressed_sha256"],
            "sealed Go build differs")
    with gzip.open(compressed, "rb") as stream:
        ident = stream.read(20)
        require(ident[:6] == b"\x7fELF\x02\x01" and ident[18:20] == b"\x3e\x00",
                "candidate is not Linux-target x86-64 ELF")
        checksum = hashlib.sha256(ident)
        for buffer in iter(lambda: stream.read(1 << 20), b""):
            checksum.update(buffer)
    require(checksum.hexdigest() == receipt["candidate_sha256"] != STOCK_ELF, "candidate ELF digest differs")
    for name, expected in receipt["source_pins"].items():
        require(sha(SUITE / "go" / name) == expected, f"built Go source changed: {name}")
    return receipt


def make_config(build):
    config, _ = historical_config()
    config["description"] = "short16 steady-service sensitivity: one River shared-pool readiness change"
    lanes = config["lanes"]
    lanes.pop("protected-mesh")
    stock = lanes["river"]["inventory"]["engines"]["go"]
    require(stock["artifact_sha256"] == STOCK_ELF, "wrong frozen stock River")
    binary = remote_binary(build)
    for label, mode in CANDIDATES.items():
        lane = deepcopy(lanes["river"])
        spec = lane["inventory"]["engines"]["go"]
        argv = spec["argv"]
        binary_pos = argv.index("--binary") + 1
        require(spec["verified_files"][argv[binary_pos]] == STOCK_ELF, "stock binary argument mismatch")
        argv[binary_pos] = binary
        spec["argv"] = ["/usr/bin/env", "RIVER_BENCH_POOL_READINESS=" + mode, *argv]
        spec["artifact_sha256"] = build["candidate_sha256"]
        spec["verified_files"][binary] = spec["artifact_sha256"]
        lanes[label] = lane
    validate(config)
    return config


def remote_binary(build):
    return f"{RIVER_PREFIX}/river-readiness-{build['candidate_sha256'][:16]}/river-bench"


def check_packet():
    packet = run.read_json(HERE / "controller-packet.json")
    require(packet["id"] == ID and packet["ready"] and packet["controller"] == CONTROLLER_ID,
            "wrong controller packet")
    for name, expected in packet["pins"].items():
        require(sha(ROOT / name) == expected, f"controller pin changed: {name}")
    build = build_receipt()
    require(build["compressed_sha256"] == packet["stage_sha256"] and
            build["candidate_sha256"] == packet["candidate_elf_sha256"], "request and stage disagree")
    config = make_config(build)
    require(list(schedule(config["profiles"], config["lanes"])) == packet["group_plan"], "frozen order differs")
    return packet, build, config


def grant(kind):
    pattern = r"^(SETUP|TIMING|RELEASED|HOLD) ([\w-]+) (\S+)(?: (\S+))?"
    events = re.findall(pattern, (REQUESTS / "STATUS.md").read_text(), re.MULTILINE)
    require(bool(events), "central controller grant required")
    mode, identifier, start, end = events[-1]
    now = datetime.now(timezone.utc)
    require((mode, identifier) == (kind, ID) and end and
            datetime.fromisoformat(start) <= now < datetime.fromisoformat(end),
            f"active central controller {kind} grant required for {ID}")
    return start, end


def install(build, config, out):
    binary = remote_binary(build)
    pins = {}
    for lane in config["lanes"].values():
        for path, expected in lane["inventory"]["engines"][lane["engine"]]["verified_files"].items():
            require(path not in pins or pins[path] == expected, f"conflicting host pin: {path}")
            pins[path] = expected
    frozen = run.read_json(HISTORICAL / "controller-packet.json")["native_cold_pins"]["files"]
    pins.update({name: digest for name, digest in frozen.items() if "/pgsql-17.11/bin/" in name})
    pins.pop(binary)
    checksum_input = "\n".join(f"{digest}  {path}" for path, digest in sorted(pins.items()))
    script = (matrix.HOST_IDLE + (HERE / "native-install.sh").read_text()
              .replace("@PINNED_FILES@", checksum_input))
    with (EVIDENCE / "river-linux-amd64.gz").open("rb") as source, (out / "install.log").open("xb") as log:
        completed = run.ssh(["bash", "-c", script, "--", binary, build["compressed_sha256"],
                             build["candidate_sha256"], *config["host"], config["clock_ticks"]],
                            stdin=source, stdout=log, stderr=log, timeout=75)
    run.write_json(out / "install-status.json", dict(exit_code=completed.returncode, finished=run.utc_now()))
    completed.check_returncode()


def setup_scores(config, out):
    scores = {}
    for label in RIVERS:
        lane = config["lanes"][label]
        folder = out / label / "control-go"
        score = run.score_arm(folder, dict(profile="short"), "control", "go", lane["inventory"])
        require([score["host_boot_id"], score["host_kernel"], score["pg_mount"]["maj:min"]] == config["host"],
                "SETUP host changed")
        scores[label] = dict(raw_sha256=sha(folder / "raw.tar.gz"),
                             detail=arm_detail(folder, "go", score, config["clock_ticks"], CANDIDATES.get(label)))
    require(scores["river-cold"]["detail"]["physical_connections"]["last_idle"] < 18 and
            scores["river"]["detail"]["physical_connections"]["last_idle"] < 18 and
            scores["river-warm"]["detail"]["physical_connections"]["last_idle"] == 18,
            "SETUP failed to separate the cold and warm readiness policies")
    return scores


def setup(build, config):
    start, end = grant("SETUP")
    attempt = uuid4().hex
    out = SETUP / attempt
    out.mkdir(parents=True)
    run.write_json(out / "grant.json", dict(id=ID, start=start, end=end))
    with (SUITE / ".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        grant("SETUP")
        install(build, config, out)
        for label in RIVERS:
            grant("SETUP")
            local = out / label
            local.mkdir()
            remote = run.REMOTE / f"{ID}-setup-{attempt}-{label}"
            spec = local / "inventory.json"
            run.write_json(spec, config["lanes"][label]["inventory"])
            with (local / "stage.log").open("xb") as log:
                run.stage(SUITE / "worker.py", remote, log)
                run.stage(spec, remote, log)
            run.dispatch_arm(local, remote, dict(profile="short"), "control", "go")
        scores = setup_scores(config, out)
        run.write_json(EVIDENCE / "setup-qualified.json", dict(attempt=attempt, arms=scores,
                       install_sha256=sha(out / "install.log"), elf=build["candidate_sha256"]))


def verify_setup(build, config):
    qualified = run.read_json(EVIDENCE / "setup-qualified.json")
    require(re.fullmatch(r"[a-f0-9]{32}", qualified["attempt"]) and qualified["elf"] == build["candidate_sha256"],
            "invalid native SETUP qualification")
    out = SETUP / qualified["attempt"]
    require(run.read_json(out / "install-status.json")["exit_code"] == 0 and
            sha(out / "install.log") == qualified["install_sha256"] and
            qualified["arms"] == setup_scores(config, out), "SETUP receipt replay differs")
    return qualified


def replay(path, config, block):
    result = original.replay(path, config, block)
    details = {label: arm_detail(folder, config["lanes"][label]["engine"], result["scores"][label],
                                 config["clock_ticks"], CANDIDATES.get(label))
               for _, label, folder in matrix.arms(path, config, block)}
    require(details["river-cold"]["physical_connections"]["last_idle"] < 18 and
            details["river"]["physical_connections"]["last_idle"] < 18 and
            details["river-warm"]["physical_connections"]["last_idle"] == 18,
            "timing did not preserve the deliberate readiness contrast")
    return dict(**result, lanes=details)


def paired(rows, target, reference):
    comparisons = {}
    for metric in CLOCKS:
        values = [100 * (row["lanes"][target][metric] / row["lanes"][reference][metric] - 1) for row in rows]
        comparisons[metric] = dict(**distribution(values), target_faster=sum(value > 0 for value in values),
                                   per_rotation_percent=values)
    differences = {}
    for key, value in rows[0]["lanes"][target].items():
        if key.endswith("_ms") and isinstance(value, (float, int)):
            values = [row["lanes"][target][key] - row["lanes"][reference][key] for row in rows]
            differences[key] = dict(**distribution(values), per_rotation=values)
    comparisons["target_minus_reference_ms"] = differences
    return comparisons


def configure_matrix():
    matrix.RESULTS = TIMING.parent
    matrix.plan = schedule
    matrix.validate = validate
    matrix.replay = replay


def timing(build, config, groups):
    grant("TIMING")
    released = run.read_json(REQUESTS / f"{ID}.released.json")
    require(released["id"] == ID and released["exit_status"] == 0 and released["released_at"],
            "successful fenced SETUP release required")
    verify_setup(build, config)
    configure_matrix()
    if groups == [3, 4, 5, 6]:
        pilot = run.read_json(REQUESTS / f"{ID}-groups-1-2.released.json")
        require(pilot["id"] == ID and pilot["groups"] == [1, 2] and
                pilot["exit_status"] == 0 and pilot["released_at"], "successful pilot release required")
        for block in list(schedule(config["profiles"], config["lanes"]))[:2]:
            require(matrix.select_attempt(TIMING, config, block, {}), "both opposite orders must reconcile")
    return matrix.dispatch(TIMING, config, groups)


def summarize(config):
    configure_matrix()
    manifest = run.read_json(TIMING / "series.json")
    require(manifest["config"] == config and manifest["plan"] == list(schedule(config["profiles"], config["lanes"])),
            "sensitivity series configuration changed")
    rows, issues, missing = [], {}, []
    for block in manifest["plan"]:
        selected = matrix.select_attempt(TIMING, config, block, issues)
        if selected:
            attempt, result = selected
            rows.append(dict(**block, attempt=attempt, **result))
        else:
            missing.append(block["group"])
    comparisons = {}
    lanes = {}
    if rows:
        for target, reference in (("river-warm", "river-cold"), ("river-cold", "river"),
                                  *((mesh, peer) for mesh in ("r2-off", "r2-d2") for peer in RIVERS)):
            comparisons[f"{target}_vs_{reference}"] = paired(rows, target, reference)
        for label in config["lanes"]:
            measurements = [row["lanes"][label] for row in rows]
            lanes[label] = {
                field: distribution([measurement[field] for measurement in measurements])
                for field, value in measurements[0].items() if isinstance(value, (int, float))
            }
            for field in ("last_idle", "peak_during_load"):
                lanes[label][field] = distribution(
                    [measurement["physical_connections"][field] for measurement in measurements])
    startup_totals = {}
    for label in CANDIDATES:
        points = [row["lanes"][label]["startup"] for row in rows]
        if points:
            startup_totals[label] = {
                field: distribution([point[field] for point in points])
                for field in points[0] if isinstance(points[0][field], (float, int))
            }
            for process in ("runner", "producer_after_schema", "producer_connection"):
                startup_totals[label][process] = {
                    field: distribution([point[process][field] for point in points])
                    for field in ("cpu_ms", "rss_bytes", "threads")
                }
    print(json.dumps(dict(sensitivity=ID, missing=missing, issues=issues, rotations=rows,
                          lanes=lanes, paired=comparisons, startup=startup_totals),
                     indent=2, sort_keys=True, allow_nan=False))
    return bool(missing)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("historical", "check", "setup", "verify-setup", "timing", "summarize"))
    parser.add_argument("groups", type=int, nargs="*")
    args = parser.parse_args()
    if args.action == "historical":
        print(json.dumps(historical(), indent=2, sort_keys=True, allow_nan=False))
        return 0
    if args.action == "timing" and args.groups not in ([1, 2], [3, 4, 5, 6]):
        parser.error("select groups 1 2 or 3 4 5 6")
    packet, build, config = check_packet()
    if args.action == "check":
        require(historical() == run.read_json(EVIDENCE / "stock-replay.json"), "stored stock analysis differs")
        print(json.dumps(dict(id=ID, candidate_elf=build["candidate_sha256"], stock_blocks=5,
                              planned_groups=packet["group_plan"], native_work_performed=False), indent=2))
    elif args.action == "setup":
        setup(build, config)
    elif args.action == "verify-setup":
        print(json.dumps(verify_setup(build, config), sort_keys=True, indent=2))
    elif args.action == "timing":
        return timing(build, config, args.groups)
    elif args.action == "summarize":
        return summarize(config)
    return 0


if __name__ == "__main__":
    sys.exit(main())
