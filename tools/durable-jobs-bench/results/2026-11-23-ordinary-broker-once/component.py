#!/usr/bin/env python3
"""Check full serialized call membership and time the compiled collector off-DB."""

import hashlib
import json
import os
from pathlib import Path
import runpy
import subprocess
import sys

from build import ROOT, STATE, check, digest

ORDER = (
    ("A", "once", "B"), ("once", "B", "A"), ("B", "A", "once"),
    ("A", "B", "once"), ("B", "once", "A"), ("once", "A", "B"),
)
PROFILES = json.loads((ROOT / "scripts/jobs-parity-profiles.json").read_text())
HARNESS = runpy.run_path(str(ROOT / "scripts/jobs-parity.py"))
LIMIT = 1_048_576


def write(path, contents):
    path.write_text(json.dumps(contents, indent=2, sort_keys=True) + "\n")


def job(key, payload="", hold_ms=0):
    return dict(key=key, payload=payload, hold_ms=hold_ms, fail_first=False)


def batch(number, jobs, rollback=False):
    return dict(request_id=f"batch_{number:06d}", jobs=jobs, rollback=rollback)


def spec(job):
    return dict(worker="parity", args=job, request_key=job["key"], max_attempts=3,
                backoff_base_ms=1000, backoff_max_ms=1000)


def bound(key, size):
    empty_bytes = len(json.dumps(spec(job(key)), separators=(",", ":")).encode())
    return job(key, "x" * (size - empty_bytes))


def examples():
    for label, profile, slots in (("large", "large", 64), ("slow", "slow", 64),
                                   ("dense", "noop", 64), ("sparse", "sparse", 1)):
        yield label, HARNESS["inputs"](PROFILES["workloads"][profile], slots, PROFILES["seed"])
    yield "mixed", [batch(0, [
        job(f"mixed_{i:03d}", (chr(233) + "🙂" + "\\\t\n") * copies, i % 9)
        for i, copies in enumerate(([0, 1, 3, 34, 0, 1] * 22) +
                                    [40_000, 2, 80_000, 5, 40_000, 2, 0])
    ])]
    # Include both brackets and the comma. A third spec must start a new call.
    yield "exact", [batch(0, [bound("one", 350_000), bound("two", LIMIT - 350_003), job("three")])]
    yield "count", [batch(0, [job(f"count_{i:03d}", "x") for i in range(129)])]
    yield "empty", [batch(0, [])]


def invoke(exe, path, dest=None):
    env = dict(os.environ, INPUT=str(path), TMPDIR=str(STATE / "tmp"), JOBS_PARITY_MODE="dense")
    if dest:
        dest.mkdir()
        env["TRACE_DIR"] = str(dest)
    result = subprocess.run([exe], env=env, capture_output=True, timeout=90)
    if dest:
        (dest / "stderr.log").write_bytes(result.stderr)
        (dest / "stdout.log").write_bytes(result.stdout)
    if result.returncode:
        raise RuntimeError(result.stderr.decode(errors="replace"))
    return json.loads(result.stdout)


def measure(out, bins):
    out.mkdir()
    pins = json.loads((bins / "pins.json").read_text())
    work = out / "fixtures"
    work.mkdir()
    samples = []
    outcomes = {}
    for label, batches in examples():
        case = work / label
        case.mkdir()
        paths = HARNESS["batch_files"](case, batches)
        outcomes[label] = []
        for path, fixture in zip(paths, batches, strict=True):
            expected = [spec(job) for job in fixture["jobs"]]
            instance = {"input_sha256": digest(path), "jobs": len(expected)}
            for treatment in ("A", "once"):
                trace = case / f"{path.stem}-{treatment}"
                signal = f"trace-{treatment}"
                check(bins / signal, pins[signal])
                result = invoke(bins / signal, path, trace)
                flattened, chunks = [], []
                for i in range(result["chunks"]):
                    chunk = trace / str(i)
                    payload = chunk.read_bytes()
                    specs = json.loads(payload)
                    assert 1 <= len(specs) <= 128 and len(payload) <= LIMIT
                    flattened.extend(specs)
                    chunks.append(dict(keys=[row["args"]["key"] for row in specs],
                                       bytes=len(payload), sha256=digest(chunk)))
                    chunk.unlink()
                assert flattened == expected
                counter = [row.split() for row in (trace / "stderr.log").read_text().splitlines()]
                assert all(len(row) == 3 and row[0] == "ENCODE" for row in counter)
                events = [(int(i), int(n)) for _, i, n in counter]
                assert sorted({i for i, _ in events}) == list(range(len(expected)))
                measured_bytes = sum(item["bytes"] - 1 - len(item["keys"]) for item in chunks)
                if treatment == "once":
                    assert [i for i, _ in events] == list(range(len(expected)))
                    assert sum(n for _, n in events) == measured_bytes
                instance[treatment] = dict(chunks=chunks, encode_count=len(events),
                                            encode_bytes=sum(n for _, n in events))
            assert instance["A"]["chunks"] == instance["once"]["chunks"]
            outcomes[label].append(instance)

        if label not in {"large", "slow", "dense", "sparse", "mixed"}:
            continue
        for rotation, order in enumerate(ORDER, 1):
            for arm in order:
                executable = f"bench-{'A' if arm == 'B' else arm}"
                # Both A and B call the same pinned build; each starts a fresh process.
                check(bins / executable, pins[executable])
                rows = [invoke(bins / executable, path) for path in paths]
                assert [row["chunks"] for row in rows] == [len(i["A"]["chunks"]) for i in outcomes[label]]
                samples.append(dict(profile=label, arm=arm, rotation=rotation,
                                    measurements=rows, loadavg=os.getloadavg()))
        write(out / "timing.json", samples)
    assert outcomes["exact"][0]["A"]["chunks"][0]["bytes"] == LIMIT
    assert [len(c["keys"]) for c in outcomes["count"][0]["A"]["chunks"]] == [128, 1]
    assert outcomes["large"][0]["A"]["encode_count"] == 127
    assert len(outcomes["large"][0]["A"]["chunks"]) == 64
    write(out / "counts-and-membership.json", outcomes)


if __name__ == "__main__":
    measure(*(Path(arg).resolve() for arg in sys.argv[1:]))
