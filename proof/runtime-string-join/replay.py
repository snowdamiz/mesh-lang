#!/usr/bin/env python3
"""Build and compare compiled Mesh join workloads against two stock arms."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import runpy
import shutil
import statistics
import subprocess
import time

ROOT = Path(__file__).resolve().parents[2]
FROZEN = ROOT / "proof/jobs-parity/linux-integrated-uncut/fixture/main.mpl"
ASSETS = Path(__file__).resolve().parent / "probe"
MASK = (1 << 63) - 1

def sha(data):
    return hashlib.sha256(data).hexdigest()

def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()

def checksum(value, acc):
    size = len(value)
    acc = acc * 1000003 + size
    for index in (0, size // 4, size // 2, size - 1):
        acc = acc * 257 + value[index]
    return acc & MASK

def workloads(out):
    assert digest(FROZEN) == "0c72aaa2f0f34c49d07fd2f6cad2d39f491c0857897a0ac7a0facb6b85b4464d"
    source = FROZEN.read_text()
    expression, = re.findall(r"^  let batch = (.*String\.join\(specs,.*)$", source, re.M)
    frozen = (
        "from Bindings import cpu_ns, checksum\n\n"
        + source[source.index("fn job_spec("):source.index("fn collect_ids(")]
        + (ASSETS / "frozen-tail.mpl").read_text().replace("BATCH_EXPRESSION", expression)
    )
    fields = [field.encode() for field in (
        "event=service.start", "region=eu-west-1", "user=Zoë", "path=/api/export",
        "bytes=1048576", "ok=true", "note=準備完了", "instance=blue-17",
    )]
    separator = " · ".encode()
    ordinary = [fields, fields + [b"trace_id=6e46212af88443938749c0a8097cb922"]]
    rendered = [separator.join(parts) for parts in ordinary]
    yield "ordinary", (ASSETS / "ordinary.mpl").read_text(), None, 250000, rendered, list(map(sha, rendered)), ordinary

    harness = runpy.run_path(str(ROOT / "scripts/jobs-parity.py"))
    settings = json.loads(harness["CONFIG"].read_text())
    for label, profile, rounds in (("large", "large", 16), ("dense", "noop", 2000)):
        batch, = harness["inputs"](settings["workloads"][profile], 64, settings["seed"])
        folder = out / "inputs" / label
        folder.mkdir(parents=True)
        path, = harness["batch_files"](folder, [batch])
        if label == "large":
            assert digest(path) == "30f91354336fab21dbe85f2933f3347ec53ab7634af3805169a892536747fa1c"
        parts = []
        for job in json.loads(path.read_text())["jobs"]:
            spec = dict(worker="parity", args=job, request_key=job["key"], max_attempts=3,
                        backoff_base_ms=1000, backoff_max_ms=1000)
            parts.append(json.dumps(spec, separators=(",", ":")).encode())
        groups = [parts[i:i + (1 if label == "large" else 128)]
                  for i in range(0, len(parts), 1 if label == "large" else 128)]
        rendered = [b"[" + b",".join(group) + b"]" for group in groups]
        audit = [f"{len(group)} {len(value)} {sha(value)}" for group, value in zip(groups, rendered)]
        yield label, frozen, path, rounds, rendered, audit, groups

def invoke(command, env, log):
    result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=120)
    log.write_text(result.stdout + result.stderr)
    if result.returncode:
        raise RuntimeError(f"{command[0]} exited {result.returncode}, see {log}")
    return result.stdout

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("compiler", "stock", "candidate", "out"):
        parser.add_argument(name, type=lambda p: Path(p).resolve())
    parser.add_argument("--target", default="aarch64-apple-darwin")
    parser.add_argument("--rotations", type=int, default=6)
    args = parser.parse_args()
    if args.rotations < 1:
        parser.error("--rotations must be positive")
    args.out.mkdir()
    tmp = args.out / "tmp"
    tmp.mkdir()
    env = dict(os.environ, TMPDIR=f"{tmp}/", MESH_SCHEDULER_MIN_WORKERS="1",
               MESH_SCHEDULER_MAX_WORKERS="1", JOBS_PARITY_MODE="dense")
    env.pop("MESH_GC_STRESS", None)
    native_object, native_archive = args.out / "probe.o", args.out / "probe.a"
    invoke(["cc", "-std=c11", "-O2", "-c", ASSETS / "probe.c", "-o", native_object],
           env, args.out / "cc.log")
    invoke(["ar", "rcs", native_archive, native_object], env, args.out / "ar.log")
    manifest = {name: digest(getattr(args, name)) for name in ("compiler", "stock", "candidate")}
    manifest.update(frozen_source=digest(FROZEN), native_object=digest(native_object),
                    native_archive=digest(native_archive), target=args.target)
    for case, source, input_path, rounds, values, audit, groups in workloads(args.out):
        project = args.out / case
        project.mkdir()
        (project / "main.mpl").write_text(source)
        shutil.copyfile(ASSETS / "bindings.mpl", project / "bindings.mpl")
        os.link(native_archive, project / "probe.a")
        (project / "mesh.toml").write_text(f"""[package]
