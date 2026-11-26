#!/usr/bin/env python3
"""Compile ordinary Mesh and frozen broker probes and time fresh child processes."""

import argparse
import gzip
import hashlib
import importlib.util
import itertools
import json
import os
from pathlib import Path
import random
import resource
import runpy
import shutil
import statistics
import subprocess
import sys
import time

HERE = Path(__file__).resolve().parent
ROOT = Path(os.environ["MESH_ESCAPE_ROOT"]).resolve() if "MESH_ESCAPE_ROOT" in os.environ else HERE.parents[3]
STATE = Path(os.environ.get("MESH_ESCAPE_STATE", ROOT / "target/general-json-escape")).resolve()
BINS = STATE / "bins"
FROZEN = ROOT / "target/linode-jobs-stepchange/direct-producer-v3"
ARTIFACT = FROZEN / "local-artifact-03"
SIBLING = Path(os.environ.get("MESH_ESCAPE_SIBLING", HERE.parent / "2026-11-23-ordinary-broker-once"))
FROZEN_PIN = Path(os.environ.get("MESH_ESCAPE_FROZEN_PIN", HERE.parent / "2026-11-23-direct-producer-v3-setup/local-manifest.json"))
WORKLOADS = {"broker-large": "large", "broker-short": "slow", "broker-small": "noop"}
TREATMENTS = ("A", "B", "wide", "reserve", "wide-reserve")


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def write(path, value):
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")


def json_text(value):
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"))


