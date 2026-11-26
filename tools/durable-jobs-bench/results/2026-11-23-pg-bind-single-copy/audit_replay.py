#!/usr/bin/env python3
"""Verify the archived Bind evidence and replay the pinned source differential."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import tempfile

import replay
from wire import Bridge

CANDIDATE = "436d705463a01e2fb4ecfecc493abbd3d79e69b3"
PRODUCT = "7e6c956048270de94d229841bff83d40577ea2c8"
PATCH_SHA256 = "579d6d39676b71e36920bc4f2f88ef308bd85e0bcc252abf2c8f4ad1701e7420"
PATHS = (replay.PG, "compiler/mesh-rt/src/db/pg/jobs_effect_session.rs")
METRICS = ("allocs", "reallocs", "peak_live_requested", "frame_bytes",
           "explicit_bind_body_copy_bytes")


def git(root, *args):
    return subprocess.check_output(["git", "-C", root, *args])


def snapshot(root, scratch):
    assert git(root, "rev-parse", f"{CANDIDATE}^").decode().strip() == replay.BASE
    assert tuple(git(root, "diff", "--name-only", replay.BASE, CANDIDATE).decode().splitlines()) == PATHS
    assert tuple(git(root, "diff", "--name-only", f"{PRODUCT}^", PRODUCT).decode().splitlines()) == PATHS

    patch = replay.HERE / "pg-bind-single-copy.patch"
    assert replay.digest(patch) == PATCH_SHA256
    assert patch.read_bytes().splitlines()[0].split()[1] == CANDIDATE.encode()
    patched = scratch / "patched-base"
    candidate = scratch / "isolated-source"
    for path in PATHS:
        dest = patched / path
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_bytes(git(root, "show", f"{replay.BASE}:{path}"))

    git(root, "apply", "--directory", str(patched.relative_to(root)), str(patch))
    blobs = {}
    for path in PATHS:
        isolated = git(root, "show", f"{CANDIDATE}:{path}")
        assert isolated == (patched / path).read_bytes() == git(root, "show", f"{PRODUCT}:{path}")
        dest = candidate / path
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_bytes(isolated)
        blobs[path] = {
            "git_blob": git(root, "rev-parse", f"{CANDIDATE}:{path}").decode().strip(),
            "sha256": hashlib.sha256(isolated).hexdigest(),
        }
    return candidate, blobs


def archived(root, scratch):
    evidence = replay.HERE / "evidence"
    summary = subprocess.check_output([sys.executable, "-B", replay.HERE / "summarize.py", evidence])
    assert summary == (replay.HERE / "summary.json").read_bytes()
    builds = json.loads((evidence / "builds.json").read_text())
    assert builds["base"] == replay.BASE
    assert builds["archives"]["optimized-a"] == builds["archives"]["optimized-b"]
    micro = json.loads((evidence / "micro.json").read_text())
    live = json.loads((evidence / "live.json").read_text())
    for data, shapes in ((micro, ("short", "scalar", "mixed", "jobs")), (live, ("short", "large", "jobs"))):
        for group, order in enumerate(replay.ORDERS, 1):
            for shape in shapes:
                rows = [row for row in data if row["group"] == group and row["shape"] == shape
                        and row.get("mode", "cpu") == "cpu"]
                assert tuple(row["arm"] for row in rows) == order
    report = json.loads(summary)
    for shape in ("short", "scalar", "mixed", "jobs"):
        stock = report["micro_alloc"][f"{shape}/stock"]
        candidate = report["micro_alloc"][f"{shape}/optimized-a"]
        assert 0 < stock["explicit_bind_body_copy_bytes"]
        assert candidate["explicit_bind_body_copy_bytes"] == 0
        assert candidate["peak_live_requested"] < stock["peak_live_requested"]
    wire = json.loads((evidence / "wire.json").read_text())
    assert wire["input_sha256"] == replay.LARGE_INPUT_SHA
    for shape in ("large", "jobs"):
        assert wire["results"][f"{shape}-stock"]["wire"] == wire["results"][f"{shape}-optimized-a"]["wire"]
    parity = replay.load_parity(root)
    batch = replay.job_inputs(parity, scratch)
    return report, wire, parity, batch


def allocs(scratch, shapes, archived_summary):
    report = {}
    for shape in shapes:
        for arm in ("stock", "optimized-a"):
            raw = replay.run(
                scratch / "alloc", arm, shape, "alloc", scratch / "job-capture", capture_output=True
            ).stdout
            fields = dict(line.split("=") for line in raw.splitlines())
            result = {key: int(fields[key]) for key in METRICS}
            assert result == {key: archived_summary["micro_alloc"][f"{shape}/{arm}"][key] for key in METRICS}
            report[f"{shape}/{arm}"] = result
    return report


def compile_fixture(compiler, runtime, project, executable):
    trace = executable.with_suffix(".trace.json")
    env = dict(os.environ, MESH_RT_LIB_PATH=str(runtime), MESH_BUILD_TRACE_PATH=str(trace))
    replay.run(compiler, "build", project, "-o", executable, env=env, capture_output=True)
    receipt = json.loads(trace.read_text())
    assert receipt["success"] and Path(receipt["runtimeLibraryPath"]).resolve() == runtime
    return replay.digest(executable)


def wire_replay(root, scratch, wire, parity, batch):
    runtime = (root / "target/debug/libmesh_rt.a").resolve()
    compiler = runtime.with_name("meshc")
    build = {p.name: replay.digest(p) for p in (runtime, compiler)}
    fixture = scratch / "fixture"
    fixture.mkdir()
    (fixture / "main.mpl").write_bytes((replay.HERE / "main.mpl").read_bytes())
    jobs = replay.jobs_fixture(root, scratch / "jobs-fixture")
    bind_executable = scratch / "bind-integrated"
    jobs_executable = scratch / "jobs-integrated"
    build[bind_executable.name] = compile_fixture(compiler, runtime, fixture, bind_executable)
    build[jobs_executable.name] = compile_fixture(compiler, runtime, jobs, jobs_executable)
    assert build[runtime.name] == replay.digest(runtime) and build[compiler.name] == replay.digest(compiler)

    os.environ.pop("MESH_JOBS_SET_BASED_ENQUEUE", None)
    logs = scratch / "postgres-logs"
    logs.mkdir()
    cluster = parity.Cluster(scratch, logs, 32, "17.11")
    results = {}
    try:
        cluster.start()
        replay.new_db(cluster, "bind_template")
        migration = dict(cluster.env, JOBS_PARITY_DB_URL=cluster.url("bind_template"),
                         JOBS_PARITY_NAMESPACE="parity_ordered_fixture", JOBS_PARITY_TOTAL_SLOTS="64")
        replay.run(jobs_executable, "migrate", env=migration, capture_output=True)
        for shape in ("large", "jobs"):
            replay.new_db(cluster, "bind_wire", "bind_template" if shape == "jobs" else None)
            with Bridge(cluster, scratch / "job-capture" if shape == "jobs" else None) as bridge:
                if shape == "jobs":
                    verified = replay.jobs_trial(cluster, "bind_wire", jobs_executable, batch, scratch, bridge.port)
                else:
                    verified = replay.bind_trial(cluster, "bind_wire", bind_executable, 393216, 2, scratch, bridge.port)
            transcript = replay.keep_wire(bridge)
            expected = wire["results"][f"{shape}-stock"]["wire"]
            assert transcript == expected, (shape, transcript["transcript_sha256"], expected["transcript_sha256"])
            replay.drop_db(cluster, "bind_wire")
            counts = {key: verified[key] for key in ("jobs", "enqueue_calls", "committed_rows",
                                                     "committed_transactions") if key in verified}
            results[shape] = {"transcript": transcript, "verified": counts}
        assert wire["jobs_enqueue_arg_bytes"] == [
            (scratch / f"job-capture.{i}").stat().st_size for i in range(4)
        ]
    finally:
        cluster.stop()
    return {"runtime_path": str(runtime.relative_to(root)), "sha256": build, "results": results}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--wire", action="store_true",
                        help="controller-only historical diagnostic, including allocation instrumentation")
    parser.add_argument("--stage-native", type=Path,
                        help="copy the hash-pinned extracted Rust into a prepared checkout directory")
    args = parser.parse_args()
    if args.wire and (sys.platform != "linux" or platform.machine() != "x86_64"):
        parser.error("--wire includes measurement instrumentation and requires the native Linux controller")
    root = Path(git(replay.HERE, "rev-parse", "--show-toplevel").decode().strip())
    target = root / "target"
    target.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="bind-audit-", dir=target) as directory:
        scratch = Path(directory)
        os.environ["TMPDIR"] = str(scratch)
        candidate, blobs = snapshot(root, scratch)
        summary, wire, parity, batch = archived(root, scratch)
        differential = replay.extract(root, candidate, scratch)
        assert differential + "\n" == (replay.HERE / "evidence/differential.txt").read_text()
        evidence = {
            "stock_commit": replay.BASE,
            "isolated_candidate": CANDIDATE,
            "integrated_commit": PRODUCT,
            "identical_product_blobs": blobs,
            "raw_patch_sha256": replay.digest(replay.HERE / "pg-bind-single-copy.patch"),
            "compiled_extracted_source_sha256": replay.digest(scratch / "extracted.rs"),
            "rustc": replay.run("rustc", "-Vv", capture_output=True).stdout.strip(),
            "source_differential": differential,
            "jobs_input_sha256": replay.LARGE_INPUT_SHA,
            "archived_summary_matches": True,
            "archived_wire_pair_digests_match": True,
        }
        if args.stage_native:
            stage = args.stage_native.resolve()
            if not stage.is_relative_to(root):
                parser.error("--stage-native must stay in the checkout")
            shutil.copyfile(scratch / "extracted.rs", stage / "extracted.rs")
            shutil.copyfile(scratch / "micro.rs", stage / "micro.rs")
        if args.wire:
            evidence["integrated_wire"] = wire_replay(root, scratch, wire, parity, batch)
            evidence["fresh_driver_allocations"] = allocs(scratch, ("short", "scalar", "mixed", "jobs"), summary)
        print(json.dumps(evidence, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
