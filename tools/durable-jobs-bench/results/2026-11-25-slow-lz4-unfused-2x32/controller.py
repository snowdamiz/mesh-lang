"""Parent-routed, build-free SLOW stock codec proof and gated mirrored timing."""
import argparse
from copy import deepcopy
import fcntl
import hashlib
import json
from pathlib import Path
import shlex
from statistics import median
import sys
import tarfile

HERE = Path(__file__).resolve().parent
RESULTS = HERE.parent
SUITE = RESULTS.parent
ROOT = SUITE.parents[1]
CAP71 = RESULTS / "2026-11-24-ordinary-slow-equal-topology-cap71"
CODEC = RESULTS / "2026-11-25-slow-lz4-isolated-qualification"
OLD = RESULTS / "2026-11-23-slow-storage-transfer"
ID = HERE.name
OUT = HERE / "evidence"
MODES = {"proof": ([0], 25), "pilot": ([1, 2], 35), "full": ([3, 4, 5, 6], 55)}
ENGINES = {"bulk": "mesh", "river": "go", "oban": "oban", "apalis": "rust"}
PHASES = ("warmup", "measured")
CLOCKS = ("submit_ms", "drain_ms")
PHASE_CLOCKS = ("commit_to_64th_first_ms", "first64_to_last_effect_ms", "last_effect_to_terminal_ms")
RIVER = "/var/tmp/mesh-durable-jobs-matrix/slow-equal-cap71-timing-bc860b6870ef-go/river-slow-2x32-cap71"
RIVER_SHA = "a23130f74210946291ed3609797e5b218ece5a7bd70df318851cf91e634da1e1"
PG_SHA = "3f273b467aa57364f603a6758de24f1937098bddfcd5ef8b65c90df9caaec4b8"
INVENTORY = CODEC / "recovery-3/matched-inventory.json"
CODEC_RELEASE = CODEC / "evidence/native/recovery-3/timing-continuation/controller-release.json"
CAP71_RAW = CAP71 / "evidence/native/sql-retry-timing"
BUNDLE = {
    "native-run.sh": HERE / "native-run.sh", "adapter.py": HERE / "adapter.py",
    "cap71_adapter.py": CAP71 / "sql-audit-retry/adapter.py",
    "overlay.py": CODEC / "overlay.py", "strict.py": CODEC / "strict.py",
    "storage.py": OLD / "replay/storage.py", "slow-river.sql": OLD / "replay/slow-river.sql",
}
EXTRA = ("physical-postgres.log", "payload-storage.json", "codec-postgres.json", "startup-clock.json", "*.json")

sys.path[:0] = [str(CAP71), str(SUITE)]
import qualification as equal
import run as matrix

sys.path[:0] = [str(CODEC), str(OLD / "replay")]
import packet as codec_packet
from strict import verify_codec

require = matrix.require
read = matrix.read_json
write = matrix.write_json
sha = matrix.digest


def proof_labels():
    return [f"two-{eng}-{policy}" for eng in ENGINES for policy in ("pglz", "lz4")] + [
        f"one-{eng}-pglz" for eng in ENGINES
    ]