def import_sibling(name):
    sys.path.insert(0, str(SIBLING))
    try:
        spec = importlib.util.spec_from_file_location(name, SIBLING / f"{name}.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module
    finally:
        sys.path.remove(str(SIBLING))


def link(arms):
    source = STATE / "ordinary-source"
    source.mkdir(exist_ok=True)
    (source / "main.mpl").write_bytes((HERE / "ordinary.mpl").read_bytes())
    manifest = json.loads(FROZEN_PIN.read_text())
    compiler = FROZEN / "meshc"
    assert digest(compiler) == manifest["compiler_sha256"]
    assert digest(FROZEN / "libmesh_rt.a") == manifest["runtime_sha256"]
    for name, sha in manifest["files"].items():
        assert digest(ARTIFACT / name) == sha, name
    build = import_sibling("build")
    original = STATE / "frozen-broker-source"
    if not original.exists():
        shutil.copytree(ARTIFACT / "broker-source", original)
    original_code = (original / "main.mpl").read_text()
    assert (SIBLING / "broker-control.mpl").read_text() == original_code

    projects = dict(ordinary=source, broker=original)
    for template, trace in (("broker-component", False), ("broker-trace", True)):
        source = STATE / f"{template}-source"
        source.mkdir(exist_ok=True)
        (source / "main.mpl").write_text(build.component(original_code, False, trace))
        projects[template] = source

    pins = {}
    for arm in arms:
        runtime = FROZEN / "libmesh_rt.a"
        if arm != "original":
            archive = BINS / f"{arm}.a"
            runtime = BINS / arm / "libmesh_rt.a"
            runtime.parent.mkdir(exist_ok=True)
            if not runtime.exists():
                os.link(archive, runtime)
            assert digest(archive) == digest(runtime)
        for name, project in projects.items():
            binary = BINS / f"{name}-{arm}"
            if arm == "original" and name == "broker":
                binary = ARTIFACT / "broker"
            else:
                with (STATE / f"logs/link-{name}-{arm}.log").open("wb") as log:
                    subprocess.run(
                        [compiler, "build", project, "--opt-level", "2", "--output", binary],
                        env=dict(os.environ, MESH_RT_LIB_PATH=str(runtime), TMPDIR=str(STATE / "tmp")),
                        stdout=log,
                        stderr=subprocess.STDOUT,
                        check=True,
                    )
            pins[f"{name}-{arm}"] = dict(binary=digest(binary), runtime=digest(runtime))
    write(STATE / ("link-" + "-".join(arms) + ".json"), pins)


def fixtures():
    out = STATE / "fixtures"
    out.mkdir(exist_ok=True)
    rng = random.Random(120347677)
    alphabets = [(0x20, 0x7F), (0xA0, 0x500), (0x600, 0x900), (0x4E00, 0x9FFF), (0x1F300, 0x1FAC0)]
    controls = [chr(c) for c in range(32)] + ['"', "\\"]
    ranges = [range(low, high) for low, high in alphabets]

    def story(n, period=0):
        chars = []
        for _ in range(n):
            if period and rng.randrange(period) == 0:
                ch = rng.choice(controls)
            else:
                ch = chr(rng.choice(rng.choice(ranges)))
                if ch in ('"', "\\"):
                    ch = "/"
            chars.append(ch)
        return "".join(chars)

    def rows(payloads):
        return [
            {"name": f"résumé {i}", "ok": i % 2 == 0, "count": i, "content": payload,
             "extra": [None, {"k": "雪 / 𐐷"}, -112, []]}
            for i, payload in enumerate(payloads)
        ]

    vectors = {
        "small": (rows([story(rng.randint(0, 25), 12) for _ in range(96)]), 2400),
        "unicode": (rows([story(rng.randint(15_000, 26_000)) for _ in range(48)]), 70),
        "mixed": (rows([story(rng.randint(10_000, 21_000), 151) for _ in range(48)]), 75),
        "dense": (rows([story(rng.randint(4_000, 9_000), 2) for _ in range(32)]), 38),
        "long-keys": ([{story(2500, 31): {"x": i, "str": "tiny"}} for i in range(24)], 65),
    }
    vectors["dense-late"] = (rows([story(2400) + story(rng.randint(6500, 11_000), 2)
                                  for _ in range(32)]), 28)
    vectors["bursts"] = (rows(["".join(story(600, 2) + story(2200) for _ in range(7))
                               for _ in range(24)]), 62)
    spec = {}
    for name, (values, rounds) in vectors.items():
        path = out / f"{name}.json"
        path.write_text(json_text(values))
        expected = [json_text(v).encode() for v in values]
        payload = b"\n".join(expected) + b"\n"
        spec[name] = dict(rounds=rounds, count=len(values), path=path.name,
                          sha256=digest(path), bytes_per_round=sum(map(len, expected)),
                          output_sha256=hashlib.sha256(payload).hexdigest())

    raw = r'''[{"z":0,"q\u0075ote":"obsolete","é":"\u0000\u001f\b\t\n\f\r\"\\/\ud83d\ude03\u2028\u2029","quote":[-9223372036854775808,18446744073709551615,-0.0,1.0,1e300,5e-324],"a":{}},{"":"", "z":"a","a":"b","z":"c"}]'''
    path = out / "edges.json"
    path.write_text(raw)
    expected = ('{"z":0,"quote":[-9223372036854775808,18446744073709551615,-0.0,1.0,1e+300,5e-324],'
                '"é":"\\u0000\\u001f\\b\\t\\n\\f\\r\\"\\\\/😃\u2028\u2029","a":{}}\n'
                '{"":"","z":"c","a":"b"}\n').encode()
    spec["edges"] = dict(rounds=0, count=2, path=path.name, sha256=digest(path),
                         output_sha256=hashlib.sha256(expected).hexdigest())

    harness = runpy.run_path(str(ROOT / "scripts/jobs-parity.py"))
    config = json.loads((ROOT / "scripts/jobs-parity-profiles.json").read_text())
    for name, profile in WORKLOADS.items():
        incoming, = harness["inputs"](config["workloads"][profile], 64, config["seed"])
        folder = out / name
        folder.mkdir(exist_ok=True)
        path = folder / f"{incoming['request_id']}.json"
        if path.exists():
            assert path.read_text() == json.dumps(incoming, indent=2, sort_keys=True, allow_nan=False) + "\n"
        else:
            path, = harness["batch_files"](folder, [incoming])
        count = len(incoming["jobs"])
        spec[name] = dict(path=str(path.relative_to(out)), profile=profile, count=count,
                          payload_bytes=config["workloads"][profile]["payload_bytes"],
                          sha256=digest(path))
    write(out / "manifest.json", spec)
    return spec


def invoke(binary, path, **overrides):
    env = dict(os.environ, INPUT=str(path), TMPDIR=str(STATE / "tmp"),
               JOBS_PARITY_MODE="dense", TRACE="")
    env.update(overrides)
    before = resource.getrusage(resource.RUSAGE_CHILDREN)
    start = time.monotonic_ns()
    result = subprocess.run([binary], env=env, capture_output=True, timeout=180)
    clock = time.monotonic_ns() - start
    after = resource.getrusage(resource.RUSAGE_CHILDREN)
    if result.returncode or (result.stderr and "TRACE_DIR" not in overrides):
        raise RuntimeError((binary, path, result.returncode, result.stdout[-500:], result.stderr[-4000:]))
    cpu = dict(user_ns=round((after.ru_utime - before.ru_utime) * 1e9),
               system_ns=round((after.ru_stime - before.ru_stime) * 1e9), process_clock_ns=clock)
    return result, cpu


def binary_for(kind, arm):
    return BINS / f"{kind}-{'stock' if arm in ('A', 'B') else arm}"


def prove(spec, arms):
    proofs = {}
    outputs = STATE / "verified-output"
    outputs.mkdir(exist_ok=True)
    for name, item in spec.items():
        proofs[name] = {}
        path = STATE / "fixtures" / item["path"]
        if name not in WORKLOADS:
            for arm in arms:
                result, _ = invoke(binary_for("ordinary", arm), path, TRACE="1")
                output_sha = hashlib.sha256(result.stdout).hexdigest()
                if output_sha != item["output_sha256"]:
                    (STATE / f"logs/failed-{name}-{arm}.out").write_bytes(result.stdout)
                    raise AssertionError((name, arm, output_sha, item["output_sha256"]))
                if arm == "A":
                    with gzip.open(outputs / f"{name}.jsonl.gz", "wb") as stream:
                        stream.write(result.stdout)
                proofs[name][arm] = dict(sha=output_sha, bytes=len(result.stdout), rows=item["count"])
            continue

        trials = json.loads(path.read_text())["jobs"]
        expected = [
            dict(worker="parity", args=job, request_key=job["key"], max_attempts=3,
                 backoff_base_ms=1000, backoff_max_ms=1000)
            for job in trials
        ]
        for arm in arms:
            trace = STATE / "traces" / name / arm
            trace.mkdir(parents=True, exist_ok=True)
            result, _ = invoke(binary_for("broker-trace", arm), path, TRACE_DIR=str(trace))
            (trace / "stderr.log").write_bytes(result.stderr)
            meta = json.loads(result.stdout)
            seen, chunks = [], []
            for i in range(meta["chunks"]):
                segment = trace / str(i)
                payload = segment.read_bytes()
                specs = json.loads(payload)
                assert 1 <= len(specs) <= 128 and len(payload) <= 1_048_576
                seen.extend(specs)
                chunks.append(dict(bytes=len(payload), rows=len(specs),
                                   sha=hashlib.sha256(payload).hexdigest()))
                if arm == "A":
                    with gzip.open(outputs / f"{name}-{i:03}.json.gz", "wb") as stream:
                        stream.write(payload)
                segment.unlink()
            assert seen == expected and meta["count"] == len(expected)
            counts = [line.split() for line in result.stderr.decode().splitlines()]
            assert all(len(line) == 3 and line[0] == "ENCODE" for line in counts)
            events = [(int(index), int(size)) for _, index, size in counts]
            assert {i for i, _ in events} == set(range(len(expected)))
            proofs[name][arm] = dict(chunks=chunks, encodes=len(events),
                                    encode_bytes=sum(n for _, n in events), events=events)
            assert proofs[name][arm] == proofs[name][arms[0]]
        if name == "broker-large":
            assert len(proofs[name][arms[0]]["chunks"]) == 64
            assert proofs[name][arms[0]]["encodes"] == 127
    write(STATE / "correctness.json", proofs)
    return proofs


def measure(spec, proofs, arms, rotations):
    output = STATE / "component.jsonl"
    with output.open("a") as records:
        for i in range(rotations):
            for name, item in spec.items():
                if name == "edges":
                    continue
                path = STATE / "fixtures" / item["path"]
                for arm in rotated(arms, i):
                    kind = "broker-component" if name in WORKLOADS else "ordinary"
                    binary = binary_for(kind, arm)
                    options = {} if name in WORKLOADS else dict(ROUNDS=str(item["rounds"]))
                    result, timing = invoke(binary, path, **options)
                    clock = json.loads(result.stdout)
                    assert clock["count"] == item["count"]
                    verified = proofs[name][arm]
                    if name in WORKLOADS:
                        assert clock["chunks"] == len(verified["chunks"])
                        timing["bytes"] = verified["encode_bytes"]
                        timing["encodes"] = verified["encodes"]
                    else:
                        assert clock["bytes"] == item["rounds"] * item["bytes_per_round"]
                        timing["bytes"] = clock["bytes"]
                        timing["encodes"] = item["rounds"] * clock["count"]
                    row = dict(workload=name, rotation=i, arm=arm, fixture_sha256=item["sha256"],
                               loadavg=os.getloadavg(), timed_region_ns=clock["elapsed_ns"], **timing)
                    records.write(json.dumps(row) + "\n")
                    records.flush()
            print("completed rotation", i + 1, flush=True)


def rotated(arms, rotation):
    order = arms[rotation % len(arms):] + arms[:rotation % len(arms)]
    return order[::-1] if rotation // len(arms) % 2 else order


def summarize(path):
    data = [json.loads(line) for line in path.read_text().splitlines()]
    grouped = {}
    for group, rows in itertools.groupby(sorted(data, key=lambda r: (r["workload"], r["arm"])),
                                          key=lambda r: (r["workload"], r["arm"])):
        samples = list(rows)
        times = {"cpu_ms": [(r["user_ns"] + r["system_ns"]) / 1e6 for r in samples]}
        times.update({key: [r[key] / 1e6 for r in samples]
                      for key in ["timed_region_ns", "process_clock_ns"]})
        grouped.setdefault(group[0], {})[group[1]] = dict(
            count=len(samples), output_bytes=samples[0]["bytes"], encodes=samples[0]["encodes"],
            **{name: [min(values), statistics.median(values), max(values)] for name, values in times.items()}
        )
    for arms in grouped.values():
        baseline = statistics.median([arms[a]["cpu_ms"][1] for a in ("A", "B") if a in arms])
        for arm in arms.values():
            arm["cpu_pct_vs_dup_control"] = (arm["cpu_ms"][1] / baseline - 1) * 100
    write(STATE / "component-summary.json", grouped)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("command", choices=("link", "fixtures", "prove", "measure", "summarize"))
    parser.add_argument("--arms", nargs="+", default=list(TREATMENTS))
    parser.add_argument("--rotations", type=int, default=5)
    args = parser.parse_args()
    if args.command == "link":
        link(args.arms)
    elif args.command == "fixtures":
        fixtures()
    elif args.command == "prove":
        prove(json.loads((STATE / "fixtures/manifest.json").read_text()), args.arms)
    elif args.command == "measure":
        measure(json.loads((STATE / "fixtures/manifest.json").read_text()),
                json.loads((STATE / "correctness.json").read_text()), args.arms, args.rotations)
    else:
        summarize(STATE / "component.jsonl")
