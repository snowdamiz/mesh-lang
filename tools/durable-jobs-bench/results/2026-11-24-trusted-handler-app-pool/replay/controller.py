"""Verify or dispatch the bounded app-pool comparison under the matrix controller."""
import argparse
from copy import deepcopy
from datetime import datetime, timezone
import fcntl
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile
from uuid import uuid4

HERE = Path(__file__).resolve().parent
PACKET = HERE.parent
SUITE = PACKET.parents[1]
ROOT = SUITE.parents[1]
DENSE = PACKET.parent / "2026-11-23-dense-matched-topology"
TOPOLOGY = PACKET.parent / "2026-11-23-ordinary-runner-topology"
V1 = PACKET.parent / "2026-11-23-ordinary-trusted-v1"
EXIT_CONFIRMED = PACKET.parent / "2026-11-23-ordinary-trusted-product-db5937e7"
sys.path[:0] = [str(SUITE), str(DENSE)]

import run
from fixtures import FILES, FROZEN_MAIN, MODES as BUILDS, WRITES, main_source
from interleave import HOST_IDLE
from score import GEOMETRY, MODES, evaluate, score_arm
from verify import expected_inputs, reconcile, require

ID = PACKET.name
QUEUE = ROOT / "target/linode-jobs-stepchange/requests"
BASE = Path("/var/lib/durable-jobs-bench/native")
STOCK = BASE / "builds/ordinary-trusted-v1-23fdb3f5530d"
HOST = ["90ec1c24-08c8-4bc1-aa28-79b6db1bd13a", "6.8.0-149-generic", "8:0"]
POSTGRES = "b70863988c4762b6b914ac54d194fa5a1723201100d8155caf7781edba29faa1"
SOURCE = "db5937e7e8988f935e9be16b68a2561c2250cc27"
EVIDENCE = PACKET / "evidence"
REQUEST = PACKET / "request.json"
STAGING = ROOT / "target/trusted-handler-app-pool"
BUNDLE = PACKET / "candidate-source.tar.gz"
LANES = ("service-a", "direct", "service-observed", "v1", "direct-observed", "service-b")
OFFSETS = (0, 1, -1, 2, -2, 3)


def schedule():
    return [dict(group=group, profile=profile,
                 phases=["warmup", "measured"] if profile == "noop" else ["measured"],
                 order=[LANES[(group - 1 + offset) % len(LANES)] for offset in OFFSETS])
            for group in range(1, 7)
            for profile in (("noop", "short") if group % 2 else ("short", "noop"))]


def historical_paths():
    equal = DENSE / "evidence/equal"
    measured = run.read_json(equal / "replay.json")["blocks"]
    dense = [equal / "blocks" / key / f"measured-{lane}/measured-{engine}/raw.tar.gz"
             for key in sorted(measured)
             for lane, engine in (("mesh-v1-2x32", "mesh"), ("river-2x32", "go"))]
    short_series = V1 / "evidence/ordinary-trusted-v1-short"
    short = sorted((short_series / "blocks").glob("*/measured-ordinary/measured-mesh/raw.tar.gz"))
    exit_series = EXIT_CONFIRMED / "evidence/ordinary-product-v1-short-dense"
    exit_archives = sorted((exit_series / "blocks").glob("*/measured-*/measured-mesh/raw.tar.gz"))
    require(len(dense) == 10 and len(short) == 5 and len(exit_archives) == 4, "historical raw set drifted")
    return {"equal_dense": dense, "v1_short": short, "exit_confirmed": exit_archives}


def history():
    from qualification import diagnostic_reconciler
    paths = historical_paths()
    dense_summary = run.read_json(DENSE / "evidence/equal/replay.json")["blocks"]
    prior = {}
    for group, archives in paths.items():
        rows = []
        for path in archives:
            with tarfile.open(path) as archive:
                ledger = run.file_json(archive, "adapter/ledger.json")
                build = {field: ledger[field] for field in ("version", "artifact_sha256")}
                engine, profile = ledger["engine"], ledger["profile"]
                verify = diagnostic_reconciler(2) if group == "equal_dense" else reconcile
                # Historical product transfer and its V1 peers used the original dense topology.
                score = verify(ledger, engine, profile, build)
                sha = run.digest(path)
                record = dict(path=str(path.relative_to(ROOT)), sha256=sha, engine=engine,
                              artifact_sha256=build["artifact_sha256"], profile=profile,
                              submit_jobs_per_s=score["submit_jobs_per_s"],
                              commit_jobs_per_s=score["commit_jobs_per_s"])
                if group == "equal_dense":
                    lane = "mesh-v1-2x32" if engine == "mesh" else "river-2x32"
                    old = dense_summary[path.parents[2].name][f"measured-{lane}"]
                    require(old["raw_sha256"] == sha and old["commit_jobs_per_s"] == record["commit_jobs_per_s"],
                            "historical dense raw replay drifted")
                    if engine == "mesh":
                        record["used_application_sessions"] = old["topology"]["used_application_sessions"]
                rows.append(record)
        prior[group] = rows
    return prior


