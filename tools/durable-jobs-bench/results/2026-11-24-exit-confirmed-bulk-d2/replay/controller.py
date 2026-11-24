#!/usr/bin/env python3
"""Offline replay and controller-owned setup/timing for the exit-confirmed D2 transfer."""
import argparse
from copy import deepcopy
from datetime import datetime, timezone
import difflib
import fcntl
import hashlib
import importlib
from io import BytesIO
import json
import math
from pathlib import Path
import re
import sys
import tarfile
from uuid import uuid4

from qualify import (HERE, INPUT, REPAIR, RESULTS, ROOT, check_arm,
                     check_earlier_faults, check_faults, check_runner_log, digest)

PACKET = HERE.parent
SUITE = RESULTS.parent
ORIGINAL = RESULTS / "2026-11-23-ordinary-completion-writer/combined"
PRIOR = RESULTS / "2026-11-23-bulk-enqueue-v2-short"
PEERS = RESULTS / "2026-11-24-bulk-d2-short-peers/short.json"
WORK = ROOT / "target/short-exit-bulk-d2"
REQUESTS = ROOT / "target/linode-jobs-stepchange/requests"
ID = PACKET.name
NATIVE = "/var/lib/durable-jobs-bench/native"
RETAINED = f"{NATIVE}/builds/bulk-writer-b1dcd34af288"
RETAINED_ELF = "93fef96704ae412cf3c55fae73cc2aac405c8b423670826b4876954a44c8b010"
RETAINED_MANIFEST = "361f10d2c2866f694989198bb8359d29c91dd9928dcd95e8f92293293f226d29"
BROKER_ELF = "55d7000d562a6ad395f5e64e33e0e973fac0084ed05ad6c30a33db2a3fc6303a"
STOCK_COMPILER = "e023d5944764878969ca40b7edb59c779498a435b44d308776076f4c7f2e8c19"
BASE_VERSION = "3618be08a9dda3122f7161148fc5ad810cd2ebc4"
COLD_PEERS = ("protected-mesh", "river", "oban", "apalis")
A = ("r2-off", "river", "r2-d2", "protected-mesh", "apalis", "oban")
B = ("r2-d2", "apalis", "protected-mesh", "oban", "r2-off", "river")
C = ("r2-off", "oban", "apalis", "r2-d2", "river", "protected-mesh")
SHIPPED = ("combined-run", "split_arm.py", "identity.py", "faults.py", "wire.py", "link-candidate.sh")
METRICS = ("submit_jobs_per_s", "commit_jobs_per_s")

sys.path.insert(0, str(SUITE))
sys.path.insert(0, str(PRIOR / "replay"))
matrix = importlib.import_module("interleave-v2")
run = matrix.run
base_replay = matrix.replay
matrix.RESULTS = PACKET / "evidence/native-timing"


def plan(profiles, lanes):
    assert profiles == ["short"] and set(lanes) == set(A)
    for group, order in enumerate((A, A[::-1], B, B[::-1], C), 1):
        yield dict(group=group, profile="short", phases=["measured"], order=list(order))


def raw_bytes(archive, name):
    member = archive.getmember("./" + name)
    assert member.isfile() and sum(item.name == member.name for item in archive) == 1
    with archive.extractfile(member) as stream:
        return stream.read()


def raw_json(archive, name):
    return json.loads(raw_bytes(archive, name))


def template():
    base = run.read_json(PRIOR / "inventory.json")
    peers = run.read_json(PEERS)
    assert all(base[k] == peers[k] for k in ("cpus", "host", "clock_ticks", "contract", "profiles"))
    assert base["cpus"] == [0, 1] and base["profiles"] == ["short"]
    for label in ("river", "oban", "apalis"):
        assert base["lanes"][label] == peers["lanes"][label]
    return base, {label: peers["lanes"][label] for label in COLD_PEERS}