def design():
    cfg = read(INVENTORY)
    sealed = read(CAP71_RAW / "series.json")["design"]
    pins = {name: sha(path) for name, path in BUNDLE.items()}
    require(pins["cap71_adapter.py"] == "31ec825ef073668d279cce9eda193aa8f016027aacfc3077fb2f75bde7ce8ad0",
            "repaired cap-71 adapter pin changed")
    require(sealed["river_sha"] == RIVER_SHA and sealed["host"] == cfg["host"]
            and cfg["postgres"]["installed"]["bin/postgres"] == PG_SHA and read(CODEC_RELEASE)["exit_status"] == 0,
            "original host or retained PG or River custody changed")
    remote = matrix.REMOTE / f"{ID}-{hashlib.sha256(json.dumps(pins, sort_keys=True).encode()).hexdigest()[:16]}"
    pg = Path(cfg["postgres"]["prefix"])
    base_lanes = cfg["lanes"]
    lanes = {}
    for label in proof_labels():
        shape, name, policy = label.split("-")
        engine = ENGINES[name]
        lane = deepcopy(base_lanes[f"{name}-{policy}"])
        lanes[label] = lane
        spec = lane["inventory"]["engines"][engine]
        if shape == "one":
            continue
        old = sealed["stock_engines"][engine]
        original_argv = old["argv"]
        original_pins = old["verified_files"]
        if engine in ("mesh", "oban"):
            launcher = str(Path(original_argv[-2]).with_name("arm.py"))
            artifact = original_argv[-3].split("=", 1)[1]
        else:
            launcher = str(Path(original_argv[0]).with_name("run.py"))
            artifact = RIVER if engine == "go" else str(
                Path(launcher).parents[3] / "target/linode-apalis/build/release/durable-jobs-apalis-bench")
        require(original_pins[launcher] == spec["verified_files"][launcher],
                "stock launcher seals differ between released studies")
        require(spec["artifact_sha256"] == old["artifact_sha256"], f"{engine} differs between prior studies")
        spec.pop("run_as", None)
        spec["argv"] = ["/bin/bash", str(remote / "native-run.sh"), engine, policy,
                        launcher, artifact, str(pg.parent), str(pg / "bin")]
        if engine == "go":
            spec["artifact_sha256"] = RIVER_SHA
        if engine in ("go", "rust"):
            spec["verified_files"][artifact] = spec["artifact_sha256"]
        spec["verified_files"].update({str(remote / name): digest for name, digest in pins.items()})
        spec["evidence_globs"] = sorted(set(spec["evidence_globs"]) | set(EXTRA))
        matrix.check_inventory(lane["inventory"])

    first = ["two-bulk-pglz", "two-river-lz4", "one-oban-pglz", "two-apalis-lz4",
             "two-oban-pglz", "one-river-pglz", "two-bulk-lz4", "two-apalis-pglz",
             "one-bulk-pglz", "two-oban-lz4", "two-river-pglz", "one-apalis-pglz"]
    require(len(first) == len(set(first)) and set(first) == lanes.keys(), "incomplete frozen lane order")
    orders = {0: first}
    for pair, shift in enumerate((0, 4, 8)):
        row = first[shift:] + first[:shift]
        orders[pair * 2 + 1] = row
        orders[pair * 2 + 2] = row[::-1]
    cfg.update(lanes=lanes, cap71_design=sealed, composition_remote=str(remote), orders=orders)
    return cfg, pins


def all_pins():
    paths = [Path(__file__), *BUNDLE.values(), INVENTORY, CODEC_RELEASE, CODEC / "packet.py",
             CAP71 / "qualification.py", CAP71_RAW / "series.json", CAP71 / "request-sql-retry-timing.json",
             SUITE / "run.py", SUITE / "verify.py", SUITE / "worker.py", SUITE / "interleave.py",
             OLD / "replay/native.py", HERE / "history.py", HERE / "released-comparisons.json"]
    return {str(path.relative_to(ROOT)): sha(path) for path in paths}


def request(mode, cfg, pins):
    groups, minutes = MODES[mode]
    command = f"python3 -B {shlex.quote(str(Path(__file__)))}"
    phases = ["diagnostic"] if mode == "proof" else list(PHASES)
    return dict(id=f"{ID}-{mode}", kind="SETUP" if mode == "proof" else "TIMING",
                requested_window_minutes=minutes, build=False, input_size=512, hold_ms=100,
                runner_policy="stock unfused; River stock lazy; no explicit warming",
                groups=groups, phases=phases, arms=len(groups)*len(phases)*len(cfg["lanes"]),
                orders={str(g): cfg["orders"][g] for g in groups}, source_sha256=all_pins(),
                bundle_sha256=pins, inventory_sha256=sha(HERE / "inventory.json"),
                evidence_destination=str(OUT / mode), command=command + f" {mode}",
                offline_check_command=command + " check",
                replay_command=command + f" replay-{mode}",
                requires={"proof": [], "pilot": [f"{ID}-proof"], "full": [f"{ID}-proof", f"{ID}-pilot"]}[mode])


def file_for(mode):
    return HERE / f"request-{mode}.json"


def grant(mode, seconds=120):
    return codec_packet.grant("SETUP" if mode == "proof" else "TIMING", file_for(mode), seconds)


