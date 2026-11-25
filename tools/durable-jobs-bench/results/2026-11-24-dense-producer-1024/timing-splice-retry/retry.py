"""Replay the private dense failure and reuse SETUP under a fresh timing grant."""
import argparse
from datetime import datetime, timezone
import fcntl
from io import BytesIO
import json
from pathlib import Path
import re
import runpy
import subprocess
import sys
import tarfile
from uuid import uuid4

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import controller as original

run, matrix, require = original.run, original.matrix, original.require
ROOT = original.ROOT
OLD = original.EVIDENCE
SETUP = OLD / "setup"
FAILED = OLD / "pilot"
FROZEN = original.BASE / "source-bulk-enqueue-3b09dc3d0e23/ordinary-v2"
FROZEN_ARCHIVE = HERE.parents[1] / "2026-11-23-set-based-enqueue/native/bulk-enqueue-ce5e551d079e.tar.gz"
EVIDENCE = HERE / "evidence"
PACKET = HERE / "intake.json"
ORIGINAL_STAGES = original.stages
HOST_PAYLOAD = ("adapter.py", "native-run", "entry_probe.py")
ARM = FROZEN / "tools/durable-jobs-bench/native/arm.py"
ARM_SHA = "d9f984dad988e2380b22e971ae5abf122619f246bf592a9402720d49996685ad"


def bind():
    packet = original.read_request()
    request = run.read_json(PACKET)
    for relative, expected in request["pins"].items():
        require(run.digest(ROOT / relative) == expected, f"recovery pin changed: {relative}")
    setup_release = SETUP / "controller-release.json"
    release = original.verify_release("setup", setup_release)
    require(release["released_at"] and release["request_sha256"] == run.digest(HERE.parent / "setup.json"),
            "successful SETUP release missing")
    failed = run.read_json(FAILED / "controller-release.json")
    require(failed["id"] == original.ID + "-pilot" and failed["groups"] == [1, 2]
            and failed["exit_status"] == 1 and failed["released_at"], "original failure has not released")
    setup_raw = ROOT / request["setup_raw"]
    broker = original.setup_binding(setup_raw, packet)
    require(broker == run.read_json(SETUP / "summary.json"), "archived SETUP binding changed")
    return request, packet, broker


def sibling(config):
    return Path(config["private_stage"]) / "packet-timing-splice"


def stages(config, label, remote):
    files, inventory = ORIGINAL_STAGES(config, label, remote)
    if label in original.dense.PEERS:
        return files, inventory
    spec = inventory["engines"]["mesh"]
    old = Path(config["private_stage"]) / "packet/native-run"
    new = sibling(config) / "native-run"
    require(spec["argv"].count(str(old)) == 1, "expected single staged launcher")
    spec["argv"][spec["argv"].index(str(old))] = str(new)
    spec["verified_files"].update(sibling_pins(config))
    run.check_inventory(inventory)
    return files, inventory


def sibling_pins(config):
    result = {str(sibling(config) / name): run.digest(HERE / name) for name in HOST_PAYLOAD}
    for name in ("backend.py", "identity.py", "wire.py", "install-writer.sql"):
        path = str(Path(config["private_stage"]) / "packet" / name)
        result[str(sibling(config) / name)] = config["private_files"][path]
    return result


def stage_request(mode):
    path = HERE / (mode + ".json")
    request = run.read_json(path)
    require(request["id"] == original.ID + f"-{mode}-splice-retry"
            and request["intake_sha256"] == run.digest(PACKET) and request["groups"] == [1, 2],
            "stage request does not bind to recovery intake")
    return request, path


def current_event():
    events = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD) (\S+) (\S+)(?: (\S+))?",
                        (matrix.QUEUE / "STATUS.md").read_text(), re.M)
    require(bool(events), "missing controller grant state")
    return events[-1]


def grant(mode):
    request, path = stage_request(mode)
    kind, ident, start, end = current_event()
    require((kind, ident) == ("TIMING", request["id"]) and end
            and datetime.fromisoformat(start) <= datetime.now(timezone.utc) < datetime.fromisoformat(end)
            and (matrix.QUEUE / (ident + ".json")).read_bytes() == path.read_bytes(),
            "matching live controller grant and sealed inbox bytes required")
    return dict(id=ident, start=start, end=end, request_sha256=run.digest(path))


def state(packet, broker, mode):
    config = original.make_config(packet, broker, mode)
    blocks = original.schedules(packet, mode)
    _, request = stage_request(mode)
    return EVIDENCE / mode, config, blocks, dict(
        base=run.digest(original.PACKET), recovery=run.digest(PACKET), grant=run.digest(request))


