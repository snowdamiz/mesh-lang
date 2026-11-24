"""Controller-only setup and native timing of the short R2 D2 producer."""
import argparse
from copy import deepcopy
from datetime import datetime, timezone
import fcntl
import gzip
from hashlib import sha256
import io
import importlib.util
import json
from pathlib import Path
import re
import subprocess
import sys
import tarfile
from uuid import uuid4

from adapter import launch_source, split_source
from audit import BROKER, INPUT, SERIES, audit, digest
from score import attribution, read_arm, summary

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
SUITE = HERE.parents[1]
QUEUE = ROOT / "target/linode-jobs-stepchange/requests"
ID = "r2-d2-producer-boundary"
REQUEST = HERE / "request.json"
EVIDENCE = HERE / "evidence"
R2 = HERE.parent / "2026-11-24-exit-confirmed-bulk-d2"
R2_REPLAY = R2 / "replay"
SPEC = HERE.parent / "2026-11-24-enqueue-spec-preparation"
SOURCE = SPEC / "native-amd64-source.tar.gz"
BASE = "/var/lib/durable-jobs-bench/native"
ARTIFACT = BASE + "/builds/bulk-enqueue-3b09dc3d0e23-ordinary-v2"
STOCK_COMPILER = BASE + "/builds/mesh-stock-3828f20d/abi/meshc"
STOCK_COMPILER_SHA = "e023d5944764878969ca40b7edb59c779498a435b44d308776076f4c7f2e8c19"
STOCK_MANIFEST = "7f228c192630bb6d95cbf9e06c5e0132931f429c1cfa07391bb28cb6fdec8911"
LANES = ("off-a", "client", "off-b", "nested")
ORDERS = [
    ("off-a", "client", "nested", "off-b"),
    ("off-b", "nested", "client", "off-a"),
    ("client", "off-a", "off-b", "nested"),
    ("nested", "off-b", "off-a", "client"),
    ("off-b", "client", "off-a", "nested"),
]

sys.path.insert(0, str(R2_REPLAY))
spec = importlib.util.spec_from_file_location("retained_r2_driver", R2_REPLAY / "controller.py")
original = importlib.util.module_from_spec(spec)
spec.loader.exec_module(original)
from qualify import check_arm, check_runner_log

run, matrix = original.run, original.matrix
base_replay = original.base_replay


def required_configuration():
    receipt = run.read_json(R2 / "recovery/evidence/qualification.json")
    timer = R2 / "recovery/evidence" / receipt["attempt"] / "qualified-timing.json"
    assert digest(timer) == receipt["inventory_sha256"]
    config = run.read_json(timer)
    series = run.read_json(SERIES / "series.json")["config"]
    assert config == series
    return config


def plan(profiles, lanes):
    assert profiles == ["short"] and set(lanes) == set(LANES)
    for i, order in enumerate(ORDERS, 1):
        yield dict(group=i, profile="short", phases=["measured"], order=list(order))


def shipped():
    return {
        **{name: (HERE / name).read_bytes() for name in ("probe.rs", "materialize.py")},
        "replay/combined-run": launch_source((R2_REPLAY / "combined-run").read_text()).encode(),
        "replay/split_arm.py": split_source((R2_REPLAY / "split_arm.py").read_text()).encode(),
        "replay/identity.py": (R2_REPLAY / "identity.py").read_bytes(),
        **{f"replay/{name}": (HERE / name).read_bytes() for name in ("adapter.py", "observe.py", "materialize.py")},
        "runtime-pins.json": (EVIDENCE / "offline/runtime-pins.json").read_bytes(),
    }


def native_pins(config):
    stock = config["lanes"]["r2-d2"]["inventory"]["engines"]["mesh"]["verified_files"]
    pins = {**stock, STOCK_COMPILER: STOCK_COMPILER_SHA}
    for lane in ("oban", "river", "apalis"):
        peer = config["lanes"][lane]
        for file, sha in peer["inventory"]["engines"][peer["engine"]]["verified_files"].items():
            assert file not in pins or pins[file] == sha
            pins[file] = sha
    return pins


def payload(pins):
    files = shipped()
    files["native-pins.txt"] = "".join(f"{sha}  {file}\n" for file, sha in sorted(pins.items())).encode()
    package = io.BytesIO()
    with gzip.GzipFile(fileobj=package, filename="", mode="wb", mtime=0) as compressed:
        with tarfile.open(fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT) as out:
            for name, contents in sorted(files.items()):
                info = tarfile.TarInfo(name)
                info.size = len(contents)
                info.mode = 0o755 if name == "replay/combined-run" else 0o644
                out.addfile(info, io.BytesIO(contents))
            with tarfile.open(SOURCE) as tree:
                for item in tree:
                    if item.name != "tree-short" and not item.name.startswith("tree-short/"):
                        continue
                    assert ".." not in Path(item.name).parts and (item.isfile() or item.isdir())
                    item.uid = item.gid = item.mtime = 0
                    item.uname = item.gname = ""
                    item.pax_headers = {}
                    out.addfile(item, tree.extractfile(item) if item.isfile() else None)
    return package.getvalue()