def phases_and_readiness(raw, ledger, phase, engine, starts, row, equal_shape):
    jobs = ledger["jobs"]
    first = sorted(job["attempt_committed_us"] for job in jobs)[63]
    effect = max(job["effect_committed_us"] for job in jobs)
    txn, = ledger["transactions"]
    terminal = max(job["settled_us"] for job in jobs)
    origin = txn["committed_us"]
    require(origin <= first <= effect <= terminal, "invalid durable phase order")
    phase_cost = dict(zip(PHASE_CLOCKS, ((first-origin)/1000, (effect-first)/1000, (terminal-effect)/1000)))
    if engine == "go":
        with raw.extractfile("adapter/river/measured/samples.jsonl") as stream:
            samples = [json.loads(line) for line in stream]
        idle = [sample for sample in samples if sample["idle"]]
        require(len(idle) == 2 and all(sample["all_connections_tls"] and sample["database_running_rows"] == 0
                                      and sample["terminal"] == 0 for sample in idle), "River idle roster differs")
        expected = 7 if equal_shape else 4
        require(all(sample["processes"] == (2 if equal_shape else 1) and sample["connections"] == expected
                    for sample in idle), "River stock connection policy changed")
        row["river_stock_idle"] = dict(tls_clients=[sample["connections"] for sample in idle],
                                       runner_processes=idle[0]["processes"],
                                       idle_interval_ms=idle[-1]["offset_ms"] - idle[0]["offset_ms"],
                                       observed_in_load_peak=max(sample["connections"] for sample in samples if not sample["idle"]))
    if starts:
        endpoints = [starts[k] for k in ("adapter_start", "postgres_start", "postgres_ready",
                                         "runner_launch_dispatch", "runners_ready")]
        ready = starts.get("verified_tls_roster", starts["runners_ready"])
        require([point["monotonic_ns"] for point in endpoints] == sorted(point["monotonic_ns"] for point in endpoints)
                and endpoints[-1]["monotonic_ns"] <= ready["monotonic_ns"] < starts["adapter_end"]["monotonic_ns"]
                and ready["epoch_ns"] // 1000 <= txn["submitted_us"]
                and starts["adapter_end"]["epoch_ns"] // 1000 >= terminal, "startup or close clock crossed workload")
        row["cold_lifecycle_ms"] = {
            "postgres_start_to_ready": (endpoints[2]["monotonic_ns"]-endpoints[1]["monotonic_ns"])/1e6,
            "runner_dispatch_to_readiness_observation": (endpoints[-1]["monotonic_ns"]-endpoints[-2]["monotonic_ns"])/1e6,
            "ready_observation_to_submit_wall": (txn["submitted_us"]-ready["epoch_ns"]//1000)/1000,
            "adapter_run": (starts["adapter_end"]["monotonic_ns"]-endpoints[0]["monotonic_ns"])/1e6,
        }
    row.update(phase_cost)


def score(folder, label, cfg, phase):
    shape, name, policy = label.split("-")
    lane = cfg["lanes"][label]
    engine, inv = lane["engine"], lane["inventory"]
    raw_folder = folder / f"{phase}-{engine}"
    raw_path = raw_folder / "raw.tar.gz"
    if shape == "two":
        row = equal.score(raw_folder, engine, phase, inv, cfg["cap71_design"])
    else:
        row = codec_packet.score(folder, label, cfg, phase)
        row.update(submit_ms=row["submit_to_last_settlement_ms"], drain_ms=row["commit_to_last_settlement_ms"])
    with tarfile.open(raw_path, "r:gz") as archive:
        def artifact(member):
            return matrix.file_json(archive, "adapter/" + member)
        ledger = artifact("ledger.json")
        starts = None
        if shape == "two":
            pg = artifact("codec-postgres.json")
            settings = {key: "on" for key in ("ssl", "fsync", "synchronous_commit", "full_page_writes",
                                               "track_commit_timestamp")}
            settings.update(server_version_num="170011", default_toast_compression=policy, max_connections="71",
                            reserved_connections="0", superuser_reserved_connections="0",
                            shared_preload_libraries="pg_stat_statements")
            require(pg == dict(path=cfg["postgres"]["prefix"] + "/bin/postgres", sha256=PG_SHA, settings=settings),
                    "actual cap-71 PostgreSQL identity or policy differs")
            receipt = artifact("payload-storage.json")
            security = receipt["security"]
            require(security["observer_tls"] is True and security["database_limit"] == 71
                    and security["codec_overrides"] == 0, "TLS/cap or role codec overrides differ")
            row.update(codec_packet.original.verify_storage(receipt, ledger, engine, policy))
            row["compression_counts"] = verify_codec(codec_packet.storage, engine, receipt["persisted"],
                                                      ledger["jobs"], policy)
            starts = artifact("startup-clock.json")
        phases_and_readiness(archive, ledger, phase, engine, starts, row, shape == "two")
        with archive.extractfile("host-samples.jsonl") as stream:
            points = [json.loads(line) for line in stream]
        require(points[-1]["monotonic_ns"] > points[0]["monotonic_ns"], "nonpositive recorder wall time")
        row["recorder_wall_ms"] = (points[-1]["monotonic_ns"]-points[0]["monotonic_ns"])/1e6
    row["attempt_id"] = read(raw_folder / "request.json")["attempt"]
    row["raw_sha256"] = sha(raw_path)
    return row


def arm_grid(mode, group, cfg):
    phases = ("diagnostic",) if mode == "proof" else PHASES
    return [(phase, label) for phase in phases for label in cfg["orders"][group]]


def replay(mode, cfg):
    rows = {}
    previous = set()
    for group in MODES[mode][0]:
        directory = OUT / mode / f"{group:02d}"
        scored, rejected = {}, {}
        planned = [list(arm) for arm in arm_grid(mode, group, cfg)]
        order = directory / "order.json"
        if not order.is_file() or read(order) != planned:
            rejected["order"] = "missing or changed recorded phase order"
        for phase, label in planned:
            key = f"{phase}-{label}"
            try:
                row = score(directory / key, label, cfg, phase)
                require(row["attempt_id"] not in previous, "native attempt reused")
                previous.add(row["attempt_id"])
                scored[key] = row
            except (OSError, KeyError, ValueError, tarfile.TarError, StopIteration, TypeError) as error:
                rejected[key] = str(error)
        rows[str(group)] = dict(scores=scored, rejected=rejected, eligible=not rejected)
    return rows


def released(mode):
    req = file_for(mode)
    path = codec_packet.QUEUE / f"{ID}-{mode}.released.json"
    data = read(path)
    if "raw_sha256" in data:
        require(data["raw_sha256"] == read(OUT / mode / "raw-sha256.json"), f"{mode} release raw hashes differ")
    require(data["id"] == f"{ID}-{mode}" and data["exit_status"] == 0 and data["released_at"]
            and data["request_sha256"] == sha(req)
            and read(OUT / mode / "grant.json")["packet_sha256"] == sha(req),
            f"{mode} canonical release or custody missing")
    return path


def stage(cfg):
    common = Path(cfg["composition_remote"])
    # Only the controller stages these pinned source members. The
    # old River executable and the isolated PG directory are read-only inputs.
    for name, source in BUNDLE.items():
        # BUNDLE contains aliases, so stage to the exact sealed target name.
        if source.name == name:
            with (OUT / "proof" / f"stage-{name}.log").open("xb") as log:
                matrix.stage(source, common, log)
        else:
            with source.open("rb") as stream:
                command = ["bash", "-ec", 'set -o pipefail; umask 077; '
                           'if test -f "$1"; then cat >/dev/null; '
                           'printf "%s  %s\\n" "$2" "$1" | sha256sum -c; else '
                           'tmp=$(mktemp "$1.partial.XXXXXX"); cat >"$tmp"; '
                           'printf "%s  %s\\n" "$2" "$tmp" | sha256sum -c; '
                           'ln "$tmp" "$1"; unlink "$tmp"; fi', "--", str(common / name), sha(source)]
                matrix.ssh(command, stdin=stream, timeout=30).check_returncode()
    matrix.ssh(["chmod", "0711", str(common)], timeout=12).check_returncode()
    matrix.ssh(["chmod", "0444", *(str(common / name) for name in BUNDLE)], timeout=12).check_returncode()


def verify_retained_host_files(cfg, folder):
    common = cfg["composition_remote"] + "/"
    pins = {}
    for lane in cfg["lanes"].values():
        spec = lane["inventory"]["engines"][lane["engine"]]
        for name, digest in spec["verified_files"].items():
            if not name.startswith(common):
                require(name not in pins or pins[name] == digest, f"conflicting retained artifact pin: {name}")
                pins[name] = digest
    contents = "".join(f"{digest}  {path}\n" for path, digest in sorted(pins.items()))
    with (folder / "retained-native-sha256.log").open("x") as log:
        matrix.ssh(["sha256sum", "-c", "-"], input=contents, text=True,
                   stdout=log, stderr=log, timeout=120).check_returncode()


def dispatch(mode, cfg):
    grant(mode)
    with (SUITE / ".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        admission = grant(mode)
        folder = OUT / mode
        folder.mkdir(parents=True)
        write(folder / "grant.json", admission)
        matrix.ssh(["bash", "-c", codec_packet.original.HOST_IDLE], timeout=12).check_returncode()
        verify_retained_host_files(cfg, folder)
        if mode == "proof":
            stage(cfg)
        for label, lane in cfg["lanes"].items():
            remote = matrix.REMOTE / f"{ID}-{mode}-{label}"
            temp = folder / "inventory" / label
            temp.mkdir(parents=True)
            write(temp / "inventory.json", lane["inventory"])
            with (temp / "transfer.log").open("xb") as log:
                matrix.stage(SUITE / "worker.py", remote, log)
                matrix.stage(temp / "inventory.json", remote, log)
        for group in MODES[mode][0]:
            matrix.ssh(["bash", "-c", codec_packet.original.HOST_IDLE], timeout=12).check_returncode()
            directory = folder / f"{group:02d}"
            directory.mkdir()
            grid = arm_grid(mode, group, cfg)
            write(directory / "order.json", grid)
            for phase, label in grid:
                grant(mode, 90)
                arm = directory / f"{phase}-{label}"
                arm.mkdir()
                matrix.dispatch_arm(arm, matrix.REMOTE / f"{ID}-{mode}-{label}", {"profile": "slow"},
                                    phase, cfg["lanes"][label]["engine"])
        result = replay(mode, cfg)
        write(folder / "replay.json", result)
        hashes = {path.relative_to(folder).as_posix(): sha(path)
                  for path in sorted(folder.rglob("raw.tar.gz"))}
        write(folder / "raw-sha256.json", hashes)
        matrix.ssh(["bash", "-c", codec_packet.original.HOST_IDLE], timeout=12).check_returncode()
        return 0 if all(row["eligible"] for row in result.values()) else 1


def paired(rows):
    metrics = (*CLOCKS, *PHASE_CLOCKS, "recorder_wall_ms")
    by_order = {}
    for group, result in rows.items():
        if not result["eligible"]:
            continue
        arms = result["scores"]
        def diff(a, b):
            left, right = (arms[f"measured-{key}"] for key in (a, b))
            return {clock: left[clock]-right[clock] for clock in metrics}
        by_order[group] = dict(
            codec={name: diff(f"two-{name}-pglz", f"two-{name}-lz4") for name in ENGINES},
            mesh_minus_peer={policy: {name: diff(f"two-bulk-{policy}", f"two-{name}-{policy}")
                                      for name in ENGINES if name != "bulk"} for policy in ("pglz", "lz4")},
            pglz_deployment_bridge={name: diff(f"one-{name}-pglz", f"two-{name}-pglz") for name in ENGINES},
        )
    return by_order


def distribution(leaves):
    if isinstance(leaves[0], dict):
        return {key: distribution([row[key] for row in leaves]) for key in leaves[0]}
    return dict(n=len(leaves), median=median(leaves), minimum=min(leaves), maximum=max(leaves),
                positive=sum(value > 0 for value in leaves), negative=sum(value < 0 for value in leaves))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("seal", "check", "proof", "pilot", "full", "replay-proof",
                                          "replay-pilot", "replay-full", "score"))
    action = parser.parse_args().action
    cfg, pins = design()
    inventory = HERE / "inventory.json"
    if action == "seal":
        require(not any((codec_packet.QUEUE / f"{ID}-{mode}{suffix}").exists() for mode in MODES
                        for suffix in (".json", ".released.json")),
                "matching controller request already exists")
        write(inventory, cfg)
        for mode in MODES:
            write(file_for(mode), request(mode, cfg, pins))
        return 0
    cfg["orders"] = {str(key): value for key, value in cfg["orders"].items()}
    require(read(inventory) == cfg, "inventory changed")
    cfg["orders"] = {int(key): value for key, value in cfg["orders"].items()}
    for mode in MODES:
        require(read(file_for(mode)) == request(mode, cfg, pins), f"{mode} request changed")
    if action == "check":
        print(json.dumps({mode: sha(file_for(mode)) for mode in MODES}, indent=2))
        return 0
    if action in MODES:
        prerequisites = {"proof": (), "pilot": ("proof",), "full": ("proof", "pilot")}[action]
        for mode in prerequisites:
            released(mode)
            require(all(row["eligible"] for row in replay(mode, cfg).values()), f"{mode} replay failed")
        return dispatch(action, cfg)
    if action == "score":
        for mode in MODES:
            released(mode)
        rows = {**replay("pilot", cfg), **replay("full", cfg)}
        require(len(rows) == 6 and all(row["eligible"] for row in rows.values()), "complete six matched rotations required")
        pairs = paired(rows)
        print(json.dumps(dict(selected=rows, per_order=pairs, distributions=distribution(list(pairs.values()))),
                         indent=2, sort_keys=True))
    else:
        print(json.dumps(replay(action.removeprefix("replay-"), cfg), indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
