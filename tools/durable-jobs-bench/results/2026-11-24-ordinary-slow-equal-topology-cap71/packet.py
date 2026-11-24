"""Run the separate slow cap-71 sensitivity through the sole controller."""

import argparse
from datetime import datetime, timezone
import fcntl
import json
from pathlib import Path
import re
from statistics import median
import subprocess
import sys
import tarfile
import time
from uuid import uuid4

from adapter import CAP, INPUT, LAUNCHERS
from qualification import CPUS, PACKET, PRIOR, ROOT, SUITE, TOPOLOGY, require, run, score
from interleave import HOST_IDLE

ID = "ordinary-slow-equal-topology-cap71"
TARGET = ROOT / "target" / ID
QUEUE = ROOT / "target/linode-jobs-stepchange/requests"
BASELINE = PRIOR / "evidence/native-import-retry/series.json"
BUILD = PACKET / "evidence/river-build.json"
BINARY = "river-slow-2x32-cap71"
PG = "/var/lib/durable-jobs-bench/native"
PG_PINS = {
    f"{PG}/toolchain/pgsql-17.11/bin/postgres": "b70863988c4762b6b914ac54d194fa5a1723201100d8155caf7781edba29faa1",
    f"{PG}/toolchain/pgsql-17.11/lib/pg_stat_statements.so": "0cb86ff6113361a6e677309642c97a2779d4234d5b615ef8e740d7d9f8a4d637",
}
ERRORS = (OSError, ValueError, TypeError, KeyError, IndexError, tarfile.TarError)
INPUTS = (
    BASELINE, BUILD, PACKET / "evidence/river-profile.diff", PACKET / "evidence/cap-proof.json",
    PACKET / "build-river.py", PACKET / "adapter.py", PACKET / "run-native.sh", Path(__file__),
    PACKET / "qualification.py", PACKET / "offline.py",
    PRIOR / "replay.py", PRIOR / "slow_arm.py", TOPOLOGY / "replay.py", TOPOLOGY / "arm.py",
    PRIOR.parent / "2026-11-23-ordinary-claim-turns/replay.py",
    SUITE / "run.py", SUITE / "worker.py", SUITE / "verify.py", SUITE / "interleave.py",
    ROOT / "scripts/jobs-parity.py", ROOT / "scripts/jobs-parity-profiles.json",
)


def source_pins():
    return {str(path.relative_to(ROOT)): run.digest(path) for path in INPUTS}


def design():
    base = run.read_json(BASELINE)["config"]
    engines = base["lanes"]["two"]["inventory"]["engines"]
    require(base["cpus"] == CPUS and base["profiles"] == ["slow"], "wrong slow source manifest")
    river = run.read_json(BUILD)
    require(engines["go"]["artifact_sha256"] == river["stock_sha256"]
            and river["before"] == f'"slow":   {{1, 64, 68, true, "{INPUT}"}},'
            and river["after"] == f'"slow":   {{2, 64, 71, true, "{INPUT}"}},'
            and river["patch_sha256"] == run.digest(PACKET / "evidence/river-profile.diff"),
            "River source receipt does not describe the verified configuration-only change")
    base_order = ("mesh", "go", "rust", "oban")
    orders = [list(base_order[i:] + base_order[:i]) for i in range(4)]
    orders = [order for pair in zip(orders, [order[::-1] for order in orders], strict=True) for order in pair]
    for engine in base_order:
        require(all(order.count(engine) == 1 for order in orders)
                and all(sum(order[index] == engine for order in orders) == 2 for index in range(4)),
                "timing orders are not position balanced")
    plan = [dict(group=index, profile="slow", phases=["warmup", "measured"], order=order)
            for index, order in enumerate(orders, 1)]
    return dict(host=base["host"], clock_ticks=base["clock_ticks"], cpus=CPUS, cap=CAP, input_sha256=INPUT,
                plan=plan, stock_engines=engines, river_sha=river["candidate_sha256"],
                mesh_elf="e86640db9d7dcd9b7c7fff7e8a2183000cdbbb0c5d277f3544ebf88438d60e25")


