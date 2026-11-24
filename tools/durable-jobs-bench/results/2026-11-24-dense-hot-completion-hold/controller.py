"""Price one millisecond of confirmed-completion batching on the frozen hot R2 runner."""
import argparse
from copy import deepcopy
from datetime import datetime, timezone
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile

HERE = Path(__file__).resolve().parent
PRIOR = HERE.parent / "2026-11-24-dense-two-runner-checkout"
sys.path.insert(0, str(PRIOR))
from importlib import import_module

dense = import_module("native-controller")
run, matrix, require = dense.run, dense.matrix, dense.require
ID = HERE.name
PACKET = HERE / "request.json"
OUT = HERE / "evidence/native"
MODES = {"hot-a": "d2-on", "hot-b": "d2-on", "hot-zero": "d2-zero", "hot-hold1": "d2-hold1"}
FILES = ("native-arm.py", "native-score.py", "native-dense-import-retry.sh")
REFERENCE = PRIOR / "evidence/native-import-retry/series.json"
ORDER = ["hot-a", "river-2x32", "hot-zero", "apalis-2x32", "hot-hold1", "oban-2x32", "hot-b"]
EXECUTABLE = "972514efac688fd029821152c3bb70a7e62bf1c6227ab8b2d061dc69c47a2dc7"


def grant(_=None):
    entries = (line.split() for line in (dense.QUEUE / "STATUS.md").read_text().splitlines())
    events = [row for row in entries if row and row[0] in {"HOLD", "TIMING", "SETUP", "RELEASED"}]
    require(events and len(events[-1]) == 4 and events[-1][:2] == ["TIMING", ID], "dedicated TIMING required")
    start, end = events[-1][2:]
    require(datetime.fromisoformat(start) <= datetime.now(timezone.utc) < datetime.fromisoformat(end),
            "outside the controller grant")
    require(PACKET.read_bytes() == (dense.QUEUE / f"{ID}.json").read_bytes(), "intake changed")
    return dict(start=start, end=end, request_sha256=run.digest(PACKET))


def configure():
    config = run.read_json(REFERENCE)["config"]
    require(config["cpus"] == [0, 1] and config["mesh_runner"]["d2-on"]["elf"] == EXECUTABLE,
            "wrong source inventory")
    config["lanes"] = {name: config["lanes"][name if name in dense.PEERS else "d2-on"] for name in ORDER}
    for key in ("mesh_runner", "mesh_broker", "mesh_spec"):
        config[key] = {label: deepcopy(config[key]["d2-on"]) for label in MODES}
    config["description"] = "Dense hot R2 with 1 ms, zero hold, duplicate writer-off controls and equal peers"
    return config


def decision(arms):
    off_a, off_b, zero, one = (arms["measured-" + key] for key in ("hot-a", "hot-b", "hot-zero", "hot-hold1"))
    controls = (off_a, off_b, zero)
    margins = {}
    for key in ("submit_ms", "drain_ms", "middle_write_ms"):
        best = min(row[key] for row in controls)
        threshold = max(best * 0.05, abs(off_a[key]-off_b[key]))
        margins[key] = {"saving_ms": best-one[key], "threshold_ms": threshold}
    return dict(margins=margins, passes=(
        all(row["saving_ms"] > row["threshold_ms"] for row in margins.values())
        and one["terminal_xids"] < min(row["terminal_xids"] for row in controls)
        and one["final_write_ms"] < min(row["final_write_ms"] for row in controls)))


