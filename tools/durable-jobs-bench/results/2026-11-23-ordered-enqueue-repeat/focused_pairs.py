"""Dispatch or replay five alternating stock/candidate Mesh pairs per repeat profile."""

import argparse
import fcntl
import json
import tarfile
import time
from uuid import uuid4

from repeat_evidence import BASELINE, host, metrics, paired, run

VARIANTS = ("baseline", "candidate")


def order(block):
    return list(VARIANTS if block["group"] % 2 else reversed(VARIANTS))


def replay_pair(path, manifest, block):
    run.require(run.read_json(path / "order.json") == dict(block, order=order(block)),
                "pair does not follow the alternating plan")
    scores = {}
    for phase in block["phases"]:
        for name in order(block):
            folder = path / f"{phase}-{name}" / f"{phase}-mesh"
            score = run.score_arm(folder, block, phase, "mesh", manifest["inventory"][name])
            run.require(host(score) == manifest["reference"]["host"], "pair changed the host, kernel, or disk")
            if phase == "measured":
                scores[name] = metrics(folder, score)
    return dict(scores, group=block["group"], order=order(block), attempt=path.name)


def pair_attempts(directory, block):
    return sorted((directory / "pairs").glob(f"{run.block_prefix(block)}.*"))


def validated(directory, manifest, block, issues):
    for path in pair_attempts(directory, block):
        try:
            return replay_pair(path, manifest, block)
        except (OSError, ValueError, KeyError, TypeError, tarfile.TarError) as error:
            issues[path.name] = str(error)
    return None


def get_summary(directory, manifest):
    profiles, missing, issues = {}, [], {}
    for block in manifest["plan"]:
        result = validated(directory, manifest, block, issues)
        if result:
            profiles.setdefault(block["profile"], []).append(result)
        else:
            missing.append(run.block_prefix(block))
    run.require(not missing, f"missing alternating pairs: {missing}. Invalid attempts: {issues}")
    return {"reference": manifest["reference"], "invalid_or_incomplete_attempts": issues,
            "profiles": {profile: {"pairs": rows, "distribution": paired(rows)}
                         for profile, rows in profiles.items()}}


def stage_variants(directory, manifest):
    for name in VARIANTS:
        folder = directory / name
        folder.mkdir(exist_ok=True)
        inventory = folder / "inventory.json"
        if not inventory.is_file():
            run.write_json(inventory, manifest["inventory"][name])
        run.require(run.read_json(inventory) == manifest["inventory"][name], f"{name} inventory changed")
        with (folder / f"stage-{uuid4().hex}.txt").open("xb") as log:
            remote = run.REMOTE / manifest["host_series"][name]
            run.stage(run.SUITE / "worker.py", remote, log)
            run.stage(inventory, remote, log)


def dispatch_pair(directory, manifest, block):
    for prior in pair_attempts(directory, block):
        for phase in block["phases"]:
            for name in order(block):
                arm = prior / f"{phase}-{name}" / f"{phase}-mesh"
                if (arm / "request.json").is_file():
                    run.archive_arm(arm, run.REMOTE / manifest["host_series"][name],
                                    run.read_json(arm / "request.json")["attempt"])
    if validated(directory, manifest, block, {}):
        return
    path = directory / "pairs" / f"{run.block_prefix(block)}.{time.time_ns()}"
    path.mkdir(parents=True)
    run.write_json(path / "order.json", dict(block, order=order(block)))
    for phase in block["phases"]:
        for name in order(block):
            folder = path / f"{phase}-{name}"
            folder.mkdir()
            run.dispatch_arm(folder, run.REMOTE / manifest["host_series"][name], block, phase, "mesh")
    checked = replay_pair(path, manifest, block)
    run.write_json(path / "reconciled.json", checked)
    print(f"{path.name} reconciled", flush=True)


def dispatch(directory, repeat, groups):
    candidate = repeat["inventory"]
    baseline = run.read_json(run.RESULTS / BASELINE / "inventory.json")
    reference = run.verify_reference(BASELINE, candidate)
    run.require(repeat["kind"] == "repeat" and repeat["reference"] == reference, "unverified repeat")
    identity = {"reference": reference, "inventory": dict(baseline=baseline, candidate=candidate),
                "plan": repeat["plan"], "worker_sha256": run.digest(run.SUITE / "worker.py"),
                "scorer_sha256": run.digest(run.SUITE / "verify.py"),
                "driver_sha256": run.digest(run.SUITE / "run.py")}
    manifest_path = directory / "series.json"
    with (run.SUITE / ".timing.lock").open("r") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        directory.mkdir(exist_ok=True)
        if not manifest_path.is_file():
            remote = {name: f"{directory.parent.name}-pairs-{name}-{uuid4().hex}" for name in VARIANTS}
            run.write_json(manifest_path, dict(identity, host_series=remote))
        manifest = run.read_json(manifest_path)
        run.require(all(manifest[key] == value for key, value in identity.items()), "focused series identity changed")
        stage_variants(directory, manifest)
        for block in manifest["plan"]:
            if not groups or block["group"] in groups:
                dispatch_pair(directory, manifest, block)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("dispatch", "summarize"))
    parser.add_argument("series", help="Name of the four-engine repeat in results/")
    parser.add_argument("--group", type=int, choices=range(1, 6), action="append")
    args = parser.parse_args()
    series = (run.RESULTS / args.series).resolve()
    run.require(series.parent == run.RESULTS, "use an archived series name")
    directory = series / "focused"
    if args.action == "dispatch":
        dispatch(directory, run.read_json(series / "series.json"), args.group)
    else:
        manifest = run.read_json(directory / "series.json")
        repeat = run.read_json(series / "series.json")
        baseline = run.read_json(run.RESULTS / BASELINE / "inventory.json")
        run.require(manifest["inventory"] == dict(baseline=baseline, candidate=repeat["inventory"])
                    and manifest["plan"] == repeat["plan"], "focused series differs from the repeat plan and inventories")
        reference = run.verify_reference(BASELINE, manifest["inventory"]["candidate"])
        run.require(reference == manifest["reference"], "focused series reference changed")
        print(json.dumps(get_summary(directory, manifest), indent=2, sort_keys=True, allow_nan=False))


if __name__ == "__main__":
    main()