def native_pins():
    base, peers = template()
    files = {}
    for lane in [base["lanes"]["bulk"], *peers.values()]:
        for path, expected in lane["inventory"]["engines"][lane["engine"]]["verified_files"].items():
            assert path not in files or files[path] == expected
            files[path] = expected
    return dict(cpus=base["cpus"], host=base["host"], clock_ticks=base["clock_ticks"], files=files)


def package_stage():
    parts = {"pinned-native.json": (json.dumps(native_pins(), sort_keys=True) + "\n").encode(),
             "combined-source.tar.gz": (PACKET / "combined-source.tar.gz").read_bytes(),
             "d2-transfer.patch": (PACKET / "d2-transfer.patch").read_bytes()}
    parts.update({f"replay/{name}": (HERE / name).read_bytes() for name in SHIPPED})
    data = BytesIO()
    with tarfile.open(fileobj=data, mode="w", format=tarfile.USTAR_FORMAT) as archive:
        for name, content in sorted(parts.items()):
            entry = tarfile.TarInfo(name)
            entry.mode = 0o755 if name in {"replay/combined-run", "replay/link-candidate.sh"} else 0o644
            entry.size = len(content)
            archive.addfile(entry, BytesIO(content))
    return data.getvalue()


def verify_delta():
    with (tarfile.open(ORIGINAL / "combined-source.tar.gz") as original,
          tarfile.open(PACKET / "combined-source.tar.gz") as candidate):
        assert original.getnames() == candidate.getnames()
        delta = {}
        for base, edited in zip(original.getmembers(), candidate.getmembers(), strict=True):
            assert (base.name, base.type, base.mode, base.linkname) == (edited.name, edited.type, edited.mode, edited.linkname)
            if base.isfile():
                with original.extractfile(base) as old, candidate.extractfile(edited) as new:
                    before, after = old.read(), new.read()
                    if before != after:
                        path = base.name.removeprefix("source/")
                        delta[base.name] = "".join(difflib.unified_diff(
                            before.decode().splitlines(keepends=True), after.decode().splitlines(keepends=True),
                            fromfile="a/" + path, tofile="b/" + path))
        assert set(delta) == {f"source/compiler/mesh-rt/src/db/{file}.rs" for file in ("pg", "pool")}
    assert "".join(delta[key] for key in sorted(delta)) == (PACKET / "d2-transfer.patch").read_text()
    return sorted(delta)


def earlier_proof():
    receipt = run.read_json(REPAIR / "evidence/native-qualification.json")
    attempt = receipt["attempt"]
    proof = REPAIR / "evidence" / attempt / "raw.tar.gz"
    assert digest(proof) == receipt["raw_sha256"]
    base, _ = template()
    runner = dict(elf=RETAINED_ELF, manifest=RETAINED_MANIFEST, version=BASE_VERSION)
    with tarfile.open(proof) as archive:
        host = raw_json(archive, "host.json")
        manifest = raw_bytes(archive, "runner-build.json")
        assert host == {key: base[key] for key in ("host", "clock_ticks")}
        assert hashlib.sha256(manifest).hexdigest() == RETAINED_MANIFEST
        build = json.loads(manifest)
        assert build["files"]["mesh"] == RETAINED_ELF and build["compiler_sha256"] == STOCK_COMPILER
        score = check_arm(lambda name: raw_json(archive, f"smoke-off/{name}"),
                          "off", runner, native=True, historical=True)
    archived = run.read_json(REPAIR / "evidence/archived-setup-replay.json")
    assert archived["runner_sha256"] == RETAINED_ELF and archived["runner_manifest_sha256"] == RETAINED_MANIFEST
    assert set(archived["teardown_witnesses"]) == {"off", "zero"} and len(archived["fault_cases"]) == 7
    assert archived["teardown_witnesses"]["off"]["held_callback_state"]["state"] == "executing"
    assert archived["teardown_witnesses"]["off"]["after_callback_state"]["state"] == "completed"
    first = ORIGINAL / "evidence" / archived["attempt"] / "raw.tar.gz"
    assert digest(first) == archived["raw_sha256"]
    with tarfile.open(first) as archive:
        assert raw_json(archive, "proof/teardown-off/teardown.json") == archived["teardown_witnesses"]["off"]
        check_earlier_faults({case: raw_json(archive, f"proof/faults/{case}/result.json")
                              for case in archived["fault_cases"]})
    return dict(host=host, smoke=score, archive=receipt["raw_sha256"], first_archive=archived["raw_sha256"])