def launcher(config, engine, remote):
    original = config["stock_engines"][engine]
    argv = original["argv"]
    if engine in ("mesh", "oban"):
        path = Path(argv[-2]).with_name("arm.py")
        arts = argv[-3].split("=", 1)[1]
        scratch = f"{PG}/scratch"
    else:
        path = Path(argv[0]).with_name("run.py")
        scratch = argv[argv.index("--scratch") + 1]
        arts = (str(remote / BINARY) if engine == "go" else
                str(path.parents[3] / "target/linode-apalis/build/release/durable-jobs-apalis-bench"))
    prior = original["verified_files"]
    require(prior.get(str(path)) == LAUNCHERS[engine], f"{engine} launcher is not pinned")
    require(engine != "mesh" or config["mesh_elf"] in prior.values(), "missing retained bulk V1 executable")
    pins = {**prior, **PG_PINS,
            **{str(remote / name): run.digest(PACKET / name) for name in ("adapter.py", "run-native.sh")}}
    if engine == "go":
        pins[arts] = config["river_sha"]
    if engine == "rust":
        pins[arts] = original["artifact_sha256"]
    spec = {key: value for key, value in original.items() if key != "run_as"}
    spec.update(argv=["/bin/bash", str(remote / "run-native.sh"), engine, str(path), arts, scratch],
                verified_files=pins,
                artifact_sha256=config["river_sha"] if engine == "go" else original["artifact_sha256"],
                evidence_globs=list(dict.fromkeys([*original["evidence_globs"], "*.json", "physical-postgres.log"])))
    result = dict(cpus=CPUS, engines={**config["stock_engines"], engine: spec})
    run.check_inventory(result)
    return result


def grant(mode, pins):
    stage = "SETUP" if mode == "setup" else "TIMING"
    ident = f"{ID}-{mode}"
    events = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD)\s+(\S+)\s+(\S+)\s+(\S+)",
                        (QUEUE / "STATUS.md").read_text(), flags=re.M)
    require(events and events[-1][:2] == (stage, ident), f"no active sole-controller {stage} grant")
    _, _, start, end = events[-1]
    require(datetime.fromisoformat(start) <= datetime.now(timezone.utc) < datetime.fromisoformat(end),
            "controller grant is outside its window")
    request = PACKET / f"request-{mode}.json"
    require(request.read_bytes() == (QUEUE / f"{ident}.json").read_bytes()
            and run.read_json(request)["local_sha256"] == pins, "controller request or source seal differs")
    return dict(start=start, end=end, request_sha256=run.digest(request), mode=stage)


def folders(directory, block):
    for phase in block["phases"]:
        for engine in block["order"]:
            parent = directory / f"{phase}-{engine}"
            yield phase, engine, parent, parent / f"{phase}-{engine}"


def replay_block(path, config, block, remotes):
    require(run.read_json(path / "order.json") == block, "archived order changed")
    rows = {}
    for phase, engine, _, folder in folders(path, block):
        inventory = launcher(config, engine, run.REMOTE / remotes[engine])
        rows[f"{phase}-{engine}"] = score(folder, engine, phase, inventory, config)
    return rows


def select(directory, config, block, remotes, rejected):
    for attempt in sorted((directory / "blocks").glob(f"{run.block_prefix(block)}.*")):
        try:
            return attempt.name, replay_block(attempt, config, block, remotes)
        except ERRORS as error:
            rejected[attempt.name] = f"{type(error).__name__}: {error}"
    return None


def plan(config, mode):
    if mode == "setup":
        return [dict(config["plan"][0], phases=["warmup"])]
    return config["plan"]


def frozen(config, mode, pins):
    return dict(design=config, plan=plan(config, mode), pins=pins, mode=mode)


def replay(directory, config, mode, pins):
    manifest = run.read_json(directory / "series.json")
    require(all(manifest[key] == value for key, value in frozen(config, mode, pins).items()),
            "series manifest no longer matches the sealed packet")
    rows, rejected, missing, peer_deltas = {}, {}, [], {}
    for block in manifest["plan"]:
        entry = select(directory, config, block, manifest["remotes"], rejected)
        if entry is None:
            missing.append(block["group"])
        else:
            name, arms = entry
            rows[name] = arms
    if mode == "timing" and rows:
        for engine in ("go", "oban", "rust"):
            peer_deltas[engine] = {}
            for metric in ("submit_ms", "drain_ms"):
                values = [arms[f"measured-{engine}"][metric] - arms["measured-mesh"][metric]
                          for arms in rows.values()]
                peer_deltas[engine][metric] = dict(by_block=values, median=median(values),
                    minimum=min(values), maximum=max(values), mesh_wins=sum(value > 0 for value in values))
    return dict(mode=mode, common_cap=CAP, qualified=rows, missing=missing, rejected=rejected,
                peer_minus_mesh_ms=peer_deltas)