def install(config, mode):
    stamp = grant(mode)
    out = EVIDENCE / mode / "entry-preflight" / uuid4().hex
    out.mkdir(parents=True)
    run.write_json(out / "grant.json", stamp)
    with (original.dense.SUITE / ".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        grant(mode)
        _, inventory = ORIGINAL_STAGES(config, original.NARROW[0], run.REMOTE / "unused")
        pins = inventory["engines"]["mesh"]["verified_files"].copy()
        pins.pop(str(run.REMOTE / "unused/worker.py"))
        pins[str(ARM)] = ARM_SHA
        grants = "source/scripts/fixtures/jobs-role-grants.sql"
        with tarfile.open(original.STAGE) as archive:
            pins[str(Path(config["private_stage"]) / grants)] = original.dense.digest(archive.extractfile(grants).read())
        script = matrix.HOST_IDLE + (HERE / "install.sh").read_text()
        for name, files in (("ORIGINAL", pins), ("SIBLING", sibling_pins(config))):
            script = script.replace(f"@{name}_PINS@", "\n".join(f"{sha}  {key}" for key, sha in sorted(files.items())))
        payload = BytesIO()
        with tarfile.open(fileobj=payload, mode="w") as archive:
            for name in HOST_PAYLOAD:
                archive.add(HERE / name, arcname=name, recursive=False)
        target = out / "payload.tar"
        target.write_bytes(payload.getvalue())
        with target.open("rb") as source, (out / "preflight.log").open("xb") as log:
            call = run.ssh(["bash", "-c", script, "--", config["private_stage"], FROZEN,
                            config["broker"]["files"]["mesh"], original.stock.MANIFEST, run.digest(target)],
                           stdin=source, stdout=log, stderr=log, timeout=60)
        run.write_json(out / "status.json", dict(exit_status=call.returncode))
        call.check_returncode()
    check_preflight(out / "preflight.log", config, "retained-files")


def check_preflight(path, config, asset_checks):
    log = path.read_text()
    require(f'{config["private_stage"]}/packet/adapter.py", line 126' in log
            and "ValueError: substring not found" in log, "original traceback was not reproduced")
    rows = [json.loads(line) for line in log.splitlines() if line.startswith("{")]
    require([row["width"] for row in rows] == ["128", "1024"], "entry probe skipped a width")
    for row in rows:
        require(row == dict(adapter=str(sibling(config) / "adapter.py"),
                            adapter_sha256=run.digest(HERE / "adapter.py"),
                            harness=str(ARM), harness_sha256=ARM_SHA, uid=65534, gid=65534,
                            cpus=[0, 1], width=row["width"], asset_checks=asset_checks,
                            entered_without_workload=True), "entry probe source, behavior or UID changed")
    return rows


def check_stages(config):
    with tarfile.open(FROZEN_ARCHIVE) as archive:
        member = archive.extractfile(f"ordinary-v2/{ARM.relative_to(FROZEN)}").read()
        require(original.dense.digest(member) == ARM_SHA, "frozen arm differs from host inventory")
    for label in config["lanes"]:
        remote = run.REMOTE / "unused"
        files, old = ORIGINAL_STAGES(config, label, remote)
        new_files, new = stages(config, label, remote)
        require(files == new_files, "staged driver set changed")
        if label in original.dense.PEERS:
            require(old == new, "peer inventory changed")
            continue
        spec = old["engines"]["mesh"]
        spec["verified_files"].update(sibling_pins(config))
        spec["argv"] = [str(sibling(config) / "native-run") if arg == config["private_stage"] + "/packet/native-run"
                        else arg for arg in spec["argv"]]
        require(old == new, "inventory changes beyond the launcher and sibling pins")


def check_old(packet, broker):
    config = original.make_config(packet, broker, "pilot")
    manifest = run.read_json(FAILED / "series.json")
    require(config == manifest["config"], "old pilot configuration changed")
    matrix.replay_block = original.replay_block
    report = original.summarize(FAILED, config, manifest["schedule"], manifest["pins"])
    old = run.read_json(FAILED / "summary.json")
    serialized = json.loads(json.dumps(report))
    require(serialized == old and not report["qualifies"] and report["missing"] == [1, 2]
            and len(report["raw"]) == 12, "old rejected pilot did not replay")
    rejected = {}
    for block in manifest["schedule"]:
        folder, = (FAILED / "blocks").glob(f"{run.block_prefix(block)}.*")
        for phase, label, _, arm in matrix.paths(folder, config, block):
            remote = run.REMOTE / manifest["remotes"][label]
            _, inv = ORIGINAL_STAGES(config, label, remote)
            path = arm / "raw.tar.gz"
            with tarfile.open(path) as raw:
                status = run.file_json(raw, "status.json")
                host = run.file_json(raw, "host.json")
                log = raw.extractfile("adapter.log").read().decode()
                require(status["exit_code"] == 1 and f'{config["private_stage"]}/packet/adapter.py", line 126' in log
                        and log.endswith("ValueError: substring not found\n")
                        and host["verified_files"] == inv["engines"]["mesh"]["verified_files"],
                        "archived failure or host membership changed")
            try:
                original.score_arm(arm, phase, label, config, remote)
            except ValueError as exc:
                require(str(exc) == "arm failed or wrong invocation", "old arm rejected at an unexpected stage")
                rejected[str(arm.relative_to(FAILED))] = dict(raw_sha256=run.digest(path), reason=str(exc))
            else:
                raise ValueError("old failed arm received a score")
    return rejected


def use_retry(mode):
    original.stages = stages
    original._scoring_scope["stages"] = stages
    matrix.stages = stages
    matrix.replay_block = original.replay_block
    matrix.summarize = original.summarize
    ident = stage_request(mode)[0]["id"]
    matrix.grant = lambda _: grant(mode)
    matrix.SERIES[ident] = ident
    return ident


def controller_release(mode):
    ident = stage_request(mode)[0]["id"]
    path = matrix.QUEUE / f"{ident}-groups-1-2.released.json"
    result = run.read_json(path)
    require(result["id"] == ident and result["released_at"] and result["groups"] == [1, 2],
            "wrong recovery timing release")
    return path, result


def entry():
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("command", choices=("check", "dispatch", "replay", "slot"))
    cli.add_argument("mode", nargs="?", choices=("pilot", "peers"))
    cli.add_argument("request_sha256", nargs="?")
    args = cli.parse_args()
    _, packet, broker = bind()
    if args.command == "check":
        for mode in ("pilot", "peers"):
            check_stages(state(packet, broker, mode)[1])
        config = state(packet, broker, "pilot")[1]
        result = dict(rejected=check_old(packet, broker),
                      entry=check_preflight(EVIDENCE / "local-entry.log", config, "archived-digests"))
        from score_controls import run_controls
        use_retry("pilot")
        request = run.read_json(PACKET)
        result["score_controls"] = run_controls(original, config, stages, ROOT / request["setup_raw"])
        print(json.dumps(result, sort_keys=True, indent=2))
        return 0

    require(args.mode, "pilot or peers is required")
    ident = use_retry(args.mode)
    timing = state(packet, broker, args.mode)
    if args.command == "replay":
        result = original.summarize(*timing)
        print(json.dumps(result, sort_keys=True, indent=2))
        return bool(result["missing"])
    if args.mode == "peers":
        require(controller_release("pilot")[1]["exit_status"] == 0
                and original.summarize(*state(packet, broker, "pilot"))["qualifies"],
                "both retry pilots must pass and release before peers")
    if args.command == "dispatch":
        install(timing[1], args.mode)
        return matrix.dispatch(*timing[:2], ident, *timing[2:], [1, 2])

    # Entry point for the sole controller to open and release one fresh grant.
    request, source = stage_request(args.mode)
    require(args.request_sha256 == run.digest(source), "exact stage request SHA-256 required")
    receipts = timing[0] / "controller"
    receipts.mkdir(parents=True, exist_ok=True)
    with (EVIDENCE / ".retry-controller.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        require(current_event()[0] == "RELEASED", "an existing controller grant is still active")
        require((matrix.QUEUE / (ident + ".json")).read_bytes() == source.read_bytes(),
                "sole controller must register the sealed request first")
        release_path = matrix.QUEUE / f"{ident}-groups-1-2.released.json"
        if not release_path.exists():
            ops = runpy.run_path(str(matrix.QUEUE.parent / "completion-writer-r2.py"))
            ops["measure"](dict(request, timing_grant_id=ident, command_attempts=1,
                                timing_window_minutes=request["requested_window_minutes"],
                                controller_receipt_dir=receipts), "timing_command", [1, 2])
        release_path, release = controller_release(args.mode)
        path = timing[0] / "controller-release.json"
        if not path.exists():
            with path.open("xb") as stream:
                stream.write(release_path.read_bytes())
        require(path.read_bytes() == release_path.read_bytes(), "recovery release archive changed")
        if (timing[0] / "series.json").is_file():
            data = json.dumps(original.summarize(*timing), sort_keys=True, indent=2) + "\n"
            summary = timing[0] / "replay.json"
            if not summary.exists():
                with summary.open("x") as stream:
                    stream.write(data)
            require(summary.read_text() == data, "recovery replay archive changed")
        return release["exit_status"]


if __name__ == "__main__":
    sys.exit(entry())