def pins():
    packet = run.read_json(PACKET / "controller-packet.json")
    assert packet["id"] == ID and packet["input_sha256"] == INPUT
    for path, expected in packet["pins"].items():
        assert digest(ROOT / path) == expected, path
    assert digest(PACKET / "combined-source.tar.gz") == packet["source_archive_sha256"]
    assert digest(PACKET / "d2-transfer.patch") == packet["d2_patch_sha256"]
    assert hashlib.sha256(package_stage()).hexdigest() == packet["stage_sha256"]
    assert packet["native_cold_pins"] == native_pins()
    assert packet["group_plan"] == list(plan(["short"], A))
    return packet


def grant(kind):
    events = re.findall(r"^(SETUP|TIMING|RELEASED) ([\w-]+) (\S+)(?: (\S+))?",
                        (REQUESTS / "STATUS.md").read_text(), re.MULTILINE)
    mode, identifier, start, end = events[-1]
    now = datetime.now(timezone.utc)
    run.require((mode, identifier) == (kind, ID) and end and
                datetime.fromisoformat(start) <= now < datetime.fromisoformat(end),
                f"central controller {kind} grant required for {ID}")
    return start, end


def inventory(attempt, elf, candidate_manifest, packet):
    assert re.fullmatch(r"exit-bulk-d2-[a-f0-9]{12}", attempt)
    assert all(re.fullmatch(r"[a-f0-9]{64}", seal) for seal in (elf, candidate_manifest))
    assert elf != RETAINED_ELF and candidate_manifest != RETAINED_MANIFEST
    base, peers = template()
    bulk = base.pop("lanes")["bulk"]
    prefix = f"{NATIVE}/source-{attempt}/replay"
    candidate = f"{NATIVE}/builds/{attempt}"
    wrapper = f"{prefix}/combined-run"
    drivers = {f"{prefix}/{name}": digest(HERE / name) for name in SHIPPED[:3]}
    base.update(description="Exact bulk V2 broker and migrator with r2 writer off, with or without D2, plus cold stock peers.",
                lanes={})
    expected = dict(wrapper=wrapper, baseline_directory=RETAINED, candidate_directory=candidate, files=dict(drivers))
    for mode, directory, exe, seal, version in (
        ("off", RETAINED, RETAINED_ELF, RETAINED_MANIFEST, BASE_VERSION),
        ("d2", candidate, elf, candidate_manifest, packet["source_archive_sha256"][:40]),
    ):
        lane = deepcopy(bulk)
        spec = lane["inventory"]["engines"]["mesh"]
        args = ["/usr/bin/env", f"EXIT_BULK_CHECKOUT={mode}", f"COMBINED_RUNNER_ARTIFACTS={directory}"]
        if mode == "d2":
            args.extend((f"EXIT_BULK_RUNNER_ELF={exe}", f"EXIT_BULK_RUNNER_MANIFEST={seal}"))
        spec["argv"] = [*args, wrapper, "mesh"]
        artifacts = {f"{directory}/mesh": exe, f"{directory}/mesh.json": seal}
        expected["files"].update(artifacts)
        spec["verified_files"].update(drivers | artifacts)
        lane["runner_seal"] = dict(elf=exe, manifest=seal, version=version)
        base["lanes"][f"r2-{mode}"] = lane
    base["lanes"].update(peers)
    matrix.validate(base)
    return base, expected