def inputs():
    packet = run.read_json(REQUEST)
    assert packet["id"] == ID and packet["input_sha256"] == INPUT
    for name, sha in packet["pins"].items():
        assert digest(ROOT / name) == sha, name
    config = required_configuration()
    assert config["host"] == ["90ec1c24-08c8-4bc1-aa28-79b6db1bd13a", "6.8.0-149-generic", "8:0"]
    assert config["cpus"] == [0, 1] and config["clock_ticks"] == 100
    assert packet["groups"] == list(plan(["short"], LANES))
    assert packet["native_pins"] == native_pins(config)
    return packet, config


def grant(kind):
    selected = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD) ([\w-]+) (\S+)(?: (\S+))?",
                          (QUEUE / "STATUS.md").read_text(), re.MULTILINE)
    mode, name, start, end = selected[-1]
    assert (mode, name) == (kind, ID) and end, f"require controller {kind} for {ID}"
    assert (QUEUE / f"{ID}.json").read_bytes() == REQUEST.read_bytes()
    assert datetime.fromisoformat(start) <= datetime.now(timezone.utc) < datetime.fromisoformat(end)
    return end


def verify_setup(folder, pinned):
    assert run.read_json(folder / "status.json")["exit_code"] == 0
    with tarfile.open(folder / "raw.tar.gz") as raw:
        def contents(name):
            with raw.extractfile("./" + name) as stream:
                return stream.read()
        assert contents("SETUP-PASS") == b""
        assert contents("runtime-pins.json") == (EVIDENCE / "offline/runtime-pins.json").read_bytes()
        image = json.loads(contents("image.json"))
        manifest = contents("broker-manifest.json")
        assert sha256(manifest).hexdigest() == image["manifest"]
        build, stock = json.loads(manifest), json.loads(contents("smoke/build.json"))
        assert image["elf"] == build["files"]["mesh"] != BROKER
        assert build["files"]["main.mpl"] == stock["files"]["main.mpl"]
        assert build["runtime_sha256"] == image["runtime"] != stock["runtime_sha256"]
        assert all(build["files"][name] == digest for name, digest in stock["files"].items() if name != "mesh")
        assert build["benchmark_observer"] == ID
        result = read_arm(lambda name: json.loads(contents("smoke/" + name)),
                          lambda name: contents("smoke/" + name).decode(),
                          "nested", pinned["lanes"]["r2-d2"]["runner_seal"], image,
                          check_arm, check_runner_log)
        split = json.loads(contents("smoke/split-executables.json"))
        assert split["producer"] == f"{BASE}/source-{folder.name}/broker/mesh"
    return image, result