def check(require_request=True):
    if require_request:
        request = run.read_json(REQUEST)
        for path, digest in request["local_sha256"].items():
            require(run.digest(ROOT / path) == digest, f"changed input {path}")
        require(request["group_plan"] == schedule() and request["source_sha"] == SOURCE
                and request["native_host"] == HOST and request["source_archive_sha256"] == run.digest(BUNDLE),
                "schedule, source, or host pin differs")
        require(len(request["group_plan"]) == 12
                and sum(len(block["order"]) * len(block["phases"]) for block in schedule()) == 108,
                "comparison exceeded the frozen work bound")
    with tarfile.open(BUNDLE) as archive:
        fixture = archive.extractfile("stage/fixture/main.mpl").read()
        require(hashlib.sha256(fixture).hexdigest() == FROZEN_MAIN, "wrong baseline fixture")
        require(hashlib.sha256(archive.extractfile("stage/fixture/writes.mpl").read()).hexdigest() == WRITES,
                "application SQL source changed")
        require(main_source(fixture.decode(), "service").encode() == fixture, "service control changed")
        direct = main_source(fixture.decode(), "direct")
        require("ApplicationPool" not in direct and "Process.whereis" not in direct
                and "Jobs.start_with_app_pool(pool, trusted, handle, app_pool)" in direct, "direct branch still uses registry")
        for mode in BUILDS:
            compiled = main_source(fixture.decode(), mode)
            require(compiled.count("record_attempt(pool,") == compiled.count("record_effect(pool,") == 1,
                    "fixture write call count changed")
        for name in ("fixtures.py", "link-fixtures.sh", "semantic.sh"):
            require((HERE / name).read_bytes() == archive.extractfile(f"stage/{name}").read(), f"archived {name} changed")
        require(archive.extractfile("stage/semantics.mpl").read() == (PACKET / "semantics/main.mpl").read_bytes(),
                "archived semantic witness changed")
        for name in ("db/pool.rs", "db/pg.rs", "jobs/schema.sql", "actor/scheduler.rs", "actor/stack.rs"):
            path = f"compiler/mesh-rt/src/{name}"
            baseline = subprocess.check_output(["git", "show", f"{SOURCE}:{path}"], cwd=ROOT)
            require(archive.extractfile(path).read() == baseline, f"changed protected substrate: {path}")
    for path in HERE.glob("*.py"):
        compile(path.read_text(), str(path), "exec")
    for path in HERE.glob("*.sh"):
        subprocess.run(["bash", "-n", path], check=True)
    for profile in GEOMETRY:
        expected_inputs(profile)
    current_history = history()
    if require_request:
        require(run.read_json(PACKET / "historical-fingerprints.json") == current_history, "historical replay drifted")
    return current_history


def grant(kind):
    require(REQUEST.read_bytes() == (QUEUE / f"{ID}.json").read_bytes(), "sole-controller inbox differs from seal")
    events = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD)\s+(\S+)\s+(\S+)\s+(\S+)",
                        (QUEUE / "STATUS.md").read_text(), flags=re.M)
    require(events and events[-1][:2] == (kind, ID), f"active sole-controller {kind} grant required")
    _, _, start, end = events[-1]
    now = datetime.now(timezone.utc)
    require(datetime.fromisoformat(start) <= now < datetime.fromisoformat(end), "controller grant is expired")
    return dict(start=start, end=end, request_sha256=run.digest(REQUEST))


def budget_left(active, maximum, spare=30):
    left = int((datetime.fromisoformat(active["end"]) - datetime.now(timezone.utc)).total_seconds())
    require(left > spare, "insufficient time left in controller grant")
    return min(maximum, left - spare)


