#!/usr/bin/env python3
"""Replay portable diagnostic arms under the local shared timing lock."""
import argparse
import fcntl
import json
import os
from pathlib import Path
from statistics import median
import subprocess
import sys

from qualify import HERE, ROOT, check_arm, check_faults, check_runner_log, check_teardown, manifest

ORDERS = (("off", "d2"), ("d2", "off")) * 2 + (("off", "d2"),)
METRICS = ("submit_jobs_per_s", "commit_jobs_per_s")


def load(directory):
    return lambda name: json.loads((directory / name).read_text())


def paired(groups):
    result = {}
    for metric in METRICS:
        values = [100 * (g["scores"]["d2"][metric] / g["scores"]["off"][metric] - 1) for g in groups]
        result[metric] = dict(median=median(values), min=min(values), max=max(values),
                              wins=sum(value > 0 for value in values))
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("work", type=Path, help="candidate work directory containing artifacts and witness")
    parser.add_argument("bulk_source", type=Path)
    parser.add_argument("broker", type=Path, help="locally rebuilt official bulk artifact directory")
    parser.add_argument("baseline", type=Path, help="exact archived local r2 runner artifact directory")
    parser.add_argument("baseline_witness", type=Path)
    parser.add_argument("out", type=Path)
    args = parser.parse_args()
    work, bulk_source, broker, baseline, witness, out = (
        getattr(args, key).resolve() for key in
        ("work", "bulk_source", "broker", "baseline", "baseline_witness", "out"))
    candidate = work / "artifacts"
    manifests = {"off": manifest(baseline), "d2": manifest(candidate)}
    broker_seal = manifest(broker)
    assert broker_seal["manifest"] == "ac239c8296a50d6439b1c5f86e626a138c70a36b4442a8c27d72fc979564d18a"
    assert manifests["off"]["manifest"] == "2e1c6a1b56a2b8a67e0659c7f8960420311a47ea80225affb35bfb8be498d3cb"
    assert manifests["d2"]["elf"] != manifests["off"]["elf"]
    assert manifest(witness)["manifest"] == "6eb67edf93b1ed864a74d13af06af296d464bdfa8546316b79b4452f340817ba"
    manifests_on_disk = [(directory / "mesh.json").read_bytes()
                         for directory in (baseline, candidate, witness, work / "witness")]
    a, b, aw, bw = map(json.loads, manifests_on_disk)
    assert a["compiler_sha256"] == b["compiler_sha256"]
    for name in ("main.mpl", "writes.mpl", "worker_identity.mpl", "worker_identity.c"):
        assert a["files"][name] == b["files"][name]
    assert a["runtime_sha256"] == aw["runtime_sha256"] and b["runtime_sha256"] == bw["runtime_sha256"]
    out.mkdir()
    for label, build in zip(("baseline", "d2", "baseline-witness", "d2-witness"), manifests_on_disk, strict=True):
        (out / f"{label}-build.json").write_bytes(build)
    state = work / "state"
    state.mkdir(exist_ok=True)
    env = dict(os.environ, COMBINED_BULK_SOURCE=str(bulk_source), NATIVE_JOBS_ARTIFACTS=str(broker),
               NATIVE_JOBS_STATE=str(state), MESH_JOBS_SET_BASED_ENQUEUE="1",
               TRUSTED_ADMISSION_POLICY="success_cohort", DURABLE_JOBS_PHASE="diagnostic")
    env.pop("TRUSTED_COMPLETION_WRITER_MS", None)
    env.pop("MESH_PG_HOT_CHECKOUT", None)

    def run(script, label, *args, **mode):
        with (out / f"{label}.log").open("x") as log:
            subprocess.run([sys.executable, "-B", str(HERE / script), *map(str, args)], check=True,
                           env=dict(env, **mode), stdout=log, stderr=subprocess.STDOUT, timeout=150)
        print(label, "passed", flush=True)

    def smoke(label, mode):
        directory = out / label
        runner = {"off": baseline, "d2": candidate, "d2-disabled": candidate}[mode]
        run("split_arm.py", label, "--local-smoke", "mesh", "short", directory,
            EXIT_BULK_CHECKOUT=mode, COMBINED_RUNNER_ARTIFACTS=str(runner))
        check_runner_log((directory / "runner-0.log").read_text())
        result = check_arm(load(directory), mode, manifests["off" if mode == "off" else "d2"], native=False)
        return result

    with (ROOT / "target/queue-performance.lock").open("rb") as queue:
        fcntl.flock(queue, fcntl.LOCK_EX)
        (out / "host-load.txt").write_text(subprocess.check_output(["uptime"], text=True))
        default = smoke("candidate-default", "d2-disabled")
        groups = []
        for index, order in enumerate(ORDERS, 1):
            scores = {mode: smoke(f"group-{index}-{mode}", mode) for mode in order}
            groups.append(dict(group=index, order=order, scores=scores))
        with (out / "host-load.txt").open("a") as log:
            subprocess.run(["uptime"], stdout=log, check=True)
    for mode in ("off", "d2", "d2-forced"):
        label = "teardown-" + mode
        run("teardown.py", label, out / label, broker / "mesh",
            (witness if mode == "off" else work / "witness") / "mesh", mode)
        check_teardown(load(out / label)("teardown.json"), mode)
    run("faults.py", "d2-faults", out / "d2-faults", broker / "mesh", candidate / "mesh", "d2")
    faults = check_faults(load(out / "d2-faults"), manifests["d2"]["elf"], broker_seal["elf"])
    run("faults.py", "off-faults", out / "off-faults", broker / "mesh", baseline / "mesh", "off",
        "lost-application-reply", "lost-claim-reply", "lost-terminal-reply")
    off_faults = check_faults(load(out / "off-faults"), manifests["off"]["elf"], broker_seal["elf"], "off")
    summary = dict(runners=manifests, broker=broker_seal, candidate_disabled=default, groups=groups,
                   faults=faults, off_faults=off_faults, paired_percent=paired(groups))
    (out / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")


if __name__ == "__main__":
    main()