def setup(packet, config):
    stage = payload(packet["native_pins"])
    assert sha256(stage).hexdigest() == packet["setup_payload_sha256"]
    attempt = ID + "-" + uuid4().hex[:12]
    folder = EVIDENCE / "setup" / attempt
    with (SUITE / ".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        end = grant("SETUP")
        remaining = (datetime.fromisoformat(end) - datetime.now(timezone.utc)).total_seconds()
        assert remaining >= 180
        folder.mkdir(parents=True)
        code = None
        try:
            with (folder / "setup.log").open("xb") as log:
                result = run.ssh(["bash", "-c", (HERE / "native-setup.sh").read_text(), "--",
                                  attempt, packet["setup_payload_sha256"]],
                                 input=stage, stdout=log, stderr=log, timeout=remaining - 70)
                code = result.returncode
        finally:
            run.write_json(folder / "status.json", dict(exit_code=code, observed=run.utc_now()))
            with ((folder / "raw.tar.gz").open("xb") as raw,
                  (folder / "export.log").open("xb") as log):
                exported = run.ssh(["bash", "-c", matrix.HOST_IDLE + '\nexec tar -czf - -C "$1" .',
                                    "--", f"{BASE}/runs/{attempt}"], stdout=raw, stderr=log, timeout=60)
                if code == 0:
                    exported.check_returncode()
        result.check_returncode()
    image, proof = verify_setup(folder, config)
    run.write_json(folder / "qualification.json", dict(image=image, proof=proof,
                                                       raw_sha256=digest(folder / "raw.tar.gz")))
    print(folder, flush=True)


def configure(known, attempt, image):
    assert re.fullmatch(ID + r"-[a-f0-9]{12}", attempt)
    path = f"{BASE}/source-{attempt}"
    conf = dict(known, description="Short R2 D2 producer, two exact stock controls and two diagnostic observers.",
                lanes={name: deepcopy(known["lanes"]["r2-d2"]) for name in LANES})
    for label in ("client", "nested"):
        lane = conf["lanes"][label]
        spec = lane["inventory"]["engines"]["mesh"]
        driver = f"{path}/replay/combined-run"
        broker = f"{path}/broker/mesh"
        spec["argv"][-2:-1] = [
            f"R2_OBSERVATION={label}", f"R2_OBSERVED_BROKER={broker}",
            f"R2_OBSERVED_BROKER_SHA={image['elf']}", f"R2_OBSERVED_MANIFEST_SHA={image['manifest']}", driver,
        ]
        for file, content in shipped().items():
            if file.startswith("replay/"):
                spec["verified_files"][f"{path}/{file}"] = sha256(content).hexdigest()
        spec["verified_files"].update({broker: image["elf"], f"{path}/broker/mesh.json": image["manifest"]})
        lane["observer_image"] = image
    matrix.validate(conf)
    return conf


def recheck_block(path, config, block):
    scored = base_replay(path, config, block)
    observed = {}
    for phase, label, folder in matrix.arms(path, config, block):
        assert phase == "measured"
        lane = config["lanes"][label]
        with tarfile.open(folder / "raw.tar.gz") as raw:
            try:
                observed[label] = read_arm(
                    lambda name: run.file_json(raw, "adapter/" + name),
                    lambda name: raw.extractfile("adapter/" + name).read().decode(),
                    label if label in ("client", "nested") else "off", lane["runner_seal"],
                    lane.get("observer_image", {"elf": BROKER, "manifest": STOCK_MANIFEST}),
                    check_arm, check_runner_log)
            except (AssertionError, KeyError, ValueError) as error:
                raise ValueError(f"{label} producer receipt failed: {error}") from error
    return dict(scored, observations=observed)


def replay(directory):
    manifest = run.read_json(directory / "series.json")
    issues, groups = {}, []
    for block in manifest["plan"]:
        selected = matrix.select_attempt(directory, manifest["config"], block, issues)
        if selected:
            attempt, result = selected
            groups.append(dict(group=block["group"], block=attempt, **result))
    complete = len(groups) == 5
    result = dict(groups=groups, rejected=issues, complete=complete, perturbation=None, attribution=None)
    if complete:
        peers = run.read_json(EVIDENCE / "offline/audited-peer-scores.json")
        perturbation = summary([g["observations"] for g in groups])
        result.update(perturbation=perturbation, attribution=attribution(groups, peers, perturbation))
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("check", "setup", "verify-setup", "timing", "replay"))
    parser.add_argument("attempt", nargs="?")
    parser.add_argument("groups", type=int, nargs="*")
    args = parser.parse_args()
    packet, known = inputs()
    matrix.plan, matrix.replay = plan, recheck_block
    if args.action == "check":
        assert audit() == run.read_json(EVIDENCE / "offline/audited-peer-scores.json")
        assert sha256(payload(packet["native_pins"])).hexdigest() == packet["setup_payload_sha256"]
        dummy = {"elf": "a" * 64, "manifest": "b" * 64, "runtime": "c" * 64}
        checked = configure(known, ID + "-" + "0" * 12, dummy)
        assert checked["lanes"]["off-a"] == checked["lanes"]["off-b"] == known["lanes"]["r2-d2"]
        print(json.dumps(dict(status="PASS", original_groups=5, groups=packet["groups"], ready=True), indent=2))
        return
    if args.action == "setup":
        setup(packet, known)
        return
    assert args.attempt and re.fullmatch(ID + r"-[a-f0-9]{12}", args.attempt)
    folder = EVIDENCE / "setup" / args.attempt
    if args.action == "replay":
        print(json.dumps(replay(EVIDENCE / "native-timing" / args.attempt), indent=2, sort_keys=True))
        return
    image, proof = verify_setup(folder, known)
    retained = run.read_json(folder / "qualification.json")
    assert retained == dict(image=image, proof=proof, raw_sha256=digest(folder / "raw.tar.gz"))
    if args.action == "verify-setup":
        print(json.dumps(retained, indent=2, sort_keys=True))
        return
    grant("TIMING")
    release = run.read_json(QUEUE / f"{ID}.released.json")
    assert release["id"] == ID and release["exit_status"] == 0
    series = EVIDENCE / "native-timing" / args.attempt
    series.parent.mkdir(parents=True, exist_ok=True)
    assert args.groups and set(args.groups) <= set(range(1, 6))
    if args.groups != [1, 2]:
        assert args.groups == [3, 4, 5]
        assert {g["group"] for g in replay(series)["groups"]} >= {1, 2}
    return matrix.dispatch(series, configure(known, args.attempt, image), args.groups)


if __name__ == "__main__":
    sys.exit(main())