name = "{case}"
version = "0.1.0"

[native]
abi = 1
bindings = ["bindings.mpl"]

[[native.libraries]]
target = "{args.target}"
path = "probe.a"
sha256 = "{digest(native_archive)}"
""")
        case_env = dict(env, PROBE_ROUNDS=str(rounds))
        if input_path:
            case_env["PROBE_INPUT"] = str(input_path)
        checksum_expected = 0
        for _ in range(rounds):
            for value in values:
                checksum_expected = checksum(value, checksum_expected)
        case_manifest = dict(
            source=digest(project / "main.mpl"), bindings=digest(project / "bindings.mpl"),
            input_sha256=digest(input_path) if input_path else None,
            rounds=rounds, checksum=checksum_expected, audit=audit,
            joins=len(values) * rounds, elements=sum(map(len, groups)) * rounds,
            join_bytes=sum(len(value) - (2 if input_path else 0) for value in values) * rounds,
        )
        manifest[case] = case_manifest
        variants = ("stock", "candidate")
        for variant in variants:
            exe = project / variant
            build_env = dict(env, MESH_RT_LIB_PATH=str(getattr(args, variant)))
            invoke([args.compiler, "build", project, "--target", args.target, "--opt-level", "2",
                    "--emit-llvm", "-o", exe], build_env, project / f"{variant}-build.log")
            case_manifest[variant] = digest(exe)
        ir = (project / "stock.ll").read_bytes()
        assert b"call ptr @mesh_string_join(" in ir, "compiled join must be live"
        assert ir == (project / "candidate.ll").read_bytes()
        case_manifest["ir"] = sha(ir)

        def sample(variant, stem, run_env, expected_checksum):
            start = time.perf_counter_ns()
            lines = invoke([project / variant], run_env, project / f"{stem}.log").splitlines()
            wall = time.perf_counter_ns() - start
            sum_observed, cpu = map(int, lines.pop().split())
            assert lines == audit, (case, variant, lines[:3], audit[:3])
            assert sum_observed == expected_checksum and cpu > 0, (case, variant, sum_observed, cpu)
            return dict(executable=variant, checksum=sum_observed, thread_cpu_ns=cpu, wall_ns=wall)

        if case == "ordinary":
            stress_rounds = 1200
            expected = 0
            for _ in range(stress_rounds):
                for value in values:
                    expected = checksum(value, expected)
            for variant in variants:
                sample(variant, f"{variant}-gc-stress",
                       dict(case_env, PROBE_ROUNDS=str(stress_rounds), MESH_GC_STRESS="1"), expected)
        rows = []
        for rotation in range(args.rotations):
            order = ["stock-a", "candidate", "stock-b"]
            if rotation % 2:
                order = ["candidate", "stock-b", "stock-a"]
            for label in order:
                variant = "stock" if label.startswith("stock-") else label
                row = sample(variant, f"{rotation}-{label}", case_env, checksum_expected)
                rows.append(dict(rotation=rotation, label=label, load=os.getloadavg()[0], **row))
                print(case, rotation, label, f"{row['thread_cpu_ns']/1e6:.3f} ms", flush=True)
            (project / "samples.json").write_text(json.dumps(rows, indent=2) + "\n")
        for label in dict.fromkeys(row["label"] for row in rows):
            values_ms = [row["thread_cpu_ns"] / 1e6 for row in rows if row["label"] == label]
            print(case, label, f"{statistics.median(values_ms):.3f} [{min(values_ms):.3f}, {max(values_ms):.3f}] ms")
        (args.out / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")

if __name__ == "__main__":
    main()