def setup_qualification(attempt, out):
    with tarfile.open(out / "raw.tar.gz") as archive:
        require(run.file_json(archive, "./status.json")["exit_code"] == 0, "AMD64 setup failed")
        host = archive.extractfile("./setup-host.txt").read().decode().splitlines()
        require(host[:5] == [f"Linux {HOST[1]} x86_64", HOST[0], HOST[2], "100", "4"]
                and host[5].split()[0] == POSTGRES, "AMD64 boot, PG, disk, or toolchain differs")
        require("APP_POOL_SCOPE_OK" in archive.extractfile("./semantics.log").read().decode().splitlines(),
                "native two-runner pool proof did not pass")
        require(host[6].startswith("rustc 1.99.0") and host[7] == "21.1.8", "native build tools changed")
        fixtures = {}
        with tarfile.open(BUNDLE) as frozen:
            source = frozen.extractfile("stage/fixture/main.mpl").read().decode()
            input_pins = {name: hashlib.sha256(frozen.extractfile(f"stage/fixture/{name}").read()).hexdigest()
                          for name in FILES[1:]}
        for mode in BUILDS:
            data = archive.extractfile(f"./{mode}.json").read()
            build = json.loads(data)
            require((build["system"], build["architecture"], build["version"], build["mode"])
                    == ("Linux", "x86_64", SOURCE, mode)
                    and build["source_archive_sha256"] == run.digest(BUNDLE), "candidate build provenance differs")
            require(build["files"]["main.mpl"] == hashlib.sha256(main_source(source, mode).encode()).hexdigest()
                    and all(build["files"][name] == digest for name, digest in input_pins.items()),
                    "linked fixture or unchanged SQL differs")
            fixtures[mode] = dict(build=build, manifest_sha256=hashlib.sha256(data).hexdigest())
        require(len({f["build"]["runtime_sha256"] for f in fixtures.values()}) == 1
                and len({f["build"]["compiler_sha256"] for f in fixtures.values()}) == 1
                and len({f["build"]["files"]["mesh"] for f in fixtures.values()}) == 4, "treatment isolation changed")
    return dict(attempt=attempt, raw_sha256=run.digest(out / "raw.tar.gz"), fixtures=fixtures)