def reduce_setup(packet, attempt, path):
    known = earlier_proof()
    with tarfile.open(path) as archive:
        assert raw_json(archive, "host.json") == known["host"]
        baseline_bytes = raw_bytes(archive, "baseline-build.json")
        assert hashlib.sha256(baseline_bytes).hexdigest() == RETAINED_MANIFEST
        baseline = json.loads(baseline_bytes)
        build_bytes = raw_bytes(archive, "runner-build.json")
        build = json.loads(build_bytes)
        assert (build["system"], build["architecture"]) == ("Linux", "x86_64")
        assert build["capsule"] == "bulk-v2-completion-writer-exit-r2"
        assert build["checkout_env"] == "MESH_PG_HOT_CHECKOUT=1" and build["checkout_max_idle_ms"] == 500
        assert build["compiler_sha256"] == baseline["compiler_sha256"] == STOCK_COMPILER
        fixture = ("main.mpl", "writes.mpl", "worker_identity.mpl", "worker_identity.c")
        assert all(build["files"][name] == baseline["files"][name] for name in fixture)
        assert build["source_patch_sha256"][:40] == BASE_VERSION
        assert build["combined_source_sha256"] == packet["source_archive_sha256"]
        assert build["d2_patch_sha256"] == packet["d2_patch_sha256"]
        assert build["version"] == packet["source_archive_sha256"][:40]
        elf = build["files"]["mesh"]
        config, expected = inventory(attempt, elf, hashlib.sha256(build_bytes).hexdigest(), packet)
        assert raw_json(archive, "native-seal.json") == expected
        faults = check_faults(lambda name: raw_json(archive, f"proof/d2/{name}"), elf, BROKER_ELF)
        scores = {}
        for mode in ("off", "d2"):
            def read(name):
                return raw_json(archive, f"smoke-{mode}/{name}")
            assert read("invocation.json")["phase"] == "diagnostic"
            check_runner_log(raw_bytes(archive, f"smoke-{mode}/runner-0.log").decode())
            scores[mode] = check_arm(read, mode, config["lanes"][f"r2-{mode}"]["runner_seal"], native=True)
    return config, scores, faults


def setup(packet):
    assert packet["ready"]
    start, end = grant("SETUP")
    attempt = "exit-bulk-d2-" + uuid4().hex[:12]
    out = PACKET / "evidence" / attempt
    out.mkdir(parents=True)
    run.write_json(out / "grant.json", dict(id=ID, attempt=attempt, start=start, end=end))
    WORK.mkdir(parents=True, exist_ok=True)
    stage = WORK / f"{attempt}.tar"
    stage.write_bytes(package_stage())
    try:
        with (SUITE / ".timing.lock").open("rb") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            grant("SETUP")
            remaining = (datetime.fromisoformat(end) - datetime.now(timezone.utc)).total_seconds()
            run.require(remaining >= 240, "retain at least four minutes for build, proof and export")
            exit_code = None
            try:
                with (out / "setup.log").open("xb") as log, stage.open("rb") as archive:
                    result = run.ssh(["bash", "-c", (HERE / "native-setup.sh").read_text(), "--",
                                      packet["stage_sha256"], attempt, packet["source_archive_sha256"],
                                      packet["d2_patch_sha256"]], stdin=archive, stdout=log, stderr=log,
                                     timeout=remaining - 75)
                    exit_code = result.returncode
            finally:
                run.write_json(out / "status.json", dict(exit_code=exit_code, utc=run.utc_now()))
                with (out / "raw.tar.gz").open("xb") as data, (out / "export.log").open("xb") as err:
                    exported = run.ssh(["tar", "-czf", "-", "-C", f"{NATIVE}/runs/{attempt}", "."],
                                       stdout=data, stderr=err, timeout=70)
                    if exit_code == 0:
                        exported.check_returncode()
            result.check_returncode()
    finally:
        stage.unlink(missing_ok=True)
    config, scores, faults = reduce_setup(packet, attempt, out / "raw.tar.gz")
    target = out / "qualified-timing.json"
    run.write_json(target, config)
    run.write_json(PACKET / "evidence/native-qualification.json", dict(
        attempt=attempt, inventory_sha256=digest(target), raw_sha256=digest(out / "raw.tar.gz"),
        scores=scores, faults=faults))
    print(target, flush=True)