def dispatch(directory, config, mode, pins, groups, binary, setup=None):
    assignment = grant(mode, pins)
    require(run.digest(binary) == config["river_sha"], "staged River ELF does not match the build seal")
    if mode == "timing":
        require(setup is not None, "timing requires the full native SETUP archive")
        result = replay(setup, config, "setup", pins)
        require(not result["missing"] and len(result["qualified"]) == 1,
                "all four real slow lifecycles must qualify before timing")
    with (SUITE / ".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        grant(mode, pins)
        run.ssh(["bash", "-c", HOST_IDLE], timeout=12).check_returncode()
        directory.mkdir(parents=True, exist_ok=True)
        manifest_path = directory / "series.json"
        seal = frozen(config, mode, pins)
        if not manifest_path.exists():
            slug = f"slow-equal-cap71-{mode}-{uuid4().hex[:12]}"
            run.write_json(manifest_path, dict(seal, remotes={engine: f"{slug}-{engine}" for engine in run.ENGINES}))
        manifest = run.read_json(manifest_path)
        require(all(manifest[key] == value for key, value in seal.items()), "series manifest drifted")
        run.write_json(directory / f"grant-{time.time_ns()}.json", assignment)
        remotes = manifest["remotes"]
        for engine, name in remotes.items():
            grant(mode, pins)
            stage = directory / "staging" / engine
            stage.mkdir(parents=True, exist_ok=True)
            remote = run.REMOTE / name
            inv = stage / "inventory.json"
            material = launcher(config, engine, remote)
            if not inv.exists():
                run.write_json(inv, material)
            require(run.read_json(inv) == material, "staged inventory differs")
            assets = [SUITE / "worker.py", PACKET / "adapter.py", PACKET / "run-native.sh", inv]
            if engine == "go":
                assets.append(binary)
            with (stage / f"transfer-{time.time_ns()}.log").open("xb") as log:
                for path in assets:
                    run.stage(path, remote, log)
        for block in manifest["plan"]:
            if groups and block["group"] not in groups:
                continue
            grant(mode, pins)
            run.ssh(["bash", "-c", HOST_IDLE], timeout=12).check_returncode()
            for attempt in sorted((directory / "blocks").glob(f"{run.block_prefix(block)}.*")):
                for _, engine, _, folder in folders(attempt, block):
                    if (folder / "request.json").exists():
                        saved = run.read_json(folder / "request.json")
                        run.archive_arm(folder, run.REMOTE / remotes[engine], saved["attempt"])
            if select(directory, config, block, remotes, {}):
                continue
            attempt = directory / "blocks" / f"{run.block_prefix(block)}.{time.time_ns()}"
            attempt.mkdir(parents=True)
            run.write_json(attempt / "order.json", block)
            for phase, engine, parent, folder in folders(attempt, block):
                grant(mode, pins)
                parent.mkdir()
                run.dispatch_arm(parent, run.REMOTE / remotes[engine], block, phase, engine)
                outcome = run.read_json(folder / "dispatch.json")
                require(outcome.get("exit_code") == 0 and (folder / "raw.tar.gz").exists(),
                        f"{engine} arm did not exit cleanly with evidence; preserve attempt")
            run.write_json(attempt / "reconciled.json", replay_block(attempt, config, block, remotes))
        grant(mode, pins)
        run.ssh(["bash", "-c", HOST_IDLE], timeout=12).check_returncode()
    report = replay(directory, config, mode, pins)
    run.write_json(directory / f"summary-{time.time_ns()}.json", report)
    return bool(set(report["missing"]) & set(groups or [b["group"] for b in manifest["plan"]]))


def main():
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("action", choices=("check", "setup", "timing", "replay"))
    cli.add_argument("--mode", choices=("setup", "timing"), default="timing")
    cli.add_argument("--directory", type=Path)
    cli.add_argument("--setup-archive", type=Path, default=TARGET / "native-setup")
    cli.add_argument("--group", action="append", type=int, choices=range(1, 9))
    cli.add_argument("--binary", type=Path, default=TARGET / BINARY)
    args = cli.parse_args()
    config, pins = design(), source_pins()
    if args.action == "check":
        for engine in run.ENGINES:
            launcher(config, engine, run.REMOTE / engine)
        require(args.binary.name == BINARY and run.digest(args.binary) == config["river_sha"],
                "River candidate path or bytes differ")
        subprocess.run(["bash", "-n", str(PACKET / "run-native.sh")], check=True)
        print(json.dumps(dict(design=config, local_sha256=pins), sort_keys=True, indent=2))
        return 0
    mode = args.mode if args.action == "replay" else args.action
    directory = args.directory or TARGET / f"native-{mode}"
    if args.action == "replay":
        print(json.dumps(replay(directory, config, mode, pins), sort_keys=True, indent=2))
        return 0
    require(args.binary.name == BINARY and (mode != "setup" or args.group in (None, [1])), "invalid plan selection")
    return dispatch(directory, config, mode, pins, args.group, args.binary, args.setup_archive)


if __name__ == "__main__":
    sys.exit(main())