def setup():
    active = grant("SETUP")
    check()
    attempt = f"app-pool-{uuid4().hex[:12]}"
    out = EVIDENCE / "setup" / attempt
    require(not list(out.parent.glob("app-pool-*")), "setup already attempted, preserve evidence and reseal a retry")
    with (SUITE / ".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        require(grant("SETUP") == active, "SETUP grant changed")
        out.mkdir(parents=True)
        budget = budget_left(active, 1260, spare=240)
        shell = (HERE / "native-setup.sh").read_text()
        argv = ["bash", "-c", shell, "app-pool-native-setup", run.digest(BUNDLE), attempt, str(budget)]
        run.write_json(out / "request.json", dict(grant=active, attempt=attempt, argv=argv,
                                                 source=str(BUNDLE.relative_to(ROOT))))
        with BUNDLE.open("rb") as source, (out / "raw.tar.gz").open("xb") as raw, (out / "dispatch.log").open("xb") as log:
            try:
                result = run.ssh(argv, stdin=source, stdout=raw, stderr=log,
                                 timeout=budget_left(active, 1500, spare=20))
            except subprocess.TimeoutExpired:
                run.write_json(out / "dispatch.json", dict(interrupted=True))
                raise
        run.write_json(out / "dispatch.json", dict(exit_code=result.returncode, finished=run.utc_now()))
        result.check_returncode()
        qualified = setup_qualification(attempt, out)
        run.ssh(["bash", "-c", HOST_IDLE], timeout=12).check_returncode()
        run.write_json(EVIDENCE / "native-qualification.json", qualified)


def qualified_setup():
    result = run.read_json(EVIDENCE / "native-qualification.json")
    attempt = result["attempt"]
    require(bool(re.fullmatch(r"app-pool-[0-9a-f]{12}", attempt)), "invalid setup attempt ID")
    require(setup_qualification(attempt, EVIDENCE / "setup" / attempt) == result, "SETUP receipts changed")
    return result


def inventories(qualification):
    native = run.read_json(DENSE / "equal.json")["inventory"]
    worker = SUITE / "worker.py"
    files = {"worker.py": worker, "native-arm.sh": HERE / "native-arm.sh",
             "native-arm.py": HERE / "native-arm.py", "topology_arm.py": TOPOLOGY / "arm.py"}
    for lane, mode in MODES.items():
        inventory = deepcopy(native)
        spec = inventory["engines"]["mesh"]
        remote = run.REMOTE / f"{qualification['attempt']}-{lane}"
        runner = STOCK
        if mode != "v1":
            runner = BASE / "builds" / qualification["attempt"] / mode
            fixture = qualification["fixtures"][mode]
            build = fixture["build"]
            spec["artifact_sha256"], spec["version"] = fixture["manifest_sha256"], build["version"]
            spec["verified_files"].update({str(runner / "mesh"): build["files"]["mesh"],
                                           str(runner / "mesh.json"): fixture["manifest_sha256"]})
        spec["verified_files"].update({str(remote / name): run.digest(path) for name, path in files.items()})
        spec["verified_files"][str(BASE / "toolchain/pgsql-17.11/bin/postgres")] = POSTGRES
        spec["argv"] = ["/bin/bash", str(remote / "native-arm.sh"), mode, str(runner)]
        run.check_inventory(inventory)
        yield lane, remote, inventory, files


def replay_block(folder, block, entries):
    require(run.read_json(folder / "order.json") == block, "lane order differs from seal")
    scores = {}
    for phase in block["phases"]:
        scores[phase] = {}
        for lane in block["order"]:
            _, inventory, _ = entries[lane]
            directory = folder / f"{phase}-{lane}" / f"{phase}-mesh"
            outcome = run.read_json(directory / "dispatch.json")
            require(outcome.get("exit_code") == 0, f"{phase}-{lane} failed or was interrupted")
            scores[phase][lane] = score_arm(directory, block, phase, lane, inventory, HOST, 100)
    return {**block, **scores}


def timing(groups):
    active = grant("TIMING")
    check()
    require(groups in ([1, 2], [3, 4, 5, 6]), "timing requires the two sealed group batches")
    qualification = qualified_setup()
    entries = {lane: (remote, inventory, files) for lane, remote, inventory, files in inventories(qualification)}
    output = EVIDENCE / "timing"
    output.mkdir(exist_ok=True)
    if groups[0] == 3:
        for block in schedule():
            if block["group"] <= 2:
                replay_block(output / run.block_prefix(block), block, entries)
    with (SUITE / ".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        require(grant("TIMING") == active, "TIMING grant changed")
        run.ssh(["bash", "-c", HOST_IDLE], timeout=12).check_returncode()
        slot = output / f"grant-{groups[0]}"
        slot.mkdir()
        run.write_json(slot / "grant.json", active)
        for lane, (remote, inventory, files) in entries.items():
            stage = slot / lane
            stage.mkdir()
            run.write_json(stage / "inventory.json", inventory)
            with (stage / "transfer.log").open("xb") as log:
                for name, path in files.items():
                    dest = stage / name
                    shutil.copyfile(path, dest)
                    run.stage(dest, remote, log)
                run.stage(stage / "inventory.json", remote, log)
        for block in schedule():
            if block["group"] not in groups:
                continue
            folder = output / run.block_prefix(block)
            folder.mkdir()
            run.write_json(folder / "order.json", block)
            for phase in block["phases"]:
                for lane in block["order"]:
                    require(grant("TIMING") == active, "TIMING grant changed")
                    budget_left(active, 930, spare=210)
                    directory = folder / f"{phase}-{lane}"
                    directory.mkdir()
                    remote, _, _ = entries[lane]
                    run.dispatch_arm(directory, remote, block, phase, "mesh")
                    received = directory / f"{phase}-mesh"
                    require(run.read_json(received / "dispatch.json").get("exit_code") == 0
                            and (received / "raw.tar.gz").is_file(), "failed native arm, preserve raw and reconcile transport")
            run.write_json(folder / "reconciled.json", replay_block(folder, block, entries))
        run.ssh(["bash", "-c", HOST_IDLE], timeout=12).check_returncode()
    replay()


def replay():
    check()
    qualification = qualified_setup()
    entries = {lane: (remote, inventory, files) for lane, remote, inventory, files in inventories(qualification)}
    output = EVIDENCE / "timing"
    blocks, failures, missing = [], {}, []
    for block in schedule():
        name = run.block_prefix(block)
        folder = output / name
        if not folder.is_dir():
            missing.append(name)
            continue
        try:
            blocks.append(replay_block(folder, block, entries))
        except (OSError, ValueError, KeyError, TypeError, IndexError, tarfile.TarError) as error:
            failures[name] = f"{type(error).__name__}: {error}"
    result = dict(blocks=blocks, missing=missing, failures=failures, paired=evaluate(blocks),
                  native_qualification=qualification["raw_sha256"])
    run.write_json(output / f"replay-{uuid4().hex[:12]}.json", result)
    print(json.dumps(dict(count=len(blocks), missing=missing, failures=failures), indent=2))
    return bool(failures)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=("check", "setup", "verify-setup", "timing", "replay"))
    parser.add_argument("groups", nargs="*", type=int)
    args = parser.parse_args()
    if args.operation == "check":
        check()
        print("Sealed sources, six rotations per workload, and 19 original native ledgers verified offline")
    elif args.operation == "setup":
        setup()
    elif args.operation == "verify-setup":
        check()
        print(qualified_setup()["raw_sha256"])
    elif args.operation == "timing":
        timing(args.groups)
    else:
        return replay()


if __name__ == "__main__":
    sys.exit(main())
