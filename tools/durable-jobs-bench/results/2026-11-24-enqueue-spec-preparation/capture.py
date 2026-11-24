#!/usr/bin/env python3
"""Untimed wire capture, using the exact broker's collector and batch assembly."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile

HERE = Path(__file__).resolve().parent


def sha(data):
    return hashlib.sha256(data).hexdigest()


def write(path, doc):
    path.write_text(json.dumps(doc, sort_keys=True, indent=2) + "\n")


def collector(source, short):
    start = source.index("fn job_spec(")
    end = source.index("\nfn collect_ids(", start)
    collect = source[start:end]
    batch = source.split('fn enqueue_specs(', 1)[1].split("\n", 2)[1]
    assert batch == '  let batch = "[" <> String.join(specs, ",") <> "]"'
    stop = source.split("fn enqueue_batches(", 1)[1].splitlines()[2]
    assert stop == "    let stop = if count - index > 128 do index + 128 else count end"
    select = '  let input = Json.object_get(root, "batch")?\n' if short else "  let input = root\n"
    return collect + """
fn capture(jobs :: Json, index :: Int, count :: Int, frame :: Int) -> Unit!String do
  if index < count do
""" + stop + """
    let specs = collect_specs(jobs, index, stop, [], 2)?
    let size = List.length(specs)
""" + batch + """
    let _ = File.write(Env.get("CAPTURE_DIR", "") <> "/" <> String.from(frame), batch)?
    capture(jobs, index + size, count, frame + 1)
  else
    Ok(())
  end
end

fn run() -> Unit!String do
  let root = Json.parse(File.read(Env.get("CAPTURE_INPUT", ""))?)?
""" + select + """  let jobs = Json.object_get(input, "jobs")?
  capture(jobs, 0, Json.array_length(jobs)?, 0)
end

fn main() do
  case run() do
    Ok(_) -> ()
    Err(error) ->
      IO.eprintln(error)
      Process.exit(1)
  end
end
"""


def capture(profile, source_file, work, compiler, runtime, ledger):
    short = profile == "short"
    work.mkdir()
    project = work / "project"
    project.mkdir()
    source = source_file.read_text()
    (project / "main.mpl").write_text(collector(source, short))
    # The profile and generator are pinned to the producer trial source. The
    # durable witness below independently proves every key and payload hash.
    source_generator = (HERE / "fixture-generator.py").read_text()
    settings = json.loads((HERE / "fixture-profiles.json").read_text())
    state = {}
    exec(source_generator, state)
    slots = 16 if short else 64
    batches = state["inputs"](settings["workloads"][profile], slots, settings["seed"])
    batch, = batches
    transport = (json.dumps({"verb": "enqueue", "batch": json.loads(
        json.dumps(batch, indent=2, sort_keys=True))}, separators=(",", ":")) + "\n"
                 if short else json.dumps(batch, indent=2, sort_keys=True, allow_nan=False) + "\n")
    path = work / "transport.json"
    path.write_text(transport)
    frames = work / "frames"
    frames.mkdir()
    executable = work / "capture"
    with (work / "link.log").open("wb") as log:
        subprocess.run([compiler, "build", project, "--opt-level", "2", "--output", executable],
                       env=dict(os.environ, MESH_RT_LIB_PATH=str(runtime)),
                       stdout=log, stderr=subprocess.STDOUT, check=True)
    with (work / "capture.log").open("wb") as log:
        subprocess.run([executable], env=dict(os.environ, CAPTURE_INPUT=str(path),
                       CAPTURE_DIR=str(frames), JOBS_PARITY_MODE="dense"), stdout=log,
                       stderr=subprocess.STDOUT, check=True)
    captures = []
    jobs = []
    for index in range(len(list(frames.iterdir()))):
        data = (frames / str(index)).read_bytes()
        specs = json.loads(data)
        assert len(specs) == (128 if short else 1) and len(data) <= 1048576
        captures.append({"jobs": len(specs), "bytes": len(data), "sha256": sha(data)})
        for spec in specs:
            job = spec["args"]
            assert spec == {"worker": "parity", "args": job, "request_key": job["key"],
                            "max_attempts": 3, "backoff_base_ms": 1000, "backoff_max_ms": 1000}
            jobs.append(job)
    assert jobs == batch["jobs"] and len(jobs) == (128 if short else 64)
    with tarfile.open(ledger, "r:gz") as archive:
        with archive.extractfile("adapter/ledger.json") as f:
            witness = json.load(f)
    transaction, = witness["transactions"]
    assert transaction["committed_us"] >= transaction["submitted_us"]
    assert witness["profile"] == profile and witness["phase"] == "measured" and witness["slots"] == slots
    assert {row["id"] for row in witness["jobs"]} == set(transaction["enqueued_ids"].values())
    durable = {row["job_key"]: row["payload_sha256"] for row in witness["jobs"]}
    assert len(durable) == len(jobs)
    for row in jobs:
        assert durable[row["key"]] == sha(row["payload"].encode())
    write(work / "capture.json", {
        "profile": profile, "broker_source_sha256": sha(source.encode()),
        "collector_sha256": sha((project / "main.mpl").read_bytes()),
        "compiler_sha256": sha(compiler.read_bytes()), "runtime_sha256": sha(runtime.read_bytes()),
        "collector_executable_sha256": sha(executable.read_bytes()),
        "fixture_transport_sha256": sha(transport.encode()), "jobs": len(jobs),
        "raw_ledger_archive_sha256": sha(ledger.read_bytes()), "frames": captures,
    })


if __name__ == "__main__":
    capture(sys.argv[1], *(Path(p).resolve() for p in sys.argv[2:]))