def execute(action, groups, stage):
    packet = run.read_json(PACKET)
    for relative, digest in packet["inputs"].items():
        require(run.digest(dense.ROOT / relative) == digest, f"changed input {relative}")
    for name in FILES:
        shutil.copyfile(PRIOR / name, stage / name)
    subprocess.run(["patch", "-s", "-p1", "-i", str(HERE / "adapter.patch")], check=True, cwd=stage)
    for name in FILES:
        require(run.digest(stage / name) == packet["derived"][name], f"adapter differs: {name}")
    spec = importlib.util.spec_from_file_location("hold_score", stage / "native-score.py")
    score = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(score)
    config = configure()
    plan = packet["schedule"]
    pins = dict(request=run.digest(PACKET), inputs=packet["inputs"])
    dense.native_score.MODES.update(MODES)
    original_stages = dense.stages

    def stages(config, label, remote):
        files, inventory = original_stages(config, label, remote)
        if label in dense.PEERS:
            return files, inventory
        entry = inventory["engines"]["mesh"]
        launcher = stage / "native-dense-import-retry.sh"
        replacements = {"native-arm.py": stage / "native-arm.py", "native-dense.sh": launcher}
        for old, new in replacements.items():
            del entry["verified_files"][str(remote / old)]
            entry["verified_files"][str(remote / new.name)] = run.digest(new)
        entry["argv"][1] = str(remote / launcher.name)
        run.check_inventory(inventory)
        return [replacements.get(path.name, path) for path in files], inventory

    def score_arm(path, phase, label, remote):
        _, inv = stages(config, label, remote)
        if label in MODES:
            row = score.raw_score(path, phase, label, config, inv)
        else:
            row = dense.base_score(path, config["lanes"][label]["engine"], 2, phase, inv, config)
        with tarfile.open(path / "raw.tar.gz") as raw:
            jobs = run.file_json(raw, "adapter/ledger.json")["jobs"]
        writes = sorted(job["effect_committed_us"] for job in jobs)
        require(len(writes) == 1024, "wrong work count")
        return dict(row, submit_ms=(row["last_settlement_us"]-row["first_submit_us"])/1000,
                    drain_ms=(row["last_settlement_us"]-row["first_commit_us"])/1000,
                    final_write_ms=(writes[-1]-row["first_commit_us"])/1000,
                    middle_write_ms=(writes[921]-writes[102])/1000)

    def replay_block(path, block, config, remotes):
        require(run.read_json(path / "order.json") == block, "order changed")
        return {f"{phase}-{label}": score_arm(folder, phase, label, run.REMOTE / remotes[label])
                for phase, label, _, folder in matrix.paths(path, config, block)}

    def summarize(directory, config, schedule, pins):
        manifest = run.read_json(directory / "series.json")
        require(all(manifest[k] == v for k,v in dict(config=config,schedule=schedule,pins=pins).items()),
                "series binding changed")
        decisions, missing, failures = {}, [], {}
        for block in schedule:
            result = matrix.pick(directory, block, config, manifest["remotes"], failures)
            if result is None:
                missing.append(block["group"])
                continue
            attempt, rows = result
            require(rows == run.read_json(directory / "blocks" / attempt / "reconciled.json"), "score changed")
            decisions[block["group"]] = decision(rows)
        return dict(decisions=decisions, missing=missing, failures=failures)

    matrix.stages, matrix.grant, matrix.replay_block, matrix.summarize = stages, grant, replay_block, summarize
    matrix.SERIES[ID] = ID
    for label in ORDER:
        stages(config, label, run.REMOTE / label)
    if action == "check":
        subprocess.run(["bash", "-n", stage / FILES[-1]], check=True)
        compile((stage / "native-arm.py").read_bytes(), "native-arm.py", "exec")
        reference = sorted((REFERENCE.parent / "blocks").glob("*/reconciled.json"))[0]
        original = run.read_json(reference)["measured-d2-on"]
        folder = reference.parent / "measured-d2-on/measured-mesh"
        require(run.digest(folder / "raw.tar.gz") == original["raw_sha256"], "reference arm changed")
        with tarfile.open(folder / "raw.tar.gz") as archive:
            for label in ("hot-a", "hot-b"):
                row = score.score(archive, "measured", label, config, config["mesh_spec"][label])
                require(row["terminal_xids"] == original["terminal_xids"] and row["checkout_probes"] == 4,
                        "wrong reference evidence")
            for label in ("hot-zero", "hot-hold1"):
                try:
                    score.score(archive, "measured", label, config, config["mesh_spec"][label])
                except (ValueError, AssertionError, RuntimeError) as err:
                    require("arm mode differs" in str(err), "unexpected control rejection")
                else:
                    raise ValueError(f"accepted writer-off evidence for {label}")
        return 0
    if action == "replay":
        result = summarize(OUT, config, plan, pins)
        print(json.dumps(result, sort_keys=True, indent=2))
        return int(bool(result["missing"]))
    require(groups == [1, 2], "two reverse-order pilot groups required")
    dispatch_arm = run.dispatch_arm

    def dispatch_checked(path, remote, block, phase, engine):
        dispatch_arm(path, remote, block, phase, engine)
        folder = path / f"{phase}-{engine}"
        require(run.read_json(folder / "dispatch.json").get("exit_code") == 0, f"{path.name} failed")
        score_arm(folder, phase, path.name.removeprefix(phase + "-"), remote)

    run.dispatch_arm = dispatch_checked
    OUT.parent.mkdir(exist_ok=True)
    return matrix.dispatch(OUT, config, ID, plan, pins, groups)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("check", "replay", "timing"))
    parser.add_argument("groups", type=int, nargs="*")
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(dir=dense.ROOT / "target", prefix="dense-hold-adapter-") as temp:
        sys.exit(execute(args.action, args.groups, Path(temp)))
