#!/usr/bin/env python3
"""Link the candidate and isolated collector probes with the matched toolchain."""

import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
STATE = ROOT / "target/ordinary-broker-once"
DIRECT = ROOT / "target/linode-jobs-stepchange/direct-producer-v3"
ASSETS = DIRECT / "local-artifact-03"
ADAPTER = DIRECT / "local/tools/durable-jobs-bench/direct-adapter-03"
PINS = HERE / "pins/direct-local.json"


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def check(path, expected):
    actual = digest(path)
    if actual != expected:
        raise ValueError(f"{path.name}: {actual} != {expected}")
    return actual


def replace(source, old, new):
    if source.count(old) != 1:
        raise ValueError(f"expected one source anchor: {old!r}")
    return source.replace(old, new)


def component(source, candidate, trace):
    batch_start = source.index("fn enqueue_batches(")
    batch_end = source.index("\nfn transaction_id(", batch_start)
    chunk_loop = source[batch_start:batch_end]
    chunk_loop = replace(chunk_loop, "conn :: borrow PgConn, ", "")
    chunk_loop = replace(chunk_loop, "enqueue_batches(conn, ", "enqueue_batches(")
    chunk_loop = chunk_loop.replace("ids :: Map<String, String>", "chunks :: Int")
    chunk_loop = chunk_loop.replace("Map<String, String>!String", "Int!String")
    chunk_loop = replace(chunk_loop, """    let receipts = enqueue_specs(conn, specs, size)?
    let ids = collect_ids(receipts, jobs, index, 0, size, ids)?""", """    let batch_text = "[" <> String.join(specs, ",") <> "]"
    if batch_text == "" do return Err("missing serialized batch") end""" + (
        """
    let _ = File.write(Env.get("TRACE_DIR", "") <> "/" <> String.from(chunks), batch_text)?"""
        if trace else ""))
    chunk_loop = chunk_loop.replace("index + size, count, ids", "index + size, count, chunks + 1")
    chunk_loop = replace(chunk_loop, "Ok(ids)", "Ok(chunks)")
    start = source.index("fn job_spec(")
    stop = source.index("\nfn collect_ids(", start)
    collector = source[start:stop]
    if trace:
        if candidate:
            collector = replace(collector,
                "      EncodedSpec { text: text, bytes: Bytes.length(Bytes.from_utf8(text)) }",
                """      let bytes = Bytes.length(Bytes.from_utf8(text))
      IO.eprintln("ENCODE " <> String.from(index) <> " " <> String.from(bytes))
      EncodedSpec { text: text, bytes: bytes }""")
        else:
            collector = replace(collector, "  let spec_bytes = Bytes.length(Bytes.from_utf8(spec))",
                """  let spec_bytes = Bytes.length(Bytes.from_utf8(spec))
  IO.eprintln("ENCODE " <> String.from(index) <> " " <> String.from(spec_bytes))""")
    pending = ", None" if candidate else ""
    return collector + "\n" + chunk_loop + f"""
fn run() -> Unit!String do
  let input = Json.parse(File.read(Env.get("INPUT", ""))?)?
  let jobs = Json.object_get(input, "jobs")?
  let count = Json.array_length(jobs)?
  let started = Monotonic.now_nanos()
  let chunks = enqueue_batches(jobs, 0, count, 0{pending})?
  let elapsed = Monotonic.now_nanos() - started
  println(Json.encode(json {{ chunks: chunks, count: count, elapsed_ns: elapsed }}))
  Ok(())
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


def main(out):
    out.mkdir()
    pins = json.loads(PINS.read_text())
    compiler, runtime = DIRECT / "meshc", DIRECT / "libmesh_rt.a"
    actual = dict(compiler=check(compiler, pins["compiler_sha256"]),
                  runtime=check(runtime, pins["runtime_sha256"]),
                  stock_migrator=check(DIRECT / "local-stock/mesh", pins["stock_runner_sha256"]))
    for name, expected in pins["adapter_files"].items():
        check(ADAPTER / name, expected)
    for name, expected in pins["files"].items():
        check(ASSETS / name, expected)
    control = HERE / "broker-control.mpl"
    check(control, pins["files"]["broker-source/main.mpl"])
    candidate = HERE / "broker-once.mpl"
    shutil.copy2(ASSETS / "broker", out / "A")
    shutil.copy2(ASSETS / "broker", out / "B")
    actual["A"] = check(out / "A", pins["files"]["broker"])
    actual["B"] = check(out / "B", actual["A"])
    actual["postgres_adapter_files"] = pins["adapter_files"]
    actual["source"] = {"A/B": digest(control), "once": digest(candidate)}

    def link(name, source, full=False):
        project = out / f"{name}-source"
        if full:
            shutil.copytree(ASSETS / "broker-source", project)
        else:
            project.mkdir()
        (project / "main.mpl").write_text(source)
        with (out / f"{name}.log").open("wb") as log:
            subprocess.run([compiler, "build", project, "--opt-level", "2", "--output", out / name],
                           env=dict(os.environ, MESH_RT_LIB_PATH=str(runtime), TMPDIR=str(STATE / "tmp")),
                           stdout=log, stderr=subprocess.STDOUT, check=True)
        actual[name] = digest(out / name)

    link("once", candidate.read_text(), full=True)
    for label, path in (("A", control), ("once", candidate)):
        for mode in ("bench", "trace"):
            link(f"{mode}-{label}", component(path.read_text(), label == "once", mode == "trace"))
    (out / "pins.json").write_text(json.dumps(actual, indent=2, sort_keys=True) + "\n")


if __name__ == "__main__":
    main(Path(sys.argv[1]).resolve())
