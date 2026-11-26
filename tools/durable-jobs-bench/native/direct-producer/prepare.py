#!/usr/bin/env python3
"""Link a producer-only broker and stage a separate native benchmark adapter."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def replace_once(source, before, after):
    if source.count(before) != 1:
        raise ValueError(f"frozen source anchor changed: {before!r}")
    return source.replace(before, after)


def producer_source(source):
    source = replace_once(
        source, '    let receipt = transact_batch(conn, Json.object_get(request, "batch")?)?',
        '''    let path = Json.as_string(Json.object_get(request, "batch")?)?
    let fixture_dir = Env.get("JOBS_PARITY_FIXTURE_DIR", "")
    let request_id = Json.as_string(Json.object_get(request, "request_id")?)?
    if !String.starts_with(fixture_dir, "/") || !Regex.is_match(~r/^batch_[0-9]{6}$/, request_id) || path != fixture_dir <> "/" <> request_id <> ".json" do
      return Err("expected an absolute batch path in the private fixture directory")
    end
    let input = read_batch(path)?
    let current_id = Json.as_string(Json.object_get(input, "request_id")?)?
    if current_id != request_id do
      return Err("batch file does not match request ID")
    end
    let receipt = transact_batch(conn, input)?''',
    )
    begin = source.index("fn run() -> String!String do\n")
    end = source.index("\nfn main() do\n", begin)
    return source[:begin] + '''fn run() -> String!String do
  let argv = Env.args()
  if List.length(argv) != 2 || List.get(argv, 1) != "session" do
    return Err("usage: jobs-parity-direct-producer session")
  end
  if !Regex.is_match(~r/^[a-z][a-z0-9_]{0,62}$/, namespace()) do
    return Err("JOBS_PARITY_NAMESPACE must match [a-z][a-z0-9_]{0,62}")
  end
  if database_url() == "" do
    return Err("JOBS_PARITY_DB_URL is required")
  end
  session()
end
''' + source[end:]


def prepare_adapter(source, adapter):
    native = source / "tools/durable-jobs-bench/native"
    adapter.mkdir()
    for name in ("run", "ledger.py"):
        shutil.copy2(native / name, adapter / name)

    arm = (native / "arm.py").read_text()
    arm = replace_once(
        arm, '    path = PROOF / "trial.py"', '    path = HERE / "direct-trial.py"',
    )
    arm = replace_once(
        arm, '    command = artifacts / ("mesh" if engine == "mesh" else "oban")',
        '''    if engine != "mesh" or build.get("producer") != "mesh-direct-file":
        raise RuntimeError("expected a direct-file Mesh manifest")
    stock_manifest = Path(build["stock_manifest"])
    if h.digest(stock_manifest) != build["stock_manifest_sha256"]:
        raise RuntimeError("stock Mesh manifest changed")
    stock = json.loads(stock_manifest.read_text())
    command = stock_manifest.parent / "mesh"
    if stock["version"] != build["version"] or h.digest(command) != stock["files"]["mesh"]:
        raise RuntimeError("stock Mesh runner identity changed")''',
    )
    arm = replace_once(
        arm, '        subject = Subject(engine, [str(command)], artifacts, cluster, env)',
        '''        subject = Subject(engine, [str(command)], artifacts, cluster, env)
        subject.direct_producer = str(artifacts / "broker")
        subject.fixture_dir = scratch.resolve()''',
    )
    (adapter / "arm.py").write_text(arm)

    trial = (source / "proof/jobs-parity/linux-regression/trial.py").read_text()
    trial = replace_once(
        trial, '        output = evidence / "broker-output.log"',
        '''        env["JOBS_PARITY_FIXTURE_DIR"] = str(self.fixture_dir)
        output = evidence / "broker-output.log"''',
    )
    trial = replace_once(
        trial, '            [*self.command, "session"]', '            [self.direct_producer, "session"]',
    )
    trial = replace_once(
        trial, '''                batch = json.loads(path.read_text()) if self.engine == "mesh" else str(path)
                response = request({"verb": "enqueue", "batch": batch})''',
        '''                if not path.is_absolute() or path.parent.resolve() != self.fixture_dir or path.is_symlink():
                    raise ValueError("expected an absolute trusted batch path in private trial scratch")
                response = request({"verb": "enqueue", "batch": str(self.fixture_dir / path.name),
                                    "request_id": path.stem})''',
    )
    trial = replace_once(
        trial, "                cluster.broker = request",
        # A blank reply is represented as a newline so the poll loop can
        # distinguish it from an incomplete line. SQL NULL needs empty text.
        '                cluster.broker = lambda command: request(command).rstrip("\\n")',
    )
    (adapter / "direct-trial.py").write_text(trial)


def prepare(source, stock, meshc, runtime, output, adapter):
    stock_manifest = stock / "mesh.json"
    build = json.loads(stock_manifest.read_text())
    for name, checksum in build["files"].items():
        if digest(stock / name) != checksum:
            raise ValueError(f"stock artifact {name} changed")
    toolchain = (stock / "mesh-toolchain.txt").read_text()
    compiler_sha, runtime_sha = digest(meshc), digest(runtime)
    for name, checksum in (("meshc", compiler_sha), ("libmesh_rt.a", runtime_sha)):
        entries = [parts for line in toolchain.splitlines()
                   if len(parts := line.split()) == 2 and Path(parts[1]).name == name]
        if len(entries) != 1 or entries[0][0] != checksum:
            raise ValueError(f"{name} differs from the toolchain used for stock")
    output.mkdir()
    broker_source = output / "broker-source"
    shutil.copytree(stock / "mesh-source", broker_source)
    fixture = broker_source / "main.mpl"
    archive = source / "proof/jobs-parity/linux-warm-replay-20261122.tar.gz"
    with tarfile.open(archive, "r:gz") as frozen:
        if fixture.read_bytes() != frozen.extractfile("frozen-fixture/main.mpl").read():
            raise ValueError("stock build did not use the frozen Mesh fixture")
    fixture.write_text(producer_source(fixture.read_text()))
    broker = output / "broker"
    with (output / "broker-link.log").open("wb") as log:
        subprocess.run([meshc, "build", broker_source, "--opt-level", "2", "--output", broker],
                       env=dict(os.environ, MESH_RT_LIB_PATH=str(runtime)), stdout=log,
                       stderr=subprocess.STDOUT, check=True)
    prepare_adapter(source, adapter)
    files = {str(path.relative_to(output)): digest(path)
             for path in [broker, *sorted(broker_source.iterdir())]}
    manifest = dict(build, files=files, producer="mesh-direct-file",
                    stock_manifest=str(stock_manifest), stock_manifest_sha256=digest(stock_manifest),
                    stock_runner_sha256=build["files"]["mesh"], compiler_sha256=compiler_sha,
                    runtime_sha256=runtime_sha, fixture_archive_sha256=digest(archive),
                    adapter_files={path.name: digest(path) for path in sorted(adapter.iterdir())})
    (output / "mesh.json").write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("source", "stock", "meshc", "runtime", "output", "adapter"):
        parser.add_argument(name, type=Path)
    args = parser.parse_args()
    prepare(*(path.resolve() for path in vars(args).values()))


if __name__ == "__main__":
    main()
