#!/usr/bin/env python3
"""Measure general runtime calls inside a separate warm broker diagnostic."""

import argparse
from collections import defaultdict
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

from prepare import digest, replace_once


def instrument(source):
    def stage(name, arguments, result, call):
        return f"""fn measured_{name}({arguments}) -> {result} do
  let start = Monotonic.now_nanos()
  let value = {call}
  IO.eprintln("PRODUCER_STAGE {name} " <> String.from(Monotonic.now_nanos() - start))
  value
end

"""
    helpers = ""
    for name, args, result, call in (
        ("file_read", "path :: String", "String!String", "File.read(path)"),
        ("json_parse", "data :: String", "Json!String", "Json.parse(data)"),
        ("json_encode", "value :: Json", "String", "Json.encode(value)"),
        ("bytes_from_utf8", "text :: String", "Bytes", "Bytes.from_utf8(text)"),
        ("string_join", "items :: List<String>", "String", 'String.join(items, ",")'),
    ):
        helpers += stage(name, args, result, call)
    source = replace_once(source, '  Json.parse(File.read(path)?)',
                           '  measured_json_parse(measured_file_read(path)?)')
    source = replace_once(source, "  Ok(Json.encode(json {\n    worker:", "  Ok(measured_json_encode(json {\n    worker:")
    source = replace_once(source, "Bytes.length(Bytes.from_utf8(spec))",
                           "Bytes.length(measured_bytes_from_utf8(spec))")
    source = replace_once(source, 'String.join(specs, ",")', 'measured_string_join(specs)')
    source = replace_once(
        source, '  let receipts = Json.parse(Jobs.enqueue_in(conn, namespace(), batch)?)?',
        '''  let stage_start = Monotonic.now_nanos()
  let response = Jobs.enqueue_in(conn, namespace(), batch)?
  IO.eprintln("PRODUCER_STAGE jobs_enqueue_in " <> String.from(Monotonic.now_nanos() - stage_start))
  let receipts = Json.parse(response)?''',
    )
    # Print a delimiter at the actual start of each enqueue request. The
    # positive 64-job batch is first and followed by one explicit rollback.
    source = replace_once(source, '  if verb == "enqueue" do',
                          '  if verb == "enqueue" do\n    IO.eprintln("PRODUCER_BEGIN")')
    return helpers + source


def measure(args):
    source, out = args.artifact, args.output
    manifest = json.loads((source / "mesh.json").read_text())
    assert manifest["producer"] == "mesh-direct-file"
    assert digest(args.meshc) == manifest["compiler_sha256"]
    assert digest(args.runtime) == manifest["runtime_sha256"]
    for name, checksum in manifest["files"].items():
        assert digest(source / name) == checksum, name
    out.mkdir()
    project = out / "broker-source"
    shutil.copytree(source / "broker-source", project)
    mesh = project / "main.mpl"
    mesh.write_text(instrument(mesh.read_text()))
    with (out / "link.log").open("wb") as log:
        subprocess.run([args.meshc, "build", project, "--opt-level", "2", "--output", out / "broker"],
                       env=dict(os.environ, MESH_RT_LIB_PATH=str(args.runtime)), stdout=log,
                       stderr=subprocess.STDOUT, check=True)
    manifest["files"]["broker-source/main.mpl"] = digest(mesh)
    manifest["files"]["broker"] = digest(out / "broker")
    manifest["diagnostic"] = "source stage timers. Exclude from timed comparison."
    (out / "mesh.json").write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
    proof = out / "proof"
    subprocess.run([sys.executable, "-B", Path(__file__).with_name("prove.py"),
                    args.adapter, out, args.state, proof], check=True)
    prove = json.loads((proof / "proof.json").read_text())
    first, _ = (proof / "accepted-and-rollback/broker-errors.log").read_text().strip().split("PRODUCER_BEGIN\n")[1:]
    stages = defaultdict(list)
    for line in first.splitlines():
        marker, name, elapsed_ns = line.split()
        assert marker == "PRODUCER_STAGE" and int(elapsed_ns) >= 0
        stages[name].append(int(elapsed_ns))
    assert {name: len(values) for name, values in stages.items()} == {
        "file_read": 1, "json_parse": 1, "json_encode": 127,
        "bytes_from_utf8": 127, "string_join": 64, "jobs_enqueue_in": 64,
    }
    tx = prove["transactions"]["batch_000000"]
    result = dict(system=manifest["system"], architecture=manifest["architecture"], input_bytes=prove["input_bytes"],
                  submit_to_receipt_ms=(tx["acknowledged_us"] - tx["submitted_us"]) / 1000,
                  stages={name: dict(calls=len(values), total_wall_ms=sum(values) / 1e6)
                          for name, values in stages.items()})
    (out / "attribution.json").write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("artifact", "meshc", "runtime", "adapter", "state", "output"):
        parser.add_argument(name, type=lambda value: Path(value).resolve())
    measure(parser.parse_args())