def qualified(packet):
    receipt = run.read_json(PACKET / "evidence/native-qualification.json")
    assert re.fullmatch(r"exit-bulk-d2-[a-f0-9]{12}", receipt["attempt"])
    out = PACKET / "evidence" / receipt["attempt"]
    archive, spec = out / "raw.tar.gz", out / "qualified-timing.json"
    assert run.read_json(out / "status.json")["exit_code"] == 0
    assert digest(archive) == receipt["raw_sha256"] and digest(spec) == receipt["inventory_sha256"]
    config, scores, faults = reduce_setup(packet, receipt["attempt"], archive)
    assert config == run.read_json(spec)
    assert json.loads(json.dumps(scores)) == receipt["scores"]
    assert faults == receipt["faults"]
    return config


def replay(path, config, block):
    scored = base_replay(path, config, block)
    for phase, label, folder in matrix.arms(path, config, block):
        if label in {"r2-off", "r2-d2"}:
            spec = config["lanes"][label]
            with tarfile.open(folder / "raw.tar.gz") as archive:
                try:
                    with archive.extractfile("adapter/runner-0.log") as log:
                        check_runner_log(log.read().decode())
                    checked = check_arm(lambda name: run.file_json(archive, "adapter/" + name),
                                        label.removeprefix("r2-"), spec["runner_seal"], native=True)
                    for metric in METRICS:
                        assert math.isclose(checked[metric], scored["scores"][label][metric], rel_tol=1e-9)
                except AssertionError as error:
                    raise ValueError(f"{label} violated its exit-confirmed D2 proof") from error
            assert phase == "measured"
            scored["scores"][label]["checkout_probes"] = checked["checkout_probes"]
    arms = scored["scores"]
    scored["paired_d2_vs_off_percent"] = {metric: 100 * (arms["r2-d2"][metric] / arms["r2-off"][metric] - 1)
                                           for metric in METRICS}
    return scored


matrix.plan = plan
matrix.replay = replay


def timing(packet, groups):
    grant("TIMING")
    release = run.read_json(REQUESTS / f"{ID}.released.json")
    assert release["id"] == ID and release["exit_status"] == 0, "require the successful central SETUP release"
    config = qualified(packet)
    series = matrix.RESULTS / ID
    if list(groups) == [3, 4, 5]:
        initial = list(plan(config["profiles"], config["lanes"]))[:2]
        results = [matrix.select_attempt(series, config, block, {}) for block in initial]
        assert all(results), "require both opposite-order timing receipts before extending"
    matrix.RESULTS.mkdir(parents=True, exist_ok=True)
    return matrix.dispatch(series, config, groups)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("check", "stage-sha", "setup", "verify-setup", "timing", "summarize"))
    parser.add_argument("groups", nargs="*", type=int)
    options = parser.parse_args()
    if options.action == "stage-sha":
        print(hashlib.sha256(package_stage()).hexdigest())
        return
    packet = pins()
    if options.action == "check":
        earlier = earlier_proof()
        local = importlib.import_module("local-replay").verify_local()
        shape, _ = inventory("exit-bulk-d2-" + "0" * 12, "1" * 64, "2" * 64, packet)
        print(json.dumps(dict(id=ID, changed_source=verify_delta(), archived_native_r2=earlier,
                              local=local, cold_stock_peers=COLD_PEERS,
                              pinned_native_files=len(native_pins()["files"]),
                              plan=list(plan(shape["profiles"], shape["lanes"]))), indent=2))
    elif options.action == "setup":
        setup(packet)
    elif options.action == "verify-setup":
        qualified(packet)
        print("Verified native setup and cold-peer timing inventory.")
    elif options.action == "summarize":
        return matrix.summarize(matrix.RESULTS / ID)
    elif options.action == "timing":
        if options.groups not in ([1, 2], [3, 4, 5]):
            parser.error("select the first two or final three timing groups")
        return timing(packet, options.groups)


if __name__ == "__main__":
    raise SystemExit(main())
